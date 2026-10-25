//! The iPhone ⇄ bridge JSON protocol (see docs/protocol.md). One object per `\n`-terminated line.

use serde::{Deserialize, Serialize};

use crate::call::{CallSnapshot, CallState};

/// Phone → bridge.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Command {
    Status,
    Dial {
        num: String,
    },
    Answer,
    Hangup,
    Dtmf {
        d: String,
    },
    SmsSend {
        #[serde(rename = "ref")]
        reference: String,
        to: String,
        body: String,
    },
    SmsAck {
        id: u32,
    },
    SmsSync,
    /// Ask for the current hotspot credentials (answered with `Event::Wifi`).
    WifiGet,
    /// Change the hotspot credentials; persisted and applied immediately.
    WifiSet {
        ssid: String,
        pass: String,
    },
}

pub fn parse_command(line: &str) -> Result<Command, serde_json::Error> {
    serde_json::from_str(line)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    pub sim: bool,
    pub reg: bool,
    pub data: bool,
    pub op: String,
    pub rat: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rssi: Option<i32>,
    pub ip: String,
    pub ssid: String,
    /// Battery state of charge in percent; omitted on boards without a fuel gauge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bat: Option<u8>,
    /// Battery is charging; omitted on boards without a charger.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chg: Option<bool>,
    /// USB input power present; omitted on boards without a charger.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    In,
    Out,
}

/// Bridge → phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Event {
    Status(Status),
    Call {
        state: CallState,
        num: String,
        dir: Direction,
    },
    Sms {
        id: u32,
        from: String,
        ts: String,
        body: String,
    },
    SmsSent {
        #[serde(rename = "ref")]
        reference: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        err: Option<String>,
    },
    /// Current hotspot credentials (reply to `wifi_get` and to a successful `wifi_set`).
    Wifi {
        ssid: String,
        pass: String,
    },
    Err {
        ctx: String,
        msg: String,
    },
}

impl Event {
    pub fn call(call: &CallSnapshot) -> Self {
        Self::Call {
            state: call.state,
            num: call.number.clone(),
            dir: if call.outgoing { Direction::Out } else { Direction::In },
        }
    }

    pub fn error(ctx: impl Into<String>, msg: impl Into<String>) -> Self {
        Self::Err { ctx: ctx.into(), msg: msg.into() }
    }

    /// Serialized JSON plus the terminating newline.
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("event serialization cannot fail");
        line.push('\n');
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(parse_command(r#"{"t":"answer"}"#).unwrap(), Command::Answer);
        assert_eq!(parse_command(r#"{"t":"dial","num":"+1555"}"#).unwrap(), Command::Dial { num: "+1555".into() });
        assert_eq!(
            parse_command(r#"{"t":"sms_send","ref":"abc","to":"+1","body":"hi"}"#).unwrap(),
            Command::SmsSend { reference: "abc".into(), to: "+1".into(), body: "hi".into() }
        );
        assert_eq!(parse_command(r#"{"t":"sms_ack","id":3}"#).unwrap(), Command::SmsAck { id: 3 });
        assert_eq!(parse_command(r#"{"t":"wifi_get"}"#).unwrap(), Command::WifiGet);
        assert_eq!(
            parse_command(r#"{"t":"wifi_set","ssid":"Bridge","pass":"s3cret-pass"}"#).unwrap(),
            Command::WifiSet { ssid: "Bridge".into(), pass: "s3cret-pass".into() }
        );
        assert!(parse_command(r#"{"t":"reboot"}"#).is_err());
        assert!(parse_command("not json").is_err());
    }

    #[test]
    fn serializes_events() {
        let status = Event::Status(Status {
            sim: true,
            reg: true,
            data: false,
            op: "T-Mobile".into(),
            rat: "5G SA".into(),
            rssi: None,
            ip: String::new(),
            ssid: "5G-Bridge".into(),
            ..Default::default()
        });
        assert_eq!(
            status.to_line(),
            "{\"t\":\"status\",\"sim\":true,\"reg\":true,\"data\":false,\"op\":\"T-Mobile\",\"rat\":\"5G SA\",\"ip\":\"\",\"ssid\":\"5G-Bridge\"}\n"
        );

        let battery = Event::Status(Status { bat: Some(82), chg: Some(true), usb: Some(true), ..Default::default() });
        assert!(battery.to_line().ends_with(",\"bat\":82,\"chg\":true,\"usb\":true}\n"));

        let call = CallSnapshot { state: CallState::Incoming, number: "+1".into(), outgoing: false };
        assert_eq!(
            Event::call(&call).to_line(),
            "{\"t\":\"call\",\"state\":\"incoming\",\"num\":\"+1\",\"dir\":\"in\"}\n"
        );

        let wifi = Event::Wifi { ssid: "Bridge".into(), pass: "s3cret-pass".into() };
        assert_eq!(wifi.to_line(), "{\"t\":\"wifi\",\"ssid\":\"Bridge\",\"pass\":\"s3cret-pass\"}\n");

        let sent = Event::SmsSent { reference: "r1".into(), ok: true, err: None };
        assert_eq!(sent.to_line(), "{\"t\":\"sms_sent\",\"ref\":\"r1\",\"ok\":true}\n");
    }
}
