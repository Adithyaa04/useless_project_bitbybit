#!/usr/bin/env bash
# CardKB setup for Raspberry Pi / DietPi.
# Does: enable I2C, install i2c-tools, scan for 0x5F, install
# smbus2+evdev with uv, configure uinput, install systemd service.
#
# Usage:
#   chmod +x setup.sh
#   ./setup.sh            # interactive, reboots when required
#   ./setup.sh --yes      # non-interactive (assume yes)
#   ./setup.sh --no-reboot  # don't reboot even if required
set -euo pipefail

YES=0
DO_REBOOT=1
for arg in "$@"; do
  case "$arg" in
    -y|--yes) YES=1 ;;
    --no-reboot) DO_REBOOT=0 ;;
    -h|--help)
      echo "Usage: $0 [--yes] [--no-reboot]"
      exit 0
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_USER="${SUDO_USER:-${USER:-dietpi}}"
TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
[ -n "$TARGET_HOME" ] || TARGET_HOME="$HOME"
KEYBOARD_SRC="$SCRIPT_DIR/cardkb_keyboard.py"
KEYBOARD_DST="$TARGET_HOME/cardkb_keyboard.py"

SERVICE_NAME="cardkb.service"
SERVICE_DST="/etc/systemd/system/$SERVICE_NAME"
UDEV_RULE="/etc/udev/rules.d/99-uinput.rules"
CONFIG_TXT=""
for c in /boot/firmware/config.txt /boot/config.txt; do
  if [ -f "$c" ]; then CONFIG_TXT="$c"; break; fi
done

NEEDS_REBOOT=0

msg()  { echo -e "\033[1;32m[cardkb]\033[0m $*"; }
warn() { echo -e "\033[1;33m[cardkb]\033[0m $*"; }
err()  { echo -e "\033[1;31m[cardkb]\033[0m $*" >&2; }

need_root() {
  if [ "$(id -u)" -ne 0 ]; then
    err "Please run with sudo:  sudo ./setup.sh"
    exit 1
  fi
}

ask_yes() {
  # $1 = prompt. Returns 0 for yes.
  if [ "$YES" -eq 1 ]; then return 0; fi
  read -r -p "$1 [Y/n] " ans || true
  [[ "${ans:-Y}" =~ ^[Yy]$|^$ ]]
}

need_root

echo "=== CardKB setup (user=$TARGET_USER home=$TARGET_HOME) ==="

# ---------------------------------------------------------------- 1. Enable I2C
msg "Step 1/7: enabling I2C (dtparam=i2c_arm=on)..."
if [ -z "$CONFIG_TXT" ]; then
  warn "No /boot/firmware/config.txt or /boot/config.txt found; skipping config.txt edit."
  warn "Enable I2C via 'sudo raspi-config nonint do_i2c 0' or dietpi-config instead."
else
  msg "Using $CONFIG_TXT"
  if grep -Eq '^\s*dtparam=i2c_arm=on' "$CONFIG_TXT"; then
    msg "dtparam=i2c_arm=on already present."
  else
    if grep -Eq '^\s*#?\s*dtparam=i2c_arm=' "$CONFIG_TXT"; then
      sed -i -E 's/^\s*#?\s*dtparam=i2c_arm=.*/dtparam=i2c_arm=on/' "$CONFIG_TXT"
    else
      printf '\n# Enabled by CardKB setup\ndtparam=i2c_arm=on\n' >> "$CONFIG_TXT"
    fi
    msg "Added dtparam=i2c_arm=on to $CONFIG_TXT"
    NEEDS_REBOOT=1
  fi
fi

# Auto-load I2C kernel modules every boot
for mod in i2c-dev i2c-bcm2835; do
  if ! grep -Eq "^${mod}$" /etc/modules 2>/dev/null; then
    echo "$mod" >> /etc/modules
    msg "Added $mod to /etc/modules"
  fi
  if ! lsmod | grep -q "^${mod//-/[_-]}"; then
    if modprobe "$mod"; then
      msg "Loaded module $mod now."
    else
      warn "Could not modprobe $mod now (may need reboot)."
      NEEDS_REBOOT=1
    fi
  else
    msg "Module $mod already loaded."
  fi
done

# ------------------------------------------------- 2. Check /dev/i2c* + tools
msg "Step 2/7: checking /dev/i2c* ..."
if ls /dev/i2c* >/dev/null 2>&1; then
  ls /dev/i2c*
else
  warn "No /dev/i2c* found. Installing i2c-tools and flagging reboot."
  NEEDS_REBOOT=1
fi

