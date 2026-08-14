# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy", "pillow"]
# ///
"""Downsample NoneCG Earth maps to 8K runtime derivatives.

Albedo / clouds / normal / roughness come from the 16K set. Night lights
are Lanczos-downsampled from the 32K Earth_Nightlights so the 8K sheet
and the city-tile CDF share that source.
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
from PIL import Image

Image.MAX_IMAGE_PIXELS = None

ROOT = Path("/Users/danieldriscoll/dual")
SRC = ROOT / "ai-context/high-res-earth/Earth-Textures/16K"
NIGHT_SRC = ROOT / "ai-context/high-res-earth/Earth-Textures/32K/Earth_Nightlights.jpg"
DST = ROOT / "pyrotechnique/assets/textures/earth"
SIZE = (8192, 4096)
TILES = (128, 64)


def resize(im: Image.Image) -> Image.Image:
    return im.resize(SIZE, Image.Resampling.LANCZOS)


def save_jpeg(im: Image.Image, path: Path, quality: int) -> None:
    im.convert("RGB").save(path, "JPEG", quality=quality, optimize=True, subsampling=0)
    print(f"wrote {path} ({path.stat().st_size / 1e6:.1f} MB)", flush=True)


def save_png(im: Image.Image, path: Path) -> None:
    im.save(path, "PNG", optimize=True)
    print(f"wrote {path} ({path.stat().st_size / 1e6:.1f} MB)", flush=True)


# Bright night set has a dim global veil (~0.02–0.05). Cities start ~0.06.
NIGHT_LUMA_KILL = 0.06


def night_luma(arr: np.ndarray) -> np.ndarray:
    luma = (
        arr[..., 0].astype(np.float32) * 0.3
        + arr[..., 1].astype(np.float32) * 0.6
        + arr[..., 2].astype(np.float32) * 0.1
    ) / 255.0
    luma[luma < NIGHT_LUMA_KILL] = 0.0
    return luma


def crush_night_veil(im: Image.Image) -> Image.Image:
    arr = np.asarray(im.convert("RGB"), dtype=np.uint8).copy()
    arr[night_luma(arr) <= 0.0] = 0
    return Image.fromarray(arr, mode="RGB")


def bake_city_tile_cdf(im: Image.Image, path: Path) -> None:
    arr = np.asarray(im.convert("RGB"), dtype=np.uint8)
    height, width = arr.shape[:2]
    tiles_u, tiles_v = TILES
    if width % tiles_u or height % tiles_v:
        raise SystemExit(f"night size {width}x{height} not divisible by {tiles_u}x{tiles_v}")
    luma = night_luma(arr)
    v = (np.arange(height, dtype=np.float32) + 0.5) / height
    cos_lat = np.cos((0.5 - v) * np.pi)[:, None]
    weight = luma * cos_lat
    tile_h = height // tiles_v
    tile_w = width // tiles_u
    tiles = weight.reshape(tiles_v, tile_h, tiles_u, tile_w).mean(axis=(1, 3))
    cdf = np.cumsum(tiles.ravel(), dtype=np.float64)
    total = float(cdf[-1])
    if total <= 0.0:
        raise SystemExit("night map has no lights")
    path.write_bytes((cdf / total).astype(np.float32).tobytes())
    print(f"wrote {path} ({tiles_u}x{tiles_v} cdf, {path.stat().st_size} bytes)", flush=True)


def bake_night() -> int:
    if not NIGHT_SRC.is_file():
        print(f"missing {NIGHT_SRC}", file=sys.stderr)
        return 1
    DST.mkdir(parents=True, exist_ok=True)
    print(f"resize {NIGHT_SRC} -> night.jpg", flush=True)
    with Image.open(NIGHT_SRC) as im:
        out = crush_night_veil(resize(im.convert("RGB")))
    bake_city_tile_cdf(out, DST / "city_tile_cdf.bin")
    save_jpeg(out, DST / "night.jpg", 95)
    return 0


def bake_day_maps() -> int:
    if not SRC.is_dir():
        print(f"missing source: {SRC}", file=sys.stderr)
        return 1
    DST.mkdir(parents=True, exist_ok=True)

    jobs = [
        ("Earth_Color_08_Aug.jpg", "color_aug.jpg", 92, "jpeg"),
        ("Earth_Less_Clouds/Earth_Clouds.jpg", "clouds_color.jpg", 90, "jpeg"),
        ("Earth_Less_Clouds/Earth_Clouds_Transp.jpg", "clouds_alpha.jpg", 90, "jpeg"),
        ("Earth_Normal.jpg", "normal.png", 0, "png"),
    ]
    for src_name, dst_name, quality, kind in jobs:
        src = SRC / src_name
        dst = DST / dst_name
        if not src.is_file():
            print(f"missing {src}", file=sys.stderr)
            return 1
        print(f"resize {src.name} -> {dst.name}", flush=True)
        with Image.open(src) as im:
            out = resize(im)
            if kind == "png":
                save_png(out.convert("RGB"), dst)
            else:
                save_jpeg(out, dst, quality)

    gloss_path = SRC / "Earth_Glossiness.jpg"
    cover_path = SRC / "Earth_Landcover.jpg"
    print("bake roughness from gloss + landcover", flush=True)
    with Image.open(gloss_path) as gloss_im, Image.open(cover_path) as cover_im:
        gloss = np.asarray(resize(gloss_im.convert("L")), dtype=np.float32)
        cover = np.asarray(resize(cover_im.convert("L")), dtype=np.float32) / 255.0
        rough = np.clip((255.0 - gloss) * (1.0 - 0.75 * cover), 0, 255).astype(np.uint8)
        save_jpeg(Image.fromarray(rough, mode="L").convert("RGB"), DST / "roughness.jpg", 90)
    return 0


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] == "--night":
        return bake_night()
    day = bake_day_maps()
    if day != 0:
        return day
    return bake_night()


if __name__ == "__main__":
    raise SystemExit(main())
