//! Rust builders for the built-in `.effect` files, plus generated sprite
//! textures.
//!
//! `pyrotechnique gen-effects` serializes these to `assets/effects/*.effect`
//! using Hanabi's canonical format and writes the particle sprite textures to
//! `assets/textures/`. Day-to-day tuning edits the RON directly (or through
//! the editor UI); regenerate only to change effect *structure*.
//!
//! Conventions shared by all built-in effects:
//! - Exhaust axis is **local -Y**; the emitter entity is rotated so -Y points
//!   along the scene's `direction`.
//! - Texture slot names map to generated sprites: `mask` -> soft_circle.png,
//!   `veil` -> glow_veil.png, `smoke` -> smoke_puff.png (see `effects::mod`).
//! - Colors are HDR (components above 1.0) and rely on viewport bloom.

use bevy::ecs::reflect::AppTypeRegistry;
use bevy::math::{Vec3, Vec4};
use bevy_hanabi::graph::expr::{ExprHandle, PropertyHandle, WriterExpr};
use bevy_hanabi::prelude::*;
use bevy_hanabi::register_modifiers;

use crate::GenEffectsArgs;

/// Per-particle random brightness/alpha stored in `Attribute::COLOR`, for use
/// with `ColorBlendMode::Modulate` gradients. This is what turns uniform fog
/// into structured, billowing smoke.
fn init_random_modulation(
    writer: &ExprWriter,
    value_min: f32,
    value_max: f32,
    alpha_min: f32,
    alpha_max: f32,
) -> SetAttributeModifier {
    let v =
        writer.rand(ScalarType::Float) * writer.lit(value_max - value_min) + writer.lit(value_min);
    let a =
        writer.rand(ScalarType::Float) * writer.lit(alpha_max - alpha_min) + writer.lit(alpha_min);
    let rgba = v.clone().vec3(v.clone(), v).vec4_xyz_w(a).pack4x8unorm();
    SetAttributeModifier::new(Attribute::COLOR, rgba.expr())
}

/// All built-in effects: (project, file stem, builder).
pub fn builtin_effects() -> Vec<(&'static str, &'static str, EffectAsset)> {
    vec![
        ("falcon9", "merlin_core", merlin_core()),
        ("falcon9", "merlin_flame", merlin_flame()),
        ("falcon9", "exhaust_smoke", exhaust_smoke()),
        ("falcon9", "pad_smoke", pad_smoke()),
        ("falcon9", "rcs_dart", rcs_dart()),
        ("apollo-lander", "descent_plume", descent_plume()),
        ("apollo-lander", "descent_glow", descent_glow()),
        ("apollo-lander", "rcs_puff", rcs_puff()),
        ("apollo-lander", "ground_dust", ground_dust()),
        ("rocket", "motor_core", motor_core()),
        ("rocket", "motor_flame", motor_flame()),
        ("rocket", "boost_trail", boost_trail()),
        ("rocket", "launch_smoke", launch_smoke()),
        ("satellite", "stars_dim", stars_dim()),
        ("satellite", "stars_bright", stars_bright()),
        ("satellite", "milky_way", milky_way()),
        ("satellite", "city_lights", city_lights()),
        ("satellite", "airglow_green", airglow_green()),
        ("satellite", "airglow_red", airglow_red()),
    ]
}