msg "Installing system packages (i2c-tools)..."
apt-get update
apt-get install -y i2c-tools python3 python3-venv

msg "Step 3/7: scanning I2C bus 1 for CardKB (expect 5F)..."
if [ -e /dev/i2c-1 ]; then
  i2cdetect -y 1 || warn "i2cdetect failed."
  if i2cdetect -y 1 2>/dev/null | grep -qi "5f"; then
    msg "Found 0x5F on bus 1 — CardKB detected."
  else
    warn "0x5F NOT seen. Check wiring: VCC->3.3V, GND->GND, SDA->GPIO2, SCL->GPIO3."
  fi
else
  warn "/dev/i2c-1 missing — reboot first, then re-run: sudo i2cdetect -y 1"
fi

# ------------------------------------------------- 3. Python deps via uv (OPTIONAL)
# The Rust driver (zdeck-cardkb) needs no Python at all — it is the default.
# This step only matters for the legacy cardkb_keyboard.py fallback, so it
# must NEVER abort setup (e.g. PEP 668 "externally managed environment").
msg "Step 4/7: trying optional Python deps (smbus2 + evdev, legacy fallback)..."
if ! command -v uv >/dev/null 2>&1; then
  msg "uv not found, trying to install (best effort)..."
  curl -LsSf https://astral.sh/uv/install.sh | sh || warn "uv installer failed, continuing."
  export PATH="$HOME/.local/bin:$PATH"
  if [ -n "${SUDO_USER:-}" ]; then
    USER_HOME="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
    export PATH="$USER_HOME/.local/bin:$PATH"
    if [ -x "$USER_HOME/.local/bin/uv" ]; then
      cp "$USER_HOME/.local/bin/uv" /usr/local/bin/uv 2>/dev/null || true
    fi
    [ -x "$HOME/.local/bin/uv" ] && cp "$HOME/.local/bin/uv" /usr/local/bin/uv 2>/dev/null || true
  fi
fi
if ! command -v uv >/dev/null 2>&1; then
  export PATH="/usr/local/bin:$HOME/.local/bin:$PATH"
fi
if command -v uv >/dev/null 2>&1; then
  # NOTE: --break-system-packages is required on PEP 668 distros
  # (that "externally managed environment" error); harmless elsewhere.
  if uv pip install --system --break-system-packages smbus2 evdev; then
    msg "Python deps installed (legacy fallback ready)."
  else
    warn "uv install failed — skipping Python fallback (Rust driver unaffected)."
  fi
elif python3 -m pip install --break-system-packages smbus2 evdev 2>/dev/null; then
  msg "Python deps installed via pip (legacy fallback ready)."
else
  warn "No uv/pip path worked — skipping Python fallback (Rust driver unaffected)."
fi
msg "CardKB driver: Rust zdeck-cardkb (no Python needed)."

# ------------------------------------------------- 4. uinput setup
msg "Step 5/7: configuring uinput..."
if ! lsmod | grep -q "^uinput"; then
  modprobe uinput && msg "Loaded uinput." || warn "modprobe uinput failed."
fi
if ! grep -Eq "^uinput$" /etc/modules 2>/dev/null; then
  echo "uinput" >> /etc/modules
  msg "Added uinput to /etc/modules (loads every boot)."
fi
cat > "$UDEV_RULE" <<'EOF'
KERNEL=="uinput", MODE="0660", GROUP="input"
EOF
msg "Wrote $UDEV_RULE"
if ! id "$TARGET_USER" 2>/dev/null | grep -q "(input)"; then
  usermod -aG input "$TARGET_USER"
  msg "Added $TARGET_USER to 'input' group (re-login/reboot to take effect)."
  NEEDS_REBOOT=1
else
  msg "$TARGET_USER already in 'input' group."
fi
# /dev/i2c-* is root:i2c on Pi OS/DietPi — without this group the driver
# gets "Permission denied" and the service crash-loops (no keypresses).
if ! id "$TARGET_USER" 2>/dev/null | grep -q "(i2c)"; then
  if getent group i2c >/dev/null 2>&1; then
    usermod -aG i2c "$TARGET_USER"
    msg "Added $TARGET_USER to 'i2c' group (re-login/reboot to take effect)."
    NEEDS_REBOOT=1
  else
    warn "No 'i2c' group on this system — skipping (driver may lack /dev/i2c-1 access)."
  fi
else
  msg "$TARGET_USER already in 'i2c' group."
fi
udevadm control --reload-rules && udevadm trigger --subsystem-match=misc --attr-match=name=uinput || true

