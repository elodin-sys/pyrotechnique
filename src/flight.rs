//! Drives the rocket root transform along the scene's flight path.

use bevy::prelude::*;

use crate::app::SimClock;
use crate::rocket::RocketRoot;
use crate::scene::SceneConfig;

pub struct FlightPlugin;

impl Plugin for FlightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drive_flight);
    }
}

fn drive_flight(
    clock: Res<SimClock>,
    scene: Res<SceneConfig>,
    mut rocket: Query<&mut Transform, With<RocketRoot>>,
) {
    let Ok(mut transform) = rocket.single_mut() else {
        return;
    };
    let (pos, vel) = scene.flight.sample(clock.t);
    transform.translation = pos;
    let dir = vel.normalize_or(Vec3::Y);
    transform.rotation = Quat::from_rotation_arc(Vec3::Y, dir);
}
