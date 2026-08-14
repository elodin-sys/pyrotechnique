//! Projects: named effect workspaces (falcon9, apollo-lander, ...).
//!
//! A project is a name that resolves by convention:
//! - scene:   `assets/scenes/<name>.scene.ron`
//! - effects: `assets/effects/<name>/*.effect`
//! - targets: `targets/<name>/` (reference images)
//! - shots:   `shots/<name>/` (captures + editor screenshots)
//!
//! Switching projects at runtime (`LoadProject`) swaps the `SceneConfig`
//! resource and despawns the rocket/emitters/environment; the reactive spawn
//! systems then rebuild the world from the new scene.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCamera;

use crate::app::SimClock;
use crate::effects::Emitter;
use crate::render::{EarthReady, EnvironmentEntity, MainCamera, snap_orbit_to_preset};
use crate::rocket::{RocketBounds, RocketRoot};
use crate::scene::{CameraPreset, SceneConfig};

/// The currently open project.
#[derive(Resource, Clone, Debug)]
pub struct Project {
    pub name: String,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Scene file path relative to the assets folder.
    pub fn scene_path(&self) -> String {
        format!("scenes/{}.scene.ron", self.name)
    }

    /// Workspace-relative directory captures and screenshots are written to.
    pub fn shots_dir(&self) -> PathBuf {
        PathBuf::from("shots").join(&self.name)
    }
}

/// Request to open a different project (or re-read the current one from disk).
#[derive(Message)]
pub struct LoadProject(pub String);

/// Human-readable outcome of the last project load, for the UI status line.
#[derive(Resource, Default)]
pub struct ProjectStatus(pub Option<String>);

/// All project names, discovered from `assets/scenes/*.scene.ron`.
pub fn discover_projects() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir("assets/scenes")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            name.strip_suffix(".scene.ron").map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

/// Camera preset used right after a project loads: the scene's first preset,
/// or a pose scaled from the model height.
pub fn initial_preset(scene: &SceneConfig) -> CameraPreset {
    scene.cameras.first().cloned().unwrap_or_else(|| {
        let h = scene.model.target_height;
        CameraPreset {
            name: "default".to_string(),
            pos: [-1.3 * h, 0.65 * h, 1.9 * h],
            look_at: [0.0, 0.5 * h, 0.0],
            fov_deg: 45.0,
            track_rocket: false,
            follow_pos: false,
        }
    })
}

pub struct ProjectPlugin;

impl Plugin for ProjectPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<LoadProject>()
            .init_resource::<ProjectStatus>()
            .add_systems(Update, handle_load_project);
    }
}

/// Swaps `SceneConfig`/`Project`, tears down the world, and resets sim state.
/// The reactive spawn systems (rocket, environment, emitters) rebuild from the
/// new scene over the following frames.
#[allow(clippy::too_many_arguments)]
fn handle_load_project(
    mut events: MessageReader<LoadProject>,
    mut commands: Commands,
    mut scene: ResMut<SceneConfig>,
    mut project: ResMut<Project>,
    mut status: ResMut<ProjectStatus>,
    mut clock: ResMut<SimClock>,
    mut bounds: ResMut<RocketBounds>,
    mut emitters_ready: ResMut<crate::effects::EmittersReady>,
    mut earth_ready: ResMut<EarthReady>,
    rockets: Query<Entity, With<RocketRoot>>,
    emitters: Query<Entity, With<Emitter>>,
    environment: Query<Entity, With<EnvironmentEntity>>,
    mut camera: Query<(&mut PanOrbitCamera, &mut Projection), With<MainCamera>>,
) {
    let Some(LoadProject(name)) = events.read().last() else {
        return;
    };
    let next = Project::new(name.clone());
    let loaded = match crate::scene::load_scene_config(&next.scene_path()) {
        Ok(config) => config,
        Err(e) => {
            error!("failed to load project '{name}': {e}");
            status.0 = Some(format!("load '{name}' failed: {e}"));
            return;
        }
    };

    for entity in rockets.iter().chain(&emitters).chain(&environment) {
        commands.entity(entity).despawn();
    }
    *bounds = RocketBounds::default();
    *emitters_ready = crate::effects::EmittersReady(false);
    *earth_ready = EarthReady(false);
    *clock = SimClock {
        t: 0.0,
        playing: false,
        speed: 1.0,
    };
    *scene = loaded;
    *project = next;

    if let Ok((mut orbit, mut projection)) = camera.single_mut() {
        let preset = initial_preset(&scene);
        let (rocket_pos, _) = scene.flight.sample(0.0);
        snap_orbit_to_preset(&mut orbit, &mut projection, &preset, rocket_pos, &scene);
    }
    status.0 = Some(format!("opened project '{name}'"));
    info!("opened project '{name}'");
}
