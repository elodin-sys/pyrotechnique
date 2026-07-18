//! Environment: HDR camera, procedural atmosphere, sun, ground.
//!
//! The camera is spawned once and persists across project switches; the
//! sun/atmosphere/ground entities are tagged [`EnvironmentEntity`] and are
//! respawned from the current `SceneConfig` whenever they are missing (first
//! boot, or after `LoadProject` despawned them).

use bevy::camera::{ClearColorConfig, Exposure, Hdr};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, SunDisk};
use bevy::pbr::AtmosphereSettings;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCamera;

use crate::scene::{CameraPreset, SceneConfig};

/// Marker for the single main viewport camera.
#[derive(Component)]
pub struct MainCamera;

/// Marker for scene-owned environment entities (sun, atmosphere, ground)
/// that are torn down and rebuilt on project switch.
#[derive(Component)]
pub struct EnvironmentEntity;

pub struct EnvironmentPlugin;

impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_camera)
            .add_systems(Update, ensure_environment);
    }
}

/// Point an orbit camera at a preset pose (uses the smoothed targets so the
/// transition eases in edit mode).
pub fn snap_orbit_to_preset(
    orbit: &mut PanOrbitCamera,
    projection: &mut Projection,
    preset: &CameraPreset,
    rocket_pos: Vec3,
) {
    let (pos, look_at) = crate::capture::preset_pose(preset, rocket_pos);
    let offset = pos - look_at;
    let radius = offset.length().max(0.01);
    orbit.target_focus = look_at;
    orbit.target_radius = radius;
    orbit.target_yaw = f32::atan2(offset.x, offset.z);
    orbit.target_pitch = (offset.y / radius).clamp(-1.0, 1.0).asin();
    *projection = Projection::Perspective(PerspectiveProjection {
        fov: preset.fov_deg.to_radians(),
        ..default()
    });
}

fn setup_camera(mut commands: Commands, scene: Res<SceneConfig>) {
    let env = &scene.environment;
    let preset = crate::project::initial_preset(&scene);
    let (rocket_pos, _) = scene.flight.sample(0.0);
    let (pos, look_at) = crate::capture::preset_pose(&preset, rocket_pos);

    // Main HDR camera. Bloom is what makes HDR particle colors (>1.0) glow.
    commands.spawn((
        MainCamera,
        Camera3d::default(),
        Hdr,
        Tonemapping::TonyMcMapface,
        Exposure {
            ev100: env.exposure_ev100,
        },
        // Additive ("old school") bloom: only genuinely hot pixels halo,
        // which is what makes HDR flame cores read as blinding.
        Bloom {
            intensity: env.bloom_intensity,
            ..Bloom::OLD_SCHOOL
        },
        AtmosphereSettings::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: preset.fov_deg.to_radians(),
            ..default()
        }),
        Transform::from_translation(pos).looking_at(look_at, Vec3::Y),
        PanOrbitCamera {
            focus: look_at,
            ..default()
        },
    ));
}

/// (Re)spawns sun/atmosphere/ground from the current scene when missing, and
/// syncs the persistent camera's exposure/bloom/clear color to the scene.
fn ensure_environment(
    mut commands: Commands,
    scene: Res<SceneConfig>,
    mut media: ResMut<Assets<ScatteringMedium>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing: Query<(), With<EnvironmentEntity>>,
    mut camera: Query<(&mut Camera, &mut Exposure, &mut Bloom), With<MainCamera>>,
) {
    if !existing.is_empty() {
        return;
    }
    let env = &scene.environment;

    // Sun.
    let azimuth = env.sun_azimuth_deg.to_radians();
    let elevation = env.sun_elevation_deg.to_radians();
    let sun_rotation = Quat::from_euler(EulerRot::YXZ, -azimuth, -elevation, 0.0);
    commands.spawn((
        EnvironmentEntity,
        Name::new("sun"),
        DirectionalLight {
            illuminance: env.sun_illuminance,
            shadow_maps_enabled: true,
            ..default()
        },
        SunDisk::EARTH,
        Transform::from_rotation(sun_rotation),
    ));

    // Procedural earth atmosphere (sky + aerial perspective); skipped for
    // airless bodies, which fall back to a black clear color.
    if env.atmosphere {
        let medium = media.add(ScatteringMedium::earth(256, 256));
        commands.spawn((
            EnvironmentEntity,
            Name::new("atmosphere"),
            Atmosphere::earth(medium),
        ));
    }

    // Ground disc.
    let c = env.ground_color;
    commands.spawn((
        EnvironmentEntity,
        Name::new("ground"),
        Mesh3d(meshes.add(Circle::new(env.ground_radius))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(c[0], c[1], c[2]),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            ..default()
        })),
        Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
    ));

    // Sync camera env parameters (also covers project switches).
    if let Ok((mut cam, mut exposure, mut bloom)) = camera.single_mut() {
        exposure.ev100 = env.exposure_ev100;
        bloom.intensity = env.bloom_intensity;
        cam.clear_color = if env.atmosphere {
            ClearColorConfig::Default
        } else {
            ClearColorConfig::Custom(Color::BLACK)
        };
    }
}
