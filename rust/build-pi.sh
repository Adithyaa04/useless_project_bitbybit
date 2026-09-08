#!/usr/bin/env bash
# Cross-compile the Rust app for Raspberry Pi 3 (Debian-based OS) via Docker
# and export the binaries into ./binary/.
#
# Usage:
#   ./rust/build-pi.sh              # build BOTH armv7 (32-bit) + arm64 (64-bit)
#   ./rust/build-pi.sh armv7        # 32-bit only  (Raspberry Pi OS 32-bit / armhf)
#   ./rust/build-pi.sh arm64        # 64-bit only  (Raspberry Pi OS 64-bit / arm64)
#   ./rust/build-pi.sh armv7 --no-cache   # extra args are forwarded to `docker build`
#
# Output layout:
#   binary/pi3-armv7/zdeck-{game,gps,fetch,run}   (armv7-unknown-linux-gnueabihf)
#   binary/pi3-arm64/zdeck-{game,gps,fetch,run}   (aarch64-unknown-linux-gnu)
#
# Requirements: Docker (user in `docker` group) or rootless Podman.
# Override the engine explicitly with: CONTAINER_ENGINE=podman ./rust/build-pi.sh
set -euo pipefail

ENGINE="${CONTAINER_ENGINE:-}"
if [[ -z "$ENGINE" ]]; then
  if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    ENGINE=docker
  elif command -v podman >/dev/null 2>&1; then
    ENGINE=podman
  else
    echo "ERROR: neither 'docker info' nor 'podman' works." >&2
    echo "  - For Docker: add your user to the docker group (sudo usermod -aG docker \$USER) and re-login," >&2
    echo "    or run with sudo." >&2
    echo "  - Or install podman for a rootless build." >&2
    exit 1
  fi
fi
echo "Container engine: $ENGINE"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="$REPO_ROOT/rust"
BIN_DIR="$REPO_ROOT/binary"
BINS=(zdeck-game zdeck-gps zdeck-fetch zdeck-run zdeck-cardkb)

declare -A TARGETS=(
  [armv7]=armv7-unknown-linux-gnueabihf
  [armhf]=armv7-unknown-linux-gnueabihf
  [arm64]=aarch64-unknown-linux-gnu
  [aarch64]=aarch64-unknown-linux-gnu
)
declare -A OUTDIRS=(
  [armv7-unknown-linux-gnueabihf]=pi3-armv7
  [aarch64-unknown-linux-gnu]=pi3-arm64
)

want="${1:-all}"
extra_args=()
if [[ "$want" == -* ]]; then
  want="all"
  extra_args=("$@")
elif [[ $# -gt 1 ]]; then
  extra_args=("${@:2}")
fi

to_build=()
case "$want" in
  all|both) to_build=(armv7-unknown-linux-gnueabihf aarch64-unknown-linux-gnu) ;;
  armv7|armhf|arm64|aarch64) to_build=("${TARGETS[$want]}") ;;
  *) echo "Unknown target '$want'. Use: all | armv7 | arm64" >&2; exit 1 ;;
esac

if ! command -v "$ENGINE" >/dev/null 2>&1; then
  echo "ERROR: container engine '$ENGINE' not found." >&2
  exit 1
fi

# DOCKER_BUILDKIT=1 enables the cache mounts used in Dockerfile.cross (Docker only)
if [[ "$ENGINE" == "docker" ]]; then
  export DOCKER_BUILDKIT=1
fi

for target in "${to_build[@]}"; do
  outdir="$BIN_DIR/${OUTDIRS[$target]}"
  tag="zdeck-cross:${target}"
  mkdir -p "$outdir"

  echo "==> [${target}] ${ENGINE} build (${RUST_DIR})"
  build_cmd=("$ENGINE" build -f "$RUST_DIR/Dockerfile.cross"
    --build-arg "TARGET=${target}"
    -t "$tag")
  if [[ ${#extra_args[@]} -gt 0 ]]; then
    build_cmd+=("${extra_args[@]}")
  fi
  build_cmd+=("$RUST_DIR")
  "${build_cmd[@]}"

  echo "==> [${target}] exporting binaries -> ${outdir}/"
  cid="$("$ENGINE" create "$tag")"
  trap '"$ENGINE" rm -f "$cid" >/dev/null 2>&1 || true' EXIT
  for bin in "${BINS[@]}"; do
    # Binaries were copied to /out/<target>/ in the image; fall back to the
    # cargo target dir in case the Dockerfile changes.
    if "$ENGINE" cp "$cid:/out/${target}/${bin}" "$outdir/$bin" 2>/dev/null; then
      :
    else
      "$ENGINE" cp "$cid:/src/target/${target}/release/${bin}" "$outdir/$bin"
    fi
    chmod +x "$outdir/$bin"
  done
  "$ENGINE" rm -f "$cid" >/dev/null
  trap - EXIT

  echo "==> [${target}] done:"
  ls -la "$outdir"
  file "$outdir"/* || true
done

echo
echo "All requested targets built. Binaries live in:"
ls -la "$BIN_DIR"
echo
echo "Copy to the Pi, e.g.:"
echo "  scp binary/pi3-armv7/* pi@raspberrypi:/home/pi/zdeck/"
echo "  # or for 64-bit Pi OS:"
echo "  scp binary/pi3-arm64/* pi@raspberrypi:/home/pi/zdeck/"
