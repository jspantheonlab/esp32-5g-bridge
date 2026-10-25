# Board design (battery-powered custom PCB)

This is the target design for a single pocket-sized board running on its own battery. To prototype with dev boards and a bench supply first, see [hardware.md](hardware.md).

## Block diagram

```
 USB-C ──┬─ VBUS ─► TVS ─► BQ25895 charger / power-path ──► SYS (3.5–4.35 V) ──┬─► TPS22965 load switch ─► VBAT_5G (3 A bursts) ─► 5G module
         │                      │  ▲ I²C, INT, QON                              │                                                   │
         │                      ▼  │                                            └─► TPS63802 buck-boost ─► 3V3 ─► ESP32-S3, level shifters, LEDs
         │                1S Li-ion/LiPo (5000 mAh, NTC, PCM) ◄─ MAX17048 fuel gauge (I²C)
         │
         └─ D+/D− ─► TS3USB221 USB mux ─┬─► ESP32-S3 native USB  (flash / logs)
                         ▲ SEL (GPIO14) └─► 5G module USB 2.0    (QFlash firmware update, QCOM debug)

 ESP32-S3-WROOM-1 ◄── UART+RTS/CTS, PCM, PWRKEY, RESET, STATUS, RI, DTR ──► TXB0108 / transistors ◄── 1.8 V I/O ── 5G module
                                                                                          VDD_EXT (1.8 V ref) ──┘
 5G module ── ANT0..ANT3 (50 Ω CPWG + π-match) ──► 4× u.FL ──► 4× flexible 5G antennas
          └── USIM1 ─► nano-SIM socket (ESD + 22 Ω + 33 pF)      USIM2 ─► optional MFF2 eSIM footprint
```

## Key parts

| Block | Part | Why |
|---|---|---|
| 5G module | **Quectel RG520N-xx** (LGA, ~41 × 44 mm), regional variant for your carrier | Exposes main UART + PCM, which the firmware needs. M.2 cards are easier to assemble but rarely expose both. |
| MCU | **ESP32-S3-WROOM-1-N16R8** (PCB antenna) or **-1U** (u.FL for an external antenna in a metal case) | Wi-Fi + BLE + native USB. The octal PSRAM uses GPIO35–37. |
| Charger / power path | **TI BQ25895** (I²C, 5 A, NVDC) | The system rail is held at ≥ 3.5 V even with an empty battery while plugged in. The battery tops up 3 A modem bursts. It has a ship mode for storage and an ADC for VBUS/VBAT/temperature. |
| Fuel gauge | **MAX17048** | Accurate battery % to show in the app, with a low-battery alert pin. |
| 3.3 V rail | **TI TPS63802** buck-boost (2 A) | SYS sags to ~3.3 V under modem bursts on a low cell. An LDO would brown out the ESP32. |
| Modem switch | **TI TPS22965** (4 A, 16 mΩ) | Cuts the modem completely in "off" / low-battery states. Rail sequencing is under ESP32 control. |
| Level shifting | **TXB0108** (UART + PCM, push-pull) + NPN/N-MOSFET for PWRKEY, RESET_N; BSS138 for open-drain STATUS/RI | Quectel I/O is 1.8 V. VDD_EXT is the B-side reference, so the shifter powers down with the module and never back-powers it. |
| USB | USB-C receptacle (USB 2.0) + 2× 5.1 kΩ CC pull-downs + **TS3USB221** mux + TVS (e.g., TPD2E2U06) | One port for charging, ESP32 flashing and modem firmware updates. |
| SIM | Push-push nano-SIM socket + ESD array (e.g., TPD4E001) | Follow the module's reference SIM circuit. |
| Antennas | 4× flexible FPC 5G antennas (600 MHz–6 GHz, e.g., Taoglas FXUB66 or equivalent) + 4× u.FL/MHF4 | Mount on the enclosure walls, as far apart as possible, away from the battery and USB. |
| UI | Power button (to BQ25895 QON + ESP32), user/pair button, WS2812 RGB LED, optional 0.96" SSD1306 OLED | With an OLED, the BLE passkey can be random per pairing instead of fixed (stronger security). |

## Power budget & battery

All figures are planning estimates. Measure them on your board (module datasheets give per-band numbers).

