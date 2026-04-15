#!/usr/bin/env bash
# setup.sh – Pair your iPhone with this Mac via USB.
# Run once per device; pairing is stored by macOS automatically.
# Requires: pip3 install pymobiledevice3
#
# Usage: bash scripts/setup.sh

set -euo pipefail

if ! python3 -c "import pymobiledevice3" 2>/dev/null; then
    echo "Installing pymobiledevice3..."
    pip3 install pymobiledevice3
fi

echo "========================================"
echo "  iPhone Backup – Device Pairing"
echo "========================================"
echo ""
echo "Plug your iPhone into this Mac via USB and unlock it."
echo ""

read -rp "Press Enter when ready..."

echo ""
echo "Pairing (tap 'Trust This Computer' on the device if prompted)..."

python3 - <<'PYEOF'
import sys
import asyncio
import plistlib
from pathlib import Path
from pymobiledevice3.lockdown import create_using_usbmux

async def pair():
    lockdown = await create_using_usbmux()
    print(f"  ✓ Paired: {lockdown.display_name} ({lockdown.udid})")
    print(f"    iOS {lockdown.product_version}  ·  {lockdown.product_type}")

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

echo ""
echo "Pairing complete. You can unplug the iPhone."
echo "Enable Wi-Fi sync so backups work without a cable:"
echo "  Settings → General → VPN & Device Management → (this Mac) → Connect via Wi-Fi"
echo ""

read -rp "Run a backup now? [y/N]: " DO_BACKUP
if [[ "${DO_BACKUP,,}" == "y" ]]; then
    iphone-backup backup
fi
