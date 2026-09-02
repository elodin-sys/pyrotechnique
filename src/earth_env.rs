//! Camera-driven Earth environment: one globe, continuous knobs from
//! camera altitude. Any scene with `EarthRoot` + `MainCamera` uses this.

use bevy::camera::Exposure;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, DirectionalLight, GlobalAmbientLight, Skybox};
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy_hanabi::EffectProperties;

use crate::effects::{Emitter, INTENSITY_PROPERTY, SUN_DIR_PROPERTY, VIEW_POS_PROPERTY};
use crate::render::{
    EarthCloudsMaterial, EarthGlobeMaterial, EarthRoot, Earthshine, MainCamera, NightGlobeFill,
    OrbitalAtmosphere, PadDisc, SceneSun, SkyRoot,
};
use crate::rocket::RocketRoot;
use crate::scene::SceneConfig;

/// The Milky Way master is a dim exposure (mean 3/255, dust p99 37/255), so
/// this gain — not the texture — sets how much band survives tonemapping.
const SKYBOX_NIGHT_BRIGHTNESS: f32 = 4000.0;
const EARTH_EMISSIVE_NIGHT: f32 = 120.0;
const CLOUD_NIGHT_ALPHA: f32 = 0.05;
const SPACE_VIS_START_M: f32 = 20_000.0;
const SPACE_VIS_SPAN_M: f32 = 60_000.0;
const ATMO_DENSITY_PAD: f32 = 1.0;
const ATMO_DENSITY_LEO: f32 = 0.16;
const PAD_FADE_START_M: f32 = 3_000.0;
const PAD_FADE_SPAN_M: f32 = 5_000.0;
const DENSITY_STEP: f32 = 0.01;
const PAD_HIDE_ALPHA: f32 = 0.005;

#[derive(Resource, Default)]
struct DensityTune {
    last_density: Option<f32>,
}

/// Camera-relative Earth frame, computed once per tick.
#[derive(Resource, Clone, Debug)]
pub struct ViewerFrame {
    pub active: bool,
    /// Camera radial; reserved for consumers / Elodin port.
    #[allow(dead_code)]
    pub up: Vec3,
    pub altitude_m: f32,
    pub space_vis: f32,
    pub star_vis: f32,
    pub nightglow_vis: f32,
    pub sun_elevation: f32,
    #[allow(dead_code)]
    pub to_sun_world: Vec3,
    pub to_sun_earth: Vec3,
    pub craft_up: Vec3,
}

impl Default for ViewerFrame {
    fn default() -> Self {
        Self {
            active: false,
            up: Vec3::Y,
            altitude_m: 0.0,
            space_vis: 0.0,
            star_vis: 0.0,
            nightglow_vis: 0.0,
            sun_elevation: 1.0,
            to_sun_world: Vec3::Y,
            to_sun_earth: Vec3::Y,
            craft_up: Vec3::Y,
        }
    }
}

pub struct EarthEnvPlugin;

impl Plugin for EarthEnvPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewerFrame>()
            .init_resource::<DensityTune>()
            .add_systems(Update, reset_density_tune)
            .add_systems(
                PostUpdate,
                (
                    compute_viewer_frame,
                    (apply_earth_lighting, tune_earth_environment)
                        .after(compute_viewer_frame),
                )
                    .after(TransformSystems::Propagate),
            );
    }
}

/// Sun elevation vs `up`: 1 = noon, −1 = midnight.
pub fn sun_elevation(to_sun: Vec3, up: Vec3) -> f32 {
    let up = up.normalize_or(Vec3::Y);
    to_sun.normalize_or(up).dot(up)
}

/// Star visibility: 0 in daylight, 1 in eclipse, smooth through terminator.
pub fn star_visibility(elevation: f32) -> f32 {
    ((0.25 - elevation) / 0.35).clamp(0.0, 1.0)
}

/// Nightglow: stays off through dusk so Rayleigh fire is not painted green.
pub fn nightglow_visibility(elevation: f32) -> f32 {
    ((-0.05 - elevation) / 0.3).clamp(0.0, 1.0)
}

/// 0 below 20 km, 1 above 80 km.
pub fn space_visibility(altitude_m: f32) -> f32 {
    ((altitude_m - SPACE_VIS_START_M) / SPACE_VIS_SPAN_M).clamp(0.0, 1.0)
}

