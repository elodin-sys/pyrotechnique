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
//!   `smoke` -> smoke_puff.png (see `effects::mod` for the binding logic).
//! - Colors are HDR (components above 1.0) and rely on viewport bloom.

use bevy::ecs::reflect::AppTypeRegistry;
use bevy::math::{Vec3, Vec4};
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
    let v = writer.rand(ScalarType::Float) * writer.lit(value_max - value_min)
        + writer.lit(value_min);
    let a = writer.rand(ScalarType::Float) * writer.lit(alpha_max - alpha_min)
        + writer.lit(alpha_min);
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
        ("apollo-lander", "descent_plume", descent_plume()),
        ("apollo-lander", "rcs_puff", rcs_puff()),
        ("apollo-lander", "ground_dust", ground_dust()),
    ]
}

pub fn generate(args: &GenEffectsArgs) -> anyhow::Result<()> {
    let type_registry = AppTypeRegistry::new_with_derived_types();
    register_modifiers(&type_registry);
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
fn merlin_core() -> EffectAsset {
    let writer = ExprWriter::new();

    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(1.2).expr(),
        base_radius: writer.lit(0.8).expr(),
        top_radius: writer.lit(0.55).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Exhaust along local -Y, fast with a little per-particle variation.
    let speed = writer.lit(90.0).uniform(writer.lit(130.0));
    let vel = writer.lit(Vec3::NEG_Y) * speed;
    let init_vel = SetAttributeModifier::new(Attribute::VELOCITY, vel.expr());

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.12).uniform(writer.lit(0.3)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    let drag = LinearDragModifier::new(writer.lit(0.4).expr());

    // Blinding white-yellow core cooling through orange. Values are far above
    // 1.0 on purpose: the sky is physically lit, so saturating through the
    // exposure + tonemapper takes serious radiance.
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(34.0, 30.0, 22.0, 1.0));
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
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
}

/// Orange turbulent flame column surrounding/extending the core. Alpha
/// blended, longer lived, with drag so it billows out and fades.
fn merlin_flame() -> EffectAsset {
    let writer = ExprWriter::new();

    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(1.5).expr(),
        base_radius: writer.lit(1.1).expr(),
        top_radius: writer.lit(1.5).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Diverging cone: velocity radiates from a virtual center "behind" the
    // nozzle (inside the rocket, +Y), so the column expands downstream like a
    // real underexpanded plume.
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::new(0.0, 9.0, 0.0)).expr(),
        speed: writer.lit(70.0).uniform(writer.lit(110.0)).expr(),
    };

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.5).uniform(writer.lit(1.4)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    let drag = LinearDragModifier::new(writer.lit(1.0).expr());

    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(26.0, 8.5, 0.9, 1.0));
    color.add_key(0.3, Vec4::new(19.0, 5.0, 0.5, 0.85));
    color.add_key(0.7, Vec4::new(8.0, 2.0, 0.28, 0.5));
    color.add_key(1.0, Vec4::new(2.4, 0.7, 0.12, 0.0));

    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(5.0, 3.4, 3.4));
    size.add_key(0.35, Vec3::new(8.5, 5.2, 5.2));
    size.add_key(1.0, Vec3::new(5.5, 3.6, 3.6));

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
        .render(ColorOverLifetimeModifier {
            gradient: color,
            blend: ColorBlendMode::Overwrite,
            mask: ColorBlendMask::RGBA,
        })
}

/// Persistent world-space smoke column. `SimulationSpace::Global` is the whole
/// point: particles detach from the rocket at spawn and hang in the sky for
/// tens of seconds, painting the trail the `smoke-trail` target shows.
fn exhaust_smoke() -> EffectAsset {
    let writer = ExprWriter::new();

    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(3.0).expr(),
        base_radius: writer.lit(1.4).expr(),
        top_radius: writer.lit(2.2).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Diverging cone (virtual center behind the nozzle) so the trail widens
    // with distance instead of staying a pencil line.
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::new(0.0, 5.0, 0.0)).expr(),
        speed: writer.lit(20.0).uniform(writer.lit(36.0)).expr(),
    };

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
        .with_simulation_space(SimulationSpace::Global)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
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