# ------------------------------------------------- 5. Install keyboard script
msg "Step 6/7: installing cardkb_keyboard.py -> $KEYBOARD_DST ..."
cp "$KEYBOARD_SRC" "$KEYBOARD_DST"
chown "$TARGET_USER:$(id -gn "$TARGET_USER")" "$KEYBOARD_DST"
chmod 644 "$KEYBOARD_DST"
python3 -m py_compile "$KEYBOARD_DST" && msg "Syntax OK."
msg "Manual test:  python3 $KEYBOARD_DST"
msg "Then type in a text editor, and verify with:"
msg "  cat /proc/bus/input/devices | grep -A5 CardKB"

# ------------------------------------------------- 6. systemd service (autostart)
# Prefer the Rust driver (no Python); fall back to cardkb_keyboard.py
# ONLY if its imports actually work — otherwise the service would just
# crash-loop and you'd get silence (no keypresses) with no obvious error.
RUST_BIN="$TARGET_HOME/zdeck/zdeck-cardkb"
if [ -x "$RUST_BIN" ]; then
  EXEC_LINE="ExecStart=$RUST_BIN"
  msg "Step 7/7: creating $SERVICE_DST (Rust driver: $RUST_BIN) ..."
elif RUST_BIN_SYS="$(command -v zdeck-cardkb 2>/dev/null)" && [ -n "$RUST_BIN_SYS" ]; then
  EXEC_LINE="ExecStart=$RUST_BIN_SYS"
  msg "Step 7/7: creating $SERVICE_DST (Rust driver: $RUST_BIN_SYS) ..."
elif python3 -c "import smbus2, evdev" 2>/dev/null; then
  EXEC_LINE="ExecStart=/usr/bin/python3 $KEYBOARD_DST"
  msg "Step 7/7: creating $SERVICE_DST (Python fallback, imports OK) ..."
else
  EXEC_LINE="ExecStart=/usr/bin/python3 $KEYBOARD_DST"
  warn "Step 7/7: NO working driver! Rust binary missing AND python smbus2/evdev broken."
  warn "The service is installed but WILL crash-loop until you fix one of:"
  warn "  a) cp binary/pi3-arm64/zdeck-cardkb $TARGET_HOME/zdeck/ && sudo $0"
  warn "     (Rust, recommended — then re-run this setup so the service uses it)"
  warn "  b) fix Python: uv pip install --system --break-system-packages smbus2 evdev"
fi
cat > "$SERVICE_DST" <<EOF
[Unit]
Description=M5Stack CardKB virtual keyboard (I2C 0x5F -> uinput)
After=multi-user.target

[Service]
Type=simple
User=$TARGET_USER
$EXEC_LINE
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable "$SERVICE_NAME"
systemctl restart "$SERVICE_NAME" || warn "Service failed to start (may need reboot for i2c/uinput). Check: sudo journalctl -u $SERVICE_NAME -e"
sleep 2
if systemctl is-active --quiet "$SERVICE_NAME" && grep -q "CardKB" /proc/bus/input/devices 2>/dev/null; then
  msg "Self-check PASSED: service active + CardKB registered. Type in any editor to test."
else
  warn "Self-check: service state / CardKB device not confirmed yet."
  warn "After any reboot, run the diagnostic:  sudo bash $SCRIPT_DIR/diag.sh"
  warn "It pinpoints the failing layer (wiring, perms, driver, service)."
fi
systemctl --no-pager --full status "$SERVICE_NAME" || true

echo
msg "Done. Useful commands:"
echo "  sudo bash $SCRIPT_DIR/diag.sh                   # full diagnostic (use this first if keys don't show)"
echo "  sudo i2cdetect -y 1                              # expect 5F"
echo "  $RUST_BIN                                    # manual Rust run (foreground, Ctrl+C)"
echo "  python3 $KEYBOARD_DST                            # manual Python run (fallback)"
echo "  cat /proc/bus/input/devices | grep -A5 CardKB    # verify virtual keyboard"
echo "  sudo systemctl status $SERVICE_NAME"
echo "  sudo journalctl -u $SERVICE_NAME -f"

if [ "$NEEDS_REBOOT" -eq 1 ]; then
  warn "A reboot is required (I2C/uinput/group change)."
  if [ "$DO_REBOOT" -eq 1 ] && ask_yes "Reboot now?"; then
    msg "Rebooting..."
    reboot
  else
    msg "Reboot later with: sudo reboot"
    msg "After reboot re-run: ls /dev/i2c* && sudo i2cdetect -y 1"
  fi
else
  msg "No reboot strictly required, but reboot if /dev/i2c-1 or CardKB is missing."
fi