/// Full column on the pad, 0.16 once the limb is a disc.
pub fn atmosphere_density(altitude_m: f32) -> f32 {
    ATMO_DENSITY_PAD + (ATMO_DENSITY_LEO - ATMO_DENSITY_PAD) * space_visibility(altitude_m)
}

pub fn quantize_density(altitude_m: f32) -> f32 {
    let raw = atmosphere_density(altitude_m);
    (raw / DENSITY_STEP).round() * DENSITY_STEP
}

pub fn radial_up(pos: Vec3, earth_center: Vec3) -> Vec3 {
    (pos - earth_center).normalize_or(Vec3::Y)
}

pub fn altitude_above(pos: Vec3, earth_center: Vec3, radius_m: f32) -> f32 {
    (pos - earth_center).length() - radius_m
}

/// Camera first, then rocket, then origin.
pub fn viewer_position(camera: Option<Vec3>, rocket: Option<Vec3>) -> Vec3 {
    camera.or(rocket).unwrap_or(Vec3::ZERO)
}

/// 1 below 3 km, 0 above 8 km.
pub fn pad_disc_visibility(altitude_m: f32) -> f32 {
    1.0 - ((altitude_m - PAD_FADE_START_M) / PAD_FADE_SPAN_M).clamp(0.0, 1.0)
}

fn reset_density_tune(scene: Res<SceneConfig>, mut tune: ResMut<DensityTune>) {
    if scene.is_changed() {
        *tune = DensityTune::default();
    }
}

fn earth_center_radius(scene: &SceneConfig, earth: Option<&GlobalTransform>) -> (Vec3, f32) {
    let radius = scene
        .environment
        .earth
        .as_ref()
        .map(|cfg| cfg.radius_m)
        .unwrap_or(6_378_140.0);
    let center = earth
        .map(|gt| gt.translation())
        .or_else(|| scene.environment.earth.as_ref().map(|cfg| cfg.center()))
        .unwrap_or(Vec3::new(0.0, -radius, 0.0));
    (center, radius)
}

fn compute_viewer_frame(
    scene: Res<SceneConfig>,
    camera_gt: Query<&GlobalTransform, With<MainCamera>>,
    rocket_gt: Query<&GlobalTransform, With<RocketRoot>>,
    earth_gt: Query<&GlobalTransform, With<EarthRoot>>,
    sun: Query<&GlobalTransform, With<SceneSun>>,
    mut frame: ResMut<ViewerFrame>,
) {
    let env = &scene.environment;
    if env.earth.is_none() && env.orbit_period_s <= 1e-3 {
        *frame = ViewerFrame::default();
        return;
    }

    let earth = earth_gt.single().ok();
    let rocket = rocket_gt.single().ok();
    let camera = camera_gt.single().ok();
    let (center, radius) = earth_center_radius(&scene, earth);
    let cam_pos = viewer_position(
        camera.map(|gt| gt.translation()),
        rocket.map(|gt| gt.translation()),
    );
    let craft_pos = rocket.map(|gt| gt.translation()).unwrap_or(cam_pos);
    let up = radial_up(cam_pos, center);
    let craft_up = radial_up(craft_pos, center);
    let altitude_m = altitude_above(cam_pos, center, radius);
    let space_vis = space_visibility(altitude_m);
    let to_sun_world = sun
        .single()
        .map(|gt| gt.rotation() * Vec3::Z)
        .unwrap_or(up);
    let elevation = sun_elevation(to_sun_world, up);
    let earth_rot = earth.map(|gt| gt.rotation()).unwrap_or(Quat::IDENTITY);

    *frame = ViewerFrame {
        active: true,
        up,
        altitude_m,
        space_vis,
        star_vis: star_visibility(elevation) * space_vis,
        nightglow_vis: nightglow_visibility(elevation) * space_vis,
        sun_elevation: elevation,
        to_sun_world,
        to_sun_earth: earth_rot.inverse() * to_sun_world,
        craft_up,
    };
}

