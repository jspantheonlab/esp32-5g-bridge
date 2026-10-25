//! AT command channel over the modem's USB serial port.
//!
//! One reader thread splits incoming bytes into lines. While a command is in flight its
//! response lines are collected until a final result (`OK`, `ERROR`, `+CME/+CMS ERROR`, or the
//! `>` SMS prompt when asked for); unsolicited result codes (URCs) always go to a channel.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use bridge_core::framing::LineAssembler;

/// Lines that are always unsolicited, even in the middle of another command's response.
const URC_PREFIXES: &[&str] = &[
    "RING",
    "+CRING:",
    "+CLIP:",
    "+CMTI:",
    "+CMT:",
    "^DSCI:",
    "+QIND:",
    "+CIREGU:",
    "NO CARRIER",
    "BUSY",
    "NO ANSWER",
];

#[derive(Debug)]
pub enum AtError {
    Timeout,
    /// The modem answered with an error; the full response is included.
    Rejected(String),
    Io(std::io::Error),
    Closed,
}

impl std::fmt::Display for AtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "modem did not answer in time"),
            Self::Rejected(resp) => write!(f, "modem error: {}", resp.trim()),
            Self::Io(e) => write!(f, "serial port error: {e}"),
            Self::Closed => write!(f, "serial port closed"),
        }
    }
}

impl std::error::Error for AtError {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Wait for a final result code.
    Final,
    /// Wait for the `>` prompt (AT+CMGS), or an error.
    Prompt,
}

struct Pending {
    expect: Expect,
    lines: Vec<String>,
    done: Sender<Result<String, String>>,
}

fn is_urc(line: &str) -> bool {
    URC_PREFIXES.iter().any(|p| line.starts_with(p))
}

fn is_error(line: &str) -> bool {
    line == "ERROR" || line.starts_with("+CME ERROR") || line.starts_with("+CMS ERROR")
}

pub struct AtPort {
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Arc<Mutex<Option<Pending>>>,
    command_lock: Mutex<()>,
}

/// Holds the AT channel for a multi-command sequence (e.g. switching the SMS character set).
pub struct AtSession<'a> {
    port: &'a AtPort,
    _guard: MutexGuard<'a, ()>,
}

impl AtPort {
    /// Starts the reader thread. URC lines are delivered on `urc_tx`.
    pub fn new<R, W>(reader: R, writer: W, urc_tx: SyncSender<String>) -> Arc<Self>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let port = Arc::new(Self {
            writer: Mutex::new(Box::new(writer)),
            pending: Arc::new(Mutex::new(None)),
            command_lock: Mutex::new(()),
        });
        let pending = port.pending.clone();
        thread::Builder::new()
            .name("at-reader".into())
            .spawn(move || read_loop(reader, pending, urc_tx))
            .expect("spawn AT reader");
        port
    }

    pub fn session(&self) -> AtSession<'_> {
        AtSession { port: self, _guard: self.command_lock.lock().unwrap_or_else(|e| e.into_inner()) }
    }

    /// Sends `cmd\r` and waits for the final result. Returns all response lines.
    pub fn at(&self, cmd: &str, timeout: Duration) -> Result<String, AtError> {
        self.session().at(cmd, timeout)
    }

    fn exchange(&self, data: &str, expect: Expect, timeout: Duration) -> Result<String, AtError> {
        let (tx, rx) = mpsc::channel();
        *lock(&self.pending) = Some(Pending { expect, lines: Vec::new(), done: tx });
        let written = {
            let mut w = lock(&self.writer);
            w.write_all(data.as_bytes()).and_then(|_| w.flush())
        };
        if let Err(e) = written {
            lock(&self.pending).take();
            return Err(AtError::Io(e));
        }
        let result = rx.recv_timeout(timeout);
        lock(&self.pending).take();
        match result {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(resp)) => Err(AtError::Rejected(resp)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(AtError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(AtError::Closed),
        }
    }
}

impl AtSession<'_> {
    pub fn at(&self, cmd: &str, timeout: Duration) -> Result<String, AtError> {
        self.port.exchange(&format!("{cmd}\r"), Expect::Final, timeout)
    }

    /// Sends a command and waits for the `>` prompt (used by AT+CMGS).
    pub fn at_prompt(&self, cmd: &str, timeout: Duration) -> Result<String, AtError> {
        self.port.exchange(&format!("{cmd}\r"), Expect::Prompt, timeout)
    }

    /// Sends raw data (e.g. an SMS body ending in Ctrl-Z) and waits for the final result.
    pub fn raw(&self, data: &str, timeout: Duration) -> Result<String, AtError> {
        self.port.exchange(data, Expect::Final, timeout)
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn read_loop<R: Read>(mut reader: R, pending: Arc<Mutex<Option<Pending>>>, urc_tx: SyncSender<String>) {
    let mut lines = LineAssembler::new(4096, true);
    let mut buf = [0u8; 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                log::error!("AT port read failed: {e}");
                break;
            }
        };
        for line in lines.feed(&buf[..n]) {
            handle_line(line, &pending, &urc_tx);
        }
        // The SMS prompt ("> ") is not followed by a line break.
        if buf[..n].ends_with(b"> ") || buf[..n].ends_with(b">") {
            let mut slot = lock(&pending);
            if slot.as_ref().is_some_and(|p| p.expect == Expect::Prompt) {
                let p = slot.take().expect("checked above");
                let _ = p.done.send(Ok(">".into()));
                lines.reset();
            }
        }
    }
    // Wake up any waiting command.
    lock(&pending).take();
}

