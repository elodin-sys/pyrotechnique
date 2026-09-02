//! Environment: HDR camera, procedural atmosphere, sun, ground, Earth, sky.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{ClearColorConfig, Exposure, Hdr};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::image::{
    ImageAddressMode, ImageFilterMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor,
};
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, GlobalAmbientLight, Skybox, SunDisk};
use bevy::pbr::{AtmosphereMode, AtmosphereSettings};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureViewDescriptor, TextureViewDimension};
use bevy::transform::TransformSystems;
use bevy_panorbit_camera::PanOrbitCamera;

use crate::scene::{CameraPreset, SceneConfig};

/// Marker for the single main viewport camera.
#[derive(Component)]
pub struct MainCamera;

/// Marker for scene-owned environment entities (sun, atmosphere, ground,
/// Earth, sky) that are torn down and rebuilt on project switch.
#[derive(Component)]
pub struct EnvironmentEntity;

/// Inertial frame: sun + sky-attached emitters. Orbit rotation lives here.
#[derive(Component)]
pub struct SkyRoot;

/// True-scale Earth globe root (planet center). City lights / airglow parent.
#[derive(Component)]
pub struct EarthRoot;

/// Fill light from the Earth disc onto the craft.
#[derive(Component)]
pub struct Earthshine;

/// Dim night fill on the globe (layer 0). Not the craft.
#[derive(Component)]
pub struct NightGlobeFill;

/// Globe mesh whose emissive map is scaled by orbit night visibility.
#[derive(Component)]
pub struct EarthGlobeMaterial;

/// Cloud shell faded by `nightglow_visibility` so it does not hide night lights.
#[derive(Component)]
pub struct EarthCloudsMaterial;

/// Local pad disc; faded out by ~5–8 km so the globe takes over.
#[derive(Component)]
pub struct PadDisc;

/// Atmosphere whose [`GlobalTransform`] is the planet center (Earth).
#[derive(Component)]
pub struct OrbitalAtmosphere;

/// True once the Earth GLB has mesh descendants (or the scene has no Earth).
#[derive(Resource, Default)]
pub struct EarthReady(pub bool);

pub(crate) fn earth_map_sampler() -> ImageSamplerDescriptor {
    ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    }
}

pub(crate) fn load_earth_map(
    asset_server: &AssetServer,
    path: &'static str,
    srgb: bool,
) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(move |settings: &mut ImageLoaderSettings| {
            settings.is_srgb = srgb;
            settings.sampler = ImageSampler::Descriptor(earth_map_sampler());
            settings.asset_usage = RenderAssetUsages::RENDER_WORLD;
        })
        .load(path)
}

/// Scene directional sun (child of [`SkyRoot`]).
#[derive(Component)]
pub struct SceneSun;

pub struct EnvironmentPlugin;

impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_camera)
            .init_resource::<EarthReady>()
            .add_systems(
                Update,
                (
                    ensure_environment,
                    hide_earth_placeholder,
                    uncull_earth_meshes,
                    tag_earth_globe_material,
                    configure_skybox_cubemap,
                    mark_earth_ready,
                ),
            )
            .add_systems(
                PostUpdate,
                pin_orbital_atmosphere.after(TransformSystems::Propagate),
            );
    }
}

/// Point an orbit camera at a preset pose (uses the smoothed targets so the
/// transition eases in edit mode).
pub fn snap_orbit_to_preset(
    orbit: &mut PanOrbitCamera,
    projection: &mut Projection,
    preset: &CameraPreset,
    rocket_pos: Vec3,
    scene: &SceneConfig,
) {
    let (pos, look_at) = crate::capture::preset_pose(preset, rocket_pos);
    let offset = pos - look_at;
    let radius = offset.length().max(0.01);
    orbit.target_focus = look_at;
    orbit.target_radius = radius;
    orbit.target_yaw = f32::atan2(offset.x, offset.z);
    orbit.target_pitch = (offset.y / radius).clamp(-1.0, 1.0).asin();
    *projection = Projection::Perspective(scene.environment.perspective(preset.fov_deg));
}

fn setup_camera(mut commands: Commands, scene: Res<SceneConfig>) {
    let env = &scene.environment;
    let preset = crate::project::initial_preset(&scene);
    let (rocket_pos, _) = scene.flight.sample(0.0);
    let (pos, look_at) = crate::capture::preset_pose(&preset, rocket_pos);

    commands.spawn((
        MainCamera,
        Camera3d::default(),
        Hdr,
        Tonemapping::TonyMcMapface,
        Exposure {
            ev100: env.exposure_ev100,
        },
        Bloom {
            intensity: env.bloom_intensity,
            ..Bloom::OLD_SCHOOL
        },
        atmosphere_settings(env),
        Projection::Perspective(env.perspective(preset.fov_deg)),
        Transform::from_translation(pos).looking_at(look_at, Vec3::Y),
        RenderLayers::from_layers(&[0, 1]),
        PanOrbitCamera {
            focus: look_at,
            ..default()
        },
    ));
}

