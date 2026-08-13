# Agent guide: pyrotechnique

You are editing GPU particle effects for a 3D HDR vehicle scene. The tool is
built around a tight loop you can drive entirely from files + CLI.

Work is organized into **projects** (`falcon9`, `apollo-lander`, ...). A
project name resolves to:

```text
assets/scenes/<project>.scene.ron    scene: model, emitters, flight, cameras, scenarios
assets/effects/<project>/*.effect    the effect files you tune
targets/<project>/                   reference images
shots/<project>/                     capture output
```

## The loop

```bash
# 1. Edit an effect (RON) or the scene
#    assets/effects/<project>/*.effect | assets/scenes/<project>.scene.ron

# 2. Capture the scenario you're tuning (deterministic; ~15-60 s wall time).
#    Output defaults to shots/<project>/<scenario>.png
cargo run -q -- capture --project falcon9 --scenario lift-off --compare auto

# 3. LOOK at shots/<project>/<scenario>_vs_target.png (your render left, target right)
#    Then iterate.
```

Scenarios — falcon9: `lift-off`, `ascent`, `max-q`, `mid-flight`,
`smoke-trail`. apollo-lander: `plume-side`, `plume-side-90` (90° azimuth
consistency check), `plume-closeup` (First Man bell close-up), `plume-top`,
`rcs-far`, `rcs-close`, `ground-effect`, `touchdown`. rocket (2 m model
rocket, 6 s boost): `initial-boost`, `mid-boost`, `late-boost`. satellite:
`day-limb`, `day-look-down`, `dusk-terminator`, `night-airglow`,
`night-cities`, `starboard`, `milky-way`. Debug isolates:
`debug_sat_only`, `debug_earth_only`, `debug_stars_only`,
`debug_cities_only`.

Useful flags: `--time <s>` (override capture time), `--seed <n>`, `--size WxH`,
`--fps <hz>`, `--out <path>`. `--project` also selects debug scenes (e.g.
`--project debug_flame_only`, a falcon9 variant with smoke disabled).

Rules of thumb:

- Always view the `_vs_target.png` composite, not just your render.
- Wall time scales with capture_time x particle density; falcon9 `lift-off`
  (t=5) is fast, `max-q` (t=60) is the slowest.
- Isolate one effect by setting other emitters' `intensity: 0.0` in a copy of
  the scene (see `assets/scenes/debug_flame_only.scene.ron`, or the satellite
  `debug_*_only` scenes).
- Satellite once-burst fields are static after spawn; capture jumps the clock
  (warmup ~24 steps) so a 45 s midnight shot is not 2700 frames.

## Shared effect-property contracts (port to Elodin as-is)

Two optional hanabi property conventions ride inside `.effect` files and are
honored identically by pyrotechnique and Elodin:

- **`intensity` (scalar, 1.0 = full throttle):** the runtime writes the live
  `intensity x activity(t)` signal every frame next to the spawner-rate
  scaling. Wire it into speed/size/color expressions so throttle drives plume
  length/brightness, not just density. Keep the effect pixel-identical at
  `intensity = 1.0` (that is the tuned look). Reference: `merlin_core`,
  `merlin_flame` builders. **Exception — once-burst fields** (`SpawnerSettings::is_once()`,
  satellite stars/cities/airglow): do **not** scale `count` or deactivate the
  spawner with intensity. Dim via the `intensity` / `sun_dir` **property**
  only. If intensity is 0 before the first sim frame, the field can be empty
  forever. Sky/earth intensity is owned by `orbit.rs`, not `apply_emitter_intensity`.
- **`sun_dir` + `view_pos` (vec3, satellite Earth effects):** direction to the
  sun and camera position in the effect's local (EarthRoot) frame. City lights
  and airglow fade the day hemisphere and tighten the limb band. Defaults are
  noon-safe (`sun_dir = +Y`, `intensity = 0`) so a missed property write does
  not paint a green limb on the day stills.
- **`spawn_origin` + `spawn_axis` (vec3, the anchored-trail contract):** the
  effect stays `SimulationSpace::Local` but runs on a **world-fixed anchor**
  (here: the world origin; Elodin: a grid-cell entity frozen at ignition).
  The runtime feeds the moving nozzle pose through the properties each frame,
  so particles spawn at the nozzle and hang in world space — persistent
  launch trails that survive Elodin's floating origin. Never use
  `SimulationSpace::Global` for new effects; it cannot port. Reference:
  `exhaust_smoke` builder + `anchor_trail_emitters`/`apply_trail_properties`
  in `src/effects/mod.rs`.

## What to edit where

