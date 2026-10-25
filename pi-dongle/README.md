# bridge5g-dongle

Linux daemon for the Raspberry Pi USB-C dongle prototype. The user guide is [docs/pi-dongle.md](../docs/pi-dongle.md).

```
src/main.rs     wiring, command dispatch
src/at.rs       AT command engine over the modem's USB serial port (tested against a fake modem)
src/modem.rs    SIM/network bring-up, carrier APN, ECM data, status polling
src/voice.rs    call commands + CLCC state machine
src/sms.rs      read / sync / delete / send
src/audio.rs    modem USB audio (arecord/aplay) ⇄ UDP to the iPhone
src/server.rs   TCP control link on the USB network (same JSON protocol as the ESP32's BLE link)
system/         USB gadget script, systemd units, installer, one-time modem setup
```

Protocol parsing, SMS encoding, call state, carrier APNs and audio framing come from [`bridge-core`](../firmware-rs/bridge-core), shared with the ESP32 firmware.

## Build

On the Pi itself (Raspberry Pi OS 64-bit):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cd pi-dongle && cargo build --release
sudo system/setup.sh
```

Cross-compiling from an x86 Linux machine for Raspberry Pi OS Bookworm (glibc 2.36), as CI does:

```bash
rustup target add aarch64-unknown-linux-gnu
cargo install cargo-zigbuild        # also needs `zig` on PATH
cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36
```

## Test

```bash
cargo test     # AT engine against a fake modem, parsers
```

## Configuration

Environment variables, set in `/etc/default/bridge5g` on the Pi:

| Variable | Default | Meaning |
|---|---|---|
| `BRIDGE_AT_PORT` | `auto` | Modem AT port (`auto` = Quectel interface 2 under `/dev/serial/by-id`) |
| `BRIDGE_APN` | `internet` | `internet` = auto-detect from the SIM's carrier |
| `BRIDGE_SIM_PIN` | — | SIM PIN, if set |
| `BRIDGE_AUDIO_DEVICE` | `auto` | ALSA device of the modem's USB audio (`none` disables) |
| `BRIDGE_VOICE_ROUTE_CMD` | `AT+QPCMV=1,2` | Routes call audio to USB audio (module-specific) |
| `BRIDGE_LISTEN` | `192.168.7.1:5005` | Control link address on the USB network |
| `BRIDGE_AUDIO_PORT` | `5004` | Call-audio UDP port |
| `BRIDGE_USB_FUNCTION` | `ecm` | USB gadget function shown to the iPhone (`ecm` or `ncm`) |
| `RUST_LOG` | `info` | Log level |
