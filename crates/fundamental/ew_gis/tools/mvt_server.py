#!/usr/bin/env python3
"""Serve generated MVT vector tiles for the ew_gis acceptance run.

The tiles are encoded by hand (no protobuf library) so the tool stays a single
file and the encoding is readable next to the `ew_gis` decoder it exercises.

    python tools/mvt_server.py --port 8765

Then point the app at it:

    cargo run -p ew_gis -- --online --no-demo \
        --mvt-url http://127.0.0.1:8765/{z}/{x}/{y}.mvt \
        --frames 3 --screenshot target/ew-gis-mvt-2d.png

Each tile is generated on demand for its own z/x/y, with three layers:

* ``points``  - a 5x5 grid of Point features (one MultiPoint: MoveTo with a
                count, ZigZag deltas from the running cursor),
* ``roads``   - one zigzag LineString (MoveTo + LineTo),
* ``landuse`` - one Polygon with an exterior ring and an interior ring (a hole;
                the plot scene counts and skips it).

Geometry is placed in tile-local 0..extent coordinates with y growing south,
exactly the frame the decoder maps to lon/lat.
"""

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

EXTENT = 4096

# Geometry type enum values from the MVT spec.
POINT, LINESTRING, POLYGON = 1, 2, 3


# --- protobuf / MVT encoding -------------------------------------------------


def varint(value):
    """Base-128 varint for a non-negative integer."""
    if value < 0:
        raise ValueError("varint is unsigned")
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def zigzag(value):
    """ZigZag-encode a signed geometry parameter."""
    return (value << 1) ^ (value >> 31)


def field(tag, wire_type):
    return varint((tag << 3) | wire_type)


def length_delimited(tag, payload):
    return field(tag, 2) + varint(len(payload)) + payload


def packed_u32(tag, values):
    payload = b"".join(varint(value) for value in values)
    return length_delimited(tag, payload)


def encode_layer(name, features, extent=EXTENT):
    # Layer: version (15), name (1), features (2), extent (5).
    body = varint((15 << 3) | 0) + varint(2)
    body += length_delimited(1, name.encode("utf-8"))
    for feature in features:
        body += length_delimited(2, feature)
    body += varint((5 << 3) | 0) + varint(extent)
    return body


def encode_feature(kind, geometry, feature_id=None):
    # Feature: id (1), type (3), geometry (4). No tags: the plot layer styles by
    # layer name, not by attribute.
    body = b""
    if feature_id is not None:
        body += varint((1 << 3) | 0) + varint(feature_id)
    body += varint((3 << 3) | 0) + varint(kind)
    body += packed_u32(4, geometry)
    return body


def move_to(points):
    """MoveTo command for one or more points, delta-encoded from the cursor."""
    stream = [((len(points)) << 3) | 1]
    cursor = (0, 0)
    for point in points:
        stream.append(zigzag(point[0] - cursor[0]))
        stream.append(zigzag(point[1] - cursor[1]))
        cursor = point
    return stream


def line_to(points):
    """LineTo command for the deltas from the previous position to the rest."""
    if len(points) < 2:
        raise ValueError("LineTo needs at least two positions")
    stream = [((len(points) - 1) << 3) | 2]
    previous = points[0]
    for point in points[1:]:
        stream.append(zigzag(point[0] - previous[0]))
        stream.append(zigzag(point[1] - previous[1]))
        previous = point
    return stream


def signed_area(points):
    """Surveyor's formula; positive is the MVT exterior-ring winding."""
    total = 0
    for index, (x0, y0) in enumerate(points):
        x1, y1 = points[(index + 1) % len(points)]
        total += x0 * y1 - y0 * x1
    return total


def polygon_stream(rings):
    """Encode ``(vertices, is_exterior)`` rings as one polygon geometry.

    The MVT cursor is feature-wide and is not reset between rings, so the ring
    deltas are encoded against the running cursor (the last vertex of the
    previous ring, the close edge has no parameters of its own).
    """
    stream = []
    cursor = (0, 0)
    for points, exterior in rings:
        if (signed_area(points) > 0) != exterior:
            points = list(reversed(points))
        stream.append(9)  # MoveTo(1)
        stream.append(zigzag(points[0][0] - cursor[0]))
        stream.append(zigzag(points[0][1] - cursor[1]))
        cursor = points[0]
        stream.append(((len(points) - 1) << 3) | 2)  # LineTo(n - 1)
        for point in points[1:]:
            stream.append(zigzag(point[0] - cursor[0]))
            stream.append(zigzag(point[1] - cursor[1]))
            cursor = point
        stream.append(15)  # ClosePath(1)
    return stream


def tile_local(u, v):
    return int(round(u * EXTENT)), int(round(v * EXTENT))


def build_tile(z, x, y):
    # A little per-tile phase shift keeps neighbouring tiles visibly distinct
    # while staying deterministic.
    phase = ((x * 37 + y * 53 + z * 11) % 6) * 0.004

    points = []
    for row in range(5):
        for column in range(5):
            u = 0.16 + column * 0.17 + phase
            v = 0.16 + row * 0.17 + phase
            points.append(tile_local(u, v))

    road = [
        tile_local(0.12, 0.24 + phase),
        tile_local(0.30, 0.36 + phase),
        tile_local(0.46, 0.30 + phase),
        tile_local(0.62, 0.48 + phase),
        tile_local(0.80, 0.40 + phase),
        tile_local(0.90, 0.58 + phase),
    ]

    exterior = [
        tile_local(0.50, 0.14),
        tile_local(0.82, 0.34),
        tile_local(0.80, 0.66),
        tile_local(0.52, 0.86),
        tile_local(0.24, 0.70),
        tile_local(0.18, 0.38),
    ]
    hole = [
        tile_local(0.44, 0.40),
        tile_local(0.50, 0.46),
        tile_local(0.44, 0.52),
        tile_local(0.38, 0.46),
    ]
    polygon = polygon_stream([(exterior, True), (hole, False)])

    layers = [
        encode_layer("points", [encode_feature(POINT, move_to(points))]),
        encode_layer("roads", [encode_feature(LINESTRING, move_to(road[:1]) + line_to(road))]),
        encode_layer("landuse", [encode_feature(POLYGON, polygon)]),
    ]
    return b"".join(length_delimited(3, layer) for layer in layers)


# --- HTTP --------------------------------------------------------------------


class Handler(BaseHTTPRequestHandler):
    server_version = "ew_gis_mvt/0.1"

    def do_GET(self):  # noqa: N802 (BaseHTTPRequestHandler API)
        path = self.path.split("?", 1)[0].strip("/")
        parts = path.split("/")
        if len(parts) != 3 or not parts[2].endswith(".mvt"):
            self.send_error(404, "expected /{z}/{x}/{y}.mvt")
            return
        try:
            z = int(parts[0])
            x = int(parts[1])
            y = int(parts[2][: -len(".mvt")])
            if z < 0 or z > 22 or not (0 <= x < (1 << z)) or not (0 <= y < (1 << z)):
                raise ValueError
        except ValueError:
            self.send_error(400, "bad tile coordinates")
            return
        body = build_tile(z, x, y)
        self.send_response(200)
        self.send_header("Content-Type", "application/vnd.mapbox-vector-tile")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):  # noqa: A002 - stdlib signature
        # Keep the acceptance log readable: only errors matter.
        if args and isinstance(args[0], str) and args[0].startswith("code 4"):
            super().log_message(format, *args)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--host", default="127.0.0.1")
    args = parser.parse_args()

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    print(
        f"MVT fixture server on http://{args.host}:{args.port}/{{z}}/{{x}}/{{y}}.mvt",
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
