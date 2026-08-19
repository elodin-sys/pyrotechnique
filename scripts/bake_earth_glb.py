"""Headless Blender: OBJ -> mesh-only earth_v5.glb (globe + cloud shell).

Materials keep the Earth / Clouds names so runtime adoption still works.
Textures are bound in code from KTX2; this file is geometry + factors only.
"""

from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

import bpy

ROOT = Path("/Users/danieldriscoll/dual")
OBJ = ROOT / "ai-context/high-res-earth/Earth-OBJ/Earth.obj"
OUT = ROOT / "pyrotechnique/assets/models/earth_v5.glb"

EARTH_SCALE = 1004.906
CLOUD_SCALE = EARTH_SCALE * (1.0 + 4000.0 / 6_378_140.0)


def fail(msg: str) -> None:
    print(msg, file=sys.stderr)
    raise SystemExit(1)


def principled(mat):
    nodes = mat.node_tree.nodes
    for node in nodes:
        if node.type == "BSDF_PRINCIPLED":
            return node
    fail(f"no Principled BSDF on {mat.name}")


def clear_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for block in (bpy.data.meshes, bpy.data.materials, bpy.data.images):
        for item in list(block):
            block.remove(item)


def import_earth():
    if not OBJ.is_file():
        fail(f"missing {OBJ}")
    bpy.ops.wm.obj_import(filepath=str(OBJ))
    objs = [o for o in bpy.context.selected_objects if o.type == "MESH"]
    if not objs:
        fail("OBJ import produced no mesh")
    earth = objs[0]
    earth.name = "Earth"
    earth.data.name = "Earth"
    earth.scale = (EARTH_SCALE, EARTH_SCALE, EARTH_SCALE)
    earth.location = (0.0, 0.0, 0.0)
    earth.rotation_euler = (0.0, 0.0, 0.0)
    return earth


def make_clouds(earth):
    clouds = earth.copy()
    clouds.data = earth.data.copy()
    clouds.name = "Clouds"
    clouds.data.name = "Clouds"
    bpy.context.collection.objects.link(clouds)
    clouds.scale = (CLOUD_SCALE, CLOUD_SCALE, CLOUD_SCALE)
    return clouds


def earth_material():
    mat = bpy.data.materials.new("Earth")
    mat.use_nodes = True
    mat.use_backface_culling = True
    bsdf = principled(mat)
    bsdf.inputs["Base Color"].default_value = (0.0, 0.0, 0.0, 1.0)
    bsdf.inputs["Metallic"].default_value = 0.0
    bsdf.inputs["Emission Strength"].default_value = 0.0
    return mat


def clouds_material():
    mat = bpy.data.materials.new("Clouds")
    mat.use_nodes = True
    mat.use_backface_culling = True
    mat.blend_method = "BLEND"
    if hasattr(mat, "shadow_method"):
        mat.shadow_method = "NONE"
    bsdf = principled(mat)
    bsdf.inputs["Base Color"].default_value = (1.0, 1.0, 1.0, 0.0)
    bsdf.inputs["Alpha"].default_value = 0.0
    bsdf.inputs["Metallic"].default_value = 0.0
    bsdf.inputs["Roughness"].default_value = 0.95
    bsdf.inputs["Emission Strength"].default_value = 0.0
    return mat


def assign(obj, mat):
    obj.data.materials.clear()
    obj.data.materials.append(mat)


def export_glb():
    OUT.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.object.select_all(action="DESELECT")
    for name in ("Earth", "Clouds"):
        bpy.data.objects[name].select_set(True)
    bpy.context.view_layer.objects.active = bpy.data.objects["Earth"]
    bpy.ops.export_scene.gltf(
        filepath=str(OUT),
        export_format="GLB",
        use_selection=True,
        export_texcoords=True,
        export_normals=True,
        export_materials="EXPORT",
        export_cameras=False,
        export_lights=False,
        export_animations=False,
        export_apply=False,
    )
    print(f"wrote {OUT} ({OUT.stat().st_size / 1e6:.1f} MB)")


def assert_no_images() -> None:
    data = OUT.read_bytes()
    json_len, _ = struct.unpack_from("<I4s", data, 12)
    gltf = json.loads(data[20 : 20 + json_len])
    images = gltf.get("images", [])
    names = [img.get("name", "?") for img in images]
    if images:
        fail(f"expected mesh-only GLB, found {len(images)} images: {names}")
    print(f"  mesh-only OK ({OUT.stat().st_size / 1e6:.1f} MB, 0 images)")


def main():
    clear_scene()
    earth = import_earth()
    clouds = make_clouds(earth)
    assign(earth, earth_material())
    assign(clouds, clouds_material())
    print(f"earth scale {tuple(earth.scale)} clouds scale {tuple(clouds.scale)}")
    print(f"earth verts {len(earth.data.vertices)} clouds verts {len(clouds.data.vertices)}")
    export_glb()
    assert_no_images()


if __name__ == "__main__":
    main()
