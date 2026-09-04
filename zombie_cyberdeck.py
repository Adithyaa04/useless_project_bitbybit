#!/usr/bin/env python3
"""
Zombie Cyberdeck - GPS zombie-chase game for a terminal on a small TFT display.
Designed for Raspberry Pi 3 running Debian in console mode on a 3.5" TFT.

Run for real, with a NMEA GPS module on serial:
    python3 zombie_cyberdeck.py --gps /dev/ttyS0 --baud 9600

Run indoors for testing, with WASD-simulated GPS:
    python3 zombie_cyberdeck.py --sim

Dependencies (only needed for real GPS mode):
    pip install pynmea2 pyserial
"""

import argparse
import curses
import math
import random
import time
import threading

# ---------------- Config (tune these to taste) ----------------
SCALE_M_PER_CELL = 3.0      # meters represented by one terminal character cell
ZOMBIE_COUNT = 6
ZOMBIE_SPEED_MPS = 1.1      # zombie walking speed
ZOMBIE_JITTER = 0.35        # heading randomness (radians) so paths aren't dead straight
CATCH_RADIUS_M = 2.5        # distance at which a zombie "gets" you
TICK_HZ = 4                 # game update rate
SPAWN_MIN_M = 40            # zombies spawn this far from you at minimum
SPAWN_MAX_M = 90

GRID_SPACING_M = 15         # spacing of the background reference grid, in meters
TRAIL_MIN_STEP_M = 2.0      # drop a breadcrumb every time you move at least this far
TRAIL_MAX_POINTS = 300      # cap so it doesn't grow forever on a long walk


# ---------------- Coordinate helpers ----------------
def latlon_to_local(lat, lon, lat0, lon0):
    """Equirectangular approximation -> local meters (x = east, y = north).
    Accurate enough for play areas up to a few km across."""
    R = 6371000.0
    x = math.radians(lon - lon0) * R * math.cos(math.radians(lat0))
    y = math.radians(lat - lat0) * R
    return x, y


# ---------------- GPS sources ----------------
class GPSSource:
    """Base class. get() -> (lat, lon) or (None, None) until a fix exists."""
    def __init__(self):
        self.lat = None
        self.lon = None
        self.lock = threading.Lock()

    def get(self):
        with self.lock:
            return self.lat, self.lon


class SerialGPS(GPSSource):
    """Reads NMEA GGA sentences from a serial GPS module (e.g. NEO-6M) in a
    background thread so a slow read never stalls the render loop."""
    def __init__(self, port, baud):
        super().__init__()
        import serial
        import pynmea2
        self._pynmea2 = pynmea2
        self.serial = serial.Serial(port, baud, timeout=1)
        self.running = True
        self.thread = threading.Thread(target=self._loop, daemon=True)
        self.thread.start()

    def _loop(self):
        while self.running:
            try:
                raw = self.serial.readline().decode('ascii', errors='replace').strip()
                if raw.startswith('$GPGGA') or raw.startswith('$GNGGA'):
                    msg = self._pynmea2.parse(raw)
                    if msg.gps_qual and int(msg.gps_qual) > 0:
                        with self.lock:
                            self.lat = msg.latitude
                            self.lon = msg.longitude
            except Exception:
                continue  # bad/partial sentence, just try the next line


class SimulatedGPS(GPSSource):
    """WASD-controlled fake GPS for testing indoors, no hardware needed."""
    def __init__(self, start_lat=10.000000, start_lon=76.300000):
        super().__init__()
        self.origin_lat = start_lat
        self.origin_lon = start_lon
        self._x = 0.0
        self._y = 0.0
        self.lat = start_lat
        self.lon = start_lon

    def move(self, dx, dy):
        R = 6371000.0
        with self.lock:
            self._x += dx
            self._y += dy
            self.lat = self.origin_lat + math.degrees(self._y / R)
            self.lon = self.origin_lon + math.degrees(
                self._x / (R * math.cos(math.radians(self.origin_lat)))
            )


