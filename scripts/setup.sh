#!/usr/bin/env bash
# setup.sh – Run ONCE with iPhone connected via USB to pair and enable WiFi sync.
# Usage: docker exec -it iphone-backup /usr/local/bin/setup.sh

set -euo pipefail

echo "========================================"
echo "  iPhone Backup – First-Time Setup"
echo "========================================"
echo ""
echo "1. Make sure your iPhone is plugged in via USB and unlocked."
echo "   Press ENTER when ready..."
read -r

echo ""
echo "Checking for connected devices..."
UDIDS=$(idevice_id -l 2>/dev/null || true)

if [[ -z "$UDIDS" ]]; then
    echo "ERROR: No device detected. Check USB cable and unlock your iPhone."
    exit 1
fi

echo ""
echo "Found device(s):"
echo "$UDIDS"
echo ""

while IFS= read -r UDID; do
    [[ -z "$UDID" ]] && continue
    NAME=$(ideviceinfo -u "$UDID" -k DeviceName 2>/dev/null || echo "$UDID")
    echo "--- Setting up: $NAME ($UDID)"

    echo "  [1/3] Pairing device (accept 'Trust' dialog on iPhone if prompted)..."
    idevicepair -u "$UDID" pair
    sleep 2

    echo "  [2/3] Verifying pairing..."
    idevicepair -u "$UDID" validate && echo "  ✓ Paired OK" || {
        echo "  Pairing failed. Make sure you tapped 'Trust' on the iPhone."
        exit 1
    }

    echo "  [3/3] Enabling WiFi sync..."
    pymobiledevice3 lockdown wifi-connections on -u "$UDID" && echo "  ✓ WiFi sync enabled" || \
        echo "  WARNING: WiFi sync could not be enabled automatically. Enable it manually in Finder."

    echo ""
    echo "  ✓ Setup complete for $NAME. You can now unplug the cable."
    echo ""
done <<< "$UDIDS"

echo "All devices configured. Backups will run automatically on the cron schedule."
