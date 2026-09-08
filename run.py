#!/usr/bin/env python3
"""
Z-DECK master script — one entry point for everything.

It CHECKS the repo + hardware, offers to INSTALL / SET UP whatever is
missing (asking first — nothing destructive runs silently), and LAUNCHES
the game when you're ready.

Usage:
    python3 run.py               # interactive master menu (recommended)
    python3 run.py --check       # verify only: no changes, no questions
    python3 run.py --sim         # checks, then straight into SIM mode
    python3 run.py --setup       # setup flows only, then exit (no launch)
    python3 run.py --yes         # assume YES to every prompt (careful: may
                                 # run sudo installs + reboot prompts)

What it covers:
  [repo]     app + rust + os/pi3-arm64 + cardkb files, Pi binaries, map data
  [deps]     python pkgs (pynmea2/pyserial…), cargo, container engine
  [hardware] I2C/CardKB 0x5F, GPS /dev/ttyAMA0, uinput, services (Pi only)
  [setup]    cardkb/setup.sh, install-autostart.sh, map fetch (all optional)
  [launch]   Rust (cargo) or Python cyberdeck, sim / serial-GPS / gpio-GPS
"""

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent
APP = ROOT / "app"
RUST = ROOT / "rust"
OS_PI = ROOT / "os" / "pi3-arm64"
CARDKB = ROOT / "cardkb"
BIN_ARM64 = ROOT / "binary" / "pi3-arm64"
MAP_FILE = APP / "map_data.json"

# ---------------------------------------------------------------- colours
BOLD = "\033[1m"
DIM = "\033[2m"
GRN = "\033[92m"
RED = "\033[91m"
YEL = "\033[93m"
CYA = "\033[96m"
RST = "\033[0m"

OK, WARN, FAIL, INFO = "ok", "warn", "fail", "info"


@dataclass
class Check:
    group: str
    name: str
    status: str  # ok | warn | fail | info
    detail: str = ""
    fix: str = ""


CHECKS: list[Check] = []


def add(group, name, status, detail="", fix=""):
    CHECKS.append(Check(group, name, status, detail, fix))


def has(cmd: str) -> bool:
    return shutil.which(cmd) is not None


def run(cmd, cwd=None, check=False, capture=False):
    """Run a command, echoing it first. Returns CompletedProcess."""
    print(f"{DIM}$ {' '.join(str(c) for c in cmd)}{RST}")
    return subprocess.run(
        [str(c) for c in cmd],
        cwd=str(cwd) if cwd else None,
        check=check,
        capture_output=capture,
        text=capture,
    )


def is_root() -> bool:
    return os.geteuid() == 0 if hasattr(os, "geteuid") else False


def sudo_prefix():
    """[] if root, ['sudo'] if sudo exists, None if no privilege path."""
    if is_root():
        return []
    return ["sudo"] if has("sudo") else None


# ------------------------------------------------------------- prompts
ASSUME_YES = False


def ask_yes(prompt, default=True) -> bool:
    if ASSUME_YES:
        print(f"{prompt} [{'Y/n' if default else 'y/N'}] -> {'Y' if default else 'n'} (auto)")
        return default
    hint = "Y/n" if default else "y/N"
    try:
        ans = input(f"{BOLD}{prompt}{RST} {DIM}[{hint}]{RST} ").strip().lower()
    except EOFError:
        return default
    if not ans:
        return default
    return ans in ("y", "yes")


def ask_choice(prompt, options, default=None):
    """options = [(key, label, ...)]. Returns key."""
    keys = [k for k, *_ in options]
    if ASSUME_YES and default:
        print(f"{prompt} -> {default} (auto)")
        return default
    print(f"\n{BOLD}{prompt}{RST}")
    for k, label, *_ in options:
        print(f"  {YEL}{k}{RST}) {label}")
    dflt = f" [{default}]" if default else ""
    while True:
        try:
            ans = input(f"> {DIM}(choice{dflt}){RST} ").strip().lower()
        except EOFError:
            return default or "q"
        if not ans and default:
            return default
        if ans in keys:
            return ans
        print(f"{RED}  pick one of: {', '.join(keys)}{RST}")


def ask_str(prompt, default=""):
    hint = f" [{default}]" if default else ""
    try:
        ans = input(f"{prompt}{hint}: ").strip()
    except EOFError:
        return default
    return ans or default


