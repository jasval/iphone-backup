#!/usr/bin/env bash
# setup.sh – Run on your Mac (NOT inside Docker).
# Copies iOS pairing .plist files to the NAS over SCP so the container
# can discover and back up your devices over Wi-Fi or Tailscale.
#
# Usage: bash scripts/setup.sh

set -euo pipefail

LOCKDOWN_SRC="/var/db/lockdown"

echo "========================================"
echo "  iPhone Backup – Pairing File Sync"
echo "========================================"
echo ""

PLISTS=("$LOCKDOWN_SRC"/*.plist)
if [[ ! -e "${PLISTS[0]}" ]]; then
    echo "ERROR: No .plist files found at $LOCKDOWN_SRC"
    echo ""
    echo "  Make sure you have trusted this Mac on each iPhone:"
    echo "  1. Plug iPhone into Mac via USB"
    echo "  2. Tap 'Trust This Computer' on the iPhone"
    echo "  3. Re-run this script"
    exit 1
fi

echo "Found ${#PLISTS[@]} pairing file(s):"
for f in "${PLISTS[@]}"; do
    echo "  $(basename "$f")"
done
echo ""

read -rp "NAS SSH target (e.g. admin@192.168.1.100): " NAS_TARGET
read -rp "Lockdown path on NAS [/volume1/iphone-backups/.lockdown]: " NAS_PATH
NAS_PATH="${NAS_PATH:-/volume1/iphone-backups/.lockdown}"

echo ""
echo "Creating target directory on NAS..."
ssh "$NAS_TARGET" "mkdir -p '$NAS_PATH'"

echo "Copying pairing files..."
COPIED=0
FAILED=0
for f in "${PLISTS[@]}"; do
    UDID=$(basename "$f" .plist)
    if scp "$f" "${NAS_TARGET}:${NAS_PATH}/"; then
        echo "  ✓ $UDID"
        COPIED=$((COPIED + 1))
    else
        echo "  ✗ $UDID  (scp failed)"
        FAILED=$((FAILED + 1))
    fi
done

echo ""
echo "Done. $COPIED file(s) copied, $FAILED failed."
echo ""
echo "Verify the container sees the pairing files:"
echo "  docker exec iphone-backup ls /var/lib/lockdown/"
echo ""
echo "Run a manual backup to confirm device discovery:"
echo "  docker exec iphone-backup /usr/local/bin/ibackup.sh"
