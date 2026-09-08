#!/usr/bin/env bash
# Z-DECK MAIN run script — single entry point that links everything.
#
#   CardKB virtual keyboard  +  zdeck-auto.sh -> zdeck-run -> game
#
# This is what the ~/.bash_profile login hook runs on /dev/tty1, and what
# zdeck.service runs when you opt into systemd. Direct use:
#   ~/zdeck/zdeck-main.sh              # splash + launcher menu
#   ~/zdeck/zdeck-main.sh --sim        # indoor testing, no GPS
#   ~/zdeck/zdeck-main.sh --check-only # checks only, starts nothing
#   ~/zdeck/zdeck-main.sh --no-cardkb  # skip keyboard, game only
#
# CardKB strategy (in order):
#   1. If cardkb.service is active, do nothing (it owns the keyboard).
#   2. Else if a CardKB device is already registered, do nothing.
#   3. Else start the staged driver in the background, log to $ZDIR/cardkb.log:
#      Rust $ZDIR/zdeck-cardkb first (no Python), else cardkb_keyboard.py.
# The background driver is killed on exit.

set -u

ZDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTO="$ZDIR/zdeck-auto.sh"

# Sentinel: the tty1 login hook skips itself while this is set, so nested
# login shells (and the shell that "Quit to terminal" lands on) don't
# relaunch the deck.
export ZDECK_ACTIVE=1

ARGS=()
CHECK_ONLY=0
NO_CARDKB=0
for a in "$@"; do
    case "$a" in
        --check-only) CHECK_ONLY=1 ;;
        --no-cardkb) NO_CARDKB=1 ;;
        -h|--help)
            sed -n '2,12p' "$0"
            exit 0
            ;;
        *) ARGS+=("$a") ;;
    esac
done

pass=0; warn_n=0
ok()   { pass=$((pass+1)); echo "  [OK]   $*"; }
warn() { warn_n=$((warn_n+1)); echo "  [WARN] $*"; }

CARDKB_PID=""
cleanup() {
    if [[ -n "$CARDKB_PID" ]] && kill -0 "$CARDKB_PID" 2>/dev/null; then
        kill "$CARDKB_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

find_driver() {  # echo "rust|python <path>" or empty; Rust preferred
    for c in "$ZDIR/zdeck-cardkb" "$HOME/zdeck/zdeck-cardkb"; do
        [[ -x "$c" ]] && { echo "rust $c"; return 0; }
    done
    if command -v zdeck-cardkb >/dev/null 2>&1; then
        echo "rust $(command -v zdeck-cardkb)"
        return 0
    fi
    for c in "$ZDIR/cardkb_keyboard.py" "$HOME/cardkb_keyboard.py" \
             "$ZDIR/../../cardkb/cardkb_keyboard.py"; do
        [[ -f "$c" ]] && { echo "python $c"; return 0; }
    done
    echo ""
}

cardkb_registered() {
    grep -q "CardKB" /proc/bus/input/devices 2>/dev/null
}

start_cardkb() {
    if [[ "$NO_CARDKB" -eq 1 ]]; then
        echo "  [INFO] --no-cardkb: skipping keyboard."
        return 0
    fi
    if systemctl is-active --quiet cardkb.service 2>/dev/null; then
        ok "cardkb.service active — keyboard owned by systemd"
        return 0
    fi
    if cardkb_registered; then
        ok "CardKB device already registered"
        return 0
    fi
    local kind drv
    read -r kind drv <<<"$(find_driver)"
    if [[ -z "$kind" ]]; then
        warn "no CardKB driver found — game starts without it (build: ./rust/build-pi.sh arm64)"
        return 0
    fi
    if [[ ! -e /dev/i2c-1 ]]; then
        warn "/dev/i2c-1 missing — cannot start CardKB (enable I2C + reboot)"
        return 0
    fi
    if [[ "$kind" == "python" ]] && ! python3 -c "import smbus2, evdev" 2>/dev/null; then
        warn "smbus2/evdev missing — cannot start Python CardKB fallback (Rust zdeck-cardkb needs no Python)"
        return 0
    fi
    if ! groups 2>/dev/null | tr ' ' '\n' | grep -qx input; then
        warn "user '$USER' not in 'input' group — CardKB may fail (sudo usermod -aG input \$USER)"
    fi
    echo "  [INFO] starting CardKB driver ($kind): $drv (log: $ZDIR/cardkb.log)"
    if [[ "$kind" == "rust" ]]; then
        nohup "$drv" >>"$ZDIR/cardkb.log" 2>&1 &
    else
        nohup python3 "$drv" >>"$ZDIR/cardkb.log" 2>&1 &
    fi
    CARDKB_PID=$!
    sleep 1
    if cardkb_registered; then
        ok "CardKB driver started (pid $CARDKB_PID)"
    elif kill -0 "$CARDKB_PID" 2>/dev/null; then
        ok "CardKB driver running in background (pid $CARDKB_PID, device pending)"
    else
        warn "CardKB driver exited — see $ZDIR/cardkb.log"
        CARDKB_PID=""
    fi
}

echo "=== Z-DECK main ==="
echo "--- linked components ---"
[[ -x "$AUTO" ]] && ok "game loop: $AUTO" || warn "missing $AUTO — run install-autostart.sh"
[[ -x "$ZDIR/zdeck-run" ]] && ok "game binary: $ZDIR/zdeck-run" || warn "missing $ZDIR/zdeck-run"
DRV="$(find_driver)"
[[ -n "$DRV" ]] && ok "keyboard driver ($DRV)" || warn "no keyboard driver staged"
if systemctl is-active --quiet cardkb.service 2>/dev/null; then ok "keyboard service: active"
else echo "  [INFO] keyboard service: inactive (fallback: background driver)"
fi
[[ -e /dev/ttyAMA0 ]] && ok "GPS device: /dev/ttyAMA0" || echo "  [INFO] GPS device: absent (--sim still works)"
ls /dev/i2c* >/dev/null 2>&1 && ok "I2C: $(ls /dev/i2c* 2>/dev/null | tr '\n' ' ')" \
    || echo "  [INFO] I2C: absent (keyboard unavailable)"
echo "---"

[[ "$CHECK_ONLY" -eq 1 ]] && { echo "Check-only: not starting anything."; exit 0; }

start_cardkb

echo "Starting game loop ..."
exec "$AUTO" "${ARGS[@]}"
