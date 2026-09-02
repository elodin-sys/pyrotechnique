//! Render modifier: sample an equirectangular texture from particle position.
//!
//! Used for city lights (NASA Black Marble) so geography lives in a texture
//! while placement is a Hanabi sphere. Sampling happens in the fragment
//! shader (`textureSampleLevel`); init/update compute cannot bind material
//! images. `textureSample` is avoided: UV is constant across a billboard, so
//! screen-space derivatives are zero.

use bevy::prelude::*;
use bevy::reflect::Reflect;
use bevy_hanabi::graph::ExprError;
use bevy_hanabi::prelude::*;
use serde::{Deserialize, Serialize};

/// Multiply particle color by an equirectangular map sampled at the particle's
/// spherical direction (`normalize(position)` in local space).
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct SphereMapColorModifier {
    /// Index into the effect's texture layout (`0` = first slot).
    pub texture_slot: u32,
    /// HDR multiplier on the sampled RGB.
    pub hdr_boost: f32,
    /// Discard samples dimmer than this luma (keeps oceans from drawing).
    pub luma_kill: f32,
}

impl Modifier for SphereMapColorModifier {
    fn context(&self) -> ModifierContext {
        ModifierContext::Render
    }

    fn as_render(&self) -> Option<&dyn RenderModifier> {
        Some(self)
    }

    fn as_render_mut(&mut self) -> Option<&mut dyn RenderModifier> {
        Some(self)
    }

    fn into_boxed_render(self: Box<Self>) -> Option<Box<dyn RenderModifier>> {
        Some(self)
    }

    fn attributes(&self) -> &[Attribute] {
        &[Attribute::POSITION]
    }

    fn boxed_clone(&self) -> BoxedModifier {
        Box::new(*self)
    }

    fn apply(&self, _module: &mut Module, context: &mut ShaderWriter) -> Result<(), ExprError> {
        Err(ExprError::InvalidModifierContext(
            context.modifier_context(),
            ModifierContext::Render,
            "",
        ))
    }
}

impl RenderModifier for SphereMapColorModifier {
    fn apply_render(
        &self,
        _module: &mut Module,
        context: &mut RenderContext,
    ) -> Result<(), ExprError> {
        context.set_needs_particle_fragment();
        let slot = self.texture_slot;
        let boost = self.hdr_boost;
        let kill = self.luma_kill;
        context.fragment_code += &format!(
            "    {{
    let sm_n = normalize(particle.position);
    let sm_u = atan2(sm_n.z, sm_n.x) * 0.15915494309 + 0.5;
    let sm_v = 0.5 - asin(clamp(sm_n.y, -1.0, 1.0)) * 0.31830988618;
    let sm_tex = textureSampleLevel(material_texture_{slot}, material_sampler_{slot}, vec2<f32>(sm_u, sm_v), 0.0);
    let sm_luma = dot(sm_tex.rgb, vec3<f32>(0.3, 0.6, 0.1));
    color = vec4<f32>(color.rgb * sm_tex.rgb * {boost:.4}, color.a * step({kill:.4}, sm_luma));
    }}
"
        );
        Ok(())
    }

    fn boxed_render_clone(&self) -> Box<dyn RenderModifier> {
        Box::new(*self)
    }

    fn as_modifier(&self) -> &dyn Modifier {
        self
    }
}

/// Register this modifier for `.effect` RON de/serialization.
pub fn register(type_registry: &bevy::ecs::reflect::AppTypeRegistry) {
    type_registry.write().register::<SphereMapColorModifier>();
    register_reflect_modifier::<SphereMapColorModifier>(type_registry, |_module| {
        Box::new(SphereMapColorModifier {
            texture_slot: 1,
            hdr_boost: 12.0,
            luma_kill: 0.04,
        })
    });
}
