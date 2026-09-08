#!/usr/bin/env bash
# Z-DECK autostart installer — Raspberry Pi 3, 64-bit OS only.
#
# What it does (run ON the Pi, from this repo):
#   1. CHECKS everything: arch, binaries, UART (/dev/ttyAMA0), I2C
#      (/dev/i2c-1 + CardKB 0x5F), uinput, CardKB script/service,
#      python deps (smbus2/evdev).
#   2. Copies binary/pi3-arm64/{zdeck-run,zdeck-game,zdeck-gps,zdeck-fetch,zdeck-cardkb}
#      + os/pi3-arm64/{zdeck-auto.sh,zdeck-main.sh} [+ cardkb_keyboard.py]
#      -> ~/zdeck/  (chmod +x)
#   3. Sets up auto-start. On DietPi (detected via /boot/dietpi or an
#      existing custom.sh) it writes the launch command into DietPi's
#      custom autostart file (/var/lib/dietpi-autostart/custom.sh on old
#      images, /var/lib/dietpi/dietpi-autostart/custom.sh on new ones) —
#      then select "Custom script (foreground, with auto login)" in
#      `sudo dietpi-autostart`. On other systems it uses a ~/.bash_profile
#      hook on /dev/tty1 (-> zdeck-main.sh -> CardKB keyboard + game).
#      No auto-login is configured here — set that up yourself if you want
#      it (see notes at the end).
#   4. Ensures enable_uart=1 (GPS on /dev/ttyAMA0) and dtparam=i2c_arm=on
#      (CardKB on /dev/i2c-1). Does NOT touch bluetooth overlays.
#
# Usage on the Pi:
#   ./os/pi3-arm64/install-autostart.sh
#   ./os/pi3-arm64/install-autostart.sh --yes
#   ./os/pi3-arm64/install-autostart.sh --check-only   # checks, changes nothing
#   ./os/pi3-arm64/install-autostart.sh --uninstall    # remove autostart again
#        (login hook and/or DietPi custom.sh entry)
#   ./os/pi3-arm64/install-autostart.sh --with-systemd # ALSO install+enable
#        zdeck.service (opt-in; default is login-hook only so the game
#        doesn't launch twice).
#
# After install: DietPi -> pick the Custom foreground mode in
# `sudo dietpi-autostart` and reboot; other systems -> log in on the Pi
# console (tty1) and the deck starts. SSH sessions are never hooked.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC_BIN="$REPO_ROOT/binary/pi3-arm64"
SRC_OS="$REPO_ROOT/os/pi3-arm64"
SRC_CARDKB="$REPO_ROOT/cardkb/cardkb_keyboard.py"
DEST="$HOME/zdeck"

YES=0
CHECK_ONLY=0
UNINSTALL=0
WITH_SYSTEMD=0
for arg in "$@"; do
    case "$arg" in
        -y|--yes) YES=1 ;;
        --check-only) CHECK_ONLY=1 ;;
        --uninstall) UNINSTALL=1 ;;
        --with-systemd) WITH_SYSTEMD=1 ;;
        -h|--help)
            sed -n '2,22p' "$0"
            exit 0
            ;;
        *) echo "Unknown arg: $arg (try --help)" >&2; exit 1 ;;
    esac
done

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

