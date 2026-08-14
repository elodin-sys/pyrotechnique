//! Orbital day/night: rotate the inertial sky and spin Earth from sim time.
//! View-dependent Earth knobs live in [`crate::earth_env`].

use bevy::prelude::*;
use std::f32::consts::TAU;

use crate::app::SimClock;
use crate::render::{EarthRoot, Earthshine, SkyRoot};
use crate::scene::SceneConfig;

pub struct OrbitPlugin;

impl Plugin for OrbitPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, apply_orbit_transforms);
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
