#!/usr/bin/env bash
# restore.sh – Restore an iPhone/iPad backup from the NAS to a device.
# Run on your Mac: bash scripts/restore.sh
#
# Prerequisites: brew install libimobiledevice
# The device must be on the same Wi-Fi as the Mac, or plugged in via USB.

set -uo pipefail

NAS_MOUNT="${NAS_MOUNT:-/Volumes/ios-backups}"

echo ""
echo "╔══════════════════════════════════════╗"
echo "║   iPhone Backup – Restore            ║"
echo "╚══════════════════════════════════════╝"
echo ""

# ── Check dependencies ────────────────────────────────────────────────────────
if ! command -v idevicebackup2 &>/dev/null; then
    echo "ERROR: libimobiledevice not found. Install with: brew install libimobiledevice"
    exit 1
fi

# ── Check NAS is mounted ──────────────────────────────────────────────────────
if ! ls "$NAS_MOUNT" &>/dev/null 2>&1; then
    echo "ERROR: NAS not mounted at $NAS_MOUNT"
    echo "  Mount the NAS share in Finder first, or set NAS_MOUNT=/Volumes/your-share"
    exit 1
fi

# ── List available backups ────────────────────────────────────────────────────
echo "Available backups on NAS:"
echo ""

BACKUPS=()
i=1
for dir in "$NAS_MOUNT"/*/; do
    name=$(basename "$dir")
    # Skip hidden dirs like .lockdown, .status
    [[ "$name" == .* ]] && continue
    # Must contain a backup manifest
    if ls "$dir"/*.mdbackup &>/dev/null 2>&1 || ls "$dir"/*.mdinfo &>/dev/null 2>&1 || \
       ls "$dir"/Manifest.plist &>/dev/null 2>&1 || ls "$dir"/Manifest.mbdb &>/dev/null 2>&1; then
        SIZE=$(du -sh "$dir" 2>/dev/null | cut -f1 || echo "?")
        # Try to read last backup time from status JSON
        STATUS_FILE="$NAS_MOUNT/.status/${name}.json"
        LAST_RUN="unknown"
        IOS_VER=""
        if [[ -f "$STATUS_FILE" ]]; then
            LAST_RUN=$(jq -r '.last_run // "unknown"' "$STATUS_FILE" 2>/dev/null || echo "unknown")
            IOS_VER=$(jq -r '.ios // ""' "$STATUS_FILE" 2>/dev/null || echo "")
        fi
        echo "  [$i] $name"
        echo "      Size: $SIZE  |  Last backup: $LAST_RUN${IOS_VER:+  |  iOS $IOS_VER}"
        echo ""
        BACKUPS+=("$dir")
        i=$((i + 1))
    fi
done

if [[ ${#BACKUPS[@]} -eq 0 ]]; then
    echo "  No backups found at $NAS_MOUNT"
    echo "  Run a backup first: launchctl start com.user.iphone-backup"
    exit 1
fi

read -rp "Select backup to restore [1-${#BACKUPS[@]}]: " CHOICE
if ! [[ "$CHOICE" =~ ^[0-9]+$ ]] || [[ "$CHOICE" -lt 1 ]] || [[ "$CHOICE" -gt ${#BACKUPS[@]} ]]; then
    echo "Invalid choice."
    exit 1
fi

BACKUP_DIR="${BACKUPS[$((CHOICE - 1))]}"
BACKUP_NAME=$(basename "$BACKUP_DIR")
echo ""
echo "Selected: $BACKUP_NAME"

# ── Detect connected device ───────────────────────────────────────────────────
echo ""
echo "Detecting devices..."
echo "  Make sure the iPhone is connected via USB or on the same Wi-Fi."
echo "  If prompted on the device, tap 'Trust This Computer'."
echo ""

UDIDS=$(idevice_id -l 2>/dev/null || true)

if [[ -z "$UDIDS" ]]; then
    echo "ERROR: No devices found."
    echo "  - Plug iPhone in via USB, or enable Wi-Fi sync"
    echo "  - Unlock the device"
    exit 1
fi

# If multiple devices, let user pick
DEVICE_COUNT=$(echo "$UDIDS" | wc -l | tr -d ' ')
TARGET_UDID=""

if [[ "$DEVICE_COUNT" -eq 1 ]]; then
    TARGET_UDID="$UDIDS"
    NAME=$(ideviceinfo -u "$TARGET_UDID" -k DeviceName 2>/dev/null || echo "$TARGET_UDID")
    echo "  Found: $NAME ($TARGET_UDID)"
else
    echo "  Multiple devices found:"
    i=1
    UDID_LIST=()
    while IFS= read -r UDID; do
        [[ -z "$UDID" ]] && continue
        NAME=$(ideviceinfo -u "$UDID" -k DeviceName 2>/dev/null || echo "$UDID")
        IOS=$(ideviceinfo -u "$UDID" -k ProductVersion 2>/dev/null || echo "?")
        echo "  [$i] $NAME ($UDID)  iOS $IOS"
        UDID_LIST+=("$UDID")
        i=$((i + 1))
    done <<< "$UDIDS"
    echo ""
    read -rp "  Select device [1-${#UDID_LIST[@]}]: " DEV_CHOICE
    TARGET_UDID="${UDID_LIST[$((DEV_CHOICE - 1))]}"
fi

# ── Restore options ───────────────────────────────────────────────────────────
echo ""
echo "Restore options:"
echo "  [1] Standard restore (apps + data, recommended for same device)"
echo "  [2] Full restore including system settings (same device, same iOS version)"
echo ""
read -rp "Select restore type [1-2, default 1]: " RESTORE_TYPE
RESTORE_TYPE="${RESTORE_TYPE:-1}"

echo ""
echo "╔════════════════════════════════════════════════════════════════╗"
echo "║  WARNING: This will overwrite ALL data on the target device.  ║"
echo "╚════════════════════════════════════════════════════════════════╝"
echo ""
echo "  Backup:  $BACKUP_NAME"
echo "  Device:  $(ideviceinfo -u "$TARGET_UDID" -k DeviceName 2>/dev/null || echo "$TARGET_UDID")"
echo ""
read -rp "  Type 'yes' to confirm: " CONFIRM
if [[ "$CONFIRM" != "yes" ]]; then
    echo "Aborted."
    exit 0
fi

# ── Run restore ───────────────────────────────────────────────────────────────
echo ""
echo "Starting restore from $BACKUP_DIR ..."
echo "  Keep the device connected and unlocked throughout."
echo ""

case "$RESTORE_TYPE" in
    2)
        idevicebackup2 -u "$TARGET_UDID" restore \
            --system --settings --interactive \
            "$BACKUP_DIR"
        ;;
    *)
        idevicebackup2 -u "$TARGET_UDID" restore \
            --interactive \
            "$BACKUP_DIR"
        ;;
esac

echo ""
echo "Restore complete. The device will restart."
