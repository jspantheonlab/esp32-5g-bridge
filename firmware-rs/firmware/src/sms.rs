//! SMS worker: read new messages, re-sync the SIM, delete acknowledged messages, send.
//! Sending can block for up to a minute, so it runs on its own thread.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;

use bridge_core::protocol::Event;
use bridge_core::{at, sms};
use log::{info, warn};

use crate::modem::Modem;
use crate::Emit;

pub enum SmsJob {
    Read(u32),
    Sync,
    Delete(u32),
    Send { reference: String, to: String, body: String },
}

pub fn spawn(modem: Arc<Modem>, emit: Emit) -> anyhow::Result<SyncSender<SmsJob>> {
    let (tx, rx) = mpsc::sync_channel(16);
    thread::Builder::new().name("sms".into()).stack_size(8192).spawn(move || run(rx, &modem, &emit))?;
    Ok(tx)
}

fn run(rx: Receiver<SmsJob>, modem: &Modem, emit: &Emit) {
    for job in rx {
        match job {
            SmsJob::Read(index) => {
                read_index(modem, index, emit);
            }
            SmsJob::Sync => sync_all(modem, emit),
            SmsJob::Delete(index) => {
                let _ = modem.at(&format!("AT+CMGD={index}"), 5000);
            }
            SmsJob::Send { reference, to, body } => send(modem, emit, reference, &to, &body),
        }
    }
}

/// Reads one stored message and forwards it to the phone. Returns true if it was an inbound SMS.
fn read_index(modem: &Modem, index: u32, emit: &Emit) -> bool {
    let resp = {
        let session = modem.session();
        let _ = session.at(sms::UCS2_CHARSET, 1000);
        let resp = session.at(&format!("AT+CMGR={index}"), 5000);
        let _ = session.at(sms::RESTORE_CHARSET, 1000);
        resp
    };
    let Some(msg) = resp.ok().as_deref().and_then(at::parse_cmgr).as_ref().and_then(sms::decode_received) else {
        return false;
    };
    emit(Event::Sms { id: index, from: msg.from, ts: msg.timestamp, body: msg.body });
    true
}

fn sync_all(modem: &Modem, emit: &Emit) {
    let Some((used, total)) = modem.at("AT+CPMS?", 2000).ok().as_deref().and_then(at::parse_cpms) else {
        warn!("Could not read SMS storage");
        return;
    };
    info!("SIM storage: {used}/{total}");
    let mut found = 0;
    for index in 0..=total {
        if found >= used {
            break;
        }
        if read_index(modem, index, emit) {
            found += 1;
        }
    }
}

fn send(modem: &Modem, emit: &Emit, reference: String, to: &str, body: &str) {
    let result = match sms::plan_send(to, body) {
        Err(e) => Err(e.to_string()),
        Ok(plan) => {
            let session = modem.session();
            let _ = session.at(plan.charset_cmd, 1000);
            let _ = session.at(plan.csmp_cmd, 1000);
            let sent = match session.raw(&plan.cmgs_cmd, ">", "ERROR", 5000) {
                Ok(_) => session.raw(&plan.payload, "+CMGS:", "ERROR", 60_000).map(|_| ()),
                Err(e) => {
                    let _ = session.raw("\x1B", "OK", "ERROR", 1000);
                    Err(e)
                }
            };
            let _ = session.at(sms::RESTORE_CHARSET, 1000);
            let _ = session.at(sms::RESTORE_CSMP, 1000);
            sent.map_err(|_| "modem rejected message".to_string())
        }
    };
    info!("SMS to {to}: {}", if result.is_ok() { "sent" } else { "failed" });
    emit(Event::SmsSent { reference, ok: result.is_ok(), err: result.err() });
}
