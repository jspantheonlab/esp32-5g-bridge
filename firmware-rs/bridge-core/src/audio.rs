//! Call-audio UDP wire format and jitter buffer.
//!
//! Packet: `"5G"`, seq u16 LE, timestamp u32 LE, then one 20 ms frame of mono i16 LE PCM.

use std::collections::VecDeque;

pub const HEADER_LEN: usize = 8;
pub const MAGIC: [u8; 2] = *b"5G";

/// Samples in one 20 ms frame.
pub const fn frame_samples(sample_rate: u32) -> usize {
    (sample_rate / 50) as usize
}

pub const fn frame_bytes(sample_rate: u32) -> usize {
    frame_samples(sample_rate) * 2
}

/// Writes a packet into `out` (cleared first). `pcm` is little-endian i16 samples as raw bytes.
pub fn encode_packet(seq: u16, timestamp: u32, pcm: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&timestamp.to_le_bytes());
    out.extend_from_slice(pcm);
}

#[derive(Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    pub seq: u16,
    pub timestamp: u32,
    pub pcm: &'a [u8],
}

pub fn decode_packet(buf: &[u8]) -> Option<Packet<'_>> {
    if buf.len() < HEADER_LEN || buf[..2] != MAGIC {
        return None;
    }
    Some(Packet {
        seq: u16::from_le_bytes([buf[2], buf[3]]),
        timestamp: u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
        pcm: &buf[HEADER_LEN..],
    })
}

/// Fixed-size frame FIFO that waits for `prefill` frames before playing and drops the
/// oldest frame on overflow, keeping latency bounded.
pub struct JitterBuffer {
    frames: VecDeque<Vec<u8>>,
    frame_bytes: usize,
    capacity: usize,
    prefill: usize,
    playing: bool,
}

impl JitterBuffer {
    pub fn new(frame_bytes: usize, capacity: usize, prefill: usize) -> Self {
        Self { frames: VecDeque::with_capacity(capacity), frame_bytes, capacity, prefill, playing: false }
    }

    /// Frames of the wrong size are ignored.
    pub fn push(&mut self, frame: &[u8]) {
        if frame.len() != self.frame_bytes {
            return;
        }
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(frame.to_vec());
    }

    /// Fills `out` with the next frame, or silence while prefilling / on underrun.
    /// Returns true if real audio was written.
    pub fn pop_into(&mut self, out: &mut [u8]) -> bool {
        if !self.playing && self.frames.len() >= self.prefill {
            self.playing = true;
        }
        let next = if self.playing { self.frames.pop_front() } else { None };
        match next {
            Some(frame) => {
                out.copy_from_slice(&frame);
                true
            }
            None => {
                self.playing = false;
                out.fill(0);
                false
            }
        }
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.playing = false;
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_round_trip() {
        let mut buf = Vec::new();
        encode_packet(0x0102, 0x0A0B0C0D, &[1, 2, 3, 4], &mut buf);
        assert_eq!(buf, [b'5', b'G', 0x02, 0x01, 0x0D, 0x0C, 0x0B, 0x0A, 1, 2, 3, 4]);
        assert_eq!(decode_packet(&buf), Some(Packet { seq: 0x0102, timestamp: 0x0A0B0C0D, pcm: &[1, 2, 3, 4] }));
        assert_eq!(decode_packet(b"XX123456"), None);
        assert_eq!(decode_packet(b"5G"), None);
    }

    #[test]
    fn frame_sizes() {
        assert_eq!(frame_samples(8000), 160);
        assert_eq!(frame_bytes(16000), 640);
    }

    #[test]
    fn jitter_prefill_underrun_and_overflow() {
        let mut jb = JitterBuffer::new(2, 3, 2);
        let mut out = [9u8; 2];

        jb.push(&[1, 1]);
        assert!(!jb.pop_into(&mut out), "still prefilling");
        assert_eq!(out, [0, 0]);

        jb.push(&[2, 2]);
        assert!(jb.pop_into(&mut out));
        assert_eq!(out, [1, 1]);
        assert!(jb.pop_into(&mut out));
        assert_eq!(out, [2, 2]);
        assert!(!jb.pop_into(&mut out), "underrun");

        for i in 3..=6 {
            jb.push(&[i, i]);
        }
        assert_eq!(jb.len(), 3, "oldest dropped");
        assert!(jb.pop_into(&mut out));
        assert_eq!(out, [4, 4]);

        jb.push(&[1, 2, 3]);
        assert_eq!(jb.len(), 2, "wrong-size frame ignored");
    }
}