fn handle_line(line: String, pending: &Mutex<Option<Pending>>, urc_tx: &SyncSender<String>) {
    let line = line.trim().to_string();
    if line.is_empty() {
        return;
    }
    if is_urc(&line) {
        let _ = urc_tx.try_send(line);
        return;
    }
    let mut slot = lock(pending);
    let Some(p) = slot.as_mut() else {
        log::debug!("unexpected line from modem: {line}");
        return;
    };
    let final_ok = line == "OK";
    let final_err = is_error(&line);
    p.lines.push(line);
    if final_ok || final_err {
        let p = slot.take().expect("pending present");
        let resp = p.lines.join("\r\n");
        let _ = p.done.send(if final_ok { Ok(resp) } else { Err(resp) });
    }
}

/// Collects URC lines for tests and simple consumers.
pub fn urc_channel() -> (SyncSender<String>, Receiver<String>) {
    mpsc::sync_channel(64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    /// A scripted fake modem: answers each command with the matching canned response.
    fn fake_modem(script: Vec<(&'static str, &'static str)>) -> (Arc<AtPort>, Receiver<String>) {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let (urc_tx, urc_rx) = urc_channel();
        let port = AtPort::new(ours.try_clone().unwrap(), ours, urc_tx);
        thread::spawn(move || {
            let mut writer = theirs.try_clone().unwrap();
            let mut reader = BufReader::new(theirs);
            for (expect, reply) in script {
                let mut cmd = Vec::new();
                // Commands end in \r; SMS bodies end in Ctrl-Z.
                let end = if expect.ends_with('\x1a') { 0x1a } else { b'\r' };
                if reader.read_until(end, &mut cmd).unwrap_or(0) == 0 {
                    return;
                }
                assert_eq!(String::from_utf8_lossy(&cmd), expect);
                writer.write_all(reply.as_bytes()).unwrap();
            }
            // Keep the connection open like a real modem would.
            thread::sleep(Duration::from_secs(3));
        });
        (port, urc_rx)
    }

    const T: Duration = Duration::from_secs(2);

    #[test]
    fn collects_response_until_ok() {
        let (port, _) = fake_modem(vec![("AT+CSQ\r", "\r\n+CSQ: 20,99\r\n\r\nOK\r\n")]);
        assert_eq!(port.at("AT+CSQ", T).unwrap(), "+CSQ: 20,99\r\nOK");
    }

    #[test]
    fn reports_errors() {
        let (port, _) = fake_modem(vec![("AT+CPIN?\r", "\r\n+CME ERROR: 10\r\n")]);
        match port.at("AT+CPIN?", T) {
            Err(AtError::Rejected(resp)) => assert_eq!(resp, "+CME ERROR: 10"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn separates_urcs_from_responses() {
        let (port, urcs) =
            fake_modem(vec![("AT+CLCC\r", "\r\nRING\r\n\r\n+CLCC: 1,1,4,0,0,\"123\",129\r\n\r\nOK\r\n")]);
        assert_eq!(port.at("AT+CLCC", T).unwrap(), "+CLCC: 1,1,4,0,0,\"123\",129\r\nOK");
        assert_eq!(urcs.recv_timeout(T).unwrap(), "RING");
    }

    #[test]
    fn sms_prompt_then_body() {
        let (port, _) = fake_modem(vec![("AT+CMGS=\"123\"\r", "\r\n> "), ("hello\x1a", "\r\n+CMGS: 7\r\n\r\nOK\r\n")]);
        let session = port.session();
        assert_eq!(session.at_prompt("AT+CMGS=\"123\"", T).unwrap(), ">");
        assert_eq!(session.raw("hello\x1a", T).unwrap(), "+CMGS: 7\r\nOK");
    }

    #[test]
    fn reports_closed_port() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let (urc_tx, _urcs) = urc_channel();
        let port = AtPort::new(ours.try_clone().unwrap(), ours, urc_tx);
        drop(theirs);
        assert!(matches!(port.at("AT", T), Err(AtError::Closed | AtError::Io(_))));
    }

    #[test]
    fn times_out_without_answer() {
        let (port, _) = fake_modem(vec![("AT\r", "")]);
        assert!(matches!(port.at("AT", Duration::from_millis(200)), Err(AtError::Timeout)));
    }
}
