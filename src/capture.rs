//! Deterministic capture mode: step the simulation to a scenario's capture
//! time with a fixed timestep, screenshot the window, optionally compose a
//! side-by-side with the target reference image, then exit.
//!
//! This is the AI-agent loop: edit `.effect` RON -> `pyrotechnique capture
//! --scenario X --compare auto` -> look at the PNG.
//!
//! Determinism: virtual time advances by a fixed `ManualDuration` step each
//! frame regardless of wall clock; the CPU spawner RNG and every effect's GPU
//! `prng_seed` are seeded from `--seed`; the sim clock only starts once all
//! effect assets are loaded and the rocket is normalized.

use std::path::PathBuf;
use std::time::Duration;

use bevy::app::AppExit;
use bevy::camera::Exposure;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use bevy_hanabi::{EffectAsset, EffectMaterial, ParticleEffect, Random};
use bevy_panorbit_camera::PanOrbitCamera;
use rand::SeedableRng;

use crate::app::SimClock;
use crate::effects::{Emitter, ShowEmitterGizmos};
use crate::render::{EarthReady, MainCamera};
use crate::rocket::RocketBounds;
use crate::scene::SceneConfig;

#[derive(Resource, Clone, Debug)]
pub struct CaptureConfig {
    pub scenario: String,
    pub end_time: f32,
    pub out: PathBuf,
    pub compare: Option<PathBuf>,
    pub seed: u32,
    pub step: Duration,
}

#[derive(Default, PartialEq, Debug, Clone, Copy)]
enum CapturePhase {
    /// Waiting for effect assets + rocket normalization. Virtual dt is zero.
    #[default]
    WaitingForAssets,
    /// RNGs seeded; letting the recompile settle for a few zero-dt frames.
    Settling,
    /// Fixed-dt stepping until the scenario capture time.
    Running,
    /// Screenshot requested; waiting for readback.
    Frozen,
}

#[derive(Resource, Default)]
struct CaptureState {
    phase: CapturePhase,
    settle_frames: u32,
}

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CaptureState>()
            .insert_resource(ShowEmitterGizmos(false))
            .add_systems(Update, (apply_scenario_camera, gate_and_capture).chain());
    }
}

/// Applies the scenario's camera preset every frame (tracking presets follow
/// the rocket), and disables orbit input.
fn apply_scenario_camera(
    scene: Res<SceneConfig>,
    config: Res<CaptureConfig>,
    clock: Res<SimClock>,
    mut camera: Query<
        (
            &mut Transform,
            &mut Projection,
            &mut Exposure,
            &mut PanOrbitCamera,
        ),
        With<MainCamera>,
    >,
) {
    let Some(scenario) = scene.scenario(&config.scenario) else {
        return;
    };
    let Some(preset) = scene.camera(&scenario.camera) else {
        return;
    };
    let Ok((mut transform, mut projection, mut exposure, mut orbit)) = camera.single_mut() else {
        return;
    };
    orbit.enabled = false;

    let (rocket_pos, _) = scene.flight.sample(clock.t);
    let (pos, look_at) = preset_pose(preset, rocket_pos);
    *transform = Transform::from_translation(pos).looking_at(look_at, Vec3::Y);
    *projection = Projection::Perspective(scene.environment.perspective(preset.fov_deg));
    if let Some(ev100) = scenario.exposure_ev100 {
        exposure.ev100 = ev100;
    }
}

/// Jump the sim clock for once-burst LEO skies. Pad/ascent on an Earth
/// scene still integrate from t=0 so Merlin/smoke have history.
fn capture_jumps_clock(env: &crate::scene::EnvironmentConfig, end_time: f32) -> bool {
    if env.orbit_period_s <= 1e-3 {
        return false;
    }
    env.orbit_start_s <= 1e-3 || end_time + 1e-3 >= env.orbit_start_s
}

/// Resolve a camera preset to a world-space (position, look_at) pair.
pub fn preset_pose(preset: &crate::scene::CameraPreset, rocket_pos: Vec3) -> (Vec3, Vec3) {
    let base_pos = Vec3::from(preset.pos);
    let base_look = Vec3::from(preset.look_at);
    if preset.track_rocket {
        let pos = if preset.follow_pos {
            rocket_pos + base_pos
        } else {
            base_pos
        };
        (pos, rocket_pos + base_look)
    } else {
        (base_pos, base_look)
    }
}

