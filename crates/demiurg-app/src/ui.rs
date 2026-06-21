//! The egui editor UI: a top menu bar and a left tool panel. The panels
//! mutate the [`Editor`] in place (tool, colour, mirror, pivot, language)
//! and record one-shot menu choices in [`UiActions`] for the host to run
//! after the frame (file dialogs, undo/redo) — egui closures can't borrow
//! the renderer, so deferred actions keep the borrow graph clean.
//!
//! All user-facing strings come from [`demiurg_i18n`]; the language is
//! `editor.lang`, switchable live from the Language menu.

use demiurg_i18n::{Lang, Msg, tr};
use demiurg_view::{AXIS_COLORS, RenderMode, ViewDir};
use roxlap_render::egui;

use crate::reference::RefAxis;
use crate::{Editor, RigMode, Tool};

/// Build stamp shown at the foot of the tool panel: the crate version and
/// the git commit it was built from (stamped by `build.rs`).
const BUILD_INFO: &str = concat!(
    "demiurg ",
    env!("CARGO_PKG_VERSION"),
    " · ",
    env!("DEMIURG_GIT_COMMIT"),
);

/// The viewport axis colour (X red, Y green, Z blue) as an egui colour,
/// so panel axis labels match the gizmo.
#[allow(clippy::cast_possible_truncation)] // channels masked to 0..=255
fn axis_color(axis: usize) -> egui::Color32 {
    let c = AXIS_COLORS[axis];
    egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

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
    pub open_vox: bool,
    pub open_project: bool,
    /// Open a reference image (file dialog).
    pub open_reference: bool,
    /// Open a `.rkc` rigged character (file dialog).
    pub open_character: bool,
    /// Remove the current reference image.
    pub remove_reference: bool,
    /// Save the project (Ctrl+S): overwrite the known path or prompt.
    pub save: bool,
    /// Save the project to a new path (dialog).
    pub save_as: bool,
    pub export_kv6: bool,
    pub export_vxl: bool,
    pub export_vox: bool,
    /// Export the rig as a `.rkc` character.
    pub export_rkc: bool,
    /// Switch the active rig bone (index into `rig.bones`).
    pub select_bone: Option<usize>,
    /// Append a new bone as a child of the active bone.
    pub add_bone: bool,
    /// Duplicate the bone at this index (the active bone) as a sibling.
    pub duplicate_bone: Option<usize>,
    /// Delete the bone at this index (the active bone).
    pub delete_bone: Option<usize>,
    /// Switch the rig sub-mode (Sculpt / Skeleton / Animate).
    pub set_rig_mode: Option<RigMode>,
    pub undo: bool,
    pub redo: bool,
    pub delete_sel: bool,
    pub copy_sel: bool,
    pub paste_sel: bool,
    /// A camera view-preset button was clicked this frame.
    pub set_view: Option<ViewDir>,
    /// Quit-confirmation modal: the user chose to quit / to cancel.
    pub quit_confirm: bool,
    pub quit_cancel: bool,
    /// The autosave-recovery banner was dismissed.
    pub recovered_ok: bool,
}

/// Which overlay modals to draw this frame.
#[derive(Clone, Copy)]
pub struct Modals {
    /// The unsaved-changes quit confirmation.
    pub quit_confirm: bool,
    /// A user save is in flight (show a blocking spinner).
    pub saving: bool,
    /// Work was recovered from an autosave (show a dismissible banner).
    pub recovered: bool,
}