def ask_float(prompt, default):
    while True:
        try:
            ans = input(f"{prompt} [{default}]: ").strip()
        except EOFError:
            return float(default)
        if not ans:
            return float(default)
        try:
            return float(ans)
        except ValueError:
            print(f"{RED}  enter a number.{RST}")


# ------------------------------------------------------------- platform
def detect_platform():
    arch = platform.machine()
    pretty = arch
    try:
        with open("/etc/os-release") as f:
            for line in f:
                if line.startswith("PRETTY_NAME="):
                    pretty = f"{line.split('=', 1)[1].strip().strip(chr(34))} ({arch})"
    except OSError:
        pretty = f"{platform.system()} ({arch})"
    model = ""
    try:
        model = Path("/proc/device-tree/model").read_text().strip("\x00").strip()
    except OSError:
        pass
    pi_cfg = next(
        (c for c in ("/boot/firmware/config.txt", "/boot/config.txt") if Path(c).exists()),
        "",
    )
    is_pi = bool(pi_cfg) or "raspberry" in model.lower()
    return {"arch": arch, "pretty": pretty, "model": model, "pi_cfg": pi_cfg, "is_pi": is_pi}


# ------------------------------------------------------------- checks
def check_repo():
    add("repo", "layout", INFO, f"root={ROOT}")
    files = {
        APP / "cyberdeck.py": "Python game",
        APP / "fetch_map.py": "map fetcher",
        RUST / "Cargo.toml": "Rust workspace",
        RUST / "src" / "bin" / "zdeck-run.rs": "Rust launcher",
        RUST / "src" / "bin" / "zdeck-cardkb.rs": "Rust CardKB driver",
        OS_PI / "zdeck-main.sh": "main run script",
        OS_PI / "zdeck-auto.sh": "game loop",
        OS_PI / "install-autostart.sh": "autostart installer",
        CARDKB / "setup.sh": "CardKB setup",
    }
    for path, label in files.items():
        rel = path.relative_to(ROOT)
        if path.exists():
            add("repo", label, OK, str(rel))
        else:
            add("repo", label, FAIL, f"missing {rel}", "git pull / restore the file")
    if MAP_FILE.exists():
        try:
            d = json.loads(MAP_FILE.read_text())
            add("repo", "map data", OK,
                f"{len(d.get('ways', []))} ways, {len(d.get('pois', []))} POIs "
                f"@ {d.get('origin_lat')},{d.get('origin_lon')}")
        except Exception:
            add("repo", "map data", WARN, "map_data.json corrupt",
                "menu > fetch fresh map")
    else:
        add("repo", "map data", WARN, "no app/map_data.json yet",
            "menu > fetch fresh map (needs internet once)")


def check_pi_binaries():
    for b in ("zdeck-run", "zdeck-game", "zdeck-gps", "zdeck-fetch", "zdeck-cardkb"):
        p = BIN_ARM64 / b
        if p.exists() and os.access(p, os.X_OK):
            add("repo", f"Pi bin {b}", OK, f"{p.stat().st_size // 1024} KB")
        elif p.exists():
            add("repo", f"Pi bin {b}", WARN, "not executable",
                "chmod +x binary/pi3-arm64/*")
        else:
            add("repo", f"Pi bin {b}", FAIL, "missing",
                "./rust/build-pi.sh arm64 (needs docker/podman)")


def check_deps(plat):
    for mod, label, optional in (
        ("pynmea2", "pynmea2 (GPS parse)", False),
        ("serial", "pyserial (GPS serial)", False),
        ("pigpio", "pigpio (GPIO bit-bang)", True),
    ):
        try:
            __import__(mod)
            add("deps", label, OK, "importable")
        except ImportError:
            add("deps", label, WARN if optional else FAIL,
                "not installed",
                "menu > install Python deps" + (" (only for --gpio)" if optional else ""))
    if has("cargo"):
        add("deps", "cargo", OK, "native Rust builds work")
    else:
        add("deps", "cargo", WARN, "not found", "https://rustup.rs (dev machines only)")
    if has("docker") or has("podman"):
        add("deps", "container engine", OK, "Pi cross-builds work")
    else:
        add("deps", "container engine", WARN, "no docker/podman",
            "needed for ./rust/build-pi.sh")
    if has("uv"):
        add("deps", "uv", OK, "fast Python installs")
    else:
        add("deps", "uv", INFO, "not found", "pip is used instead (fine)")


