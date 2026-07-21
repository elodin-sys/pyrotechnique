//! Editor UI: egui panels around the 3D viewport.
//!
//! Layout:
//! - Top bar: project picker, playback (play/pause/restart/speed/scrub),
//!   scenario + camera presets, screenshot, gizmo toggle.
//! - Left panel: emitter list, reference image overlay controls.
//! - Right panel: inspector for the selected emitter's effect asset —
//!   spawner, alpha mode, HDR color gradient, size gradient.
//!
//! Live edits mutate the `EffectAsset` in place (which recompiles the GPU
//! effect) and auto-save: the canonical Hanabi RON is written back to the
//! `.effect` file once edits settle (`AUTOSAVE_DEBOUNCE`), and flushed on
//! project switch and app exit. External file edits hot-reload and refresh
//! the inspector.

// egui 0.35 deprecated the SidePanel/TopBottomPanel context API in favor of
// Ui-scoped `Panel`; the old API still works and keeps this file simple.
#![allow(deprecated)]

use std::collections::HashMap;
use std::time::Instant;

use bevy::app::AppExit;
use bevy::camera::Exposure;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::reflect::ReflectMut;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use bevy_hanabi::{
    AlphaMode as HanabiAlphaMode, ColorOverLifetimeModifier, CpuValue, EffectAsset, Gradient,
    Modifiers, ParticleEffect, SizeOverLifetimeModifier,
};
use bevy_panorbit_camera::PanOrbitCamera;

use crate::app::SimClock;
use crate::effects::{Emitter, ShowEmitterGizmos};
use crate::project::{LoadProject, Project, ProjectStatus, discover_projects};
use crate::render::MainCamera;
use crate::scene::{CameraPreset, SceneConfig};

/// Seconds of edit inactivity before a dirty effect is written to disk.
const AUTOSAVE_DEBOUNCE: f32 = 0.6;

pub struct EditorUiPlugin;

impl Plugin for EditorUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .init_resource::<UiState>()
            .add_systems(EguiPrimaryContextPass, ui_system)
            .add_systems(Update, invalidate_on_external_reload)
            .add_systems(Last, flush_saves_on_exit);
    }
}

/// Editable snapshot of the tunable parts of one effect asset.
#[derive(Clone, Default)]
struct EffectEditModel {
    /// (ratio, raw HDR rgba)
    color_keys: Option<Vec<(f32, [f32; 4])>>,
    /// (ratio, xyz size)
    size_keys: Option<Vec<(f32, Vec3)>>,
}

/// An effect with unsaved UI edits, pending auto-save.
struct DirtyEffect {
    /// Asset-relative `.effect` path.
    path: String,
    last_edit: Instant,
}

#[derive(Resource, Default)]
pub struct UiState {
    selected: usize,
    models: HashMap<AssetId<EffectAsset>, EffectEditModel>,
    dirty: HashMap<AssetId<EffectAsset>, DirtyEffect>,
    /// Project name as of last frame; a change means a project was (re)loaded
    /// and per-project UI caches must reset.
    last_project: String,
    /// Frame counter at the time of our last programmatic asset mutation,
    /// used to distinguish our own Modified events from external file edits.
    last_apply_frame: u32,
    frame: u32,
    show_reference: bool,
    reference_opacity: f32,
    reference_scenario: Option<String>,
    reference_texture: Option<(String, egui::TextureHandle, egui::Vec2)>,
    status: String,
}

#[derive(SystemParam)]
struct UiParams<'w, 's> {
    commands: Commands<'w, 's>,
    ui_state: ResMut<'w, UiState>,
    clock: ResMut<'w, SimClock>,
    scene: Res<'w, SceneConfig>,
    project: Res<'w, Project>,
    project_status: ResMut<'w, ProjectStatus>,
    load_project: MessageWriter<'w, LoadProject>,
    effects: ResMut<'w, Assets<EffectAsset>>,
    registry: Res<'w, AppTypeRegistry>,
    gizmos: ResMut<'w, ShowEmitterGizmos>,
    emitters: Query<'w, 's, (Entity, &'static Emitter, &'static ParticleEffect)>,
    camera: Query<
        'w,
        's,
        (
            &'static mut PanOrbitCamera,
            &'static mut Projection,
            &'static mut Exposure,
        ),
        With<MainCamera>,
    >,
}

