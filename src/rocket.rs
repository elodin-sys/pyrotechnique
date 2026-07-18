//! Rocket model: GLB loading and normalization.
//!
//! The GLB (a Sketchfab rig) is loaded as a scene, then uniformly rescaled and
//! re-based so that its bounding box height equals `model.target_height`, its
//! base center sits at the rocket-root origin, and +Y is up. Emitters and the
//! flight path then work in this normalized "rocket frame".

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;

use crate::scene::SceneConfig;

/// Root entity of the rocket. Its transform is driven by the flight path;
/// the GLB scene hangs underneath a child "model" entity which carries the
/// normalization transform.
#[derive(Component)]
pub struct RocketRoot;

/// Child of [`RocketRoot`] that owns the raw GLB scene; normalization rewrites
/// this entity's transform.
#[derive(Component)]
pub struct RocketModel;

/// Marker inserted once normalization has been applied.
#[derive(Component)]
pub struct RocketNormalized;

/// World-facts about the normalized rocket, for emitter placement and cameras.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct RocketBounds {
    pub ready: bool,
    /// Height in meters after normalization (== model.target_height).
    pub height: f32,
    /// Max horizontal half-extent after normalization.
    pub radius: f32,
}

pub struct RocketPlugin;

impl Plugin for RocketPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RocketBounds>()
            .add_systems(Startup, spawn_rocket)
            .add_systems(Update, normalize_rocket);
    }
}

fn spawn_rocket(mut commands: Commands, scene: Res<SceneConfig>, asset_server: Res<AssetServer>) {
    let world_handle: Handle<WorldAsset> =
        asset_server.load(GltfAssetLabel::Scene(0).from_asset(scene.model.path.clone()));
    commands
        .spawn((
            Name::new("rocket"),
            RocketRoot,
            Transform::default(),
            Visibility::default(),
        ))
        .with_children(|parent| {
            parent.spawn((
                Name::new("rocket model"),
                RocketModel,
                WorldAssetRoot(world_handle),
                Transform::default(),
                Visibility::default(),
            ));
        });
}

/// Runs until the GLB meshes are in and measurable, then rescales/re-bases the
/// model child so the rocket occupies `[0, target_height]` on +Y with its base
/// centered at the root origin.
fn normalize_rocket(
    mut commands: Commands,
    scene: Res<SceneConfig>,
    mut bounds: ResMut<RocketBounds>,
    model: Query<(Entity, &GlobalTransform), (With<RocketModel>, Without<RocketNormalized>)>,
    children: Query<&Children>,
    aabbs: Query<(&Aabb, &GlobalTransform)>,
    mut transforms: Query<&mut Transform>,
) {
    let Ok((model_entity, model_gt)) = model.single() else {
        return;
    };

    // Merge descendant AABBs, expressed in the model entity's local frame.
    let to_model = model_gt.affine().inverse();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    for descendant in children.iter_descendants(model_entity) {
        let Ok((aabb, gt)) = aabbs.get(descendant) else {
            continue;
        };
        let local_to_model = to_model * gt.affine();
        let center = Vec3::from(aabb.center);
        let half = Vec3::from(aabb.half_extents);
        for i in 0..8 {
            let corner = center
                + half
                    * Vec3::new(
                        if i & 1 == 0 { -1.0 } else { 1.0 },
                        if i & 2 == 0 { -1.0 } else { 1.0 },
                        if i & 4 == 0 { -1.0 } else { 1.0 },
                    );
            let p = local_to_model.transform_point3(corner);
            min = min.min(p);
            max = max.max(p);
        }
        found = true;
    }
    if !found {
        return;
    }

    let extent = max - min;
    if extent.y <= 0.0 {
        return;
    }
    let scale = scene.model.target_height / extent.y;
    let base_center = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);

    if let Ok(mut transform) = transforms.get_mut(model_entity) {
        transform.scale = Vec3::splat(scale);
        transform.translation = -base_center * scale;
    }
    commands.entity(model_entity).insert(RocketNormalized);

    bounds.ready = true;
    bounds.height = scene.model.target_height;
    bounds.radius = extent.x.max(extent.z) * 0.5 * scale;
    info!(
        "rocket normalized: raw extent {:?}, scale {:.4}, height {:.1} m, radius {:.2} m",
        extent, scale, bounds.height, bounds.radius
    );
}
