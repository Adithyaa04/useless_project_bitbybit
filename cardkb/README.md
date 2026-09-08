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
3. Installs `smbus2` (I2C talk) + `evdev` (virtual keyboard) with
   `uv` instead of pip: `uv pip install --system smbus2 evdev`.
4. `modprobe uinput`, persists `uinput` in `/etc/modules`, writes
   `/etc/udev/rules.d/99-uinput.rules`:
   `KERNEL=="uinput", MODE="0660", GROUP="input"`,
   runs `usermod -aG input $USER` + `udevadm control --reload-rules`.
5. Copies `cardkb_keyboard.py` to `~/cardkb_keyboard.py`, syntax-checks it.
6. Installs `/etc/systemd/system/cardkb.service`
   (`After=multi-user.target`, `Restart=always`,
   `ExecStart=/usr/bin/python3 /home/<user>/cardkb_keyboard.py`),
   then `systemctl enable + restart`.
7. Reboots if I2C/uinput/group changed (`sudo reboot`).

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