dietpi_custom_path() {  # echo DietPi custom.sh path, or empty if not DietPi
    # Old images: /var/lib/dietpi-autostart/custom.sh (yours),
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
    PROFILE="$HOME/.bash_profile"
    if [[ -f "$PROFILE" ]] && grep -q "zdeck/zdeck-main.sh\|zdeck/zdeck-auto.sh" "$PROFILE"; then
        remove_profile_hook "$PROFILE"
        echo "Removed Z-DECK hook from $PROFILE."
    else
        echo "No Z-DECK hook in ${PROFILE:-$HOME/.bash_profile}."
    fi
    DCUSTOM="$(dietpi_custom_path)"
    if [[ -n "$DCUSTOM" && -f "$DCUSTOM" ]] && grep -q "Z-DECK" "$DCUSTOM"; then
        if [[ -f "$DCUSTOM.zdeck-bak" ]]; then
            sudo cp -f "$DCUSTOM.zdeck-bak" "$DCUSTOM"
            echo "Restored $DCUSTOM from backup."
        else
            printf '#!/bin/dash\n# DietPi-AutoStart custom script\n# Location: %s\n\nexit 0\n' "$DCUSTOM" | sudo tee "$DCUSTOM" >/dev/null
            echo "Reset $DCUSTOM to the DietPi default (exit 0)."
        fi
    elif [[ -n "$DCUSTOM" ]]; then
        echo "No Z-DECK entry in $DCUSTOM."
    fi
    if systemctl is-enabled zdeck.service >/dev/null 2>&1; then
        echo "Disabling zdeck.service ..."
        sudo systemctl disable --now zdeck.service || true
    fi
    exit 0
fi

echo "=== Z-DECK preflight checks ==="

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
if id -nG 2>/dev/null | tr ' ' '\n' | grep -qx input; then ok "user '$USER' in 'input' group"
else warn "user '$USER' NOT in 'input' group (cardkb/setup.sh: sudo usermod -aG input \$USER)"
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

# 6. autostart state (DietPi custom.sh and/or login hook) + auto-login note
echo "--- autostart state ---"
DIETPI_CUSTOM="$(dietpi_custom_path)"
if [[ -n "$DIETPI_CUSTOM" ]]; then
    echo "  [INFO] DietPi detected — custom.sh path: $DIETPI_CUSTOM"
    if [[ -f "$DIETPI_CUSTOM" ]] && grep -q "zdeck/zdeck-main.sh" "$DIETPI_CUSTOM"; then
        ok "DietPi custom.sh already launches zdeck-main.sh"
    else
        warn "DietPi custom.sh has no Z-DECK entry — installer will write it"
        if [[ -f "$DIETPI_CUSTOM" ]]; then
            echo "  [INFO] current content: $(head -c 120 "$DIETPI_CUSTOM" | tr '\n' '|')"
        fi
    fi
fi
PROFILE="$HOME/.bash_profile"
if [[ -f "$PROFILE" ]] && grep -q "zdeck/zdeck-main.sh" "$PROFILE"; then
    if [[ -n "$DIETPI_CUSTOM" ]]; then
        warn "login hook in $PROFILE AND DietPi mode — installer keeps ONE (no double launch)"
    else
        ok "login hook already in $PROFILE (tty1 -> zdeck-main.sh)"
    fi
elif [[ -f "$PROFILE" ]] && grep -q "zdeck/zdeck-auto.sh" "$PROFILE"; then
    warn "legacy hook (zdeck-auto.sh) in $PROFILE — installer upgrades it to zdeck-main.sh"
elif [[ -z "$DIETPI_CUSTOM" ]]; then
    warn "no login hook yet — installer will add it"
fi
echo "  [INFO] This script does NOT configure auto-login. If you want the deck"
echo "  [INFO] to boot straight into the game, enable console auto-login yourself:"
echo "  [INFO]   sudo raspi-config  -> System Options -> Boot / Auto Login -> Console Autologin"

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

# ------------------------------------------------- autostart method
if [[ -n "$DIETPI_CUSTOM" ]]; then
    echo "==> [3/4] DietPi autostart -> $DIETPI_CUSTOM"
    echo "  DietPi runs this file after auto-login on the main screen (foreground)."
    METHOD="dietpi"
    if grep -q "zdeck/zdeck-main.sh" "$PROFILE" 2>/dev/null; then
        echo "  A ~/.bash_profile hook also exists — pick ONE method (both = game launches twice)."
        if [[ "$YES" -eq 1 ]] || ask_yes "  Use DietPi custom.sh and remove the profile hook?"; then
            METHOD="dietpi"
        else
            METHOD="hook"
        fi
    elif [[ "$YES" -eq 0 ]]; then
        echo "  Methods:"
        echo "    1) DietPi custom.sh (recommended on DietPi)"
        echo "    2) ~/.bash_profile hook instead"
        read -r -p "  Pick [1/2] " pick || true
        [[ "${pick:-1}" == "2" ]] && METHOD="hook" || METHOD="dietpi"
    fi
else
    METHOD="hook"
fi

if [[ "$METHOD" == "dietpi" ]]; then
    echo "  Writing Z-DECK launch into $DIETPI_CUSTOM ..."
    sudo mkdir -p "$(dirname "$DIETPI_CUSTOM")"
    if [[ -f "$DIETPI_CUSTOM" ]] && ! grep -q "Z-DECK" "$DIETPI_CUSTOM"; then
        sudo cp -n "$DIETPI_CUSTOM" "$DIETPI_CUSTOM.zdeck-bak" || true
        echo "  (your old custom.sh backed up to $DIETPI_CUSTOM.zdeck-bak)"
    fi
    {
        echo "#!/bin/dash"
        echo "# Z-DECK kiosk (managed by install-autostart.sh, $(date -u +%F))."
        echo "# Needs DietPi-AutoStart mode: Custom script (foreground, with auto login)."
        echo "# Runs after auto-login on the main screen. exec: when the deck exits,"
        echo "# the session ends and DietPi logs straight back in (crash = relaunch)."
        echo "exec \"$DEST/zdeck-main.sh\""
    } | sudo tee "$DIETPI_CUSTOM" >/dev/null
    sudo chmod +x "$DIETPI_CUSTOM"
    echo "  wrote $DIETPI_CUSTOM:"
    cat "$DIETPI_CUSTOM" | sed 's/^/    /'
    # The profile hook would double-launch alongside custom.sh — drop it.
    remove_profile_hook "$PROFILE"
    echo
    echo "  NEXT (required): sudo dietpi-autostart  ->  Autostart Options"
    echo "     ->  'Custom script (foreground, with auto login)'  -> reboot."
    echo "  (No auto-login option is changed here — that DietPi mode brings its own.)"
else
    echo "==> [3/4] Setting up auto-start AFTER LOGIN (tty1 -> zdeck-main.sh)"
    echo "  (No auto-login is configured — the hook only fires once you log in.)"
    HOOK='if [ -z "${SSH_CONNECTION:-}" ] && [ "$(tty 2>/dev/null)" = "/dev/tty1" ]; then exec "$HOME/zdeck/zdeck-main.sh"; fi'
    touch "$PROFILE"
    if grep -q 'zdeck/zdeck-main.sh' "$PROFILE"; then
        echo "  hook already present in $PROFILE"
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
            echo "# Z-DECK kiosk: start the deck after login on the local console."
            echo "# (Auto-login is NOT managed here — enable it via raspi-config if wanted.)"
            echo "$HOOK"
        } >> "$PROFILE"
        echo "  hook added to $PROFILE"
    fi
