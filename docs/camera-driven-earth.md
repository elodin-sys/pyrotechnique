# Camera-driven Earth environment

Status: **implemented** in `src/earth_env.rs`. `falcon9-orbit` and `satellite`
share one camera-driven stack. Elodin KDL packaging is the next project.

`ViewerFrame` is computed each frame from `MainCamera` (fallback: rocket,
then origin). Curves: `h = |camera − earth_center| − R`.

| Knob | Curve |
|---|---|
| Atmosphere mode | Raymarched whenever `earth` is set (no flip) |
| Density | 1.0 → 0.16 smoothstep 20–80 km, 0.01 quanta |
| `space_visibility` | 0 → 1 over 20–80 km |
| Pad disc | Blend from spawn; alpha 1 below 3 km → 0 at 8 km |
| Ambient / EV / skybox / star+nightglow | camera `h` and camera-radial sun elevation |
| Earthshine / night fill aim | Earth→**craft** (lights the vehicle) |

Gating: `earth.is_some()` or `orbit_period_s > 0`. `falcon9` / apollo stay
inert. `satellite` sits at the 400 km asymptote (no scene edits).

---

## Why the disc popped at t≈40 s (history)

## Why the disc pops at t≈40 s (root cause)

Chain of events in `tune_ascent_environment` (`src/orbit.rs`):

