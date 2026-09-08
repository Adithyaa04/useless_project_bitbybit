#!/usr/bin/env bash
# Z-DECK autostart installer — Raspberry Pi, Debian-family OS (incl. DietPi).
#
# Standard-Debian way, no DietPi tooling involved:
#   1. CHECKS everything: arch, binaries, UART (/dev/ttyAMA0), I2C
#      (/dev/i2c-1 + CardKB 0x5F), uinput, CardKB script/service.
#   2. Copies binary/pi3-arm64/{zdeck-run,zdeck-game,zdeck-gps,zdeck-fetch,zdeck-cardkb}
#      + os/pi3-arm64/{zdeck-auto.sh,zdeck-main.sh} [+ cardkb_keyboard.py]
#      -> ~/zdeck/  (chmod +x)
#   3. Console auto-login via a getty override drop-in
#      (/etc/systemd/system/getty@tty1.service.d/zdeck-autologin.conf) —
#      the same mechanism raspi-config's "Boot / Auto Login" uses.
#      Skip with --no-autologin (deck then starts after a manual login).
#   4. A ~/.bash_profile hook that RUNS ~/zdeck/zdeck-main.sh as a child on
#      /dev/tty1 (never execs, SSH untouched). Quit chain is fully wired:
#      launcher "Quit to terminal" exits 42 -> game loop stops -> main
#      script ends -> hook returns -> you land on a persistent login shell.
#      Log out (or run ~/zdeck/zdeck-main.sh) to return to the deck menu.
#   5. Ensures enable_uart=1 (GPS on /dev/ttyAMA0) and dtparam=i2c_arm=on
#      (CardKB on /dev/i2c-1). Does NOT touch bluetooth overlays.
#
# If a previous DietPi custom.sh install is found (Z-DECK marker), it is
# cleaned up (backup restored) since that method is superseded.
#
# Run UNPRIVILEGED as the deck user (it asks for sudo itself):
#   ./os/pi3-arm64/install-autostart.sh
#   ./os/pi3-arm64/install-autostart.sh --yes
#   ./os/pi3-arm64/install-autostart.sh --check-only   # checks, changes nothing
#   ./os/pi3-arm64/install-autostart.sh --uninstall    # remove autostart again
#   ./os/pi3-arm64/install-autostart.sh --no-autologin # hook only, you log in
#   ./os/pi3-arm64/install-autostart.sh --with-systemd # ALSO install+enable
#        zdeck.service (opt-in; default is getty+hook so the game
#        doesn't launch twice).
#
# After install + reboot, the deck menu appears on the Pi console.
# SSH sessions are never hooked.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_BIN="$REPO_ROOT/binary/pi3-arm64"
SRC_OS="$REPO_ROOT/os/pi3-arm64"
SRC_CARDKB="$REPO_ROOT/cardkb/cardkb_keyboard.py"
# DEST is resolved after arg parsing (deck user's home, not root's).

YES=0
CHECK_ONLY=0
UNINSTALL=0
WITH_SYSTEMD=0
NO_AUTOLOGIN=0
for arg in "$@"; do
    case "$arg" in
        -y|--yes) YES=1 ;;
        --check-only) CHECK_ONLY=1 ;;
        --uninstall) UNINSTALL=1 ;;
        --with-systemd) WITH_SYSTEMD=1 ;;
        --no-autologin) NO_AUTOLOGIN=1 ;;
        -h|--help)
            sed -n '2,32p' "$0"
            exit 0
            ;;
        *) echo "Unknown arg: $arg (try --help)" >&2; exit 1 ;;
    esac
done

# Must run as the deck user (sudo is requested internally where needed);
# re-exec down from root so $HOME/$USER stay consistent.
if [[ "$(id -u)" -eq 0 && -n "${SUDO_USER:-}" ]]; then
    echo "Re-running as $SUDO_USER (run this script unprivileged)..."
    exec sudo -u "$SUDO_USER" "$0" "$@"
fi
INSTALL_USER="${SUDO_USER:-$USER}"
INSTALL_HOME="$(getent passwd "$INSTALL_USER" | cut -d: -f6)"
[[ -n "$INSTALL_HOME" ]] || INSTALL_HOME="$HOME"
DEST="$INSTALL_HOME/zdeck"
GETTY_DROPIN_DIR="/etc/systemd/system/getty@tty1.service.d"
GETTY_DROPIN="$GETTY_DROPIN_DIR/zdeck-autologin.conf"

