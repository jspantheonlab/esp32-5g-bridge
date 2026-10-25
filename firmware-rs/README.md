# Rust firmware

A Rust port of [`../firmware`](../firmware) (C). It behaves the same, uses the same BLE protocol and UDP audio format, and works with the same iOS app.

```
firmware-rs/
  bridge-core/   Pure Rust, no ESP dependencies: AT parsing, SMS/UCS2, call state,
                 JSON protocol (serde), audio packets + jitter buffer. Unit-tested on the host.
  firmware/      ESP32-S3 binary (std Rust on ESP-IDF via esp-idf-svc)
    src/main.rs      wiring, command dispatch
    src/modem.rs     AT channel, URCs, supervisor thread (power, SIM, IMS, registration, CMUX+PPP)
    src/hotspot.rs   Wi-Fi AP + NAPT onto PPP
    src/ble.rs       esp32-nimble GATT server, passkey bonding
    src/voice.rs     call commands + CLCC state machine
    src/sms.rs       read / sync / delete / send
    src/audio.rs     I2S PCM ⇄ UDP
    src/config.rs    APN, SSID, passkey, pins
    components/modem_shim/   ~150 lines of C over esp_modem (keeps its C/C++ config macros in C)
```

## Why there's still a little C

`esp_modem` is a C++ component that's set up through C macros (`ESP_MODEM_DTE_DEFAULT_CONFIG`, `ESP_NETIF_DEFAULT_PPP`). Bindgen can't translate those macros, so `modem_shim` wraps them in plain functions and Rust calls those. Everything else is Rust.

## Toolchain setup (once)

```bash
# Rust + the Espressif Xtensa toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo install espup ldproxy espflash
espup install --targets esp32s3
. ~/export-esp.sh            # every new shell
```

ESP-IDF needs Python with `venv`. On Debian/Ubuntu, `sudo apt install python3-venv`, or put a standalone Python first on `PATH` (e.g., `uv python install 3.12`).

## Build & flash

```bash
cd firmware-rs/firmware
# Builds, flashes (espflash, 8 MB flash, partitions.csv) and opens the serial monitor.
# Pass the same variables every time: settings are compiled in, and changing them triggers a rebuild.
BRIDGE_AP_PASSWORD='your-wifi-password' \
BRIDGE_BLE_PASSKEY=482913 \
BRIDGE_APN=your.apn \
cargo run --release
```

The first build downloads ESP-IDF v5.5.5 and its tools into `firmware/.embuild/` (~5 GB) and takes a while. Later builds are incremental.

Build-time settings (environment variables): `BRIDGE_APN`, `BRIDGE_SIM_PIN`, `BRIDGE_AP_SSID`, `BRIDGE_AP_PASSWORD`, `BRIDGE_BLE_NAME`, `BRIDGE_BLE_PASSKEY`. Pins and PCM settings are in `src/config.rs` and `src/main.rs`. sdkconfig options are in `firmware/sdkconfig.defaults`.

## Tests

```bash
cd firmware-rs/bridge-core
cargo test
```

## Differences from the C firmware

- Data-session loss is detected by polling the PPP address every second, not through an IP event callback.
- Call commands run on the voice thread, so they're serialized with the CLCC polling.
- Same caveats as the C version apply. Verify on hardware: the esp_modem URC hook in CMUX mode, the `AT+QDAI` values, and IMS provisioning. See [../docs/architecture.md](../docs/architecture.md#known-limitations--verify-on-real-hardware).
