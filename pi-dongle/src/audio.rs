//! Call audio: the modem's USB audio (ALSA, via arecord/aplay) ⇄ UDP to the iPhone.
//! Same packet format and jitter buffer as the ESP32 firmware (bridge-core::audio).

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use bridge_core::audio::{self, JitterBuffer};
use log::{info, warn};

pub const SAMPLE_RATE: u32 = 8000;
const FRAME_BYTES: usize = audio::frame_bytes(SAMPLE_RATE);
const FRAME_SAMPLES: usize = audio::frame_samples(SAMPLE_RATE);

pub enum AudioCmd {
    Start,
    Stop,
}

pub fn spawn(device: Option<String>, udp_port: u16) -> anyhow::Result<Sender<AudioCmd>> {
    let socket = UdpSocket::bind(("0.0.0.0", udp_port))?;
    socket.set_nonblocking(true)?;
    let (tx, rx) = mpsc::channel();
    thread::Builder::new().name("audio".into()).spawn(move || run(device, socket, rx))?;
    Ok(tx)
}

struct AlsaStreams {
    capture: Child,
    playback: Child,
    mic: ChildStdout,
    speaker: ChildStdin,
}

impl AlsaStreams {
    fn open(device: &str) -> std::io::Result<Self> {
        let fmt = ["-q", "-t", "raw", "-f", "S16_LE", "-c", "1", "-r", "8000", "-D", device];
        let mut capture = Command::new("arecord").args(fmt).stdout(Stdio::piped()).spawn()?;
        let mut playback = Command::new("aplay").args(fmt).stdin(Stdio::piped()).spawn()?;
        let mic = capture.stdout.take().expect("piped stdout");
        let speaker = playback.stdin.take().expect("piped stdin");
        Ok(Self { capture, playback, mic, speaker })
    }
}

impl Drop for AlsaStreams {
    fn drop(&mut self) {
        let _ = self.capture.kill();
        let _ = self.playback.kill();
        let _ = self.capture.wait();
        let _ = self.playback.wait();
    }
}

fn run(device: Option<String>, socket: UdpSocket, rx: Receiver<AudioCmd>) {
    let mut jitter = JitterBuffer::new(FRAME_BYTES, 8, 3);
    let mut mic = [0u8; FRAME_BYTES];
    let mut speaker = [0u8; FRAME_BYTES];
    let mut packet = Vec::with_capacity(audio::HEADER_LEN + FRAME_BYTES);
    let mut recv_buf = [0u8; audio::HEADER_LEN + FRAME_BYTES];
    let (mut seq, mut ts) = (0u16, 0u32);

    loop {
        match rx.recv() {
            Ok(AudioCmd::Start) => {}
            Ok(AudioCmd::Stop) => continue,
            Err(_) => return,
        }
        let Some(device) = device.as_deref() else {
            warn!("Call is active but no modem audio device was found (set BRIDGE_AUDIO_DEVICE)");
            continue;
        };
        let mut streams = match AlsaStreams::open(device) {
            Ok(s) => s,
            Err(e) => {
                warn!("Could not start arecord/aplay on {device}: {e}");
                continue;
            }
        };
        info!("Call audio started on {device}");
        jitter.clear();
        let mut peer: Option<SocketAddr> = None;

        loop {
            match rx.try_recv() {
                Ok(AudioCmd::Stop) => break,
                Ok(AudioCmd::Start) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return,
            }
            // Reading one 20 ms frame from the modem paces the loop.
            if let Err(e) = streams.mic.read_exact(&mut mic) {
                warn!("Modem audio capture stopped: {e}");
                break;
            }
            drain(&socket, &mut recv_buf, &mut jitter, &mut peer);
            if let Some(addr) = peer {
                audio::encode_packet(seq, ts, &mic, &mut packet);
                let _ = socket.send_to(&packet, addr);
            }
            seq = seq.wrapping_add(1);
            ts = ts.wrapping_add(FRAME_SAMPLES as u32);
            jitter.pop_into(&mut speaker);
            if streams.speaker.write_all(&speaker).is_err() {
                warn!("Modem audio playback stopped");
                break;
            }
        }
        drop(streams);
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
