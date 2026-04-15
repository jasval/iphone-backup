# iPhone Backup on Synology NAS

Automatic, incremental, multi-device iPhone/iPad backups running in Docker
on a Synology NAS. Discovers devices via LAN (mDNS) and Tailscale.
Status dashboard served via Caddy.

## File Structure

```
iphone-backup/
├── README.md
├── docker/
│   ├── Dockerfile              – Container image
│   ├── docker-compose.yml      – Portainer/Compose stack definition
│   └── .env.example            – Copy to .env and fill in
├── scripts/
│   ├── entrypoint.sh           – Container startup (uses Synology host usbmuxd)
│   ├── ibackup.sh              – Main backup script (run by cron)
│   └── setup.sh                – First-time USB pairing (run once)
├── config/
│   ├── Caddyfile.snippet       – Paste into your Caddyfile
│   └── adguard-dns-rewrite.md
└── www/
    └── dashboard.html          – Status dashboard (light/dark, auto-refresh)
```

---

## Setup Guide

### Step 1 – Build & push the Docker image

Run this from your Mac (or directly on the NAS via SSH):

```bash
# From Apple Silicon Mac (cross-compile for NAS x86_64):
docker buildx build --platform linux/amd64 \
  -t youruser/iphone-backup:latest \
  -f docker/Dockerfile --push .

# OR directly on NAS via SSH (no cross-compilation needed):
docker build -t youruser/iphone-backup:latest -f docker/Dockerfile .
```

Replace `youruser` with your Docker Hub username.
Only needs to be repeated if you modify the Dockerfile or scripts.

### Step 2 – Configure environment

```bash
cp docker/.env.example docker/.env
```

Edit `docker/.env`:
```
DOCKER_IMAGE=youruser/iphone-backup:latest
BACKUP_PATH=/volume1/iphone-backups
BACKUP_CRON=0 2 * * *
IPHONE_TAILSCALE_IPS=100.x.x.x        # your iPhone's Tailscale IP
TZ=Europe/London
```

### Step 3 – Deploy via Portainer

1. Portainer → **Stacks** → **Add Stack**
2. Upload `docker/docker-compose.yml`
3. Under **Environment variables**, add the contents of your `.env`
4. Click **Deploy the stack**

> Portainer does not support `build:` in compose files — this stack uses
> `image:` so it pulls directly from Docker Hub. ✓

### Step 4 – First-time USB pairing (ONCE ONLY)

Plug your iPhone into the NAS via USB, unlock it, then:

```bash
docker exec -it iphone-backup /usr/local/bin/setup.sh
```

Follow the prompts. Accept **"Trust This Computer"** on each iPhone.
Unplug the cable — all future backups run wirelessly.

### Step 5 – Dashboard (optional)

1. Copy `www/dashboard.html` to `/opt/iphone-backup/www/` on your NAS
2. Paste `config/Caddyfile.snippet` into your Caddyfile and run `caddy reload`
3. Add a DNS rewrite in AdGuard Home: `iphone-backup.home` → NAS LAN IP

Dashboard available at:
- **LAN:** https://iphone-backup.home/dashboard/dashboard.html
- **Tailscale:** https://&lt;nas-hostname&gt;.&lt;tailnet&gt;.ts.net/dashboard/dashboard.html

---

## How It Works

1. Container starts → verifies Synology's host `usbmuxd` socket at `/var/run/usbmuxd`
2. Cron fires on schedule → runs `ibackup.sh`
3. `ibackup.sh`:
   - Opens `pymobiledevice3` tunnels for any configured Tailscale IPs
   - Enumerates all paired devices via `idevice_id -l`
   - Backs up each device with `idevicebackup2 --full` (incremental by design)
   - Writes per-device `.status/<DeviceName>.json` and `summary.json`
4. Caddy serves the status files; dashboard polls every 60s

---

## Backup Storage Layout

```
/volume1/iphone-backups/
├── Johns_iPhone/
├── Janes_iPad/
└── .status/
    ├── summary.json
    ├── manifest.json
    ├── Johns_iPhone.json
    ├── Janes_iPad.json
    └── ibackup.log
```

---

## Restoring a Backup

USB connection required for restore. Run inside the container:

```bash
docker exec -it iphone-backup bash

# List all paired devices and their UDIDs
idevice_id -l | while read UDID; do
  NAME=$(ideviceinfo -u "$UDID" -k DeviceName)
  echo "$UDID  →  $NAME"
done

# Restore
idevicebackup2 -u <UDID> restore \
  --system --settings --interactive \
  /backups/Johns_iPhone/
```

---

## Manual Backup Trigger

```bash
docker exec iphone-backup /usr/local/bin/ibackup.sh
```

---

## Updating the Image

```bash
# Rebuild and push from Mac or NAS
docker buildx build --platform linux/amd64 \
  -t youruser/iphone-backup:latest -f docker/Dockerfile --push .

# In Portainer: Stack → Recreate → Pull latest image ✓
```

---

## Synology-Specific Notes

- **usbmuxd**: DSM runs its own instance. The container uses the host socket
  at `/var/run/usbmuxd` (mounted via `docker-compose.yml`). A second `usbmuxd`
  is never started inside the container.
- **Privileged mode**: Required for USB device passthrough (`/dev/bus/usb`).
- **host network mode**: Required for mDNS/Bonjour LAN discovery of iPhones.

---

## Troubleshooting

| Problem | Fix |
|---|---|
| Device not found over WiFi | Re-pair via USB with `setup.sh`, re-run WiFi sync enable |
| Tailscale device not found | Check iPhone's Tailscale IP in `.env`, confirm both are connected |
| usbmuxd socket warning | Check `/var/run/usbmuxd` volume mount in `docker-compose.yml` |
| Dashboard shows no data | Run a manual backup first; check Caddyfile path |
| Portainer build error | Confirm `image:` is used in compose, not `build:` |
