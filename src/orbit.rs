//! Orbital day/night: rotate the inertial sky, spin Earth, drive star
//! intensity / city `sun_dir` / exposure from sim time. Ascent scenes also
//! retune atmosphere and pad visibility from radial altitude.

use bevy::camera::{ClearColorConfig, Exposure};
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, DirectionalLight, GlobalAmbientLight, Skybox};
use bevy::pbr::AtmosphereSettings;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy_hanabi::EffectProperties;
use std::f32::consts::TAU;

use crate::app::SimClock;
use crate::effects::{Emitter, INTENSITY_PROPERTY, SUN_DIR_PROPERTY, VIEW_POS_PROPERTY};
use crate::render::{
    atmosphere_settings_mode, EarthCloudsMaterial, EarthGlobeMaterial, EarthRoot, Earthshine,
    MainCamera, NightGlobeFill, OrbitalAtmosphere, PadDisc, SceneSun, SkyRoot,
};
use crate::rocket::RocketRoot;
use crate::scene::SceneConfig;

/// Night-peak skybox brightness (cd/m²). Zero at noon via `star_visibility`.
const SKYBOX_NIGHT_BRIGHTNESS: f32 = 1000.0;
/// Globe city-light emissive. Noon is black via `star_visibility`.
const EARTH_EMISSIVE_NIGHT: f32 = 120.0;
/// Cloud opacity at midnight (`nightglow_visibility` = 1). Full through dusk.
const CLOUD_NIGHT_ALPHA: f32 = 0.05;
const SPACE_VIS_START_M: f32 = 20_000.0;
const SPACE_VIS_SPAN_M: f32 = 60_000.0;
const ATMO_DENSITY_PAD: f32 = 1.0;
const ATMO_DENSITY_LEO: f32 = 0.16;
const PAD_FADE_START_M: f32 = 5_000.0;
const PAD_FADE_SPAN_M: f32 = 3_000.0;
const DENSITY_STEP: f32 = 0.04;

#[derive(Resource, Default)]
struct AscentTune {
    last_density: Option<f32>,
    last_raymarched: Option<bool>,
}

pub struct OrbitPlugin;

