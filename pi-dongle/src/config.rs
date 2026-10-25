//! Runtime configuration from environment variables (systemd reads /etc/default/bridge5g).

use std::fs;
use std::path::PathBuf;

use bridge_core::carrier::DEFAULT_APN;

pub struct Config {
    /// Modem AT serial port.
    pub at_port: PathBuf,
    /// Address the iPhone app connects to (the Pi's end of the USB-C network link).
    pub listen: String,
    pub audio_port: u16,
    /// "internet" means: pick the APN from the SIM's carrier.
    pub apn: String,
    pub sim_pin: String,
    /// ALSA device of the modem's USB audio, or None to disable call audio.
    pub audio_device: Option<String>,
    /// AT command that routes call audio to USB audio (module specific).
    pub voice_route_cmd: String,
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string())
}

impl Config {
    pub fn from_env() -> Self {
        let at_port = std::env::var("BRIDGE_AT_PORT")
            .ok()
            .filter(|v| !v.is_empty() && v != "auto")
            .map(PathBuf::from)
            .unwrap_or_else(find_quectel_at_port);
        let audio_device = match env("BRIDGE_AUDIO_DEVICE", "auto").as_str() {
            "none" => None,
            "auto" => find_modem_audio_card().map(|card| format!("plughw:CARD={card},DEV=0")),
            dev => Some(dev.to_string()),
        };
        Self {
            at_port,
            listen: env("BRIDGE_LISTEN", "192.168.7.1:5005"),
            audio_port: env("BRIDGE_AUDIO_PORT", "5004").parse().unwrap_or(5004),
            apn: env("BRIDGE_APN", DEFAULT_APN),
            sim_pin: env("BRIDGE_SIM_PIN", ""),
            audio_device,
            voice_route_cmd: env("BRIDGE_VOICE_ROUTE_CMD", "AT+QPCMV=1,2"),
        }
    }
}

/// Quectel modules expose the AT command port as USB interface 2.
fn find_quectel_at_port() -> PathBuf {
    let by_id = PathBuf::from("/dev/serial/by-id");
    fs::read_dir(&by_id)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            name.contains("quectel") && name.contains("if02")
        })
        .unwrap_or_else(|| PathBuf::from("/dev/ttyUSB2"))
}

/// Finds the modem's USB audio card name in /proc/asound/cards (e.g. "Module" or "Quectel").
fn find_modem_audio_card() -> Option<String> {
    let cards = fs::read_to_string("/proc/asound/cards").ok()?;
    cards.lines().find_map(|line| {
        let lower = line.to_lowercase();
        if !(lower.contains("quectel") || lower.contains("module")) {
            return None;
        }
        // " 2 [Module         ]: USB-Audio - ..."  → "Module"
        let start = line.find('[')? + 1;
        let end = line.find(']')?;
        Some(line[start..end].trim().to_string())
    })
}
