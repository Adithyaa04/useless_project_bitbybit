#!/usr/bin/env bash
# CardKB diagnostic — finds WHY no keypresses show up.
# Read-only (changes nothing), unless --live is passed.
#
# Usage on the Pi:
#   sudo bash cardkb/diag.sh              # full report + verdict
#   sudo bash cardkb/diag.sh --live       # + 10 s live keypress test
#   sudo bash cardkb/diag.sh --user dietpi
set -uo pipefail

USER_ARG=""
LIVE=0
while [ $# -gt 0 ]; do
    case "$1" in
        --user) USER_ARG="${2:-}"; shift 2 ;;
        --user=*) USER_ARG="${1#--user=}"; shift ;;
        --live) LIVE=1; shift ;;
        -h|--help) sed -n '2,9p' "$0"; exit 0 ;;
        *) echo "Unknown arg: $1 (try --help)" >&2; exit 1 ;;
    esac
done

TARGET="${USER_ARG:-${SUDO_USER:-}}"
[ -n "$TARGET" ] || TARGET="$(whoami)"
if [ "$TARGET" = "root" ]; then
    # Service never runs as root by our setup; guess the console user.
    for u in dietpi pi; do
        if id "$u" >/dev/null 2>&1; then TARGET="$u"; break; fi
    done
fi
THOME="$(getent passwd "$TARGET" | cut -d: -f6 2>/dev/null)"
[ -n "$THOME" ] || THOME="$HOME"

pass=0; warn_n=0; fail_n=0
ok()   { pass=$((pass+1)); echo "  [PASS] $*"; }
warn() { warn_n=$((warn_n+1)); echo "  [WARN] $*"; }
fail() { fail_n=$((fail_n+1)); echo "  [FAIL] $*"; }
info() { echo "  [info] $*"; }

echo "=== CardKB diagnostic (user=$TARGET home=$THOME) ==="
[ "$(id -u)" -eq 0 ] || echo "  [info] not root — device permission tests may be incomplete; prefer sudo."

# 1. config.txt
echo "--- 1. boot config ---"
CFG=""
for c in /boot/firmware/config.txt /boot/config.txt; do
    [ -f "$c" ] && CFG="$c" && break
done
if [ -z "$CFG" ]; then
    fail "no Pi config file found"
else
    grep -Eq '^\s*dtparam=i2c_arm=on' "$CFG" \
        && ok "dtparam=i2c_arm=on in $CFG" \
        || fail "I2C not enabled in $CFG (run setup.sh, then reboot)"
fi

# 2. kernel modules
echo "--- 2. kernel modules ---"
for m in i2c-dev i2c-bcm2835 uinput; do
    if lsmod 2>/dev/null | grep -q "^${m//-/[_-]}"; then
        ok "$m loaded"
    else
        fail "$m NOT loaded"
    fi
    grep -Eq "^${m}$" /etc/modules 2>/dev/null \
        && info "$m persists in /etc/modules" \
        || warn "$m missing from /etc/modules (won't survive reboot)"
done

# 3. I2C bus + address
echo "--- 3. I2C bus ---"
if [ -e /dev/i2c-1 ]; then
    ok "/dev/i2c-1 exists ($(ls -l /dev/i2c-1 | awk '{print $1, $3":"$4}'))"
    if sudo -u "$TARGET" test -r /dev/i2c-1 -a -w /dev/i2c-1 2>/dev/null; then
        ok "$TARGET can read+write /dev/i2c-1"
    else
        fail "$TARGET CANNOT access /dev/i2c-1 (need: sudo usermod -aG i2c $TARGET + reboot)"
    fi
    if command -v i2cdetect >/dev/null 2>&1; then
        SCAN="$(i2cdetect -y 1 2>&1 || true)"
        echo "$SCAN" | sed 's/^/    /'
        echo "$SCAN" | grep -qi "5f" \
            && ok "CardKB 0x5F present on bus 1" \
            || fail "0x5F NOT on bus 1 (wiring? CardKB powered? SDA->GPIO2 SCL->GPIO3?)"
    else
        warn "i2cdetect missing (sudo apt install i2c-tools)"
    fi
else
    fail "/dev/i2c-1 missing (enable I2C + reboot)"
fi

# 4. uinput + groups
echo "--- 4. uinput path ---"
[ -e /dev/uinput ] \
    && ok "/dev/uinput exists ($(ls -l /dev/uinput | awk '{print $1, $3":"$4}'))" \
    || fail "/dev/uinput missing (modprobe uinput failed?)"
[ -f /etc/udev/rules.d/99-uinput.rules ] \
    && ok "udev rule present: $(cat /etc/udev/rules.d/99-uinput.rules)" \
    || warn "99-uinput.rules missing"
