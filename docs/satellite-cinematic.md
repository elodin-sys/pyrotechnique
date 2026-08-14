# LEO satellite cinematic (Hanabi sky)

Pyrotechnique project `satellite`: OreSat in LEO against true-scale Earth.
Stars, Milky Way dust, city lights, and airglow are **Hanabi**. A cubemap
skybox (`milky_way.cubemap.ktx2`) sits under the particles and fades with
`star_visibility` so noon stays empty. Reference stills live in
`targets/satellite/`. The vehicle is Elodin’s `oresat-low.glb`; Earth is
`earth_v5.glb` (8K August + Less_Clouds). See `docs/high-res-earth.md`.

The stills are ISS photography. We are not rebuilding the ISS. The job is the
lighting those photos have: hard sun, a curved Earth filling the frame, a thin
atmosphere limb, and (at night) city lights, airglow, and a dense star field.
OreSat sits in the foreground as the craft we actually own.

Elodin port is **out of scope**. Contracts stay portable: `SimulationSpace::Local`,
`intensity` property, never `SimulationSpace::Global`.

## Frame

Satellite-local, true Earth. Craft at the origin. Earth center at
`(0, -(R+h), 0)` with `R = 6_378_140`, `h = 400_000`. Camera 6–12 m from
OreSat. `far ≥ 2e7`. No ground disc.

Day/night is **orbit phase**, not Earth-spin. `SkyRoot` (sun + sky-attached
emitters) rotates about +X once per 90 s of `SimClock` (~92 min realtime).
`t=0` noon, `t=22.5` dusk, `t=45` midnight, `t=67.5` dawn. Earth can have a
slower extra Y-spin so continents drift (`earth_spin_deg_per_orbit`).

OreSat stays at the authored GLB size (~5.4 m AABB, Elodin cube-sat scale).
`target_height: 5.4` so normalize ≈ 1. `align_to_velocity: false`. Flight
keyframes stay at the origin.

`earth.rotation_deg` is `(lat_deg, lon_deg, roll_deg)`: that geographic point
faces the craft. Current nadir is Cairo/Nile, `(30, 31, 0)`.

## What the stills agree on

Daylight (`iss-day.webp`, `iss-day-2.jpg`, `iss-day-3.webp`):

- One directional sun. Lit faces near-white. Shadows black and sharp. Almost no fill.
- Earth occupies a large fraction of the frame, with obvious curvature.
- Atmosphere is a thin cyan/white band on the limb, then cut to black. No stars.
- Camera a few tens of meters from a large craft, looking slightly down.

Night and terminator (`low-earth-orbit-*`, `iss-night.jpg`):

- Earth is a dark disc. Warm city clusters bloom. Oceans stay black.
- Limb is layered: a bright thin band (green-yellow) with a fainter red/orange
  haze above it. That is airglow, not Rayleigh.
- Stars are dense and vary in brightness. Several frames show the Milky Way
  as a dusty band.
- Ignore window frames, Canadarm, ISS truss.

## Particle design

All satellite effects: `SimulationSpace::Local`, `SpawnerSettings::once(capacity)`,
lifetime `1e9`, velocity 0, `AlphaMode::Add`, `OrientMode::FaceCamera`. Vacuum:
no drag, HDR colors, bloom does the glow.

Hanabi 0.20 stores `screen_space_size` on `SizeOverLifetimeModifier` but never
applies it. Stars sized in “pixels” at 15,000 km are invisible. Use world metres
(`star_world_size` in `builders.rs`).

| Effect | Attach | Capacity | Role |
|---|---|---|---|
| `stars_dim` | sky | ~800k | Uniform sphere, radius 1.5e7 m. Power-law magnitude. |
| `stars_bright` | sky | ~40k | Same sphere, larger/hotter, some color temp. Bloom bait. |
| `milky_way` | sky | ~400k | Same sphere, keep a band near a galactic plane. Warmer, dusty. |
| `city_lights` | earth | ~1.5M | Inverse-CDF on the 128×64 night-tile map, then `R+8 km`. Color from `night.jpg`; `luma_kill` drops leftover ocean. |
| `airglow_green` | earth | ~520k | Shell at `R+95 km`. Tight limb × night. Intensity ramps after the terminator. |
| `airglow_red` | earth | ~340k | Shell at `R+150 km`, a faint red/orange whisper above the green. |

