"""Headless Blender: OBJ + 8K maps -> earth_v5.glb (globe + cloud shell)."""

from __future__ import annotations

import math
import sys
from pathlib import Path

import bpy

ROOT = Path("/Users/danieldriscoll/dual")
OBJ = ROOT / "ai-context/high-res-earth/Earth-OBJ/Earth.obj"
TEX = ROOT / "pyrotechnique/assets/textures/earth"
OUT = ROOT / "pyrotechnique/assets/models/earth_v5.glb"

EARTH_SCALE = 1004.906
CLOUD_SCALE = EARTH_SCALE * (1.0 + 4000.0 / 6_378_140.0)


def fail(msg: str) -> None:
    print(msg, file=sys.stderr)
    raise SystemExit(1)


def image(path: Path, non_color: bool = False):
    if not path.is_file():
        fail(f"missing texture {path}")
    img = bpy.data.images.load(str(path), check_existing=True)
    if non_color:
        img.colorspace_settings.name = "Non-Color"
    return img


def principled(mat):
    nodes = mat.node_tree.nodes
    for node in nodes:
        if node.type == "BSDF_PRINCIPLED":
            return node
    fail(f"no Principled BSDF on {mat.name}")


def tex_node(mat, img, non_color=False):
    nodes = mat.node_tree.nodes
    node = nodes.new("ShaderNodeTexImage")
    node.image = img
    if non_color and img:
        img.colorspace_settings.name = "Non-Color"
    return node


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
    bsdf = principled(mat)
    links = mat.node_tree.links
    color = tex_node(mat, image(TEX / "color_aug.jpg"))
    links.new(color.outputs["Color"], bsdf.inputs["Base Color"])
    emit = tex_node(mat, image(TEX / "night.jpg"))
    links.new(emit.outputs["Color"], bsdf.inputs["Emission Color"])
    bsdf.inputs["Emission Strength"].default_value = 1.0
    rough = tex_node(mat, image(TEX / "roughness.jpg", non_color=True), non_color=True)
    links.new(rough.outputs["Color"], bsdf.inputs["Roughness"])
    bsdf.inputs["Metallic"].default_value = 0.0
    normal_tex = tex_node(mat, image(TEX / "normal.png", non_color=True), non_color=True)
    normal_map = mat.node_tree.nodes.new("ShaderNodeNormalMap")
    links.new(normal_tex.outputs["Color"], normal_map.inputs["Color"])
    links.new(normal_map.outputs["Normal"], bsdf.inputs["Normal"])
    return mat


def clouds_material():
    mat = bpy.data.materials.new("Clouds")
    mat.use_nodes = True
    mat.blend_method = "BLEND"
    if hasattr(mat, "shadow_method"):
        mat.shadow_method = "NONE"
    bsdf = principled(mat)
    links = mat.node_tree.links
    color = tex_node(mat, image(TEX / "clouds_color.jpg"))
    alpha = tex_node(mat, image(TEX / "clouds_alpha.jpg", non_color=True), non_color=True)
    links.new(color.outputs["Color"], bsdf.inputs["Base Color"])
    links.new(alpha.outputs["Color"], bsdf.inputs["Alpha"])
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


def main():
    clear_scene()
    earth = import_earth()
    clouds = make_clouds(earth)
    assign(earth, earth_material())
    assign(clouds, clouds_material())
    print(f"earth scale {tuple(earth.scale)} clouds scale {tuple(clouds.scale)}")
    print(f"earth verts {len(earth.data.vertices)} clouds verts {len(clouds.data.vertices)}")
    export_glb()


if __name__ == "__main__":
    main()
