//! SMS send planning (GSM-7 vs UCS2, text mode) and decoding of received messages.

use std::fmt;

use crate::at::RawSms;
use crate::ucs2::{is_gsm_safe, ucs2_hex_to_utf8, utf8_to_ucs2_hex};

pub const MAX_GSM_CHARS: usize = 160;
pub const MAX_UCS2_UNITS: usize = 70;
pub const MAX_NUMBER_LEN: usize = 20;

/// Commands that put the modem back into its default (IRA / GSM-7) state after an SMS operation.
pub const RESTORE_CHARSET: &str = "AT+CSCS=\"IRA\"";
pub const RESTORE_CSMP: &str = "AT+CSMP=17,167,0,0";
pub const UCS2_CHARSET: &str = "AT+CSCS=\"UCS2\"";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmsError {
    InvalidNumber,
    TooLong { limit: usize, unicode: bool },
}

impl fmt::Display for SmsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNumber => write!(f, "invalid number"),
            Self::TooLong { limit, unicode: false } => write!(f, "message longer than {limit} characters"),
            Self::TooLong { limit, unicode: true } => write!(f, "unicode message longer than {limit} characters"),
        }
    }
}

impl std::error::Error for SmsError {}

/// The AT sequence for sending one single-segment SMS in text mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendPlan {
    pub charset_cmd: &'static str,
    pub csmp_cmd: &'static str,
    /// `AT+CMGS="<number>"\r`, answered by a `>` prompt.
    pub cmgs_cmd: String,
    /// Message text followed by Ctrl-Z.
    pub payload: String,
}

pub fn valid_number(number: &str) -> bool {
    let digits = number.strip_prefix('+').unwrap_or(number);
    !digits.is_empty() && number.len() <= MAX_NUMBER_LEN && digits.bytes().all(|b| b.is_ascii_digit())
}

pub fn plan_send(to: &str, body: &str) -> Result<SendPlan, SmsError> {
    if !valid_number(to) {
        return Err(SmsError::InvalidNumber);
    }
    if is_gsm_safe(body) {
        if body.len() > MAX_GSM_CHARS {
            return Err(SmsError::TooLong { limit: MAX_GSM_CHARS, unicode: false });
        }
        return Ok(SendPlan {
            charset_cmd: RESTORE_CHARSET,
            csmp_cmd: RESTORE_CSMP,
            cmgs_cmd: format!("AT+CMGS=\"{to}\"\r"),
            payload: format!("{body}\x1A"),
        });
    }
    let (hex_body, units) = utf8_to_ucs2_hex(body);
    if units > MAX_UCS2_UNITS {
        return Err(SmsError::TooLong { limit: MAX_UCS2_UNITS, unicode: true });
    }
    let (hex_to, _) = utf8_to_ucs2_hex(to);
    Ok(SendPlan {
        charset_cmd: UCS2_CHARSET,
        csmp_cmd: "AT+CSMP=17,167,0,8",
        cmgs_cmd: format!("AT+CMGS=\"{hex_to}\"\r"),
        payload: format!("{hex_body}\x1A"),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedSms {
    pub from: String,
    pub timestamp: String,
    pub body: String,
}

/// Decodes a message read with `AT+CSCS="UCS2"`. Returns `None` for stored outgoing messages.
pub fn decode_received(raw: &RawSms) -> Option<ReceivedSms> {
    let decode = |s: &str| ucs2_hex_to_utf8(s).unwrap_or_else(|| s.to_string());
    if !decode(&raw.stat).starts_with("REC") {
        return None;
    }
    Some(ReceivedSms { from: decode(&raw.sender), timestamp: decode(&raw.timestamp), body: decode(&raw.body) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gsm_plan() {
        let plan = plan_send("+15551234567", "Hello!").unwrap();
        assert_eq!(plan.charset_cmd, RESTORE_CHARSET);
        assert_eq!(plan.cmgs_cmd, "AT+CMGS=\"+15551234567\"\r");
        assert_eq!(plan.payload, "Hello!\x1A");
    }

    #[test]
    fn unicode_plan_encodes_number_and_body() {
        let plan = plan_send("+1", "é").unwrap();
        assert_eq!(plan.charset_cmd, UCS2_CHARSET);
        assert_eq!(plan.csmp_cmd, "AT+CSMP=17,167,0,8");
        assert_eq!(plan.cmgs_cmd, "AT+CMGS=\"002B0031\"\r");
        assert_eq!(plan.payload, "00E9\x1A");
    }

    #[test]
    fn limits() {
        assert_eq!(plan_send("123", &"a".repeat(161)), Err(SmsError::TooLong { limit: 160, unicode: false }));
        assert!(plan_send("123", &"a".repeat(160)).is_ok());
        assert_eq!(plan_send("123", &"é".repeat(71)), Err(SmsError::TooLong { limit: 70, unicode: true }));
        assert_eq!(plan_send("12;ATH", "hi"), Err(SmsError::InvalidNumber));
        assert_eq!(plan_send("+", "hi"), Err(SmsError::InvalidNumber));
    }

    #[test]
    fn decodes_received_ucs2() {
        let raw = RawSms {
            stat: "REC UNREAD".into(),
            sender: "002B0031".into(),
            timestamp: "25/10/25,14:03:11+32".into(),
            body: "004800690020D83DDC4B".into(),
        };
        assert_eq!(
            decode_received(&raw),
            Some(ReceivedSms { from: "+1".into(), timestamp: "25/10/25,14:03:11+32".into(), body: "Hi 👋".into() })
        );
        assert_eq!(decode_received(&RawSms { stat: "STO SENT".into(), ..raw }), None);
    }
}
