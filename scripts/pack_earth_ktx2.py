# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy", "pillow"]
# ///
"""Pack 16K/8K Earth maps to mipmapped UASTC KTX2 for Bevy."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
from PIL import Image

Image.MAX_IMAGE_PIXELS = None

ROOT = Path("/Users/danieldriscoll/dual")
TEX = ROOT / "pyrotechnique/assets/textures/earth"
BUDGET_BYTES = 150 * 1_000_000
UASTC_QUALITY = "2"
ZCMP = "19"
RDO_START = 0.75
RDO_STEPS = (0.75, 1.5, 2.5)


def resolve_toktx() -> str:
    env = os.environ.get("TOKTX")
    if env:
        return env
    found = shutil.which("toktx")
    if found:
        return found
    local = Path(__file__).resolve().parent / ".tools" / "toktx"
    if local.is_file():
        os.environ.setdefault("DYLD_LIBRARY_PATH", str(local.parent))
        return str(local)
    print("toktx not found; set TOKTX or install KTX-Software", file=sys.stderr)
    raise SystemExit(1)


def toktx_cmd(toktx: str, out: Path, src: Path, *, srgb: bool, rdo: float) -> list[str]:
    return [
        toktx,
        "--t2",
        "--encode",
        "uastc",
        "--uastc_quality",
        UASTC_QUALITY,
        "--uastc_rdo_l",
        f"{rdo:g}",
        "--zcmp",
        ZCMP,
        "--genmipmap",
        "--assign_oetf",
        "srgb" if srgb else "linear",
        str(out),
        str(src),
    ]


def run_toktx(cmd: list[str]) -> None:
    print(" ".join(cmd), flush=True)
    subprocess.run(cmd, check=True)


def jpeg_to_png(src: Path, tmp: Path) -> Path:
    out = tmp / f"{src.stem}.png"
    print(f"jpeg -> png {src.name}", flush=True)
    with Image.open(src) as im:
        im.convert("RGB").save(out, "PNG")
    return out


def pack_metallic_roughness(tmp: Path) -> Path:
    src = TEX / "roughness.jpg"
    if not src.is_file():
        raise SystemExit(f"missing {src}")
    print("pack metallic_roughness G=roughness B=0", flush=True)
    with Image.open(src) as im:
        rough = np.asarray(im.convert("L"), dtype=np.uint8)
    zeros = np.zeros_like(rough)
    rgb = np.dstack([zeros, rough, zeros])
    out = tmp / "metallic_roughness.png"
    Image.fromarray(rgb, mode="RGB").save(out, "PNG")
    return out


def encode_maps(toktx: str, tmp: Path, rdo: float) -> list[Path]:
    jobs = [
        ("color.ktx2", TEX / "color_aug.jpg", True, "jpeg"),
        ("night.ktx2", TEX / "night.png", True, "png"),
        ("clouds.ktx2", TEX / "clouds_rgba.png", True, "png"),
        ("normal.ktx2", TEX / "normal.png", False, "png"),
        ("metallic_roughness.ktx2", None, False, "mr"),
    ]
    written: list[Path] = []
    for name, src, srgb, kind in jobs:
        out = TEX / name
        if kind == "jpeg":
            if not src.is_file():
                raise SystemExit(f"missing {src}")
            inp = jpeg_to_png(src, tmp)
        elif kind == "mr":
            inp = pack_metallic_roughness(tmp)
        else:
            if not src.is_file():
                raise SystemExit(f"missing {src}")
            inp = src
        run_toktx(toktx_cmd(toktx, out, inp, srgb=srgb, rdo=rdo))
        print(f"wrote {out} ({out.stat().st_size / 1e6:.1f} MB)", flush=True)
        written.append(out)
    return written


def main() -> int:
    toktx = resolve_toktx()
    TEX.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="earth-ktx2-") as tmp_s:
        tmp = Path(tmp_s)
        written: list[Path] = []
        for rdo in RDO_STEPS:
            print(f"encode UASTC rdo_l={rdo:g}", flush=True)
            written = encode_maps(toktx, tmp, rdo)
            total = sum(p.stat().st_size for p in written)
            print("sizes:", flush=True)
            for path in written:
                print(f"  {path.name:28s} {path.stat().st_size / 1e6:7.1f} MB", flush=True)
            print(f"  total {total / 1e6:.1f} MB (budget {BUDGET_BYTES / 1e6:.0f} MB)", flush=True)
            if total <= BUDGET_BYTES:
                return 0
            print(f"over budget at rdo_l={rdo:g}; raising RDO", flush=True)
        print(
            f"still over budget after rdo_l={RDO_STEPS[-1]:g}",
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