pub fn generate(args: &GenEffectsArgs) -> anyhow::Result<()> {
    let type_registry = AppTypeRegistry::new_with_derived_types();
    register_modifiers(&type_registry);
    crate::effects::sphere_map::register(&type_registry);
    crate::effects::city_tile_cdf::register(&type_registry);
    let registry = type_registry.read();

    for (project, name, effect) in builtin_effects() {
        let dir = args.out_dir.join(project);
        std::fs::create_dir_all(&dir)?;
        let text = effect
            .serialize(&registry)
            .map_err(|e| anyhow::anyhow!("serializing {project}/{name}: {e}"))?;
        let path = dir.join(format!("{name}.effect"));
        std::fs::write(&path, text)?;
        println!("wrote {}", path.display());
    }

    // Sprite textures are shared by all projects, under assets/textures.
    let tex_dir = args
        .out_dir
        .parent()
        .unwrap_or(std::path::Path::new("assets"))
        .join("textures");
    std::fs::create_dir_all(&tex_dir)?;
    write_soft_circle(&tex_dir.join("soft_circle.png"))?;
    write_glow_veil(&tex_dir.join("glow_veil.png"))?;
    write_smoke_puff(&tex_dir.join("smoke_puff.png"))?;
    println!("wrote textures to {}", tex_dir.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Textures
// ---------------------------------------------------------------------------

/// Radial falloff sprite: opaque center feathering to transparent rim.
fn write_soft_circle(path: &std::path::Path) -> anyhow::Result<()> {
    const SIZE: u32 = 128;
    let mut img = image::GrayImage::new(SIZE, SIZE);
    let center = (SIZE as f32 - 1.0) * 0.5;
    for (x, y, px) in img.enumerate_pixels_mut() {
        let dx = (x as f32 - center) / center;
        let dy = (y as f32 - center) / center;
        let d = (dx * dx + dy * dy).sqrt().min(1.0);
        let falloff = (1.0 - d).powf(1.7);
        *px = image::Luma([(falloff * 255.0) as u8]);
    }
    img.save(path)?;
    Ok(())
}

/// Wide Gaussian with live corners — no hard disc edge, no hot core after overlap.
fn write_glow_veil(path: &std::path::Path) -> anyhow::Result<()> {
    const SIZE: u32 = 256;
    let mut img = image::GrayImage::new(SIZE, SIZE);
    let center = (SIZE as f32 - 1.0) * 0.5;
    for (x, y, px) in img.enumerate_pixels_mut() {
        let dx = (x as f32 - center) / center;
        let dy = (y as f32 - center) / center;
        let r2 = dx * dx + dy * dy;
        let falloff = (-1.9 * r2).exp();
        *px = image::Luma([(falloff * 255.0) as u8]);
    }
    img.save(path)?;
    Ok(())
}

/// Mottled, internally shaded smoke sprite (RGBA, sampled with full
/// `Modulate`): alpha carries radial falloff x noise, RGB carries baked
/// shading (bright top-lit lobes, dusky creases) so overlapping billboards
/// read as billowing volume instead of flat fog.
fn write_smoke_puff(path: &std::path::Path) -> anyhow::Result<()> {
    const SIZE: u32 = 256;
    let mut img = image::RgbaImage::new(SIZE, SIZE);
    let center = (SIZE as f32 - 1.0) * 0.5;
    for (x, y, px) in img.enumerate_pixels_mut() {
        let dx = (x as f32 - center) / center;
        let dy = (y as f32 - center) / center;
        let d = (dx * dx + dy * dy).sqrt().min(1.0);
        // Soft core with a long feathered rim so overlaps never show discs.
        let falloff = (1.0 - d * d).powf(2.0);
        let n = fbm(x as f32 * 0.028, y as f32 * 0.028, 4);
        let alpha = falloff * (0.55 + 0.55 * n).clamp(0.0, 1.0);

        // Shading: sun-from-above vertical ramp + noise creases. Kept bright —
        // real launch smoke is blinding white in sunlight; the creases only
        // suggest structure.
        let vertical = 1.0 - 0.18 * ((dy + 1.0) * 0.5); // slightly brighter at top
        let crease = 0.82 + 0.35 * fbm(x as f32 * 0.02 + 37.0, y as f32 * 0.02 + 11.0, 3);
        let shade = (vertical * crease).clamp(0.0, 1.1);
        let v = (shade * 252.0).clamp(0.0, 255.0) as u8;
        *px = image::Rgba([v, v, v, (alpha * 255.0).clamp(0.0, 255.0) as u8]);
    }
    img.save(path)?;
    Ok(())
}

fn hash2(ix: i32, iy: i32) -> f32 {
    let mut h = (ix.wrapping_mul(374_761_393)).wrapping_add(iy.wrapping_mul(668_265_263)) as u32;
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn value_noise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let top = lerp(hash2(ix, iy), hash2(ix + 1, iy), sx);
    let bottom = lerp(hash2(ix, iy + 1), hash2(ix + 1, iy + 1), sx);
    lerp(top, bottom, sy)
}

fn fbm(x: f32, y: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for _ in 0..octaves {
        sum += amp * value_noise(x * freq, y * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.13;
    }
    sum / norm
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

/// Blinding additive HDR core right at the engine cluster. Fast, dense, and
/// stretched along velocity — reads as the continuous white-hot flame column
/// (~30 m at sea level) in every target photo.
///
/// Declares the `intensity` property (1.0 = full throttle): the runtime sets
/// it every frame next to the spawner rate, and the effect wires it into
/// exhaust speed (plume length) and particle brightness, so a throttled
/// engine reads shorter and dimmer, not just thinner. At `intensity = 1.0`
/// the effect is exactly the tuned full-throttle look.
fn merlin_core() -> EffectAsset {
    let writer = ExprWriter::new();
    let intensity = writer.add_property("intensity", 1.0f32.into());

    // Spawn volume sized to a Merlin bell exit (~0.9 m), not the full octaweb.
    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(1.2).expr(),
        base_radius: writer.lit(0.5).expr(),
        top_radius: writer.lit(0.4).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Exhaust along local -Y, fast with a little per-particle variation.
    // Speed scales with throttle: length ~ speed x lifetime.
    let speed = writer.lit(90.0).uniform(writer.lit(130.0))
        * (writer.lit(0.35) + writer.lit(0.65) * writer.prop(intensity));
    let vel = writer.lit(Vec3::NEG_Y) * speed;
    let init_vel = SetAttributeModifier::new(Attribute::VELOCITY, vel.expr());

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.12).uniform(writer.lit(0.3)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Brightness tracks throttle through the per-particle COLOR modulation
    // (identity at full throttle; LDR clamp makes it dim-only).
    let brightness = writer.lit(0.4) + writer.lit(0.6) * writer.prop(intensity);
    let rgba = brightness
        .clone()
        .vec3(brightness.clone(), brightness)
        .vec4_xyz_w(writer.lit(1.0))
        .pack4x8unorm();
    let init_brightness = SetAttributeModifier::new(Attribute::COLOR, rgba.expr());

    let drag = LinearDragModifier::new(writer.lit(0.4).expr());

    // Blinding white-yellow core cooling through orange. Values are far above
    // 1.0 on purpose: the sky is physically lit, so saturating through the
    // exposure + tonemapper takes serious radiance. (Key 0 carries the
    // hand-tuned value from the shipped .effect, not the original authoring
    // guess — keep them in sync when regenerating.)
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(32.0, 28.235298, 20.705883, 1.0));
    color.add_key(0.22, Vec4::new(26.0, 12.0, 2.4, 1.0));
    color.add_key(0.6, Vec4::new(12.0, 3.6, 0.55, 0.6));
    color.add_key(1.0, Vec4::new(3.0, 0.8, 0.1, 0.0));

    // Stretched along velocity (x = along-velocity axis with OrientMode::AlongVelocity).
    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(4.0, 1.0, 1.0));
    size.add_key(0.4, Vec3::new(6.0, 1.4, 1.4));
    size.add_key(1.0, Vec3::new(3.0, 0.75, 0.75));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(16384, SpawnerSettings::rate(2200.0.into()), module)
        .with_name("merlin_core")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .init(init_brightness)
        .update(drag)
        .render(OrientModifier::new(OrientMode::AlongVelocity))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        // Modulate: gradient x per-particle COLOR (identity at intensity 1.0,
        // dims with throttle via the init COLOR write above).
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

/// Orange turbulent flame column surrounding/extending the core. Alpha
/// blended, longer lived, with drag so it billows out and fades. Declares the
/// `intensity` throttle property like `merlin_core` (length + brightness).
fn merlin_flame() -> EffectAsset {
    let writer = ExprWriter::new();
    let intensity = writer.add_property("intensity", 1.0f32.into());

    // Near-parallel column (webcast ascent): tight spawn + distant apex.
    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(1.5).expr(),
        base_radius: writer.lit(0.7).expr(),
        top_radius: writer.lit(0.8).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Diverging cone: velocity radiates from a virtual center far behind the
    // nozzle (+Y). Apex at 26 m → ~2° half-angle (was 9 m / ~9°). Speed scales
    // with throttle (plume length).
    let center = writer.lit(Vec3::new(0.0, 26.0, 0.0));
    let speed = writer.lit(70.0).uniform(writer.lit(110.0))
        * (writer.lit(0.35) + writer.lit(0.65) * writer.prop(intensity));
    let init_vel = SetAttributeModifier::new(
        Attribute::VELOCITY,
        ((writer.attr(Attribute::POSITION) - center).normalized() * speed).expr(),
    );

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.5).uniform(writer.lit(1.4)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Dim with throttle via per-particle COLOR (identity at full throttle).
    let brightness = writer.lit(0.4) + writer.lit(0.6) * writer.prop(intensity);
    let rgba = brightness
        .clone()
        .vec3(brightness.clone(), brightness)
        .vec4_xyz_w(writer.lit(1.0))
        .pack4x8unorm();
    let init_brightness = SetAttributeModifier::new(Attribute::COLOR, rgba.expr());

    let drag = LinearDragModifier::new(writer.lit(1.0).expr());

    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(26.0, 8.5, 0.9, 1.0));
    color.add_key(0.3, Vec4::new(19.0, 5.0, 0.5, 0.85));
    color.add_key(0.7, Vec4::new(8.0, 2.0, 0.28, 0.5));
    color.add_key(1.0, Vec4::new(2.4, 0.7, 0.12, 0.0));

    // Widths ×0.55 so AlongVelocity billboards stay ≈ body diameter.
    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(5.0, 1.9, 1.9));
    size.add_key(0.35, Vec3::new(8.5, 2.9, 2.9));
    size.add_key(1.0, Vec3::new(5.5, 2.0, 2.0));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(16384, SpawnerSettings::rate(1900.0.into()), module)
        .with_name("merlin_flame")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .init(init_brightness)
        .update(drag)
        .render(OrientModifier::new(OrientMode::AlongVelocity))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        // Modulate: gradient x per-particle COLOR (identity at intensity 1.0).
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

/// Map cone-local POSITION (+Y axis) onto `spawn_axis`, then translate by
/// `spawn_origin`. `SetPositionCone3dModifier` is always Y-aligned; without
/// this, a long birth volume stays world-up while the vehicle tips.
fn init_cone_on_spawn_axis(
    writer: &ExprWriter,
    spawn_origin: PropertyHandle,
    spawn_axis: PropertyHandle,
) -> SetAttributeModifier {
    let p = writer.attr(Attribute::POSITION);
    let axis = writer.prop(spawn_axis);
    // cross(axis, Y) vanishes near ±Y — switch the helper to X.
    let use_x = axis.clone().y().abs().step(writer.lit(0.9));
    let helper = writer.lit(Vec3::Y).mix(writer.lit(Vec3::X), use_x);
    let binormal = axis.clone().cross(helper).normalized();
    let tangent = binormal.clone().cross(axis.clone()).normalized();
    let oriented =
        binormal * p.clone().x() + axis * p.clone().y() + tangent * p.z();
    SetAttributeModifier::new(
        Attribute::POSITION,
        (oriented + writer.prop(spawn_origin)).expr(),
    )
}

/// Persistent world-space smoke column, floating-origin-safe edition.
///
/// The effect runs in `SimulationSpace::Local` on a **world-fixed anchor
/// entity**; the moving nozzle is fed in per frame through two vec3
/// properties (the "anchored trail" contract shared with Elodin):
///
/// - `spawn_origin`: nozzle position in the anchor's local frame [m]
/// - `spawn_axis`: unit exhaust direction in the anchor frame
///
/// Runtimes that see these properties on an effect must attach the instance
/// to a world-fixed anchor (pyrotechnique: the world origin; Elodin: a
/// grid-cell entity frozen at ignition) and write the properties every
/// frame. Particles then hang in world space — the trail the `smoke-trail`
/// target shows — without `SimulationSpace::Global`, which cannot survive
/// Elodin's floating-origin rebasing.
fn exhaust_smoke() -> EffectAsset {
    let writer = ExprWriter::new();

    let spawn_origin = writer.add_property("spawn_origin", Vec3::ZERO.into());
    let spawn_axis = writer.add_property("spawn_axis", Vec3::NEG_Y.into());

    // Small birth volume around the (property-driven) nozzle point. The cone
    // is axis-aligned rather than rotated onto `spawn_axis`: its 3 m extent
    // vanishes against 30-110 s lifetimes and 540 m puffs.
    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(3.0).expr(),
        base_radius: writer.lit(1.4).expr(),
        top_radius: writer.lit(2.2).expr(),
        dimension: ShapeDimension::Volume,
    };
    let offset_pos = SetAttributeModifier::new(
        Attribute::POSITION,
        (writer.attr(Attribute::POSITION) + writer.prop(spawn_origin)).expr(),
    );

    // Diverging cone: radiate from a virtual center 5 m up-plume of the
    // nozzle, so the trail widens downstream exactly like the old
    // Global-space SetVelocitySphereModifier version.
    let center = writer.prop(spawn_origin) - writer.prop(spawn_axis) * writer.lit(5.0);
    let speed = writer.lit(20.0).uniform(writer.lit(36.0));
    let init_vel = SetAttributeModifier::new(
        Attribute::VELOCITY,
        ((writer.attr(Attribute::POSITION) - center).normalized() * speed).expr(),
    );

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // Wide lifetime spread desynchronizes the growth curve along the column,
    // breaking up the "uniform tube" look into billows.
    let lifetime = writer.lit(30.0).uniform(writer.lit(110.0)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Strong per-puff brightness/opacity variation -> cauliflower lumps.
    let init_modulation = init_random_modulation(&writer, 0.72, 1.0, 0.25, 1.0);

    // Slow to a hang, with gentle buoyancy + wind drift.
    let drag = LinearDragModifier::new(writer.lit(0.5).expr());
    let accel = AccelModifier::new(writer.lit(Vec3::new(3.0, 1.0, 0.8)).expr());

    let mut color = Gradient::new();
    // Fresh exhaust is flame-lit warm white, then sunlit white, then thins.
    // Alpha stays low near birth so the orange flame column shows through.
    // RGB sits above 1.0 to compensate for the baked texture shading.
    color.add_key(0.0, Vec4::new(2.2, 1.8, 1.3, 0.12));
    color.add_key(0.05, Vec4::new(1.55, 1.45, 1.35, 0.42));
    color.add_key(0.4, Vec4::new(1.38, 1.38, 1.42, 0.6));
    color.add_key(1.0, Vec4::new(1.25, 1.25, 1.3, 0.0));

    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::splat(10.0));
    size.add_key(0.08, Vec3::splat(36.0));
    size.add_key(0.45, Vec3::splat(170.0));
    size.add_key(1.0, Vec3::splat(540.0));

    let smoke_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("smoke");

    EffectAsset::new(65536, SpawnerSettings::rate(220.0.into()), module)
        .with_name("exhaust_smoke")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(offset_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .init(init_modulation)
        .update(drag)
        .update(accel)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(ParticleTextureModifier {
            texture_slot: smoke_slot,
            sample_mapping: ImageSampleMapping::Modulate,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        // Modulate: gradient x per-particle random COLOR from init.
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

// ---------------------------------------------------------------------------
// apollo-lander effects
// ---------------------------------------------------------------------------

/// LM descent engine plume in vacuum, core layer: a solid, neutral-cool white
/// column that fills the full bell mouth — matched against the "First Man"
/// main-engine close-up. Thousands of short-lived overlapping streaks fuse
/// into a continuous tube; `descent_glow` adds the camera-facing halo.
///
/// Metric contract: sized for the LM GLB at its native ~5.0 m height (what the
/// Elodin sim renders); see docs/design-thruster-effects-port.md §4.1.
///
/// Spawn is **downstream of the bell exit** (not inside the nozzle). Birth
/// AlongVelocity sprites are ~1 m long; spawning inside the bell hid half of
/// each sprite behind the hull, and when the vehicle is pitched that
/// one-sided occlusion shears the bright envelope off the geometric axis
/// (F0b). Shifting the cone clear of the wall keeps every sprite fully
/// visible and coaxial with the bell at any attitude.
fn descent_plume() -> EffectAsset {
    let writer = ExprWriter::new();

    // Compact spawn disk just outside the exit plane. Emitter origin in the
    // KDL is ~0.2 m inside the bell; -0.70 m puts the disk ~0.5 m past the
    // lip so a 0.7 m birth half-length clears the nozzle wall.
    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(0.12).expr(),
        base_radius: writer.lit(0.26).expr(),
        top_radius: writer.lit(0.22).expr(),
        dimension: ShapeDimension::Volume,
    };
    let shifted = writer.attr(Attribute::POSITION) + writer.lit(Vec3::new(0.0, -0.70, 0.0));
    let init_shift = SetAttributeModifier::new(Attribute::POSITION, shifted.expr());

    // Strictly parallel exhaust (not a velocity sphere). A sphere-center spray
    // gives each AlongVelocity streak its own yaw; under pitch the bright
    // envelope of those angled streaks reads as a lean off the bell axis (F0b).
    // Parallel -Y keeps every sprite coaxial with the geometric exhaust.
    let speed = writer.lit(20.0).uniform(writer.lit(28.0));
    let init_vel = SetAttributeModifier::new(
        Attribute::VELOCITY,
        (writer.lit(Vec3::NEG_Y) * speed).expr(),
    );

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // Narrow lifetime spread: a wide spread leaves individual streak tips
    // hanging at random depths (ragged fiber look); the fade-out gradient
    // shapes the column tail instead.
    let lifetime = writer.lit(0.10).uniform(writer.lit(0.16)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Neutral-cool white (hypergolic vacuum plume, movie-graded): brightest
    // slightly blue at birth, cooling to translucent blue-gray. Per-sprite
    // alpha is low; the solid look comes from overlap count.
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(9.0, 9.6, 11.0, 0.19));
    color.add_key(0.25, Vec4::new(6.5, 7.0, 8.4, 0.11));
    color.add_key(0.6, Vec4::new(2.4, 2.7, 3.5, 0.035));
    color.add_key(1.0, Vec4::new(0.8, 0.95, 1.3, 0.0));

    // Slightly shorter birth length than the in-bell version so the first
    // visible segment sits fully past the lip; across-width still holds so
    // the sides stay near-parallel like the movie column.
    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(0.7, 0.22, 0.22));
    size.add_key(0.15, Vec3::new(1.3, 0.50, 0.50));
    size.add_key(1.0, Vec3::new(1.6, 0.58, 0.58));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(32768, SpawnerSettings::rate(22000.0.into()), module)
        .with_name("descent_plume")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_shift)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .render(OrientModifier::new(OrientMode::AlongVelocity))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
}

/// LM descent plume, halo layer: camera-facing soft circles **distributed
/// along the column axis** (not a single fat mouth blob). A mouth-anchored
/// isotropic blob cannot encode the bell axis under pitch (F0b); a train of
/// billboards spanning the visible column tilts rigidly with the vehicle.
///
/// Still FaceCamera so azimuth uniformity from F0 is preserved. Stacked on
/// the same emitter as `descent_plume`.
fn descent_glow() -> EffectAsset {
    let writer = ExprWriter::new();

    // Tall thin spawn volume along -Y covering the visible column (~3.5 m).
    // Cone height is along +Y from the (shifted) base; after the -3.6 m shift
    // particles occupy roughly y ∈ [-3.6, -0.1] in emitter space.
    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(3.5).expr(),
        base_radius: writer.lit(0.30).expr(),
        top_radius: writer.lit(0.38).expr(),
        dimension: ShapeDimension::Volume,
    };
    let shifted = writer.attr(Attribute::POSITION) + writer.lit(Vec3::new(0.0, -3.6, 0.0));
    let init_shift = SetAttributeModifier::new(Attribute::POSITION, shifted.expr());

    // Parallel down-axis drift (same F0b rationale as the core): a radial
    // velocity sphere would smear FaceCamera billboards off the bell axis.
    let speed = writer.lit(4.0).uniform(writer.lit(7.0));
    let init_vel = SetAttributeModifier::new(
        Attribute::VELOCITY,
        (writer.lit(Vec3::NEG_Y) * speed).expr(),
    );

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.22).uniform(writer.lit(0.36)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Low alpha, cool white: pure additive fill. Per-sprite energy kept
    // modest — density along the column replaces a single bright mouth blob.
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(5.0, 5.5, 6.8, 0.045));
    color.add_key(0.5, Vec4::new(2.2, 2.6, 3.4, 0.022));
    color.add_key(1.0, Vec4::new(0.8, 0.95, 1.3, 0.0));

    // Cap near column diameter so the halo cannot outvote the core axis
    // under pitch (old peak 2.5 m was ~2x the column and read as lean).
    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::splat(0.7));
    size.add_key(0.5, Vec3::splat(1.1));
    size.add_key(1.0, Vec3::splat(1.4));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(4096, SpawnerSettings::rate(1800.0.into()), module)
        .with_name("descent_glow")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_shift)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
}