/// Draw the editor chrome for one frame (menus, tool panel, quit modal).
/// The 3D reference lines / hover box are drawn by the host via
/// `SceneRenderer::draw_lines`, not here.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)] // flat panel; dims are tiny
pub fn build(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    actions: &mut UiActions,
    modals: Modals,
    marquee: Option<[(f64, f64); 2]>,
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
                if ui.button(t(Msg::OpenVox)).clicked() {
                    actions.open_vox = true;
                    ui.close();
                }
                if ui.button(t(Msg::OpenProject)).clicked() {
                    actions.open_project = true;
                    ui.close();
                }
                if ui.button(t(Msg::OpenReference)).clicked() {
                    actions.open_reference = true;
                    ui.close();
                }
                if ui.button(t(Msg::OpenCharacter)).clicked() {
                    actions.open_character = true;
                    ui.close();
                }
                ui.separator();
                if ui.button(t(Msg::Save)).clicked() {
                    actions.save = true;
                    ui.close();
                }
                if ui.button(t(Msg::SaveAs)).clicked() {
                    actions.save_as = true;
                    ui.close();
                }
                ui.separator();
                if ui.button(t(Msg::ExportKv6)).clicked() {
                    actions.export_kv6 = true;
                    ui.close();
                }
                if ui.button(t(Msg::ExportVxl)).clicked() {
                    actions.export_vxl = true;
                    ui.close();
                }
                if ui.button(t(Msg::ExportVox)).clicked() {
                    actions.export_vox = true;
                    ui.close();
                }
                if editor.rig.is_some() {
                    ui.separator();
                    if ui.button(t(Msg::ExportCharacter)).clicked() {
                        actions.export_rkc = true;
                        ui.close();
                    }
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
                ui.checkbox(&mut editor.show_edges, t(Msg::VoxelEdges));
                ui.checkbox(&mut editor.flip_x, t(Msg::FlipX));
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
            // Build stamp at the far right: version + the git commit this
            // binary was built from (selectable so it copies into a bug
            // report). See BUILD_INFO / build.rs.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(BUILD_INFO).small().weak())
                        .selectable(true),
                );
            });
        });
    });

    egui::Panel::left("tools")
        .default_size(200.0)
        .show_inside(ui, |ui| {
            // Scroll the tool panel: with all sections it can exceed a
            // short window's height.
            egui::ScrollArea::vertical().show(ui, |ui| {
                // Panels are scoped to the editing context so each mode
                // shows only what applies (a posed-rig mode hides the voxel
                // tools, a plain model hides the rig controls):
                //   Model           — voxel tools only
                //   Rig ▸ Sculpt    — rig header + voxel tools (edit a bone)
                //   Rig ▸ Skeleton  — rig header + hinge editor
                //   Rig ▸ Animate   — rig header + animation (read-only)
                let rig_mode = editor.rig.is_some().then_some(editor.rig_mode);
                if editor.rig.is_some() {
                    rig_panel(ui, editor, actions, &t);
                }
                match rig_mode {
                    None | Some(RigMode::Sculpt) => voxel_tools_panel(ui, editor, actions, &t),
                    Some(RigMode::Skeleton) => skeleton_panel(ui, editor, &t),
                    Some(RigMode::Animate) => animate_panel(ui, editor, &t),
                }
                views_panel(ui, actions, &t);

                ui.separator();
                if matches!(rig_mode, None | Some(RigMode::Sculpt)) {
                    if editor.tool == Tool::Select {
                        ui.small(t(Msg::HelpSelect));
                    } else {
                        ui.small(t(Msg::HelpApply));
                    }
                }
                ui.small(t(Msg::HelpOrbit));
            });
        });

    draw_marquee(ui, marquee);

    // In-app quit confirmation (replaces a native message box, which the
    // XDG portal doesn't reliably show here).
    if modals.quit_confirm {
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

    // Saving spinner: the write runs on a worker thread, so this animates
    // (the loop keeps rendering) — the OS sees a responsive window instead
    // of a frozen one it would offer to kill.
    if modals.saving {
        egui::Window::new(t(Msg::Saving))
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label(t(Msg::Saving));
                });
            });
    }

    // Autosave-recovery banner (dismissible).
    if modals.recovered {
        egui::Window::new(t(Msg::RecoveredTitle))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.label(t(Msg::RecoveredBody));
                ui.add_space(8.0);
                if ui.button(t(Msg::Ok)).clicked() {
                    actions.recovered_ok = true;
                }
            });
    }
}

fn dims_arr(d: (u32, u32, u32)) -> [u32; 3] {
    [d.0, d.1, d.2]
}

