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
│   ├── entrypoint.sh           – Container startup; checks lockdown dir, starts upload server
│   ├── ibackup.sh              – Main backup script (run by cron)
│   ├── setup.sh                – Mac-side helper: copies pairing files to NAS via SCP
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

### Step 4 – Pair your iPhone (ONCE per device)

DSM 7 does not support USB pairing at the kernel level. Pairing works by
copying the `.plist` file that macOS creates when you trust this Mac on your
iPhone. Choose one of the three options below.

**Option A – Run setup.sh on your Mac (recommended)**

```bash
bash scripts/setup.sh
```

The script finds all pairing files at `/var/db/lockdown/*.plist`, prompts for
your NAS SSH address, and copies them over automatically.

**Option B – Upload via browser**

Open `http://<NAS-IP>:8765/` (or `https://iphone-backup.home/upload` if Caddy
is configured). Drag your `.plist` files from `/var/db/lockdown/` on your Mac
and click Upload.

**Option C – Manual SCP**

```bash
scp /var/db/lockdown/*.plist admin@<NAS-IP>:/volume1/iphone-backups/.lockdown/
```

**Don't have a pairing file yet?** Plug your iPhone into your Mac via USB,
tap "Trust This Computer" on the iPhone, then check `/var/db/lockdown/` — a
new `.plist` will have appeared.

After any of the above, verify and trigger a test run:

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
   (mounted from `/volume1/iphone-backups/.lockdown` on the NAS)
2. Upload server starts on `:8765` (proxied by Caddy at `/upload`)
3. Cron fires on schedule → runs `ibackup.sh`
4. `ibackup.sh`:
   - Opens `pymobiledevice3` tunnels for any configured Tailscale IPs
   - Enumerates all paired devices via `idevice_id -l` (mDNS/Bonjour on LAN)
   - Backs up each device with `idevicebackup2 --full` (incremental by design)
   - Writes per-device `.status/<DeviceName>.json` and `summary.json`
5. Caddy serves the status files; dashboard polls every 60s

---

## Backup Storage Layout

```
/volume1/iphone-backups/
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

Restore requires a Mac with Finder (or iTunes). Run inside the container to
identify the device UDID, then use `idevicebackup2 restore` with the iPhone
on the same Wi-Fi network:

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

- **No USB required**: DSM 7 lacks the `ipheth` kernel module — USB pairing
  is impossible at the kernel level. All backups run over Wi-Fi (mDNS/Bonjour)
  or Tailscale.
- **Pairing files**: libimobiledevice reads pairing state from `/var/lib/lockdown/`
  inside the container, which is volume-mounted from
  `/volume1/iphone-backups/.lockdown` on the NAS. Files survive container
  restarts and can be added at any time without restarting the container.
- **host network mode**: Required for mDNS/Bonjour LAN discovery of iPhones.
- **No privileged mode**: Container runs unprivileged — no USB passthrough needed.

---

## Troubleshooting

| Problem | Fix |
|---|---|
| Device not found | Confirm `.plist` exists in `/volume1/iphone-backups/.lockdown`; re-run `setup.sh` or re-upload |
| "No pairing files" warning at startup | Add `.plist` via browser upload, `setup.sh`, or manual SCP |
| Tailscale device not found | Check iPhone's Tailscale IP in `.env`, confirm both are connected |
| Upload page not reachable on :8765 | Confirm container is running; check `docker logs iphone-backup` for upload server PID |
| Upload page not reachable via Caddy | Confirm Caddyfile snippet was added and `caddy reload` was run |
| Dashboard shows no data | Run a manual backup first; check Caddyfile path |
| Portainer build error | Confirm `image:` is used in compose, not `build:` |