/// Falcon 9 cold-gas RCS dart: same look as apollo `rcs_puff` but sized for a
/// 70 m booster at chase-camera distance (~4× apparent area: longer life,
/// faster spray, larger sprites, slightly brighter).
fn rcs_dart() -> EffectAsset {
    let writer = ExprWriter::new();

    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(0.25).expr(),
        base_radius: writer.lit(0.14).expr(),
        top_radius: writer.lit(0.1).expr(),
        dimension: ShapeDimension::Volume,
    };

    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::new(0.0, 1.4, 0.0)).expr(),
        speed: writer.lit(18.0).uniform(writer.lit(32.0)).expr(),
    };

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.35).uniform(writer.lit(0.85)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(14.0, 15.5, 19.0, 1.0));
    color.add_key(0.35, Vec4::new(7.0, 8.0, 11.0, 0.65));
    color.add_key(1.0, Vec4::new(1.8, 2.1, 3.0, 0.0));

    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(1.6, 0.45, 0.45));
    size.add_key(0.4, Vec3::new(3.2, 0.85, 0.85));
    size.add_key(1.0, Vec3::new(2.2, 0.6, 0.6));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(4096, SpawnerSettings::rate(1200.0.into()), module)
        .with_name("rcs_dart")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .render(OrientModifier::new(OrientMode::AlongVelocity))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
}

