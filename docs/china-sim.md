# Testing with a China SIM card

China Mobile, China Unicom, China Telecom and China Broadnet SIMs work with the bridge, with some caveats about **which 5G module** you use and **where** you test.

## 1. Pick the right module first

The firmware was designed around the **Quectel RG520N**. Quectel's specification lists its variants (EU, NA, EB, GT, LA) as covering *"EMEA/APAC, excluding China/Japan"*, and **none supports band n79** (4.9 GHz, used by China Mobile).

5G bands of the RG520N-EU/EB against the Chinese carriers:

| Carrier | Main 5G bands | RG520N-EU/EB |
|---|---|---|
| China Mobile | n41 (2.6 GHz), **n79** (4.9 GHz), n28 (700 MHz) | n41 ✅, n28 ✅, **n79 ❌** |
| China Unicom | n78 (3.5 GHz), n1 (2.1 GHz) | ✅ |
| China Telecom | n78 (3.5 GHz), n1 (2.1 GHz) | ✅ |
| China Broadnet | n28 (700 MHz), n79 | n28 ✅, n79 ❌ |

So:

| Where you test | Recommendation |
|---|---|
| **In mainland China, long term** | Use a **China-certified module** (Quectel "-CN" parts, e.g. the RG500U-CN / RG200U-CN LGA modules or the RM500U-CN M.2 card, which list n1/n28/n41/n77/n78/n79). These carry the Chinese certifications (CCC/SRRC/CTA) that carriers and regulations expect. Check that its evaluation board breaks out **UART + PCM** (see the shopping list). |
| **In mainland China, short bench test with an RG520N-EU/EB** | It can register, and **China Unicom or China Telecom** SIMs match best (n78). China Mobile 5G works on n41 only. The module isn't certified for China, so carriers may block VoLTE for its IMEI, and radio-equipment rules apply. Keep it to short bench tests. |
| **Outside China, with a Chinese SIM roaming** | Use the RG520N variant for *your* region. Turn on **international data roaming** for the SIM. SMS and data usually work; **VoLTE calls often don't work while roaming**. That depends on the carrier and country, not on the bridge. |

> The "-CN" modules use a UNISOC chipset. The standard AT commands the firmware relies on (`AT+CLCC`, `AT+CMGR`, `AT+CNMI`, `AT+COPS`, `AT+CSQ`, `AT+C5GREG`) are the same. The Quectel/Qualcomm-specific ones (`AT+QMBNCFG`, `AT+QCFG="ims"`, `AT+QDAI` for call audio) may differ. The firmware ignores errors from them, but call audio and VoLTE setup may need the equivalent commands from that module's AT manual.

## 2. Prepare the SIM

1. The SIM must be **real-name registered** with data enabled (normal for any Chinese SIM).
2. Turn on **VoLTE / 高清通话** for the line in the carrier's app or by contacting the carrier.
3. Put it in a phone and check: a call, an SMS, and mobile data with Wi-Fi off.
4. Remove the SIM PIN, or build with `BRIDGE_SIM_PIN=1234`.

## 3. APN: automatic

Since this version the firmware reads the SIM's IMSI (`AT+CIMI`, starting with **460**) and picks the APN itself:

| Carrier | IMSI prefix | APN used |
|---|---|---|
| China Mobile | 46000 / 02 / 04 / 07 / 08 | `cmnet` |
| China Unicom | 46001 / 06 / 09 | `3gnet` |
| China Telecom | 46003 / 05 / 11 | `ctnet` |
| China Broadnet | 46015 | `cbnet` |

The serial log shows e.g. `SIM carrier: China Unicom (APN "3gnet")`. If you build with your own `BRIDGE_APN` (Rust) or set `CONFIG_BRIDGE_MODEM_APN` (C) to anything other than `internet`, your value is always used instead. Your plan might need something else (some IoT or enterprise SIMs do).

## 4. Check the modem (step 3 of the build guide)

In the modem's AT terminal:

| Command | Expect |
|---|---|
| `AT+CIMI` | `460xx…` (your carrier code) |
| `AT+COPS?` | `CHINA MOBILE`, `CHN-UNICOM`, `CHN-CT` or similar |
| `AT+QENG="servingcell"` | `NR5G-SA` / `NR5G-NSA` / `LTE` |
| `AT+QMBNCFG="AutoSel",1` then `AT+QMBNCFG="List"` | The profile marked active should name your carrier (e.g. a CMCC / CU / CT profile) |
| `AT+QCFG="ims",1` | `OK` |
| `AT+CIREG?` | `+CIREG: 0,1` means IMS (VoLTE) is registered, so calls should work |
| `ATD1xxxxxxxxxx;` | Rings the phone you dialled (11-digit mobile number; `+86` also works) |

If `AT+CIREG?` keeps showing `0,0` while data works, the carrier isn't accepting VoLTE from this module or SIM. Check that VoLTE is enabled on the line. If you're using an RG520N in China, this is the certification limit described above.

## 5. Chinese text messages

SMS in Chinese works out of the box. The bridge switches to UCS2 encoding automatically. One SMS holds up to **70 Chinese characters**; longer messages are rejected for now (multi-part SMS is on the roadmap). Emoji count as two characters.

## 6. Building the firmware from inside mainland China

Downloads from GitHub and the Rust servers can be slow. These mirrors help:

```bash
# Rust toolchain (USTC mirror)
export RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static
export RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup

# ESP-IDF tool downloads (Espressif's China mirror)
export IDF_GITHUB_ASSETS="dl.espressif.cn/github_assets"
```

For crates, add a mirror to `~/.cargo/config.toml` (e.g. the USTC or rsproxy sparse index). If cloning ESP-IDF itself is slow, esp-idf-sys can be pointed at a mirror with the `ESP_IDF_REPOSITORY` environment variable (e.g. Espressif's Gitee mirror).

Alternatively, skip building and flash the ready-made images from the [Releases](https://github.com/jspantheonlab/esp32-5g-bridge/releases) page. From the release that includes this change, they pick the Chinese APN automatically.
