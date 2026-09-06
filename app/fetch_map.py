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
    # The node[...] clauses pull named points of interest (schools, churches,
    # shops, hospitals, parks...) so the game can label them, mapscii-style,
    # instead of just drawing blank road lines.
    return f"""
    [out:json][timeout:25];
    (
      way["highway"](around:{radius_m},{lat},{lon});
      way["building"](around:{radius_m},{lat},{lon});
      way["natural"="water"](around:{radius_m},{lat},{lon});
      way["leisure"~"^(park|garden|playground|pitch)$"](around:{radius_m},{lat},{lon});
      way["landuse"~"^(grass|forest|meadow|recreation_ground|village_green)$"](around:{radius_m},{lat},{lon});
      node["name"]["amenity"](around:{radius_m},{lat},{lon});
      node["name"]["emergency"](around:{radius_m},{lat},{lon});
      node["name"]["healthcare"](around:{radius_m},{lat},{lon});
      node["name"]["shop"](around:{radius_m},{lat},{lon});
      node["name"]["tourism"](around:{radius_m},{lat},{lon});
      node["name"]["leisure"](around:{radius_m},{lat},{lon});
      node["name"]["office"](around:{radius_m},{lat},{lon});
      node["name"]["craft"](around:{radius_m},{lat},{lon});
      node["name"]["historic"](around:{radius_m},{lat},{lon});
      node["name"]["natural"](around:{radius_m},{lat},{lon});
    );
    out geom;
    """


# Maps an OSM tag value (amenity/shop/tourism/leisure/...) to a short category
# label we store in the map file. Keeps the game's marker/color logic simple.
# Schema v2 additions: supply (groceries/markets), civic (offices/admin),
# culture (museums/sights), shelter (hotels/hostels -- hide here!), water.
POI_TAG_TO_CATEGORY = {
    'school': 'school', 'college': 'school', 'university': 'school', 'kindergarten': 'school',
    'place_of_worship': 'worship',
    'hospital': 'hospital', 'clinic': 'hospital', 'pharmacy': 'hospital', 'doctors': 'hospital',
    'dentist': 'hospital', 'ambulance_station': 'hospital',
    'restaurant': 'food', 'cafe': 'food', 'fast_food': 'food', 'bar': 'food', 'pub': 'food',
    'food_court': 'food', 'ice_cream': 'food',
    'supermarket': 'supply', 'convenience': 'supply', 'mall': 'supply',
    'department_store': 'supply', 'marketplace': 'supply', 'greengrocer': 'supply',
    'bakery': 'supply', 'butcher': 'supply', 'beverages': 'supply', 'kiosk': 'supply',
    'newsagent': 'supply', 'hardware': 'supply',
    'park': 'park', 'garden': 'park', 'playground': 'park', 'pitch': 'park',
    'fuel': 'fuel', 'charging': 'fuel',
    'bank': 'bank', 'atm': 'bank', 'bureau_de_change': 'bank',
    'police': 'police', 'fire_station': 'police',
    'government': 'civic', 'townhall': 'civic', 'community_centre': 'civic',
    'courthouse': 'civic', 'embassy': 'civic',
    'museum': 'culture', 'gallery': 'culture', 'theatre': 'culture', 'cinema': 'culture',
    'library': 'culture', 'arts_centre': 'culture', 'attraction': 'culture',
    'viewpoint': 'culture', 'artwork': 'culture', 'historic': 'culture',
    'hotel': 'shelter', 'hostel': 'shelter', 'guest_house': 'shelter', 'motel': 'shelter',
    'water': 'water', 'spring': 'water',
}


def categorize_poi(tags):
    for key in ('amenity', 'emergency', 'healthcare', 'shop', 'tourism', 'leisure',
                'office', 'craft', 'historic', 'natural'):
        val = tags.get(key)
        if val:
            if key == 'office':
                return 'civic'  # any office is civic infrastructure
            return POI_TAG_TO_CATEGORY.get(val, key)  # fall back to the raw tag key
    return 'other'


def poi_address(tags):
    """'addr:housenumber, addr:street' if OSM has either part."""
    num, street = tags.get('addr:housenumber', ''), tags.get('addr:street', '')
    addr = ', '.join(p for p in (num, street) if p)
    return addr or None


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
    roads = []  # v2: same geometry + OSM name/highway tags
    areas = []  # v2: building / water / green polygons
    pois = []
    for el in raw.get('elements', []):
        if el.get('type') == 'way' and 'geometry' in el:
            pts = [[pt['lat'], pt['lon']] for pt in el['geometry']]
            if len(pts) < 2:
                continue
            tags = el.get('tags', {})
            highway = tags.get('highway', '')
            if highway:
                ways.append(pts)  # v1 shape, unchanged for old readers
                roads.append({'name': tags.get('name'), 'highway': highway, 'pts': pts})
            elif tags.get('building'):
                if len(areas) < 2000:
                    areas.append({'kind': 'building', 'pts': pts})
            elif tags.get('natural') == 'water':
                areas.append({'kind': 'water', 'name': tags.get('name'), 'pts': pts})
            elif tags.get('leisure') in ('park', 'garden', 'playground', 'pitch') or \
                    tags.get('landuse') in ('grass', 'forest', 'meadow',
                                            'recreation_ground', 'village_green'):
                areas.append({'kind': 'green', 'name': tags.get('name'), 'pts': pts})
        elif el.get('type') == 'node' and 'tags' in el:
            tags = el['tags']
            name = tags.get('name')
            if name:
                poi = {
                    'lat': el['lat'],
                    'lon': el['lon'],
                    'name': name,
                    'category': categorize_poi(tags),
                }
                if tags.get('opening_hours'):
                    poi['hours'] = tags['opening_hours']
                addr = poi_address(tags)
                if addr:
                    poi['addr'] = addr
                pois.append(poi)

    out = {
        'version': 2,
        'origin_lat': lat,
        'origin_lon': lon,
        'radius_m': args.radius,
        'ways': ways,
        'pois': pois,
        'roads': roads,
        'areas': areas,
    }
    with open(args.out, 'w') as f:
        json.dump(out, f)

    print(f"Saved {len(roads)} roads ({len(ways)} legacy ways), "
          f"{len(areas)} areas and {len(pois)} named places to {args.out}")
    print("Copy this file next to zombie_cyberdeck.py on the Pi -- "
          "the game will load it automatically and needs no internet from here on.")


if __name__ == '__main__':
    main()