# /// script
# requires-python = ">=3.10"
# dependencies = ["trimesh", "numpy", "pygltflib"]
# ///
"""Measure the Apollo LM GLB: overall bounds and the DPS nozzle bell geometry.

Prints, in raw GLB coordinates, the bell's axis (x, z), exit-plane y, throat y,
and the inner-radius profile so the thruster emitter and hanabi spawn cone can
be positioned from real numbers instead of eyeballed offsets.
Elodin object frame = GLB frame + translate (0, -2.5, 0).
"""

import sys

import numpy as np
import trimesh

GLB = "/Users/danieldriscoll/dual/elodin/assets/apollo-lunar-module.glb"

scene = trimesh.load(GLB, force="scene", process=False)
print(f"scene bounds (glb coords):\n{scene.bounds}\n")

print("=== nodes / geometry ===")
rows = []
for node_name in scene.graph.nodes_geometry:
    transform, geom_name = scene.graph[node_name]
    mesh = scene.geometry[geom_name]
    v = trimesh.transform_points(mesh.vertices, transform)
    lo, hi = v.min(axis=0), v.max(axis=0)
    center = (lo + hi) / 2
    rows.append((node_name, geom_name, lo, hi, center, v))

for name, geom, lo, hi, center, _ in sorted(rows, key=lambda r: r[2][1]):
    print(
        f"{name[:40]:40s} geom={geom[:28]:28s} "
        f"y=[{lo[1]:7.3f},{hi[1]:7.3f}] x=[{lo[0]:7.3f},{hi[0]:7.3f}] "
        f"z=[{lo[2]:7.3f},{hi[2]:7.3f}] c=({center[0]:6.3f},{center[1]:6.3f},{center[2]:6.3f})"
    )

# The DPS bell: a mesh centered near x=0,z=0 whose xz footprint is roughly
# circular and small (< 1.5 m radius) sitting in the lower half of the model.
y_mid = scene.bounds[:, 1].mean()
candidates = []
for name, geom, lo, hi, center, v in rows:
    rx = (hi[0] - lo[0]) / 2
    rz = (hi[2] - lo[2]) / 2
    if abs(center[0]) < 0.4 and abs(center[2]) < 0.4 and 0.1 < rx < 1.5 and 0.1 < rz < 1.5:
        if abs(rx - rz) / max(rx, rz) < 0.25 and lo[1] < y_mid:
            candidates.append((name, geom, lo, hi, center, v, rx, rz))

print("\n=== bell candidates (round, centered, lower half) ===")
for name, geom, lo, hi, center, v, rx, rz in candidates:
    print(f"{name[:40]:40s} y=[{lo[1]:7.3f},{hi[1]:7.3f}] rx={rx:.3f} rz={rz:.3f}")

if not candidates:
    print("no obvious bell candidate; inspect the node list above")
    sys.exit(0)

# Pick the candidate with the lowest bottom edge (the bell hangs lowest of the
# centered round parts).
name, geom, lo, hi, center, v, rx, rz = min(candidates, key=lambda c: c[2][1])
print(f"\n=== bell: {name} ===")
axis_x, axis_z = v[:, 0].mean(), v[:, 2].mean()
print(f"axis (vertex mean): x={axis_x:.4f} z={axis_z:.4f}")
print(f"exit plane (min y): {lo[1]:.4f}   top (max y): {hi[1]:.4f}")

print("\nradius profile (glb y, min/max vertex radius from axis):")
for frac in np.linspace(0.0, 1.0, 9):
    y = lo[1] + frac * (hi[1] - lo[1])
    band = v[np.abs(v[:, 1] - y) < 0.03]
    if len(band) == 0:
        continue
    r = np.hypot(band[:, 0] - axis_x, band[:, 2] - axis_z)
    print(f"  y={y:7.3f} (obj {y - 2.5:7.3f})  r_min={r.min():.3f} r_max={r.max():.3f} n={len(band)}")

print(f"\nobject frame (translate 0,-2.5,0): exit plane y={lo[1] - 2.5:.4f}, "
      f"bell top y={hi[1] - 2.5:.4f}, axis=({axis_x:.4f}, ., {axis_z:.4f})")
