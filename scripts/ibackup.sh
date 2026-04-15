#!/usr/bin/env bash
# ibackup.sh – incremental backup of all paired iPhones/iPads over Wi-Fi or Tailscale
# Devices are discovered via idevice_id -l (mDNS/Bonjour on LAN, or Tailscale tunnel).
# Pairing state is read from /var/lib/lockdown/ (mounted from NAS at container start).
# Writes per-device status JSON to /backups/.status/<DeviceName>.json

set -uo pipefail

BACKUP_ROOT="/backups"
STATUS_DIR="/backups/.status"
LOG="$STATUS_DIR/ibackup.log"
TAILSCALE_IPS="${IPHONE_TAILSCALE_IPS:-}"

mkdir -p "$STATUS_DIR"

log()  { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG"; }
fail() { log "ERROR: $*"; }

# ── Write device manifest so dashboard can discover status files ─────────────
write_manifest() {
    local NAMES=()
    for f in "$STATUS_DIR"/*.json; do
        [[ -f "$f" ]] || continue
        fname=$(basename "$f" .json)
        [[ "$fname" == "summary" || "$fname" == "manifest" ]] && continue
        NAMES+=("$fname")
    done
    printf '%s\n' "${NAMES[@]}" | jq -Rcs '{"devices": split("\n") | map(select(length>0))}' \
        > "$STATUS_DIR/manifest.json"
}

# ── Tailscale: not yet implemented ──────────────────────────────────────────
# IPHONE_TAILSCALE_IPS is reserved for future use. pymobiledevice3's remote
# start-tunnel does not support direct IP connections; Tailscale backup
# requires mDNS to be bridged or a tunneld-based approach. For now, LAN
# (mDNS/Bonjour) is the only supported discovery method.
if [[ -n "$TAILSCALE_IPS" ]]; then
    log "NOTE: IPHONE_TAILSCALE_IPS is set but Tailscale backup is not yet supported."
    log "      Devices will be discovered via LAN (mDNS) only."
fi

# ── Discover all available devices ──────────────────────────────────────────
log "Discovering devices..."
UDIDS=$(idevice_id -l 2>/dev/null || true)

if [[ -z "$UDIDS" ]]; then
    log "No devices found. Exiting."
    jq -n --arg ts "$(date -Iseconds)" '{last_run: $ts, status: "no_devices", total_devices: 0, failed: 0}' \
        > "$STATUS_DIR/summary.json"
    exit 0
fi

DEVICE_COUNT=0
FAILED_COUNT=0

while IFS= read -r UDID; do
    [[ -z "$UDID" ]] && continue
    DEVICE_COUNT=$((DEVICE_COUNT + 1))

    NAME=$(ideviceinfo -u "$UDID" -k DeviceName 2>/dev/null | tr ' ' '_' || echo "$UDID")
    MODEL=$(ideviceinfo -u "$UDID" -k ProductType 2>/dev/null || echo "Unknown")
    IOS=$(ideviceinfo -u "$UDID" -k ProductVersion 2>/dev/null || echo "Unknown")
    DEST="$BACKUP_ROOT/$NAME"

    mkdir -p "$DEST"
    log "--- Backing up: $NAME ($UDID) → $DEST"

    START=$(date +%s)
    if idevicebackup2 -u "$UDID" backup --full "$DEST" >> "$LOG" 2>&1; then
        END=$(date +%s)
        ELAPSED=$((END - START))
        SIZE=$(du -sh "$DEST" 2>/dev/null | cut -f1 || echo "unknown")
        STATUS="success"
        log "✓ $NAME backed up in ${ELAPSED}s, size: $SIZE"
    else
        END=$(date +%s)
        ELAPSED=$((END - START))
        SIZE=$(du -sh "$DEST" 2>/dev/null | cut -f1 || echo "unknown")
        STATUS="failed"
        FAILED_COUNT=$((FAILED_COUNT + 1))
        fail "$NAME backup failed after ${ELAPSED}s"
    fi

    jq -n \
        --arg name "$NAME" \
        --arg udid "$UDID" \
        --arg model "$MODEL" \
        --arg ios "$IOS" \
        --arg status "$STATUS" \
        --arg ts "$(date -Iseconds)" \
        --arg size "$SIZE" \
        --argjson elapsed "$ELAPSED" \
        '{name: $name, udid: $udid, model: $model, ios: $ios,
          status: $status, last_run: $ts, size: $size, elapsed_sec: $elapsed}' \
        > "$STATUS_DIR/${NAME}.json"

done <<< "$UDIDS"

pkill -f "pymobiledevice3 remote start-tunnel" 2>/dev/null || true

write_manifest

jq -n \
    --arg ts "$(date -Iseconds)" \
    --argjson total "$DEVICE_COUNT" \
    --argjson failed "$FAILED_COUNT" \
    '{last_run: $ts, total_devices: $total, failed: $failed,
      status: (if $failed == 0 then "ok" else "partial_failure" end)}' \
    > "$STATUS_DIR/summary.json"

log "=== Backup run complete. Devices: $DEVICE_COUNT, Failed: $FAILED_COUNT ==="