def check_hardware(plat):
    if not plat["is_pi"]:
        add("hardware", "Pi hardware", INFO, "not a Pi — hardware checks skipped",
            "run this on the deck for I2C/GPS/uinput checks")
        return
    # I2C / CardKB
    if Path("/dev/i2c-1").exists():
        add("hardware", "/dev/i2c-1", OK, "I2C bus present")
        if has("i2cdetect"):
            try:
                out = subprocess.run(["i2cdetect", "-y", "1"], capture_output=True,
                                     text=True, timeout=15).stdout.lower()
                if "5f" in out:
                    add("hardware", "CardKB 0x5F", OK, "seen on bus 1")
                else:
                    add("hardware", "CardKB 0x5F", WARN, "not on bus 1",
                        "check wiring SDA->GPIO2 SCL->GPIO3 VCC->3.3V")
            except Exception as e:
                add("hardware", "CardKB 0x5F", WARN, f"scan failed: {e}")
        else:
            add("hardware", "CardKB 0x5F", WARN, "i2c-tools missing",
                "sudo apt install i2c-tools (cardkb/setup.sh does it)")
    else:
        add("hardware", "/dev/i2c-1", FAIL, "missing",
            "enable I2C: cardkb/setup.sh (dtparam=i2c_arm=on) + reboot")
    # GPS UART
    if Path("/dev/ttyAMA0").exists():
        add("hardware", "/dev/ttyAMA0", OK, "deck GPS device")
    else:
        add("hardware", "/dev/ttyAMA0", WARN, "missing",
            "enable_uart=1 + reboot (--sim still works)")
    # uinput
    try:
        mods = Path("/proc/modules").read_text() if Path("/proc/modules").exists() else ""
        add("hardware", "uinput module", OK if "\nuinput " in "\n" + mods else WARN,
            "loaded" if "\nuinput " in "\n" + mods else "not loaded",
            "" if "\nuinput " in "\n" + mods else "cardkb/setup.sh (modprobe uinput)")
    except OSError:
        add("hardware", "uinput module", WARN, "cannot read /proc/modules")
    try:
        groups = subprocess.run(["groups"], capture_output=True, text=True,
                                timeout=5).stdout.split()
        add("hardware", "input group", OK if "input" in groups else WARN,
            f"groups: {' '.join(groups)}",
            "" if "input" in groups else "cardkb/setup.sh (usermod -aG input)")
    except Exception:
        add("hardware", "input group", WARN, "groups lookup failed")
    try:
        devs = Path("/proc/bus/input/devices").read_text()
        add("hardware", "CardKB input dev", OK if "CardKB" in devs else WARN,
            "registered" if "CardKB" in devs else "not registered",
            "" if "CardKB" in devs else "sudo bash cardkb/diag.sh (finds the failing layer)")
    except OSError:
        add("hardware", "CardKB input dev", WARN, "no /proc/bus/input/devices")
    # services / hook
    if has("systemctl"):
        for svc, label in (("cardkb.service", "cardkb.service"),
                           ("zdeck.service", "zdeck.service")):
            try:
                r = subprocess.run(["systemctl", "is-active", svc], capture_output=True,
                                   text=True, timeout=5)
                active = r.stdout.strip() == "active"
                if svc == "zdeck.service" and not active:
                    add("hardware", label, INFO, "inactive (login hook is the default)")
                else:
                    add("hardware", label, OK if active else WARN,
                        "active" if active else "inactive")
            except Exception:
                add("hardware", label, WARN, "systemctl query failed")
    home = Path.home()
    prof = home / ".bash_profile"
    hook = prof.exists() and "zdeck/zdeck-main.sh" in prof.read_text()
    add("hardware", "login hook", OK if hook else WARN,
        "~/.bash_profile -> zdeck-main.sh" if hook else "no hook",
        "" if hook else "menu > autostart setup")


def print_report():
    counts = {OK: 0, WARN: 0, FAIL: 0, INFO: 0}
    last_group = None
    for c in CHECKS:
        counts[c.status] += 1
        if c.group != last_group:
            print(f"\n{BOLD}[{c.group}]{RST}")
            last_group = c.group
        dot = {OK: f"{GRN}●{RST}", WARN: f"{YEL}●{RST}",
               FAIL: f"{RED}●{RST}", INFO: f"{DIM}●{RST}"}[c.status]
        line = f"  {dot} {c.name}: {c.detail}"
        if c.fix:
            line += f" {DIM}→ {c.fix}{RST}"
        print(line)
    print(f"\n{BOLD}result: {GRN}{counts[OK]} ok{RST}, "
          f"{YEL}{counts[WARN]} warnings{RST}, {RED}{counts[FAIL]} failures{RST} "
          f"{DIM}({counts[INFO]} info){RST}")
    return counts