fn ui_system(mut contexts: EguiContexts, mut p: UiParams) -> Result {
    let ctx = contexts.ctx_mut()?;
    p.ui_state.frame = p.ui_state.frame.wrapping_add(1);

    // A project was opened (from the picker or CLI): reset per-project caches.
    if p.ui_state.last_project != p.project.name {
        p.ui_state.last_project = p.project.name.clone();
        p.ui_state.selected = 0;
        p.ui_state.models.clear();
        p.ui_state.dirty.clear();
        p.ui_state.reference_scenario = None;
        p.ui_state.reference_texture = None;
        p.ui_state.show_reference = false;
    }
    if let Some(message) = p.project_status.0.take() {
        p.ui_state.status = message;
    }

    top_bar(ctx, &mut p);
    left_panel(ctx, &mut p);
    right_panel(ctx, &mut p);
    reference_overlay(ctx, &mut p);

    autosave_pending(&mut p, false);

    Ok(())
}

/// Writes dirty effects whose edits have settled (or all of them, if
/// `force`) back to their `.effect` files.
fn autosave_pending(p: &mut UiParams, force: bool) {
    let due: Vec<AssetId<EffectAsset>> = p
        .ui_state
        .dirty
        .iter()
        .filter(|(_, dirty)| force || dirty.last_edit.elapsed().as_secs_f32() >= AUTOSAVE_DEBOUNCE)
        .map(|(id, _)| *id)
        .collect();
    for id in due {
        let Some(dirty) = p.ui_state.dirty.remove(&id) else {
            continue;
        };
        match save_effect_file(&p.effects, &p.registry, id, &dirty.path) {
            Ok(path) => p.ui_state.status = format!("auto-saved {path}"),
            Err(e) => p.ui_state.status = format!("auto-save failed for {}: {e}", dirty.path),
        }
    }
}

/// Last-chance flush when the app is quitting (window closed mid-debounce).
fn flush_saves_on_exit(
    mut exits: MessageReader<AppExit>,
    mut ui_state: ResMut<UiState>,
    effects: Res<Assets<EffectAsset>>,
    registry: Res<AppTypeRegistry>,
) {
    if exits.read().next().is_none() {
        return;
    }
    let pending: Vec<(AssetId<EffectAsset>, DirtyEffect)> = ui_state.dirty.drain().collect();
    for (id, dirty) in pending {
        if let Err(e) = save_effect_file(&effects, &registry, id, &dirty.path) {
            error!("exit auto-save failed for {}: {e}", dirty.path);
        }
    }
}

// ---------------------------------------------------------------------------
// Top bar
// ---------------------------------------------------------------------------

