#!/usr/bin/env bash
# Z-DECK autostart installer — Raspberry Pi 3, 64-bit OS only.
#
# What it does (run ON the Pi, from this repo):
#   1. Copies binary/pi3-arm64/{zdeck-run,zdeck-game,zdeck-gps,zdeck-fetch}
#      + os/pi3-arm64/zdeck-auto.sh  ->  ~/zdeck/  (chmod +x)
#   2. Enables console auto-login (raspi-config, non-interactive)
#   3. Hooks ~/zdeck/zdeck-auto.sh into the tty1 auto-login shell, so the
#      master script (and zdeck-run) starts automatically after boot.
#   4. Ensures enable_uart=1 so /dev/ttyAMA0 (the deck's only GPS device)
#      exists. It does NOT touch bluetooth overlays — if your GPS is wired
#      to the AMA0 pins and you see bluetooth owning them, also add
#      `dtoverlay=disable-bt` to the same config file and reboot.
#
# Usage on the Pi:
#   ./os/pi3-arm64/install-autostart.sh
#   sudo reboot   # deck boots -> auto-login -> logo -> 5 s AUTO countdown -> game
#
# Primary path is the login hook. zdeck.service is shipped as an
# alternative (systemd on tty1); it is NOT enabled by default because
# running both would launch the game twice.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_BIN="$REPO_ROOT/binary/pi3-arm64"
SRC_OS="$REPO_ROOT/os/pi3-arm64"
DEST="$HOME/zdeck"

echo "==> [1/4] Installing pi3-arm64 binaries + auto-boot script -> $DEST"
mkdir -p "$DEST"
for b in zdeck-run zdeck-game zdeck-gps zdeck-fetch; do
    if [[ ! -f "$SRC_BIN/$b" ]]; then
        echo "ERROR: $SRC_BIN/$b not found. Build it first:" >&2
        echo "  ./rust/build-pi.sh arm64" >&2
        exit 1
    fi
    cp -f "$SRC_BIN/$b" "$DEST/$b"
    chmod +x "$DEST/$b"
done
cp -f "$SRC_OS/zdeck-auto.sh" "$DEST/zdeck-auto.sh"
chmod +x "$DEST/zdeck-auto.sh"
ls -la "$DEST"

echo "==> [2/4] Enabling console auto-login"
if command -v raspi-config >/dev/null 2>&1; then
    sudo raspi-config nonint do_boot_behaviour B2 || true
    echo "  console autologin requested (B2)."
else
    echo "  raspi-config not found — enable autologin manually:"
    echo "    sudo raspi-config  ->  System Options  ->  Boot / Auto Login  ->  Console Autologin"
fi

echo "==> [3/4] Hooking zdeck-auto.sh into the tty1 login shell"
HOOK='if [ -z "${SSH_CONNECTION:-}" ] && [ "$(tty 2>/dev/null)" = "/dev/tty1" ]; then exec "$HOME/zdeck/zdeck-auto.sh"; fi'
PROFILE="$HOME/.bash_profile"
touch "$PROFILE"
if grep -q "zdeck/zdeck-auto.sh" "$PROFILE"; then
    echo "  hook already present in $PROFILE"
else
    {
        echo ""
        echo "# Z-DECK kiosk: boot straight into the game on the local console."
        echo "$HOOK"
    } >> "$PROFILE"
    echo "  hook added to $PROFILE"
fi

echo "==> [4/4] Ensuring UART for /dev/ttyAMA0"
CFGCANDIDATES=("/boot/firmware/config.txt" "/boot/config.txt")
CFG=""
for c in "${CFGCANDIDATES[@]}"; do
    if [[ -f "$c" ]]; then CFG="$c"; break; fi
done
if [[ -z "$CFG" ]]; then
    echo "  no Pi config file found — skipping (create one and set enable_uart=1)."
else
    if grep -q "^enable_uart=1" "$CFG"; then
        echo "  enable_uart=1 already in $CFG"
    else
        echo "  adding enable_uart=1 to $CFG (backup: ${CFG}.zdeck-bak)"
        sudo cp -n "$CFG" "${CFG}.zdeck-bak" || true
        if grep -q "^enable_uart=" "$CFG"; then
            sudo sed -i 's/^enable_uart=.*/enable_uart=1/' "$CFG"
        else
            echo "enable_uart=1" | sudo tee -a "$CFG" >/dev/null
        fi
        echo "  UART change needs a reboot to take effect."
    fi
fi

echo
echo "Done. Reboot to test:"
echo "  sudo reboot"
echo "After boot you should see: logo -> AUTO (${DEST} on /dev/ttyAMA0, 5 s)"
echo "  -> live GPS fix -> fresh 300 m map -> game."
echo
echo "Maintenance: SSH in (the hook only fires on /dev/tty1), or press"
echo "Ctrl+C during the 8 s restart pause on the console."
echo "Alternative (not enabled): sudo cp $SRC_OS/zdeck.service /etc/systemd/system/ && sudo systemctl enable --now zdeck"