impl Plugin for OrbitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AscentTune>()
            .add_systems(Update, (reset_ascent_tune, apply_orbit_transforms))
            .add_systems(
                PostUpdate,
                (apply_orbit_properties, tune_ascent_environment)
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

/// 0 below 20 km, 1 above 80 km. Keeps stars off a noon pad.
pub fn space_visibility(altitude_m: f32) -> f32 {
    ((altitude_m - SPACE_VIS_START_M) / SPACE_VIS_SPAN_M).clamp(0.0, 1.0)
}

/// Full column on the pad, satellite's 0.16 once the limb is a disc.
pub fn atmosphere_density(altitude_m: f32) -> f32 {
    ATMO_DENSITY_PAD + (ATMO_DENSITY_LEO - ATMO_DENSITY_PAD) * space_visibility(altitude_m)
}

pub fn radial_up(rocket_pos: Vec3, earth_center: Vec3) -> Vec3 {
    (rocket_pos - earth_center).normalize_or(Vec3::Y)
}

pub fn rocket_altitude(rocket_pos: Vec3, earth_center: Vec3, radius_m: f32) -> f32 {
    (rocket_pos - earth_center).length() - radius_m
}

pub fn pad_disc_visibility(altitude_m: f32) -> f32 {
    1.0 - ((altitude_m - PAD_FADE_START_M) / PAD_FADE_SPAN_M).clamp(0.0, 1.0)
}

fn reset_ascent_tune(scene: Res<SceneConfig>, mut tune: ResMut<AscentTune>) {
    if scene.is_changed() {
        *tune = AscentTune::default();
    }
}

fn orbit_phase(clock: &SimClock, period: f32, start: f32) -> f32 {
    if period <= 1e-3 {
        return 0.0;
    }
    let t = clock.t - start;
    if t <= 0.0 {
        return 0.0;
    }
    (t / period).rem_euclid(1.0)
}

fn viewer_frame(
    scene: &SceneConfig,
    rocket: Option<&GlobalTransform>,
    earth: Option<&GlobalTransform>,
) -> (Vec3, f32) {
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
    let pos = rocket.map(|gt| gt.translation()).unwrap_or(Vec3::ZERO);
    (radial_up(pos, center), rocket_altitude(pos, center, radius))
}

fn apply_orbit_transforms(
    clock: Res<SimClock>,
    scene: Res<SceneConfig>,
    mut sky: Query<&mut Transform, (With<SkyRoot>, Without<EarthRoot>, Without<Earthshine>)>,
    mut earth: Query<&mut Transform, (With<EarthRoot>, Without<SkyRoot>, Without<Earthshine>)>,
) {
    let env = &scene.environment;
    let phase = orbit_phase(&clock, env.orbit_period_s, env.orbit_start_s);
    if let Ok(mut transform) = sky.single_mut() {
        transform.rotation = Quat::from_axis_angle(Vec3::X, phase * TAU);
    }
    if let Ok(mut transform) = earth.single_mut() {
        let spin = Quat::from_axis_angle(Vec3::Y, (env.earth_spin_deg_per_orbit * phase).to_radians());
        if let Some(cfg) = &env.earth {
            transform.translation = cfg.center();
            transform.rotation = cfg.orient() * spin;
        } else {
            transform.rotation = spin;
        }
    }
}

fn apply_orbit_properties(
    mut commands: Commands,
    scene: Res<SceneConfig>,
    earth_gt: Query<&GlobalTransform, With<EarthRoot>>,
    rocket_gt: Query<&GlobalTransform, With<RocketRoot>>,
    sun: Query<&GlobalTransform, With<SceneSun>>,
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
    let env = &scene.environment;
    if env.earth.is_none() && env.orbit_period_s <= 1e-3 {
        return;
    }

    let (up, altitude) = viewer_frame(&scene, rocket_gt.single().ok(), earth_gt.single().ok());
    let space_vis = space_visibility(altitude);
    let to_sun_world = sun
        .single()
        .map(|gt| gt.rotation() * Vec3::Z)
        .unwrap_or(up);
    let elevation = sun_elevation(to_sun_world, up);
    let star_vis = star_visibility(elevation) * space_vis;
    let nightglow_vis = nightglow_visibility(elevation) * space_vis;
    let night_fill = (1.0 - elevation.max(0.0)) * space_vis;
    let earth_rot = earth_gt
        .single()
        .map(|gt| gt.rotation())
        .unwrap_or(Quat::IDENTITY);
    let to_sun_earth = earth_rot.inverse() * to_sun_world;

    if capturing.is_none()
        && let Some(night_ev) = env.night_exposure_ev100
        && let Ok(mut exposure) = camera.single_mut()
    {
        let day_w = ((elevation + 0.2) / 0.6).clamp(0.0, 1.0);
        exposure.ev100 = night_ev + (env.exposure_ev100 - night_ev) * day_w;
    }

    if let Ok((mut transform, mut light)) = earthshine.single_mut() {
        transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, up);
        light.illuminance = env.earthshine_illuminance * night_fill;
    }
    if let Ok((mut transform, mut light)) = globe_fill.single_mut() {
        transform.rotation = Quat::from_rotation_arc(Vec3::Z, up);
        light.illuminance = env.night_globe_illuminance * night_fill;
    }

    if let Ok(mut skybox) = skybox.single_mut() {
        skybox.brightness = SKYBOX_NIGHT_BRIGHTNESS * star_vis;
        if let Ok(sky) = sky_root.single() {
            skybox.rotation = sky.rotation;
        }
    }

    let emissive = LinearRgba::rgb(
        EARTH_EMISSIVE_NIGHT * star_vis,
        EARTH_EMISSIVE_NIGHT * star_vis,
        EARTH_EMISSIVE_NIGHT * star_vis,
    );
    for handle in &globe_mats {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.emissive = emissive;
        }
    }

    let cloud_alpha = 1.0 - (1.0 - CLOUD_NIGHT_ALPHA) * nightglow_vis;
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
                &[(INTENSITY_PROPERTY, star_vis.into())],
            ),
            "earth" => {
                let vis = if config.name.starts_with("airglow") {
                    nightglow_vis
                } else {
                    star_vis
                };
                set_properties(
                    &mut commands,
                    entity,
                    properties,
                    &[
                        (INTENSITY_PROPERTY, vis.into()),
                        (SUN_DIR_PROPERTY, to_sun_earth.into()),
                        (VIEW_POS_PROPERTY, view_pos.into()),
                    ],
                );
            }
            _ => {}
        }
    }
}