fn top_bar(ctx: &mut egui::Context, p: &mut UiParams) {
    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            // Project picker. Selecting the open project re-reads it from
            // disk (useful after external scene edits).
            let current_project = p.project.name.clone();
            let mut chosen_project: Option<String> = None;
            egui::ComboBox::from_label("project")
                .selected_text(current_project.clone())
                .show_ui(ui, |ui| {
                    for name in discover_projects() {
                        if ui
                            .selectable_label(name == current_project, &name)
                            .clicked()
                        {
                            chosen_project = Some(name);
                        }
                    }
                });
            if let Some(name) = chosen_project {
                // Don't lose pending edits when the world is torn down.
                autosave_pending(p, true);
                p.load_project.write(LoadProject(name));
            }

            ui.separator();

            let play_label = if p.clock.playing { "Pause" } else { "Play" };
            if ui.button(play_label).clicked() {
                p.clock.playing = !p.clock.playing;
            }
            if ui.button("Restart").clicked() {
                p.clock.t = 0.0;
                // Despawn emitters; spawn_emitters recreates them next frame,
                // clearing all GPU particle state (incl. world-space smoke).
                for (entity, _, _) in &p.emitters {
                    p.commands.entity(entity).despawn();
                }
            }
            ui.label("speed");
            ui.add(
                egui::DragValue::new(&mut p.clock.speed)
                    .speed(0.05)
                    .range(0.0..=8.0),
            );

            let duration = p.scene.flight.duration().max(1.0);
            ui.label("t");
            let mut t = p.clock.t;
            if ui
                .add(egui::Slider::new(&mut t, 0.0..=duration).show_value(true))
                .changed()
            {
                p.clock.t = t;
            }

            ui.separator();

            // Scenario preset: camera + exposure + jump to capture time.
            let mut chosen_scenario: Option<String> = None;
            egui::ComboBox::from_label("scenario")
                .selected_text(p.ui_state.reference_scenario.as_deref().unwrap_or("-"))
                .show_ui(ui, |ui| {
                    for s in &p.scene.scenarios {
                        if ui.selectable_label(false, &s.name).clicked() {
                            chosen_scenario = Some(s.name.clone());
                        }
                    }
                });
            if let Some(name) = chosen_scenario {
                apply_scenario(p, &name);
            }

            // Bare camera preset snap.
            let mut chosen_camera: Option<String> = None;
            egui::ComboBox::from_label("camera")
                .selected_text("-")
                .show_ui(ui, |ui| {
                    for c in &p.scene.cameras {
                        if ui.selectable_label(false, &c.name).clicked() {
                            chosen_camera = Some(c.name.clone());
                        }
                    }
                });
            if let Some(name) = chosen_camera
                && let Some(preset) = p.scene.camera(&name).cloned()
            {
                snap_camera(p, &preset);
            }

            ui.separator();
            if ui.button("Screenshot").clicked() {
                let dir = p.project.shots_dir();
                let path = dir.join(format!(
                    "edit_{}.png",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                ));
                let _ = std::fs::create_dir_all(&dir);
                p.ui_state.status = format!("saved {}", path.display());
                p.commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(path));
            }
            ui.checkbox(&mut p.gizmos.0, "gizmos");

            if !p.ui_state.status.is_empty() {
                ui.separator();
                ui.weak(p.ui_state.status.clone());
            }
        });
    });
}

fn apply_scenario(p: &mut UiParams, name: &str) {
    let Some(scenario) = p.scene.scenario(name).cloned() else {
        return;
    };
    p.clock.t = scenario.capture_time;
    p.clock.playing = false;
    if let Some(preset) = p.scene.camera(&scenario.camera).cloned() {
        snap_camera(p, &preset);
    }
    if let Some(ev100) = scenario.exposure_ev100
        && let Ok((_, _, mut exposure)) = p.camera.single_mut()
    {
        exposure.ev100 = ev100;
    }
    p.ui_state.reference_scenario = Some(name.to_string());
    p.ui_state.show_reference = scenario.reference.is_some();
}

fn snap_camera(p: &mut UiParams, preset: &CameraPreset) {
    let (rocket_pos, _) = p.scene.flight.sample(p.clock.t);
    let Ok((mut orbit, mut projection, _)) = p.camera.single_mut() else {
        return;
    };
    crate::render::snap_orbit_to_preset(&mut orbit, &mut projection, preset, rocket_pos);
}

// ---------------------------------------------------------------------------
// Left panel: emitters + reference controls
// ---------------------------------------------------------------------------

