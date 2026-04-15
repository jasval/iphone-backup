# iOS Backup on Synology NAS

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
│   ├── entrypoint.sh           – Container startup; checks lockdown dir, starts upload server
│   ├── ibackup.sh              – Main backup script (run by cron)
│   ├── setup.sh                – Mac-side helper: pairs iPhone via USB and copies record to NAS
│   └── upload_server.py        – Pairing file upload server (stdlib Python, port 8765)
├── config/
│   ├── Caddyfile.snippet       – Paste into your Caddyfile
│   └── adguard-dns-rewrite.md
└── www/
    ├── dashboard.html          – Status dashboard (light/dark, auto-refresh)
    └── upload.html             – Pairing file upload page (served from container)
```

---

## Setup Guide

### Step 1 – Build & push the Docker image

Run this from your Mac (or directly on the NAS via SSH):

```bash
# From Apple Silicon Mac (multi-platform build):
docker buildx build \
  --platform linux/amd64,linux/arm64 \
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
BACKUP_PATH=/volume1/ios-backups
BACKUP_CRON=0 2 * * *
TZ=Europe/London
```

### Step 3 – Deploy via Portainer

1. Portainer → **Stacks** → **Add Stack**
2. Upload `docker/docker-compose.yml`
3. Under **Environment variables**, add the contents of your `.env`
4. Click **Deploy the stack**

> Portainer does not support `build:` in compose files — this stack uses
> `image:` so it pulls directly from Docker Hub. ✓

### Step 4 – Pair your iPhone (ONCE per device)

DSM 7 does not support USB pairing at the kernel level. Instead, `setup.sh`
uses the `pymobiledevice3` Python API to pair directly via USB on your Mac
and copies the resulting pairing record to the NAS over SCP — no file system
access to `/var/db/lockdown` required.

**Prerequisites (Mac only):**
```bash
pip3 install pymobiledevice3
```

**Run setup.sh with your iPhone plugged into your Mac via USB:**
```bash
bash scripts/setup.sh
```

The script will:
1. Pair with the connected iPhone using the pymobiledevice3 API
2. Prompt for your NAS SSH address
3. Copy the pairing record directly to the NAS

Accept **"Trust This Computer"** on the iPhone if prompted.

**Alternative – Upload via browser**

If you already have a pairing `.plist` file, open `http://<NAS-IP>:8765/`
(or `https://iphone-backup.home/upload` if Caddy is configured) and upload it.

After pairing, verify and trigger a test run:

```bash
docker exec iphone-backup ls /var/lib/lockdown/
docker exec iphone-backup /usr/local/bin/ibackup.sh
```

### Step 5 – Dashboard (optional)

1. Copy `www/dashboard.html` to `/opt/iphone-backup/www/` on your NAS
2. Paste `config/Caddyfile.snippet` into your Caddyfile and run `caddy reload`
3. Add a DNS rewrite in AdGuard Home: `iphone-backup.home` → NAS LAN IP

Dashboard available at:
- **LAN:** https://iphone-backup.home/dashboard/dashboard.html
- **Upload page:** https://iphone-backup.home/upload
- **Tailscale:** https://&lt;nas-hostname&gt;.&lt;tailnet&gt;.ts.net/dashboard/dashboard.html

---

## How It Works

1. Container starts → checks `/var/lib/lockdown/` for `.plist` pairing files
   (mounted from `/volume1/ios-backups/.lockdown` on the NAS)
2. Upload server starts on `:8765` (proxied by Caddy at `/upload`)
3. Cron fires on schedule → runs `ibackup.sh`
4. `ibackup.sh`:
   - Enumerates all paired devices via `idevice_id -l` (mDNS/Bonjour on LAN)
   - Backs up each device with `idevicebackup2 --full` (incremental by design)
   - Writes per-device `.status/<DeviceName>.json` and `summary.json`
5. Caddy serves the status files; dashboard polls every 60s

---

## Backup Storage Layout

```
/volume1/ios-backups/
├── .lockdown/               ← pairing .plist files (mounted into container)
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

Run inside the container to identify the device UDID, then restore with the
iPhone on the same Wi-Fi network:

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
# Rebuild and push from Mac
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -t youruser/iphone-backup:latest -f docker/Dockerfile --push .

# In Portainer: Stack → Recreate → Pull latest image ✓
```

---

## Synology-Specific Notes

- **No USB required for backups**: DSM 7 lacks the `ipheth` kernel module —
  USB pairing via the NAS is impossible. `setup.sh` handles pairing from your
  Mac over USB once; all backups then run over Wi-Fi (mDNS/Bonjour).
- **Pairing files**: libimobiledevice reads pairing state from `/var/lib/lockdown/`
  inside the container, which is volume-mounted from
  `/volume1/ios-backups/.lockdown` on the NAS. Files survive container
  restarts and can be added at any time without restarting the container.
- **macOS access restriction**: `/var/db/lockdown` is protected by macOS SIP/TCC
  even from root. `setup.sh` works around this using the `pymobiledevice3`
  Python API which pairs directly and saves the record to a writable location.
- **host network mode**: Required for mDNS/Bonjour LAN discovery of iPhones.
- **No privileged mode**: Container runs unprivileged — no USB passthrough needed.

---

## Troubleshooting

| Problem | Fix |
|---|---|
| `setup.sh` can't find device | Confirm iPhone is plugged in, unlocked, and `pip3 install pymobiledevice3` was run |
| Device not found during backup | Confirm `.plist` exists in `/volume1/ios-backups/.lockdown`; re-run `setup.sh` |
| "No pairing files" warning at startup | Run `setup.sh` or upload via browser at `:8765` |
| scp fails during setup | Ensure SSH is enabled on the NAS (DSM → Control Panel → Terminal & SNMP) |
| Upload page not reachable on :8765 | Confirm container is running; check `docker logs iphone-backup` for upload server PID |
| Upload page not reachable via Caddy | Confirm Caddyfile snippet was added and `caddy reload` was run |
| Dashboard shows no data | Run a manual backup first; check Caddyfile path |
| Portainer build error | Confirm `image:` is used in compose, not `build:` |
