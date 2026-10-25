//! SMS worker: read new messages, re-sync the SIM, delete acknowledged messages, send.
//! Same behaviour as firmware-rs/firmware/src/sms.rs.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bridge_core::protocol::Event;
use bridge_core::{at, sms};
use log::{info, warn};

use crate::at::AtPort;
use crate::Emit;

const SHORT: Duration = Duration::from_secs(2);

pub enum SmsJob {
    Read(u32),
    Sync,
    Delete(u32),
    Send { reference: String, to: String, body: String },
}

pub fn spawn(at: Arc<AtPort>, emit: Emit) -> anyhow::Result<SyncSender<SmsJob>> {
    let (tx, rx) = mpsc::sync_channel(16);
    thread::Builder::new().name("sms".into()).spawn(move || run(rx, &at, &emit))?;
    Ok(tx)
}

fn run(rx: Receiver<SmsJob>, at: &AtPort, emit: &Emit) {
    for job in rx {
        match job {
            SmsJob::Read(index) => {
                read_index(at, index, emit);
            }
            SmsJob::Sync => sync_all(at, emit),
            SmsJob::Delete(index) => {
                let _ = at.at(&format!("AT+CMGD={index}"), Duration::from_secs(5));
            }
            SmsJob::Send { reference, to, body } => send(at, emit, reference, &to, &body),
        }
    }
}

fn read_index(at: &AtPort, index: u32, emit: &Emit) -> bool {
    let resp = {
        let session = at.session();
        let _ = session.at(sms::UCS2_CHARSET, SHORT);
        let resp = session.at(&format!("AT+CMGR={index}"), Duration::from_secs(5));
        let _ = session.at(sms::RESTORE_CHARSET, SHORT);
        resp
    };
    let Some(msg) = resp.ok().as_deref().and_then(at::parse_cmgr).as_ref().and_then(sms::decode_received) else {
        return false;
    };
    emit(Event::Sms { id: index, from: msg.from, ts: msg.timestamp, body: msg.body });
    true
}

fn sync_all(at: &AtPort, emit: &Emit) {
    let Some((used, total)) = at.at("AT+CPMS?", SHORT).ok().as_deref().and_then(at::parse_cpms) else {
        warn!("Could not read SMS storage");
        return;
    };
    info!("SIM storage: {used}/{total}");
    let mut found = 0;
    for index in 0..=total {
        if found >= used {
            break;
        }
        if read_index(at, index, emit) {
            found += 1;
        }
    }
}

fn send(at: &AtPort, emit: &Emit, reference: String, to: &str, body: &str) {
    let result = match sms::plan_send(to, body) {
        Err(e) => Err(e.to_string()),
        Ok(plan) => {
            let session = at.session();
            let _ = session.at(plan.charset_cmd, SHORT);
            let _ = session.at(plan.csmp_cmd, SHORT);
            // plan.cmgs_cmd already ends in \r; strip it because at_prompt adds one.
            let sent = match session.at_prompt(plan.cmgs_cmd.trim_end_matches('\r'), Duration::from_secs(5)) {
                Ok(_) => session.raw(&plan.payload, Duration::from_secs(60)).map(|_| ()),
                Err(e) => {
                    let _ = session.raw("\x1B", SHORT);
                    Err(e)
                }
            };
            let _ = session.at(sms::RESTORE_CHARSET, SHORT);
            let _ = session.at(sms::RESTORE_CSMP, SHORT);
            sent.map_err(|_| "modem rejected message".to_string())
        }
    };
    info!("SMS to {to}: {}", if result.is_ok() { "sent" } else { "failed" });
    emit(Event::SmsSent { reference, ok: result.is_ok(), err: result.err() });
}