pass=0; warn_n=0; fail_n=0
ok()   { pass=$((pass+1)); echo "  [OK]   $*"; }
warn() { warn_n=$((warn_n+1)); echo "  [WARN] $*"; }
fail() { fail_n=$((fail_n+1)); echo "  [FAIL] $*"; }
have() { command -v "$1" >/dev/null 2>&1; }

ask_yes() {  # $1 = prompt -> 0=yes
    if [[ "$YES" -eq 1 ]]; then return 0; fi
    read -r -p "$1 [Y/n] " ans || true
    [[ "${ans:-Y}" =~ ^[Yy]$|^$ ]]
}

pi_config() {  # echo first existing Pi config file, or empty
    for c in /boot/firmware/config.txt /boot/config.txt; do
        [[ -f "$c" ]] && { echo "$c"; return 0; }
    done
    echo ""
}

dietpi_custom_path() {  # legacy cleanup only: previous DietPi-method installs
    # Old images: /var/lib/dietpi-autostart/custom.sh,
    # new images: /var/lib/dietpi/dietpi-autostart/custom.sh
    for c in /var/lib/dietpi-autostart/custom.sh \
             /var/lib/dietpi/dietpi-autostart/custom.sh; do
        [[ -f "$c" ]] && { echo "$c"; return 0; }
    done
    if [[ -d /boot/dietpi ]]; then
        echo "/var/lib/dietpi/dietpi-autostart/custom.sh"
        return 0
    fi
    echo ""
}

remove_profile_hook() {  # $1 = profile path
    local profile="$1"
    if [[ -f "$profile" ]] && grep -q "zdeck/zdeck-main.sh\|zdeck/zdeck-auto.sh" "$profile"; then
        cp -n "$profile" "$profile.zdeck-bak" || true
        grep -v "zdeck/zdeck-main.sh\|zdeck/zdeck-auto.sh\|Z-DECK kiosk" "$profile" > "$profile.tmp" || true
        mv "$profile.tmp" "$profile"
        echo "  removed Z-DECK hook from $profile (backup: $profile.zdeck-bak)"
    fi
}

# ---------------------------------------------------------------- uninstall
if [[ "$UNINSTALL" -eq 1 ]]; then
    PROFILE="$INSTALL_HOME/.bash_profile"
    if [[ -f "$PROFILE" ]] && grep -q "zdeck/zdeck-main.sh\|zdeck/zdeck-auto.sh" "$PROFILE"; then
        remove_profile_hook "$PROFILE"
        echo "Removed Z-DECK hook from $PROFILE."
    else
        echo "No Z-DECK hook in $PROFILE."
    fi
    if [[ -f "$GETTY_DROPIN" ]]; then
        sudo rm -f "$GETTY_DROPIN"
        sudo systemctl daemon-reload || true
        echo "Removed getty autologin drop-in $GETTY_DROPIN."
    else
        echo "No getty drop-in to remove."
    fi
    DCUSTOM="$(dietpi_custom_path)"
    if [[ -n "$DCUSTOM" && -f "$DCUSTOM" ]] && grep -q "Z-DECK" "$DCUSTOM"; then
        if [[ -f "$DCUSTOM.zdeck-bak" ]]; then
            sudo cp -f "$DCUSTOM.zdeck-bak" "$DCUSTOM"
            echo "Restored legacy DietPi $DCUSTOM from backup."
        else
            printf '#!/bin/dash\n# DietPi-AutoStart custom script\n# Location: %s\n\nexit 0\n' "$DCUSTOM" | sudo tee "$DCUSTOM" >/dev/null
            echo "Reset legacy DietPi $DCUSTOM to the default (exit 0)."
        fi
    fi
    if { systemctl is-enabled zdeck.service >/dev/null 2>&1; }; then
        echo "Disabling zdeck.service ..."
        sudo systemctl disable --now zdeck.service || true
    fi
    exit 0
fi

echo "=== Z-DECK preflight checks (user=$INSTALL_USER home=$INSTALL_HOME) ==="

# 1. arch / OS
ARCH="$(uname -m || true)"
if [[ "$ARCH" == "aarch64" || "$ARCH" == "arm64" ]]; then
    ok "arch=$ARCH (64-bit, supported)"
else
    warn "arch=$ARCH — this installer targets 64-bit Pi OS (binaries are pi3-arm64)"
