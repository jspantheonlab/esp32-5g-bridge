//! ESP32-S3 5G bridge firmware (Rust, std on ESP-IDF).
//!
//! Threads: modem supervisor, URC dispatcher, voice, sms, audio, plus this main thread which
//! handles commands from the iPhone. Protocol and parsing logic lives in `bridge-core`.

mod audio;
mod ble;
mod config;
mod hotspot;
mod modem;
mod power;
mod sms;
mod voice;

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use bridge_core::protocol::{self, Command, Event, Status};
use bridge_core::{at, call};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::units::FromValueType;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use log::{info, warn};

use crate::ble::{BleLink, LinkEvent};
use crate::modem::{Modem, Supervisor};
use crate::sms::SmsJob;
use crate::voice::VoiceCmd;

/// Sends an event to the phone over BLE.
pub type Emit = Arc<dyn Fn(Event) + Send + Sync>;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    // Keep the Wi-Fi driver alive for the lifetime of the program.
    let (_wifi, router) = hotspot::start(peripherals.modem, sysloop, nvs)?;

    // Board wiring: PCM_CLK=GPIO4, PCM_SYNC=GPIO5, DIN=GPIO6 (modem PCM_DOUT), DOUT=GPIO7 (modem PCM_DIN).
    let i2s = audio::new_i2s(peripherals.i2s0, pins.gpio4, pins.gpio6, pins.gpio7, pins.gpio5)?;
    let audio_tx = audio::spawn(i2s)?;

    let (link_tx, link_rx) = mpsc::channel();
    let link = BleLink::start(link_tx)?;
    let emit: Emit = {
        let link = link.clone();
        Arc::new(move |event: Event| link.send(&event.to_line()))
    };

    let status = Arc::new(Mutex::new(Status { ssid: config::AP_SSID.into(), rat: "—".into(), ..Default::default() }));
    let (urc_tx, urc_rx) = mpsc::sync_channel::<String>(32);
    let modem = Modem::init(urc_tx)?;
    let voice_tx = voice::spawn(modem.clone(), audio_tx, emit.clone())?;
    let sms_tx = sms::spawn(modem.clone(), emit.clone())?;

    {
        let voice_tx = voice_tx.clone();
        let sms_tx = sms_tx.clone();
        thread::Builder::new().name("urc".into()).stack_size(4096).spawn(move || {
            for line in urc_rx {
                if call::is_call_urc(&line) {
                    let _ = voice_tx.send(VoiceCmd::Kick);
                }
                if let Some(index) = at::parse_cmti(&line) {
                    let _ = sms_tx.try_send(SmsJob::Read(index));
                }
            }
        })?;
    }

    // PWRKEY on GPIO21 (through an NPN; see docs/board-design.md).
    let pwrkey = PinDriver::output(pins.gpio21)?;
    modem::spawn_supervisor(Supervisor {
        modem: modem.clone(),
        pwrkey,
        status: status.clone(),
        router,
        emit: emit.clone(),
    })?;

    // Fuel gauge + charger on I2C SDA=GPIO8, SCL=GPIO9 (custom board; absent on a DevKit).
    let i2c = I2cDriver::new(peripherals.i2c0, pins.gpio8, pins.gpio9, &I2cConfig::new().baudrate(100.kHz().into()))?;
    power::spawn(power::PowerMonitor { i2c, status: status.clone(), modem, emit: emit.clone() })?;

    info!("5G bridge started");
    handle_link_events(link_rx, &status, &voice_tx, &sms_tx, &emit);
    Ok(())
}

fn handle_link_events(
    events: Receiver<LinkEvent>,
    status: &Mutex<Status>,
    voice: &mpsc::Sender<VoiceCmd>,
    sms: &SyncSender<SmsJob>,
    emit: &Emit,
) {
    let current_status = || status.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for event in events {
        match event {
            LinkEvent::Ready => {
                emit(Event::Status(current_status()));
                let _ = voice.send(VoiceCmd::Resend);
                let _ = sms.try_send(SmsJob::Sync);
            }
            LinkEvent::Line(line) => match protocol::parse_command(&line) {
                Ok(Command::Status) => emit(Event::Status(current_status())),
                Ok(Command::Dial { num }) => _ = voice.send(VoiceCmd::Dial(num)),
                Ok(Command::Answer) => _ = voice.send(VoiceCmd::Answer),
                Ok(Command::Hangup) => _ = voice.send(VoiceCmd::Hangup),
                Ok(Command::Dtmf { d }) => match d.chars().next() {
                    Some(digit) => _ = voice.send(VoiceCmd::Dtmf(digit)),
                    None => emit(Event::error("dtmf", "missing digit")),
                },
                Ok(Command::SmsSend { reference, to, body }) => {
                    if sms.try_send(SmsJob::Send { reference: reference.clone(), to, body }).is_err() {
                        emit(Event::SmsSent { reference, ok: false, err: Some("bridge busy".into()) });
                    }
                }
                Ok(Command::SmsAck { id }) => _ = sms.try_send(SmsJob::Delete(id)),
                Ok(Command::SmsSync) => _ = sms.try_send(SmsJob::Sync),
                Err(e) => warn!("Bad command {line:?}: {e}"),
            },
        }
    }
}
