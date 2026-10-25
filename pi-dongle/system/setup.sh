#!/bin/bash
# Installs the 5G Bridge USB-C dongle on Raspberry Pi OS (Bookworm, 64-bit).
# Run as root from the directory that contains this script and the bridge5g-dongle binary
# (the release tarball), or from pi-dongle/system in a checkout after `cargo build --release`.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
[ "$(id -u)" = 0 ] || { echo "Please run with sudo." >&2; exit 1; }

BIN="$HERE/bridge5g-dongle"
[ -x "$BIN" ] || BIN="$HERE/../target/release/bridge5g-dongle"
[ -x "$BIN" ] || { echo "bridge5g-dongle binary not found next to setup.sh." >&2; exit 1; }

model=$(tr -d '\0' </proc/device-tree/model 2>/dev/null || echo unknown)
case "$model" in
    *"Raspberry Pi 4"* | *"Raspberry Pi 5"* | *"Compute Module 4"* | *"Compute Module 5"*) ;;
    *) echo "Warning: designed for Raspberry Pi 4/5 (found: $model)." ;;
esac

echo "1/5 USB-C port in device mode"
CONFIG=/boot/firmware/config.txt
[ -f "$CONFIG" ] || CONFIG=/boot/config.txt
grep -q '^dtoverlay=dwc2,dr_mode=peripheral' "$CONFIG" || printf '\n[all]\ndtoverlay=dwc2,dr_mode=peripheral\n' >>"$CONFIG"
for module in dwc2 libcomposite; do
    grep -qx "$module" /etc/modules || echo "$module" >>/etc/modules
done

echo "2/5 Packages (call audio tools, DHCP for the iPhone)"
apt-get update -qq
apt-get install -y -qq alsa-utils dnsmasq-base

echo "3/5 Disabling ModemManager (the daemon drives the modem directly)"
systemctl disable --now ModemManager.service 2>/dev/null || true

echo "4/5 Installing files"
install -Dm755 "$BIN" /usr/local/bin/bridge5g-dongle
install -Dm755 "$HERE/usb-gadget.sh" /usr/local/lib/bridge5g/usb-gadget.sh
install -Dm755 "$HERE/setup-modem.sh" /usr/local/bin/bridge5g-setup-modem
install -Dm644 "$HERE/70-bridge5g-usb.link" /etc/systemd/network/70-bridge5g-usb.link
install -Dm644 "$HERE/bridge5g-gadget.service" /etc/systemd/system/bridge5g-gadget.service
install -Dm644 "$HERE/bridge5g.service" /etc/systemd/system/bridge5g.service
[ -f /etc/default/bridge5g ] || install -Dm644 "$HERE/bridge5g.env" /etc/default/bridge5g

echo "5/5 Network: address + shared 5G connection for the iPhone on USB-C"
if ! nmcli -t -f NAME con show | grep -qx bridge5g-usb; then
    nmcli con add type ethernet ifname iphone0 con-name bridge5g-usb \
        ipv4.method shared ipv4.addresses 192.168.7.1/24 ipv6.method disabled \
        connection.autoconnect yes connection.autoconnect-priority 10 >/dev/null
fi

systemctl daemon-reload
systemctl enable bridge5g-gadget.service bridge5g.service >/dev/null

cat <<'EOF'

Installed. Next steps:
  1. With the modem plugged in:  sudo bridge5g-setup-modem   (one time)
  2. Reboot:                      sudo reboot
  3. Connect the iPhone to the Pi's USB-C port (power the Pi separately, see docs/pi-dongle.md)
  4. Watch the log:               journalctl -fu bridge5g
Settings live in /etc/default/bridge5g.
EOF
