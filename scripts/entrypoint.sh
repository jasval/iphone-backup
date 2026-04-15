#!/usr/bin/env bash
set -euo pipefail

CRON_SCHEDULE="${BACKUP_CRON:-0 2 * * *}"
LOG="/backups/.status/container.log"

mkdir -p /backups/.status

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG"; }

log "Starting iPhone backup container (Synology mode)"
log "Cron schedule: $CRON_SCHEDULE"

# Synology DSM runs its own usbmuxd for all USB peripherals.
# We use the host socket mounted at /var/run/usbmuxd — do NOT start a second instance.
if [ -S /var/run/usbmuxd ]; then
    log "usbmuxd socket found at /var/run/usbmuxd — OK"
else
    log "WARNING: /var/run/usbmuxd socket not found."
    log "  → Check the volume mount in docker-compose.yml"
    log "  → Ensure a USB device is plugged in or DSM's usbmuxd is running"
fi

# Write crontab
echo "$CRON_SCHEDULE /usr/local/bin/ibackup.sh >> $LOG 2>&1" | crontab -
log "Cron registered: $CRON_SCHEDULE"

log "Entering cron loop..."
exec cron -f
