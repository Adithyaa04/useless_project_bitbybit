#!/usr/bin/env python3
"""
Map prep tool - run this ONCE, wherever you have internet (home wifi, phone
hotspot, laptop). It downloads road/path geometry around a center coordinate
from OpenStreetMap (via the Overpass API) and saves a small local file that
zombie_cyberdeck.py reads completely offline in the field.

Uses only the Python standard library - nothing to install.

Usage (enter coordinates as arguments):
    python3 fetch_map.py --lat 10.0123 --lon 76.3456 --radius 300

Or run with no arguments and it will ask you to type them in:
    python3 fetch_map.py
"""

import argparse
import json
import urllib.error
import urllib.parse
import urllib.request

OVERPASS_URLS = [
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass.openstreetmap.ru/api/interpreter",
]


def build_query(lat, lon, radius_m):
    # "highway" covers roads, paths, tracks, footways -- anything walkable/driveable.
    return f"""
    [out:json][timeout:25];
    (
      way["highway"](around:{radius_m},{lat},{lon});
    );
    out geom;
    """


def fetch(lat, lon, radius_m):
    query = build_query(lat, lon, radius_m)
    data = urllib.parse.urlencode({'data': query}).encode()
    headers = {
        'User-Agent': 'zombie-cyberdeck-map-fetch/1.0 (personal hobby project)',
        'Accept': 'application/json',
    }

    last_err = None
    for url in OVERPASS_URLS:
        print(f"  trying {url} ...")
        req = urllib.request.Request(url, data=data, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                return json.load(resp)
        except urllib.error.HTTPError as e:
            print(f"  -> HTTP {e.code} {e.reason}")
            last_err = e
        except urllib.error.URLError as e:
            print(f"  -> failed: {e.reason}")
            last_err = e

    raise RuntimeError(
        "All Overpass mirrors failed. Check your internet connection, or the "
        "server may be temporarily down/rate-limiting -- wait a minute and retry."
    ) from last_err


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--lat', type=float, help='center latitude of your play area')
    ap.add_argument('--lon', type=float, help='center longitude of your play area')
    ap.add_argument('--radius', type=float, default=300,
                     help='meters around the center to fetch (default 300)')
    ap.add_argument('--out', default='map_data.json', help='output file path')
    args = ap.parse_args()

    lat = args.lat
    lon = args.lon
    if lat is None:
        lat = float(input("Center latitude (e.g. 10.012345): ").strip())
    if lon is None:
        lon = float(input("Center longitude (e.g. 76.345678): ").strip())

    print(f"Querying OpenStreetMap for roads within {args.radius:.0f}m of "
          f"({lat}, {lon})...")
    raw = fetch(lat, lon, args.radius)

    ways = []
    for el in raw.get('elements', []):
        if el.get('type') == 'way' and 'geometry' in el:
            pts = [[pt['lat'], pt['lon']] for pt in el['geometry']]
            if len(pts) >= 2:
                ways.append(pts)

    out = {
        'origin_lat': lat,
        'origin_lon': lon,
        'radius_m': args.radius,
        'ways': ways,
    }
    with open(args.out, 'w') as f:
        json.dump(out, f)

    print(f"Saved {len(ways)} road segments to {args.out}")
    print("Copy this file next to zombie_cyberdeck.py on the Pi -- "
          "the game will load it automatically and needs no internet from here on.")


if __name__ == '__main__':
    main()
