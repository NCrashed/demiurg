//! The egui editor UI: a top menu bar and a left tool panel. The panels
//! mutate the [`Editor`] in place (tool, colour, mirror, pivot, language)
//! and record one-shot menu choices in [`UiActions`] for the host to run
//! after the frame (file dialogs, undo/redo) — egui closures can't borrow
//! the renderer, so deferred actions keep the borrow graph clean.
//!
//! All user-facing strings come from [`demiurg_i18n`]; the language is
//! `editor.lang`, switchable live from the Language menu.

use demiurg_i18n::{Lang, Msg, tr};
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

/// Draw the editor chrome for one frame. `highlight` is the wire box of
/// the voxel under the cursor, as framebuffer-pixel line segments.
#[allow(clippy::too_many_lines)] // a flat panel layout reads better unsplit
pub fn build(
    ctx: &egui::Context,
    editor: &mut Editor,
    actions: &mut UiActions,
    highlight: &[[(f64, f64); 2]],
) {
    let lang = editor.lang;
    let t = |m: Msg| tr(lang, m);

    egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button(t(Msg::File), |ui| {
                if ui.button(t(Msg::New)).clicked() {
                    actions.new_model = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button(t(Msg::OpenKv6)).clicked() {
                    actions.open_kv6 = true;
                    ui.close_menu();
                }
                if ui.button(t(Msg::SaveKv6)).clicked() {
                    actions.save_kv6 = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button(t(Msg::OpenProject)).clicked() {
                    actions.open_project = true;
                    ui.close_menu();
                }
                if ui.button(t(Msg::SaveProject)).clicked() {
                    actions.save_project = true;
                    ui.close_menu();
                }
            });
            ui.menu_button(t(Msg::Edit), |ui| {
                if ui
                    .add_enabled(editor.document.can_undo(), egui::Button::new(t(Msg::Undo)))
                    .clicked()
                {
                    actions.undo = true;
                    ui.close_menu();
                }
                if ui
                    .add_enabled(editor.document.can_redo(), egui::Button::new(t(Msg::Redo)))
                    .clicked()
                {
                    actions.redo = true;
                    ui.close_menu();
                }
            });
            ui.menu_button(t(Msg::View), |ui| {
                ui.checkbox(&mut editor.lighting, t(Msg::Lighting));
            });
            ui.menu_button(t(Msg::Language), |ui| {
                for l in Lang::all() {
                    ui.selectable_value(&mut editor.lang, l, l.native_name());
                }
            });
        });
    });

    egui::SidePanel::left("tools")
        .default_width(200.0)
        .show(ctx, |ui| {
            ui.heading(t(Msg::Tools));
            for (tool, msg) in [
                (Tool::Place, Msg::Place),
                (Tool::Erase, Msg::Erase),
                (Tool::Paint, Msg::Paint),
                (Tool::Eyedropper, Msg::Eyedropper),
                (Tool::Box, Msg::BoxTool),
                (Tool::Sphere, Msg::Sphere),
                (Tool::Fill, Msg::FloodFill),
            ] {
                ui.selectable_value(&mut editor.tool, tool, t(msg));
            }
            if editor.tool == Tool::Sphere {
                ui.add(egui::Slider::new(&mut editor.radius, 0..=8).text(t(Msg::Radius)));
            }

            ui.separator();
            ui.label(t(Msg::Colour));
            let mut rgb = color_to_rgb(editor.color);
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                editor.color = rgb_to_color(rgb);
            }
            ui.horizontal_wrapped(|ui| {
                for &preset in &PRESETS {
                    if swatch(ui, preset).clicked() {
                        editor.color = preset;
                    }
                }
            });

            // Colours already used in the model, so artists can re-pick an
            // exact existing shade. Cloned out first to avoid borrowing
            // `editor` immutably while the closure writes `editor.color`.
            if !editor.model_palette.is_empty() {
                ui.separator();
                ui.label(t(Msg::ModelColours));
                let used = editor.model_palette.clone();
                ui.horizontal_wrapped(|ui| {
                    for c in used {
                        if swatch(ui, c).clicked() {
                            editor.color = c;
                        }
                    }
                });
            }

            ui.separator();
            ui.label(t(Msg::Mirror));
            ui.horizontal(|ui| {
                ui.checkbox(&mut editor.document.mirror[0], "X");
                ui.checkbox(&mut editor.document.mirror[1], "Y");
                ui.checkbox(&mut editor.document.mirror[2], "Z");
            });

            ui.separator();
            ui.label(t(Msg::Pivot));
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
            ui.label(format!("{}  {dx} × {dy} × {dz}", t(Msg::Size)));
            ui.label(format!(
                "{}  {}",
                t(Msg::Voxels),
                editor.document.model().occupied_count()
            ));

            ui.separator();
            ui.small(t(Msg::HelpApply));
            ui.small(t(Msg::HelpOrbit));
        });

    // Paint the hover wire box over the 3D render. A bare layer painter
    // (not a panel) is used on purpose: a CentralPanel would register an
    // interactive area over the whole viewport and swallow clicks /
    // scroll. Clip to the region the panels leave so it never draws over
    // them.
    if !highlight.is_empty() {
        let ppp = f64::from(ctx.pixels_per_point());
        let painter = ctx
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("voxel-highlight"),
            ))
            .with_clip_rect(ctx.available_rect());
        let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 230, 0));
        for seg in highlight {
            painter.line_segment([to_point(seg[0], ppp), to_point(seg[1], ppp)], stroke);
        }
    }
}

/// Framebuffer pixel -> egui point (logical).
#[allow(clippy::cast_possible_truncation)] // screen coords fit f32 comfortably
fn to_point((x, y): (f64, f64), ppp: f64) -> egui::Pos2 {
    egui::pos2((x / ppp) as f32, (y / ppp) as f32)
}

/// A small square colour button for `0x80RRGGBB`.
fn swatch(ui: &mut egui::Ui, color: u32) -> egui::Response {
    let [r, g, b] = color_to_rgb(color);
    ui.add(
        egui::Button::new("")
            .min_size(egui::vec2(18.0, 18.0))
            .fill(egui::Color32::from_rgb(r, g, b)),
    )
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