/// Waits for readiness, seeds RNGs, steps the clock, and fires the screenshot.
///
/// Virtual dt is held at zero (frozen particles) until everything is loaded
/// and seeded, so the number of loading/warmup frames cannot affect the
/// simulation outcome.
#[allow(clippy::too_many_arguments)]
fn gate_and_capture(
    mut commands: Commands,
    mut state: ResMut<CaptureState>,
    config: Res<CaptureConfig>,
    scene: Res<SceneConfig>,
    bounds: Res<RocketBounds>,
    earth_ready: Res<EarthReady>,
    asset_server: Res<AssetServer>,
    emitters: Query<(&ParticleEffect, Option<&EffectMaterial>), With<Emitter>>,
    mut effects: ResMut<Assets<EffectAsset>>,
    mut rng: ResMut<Random>,
    mut clock: ResMut<SimClock>,
    mut strategy: ResMut<TimeUpdateStrategy>,
) {
    match state.phase {
        CapturePhase::WaitingForAssets => {
            clock.t = 0.0;
            clock.playing = false;

            let assets_ready = scene.emitters.is_empty() || {
                let list: Vec<_> = emitters.iter().collect();
                !list.is_empty()
                    && list.iter().all(|(effect, material)| {
                        if !asset_server.is_loaded_with_dependencies(effect.handle.id()) {
                            return false;
                        }
                        let Some(asset) = effects.get(&effect.handle) else {
                            return false;
                        };
                        if asset.texture_layout().layout.is_empty() {
                            return true;
                        }
                        let Some(material) = material else {
                            return false;
                        };
                        material.images.iter().all(|handle| {
                            asset_server.is_loaded_with_dependencies(handle.id())
                        })
                    })
            };
            if !(assets_ready && bounds.ready && earth_ready.0) {
                return;
            }

            // Seed CPU spawner RNG and each effect's GPU PRNG. Mutating the
            // assets fires Modified events, which trigger a recompile.
            rng.0 = rand_pcg::Pcg32::seed_from_u64(config.seed as u64);
            for (emitter, _) in &emitters {
                if let Some(mut asset) = effects.get_mut(emitter.handle.id()) {
                    asset.prng_seed = config.seed;
                }
            }
            state.phase = CapturePhase::Settling;
            state.settle_frames = 0;
        }
        CapturePhase::Settling => {
            state.settle_frames += 1;
            if state.settle_frames < 8 {
                return;
            }
            *strategy = TimeUpdateStrategy::ManualDuration(config.step);
            clock.playing = true;
            // Once-burst skies are static. Jump LEO lighting shots; keep
            // pad/ascent on a real 0→t integrate so plumes have history.
            if capture_jumps_clock(&scene.environment, config.end_time) {
                let warmup = 24.0 * config.step.as_secs_f32();
                clock.t = (config.end_time - warmup).max(0.0);
            }
            state.phase = CapturePhase::Running;
            info!(
                "capture started: scenario '{}' to t={:.2}s, seed {}, step {:?}",
                config.scenario, config.end_time, config.seed, config.step
            );
        }
        CapturePhase::Running => {
            if clock.t < config.end_time {
                return;
            }
            // Freeze everything (virtual dt = 0) and take the shot.
            clock.playing = false;
            *strategy = TimeUpdateStrategy::ManualDuration(Duration::ZERO);
            state.phase = CapturePhase::Frozen;

            let out = config.out.clone();
            let compare = config.compare.clone();
            if let Some(parent) = out.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            info!("capturing screenshot to {}", out.display());
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_and_exit(out, compare));
        }
        CapturePhase::Frozen => {}
    }
}

/// Observer: save the captured frame, optionally compose the comparison
/// image, then quit the app.
fn save_and_exit(
    out: PathBuf,
    compare: Option<PathBuf>,
) -> impl FnMut(On<ScreenshotCaptured>, MessageWriter<AppExit>) {
    move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
        let result = (|| -> anyhow::Result<()> {
            let dynamic = captured
                .image
                .clone()
                .try_into_dynamic()
                .map_err(|e| anyhow::anyhow!("converting screenshot: {e:?}"))?;
            let rgba = dynamic.to_rgba8();
            rgba.save(&out)?;
            println!("wrote {}", out.display());

            if let Some(reference) = &compare {
                let composite_path = composite_path(&out);
                compose_side_by_side(&dynamic, reference, &composite_path)?;
                println!("wrote {}", composite_path.display());
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("capture failed: {e}");
            exit.write(AppExit::error());
            return;
        }
        exit.write(AppExit::Success);
    }
}

fn composite_path(out: &std::path::Path) -> PathBuf {
    let stem = out
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "capture".to_string());
    out.with_file_name(format!("{stem}_vs_target.png"))
}

/// Left: capture, right: reference, scaled to the same height.
fn compose_side_by_side(
    capture: &image::DynamicImage,
    reference: &std::path::Path,
    out: &std::path::Path,
) -> anyhow::Result<()> {
    let reference_img = image::open(reference)
        .map_err(|e| anyhow::anyhow!("opening reference {}: {e}", reference.display()))?;

    let height = capture.height();
    let ref_scaled = if reference_img.height() != height {
        let w = (reference_img.width() as f32 * height as f32 / reference_img.height() as f32)
            .round()
            .max(1.0) as u32;
        image::imageops::resize(
            &reference_img,
            w,
            height,
            image::imageops::FilterType::CatmullRom,
        )
    } else {
        reference_img.to_rgba8()
    };

    const GUTTER: u32 = 8;
    let total_w = capture.width() + GUTTER + ref_scaled.width();
    let mut canvas = image::RgbaImage::from_pixel(total_w, height, image::Rgba([24, 24, 24, 255]));
    image::imageops::overlay(&mut canvas, &capture.to_rgba8(), 0, 0);
    image::imageops::overlay(
        &mut canvas,
        &ref_scaled,
        (capture.width() + GUTTER) as i64,
        0,
    );
    canvas.save(out)?;
    Ok(())
}
