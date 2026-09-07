#!/usr/bin/env bash
# Z-DECK master auto-boot — Raspberry Pi 3, 64-bit OS only.
#
# Runs right after console auto-login and launches `zdeck-run` first.
# `zdeck-run` itself shows the fullscreen logo, offers AUTO (GPS on the
# deck's only GPS device, /dev/ttyAMA0) vs SIM with a 5 s countdown that
# defaults to AUTO, fetches a fresh 300 m map around the live fix, and
# starts the game (which keeps polling the sensor and reloads areas as
# you walk out of them).
#
# Install:  ./install-autostart.sh   (copies pi3-arm64 binaries + this
#             script to ~/zdeck and hooks it into the tty1 auto-login)
# Kiosk loop: when the game exits, we wait a few seconds (Ctrl+C here
# drops back to a shell for maintenance) and start over — the deck never
# sits on a bare prompt at an event.

set -u

ZDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN="$ZDIR/zdeck-run"

# Sanity: the three siblings must sit next to this script (~/zdeck).
for b in zdeck-run zdeck-game zdeck-gps zdeck-fetch; do
    if [[ ! -x "$ZDIR/$b" ]]; then
        echo "Z-DECK: missing $ZDIR/$b" >&2
        echo "  Run install-autostart.sh, or copy binary/pi3-arm64/* to $ZDIR/ and chmod +x." >&2
        sleep 5
    fi
done

export TERM="${TERM:-xterm-256color}"
cd "$ZDIR" || exit 1

# Console finishing touches (best effort, never fatal).
clear
stty sane 2>/dev/null || true
setterm -blank 0 -powersave off -cursor on 2>/dev/null || true

echo "Z-DECK auto-boot (pi3-arm64): starting zdeck-run ..."

while true; do
    # No args = splash + 5 s AUTO/SIM countdown (AUTO wins on timeout).
    # Pass-through allows `zdeck-auto.sh --sim` for indoor testing.
    "$RUN" "$@"
    code=$?
    echo
    echo "Z-DECK exited (code $code)."
    echo "Restarting the deck in 8 s — press Ctrl+C now to stay in the shell."
    sleep 8
    clear
done
