#!/usr/bin/env python3
"""
Zombie Deck - Main Launcher
One script to rule them all: install deps, fetch map, launch game.

Usage:
    python3 run.py              # interactive (recommended)
    python3 run.py --sim        # skip prompts, sim mode
    python3 run.py --gps /dev/rfcomm0 --baud 9600
    python3 run.py --gpio 16 --baud 9600
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

# --- paths — works whether run.py is at project root OR inside app/ ---
_THIS_DIR = Path(__file__).resolve().parent
if _THIS_DIR.name == "app" and (_THIS_DIR / "cyberdeck.py").exists():
    # invoked as app/run.py
    ROOT = _THIS_DIR.parent
    APP_DIR = _THIS_DIR
else:
    ROOT = _THIS_DIR
    APP_DIR = ROOT / "app"
    # fallback if app/ doesn't exist (single-folder layout)
    if not APP_DIR.exists():
        APP_DIR = ROOT
# clean names (new) + legacy fallback
FETCH_MAP = APP_DIR / "fetch_map.py"
if not FETCH_MAP.exists():
    FETCH_MAP = APP_DIR / "fetch_map1.py"
CYBERDECK = APP_DIR / "cyberdeck.py"
if not CYBERDECK.exists():
    CYBERDECK = APP_DIR / "zombie_cyberdeck.py"
MAP_FILE = APP_DIR / "map_data.json"

BOLD = "\033[1m"
DIM = "\033[2m"
GRN = "\033[92m"
RED = "\033[91m"
CYA = "\033[96m"
YEL = "\033[93m"
RST = "\033[0m"

def banner():
    print(f"""{GRN}{BOLD}
  ███████╗ ██████╗ ███╗   ███╗██████╗ ██╗███████╗   ██████╗ ███████╗ ██████╗██╗  ██╗
  ╚══███╔╝██╔═══██╗████╗ ████║██╔══██╗██║██╔════╝   ██╔══██╗██╔════╝██╔════╝██║ ██╔╝
    ███╔╝ ██║   ██║██╔████╔██║██████╔╝██║█████╗     ██║  ██║█████╗  ██║     █████╔╝ 
   ███╔╝  ██║   ██║██║╚██╔╝██║██╔══██╗██║██╔══╝     ██║  ██║██╔══╝  ██║     ██╔═██╗ 
  ███████╗╚██████╔╝██║ ╚═╝ ██║██████╔╝██║███████╗   ██████╔╝███████╗╚██████╗██║  ██╗
  ╚══════╝ ╚═════╝ ╚═╝     ╚═╝╚═════╝ ╚═╝╚══════╝   ╚═════╝ ╚══════╝ ╚═════╝╚═╝  ╚═╝
{RST}{DIM}  Bit By Bit — Run for your life. The street is the map.{RST}
""")

def run_cmd(cmd, check=True):
    print(f"{DIM}$ {' '.join(cmd)}{RST}")
    return subprocess.run(cmd, check=check)

def ensure_deps(skip_install=False):
    """Install required pip deps if missing."""
    if skip_install:
        return
    print(f"\n{BOLD}[1/3] Checking dependencies...{RST}")
    deps = ["pynmea2", "pyserial"]
    # pigpio only needed for GPIO bit-bang mode, but install anyway for convenience
    try:
        import pynmea2  # noqa
        import serial   # noqa
        print(f"  {GRN}✓ pynmea2 + pyserial already installed{RST}")
        try:
            import pigpio  # noqa
            print(f"  {GRN}✓ pigpio already installed{RST}")
        except ImportError:
            print(f"  {YEL}○ pigpio not installed (only needed for --gpio bit-bang mode){RST}")
        return
    except ImportError:
        pass

    print(f"  Installing {', '.join(deps)} ...")
    # --break-system-packages needed on Debian Pi (PEP 668)
    cmd = [sys.executable, "-m", "pip", "install", "--break-system-packages", "pynmea2", "pyserial"]
    try:
        run_cmd(cmd)
        print(f"  {GRN}✓ deps installed{RST}")
    except subprocess.CalledProcessError:
        print(f"  {RED}pip install failed — trying without --break-system-packages{RST}")
        run_cmd([sys.executable, "-m", "pip", "install", "pynmea2", "pyserial"], check=False)

    # optionally offer pigpio
    try:
        import pigpio  # noqa
    except ImportError:
        print(f"  {DIM}Tip: for GPIO bit-bang GPS (GPIO16) also run: pip install pigpio --break-system-packages{RST}")

def ask_choice(prompt, options, default=None):
    """Prompt with numbered options. options = list of (key, label). Returns key."""
    opt_str = "/".join(f"{YEL}{k}{RST}" for k, _ in options)
    default_hint = f" [{default}]" if default else ""
    while True:
        print(f"\n{BOLD}{prompt}{RST} {DIM}({opt_str}{default_hint}){RST}")
        for k, label in options:
            print(f"  {YEL}{k}{RST}) {label}")
        try:
            ans = input(f"> ").strip().lower()
        except EOFError:
            print(f"\n{DIM}No input — using default {default}{RST}")
            if default:
                return default.lower()
            sys.exit(0)
        if not ans and default:
            return default.lower()
        for k, _ in options:
            if ans == k.lower():
                return k.lower()
        print(f"{RED}  Invalid choice. Enter one of: {', '.join(k for k,_ in options)}{RST}")

def ask_float(prompt, default=None):
    while True:
        hint = f" [{default}]" if default is not None else ""
        try:
            ans = input(f"{prompt}{hint}: ").strip()
        except EOFError:
            if default is not None:
                print(f"\n{DIM}No input — using {default}{RST}")
                return float(default)
            sys.exit(0)
        if not ans and default is not None:
            return float(default)
        try:
            return float(ans)
        except ValueError:
            print(f"{RED}  Enter a number.{RST}")

def ask_str(prompt, default=None):
    hint = f" [{default}]" if default else ""
    try:
        ans = input(f"{prompt}{hint}: ").strip()
    except EOFError:
        print(f"\n{DIM}No input — using {default}{RST}")
        return default or ""
    return ans if ans else (default or "")

def ensure_map(interactive=True, force_fetch=False, lat=None, lon=None, radius=300):
    """Ensure map_data.json exists; prompt to fetch if missing or requested."""
    print(f"\n{BOLD}[2/3] Map data{RST}")
    if MAP_FILE.exists():
        try:
            d = json.loads(MAP_FILE.read_text())
            print(f"  {GRN}✓ Found {MAP_FILE.relative_to(ROOT)} — "
                  f"origin {d.get('origin_lat')},{d.get('origin_lon')} "
                  f"radius {d.get('radius_m')}m, {len(d.get('ways',[]))} ways, {len(d.get('pois',[]))} POIs{RST}")
            if not interactive and not force_fetch:
                return str(MAP_FILE)
            if interactive:
                ch = ask_choice("Map found. What to do?", [("K","Keep existing"), ("F","Fetch new map")], default="K")
                if ch == "k":
                    return str(MAP_FILE)
        except Exception:
            print(f"  {YEL}○ Existing map file is corrupt, will re-fetch{RST}")

    # need to fetch
    if not interactive:
        # non-interactive fetch if lat/lon provided
        if lat is not None and lon is not None:
            cmd = [sys.executable, str(FETCH_MAP), "--lat", str(lat), "--lon", str(lon), "--radius", str(radius), "--out", str(MAP_FILE)]
            run_cmd(cmd)
            return str(MAP_FILE)
        print(f"  {YEL}○ No map file. Run with --lat/--lon or interactively to fetch.{RST}")
        return str(MAP_FILE)

    print(f"\n  Fetch map from OpenStreetMap (needs internet once, then offline).")
    print(f"  {DIM}Find your lat/lon on https://www.openstreetmap.org (right-click → Show address){RST}")
    lat_v = ask_float("  Center latitude  (e.g. 9.9649)", default=lat if lat else 9.9649)
    lon_v = ask_float("  Center longitude (e.g. 76.2868)", default=lon if lon else 76.2868)
    rad_v = ask_float("  Radius meters", default=radius)

    cmd = [sys.executable, str(FETCH_MAP), "--lat", str(lat_v), "--lon", str(lon_v), "--radius", str(rad_v), "--out", str(MAP_FILE)]
    try:
        run_cmd(cmd)
        print(f"  {GRN}✓ Map saved to {MAP_FILE}{RST}")
    except subprocess.CalledProcessError as e:
        print(f"  {RED}✗ fetch failed: {e}{RST}")
        print(f"  {DIM}Try again with hotspot, or copy an existing map_data.json next to cyberdeck.py{RST}")
    return str(MAP_FILE)

def launch_game(mode, gps_port=None, gpio_pin=16, baud=9600, map_path=None):
    print(f"\n{BOLD}[3/3] Launching game...{RST}")
    if map_path is None:
        map_path = str(MAP_FILE)

    if mode == "sim":
        cmd = [sys.executable, str(CYBERDECK), "--sim", "--map", map_path]
        print(f"  {CYA}Mode: SIM (WASD to move){RST}")
        print(f"  {DIM}Controls: W/A/S/D move, Q quit. Zombies spawn 40-90m away.{RST}")
    elif mode == "serial":
        cmd = [sys.executable, str(CYBERDECK), "--gps", gps_port, "--baud", str(baud), "--map", map_path]
        print(f"  {CYA}Mode: GPS Serial {gps_port} @ {baud} baud{RST}")
    else:  # gpio
        cmd = [sys.executable, str(CYBERDECK), "--gpio", str(gpio_pin), "--baud", str(baud), "--map", map_path]
        print(f"  {CYA}Mode: GPS GPIO bit-bang (GPIO{gpio_pin}, {baud} baud){RST}")
        print(f"  {DIM}Needs pigpiod: sudo systemctl start pigpiod{RST}")

    print(f"{DIM}$ {' '.join(cmd)}{RST}\n")
    try:
        subprocess.run(cmd, check=False)
    except KeyboardInterrupt:
        print(f"\n{DIM}Exited.{RST}")

def main():
    ap = argparse.ArgumentParser(description="Zombie Deck launcher", add_help=True)
    ap.add_argument("--sim", action="store_true", help="launch directly in sim (WASD) mode")
    ap.add_argument("--gps", metavar="PORT", help="launch directly in serial GPS mode (e.g. /dev/rfcomm0)")
    ap.add_argument("--gpio", type=int, metavar="PIN", help="launch directly in GPIO bit-bang mode (e.g. 16)")
    ap.add_argument("--baud", type=int, default=9600, help="baud rate (default 9600)")
    ap.add_argument("--map", dest="map_path", help="path to map_data.json")
    ap.add_argument("--lat", type=float, help="fetch map lat (non-interactive)")
    ap.add_argument("--lon", type=float, help="fetch map lon (non-interactive)")
    ap.add_argument("--radius", type=float, default=300, help="fetch map radius")
    ap.add_argument("--yes", action="store_true", help="non-interactive, auto-accept defaults")
    ap.add_argument("--skip-install", action="store_true", help="skip pip install check")
    args = ap.parse_args()

    banner()

    # quick non-interactive paths
    if args.sim or args.gps or args.gpio is not None:
        ensure_deps(skip_install=args.skip_install)
        mp = ensure_map(interactive=False, lat=args.lat, lon=args.lon, radius=args.radius)
        if args.map_path:
            mp = args.map_path
        if args.sim:
            launch_game("sim", map_path=mp)
        elif args.gps:
            launch_game("serial", gps_port=args.gps, baud=args.baud, map_path=mp)
        else:
            launch_game("gpio", gpio_pin=args.gpio, baud=args.baud, map_path=mp)
        return

    # interactive flow
    ensure_deps(skip_install=args.skip_install)
    ensure_map(interactive=True)

    print(f"\n{BOLD}How do you want to play?{RST}")
    mode = ask_choice("Select mode", [
        ("1", "Simulation  — WASD keys, no hardware (indoor testing)"),
        ("2", "GPS Serial  — NEO-6M via /dev/rfcomm0 / /dev/ttyUSB0 / /dev/ttyAMA0"),
        ("3", "GPS GPIO    — bit-banged on GPIO pin (TFT occupies UART, pigpio)"),
    ], default="1")

    if mode == "1":
        launch_game("sim")
    elif mode == "2":
        print(f"\n{BOLD}GPS Serial setup{RST}")
        port = ask_str("  Serial port", default="/dev/rfcomm0")
        baud = int(ask_float("  Baud rate", default=9600))
        # quick sanity check
        if not Path(port).exists():
            print(f"  {YEL}⚠ {port} not found — will still try (Bluetooth may create it on connect){RST}")
        launch_game("serial", gps_port=port, baud=baud)
    else:
        print(f"\n{BOLD}GPS GPIO bit-bang setup{RST} {DIM}(when TFT uses pins 8/10){RST}")
        pin = int(ask_float("  GPIO pin (BCM numbering)", default=16))
        baud = int(ask_float("  Baud rate", default=9600))
        launch_game("gpio", gpio_pin=pin, baud=baud)

if __name__ == "__main__":
    main()
