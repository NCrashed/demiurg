//! The egui editor UI: a top menu bar and a left tool panel. The panels
//! mutate the [`Editor`] in place (tool, colour, mirror, pivot, language)
//! and record one-shot menu choices in [`UiActions`] for the host to run
//! after the frame (file dialogs, undo/redo) — egui closures can't borrow
//! the renderer, so deferred actions keep the borrow graph clean.
//!
//! All user-facing strings come from [`demiurg_i18n`]; the language is
//! `editor.lang`, switchable live from the Language menu.

use demiurg_i18n::{Lang, Msg, tr};
use demiurg_view::RenderMode;
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
    pub save_vxl: bool,
    pub open_project: bool,
    pub save_project: bool,
    pub undo: bool,
    pub redo: bool,
    /// Quit-confirmation modal: the user chose to quit / to cancel.
    pub quit_confirm: bool,
    pub quit_cancel: bool,
}

/// Draw the editor chrome for one frame (menus, tool panel, quit modal).
/// The 3D reference lines / hover box are drawn by the host via
/// `SceneRenderer::draw_lines`, not here.
#[allow(clippy::too_many_lines)] // a flat panel layout reads better unsplit
pub fn build(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    actions: &mut UiActions,
    show_quit_confirm: bool,
) {
    let lang = editor.lang;
    let t = |m: Msg| tr(lang, m);

    egui::Panel::top("menubar").show_inside(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(t(Msg::File), |ui| {
                if ui.button(t(Msg::New)).clicked() {
                    actions.new_model = true;
                    ui.close();
                }
                ui.separator();
                if ui.button(t(Msg::OpenKv6)).clicked() {
                    actions.open_kv6 = true;
                    ui.close();
                }
                if ui.button(t(Msg::SaveKv6)).clicked() {
                    actions.save_kv6 = true;
                    ui.close();
                }
                if ui.button(t(Msg::SaveVxl)).clicked() {
                    actions.save_vxl = true;
                    ui.close();
                }
                ui.separator();
                if ui.button(t(Msg::OpenProject)).clicked() {
                    actions.open_project = true;
                    ui.close();
                }
                if ui.button(t(Msg::SaveProject)).clicked() {
                    actions.save_project = true;
                    ui.close();
                }
            });
            ui.menu_button(t(Msg::Edit), |ui| {
                if ui
                    .add_enabled(editor.document.can_undo(), egui::Button::new(t(Msg::Undo)))
                    .clicked()
                {
                    actions.undo = true;
                    ui.close();
                }
                if ui
                    .add_enabled(editor.document.can_redo(), egui::Button::new(t(Msg::Redo)))
                    .clicked()
                {
                    actions.redo = true;
                    ui.close();
                }
            });
            ui.menu_button(t(Msg::View), |ui| {
                ui.checkbox(&mut editor.lighting, t(Msg::Lighting));
                ui.checkbox(&mut editor.show_grid, t(Msg::Grid));
                ui.separator();
                ui.label(t(Msg::Render));
                if ui
                    .selectable_value(
                        &mut editor.render_mode,
                        RenderMode::Voxel,
                        t(Msg::RenderVoxel),
                    )
                    .changed()
                {
                    editor.dirty = true;
                }
                if ui
                    .selectable_value(
                        &mut editor.render_mode,
                        RenderMode::Sprite,
                        t(Msg::RenderSprite),
                    )
                    .changed()
                {
                    editor.dirty = true;
                }
            });
            ui.menu_button(t(Msg::Language), |ui| {
                for l in Lang::all() {
                    ui.selectable_value(&mut editor.lang, l, l.native_name());
                }
            });
        });
    });

    egui::Panel::left("tools")
        .default_size(200.0)
        .show_inside(ui, |ui| {
            ui.heading(t(Msg::Tools));
            // The 1-7 digits double as keyboard shortcuts (see on_key);
            // show them on the buttons so they're discoverable.
            for (i, (tool, msg)) in [
                (Tool::Place, Msg::Place),
                (Tool::Erase, Msg::Erase),
                (Tool::Paint, Msg::Paint),
                (Tool::Eyedropper, Msg::Eyedropper),
                (Tool::Box, Msg::BoxTool),
                (Tool::Sphere, Msg::Sphere),
                (Tool::Fill, Msg::FloodFill),
            ]
            .into_iter()
            .enumerate()
            {
                ui.selectable_value(&mut editor.tool, tool, format!("{}.  {}", i + 1, t(msg)));
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

    // In-app quit confirmation (replaces a native message box, which the
    // XDG portal doesn't reliably show here).
    if show_quit_confirm {
        egui::Window::new(t(Msg::ConfirmQuitTitle))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.label(t(Msg::ConfirmQuitBody));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t(Msg::QuitAnyway)).clicked() {
                        actions.quit_confirm = true;
                    }
                    if ui.button(t(Msg::Cancel)).clicked() {
                        actions.quit_cancel = true;
                    }
                });
            });
    }
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