/// RCS quad puff: a small, sharp white-blue dart. Fired in short pulses via
/// the emitter's `activity` keyframes in the scene.
fn rcs_puff() -> EffectAsset {
    let writer = ExprWriter::new();

    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(0.06).expr(),
        base_radius: writer.lit(0.045).expr(),
        top_radius: writer.lit(0.035).expr(),
        dimension: ShapeDimension::Volume,
    };

    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::new(0.0, 0.36, 0.0)).expr(),
        speed: writer.lit(6.5).uniform(writer.lit(11.5)).expr(),
    };

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.08).uniform(writer.lit(0.28)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Cold-gas white with a blue cast, vanishing quickly.
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(9.0, 10.0, 12.5, 0.9));
    color.add_key(0.35, Vec4::new(4.5, 5.2, 7.0, 0.5));
    color.add_key(1.0, Vec4::new(1.2, 1.4, 2.0, 0.0));

    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(0.4, 0.11, 0.11));
    size.add_key(0.4, Vec3::new(0.8, 0.21, 0.21));
    size.add_key(1.0, Vec3::new(0.57, 0.16, 0.16));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(4096, SpawnerSettings::rate(600.0.into()), module)
        .with_name("rcs_puff")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .render(OrientModifier::new(OrientMode::AlongVelocity))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
}

/// Lunar regolith blast: no air, so dust doesn't billow — it sprays outward
/// in a flat ballistic sheet of streaks hugging the surface, arcing back
/// down under lunar gravity. Attached to a *static* landing-site emitter;
/// `SimulationSpace::Local` on a static emitter is world-fixed in practice
/// and stays safe under floating-origin rebasing (Elodin big_space).
fn ground_dust() -> EffectAsset {
    let writer = ExprWriter::new();

    // Ring just above the surface under the engine.
    let init_pos = SetPositionCircleModifier {
        center: writer.lit(Vec3::new(0.0, 0.1, 0.0)).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        radius: writer.lit(1.0).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Flat radial spray, fast: entrained grains leave at tens of m/s.
    let init_vel = SetVelocityCircleModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        speed: writer.lit(5.0).uniform(writer.lit(16.0)).expr(),
    };

    // Small random upward component so the sheet has a little thickness.
    let up_kick = writer.rand(ScalarType::Float) * writer.lit(1.2);
    let kicked = writer.attr(Attribute::VELOCITY) + writer.lit(0.0).vec3(up_kick, writer.lit(0.0));
    let init_up_kick = SetAttributeModifier::new(Attribute::VELOCITY, kicked.expr());

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.5).uniform(writer.lit(1.3)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Grain-to-grain brightness variation reads as streaky spray.
    let init_modulation = init_random_modulation(&writer, 0.55, 1.0, 0.3, 1.0);

    // Lunar gravity only — no drag in vacuum.
    let gravity = AccelModifier::new(writer.lit(Vec3::new(0.0, -1.62, 0.0)).expr());

    // Kill grains that fall back through the surface.
    let kill_ground = KillAabbModifier::new(
        writer.lit(Vec3::new(0.0, -1000.0, 0.0)).expr(),
        writer.lit(Vec3::new(1.0e6, 1000.0, 1.0e6)).expr(),
    )
    .with_kill_inside(true);

    // Sunlit gray regolith; alpha carries the look (reflective, not emissive).
    // Bright enough to read against high-albedo terrain (the Elodin landing
    // site renders near-white at correct exposure).
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(1.9, 1.8, 1.62, 0.55));
    color.add_key(0.4, Vec4::new(1.55, 1.48, 1.36, 0.4));
    color.add_key(1.0, Vec4::new(1.2, 1.15, 1.1, 0.0));

    // Stretched along velocity: fine streaks, not puffs.
    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(0.7, 0.1, 0.1));
    size.add_key(0.4, Vec3::new(1.6, 0.21, 0.21));
    size.add_key(1.0, Vec3::new(2.3, 0.32, 0.32));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    // Local space on a static emitter == world-fixed (see doc comment).
    EffectAsset::new(32768, SpawnerSettings::rate(4000.0.into()), module)
        .with_name("ground_dust")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(init_vel)
        .init(init_up_kick)
        .init(init_age)
        .init(init_lifetime)
        .init(init_modulation)
        .update(gravity)
        .update(kill_ground)
        .render(OrientModifier::new(OrientMode::AlongVelocity))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        // Modulate: gradient x per-particle random COLOR from init.
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

