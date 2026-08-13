# High-res Earth (NoneCG v5) — what shipped

NoneCG 12 Month Cycle Earth Model v5, staged in `ai-context/high-res-earth`.
Runtime assets are 8K derivatives plus a baked GLB. 32K and the zips stay
out of `pyrotechnique/assets/`.

## Runtime files

| Path | Role |
|---|---|
| `assets/models/earth_v5.glb` | 200k-tri globe + cloud shell, 8K maps embedded. Node scale 1004.906 → world R = 6_378_140 m. Clouds at ×1.000627 (R+4 km). |
| `assets/textures/earth/color_aug.jpg` | August albedo (also inside the GLB). |
| `assets/textures/earth/night.jpg` | Bright night lights. Hanabi `"night"` slot **and** GLB emissive. |
| `assets/textures/earth/clouds_*.jpg` | Less_Clouds color + alpha (also inside the GLB). |
| `assets/textures/earth/normal.png` | Terrain normal (also inside the GLB). |
| `assets/textures/earth/roughness.jpg` | Invert(gloss) × water mask from landcover. |
| `assets/skyboxes/milky_way.cubemap.ktx2` | Elodin-format UASTC cubemap, 2048 px faces, from `Stars_Milky_Way.jpg`. |

`models/earth.glb` and `textures/earth_night.png` are the old low-res pair.
Keep them. `gen-effects` does not touch any of these.

## How it is used

- Scenes `satellite`, `debug_earth_only`, `debug_cities_only` load `earth_v5.glb`.
- Globe emissive is tagged `EarthGlobeMaterial` and scaled by `star_visibility`
  (`EARTH_EMISSIVE_NIGHT = 1.6`). Noon is black. Hanabi cities still do bloom.
- Clouds are a second mesh, alpha blend, parented with the globe so
  `orient()` and the 20°/orbit spin stay locked.
- `environment.skybox` loads the cubemap onto `MainCamera`. Brightness is
  `1000 * star_visibility` (noon = 0). Rotation follows `SkyRoot`. No
  `EnvironmentMapLight`. Bevy needs `ktx2` + `basis-universal` to transcode
  the UASTC file.
- Hanabi stars / MW stay. Accept some doubling at night; dim cubemap
  brightness before touching particle counts.
- Capture waits for Earth material images and a configured cube view, not
  just a spawned mesh.

## Rebuild

Needs Blender 4 and `toktx` (KTX-Software 4.4.x; v5 dropped `toktx`).

```bash
uv run scripts/prepare_earth_textures.py
/Applications/Blender.app/Contents/MacOS/Blender --background --python scripts/bake_earth_glb.py
TOKTX=scripts/.tools/toktx uv run scripts/pack_skybox.py
```

`scripts/.tools/` is gitignored. Drop a signed `toktx` + `libktx.4.dylib`
there (rpath is `@executable_path`). Do not run `install_name_tool` on it —
that breaks the hardened-runtime signature and macOS SIGKILLs the binary.

gltf-transform lossless cleanup was tried; it grew the GLB (50.7 → 53.1 MB)
and added no extensions either way. The Blender export is the file we ship.
Do not decimate the sphere. Do not add Draco / meshopt / basisu / webp.

## Locked look-dev

- Albedo: August (`Earth_Color_08_Aug.jpg`)
- Night: bright set
- Clouds: Less_Clouds
- Nadir: Cairo/Nile `(30, 31, 0)`
- No height displacement, no cloud bump, no Blender atmosphere object
