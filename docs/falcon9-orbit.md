# Falcon 9 sky-to-space

Project `falcon9-orbit`: the same 70 m booster as `falcon9`, on a true-scale
Florida Earth from t=0, then a gravity-turn into LEO. Pad stills and the
satellite cinematic share one environment. `falcon9.scene.ron` stays the
pad-only look-dev loop.

## Frame

Earth center is fixed at `(0, -R, 0)` with `R = 6_378_140`. Nadir is LC-39A
(`rotation_deg: (28.608, -80.604, 0)`). The rocket moves; cameras and
`SimulationSpace::Local` plumes stay in that pad-origin ENU. `far = 2e7`
from t=0 so the limb exists even when you are not looking at it.

Do not put the vehicle at ECEF radius 6.37e6. A 400 km climb is ~3% of the
15,000 km star sphere, so stars stay parented to `SkyRoot` at the pad.

View-dependent knobs use **camera** radial
`up = normalize(camera.pos - earth.center)` (`src/earth_env.rs`). Earthshine
still aims Earth→craft. Sun for Act 1 is the falcon9 afternoon (az 300,
el 38). `orbit_start_s: 140` begins the compressed SkyRoot +X day/night
(`t=140` day, `185` night).

## Altitude contract

`earth_env.rs` derives `|camera.pos - earth.center| - R` each frame:

| Camera altitude | Atmosphere | Density | Stars / cities / airglow |
|---|---|---|---|
| 0–20 km | Raymarched | 1.0 | Off (`space_visibility = 0`) |
| 20–80 km | Raymarched | 1.0 → 0.16 | Fades in if the sun is down |
| 80 km+ | Raymarched | 0.16 | `star_vis = star_visibility(el) * space_visibility(h)` |

The local khaki pad disc (~20 km) is Blend from spawn and fades 3–8 km.
8K Earth is too coarse for lift-off grit. See `docs/camera-driven-earth.md`.

## Scenarios

Act 1 cameras and times match `falcon9` (`lift-off` … `smoke-trail`). New:

| Scenario | t (s) | EV | Bar |
|---|---|---|---|
| `karman` | 100 | ~14 | Sky going black, disc curvature |
| `leo-day-limb` | 140 | 13.5 | vs `targets/satellite/` day limb |
| `leo-night-cities` | 185 | 9 | vs satellite night cities |
| `leo-airglow` | 185 | 9 | vs satellite night limb |
| `zoom-2km` / `30km` / `150km` | 5 | — | Camera-altitude continuity (no reference) |

LEO composition differs (Falcon 9 vs OreSat); lighting is the check.
Capture jumps only for `end_time >= orbit_start_s` (or satellite, where
`orbit_start_s` is 0). `karman` still integrates from 0 so the trail exists.

```bash
cargo run -q -- capture --project falcon9-orbit --scenario lift-off --compare auto
cargo run -q -- capture --project falcon9-orbit --scenario leo-day-limb --compare auto
```

Tune order if a still is wrong: density, then `space_visibility`, then EV.
Milky pad → density too low or raymarched too early. Hazed LEO disc →
density did not fall. Stars on the pad → `space_visibility` is wrong.

Elodin already does ECEF + `earth.glb` + a 10 km pad disc. The portable
contract is radial up, altitude-driven atmosphere, `space_visibility` on
sky layers, pad disc as a local supplement. Do not port until this scene
looks right.