fi
if [[ -f /etc/os-release ]]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    echo "  [INFO] ${PRETTY_NAME:-unknown distro}"
fi

# 2. repo binaries
echo "--- binaries ($SRC_BIN) ---"
for b in zdeck-run zdeck-game zdeck-gps zdeck-fetch zdeck-cardkb; do
    if [[ -x "$SRC_BIN/$b" ]]; then
        ok "$b present+executable ($(du -h "$SRC_BIN/$b" | cut -f1))"
    elif [[ -f "$SRC_BIN/$b" ]]; then
        warn "$b present but not executable (will chmod on install)"
    else
        fail "$b MISSING — build first: ./rust/build-pi.sh arm64"
    fi
done
for s in zdeck-auto.sh zdeck-main.sh; do
    if [[ -f "$SRC_OS/$s" ]]; then ok "$s present in repo"
    elif [[ "$s" == "zdeck-main.sh" ]]; then warn "$s not in repo yet (this installer ships it)"
    else fail "$s MISSING in $SRC_OS"; fi
done

# 3. UART / GPS device
echo "--- UART / GPS ---"
CFG="$(pi_config)"
if [[ -n "$CFG" ]]; then
    if grep -q "^enable_uart=1" "$CFG"; then ok "enable_uart=1 in $CFG"
    else warn "enable_uart!=1 in $CFG (installer will set it; reboot needed)"
    fi
else
    warn "no Pi config file found (/boot/firmware/config.txt, /boot/config.txt)"
fi
if [[ -e /dev/ttyAMA0 ]]; then ok "/dev/ttyAMA0 exists (deck GPS device)"
else warn "/dev/ttyAMA0 missing — enable_uart=1 + reboot, or use --sim indoors"
fi

# 4. I2C / CardKB
echo "--- I2C / CardKB ---"
if [[ -n "$CFG" ]]; then
    if grep -Eq '^\s*dtparam=i2c_arm=on' "$CFG"; then ok "dtparam=i2c_arm=on in $CFG"
    else warn "I2C not enabled in $CFG (installer will set dtparam=i2c_arm=on)"
    fi
fi
if ls /dev/i2c* >/dev/null 2>&1; then
    ok "I2C devices: $(ls /dev/i2c* 2>/dev/null | tr '\n' ' ')"
else
    warn "no /dev/i2c* — enable I2C + reboot (sudo raspi-config nonint do_i2c 0)"
fi
if [[ -e /dev/i2c-1 ]] && have i2cdetect; then
    if i2cdetect -y 1 2>/dev/null | grep -qi "5f"; then
        ok "CardKB found at 0x5F on bus 1"
    else
        warn "0x5F NOT on bus 1 — check wiring SDA->GPIO2 SCL->GPIO3 VCC->3.3V"
    fi
elif ! have i2cdetect; then
    warn "i2c-tools not installed (installer adds it)"
fi

# 5. uinput / input group (CardKB virtual keyboard)
echo "--- uinput / virtual keyboard ---"
if lsmod 2>/dev/null | grep -q "^uinput"; then ok "uinput module loaded"
else warn "uinput not loaded (cardkb/setup.sh handles: modprobe uinput + udev rule)"
fi
if [[ -f /etc/udev/rules.d/99-uinput.rules ]]; then ok "udev rule 99-uinput.rules present"
else warn "udev rule 99-uinput.rules missing (cardkb/setup.sh creates it)"
fi
if id -nG "$INSTALL_USER" 2>/dev/null | tr ' ' '\n' | grep -qx input; then ok "user '$INSTALL_USER' in 'input' group"
else warn "user '$INSTALL_USER' NOT in 'input' group (cardkb/setup.sh: sudo usermod -aG input \$USER)"
fi
if [[ -x "$SRC_BIN/zdeck-cardkb" ]]; then ok "zdeck-cardkb Rust driver in repo (no Python needed)"
elif [[ -f "$SRC_CARDKB" ]]; then ok "cardkb_keyboard.py in repo (Python fallback)"
elif [[ -f "$HOME/cardkb_keyboard.py" ]]; then ok "~/cardkb_keyboard.py present (Python fallback)"
else warn "no CardKB driver found (build: ./rust/build-pi.sh arm64, or run cardkb/setup.sh)"
fi
if have python3 && python3 -c "import smbus2, evdev" 2>/dev/null; then
    ok "python smbus2+evdev importable (fallback ready)"
