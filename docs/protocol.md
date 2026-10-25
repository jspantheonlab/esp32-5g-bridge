# iPhone ⇄ ESP32 protocol

## 1. Control link: BLE GATT

| | UUID | Properties |
|---|---|---|
| Service | `A5E10001-5C0E-4B6A-9D3E-6B2F0C5A1E01` | primary, advertised |
| RX (phone → bridge) | `A5E10002-5C0E-4B6A-9D3E-6B2F0C5A1E01` | write, write-without-response, **encrypted** |
| TX (bridge → phone) | `A5E10003-5C0E-4B6A-9D3E-6B2F0C5A1E01` | notify, read, **encrypted** |

- Pairing uses LE Secure Connections with a passkey. The bridge has the "display" role and uses a fixed passkey (`CONFIG_BRIDGE_BLE_PASSKEY`); the user types it on the iPhone. Bonds persist in NVS.
- Framing: UTF-8 JSON, one object per line, terminated by `\n`. A line may span multiple writes/notifications (chunked to ATT MTU − 3).
- The bridge treats the link as *ready* once it's encrypted and the phone has subscribed to TX. Then it sends `status` and the current `call` state, and re-delivers every SMS still on the SIM.

### Bridge → phone

```jsonc
{"t":"status","sim":true,"reg":true,"data":true,"op":"T-Mobile","rat":"5G SA","rssi":-87,"ip":"10.12.0.4","ssid":"5G-Bridge","bat":82,"chg":true,"usb":true}
//   bat/chg/usb: battery %, charging, USB power present. Omitted on boards without a gauge/charger.
//   The bridge powers down at 3% on battery after sending {"t":"err","ctx":"power",...}.
{"t":"call","state":"incoming","num":"+15551234567","dir":"in"}
//   state: idle | incoming | dialing | alerting | active | held | waiting   ("idle" = call ended)
{"t":"sms","id":3,"from":"+15551234567","ts":"25/10/25,14:03:11+32","body":"Hello 👋"}
//   id = SIM storage index; ts = SCTS (offset in quarter hours)
{"t":"sms_sent","ref":"<client ref>","ok":true}
{"t":"sms_sent","ref":"<client ref>","ok":false,"err":"message longer than 160 characters"}
{"t":"wifi","ssid":"5G-Bridge","pass":"correct horse"}
//   reply to wifi_get, and to wifi_set once the new settings are live
{"t":"notice","msg":"Factory reset in 5 s. Release the bridge's button to cancel."}
//   informational message for the user (factory-reset countdown, cancel, restart)
{"t":"err","ctx":"dial","msg":"ESP_FAIL"}
{"t":"err","ctx":"wifi_set","msg":"password must be 8 to 63 characters"}
```

### Phone → bridge

```jsonc
{"t":"status"}                               // request a status message
{"t":"dial","num":"+15551234567"}            // digits, * # and a leading +
{"t":"answer"}
{"t":"hangup"}                               // also rejects an incoming call
{"t":"dtmf","d":"5"}
{"t":"sms_send","ref":"uuid","to":"+15551234567","body":"text"}
{"t":"sms_ack","id":3}                       // delete message from SIM after it's stored on the phone
{"t":"sms_sync"}                             // re-deliver all SIM messages
{"t":"wifi_get"}                             // read hotspot name + password
{"t":"wifi_set","ssid":"My Bridge","pass":"correct horse"}
```

`wifi_set` rules: SSID 1–32 bytes, password 8–63 printable ASCII characters (WPA2). The bridge validates, restarts the hotspot with the new settings, re-enables 5G routing, and saves them to NVS. Saved settings override the build-time defaults until a factory reset (hold the BOOT button for 10 s) or a flash erase. Every Wi-Fi client, including the phone, drops off and has to rejoin with the new password. The BLE link isn't affected, so the reply still arrives. The password only travels over the encrypted, passkey-bonded BLE link.

SMS limits (current firmware): a single segment only. That means 160 GSM-7 characters, or 70 UTF-16 units for anything else (emoji count as 2). Concatenated SMS needs PDU mode and is on the roadmap.

## 2. Call audio: UDP over the bridge's Wi-Fi

- Bridge listens on `192.168.4.1:5004` (`CONFIG_BRIDGE_AUDIO_UDP_PORT`).
- When CallKit activates audio, the phone sends at least one packet. The bridge learns the phone's address from it and starts sending audio back.
- Each packet carries 20 ms of audio:

```
offset size  field
0      2     magic "5G"
2      2     sequence number (u16, little-endian)
4      4     timestamp in samples (u32, little-endian)
8      N     PCM, mono, signed 16-bit little-endian
             N = 320 bytes @ 8 kHz, 640 bytes @ 16 kHz
```

- Packets with a header only (no payload) are keep-alives/hellos.
- The bridge paces itself from the modem's PCM clock and runs a 3-frame (60 ms) jitter buffer. The phone caps its playback queue at 8 frames.

Security note: audio is unencrypted UDP, protected only by the WPA2/WPA3 hotspot. Don't put untrusted clients on the bridge's Wi-Fi, or add SRTP (see roadmap).

## 3. Data: Wi-Fi hotspot

Standard WPA2/WPA3-PSK access point at `192.168.4.1/24` with DHCP. Clients get the carrier's DNS servers and are NATed (lwIP NAPT) onto the PPP uplink.
