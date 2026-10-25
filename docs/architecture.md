# Architecture

```
                         ┌───────────────────────── ESP32-S3 ──────────────────────────┐
                         │                                                              │
 ┌──────────┐  UART+CMUX │  ┌────────────┐   PPP netif   ┌──────────┐  NAPT  ┌─────────┐ │   Wi-Fi (WPA2/3)   ┌────────────────┐
 │          │◄──────────►│  │  modem.c   │──────────────►│  lwIP    │◄──────►│hotspot.c│◄├───────────────────►│                │
 │   5G     │  ch1: PPP  │  │ esp_modem  │               └──────────┘        └─────────┘ │  data              │                │
 │  module  │  ch2: AT   │  │ AT + URCs  │──┬─► sms.c  ──┐                               │                    │    iPhone      │
 │ (RG520N) │            │  └────────────┘  └─► voice.c ┼──► app_emit ──► ble_link.c ◄──├───── BLE GATT ────►│  Bridge5G app  │
 │          │  PCM/I2S   │  ┌──────────────────┐   ▲    │     (JSON lines)             │  control, events   │  (CallKit UI)  │
 │          │◄──────────►│  │ audio_bridge.c   │◄──┘    │                              │                    │                │
 └──────────┘  8/16 kHz  │  │ I2S ⇄ UDP :5004  │◄───────┴──────────────────────────────├───── UDP audio ───►│                │
      ▲                  │  └──────────────────┘                                       │  over Wi-Fi        └────────────────┘
   SIM card              └──────────────────────────────────────────────────────────────┘
```

## Why the work is split this way

**Data → Wi-Fi hotspot.** iOS has no API for an accessory to act as a cellular modem. The only system-wide way to give an iPhone internet from an external box is to make it a Wi-Fi network (or Ethernet via a USB adapter). The ESP32 runs a SoftAP and NATs it onto the modem's PPP link.

**Control → BLE.** Incoming calls and SMS must reach the app while it's suspended. iOS keeps an app's BLE central connection alive in the background (`bluetooth-central` mode with state restoration) and wakes the app on notifications. A Wi-Fi TCP socket gets frozen. An incoming-call notification over BLE lets the app call `CXProvider.reportNewIncomingCall`, which shows the native full-screen call UI even on the lock screen.

**Call audio → UDP over Wi-Fi.** BLE can't carry duplex voice from a third-party app: iOS won't act as an HFP *hands-free* unit, and LE Audio isn't open to apps for this. The iPhone is already joined to the bridge's Wi-Fi for data, so 20 ms PCM frames go over UDP (32 kbit/s each way at 8 kHz). `AVAudioEngine` voice processing handles echo cancellation.

**SMS → companion app.** Third-party apps can't insert messages into Messages.app. SMS live in the Bridge5G app with local notifications. Messages stay on the SIM until the app acknowledges them, so nothing is lost while the phone is away.

## Firmware tasks

| Task | Priority | Role |
|---|---|---|
| `audio` | 10 | I2S ⇄ UDP loop, paced by the PCM clock. Runs only during active calls. |
| `modem_urc` | 6 | Dispatches unsolicited result codes (RING, +CLIP, +CMTI, NO CARRIER…). |
| esp_modem DTE | 6 | UART/CMUX I/O. |
| `modem` | 5 | Power-on, baud negotiation, SIM/IMS/registration, CMUX+PPP, 10 s status poll, reconnect. |
| `voice` | 5 | Call state machine. Polls `AT+CLCC` every 500 ms while a call exists; URCs kick it. |
| `app` | 5 | Parses BLE commands and dispatches them. |
| `sms` | 4 | Read / sync / delete / send queue (sending can block for up to 60 s). |
| NimBLE host | — | GATT, pairing, notifications. |

All AT access goes through `modem_at()` under one recursive mutex. The SMS code holds the mutex while it temporarily switches `AT+CSCS="UCS2"`, so other tasks never see UCS2-encoded responses.

## Known limitations / verify on real hardware

These parts depend on module- and library-version details that couldn't be checked without hardware:

1. **esp_modem URC callback** (`esp_modem_set_urc`, `CONFIG_ESP_MODEM_URC_HANDLER`, checked against esp_modem 1.4.3). The callback receives the whole unconsumed receive buffer, not just new bytes. In UART mode the same bytes repeat until a command completes, so both firmwares skip the already-seen prefix (`BufferDelta` in `bridge-core`, `urc_rx` in `modem.c`). Confirm on hardware that `RING` and `+CMTI` arrive in CMUX mode. If they don't, poll `AT+CLCC` / `AT+CMGL` periodically as a fallback.
2. **`AT+QDAI` parameters** differ between Quectel families. Confirm the PCM clock and sync format on a scope, and match `CONFIG_BRIDGE_I2S_MASTER`.
3. **IMS/VoLTE provisioning** is carrier-specific (MBN selection). Without it, `ATD` fails on 4G/5G-only networks.
4. **Background incoming calls** rely on the BLE link staying connected. If the user force-quits the app, iOS won't relaunch it from BLE. That's an iOS rule.
5. **Carrier rules:** using a 5G module in a self-built device may require the module's certification to carry over (PTCRB/GCF, FCC/CE). Check your carrier's IMEI whitelist.

## Upgrade paths

| Goal | Change |
|---|---|
| Real 5G throughput (100+ Mbit/s) | Let the module route by itself (Quectel RM5xx "router mode" over Ethernet/RGMII to a Wi-Fi 6 SoC). Keep the ESP32 only for BLE control + voice. |
| 30–200 Mbit/s with Espressif parts | **ESP32-P4** (USB 2.0 High-Speed host → modem RNDIS/ECM) + **ESP32-C5** (dual-band Wi-Fi 6) via esp-hosted. |
| Fewer wires for voice | Quectel USB voice (`AT+QPCMV=1,0` or UAC) instead of PCM. Needs a USB host (S3 full-speed is enough for voice). |
| Long SMS | PDU mode (`AT+CMGF=0`) with UDH concatenation. |
| Encrypted audio | SRTP keyed over the BLE link (it's already encrypted). |
| Calls when the app isn't running | Relay via your own server → APNs VoIP push (PushKit). |
