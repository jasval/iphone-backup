#!/usr/bin/env bash
set -euo pipefail

CRON_SCHEDULE="${BACKUP_CRON:-0 2 * * *}"
LOG="/backups/.status/container.log"
LOCKDOWN_DIR="/var/lib/lockdown"

mkdir -p /backups/.status

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG"; }

log "Starting iPhone backup container (Wi-Fi/Tailscale mode)"
log "Cron schedule: $CRON_SCHEDULE"

# Check for pre-copied pairing files
PLIST_COUNT=$(find "$LOCKDOWN_DIR" -maxdepth 1 -name '*.plist' 2>/dev/null | wc -l | tr -d ' ')

if [[ "$PLIST_COUNT" -eq 0 ]]; then
    log "WARNING: No pairing .plist files found in $LOCKDOWN_DIR"
    log "  → Option A: Run scripts/setup.sh on your Mac (copies via SCP)"
    log "  → Option B: Upload via browser at http://<NAS-IP>:8765/"
    log "  → Option C: scp /var/db/lockdown/*.plist <user>@<nas-ip>:$LOCKDOWN_DIR/"
    log "  Backup will still run but will find no devices until pairing files are present."
else
    log "Found $PLIST_COUNT pairing file(s) in $LOCKDOWN_DIR — OK"
fi

# Start upload server in background
log "Starting pairing-file upload server on :8765"
python3 /usr/local/bin/upload_server.py >> "$LOG" 2>&1 &
log "Upload server PID: $!"

# Write crontab
echo "$CRON_SCHEDULE /usr/local/bin/ibackup.sh >> $LOG 2>&1" | crontab -
log "Cron registered: $CRON_SCHEDULE"

log "Entering cron loop..."
exec cron -f