fn tune_ascent_environment(
    scene: Res<SceneConfig>,
    rocket_gt: Query<&GlobalTransform, With<RocketRoot>>,
    earth_gt: Query<&GlobalTransform, With<EarthRoot>>,
    atmospheres: Query<&Atmosphere, With<OrbitalAtmosphere>>,
    mut media: ResMut<Assets<ScatteringMedium>>,
    mut camera: Query<(&mut Camera, &mut AtmosphereSettings), With<MainCamera>>,
    mut pad: Query<(&MeshMaterial3d<StandardMaterial>, &mut Visibility), With<PadDisc>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut tune: ResMut<AscentTune>,
) {
    let env = &scene.environment;
    if env.earth.is_none() {
        return;
    }
    let (_, altitude) = viewer_frame(&scene, rocket_gt.single().ok(), earth_gt.single().ok());
    let space_vis = space_visibility(altitude);
    ambient.brightness = env.ambient_brightness * (1.0 - space_vis);

    let raymarched = altitude >= SPACE_VIS_START_M;
    if tune.last_raymarched != Some(raymarched) {
        if let Ok((mut cam, mut settings)) = camera.single_mut() {
            *settings = atmosphere_settings_mode(raymarched);
            cam.clear_color = if env.atmosphere && !raymarched {
                ClearColorConfig::Default
            } else {
                ClearColorConfig::Custom(Color::BLACK)
            };
        }
        tune.last_raymarched = Some(raymarched);
    }

    let density = {
        let raw = atmosphere_density(altitude);
        (raw / DENSITY_STEP).round() * DENSITY_STEP
    };
    if tune.last_density.map(|d| (d - density).abs() > 1e-4) != Some(false) {
        if let Ok(atmosphere) = atmospheres.single()
            && let Some(mut medium) = media.get_mut(&atmosphere.medium)
        {
            *medium = ScatteringMedium::earth(256, 256).with_density_multiplier(density);
        }
        tune.last_density = Some(density);
    }

    let fade = pad_disc_visibility(altitude);
    for (handle, mut visibility) in &mut pad {
        *visibility = if fade <= 0.02 {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if let Some(mut material) = materials.get_mut(&handle.0) {
            let c = env.ground_color;
            material.base_color = Color::srgba(c[0], c[1], c[2], fade);
            material.alpha_mode = if fade >= 0.99 {
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
    fn pad_disc_fades_between_five_and_eight_km() {
        assert_eq!(pad_disc_visibility(0.0), 1.0);
        assert_eq!(pad_disc_visibility(5_000.0), 1.0);
        assert!((pad_disc_visibility(6_500.0) - 0.5).abs() < 1e-5);
        assert_eq!(pad_disc_visibility(8_000.0), 0.0);
    }

    #[test]
    fn radial_up_is_plus_y_on_the_pad() {
        let center = Vec3::new(0.0, -6_378_140.0, 0.0);
        let up = radial_up(Vec3::ZERO, center);
        assert!((up - Vec3::Y).length() < 1e-5);
        assert!((rocket_altitude(Vec3::ZERO, center, 6_378_140.0)).abs() < 1.0);
    }

    #[test]
    fn sun_elevation_noon_and_midnight() {
        assert!((sun_elevation(Vec3::Y, Vec3::Y) - 1.0).abs() < 1e-5);
        assert!((sun_elevation(Vec3::NEG_Y, Vec3::Y) + 1.0).abs() < 1e-5);
    }
}
