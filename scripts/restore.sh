#!/usr/bin/env bash
# restore.sh – Restore an iPhone/iPad backup to a device.
# Run on your Mac: bash scripts/restore.sh
#
# Prerequisites: brew install libimobiledevice
# The device must be on the same Wi-Fi as the Mac, or plugged in via USB.

set -uo pipefail

# Read backup_path from config file
CONFIG_FILE="$HOME/.config/iphone-backup/config.toml"
if [[ -f "$CONFIG_FILE" ]]; then
    BACKUP_PATH=$(grep 'backup_path' "$CONFIG_FILE" | sed 's/.*= *"\(.*\)"/\1/')
else
    BACKUP_PATH="$HOME/Backups/iOS"
fi

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

# ── Check backup path exists ──────────────────────────────────────────────────
if ! ls "$BACKUP_PATH" &>/dev/null 2>&1; then
    echo "ERROR: Backup path not accessible: $BACKUP_PATH"
    echo "  Check your config: $CONFIG_FILE"
    exit 1
fi

# ── List available backups ────────────────────────────────────────────────────
echo "Available backups in $BACKUP_PATH:"
echo ""

BACKUPS=()
i=1
for dir in "$BACKUP_PATH"/*/; do
    name=$(basename "$dir")
    [[ "$name" == .* ]] && continue
    if ls "$dir"/Manifest.plist &>/dev/null 2>&1 || \
       ls "$dir"/*.mdbackup   &>/dev/null 2>&1 || \
       ls "$dir"/Manifest.mbdb &>/dev/null 2>&1; then
        SIZE=$(du -sh "$dir" 2>/dev/null | cut -f1 || echo "?")
        STATUS_FILE="$BACKUP_PATH/.status/${name}.json"
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
    echo "  No backups found in $BACKUP_PATH"
    echo "  Run a backup first: iphone-backup backup"
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
echo "Detecting devices (plug in via USB or ensure Wi-Fi sync is on)..."
echo ""

UDIDS=$(idevice_id -l 2>/dev/null || true)

if [[ -z "$UDIDS" ]]; then
    echo "ERROR: No devices found."
    echo "  - Plug iPhone in via USB and unlock it"
    echo "  - Or enable Wi-Fi sync: Settings → General → VPN & Device Management → Connect via Wi-Fi"
    exit 1
fi

DEVICE_COUNT=$(echo "$UDIDS" | wc -l | tr -d ' ')

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
echo "  [1] Standard  – apps + data (recommended)"
echo "  [2] Full      – includes system settings (same device, same iOS version only)"
echo ""
read -rp "Select [1-2, default 1]: " RESTORE_TYPE
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
echo "Restoring from $BACKUP_DIR ..."
echo "Keep the device connected and unlocked throughout."
echo ""

if [[ "$RESTORE_TYPE" == "2" ]]; then
    idevicebackup2 -u "$TARGET_UDID" restore --system --settings --interactive "$BACKUP_DIR"
else
    idevicebackup2 -u "$TARGET_UDID" restore --interactive "$BACKUP_DIR"
fi

echo ""
echo "Restore complete. The device will restart."