fi

# ------------------------------------------------- optional systemd unit
echo "==> [4/4] zdeck.service (opt-in systemd alternative)"
if [[ "$WITH_SYSTEMD" -eq 1 ]]; then
    if ask_yes "Install + enable zdeck.service (starts at boot WITHOUT login)?"; then
        ESCAPED_DEST="$(printf '%s' "$DEST" | sed 's/[\/&]/\\&/g')"
        sed -e "s|^User=.*|User=$USER|" \
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
if [[ "${METHOD:-hook}" == "dietpi" ]]; then
    echo "  1. sudo dietpi-autostart -> 'Custom script (foreground, with auto login)'."
    echo "  2. sudo reboot — the deck starts on the main screen after auto-login."
else
    echo "  1. Log in on the Pi console (tty1) — the deck starts automatically."
    echo "     (For boot-to-game with no password prompt, enable auto-login manually:"
    echo "      DietPi: sudo dietpi-autostart | Pi OS: raspi-config -> Boot / Auto Login)"
fi
echo "  2. Test now without reboot:  ~/zdeck/zdeck-main.sh --sim"
echo "  3. Checks:  ~/zdeck/zdeck-main.sh --check-only   |   $SRC_OS/install-autostart.sh --check-only"
echo "  4. Reboot only if UART/I2C config changed:  sudo reboot"
echo
echo "Maintenance: SSH in (hook fires on /dev/tty1 only), or Ctrl+C during the"
echo "8 s restart pause on the console. Remove autostart: $0 --uninstall"