fn left_panel(ctx: &mut egui::Context, p: &mut UiParams) {
    egui::SidePanel::left("left_panel")
        .default_width(230.0)
        .show(ctx, |ui| {
            ui.heading("Emitters");
            ui.separator();
            // `.get` guards: emitter entities and the scene resource can be
            // one frame out of sync while a project switch tears down.
            let mut ordered: Vec<(usize, String, String)> = p
                .emitters
                .iter()
                .filter_map(|(_, emitter, _)| {
                    let config = p.scene.emitters.get(emitter.index)?;
                    Some((emitter.index, config.name.clone(), config.effect.clone()))
                })
                .collect();
            ordered.sort_by_key(|(index, _, _)| *index);
            for (index, name, effect_path) in ordered {
                let Some(config) = p.scene.emitters.get(index) else {
                    continue;
                };
                let multiplier = config.intensity * config.activity_at(p.clock.t);
                let selected = p.ui_state.selected == index;
                let label = format!("{name}  (x{multiplier:.2})");
                if ui.selectable_label(selected, label).clicked() {
                    p.ui_state.selected = index;
                }
                ui.weak(effect_path);
                ui.add_space(4.0);
            }

            ui.separator();
            ui.heading("Reference");
            let scenario_name = p.ui_state.reference_scenario.clone();
            match scenario_name
                .as_deref()
                .and_then(|n| p.scene.scenario(n))
                .and_then(|s| s.reference.clone())
            {
                Some(path) => {
                    ui.checkbox(&mut p.ui_state.show_reference, "show target image");
                    ui.add(
                        egui::Slider::new(&mut p.ui_state.reference_opacity, 0.1..=1.0)
                            .text("opacity"),
                    );
                    ui.weak(path);
                }
                None => {
                    ui.weak("select a scenario with a reference");
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Right panel: effect inspector
// ---------------------------------------------------------------------------

fn right_panel(ctx: &mut egui::Context, p: &mut UiParams) {
    egui::SidePanel::right("right_panel")
        .default_width(330.0)
        .show(ctx, |ui| {
            let selected = p.ui_state.selected;
            let Some((_, emitter, effect)) =
                p.emitters.iter().find(|(_, e, _)| e.index == selected)
            else {
                ui.weak("no emitter selected");
                return;
            };
            let Some(config) = p.scene.emitters.get(emitter.index) else {
                ui.weak("emitter out of sync (project switching)");
                return;
            };
            let effect_path = config.effect.clone();
            let emitter_name = config.name.clone();
            let asset_id = effect.handle.id();

            ui.heading(&emitter_name);
            ui.weak(&effect_path);
            ui.separator();

            let Some(asset) = p.effects.get(asset_id) else {
                ui.weak("effect asset not loaded yet");
                return;
            };

            // Snapshot immutable facts + build the edit model if missing.
            let capacity = asset.capacity();
            let sim_space = format!("{:?}", asset.simulation_space);
            let mut alpha_mode = asset.alpha_mode;
            let mut spawner = asset.spawner;
            let model = p
                .ui_state
                .models
                .entry(asset_id)
                .or_insert_with(|| extract_model(asset))
                .clone();

            let mut model = model;
            let mut changed = false;

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label(format!("capacity: {capacity}   space: {sim_space}"));

                // Alpha mode
                let mut mode_idx = alpha_mode_index(&alpha_mode);
                egui::ComboBox::from_label("alpha mode")
                    .selected_text(ALPHA_MODE_NAMES[mode_idx])
                    .show_ui(ui, |ui| {
                        for (i, name) in ALPHA_MODE_NAMES.iter().enumerate() {
                            if ui.selectable_label(mode_idx == i, *name).clicked() {
                                mode_idx = i;
                            }
                        }
                    });
                if alpha_mode_index(&alpha_mode) != mode_idx {
                    alpha_mode = alpha_mode_from_index(mode_idx);
                    changed = true;
                }

                // Spawner
                ui.separator();
                ui.strong("Spawner");
                changed |= cpu_value_ui(ui, "rate (particles/s)", &mut spawner_count(&mut spawner));

                // Color gradient
                if let Some(keys) = &mut model.color_keys {
                    ui.separator();
                    ui.strong("Color over lifetime (HDR)");
                    changed |= color_gradient_ui(ui, keys);
                }

                // Size gradient
                if let Some(keys) = &mut model.size_keys {
                    ui.separator();
                    ui.strong("Size over lifetime");
                    changed |= size_gradient_ui(ui, keys);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reload from file").clicked() {
                        p.ui_state.models.remove(&asset_id);
                        p.ui_state.dirty.remove(&asset_id);
                        p.ui_state.status = format!("reloading {effect_path}");
                        // Touching the file path forces the asset server to reload.
                        let full = std::path::Path::new("assets").join(&effect_path);
                        if let Ok(time) = std::fs::metadata(&full).and_then(|m| m.modified()) {
                            let _ = filetime_touch(&full, time);
                        }
                    }
                    if p.ui_state.dirty.contains_key(&asset_id) {
                        ui.weak("unsaved edits (auto-saves)");
                    } else {
                        ui.weak("edits auto-save");
                    }
                });
            });

            if changed {
                let frame = p.ui_state.frame;
                if let Some(mut asset) = p.effects.get_mut(asset_id) {
                    asset.alpha_mode = alpha_mode;
                    asset.spawner = spawner;
                    apply_model(&mut asset, &model);
                    p.ui_state.last_apply_frame = frame;
                }
                p.ui_state.models.insert(asset_id, model);
                p.ui_state.dirty.insert(
                    asset_id,
                    DirtyEffect {
                        path: effect_path.clone(),
                        last_edit: Instant::now(),
                    },
                );
            }
        });
}

/// Bump a file's mtime so the asset watcher reloads it.
fn filetime_touch(path: &std::path::Path, _now: std::time::SystemTime) -> std::io::Result<()> {
    // Rewrite the file with its own contents; simplest portable "touch".
    let bytes = std::fs::read(path)?;
    std::fs::write(path, bytes)
}

const ALPHA_MODE_NAMES: [&str; 4] = ["Blend", "Premultiply", "Add", "Multiply"];

fn alpha_mode_index(mode: &HanabiAlphaMode) -> usize {
    match mode {
        HanabiAlphaMode::Blend => 0,
        HanabiAlphaMode::Premultiply => 1,
        HanabiAlphaMode::Add => 2,
        HanabiAlphaMode::Multiply => 3,
        _ => 0,
    }
}

fn alpha_mode_from_index(index: usize) -> HanabiAlphaMode {
    match index {
        1 => HanabiAlphaMode::Premultiply,
        2 => HanabiAlphaMode::Add,
        3 => HanabiAlphaMode::Multiply,
        _ => HanabiAlphaMode::Blend,
    }
}

/// Accessor kludge: expose the spawner count for editing and write it back.
struct SpawnerCount<'a>(&'a mut bevy_hanabi::SpawnerSettings);

