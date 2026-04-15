# iphone-backup

Automated, incremental iPhone/iPad backups on macOS — native Rust binary with a Ratatui TUI dashboard, scheduled via launchd. No Docker, no NAS required (though any SMB mount works as the backup destination).

## Requirements

- macOS (Apple Silicon or Intel)
- Rust (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- `brew install libimobiledevice jq`
- `pip3 install pymobiledevice3` (for USB pairing)
- iPhone with **Wi-Fi sync enabled**: Settings → General → VPN & Device Management → (your Mac) → Connect via Wi-Fi

---

## Quick Start

```bash
bash install.sh
```

That's it. The script handles everything below automatically.

---

## Manual Setup

### 1 — Build and install the binary

```bash
cargo build --release
sudo cp target/release/iphone-backup /usr/local/bin/
```

### 2 — Configure the backup path

```bash
mkdir -p ~/.config/iphone-backup
cat > ~/.config/iphone-backup/config.toml <<EOF
backup_path = "/Volumes/ios-backups"   # or ~/Backups/iOS for local storage
EOF
```

### 3 — Pair your iPhone (once per device)

Plug in via USB:
```bash
bash scripts/setup.sh
```

Pairing records are saved to `~/.config/iphone-backup/` and read by libimobiledevice.

### 4 — Install the launchd agent

```bash
cp config/com.user.iphone-backup.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.user.iphone-backup.plist
```

Runs `iphone-backup backup` daily at 2:00 am.

---

## Usage

```bash
iphone-backup          # open TUI dashboard
iphone-backup backup   # run a backup headlessly (used by launchd)
iphone-backup config   # show config file path and current settings
```

### TUI keybindings

| Key | Action |
|-----|--------|
| `r` | Trigger backup now |
| `↑` / `↓` or `k` / `j` | Select device |
| `PgUp` / `PgDn` | Scroll log |
| `G` / `End` | Jump to end of log |
| `q` / `Esc` | Quit |

---

## File structure

```
iphone-backup/
├── Cargo.toml
├── install.sh                  – One-command setup
├── src/
│   ├── main.rs                 – Entry point; dispatches to TUI or backup mode
│   ├── config.rs               – Config (~/.config/iphone-backup/config.toml)
│   ├── status.rs               – Status file types (DeviceStatus, Summary)
│   ├── backup.rs               – Backup runner (idevice_id + idevicebackup2)
│   └── tui/
│       ├── mod.rs              – App state and event loop
│       └── ui.rs               – Ratatui rendering
├── scripts/
│   ├── setup.sh                – USB pairing helper (run once per device)
│   └── restore.sh              – Guided restore from backup to device
└── config/
    └── com.user.iphone-backup.plist  – launchd agent template
```

---

## Backup storage layout

```
<backup_path>/
├── Jaspers_iPhone/         ← backup data (idevicebackup2 format)
├── Jaspers_iPad/
└── .status/
    ├── summary.json        ← last run summary
    ├── manifest.json       ← list of backed-up device names
    ├── Jaspers_iPhone.json ← per-device status (name, iOS, size, last_run)
    └── ibackup.log         ← full backup log
```

---

## Restoring a backup

```bash
bash scripts/restore.sh
```

Interactive: lists available backups with size and date, detects connected devices, runs `idevicebackup2 restore`.

---

## Updating

```bash
git pull
cargo build --release
sudo cp target/release/iphone-backup /usr/local/bin/
```

---

## Troubleshooting

| Problem | Fix |
|---|---|
| `idevice_id` not found | `brew install libimobiledevice` |
| No devices found | Enable Wi-Fi sync: Settings → General → VPN & Device Management → (your Mac) → Connect via Wi-Fi |
| `setup.sh` can't find device | Plug in via USB, unlock phone, run `pip3 install pymobiledevice3` |
| Backup path not accessible | Check `~/.config/iphone-backup/config.toml`; `iphone-backup config` shows current value |
| launchd not firing | `launchctl list | grep iphone` — check it's loaded; see `/tmp/iphone-backup-launchd.log` |
