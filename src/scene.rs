//! Scene definition: model, environment, emitters, flight path, cameras, scenarios.
//!
//! The scene is a small RON file (see `assets/scenes/falcon9.scene.ron`). It is
//! read once at startup with plain `ron` (restart to pick up scene edits);
//! `.effect` files referenced by emitters hot-reload through the Bevy asset
//! server while the app runs.
//!
//! Emitter fields deliberately mirror Elodin's `thruster` KDL schema
//! (`position`, `direction`, `intensity`) so tuned results port straight back.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Resource, Deserialize, Serialize, Clone, Debug)]
pub struct SceneConfig {
    pub model: ModelConfig,
    pub environment: EnvironmentConfig,
    #[serde(default)]
    pub emitters: Vec<EmitterConfig>,
    pub flight: FlightConfig,
    #[serde(default)]
    pub cameras: Vec<CameraPreset>,
    #[serde(default)]
    pub scenarios: Vec<ScenarioConfig>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ModelConfig {
    /// Asset-relative GLB path.
    pub path: String,
    /// The model is uniformly rescaled so its bounding box height equals this
    /// (meters), base centered at the origin, +Y up.
    pub target_height: f32,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct EnvironmentConfig {
    pub sun_azimuth_deg: f32,
    pub sun_elevation_deg: f32,
    /// Direct sunlight is ~100k lux.
    pub sun_illuminance: f32,
    /// Camera exposure (EV100). Sunny-16 daylight is ~14-15.
    pub exposure_ev100: f32,
    pub bloom_intensity: f32,
    pub ground_radius: f32,
    pub ground_color: [f32; 3],
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct EmitterConfig {
    pub name: String,
    /// Asset-relative `.effect` path, e.g. "effects/merlin_flame.effect".
    pub effect: String,
    /// Rocket-local position (after normalization: base center = origin, +Y up).
    pub position: [f32; 3],
    /// Exhaust direction in the rocket frame (particles fly this way).
    pub direction: [f32; 3],
    /// Spawn-rate multiplier at full activity.
    #[serde(default = "default_intensity")]
    pub intensity: f32,
    /// Optional (time, multiplier) keyframes over flight time; linearly
    /// interpolated, clamped at the ends. Empty means "always 1.0".
    #[serde(default)]
    pub activity: Vec<[f32; 2]>,
    /// What the emitter is attached to: "rocket" (default; rides the vehicle,
    /// position/direction in the rocket frame) or "world" (fixed in world
    /// space — e.g. pad smoke that must stay at the launch pad).
    #[serde(default = "default_attach")]
    pub attach: String,
}

fn default_intensity() -> f32 {
    1.0
}

fn default_attach() -> String {
    "rocket".to_string()
}

impl EmitterConfig {
    /// Activity multiplier at flight time `t` (see `activity`).
    pub fn activity_at(&self, t: f32) -> f32 {
        sample_keyframes(&self.activity, t).unwrap_or(1.0)
    }
}

/// Linear interpolation over `[t, value]` keyframes, clamped at both ends.
pub fn sample_keyframes(keys: &[[f32; 2]], t: f32) -> Option<f32> {
    if keys.is_empty() {
        return None;
    }
    if t <= keys[0][0] {
        return Some(keys[0][1]);
    }
    if let Some(last) = keys.last()
        && t >= last[0]
    {
        return Some(last[1]);
    }
    for pair in keys.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t >= a[0] && t <= b[0] {
            let span = (b[0] - a[0]).max(1e-6);
            let alpha = (t - a[0]) / span;
            return Some(a[1] + (b[1] - a[1]) * alpha);
        }
    }
    keys.last().map(|k| k[1])
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct FlightConfig {
    /// Position keyframes of the rocket base over time. Attitude is derived
    /// from the path tangent (+Y along velocity).
    pub keyframes: Vec<FlightKey>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct FlightKey {
    pub t: f32,
    pub pos: [f32; 3],
}

impl FlightConfig {
    pub fn duration(&self) -> f32 {
        self.keyframes.last().map(|k| k.t).unwrap_or(0.0)
    }

    /// Sample position and velocity at time `t` (cubic Hermite with
    /// finite-difference tangents; clamped outside the keyframe range).
    pub fn sample(&self, t: f32) -> (Vec3, Vec3) {
        let keys = &self.keyframes;
        match keys.len() {
            0 => return (Vec3::ZERO, Vec3::Y),
            1 => return (Vec3::from(keys[0].pos), Vec3::Y),
            _ => {}
        }
        let t = t.clamp(keys[0].t, keys[keys.len() - 1].t);
        let i = keys
            .windows(2)
            .position(|w| t >= w[0].t && t <= w[1].t)
            .unwrap_or(keys.len() - 2);

        let p = |j: usize| Vec3::from(keys[j.clamp(0, keys.len() - 1)].pos);
        let kt = |j: usize| keys[j.clamp(0, keys.len() - 1)].t;

        let (t0, t1) = (keys[i].t, keys[i + 1].t);
        let h = (t1 - t0).max(1e-6);
        let s = (t - t0) / h;

        // Finite-difference tangents (scaled to segment length).
        let m0 = if i == 0 {
            (p(1) - p(0)) / (kt(1) - kt(0)).max(1e-6)
        } else {
            (p(i + 1) - p(i - 1)) / (kt(i + 1) - kt(i - 1)).max(1e-6)
        };
        let m1 = if i + 2 >= keys.len() {
            (p(i + 1) - p(i)) / h
        } else {
            (p(i + 2) - p(i)) / (kt(i + 2) - kt(i)).max(1e-6)
        };

        let (p0, p1) = (p(i), p(i + 1));
        let s2 = s * s;
        let s3 = s2 * s;
        let pos = (2.0 * s3 - 3.0 * s2 + 1.0) * p0
            + (s3 - 2.0 * s2 + s) * h * m0
            + (-2.0 * s3 + 3.0 * s2) * p1
            + (s3 - s2) * h * m1;
        // Derivative of the Hermite basis, divided by h to get world units/s.
        let vel = ((6.0 * s2 - 6.0 * s) * p0
            + (3.0 * s2 - 4.0 * s + 1.0) * h * m0
            + (-6.0 * s2 + 6.0 * s) * p1
            + (3.0 * s2 - 2.0 * s) * h * m1)
            / h;
        (pos, vel)
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct CameraPreset {
    pub name: String,
    pub pos: [f32; 3],
    pub look_at: [f32; 3],
    #[serde(default = "default_fov")]
    pub fov_deg: f32,
    /// When true, `look_at` (and optionally `pos`) are offsets added to the
    /// rocket's current position instead of absolute world coordinates.
    #[serde(default)]
    pub track_rocket: bool,
    /// With `track_rocket`, also move the camera position with the rocket.
    #[serde(default)]
    pub follow_pos: bool,
}

fn default_fov() -> f32 {
    45.0
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ScenarioConfig {
    pub name: String,
    /// Camera preset name.
    pub camera: String,
    /// Flight time (seconds since t=0) at which to capture.
    pub capture_time: f32,
    /// Reference target image for --compare auto (workspace-relative path).
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub exposure_ev100: Option<f32>,
}

impl SceneConfig {
    pub fn scenario(&self, name: &str) -> Option<&ScenarioConfig> {
        self.scenarios.iter().find(|s| s.name == name)
    }

    pub fn camera(&self, name: &str) -> Option<&CameraPreset> {
        self.cameras.iter().find(|c| c.name == name)
    }
}

/// Load a scene config from `assets/<scene_path>`.
pub fn load_scene_config(scene_path: &str) -> anyhow::Result<SceneConfig> {
    let path = std::path::Path::new("assets").join(scene_path);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("reading scene file {}: {e}", path.display()))?;
    let config: SceneConfig = ron::from_str(&text)
        .map_err(|e| anyhow::anyhow!("parsing scene file {}: {e}", path.display()))?;
    Ok(config)
}