/// The Size section: model dims + voxel count, crop-to-content, an exact
/// resize, and per-direction grow buttons. The ops mutate the document
/// directly (undoable structural edits) and resync the size fields.
fn size_panel(ui: &mut egui::Ui, editor: &mut Editor, t: &impl Fn(Msg) -> &'static str) {
    ui.separator();
    let (dx, dy, dz) = editor.document.dims();
    ui.label(format!("{}  {dx} × {dy} × {dz}", t(Msg::Size)));
    ui.label(format!(
        "{}  {}",
        t(Msg::Voxels),
        editor.document.model().occupied_count()
    ));

    if ui.button(t(Msg::Crop)).clicked() && editor.document.crop_to_content() {
        editor.resize_dims = dims_arr(editor.document.dims());
        editor.dirty = true;
    }

    ui.horizontal(|ui| {
        for (axis, name) in [(0, "x"), (1, "y"), (2, "z")] {
            ui.colored_label(axis_color(axis), name);
            ui.add(egui::DragValue::new(&mut editor.resize_dims[axis]).range(1..=256));
        }
    });
    if ui.button(t(Msg::Resize)).clicked() && editor.document.resize(editor.resize_dims) {
        editor.dirty = true;
    }

    ui.label(t(Msg::Grow));
    ui.horizontal(|ui| {
        for (axis, name) in [(0usize, "X"), (1, "Y"), (2, "Z")] {
            let col = axis_color(axis);
            if ui
                .small_button(egui::RichText::new(format!("−{name}")).color(col))
                .clicked()
            {
                editor.document.grow(axis, false);
                editor.resize_dims = dims_arr(editor.document.dims());
                editor.dirty = true;
            }
            if ui
                .small_button(egui::RichText::new(format!("+{name}")).color(col))
                .clicked()
            {
                editor.document.grow(axis, true);
                editor.resize_dims = dims_arr(editor.document.dims());
                editor.dirty = true;
            }
        }
    });
}

/// The Selection section: the selected-voxel count and delete / copy /
/// paste buttons. The buttons only record an intent in [`UiActions`]; the
/// host applies them (it owns the selection + clipboard).
fn selection_panel(
    ui: &mut egui::Ui,
    editor: &Editor,
    actions: &mut UiActions,
    t: &impl Fn(Msg) -> &'static str,
) {
    ui.separator();
    ui.label(format!("{}  {}", t(Msg::Selected), editor.selection.len()));
    let has_sel = !editor.selection.is_empty();
    let has_clip = !editor.clipboard.is_empty();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(has_sel, egui::Button::new(t(Msg::Delete)))
            .clicked()
        {
            actions.delete_sel = true;
        }
        if ui
            .add_enabled(has_sel, egui::Button::new(t(Msg::Copy)))
            .clicked()
        {
            actions.copy_sel = true;
        }
        if ui
            .add_enabled(has_clip, egui::Button::new(t(Msg::Paste)))
            .clicked()
        {
            actions.paste_sel = true;
        }
    });
}

/// The Rig section (rig mode): the character name, the Sculpt / Animate
/// sub-mode tabs, and the bone list. Switching mode or bone is deferred to
/// the host, which swaps the scene / working model.
fn rig_panel(
    ui: &mut egui::Ui,
    editor: &Editor,
    actions: &mut UiActions,
    t: &impl Fn(Msg) -> &'static str,
) {
    let Some(rig) = &editor.rig else {
        return;
    };
    let heading = if rig.name.is_empty() {
        t(Msg::Rig).to_string()
    } else {
        rig.name.clone()
    };
    ui.heading(heading);
    // Sub-mode tabs: Sculpt (edit a bone mesh), Skeleton (edit hinges),
    // Animate (posed preview). Clicking a tab records the target mode; the
    // host does the scene / camera swap.
    ui.horizontal(|ui| {
        for (mode, label) in [
            (RigMode::Sculpt, t(Msg::Sculpt)),
            (RigMode::Skeleton, t(Msg::Skeleton)),
            (RigMode::Animate, t(Msg::Animate)),
        ] {
            if ui
                .selectable_label(editor.rig_mode == mode, label)
                .clicked()
            {
                actions.set_rig_mode = Some(mode);
            }
        }
    });
    ui.separator();
    ui.label(t(Msg::Bones));
    for (i, bone) in rig.bones.iter().enumerate() {
        let label = if bone.name.is_empty() {
            format!("{i}")
        } else {
            format!("{i}  {}", bone.name)
        };
        if ui
            .selectable_label(i == editor.active_bone, label)
            .clicked()
        {
            actions.select_bone = Some(i);
        }
    }
    // Add (appends a child of the active bone) / Duplicate (sibling copy of
    // the active bone) / Delete (removes the active bone). Delete is disabled
    // for the last bone or a root — the rig must always keep a root, and clips
    // need at least one column.
    ui.separator();
    let active = editor.active_bone;
    let has_active = rig.bones.get(active).is_some();
    let can_delete =
        rig.bones.len() > 1 && rig.bones.get(active).is_some_and(|b| b.hinge.parent >= 0);
    ui.horizontal(|ui| {
        if ui.button(t(Msg::AddBone)).clicked() {
            actions.add_bone = true;
        }
        if ui
            .add_enabled(has_active, egui::Button::new(t(Msg::DuplicateBone)))
            .clicked()
        {
            actions.duplicate_bone = Some(active);
        }
        if ui
            .add_enabled(can_delete, egui::Button::new(t(Msg::DeleteBone)))
            .clicked()
        {
            actions.delete_bone = Some(active);
        }
    });
}