fn spawner_count<'a>(settings: &'a mut bevy_hanabi::SpawnerSettings) -> SpawnerCount<'a> {
    SpawnerCount(settings)
}

fn cpu_value_ui(ui: &mut egui::Ui, label: &str, count: &mut SpawnerCount) -> bool {
    let mut changed = false;
    let value = count.0.count();
    match value {
        CpuValue::Single(v) => {
            let mut v = v;
            ui.horizontal(|ui| {
                ui.label(label);
                if ui
                    .add(
                        egui::DragValue::new(&mut v)
                            .speed(5.0)
                            .range(0.0..=100000.0),
                    )
                    .changed()
                {
                    count.0.set_count(CpuValue::Single(v));
                    changed = true;
                }
            });
        }
        CpuValue::Uniform((lo, hi)) => {
            let (mut lo, mut hi) = (lo, hi);
            ui.horizontal(|ui| {
                ui.label(label);
                let a = ui
                    .add(
                        egui::DragValue::new(&mut lo)
                            .speed(5.0)
                            .range(0.0..=100000.0),
                    )
                    .changed();
                let b = ui
                    .add(
                        egui::DragValue::new(&mut hi)
                            .speed(5.0)
                            .range(0.0..=100000.0),
                    )
                    .changed();
                if a || b {
                    count.0.set_count(CpuValue::Uniform((lo, hi.max(lo))));
                    changed = true;
                }
            });
        }
        _ => {
            ui.weak(format!("{label}: <unsupported spawner value>"));
        }
    }
    changed
}

