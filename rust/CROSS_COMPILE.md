# Cross-compiling for Raspberry Pi 3 (Debian-based OS)

All cross builds run inside a container — no ARM toolchain needed on your machine.
The script auto-detects the engine: Docker if `docker info` works, otherwise
rootless Podman (`CONTAINER_ENGINE=podman ./rust/build-pi.sh` to force it).

## Quick start

```sh
./rust/build-pi.sh           # both 32-bit (armv7) + 64-bit (arm64) -> ./binary/
./rust/build-pi.sh armv7     # 32-bit Raspberry Pi OS / Debian armhf (recommended default for Pi 3)
./rust/build-pi.sh arm64     # 64-bit Raspberry Pi OS / Debian arm64
```

Output:

```text
binary/pi3-armv7/zdeck-game  zdeck-gps  zdeck-fetch  zdeck-run   (ARMv7, hard-float)
binary/pi3-arm64/zdeck-game  zdeck-gps  zdeck-fetch  zdeck-run   (AArch64)
```

## Which one do I need?

- Unsure / stock 32-bit Raspberry Pi OS → use `binary/pi3-armv7/`.
- Flashed **64-bit** Raspberry Pi OS (`arm64`, `uname -m` → `aarch64`) → use `binary/pi3-arm64/`.

Check on the Pi with `uname -m`: `armv7l` → armv7 folder, `aarch64` → arm64 folder.

## How it works

1. `rust/Dockerfile.cross` starts from `rust:<ver>-bookworm` (glibc, matching Debian),
   installs `gcc-arm-linux-gnueabihf` + `gcc-aarch64-linux-gnu`, adds both Rust targets,
   and runs `cargo build --locked --release --target <TARGET>`.
2. `rust/.cargo/config.toml` tells Cargo which cross-linker to use per target
   (used both inside Docker and for local cross builds).
3. `rust/rust-toolchain.toml` pins the toolchain (1.97) + targets for reproducibility.
4. `rust/build-pi.sh` builds the image, `docker create` + `docker cp`s the four
   release binaries out into `./binary/<profile>/`, and runs `file` on them.

## Manual Docker commands (no script)

```sh
docker build -f rust/Dockerfile.cross \
  --build-arg TARGET=armv7-unknown-linux-gnueabihf \
  -t zdeck-cross:armv7 ./rust

id=$(docker create zdeck-cross:armv7)
mkdir -p binary/pi3-armv7
for b in zdeck-game zdeck-gps zdeck-fetch zdeck-run; do
  docker cp "$id:/out/armv7-unknown-linux-gnueabihf/$b" binary/pi3-armv7/
done
docker rm "$id"
file binary/pi3-armv7/*
```

Swap `TARGET=aarch64-unknown-linux-gnu` / `pi3-arm64` for the 64-bit build.

## Local (non-Docker) cross build

```sh
sudo apt install gcc-arm-linux-gnueabihf gcc-aarch64-linux-gnu
rustup target add armv7-unknown-linux-gnueabihf aarch64-unknown-linux-gnu
cd rust
cargo build --release --target armv7-unknown-linux-gnueabihf
```

## Deploy to the Pi

```sh
scp binary/pi3-armv7/* pi@raspberrypi:/home/pi/zdeck/
ssh pi@raspberrypi 'chmod +x ~/zdeck/* && ~/zdeck/zdeck-run --sim'
```

## Troubleshooting

- `arm-linux-gnueabihf-gcc not found` → you skipped Docker; install the apt cross packages above or use the script.
- `file` reports `x86-64` → you grabbed `rust/target/release/`, not `binary/`; the script exports the right ones.
- Binary won't start on Pi (`No such file or directory`) → wrong ABI folder (armv7 vs arm64) or a musl-based OS; these are glibc builds for Debian/Raspberry Pi OS.