| Goal | File | Notes |
|---|---|---|
| Colors, brightness, sizes, spawn rate, alpha mode | `assets/effects/<project>/*.effect` | Gradients + spawner are plain data; hot-reloads in edit mode |
| Lifetime/velocity/drag constants | same file, `module.expressions` list | Literals referenced by `"#N"` handles from `init_modifiers`/`update_modifiers` |
| Effect *structure* (new modifiers, expressions) | `src/effects/builders.rs` | Then `cargo run -q -- gen-effects` (regenerates ALL built-in `.effect` files — overwrites hand edits!) |
| Emitter placement/intensity/activity, cameras, scenarios, flight path, sun/exposure/bloom | `assets/scenes/<project>.scene.ron` | Scene loads at startup (no hot reload; re-run, or re-pick the project in the editor UI) |
| Emitter dynamic light (nozzle glow on structure/ground) | same file, emitter `light: Some(LightConfig(...))` | `color`, `intensity_lm` (lumens), `range`, `offset_m` down the exhaust axis, optional `spot_angle_deg`, `shadows`. Scales with `intensity x activity(t)`. Not part of `.effect` (pure hanabi); ports 1:1 to Elodin's `thruster { light ... }` |
| Sprite textures | `write_soft_circle` / `write_smoke_puff` in builders | Regenerated by `gen-effects`. `earth_night.png` is **not** regenerated — do not overwrite it. |

**Warning:** `gen-effects` overwrites every generated `.effect` file across
all projects. If you tuned values in the RON directly, port them back into
`builders.rs` before regenerating, or don't regenerate.

## Adding a new project

1. `targets/<name>/` — drop reference images in.
2. `assets/scenes/<name>.scene.ron` — copy the closest existing scene; set
   `model.path` (GLB in `assets/models/`), `target_height`, emitters,
   flight keyframes, cameras, and scenarios (`reference:` points at a target
   image, workspace-relative).
3. `assets/effects/<name>/` — start by copying/adapting effects from another
   project, or add builders to `src/effects/builders.rs` + `gen-effects`.
4. Verify: `cargo run -q -- capture --project <name> --scenario <s> --compare auto`.

Scene flags for non-Earth/non-rocket scenes: `environment.atmosphere: false`
(airless body: black sky, no scattering) and `flight.align_to_velocity: false`
(keep landers upright instead of pitching along the path). Satellite also
needs `ground_radius: 0`, `camera_far ≥ 2e7`, `earth: Some(...)`, and
`orbit_period_s: 90`. `earth.rotation_deg` is `(lat_deg, lon_deg, roll_deg)`
so that geographic point faces the craft (current: Cairo/Nile ≈ `(30, 31, 0)`).

## Satellite (LEO / Hanabi sky)

Craft at the origin. Earth center at `(0, -(R+h), 0)` with `R = 6_378_140`,
`h = 400_000`. `SkyRoot` (sun + star emitters) rotates about +X once per 90 s
sim orbit: t=0 noon, 22.5 dusk, 45 midnight, 67.5 dawn. No cubemap.

| Effect | Attach | Role |
|---|---|---|
| `stars_dim` / `stars_bright` / `milky_way` | sky | Pinpricks on a 15e6 m sphere. Hanabi 0.20 ignores `screen_space_size`; sizes are world metres via `star_world_size` in builders. |
| `city_lights` | earth | Shell at `R+80 km` (depth-test vs the globe). Fragment `SphereMapColorModifier` samples `textures/earth_night.png`. Init/update **cannot** bind material images. |
| `airglow_green` / `airglow_red` | earth | Shells at 95 km / 250 km. Limb term uses `view_pos`; night via `sun_dir` × `intensity`. |

HDR ballpark: dim stars ~10–20×, bright ~40–60×, city cores ~8–25× after the map, airglow ~3–8× at low alpha. Day EV100 ~13.5, night ~8–10.

Vacuum: no drag, `SimulationSpace::Local`, `AlphaMode::Add`, `OrientMode::FaceCamera`. Elodin port is out of scope; keep Local + `intensity`, never `SimulationSpace::Global`.

## HDR intuition (why colors are > 1.0)

The viewport is a physically-exposed HDR camera (EV100 ~13-15, TonyMcMapface
tonemap, additive bloom). The sunlit sky is ~1.0; anything that should read as
*emitting light* needs RGB well above 1.0:

- flame core: 20-60x
- orange flame body: 5-25x
- vacuum plume (apollo): 5-12x, low alpha — it's translucent shimmer
- sunlit smoke/dust: ~1.1-2.2x (it reflects, not emits; alpha does the work)
- star pinpricks (satellite): ~10–20× dim / ~40–60× bright; Milky Way dust lower
- city cores / airglow: ~8–25× / ~3–8× at low alpha

If a flame looks washed-out/pale: raise color intensity or drop `alpha_mode`
to `Add`. If smoke blooms into glare: it's too bright — smoke should stay
`Blend` with modest RGB.

## Effect anatomy

falcon9 (dense atmosphere — drag, buoyancy, billows):