/// HDR color key rows: unit color picker + intensity multiplier + alpha.
fn color_gradient_ui(ui: &mut egui::Ui, keys: &mut Vec<(f32, [f32; 4])>) -> bool {
    let mut changed = false;
    let mut remove: Option<usize> = None;
    for (i, (ratio, rgba)) in keys.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(ratio)
                        .speed(0.01)
                        .range(0.0..=1.0)
                        .fixed_decimals(2),
                )
                .changed();

            let intensity = rgba[0].max(rgba[1]).max(rgba[2]).max(1e-4);
            let mut unit = [
                rgba[0] / intensity,
                rgba[1] / intensity,
                rgba[2] / intensity,
            ];
            let mut intensity_edit = intensity;
            if ui.color_edit_button_rgb(&mut unit).changed() {
                changed = true;
            }
            ui.label("x");
            if ui
                .add(
                    egui::DragValue::new(&mut intensity_edit)
                        .speed(0.05)
                        .range(0.0..=32.0),
                )
                .changed()
            {
                changed = true;
            }
            ui.label("a");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut rgba[3])
                        .speed(0.01)
                        .range(0.0..=1.0)
                        .fixed_decimals(2),
                )
                .changed();
            if changed {
                rgba[0] = unit[0] * intensity_edit;
                rgba[1] = unit[1] * intensity_edit;
                rgba[2] = unit[2] * intensity_edit;
            }
            if ui.small_button("x").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove
        && keys.len() > 1
    {
        keys.remove(i);
        changed = true;
    }
    if ui.small_button("+ add key").clicked() {
        let last = keys.last().cloned().unwrap_or((0.5, [1.0, 1.0, 1.0, 0.5]));
        keys.push((1.0f32.min(last.0 + 0.1), last.1));
        changed = true;
    }
    changed
}

fn size_gradient_ui(ui: &mut egui::Ui, keys: &mut Vec<(f32, Vec3)>) -> bool {
    let mut changed = false;
    let mut remove: Option<usize> = None;
    for (i, (ratio, size)) in keys.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(ratio)
                        .speed(0.01)
                        .range(0.0..=1.0)
                        .fixed_decimals(2),
                )
                .changed();
            for c in 0..3 {
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut size[c])
                            .speed(0.05)
                            .range(0.0..=200.0),
                    )
                    .changed();
            }
            if ui.small_button("x").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove
        && keys.len() > 1
    {
        keys.remove(i);
        changed = true;
    }
    if ui.small_button("+ add key").clicked() {
        let last = keys.last().cloned().unwrap_or((0.5, Vec3::ONE));
        keys.push((1.0f32.min(last.0 + 0.1), last.1));
        changed = true;
    }
    changed
}

// ---------------------------------------------------------------------------
// Edit model extraction / application
// ---------------------------------------------------------------------------

fn extract_model(asset: &EffectAsset) -> EffectEditModel {
    let mut model = EffectEditModel::default();
    for modifier in asset.render_modifiers() {
        if let Some(m) = modifier
            .as_reflect()
            .downcast_ref::<ColorOverLifetimeModifier>()
        {
            model.color_keys = Some(
                m.gradient
                    .keys()
                    .iter()
                    .map(|k| (k.ratio(), k.value.to_array()))
                    .collect(),
            );
        } else if let Some(m) = modifier
            .as_reflect()
            .downcast_ref::<SizeOverLifetimeModifier>()
        {
            model.size_keys = Some(
                m.gradient
                    .keys()
                    .iter()
                    .map(|k| (k.ratio(), k.value))
                    .collect(),
            );
        }
    }
    model
}

