#!/usr/bin/env bash
# install.sh – One-command setup for iphone-backup on macOS.
# Run from the project root: bash install.sh
#
# What it does:
#   1. Checks/installs Homebrew dependencies (libimobiledevice, jq, Rust)
#   2. Builds the iphone-backup binary (cargo build --release)
#   3. Installs the binary to /usr/local/bin/
#   4. Installs and loads the launchd agent (daily backup at 2 am)
#   5. Optionally configures the backup path
#   6. Optionally runs a first backup

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLIST_LABEL="com.user.iphone-backup"
PLIST_DEST="$HOME/Library/LaunchAgents/$PLIST_LABEL.plist"
BINARY_DEST="/usr/local/bin/iphone-backup"

step()    { echo ""; echo "▶ $*"; }
ok()      { echo "  ✓ $*"; }
info()    { echo "  $*"; }
confirm() { read -rp "  $1 [y/N]: " _c; [[ "${_c,,}" == "y" ]]; }

echo ""
echo "╔══════════════════════════════════════╗"
echo "║   iphone-backup – Setup              ║"
echo "╚══════════════════════════════════════╝"

# ── Dependencies ──────────────────────────────────────────────────────────────
step "Checking dependencies..."

if ! command -v brew &>/dev/null; then
    echo "  ERROR: Homebrew is required."
    echo "    /bin/bash -c \"\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
    exit 1
fi

for pkg in libimobiledevice jq; do
    if brew list --formula "$pkg" &>/dev/null 2>&1; then
        ok "$pkg"
    else
        info "Installing $pkg..."
        brew install "$pkg"
        ok "$pkg installed"
    fi
done

if ! command -v cargo &>/dev/null; then
    info "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    ok "Rust installed"
else
    ok "Rust ($(cargo --version))"
fi

# ── Build ──────────────────────────────────────────────────────────────────────
step "Building iphone-backup..."
cd "$SCRIPT_DIR"
cargo build --release 2>&1
ok "Build succeeded"

# ── Install binary ─────────────────────────────────────────────────────────────
step "Installing binary to $BINARY_DEST..."
if [[ -w "$(dirname "$BINARY_DEST")" ]]; then
    cp "$SCRIPT_DIR/target/release/iphone-backup" "$BINARY_DEST"
else
    sudo cp "$SCRIPT_DIR/target/release/iphone-backup" "$BINARY_DEST"
fi
ok "Installed: $BINARY_DEST"

# ── Configure backup path ──────────────────────────────────────────────────────
step "Backup path configuration"
CURRENT_PATH=$(iphone-backup config 2>/dev/null | grep 'backup_path' | awk -F'"' '{print $2}' || echo "")
DEFAULT_PATH="${CURRENT_PATH:-$HOME/Backups/iOS}"
read -rp "  Backup path [$DEFAULT_PATH]: " BACKUP_PATH
BACKUP_PATH="${BACKUP_PATH:-$DEFAULT_PATH}"
mkdir -p "$BACKUP_PATH"

CONFIG_FILE="$HOME/.config/iphone-backup/config.toml"
mkdir -p "$(dirname "$CONFIG_FILE")"
cat > "$CONFIG_FILE" <<EOF
backup_path = "$BACKUP_PATH"
EOF
ok "Config saved: $CONFIG_FILE"
ok "Backup path: $BACKUP_PATH"

# ── launchd agent ──────────────────────────────────────────────────────────────
step "Installing launchd agent (runs daily at 2:00 am)..."
sed "s|/usr/local/bin/iphone-backup|$BINARY_DEST|g" \
    "$SCRIPT_DIR/config/com.user.iphone-backup.plist" > "$PLIST_DEST"
launchctl unload "$PLIST_DEST" 2>/dev/null || true
launchctl load "$PLIST_DEST"
ok "Loaded: $PLIST_LABEL"
info "Fires daily at 2:00 am while the Mac is awake."

# ── First backup ───────────────────────────────────────────────────────────────
step "First backup"
if confirm "Run a backup now?"; then
    iphone-backup backup
fi

# ── Done ───────────────────────────────────────────────────────────────────────
echo ""
echo "╔══════════════════════════════════════╗"
echo "║   Setup complete                     ║"
echo "╚══════════════════════════════════════╝"
echo ""
echo "  Open dashboard:  iphone-backup"
echo "    [1] Dashboard  – run backups, view logs"
echo "    [2] Restore    – restore a backup to a device"
echo "    [3] Services   – manage the launchd agent, pair devices"
echo ""
echo "  Config:          $CONFIG_FILE"
echo "  launchd log:     /tmp/iphone-backup-launchd.log"
echo ""