| Mode | 5G module | ESP32-S3 | Total @ 3.7 V | 5000 mAh runtime* |
|---|---|---|---|---|
| Standby: calls + SMS only, hotspot off, BLE connected | 15–40 mA | 5–15 mA (light sleep) | ~30–60 mA | **~3–6 days** |
| Hotspot on, idle | 50–100 mA | 100–130 mA | ~180–250 mA | **~17–23 h** |
| Voice call | 300–500 mA | 130–160 mA | ~450–650 mA | **~7–9 h talk** |
| Active data (UART-limited) | 500–900 mA | 130–180 mA | ~0.7–1.1 A | **~4–6 h** |
| Peak (TX burst, poor signal) | up to 3 A for ms | 0.5 A | **3.5 A** | design limit |

\*Uses ~85% of the rated capacity after converter losses and the low-voltage cutoff.

**Battery recommendation:** a 1S LiPo pouch, **5000 mAh** (e.g., 10 × 50 × 60 mm "105060" size), with a built-in protection board (PCM) and a **10 kΩ NTC** lead (3-wire JST-PH). Pick a cell rated for **≥ 2C discharge** (10 A) so the 3 A bursts don't sag the voltage. Alternative: 2× matched 18650 in parallel (6000–7000 mAh), which is thicker but cheap and robust.

- Charge at 2 A (≈ 3 h full charge), charge voltage 4.20 V. Don't use 4.35 V "HV" cells: SYS would exceed the module's 4.4 V VBAT max.
- Low-battery policy: warn the app at 15%. At ~3.5 V under load (≈ 5%), shut down the modem cleanly (`AT+QPOWD`), then the ESP32. The cell PCM is the last line of defense at ~3.0 V.
- Storage/shipping: long-press power → BQ25895 ship mode (BATFET off, ~µA drain). Press the button again to wake.
- If you ship batteries, buy cells with UN38.3 test reports.

## ESP32-S3 GPIO map

This keeps the current firmware defaults (UART, PCM, PWRKEY) unchanged.

| GPIO | Function | Notes |
|---|---|---|
| 1 | MODEM_RI (in) | RTC GPIO: wakes the ESP32 from sleep on an incoming call or SMS |
| 2 | BTN_POWER (in) | RTC GPIO, also wired to BQ25895 /QON through a diode |
| 4 / 5 / 6 / 7 | PCM_CLK / PCM_SYNC / PCM_DIN / PCM_DOUT | via TXB0108 |
| 8 / 9 | I²C SDA / SCL | BQ25895, MAX17048, OLED (4.7 kΩ pull-ups to 3V3) |
| 10 | CHARGER_INT (in) | open-drain |
| 11 | GAUGE_ALRT (in) | open-drain |
| 12 | BTN_USER (in) | reserved for a future BLE pairing-mode button (factory reset uses BOOT/GPIO0) |
| 13 | MODEM_VBAT_EN (out) | TPS22965 ON |
| 14 | USB_MUX_SEL (out) | 0 = ESP32, 1 = modem. Add a test pad/jumper to force the ESP32 for recovery. |
| 15 / 16 / 17 / 18 | UART RTS / CTS / TX / RX | via TXB0108 |
| 19 / 20 | USB D− / D+ | native USB via the mux |
| 21 | MODEM_PWRKEY (out) | NPN, high = PWRKEY low |
| 38 | MODEM_RESET (out) | NPN, high = RESET_N low |
| 39 | MODEM_STATUS (in) | BSS138 shifter |
| 40 | MODEM_DTR (out) | lets the module sleep between events |
| 41 | MODEM_W_DISABLE (out) | airplane mode |
| 47 | LED (WS2812 data) | |
| 0 | BOOT button | Strapping pin (hold at power-on = download mode). At runtime, **hold 10 s = factory reset**. |
| 3, 45, 46 | strapping, leave free | |

## PCB guidelines

- **Stack-up:** 4 layers minimum (L1 signal/RF, L2 solid GND, L3 power pours, L4 signal). 6 layers make RF and power routing much easier. Order **impedance control**.
- **RF:** 50 Ω coplanar waveguide with ground referenced to L2. Keep traces short with a π-network (0 Ω / DNP) at each antenna pad. Use via fences along RF traces. Don't route anything under the RF pads.
- **Modem power:** a VBAT_5G pour ≥ 2 mm wide from the switch to the module. Put ≥ 470 µF low-ESR polymer bulk capacitance right at the VBAT pins, plus 100 nF / 33 pF / 10 pF per pin. Add a 5.1 V TVS/Zener for surge.
- **Thermal:** a grid of thermal vias under the module's ground pads into an L2/L4 copper area. Use a thermal pad from the module shield to the enclosure (or a small aluminium plate). Read `AT+QTEMP` in firmware and throttle above ~75 °C.
- **USB:** 90 Ω differential pairs for both the modem and ESP32 legs, mux placed close to the connector.
- **ESP32 antenna:** PCB-antenna module on the board edge with a copper keep-out on all layers, or use the -1U version.
- **Test points:** VBAT_5G, SYS, 3V3, VDD_EXT, UART TX/RX, PCM_CLK/SYNC, USB_BOOT for the module, GPIO0/EN for the ESP32.
- **Size:** roughly **65 × 100 mm** board. The enclosure is about **75 × 115 × 25 mm** with the pouch cell stacked under the PCB.

