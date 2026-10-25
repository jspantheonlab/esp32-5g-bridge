//! 5G USB-C dongle daemon for Raspberry Pi.
//!
//! The Pi appears to the iPhone as a USB Ethernet adapter (see system/usb-gadget.sh) and
//! shares the modem's 5G connection. This daemon adds calls and SMS: it drives the modem over
//! its AT port and serves the Bridge5G app over TCP on the USB link, using the same protocol
//! and audio format as the ESP32 firmware (bridge-core).

mod at;
mod audio;
mod config;
mod modem;
mod server;
mod sms;
mod voice;

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Context;
use bridge_core::protocol::{self, Command, Event, Status};
use bridge_core::{at as at_parse, call};
use log::{info, warn};
use nix::sys::termios::{self, SetArg};

use crate::at::AtPort;
use crate::config::Config;
use crate::server::{Clients, LinkEvent};
use crate::sms::SmsJob;
use crate::voice::VoiceCmd;

/// Sends an event to every connected phone.
pub type Emit = Arc<dyn Fn(Event) + Send + Sync>;

/// Opens the modem's USB serial port in raw mode (no echo or line editing by the tty layer).
fn open_serial(path: &Path) -> anyhow::Result<std::fs::File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_NOCTTY)
        .open(path)
        .with_context(|| format!("opening modem AT port {}", path.display()))?;
    let mut tio = termios::tcgetattr(&file)?;
    termios::cfmakeraw(&mut tio);
    termios::tcsetattr(&file, SetArg::TCSANOW, &tio)?;
    Ok(file)
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = Config::from_env();
    info!("AT port {}, listening on {}", config.at_port.display(), config.listen);
    match &config.audio_device {
        Some(dev) => info!("Call audio device {dev}"),
        None => warn!("No modem USB audio device found; calls will have no audio (see docs/pi-dongle.md)"),
    }

    let serial = open_serial(&config.at_port)?;
    let (urc_tx, urc_rx) = at::urc_channel();
    let at = AtPort::new(serial.try_clone()?, serial, urc_tx);

    let clients = Clients::default();
    let emit: Emit = {
        let clients = clients.clone();
        Arc::new(move |event: Event| clients.broadcast(&event.to_line()))
    };

    let status = Arc::new(Mutex::new(Status { rat: "—".into(), ..Default::default() }));
    let audio_tx = audio::spawn(config.audio_device.clone(), config.audio_port)?;
    let voice_tx = voice::spawn(at.clone(), audio_tx, emit.clone())?;
    let sms_tx = sms::spawn(at.clone(), emit.clone())?;
    spawn_urc_dispatcher(urc_rx, voice_tx.clone(), sms_tx.clone())?;
    modem::spawn(modem::Supervisor::new(at, status.clone(), emit.clone(), &config))?;

    let (link_tx, link_rx) = mpsc::channel();
    server::spawn(config.listen.clone(), clients, link_tx)?;
    App { status, voice: voice_tx, sms: sms_tx, emit }.run(link_rx);
    Ok(())
}

fn spawn_urc_dispatcher(
    urcs: Receiver<String>,
    voice: mpsc::Sender<VoiceCmd>,
    sms: SyncSender<SmsJob>,
) -> anyhow::Result<()> {
    thread::Builder::new().name("urc".into()).spawn(move || {
        for line in urcs {
            log::debug!("URC: {line}");
            if call::is_call_urc(&line) {
                let _ = voice.send(VoiceCmd::Kick);
            }
            if let Some(index) = at_parse::parse_cmti(&line) {
                let _ = sms.try_send(SmsJob::Read(index));
            }
        }
    })?;
    Ok(())
}

/// Handles commands from the phone.
struct App {
    status: Arc<Mutex<Status>>,
    voice: mpsc::Sender<VoiceCmd>,
    sms: SyncSender<SmsJob>,
    emit: Emit,
}

impl App {
    fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn run(self, events: Receiver<LinkEvent>) {
        for event in events {
            match event {
                LinkEvent::Ready => {
                    (self.emit)(Event::Status(self.status()));
                    let _ = self.voice.send(VoiceCmd::Resend);
                    let _ = self.sms.try_send(SmsJob::Sync);
                }
                LinkEvent::Line(line) => match protocol::parse_command(&line) {
                    Ok(command) => self.handle(command),
                    Err(e) => warn!("Bad command {line:?}: {e}"),
                },
            }
        }
    }

    fn handle(&self, command: Command) {
        let emit = &self.emit;
        match command {
            Command::Status => emit(Event::Status(self.status())),
            Command::Dial { num } => _ = self.voice.send(VoiceCmd::Dial(num)),
            Command::Answer => _ = self.voice.send(VoiceCmd::Answer),
            Command::Hangup => _ = self.voice.send(VoiceCmd::Hangup),
            Command::Dtmf { d } => match d.chars().next() {
                Some(digit) => _ = self.voice.send(VoiceCmd::Dtmf(digit)),
                None => emit(Event::error("dtmf", "missing digit")),
            },
            Command::SmsSend { reference, to, body } => {
                if self.sms.try_send(SmsJob::Send { reference: reference.clone(), to, body }).is_err() {
                    emit(Event::SmsSent { reference, ok: false, err: Some("bridge busy".into()) });
                }
            }
            Command::SmsAck { id } => _ = self.sms.try_send(SmsJob::Delete(id)),
            Command::SmsSync => _ = self.sms.try_send(SmsJob::Sync),
            Command::WifiGet | Command::WifiSet { .. } => {
                emit(Event::error("wifi_set", "the USB dongle has no Wi-Fi hotspot"));
            }
        }
    }
}