/// The Animation section (Rig ▸ Animate): the rig's clips. Read-only for
/// now — playback runs automatically; a scrubbing timeline comes later.
fn animate_panel(ui: &mut egui::Ui, editor: &Editor, t: &impl Fn(Msg) -> &'static str) {
    let Some(rig) = &editor.rig else {
        return;
    };
    ui.separator();
    ui.label(t(Msg::Clips));
    if rig.clips.is_empty() {
        ui.small("—");
    } else {
        for clip in &rig.clips {
            ui.small(clip.name.as_str());
        }
    }
}

/// Walk parent links from `start` upward (using the snapshot `parents`); is
/// `child` reachable? Used to reject a reparent that would form a cycle.
#[allow(clippy::cast_sign_loss)] // p >= 0 is guaranteed by the loop
fn reaches(parents: &[i32], start: i32, child: usize) -> bool {
    let mut p = start;
    while p >= 0 {
        let pi = p as usize;
        if pi == child {
            return true;
        }
        p = parents[pi];
    }
    false
}

/// The Skeleton section (Rig ▸ Skeleton): edit the active bone's hinge —
/// name, parent, joint position (where it attaches to its parent), and
/// rotation axis. Mutates the rig in place and flags `rig_dirty` so the
/// rest-pose preview rebuilds.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)] // bone counts are tiny
fn skeleton_panel(ui: &mut egui::Ui, editor: &mut Editor, t: &impl Fn(Msg) -> &'static str) {
    let active = editor.active_bone;
    let Some(rig) = &mut editor.rig else {
        return;
    };
    if active >= rig.bones.len() {
        return;
    }
    let n = rig.bones.len() as i32;
    let parents: Vec<i32> = rig.bones.iter().map(|b| b.hinge.parent).collect();
    let bone = &mut rig.bones[active];
    let mut changed = false;

    ui.separator();
    ui.label(t(Msg::Skeleton));
    changed |= ui.text_edit_singleline(&mut bone.name).changed();

    ui.horizontal(|ui| {
        ui.label(t(Msg::Parent));
        let mut parent = bone.hinge.parent;
        if ui
            .add(egui::DragValue::new(&mut parent).range(-1..=(n - 1)))
            .changed()
            && parent != bone.hinge.parent
            && (parent < 0 || (parent as usize != active && !reaches(&parents, parent, active)))
        {
            bone.hinge.parent = parent;
            changed = true;
        }
    });

    // Joint: the parent-side velcro (p[1]) — where on the parent this bone
    // attaches, so dragging it moves the bone in the rest pose.
    ui.label(t(Msg::Joint));
    ui.horizontal(|ui| {
        for (axis, name) in [(0usize, "x"), (1, "y"), (2, "z")] {
            ui.colored_label(axis_color(axis), name);
            let f = match axis {
                0 => &mut bone.hinge.p[1].x,
                1 => &mut bone.hinge.p[1].y,
                _ => &mut bone.hinge.p[1].z,
            };
            changed |= ui.add(egui::DragValue::new(f).speed(0.5)).changed();
        }
    });

    // Rotation axis — pick a principal axis (always a unit vector, so the
    // pose never distorts; a free numeric axis is too easy to leave
    // non-normalised).
    ui.label(t(Msg::Axis));
    let cur = [bone.hinge.v[0].x, bone.hinge.v[0].y, bone.hinge.v[0].z];
    ui.horizontal(|ui| {
        for (axis, name) in [(0usize, "X"), (1, "Y"), (2, "Z")] {
            let mut unit = [0.0f32; 3];
            unit[axis] = 1.0;
            #[allow(clippy::float_cmp)] // unit axes are exact 0.0 / 1.0
            let active = cur == unit;
            if ui
                .selectable_label(active, egui::RichText::new(name).color(axis_color(axis)))
                .clicked()
            {
                bone.hinge.v[0].x = unit[0];
                bone.hinge.v[0].y = unit[1];
                bone.hinge.v[0].z = unit[2];
                changed = true;
            }
        }
    });
    if changed {
        bone.hinge.v[1] = bone.hinge.v[0]; // mirror the axis to the parent side
        editor.rig_dirty = true;
    }
}

