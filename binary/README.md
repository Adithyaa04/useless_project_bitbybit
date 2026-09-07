# `binary/` — prebuilt Raspberry Pi 3 executables

Docker cross-compiled release binaries land here via:

```sh
./rust/build-pi.sh          # both targets
./rust/build-pi.sh armv7    # 32-bit only
./rust/build-pi.sh arm64    # 64-bit only
```

| Folder | Rust target | For which Pi OS |
|---|---|---|
| `pi3-armv7/` | `armv7-unknown-linux-gnueabihf` | 32-bit Raspberry Pi OS / Debian `armhf` (default, safest on Pi 3) |
| `pi3-arm64/` | `aarch64-unknown-linux-gnu` | 64-bit Raspberry Pi OS / Debian `arm64` |

Each folder contains: `zdeck-game`, `zdeck-gps`, `zdeck-fetch`, `zdeck-run`.

## Run on the Pi

```sh
# 32-bit Pi OS example (use pi3-arm64/ instead on 64-bit Pi OS)
scp binary/pi3-armv7/* pi@raspberrypi:/home/pi/zdeck/
ssh pi@raspberrypi
chmod +x ~/zdeck/*
./zdeck-fetch --lat 10.0261 --lon 76.3125 --radius 300   # once, with internet
./zdeck-run --sim
```

> Binaries are glibc-linked (Debian/Raspberry Pi OS). They will **not** run on
> Alpine/musl systems.
