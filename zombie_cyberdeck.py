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
import json
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

MAP_LINE_STEP_CAP = 100     # max interpolation steps per road segment (perf safety cap)
POI_CALLOUT_RADIUS_M = 30   # how close you need to be for a place's name to show in the status bar

# category -> (single-char marker, curses color pair). Color pairs are set up
# in main(); 5-9 are POI-specific so they read differently from roads/zombies.
POI_STYLE = {
    'school':  ('S', 5),
    'worship': ('C', 6),
    'hospital':('H', 2),
    'food':    ('F', 7),
    'park':    ('P', 8),
    'fuel':    ('U', 3),
    'bank':    ('B', 3),
    'police':  ('!', 2),
}
POI_DEFAULT_STYLE = ('*', 8)


# ---------------- Coordinate helpers ----------------
def latlon_to_local(lat, lon, lat0, lon0):
    """Equirectangular approximation -> local meters (x = east, y = north).
    Accurate enough for play areas up to a few km across."""
    R = 6371000.0
    x = math.radians(lon - lon0) * R * math.cos(math.radians(lat0))
    y = math.radians(lat - lat0) * R
    return x, y


# ---------------- Offline map data (from fetch_map.py) ----------------
def load_map(path, fallback_lat, fallback_lon):
    """Loads the pre-fetched road+POI data. Returns (origin_lat, origin_lon,
    segments, pois). segments is a flat list of ((x1,y1),(x2,y2)) tuples in
    local meters, already projected. pois is a list of dicts with x, y, name,
    category, also pre-projected -- so per-frame drawing is just arithmetic."""
    try:
        with open(path) as f:
            data = json.load(f)
    except (OSError, json.JSONDecodeError):
        return fallback_lat, fallback_lon, [], []

    origin_lat = data['origin_lat']
    origin_lon = data['origin_lon']
    segments = []
    for way in data.get('ways', []):
        pts = [latlon_to_local(lat, lon, origin_lat, origin_lon) for lat, lon in way]
        for i in range(len(pts) - 1):
            segments.append((pts[i], pts[i + 1]))

    pois = []
    for poi in data.get('pois', []):
        x, y = latlon_to_local(poi['lat'], poi['lon'], origin_lat, origin_lon)
        pois.append({'x': x, 'y': y, 'name': poi['name'], 'category': poi.get('category', 'other')})

    return origin_lat, origin_lon, segments, pois


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


def draw_map(stdscr, segments, player_x, player_y, cx, cy, max_x, max_y):
    """Draws the offline road data as a dim dotted line ('.') rather than solid
    '#' blocks, so it reads as background texture instead of competing with
    the player/zombie glyphs. Cheap: it's just line interpolation between two
    already-projected points, no per-frame geometry work and no network/file
    access -- fine for a Pi 3 at a few Hz."""
    margin = 5
    for (x1, y1), (x2, y2) in segments:
        gx1, gy1 = world_to_screen(x1, y1, player_x, player_y, cx, cy)
        gx2, gy2 = world_to_screen(x2, y2, player_x, player_y, cx, cy)

        # skip segments nowhere near the visible screen
        if (gx1 < -margin and gx2 < -margin) or (gx1 > max_x + margin and gx2 > max_x + margin):
            continue
        if (gy1 < -margin and gy2 < -margin) or (gy1 > max_y + margin and gy2 > max_y + margin):
            continue

        steps = min(max(abs(gx2 - gx1), abs(gy2 - gy1), 1), MAP_LINE_STEP_CAP)
        for i in range(steps + 1):
            t = i / steps
            gx = round(gx1 + (gx2 - gx1) * t)
            gy = round(gy1 + (gy2 - gy1) * t)
            if 0 <= gy < max_y - 1 and 0 <= gx < max_x:
                stdscr.addstr(gy, gx, '.', curses.color_pair(4) | curses.A_DIM)


def draw_pois(stdscr, pois, player_x, player_y, cx, cy, max_x, max_y):
    """Draws named points of interest (schools, churches, shops...) as single
    labeled markers, mapscii-style. Just point placement, same cost as a zombie."""
    for poi in pois:
        gx, gy = world_to_screen(poi['x'], poi['y'], player_x, player_y, cx, cy)
        if 0 <= gy < max_y - 1 and 0 <= gx < max_x:
            char, pair = POI_STYLE.get(poi['category'], POI_DEFAULT_STYLE)
            stdscr.addstr(gy, gx, char, curses.color_pair(pair) | curses.A_BOLD)