/// The voxel-editing tools — shown in plain Model mode and in Rig ▸ Sculpt
/// (editing the active bone's mesh): tool picker, paint colour, symmetry,
/// pivot, model size, selection, and the reference-image guide.
#[allow(clippy::cast_precision_loss)] // pivot is centred from small voxel dims
fn voxel_tools_panel(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    actions: &mut UiActions,
    t: &impl Fn(Msg) -> &'static str,
) {
    ui.heading(t(Msg::Tools));
    // The 1-8 digits double as keyboard shortcuts (see on_key); show them on
    // the buttons so they're discoverable.
    for (i, (tool, msg)) in [
        (Tool::Place, Msg::Place),
        (Tool::Erase, Msg::Erase),
        (Tool::Paint, Msg::Paint),
        (Tool::Eyedropper, Msg::Eyedropper),
        (Tool::Box, Msg::BoxTool),
        (Tool::Sphere, Msg::Sphere),
        (Tool::Fill, Msg::FloodFill),
        (Tool::Select, Msg::Select),
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

    // Colours already used in the model, so artists can re-pick an exact
    // existing shade. Cloned out first to avoid borrowing `editor` immutably
    // while the closure writes `editor.color`.
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
        for (axis, name) in [(0, "X"), (1, "Y"), (2, "Z")] {
            ui.checkbox(
                &mut editor.document.mirror[axis],
                egui::RichText::new(name).color(axis_color(axis)),
            );
        }
    });

    ui.separator();
    ui.label(t(Msg::Pivot));
    let mut pivot = editor.document.pivot();
    let mut changed = false;
    ui.horizontal(|ui| {
        for (axis, name) in [(0, "x"), (1, "y"), (2, "z")] {
            ui.colored_label(axis_color(axis), name);
            changed |= ui
                .add(egui::DragValue::new(&mut pivot[axis]).speed(0.5))
                .changed();
        }
    });
    if changed {
        editor.document.set_pivot(pivot);
        editor.dirty = true;
    }
    if ui.button(t(Msg::CenterPivot)).clicked() {
        let (dx, dy, dz) = editor.document.dims();
        editor
            .document
            .set_pivot([dx as f32 * 0.5, dy as f32 * 0.5, dz as f32 * 0.5]);
        editor.dirty = true;
    }

    size_panel(ui, editor, t);
    selection_panel(ui, editor, actions, t);
    reference_panel(ui, editor, actions, t);
}

