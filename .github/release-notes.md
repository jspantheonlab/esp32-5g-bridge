> ⚠️ **Pre-release. Nothing has been tested on real hardware yet.** Everything compiles and the unit tests pass, but the modem, call audio and carrier VoLTE setup are unverified. Expect bring-up work; see the [build guide](https://github.com/jspantheonlab/esp32-5g-bridge/blob/main/docs/hardware.md).

## Files

| File | What it is |
|---|---|
| `esp32-5g-bridge-c-*.bin` | C firmware for ESP32-S3 (8 MB flash), single merged image |
| `esp32-5g-bridge-rust-*.bin` | Rust firmware, same features. Flash **one** of the two. |
| `bridge5g-dongle-aarch64-*.tar.gz` | Raspberry Pi USB-C dongle prototype: daemon + installer (`sudo ./setup.sh`), see [docs/pi-dongle.md](https://github.com/jspantheonlab/esp32-5g-bridge/blob/main/docs/pi-dongle.md) |
| `Bridge5G-unsigned-*.ipa` | iPhone app (iOS 17+), **unsigned**: you sign it with your own Apple ID |
| `SHA256SUMS.txt` | Checksums |

## Flash the firmware

Both images flash at offset **0x0**:

```bash
pip install esptool
esptool.py --chip esp32s3 write_flash 0x0 esp32-5g-bridge-rust-v0.1.0.bin
```

…or in a browser with [ESP Tool](https://espressif.github.io/esptool-js/) (Chrome/Edge): connect, set the address to `0x0`, choose the file, Program.

## Install the iPhone app

Apple doesn't allow installing unsigned apps directly. Either:
- **Sideload** the `.ipa` with [Sideloadly](https://sideloadly.io/) or [AltStore](https://altstore.io/), which sign it with your Apple ID. With a free Apple ID it must be refreshed every 7 days.
- Or **build from source** on a Mac with Xcode: `cd ios && xcodegen generate`, open the project, set your team, Run.

## Default settings: change them

These release images use the **public defaults**:

| Setting | Default | How to change |
|---|---|---|
| Hotspot name / password | `5G-Bridge` / `change-me-please` | In the app: Bridge → Hotspot → Network name & password |
| Bluetooth pairing passkey | `123456` | Only at build time (`BRIDGE_BLE_PASSKEY` / Kconfig). Anyone nearby who knows it can pair while the bridge is powered, so **build your own firmware** for anything beyond bench testing. |
| APN | `internet` | Build time (`BRIDGE_APN`). Many carriers work with the default; if data doesn't come up, build with your carrier's APN. |

**Factory reset:** hold the board's BOOT button for 10 s (the app warns at 5 s). This clears the app-set hotspot settings and Bluetooth pairings.

## Features in this release

- Wi-Fi hotspot routed over the 5G modem's data session (NAPT)
- Voice calls with the native iOS call screen (CallKit), audio over the hotspot
- SMS send/receive in the app (GSM-7 and Unicode, single segment)
- Encrypted, passkey-bonded Bluetooth control link with background wake-up
- Set the hotspot name and password from the app
- Battery level, charging and low-battery shutdown (custom board with MAX17048 + BQ25895)
- Factory reset by long-pressing BOOT
