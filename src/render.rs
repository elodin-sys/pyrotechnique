//! Environment: HDR camera, procedural atmosphere, sun, ground.

use bevy::camera::{Exposure, Hdr};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, SunDisk};
use bevy::pbr::AtmosphereSettings;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCamera;

use crate::scene::SceneConfig;

/// Marker for the single main viewport camera.
#[derive(Component)]
pub struct MainCamera;

pub struct EnvironmentPlugin;

impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_environment);
    }
}

fn setup_environment(
    mut commands: Commands,
    scene: Res<SceneConfig>,
    mut media: ResMut<Assets<ScatteringMedium>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let env = &scene.environment;

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
        Transform::from_xyz(-90.0, 45.0, 130.0).looking_at(Vec3::new(0.0, 35.0, 0.0), Vec3::Y),
        PanOrbitCamera {
            focus: Vec3::new(0.0, 35.0, 0.0),
            ..default()
        },
    ));

    // Procedural earth atmosphere (sky + aerial perspective).
    let medium = media.add(ScatteringMedium::earth(256, 256));
    commands.spawn((Name::new("atmosphere"), Atmosphere::earth(medium)));

    // Sun.
    let azimuth = env.sun_azimuth_deg.to_radians();
    let elevation = env.sun_elevation_deg.to_radians();
    let sun_rotation = Quat::from_euler(EulerRot::YXZ, -azimuth, -elevation, 0.0);
    commands.spawn((
        Name::new("sun"),
        DirectionalLight {
            illuminance: env.sun_illuminance,
            shadow_maps_enabled: true,
            ..default()
        },
        SunDisk::EARTH,
        Transform::from_rotation(sun_rotation),
    ));

    // Ground disc.
    let c = env.ground_color;
    commands.spawn((
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
}