- `merlin_core` (Local/Add): the blinding column. Length ~ speed x lifetime.
  Spawn cone sized to a Merlin bell exit (~0.5 m radius).
- `merlin_flame` (Local/Blend): near-parallel orange column around the core
  (velocity radiates from a virtual apex far behind the nozzle — ~26 m for
  ~2° half-angle; tighten further by moving the apex farther +Y).
- `exhaust_smoke`: persistent trail via the anchored-trail contract
  (`spawn_origin`/`spawn_axis`); hangs 30-110 s.
- `pad_smoke` (`attach: "world"`): lift-off ground clouds; `activity` cuts
  it off at t~12 s.
- `rcs_dart` (Local/Add): falcon9-sized cold-gas darts (apollo `rcs_puff` is
  for the 5 m lander — do not reuse it on the 70 m booster).

apollo-lander (vacuum — no drag, no billowing, everything ballistic):

- `descent_plume` (Local/Add): the DPS core — a solid, cool-white column
  filling the full bell mouth (matched to the First Man close-up). Very high
  rate (22k/s) + low per-sprite alpha: overlap makes the solid look. Spawn
  cone + birth width are sized to the measured bell wall (0.45 m exit
  radius); sprites widen only after clearing the exit.
- `descent_glow` (Local/Add): camera-facing halo billboards stacked on the
  same nozzle, fast/long-lived enough to wrap the full ~5 m column. The
  effect offsets its spawn volume below the exit plane itself, so both
  layers share one emitter position. **This layer is what gives the plume
  volume from every azimuth** — `AlongVelocity` streaks foreshorten into a
  flat fan wherever their divergence points at the viewer, so every
  vehicle-scale plume should ship as core + camera-facing halo. In Elodin
  the pair ports to a *single* `thruster` node: the halo goes in as an
  `effect "…"` child node (layer), not a duplicate emitter.
- `rcs_puff` (Local/Add): sharp white-blue darts. The emitters pulse via
  scene `activity` keyframes (t=18-20 s and 33-35 s braking phases).
- `ground_dust` (Global/Blend, `attach: "world"`): flat radial streak sheet;
  lunar gravity only, `AlongVelocity` stretched sprites, killed below ground.
  Activity ramps in under ~8 m altitude and stops right after engine cutoff.

Shared conventions: exhaust axis is **local -Y** (the emitter entity rotates
-Y onto the scene `direction`); smoke/dust sprites use per-particle random
modulation (`ColorBlendMode::Modulate`) for structure.

## Determinism

Captures fix the timestep and seed the CPU (`Random` resource) and GPU
(`prng_seed`) RNGs, and the sim only starts after all assets load. Two runs
with the same seed produce the same simulation; pixels can differ ~0.3% from
GPU blend-order nondeterminism. Treat captures as visually — not bitwise —
reproducible.

## Edit mode (for humans, but useful to know)

`cargo run [-- edit <project>]` opens the editor: project picker (switching
saves pending edits, re-picking the open project re-reads the scene from
disk), emitter list + inspector (spawner rate, alpha mode, HDR color/size
gradient editors), scenario/camera dropdowns, sim-time scrub, reference-image
overlay window. Inspector edits **auto-save** to the `.effect` file ~0.6 s
after the last tweak; external file edits hot-reload live. The capture CLI
never includes the UI.

## Elodin connection (live port path)

Emitter schema (`position`/`direction`/`intensity`) intentionally mirrors the
`thruster` KDL node in Elodin schematics, and Elodin loads these exact
`.effect` files: `thruster effect="effects/<project>/<name>.effect"` with
intensity from live sim telemetry. Porting rules that affect authoring here:

- **Scale contract:** author at the metric size the sim renders (apollo LM:
  5.0 m). Check the target sim's GLB scale before tuning.
- **Static emitters (`attach: "world"`) must be `SimulationSpace::Local`** so
  they survive Elodin's floating origin. Moving-emitter trails port through
  the anchored-trail contract (`spawn_origin`/`spawn_axis` properties — see
  above); falcon9 `exhaust_smoke` is the reference and runs live in
  `elodin/examples/falcon9/`.
- **Intensity contract:** rate authored in the file = full throttle; both
  tools scale the spawner count by intensity (`Uniform` ranges preserved)
  **except** `SpawnerSettings::is_once()` fields — those spawn at full
  capacity and dim through the `intensity` property. `activity` timelines
  are authoring stand-ins for sim viz channels and are not ported.
- **Texture slots bind by name** in both tools: `smoke` -> smoke_puff sprite,
  `mask`/other -> soft circle, `night` -> `textures/earth_night.png`.

Full checklist in `README.md` ("Elodin port path"); design record in
`../docs/design-thruster-effects-port.md`; Elodin internals in
`../docs/crash-course-thruster-particles.md`.
