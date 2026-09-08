#!/usr/bin/env bash
# Z-DECK game loop — Raspberry Pi 3, 64-bit OS only.
#
# Launched BY zdeck-main.sh (which starts the CardKB keyboard first).
# Can also be run directly:  zdeck-auto.sh [--sim] [--check-only] [--no-loop]
#
# Checks everything it needs (binaries, GPS device, map, keyboard device),
# warns but never hard-fails on OPTIONAL hardware — SIM mode always works.
# Kiosk loop: when the game exits, wait 8 s (Ctrl+C drops to a shell for
# maintenance) and start over, so the deck never sits on a bare prompt.

set -u

ZDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN="$ZDIR/zdeck-run"

ARGS=()
CHECK_ONLY=0
NO_LOOP=0
for a in "$@"; do
    case "$a" in
        --check-only) CHECK_ONLY=1 ;;
        --no-loop) NO_LOOP=1 ;;
        -h|--help)
            echo "Usage: $(basename "$0") [--sim] [--auto] [--check-only] [--no-loop]"
            echo "  --sim        pass through to zdeck-run (indoor testing, no GPS)"
            echo "  --check-only run preflight checks and exit"
            echo "  --no-loop    run the game once instead of looping"
            exit 0
            ;;
        *) ARGS+=("$a") ;;
    esac
done

pass=0; warn_n=0
ok()   { pass=$((pass+1)); echo "  [OK]   $*"; }
warn() { warn_n=$((warn_n+1)); echo "  [WARN] $*"; }

preflight() {
    echo "Z-DECK preflight (game):"
    # 1. binaries must sit next to this script (~/zdeck)
    local missing=0
    for b in zdeck-run zdeck-game zdeck-gps zdeck-fetch; do
        if [[ -x "$ZDIR/$b" ]]; then ok "$b executable"
        else warn "missing $ZDIR/$b — run install-autostart.sh"; missing=1
        fi
    done
    # 2. GPS hardware (optional — SIM works without it)
    if [[ -e /dev/ttyAMA0 ]]; then ok "/dev/ttyAMA0 present (GPS)"
    else warn "/dev/ttyAMA0 missing — AUTO mode needs enable_uart=1 + reboot; --sim still works"
    fi
    # 3. I2C / CardKB (optional — game runs without it, you just type less)
    if ls /dev/i2c* >/dev/null 2>&1; then ok "I2C: $(ls /dev/i2c* 2>/dev/null | tr '\n' ' ')"
    else warn "no /dev/i2c* — CardKB unavailable (enable I2C for the keyboard)"
    fi
    if grep -q "CardKB" /proc/bus/input/devices 2>/dev/null; then
        ok "CardKB virtual keyboard registered"
    else
        warn "CardKB input device not registered (zdeck-main.sh starts it; check cardkb.service)"
    fi
    if [[ -x "$ZDIR/zdeck-cardkb" ]]; then ok "zdeck-cardkb Rust driver staged (no Python needed)"
    elif [[ -f "$ZDIR/cardkb_keyboard.py" ]]; then echo "  [INFO] Python CardKB fallback staged"
    else echo "  [INFO] no CardKB driver staged — game still runs, keyboard via zdeck-main.sh/service when present"
    fi
    # 4. map data (zdeck-run fetches its own live map, so only informational)
    if ls "$ZDIR"/*.json "$ZDIR"/map* >/dev/null 2>&1; then ok "map data present in $ZDIR"
    else echo "  [INFO] no cached map in $ZDIR — zdeck-run fetches a fresh one around the GPS fix"
    fi
    # 5. console sanity
    if [[ "${TERM:-}" == *"256color"* || "${TERM:-}" == "linux" ]]; then ok "TERM=$TERM"
    else warn "TERM=${TERM:-unset} — colours may be off (export TERM=xterm-256color)"
    fi
    echo "preflight: $pass ok, $warn_n warnings"
    # shellcheck disable=SC2086
    return $missing
}

if ! preflight; then
    echo "Z-DECK: required binaries missing — not starting the loop." >&2
    echo "  Run install-autostart.sh, or copy binary/pi3-arm64/* to $ZDIR/ and chmod +x." >&2
    [[ "$CHECK_ONLY" -eq 1 ]] && exit 1
    sleep 5
    exit 1
fi
[[ "$CHECK_ONLY" -eq 1 ]] && exit 0

export TERM="${TERM:-xterm-256color}"
cd "$ZDIR" || exit 1

# Console finishing touches (best effort, never fatal).
clear
stty sane 2>/dev/null || true
setterm -blank 0 -powersave off -cursor on 2>/dev/null || true

echo "Z-DECK game loop (pi3-arm64): starting zdeck-run ${ARGS[*]:-"(splash + AUTO/SIM countdown)"} ..."

if [[ "$NO_LOOP" -eq 1 ]]; then
    exec "$RUN" "${ARGS[@]}"
fi

while true; do
    # No args = splash + 5 s AUTO/SIM countdown (AUTO wins on timeout).
    # Pass-through allows `zdeck-auto.sh --sim` for indoor testing.
    "$RUN" "${ARGS[@]}"
    code=$?
    echo
    echo "Z-DECK exited (code $code)."
    echo "Restarting the deck in 8 s — press Ctrl+C now to stay in the shell."
    sleep 8
    clear
done
