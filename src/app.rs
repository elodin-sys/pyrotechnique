//! App assembly shared by edit and capture modes.

use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use bevy_hanabi::HanabiPlugin;
use bevy_panorbit_camera::PanOrbitCameraPlugin;

use crate::scene::SceneConfig;
use crate::{CaptureArgs, EditArgs};

/// Simulated flight time, advanced from Bevy virtual time while playing.
/// Scrubbing/jumping resets particle continuity but not correctness.
#[derive(Resource, Debug, Clone)]
pub struct SimClock {
    pub t: f32,
    pub playing: bool,
    pub speed: f32,
}

impl Default for SimClock {
    fn default() -> Self {
        Self {
            t: 0.0,
            playing: true,
            speed: 1.0,
        }
    }
}

fn advance_sim_clock(mut clock: ResMut<SimClock>, time: Res<Time>) {
    if clock.playing {
        clock.t += time.delta_secs() * clock.speed;
    }
}

pub struct BaseConfig {
    pub scene: SceneConfig,
    pub window_size: (u32, u32),
    pub watch_assets: bool,
    pub vsync: bool,
    pub title: String,
}

pub fn build_base_app(config: BaseConfig) -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: config.title,
                    resolution: WindowResolution::new(
                        config.window_size.0,
                        config.window_size.1,
                    ),
                    present_mode: if config.vsync {
                        PresentMode::AutoVsync
                    } else {
                        PresentMode::AutoNoVsync
                    },
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin {
                watch_for_changes_override: Some(config.watch_assets),
                ..default()
            }),
    )
    .add_plugins(HanabiPlugin)
    .add_plugins(PanOrbitCameraPlugin)
    .insert_resource(config.scene)
    .init_resource::<SimClock>()
    .add_systems(Update, advance_sim_clock)
    .add_plugins(crate::render::EnvironmentPlugin)
    .add_plugins(crate::rocket::RocketPlugin)
    .add_plugins(crate::flight::FlightPlugin)
    .add_plugins(crate::effects::EmitterPlugin);
    app
}

pub fn run_edit(args: EditArgs) -> anyhow::Result<()> {
    let scene = crate::scene::load_scene_config(&args.scene)?;
    let mut app = build_base_app(BaseConfig {
        scene,
        window_size: (1600, 1000),
        watch_assets: true,
        vsync: true,
        title: "pyrotechnique".to_string(),
    });
    // Start paused on the pad; the user presses Play (or picks a scenario).
    app.insert_resource(SimClock {
        t: 0.0,
        playing: false,
        speed: 1.0,
    });
    app.add_plugins(crate::ui::EditorUiPlugin);
    app.run();
    Ok(())
}

pub fn run_capture(args: CaptureArgs) -> anyhow::Result<()> {
    use bevy::time::TimeUpdateStrategy;

    let scene = crate::scene::load_scene_config(&args.scene)?;
    let scenario = scene
        .scenario(&args.scenario)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "scenario '{}' not found; available: {}",
                args.scenario,
                scene
                    .scenarios
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?
        .clone();

    let end_time = args.time.unwrap_or(scenario.capture_time);
    let compare = match args.compare.as_deref() {
        None => None,
        Some("auto") => {
            let reference = scenario.reference.clone().ok_or_else(|| {
                anyhow::anyhow!("--compare auto: scenario '{}' has no reference", args.scenario)
            })?;
            Some(std::path::PathBuf::from(reference))
        }
        Some(path) => Some(std::path::PathBuf::from(path)),
    };
    if let Some(reference) = &compare
        && !reference.exists()
    {
        anyhow::bail!("reference image not found: {}", reference.display());
    }

    let (w, h) = parse_size(&args.size)?;
    let step = std::time::Duration::from_secs_f64(1.0 / args.fps.max(1.0));

    let mut app = build_base_app(BaseConfig {
        scene,
        window_size: (w, h),
        watch_assets: false,
        vsync: false,
        title: format!("pyrotechnique capture: {}", args.scenario),
    });
    // Start with dt frozen; the capture state machine enables fixed stepping
    // only once assets are loaded and RNGs are seeded (determinism).
    app.insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::ZERO))
        .insert_resource(crate::capture::CaptureConfig {
            scenario: args.scenario.clone(),
            end_time,
            out: args.out.clone(),
            compare,
            seed: args.seed,
            step,
        })
        .add_plugins(crate::capture::CapturePlugin);
    app.run();
    Ok(())
}

fn parse_size(size: &str) -> anyhow::Result<(u32, u32)> {
    let (w, h) = size
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow::anyhow!("--size must be WxH, got '{size}'"))?;
    Ok((w.trim().parse()?, h.trim().parse()?))
}