# ---------------- Zombies ----------------
class Zombie:
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def step(self, target_x, target_y, dt):
        dx, dy = target_x - self.x, target_y - self.y
        dist = math.hypot(dx, dy)
        if dist < 0.01:
            return
        heading = math.atan2(dy, dx) + random.uniform(-ZOMBIE_JITTER, ZOMBIE_JITTER)
        step = ZOMBIE_SPEED_MPS * dt
        self.x += math.cos(heading) * step
        self.y += math.sin(heading) * step


# ---------------- Rendering ----------------
def world_to_screen(wx, wy, player_x, player_y, cx, cy):
    gx = cx + int(round((wx - player_x) / SCALE_M_PER_CELL))
    gy = cy - int(round((wy - player_y) / SCALE_M_PER_CELL))
    return gx, gy


def draw_grid(stdscr, player_x, player_y, cx, cy, max_x, max_y):
    """Draws dots at fixed world-space intervals. Because these are anchored
    to absolute coordinates (not to the player), they visibly scroll past
    as you move -- that's what actually reads as 'I am walking'."""
    half_w_m = (max_x / 2 + 1) * SCALE_M_PER_CELL
    half_h_m = (max_y / 2 + 1) * SCALE_M_PER_CELL

    start_x = math.floor((player_x - half_w_m) / GRID_SPACING_M) * GRID_SPACING_M
    end_x = player_x + half_w_m
    start_y = math.floor((player_y - half_h_m) / GRID_SPACING_M) * GRID_SPACING_M
    end_y = player_y + half_h_m

    wx = start_x
    while wx <= end_x:
        wy = start_y
        while wy <= end_y:
            gx, gy = world_to_screen(wx, wy, player_x, player_y, cx, cy)
            if 0 <= gy < max_y - 1 and 0 <= gx < max_x:
                stdscr.addstr(gy, gx, '.', curses.color_pair(4))
            wy += GRID_SPACING_M
        wx += GRID_SPACING_M


def draw_trail(stdscr, trail, player_x, player_y, cx, cy, max_x, max_y):
    for wx, wy in trail:
        gx, gy = world_to_screen(wx, wy, player_x, player_y, cx, cy)
        if 0 <= gy < max_y - 1 and 0 <= gx < max_x:
            stdscr.addstr(gy, gx, '\u00b7', curses.color_pair(4))  # middle dot


def draw_compass(stdscr, cx, cy, max_x, max_y):
    stdscr.addstr(0, max(0, cx), 'N', curses.color_pair(3) | curses.A_DIM)
    stdscr.addstr(min(max_y - 2, max_y - 2), max(0, cx), 'S', curses.color_pair(3) | curses.A_DIM)
    stdscr.addstr(cy, 0, 'W', curses.color_pair(3) | curses.A_DIM)
    stdscr.addstr(cy, max_x - 1, 'E', curses.color_pair(3) | curses.A_DIM)


def render(stdscr, player_x, player_y, zombies, trail, status=""):
    stdscr.erase()
    max_y, max_x = stdscr.getmaxyx()
    cx, cy = max_x // 2, max_y // 2

    draw_grid(stdscr, player_x, player_y, cx, cy, max_x, max_y)
    draw_trail(stdscr, trail, player_x, player_y, cx, cy, max_x, max_y)
    draw_compass(stdscr, cx, cy, max_x, max_y)

    stdscr.addstr(cy, cx, '@', curses.color_pair(1) | curses.A_BOLD)

    for z in zombies:
        gx, gy = world_to_screen(z.x, z.y, player_x, player_y, cx, cy)
        if 0 <= gy < max_y - 1 and 0 <= gx < max_x:
            stdscr.addstr(gy, gx, 'Z', curses.color_pair(2) | curses.A_BOLD)

    stdscr.addstr(max_y - 1, 0, status[:max_x - 1], curses.color_pair(3))
    stdscr.refresh()