pub fn atmosphere_settings(env: &crate::scene::EnvironmentConfig) -> AtmosphereSettings {
    atmosphere_settings_mode(env.atmosphere_raymarched)
}

pub fn atmosphere_settings_mode(raymarched: bool) -> AtmosphereSettings {
    if raymarched {
        AtmosphereSettings {
            aerial_view_lut_max_distance: 3.2e5,
            rendering_method: AtmosphereMode::Raymarched,
            sky_max_samples: 48,
            sky_view_lut_samples: 32,
            sky_view_lut_size: UVec2::new(800, 400),
            ..AtmosphereSettings::default()
        }
    } else {
        AtmosphereSettings::default()
    }
}

/// (Re)spawns sun/atmosphere/ground/Earth from the current scene when missing.
fn ensure_environment(
    mut commands: Commands,
    scene: Res<SceneConfig>,
    asset_server: Res<AssetServer>,
    mut media: ResMut<Assets<ScatteringMedium>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    existing: Query<(), With<EnvironmentEntity>>,
    mut camera: Query<
        (
            Entity,
            &mut Camera,
            &mut Exposure,
            &mut Bloom,
            &mut AtmosphereSettings,
        ),
        With<MainCamera>,
    >,
) {
    if !existing.is_empty() {
        return;
    }
    let env = &scene.environment;

    let sky = commands
        .spawn((
            EnvironmentEntity,
            SkyRoot,
            Name::new("sky"),
            Transform::default(),
            Visibility::default(),
        ))
        .id();

    let azimuth = env.sun_azimuth_deg.to_radians();
    let elevation = env.sun_elevation_deg.to_radians();
    // Compressed LEO from t=0: noon along +Y. Delayed orbit (ascent) keeps az/el.
    let sun_rotation = if env.orbit_period_s > 1e-3 && env.orbit_start_s <= 1e-3 {
        Quat::from_rotation_arc(Vec3::Z, Vec3::Y)
    } else {
        Quat::from_euler(EulerRot::YXZ, -azimuth, -elevation, 0.0)
    };
    let sun = commands
        .spawn((
            EnvironmentEntity,
            SceneSun,
            Name::new("sun"),
            DirectionalLight {
                illuminance: env.sun_illuminance,
                shadow_maps_enabled: true,
                ..default()
            },
            SunDisk::EARTH,
            Transform::from_rotation(sun_rotation),
            RenderLayers::from_layers(&[0, 1]),
        ))
        .id();
    commands.entity(sky).add_child(sun);

    if let Some(earth) = &env.earth {
        let orient = earth.orient();
        let world_handle: Handle<WorldAsset> =
            asset_server.load(GltfAssetLabel::Scene(0).from_asset(earth.path.clone()));
        commands
            .spawn((
                EnvironmentEntity,
                EarthRoot,
                Name::new("earth"),
                Transform {
                    translation: earth.center(),
                    rotation: orient,
                    scale: Vec3::ONE,
                },
                Visibility::default(),
                NoFrustumCulling,
            ))
            .with_children(|parent| {
                parent.spawn((
                    Name::new("earth model"),
                    WorldAssetRoot(world_handle),
                    Transform::default(),
                    Visibility::default(),
                ));
            });
    }

    if env.atmosphere {
        let density = if let Some(earth) = &env.earth {
            let (rocket_pos, _) = scene.flight.sample(0.0);
            let preset = crate::project::initial_preset(&scene);
            let (cam_pos, _) = crate::capture::preset_pose(&preset, rocket_pos);
            crate::earth_env::atmosphere_density(crate::earth_env::altitude_above(
                cam_pos,
                earth.center(),
                earth.radius_m,
            ))
        } else if env.atmosphere_raymarched {
            0.16
        } else {
            1.0
        };
        let medium_asset =
            ScatteringMedium::earth(256, 256).with_density_multiplier(density);
        let medium = media.add(medium_asset);
        let inner = env.atmosphere_inner_radius.unwrap_or(6_360_000.0);
        let outer = env.atmosphere_outer_radius.unwrap_or(inner + 100_000.0);
        let albedo = Vec3::new(0.20, 0.22, 0.18);
        if let Some(center) = env.atmosphere_center() {
            // GlobalTransform must be non-default or Atmosphere's on_add hook
            // parks the planet at −Y × inner_radius (surface at the origin).
            commands.spawn((
                EnvironmentEntity,
                OrbitalAtmosphere,
                Name::new("atmosphere"),
                Atmosphere {
                    inner_radius: inner,
                    outer_radius: outer,
                    ground_albedo: albedo,
                    medium,
                },
                Transform::from_translation(center),
                GlobalTransform::from_translation(center),
            ));
        } else {
            commands.spawn((
                EnvironmentEntity,
                Name::new("atmosphere"),
                Atmosphere::earth(medium),
            ));
        }
    }

    if env.ground_radius > 0.0 {
        let c = env.ground_color;
        commands.spawn((
            EnvironmentEntity,
            PadDisc,
            Name::new("ground"),
            Mesh3d(meshes.add(Circle::new(env.ground_radius))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgba(c[0], c[1], c[2], 1.0),
                perceptual_roughness: 1.0,
                metallic: 0.0,
                // Opaque writes depth so Hanabi particles are not sorted against a 20 km disc.
                alpha_mode: AlphaMode::Opaque,
                ..default()
            })),
            Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            Visibility::default(),
        ));
    }

    if env.earthshine_illuminance > 0.0 {
        commands.spawn((
            EnvironmentEntity,
            Earthshine,
            Name::new("earthshine"),
            DirectionalLight {
                color: Color::srgb(0.72, 0.78, 0.88),
                illuminance: env.earthshine_illuminance,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_rotation(Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::Y)),
            RenderLayers::layer(1),
        ));
    }

    if env.night_globe_illuminance > 0.0 {
        commands.spawn((
            EnvironmentEntity,
            NightGlobeFill,
            Name::new("night_globe_fill"),
            DirectionalLight {
                color: Color::srgb(0.55, 0.48, 0.40),
                illuminance: env.night_globe_illuminance,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_rotation(Quat::from_rotation_arc(Vec3::Z, Vec3::Y)),
            RenderLayers::layer(0),
        ));
    }

    ambient.brightness = env.ambient_brightness;
    ambient.color = Color::WHITE;

    if let Ok((cam_entity, mut cam, mut exposure, mut bloom, mut atmo)) = camera.single_mut() {
        exposure.ev100 = env.exposure_ev100;
        bloom.intensity = env.bloom_intensity;
        *atmo = atmosphere_settings(env);
        cam.clear_color = if env.atmosphere && !env.atmosphere_raymarched && env.earth.is_none()
        {
            ClearColorConfig::Default
        } else {
            ClearColorConfig::Custom(Color::BLACK)
        };
        if let Some(path) = &env.skybox {
            commands.entity(cam_entity).insert(Skybox {
                image: Some(asset_server.load(path.clone())),
                brightness: 0.0,
                rotation: Quat::IDENTITY,
            });
        } else {
            commands.entity(cam_entity).remove::<Skybox>();
        }
    }
}

