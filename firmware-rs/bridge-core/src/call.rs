//! Call state derived from `AT+CLCC`.

use serde::Serialize;

use crate::at::ClccEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CallState {
    Idle,
    Incoming,
    Dialing,
    Alerting,
    Active,
    Held,
    Waiting,
}

impl CallState {
    fn from_clcc(stat: u8) -> Self {
        match stat {
            0 => Self::Active,
            1 => Self::Held,
            2 => Self::Dialing,
            3 => Self::Alerting,
            4 => Self::Incoming,
            5 => Self::Waiting,
            _ => Self::Idle,
        }
    }

    /// Which call to present when several exist (active beats ringing beats held).
    fn rank(self) -> u8 {
        match self {
            Self::Active => 3,
            Self::Incoming | Self::Dialing | Self::Alerting => 2,
            Self::Held | Self::Waiting => 1,
            Self::Idle => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSnapshot {
    pub state: CallState,
    pub number: String,
    pub outgoing: bool,
}

impl CallSnapshot {
    pub fn idle() -> Self {
        Self { state: CallState::Idle, number: String::new(), outgoing: false }
    }
}

/// The voice call to present to the phone, or idle if there is none.
pub fn current_call(entries: &[ClccEntry]) -> CallSnapshot {
    entries
        .iter()
        .filter(|e| e.voice)
        .map(|e| CallSnapshot { state: CallState::from_clcc(e.stat), number: e.number.clone(), outgoing: e.outgoing })
        .filter(|c| c.state != CallState::Idle)
        .max_by_key(|c| c.state.rank())
        .unwrap_or_else(CallSnapshot::idle)
}

/// URCs that mean the call state may have changed.
pub fn is_call_urc(line: &str) -> bool {
    const TRIGGERS: [&str; 8] =
        ["RING", "+CLIP:", "+CRING:", "NO CARRIER", "BUSY", "NO ANSWER", "^DSCI:", "+QIND: \"ccinfo\""];
    TRIGGERS.iter().any(|t| line.starts_with(t))
}

pub fn valid_dial_string(number: &str) -> bool {
    (1..=20).contains(&number.len()) && number.chars().all(|c| "0123456789*#+".contains(c))
}

pub fn valid_dtmf(digit: char) -> bool {
    "0123456789*#ABCD".contains(digit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::at::parse_clcc;

    #[test]
    fn picks_active_over_held() {
        let calls = parse_clcc("+CLCC: 1,0,1,0,0,\"111\",129\r\n+CLCC: 2,1,0,0,0,\"222\",129\r\nOK");
        assert_eq!(
            current_call(&calls),
            CallSnapshot { state: CallState::Active, number: "222".into(), outgoing: false }
        );
    }

    #[test]
    fn ignores_data_calls_and_empty_lists() {
        assert_eq!(current_call(&parse_clcc("+CLCC: 1,0,0,1,0\r\nOK")), CallSnapshot::idle());
        assert_eq!(current_call(&[]), CallSnapshot::idle());
    }

    #[test]
    fn urc_triggers() {
        assert!(is_call_urc("RING"));
        assert!(is_call_urc("NO CARRIER"));
        assert!(!is_call_urc("+CMTI: \"SM\",1"));
    }

    #[test]
    fn validation() {
        assert!(valid_dial_string("+15551234567"));
        assert!(!valid_dial_string("555;ATH"));
        assert!(!valid_dial_string(""));
        assert!(valid_dtmf('#') && !valid_dtmf('x'));
    }
}