## Enclosure

- Start with 3D-printed PETG/ASA, which is RF-transparent. Don't use metal or carbon-filled filaments near the antennas.
- Mount the antennas on the inside walls (two on the long sides, one on each short side) with 3M double-sided tape on the FPC antennas.
- Add a vent slot over the module or a thermal pad to an aluminium back plate. Leave a foam spacer between the cell and the PCB, plus room for the pouch to swell.
- Cutouts: USB-C, SIM tray (or a screw-on SIM door), 2 buttons, LED light pipe.

## Manufacturing

- The RG520N is an LGA part. It's practically impossible to hand-solder. Use a PCBA service (JLCPCB, PCBWay, etc.) and either consign the module or have them source it. Ask for X-ray inspection of the LGA.
- Everything else can come from the assembler's parts library. Hand-solder the u.FL antennas, battery connector and buttons if needed.
- Order 5 boards: 2 for bring-up, 1 to sacrifice for RF/thermal tests, 2 spares.

## Ballpark cost (prototype quantity)

| Item | ~USD |
|---|---|
| Quectel RG520N module | 100–180 |
| ESP32-S3-WROOM-1 | 4–6 |
| Charger, gauge, regulators, load switch, mux, shifters, passives | 15–25 |
| 4× 5G FPC antennas + u.FL | 20–50 |
| 5000 mAh LiPo with NTC | 10–20 |
| 4-layer PCB + assembly (5 pcs, per board) | 30–80 |
| 3D-printed enclosure, buttons, light pipe | 10–20 |
| **Per unit** | **~$190–380** |

## Tools you'll need

| Tool | Use |
|---|---|
| Bench power supply, 0–6 V / ≥ 5 A, with current limit | Power the module before you trust the battery circuit |
| Multimeter | Rails, continuity |
| Oscilloscope (≥ 50 MHz) | VBAT droop during TX bursts, PCM clock/sync |
| Logic analyzer (8 ch) | UART/CMUX, PCM, I²C debugging |
| USB-UART adapter (1.8 V capable) | Talk to the module UART directly |
| Fine-tip soldering iron, hot-air station, flux, ESD mat | Rework, connectors |
| LiPo-safe charging bag / fireproof area | Battery testing |
| Software: KiCad 8+, ESP-IDF, Quectel QFlash + QCOM/QNavigator | Design, firmware, modem updates |

## Bring-up order for the custom board

1. Without a battery or module: check SYS, 3V3 and charger I²C (read BQ25895 registers). Flash the ESP32 over USB-C.
2. Feed the battery input from the bench supply at 3.8 V with a 4 A limit. Enable the TPS22965 and check VBAT_5G and its droop on the scope.
3. Switch the USB mux to the modem and talk to it from a PC with QCOM: `AT`, `AT+CPIN?`, then update the modem firmware if needed.
4. Run the firmware: UART/CMUX/PPP → hotspot → BLE → SMS → calls → PCM audio.
5. Connect the real battery. Check charging, temperature (NTC), the fuel gauge and the low-battery shutdown.
6. In the enclosure: compare signal (`AT+QENG="servingcell"` RSRP/SINR) with the board in open air, and move antennas to optimize.

## Firmware follow-ups this board enables

Done:
- MAX17048 + BQ25895 monitoring (both firmwares). Battery %, charging and USB power go in the `status` message and show in the app, with a low-battery notification at 15%. At 3% on battery the bridge powers the modem down (`AT+QPOWD`) and deep-sleeps until the power button (GPIO2) is pressed.

Not implemented yet:
- Power modes: turn the hotspot on only when the app asks or a client is associated. Use light sleep with BLE + RI wake for multi-day standby.
- Modem power control via MODEM_VBAT_EN, ship mode on long press.
- Random BLE passkey shown on the OLED.
- Thermal throttling from `AT+QTEMP`.
