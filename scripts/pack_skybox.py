# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy", "pillow"]
# ///
"""Pack Stars_Milky_Way.jpg into an Elodin-compatible cubemap.ktx2."""

from __future__ import annotations

import math
import os
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image

Image.MAX_IMAGE_PIXELS = None

SRC = Path(
    "/Users/danieldriscoll/dual/ai-context/high-res-earth/Earth-Textures/16K/Stars_Milky_Way.jpg"
)
OUT = Path(
    "/Users/danieldriscoll/dual/pyrotechnique/assets/skyboxes/milky_way.cubemap.ktx2"
)
# 4096 = 1:1 sampling from the 16K equirect (16384 / 4 faces per ring).
FACE = 4096
# The master carries a dim veil (~1-8/255) across empty sky. The runtime gain
# needed to make the band read (SKYBOX_NIGHT_BRIGHTNESS) also multiplies that
# veil into grey haze, so zero it here and let the gain lift only real signal.
VEIL_BLACK_POINT = 8.0


def face_directions(face: int, face_size: int) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    xs = (np.arange(face_size, dtype=np.float32) + 0.5) / face_size
    ys = (np.arange(face_size, dtype=np.float32) + 0.5) / face_size
    s, t = np.meshgrid(2.0 * xs - 1.0, 2.0 * ys - 1.0)
    if face == 0:
        dx, dy, dz = np.ones_like(s), -t, -s
    elif face == 1:
        dx, dy, dz = -np.ones_like(s), -t, s
    elif face == 2:
        dx, dy, dz = s, np.ones_like(s), t
    elif face == 3:
        dx, dy, dz = s, -np.ones_like(s), -t
    elif face == 4:
        dx, dy, dz = s, -t, np.ones_like(s)
    else:
        dx, dy, dz = -s, -t, -np.ones_like(s)
    norm = np.sqrt(dx * dx + dy * dy + dz * dz)
    return dx / norm, dy / norm, dz / norm


def sample_equirect(src: np.ndarray, dx: np.ndarray, dy: np.ndarray, dz: np.ndarray) -> np.ndarray:
    height, width = src.shape[:2]
    u = (0.5 + np.arctan2(dz, dx) / (2.0 * math.pi)) % 1.0
    v = np.clip(np.arccos(np.clip(dy, -1.0, 1.0)) / math.pi, 0.0, 1.0)
    x = u * width - 0.5
    y = np.clip(v * (height - 1), 0.0, height - 1)
    x0 = np.floor(x).astype(np.int32)
    y0 = np.floor(y).astype(np.int32)
    tx = (x - x0)[..., None]
    ty = (y - y0)[..., None]
    x0 %= width
    x1 = (x0 + 1) % width
    y1 = np.minimum(y0 + 1, height - 1)
    p00 = src[y0, x0].astype(np.float32)
    p10 = src[y0, x1].astype(np.float32)
    p01 = src[y1, x0].astype(np.float32)
    p11 = src[y1, x1].astype(np.float32)
    top = p00 * (1.0 - tx) + p10 * tx
    bottom = p01 * (1.0 - tx) + p11 * tx
    return np.clip(top * (1.0 - ty) + bottom * ty, 0, 255).astype(np.uint8)


def crush_veil(path: Path, out_dir: Path) -> Path:
    """Scale each pixel by its luma headroom above the veil, preserving hue."""
    out = out_dir / path.name
    arr = np.asarray(Image.open(path).convert("RGB"), dtype=np.float32)
    luma = arr[..., 0] * 0.3 + arr[..., 1] * 0.6 + arr[..., 2] * 0.1
    gain = np.clip((luma - VEIL_BLACK_POINT) / np.maximum(luma, 1e-3), 0.0, 1.0)
    crushed = np.clip(arr * gain[..., None], 0, 255).astype(np.uint8)
    Image.fromarray(crushed, mode="RGB").save(out, "PNG")
    return out


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


def main() -> int:
    if not SRC.is_file():
        print(f"missing {SRC}", file=sys.stderr)
        return 1
    toktx = resolve_toktx()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    cache = Path(f"/tmp/pyro-skybox-faces-{FACE}")
    cache.mkdir(parents=True, exist_ok=True)
    need = [face for face in range(6) if not (cache / f"face{face}.png").is_file()]
    src = None
    if need:
        print(f"load {SRC}", flush=True)
        src = np.asarray(Image.open(SRC).convert("RGB"))
    face_paths = []
    for face in range(6):
        path = cache / f"face{face}.png"
        if path.is_file():
            print(f"reuse {path}", flush=True)
        else:
            print(f"face {face}/5", flush=True)
            dx, dy, dz = face_directions(face, FACE)
            pixels = sample_equirect(src, dx, dy, dz)
            Image.fromarray(pixels, mode="RGB").save(path, "PNG")
        face_paths.append(path)

    crushed = Path(f"/tmp/pyro-skybox-crushed-{FACE}-{VEIL_BLACK_POINT:g}")
    crushed.mkdir(parents=True, exist_ok=True)
    print(f"crush veil below {VEIL_BLACK_POINT:g}/255 -> {crushed}", flush=True)
    face_paths = [crush_veil(path, crushed) for path in face_paths]

    cmd = [
        toktx,
        "--t2",
        "--encode",
        "uastc",
        "--uastc_quality",
        "3",
        "--uastc_rdo_l",
        "0.75",
        "--zcmp",
        "19",
        "--genmipmap",
        "--cubemap",
        "--assign_oetf",
        "srgb",
        str(OUT),
        *[str(p) for p in face_paths],
    ]
    print(" ".join(cmd), flush=True)
    subprocess.run(cmd, check=True)
    print(f"wrote {OUT} ({OUT.stat().st_size / 1e6:.1f} MB)", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