# ------------------------------------------------------------- setup actions
def need_privilege(action: str):
    """Confirm sudo usage. Returns prefix list or None if declined/unavailable."""
    pre = sudo_prefix()
    if pre == []:
        return []
    if pre is None:
        print(f"{YEL}  no sudo available — cannot: {action}{RST}")
        return None
    if ask_yes(f"  Run with sudo ({action})?"):
        return ["sudo"]
    print(f"{DIM}  skipped.{RST}")
    return None


def act_python_deps():
    print(f"\n{BOLD}Install Python deps{RST} {DIM}(pynmea2 pyserial [+ pigpio]){RST}")
    pkgs = ["pynmea2", "pyserial"]
    if ask_yes("  Also install pigpio (only needed for --gpio bit-bang)?", default=False):
        pkgs.append("pigpio")
    if has("uv"):
        cmd = ["uv", "pip", "install", "--system", "--break-system-packages", *pkgs]
    else:
        cmd = [sys.executable, "-m", "pip", "install", "--break-system-packages", *pkgs]
    pre = need_privilege("python install may need root") if not is_root() else []
    if pre is None:
        return False
    try:
        run(pre + cmd)
        print(f"{GRN}  done.{RST}")
        return True
    except subprocess.CalledProcessError:
        print(f"{RED}  install failed — retry manually; Rust path needs no Python.{RST}")
        return False


def act_build_native():
    print(f"\n{BOLD}Build Rust binaries natively (dev machine){RST}")
    if not has("cargo"):
        print(f"{YEL}  cargo not found — install from https://rustup.rs{RST}")
        return False
    try:
        run(["cargo", "build", "--bins"], cwd=RUST)
        print(f"{GRN}  built: rust/target/debug/zdeck-* (run with --sim for tests){RST}")
        return True
    except subprocess.CalledProcessError:
        print(f"{RED}  build failed — see errors above.{RST}")
        return False


def act_build_pi(plat):
    print(f"\n{BOLD}Cross-build Pi binaries{RST}")
    if not (has("docker") or has("podman")):
        print(f"{YEL}  needs docker or podman — install one first.{RST}")
        return False
    if plat["is_pi"]:
        print(f"{DIM}  note: you ARE on the Pi — native build also possible, "
              f"but cross-build still works.{RST}")
    which = ask_choice("Which target?", [
        ("1", "arm64 — 64-bit Pi OS (your deck)"),
        ("2", "armv7 — 32-bit Pi OS"),
        ("3", "both"),
    ], default="1")
    args = {"1": ["arm64"], "2": ["armv7"], "3": []}[which]
    try:
        run(["./rust/build-pi.sh", *args], cwd=ROOT)
        print(f"{GRN}  built: binary/pi3-*/zdeck-* (5 binaries incl. zdeck-cardkb){RST}")
        return True
    except subprocess.CalledProcessError:
        print(f"{RED}  cross-build failed — is the container engine running?{RST}")
        return False


def act_cardkb_setup(plat):
    print(f"\n{BOLD}CardKB setup{RST} {DIM}(I2C + uinput + cardkb.service){RST}")
    if not plat["is_pi"] and not ask_yes("  Not a Pi — run anyway?", default=False):
        return False
    pre = need_privilege("cardkb/setup.sh requires root")
    if pre is None:
        return False
    yn = ask_choice("Reboot behaviour?", [
        ("1", "ask me before rebooting (recommended)"),
        ("2", "never reboot automatically (--no-reboot)"),
        ("3", "auto-yes to everything (--yes)"),
    ], default="1")
    flags = {"1": [], "2": ["--no-reboot"], "3": ["--yes"]}[yn]
    try:
        run(pre + ["bash", str(CARDKB / "setup.sh"), *flags], cwd=ROOT)
        return True
    except subprocess.CalledProcessError:
        print(f"{RED}  cardkb setup exited non-zero — read the log above.{RST}")
        return False


