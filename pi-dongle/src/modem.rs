//! Modem supervision: wait for SIM and network, pick the APN, start the data connection
//! (the modem runs in ECM mode and routes data itself), and poll status for the app.
//! Mirrors firmware-rs/firmware/src/modem.rs without the ESP32-specific parts.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Result};
use bridge_core::protocol::{Event, Status};
use bridge_core::{at, carrier};
use log::{info, warn};

use crate::at::{AtError, AtPort};
use crate::config::Config;
use crate::Emit;

const SHORT: Duration = Duration::from_secs(2);

pub struct Supervisor {
    pub at: Arc<AtPort>,
    pub status: Arc<Mutex<Status>>,
    pub emit: Emit,
    pub apn: String,
    pub sim_pin: String,
    pub voice_route_cmd: String,
}

pub fn spawn(sup: Supervisor) -> Result<()> {
    thread::Builder::new().name("modem".into()).spawn(move || sup.run())?;
    Ok(())
}

impl Supervisor {
    pub fn new(at: Arc<AtPort>, status: Arc<Mutex<Status>>, emit: Emit, config: &Config) -> Self {
        Self {
            at,
            status,
            emit,
            apn: config.apn.clone(),
            sim_pin: config.sim_pin.clone(),
            voice_route_cmd: config.voice_route_cmd.clone(),
        }
    }

    fn run(self) {
        loop {
            match self.bring_up() {
                Ok(()) => self.monitor(),
                Err(e) => warn!("Modem bring-up failed: {e:#}"),
            }
            thread::sleep(Duration::from_secs(10));
        }
    }

    /// Sends a command whose failure is not fatal.
    fn try_at(&self, cmd: &str) {
        if let Err(e) = self.at.at(cmd, SHORT) {
            exit_if_port_gone(&e);
            log::debug!("{cmd}: {e}");
        }
    }

    fn bring_up(&self) -> Result<()> {
        if !(0..30).any(|_| self.at.at("AT", Duration::from_secs(1)).is_ok() || sleep_false(1)) {
            bail!("modem not answering on the AT port");
        }
        for cmd in ["ATE0", "AT+CMEE=2", "AT+CSCS=\"IRA\"", "AT+COPS=3,0"] {
            self.try_at(cmd);
        }
        self.check_ecm_mode();
        self.wait_sim_ready()?;
        self.select_apn();
        // Carrier profile (VoLTE), IMS, and call audio over USB audio.
        for cmd in ["AT+QMBNCFG=\"AutoSel\",1", "AT+QCFG=\"ims\",1", self.voice_route_cmd.as_str()] {
            self.try_at(cmd);
        }
        for cmd in ["AT+CLIP=1", "AT+CMGF=1", "AT+CNMI=2,1,0,0,0"] {
            self.try_at(cmd);
        }
        self.wait_registered(Duration::from_secs(180))?;
        // Ask the modem to bring up its data connection (ECM mode); harmless if already up.
        self.try_at("AT+QNETDEVCTL=1,1,1");
        self.refresh_status();
        info!("Modem ready");
        Ok(())
    }

    fn check_ecm_mode(&self) {
        match self.at.at("AT+QCFG=\"usbnet\"", SHORT) {
            Ok(resp) if resp.contains(",1") => {}
            Ok(resp) => warn!(
                "Modem is not in ECM mode ({}). Run `sudo bridge5g-setup-modem` once so it provides the data link.",
                resp.lines().next().unwrap_or_default()
            ),
            Err(e) => log::debug!("usbnet query: {e}"),
        }
    }

