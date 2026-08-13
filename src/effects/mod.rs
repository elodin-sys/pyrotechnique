//! Emitters: turn scene `EmitterConfig`s into live Hanabi effect instances.
//!
//! Each emitter is a child entity of the rocket root, positioned/oriented in
//! the normalized rocket frame (exhaust axis = local -Y). Every frame the
//! emitter's spawn rate is the effect file's authored rate scaled by
//! `intensity x activity(t)`. `.effect` files hot-reload through the asset
//! server; on reload the instance is recompiled and its material rebound.

pub mod builders;
pub mod sphere_map;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy_hanabi::{
    CpuValue, EffectAsset, EffectMaterial, EffectProperties, EffectSpawner, ParticleEffect,
};

use crate::app::SimClock;
use crate::render::{EarthRoot, SkyRoot};
use crate::rocket::RocketRoot;
use crate::scene::{EmitterConfig, SceneConfig};

/// Below this effective multiplier the emitter is fully deactivated.
const ACTIVITY_CUTOFF: f32 = 1e-3;

/// Property names of the anchored-trail contract (see `builders::exhaust_smoke`):
/// an effect declaring these runs on a world-fixed anchor and receives the
/// live nozzle pose through them each frame.
pub const SPAWN_ORIGIN_PROPERTY: &str = "spawn_origin";
pub const SPAWN_AXIS_PROPERTY: &str = "spawn_axis";

/// Optional throttle property (1.0 = full throttle). Effects declaring it get
/// the live `intensity x activity(t)` signal every frame, next to the spawner
/// rate, and can wire it into speed/size/color expressions (plume length and
/// brightness, not just particle density).
pub const INTENSITY_PROPERTY: &str = "intensity";

/// Direction toward the sun in the effect's local frame. City lights / airglow
/// fade on the day side.
pub const SUN_DIR_PROPERTY: &str = "sun_dir";

/// Camera position in the effect's local frame. Airglow uses this to keep the
/// glow as a limb band instead of a face-on disc.
pub const VIEW_POS_PROPERTY: &str = "view_pos";

/// True after [`spawn_emitters`] has run for the current world (including a
/// zero-emitter scene). Cleared on project switch and Restart.
#[derive(Resource, Default)]
pub struct EmittersReady(pub bool);

/// Live emitter instance, child of the rocket root.
#[derive(Component)]
pub struct Emitter {
    pub name: String,
    /// Index into `SceneConfig::emitters`.
    pub index: usize,
}

/// Marker for emitters re-homed to a world-fixed anchor because their effect
/// declares the anchored-trail properties.
#[derive(Component)]
pub struct TrailAnchored;

/// Light child of an emitter entity; intensity follows the emitter's
/// `intensity x activity(t)` signal (see `apply_emitter_lights`).
#[derive(Component)]
pub struct EmitterLight {
    /// Index into `SceneConfig::emitters`.
    pub index: usize,
}

/// Show emitter gizmos (position + exhaust direction). Toggled from the UI.
#[derive(Resource)]
pub struct ShowEmitterGizmos(pub bool);

pub struct EmitterPlugin;

impl Plugin for EmitterPlugin {
    fn build(&self, app: &mut App) {
        {
            let registry = app.world_mut().resource::<AppTypeRegistry>().clone();
            sphere_map::register(&registry);
        }
        app.insert_resource(ShowEmitterGizmos(true))
            .init_resource::<EmittersReady>()
            .add_systems(
                Update,
                (
                    spawn_emitters,
                    bind_effect_materials,
                    recompile_on_asset_change,
                    anchor_trail_emitters,
                    apply_trail_properties,
                    apply_emitter_intensity,
                    apply_emitter_lights,
                    draw_emitter_gizmos,
                ),
            );
    }
}