def act_autostart(plat):
    print(f"\n{BOLD}Autostart setup{RST} {DIM}(login hook on tty1 -> zdeck-main.sh){RST}")
    print(f"{DIM}  This does NOT configure auto-login — enable Console Autologin "
          f"via raspi-config yourself if you want boot-to-game.{RST}")
    if not plat["is_pi"] and not ask_yes("  Not a Pi — run anyway?", default=False):
        return False
    mode = ask_choice("Install mode?", [
        ("1", "check only — change nothing (--check-only)"),
        ("2", "install login hook (recommended)"),
        ("3", "install + also enable zdeck.service (--with-systemd)"),
    ], default="1" if not plat["is_pi"] else "2")
    flags = {"1": ["--check-only"], "2": ["--yes"], "3": ["--yes", "--with-systemd"]}[mode]
    script = OS_PI / "install-autostart.sh"
    needs_sudo = mode != "1"  # config edits + systemd need root
    pre: list[str] = []
    if needs_sudo:
        maybe = need_privilege("autostart install touches /boot + systemd")
        if maybe is None:
            return False
        pre = maybe
    try:
        run(pre + ["bash", str(script), *flags], cwd=ROOT)
        return True
    except subprocess.CalledProcessError:
        print(f"{RED}  autostart installer exited non-zero.{RST}")
        return False


def act_fetch_map():
    print(f"\n{BOLD}Fetch map{RST} {DIM}(needs internet once, then offline){RST}")
    print(f"{DIM}  find lat/lon at openstreetmap.org (right-click → show address){RST}")
    lat = ask_float("  Center latitude", 9.9649)
    lon = ask_float("  Center longitude", 76.2868)
    rad = ask_float("  Radius meters", 300)
    try:
        run([sys.executable, str(APP / "fetch_map.py"), "--lat", str(lat),
             "--lon", str(lon), "--radius", str(rad),
             "--out", str(MAP_FILE)], cwd=ROOT)
        print(f"{GRN}  saved to {MAP_FILE}{RST}")
        return True
    except subprocess.CalledProcessError:
        print(f"{RED}  fetch failed — hotspot on? try again.{RST}")
        return False


# ------------------------------------------------------------- launch
def launch_rust_sim():
    exe = RUST / "target" / "debug" / "zdeck-run"
    if exe.exists():
        run([str(exe), "--sim"], cwd=RUST)
    elif has("cargo"):
        print(f"{DIM}  no debug build yet — building first…{RST}")
        run(["cargo", "run", "--quiet", "--bin", "zdeck-run", "--", "--sim"], cwd=RUST)
    else:
        print(f"{YEL}  no cargo and no debug binary — build first (menu).{RST}")


def launch_python(mode, port="/dev/ttyAMA0", gpio=16, baud=9600):
    cmd = [sys.executable, str(APP / "cyberdeck.py"), "--map", str(MAP_FILE)]
    if mode == "sim":
        cmd.append("--sim")
        print(f"{CYA}  Python SIM — WASD to move, Q to quit{RST}")
    elif mode == "serial":
        cmd += ["--gps", port, "--baud", str(baud)]
    else:
        cmd += ["--gpio", str(gpio), "--baud", str(baud)]
        print(f"{DIM}  needs pigpiod: sudo systemctl start pigpiod{RST}")
    if not MAP_FILE.exists():
        print(f"{YEL}  no map file — fetch one first (setup menu) or continue bare.{RST}")
        if not ask_yes("  Continue without map?", default=False):
            return
    run(cmd, cwd=ROOT)


def launch_menu():
    while True:
        ch = ask_choice("Launch what?", [
            ("1", "Rust SIM — cargo zdeck-run --sim (WASD, recommended dev test)"),
            ("2", "Python SIM — app/cyberdeck.py --sim"),
            ("3", "Python GPS serial — pick port"),
            ("4", "Python GPS gpio bit-bang — pick pin"),
            ("5", "Deck loop test — zdeck-main.sh --sim (if installed)"),
            ("q", "back to main menu"),
        ], default="1")
        if ch == "q":
            return
        if ch == "1":
            launch_rust_sim()
        elif ch == "2":
            launch_python("sim")
        elif ch == "3":
            port = ask_str("  Serial port", "/dev/ttyAMA0")
            baud = int(ask_float("  Baud", 9600))
            if not Path(port).exists():
                print(f"{YEL}  {port} not present — still trying…{RST}")
            launch_python("serial", port=port, baud=baud)
        elif ch == "4":
            gpio = int(ask_float("  GPIO pin (BCM)", 16))
            baud = int(ask_float("  Baud", 9600))
            launch_python("gpio", gpio=gpio, baud=baud)
        elif ch == "5":
            for cand in (Path.home() / "zdeck" / "zdeck-main.sh",
                         OS_PI / "zdeck-main.sh"):
                if cand.exists():
                    run([str(cand), "--sim"])
                    break
            else:
                print(f"{YEL}  zdeck-main.sh not installed — run autostart setup first.{RST}")


