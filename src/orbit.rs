//! Orbital day/night: rotate the inertial sky, spin Earth, drive star
//! intensity / city `sun_dir` / exposure from sim time.

use bevy::camera::Exposure;
use bevy::light::DirectionalLight;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy_hanabi::EffectProperties;
use std::f32::consts::TAU;

use crate::app::SimClock;
use crate::effects::{Emitter, INTENSITY_PROPERTY, SUN_DIR_PROPERTY, VIEW_POS_PROPERTY};
use crate::render::{EarthRoot, Earthshine, MainCamera, NightGlobeFill, SceneSun, SkyRoot};
use crate::scene::SceneConfig;

pub struct OrbitPlugin;

impl Plugin for OrbitPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, apply_orbit_transforms)
            .add_systems(
                PostUpdate,
                apply_orbit_properties.after(TransformSystems::Propagate),
            );
    }
}

/// Sun elevation vs nadir: 1 = noon (sun over the craft), −1 = midnight.
pub fn sun_elevation(to_sun: Vec3) -> f32 {
    to_sun.normalize_or(Vec3::Y).dot(Vec3::Y)
}

/// Star visibility: 0 in daylight, 1 in eclipse, smooth through terminator.
pub fn star_visibility(elevation: f32) -> f32 {
    ((0.25 - elevation) / 0.35).clamp(0.0, 1.0)
}

/// Nightglow: stays off through dusk so Rayleigh fire is not painted green.
pub fn nightglow_visibility(elevation: f32) -> f32 {
    ((-0.05 - elevation) / 0.3).clamp(0.0, 1.0)
}

fn orbit_phase(clock: &SimClock, period: f32) -> f32 {
    if period > 1e-3 {
        (clock.t / period).rem_euclid(1.0)
    } else {
        0.0
    }
}

fn apply_orbit_transforms(
    clock: Res<SimClock>,
    scene: Res<SceneConfig>,
    mut sky: Query<&mut Transform, (With<SkyRoot>, Without<EarthRoot>, Without<Earthshine>)>,
    mut earth: Query<&mut Transform, (With<EarthRoot>, Without<SkyRoot>, Without<Earthshine>)>,
) {
    let env = &scene.environment;
    let phase = orbit_phase(&clock, env.orbit_period_s);
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
    sun: Query<&GlobalTransform, With<SceneSun>>,
    camera_gt: Query<&GlobalTransform, With<MainCamera>>,
    mut earthshine: Query<&mut DirectionalLight, (With<Earthshine>, Without<NightGlobeFill>)>,
    mut globe_fill: Query<&mut DirectionalLight, (With<NightGlobeFill>, Without<Earthshine>)>,
    mut camera: Query<&mut Exposure, With<MainCamera>>,
    emitters: Query<(Entity, &Emitter, Option<&mut EffectProperties>)>,
    capturing: Option<Res<crate::capture::CaptureConfig>>,
) {
    let env = &scene.environment;
    if env.orbit_period_s <= 1e-3 {
        return;
    }

    let to_sun_world = sun
        .single()
        .map(|gt| gt.rotation() * Vec3::Z)
        .unwrap_or(Vec3::Y);
    let elevation = sun_elevation(to_sun_world);
    let star_vis = star_visibility(elevation);
    let nightglow_vis = nightglow_visibility(elevation);
    let night_fill = 1.0 - elevation.max(0.0);
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

    if let Ok(mut light) = earthshine.single_mut() {
        light.illuminance = env.earthshine_illuminance * night_fill;
    }
    if let Ok(mut light) = globe_fill.single_mut() {
        light.illuminance = env.night_globe_illuminance * night_fill;
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
