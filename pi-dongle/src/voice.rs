//! Call control and state machine (polls AT+CLCC while a call exists).
//! Same behaviour as firmware-rs/firmware/src/voice.rs.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bridge_core::call::{self, CallSnapshot, CallState};
use bridge_core::{at, protocol::Event};
use log::info;

use crate::at::AtPort;
use crate::audio::AudioCmd;
use crate::Emit;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const IDLE_WAIT: Duration = Duration::from_secs(3600);

pub enum VoiceCmd {
    Kick,
    Resend,
    Dial(String),
    Answer,
    Hangup,
    Dtmf(char),
}

pub fn spawn(at: Arc<AtPort>, audio: Sender<AudioCmd>, emit: Emit) -> anyhow::Result<Sender<VoiceCmd>> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new().name("voice".into()).spawn(move || run(rx, &at, &audio, &emit))?;
    Ok(tx)
}

fn run(rx: Receiver<VoiceCmd>, at: &AtPort, audio: &Sender<AudioCmd>, emit: &Emit) {
    let mut current = CallSnapshot::idle();
    loop {
        let wait = if current.state == CallState::Idle { IDLE_WAIT } else { POLL_INTERVAL };
        let resend = match rx.recv_timeout(wait) {
            Ok(cmd) => execute(cmd, at, emit),
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };

        if let Ok(resp) = at.at("AT+CLCC", Duration::from_secs(2)) {
            let next = call::current_call(&at::parse_clcc(&resp));
            if next != current {
                info!("call {:?} -> {:?}", current.state, next.state);
                match next.state {
                    CallState::Active => _ = audio.send(AudioCmd::Start),
                    CallState::Idle => _ = audio.send(AudioCmd::Stop),
                    _ => {}
                }
                current = next;
                emit(Event::call(&current));
                continue;
            }
        }
        if resend {
            emit(Event::call(&current));
        }
    }
}

fn execute(cmd: VoiceCmd, at: &AtPort, emit: &Emit) -> bool {
    let (ctx, result) = match cmd {
        VoiceCmd::Kick => return false,
        VoiceCmd::Resend => return true,
        VoiceCmd::Dial(number) => {
            if !call::valid_dial_string(&number) {
                emit(Event::error("dial", "invalid number"));
                return false;
            }
            ("dial", at.at(&format!("ATD{number};"), Duration::from_secs(15)))
        }
        VoiceCmd::Answer => ("answer", at.at("ATA", Duration::from_secs(10))),
        VoiceCmd::Hangup => {
            ("hangup", at.at("AT+CHUP", Duration::from_secs(5)).or_else(|_| at.at("ATH", Duration::from_secs(5))))
        }
        VoiceCmd::Dtmf(digit) => {
            if !call::valid_dtmf(digit) {
                emit(Event::error("dtmf", "invalid digit"));
                return false;
            }
            ("dtmf", at.at(&format!("AT+VTS={digit}"), Duration::from_secs(3)))
        }
    };
    if let Err(e) = result {
        emit(Event::error(ctx, e.to_string()));
    }
    false
}
