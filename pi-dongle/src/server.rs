//! TCP control link to the iPhone app: the same newline-delimited JSON protocol as the
//! ESP32's BLE link (docs/protocol.md). The USB-C cable is a point-to-point network, so only
//! the plugged-in iPhone can reach this port.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use log::{info, warn};

pub enum LinkEvent {
    Line(String),
    /// A phone connected; send it the current state.
    Ready,
}

#[derive(Clone, Default)]
pub struct Clients(Arc<Mutex<Vec<TcpStream>>>);

impl Clients {
    /// Sends one line to every connected phone, dropping connections that fail.
    pub fn broadcast(&self, line: &str) {
        let mut clients = self.0.lock().unwrap_or_else(|e| e.into_inner());
        clients.retain_mut(|c| c.write_all(line.as_bytes()).is_ok());
    }
}

pub fn spawn(listen: String, clients: Clients, events: Sender<LinkEvent>) -> anyhow::Result<()> {
    thread::Builder::new().name("server".into()).spawn(move || run(&listen, clients, events))?;
    Ok(())
}

fn run(listen: &str, clients: Clients, events: Sender<LinkEvent>) {
    // The USB network interface only exists while an iPhone is plugged in; keep retrying.
    let listener = loop {
        match TcpListener::bind(listen) {
            Ok(l) => break l,
            Err(e) => {
                log::debug!("Waiting to listen on {listen}: {e}");
                thread::sleep(Duration::from_secs(2));
            }
        }
    };
    info!("Listening for the iPhone app on {listen}");
    for stream in listener.incoming().flatten() {
        let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
        info!("iPhone app connected from {peer}");
        let _ = stream.set_nodelay(true);
        let reader = match stream.try_clone() {
            Ok(r) => r,
            Err(e) => {
                warn!("Could not set up connection: {e}");
                continue;
            }
        };
        clients.0.lock().unwrap_or_else(|e| e.into_inner()).push(stream);
        let _ = events.send(LinkEvent::Ready);
        let events = events.clone();
        thread::spawn(move || {
            for line in BufReader::new(reader).lines() {
                match line {
                    Ok(line) if !line.trim().is_empty() => {
                        if events.send(LinkEvent::Line(line)).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            info!("iPhone app disconnected ({peer})");
        });
    }
}
