# Hardware

This page covers the **prototype** built from dev boards and a bench supply. For the battery-powered custom PCB (charger, fuel gauge, power budget, layout, enclosure), see [board-design.md](board-design.md).

## Bill of materials

| Part | Suggested | Notes |
|---|---|---|
| MCU | **ESP32-S3-DevKitC-1** (N8 or N16R8) | Wi-Fi AP + BLE 5. Needs one UART with RTS/CTS and one I2S port. |
| 5G module | **Quectel RG520N / RG500Q** (LGA) on its EVB or a breakout, or a **RM520N/RM530N** M.2 card on a carrier that exposes **UART + PCM** | Must expose main UART (with flow control) **and** a PCM/I2S voice interface. Many M.2 cards only bring out USB/PCIe. Check the hardware design guide before you buy. |
| SIM | Nano-SIM socket (usually on the EVB/carrier) | You need a SIM plan with **VoLTE/VoNR** for calls. Most 5G networks have shut down 2G/3G. |
| Antennas | 4× wideband 5G NR antennas (600 MHz–6 GHz), IPEX/MHF4 | A 5G module won't register reliably with fewer than 2. Use all 4 for best throughput. |
| Modem power | 3.8 V buck, **≥ 3 A peak** (e.g., TPS56339, MP2315S) + ≥ 470 µF low-ESR bulk cap | 5G modules draw large TX bursts. Never power the module from the ESP32's 3.3 V LDO. |
| Level shifting | **TXB0108** or 2× TXS0104 (1.8 V ⇄ 3.3 V) | Quectel UART/PCM/control I/O are 1.8 V. The ESP32-S3 is 3.3 V. Don't use the TXB parts on lines with pull-ups. Use a MOSFET shifter for those. |
| PWRKEY driver | NPN (MMBT3904) or N-MOSFET (2N7002) + 10 k base/gate resistor | ESP32 GPIO high → PWRKEY pulled low. |
| Supply | 5 V / 3 A USB-C PD or 1S Li-ion (3.7 V) + charger | A Li-ion cell can feed the module directly through a load switch. |

## Wiring (defaults in `menuconfig → 5G Bridge`)

```
ESP32-S3            level shifter          5G module (1.8 V I/O)
---------           -------------          ----------------------
GPIO17 (TX)  ─────────── A1 ⇄ B1 ────────── MAIN_RXD
GPIO18 (RX)  ─────────── A2 ⇄ B2 ────────── MAIN_TXD
GPIO15 (RTS) ─────────── A3 ⇄ B3 ────────── MAIN_CTS
GPIO16 (CTS) ─────────── A4 ⇄ B4 ────────── MAIN_RTS
GPIO4  (BCLK) ────────── A5 ⇄ B5 ────────── PCM_CLK
GPIO5  (WS)   ────────── A6 ⇄ B6 ────────── PCM_SYNC
GPIO6  (DIN)  ────────── A7 ⇄ B7 ────────── PCM_DOUT
GPIO7  (DOUT) ────────── A8 ⇄ B8 ────────── PCM_DIN
GPIO21 ──[10k]── NPN base;  NPN collector ── PWRKEY;  emitter ── GND
3V3 ── VCCA                 VCCB ── module VDD_EXT (1.8 V reference output)
GND ─────────────────────────────────────── GND (star ground at the module's VBAT cap)
```

Things to get right:

- **Flow control is mandatory.** CMUX + PPP at 921600 baud drops data without RTS/CTS.
- **VBAT routing:** use short, wide traces, put the bulk cap right at the module, and add a 100 nF + 33 pF decoupling pair per VBAT pin.
- **USB port on the module:** bring it to a USB-C connector anyway. You'll need it for firmware updates (QFlash) and for debugging with QCOM/QNavigator.
- **SIM:** add ESD protection (e.g., SMF05C) on SIM_CLK/IO/RST/VDD if you're laying out your own socket.

## First power-up checklist

1. Power the module alone. Connect its USB to a PC and confirm `AT` / `AT+CPIN?` / `AT+COPS?` on the AT port.
2. Enable IMS: `AT+QCFG="ims",1`. Check `AT+QMBNCFG="list"` and make sure an MBN matching your carrier is selected (or `AT+QMBNCFG="AutoSel",1`).
3. Configure PCM: run the `CONFIG_BRIDGE_PCM_DAI_CMD` value by hand and check `AT+QDAI?`.
4. Make a test call from the PC (`ATD<number>;`) and scope PCM_CLK/PCM_SYNC. You should see 256 kHz / 8 kHz for the 8 kHz default.
5. Only then connect the ESP32.

## Throughput expectations

The ESP32 is the bottleneck, not the 5G link:

| Path | Realistic throughput |
|---|---|
| UART PPP @ 921600 | ~0.7 Mbit/s |
| UART PPP @ 3 Mbaud | ~2.5 Mbit/s |
| ESP32-S3 Wi-Fi AP NAT | ~15–20 Mbit/s ceiling |

That's enough for messaging, maps, email and music, but not 5G speeds. See [architecture.md](architecture.md#upgrade-paths) for faster designs.
