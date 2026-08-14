//! Init modifier: spawn particles from a baked equirect tile CDF.
//!
//! Hanabi init compute cannot bind material images, so the 128×64 city-light
//! CDF is embedded in the modifier (and thus the `.effect` file). Inverse-CDF
//! pick a tile, jitter UV inside it, then convert with the same equirect
//! convention as [`crate::effects::sphere_map`].

use std::path::Path;

use bevy::prelude::*;
use bevy::reflect::Reflect;
use bevy_hanabi::graph::ExprError;
use bevy_hanabi::prelude::*;
use serde::{Deserialize, Serialize};

pub const TILES_U: u32 = 128;
pub const TILES_V: u32 = 64;
pub const TILE_COUNT: usize = (TILES_U * TILES_V) as usize;

/// Place a particle on the Earth shell using a luma×cos(lat) tile CDF.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
pub struct CityTileCdfModifier {
    /// Shell radius in metres (`R + 8 km` for city lights).
    pub radius: f32,
    /// Inclusive prefix sums, length [`TILE_COUNT`], last value ≈ 1.0.
    pub cdf: Vec<f32>,
}

impl CityTileCdfModifier {
    pub fn from_bin(path: &Path, radius: f32) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        if bytes.len() != TILE_COUNT * 4 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "city tile CDF expected {} bytes, got {}",
                    TILE_COUNT * 4,
                    bytes.len()
                ),
            ));
        }
        let cdf = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();
        Ok(Self { radius, cdf })
    }

    fn extra_wgsl(&self) -> String {
        let mut out = String::with_capacity(TILE_COUNT * 12 + 2048);
        for row in 0..TILES_V {
            let start = (row * TILES_U) as usize;
            let vals = self.cdf[start..start + TILES_U as usize]
                .iter()
                .map(|value| format!("{value:.8}"))
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(&format!(
                "const CITY_CDF_R{row}: array<f32, {TILES_U}> = array<f32, {TILES_U}>({vals});\n"
            ));
        }
        out.push_str(
            r#"
fn city_cdf_at(i: u32) -> f32 {
    let row = i / 128u;
    let col = i % 128u;
    switch row {
"#,
        );
        for row in 0..TILES_V {
            out.push_str(&format!(
                "        case {row}u: {{ return CITY_CDF_R{row}[col]; }}\n"
            ));
        }
        out.push_str(
            r#"        default: { return 1.0; }
    }
}

fn city_tile_index(u: f32) -> u32 {
    var lo = 0u;
    var hi = 8192u;
    for (var step = 0u; step < 14u; step++) {
        if (lo >= hi) {
            break;
        }
        let mid = (lo + hi) >> 1u;
        if (city_cdf_at(mid) < u) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    return min(lo, 8191u);
}
"#,
        );
        out
    }
}

impl Modifier for CityTileCdfModifier {
    fn context(&self) -> ModifierContext {
        ModifierContext::Init
    }

    fn attributes(&self) -> &[Attribute] {
        &[Attribute::POSITION]
    }

    fn boxed_clone(&self) -> BoxedModifier {
        Box::new(self.clone())
    }

    fn apply(&self, _module: &mut Module, context: &mut ShaderWriter) -> Result<(), ExprError> {
        if self.cdf.len() != TILE_COUNT {
            return Err(ExprError::GraphEvalError(format!(
                "CityTileCdfModifier cdf length {} != {TILE_COUNT}",
                self.cdf.len()
            )));
        }
        context.extra_code += &self.extra_wgsl();
        let radius = self.radius;
        context.main_code += &format!(
            r#"    {{
    let ctc_idx = city_tile_index(frand());
    let ctc_tx = ctc_idx % 128u;
    let ctc_ty = ctc_idx / 128u;
    let ctc_u = (f32(ctc_tx) + frand()) * 0.0078125;
    let ctc_v = (f32(ctc_ty) + frand()) * 0.015625;
    let ctc_lon = (ctc_u - 0.5) * 6.28318530718;
    let ctc_lat = (0.5 - ctc_v) * 3.14159265359;
    let ctc_cl = cos(ctc_lat);
    let ctc_n = vec3<f32>(ctc_cl * cos(ctc_lon), sin(ctc_lat), ctc_cl * sin(ctc_lon));
    particle.position = ctc_n * {radius:.4};
    }}
"#
        );
        Ok(())
    }
}

/// Register this modifier for `.effect` RON de/serialization.
pub fn register(type_registry: &bevy::ecs::reflect::AppTypeRegistry) {
    type_registry.write().register::<CityTileCdfModifier>();
    register_reflect_modifier::<CityTileCdfModifier>(type_registry, |_module| {
        Box::new(CityTileCdfModifier {
            radius: 6_386_140.0,
            cdf: Vec::new(),
        })
    });
}