def nearest_poi_name(px, py, pois, max_dist=POI_CALLOUT_RADIUS_M):
    """Returns the name of the closest POI within max_dist meters, or None."""
    best_name, best_dist = None, max_dist
    for poi in pois:
        d = math.hypot(poi['x'] - px, poi['y'] - py)
        if d < best_dist:
            best_dist, best_name = d, poi['name']
    return best_name


def render(stdscr, player_x, player_y, zombies, segments, pois, status=""):
    stdscr.erase()
    max_y, max_x = stdscr.getmaxyx()
    cx, cy = max_x // 2, max_y // 2

    draw_map(stdscr, segments, player_x, player_y, cx, cy, max_x, max_y)
    draw_pois(stdscr, pois, player_x, player_y, cx, cy, max_x, max_y)

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
    curses.init_pair(3, curses.COLOR_WHITE, curses.COLOR_BLACK)   # status bar
    curses.init_pair(4, curses.COLOR_YELLOW, curses.COLOR_BLACK)  # map roads (background)
    curses.init_pair(5, curses.COLOR_CYAN, curses.COLOR_BLACK)    # schools
    curses.init_pair(6, curses.COLOR_MAGENTA, curses.COLOR_BLACK) # places of worship
    curses.init_pair(7, curses.COLOR_YELLOW, curses.COLOR_BLACK)  # food/shops
    curses.init_pair(8, curses.COLOR_BLUE, curses.COLOR_BLACK)    # parks / other POIs

    # Load the pre-fetched road data (see fetch_map.py). Its origin_lat/lon
    # become the fixed coordinate frame for this whole session, so the map
    # and your live GPS position always line up.
    map_origin_lat, map_origin_lon, segments, pois = load_map(
        args.map, fallback_lat=10.0, fallback_lon=76.3
    )
    if not segments:
        stdscr.addstr(0, 0, f"No map data found at '{args.map}' -- playing with a blank "
                             f"background. Run fetch_map.py first for a map.")
        stdscr.refresh()
        time.sleep(1.5)

    if args.sim:
        gps = SimulatedGPS(start_lat=map_origin_lat, start_lon=map_origin_lon)
    else:
        gps = SerialGPS(args.gps, args.baud)

    stdscr.addstr(0, 0, "Waiting for GPS fix...")
    stdscr.refresh()
    while gps.get()[0] is None:
        if args.sim:
            break  # simulated GPS has a fix immediately
        time.sleep(0.5)

    origin_lat, origin_lon = map_origin_lat, map_origin_lon

    zombies = []
    for _ in range(ZOMBIE_COUNT):
        dist = random.uniform(SPAWN_MIN_M, SPAWN_MAX_M)
        bearing = random.uniform(0, 2 * math.pi)
        zombies.append(Zombie(math.cos(bearing) * dist, math.sin(bearing) * dist))

    dt = 1.0 / TICK_HZ
    last_time = time.time()

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

        min_dist = float('inf')
        for z in zombies:
            z.step(px, py, dt)
            min_dist = min(min_dist, math.hypot(z.x - px, z.y - py))

        near = nearest_poi_name(px, py, pois)
        status = (f"nearest zombie: {min_dist:5.1f}m  "
                  f"{('near: ' + near + '  ') if near else ''}"
                  f"({'wasd to move, ' if args.sim else ''}q=quit)")
        render(stdscr, px, py, zombies, segments, pois, status)

        if min_dist <= CATCH_RADIUS_M:
            death_screen(stdscr)
            break


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--sim', action='store_true',
                         help='use keyboard-simulated GPS (WASD) for indoor testing')
    parser.add_argument('--gps', default='/dev/ttyS0', help='serial port for real GPS module')
    parser.add_argument('--baud', type=int, default=9600)
    parser.add_argument('--map', default='map_data.json',
                         help='path to the offline map file produced by fetch_map.py')
    args = parser.parse_args()
    curses.wrapper(main, args)