1. The fade signal is **rocket** altitude (`viewer_frame` reads
   `RocketRoot`'s `GlobalTransform`, not the camera).
2. The rocket crosses 5 km between the t=30 (2.9 km) and t=45 (6.5 km)
   keyframes — at t≈39–40 s.
3. `pad_disc_visibility` drops below 0.99, so the system flips the disc
   material from `AlphaMode::Opaque` to `AlphaMode::Blend` to alpha-fade it.
4. That flip moves the disc from the opaque pass to the transparent pass —
   and **Bevy's atmosphere never touches the transparent pass**.

Point 4 is the mechanism. In `bevy_pbr` 0.19 the atmosphere composite
(`render_sky`) is scheduled `after(main_opaque_pass_3d)` and
`before(main_transparent_pass_3d)` (`bevy_pbr/src/atmosphere/mod.rs`). The
fullscreen pass reads the depth buffer: sky where depth is far-plane, and
aerial perspective (in-scattering + transmittance from the aerial-view LUT)
over every **opaque** pixel. Transparent geometry draws afterwards and never
receives haze.

So for 40 seconds the opaque disc sits fogged into the horizon — pale blue,
indistinguishable from ground. The frame the material flips to Blend, the
haze vanishes and the raw khaki circle (still at alpha ≈0.98) reappears dark
against the fogged globe. That is the pop in the screenshot.

Two more hard transitions of the same species, further up:

- At rocket altitude 20 km (t≈81 s) the atmosphere flips
  `LookupTexture → Raymarched` and the clear color flips to black in a single
  frame (`tune_ascent_environment`, `last_raymarched`).
- Density steps in 0.04 quanta from 1.0 → 0.16 over 20–80 km (~21
  `ScatteringMedium` rebuilds), each a small global scattering jump.

## Current state: what drives what

| Knob | Consumer | Driver today | Transition |
|---|---|---|---|
| Atmosphere mode + clear color | camera `AtmosphereSettings` | rocket h ≥ 20 km | **hard flip** |
| Scattering density 1.0→0.16 | `ScatteringMedium` rebuild | rocket h, 20–80 km | 0.04 steps |
| Pad disc fade | `PadDisc` material | rocket h, 5–8 km | **Opaque→Blend pop**, then Hidden at ~7.9 km |
| `space_visibility` (stars/airglow/skybox/emissive gate) | Hanabi `intensity`, skybox brightness, globe emissive | rocket h, 20–80 km | smooth |
| Sun elevation for star/night gates, EV, earthshine aim | lights, exposure, emitters | radial up of **rocket** | smooth |
| Ambient 80→0 | `GlobalAmbientLight` | rocket h | smooth |

Consequences of "rocket-driven":

- Free-flying the editor camera changes nothing. Zoom from the pad to space
  at t=0 and the sky stays LookupTexture/full density; park the camera on the
  ground and scrub to t=100 and the ground turns into the space look.
- The disc fade happens exactly when the chase camera is staring at it.
- The scheme only works for the one scripted flight; it cannot serve a plane,
  a hover, a descent, or Elodin's arbitrary sims.

Adjacent warts found in the same review (not camera-related, note for later):

- All transparents skip aerial perspective in Bevy — the cloud shell and
  distant Hanabi smoke never haze. Systemic; acceptable so far.
- `capture_jumps_clock` treats any capture past t=80 s as a "LEO lighting
  shot" and jumps the clock — `karman` (t=100) loses its real exhaust-trail
  history.
- The 20 km disc is a tangent plane: its rim floats ~31 m above the curving
  globe (x²/2R). Invisible today, relevant if the disc ever grows.

## Ideal state

One environment, no scripted switches. Every view-dependent knob is a
continuous function of the camera relative to Earth:

```text
h      = |camera − earth_center| − R      camera altitude
up     = normalize(camera − earth_center) camera radial
sun_el = dot(to_sun, up)
```

| Knob | Curve on `h` |
|---|---|
| Atmosphere mode | **Raymarched always** (no flip; see below) |
| Density | 1.0 → 0.16 smoothstep 20–80 km, 0.01 quanta |
| `space_visibility` | 0 → 1 over 20–80 km (unchanged shape) |
| Pad disc alpha | 1 below ~3 km → 0 at ~8 km, Blend from spawn |
| Ambient | ×(1 − space_visibility) |
| EV day/night, skybox brightness, star/nightglow gates | sun_el vs camera radial |

Vehicle-owned things stay vehicle-driven: earthshine / night fill aim along
Earth→**craft** (they light the vehicle), plume lights, emitter activity.

Properties of this scheme:

- Capture output barely moves: scenario cameras ride the rocket, so camera
  altitude ≈ rocket altitude ± a few hundred meters.
- The editor becomes the test rig the review asked for: pause at any t, zoom
  pad → LEO, and mode/density/disc/stars all follow the camera continuously.
- Vehicle-agnostic: ground vehicle, aircraft, satellite, or a free camera
  with no vehicle at all — same code path. This is the portable contract;
  Elodin's KDL `atmosphere` node (origin/radii/`raymarched`) grows the same
  camera-altitude curves.
- `satellite` (h≈400 km: space_vis 1, density 0.16, no disc) and `falcon9`
  (no `earth`, systems early-out) are unaffected.

Why Raymarched always: Bevy documents `LookupTexture` as "tailored to scenes
mostly inside the atmosphere" with precision that "tapers as the camera moves
far from the scene origin", and `Raymarched` as correct "for any type of
scene… planets seen from orbit". One mode deletes the flip and the
clear-color swap (raymarched sky covers every sky pixel, clear color can stay
black). Cost is GPU time; this is a look-dev tool. If pad stills regress,
fallback: keep the flip but key it on camera altitude with wide hysteresis in
a regime where both modes match (~30–50 km).

Pad disc: Blend from spawn means it *never* receives aerial perspective, in
exchange for never changing passes (no pop, and no z-fighting with the globe
at the tangent point since Blend doesn't depth-write). Risk: the disc's far
rim reads sharper on pad stills (today it's opaque and hazed). Mitigations in
order: accept if `lift-off`/`ascent` `_vs_target` hold (smoke and sky
dominate); shrink the disc toward Elodin's 10 km; longer term replace the
disc with a sphere-conforming high-res ground patch parented like the cloud
shell — the true "one asset" answer.

## Done / next

Pyrotechnique steps 1–5 are in. Zoom-ladder stills (`zoom-2km` / `30km` /
`150km` at t=5) and the karman t=85/95/100 series live under
`shots/falcon9-orbit/`. Capture jumps only for satellite (`orbit_start_s=0`)
or `end_time >= orbit_start_s` — `karman` integrates the trail.

**Later (Elodin):** schematic KDL togglable cinematic Earth for ECEF sims
(cube-sat, falcon9). The `earth_env` contract — camera radial altitude in,
{mode, density, disc alpha, space_visibility} out — is what that preset
carries.
