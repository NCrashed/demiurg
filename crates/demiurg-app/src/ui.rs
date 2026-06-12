//! The egui editor UI: a top menu bar and a left tool panel. The panels
//! mutate the [`Editor`] in place (tool, colour, mirror, pivot) and
//! record one-shot menu choices in [`UiActions`] for the host to run
//! after the frame (file dialogs, undo/redo) — egui closures can't
//! borrow the renderer, so deferred actions keep the borrow graph clean.

use roxlap_render::egui;

use crate::{Editor, Tool};

/// Voxlap-packed `0x80RRGGBB` swatches for the preset row.
const PRESETS: [u32; 8] = [
    0x80ff_ffff,
    0x8080_8080,
    0x80ff_0000,
    0x8000_ff00,
    0x8000_00ff,
    0x80ff_ff00,
    0x80ff_8000,
    0x8080_40c0,
];

/// One-shot menu choices, applied by the host after the UI frame.
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // a set of one-shot menu flags, not state
pub struct UiActions {
    pub new_model: bool,
    pub open_kv6: bool,
    pub save_kv6: bool,
    pub open_project: bool,
    pub save_project: bool,
    pub undo: bool,
    pub redo: bool,
}

/// Draw the editor chrome for one frame.
#[allow(clippy::too_many_lines)] // a flat panel layout reads better unsplit
pub fn build(ctx: &egui::Context, editor: &mut Editor, actions: &mut UiActions) {
    egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    actions.new_model = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Open .kv6…").clicked() {
                    actions.open_kv6 = true;
                    ui.close_menu();
                }
                if ui.button("Save .kv6…").clicked() {
                    actions.save_kv6 = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Open project…").clicked() {
                    actions.open_project = true;
                    ui.close_menu();
                }
                if ui.button("Save project…").clicked() {
                    actions.save_project = true;
                    ui.close_menu();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui
                    .add_enabled(editor.document.can_undo(), egui::Button::new("Undo"))
                    .clicked()
                {
                    actions.undo = true;
                    ui.close_menu();
                }
                if ui
                    .add_enabled(editor.document.can_redo(), egui::Button::new("Redo"))
                    .clicked()
                {
                    actions.redo = true;
                    ui.close_menu();
                }
            });
        });
    });

    egui::SidePanel::left("tools")
        .default_width(190.0)
        .show(ctx, |ui| {
            ui.heading("Tools");
            for (tool, label) in [
                (Tool::Place, "Place"),
                (Tool::Erase, "Erase"),
                (Tool::Paint, "Paint"),
                (Tool::Eyedropper, "Eyedropper"),
                (Tool::Box, "Box (2 clicks)"),
                (Tool::Sphere, "Sphere"),
                (Tool::Fill, "Flood fill"),
            ] {
                ui.selectable_value(&mut editor.tool, tool, label);
            }
            if editor.tool == Tool::Sphere {
                ui.add(egui::Slider::new(&mut editor.radius, 0..=8).text("radius"));
            }

            ui.separator();
            ui.label("Colour");
            let mut rgb = color_to_rgb(editor.color);
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                editor.color = rgb_to_color(rgb);
            }
            ui.horizontal_wrapped(|ui| {
                for &preset in &PRESETS {
                    let p = color_to_rgb(preset);
                    let swatch = egui::Button::new("")
                        .min_size(egui::vec2(18.0, 18.0))
                        .fill(egui::Color32::from_rgb(p[0], p[1], p[2]));
                    if ui.add(swatch).clicked() {
                        editor.color = preset;
                    }
                }
            });

            ui.separator();
            ui.label("Mirror");
            ui.horizontal(|ui| {
                ui.checkbox(&mut editor.document.mirror[0], "X");
                ui.checkbox(&mut editor.document.mirror[1], "Y");
                ui.checkbox(&mut editor.document.mirror[2], "Z");
            });

            ui.separator();
            ui.label("Pivot");
            let mut pivot = editor.document.pivot();
            let mut changed = false;
            ui.horizontal(|ui| {
                changed |= ui
                    .add(egui::DragValue::new(&mut pivot[0]).speed(0.5).prefix("x "))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut pivot[1]).speed(0.5).prefix("y "))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut pivot[2]).speed(0.5).prefix("z "))
                    .changed();
            });
            if changed {
                editor.document.set_pivot(pivot);
                editor.dirty = true;
            }

            ui.separator();
            let (dx, dy, dz) = editor.document.dims();
            ui.label(format!("dims  {dx} × {dy} × {dz}"));
            ui.label(format!(
                "voxels  {}",
                editor.document.model().occupied_count()
            ));

            ui.separator();
            ui.small("LMB: apply tool");
            ui.small("RMB drag: orbit · wheel: zoom");
        });
}

/// `0x80RRGGBB` -> `[r, g, b]`.
#[allow(clippy::cast_possible_truncation)] // channels are masked to 0..=255
fn color_to_rgb(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

/// `[r, g, b]` -> `0x80RRGGBB` (brightness bit set).
fn rgb_to_color(rgb: [u8; 3]) -> u32 {
    0x8000_0000 | (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2])
}