/// The Earth GLB ships a leftover unit `Cube` next to the real globe (`Cube.001`).
fn hide_earth_placeholder(
    earth: Query<Entity, With<EarthRoot>>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut vis: Query<&mut Visibility>,
) {
    let Ok(root) = earth.single() else {
        return;
    };
    for descendant in children.iter_descendants(root) {
        let Ok(name) = names.get(descendant) else {
            continue;
        };
        if name.as_str() != "Cube" {
            continue;
        }
        if let Ok(mut visibility) = vis.get_mut(descendant) {
            *visibility = Visibility::Hidden;
        }
    }
}

fn pin_orbital_atmosphere(
    scene: Res<SceneConfig>,
    earth: Query<&GlobalTransform, (With<EarthRoot>, Without<OrbitalAtmosphere>)>,
    mut atmospheres: Query<(&mut Transform, &mut GlobalTransform), With<OrbitalAtmosphere>>,
) {
    let center = earth
        .single()
        .map(|gt| gt.translation())
        .ok()
        .or_else(|| scene.environment.atmosphere_center());
    let Some(center) = center else {
        return;
    };
    for (mut transform, mut gt) in &mut atmospheres {
        transform.translation = center;
        transform.rotation = Quat::IDENTITY;
        *gt = GlobalTransform::from_translation(center);
    }
}

fn uncull_earth_meshes(
    earth: Query<Entity, With<EarthRoot>>,
    children: Query<&Children>,
    culled: Query<Entity, (With<Mesh3d>, Without<NoFrustumCulling>)>,
    mut commands: Commands,
) {
    let Ok(root) = earth.single() else {
        return;
    };
    for descendant in children.iter_descendants(root) {
        if culled.contains(descendant) {
            commands.entity(descendant).insert(NoFrustumCulling);
        }
    }
}

