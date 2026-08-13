# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy", "pillow"]
# ///
"""Downsample NoneCG 16K Earth maps to 8K runtime derivatives."""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
from PIL import Image

Image.MAX_IMAGE_PIXELS = None

SRC = Path("/Users/danieldriscoll/dual/ai-context/high-res-earth/Earth-Textures/16K")
DST = Path("/Users/danieldriscoll/dual/pyrotechnique/assets/textures/earth")
SIZE = (8192, 4096)


def resize(im: Image.Image) -> Image.Image:
    return im.resize(SIZE, Image.Resampling.LANCZOS)


def save_jpeg(im: Image.Image, path: Path, quality: int) -> None:
    im.convert("RGB").save(path, "JPEG", quality=quality, optimize=True, subsampling=0)
    print(f"wrote {path} ({path.stat().st_size / 1e6:.1f} MB)", flush=True)


def save_png(im: Image.Image, path: Path) -> None:
    im.save(path, "PNG", optimize=True)
    print(f"wrote {path} ({path.stat().st_size / 1e6:.1f} MB)", flush=True)


def main() -> int:
    if not SRC.is_dir():
        print(f"missing source: {SRC}", file=sys.stderr)
        return 1
    DST.mkdir(parents=True, exist_ok=True)

    jobs = [
        ("Earth_Color_08_Aug.jpg", "color_aug.jpg", 92, "jpeg"),
        ("Earth_Nightlights.jpg", "night.jpg", 95, "jpeg"),
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


if __name__ == "__main__":
    raise SystemExit(main())
