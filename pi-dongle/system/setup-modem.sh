#!/bin/bash
# One-time modem configuration (installed as `bridge5g-setup-modem`):
# switch the Quectel modem's USB networking to ECM, where the modem dials the data connection
# itself and appears to Linux as a plain USB Ethernet device. Then restart the modem.
set -euo pipefail

PORT="${1:-auto}"
if [ "$PORT" = auto ]; then
    PORT=$(ls /dev/serial/by-id/*[Qq]uectel*if02* 2>/dev/null | head -n1 || true)
    PORT="${PORT:-/dev/ttyUSB2}"
fi
[ -e "$PORT" ] || { echo "Modem AT port $PORT not found. Is the modem plugged in?" >&2; exit 1; }

# The daemon would compete for the port.
systemctl stop bridge5g.service 2>/dev/null || true

stty -F "$PORT" raw -echo
exec 3<>"$PORT"

at() {
    echo "> $1"
    printf '%s\r' "$1" >&3
    while IFS= read -r -t 5 line <&3; do
        line="${line%$'\r'}"
        [ -n "$line" ] && echo "  $line"
        case "$line" in OK | *ERROR*) return 0 ;; esac
    done
    echo "  (no answer)"
}

at "AT"
at "ATI"
at 'AT+QCFG="usbnet"'
at 'AT+QCFG="usbnet",1'
echo "Restarting the modem; it comes back in about 20 seconds."
at "AT+CFUN=1,1"
exec 3>&-

echo
echo "Done. After the modem restarts, 'ip link' should show a new USB network interface"
echo "and 'ip route' a default route through it. Then: sudo systemctl start bridge5g"