// ---------------------------------------------------------------------------
// rocket effects (2 m high-power model rocket, 6 s solid-motor boost)
// ---------------------------------------------------------------------------

/// Solid-motor core: blinding white-yellow additive column right at the
/// nozzle, ~1.5-2.5 m long on a 2 m airframe (the initial-boost target shows
/// the bright column at roughly a vehicle length). Same `intensity` throttle
/// contract as `merlin_core` (length + brightness), identity at 1.0.
fn motor_core() -> EffectAsset {
    let writer = ExprWriter::new();
    let intensity = writer.add_property("intensity", 1.0f32.into());

    // Spawn volume sized to a ~5 cm motor nozzle.
    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(0.06).expr(),
        base_radius: writer.lit(0.03).expr(),
        top_radius: writer.lit(0.025).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Exhaust along local -Y; length ~ speed x lifetime, throttle-scaled.
    let speed = writer.lit(28.0).uniform(writer.lit(40.0))
        * (writer.lit(0.35) + writer.lit(0.65) * writer.prop(intensity));
    let vel = writer.lit(Vec3::NEG_Y) * speed;
    let init_vel = SetAttributeModifier::new(Attribute::VELOCITY, vel.expr());

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.06).uniform(writer.lit(0.11)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Dim with throttle via per-particle COLOR (identity at full throttle).
    let brightness = writer.lit(0.4) + writer.lit(0.6) * writer.prop(intensity);
    let rgba = brightness
        .clone()
        .vec3(brightness.clone(), brightness)
        .vec4_xyz_w(writer.lit(1.0))
        .pack4x8unorm();
    let init_brightness = SetAttributeModifier::new(Attribute::COLOR, rgba.expr());

    let drag = LinearDragModifier::new(writer.lit(0.5).expr());

    // White-hot at the nozzle cooling through yellow-orange (APCP motor).
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(34.0, 30.0, 21.0, 1.0));
    color.add_key(0.25, Vec4::new(28.0, 14.0, 2.8, 0.9));
    color.add_key(0.6, Vec4::new(13.0, 4.2, 0.6, 0.55));
    color.add_key(1.0, Vec4::new(3.2, 0.9, 0.1, 0.0));

    // Stretched along velocity (x = along-velocity axis).
    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(0.28, 0.08, 0.08));
    size.add_key(0.4, Vec3::new(0.45, 0.115, 0.115));
    size.add_key(1.0, Vec3::new(0.22, 0.06, 0.06));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(4096, SpawnerSettings::rate(3200.0.into()), module)
        .with_name("motor_core")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .init(init_brightness)
        .update(drag)
        .render(OrientModifier::new(OrientMode::AlongVelocity))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        // Modulate: gradient x per-particle COLOR (identity at intensity 1.0).
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

/// Orange flame body around/extending the core: near-parallel column
/// (virtual apex 0.8 m up-plume, ~2-3 deg half-angle) fading into the smoke
/// trail ~3-5 m behind the nozzle. `intensity` contract like `merlin_flame`.
fn motor_flame() -> EffectAsset {
    let writer = ExprWriter::new();
    let intensity = writer.add_property("intensity", 1.0f32.into());

    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(0.08).expr(),
        base_radius: writer.lit(0.04).expr(),
        top_radius: writer.lit(0.045).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Diverging cone: velocity radiates from a virtual center up-plume (+Y).
    let center = writer.lit(Vec3::new(0.0, 0.8, 0.0));
    let speed = writer.lit(18.0).uniform(writer.lit(28.0))
        * (writer.lit(0.35) + writer.lit(0.65) * writer.prop(intensity));
    let init_vel = SetAttributeModifier::new(
        Attribute::VELOCITY,
        ((writer.attr(Attribute::POSITION) - center).normalized() * speed).expr(),
    );

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.06).uniform(writer.lit(0.13)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    let brightness = writer.lit(0.4) + writer.lit(0.6) * writer.prop(intensity);
    let rgba = brightness
        .clone()
        .vec3(brightness.clone(), brightness)
        .vec4_xyz_w(writer.lit(1.0))
        .pack4x8unorm();
    let init_brightness = SetAttributeModifier::new(Attribute::COLOR, rgba.expr());

    let drag = LinearDragModifier::new(writer.lit(1.2).expr());

    // Saturated orange body (the initial-boost target column).
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(24.0, 8.0, 0.9, 1.0));
    color.add_key(0.3, Vec4::new(17.0, 4.6, 0.5, 0.8));
    color.add_key(0.7, Vec4::new(7.0, 1.8, 0.25, 0.45));
    color.add_key(1.0, Vec4::new(2.2, 0.65, 0.12, 0.0));

    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(0.3, 0.14, 0.14));
    size.add_key(0.35, Vec3::new(0.55, 0.24, 0.24));
    size.add_key(1.0, Vec3::new(0.38, 0.16, 0.16));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(4096, SpawnerSettings::rate(2600.0.into()), module)
        .with_name("motor_flame")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .init(init_brightness)
        .update(drag)
        .render(OrientModifier::new(OrientMode::AlongVelocity))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        // Modulate: gradient x per-particle COLOR (identity at intensity 1.0).
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