fn tag_earth_globe_material(
    earth: Query<Entity, With<EarthRoot>>,
    children: Query<&Children>,
    names: Query<&Name>,
    mesh_materials: Query<
        &MeshMaterial3d<StandardMaterial>,
        (Without<EarthGlobeMaterial>, Without<EarthCloudsMaterial>),
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(root) = earth.single() else {
        return;
    };
    for descendant in children.iter_descendants(root) {
        let Ok(handle) = mesh_materials.get(descendant) else {
            continue;
        };
        let name = names
            .get(descendant)
            .map(|n| n.as_str())
            .unwrap_or("");
        if name.contains("Cloud") {
            if let Some(mut material) = materials.get_mut(&handle.0) {
                material.base_color = Color::WHITE;
                material.base_color_texture =
                    Some(load_earth_map(&asset_server, "textures/earth/clouds.ktx2", true));
                material.alpha_mode = AlphaMode::Blend;
            }
            commands.entity(descendant).insert(EarthCloudsMaterial);
            continue;
        }
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.base_color = Color::WHITE;
            material.base_color_texture =
                Some(load_earth_map(&asset_server, "textures/earth/color.ktx2", true));
            material.emissive = LinearRgba::WHITE;
            material.emissive_texture =
                Some(load_earth_map(&asset_server, "textures/earth/night.ktx2", true));
            material.normal_map_texture =
                Some(load_earth_map(&asset_server, "textures/earth/normal.ktx2", false));
            material.metallic_roughness_texture = Some(load_earth_map(
                &asset_server,
                "textures/earth/metallic_roughness.ktx2",
                false,
            ));
        }
        commands.entity(descendant).insert(EarthGlobeMaterial);
    }
}

fn configure_skybox_cubemap(
    skyboxes: Query<&Skybox, With<MainCamera>>,
    mut images: ResMut<Assets<Image>>,
) {
    for skybox in &skyboxes {
        let Some(handle) = &skybox.image else {
            continue;
        };
        configure_cubemap_image(handle, &mut images);
    }
}

fn configure_cubemap_image(handle: &Handle<Image>, images: &mut Assets<Image>) -> bool {
    let Some(image) = images.get(handle) else {
        return false;
    };
    if image
        .texture_view_descriptor
        .as_ref()
        .is_some_and(|descriptor| descriptor.dimension == Some(TextureViewDimension::Cube))
    {
        return true;
    }
    if image.width() == 0 {
        return false;
    }

    let array_layers = image.texture_descriptor.array_layer_count();
    if array_layers == 6 {
        let Some(mut image) = images.get_mut(handle) else {
            return false;
        };
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        });
        return true;
    }

    if array_layers == 1 {
        let layers = image.height() / image.width();
        if layers == 6 {
            let Some(mut image) = images.get_mut(handle) else {
                return false;
            };
            let _ = image.reinterpret_stacked_2d_as_array(layers);
            image.texture_view_descriptor = Some(TextureViewDescriptor {
                dimension: Some(TextureViewDimension::Cube),
                ..default()
            });
            return true;
        }
    }

    false
}

fn material_images_ready(material: &StandardMaterial, asset_server: &AssetServer) -> bool {
    [
        material.base_color_texture.as_ref(),
        material.emissive_texture.as_ref(),
        material.normal_map_texture.as_ref(),
        material.metallic_roughness_texture.as_ref(),
    ]
    .into_iter()
    .flatten()
    .all(|handle| asset_server.is_loaded_with_dependencies(handle.id()))
}

fn mark_earth_ready(
    scene: Res<SceneConfig>,
    earth: Query<Entity, With<EarthRoot>>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    materials: Res<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    skyboxes: Query<&Skybox, With<MainCamera>>,
    mut images: ResMut<Assets<Image>>,
    mut ready: ResMut<EarthReady>,
) {
    let earth_ok = if scene.environment.earth.is_none() {
        true
    } else {
        match earth.single() {
            Ok(root) => {
                let descendants: Vec<_> = children.iter_descendants(root).collect();
                let has_mesh = descendants.iter().any(|entity| meshes.contains(*entity));
                let textures_ok = descendants.iter().all(|entity| {
                    let Ok(handle) = mesh_materials.get(*entity) else {
                        return true;
                    };
                    materials
                        .get(&handle.0)
                        .is_some_and(|material| material_images_ready(material, &asset_server))
                });
                has_mesh && textures_ok
            }
            Err(_) => false,
        }
    };

    let skybox_ok = match scene.environment.skybox.as_ref() {
        None => true,
        Some(_) => skyboxes.iter().any(|skybox| {
            skybox
                .image
                .as_ref()
                .is_some_and(|handle| configure_cubemap_image(handle, &mut images))
        }),
    };

    ready.0 = earth_ok && skybox_ok;
}