    fn wait_sim_ready(&self) -> Result<()> {
        for _ in 0..20 {
            if let Ok(resp) = self.at.at("AT+CPIN?", SHORT) {
                match at::parse_cpin(&resp) {
                    Some("READY") => return Ok(()),
                    Some(code) if code.contains("PUK") => bail!("SIM is PUK-locked"),
                    Some("SIM PIN") if !self.sim_pin.is_empty() => {
                        let _ = self.at.at(&format!("AT+CPIN=\"{}\"", self.sim_pin), Duration::from_secs(5));
                    }
                    _ => {}
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
        bail!("SIM not ready")
    }

    fn select_apn(&self) {
        let resp = self.at.at("AT+CIMI", SHORT).ok();
        let imsi = resp.as_deref().and_then(carrier::parse_cimi);
        let apn = carrier::choose_apn(&self.apn, imsi);
        match imsi.and_then(carrier::carrier_for_imsi) {
            Some(c) => info!("SIM carrier: {} (APN \"{apn}\")", c.name),
            None => info!("Using APN \"{apn}\""),
        }
        self.try_at(&format!("AT+CGDCONT=1,\"IP\",\"{apn}\""));
    }

    fn wait_registered(&self, timeout: Duration) -> Result<()> {
        const QUERIES: [(&str, &str); 2] = [("AT+C5GREG?", "+C5GREG:"), ("AT+CEREG?", "+CEREG:")];
        for _ in 0..timeout.as_secs() / 2 {
            for (cmd, prefix) in QUERIES {
                let stat = self.at.at(cmd, SHORT).ok().and_then(|r| at::parse_registration(&r, prefix));
                if let Some(stat) = stat.filter(|s| at::is_registered(*s)) {
                    info!("Registered ({prefix} stat={stat})");
                    return Ok(());
                }
            }
            thread::sleep(SHORT);
        }
        bail!("network registration timed out")
    }

    fn monitor(&self) {
        loop {
            thread::sleep(Duration::from_secs(10));
            self.refresh_status();
            if !self.status.lock().unwrap_or_else(|e| e.into_inner()).reg {
                warn!("Lost network registration");
                return;
            }
        }
    }

    fn refresh_status(&self) {
        let query = |cmd: &str| match self.at.at(cmd, Duration::from_secs(3)) {
            Ok(resp) => Some(resp),
            Err(e) => {
                exit_if_port_gone(&e);
                None
            }
        };
        let sim = query("AT+CPIN?").is_some_and(|r| at::parse_cpin(&r) == Some("READY"));
        let rssi = query("AT+CSQ").and_then(|r| at::parse_csq(&r));
        let operator = query("AT+COPS?").and_then(|r| at::parse_cops(&r));
        let ip = query("AT+CGPADDR=1").and_then(|r| parse_cgpaddr(&r));

        let snapshot = {
            let mut st = self.status.lock().unwrap_or_else(|e| e.into_inner());
            st.sim = sim;
            st.rssi = rssi;
            st.reg = operator.as_ref().is_some_and(|o| !o.name.is_empty());
            st.rat = at::rat_name(operator.as_ref().and_then(|o| o.act)).into();
            st.op = operator.map(|o| o.name).unwrap_or_default();
            st.data = ip.is_some();
            st.ip = ip.unwrap_or_default();
            st.clone()
        };
        (self.emit)(Event::Status(snapshot));
    }
}

fn sleep_false(secs: u64) -> bool {
    thread::sleep(Duration::from_secs(secs));
    false
}

/// The USB serial device disappears when the modem resets; let systemd restart us cleanly.
pub fn exit_if_port_gone(e: &AtError) {
    if matches!(e, AtError::Closed | AtError::Io(_)) {
        log::error!("Lost the modem's AT port ({e}); exiting so systemd restarts the service");
        std::process::exit(1);
    }
}

/// `+CGPADDR: 1,"10.12.0.4"` (some firmware omits the quotes) → address, if any.
fn parse_cgpaddr(resp: &str) -> Option<String> {
    let line = resp.lines().map(str::trim).find_map(|l| l.strip_prefix("+CGPADDR:"))?;
    let addr = line.split(',').nth(1)?.trim().trim_matches('"');
    (!addr.is_empty() && addr != "0.0.0.0").then(|| addr.to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_cgpaddr;

    #[test]
    fn cgpaddr() {
        assert_eq!(parse_cgpaddr("+CGPADDR: 1,\"10.12.0.4\"\r\nOK").as_deref(), Some("10.12.0.4"));
        assert_eq!(parse_cgpaddr("+CGPADDR: 1,10.1.2.3\r\nOK").as_deref(), Some("10.1.2.3"));
        assert_eq!(parse_cgpaddr("+CGPADDR: 1,\"0.0.0.0\"\r\nOK"), None);
        assert_eq!(parse_cgpaddr("+CGPADDR: 1\r\nOK"), None);
    }
}
