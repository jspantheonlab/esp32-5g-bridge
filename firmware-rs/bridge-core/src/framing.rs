//! Splits a byte stream into text lines (BLE JSON lines, modem URC lines).

pub struct LineAssembler {
    buf: Vec<u8>,
    max_len: usize,
    split_on_cr: bool,
    overflowed: bool,
}

impl LineAssembler {
    /// `split_on_cr`: treat `\r` as a line break too (modem output), otherwise only `\n` (JSON lines).
    pub const fn new(max_len: usize, split_on_cr: bool) -> Self {
        Self { buf: Vec::new(), max_len, split_on_cr, overflowed: false }
    }

    /// Feeds bytes and returns every completed, non-empty line. Lines longer than `max_len` are dropped.
    pub fn feed(&mut self, data: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &b in data {
            if b == b'\n' || (self.split_on_cr && b == b'\r') {
                if !self.buf.is_empty() && !self.overflowed {
                    lines.push(String::from_utf8_lossy(&self.buf).into_owned());
                }
                self.buf.clear();
                self.overflowed = false;
            } else if self.buf.len() < self.max_len {
                self.buf.push(b);
            } else {
                self.overflowed = true;
            }
        }
        lines
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.overflowed = false;
    }
}

/// Extracts only the not-yet-seen bytes from esp_modem's URC callback.
///
/// esp_modem passes the URC handler its whole unconsumed receive buffer
/// (`urc_handler(data, consumed + len)`), so in plain UART mode the same bytes are
/// re-delivered with every new chunk until a command completes and the buffer resets.
/// In CMUX mode only fresh chunks arrive. A call that starts with exactly the previously
/// delivered bytes is treated as a continuation (tracked by length + FNV-1a hash).
/// An identical repeat of the previous chunk is therefore dropped, which only affects
/// back-to-back identical URCs such as repeated `RING`.
#[derive(Default)]
pub struct BufferDelta {
    last_len: usize,
    last_hash: u64,
}

fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

impl BufferDelta {
    pub const fn new() -> Self {
        Self { last_len: 0, last_hash: 0 }
    }

    pub fn new_bytes<'a>(&mut self, data: &'a [u8]) -> &'a [u8] {
        let continues =
            self.last_len > 0 && data.len() >= self.last_len && fnv1a(&data[..self.last_len]) == self.last_hash;
        let start = if continues { self.last_len } else { 0 };
        self.last_len = data.len();
        self.last_hash = fnv1a(data);
        &data[start..]
    }
}

/// Turns esp_modem URC callbacks into complete modem lines.
pub struct UrcReader {
    delta: BufferDelta,
    lines: LineAssembler,
}

impl UrcReader {
    pub const fn new(max_line: usize) -> Self {
        Self { delta: BufferDelta::new(), lines: LineAssembler::new(max_line, true) }
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<String> {
        let fresh = self.delta.new_bytes(data);
        self.lines.feed(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_skips_redelivered_prefix() {
        let mut d = BufferDelta::new();
        assert_eq!(d.new_bytes(b"\r\n+CMT"), b"\r\n+CMT");
        assert_eq!(d.new_bytes(b"\r\n+CMTI: \"SM\",3\r\n"), b"I: \"SM\",3\r\n");
        // Buffer reset after a command completed: fresh content.
        assert_eq!(d.new_bytes(b"\r\nRING\r\n"), b"\r\nRING\r\n");
        // CMUX-style independent chunks.
        assert_eq!(d.new_bytes(b"+CLIP: \"1\""), b"+CLIP: \"1\"");
        assert_eq!(d.new_bytes(b",129\r\n"), b",129\r\n");
    }

    #[test]
    fn urc_reader_produces_each_line_once() {
        let mut r = UrcReader::new(256);
        let mut lines = r.feed(b"\r\nRI");
        lines.extend(r.feed(b"\r\nRING\r\n\r\n+CLIP: \"+1555\",145"));
        lines.extend(r.feed(b"\r\nRING\r\n\r\n+CLIP: \"+1555\",145\r\n"));
        assert_eq!(lines, vec!["RING", "+CLIP: \"+1555\",145"]);
    }

    #[test]
    fn reassembles_fragmented_lines() {
        let mut a = LineAssembler::new(64, false);
        assert!(a.feed(b"{\"t\":\"sta").is_empty());
        assert_eq!(a.feed(b"tus\"}\n{\"t\":\"answer\"}\n"), vec![r#"{"t":"status"}"#, r#"{"t":"answer"}"#]);
    }

    #[test]
    fn splits_modem_lines_on_cr_and_lf() {
        let mut a = LineAssembler::new(64, true);
        assert_eq!(a.feed(b"\r\nRING\r\n\r\n+CLIP: \"123\",129\r\n"), vec!["RING", "+CLIP: \"123\",129"]);
    }

    #[test]
    fn drops_overlong_lines() {
        let mut a = LineAssembler::new(4, false);
        assert_eq!(a.feed(b"toolong\nok\n"), vec!["ok"]);
    }
}