/// LM descent engine plume in vacuum: a short, translucent straw-colored
/// column that expands quickly (no atmosphere to confine it) and fades to a
/// faint shimmer — see the "First Man" close-up and the Apollo 12 sim targets.
fn descent_plume() -> EffectAsset {
    let writer = ExprWriter::new();

    // Nozzle exit disc (DPS bell exit is ~1.5 m across).
    let init_pos = SetPositionCone3dModifier {
        height: writer.lit(0.3).expr(),
        base_radius: writer.lit(0.55).expr(),
        top_radius: writer.lit(0.4).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Diverging cone from a virtual center just above the nozzle; vacuum
    // plumes spread wider than sea-level ones.
    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::new(0.0, 1.6, 0.0)).expr(),
        speed: writer.lit(24.0).uniform(writer.lit(42.0)).expr(),
    };

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(0.05).uniform(writer.lit(0.13)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Pale straw core cooling to a translucent gray-white haze.
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(11.0, 8.5, 5.6, 0.7));
    color.add_key(0.35, Vec4::new(5.5, 4.4, 3.2, 0.4));
    color.add_key(0.7, Vec4::new(2.2, 2.0, 1.7, 0.16));
    color.add_key(1.0, Vec4::new(1.1, 1.05, 1.0, 0.0));

    // Thick relative to length so overlapping sprites fuse into a smooth
    // column instead of reading as individual sparks.
    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(1.4, 0.7, 0.7));
    size.add_key(0.4, Vec3::new(2.4, 1.1, 1.1));
    size.add_key(1.0, Vec3::new(1.8, 0.9, 0.9));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(16384, SpawnerSettings::rate(3600.0.into()), module)
        .with_name("descent_plume")
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
        height: writer.lit(0.08).expr(),
        base_radius: writer.lit(0.06).expr(),
        top_radius: writer.lit(0.05).expr(),
        dimension: ShapeDimension::Volume,
    };

    let init_vel = SetVelocitySphereModifier {
        center: writer.lit(Vec3::new(0.0, 0.5, 0.0)).expr(),
        speed: writer.lit(9.0).uniform(writer.lit(16.0)).expr(),
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
    size.add_key(0.0, Vec3::new(0.55, 0.16, 0.16));
    size.add_key(0.4, Vec3::new(1.1, 0.3, 0.3));
    size.add_key(1.0, Vec3::new(0.8, 0.22, 0.22));

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
/// down under lunar gravity. World-space, attached to the landing site.
fn ground_dust() -> EffectAsset {
    let writer = ExprWriter::new();

    // Ring just above the surface under the engine.
    let init_pos = SetPositionCircleModifier {
        center: writer.lit(Vec3::new(0.0, 0.12, 0.0)).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        radius: writer.lit(1.4).expr(),
        dimension: ShapeDimension::Volume,
    };

    // Flat radial spray, fast: entrained grains leave at tens of m/s.
    let init_vel = SetVelocityCircleModifier {
        center: writer.lit(Vec3::ZERO).expr(),
        axis: writer.lit(Vec3::Y).expr(),
        speed: writer.lit(7.0).uniform(writer.lit(22.0)).expr(),
    };

    // Small random upward component so the sheet has a little thickness.
    let up_kick = writer.rand(ScalarType::Float) * writer.lit(1.6);
    let kicked = writer.attr(Attribute::VELOCITY)
        + writer.lit(0.0).vec3(up_kick, writer.lit(0.0));
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
    let mut color = Gradient::new();
    color.add_key(0.0, Vec4::new(1.5, 1.42, 1.3, 0.4));
    color.add_key(0.4, Vec4::new(1.3, 1.24, 1.15, 0.28));
    color.add_key(1.0, Vec4::new(1.1, 1.06, 1.0, 0.0));

    // Stretched along velocity: fine streaks, not puffs.
    let mut size = Gradient::new();
    size.add_key(0.0, Vec3::new(1.0, 0.14, 0.14));
    size.add_key(0.4, Vec3::new(2.2, 0.3, 0.3));
    size.add_key(1.0, Vec3::new(3.2, 0.45, 0.45));

    let mask_slot = writer.lit(0u32).expr();
    let mut module = writer.finish();
    module.add_texture_slot("mask");

    EffectAsset::new(32768, SpawnerSettings::rate(4000.0.into()), module)
        .with_name("ground_dust")
        .with_simulation_space(SimulationSpace::Global)
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

/// Lift-off ground clouds: big, slow, buoyant billows fed by the deflected
/// exhaust, world-space so they stay at the pad as the rocket climbs.
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
    let kicked = writer.attr(Attribute::VELOCITY)
        + writer.lit(0.0).vec3(up_kick, writer.lit(0.0));
    let init_up_kick = SetAttributeModifier::new(Attribute::VELOCITY, kicked.expr());

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());
    let lifetime = writer.lit(8.0).uniform(writer.lit(20.0)).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Puff-to-puff variation is what sells "billowing clouds" vs fog.
    let init_modulation = init_random_modulation(&writer, 0.6, 1.0, 0.45, 1.0);

    let drag = LinearDragModifier::new(writer.lit(1.0).expr());
    let buoyancy = AccelModifier::new(writer.lit(Vec3::new(0.0, 2.2, 0.0)).expr());

    // Kill anything that dips below the pad surface.
    let kill_ground = KillAabbModifier::new(
        writer.lit(Vec3::new(0.0, -1000.0, 0.0)).expr(),
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

    EffectAsset::new(32768, SpawnerSettings::rate(90.0.into()), module)
        .with_name("pad_smoke")
        .with_simulation_space(SimulationSpace::Global)
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