else
    echo "  [INFO] python smbus2/evdev missing — fine, Rust zdeck-cardkb needs no Python"
fi
if systemctl is-active --quiet cardkb.service 2>/dev/null; then
    ok "cardkb.service active"
else
    warn "cardkb.service not active (optional — zdeck-main.sh starts keyboard as fallback)"
fi

# 6. autostart state: getty drop-in + login hook (+ legacy DietPi cleanup)
echo "--- autostart state ---"
if [[ -f "$GETTY_DROPIN" ]] && grep -q "autologin $INSTALL_USER" "$GETTY_DROPIN"; then
    ok "getty autologin drop-in present ($GETTY_DROPIN -> $INSTALL_USER)"
elif compgen -G "$GETTY_DROPIN_DIR/*.conf" >/dev/null 2>&1; then
    warn "another getty@tty1 override exists (installer will ask before replacing):"
    grep -H "autologin\|ExecStart=" "$GETTY_DROPIN_DIR"/*.conf 2>/dev/null | sed 's/^/  [INFO] /' || true
else
    warn "no getty autologin yet — installer will add it (skip with --no-autologin)"
fi
PROFILE="$INSTALL_HOME/.bash_profile"
if [[ -f "$PROFILE" ]] && grep -q "zdeck/zdeck-main.sh" "$PROFILE"; then
    ok "login hook already in $PROFILE (tty1 -> zdeck-main.sh)"
elif [[ -f "$PROFILE" ]] && grep -q "zdeck/zdeck-auto.sh" "$PROFILE"; then
    warn "legacy hook (zdeck-auto.sh) in $PROFILE — installer upgrades it"
else
    warn "no login hook yet — installer will add it"
fi
DIETPI_CUSTOM="$(dietpi_custom_path)"
if [[ -n "$DIETPI_CUSTOM" && -f "$DIETPI_CUSTOM" ]] && grep -q "Z-DECK" "$DIETPI_CUSTOM"; then
    warn "legacy DietPi custom.sh entry found — installer cleans it up: $DIETPI_CUSTOM"
fi

echo
echo "=== result: $pass ok, $warn_n warnings, $fail_n failures ==="
if [[ "$fail_n" -gt 0 ]]; then
    echo "Fix FAILs above, then re-run. (Build: ./rust/build-pi.sh arm64)" >&2
    exit 1
fi

if [[ "$CHECK_ONLY" -eq 1 ]]; then
    echo "Check-only mode: no changes made."
    exit 0
fi

# ---------------------------------------------------------------- install
echo
echo "==> [1/4] Installing binaries + scripts -> $DEST"
mkdir -p "$DEST"
for b in zdeck-run zdeck-game zdeck-gps zdeck-fetch zdeck-cardkb; do
    cp -f "$SRC_BIN/$b" "$DEST/$b"
    chmod +x "$DEST/$b"
done
cp -f "$SRC_OS/zdeck-auto.sh" "$DEST/zdeck-auto.sh"
chmod +x "$DEST/zdeck-auto.sh"
if [[ -f "$SRC_OS/zdeck-main.sh" ]]; then
    cp -f "$SRC_OS/zdeck-main.sh" "$DEST/zdeck-main.sh"
    chmod +x "$DEST/zdeck-main.sh"
else
    warn "zdeck-main.sh not in repo — skipping copy (already warned above)"
fi
# Make a CardKB driver available next to the game so zdeck-main.sh can
# start it even when cardkb.service isn't installed. Rust binary first
# (no Python), Python script as fallback.
if [[ -x "$DEST/zdeck-cardkb" ]]; then
    echo "  [OK]   zdeck-cardkb (Rust, no Python) -> $DEST/"
fi
if [[ -f "$SRC_CARDKB" ]]; then
    cp -f "$SRC_CARDKB" "$DEST/cardkb_keyboard.py"
    chmod 644 "$DEST/cardkb_keyboard.py"
    echo "  [OK]   cardkb_keyboard.py (Python fallback) -> $DEST/"
elif [[ -f "$DEST/cardkb_keyboard.py" ]]; then
    echo "  [OK]   cardkb_keyboard.py already in $DEST/"
elif [[ ! -x "$DEST/zdeck-cardkb" ]]; then
    echo "  [WARN] no CardKB driver staged — keyboard unavailable until you build (./rust/build-pi.sh arm64) or run cardkb/setup.sh"
fi
ls -la "$DEST"

# ---------------------------------------------------------------- config
echo "==> [2/4] Ensuring UART (GPS) + I2C (CardKB) in Pi config"
if [[ -z "$CFG" ]]; then
    warn "no Pi config file — skipping (set enable_uart=1 + dtparam=i2c_arm=on manually)"
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
        echo "  UART change needs a reboot."
    fi
    if grep -Eq '^\s*dtparam=i2c_arm=on' "$CFG"; then
        echo "  dtparam=i2c_arm=on already in $CFG"
    else
        echo "  adding dtparam=i2c_arm=on to $CFG"
        if grep -Eq '^\s*#?\s*dtparam=i2c_arm=' "$CFG"; then
            sudo sed -i -E 's/^\s*#?\s*dtparam=i2c_arm=.*/dtparam=i2c_arm=on/' "$CFG"
        else
            echo -e "\n# Enabled by Z-DECK installer (CardKB)\ndtparam=i2c_arm=on" | sudo tee -a "$CFG" >/dev/null
        fi
        echo "  I2C change needs a reboot."
    fi
