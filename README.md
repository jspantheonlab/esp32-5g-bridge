# ESP32 5G Bridge

A pocket-sized box built from a 5G modem + SIM card + ESP32-S3 that gives an iPhone:

| Feature | How it reaches the iPhone |
|---|---|
| **Internet** | Wi-Fi hotspot (WPA2/WPA3) NATed onto the 5G data session |
| **Voice calls** | Native CallKit call screen in the companion app. Audio streams over the hotspot. |
| **SMS** | Send/receive in the companion app, with notifications |

Use it as a second line, a travel SIM, or a data-only iPad/iPhone companion, without swapping SIMs.

```
 [SIM]─[5G module]══UART/CMUX + PCM══[ESP32-S3]~~~ Wi-Fi (data + call audio) ~~~[iPhone]
                                         └──────── BLE (control, call/SMS alerts) ───┘
```

See [docs/architecture.md](docs/architecture.md) for why the work is split this way.

## What iOS allows (read first)

- ✅ Data works for every app, because the bridge is just a Wi-Fi network.
- ✅ Calls ring with the **native full-screen call UI** (CallKit), including on the lock screen, and show up in Recents.
- ⚠️ The bridge's number is **not** a cellular line in the Phone/Messages apps. Calls are placed from the Bridge5G keypad, and SMS live in the Bridge5G app. iOS has no public API to add a cellular line from an accessory.
- ⚠️ The iPhone must be **on the bridge's Wi-Fi** for call audio and data, and **in BLE range** for alerts. If the app is force-quit, it won't wake for calls.
- ⚠️ Voice on 5G/4G needs **VoLTE/VoNR (IMS)** support from your carrier for this module.

## Repository layout

```
firmware/            ESP-IDF project (ESP32-S3, C)
  main/modem.c         esp_modem: power-on, SIM, IMS, registration, CMUX + PPP, status, AT/URC plumbing
  main/hotspot.c       Wi-Fi SoftAP + NAPT onto the PPP uplink
  main/voice.c         Call control (ATD/ATA/CHUP/VTS) + CLCC state machine
  main/audio_bridge.c  Modem PCM (I2S) ⇄ UDP audio with jitter buffer
  main/sms.c           Receive (+CMTI/CMGR), sync, delete, send (GSM-7 or UCS2)
  main/ble_link.c      NimBLE GATT control link, passkey bonding
  main/main.c          Wiring + JSON command dispatch
firmware-rs/         Same firmware in Rust (std on ESP-IDF); pick either this or firmware/
  bridge-core/         Host-tested logic: AT parsing, SMS/UCS2, calls, protocol, audio framing
  firmware/            ESP32-S3 binary: esp-idf-svc, esp32-nimble, esp_modem via a small C shim
ios/                 SwiftUI companion app (iOS 17+)
  Bridge5G/BridgeLink.swift   CoreBluetooth central with background restore
  Bridge5G/CallManager.swift  CallKit provider
  Bridge5G/VoiceAudio.swift   AVAudioEngine ⇄ UDP audio
  Bridge5G/MessageStore.swift Local SMS history
docs/
  hardware.md          Prototype BOM, wiring, bring-up checklist
  board-design.md      Battery-powered custom PCB: power tree, battery, GPIO map, layout, enclosure, cost
  protocol.md          BLE JSON protocol + UDP audio format
  architecture.md      Design rationale, limitations, upgrade paths
```

## Build: firmware

Requires [ESP-IDF v5.2–5.x](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/get-started/).

```bash
cd firmware
idf.py set-target esp32s3
idf.py menuconfig      # 5G Bridge → APN, hotspot SSID/password, BLE passkey, pins, PCM settings
idf.py build flash monitor
```

**Change the hotspot password and the BLE passkey before you deploy.** The defaults are public.

### Rust alternative

```bash
cd firmware-rs/firmware
BRIDGE_AP_PASSWORD='…' BRIDGE_BLE_PASSKEY=482913 BRIDGE_APN=your.apn cargo run --release
```

See [firmware-rs/README.md](firmware-rs/README.md) for toolchain setup (`espup`). Both firmwares speak the same protocol, so the iOS app works with either.

## Build: iOS app

Requires Xcode 15+ and [XcodeGen](https://github.com/yonaskolb/XcodeGen).

```bash
cd ios
xcodegen generate
open Bridge5G.xcodeproj
```

Set your signing team and bundle ID, then run on a real iPhone (BLE and CallKit don't work in the Simulator).

First run:
1. The app finds the bridge over BLE. Enter the passkey when iOS asks.
2. In iOS Settings → Wi-Fi, join `5G-Bridge`.
3. The **Bridge** tab shows SIM, operator, 5G/LTE and signal. The **Keypad** places calls, and **Messages** handles SMS.

## Status

This is a **starting scaffold that hasn't run on hardware yet**.

| Part | Compiled? | Tested? |
|---|---|---|
| `firmware-rs/` (Rust) | ✅ Builds for ESP32-S3 against ESP-IDF v5.5.5, clippy + rustfmt clean | `bridge-core` unit tests (31) pass on the host; no hardware testing |
| `firmware/` (C) | ✅ Builds with ESP-IDF v5.5.5, no warnings | No hardware testing |
| `ios/` | ⏳ Built by CI on a macOS runner (`.github/workflows/ci.yml`); never compiled locally | No |
| Hardware | Design document only (`docs/board-design.md`), no schematic/PCB yet | No |

The design follows the ESP-IDF, esp_modem, NimBLE, CallKit and CoreBluetooth APIs, but expect bring-up work. In particular, check the items under *Known limitations* in [docs/architecture.md](docs/architecture.md#known-limitations--verify-on-real-hardware): the esp_modem URC hook, `AT+QDAI` PCM settings and carrier IMS provisioning.

Suggested bring-up order: modem AT over USB on a PC → ESP32 + modem data/hotspot → BLE link + SMS → voice call control → call audio.

## Legal

Check local rules for operating cellular radios in a self-built device and your carrier's terms (some restrict tethering or unknown IMEIs). Use certified modules and the antennas they were certified with.