for g in input i2c; do
    id "$TARGET" 2>/dev/null | grep -q "($g)" \
        && ok "$TARGET in '$g' group" \
        || fail "$TARGET NOT in '$g' group (sudo usermod -aG $g $TARGET + reboot)"
done

# 5. driver binary / fallback
echo "--- 5. driver ---"
DRV=""
[ -x "$THOME/zdeck/zdeck-cardkb" ] && DRV="$THOME/zdeck/zdeck-cardkb"
if [ -n "$DRV" ]; then
    ok "Rust driver: $DRV"
elif command -v zdeck-cardkb >/dev/null 2>&1; then
    DRV="$(command -v zdeck-cardkb)"; ok "Rust driver on PATH: $DRV"
else
    warn "no Rust driver at $THOME/zdeck/zdeck-cardkb"
    if sudo -u "$TARGET" python3 -c "import smbus2, evdev" 2>/dev/null; then
        ok "Python fallback imports OK"
        DRV="python3 $THOME/cardkb_keyboard.py"
    else
        fail "Python fallback BROKEN (import smbus2, evdev fails)"
    fi
fi

# 6. service
echo "--- 6. cardkb.service ---"
if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl is-enabled cardkb.service 2>/dev/null | grep -q enabled \
        && ok "service enabled" || warn "service NOT enabled"
    systemctl is-active --quiet cardkb.service \
        && ok "service active" || fail "service NOT active"
    echo "  [info] ExecStart: $(systemctl show cardkb.service -p ExecStart 2>/dev/null | head -1)"
    echo "  [info] User: $(systemctl show cardkb.service -p User 2>/dev/null | head -1)"
    echo "  [info] restarts: $(systemctl show cardkb.service -p NRestarts 2>/dev/null | head -1)"
    echo "  [info] --- journal (last 12) ---"
    journalctl -u cardkb.service --no-pager -n 12 2>/dev/null | sed 's/^/    /' || info "no journal"
else
    warn "no systemd (not a Pi boot?) — service checks skipped"
fi

# 7. registered input device
echo "--- 7. virtual keyboard registration ---"
if grep -q "CardKB" /proc/bus/input/devices 2>/dev/null; then
    ok "CardKB registered:"
    grep -A5 "CardKB" /proc/bus/input/devices | sed 's/^/    /'
    EV="$(grep -A5 "CardKB" /proc/bus/input/devices | grep -o 'event[0-9]*' | head -1)"
    [ -n "$EV" ] && info "event node: /dev/input/$EV"
else
    fail "CardKB NOT in /proc/bus/input/devices (driver never emitted successfully)"
    EV=""
fi

# 8. optional live test
if [ "$LIVE" -eq 1 ]; then
    echo "--- 8. live keypress test (10 s) ---"
    if [ -z "${EV:-}" ]; then
        # Re-scan in case driver started mid-diag
        EV="$(grep -A5 "CardKB" /proc/bus/input/devices 2>/dev/null | grep -o 'event[0-9]*' | head -1)"
    fi
    if [ -z "${EV:-}" ]; then
        fail "no event node — nothing to listen on. Fix sections above first."
    elif [ ! -r "/dev/input/$EV" ]; then
        fail "cannot read /dev/input/$EV (run diag with sudo)"
    else
        echo "  PRESS CardKB KEYS NOW for 10 s..."
        OUT="$(timeout 10 od -A d -t x1 "/dev/input/$EV" 2>/dev/null | head -6 || true)"
        if [ -n "$OUT" ]; then
            echo "$OUT" | sed 's/^/    /'
            ok "keypresses ARE reaching the kernel — if apps ignore them, check window focus (console, not SSH)"
        else
            fail "silence: presses never reached the kernel (wiring / address / driver stuck)"
        fi
    fi
fi

echo
echo "=== verdict: $pass pass, $warn_n warnings, $fail_n failures ==="
if [ "$fail_n" -eq 0 ] && [ "$warn_n" -eq 0 ]; then
    echo "All green — if keys still don't type, the app window simply lacks focus."
    echo "Focus a text editor on the CONSOLE (not SSH) and press keys."
elif ! sudo -u "$TARGET" test -r /dev/i2c-1 2>/dev/null; then
    echo "MOST LIKELY: permission on /dev/i2c-1. Fix: sudo usermod -aG i2c $TARGET && sudo reboot"
elif ! grep -q "CardKB" /proc/bus/input/devices 2>/dev/null; then
    echo "MOST LIKELY: driver crash-loop. Read: sudo journalctl -u cardkb.service -n 30"
    echo "Common: 'Permission denied' (/dev/i2c-1 or /dev/uinput) or 'No such file' (bad ExecStart)."
    echo "Fix perms/groups, or point the service at a present driver and: sudo systemctl restart cardkb"
else
    echo "Driver runs and device exists — check focus (console, not SSH) and try --live."
fi
