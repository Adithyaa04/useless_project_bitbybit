# M5Stack CardKB on Raspberry Pi / DietPi

I2C mini keyboard (fixed address `0x5F`) exposed to the whole OS as a
virtual USB keyboard via `uinput`.

Default driver is now **Rust** (`zdeck-cardkb`, no Python needed).
The Python script remains as a legacy fallback.

## Folder contents

| File | Purpose |
|---|---|
| `../rust/src/bin/zdeck-cardkb.rs` | Rust driver: polls `0x5F` on `/dev/i2c-1`, emits `CardKB-Virtual-Keyboard` (default) |
| `cardkb_keyboard.py` | Legacy Python fallback (needs `smbus2`+`evdev`) |
| `setup.sh` | Full one-shot setup: I2C, uinput, service (Python step is best-effort only) |
| `cardkb.service` | Reference unit file (setup.sh installs a user-adjusted copy) |

## Quick start (on the Pi)

Rust driver (recommended — no Python):

```bash
./rust/build-pi.sh arm64   # or copy binary/pi3-arm64/* to the Pi
sudo ./cardkb/setup.sh     # sets up I2C + uinput, points service at zdeck-cardkb
# reboot when asked, then:
ls /dev/i2c*                 # expect /dev/i2c-1
sudo i2cdetect -y 1          # expect 5F in the grid
~/zdeck/zdeck-cardkb         # manual test — type in any editor
cat /proc/bus/input/devices | grep -A5 CardKB
sudo systemctl status cardkb.service
```

Legacy Python fallback (only if you prefer it):

```bash
cd cardkb
sudo ./setup.sh              # Python install step is best-effort; setup no longer aborts on PEP 668 errors
python3 ~/cardkb_keyboard.py
```

Non-interactive / no-reboot variants:

```bash
sudo ./setup.sh --yes
sudo ./setup.sh --yes --no-reboot
```

## What setup.sh does

1. Adds `dtparam=i2c_arm=on` to `/boot/firmware/config.txt`
   (falls back to `/boot/config.txt`), adds `i2c-dev` + `i2c-bcm2835`
   to `/etc/modules`, `modprobe`s them now.
2. `apt install i2c-tools`, then `i2cdetect -y 1` — look for `5F`.
3. Tries Python `smbus2`+`evdev` (legacy fallback only, best-effort —
   never aborts setup on PEP 668 errors).
4. `modprobe uinput`, persists `uinput` in `/etc/modules`, writes
   `/etc/udev/rules.d/99-uinput.rules`:
   `KERNEL=="uinput", MODE="0660", GROUP="input"`,
   adds you to `input` **and `i2c`** groups + `udevadm control --reload-rules`.
5. Copies `cardkb_keyboard.py` to `~/cardkb_keyboard.py` (fallback).
6. Installs `/etc/systemd/system/cardkb.service`
   (`After=multi-user.target`, `Restart=always`,
   `ExecStart=/home/<user>/zdeck/zdeck-cardkb`, Python fallback only if
   its imports actually work — otherwise it warns instead of installing
   a service that would crash-loop silently), then `enable + restart`
   plus a self-check.
7. Reboots if I2C/uinput/group changed (`sudo reboot`).

## No keypresses? Diagnose first

```bash
sudo bash cardkb/diag.sh          # full report + MOST LIKELY cause
sudo bash cardkb/diag.sh --live   # + 10 s live keypress test (press keys!)
```

Usual culprits, in order:

| Symptom in diag | Fix |
|---|---|
| `0x5F NOT on bus 1` | wiring/power (VCC→3.3V, GND→GND, SDA→GPIO2, SCL→GPIO3) |
| `CANNOT access /dev/i2c-1` | `sudo usermod -aG i2c $USER && sudo reboot` |
| service crash-loop, `Permission denied` in journal | groups (`input`+`i2c`) + reboot; `sudo journalctl -u cardkb -n 30` |
| `ExecStart` points at missing file | `cp binary/pi3-arm64/zdeck-cardkb ~/zdeck/ && sudo ./cardkb/setup.sh` |
| device registered, apps ignore keys | focus a console editor (not SSH): `cat /proc/bus/input/devices \| grep -A5 CardKB` |

## DietPi autostart (custom.sh)

`install-autostart.sh` detects DietPi and writes the launch command into
`/var/lib/dietpi-autostart/custom.sh` (old images) or
`/var/lib/dietpi/dietpi-autostart/custom.sh` (new images):

```sh
#!/bin/dash
# Z-DECK kiosk ...
exec "/home/dietpi/zdeck/zdeck-main.sh"
```

Then: `sudo dietpi-autostart` → **Custom script (foreground, with auto
login)** → reboot. Manual equivalent: edit that file, paste the `exec`
line, keep it executable. Remove again with
`install-autostart.sh --uninstall` (restores your backup).

## Wiring

| CardKB | Pi |
|---|---|
| VCC | 3.3V |
| GND | GND |
| SDA | GPIO2 |
| SCL | GPIO3 |

## Keymap

Full ASCII map incl. shifted symbols, plus from the CardKB manual:

`0x08` Backspace, `0x09` Tab, `0x0D` Enter, `0x1B` Esc,
`0xB4` Left, `0xB5` Up, `0xB6` Down, `0xB7` Right.