/// The Reference section: load a pixel-art guide and place it. When one is
/// loaded, shows its name/size and controls for the plane (Front/Side/Top),
/// depth offset, horizontal/vertical flips, visibility, and removal. Edits
/// mutate the reference directly and flag it for a viewport refresh.
fn reference_panel(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    actions: &mut UiActions,
    t: &impl Fn(Msg) -> &'static str,
) {
    ui.separator();
    ui.label(t(Msg::Reference));
    if editor.reference.is_none() {
        if ui.button(t(Msg::OpenReference)).clicked() {
            actions.open_reference = true;
        }
        return;
    }

    // The Move toggle lives on the editor, not the reference, so handle it
    // before borrowing the reference for the rest of the controls. When on,
    // a left-drag in the viewport slides the reference in its plane.
    ui.checkbox(&mut editor.ref_move_mode, t(Msg::Move))
        .on_hover_text(t(Msg::Move));

    // Axis / depth / flip / visibility all only affect how the overlay is
    // projected (recomputed every frame), so none of them touch the texture.
    let mut remove = false;
    if let Some(r) = &mut editor.reference {
        ui.small(format!("{}  {}×{}", r.name, r.width, r.height));
        ui.horizontal(|ui| {
            for (axis, msg) in [
                (RefAxis::Front, Msg::Front),
                (RefAxis::Side, Msg::Side),
                (RefAxis::Top, Msg::Top),
            ] {
                ui.selectable_value(&mut r.axis, axis, t(msg));
            }
        });
        ui.horizontal(|ui| {
            ui.label(t(Msg::Depth));
            ui.add(egui::DragValue::new(&mut r.depth));
            ui.checkbox(&mut r.flip_h, "↔");
            ui.checkbox(&mut r.flip_v, "↕");
        });
        ui.horizontal(|ui| {
            ui.label(t(Msg::Opacity));
            ui.add(egui::Slider::new(&mut r.opacity, 0.0..=1.0).show_value(false));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut r.visible, t(Msg::Show));
            remove = ui.button(t(Msg::Remove)).clicked();
        });
    }
    if remove {
        actions.remove_reference = true;
    }
}

/// The Views section: six buttons that snap the camera to an axis-aligned
/// view, grouped and coloured by axis (X red, Y green, Z blue) to match
/// the viewport gizmo. The click records the choice in [`UiActions`]; the
/// host applies it (it owns the camera).
fn views_panel(ui: &mut egui::Ui, actions: &mut UiActions, t: &impl Fn(Msg) -> &'static str) {
    ui.separator();
    ui.label(t(Msg::Views));
    // One row per axis: (negative dir, positive dir, axis colour index).
    for (a, b, axis) in [
        (Msg::Front, Msg::Back, 0usize),
        (Msg::Left, Msg::Right, 1),
        (Msg::Top, Msg::Bottom, 2),
    ] {
        let col = axis_color(axis);
        ui.horizontal(|ui| {
            if ui
                .small_button(egui::RichText::new(t(a)).color(col))
                .clicked()
            {
                actions.set_view = Some(view_dir(a));
            }
            if ui
                .small_button(egui::RichText::new(t(b)).color(col))
                .clicked()
            {
                actions.set_view = Some(view_dir(b));
            }
        });
    }
}

/// Map a view-label message to its camera direction.
fn view_dir(m: Msg) -> ViewDir {
    match m {
        Msg::Back => ViewDir::Back,
        Msg::Left => ViewDir::Left,
        Msg::Right => ViewDir::Right,
        Msg::Top => ViewDir::Top,
        Msg::Bottom => ViewDir::Bottom,
        _ => ViewDir::Front,
    }
}

/// Draw the live marquee rectangle (Select-tool drag) as a 2D overlay.
/// `marquee` corners are framebuffer pixels; egui works in points, so
/// divide by the pixel ratio.
#[allow(clippy::cast_possible_truncation)] // pixel coords are small, well within f32
fn draw_marquee(ui: &egui::Ui, marquee: Option<[(f64, f64); 2]>) {
    let Some([(ax, ay), (bx, by)]) = marquee else {
        return;
    };
    let ppp = f64::from(ui.ctx().pixels_per_point());
    let p = |x: f64, y: f64| egui::pos2((x / ppp) as f32, (y / ppp) as f32);
    let rect = egui::Rect::from_two_pos(p(ax, ay), p(bx, by));
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("marquee"),
    ));
    painter.rect_filled(
        rect,
        egui::CornerRadius::ZERO,
        egui::Color32::from_rgba_unmultiplied(60, 200, 200, 40),
    );
    painter.rect_stroke(
        rect,
        egui::CornerRadius::ZERO,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 220, 220)),
        egui::StrokeKind::Middle,
    );
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
