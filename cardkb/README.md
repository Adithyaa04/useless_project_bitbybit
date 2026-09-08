# M5Stack CardKB on Raspberry Pi / DietPi

I2C mini keyboard (fixed address `0x5F`) exposed to the whole OS as a
virtual USB keyboard via `evdev`/`uinput`.

## Folder contents

| File | Purpose |
|---|---|
| `cardkb_keyboard.py` | Polls `0x5F` on `/dev/i2c-1`, emits key events as `CardKB-Virtual-Keyboard` |
| `setup.sh` | Full one-shot setup: I2C, deps via `uv`, uinput, service |
| `cardkb.service` | Reference unit file (setup.sh installs a user-adjusted copy) |

## Quick start (on the Pi)

```bash
cd cardkb
chmod +x setup.sh
sudo ./setup.sh
# reboot when asked, then:
ls /dev/i2c*                 # expect /dev/i2c-1
sudo i2cdetect -y 1          # expect 5F in the grid
python3 ~/cardkb_keyboard.py # manual test — type in any editor
cat /proc/bus/input/devices | grep -A5 CardKB
sudo systemctl status cardkb.service
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
