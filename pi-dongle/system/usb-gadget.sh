#!/bin/bash
# Makes the Pi's USB-C port appear to the iPhone as a USB Ethernet adapter (Linux USB gadget
# via configfs). Run at boot by bridge5g-gadget.service; `usb-gadget.sh stop` removes it.
set -euo pipefail

GADGET=/sys/kernel/config/usb_gadget/bridge5g
# CDC-ECM is what iPhones/iPads accept from Linux gadgets; set BRIDGE_USB_FUNCTION=ncm to try NCM.
FUNCTION="${BRIDGE_USB_FUNCTION:-ecm}"
# Fixed, locally administered MACs so the interface is always recognised (see 70-bridge5g-usb.link).
PI_MAC="02:35:47:00:00:01"
IPHONE_MAC="02:35:47:00:00:02"

stop() {
    [ -d "$GADGET" ] || return 0
    echo "" > "$GADGET/UDC" 2>/dev/null || true
    rm -f "$GADGET"/configs/c.1/"$FUNCTION".usb0
    rmdir "$GADGET"/configs/c.1/strings/0x409 "$GADGET"/configs/c.1 \
          "$GADGET"/functions/"$FUNCTION".usb0 "$GADGET"/strings/0x409 "$GADGET" 2>/dev/null || true
}

start() {
    modprobe libcomposite
    mountpoint -q /sys/kernel/config || mount -t configfs none /sys/kernel/config
    stop

    mkdir -p "$GADGET"
    cd "$GADGET"
    echo 0x1d6b > idVendor        # Linux Foundation
    echo 0x0104 > idProduct       # Multifunction Composite Gadget
    echo 0x0100 > bcdDevice
    echo 0x0200 > bcdUSB
    echo 0xEF > bDeviceClass      # Miscellaneous (IAD), needed for composite devices on Apple hosts
    echo 0x02 > bDeviceSubClass
    echo 0x01 > bDeviceProtocol

    mkdir -p strings/0x409
    cat /proc/sys/kernel/random/boot_id | tr -d '-' | cut -c1-16 > strings/0x409/serialnumber
    echo "5G Bridge" > strings/0x409/manufacturer
    echo "5G Bridge USB-C Dongle" > strings/0x409/product

    mkdir -p configs/c.1/strings/0x409
    echo "5G network" > configs/c.1/strings/0x409/configuration
    echo 0xC0 > configs/c.1/bmAttributes   # self-powered: don't ask the iPhone for power
    echo 100 > configs/c.1/MaxPower

    mkdir -p functions/"$FUNCTION".usb0
    echo "$PI_MAC" > functions/"$FUNCTION".usb0/dev_addr
    echo "$IPHONE_MAC" > functions/"$FUNCTION".usb0/host_addr
    ln -s functions/"$FUNCTION".usb0 configs/c.1/

    udc=$(ls /sys/class/udc | head -n1)
    if [ -z "$udc" ]; then
        echo "No USB device controller found. Is 'dtoverlay=dwc2,dr_mode=peripheral' in config.txt?" >&2
        exit 1
    fi
    echo "$udc" > UDC
    echo "USB gadget ($FUNCTION) active on $udc"
}

case "${1:-start}" in
    start) start ;;
    stop) stop ;;
    *) echo "usage: $0 [start|stop]" >&2; exit 2 ;;
esac
