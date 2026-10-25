//! Call audio: modem PCM (I2S, 16-bit mono) ⇄ UDP to the iPhone, paced by the PCM clock.
//! The phone's address is learned from the first packet it sends to `AUDIO_UDP_PORT`.

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use bridge_core::audio::{self, JitterBuffer};
use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::{InputPin, OutputPin};
use esp_idf_svc::hal::i2s::config::{
    Config as ChannelConfig, DataBitWidth, Role, SlotMode, StdClkConfig, StdConfig, StdGpioConfig, StdSlotConfig,
};
use esp_idf_svc::hal::i2s::{I2s, I2sBiDir, I2sDriver};
use log::{info, warn};

use crate::config;

const FRAME_BYTES: usize = audio::frame_bytes(config::PCM_SAMPLE_RATE);
const FRAME_SAMPLES: usize = audio::frame_samples(config::PCM_SAMPLE_RATE);
const JITTER_FRAMES: usize = 8;
const JITTER_PREFILL: usize = 3;

pub enum AudioCmd {
    Start,
    Stop,
}

/// PCM short-frame sync, 16-bit, modem audio in the first slot.
pub fn new_i2s(
    i2s: impl I2s + 'static,
    bclk: impl InputPin + OutputPin + 'static,
    din: impl InputPin + 'static,
    dout: impl OutputPin + 'static,
    ws: impl InputPin + OutputPin + 'static,
) -> anyhow::Result<I2sDriver<'static, I2sBiDir>> {
    let role = if config::I2S_CONTROLLER { Role::Controller } else { Role::Target };
    let std_config = StdConfig::new(
        ChannelConfig::default().role(role).frames_per_buffer(FRAME_SAMPLES as u32),
        StdClkConfig::from_sample_rate_hz(config::PCM_SAMPLE_RATE),
        StdSlotConfig::pcm_slot_default(DataBitWidth::Bits16, SlotMode::Mono),
        StdGpioConfig::default(),
    );
    let mclk: Option<esp_idf_svc::hal::gpio::AnyIOPin<'static>> = None;
    Ok(I2sDriver::new_std_bidir(i2s, &std_config, bclk, din, dout, mclk, ws)?)
}

pub fn spawn(i2s: I2sDriver<'static, I2sBiDir>) -> anyhow::Result<Sender<AudioCmd>> {
    let socket = UdpSocket::bind(("0.0.0.0", config::AUDIO_UDP_PORT))?;
    socket.set_nonblocking(true)?;
    let (tx, rx) = mpsc::channel();
    thread::Builder::new().name("audio".into()).stack_size(8192).spawn(move || run(i2s, socket, rx))?;
    Ok(tx)
}

fn run(mut i2s: I2sDriver<'static, I2sBiDir>, socket: UdpSocket, rx: Receiver<AudioCmd>) {
    let timeout = TickType::from(Duration::from_millis(100)).into();
    let mut jitter = JitterBuffer::new(FRAME_BYTES, JITTER_FRAMES, JITTER_PREFILL);
    let mut mic = [0u8; FRAME_BYTES];
    let mut speaker = [0u8; FRAME_BYTES];
    let mut packet = Vec::with_capacity(audio::HEADER_LEN + FRAME_BYTES);
    let mut recv_buf = [0u8; audio::HEADER_LEN + FRAME_BYTES];
    let (mut seq, mut ts) = (0u16, 0u32);

    loop {
        // Idle until a call becomes active.
        match rx.recv() {
            Ok(AudioCmd::Start) => {}
            Ok(AudioCmd::Stop) => continue,
            Err(_) => return,
        }
        info!("Call audio started");
        jitter.clear();
        let mut peer: Option<SocketAddr> = None;
        drain(&socket, &mut recv_buf, &mut jitter, &mut peer);
        jitter.clear();
        if let Err(e) = i2s.rx_enable().and_then(|_| i2s.tx_enable()) {
            warn!("I2S enable failed: {e}");
        }

        loop {
            match rx.try_recv() {
                Ok(AudioCmd::Stop) => break,
                Ok(AudioCmd::Start) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return,
            }

            // The I2S read blocks for one 20 ms frame, pacing the loop at the modem's PCM clock.
            let got = i2s.read(&mut mic, timeout).unwrap_or(0);
            drain(&socket, &mut recv_buf, &mut jitter, &mut peer);
            if got != FRAME_BYTES {
                continue;
            }

            if let Some(addr) = peer {
                audio::encode_packet(seq, ts, &mic, &mut packet);
                let _ = socket.send_to(&packet, addr);
            }
            seq = seq.wrapping_add(1);
            ts = ts.wrapping_add(FRAME_SAMPLES as u32);

            jitter.pop_into(&mut speaker);
            let _ = i2s.write(&speaker, timeout);
        }

        let _ = i2s.rx_disable();
        let _ = i2s.tx_disable();
        info!("Call audio stopped");
    }
}

fn drain(socket: &UdpSocket, buf: &mut [u8], jitter: &mut JitterBuffer, peer: &mut Option<SocketAddr>) {
    loop {
        match socket.recv_from(buf) {
            Ok((n, from)) => {
                if let Some(pkt) = audio::decode_packet(&buf[..n]) {
                    *peer = Some(from);
                    jitter.push(pkt.pcm);
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => return,
            Err(_) => return,
        }
    }
}