def death_screen(stdscr):
    stdscr.erase()
    max_y, max_x = stdscr.getmaxyx()
    msg = "YOU DIED"
    stdscr.attron(curses.color_pair(2) | curses.A_BOLD)
    stdscr.addstr(max_y // 2, max(0, (max_x - len(msg)) // 2), msg)
    stdscr.attroff(curses.color_pair(2) | curses.A_BOLD)
    stdscr.addstr(max_y // 2 + 2, max(0, (max_x - 16) // 2), "press q to quit")
    stdscr.refresh()
    stdscr.nodelay(False)
    while True:
        if stdscr.getch() in (ord('q'), ord('Q')):
            break


# ---------------- Main loop ----------------
def main(stdscr, args):
    curses.curs_set(0)
    stdscr.nodelay(True)
    curses.start_color()
    curses.init_pair(1, curses.COLOR_GREEN, curses.COLOR_BLACK)   # you
    curses.init_pair(2, curses.COLOR_RED, curses.COLOR_BLACK)     # zombies / death
    curses.init_pair(3, curses.COLOR_WHITE, curses.COLOR_BLACK)   # status bar / compass
    curses.init_pair(4, curses.COLOR_CYAN, curses.COLOR_BLACK)    # grid / trail (background)

    gps = SimulatedGPS() if args.sim else SerialGPS(args.gps, args.baud)

    stdscr.addstr(0, 0, "Waiting for GPS fix...")
    stdscr.refresh()
    while gps.get()[0] is None:
        if args.sim:
            break  # simulated GPS has a fix immediately
        time.sleep(0.5)

    origin_lat, origin_lon = gps.get()

    zombies = []
    for _ in range(ZOMBIE_COUNT):
        dist = random.uniform(SPAWN_MIN_M, SPAWN_MAX_M)
        bearing = random.uniform(0, 2 * math.pi)
        zombies.append(Zombie(math.cos(bearing) * dist, math.sin(bearing) * dist))

    dt = 1.0 / TICK_HZ
    last_time = time.time()
    trail = []
    last_trail_pos = None

    while True:
        now = time.time()
        if now - last_time < dt:
            time.sleep(dt - (now - last_time))
        last_time = time.time()

        key = stdscr.getch()
        if key in (ord('q'), ord('Q')):
            break
        if args.sim:
            step = ZOMBIE_SPEED_MPS * 1.6 * dt  # you're a bit faster than a zombie
            if key == ord('w'): gps.move(0, step)
            elif key == ord('s'): gps.move(0, -step)
            elif key == ord('a'): gps.move(-step, 0)
            elif key == ord('d'): gps.move(step, 0)

        lat, lon = gps.get()
        if lat is None:
            continue
        px, py = latlon_to_local(lat, lon, origin_lat, origin_lon)

        if last_trail_pos is None or math.hypot(px - last_trail_pos[0], py - last_trail_pos[1]) >= TRAIL_MIN_STEP_M:
            trail.append((px, py))
            trail = trail[-TRAIL_MAX_POINTS:]
            last_trail_pos = (px, py)

        min_dist = float('inf')
        for z in zombies:
            z.step(px, py, dt)
            min_dist = min(min_dist, math.hypot(z.x - px, z.y - py))

        status = (f"nearest: {min_dist:5.1f}m  zombies: {len(zombies)}  "
                  f"({'wasd to move, ' if args.sim else ''}q=quit)")
        render(stdscr, px, py, zombies, trail, status)

        if min_dist <= CATCH_RADIUS_M:
            death_screen(stdscr)
            break


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--sim', action='store_true',
                         help='use keyboard-simulated GPS (WASD) for indoor testing')
    parser.add_argument('--gps', default='/dev/ttyS0', help='serial port for real GPS module')
    parser.add_argument('--baud', type=int, default=9600)
    args = parser.parse_args()
    curses.wrapper(main, args)