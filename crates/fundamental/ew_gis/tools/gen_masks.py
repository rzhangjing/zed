"""Generate the deterministic equirectangular land/border masks used by ew_gis.

Provenance: the source GeoJSON is Natural Earth 1:110m admin-0 countries
(public domain), as emitted by the existing `rustgis-render` build script in
this workspace. The generated PNGs are committed so `ew_gis` has no data-file
dependency on that crate or on the network.

Usage:
    python gen_masks.py <countries.geojson> <out_dir>
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

WIDTH = 4096
HEIGHT = 2048
SUPERSAMPLE = 2


def rings(geometry: dict):
    if not geometry:
        return
    if geometry["type"] == "Polygon":
        for ring in geometry["coordinates"]:
            yield ring
    elif geometry["type"] == "MultiPolygon":
        for polygon in geometry["coordinates"]:
            for ring in polygon:
                yield ring


def to_px(ring, width: int, height: int):
    return [
        (
            (float(lon) + 180.0) / 360.0 * width,
            (90.0 - float(lat)) / 180.0 * height,
        )
        for lon, lat in ring
    ]


def main() -> None:
    source = Path(sys.argv[1])
    out_dir = Path(sys.argv[2])
    out_dir.mkdir(parents=True, exist_ok=True)
    data = json.loads(source.read_text(encoding="utf-8"))

    sw = WIDTH * SUPERSAMPLE
    sh = HEIGHT * SUPERSAMPLE
    land = Image.new("L", (sw, sh), 0)
    borders = Image.new("L", (sw, sh), 0)
    land_draw = ImageDraw.Draw(land)
    border_draw = ImageDraw.Draw(borders)

    for feature in data["features"]:
        for ring in rings(feature.get("geometry")):
            points = to_px(ring, sw, sh)
            if len(points) < 3:
                continue
            land_draw.polygon(points, fill=255)
            border_draw.line(points, fill=255, width=3, joint="curve")

    land = land.resize((WIDTH, HEIGHT), Image.Resampling.LANCZOS)
    borders = borders.resize((WIDTH, HEIGHT), Image.Resampling.LANCZOS)
    # A very light blur keeps the border line soft at globe magnification.
    borders = borders.filter(ImageFilter.GaussianBlur(0.35))

    land.save(out_dir / "land_mask.png", optimize=True)
    borders.save(out_dir / "borders_mask.png", optimize=True)
    print(f"wrote {out_dir / 'land_mask.png'} and {out_dir / 'borders_mask.png'}")


if __name__ == "__main__":
    main()