def setup_menu(plat):
    while True:
        ch = ask_choice("Setup / install what?", [
            ("1", "Python deps (pynmea2, pyserial…)"),
            ("2", "Build Rust natively (dev test)"),
            ("3", "Cross-build Pi binaries (docker/podman)"),
            ("4", "CardKB setup — I2C + uinput + service (Pi, sudo)"),
            ("5", "Autostart setup — login hook (Pi)"),
            ("6", "Fetch fresh map (internet)"),
            ("q", "back to main menu"),
        ], default="q")
        if ch == "q":
            return
        {"1": act_python_deps, "2": act_build_native, "3": lambda: act_build_pi(plat),
         "4": lambda: act_cardkb_setup(plat), "5": lambda: act_autostart(plat),
         "6": act_fetch_map}[ch]()
        print(f"\n{DIM}  re-running checks…{RST}")
        CHECKS.clear()
        run_all_checks(plat)
        print_report()


# ------------------------------------------------------------- main
def banner():
    print(f"""{GRN}{BOLD}
   ███████╗         ██████╗ ███████╗ ██████╗██╗  ██╗
   ╚══███╔╝         ██╔══██╗██╔════╝██╔════╝██║ ██╔╝
     ███╔╝  █████╗  ██║  ██║█████╗  ██║     █████╔╝
    ███╔╝   ╚════╝  ██║  ██║██╔══╝  ██║     ██╔═██╗
   ███████╗        ██████╔╝███████╗╚██████╗██║  ██╗
   ╚══════╝        ╚═════╝ ╚══════╝ ╚═════╝╚═╝  ╚═╝
{RST}{YEL}{BOLD}                    Z  —  D E C K{RST}{DIM}  — master setup & launch.{RST}""")


def run_all_checks(plat):
    check_repo()
    check_pi_binaries()
    check_deps(plat)
    check_hardware(plat)


def main():
    global ASSUME_YES
    ap = argparse.ArgumentParser(description="Z-DECK master setup & launch")
    ap.add_argument("--check", "--check-only", dest="check", action="store_true",
                    help="verify everything, change nothing, ask nothing")
    ap.add_argument("--setup", action="store_true", help="setup flows only, no launch")
    ap.add_argument("--sim", action="store_true", help="checks, then straight to Rust SIM")
    ap.add_argument("--yes", action="store_true", help="assume YES to all prompts")
    args = ap.parse_args()
    ASSUME_YES = args.yes

    plat = detect_platform()
    banner()
    print(f"{DIM}platform: {plat['pretty']}{RST}")
    if plat["model"]:
        print(f"{DIM}model: {plat['model']}{RST}")
    print(f"{DIM}host role: {'PI / deck' if plat['is_pi'] else 'dev machine'}"
          f" — hardware setup steps {'apply' if plat['is_pi'] else 'are skipped (run on the Pi)'}"
          f"{RST}")

    print(f"\n{BOLD}Checking everything…{RST}")
    run_all_checks(plat)
    counts = print_report()

    if args.check:
        return 0 if counts[FAIL] == 0 else 1

    if args.sim:
        print(f"\n{BOLD}Launching SIM…{RST}")
        launch_rust_sim()
        return 0

    if counts[FAIL]:
        print(f"\n{YEL}Some required pieces are missing. Opening setup…{RST}")
        setup_menu(plat)
    elif args.setup:
        setup_menu(plat)
        return 0

    while True:
        ch = ask_choice("Master menu", [
            ("1", "Setup / install / fix something"),
            ("2", "Re-run checks"),
            ("3", "Launch the game"),
            ("q", "quit"),
        ], default="3" if counts[FAIL] == 0 else "1")
        if ch == "q":
            print(f"{DIM}Bye — run for your life.{RST}")
            return 0
        if ch == "1":
            setup_menu(plat)
        elif ch == "2":
            CHECKS.clear()
            run_all_checks(plat)
            counts = print_report()
        else:
            launch_menu()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print(f"\n{DIM}Interrupted.{RST}")
        sys.exit(130)