fi

# ------------------------------------------------- autostart (standard Debian)
echo "==> [3/4] Autostart: getty autologin + login hook"
PROFILE="$INSTALL_HOME/.bash_profile"

# 3a. Clean up the superseded DietPi-custom.sh method, if we installed it.
DIETPI_CUSTOM="$(dietpi_custom_path)"
if [[ -n "$DIETPI_CUSTOM" && -f "$DIETPI_CUSTOM" ]] && grep -q "Z-DECK" "$DIETPI_CUSTOM"; then
    echo "  Removing legacy DietPi custom.sh entry ($DIETPI_CUSTOM)..."
    if [[ -f "$DIETPI_CUSTOM.zdeck-bak" ]]; then
        sudo cp -f "$DIETPI_CUSTOM.zdeck-bak" "$DIETPI_CUSTOM"
        echo "  restored pre-Z-DECK backup."
    else
        printf '#!/bin/dash\n# DietPi-AutoStart custom script\n# Location: %s\n\nexit 0\n' "$DIETPI_CUSTOM" | sudo tee "$DIETPI_CUSTOM" >/dev/null
        echo "  reset to DietPi default (exit 0). Reselect your dietpi-autostart mode if you use it."
    fi
fi

# 3b. Console auto-login via getty override (the raspi-config mechanism,
# written directly so it works on DietPi, Pi OS and plain Debian alike).
if [[ "$NO_AUTOLOGIN" -eq 1 ]]; then
    echo "  --no-autologin: skipping (deck starts after YOUR manual login)."
elif [[ -f "$GETTY_DROPIN" ]] && grep -q "autologin $INSTALL_USER" "$GETTY_DROPIN"; then
    echo "  autologin already configured: $GETTY_DROPIN"
else
    WRITE_DROPIN=1
    OTHER="$(grep -rl "autologin" "$GETTY_DROPIN_DIR" 2>/dev/null | grep -v "zdeck-autologin.conf" || true)"
    if [[ -n "$OTHER" ]]; then
        echo "  Other autologin override(s) already exist:"
        echo "$OTHER" | sed 's/^/    /'
        if [[ "$YES" -eq 1 ]] || ask_yes "  Replace with the Z-DECK one (user $INSTALL_USER)?"; then
            WRITE_DROPIN=1
        else
            echo "  keeping existing autologin; the hook below still starts the deck after login."
            WRITE_DROPIN=0
        fi
    fi
    if [[ "$WRITE_DROPIN" -eq 1 && "$INSTALL_USER" == "root" && "$YES" -eq 0 ]]; then
        ask_yes "  Auto-login as ROOT is insecure — continue anyway?" || WRITE_DROPIN=0
    fi
    if [[ "$WRITE_DROPIN" -eq 1 ]]; then
        sudo mkdir -p "$GETTY_DROPIN_DIR"
        sudo cp -n "$GETTY_DROPIN" "$GETTY_DROPIN.zdeck-bak" 2>/dev/null || true
        # shellcheck disable=SC2059
        printf "[Service]\nExecStart=\nExecStart=-/sbin/agetty --autologin $INSTALL_USER --noclear %%I linux\n" | sudo tee "$GETTY_DROPIN" >/dev/null
        sudo systemctl daemon-reload || true
        echo "  wrote $GETTY_DROPIN (autologin $INSTALL_USER on tty1, takes effect next boot)"
    fi
fi