/// Writes the edit model back into the asset's render modifiers, reaching the
/// private `render_modifiers` field through reflection.
fn apply_model(asset: &mut EffectAsset, model: &EffectEditModel) {
    let ReflectMut::Struct(s) = asset.reflect_mut() else {
        return;
    };
    let Some(field) = s.field_mut("render_modifiers") else {
        return;
    };
    let Some(modifiers) = field.try_downcast_mut::<Modifiers>() else {
        return;
    };
    for boxed in modifiers.0.iter_mut() {
        if let Some(m) = boxed
            .as_reflect_mut()
            .downcast_mut::<ColorOverLifetimeModifier>()
        {
            if let Some(keys) = &model.color_keys {
                let mut gradient = Gradient::new();
                for (ratio, rgba) in keys {
                    gradient.add_key(ratio.clamp(0.0, 1.0), Vec4::from_array(*rgba));
                }
                m.gradient = gradient;
            }
        } else if let Some(m) = boxed
            .as_reflect_mut()
            .downcast_mut::<SizeOverLifetimeModifier>()
        {
            if let Some(keys) = &model.size_keys {
                let mut gradient = Gradient::new();
                for (ratio, size) in keys {
                    gradient.add_key(ratio.clamp(0.0, 1.0), *size);
                }
                m.gradient = gradient;
            }
        }
    }
}

/// Serializes an effect asset to canonical Hanabi RON at `assets/<path>`.
fn save_effect_file(
    effects: &Assets<EffectAsset>,
    registry: &AppTypeRegistry,
    asset_id: AssetId<EffectAsset>,
    effect_path: &str,
) -> anyhow::Result<String> {
    let asset = effects
        .get(asset_id)
        .ok_or_else(|| anyhow::anyhow!("asset not loaded"))?;
    let registry = registry.read();
    let text = asset
        .serialize(&registry)
        .map_err(|e| anyhow::anyhow!("serialize: {e}"))?;
    let path = std::path::Path::new("assets").join(effect_path);
    std::fs::write(&path, text)?;
    Ok(path.display().to_string())
}

/// Drops cached edit models when an effect file changes externally (so the
/// inspector re-extracts), while ignoring Modified events caused by our own
/// live edits.
fn invalidate_on_external_reload(
    mut events: MessageReader<AssetEvent<EffectAsset>>,
    mut ui_state: ResMut<UiState>,
) {
    let frame = ui_state.frame;
    for event in events.read() {
        let AssetEvent::Modified { id } = event else {
            continue;
        };
        if frame.wrapping_sub(ui_state.last_apply_frame) > 3 {
            ui_state.models.remove(id);
        }
    }
}

// ---------------------------------------------------------------------------
// Reference image overlay
// ---------------------------------------------------------------------------

fn reference_overlay(ctx: &mut egui::Context, p: &mut UiParams) {
    if !p.ui_state.show_reference {
        return;
    }
    let Some(reference) = p
        .ui_state
        .reference_scenario
        .clone()
        .and_then(|n| p.scene.scenario(&n).cloned())
        .and_then(|s| s.reference)
    else {
        return;
    };

    // (Re)load the texture if the path changed.
    let needs_load = p
        .ui_state
        .reference_texture
        .as_ref()
        .map(|(path, _, _)| path != &reference)
        .unwrap_or(true);
    if needs_load {
        match image::open(&reference) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                let texture =
                    ctx.load_texture(reference.clone(), color, egui::TextureOptions::LINEAR);
                let scale = (480.0 / rgba.width() as f32).min(1.0);
                let display =
                    egui::Vec2::new(rgba.width() as f32 * scale, rgba.height() as f32 * scale);
                p.ui_state.reference_texture = Some((reference.clone(), texture, display));
            }
            Err(e) => {
                p.ui_state.status = format!("reference load failed: {e}");
                p.ui_state.show_reference = false;
                return;
            }
        }
    }

    if p.ui_state.reference_opacity <= 0.0 {
        p.ui_state.reference_opacity = 0.9;
    }
    let opacity = p.ui_state.reference_opacity;
    if let Some((_, texture, size)) = &p.ui_state.reference_texture {
        egui::Window::new("target")
            .default_size(*size)
            .resizable(true)
            .show(ctx, |ui| {
                let tint =
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, (opacity * 255.0) as u8);
                let avail = ui.available_size();
                let scale = (avail.x / size.x).min(avail.y.max(64.0) / size.y).max(0.05);
                ui.add(
                    egui::Image::new((texture.id(), *size * scale))
                        .tint(tint)
                        .maintain_aspect_ratio(true),
                );
            });
    }
}
