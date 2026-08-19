# High-res Earth (NoneCG v5) — what shipped

NoneCG 12 Month Cycle Earth Model v5, staged in `ai-context/high-res-earth`.
Runtime globe maps are mipmapped UASTC KTX2 (16K color / clouds / night, 8K
relief). The GLB is mesh-only; both apps bind the KTX2 files in code. 16K
(16384 px) is the Metal max-texture size. 32K masters stay out of
`pyrotechnique/assets/`.

## Runtime files

| Path | Role |
|---|---|
| `assets/models/earth_v5.glb` (~8.8 MB) | 200k-tri globe + cloud shell, **no embedded images**. Node scale 1004.906 → world R = 6_378_140 m. Clouds at ×1.000627 (R+4 km). Materials named Earth / Clouds with black / alpha-0 factors and backface cull. |
| `assets/textures/earth/color.ktx2` (69.2 MB) | August albedo, 16K UASTC + mips. Bound as `base_color_texture`. |
| `assets/textures/earth/night.ktx2` (3.6 MB) | 32K→16K night lights, veil crushed below luma 0.06, σ1.2 px Gaussian blur, then UASTC. Hanabi `"night"` slot **and** globe emissive. |
| `assets/textures/earth/clouds.ktx2` (49.1 MB) | Less_Clouds 16K RGBA UASTC. Bound on the Clouds material. |
| `assets/textures/earth/normal.ktx2` (5.9 MB) | Terrain normal, 8K linear UASTC (no `--normal_mode`). |
| `assets/textures/earth/metallic_roughness.ktx2` (1.4 MB) | glTF layout G=roughness, B=metallic(0), 8K linear. |
| `assets/textures/earth/color_aug.jpg` | August albedo master (pack / rebuild input). Not loaded at runtime. |
| `assets/textures/earth/night.png` | 16K night master with the crush + blur. Pack / CDF input. |
| `assets/textures/earth/city_tile_cdf.bin` | 128×64 luma×cos(lat) tile CDF (8192 f32). Embedded into `city_lights` by `gen-effects`. |
| `assets/textures/earth/clouds_rgba.png` | Less_Clouds 16K RGBA master. |
| `assets/textures/earth/normal.png` | 8K normal master. |
| `assets/textures/earth/roughness.jpg` | Invert(gloss) × water mask from landcover, 8K. Packed into the MR KTX2. |
| `assets/skyboxes/milky_way.cubemap.ktx2` (~23 MB) | Elodin-format UASTC cubemap, 4096 px faces, from `Stars_Milky_Way.jpg`. Veil crushed below luma 8/255 (see below). |

Five globe KTX2 files total **129.1 MB** (elodin embed budget 150 MB).
`models/earth.glb` and `textures/earth_night.png` are the old low-res pair.
Keep them. `gen-effects` does not touch the KTX2 / GLB files.

## How it is used

- Scenes `satellite`, `debug_earth_only`, `debug_cities_only` load `earth_v5.glb`.
- `render.rs` tags `EarthGlobeMaterial` / `EarthCloudsMaterial` and binds the
  five KTX2 maps with `RENDER_WORLD` usage, repeat-U / clamp-V, linear
  filters, anisotropy 8. Base color / emissive factors go back to white.
- Globe emissive is scaled by `star_visibility` (`EARTH_EMISSIVE_NIGHT = 120`).
  Noon is black. That sheet is the continuous city web; Hanabi sparkle sits
  on the same lights.
- Clouds are a second mesh, alpha blend, parented with the globe so
  `orient()` and the 20°/orbit spin stay locked. Opacity fades with
  `nightglow_visibility` (full through dusk, 0.05 at midnight).
- `environment.skybox` loads the cubemap onto `MainCamera`. Brightness is
  `SKYBOX_NIGHT_BRIGHTNESS (4000) * star_visibility` (noon = 0). The master is
  a dim exposure (mean 3/255, dust p99 37/255), so that gain — not the texture
  resolution — decides how much band survives tonemapping; `1000` left the
  cubemap nearly black wherever Hanabi stars were not covering for it. The gain
  would also lift the master's ~1–8/255 veil into grey haze, so `pack_skybox.py`
  zeroes it (`VEIL_BLACK_POINT`, luma-gated so hue is preserved). That crush
  also cut the file from 53 → 23 MB. Rotation follows `SkyRoot`. No
  `EnvironmentMapLight`. Bevy needs `ktx2` + `basis-universal` + `zstd_c`.
- Hanabi stars / MW stay. Accept some doubling at night; dim cubemap
  brightness before touching particle counts.
- Capture waits for Earth material images and a configured cube view, not
  just a spawned mesh.

## Rebuild

Needs Blender 4 and `toktx` (KTX-Software 4.4.x; v5 dropped `toktx`).

```bash
uv run scripts/prepare_earth_textures.py
uv run scripts/pack_earth_ktx2.py
/Applications/Blender.app/Contents/MacOS/Blender --background --python scripts/bake_earth_glb.py
TOKTX=scripts/.tools/toktx uv run scripts/pack_skybox.py
```

`scripts/.tools/` is gitignored. Drop a signed `toktx` + `libktx.4.dylib`
there (rpath is `@executable_path`). Do not run `install_name_tool` on it —
that breaks the hardened-runtime signature and macOS SIGKILLs the binary.

`pack_earth_ktx2.py` encodes UASTC quality 2, RDO λ 0.75, zstd 19, full
mips. JPEG masters go through a temp PNG. If the five outputs exceed 150 MB
it raises RDO and retries.

The bake asserts the GLB `images` list is empty. Do not decimate the
sphere. Do not add Draco / meshopt / basisu / webp.

## Locked look-dev

- Albedo: August (`Earth_Color_08_Aug.jpg`)
- Night: bright set
- Clouds: Less_Clouds
- Nadir: Cairo/Nile `(30, 31, 0)`
- No height displacement, no cloud bump, no Blender atmosphere object