# 3c. Login hook — RUNS the deck as a child on tty1 (never execs, SSH
# untouched). Quit chain: launcher exits 42 -> game loop stops -> main
# ends -> hook returns -> persistent login shell. Log out to come back.
echo "  Installing login hook -> $DEST/zdeck-main.sh ..."
HOOK='if [ -z "${ZDECK_ACTIVE:-}" ] && [ -z "${SSH_CONNECTION:-}" ] && [ "$(tty 2>/dev/null)" = "/dev/tty1" ]; then "$HOME/zdeck/zdeck-main.sh"; fi'
touch "$PROFILE"
if grep -q 'zdeck/zdeck-main.sh' "$PROFILE"; then
    if grep -qF 'exec "$HOME/zdeck/zdeck-main.sh"' "$PROFILE"; then
        cp -n "$PROFILE" "$PROFILE.zdeck-bak" || true
        sed -i 's|exec "\$HOME/zdeck/zdeck-main.sh"|"$HOME/zdeck/zdeck-main.sh"|' "$PROFILE"
        echo "  hook updated to run-as-child (was exec) in $PROFILE"
    else
        echo "  hook already present in $PROFILE"
    fi
else
    # upgrade legacy hook if present
    if grep -q 'zdeck/zdeck-auto.sh' "$PROFILE"; then
        cp -n "$PROFILE" "$PROFILE.zdeck-bak" || true
        grep -v 'zdeck/zdeck-auto.sh' "$PROFILE" > "$PROFILE.tmp" || true
        mv "$PROFILE.tmp" "$PROFILE"
        echo "  removed legacy zdeck-auto.sh hook (backup: $PROFILE.zdeck-bak)"
    fi
    {
        echo ""
        echo "# Z-DECK: run the deck after login on the local console."
        echo "# Child process (no exec): quitting the deck drops back to this shell."
        echo "$HOOK"
    } >> "$PROFILE"
    echo "  hook added to $PROFILE"
fi

# ------------------------------------------------- optional systemd unit
echo "==> [4/4] zdeck.service (opt-in systemd alternative)"
if [[ "$WITH_SYSTEMD" -eq 1 ]]; then
    if ask_yes "Install + enable zdeck.service (starts at boot WITHOUT login)?"; then
        ESCAPED_DEST="$(printf '%s' "$DEST" | sed 's/[\/&]/\\&/g')"
        sed -e "s|^User=.*|User=$INSTALL_USER|" \
            -e "s|^WorkingDirectory=.*|WorkingDirectory=$DEST|" \
            -e "s|^ExecStart=.*|ExecStart=$ESCAPED_DEST/zdeck-main.sh --auto|" \
            "$SRC_OS/zdeck.service" | sudo tee /etc/systemd/system/zdeck.service >/dev/null
        sudo systemctl daemon-reload
        sudo systemctl enable --now zdeck.service
        echo "  zdeck.service enabled. NOTE: disable the login hook to avoid double-launch:"
        echo "    $0 --uninstall   # removes hook, keeps service"
    else
        echo "  skipped."
    fi
else
    echo "  skipped (login hook is the default). To use systemd instead:"
    echo "    sudo cp $SRC_OS/zdeck.service /etc/systemd/system/  # edit User=/paths first"
    echo "    sudo systemctl enable --now zdeck"
    echo "  WARNING: running hook + service together launches the game twice."
    if systemctl is-enabled zdeck.service >/dev/null 2>&1; then
        warn "zdeck.service is currently ENABLED alongside the login hook — pick one"
    fi
fi

echo
echo "Done. Next steps:"
echo "  1. sudo reboot — getty auto-logs in $INSTALL_USER on tty1, deck menu appears."
echo "     (Used --no-autologin? Then just log in on the console instead.)"
echo "  2. Menu: Up/Down + Enter — Start / Settings / Quit to terminal."
echo "     Quit lands on a persistent shell; log out to return to the menu."
echo "  3. Test now without reboot:  ~/zdeck/zdeck-main.sh --sim"
echo "  4. Checks:  ~/zdeck/zdeck-main.sh --check-only   |   $SRC_OS/install-autostart.sh --check-only"
echo "  5. Reboot again only if UART/I2C config changed."
echo
echo "Maintenance: SSH in (hook fires on /dev/tty1 only, never over SSH),"
echo "or Ctrl+C during the 8 s restart pause on the console."
echo "Remove autostart: $0 --uninstall"