**Once-burst rule:** do not scale `count` or deactivate `is_once()` spawners.
Dim via the `intensity` / `sun_dir` **property**. Authored defaults are
noon-safe (`intensity = 0`, `sun_dir = +Y`) so a missed write does not paint
airglow on the day stills. `orbit.rs` owns sky/earth intensity after spawn.

**City geography:** NoneCG 32K night lights, downsampled to
`assets/textures/earth/night.jpg` (same pixels as the globe emissive). Init
cannot bind that texture, so `CityTileCdfModifier` embeds a 128×64 tile CDF
and inverse-CDF-spawns onto the lights. `SphereMapColorModifier` still
samples `textureSampleLevel` for color, with `alpha *= step(luma_kill, luma)`.
Do not fake cities with noise. The mesh emissive is the continuous sheet
(`star_visibility × 120`); particles are the sparkle on those same places.

Capture waits for `EffectMaterial` images, Earth material textures, and the
skybox cube view — not just `.effect` assets or a spawned mesh.

`gen-effects` regenerates `soft_circle.png` / `smoke_puff.png` but **not**
the earth / skybox files.

## Scenarios

| Scenario | t (s) | EV | Target |
|---|---|---|---|
| `day-limb` | 0 | ~13.5 | `iss-day-3.webp` |
| `day-look-down` | 0 | ~13.5 | `iss-day-2.jpg` (ignore window/arm) |
| `dusk-terminator` | 22.5 | ~11 | `low-earth-orbit-night-stars.jpeg` as limb/color ref |
| `night-airglow` | 45 | ~9 | `low-earth-orbit-night.png` |
| `night-cities` | 45 | ~9 | `low-earth-orbit-night-lights-on.jpeg` |
| `starboard` | 45 | ~8.5 | `iss-night.jpg` |
| `milky-way` | ~67.5 | ~10 | `low-earth-orbit-stars-sunrise.jpeg` |

Debug scenes: `debug_sat_only`, `debug_earth_only`, `debug_stars_only`,
`debug_cities_only`.

```bash
cargo run -q -- capture --project satellite --scenario day-limb --compare auto
# look at shots/satellite/day-limb_vs_target.png
```

Editor: `cargo run -- edit satellite`, scrub 0→90 s. Stars and cities must
ramp, not pop. Capture skips the EV interpolation so the scenario’s
`exposure_ev100` wins.

## HDR ballpark

Star pinpricks ~10–20×, bright stars ~40–60×, city cores ~8–25× after the map,
airglow ~3–8× at low alpha. Day EV100 ~13.5, night ~8–10, interpolate with sun
elevation in the editor (`night_exposure_ev100`).

Earthshine is a warm directional on render layer 1 (craft only). A second,
dimmer layer-0 fill lights the camera-facing night globe so continents stay
just readable. Both fade to zero at noon. Do not raise `GlobalAmbientLight`.

## What “awesome” means

- Day `_vs_target` shares hard sun, black shadows, Earth curve, thin cyan limb,
  empty sky (no stars, no airglow particles).
- Night `_vs_target` shares airglow layers, blooming cities, dense stars with a
  Milky Way band.
- Playing 0–90 s is a continuous day→night→day pass: stars and cities fade with
  the terminator, exposure tracks, no pops.
- Debug scenes still isolate craft / Earth / stars / cities.

The same sky/earth stack on a flying Falcon 9 is `falcon9-orbit` (Florida
nadir, pad → 400 km). See `docs/falcon9-orbit.md`. This scene stays Cairo.
