//! Call control and the call state machine. The voice thread owns the call state:
//! it executes call commands and polls `AT+CLCC` (every 500 ms while a call exists).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bridge_core::call::{self, CallSnapshot, CallState};
use bridge_core::{at, protocol::Event};
use log::info;

use crate::audio::AudioCmd;
use crate::modem::Modem;
use crate::Emit;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const IDLE_WAIT: Duration = Duration::from_secs(3600);

pub enum VoiceCmd {
    /// A URC suggests the call state changed.
    Kick,
    /// Re-send the current state (phone reconnected).
    Resend,
    Dial(String),
    Answer,
    Hangup,
    Dtmf(char),
}

pub fn spawn(modem: Arc<Modem>, audio: Sender<AudioCmd>, emit: Emit) -> anyhow::Result<Sender<VoiceCmd>> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new().name("voice".into()).stack_size(6144).spawn(move || run(rx, &modem, &audio, &emit))?;
    Ok(tx)
}

fn run(rx: Receiver<VoiceCmd>, modem: &Modem, audio: &Sender<AudioCmd>, emit: &Emit) {
    let mut current = CallSnapshot::idle();
    loop {
        let wait = if current.state == CallState::Idle { IDLE_WAIT } else { POLL_INTERVAL };
        let resend = match rx.recv_timeout(wait) {
            Ok(cmd) => execute(cmd, modem, emit),
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };

        if let Ok(resp) = modem.at("AT+CLCC", 2000) {
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

/// Runs a command; returns true if the current state should be re-sent.
fn execute(cmd: VoiceCmd, modem: &Modem, emit: &Emit) -> bool {
    let (ctx, result) = match cmd {
        VoiceCmd::Kick => return false,
        VoiceCmd::Resend => return true,
        VoiceCmd::Dial(number) => {
            if !call::valid_dial_string(&number) {
                emit(Event::error("dial", "invalid number"));
                return false;
            }
            ("dial", modem.at(&format!("ATD{number};"), 15_000))
        }
        VoiceCmd::Answer => ("answer", modem.at("ATA", 10_000)),
        VoiceCmd::Hangup => ("hangup", modem.at("AT+CHUP", 5000).or_else(|_| modem.at("ATH", 5000))),
        VoiceCmd::Dtmf(digit) => {
            if !call::valid_dtmf(digit) {
                emit(Event::error("dtmf", "invalid digit"));
                return false;
            }
            ("dtmf", modem.at(&format!("AT+VTS={digit}"), 3000))
        }
    };
    if let Err(e) = result {
        emit(Event::error(ctx, e.to_string()));
    }
    false
}
