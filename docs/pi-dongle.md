# Raspberry Pi USB-C dongle prototype

A 5G "backpack" for USB-C iPhones (iPhone 15 and newer). The iPhone sees the dongle as a wired **Ethernet adapter** and gets 5G data over the cable, with no app needed for data. The Bridge5G app adds **calls and SMS** over the same cable.

```
iPhone ──USB-C──► [USB-C splitter] ──► Raspberry Pi 4/5 (USB gadget: "Ethernet adapter")
                        ▲                     │  bridge5g-dongle daemon: calls, SMS, status
                   power supply               └──USB 3──► M.2 5G module on a USB adapter (own power)
```

This prototype answers three questions before anyone designs a battery pack:
1. Does the iPhone accept the dongle as Ethernet and route its internet through it?
2. Does the app reach the dongle over the cable for calls and SMS?
3. How much power does it really draw?

> The iPhone can supply only about 4.5 W to accessories, and a 5G modem bursts far above that. So the prototype is **powered separately**; a real product needs its own battery. The iPhone powers nothing here.

## Shopping list

Prices are rough; compare sellers.

| # | Item | Taobao search (中文) | Elsewhere | Approx. |
|---|---|---|---|---|
| 1 | **Raspberry Pi 4 Model B, 2 GB** (a Pi 5 also works, but needs a 5 V/5 A supply) | `树莓派4B 2GB` | Raspberry Pi resellers | ¥300–400 / $45 |
| 2 | **5G M.2 module**: in mainland China **RM500U-CN**; elsewhere **RM520N-GL** | `移远 RM500U-CN M.2` / `移远 RM520N-GL` | Sixfab, Waveshare, Techship | ¥500–1,000 / $240–290 |
| 3 | **M.2 to USB 3.0 adapter board for 5G modules**, with **SIM slot** and **its own power input** | `5G M.2转USB3.0 开发板 SIM卡槽` | "5G M.2 to USB 3.0 adapter" | ¥100–250 / $30–60 |
| 4 | **4× 5G antennas** for M.2 modules (MHF4/IPEX4 connector) | `5G天线 IPEX4 MHF4 全频段` | — | ¥10–30 each |
| 5 | **USB-C power/data splitter**: powers the Pi from a supply while the data lines go to the iPhone | `USB-C 数据电源分离器` / `USB-C PWR splitter 树莓派` | "USB-C PWR splitter Raspberry Pi" | ¥30–80 / $10–20 |
| 6 | Raspberry Pi power supply (Pi 4: 5 V/3 A USB-C) | `树莓派4B 官方电源` | Official Raspberry Pi PSU | ¥50–80 |
| 7 | Power supply for the M.2 adapter board (per its spec, often 12 V/2 A or 5 V/3 A) | `12V2A 电源适配器 DC5.5` | — | ¥20–40 |
| 8 | microSD card, 32 GB, A1/A2 | `TF卡 32G A1` | — | ¥30–50 |
| 9 | USB-C to USB-C data cable | `C to C 数据线` | — | ¥15–30 |
| 10 | **USB-C power meter**, to answer question 3 | `USB-C 功率计 电流表` | "USB-C power meter" | ¥30–60 |
| 11 | *(Optional)* Pi case with heatsink | `树莓派4B 散热外壳` | — | ¥30–60 |
| 12 | SIM card with data, SMS and **VoLTE** (real-name, from the carrier) | — | — | plan |

**Rough total:** about ¥1,150–2,200 in China, or $400–550 elsewhere.

**Ask the seller (问客服) about the adapter board:** 是否支持移远 RM500U-CN / RM520N？有独立供电接口吗？USB 3.0？ (Does it support the RM500U-CN / RM520N? Does it have a separate power input? Is it USB 3.0?)

### The one open question: call audio over USB

With no extra wiring, call audio must come from the modem as a **USB sound card (UAC)**. Quectel documents USB audio for its 5G M.2 series, but **confirm it for your exact module and firmware**. It's unverified for the RM500U-CN. Data, SMS and call control (ringing, answering, hanging up) work without it. Only the voice itself depends on it.

## Setup

### 1. Prepare the Pi
1. Flash **Raspberry Pi OS Lite (64-bit, Bookworm)** with Raspberry Pi Imager. In its settings, enable **SSH** and your **Wi-Fi** (used only for setup).
2. Boot the Pi and SSH in.

