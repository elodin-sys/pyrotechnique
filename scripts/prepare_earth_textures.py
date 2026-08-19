# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy", "pillow"]
# ///
"""Derive runtime Earth maps from the NoneCG masters.

16K (the Metal max-texture size) for what the camera magnifies — color,
clouds, night — and 8K for relief/gloss. Color is a byte-copy of the 16K
master (no resample, no re-encode). Night comes from the 32K master and is
saved as PNG so the veil crush does not carve JPEG block edges around
cities. Clouds are pre-merged into one RGBA so Blender embeds them without
its own channel-pack re-encode.
"""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

Image.MAX_IMAGE_PIXELS = None

ROOT = Path("/Users/danieldriscoll/dual")
SRC = ROOT / "ai-context/high-res-earth/Earth-Textures/16K"
NIGHT_SRC = ROOT / "ai-context/high-res-earth/Earth-Textures/32K/Earth_Nightlights.jpg"
DST = ROOT / "pyrotechnique/assets/textures/earth"
SIZE_16K = (16384, 8192)
SIZE_8K = (8192, 4096)
TILES = (128, 64)


def resize(im: Image.Image, size: tuple[int, int]) -> Image.Image:
    if im.size == size:
        return im
    return im.resize(size, Image.Resampling.LANCZOS)


def save_jpeg(im: Image.Image, path: Path, quality: int) -> None:
    im.convert("RGB").save(path, "JPEG", quality=quality, optimize=True, subsampling=0)
    print(f"wrote {path} ({path.stat().st_size / 1e6:.1f} MB)", flush=True)


def save_png(im: Image.Image, path: Path) -> None:
    im.save(path, "PNG")
    print(f"wrote {path} ({path.stat().st_size / 1e6:.1f} MB)", flush=True)


# Bright night set has a dim global veil (~0.02–0.05). Cities start ~0.06.
NIGHT_LUMA_KILL = 0.06
# Isolated bright texels magnify into hard squares from LEO nadir; a ~3 km
# blur rounds them into the diffuse glow the ISS long exposures show.
NIGHT_BLUR_SIGMA_PX = 1.2


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
    print(f"resize {NIGHT_SRC} -> night.png", flush=True)
    with Image.open(NIGHT_SRC) as im:
        out = crush_night_veil(resize(im.convert("RGB"), SIZE_16K))
    out = out.filter(ImageFilter.GaussianBlur(NIGHT_BLUR_SIGMA_PX))
    bake_city_tile_cdf(out, DST / "city_tile_cdf.bin")
    save_png(out, DST / "night.png")
    return 0


def copy_color() -> int:
    src = SRC / "Earth_Color_08_Aug.jpg"
    if not src.is_file():
        print(f"missing {src}", file=sys.stderr)
        return 1
    with Image.open(src) as im:
        if im.size != SIZE_16K:
            print(f"{src} is {im.size}, expected {SIZE_16K}", file=sys.stderr)
            return 1
    shutil.copyfile(src, DST / "color_aug.jpg")
    print(f"copied {src.name} -> color_aug.jpg ({src.stat().st_size / 1e6:.1f} MB)", flush=True)
    return 0


def bake_clouds() -> int:
    color_path = SRC / "Earth_Less_Clouds/Earth_Clouds.jpg"
    alpha_path = SRC / "Earth_Less_Clouds/Earth_Clouds_Transp.jpg"
    for p in (color_path, alpha_path):
        if not p.is_file():
            print(f"missing {p}", file=sys.stderr)
            return 1
    print("merge clouds color + transparency -> clouds_rgba.png", flush=True)
    with Image.open(color_path) as color_im, Image.open(alpha_path) as alpha_im:
        rgb = np.asarray(resize(color_im.convert("RGB"), SIZE_16K), dtype=np.uint8)
        alpha = np.asarray(resize(alpha_im.convert("L"), SIZE_16K), dtype=np.uint8)
    rgba = np.dstack([rgb, alpha])
    save_png(Image.fromarray(rgba, mode="RGBA"), DST / "clouds_rgba.png")
    return 0


def bake_relief() -> int:
    normal_path = SRC / "Earth_Normal.jpg"
    gloss_path = SRC / "Earth_Glossiness.jpg"
    cover_path = SRC / "Earth_Landcover.jpg"
    for p in (normal_path, gloss_path, cover_path):
        if not p.is_file():
            print(f"missing {p}", file=sys.stderr)
            return 1
    print("resize Earth_Normal.jpg -> normal.png (8K)", flush=True)
    with Image.open(normal_path) as im:
        save_png(resize(im.convert("RGB"), SIZE_8K), DST / "normal.png")
    print("bake roughness from gloss + landcover (8K)", flush=True)
    with Image.open(gloss_path) as gloss_im, Image.open(cover_path) as cover_im:
        gloss = np.asarray(resize(gloss_im.convert("L"), SIZE_8K), dtype=np.float32)
        cover = np.asarray(resize(cover_im.convert("L"), SIZE_8K), dtype=np.float32) / 255.0
        rough = np.clip((255.0 - gloss) * (1.0 - 0.75 * cover), 0, 255).astype(np.uint8)
        save_jpeg(Image.fromarray(rough, mode="L").convert("RGB"), DST / "roughness.jpg", 90)
    return 0


def main() -> int:
    DST.mkdir(parents=True, exist_ok=True)
    if len(sys.argv) > 1 and sys.argv[1] == "--night":
        return bake_night()
    for step in (copy_color, bake_clouds, bake_relief, bake_night):
        code = step()
        if code != 0:
            return code
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
