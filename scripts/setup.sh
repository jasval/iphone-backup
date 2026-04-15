#!/usr/bin/env bash
# setup.sh – Run on your Mac (NOT inside Docker).
# Pairs your iPhone via USB and copies the pairing record to the NAS.
# Requires: pip3 install pymobiledevice3
#
# Usage: bash scripts/setup.sh

set -euo pipefail

# Check pymobiledevice3 is installed
if ! python3 -c "import pymobiledevice3" 2>/dev/null; then
    echo "Installing pymobiledevice3..."
    pip3 install pymobiledevice3
fi

echo "========================================"
echo "  iPhone Backup – Pairing File Sync"
echo "========================================"
echo ""
echo "Make sure your iPhone is plugged in via USB and unlocked."
echo ""

read -rp "NAS SSH target (e.g. admin@192.168.1.100): " NAS_TARGET
read -rp "Lockdown path on NAS [/volume1/ios-backups/.lockdown]: " NAS_PATH
NAS_PATH="${NAS_PATH:-/volume1/ios-backups/.lockdown}"

# Use pymobiledevice3 Python API to pair and save record to a writable temp folder
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo ""
echo "Pairing with iPhone (tap 'Trust' on the device if prompted)..."

python3 - "$TMPDIR" <<'PYEOF'
import sys
import asyncio
from pathlib import Path
from pymobiledevice3.lockdown import create_using_usbmux

output_dir = Path(sys.argv[1])

async def pair():
    import plistlib
    lockdown = await create_using_usbmux(pairing_records_cache_folder=output_dir)
    record_path = output_dir / f"{lockdown.udid}.plist"
    with open(record_path, 'wb') as f:
        plistlib.dump(lockdown.pair_record, f)
    print(f"  ✓ Paired: {lockdown.display_name} ({lockdown.udid})")

try:
    asyncio.run(pair())
except Exception as e:
    print(f"  ERROR: {e}")
    print("")
    print("  Make sure:")
    print("  - iPhone is plugged in via USB")
    print("  - iPhone is unlocked")
    print("  - You tapped 'Trust This Computer' on the iPhone")
    sys.exit(1)
PYEOF

PLISTS=("$TMPDIR"/*.plist)
if [[ ! -e "${PLISTS[0]}" ]]; then
    echo "ERROR: No pairing record was created."
    exit 1
fi

echo ""
echo "Copying pairing record(s) to NAS..."
ssh "$NAS_TARGET" "mkdir -p '$NAS_PATH'"

COPIED=0
FAILED=0
for f in "${PLISTS[@]}"; do
    UDID=$(basename "$f" .plist)
    if scp -O "$f" "${NAS_TARGET}:${NAS_PATH}/"; then
        echo "  ✓ $UDID"
        COPIED=$((COPIED + 1))
    else
        echo "  ✗ $UDID  (scp failed)"
        FAILED=$((FAILED + 1))
    fi
done

echo ""
echo "Done. $COPIED file(s) copied, $FAILED failed."

if [[ $COPIED -gt 0 ]]; then
    echo ""
    echo "Verify the container sees the pairing files:"
    echo "  docker exec iphone-backup ls /var/lib/lockdown/"
    echo ""
    # Offer to trigger a backup immediately
    read -rp "Run a backup now? [y/N]: " DO_BACKUP
    if [[ "${DO_BACKUP,,}" == "y" ]]; then
        SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
        NAS_MOUNT="${NAS_MOUNT:-/Volumes/ios-backups}"
        if ls "$NAS_MOUNT" &>/dev/null 2>&1; then
            echo ""
            NAS_MOUNT="$NAS_MOUNT" bash "$SCRIPT_DIR/mac-backup.sh"
        else
            echo "NAS not mounted at $NAS_MOUNT — run the backup manually:"
            echo "  bash scripts/mac-backup.sh"
        fi
    fi
fi