### 2. Assemble the modem
1. Power off everything. Put the module into the M.2 adapter, attach **all four antennas**, and insert the SIM.
2. Connect the adapter's USB 3.0 port to one of the Pi's **blue** USB 3 ports, and give the adapter its **own power supply**.
3. On the Pi, `lsusb` should show a Quectel device (vendor `2c7c`).

### 3. Install the dongle software
From the [Releases](https://github.com/jspantheonlab/esp32-5g-bridge/releases) page, download `bridge5g-dongle-aarch64-<version>.tar.gz` to the Pi, then:

```bash
tar xf bridge5g-dongle-aarch64-*.tar.gz
cd bridge5g-dongle-aarch64-*/
sudo ./setup.sh
```

The setup script:
- puts the USB-C port in device mode,
- installs `alsa-utils` and `dnsmasq-base`,
- disables ModemManager,
- installs the daemon and services,
- creates a network that gives the iPhone an address (192.168.7.x) and shares the modem's 5G connection with it.

Settings live in `/etc/default/bridge5g` (APN, SIM PIN, audio device…).

### 4. One-time modem setup
```bash
sudo bridge5g-setup-modem
```
This switches the modem to **ECM** mode: the modem dials the data connection itself and appears as a USB network interface. It then restarts the modem. Enable **USB audio** too, following your module's AT manual (the exact command differs between modules and firmware).

### 5. Reboot and connect
1. `sudo reboot`
2. Power the Pi through the **splitter's power input**. Connect the splitter's **data side to the iPhone** with the USB-C cable.
3. Unlock the iPhone. If it asks **"Allow accessory to connect?"**, tap Allow.

The Pi's Wi-Fi can stay on for SSH. The modem's interface gets a lower route metric, so the iPhone's traffic goes out over 5G. Check with `ip route`: the default route should go through the modem's interface.

## Test checklist

| # | Check | How | Pass |
|---|---|---|---|
| 1 | iPhone sees Ethernet | iPhone **Settings → Ethernet** appears, with an address 192.168.7.x | ✅ |
| 1b | Internet over 5G | Turn off the iPhone's Wi-Fi **and** mobile data, then open a website. On the Pi, `journalctl -fu bridge5g` shows the modem registered. | Pages load |
| 2 | App over the cable | Open Bridge5G. The Status tab shows **Connection: USB-C dongle**, the operator and the 5G/LTE network. | ✅ |
| 2b | SMS | Text the dongle's SIM from another phone; it appears in Messages. Reply from the app. | Both ways |
| 2c | Calls | Call the dongle's number: CallKit rings on the iPhone; answer and talk. | Two-way audio (needs USB audio, see above) |
| 3 | Power | USB-C power meter between the Pi's power supply and the splitter, plus the adapter board's own supply: note idle / browsing / calling. | Write the numbers down |

## Troubleshooting

| Symptom | Fix |
|---|---|
| No **Ethernet** entry on the iPhone | `ls /sys/class/udc` must list a controller (else config.txt is missing `dtoverlay=dwc2,dr_mode=peripheral`; reboot). Try `BRIDGE_USB_FUNCTION=ncm` in `/etc/default/bridge5g`, then reboot. Unlock the iPhone and allow the accessory. |
| Ethernet shows but no internet | `ip route` on the Pi: is the default route via the modem? `journalctl -u bridge5g` should show `Registered`. Check the APN (`BRIDGE_APN`). |
| App doesn't switch to "USB-C dongle" | `sudo systemctl status bridge5g`; `ss -ltn` should show `192.168.7.1:5005`. Allow **Local Network** access for Bridge5G in iPhone Settings. |
| `Lost the modem's AT port` in the log | Normal after a modem restart. The service restarts by itself. |
| Calls ring but no audio | USB audio isn't enabled or supported: `arecord -l` should list the modem's sound card. Set `BRIDGE_AUDIO_DEVICE` and `BRIDGE_VOICE_ROUTE_CMD` to match your module. |
| Pi reboots under load | Power supply too weak: the Pi needs a proper 5 V/3 A supply through the splitter, and the modem board its own supply. |

## Developer notes

See [pi-dongle/README.md](../pi-dongle/README.md) for building the daemon (natively on the Pi or cross-compiled) and running its tests.