/// Persistent boost trail: the dense cream-white column the mid/late-boost
/// targets show hanging from vehicle to pad. Anchored-trail contract
/// (`spawn_origin`/`spawn_axis`) exactly like falcon9 `exhaust_smoke`, sized
/// down for a 2 m vehicle: puffs a few tens of cm at birth growing to ~20 m
/// as the trail disperses.
fn boost_trail() -> EffectAsset {
    let writer = ExprWriter::new();

    let spawn_origin = writer.add_property("spawn_origin", Vec3::ZERO.into());
    let spawn_axis = writer.add_property("spawn_axis", Vec3::NEG_Y.into());

    // Birth volume stretched ~5 m along spawn_axis (exhaust): per-frame spawn
    // batches land at one nozzle pose, so the axial spread must cover the
    // vehicle's inter-frame motion (250+ m/s at 60 fps ~= 4.2 m) or the trail
    // beads up. Narrow near the vehicle (top), wider downstream (base).
    // Cone modifier is Y-fixed — reorient onto spawn_axis so tip-over does not
    // leave a world-up sausage beside the flame.
    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(5.0).expr(),
        base_radius: writer.lit(0.35).expr(),
        top_radius: writer.lit(0.12).expr(),
        dimension: ShapeDimension::Volume,
    };
    let offset_pos = init_cone_on_spawn_axis(&writer, spawn_origin, spawn_axis);

    // Diverge from a virtual center just up-plume of the nozzle so the trail
    // widens downstream.
    let center = writer.prop(spawn_origin) - writer.prop(spawn_axis) * writer.lit(0.6);
    let speed = writer.lit(8.0).uniform(writer.lit(16.0));
    let init_vel = SetAttributeModifier::new(
        Attribute::VELOCITY,
        ((writer.attr(Attribute::POSITION) - center).normalized() * speed).expr(),
    );

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    // Long lifetimes: the trail must persist for the whole flight; the wide
    // spread desynchronizes growth into billows instead of a uniform tube.
    let lifetime = writer.lit(18.0).uniform(writer.lit(70.0)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Strong per-puff variation -> cauliflower lumps.
    let init_modulation = init_random_modulation(&writer, 0.7, 1.0, 0.3, 1.0);

    // Slow to a hang, then gentle wind drift bends the old trail.
    let drag = LinearDragModifier::new(writer.lit(1.1).expr());
    let accel = AccelModifier::new(writer.lit(Vec3::new(0.3, 0.25, 0.12)).expr());

    let mut color = Gradient::new();
    // Brief flame-lit warmth at birth, then bright sunlit cream (AP smoke)
    // aging toward dusty tan. Alpha ramps in fast (sub-second): the fresh
    // trail right behind the flame is already dense in the targets.
    color.add_key(0.0, Vec4::new(1.85, 1.68, 1.45, 0.28));
    color.add_key(0.002, Vec4::new(1.66, 1.56, 1.4, 0.62));
    color.add_key(0.35, Vec4::new(1.5, 1.44, 1.3, 0.74));
    color.add_key(1.0, Vec4::new(1.28, 1.22, 1.12, 0.0));

    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::splat(0.9));
    size.add_key(0.012, Vec3::splat(2.0));
    size.add_key(0.1, Vec3::splat(4.6));
    size.add_key(0.45, Vec3::splat(10.0));
    size.add_key(1.0, Vec3::splat(20.0));

    let smoke_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("smoke");

    EffectAsset::new(65536, SpawnerSettings::rate(560.0.into()), module)
        .with_name("boost_trail")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(offset_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .init(init_modulation)
        .update(drag)
        .update(accel)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(ParticleTextureModifier {
            texture_slot: smoke_slot,
            sample_mapping: ImageSampleMapping::Modulate,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        // Modulate: gradient x per-particle random COLOR from init.
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

/// Lift-off ground cloud: exhaust splashing off the pad into a low white
/// billow a few meters across (initial-boost target bottom). Static pad
/// emitter, so `SimulationSpace::Local` is world-fixed in practice.
fn launch_smoke() -> EffectAsset {
    let writer = ExprWriter::new();

    // Small disc at pad level.
    let init_pos = SetPositionCircleModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        radius: writer.lit(1.0).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Radial outward blast; buoyancy rolls the sheets up into billows.
    let init_vel = SetVelocityCircleModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        speed: writer.lit(3.0).uniform(writer.lit(10.0)).expr(),
    };

    // Random upward kick so billow tops crest at different heights.
    let up_kick = writer.rand(ScalarType::Float) * writer.lit(4.0);
    let kicked = writer.attr(Attribute::VELOCITY) + writer.lit(0.0).vec3(up_kick, writer.lit(0.0));
    let init_up_kick = SetAttributeModifier::new(Attribute::VELOCITY, kicked.expr());

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(5.0).uniform(writer.lit(14.0)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    let init_modulation = init_random_modulation(&writer, 0.6, 1.0, 0.45, 1.0);

    let drag = LinearDragModifier::new(writer.lit(0.9).expr());
    let buoyancy = AccelModifier::new(writer.lit(Vec3::new(0.0, 1.8, 0.0)).expr());

    // Kill anything dipping below the pad surface (emitter sits ~0.4 m up).
    let kill_ground = KillAabbModifier::new(
        writer.lit(Vec3::new(0.0, -1000.4, 0.0)).expr(),
        writer.lit(Vec3::new(1.0e6, 1000.0, 1.0e6)).expr(),
    )
    .with_kill_inside(true);

    let mut color = Gradient::new();
    // Bright sunlit white with a brief warm flame tint at birth.
    color.add_key(0.0, Vec4::new(3.2, 2.5, 1.7, 0.85));
    color.add_key(0.25, Vec4::new(2.0, 1.8, 1.6, 0.62));
    color.add_key(0.6, Vec4::new(1.5, 1.45, 1.42, 0.38));
    color.add_key(1.0, Vec4::new(1.2, 1.2, 1.22, 0.0));

    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::splat(2.5));
    size.add_key(0.08, Vec3::splat(5.0));
    size.add_key(0.5, Vec3::splat(9.5));
    size.add_key(1.0, Vec3::splat(14.0));

    let smoke_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("smoke");

    // Local space on a static emitter == world-fixed (see doc comment).
    EffectAsset::new(4096, SpawnerSettings::rate(130.0.into()), module)
        .with_name("launch_smoke")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(init_vel)
        .init(init_up_kick)
        .init(init_age)
        .init(init_lifetime)
        .init(init_modulation)
        .update(drag)
        .update(buoyancy)
        .update(kill_ground)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(ParticleTextureModifier {
            texture_slot: smoke_slot,
            sample_mapping: ImageSampleMapping::Modulate,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        // Modulate: gradient x per-particle random COLOR from init.
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

/// Lift-off ground clouds: big, slow, buoyant billows fed by the deflected
/// exhaust. Attached to a *static* pad emitter, so `SimulationSpace::Local`
/// is world-fixed in practice (and floating-origin-safe for the Elodin port).
fn pad_smoke() -> EffectAsset {
    let writer = ExprWriter::new();

    // Flat disc at pad level: the flame trench deflects exhaust sideways.
    let init_pos = SetPositionCircleModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        radius: writer.lit(16.0).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Radial-in-plane blast outward from the pad center; buoyancy then rolls
    // the sheets up into billows. Speeds sized so the clouds reach ~100 m at
    // lift-off framing without swallowing the pad camera.
    let init_vel = SetVelocityCircleModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        speed: writer.lit(8.0).uniform(writer.lit(38.0)).expr(),
    };

    // Small random upward kick so billow tops crest at different heights.
    let up_kick = writer.rand(ScalarType::Float) * writer.lit(5.0);
    let kicked = writer.attr(Attribute::VELOCITY) + writer.lit(0.0).vec3(up_kick, writer.lit(0.0));
    let init_up_kick = SetAttributeModifier::new(Attribute::VELOCITY, kicked.expr());

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(8.0).uniform(writer.lit(20.0)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Puff-to-puff variation is what sells "billowing clouds" vs fog.
    let init_modulation = init_random_modulation(&writer, 0.6, 1.0, 0.45, 1.0);

    let drag = LinearDragModifier::new(writer.lit(1.0).expr());
    let buoyancy = AccelModifier::new(writer.lit(Vec3::new(0.0, 2.2, 0.0)).expr());

    // Kill anything that dips below the pad surface. Local frame: the pad
    // emitter sits at world y=2, so the ground plane is local y=-2.
    let kill_ground = KillAabbModifier::new(
        writer.lit(Vec3::new(0.0, -1002.0, 0.0)).expr(),
        writer.lit(Vec3::new(1.0e6, 1000.0, 1.0e6)).expr(),
    )
    .with_kill_inside(true);

    let mut color = Gradient::new();
    // Golden flame-lit clouds (targets show flame-tinted pad smoke). Real
    // clouds stay lit by the flame for as long as the rocket is near, so the
    // warm tint holds through most of the particle's life.
    color.add_key(0.0, Vec4::new(7.5, 4.3, 1.5, 0.9));
    color.add_key(0.3, Vec4::new(4.2, 2.75, 1.5, 0.72));
    color.add_key(0.65, Vec4::new(2.0, 1.7, 1.45, 0.42));
    color.add_key(1.0, Vec4::new(1.25, 1.25, 1.28, 0.0));

    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::splat(18.0));
    size.add_key(0.2, Vec3::splat(42.0));
    size.add_key(0.6, Vec3::splat(80.0));
    size.add_key(1.0, Vec3::splat(110.0));

    let smoke_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("smoke");

    // Local space on a static emitter == world-fixed (see doc comment).
    EffectAsset::new(32768, SpawnerSettings::rate(90.0.into()), module)
        .with_name("pad_smoke")
        .with_simulation_space(SimulationSpace::Local)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(init_vel)
        .init(init_up_kick)
        .init(init_age)
        .init(init_lifetime)
        .init(init_modulation)
        .update(drag)
        .update(buoyancy)
        .update(kill_ground)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(ParticleTextureModifier {
            texture_slot: smoke_slot,
            sample_mapping: ImageSampleMapping::Modulate,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        // Modulate: gradient x per-particle random COLOR from init.
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

// ---------------------------------------------------------------------------
// Satellite (LEO / vacuum): once-burst fields, Local space, no drag.
// ---------------------------------------------------------------------------

const EARTH_R: f32 = 6_378_140.0;
const STAR_RADIUS: f32 = 15_000_000.0;
const VACUUM_LIFETIME: f32 = 1.0e9;

fn power_law_mag(writer: &ExprWriter) -> WriterExpr {
    let u = writer.rand(ScalarType::Float);
    u.clone() * u.clone() * u.clone() * u.sqrt()
}

fn init_age_lifetime(
    writer: &ExprWriter,
    lifetime: WriterExpr,
) -> (SetAttributeModifier, SetAttributeModifier) {
    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime.expr());
    (init_age, init_lifetime)
}

fn init_zero_velocity(writer: &ExprWriter) -> SetAttributeModifier {
    SetAttributeModifier::new(Attribute::VELOCITY, writer.lit(Vec3::ZERO).expr())
}

fn packed_scale(writer: &ExprWriter, scale: WriterExpr) -> ExprHandle {
    scale
        .clone()
        .vec3(scale.clone(), scale)
        .vec4_xyz_w(writer.lit(1.0))
        .pack4x8unorm()
        .expr()
}

fn star_sphere(writer: &ExprWriter) -> SetPositionSphereModifier {
    SetPositionSphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        radius: writer.lit(STAR_RADIUS).expr(),
        dimension: ShapeDimension::Surface,
    }
}

fn earth_shell(writer: &ExprWriter, radius: f32) -> SetPositionSphereModifier {
    SetPositionSphereModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        radius: writer.lit(radius).expr(),
        dimension: ShapeDimension::Surface,
    }
}

fn thicken_shell(writer: &ExprWriter, radius: f32, thickness: f32) -> SetAttributeModifier {
    let n = writer.attr(Attribute::POSITION).normalized();
    let r = writer
        .lit(radius - thickness * 0.5)
        .uniform(writer.lit(radius + thickness * 0.5));
    SetAttributeModifier::new(Attribute::POSITION, (n * r).expr())
}

fn night_and_limb(
    writer: &ExprWriter,
    sun_dir: PropertyHandle,
    view_pos: PropertyHandle,
    limb_mu: f32,
    disc_sharp: f32,
    space_sharp: f32,
) -> WriterExpr {
    let n = writer.attr(Attribute::POSITION).normalized();
    let sun = writer.prop(sun_dir).normalized();
    let night = (writer.lit(0.08) - n.clone().dot(sun)).saturate();
    // Peak at the geometric limb. Kill the Earth disc hard; fade softly into space.
    let mu = n.dot(writer.prop(view_pos).normalized());
    let d = mu - writer.lit(limb_mu);
    let toward_disc = d.clone().max(writer.lit(0.0));
    let toward_space = (writer.lit(0.0) - d).max(writer.lit(0.0));
    let limb = (writer.lit(1.0)
        - toward_disc * writer.lit(disc_sharp)
        - toward_space * writer.lit(space_sharp))
    .saturate();
    night * limb
}

/// World size that subtends `pixels` at [`STAR_RADIUS`] (~55° / 900 px).
/// Hanabi 0.20 stores `screen_space_size` but never applies it.
fn star_world_size(pixels: f32) -> f32 {
    STAR_RADIUS * 0.001067 * pixels
}

fn star_field(
    name: &str,
    capacity: u32,
    count: f32,
    pixel_size: f32,
    hdr: Vec4,
    color_vary: bool,
) -> EffectAsset {
    let writer = ExprWriter::new();
    let intensity = writer.add_property("intensity", 1.0f32.into());

    let init_pos = star_sphere(&writer);
    let init_vel = init_zero_velocity(&writer);
    let mag = power_law_mag(&writer);
    let init_mag = SetAttributeModifier::new(Attribute::F32_0, mag.expr());
    let tint = writer.rand(ScalarType::Float);
    let init_tint = SetAttributeModifier::new(Attribute::F32_1, tint.expr());
    let (init_age, init_lifetime) = init_age_lifetime(&writer, writer.lit(VACUUM_LIFETIME));

    let scale = writer.attr(Attribute::F32_0) * writer.prop(intensity);
    let update_color = if color_vary {
        let t = writer.attr(Attribute::F32_1);
        let r = (writer.lit(0.72) + writer.lit(0.4) * (writer.lit(1.0) - t.clone())) * scale.clone();
        let g = writer.lit(0.86) * scale.clone();
        let b = (writer.lit(0.7) + writer.lit(0.45) * t) * scale;
        SetAttributeModifier::new(
            Attribute::COLOR,
            r.vec3(g, b).vec4_xyz_w(writer.lit(1.0)).pack4x8unorm().expr(),
        )
    } else {
        SetAttributeModifier::new(Attribute::COLOR, packed_scale(&writer, scale))
    };

    let mut size = Gradient::new();
    let world_size = star_world_size(pixel_size);
    size.add_key(0.0, Vec3::splat(world_size));
    size.add_key(1.0, Vec3::splat(world_size));

    let mut color = Gradient::new();
    color.add_key(0.0, hdr);
    color.add_key(1.0, hdr);

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(capacity, SpawnerSettings::once(count.into()), module)
        .with_name(name)
        .with_simulation_space(SimulationSpace::Local)
        .with_simulation_condition(SimulationCondition::Always)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_mag)
        .init(init_tint)
        .init(init_age)
        .init(init_lifetime)
        .update(update_color)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

fn stars_dim() -> EffectAsset {
    star_field(
        "stars_dim",
        800_000,
        800_000.0,
        0.55,
        Vec4::new(14.0, 14.0, 18.0, 1.0),
        false,
    )
}

fn stars_bright() -> EffectAsset {
    star_field(
        "stars_bright",
        40_000,
        40_000.0,
        1.5,
        Vec4::new(48.0, 40.0, 62.0, 1.0),
        true,
    )
}

fn milky_way() -> EffectAsset {
    let writer = ExprWriter::new();
    let intensity = writer.add_property("intensity", 1.0f32.into());

    let init_pos = star_sphere(&writer);
    let init_vel = init_zero_velocity(&writer);
    let mag = power_law_mag(&writer);
    let init_mag = SetAttributeModifier::new(Attribute::F32_0, mag.expr());

    let n = writer.attr(Attribute::POSITION).normalized();
    let pole = writer.lit(Vec3::new(0.18, 0.92, 0.35).normalize());
    let keep = writer.lit(0.22).step(n.dot(pole).abs());
    let (init_age, init_lifetime) = init_age_lifetime(&writer, writer.lit(VACUUM_LIFETIME) * keep);

    let scale = writer.attr(Attribute::F32_0) * writer.prop(intensity);
    let update_color = SetAttributeModifier::new(Attribute::COLOR, packed_scale(&writer, scale));

    let mut size = Gradient::new();
    let mw_size = star_world_size(0.95);
    size.add_key(0.0, Vec3::splat(mw_size));
    size.add_key(1.0, Vec3::splat(mw_size));

    let mut color = Gradient::new();
    let hdr = Vec4::new(11.0, 8.5, 6.0, 0.85);
    color.add_key(0.0, hdr);
    color.add_key(1.0, hdr);

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(400_000, SpawnerSettings::once(400_000.0.into()), module)
        .with_name("milky_way")
        .with_simulation_space(SimulationSpace::Local)
        .with_simulation_condition(SimulationCondition::Always)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
        .init(init_pos)
        .init(init_vel)
        .init(init_mag)
        .init(init_age)
        .init(init_lifetime)
        .update(update_color)
        .render(OrientModifier::new(OrientMode::FaceCameraPosition))
        .render(ParticleTextureModifier {
            texture_slot: mask_slot,
            sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
        })
        .render(SizeOverLifetimeModifier {
            gradient: size,
            screen_space_size: false,
        })
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Modulate,
            mask: ColorBlendMask::RGBA,
        })
}

fn city_lights() -> EffectAsset {
    let writer = ExprWriter::new();
    let sun_dir = writer.add_property("sun_dir", Vec3::Y.into());
    let _view_pos = writer.add_property("view_pos", Vec3::new(0.0, 6_778_140.0, 0.0).into());
    let intensity = writer.add_property("intensity", 0.0f32.into());

    let cdf_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/textures/earth/city_tile_cdf.bin");
    let init_pos = crate::effects::city_tile_cdf::CityTileCdfModifier::from_bin(
        &cdf_path,
        EARTH_R + 8_000.0,
    )
    .unwrap_or_else(|err| panic!("city_tile_cdf.bin ({cdf_path:?}): {err}"));
    let init_vel = init_zero_velocity(&writer);
    // Geography is the Black Marble sample. Tiny mag jitter only — wide mag
    // packed into 8-bit COLOR was the limb sparkle.
    let mag = writer.lit(0.96) + writer.lit(0.04) * writer.rand(ScalarType::Float);
    let init_mag = SetAttributeModifier::new(Attribute::F32_0, mag.expr());
    let (init_age, init_lifetime) = init_age_lifetime(&writer, writer.lit(VACUUM_LIFETIME));

    let n = writer.attr(Attribute::POSITION).normalized();
    let sun = writer.prop(sun_dir).normalized();
    let night = (writer.lit(0.08) - n.dot(sun)).saturate();
    let scale = writer.attr(Attribute::F32_0) * writer.prop(intensity) * night;
    let update_color = SetAttributeModifier::new(Attribute::COLOR, packed_scale(&writer, scale));

    let mut color = Gradient::new();
    let hdr = Vec4::new(16.0, 10.0, 3.5, 0.75);
    color.add_key(0.0, hdr);
    color.add_key(1.0, hdr);

    let veil_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("veil");
    module.add_texture_slot("night");

    EffectAsset::new(
        1_500_000,
        SpawnerSettings::once(1_500_000.0.into()),
        module,
    )
    .with_name("city_lights")
    .with_simulation_space(SimulationSpace::Local)
    .with_simulation_condition(SimulationCondition::Always)
    .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
    .init(init_pos)
    .init(init_vel)
    .init(init_mag)
    .init(init_age)
    .init(init_lifetime)
    .update(update_color)
    .render(OrientModifier::new(OrientMode::FaceCameraPosition))
    .render(ParticleTextureModifier {
        texture_slot: veil_slot,
        sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
    })
    .render(crate::effects::sphere_map::SphereMapColorModifier {
        texture_slot: 1,
        hdr_boost: 1.0,
        luma_kill: 0.06,
    })
    .render(ColorOverLifetimeModifier {
        gradient: color,
        blend: ColorBlendMode::Modulate,
        mask: ColorBlendMask::RGBA,
    })
    // Constant pixels so lights don't alias as they cross the horizon.
    .render(SetSizeModifier {
        size: Vec3::splat(8.0).into(),
    })
    .render(ScreenSpaceSizeModifier)
}

fn airglow_shell(
    name: &str,
    altitude_m: f32,
    thickness_m: f32,
    capacity: u32,
    size_m: f32,
    hdr: Vec4,
    limb_mu: f32,
    disc_sharp: f32,
    space_sharp: f32,
) -> EffectAsset {
    let writer = ExprWriter::new();
    let sun_dir = writer.add_property("sun_dir", Vec3::Y.into());
    let view_pos = writer.add_property("view_pos", Vec3::new(0.0, 6_778_140.0, 0.0).into());
    let intensity = writer.add_property("intensity", 0.0f32.into());

    let init_pos = earth_shell(&writer, EARTH_R + altitude_m);
    let init_thick = thicken_shell(&writer, EARTH_R + altitude_m, thickness_m);
    let init_vel = init_zero_velocity(&writer);
    let mag = writer.lit(0.96) + writer.lit(0.04) * writer.rand(ScalarType::Float);
    let init_mag = SetAttributeModifier::new(Attribute::F32_0, mag.expr());
    let (init_age, init_lifetime) = init_age_lifetime(&writer, writer.lit(VACUUM_LIFETIME));

    let vis = night_and_limb(
        &writer,
        sun_dir,
        view_pos,
        limb_mu,
        disc_sharp,
        space_sharp,
    );
    let scale = writer.attr(Attribute::F32_0) * writer.prop(intensity) * vis;
    let update_color = SetAttributeModifier::new(Attribute::COLOR, packed_scale(&writer, scale));

    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::splat(size_m));
    size.add_key(1.0, Vec3::splat(size_m));

    let mut color = Gradient::new();
    color.add_key(0.0, hdr);
    color.add_key(1.0, hdr);

    let veil_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("veil");

    EffectAsset::new(
        capacity,
        SpawnerSettings::once((capacity as f32).into()),
        module,
    )
    .with_name(name)
    .with_simulation_space(SimulationSpace::Local)
    .with_simulation_condition(SimulationCondition::Always)
    .with_alpha_mode(bevy_hanabi::AlphaMode::Add)
    .init(init_pos)
    .init(init_thick)
    .init(init_vel)
    .init(init_mag)
    .init(init_age)
    .init(init_lifetime)
    .update(update_color)
    .render(OrientModifier::new(OrientMode::FaceCameraPosition))
    .render(ParticleTextureModifier {
        texture_slot: veil_slot,
        sample_mapping: ImageSampleMapping::ModulateOpacityFromR,
    })
    .render(SizeOverLifetimeModifier {
        gradient: size,
        screen_space_size: false,
    })
    .render(ColorOverLifetimeModifier {
        gradient: color,
        blend: ColorBlendMode::Modulate,
        mask: ColorBlendMask::RGBA,
    })
}

fn airglow_green() -> EffectAsset {
    airglow_shell(
        "airglow_green",
        95_000.0,
        10_000.0,
        520_000,
        32_000.0,
        Vec4::new(0.12, 2.8, 0.65, 0.042),
        0.955,
        55.0,
        14.0,
    )
}

fn airglow_red() -> EffectAsset {
    airglow_shell(
        "airglow_red",
        150_000.0,
        12_000.0,
        340_000,
        40_000.0,
        Vec4::new(0.32, 0.09, 0.03, 0.01),
        0.963,
        50.0,
        10.0,
    )
}