/// Spawns emitter entities once the rocket root exists (and again after Restart
/// despawns them). If the effect asset is already loaded, bind its
/// `EffectMaterial` immediately — asset-load events won't fire a second time,
/// and Hanabi panics if a texture slot exists with no bound images.
fn spawn_emitters(
    mut commands: Commands,
    scene: Res<SceneConfig>,
    asset_server: Res<AssetServer>,
    effects: Res<Assets<EffectAsset>>,
    rocket: Query<Entity, With<RocketRoot>>,
    earth: Query<Entity, With<EarthRoot>>,
    sky: Query<Entity, With<SkyRoot>>,
    mut ready: ResMut<EmittersReady>,
) {
    if ready.0 {
        return;
    }
    let Ok(rocket) = rocket.single() else {
        return;
    };
    let needs_earth = scene.emitters.iter().any(|e| e.attach == "earth");
    let needs_sky = scene.emitters.iter().any(|e| e.attach == "sky");
    let earth = earth.single().ok();
    let sky = sky.single().ok();
    if needs_earth && earth.is_none() {
        return;
    }
    if needs_sky && sky.is_none() {
        return;
    }

    for (index, config) in scene.emitters.iter().enumerate() {
        let handle: Handle<EffectAsset> = asset_server.load(config.effect.clone());
        let material = effects.get(&handle).and_then(|asset| {
            if asset.texture_layout().layout.is_empty() {
                None
            } else {
                Some(effect_material_for(asset, &asset_server))
            }
        });
        let mut entity = commands.spawn((
            Name::new(format!("emitter:{}", config.name)),
            Emitter {
                name: config.name.clone(),
                index,
            },
            ParticleEffect::new(handle),
            emitter_transform(config),
            Visibility::default(),
        ));
        if let Some(material) = material {
            entity.insert(material);
        }
        if matches!(config.attach.as_str(), "earth" | "sky") {
            entity.insert(NoFrustumCulling);
        }
        let entity = entity.id();
        if let Some(light) = &config.light {
            let child = spawn_emitter_light(&mut commands, light, index);
            commands.entity(entity).add_child(child);
        }
        match config.attach.as_str() {
            "world" => {}
            "earth" => {
                if let Some(earth) = earth {
                    commands.entity(earth).add_child(entity);
                }
            }
            "sky" => {
                if let Some(sky) = sky {
                    commands.entity(sky).add_child(entity);
                }
            }
            _ => {
                commands.entity(rocket).add_child(entity);
            }
        }
    }
    ready.0 = true;
    info!("spawned {} emitters", scene.emitters.len());
}

/// Spawns the light child for an emitter. Spawned at zero intensity;
/// `apply_emitter_lights` drives it every frame. Bevy spot lights shine along
/// local -Z, so the child rotates -Z onto the emitter's -Y exhaust axis.
fn spawn_emitter_light(
    commands: &mut Commands,
    light: &crate::scene::LightConfig,
    index: usize,
) -> Entity {
    let color = Color::srgb(light.color[0], light.color[1], light.color[2]);
    let transform = Transform {
        // Exhaust is local -Y; offset drops the light below the bell exit.
        translation: Vec3::new(0.0, -light.offset_m, 0.0),
        rotation: Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::NEG_Y),
        scale: Vec3::ONE,
    };
    let mut entity = commands.spawn((EmitterLight { index }, transform, Visibility::default()));
    match light.spot_angle_deg {
        Some(angle) => {
            entity.insert(SpotLight {
                color,
                intensity: 0.0,
                range: light.range,
                shadow_maps_enabled: light.shadows,
                outer_angle: (angle.to_radians() * 0.5).clamp(0.0, std::f32::consts::FRAC_PI_2),
                inner_angle: 0.0,
                ..default()
            });
        }
        None => {
            entity.insert(PointLight {
                color,
                intensity: 0.0,
                range: light.range,
                shadow_maps_enabled: light.shadows,
                ..default()
            });
        }
    }
    entity.id()
}

pub fn emitter_transform(config: &EmitterConfig) -> Transform {
    let dir = Vec3::from(config.direction).normalize_or(Vec3::NEG_Y);
    Transform {
        translation: Vec3::from(config.position),
        rotation: Quat::from_rotation_arc(Vec3::NEG_Y, dir),
        scale: Vec3::ONE,
    }
}

/// Maps effect texture-slot names to generated sprite assets.
fn slot_image(name: &str, asset_server: &AssetServer) -> Handle<Image> {
    match name {
        "smoke" => asset_server.load("textures/smoke_puff.png"),
        "night" => asset_server.load("textures/earth_night.png"),
        // "mask" and anything unknown get the soft round falloff.
        _ => asset_server.load("textures/soft_circle.png"),
    }
}

fn effect_material_for(asset: &EffectAsset, asset_server: &AssetServer) -> EffectMaterial {
    let images = asset
        .texture_layout()
        .layout
        .iter()
        .map(|slot| {
            let handle = slot_image(&slot.name, asset_server);
            if slot.name == "night" {
                info!("bound night-lights texture for effect '{}'", asset.name);
            }
            handle
        })
        .collect();
    EffectMaterial { images }
}

