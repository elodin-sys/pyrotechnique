//! Emitters: turn scene `EmitterConfig`s into live Hanabi effect instances.
//!
//! Each emitter is a child entity of the rocket root, positioned/oriented in
//! the normalized rocket frame (exhaust axis = local -Y). Every frame the
//! emitter's spawn rate is the effect file's authored rate scaled by
//! `intensity x activity(t)`. `.effect` files hot-reload through the asset
//! server; on reload the instance is recompiled and its material rebound.

pub mod builders;

use bevy::prelude::*;
use bevy_hanabi::{CpuValue, EffectAsset, EffectMaterial, EffectSpawner, ParticleEffect};

use crate::app::SimClock;
use crate::rocket::RocketRoot;
use crate::scene::{EmitterConfig, SceneConfig};

/// Below this effective multiplier the emitter is fully deactivated.
const ACTIVITY_CUTOFF: f32 = 1e-3;

/// Live emitter instance, child of the rocket root.
#[derive(Component)]
pub struct Emitter {
    pub name: String,
    /// Index into `SceneConfig::emitters`.
    pub index: usize,
}

/// Show emitter gizmos (position + exhaust direction). Toggled from the UI.
#[derive(Resource)]
pub struct ShowEmitterGizmos(pub bool);

pub struct EmitterPlugin;

impl Plugin for EmitterPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShowEmitterGizmos(true)).add_systems(
            Update,
            (
                spawn_emitters,
                bind_effect_materials,
                recompile_on_asset_change,
                apply_emitter_intensity,
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
    existing: Query<(), With<Emitter>>,
) {
    if !existing.is_empty() {
        return;
    }
    let Ok(rocket) = rocket.single() else {
        return;
    };
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
        let entity = entity.id();
        // World-attached emitters (e.g. pad smoke) stay put; the rest ride
        // the rocket.
        if config.attach != "world" {
            commands.entity(rocket).add_child(entity);
        }
    }
    info!("spawned {} emitters", scene.emitters.len());
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
        // "mask" and anything unknown get the soft round falloff.
        _ => asset_server.load("textures/soft_circle.png"),
    }
}

fn effect_material_for(asset: &EffectAsset, asset_server: &AssetServer) -> EffectMaterial {
    let images = asset
        .texture_layout()
        .layout
        .iter()
        .map(|slot| slot_image(&slot.name, asset_server))
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
/// with the spawn count scaled by `intensity x activity(t)`.
fn apply_emitter_intensity(
    clock: Res<SimClock>,
    scene: Res<SceneConfig>,
    effects: Res<Assets<EffectAsset>>,
    mut emitters: Query<(&Emitter, &ParticleEffect, &mut EffectSpawner)>,
) {
    for (emitter, effect, mut spawner) in &mut emitters {
        let Some(config) = scene.emitters.get(emitter.index) else {
            continue;
        };
        let Some(asset) = effects.get(&effect.handle) else {
            continue;
        };
        let multiplier = config.intensity * config.activity_at(clock.t);
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

fn draw_emitter_gizmos(
    show: Res<ShowEmitterGizmos>,
    mut gizmos: Gizmos,
    emitters: Query<&GlobalTransform, With<Emitter>>,
) {
    if !show.0 {
        return;
    }
    for transform in &emitters {
        let origin = transform.translation();
        let exhaust = transform.rotation() * Vec3::NEG_Y;
        gizmos.sphere(origin, 0.6, Color::srgb(1.0, 0.3, 0.1));
        gizmos.arrow(
            origin,
            origin + exhaust * 6.0,
            Color::srgb(1.0, 0.6, 0.1),
        );
    }
}
