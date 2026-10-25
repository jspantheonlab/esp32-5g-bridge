//! Build-time configuration.
//!
//! Secrets and carrier settings come from environment variables at build time
//! (`BRIDGE_AP_PASSWORD=... cargo build --release`), falling back to the defaults below.
//! Pin assignments live in `main.rs` (typed GPIOs) and here (modem UART, used by the C shim).

macro_rules! env_or {
    ($name:literal, $default:literal) => {
        match option_env!($name) {
            Some(value) => value,
            None => $default,
        }
    };
}

const fn parse_u32(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut value = 0u32;
    let mut i = 0;
    while i < bytes.len() {
        assert!(bytes[i].is_ascii_digit(), "expected digits");
        value = value * 10 + (bytes[i] - b'0') as u32;
        i += 1;
    }
    value
}

// Cellular
pub const APN: &str = env_or!("BRIDGE_APN", "internet");
pub const SIM_PIN: &str = env_or!("BRIDGE_SIM_PIN", "");
/// Enable IMS (VoLTE / VoNR); required for calls on networks without 2G/3G.
pub const ENABLE_IMS: bool = true;

// Wi-Fi hotspot
pub const AP_SSID: &str = env_or!("BRIDGE_AP_SSID", "5G-Bridge");
pub const AP_PASSWORD: &str = env_or!("BRIDGE_AP_PASSWORD", "change-me-please");
pub const AP_CHANNEL: u8 = 6;
pub const AP_MAX_CONN: u16 = 4;

// Bluetooth control link
pub const BLE_NAME: &str = env_or!("BRIDGE_BLE_NAME", "5G-Bridge");
pub const BLE_PASSKEY: u32 = parse_u32(env_or!("BRIDGE_BLE_PASSKEY", "123456"));
const _: () = assert!(BLE_PASSKEY <= 999_999, "BLE passkey must be 6 digits");

// Modem UART (ESP32 side; through 1.8 V level shifters)
pub const MODEM_UART_PORT: i32 = 1;
pub const MODEM_TX_GPIO: i32 = 17;
pub const MODEM_RX_GPIO: i32 = 18;
pub const MODEM_RTS_GPIO: i32 = 15;
pub const MODEM_CTS_GPIO: i32 = 16;
/// Tried first; the firmware falls back to 115200 and switches the modem with AT+IPR.
pub const MODEM_BAUD: i32 = 921_600;
/// PWRKEY is driven through an NPN/N-MOSFET (GPIO high = PWRKEY low).
pub const PWRKEY_ACTIVE_HIGH: bool = true;

// Power (custom board only; see docs/board-design.md)
/// RTC-capable GPIO of the power button, used to wake from the low-battery shutdown.
pub const POWER_BUTTON_GPIO: i32 = 2;

// Voice audio
pub const PCM_SAMPLE_RATE: u32 = 8000;
/// The modem is PCM clock master by default; set true if the ESP32 should drive PCM_CLK/SYNC.
pub const I2S_CONTROLLER: bool = false;
/// Quectel syntax: io,mode(0=master),fsync(0=short),clock,format(0=16-bit linear),sample,slots,slot_map.
/// Verify against your module's AT manual.
pub const PCM_DAI_CMD: &str =
    if PCM_SAMPLE_RATE == 16000 { "AT+QDAI=1,0,0,2,0,1,1,1" } else { "AT+QDAI=1,0,0,1,0,0,1,1" };
pub const AUDIO_UDP_PORT: u16 = 5004;