fn apply_earth_lighting(
    mut commands: Commands,
    scene: Res<SceneConfig>,
    frame: Res<ViewerFrame>,
    earth_gt: Query<&GlobalTransform, With<EarthRoot>>,
    camera_gt: Query<&GlobalTransform, With<MainCamera>>,
    mut earthshine: Query<
        (&mut Transform, &mut DirectionalLight),
        (
            With<Earthshine>,
            Without<NightGlobeFill>,
            Without<SkyRoot>,
            Without<EarthRoot>,
        ),
    >,
    mut globe_fill: Query<
        (&mut Transform, &mut DirectionalLight),
        (
            With<NightGlobeFill>,
            Without<Earthshine>,
            Without<SkyRoot>,
            Without<EarthRoot>,
        ),
    >,
    mut camera: Query<&mut Exposure, With<MainCamera>>,
    mut skybox: Query<&mut Skybox, With<MainCamera>>,
    sky_root: Query<
        &Transform,
        (
            With<SkyRoot>,
            Without<EarthRoot>,
            Without<Earthshine>,
            Without<NightGlobeFill>,
        ),
    >,
    globe_mats: Query<&MeshMaterial3d<StandardMaterial>, With<EarthGlobeMaterial>>,
    cloud_mats: Query<&MeshMaterial3d<StandardMaterial>, With<EarthCloudsMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    emitters: Query<(Entity, &Emitter, Option<&mut EffectProperties>)>,
    capturing: Option<Res<crate::capture::CaptureConfig>>,
) {
    if !frame.active {
        return;
    }
    let env = &scene.environment;
    let night_fill = (1.0 - frame.sun_elevation.max(0.0)) * frame.space_vis;

    if capturing.is_none()
        && let Some(night_ev) = env.night_exposure_ev100
        && let Ok(mut exposure) = camera.single_mut()
    {
        let day_w = ((frame.sun_elevation + 0.2) / 0.6).clamp(0.0, 1.0);
        exposure.ev100 = night_ev + (env.exposure_ev100 - night_ev) * day_w;
    }

    if let Ok((mut transform, mut light)) = earthshine.single_mut() {
        transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, frame.craft_up);
        light.illuminance = env.earthshine_illuminance * night_fill;
    }
    if let Ok((mut transform, mut light)) = globe_fill.single_mut() {
        transform.rotation = Quat::from_rotation_arc(Vec3::Z, frame.craft_up);
        light.illuminance = env.night_globe_illuminance * night_fill;
    }

    if let Ok(mut skybox) = skybox.single_mut() {
        skybox.brightness = SKYBOX_NIGHT_BRIGHTNESS * frame.star_vis;
        if let Ok(sky) = sky_root.single() {
            skybox.rotation = sky.rotation;
        }
    }

    let emissive = LinearRgba::rgb(
        EARTH_EMISSIVE_NIGHT * frame.star_vis,
        EARTH_EMISSIVE_NIGHT * frame.star_vis,
        EARTH_EMISSIVE_NIGHT * frame.star_vis,
    );
    for handle in &globe_mats {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.emissive = emissive;
        }
    }

    let cloud_alpha = 1.0 - (1.0 - CLOUD_NIGHT_ALPHA) * frame.nightglow_vis;
    for handle in &cloud_mats {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.base_color = material.base_color.with_alpha(cloud_alpha);
        }
    }

    let view_pos = match (camera_gt.single(), earth_gt.single()) {
        (Ok(cam), Ok(earth)) => earth
            .affine()
            .inverse()
            .transform_point3(cam.translation()),
        _ => Vec3::Y * 6_778_140.0,
    };

    for (entity, emitter, properties) in emitters {
        let Some(config) = scene.emitters.get(emitter.index) else {
            continue;
        };
        match config.attach.as_str() {
            "sky" => set_properties(
                &mut commands,
                entity,
                properties,
                &[(INTENSITY_PROPERTY, frame.star_vis.into())],
            ),
            "earth" => {
                let vis = if config.name.starts_with("airglow") {
                    frame.nightglow_vis
                } else {
                    frame.star_vis
                };
                set_properties(
                    &mut commands,
                    entity,
                    properties,
                    &[
                        (INTENSITY_PROPERTY, vis.into()),
                        (SUN_DIR_PROPERTY, frame.to_sun_earth.into()),
                        (VIEW_POS_PROPERTY, view_pos.into()),
                    ],
                );
            }
            _ => {}
        }
    }
}