/// (Re)binds `EffectMaterial` images from the asset's texture slots:
/// - on first asset load / hot reload (AssetEvent), and
/// - for any emitter that is missing a material once its asset is ready
///   (covers Restart, which respawns against already-loaded assets).
fn bind_effect_materials(
    mut commands: Commands,
    mut events: MessageReader<AssetEvent<EffectAsset>>,
    effects: Res<Assets<EffectAsset>>,
    asset_server: Res<AssetServer>,
    emitters: Query<(Entity, &ParticleEffect, Option<&EffectMaterial>), With<Emitter>>,
) {
    // Path 1: asset just loaded or was modified on disk.
    for event in events.read() {
        let (AssetEvent::LoadedWithDependencies { id } | AssetEvent::Modified { id }) = event
        else {
            continue;
        };
        let Some(asset) = effects.get(*id) else {
            continue;
        };
        let layout = asset.texture_layout();
        for (entity, effect, _) in &emitters {
            if effect.handle.id() != *id {
                continue;
            }
            if layout.layout.is_empty() {
                commands.entity(entity).remove::<EffectMaterial>();
            } else {
                commands
                    .entity(entity)
                    .insert(effect_material_for(asset, &asset_server));
            }
        }
    }

    // Path 2: emitter exists, asset is ready, but material was never bound
    // (Restart after initial load — no new AssetEvent fires).
    for (entity, effect, material) in &emitters {
        if material.is_some() {
            continue;
        }
        let Some(asset) = effects.get(&effect.handle) else {
            continue;
        };
        if asset.texture_layout().layout.is_empty() {
            continue;
        }
        commands
            .entity(entity)
            .insert(effect_material_for(asset, &asset_server));
    }
}

/// True when the loaded asset declares the anchored-trail properties.
fn is_anchored_trail(asset: &EffectAsset) -> bool {
    asset
        .properties()
        .iter()
        .any(|p| p.name() == SPAWN_ORIGIN_PROPERTY)
}

/// Emitters not yet re-homed by [`anchor_trail_emitters`].
type UnanchoredEmitters<'w, 's> =
    Query<'w, 's, (Entity, &'static ParticleEffect), (With<Emitter>, Without<TrailAnchored>)>;

/// Re-homes emitters whose effect declares the anchored-trail contract: the
/// instance detaches from the rocket and sits at the world origin (identity
/// transform); `apply_trail_properties` then feeds it the live nozzle pose.
/// Runs whenever the asset becomes available (spawn order independent).
fn anchor_trail_emitters(
    mut commands: Commands,
    effects: Res<Assets<EffectAsset>>,
    emitters: UnanchoredEmitters,
) {
    for (entity, effect) in &emitters {
        let Some(asset) = effects.get(&effect.handle) else {
            continue;
        };
        if !is_anchored_trail(asset) {
            continue;
        }
        commands.entity(entity).remove::<ChildOf>().insert((
            TrailAnchored,
            Transform::IDENTITY,
            EffectProperties::default(),
        ));
    }
}

/// Every frame: write the nozzle pose (in the anchor's frame — here, plain
/// world coordinates) into the anchored-trail properties.
fn apply_trail_properties(
    scene: Res<SceneConfig>,
    rocket: Query<&GlobalTransform, With<RocketRoot>>,
    mut emitters: Query<(&Emitter, &mut EffectProperties), With<TrailAnchored>>,
) {
    let Ok(rocket) = rocket.single() else {
        return;
    };
    for (emitter, mut properties) in &mut emitters {
        let Some(config) = scene.emitters.get(emitter.index) else {
            continue;
        };
        let nozzle = rocket.affine() * emitter_transform(config).compute_affine();
        let origin: Vec3 = nozzle.translation.into();
        let axis = (nozzle.matrix3 * Vec3::NEG_Y).normalize_or(Vec3::NEG_Y);
        properties.set(SPAWN_ORIGIN_PROPERTY, origin.into());
        properties.set(SPAWN_AXIS_PROPERTY, axis.into());
    }
}

/// Hanabi only recompiles when `ParticleEffect` changes, so nudge instances
/// whose underlying asset was modified on disk (hot reload).
fn recompile_on_asset_change(
    mut events: MessageReader<AssetEvent<EffectAsset>>,
    mut emitters: Query<(&mut ParticleEffect, &Emitter)>,
) {
    for event in events.read() {
        let AssetEvent::Modified { id } = event else {
            continue;
        };
        for (mut effect, emitter) in &mut emitters {
            if effect.handle.id() == *id {
                effect.set_changed();
                info!("hot-reloaded effect for emitter '{}'", emitter.name);
            }
        }
    }
}

fn scale_cpu_value(value: &CpuValue<f32>, factor: f32) -> CpuValue<f32> {
    match value {
        CpuValue::Single(v) => CpuValue::Single(v * factor),
        CpuValue::Uniform((lo, hi)) => CpuValue::Uniform((lo * factor, hi * factor)),
        // CpuValue is #[non_exhaustive]-style; pass unknown variants through.
        other => *other,
    }
}

/// Every frame: spawner settings = authored settings from the `.effect` asset,
/// with the spawn count scaled by `intensity x activity(t)`. Effects that
/// declare the `intensity` property additionally receive the same signal as a
/// shader uniform (throttle-driven plume length/brightness).
fn apply_emitter_intensity(
    mut commands: Commands,
    clock: Res<SimClock>,
    scene: Res<SceneConfig>,
    effects: Res<Assets<EffectAsset>>,
    mut emitters: Query<(
        Entity,
        &Emitter,
        &ParticleEffect,
        &mut EffectSpawner,
        Option<&mut EffectProperties>,
    )>,
) {
    for (entity, emitter, effect, mut spawner, properties) in &mut emitters {
        let Some(config) = scene.emitters.get(emitter.index) else {
            continue;
        };
        let Some(asset) = effects.get(&effect.handle) else {
            continue;
        };
        let multiplier = config.intensity * config.activity_at(clock.t);
        // Sky/earth once-bursts get intensity from orbit.rs (day/night).
        if config.attach != "sky"
            && config.attach != "earth"
            && asset
                .properties()
                .iter()
                .any(|p| p.name() == INTENSITY_PROPERTY)
        {
            let clamped = multiplier.clamp(0.0, 1.0);
            match properties {
                Some(mut properties) => properties.set(INTENSITY_PROPERTY, clamped.into()),
                None => {
                    let mut instance = EffectProperties::default();
                    instance.set(INTENSITY_PROPERTY, clamped.into());
                    commands.entity(entity).insert(instance);
                }
            }
        }
        // Once-burst fields (stars, cities, airglow) must spawn at full
        // capacity on the first sim frame. Dimming is the intensity / sun_dir
        // property, not spawn count.
        if asset.spawner.is_once() {
            spawner.active = true;
            spawner.settings = asset.spawner;
            continue;
        }
        if multiplier <= ACTIVITY_CUTOFF {
            spawner.active = false;
            continue;
        }
        spawner.active = true;
        let mut settings = asset.spawner;
        settings.set_count(scale_cpu_value(&settings.count(), multiplier));
        spawner.settings = settings;
    }
}

/// Every frame: light luminous power = configured peak scaled by the same
/// `intensity x activity(t)` multiplier as the spawner.
fn apply_emitter_lights(
    clock: Res<SimClock>,
    scene: Res<SceneConfig>,
    mut points: Query<(
        &EmitterLight,
        Option<&mut PointLight>,
        Option<&mut SpotLight>,
    )>,
) {
    for (light, point, spot) in &mut points {
        let Some(config) = scene.emitters.get(light.index) else {
            continue;
        };
        let Some(light_config) = &config.light else {
            continue;
        };
        let multiplier = config.intensity * config.activity_at(clock.t);
        let lm = if multiplier <= ACTIVITY_CUTOFF {
            0.0
        } else {
            light_config.intensity_lm * multiplier
        };
        if let Some(mut point) = point {
            point.intensity = lm;
        }
        if let Some(mut spot) = spot {
            spot.intensity = lm;
        }
    }
}

fn draw_emitter_gizmos(
    show: Res<ShowEmitterGizmos>,
    bounds: Res<crate::rocket::RocketBounds>,
    mut gizmos: Gizmos,
    emitters: Query<&GlobalTransform, With<Emitter>>,
) {
    if !show.0 {
        return;
    }
    // Scale gizmos with the vehicle so they read on a 7 m lander and a 70 m
    // rocket alike.
    let arrow = (bounds.height * 0.09).clamp(0.8, 6.0);
    for transform in &emitters {
        let origin = transform.translation();
        let exhaust = transform.rotation() * Vec3::NEG_Y;
        gizmos.sphere(origin, arrow * 0.1, Color::srgb(1.0, 0.3, 0.1));
        gizmos.arrow(origin, origin + exhaust * arrow, Color::srgb(1.0, 0.6, 0.1));
    }
}