fn tune_earth_environment(
    scene: Res<SceneConfig>,
    frame: Res<ViewerFrame>,
    atmospheres: Query<&Atmosphere, With<OrbitalAtmosphere>>,
    mut media: ResMut<Assets<ScatteringMedium>>,
    mut pad: Query<(&MeshMaterial3d<StandardMaterial>, &mut Visibility), With<PadDisc>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut tune: ResMut<DensityTune>,
) {
    if !frame.active || scene.environment.earth.is_none() {
        return;
    }
    let env = &scene.environment;
    ambient.brightness = env.ambient_brightness * (1.0 - frame.space_vis);

    let density = quantize_density(frame.altitude_m);
    if tune.last_density.map(|d| (d - density).abs() > 1e-4) != Some(false) {
        if let Ok(atmosphere) = atmospheres.single()
            && let Some(mut medium) = media.get_mut(&atmosphere.medium)
        {
            *medium = ScatteringMedium::earth(256, 256).with_density_multiplier(density);
        }
        tune.last_density = Some(density);
    }

    let fade = pad_disc_visibility(frame.altitude_m);
    for (handle, mut visibility) in &mut pad {
        *visibility = if fade <= PAD_HIDE_ALPHA {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if let Some(mut material) = materials.get_mut(&handle.0) {
            let c = env.ground_color;
            material.base_color = Color::srgba(c[0], c[1], c[2], fade);
            material.alpha_mode = if fade >= 1.0 {
                AlphaMode::Opaque
            } else {
                AlphaMode::Blend
            };
        }
    }
}

fn set_properties(
    commands: &mut Commands,
    entity: Entity,
    properties: Option<Mut<EffectProperties>>,
    pairs: &[(&str, bevy_hanabi::graph::Value)],
) {
    if let Some(mut properties) = properties {
        for (name, value) in pairs {
            properties.set(name, value.clone());
        }
    } else {
        let mut instance = EffectProperties::default();
        for (name, value) in pairs {
            instance.set(name, value.clone());
        }
        commands.entity(entity).insert(instance);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_visibility_gates_the_pad() {
        assert_eq!(space_visibility(0.0), 0.0);
        assert_eq!(space_visibility(20_000.0), 0.0);
        assert!((space_visibility(50_000.0) - 0.5).abs() < 1e-5);
        assert_eq!(space_visibility(80_000.0), 1.0);
        assert_eq!(space_visibility(400_000.0), 1.0);
    }

    #[test]
    fn atmosphere_density_follows_altitude() {
        assert!((atmosphere_density(0.0) - 1.0).abs() < 1e-5);
        assert!((atmosphere_density(20_000.0) - 1.0).abs() < 1e-5);
        assert!((atmosphere_density(80_000.0) - 0.16).abs() < 1e-5);
        assert!((atmosphere_density(400_000.0) - 0.16).abs() < 1e-5);
    }

    #[test]
    fn density_quantizes_to_one_percent() {
        let q = quantize_density(50_000.0);
        assert!((q * 100.0 - (q * 100.0).round()).abs() < 1e-4);
        assert!((q - 0.58).abs() < 1e-5);
    }

    #[test]
    fn pad_disc_fades_between_three_and_eight_km() {
        assert_eq!(pad_disc_visibility(0.0), 1.0);
        assert_eq!(pad_disc_visibility(3_000.0), 1.0);
        assert!((pad_disc_visibility(5_500.0) - 0.5).abs() < 1e-5);
        assert_eq!(pad_disc_visibility(8_000.0), 0.0);
    }

    #[test]
    fn viewer_position_prefers_camera() {
        assert_eq!(
            viewer_position(Some(Vec3::Y * 10.0), Some(Vec3::X)),
            Vec3::Y * 10.0
        );
        assert_eq!(viewer_position(None, Some(Vec3::X)), Vec3::X);
        assert_eq!(viewer_position(None, None), Vec3::ZERO);
    }

    #[test]
    fn radial_up_is_plus_y_on_the_pad() {
        let center = Vec3::new(0.0, -6_378_140.0, 0.0);
        let up = radial_up(Vec3::ZERO, center);
        assert!((up - Vec3::Y).length() < 1e-5);
        assert!(altitude_above(Vec3::ZERO, center, 6_378_140.0).abs() < 1.0);
    }

    #[test]
    fn sun_elevation_noon_and_midnight() {
        assert!((sun_elevation(Vec3::Y, Vec3::Y) - 1.0).abs() < 1e-5);
        assert!((sun_elevation(Vec3::NEG_Y, Vec3::Y) + 1.0).abs() < 1e-5);
    }
}
