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

use demiurg_core::{LoopMode, Quat};

use std::path::PathBuf;

use crate::reference::RefAxis;
use crate::{Editor, GizmoMode, RigMode, Tool};

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
    /// Start a fresh rig (one root bone), or wrap the current model as a rig.
    pub new_rig: bool,
    pub convert_to_rig: bool,
    /// Open any supported document (`.demiurg` / `.rkc` / `.kv6` / `.vox`) — a
    /// single file dialog; the loader detects the format from the extension.
    pub open: bool,
    /// Reopen this recently used document (File ▸ Open recent).
    pub open_recent: Option<PathBuf>,
    /// Forget the recent-files list (File ▸ Open recent ▸ Clear recent).
    pub clear_recent: bool,
    /// Open a reference image (file dialog).
    pub open_reference: bool,
    /// Paste an image from the system clipboard as the reference.
    pub paste_reference: bool,
    /// Remove the current reference image.
    pub remove_reference: bool,
    /// Extract the current selection into a new child bone (rig slicing).
    pub extract_to_bone: bool,
    /// Extract the current selection into a new extra attachment on the active
    /// bone (same bone, new offsettable layer).
    pub extract_to_attachment: bool,
    /// Rotate the selection 90 degrees about `editor.rotate_axis`:
    /// `Some(true)` = clockwise, `Some(false)` = counter-clockwise.
    pub rotate_sel: Option<bool>,
    /// Start a fresh animated voxel clip (one empty frame).
    pub new_clip: bool,
    /// Save the project (Ctrl+S): overwrite the known path or prompt.
    pub save: bool,
    /// Save the project to a new path (dialog).
    pub save_as: bool,
    pub export_kv6: bool,
    pub export_vxl: bool,
    pub export_vox: bool,
    /// Export the rig as a `.rkc` character.
    pub export_rkc: bool,
    /// Export the clip as a `.rvc` voxel clip.
    pub export_rvc: bool,
    /// Clip editor: switch the active frame (index into `clip.frames`).
    pub select_frame: Option<usize>,
    /// Clip editor: append a new empty frame after the active one.
    pub add_frame: bool,
    /// Clip editor: duplicate the frame at this index.
    pub duplicate_frame: Option<usize>,
    /// Clip editor: delete the frame at this index.
    pub delete_frame: Option<usize>,
    /// Clip editor: set the clip's default per-frame duration (ms).
    pub set_clip_default_ms: Option<u32>,
    /// Clip editor: set frame `.0`'s duration override (`Some(ms)` overrides the
    /// clip default; `None` reverts to it).
    pub set_frame_duration: Option<(usize, Option<u32>)>,
    /// Clip editor: set how the clip loops.
    pub set_clip_loop_mode: Option<LoopMode>,
    /// Clip timeline: scrub the playhead to this absolute time (ms). Pauses.
    pub seek_clip: Option<u32>,
    /// Clip editor: crop every frame to the union of all frames' content.
    pub crop_clip: bool,
    /// Switch the active rig bone (index into `rig.bones`).
    pub select_bone: Option<usize>,
    /// Switch the active attachment of the active bone (`0` = primary mesh,
    /// `1..` = an extra).
    pub select_attachment: Option<usize>,
    /// Add a new extra attachment to the active bone.
    pub add_attachment: bool,
    /// Add a new extra attachment that is an animated clip.
    pub add_clip_layer: bool,
    /// Turn the active attachment into an animated clip (seeded from its mesh).
    pub make_clip_layer: bool,
    /// Import a `.rvc` file as a new clip layer (file dialog).
    pub import_clip_layer: bool,
    /// Remove the active (extra) attachment from the active bone.
    pub remove_attachment: bool,
    /// Append a new bone as a child of the active bone.
    pub add_bone: bool,
    /// Append a 3-axis (ball) joint under the active bone.
    pub add_axis_joint: bool,
    /// Wrap the rig's root in a dummy root so the old root is animatable.
    pub add_dummy_root: bool,
    /// Duplicate the bone at this index (the active bone) as a sibling.
    pub duplicate_bone: Option<usize>,
    /// Reorder the active bone: move it from `.0` to index `.1`.
    pub move_bone: Option<(usize, usize)>,
    /// Delete the bone at this index (the active bone).
    pub delete_bone: Option<usize>,
    /// A Skeleton-panel hinge edit (name / parent / joint) began this frame:
    /// capture the pre-edit rig for undo.
    pub rig_edit_begin: bool,
    /// A Skeleton-panel hinge edit changed a value this frame: commit the
    /// captured pre-edit snapshot as one undo step.
    pub rig_edit_changed: bool,
    /// Set the active bone's rotation axis to this principal axis (0=X,1=Y,2=Z).
    pub set_bone_axis: Option<usize>,
    /// Switch the rig sub-mode (Sculpt / Skeleton / Animate).
    pub set_rig_mode: Option<RigMode>,
    /// Animate timeline: toggle play / pause.
    pub toggle_play: bool,
    /// Animate timeline: seek the playhead to this absolute time (ms). Implies
    /// pause.
    pub seek: Option<i32>,
    /// Animate timeline: preview this clip (index into `rig.clips`).
    pub select_clip: Option<usize>,
    /// Animate: append a new clip.
    pub add_clip: bool,
    /// Animate: rename clip `.0` to `.1`.
    pub rename_clip: Option<(usize, String)>,
    /// Animate: delete the clip at this index.
    pub delete_clip: Option<usize>,
    /// Animate timeline: select this keyframe (index into the clip's sorted
    /// keys).
    pub select_key: Option<usize>,
    /// Animate timeline: add a keyframe at the playhead from the current pose.
    pub add_key: bool,
    /// Animate timeline: delete the selected keyframe.
    pub delete_key: bool,
    /// Animate timeline: copy / cut the selected key's pose to the key
    /// clipboard; paste it as a key at the playhead.
    pub copy_key: bool,
    pub cut_key: bool,
    pub paste_key: bool,
    /// Animate timeline: retime key `.0` to absolute ms `.1` (a tick drag).
    pub move_key: Option<(usize, i32)>,
    /// Animate timeline: set key `.0`'s bone `.1` angle to `.2` (angle editor).
    pub set_key_angle: Option<(usize, usize, i16)>,
    /// Animate: set key `.0`'s bone `.1` translation / scale to `.2` (the pose
    /// inspector).
    pub set_key_translation: Option<(usize, usize, [f32; 3])>,
    pub set_key_scale: Option<(usize, usize, [f32; 3])>,
    /// Animate: set key `.0`'s bone `.1` full rotation to `.2` (free 3-DOF,
    /// from the inspector's Euler fields).
    pub set_key_rotation: Option<(usize, usize, Quat)>,
    /// Animate: switch the viewport gizmo mode (rotate / translate).
    pub set_gizmo_mode: Option<GizmoMode>,
    /// Animate timeline: set the active clip's length (loop-marker ms).
    pub set_clip_length: Option<i32>,
    /// Animate timeline: toggle whether the active clip loops.
    pub set_clip_loops: Option<bool>,
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

/// The one piece of timeline state the panel can't read from the [`Editor`]:
/// the live playhead position. It lives in the host's `KfaView` (the baked
/// sprite's `kfatim`), so it's snapshotted before the UI frame. Keyframe times,
/// the loop length, and the selection all come from the rig / editor directly.
#[derive(Clone, Copy, Default)]
pub struct Timeline {
    /// Current playhead position (ms).
    pub time: i32,
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
    timeline: Timeline,
    recent: &[PathBuf],
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
                if ui.button(t(Msg::NewRig)).clicked() {
                    actions.new_rig = true;
                    ui.close();
                }
                if ui.button(t(Msg::NewClip)).clicked() {
                    actions.new_clip = true;
                    ui.close();
                }
                // Wrap the current model as a one-bone rig (only when not
                // already a rig).
                ui.add_enabled_ui(editor.rig.is_none(), |ui| {
                    if ui.button(t(Msg::ConvertToRig)).clicked() {
                        actions.convert_to_rig = true;
                        ui.close();
                    }
                });
                ui.separator();
                if ui.button(t(Msg::Open)).clicked() {
                    actions.open = true;
                    ui.close();
                }
                // Open recent: one entry per remembered document (newest first),
                // shown by file name with the full path on hover. Only present
                // when there's history.
                ui.add_enabled_ui(!recent.is_empty(), |ui| {
                    ui.menu_button(t(Msg::OpenRecent), |ui| {
                        for path in recent {
                            let label = path
                                .file_name()
                                .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy());
                            if ui
                                .button(label)
                                .on_hover_text(path.to_string_lossy())
                                .clicked()
                            {
                                actions.open_recent = Some(path.clone());
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button(t(Msg::ClearRecent)).clicked() {
                            actions.clear_recent = true;
                            ui.close();
                        }
                    });
                });
                if ui.button(t(Msg::OpenReference)).clicked() {
                    actions.open_reference = true;
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
                if editor.clip.is_some() {
                    ui.separator();
                    if ui.button(t(Msg::ExportClip)).clicked() {
                        actions.export_rvc = true;
                        ui.close();
                    }
                }
            });
            ui.menu_button(t(Msg::Edit), |ui| {
                if ui
                    .add_enabled(editor.can_undo(), egui::Button::new(t(Msg::Undo)))
                    .clicked()
                {
                    actions.undo = true;
                    ui.close();
                }
                if ui
                    .add_enabled(editor.can_redo(), egui::Button::new(t(Msg::Redo)))
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

    // The animation timeline lives in a full-width bar along the bottom (like
    // a video/animation editor), declared before the left panel so it spans
    // the whole window width. Only in Animate mode.
    if editor.rig.is_some() && editor.rig_mode == RigMode::Animate {
        egui::Panel::bottom("timeline")
            .exact_size(64.0)
            .show_inside(ui, |ui| {
                timeline_bar(ui, editor, actions, timeline, &t);
            });
    }
    // The clip's frame timeline shares the bottom-bar slot.
    if editor.clip.is_some() {
        egui::Panel::bottom("clip_timeline")
            .exact_size(48.0)
            .show_inside(ui, |ui| {
                clip_timeline_bar(ui, editor, actions, &t);
            });
    }

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
                // A clip shows its frames strip above the sculpt tools (which
                // edit the active frame). A clip keeps `rig == None`, so the
                // match below falls through to the voxel tools.
                if editor.clip.is_some() {
                    clip_panel(ui, editor, actions, &t);
                }
                match rig_mode {
                    None | Some(RigMode::Sculpt) => voxel_tools_panel(ui, editor, actions, &t),
                    Some(RigMode::Skeleton) => skeleton_panel(ui, editor, actions, &t),
                    // Animate: the clip library lives in the left panel; the
                    // timeline for the active clip is the bottom bar.
                    Some(RigMode::Animate) => clips_panel(ui, editor, actions, &t),
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
    // Rig slicing: carve the selection out into a new child bone, or into a new
    // extra attachment on the same bone (a separately-offsettable layer). Both
    // keep the piece in place at rest; only while editing a rig.
    if editor.rig.is_some() {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(has_sel, egui::Button::new(t(Msg::ExtractToBone)))
                .clicked()
            {
                actions.extract_to_bone = true;
            }
            if ui
                .add_enabled(has_sel, egui::Button::new(t(Msg::ExtractToAttachment)))
                .clicked()
            {
                actions.extract_to_attachment = true;
            }
        });
    }
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
    let count = rig.bones.len();
    let has_active = rig.bones.get(active).is_some();
    let can_delete = count > 1 && rig.bones.get(active).is_some_and(|b| b.hinge.parent >= 0);
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
    // Reorder the active bone in the list (purely organisational). Compact
    // arrows: up toward index 0, down toward the end.
    ui.horizontal(|ui| {
        if ui
            .add_enabled(has_active && active > 0, egui::Button::new("⏶").small())
            .on_hover_text(t(Msg::MoveBoneUp))
            .clicked()
        {
            actions.move_bone = Some((active, active - 1));
        }
        if ui
            .add_enabled(
                has_active && active + 1 < count,
                egui::Button::new("⏷").small(),
            )
            .on_hover_text(t(Msg::MoveBoneDown))
            .clicked()
        {
            actions.move_bone = Some((active, active + 1));
        }
    });
    // Rig-building helpers for the 1-DOF hinge format: a 3-axis ball joint
    // (a chain of X/Y/Z rotator bones) and a dummy root (so the real root
    // becomes animatable).
    ui.horizontal(|ui| {
        if ui
            .button(t(Msg::AxisJoint))
            .on_hover_text(t(Msg::AxisJoint))
            .clicked()
        {
            actions.add_axis_joint = true;
        }
        if ui.button(t(Msg::DummyRoot)).clicked() {
            actions.add_dummy_root = true;
        }
    });
}

/// The clip library (Rig ▸ Animate), in the left panel: a selectable list of
/// the rig's clips plus add / delete / rename. Selecting drives the bottom
/// timeline bar (which animates the active clip). Emits [`UiActions`]; the
/// rename field edits a per-frame copy of the name (the host applies it, so the
/// round-trip is invisible) under the inline begin/commit-pending undo step.
#[allow(clippy::too_many_lines)] // a flat panel: clip list + pose inspector + buttons
fn clips_panel(
    ui: &mut egui::Ui,
    editor: &Editor,
    actions: &mut UiActions,
    t: &impl Fn(Msg) -> &'static str,
) {
    let Some(rig) = &editor.rig else {
        return;
    };
    ui.separator();
    ui.label(t(Msg::Clips));
    if rig.clips.is_empty() {
        ui.small("—");
    } else {
        let active = editor.active_clip.min(rig.clips.len() - 1);
        for (i, clip) in rig.clips.iter().enumerate() {
            if ui
                .selectable_label(i == active, clip.name.as_str())
                .clicked()
            {
                actions.select_clip = Some(i);
            }
        }
        // Rename the active clip in place.
        let mut name = rig.clips[active].name.clone();
        let resp = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY));
        if resp.gained_focus() {
            actions.rig_edit_begin = true;
        }
        if resp.changed() {
            actions.rename_clip = Some((active, name));
            actions.rig_edit_changed = true;
        }
    }
    ui.horizontal(|ui| {
        if ui.button(t(Msg::AddClip)).clicked() {
            actions.add_clip = true;
        }
        ui.add_enabled_ui(!rig.clips.is_empty(), |ui| {
            if ui.button(t(Msg::DeleteClip)).clicked() {
                actions.delete_clip = Some(editor.active_clip.min(rig.clips.len() - 1));
            }
        });
    });
    // Pose inspector: numeric translation + scale for the active bone in the
    // selected key (rotation has the bottom-bar slider + viewport drag). Each
    // field drag is one undo step (the inline begin/commit-pending pair). Only
    // for a non-root bone (the solver ignores a root's transform).
    if !rig.clips.is_empty() {
        let clip = editor.active_clip.min(rig.clips.len() - 1);
        let keys = rig.clip_keyframes(clip);
        let bone = editor.active_bone;
        let sel = editor.selected_key.filter(|&k| k < keys.len());
        let non_root = rig.bones.get(bone).is_some_and(|b| b.hinge.parent >= 0);
        if let (Some(k), true) = (sel, non_root) {
            ui.separator();
            let xf = keys[k].xforms[bone];
            // One labelled row of 3 DragValues; emits `make` when any changes.
            let mut vec3_row =
                |ui: &mut egui::Ui,
                 label: Msg,
                 v: [f32; 3],
                 speed: f64,
                 make: &mut dyn FnMut(&mut UiActions, [f32; 3])| {
                    let mut out = v;
                    ui.horizontal(|ui| {
                        ui.label(t(label));
                        let mut changed = false;
                        for c in &mut out {
                            let resp = ui.add(egui::DragValue::new(c).speed(speed));
                            if resp.drag_started() || resp.gained_focus() {
                                actions.rig_edit_begin = true;
                            }
                            changed |= resp.changed();
                        }
                        if changed {
                            make(actions, out);
                            actions.rig_edit_changed = true;
                        }
                    });
                };
            vec3_row(ui, Msg::Translation, xf.t, 0.1, &mut |a, t| {
                a.set_key_translation = Some((k, bone, t));
            });
            // Free 3-DOF rotation as Euler degrees (X/Y/Z). Read back the
            // stored quaternion, edit, rebuild — gimbal-limited near ±90° pitch.
            let euler_deg = xf.r.to_euler().map(f32::to_degrees);
            vec3_row(ui, Msg::Rotation, euler_deg, 1.0, &mut |a, d| {
                let r = Quat::from_euler(d[0].to_radians(), d[1].to_radians(), d[2].to_radians());
                a.set_key_rotation = Some((k, bone, r));
            });
            vec3_row(ui, Msg::Scale, xf.s, 0.01, &mut |a, s| {
                a.set_key_scale = Some((k, bone, s));
            });
        }
    }
    // Contextual posing hint: what (if anything) blocks a viewport rotate-drag
    // right now — no key selected, an un-poseable bone (root / locked), or the
    // ready state. Mirrors the begin_pose_drag guards so the viewport gesture
    // is discoverable instead of a silent no-op.
    if !rig.clips.is_empty() {
        ui.separator();
        let clip = editor.active_clip.min(rig.clips.len() - 1);
        let has_key = editor
            .selected_key
            .is_some_and(|k| k < rig.clip_keyframes(clip).len());
        let hint = if !has_key {
            Msg::PoseNeedKey
        } else if rig.is_poseable(editor.active_bone) {
            Msg::PoseHint
        } else {
            Msg::PoseUnposeable
        };
        ui.small(t(hint));
        // Gizmo mode toggle — what a viewport drag transforms. Mirrors the
        // R / G hotkeys (shown as tooltips).
        ui.horizontal(|ui| {
            ui.small(t(Msg::GizmoHint));
            for (mode, label, key) in [
                (GizmoMode::Rotate, Msg::Rotation, "R"),
                (GizmoMode::Translate, Msg::Translation, "G"),
                (GizmoMode::Scale, Msg::Scale, "S"),
            ] {
                if ui
                    .selectable_label(editor.gizmo_mode == mode, t(label))
                    .on_hover_text(key)
                    .clicked()
                {
                    actions.set_gizmo_mode = Some(mode);
                }
            }
        });
    }
}

/// The Clip editor panel (left side, when a clip document is open): the frames
/// strip (select / add / duplicate / delete), the default frame duration, and
/// the loop mode. The active frame's mesh is sculpted with the usual voxel
/// tools below this panel. Emits [`UiActions`]; the host applies them.
fn clip_panel(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    actions: &mut UiActions,
    t: &impl Fn(Msg) -> &'static str,
) {
    if editor.clip.is_none() {
        return;
    }
    ui.separator();
    // Onion-skin toggle (mutates the editor) — done before borrowing the clip.
    // Flipping it rebuilds the view (with / without ghosts).
    if ui
        .checkbox(&mut editor.onion_skin, t(Msg::OnionSkin))
        .changed()
    {
        editor.dirty = true;
    }
    let clip = editor.clip.as_ref().expect("clip present (checked above)");
    let active = editor.active_frame.min(clip.frames.len() - 1);
    ui.label(t(Msg::Frames));
    let durations = clip.durations();
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .show(ui, |ui| {
            for (i, ms) in durations.iter().enumerate() {
                // "Frame 3 · 80 ms" — 1-based for the artist.
                let label = format!("{} {} · {ms} ms", t(Msg::Frame), i + 1);
                if ui.selectable_label(i == active, label).clicked() {
                    actions.select_frame = Some(i);
                }
            }
        });
    ui.horizontal(|ui| {
        if ui.button(t(Msg::AddFrame)).clicked() {
            actions.add_frame = true;
        }
        if ui.button(t(Msg::DuplicateFrame)).clicked() {
            actions.duplicate_frame = Some(active);
        }
        ui.add_enabled_ui(clip.frames.len() > 1, |ui| {
            if ui.button(t(Msg::DeleteFrame)).clicked() {
                actions.delete_frame = Some(active);
            }
        });
    });
    // Default per-frame duration (ms): the playback rate when a frame has no
    // override. Clamped to ≥ 1 by the host.
    ui.horizontal(|ui| {
        ui.label(t(Msg::FrameMs));
        let mut ms = clip.default_frame_ms;
        if ui
            .add(egui::DragValue::new(&mut ms).speed(1.0).range(1..=10_000))
            .changed()
        {
            actions.set_clip_default_ms = Some(ms);
        }
    });
    // The active frame's own duration: edit it to override the clip default;
    // "↺" reverts the override.
    ui.horizontal(|ui| {
        ui.label(format!("{} {}", t(Msg::Frame), active + 1));
        let cur = clip.frames[active].duration_ms;
        let mut ms = cur.unwrap_or(clip.default_frame_ms);
        if ui
            .add(
                egui::DragValue::new(&mut ms)
                    .speed(1.0)
                    .range(1..=10_000)
                    .suffix(" ms"),
            )
            .changed()
        {
            actions.set_frame_duration = Some((active, Some(ms)));
        }
        if cur.is_some() && ui.button("↺").clicked() {
            actions.set_frame_duration = Some((active, None));
        }
    });
    // Loop mode: how playback advances past the last frame.
    ui.horizontal(|ui| {
        ui.label(t(Msg::LoopModeLabel));
        for (mode, label) in [
            (LoopMode::Loop, Msg::Loop),
            (LoopMode::Once, Msg::Once),
            (LoopMode::PingPong, Msg::PingPong),
        ] {
            if ui
                .selectable_label(clip.loop_mode == mode, t(label))
                .clicked()
            {
                actions.set_clip_loop_mode = Some(mode);
            }
        }
    });
    // Padding warning: when the declared bbox dwarfs the content, the `.rvc`
    // carries lots of empty columns — offer a crop-to-content.
    if clip.is_padding_wasteful() {
        ui.separator();
        ui.colored_label(
            egui::Color32::from_rgb(0xE0, 0xA0, 0x30),
            t(Msg::ClipPadWarn),
        );
        if ui.button(t(Msg::Crop)).clicked() {
            actions.crop_clip = true;
        }
    }
}

/// The Clip timeline bar (bottom, when a clip is open): transport (play/pause,
/// prev/next frame) + a frame scrubber + a `frame / count` readout. The
/// playhead and the edited frame are unified, so scrubbing also selects the
/// frame for editing. Emits [`UiActions`].
fn clip_timeline_bar(
    ui: &mut egui::Ui,
    editor: &Editor,
    actions: &mut UiActions,
    t: &impl Fn(Msg) -> &'static str,
) {
    let Some(clip) = &editor.clip else {
        return;
    };
    let n = clip.frames.len();
    let active = editor.active_frame.min(n - 1);
    ui.horizontal(|ui| {
        // Play / pause (Space). Reuses `toggle_play`; the host routes it to the
        // clip when a clip is open.
        let label = if editor.anim_playing {
            t(Msg::Pause)
        } else {
            t(Msg::Play)
        };
        if ui.button(label).clicked() {
            actions.toggle_play = true;
        }
        // Prev / next frame (`,` / `.`); scrubbing pauses.
        if ui
            .add_enabled(active > 0, egui::Button::new("◀"))
            .on_hover_text(",")
            .clicked()
        {
            actions.seek_clip = Some(clip.frame_start_ms(active - 1));
        }
        if ui
            .add_enabled(active + 1 < n, egui::Button::new("▶"))
            .on_hover_text(".")
            .clicked()
        {
            actions.seek_clip = Some(clip.frame_start_ms(active + 1));
        }
        // Frame scrubber: a slider over frame indices. Single-frame clips have
        // nothing to scrub, so the slider is omitted.
        if n > 1 {
            let mut idx = active;
            let resp = ui.add(
                egui::Slider::new(&mut idx, 0..=(n - 1))
                    .integer()
                    .show_value(false),
            );
            if resp.changed() {
                actions.seek_clip = Some(clip.frame_start_ms(idx));
            }
        }
        ui.label(format!("{} {} / {n}", t(Msg::Frame), active + 1));
    });
}

/// The Animation timeline bar (Rig ▸ Animate), drawn full-width along the
/// bottom: the active clip name, transport (play/pause, prev/next keyframe),
/// Add/Delete key, an inline angle editor, a custom-painted scrub track with
/// keyframe ticks + a draggable playhead, and a time readout. Read-only with
/// respect to the rig — it emits [`UiActions`]; it never mutates the document.
fn timeline_bar(
    ui: &mut egui::Ui,
    editor: &Editor,
    actions: &mut UiActions,
    timeline: Timeline,
    t: &impl Fn(Msg) -> &'static str,
) {
    let Some(rig) = &editor.rig else {
        return;
    };
    if rig.clips.is_empty() {
        ui.centered_and_justified(|ui| ui.weak(format!("{} —", t(Msg::Clips))));
        return;
    }
    let active = editor.active_clip.min(rig.clips.len() - 1);
    let keys = rig.clip_keyframes(active);
    let key_times: Vec<i32> = keys.iter().map(|k| k.tim).collect();
    let duration = rig.clip_loop_tim(active);
    let has_anim = duration > 0;
    // The selection, validated against the current key count.
    let selected = editor.selected_key.filter(|&k| k < keys.len());

    // Top row: clip picker + transport + key ops + (selected) angle editor +
    // a right-aligned time readout.
    ui.horizontal(|ui| {
        // The active clip name (selection / management is the left clip panel).
        ui.strong(rig.clips[active].name.as_str());
        ui.separator();

        ui.add_enabled_ui(has_anim, |ui| {
            let label = if editor.anim_playing {
                t(Msg::Pause)
            } else {
                t(Msg::Play)
            };
            if ui.button(label).clicked() {
                actions.toggle_play = true;
            }
            // Prev / next keyframe: jump the playhead to the adjacent key.
            if ui.button("|◀").on_hover_text(t(Msg::PrevKey)).clicked() {
                if let Some(&p) = key_times.iter().rev().find(|&&x| x < timeline.time) {
                    actions.seek = Some(p);
                }
            }
            if ui.button("▶|").on_hover_text(t(Msg::NextKey)).clicked() {
                if let Some(&n) = key_times.iter().find(|&&x| x > timeline.time) {
                    actions.seek = Some(n);
                }
            }
        });
        ui.separator();

        // Key ops: add at the playhead; delete the selected key.
        if ui.button(t(Msg::AddKey)).clicked() {
            actions.add_key = true;
        }
        ui.add_enabled_ui(selected.is_some(), |ui| {
            if ui.button(t(Msg::DeleteKey)).clicked() {
                actions.delete_key = true;
            }
        });
        // Copy / cut the selected key's pose; paste it as a key at the playhead
        // (paste enabled once something's on the key clipboard). Cut + paste
        // elsewhere moves a key; copy + paste duplicates a pose.
        ui.add_enabled_ui(selected.is_some(), |ui| {
            if ui.button(t(Msg::Copy)).clicked() {
                actions.copy_key = true;
            }
            if ui.button(t(Msg::Cut)).clicked() {
                actions.cut_key = true;
            }
        });
        ui.add_enabled_ui(editor.key_clipboard.is_some(), |ui| {
            if ui.button(t(Msg::Paste)).clicked() {
                actions.paste_key = true;
            }
        });

        // Angle editor for the active bone of the selected key.
        if let Some(k) = selected {
            angle_editor(ui, editor, &keys, k, actions);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(format!("{} / {duration} ms", timeline.time));
            ui.separator();
            // Per-clip playback: loop on/off + the clip length (the loop-marker
            // time). Length drags coalesce into one undo step via the inline
            // begin/commit-pending pair; the loop toggle is one discrete edit.
            let mut loops = rig.clip_loops(active);
            if ui.checkbox(&mut loops, t(Msg::Loop)).changed() {
                actions.set_clip_loops = Some(loops);
            }
            let mut len = duration;
            let last_key = key_times.iter().copied().max().unwrap_or(0);
            let resp = ui.add(
                egui::DragValue::new(&mut len)
                    .suffix(" ms")
                    .speed(10.0)
                    .range((last_key + 1)..=600_000),
            );
            ui.label(t(Msg::Length));
            if resp.drag_started() || resp.gained_focus() {
                actions.rig_edit_begin = true;
            }
            if resp.changed() {
                actions.set_clip_length = Some(len);
                actions.rig_edit_changed = true;
            }
        });
    });

    // The scrub track fills the remaining bar width.
    ui.add_enabled_ui(has_anim, |ui| {
        timeline_track(ui, timeline.time, duration, &key_times, selected, actions);
    });
}

/// Inline angle editor: a slider over the active bone's `vmin..=vmax` bound to
/// its angle in keyframe `k`, shown in degrees. Disabled for a root bone (its
/// column is ignored by the solver) or a locked bone (`vmin == vmax`). Uses the
/// inline begin/commit-pending undo pair so one drag is one step.
#[allow(clippy::cast_precision_loss)] // angle units -> degrees, tiny values
fn angle_editor(
    ui: &mut egui::Ui,
    editor: &Editor,
    keys: &[demiurg_core::Keyframe],
    k: usize,
    actions: &mut UiActions,
) {
    let bone = editor.active_bone;
    let Some(b) = editor.rig.as_ref().and_then(|r| r.bones.get(bone)) else {
        return;
    };
    let is_root = b.hinge.parent < 0;
    let (lo, hi) = (
        b.hinge.vmin.min(b.hinge.vmax),
        b.hinge.vmin.max(b.hinge.vmax),
    );
    // The slider edits the 1-DOF hinge angle; read it out of the key's stored
    // transform (about this bone's axis).
    let v = b.hinge.v[0];
    let mut val = keys[k]
        .xforms
        .get(bone)
        .map_or(0, |x| x.hinge_angle([v.x, v.y, v.z]));
    ui.separator();
    ui.add_enabled_ui(!is_root && lo < hi, |ui| {
        ui.label(format!("{}:", b.name));
        let resp = ui.add(
            egui::Slider::new(&mut val, lo..=hi)
                // i16 hinge units -> degrees (full circle = 65536).
                .custom_formatter(|n, _| format!("{:.0}°", n * 360.0 / 65536.0)),
        );
        if resp.drag_started() || resp.gained_focus() {
            actions.rig_edit_begin = true;
        }
        if resp.changed() {
            actions.set_key_angle = Some((k, bone, val));
            actions.rig_edit_changed = true;
        }
    });
}

/// Draw and handle the custom scrub track: a baseline, a tick per keyframe
/// (the selected one highlighted), and a draggable playhead. Clicking a tick
/// selects it; clicking elsewhere seeks; dragging a tick retimes it (bounded by
/// its neighbours so it can't reorder); dragging elsewhere scrubs.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)] // ms/px are tiny
fn timeline_track(
    ui: &mut egui::Ui,
    time: i32,
    duration: i32,
    key_times: &[i32],
    selected: Option<usize>,
    actions: &mut UiActions,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 22.0),
        egui::Sense::click_and_drag(),
    );
    if duration <= 0 {
        return;
    }

    // Colours copied out so the immutable visuals borrow ends before the
    // interaction code touches `ui` mutably (egui memory).
    let (col_base, col_tick, col_sel, col_head) = {
        let v = ui.visuals();
        (
            v.weak_text_color(),
            v.strong_text_color(),
            egui::Color32::from_rgb(0xff, 0x9f, 0x1c), // amber: the selected key
            v.selection.bg_fill,                       // the playhead
        )
    };

    // Inset so ticks at t=0 / t=duration aren't clipped at the edges.
    let margin = 6.0;
    let x0 = rect.left() + margin;
    let span = (rect.width() - 2.0 * margin).max(1.0);
    let dur = duration as f32;
    let x_of = |ms: i32| x0 + (ms as f32 / dur) * span;
    let time_at = |x: f32| (((x - x0) / span).clamp(0.0, 1.0) * dur).round() as i32;
    let tick_at = |x: f32| key_times.iter().position(|&ms| (x_of(ms) - x).abs() <= 5.0);

    let painter = ui.painter_at(rect);
    let mid_y = rect.center().y;
    painter.line_segment(
        [egui::pos2(x0, mid_y), egui::pos2(x0 + span, mid_y)],
        egui::Stroke::new(1.0, col_base),
    );
    for (i, &ms) in key_times.iter().enumerate() {
        let x = x_of(ms);
        let sel = selected == Some(i);
        let color = if sel { col_sel } else { col_tick };
        painter.line_segment(
            [
                egui::pos2(x, rect.top() + 3.0),
                egui::pos2(x, rect.bottom() - 3.0),
            ],
            egui::Stroke::new(if sel { 2.5 } else { 1.5 }, color),
        );
        if sel {
            painter.circle_filled(egui::pos2(x, rect.bottom() - 3.0), 3.0, color);
        }
    }
    let px = x_of(time);
    painter.line_segment(
        [egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())],
        egui::Stroke::new(2.0, col_head),
    );
    painter.circle_filled(egui::pos2(px, rect.top() + 3.0), 3.5, col_head);

    // Interaction. A drag's mode (retime key `k` vs scrub) is decided at press
    // time from what's under the cursor and kept in egui temp memory for the
    // drag's duration.
    let id = response.id;
    if response.drag_started() {
        let grabbed = response.interact_pointer_pos().and_then(|p| tick_at(p.x));
        ui.data_mut(|d| d.insert_temp(id, grabbed));
        if let Some(k) = grabbed {
            actions.select_key = Some(k);
            actions.rig_edit_begin = true; // open one undo step for the retime
        }
    }
    if response.dragged() {
        if let Some(p) = response.interact_pointer_pos() {
            let grabbed: Option<usize> = ui.data(|d| d.get_temp(id)).flatten();
            if let Some(k) = grabbed {
                // Bound the retime to between the neighbours so the key keeps
                // its index (no reorder mid-drag).
                let lo = if k > 0 { key_times[k - 1] + 1 } else { 0 };
                let hi = key_times.get(k + 1).map_or(i32::MAX, |&n| n - 1);
                actions.move_key = Some((k, time_at(p.x).clamp(lo, hi)));
                actions.rig_edit_changed = true;
            } else {
                actions.seek = Some(time_at(p.x));
            }
        }
    }
    if response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            match tick_at(p.x) {
                Some(k) => actions.select_key = Some(k),
                None => actions.seek = Some(time_at(p.x)),
            }
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
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)] // bone counts + mesh dims are tiny
#[allow(clippy::too_many_lines)] // a flat builder of the skeleton sections
#[allow(clippy::many_single_char_names)] // x/y/z + b/c begin-changed pairs
fn skeleton_panel(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    actions: &mut UiActions,
    t: &impl Fn(Msg) -> &'static str,
) {
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
    // `changed`: an inline value was mutated this frame (commit the pending
    // undo snapshot). `begin`: an interaction started this frame (capture the
    // pre-edit snapshot). Together they make one undo step per interaction.
    let mut changed = false;
    let mut begin = false;

    ui.separator();
    ui.label(t(Msg::Skeleton));
    let r = ui.text_edit_singleline(&mut bone.name);
    begin |= r.gained_focus();
    changed |= r.changed();

    ui.horizontal(|ui| {
        ui.label(t(Msg::Parent));
        let mut parent = bone.hinge.parent;
        let r = ui.add(egui::DragValue::new(&mut parent).range(-1..=(n - 1)));
        begin |= r.drag_started() || r.gained_focus();
        if r.changed()
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
            let r = ui.add(egui::DragValue::new(f).speed(0.5));
            begin |= r.drag_started() || r.gained_focus();
            changed |= r.changed();
        }
    });

    // Rotation axis — pick a principal axis (always a unit vector, so the
    // pose never distorts; a free numeric axis is too easy to leave
    // non-normalised). Deferred to the host so it can be one undo step.
    ui.label(t(Msg::Axis));
    let cur = [bone.hinge.v[0].x, bone.hinge.v[0].y, bone.hinge.v[0].z];
    ui.horizontal(|ui| {
        for (axis, name) in [(0usize, "X"), (1, "Y"), (2, "Z")] {
            let mut unit = [0.0f32; 3];
            unit[axis] = 1.0;
            #[allow(clippy::float_cmp)] // unit axes are exact 0.0 / 1.0
            let is_active = cur == unit;
            if ui
                .selectable_label(is_active, egui::RichText::new(name).color(axis_color(axis)))
                .clicked()
            {
                actions.set_bone_axis = Some(axis);
            }
        }
    });

    // Mesh pivot (the kv6 pivot): the point of this bone's model that sits on
    // the joint and which it rotates about. Editable here — with the skeleton
    // visible — so a mesh can be aligned to its joint on a complex rig; "Move
    // pivot" drags it in the viewport (mesh follows the cursor, joint fixed).
    ui.label(t(Msg::Pivot));
    ui.horizontal(|ui| {
        for (axis, name) in [(0usize, "x"), (1, "y"), (2, "z")] {
            ui.colored_label(axis_color(axis), name);
            let r = ui.add(egui::DragValue::new(&mut bone.model.pivot[axis]).speed(0.5));
            begin |= r.drag_started() || r.gained_focus();
            changed |= r.changed();
        }
        if ui.button(t(Msg::CenterPivot)).clicked() {
            let (dx, dy, dz) = bone.model.dims();
            bone.model.pivot = [dx as f32 * 0.5, dy as f32 * 0.5, dz as f32 * 0.5];
            begin = true;
            changed = true;
        }
    });
    // The viewport drag toggle lives on the editor (not the bone), so set it
    // after the bone borrow above is done with.
    ui.checkbox(&mut editor.pivot_move_mode, t(Msg::MovePivot))
        .on_hover_text(t(Msg::MovePivot));

    // Extra attachments: pick which one the "Move attachment" drag targets and
    // tune its offset against the posed view. Only when the bone has extras.
    let attach_count = editor
        .rig
        .as_ref()
        .and_then(|r| r.bones.get(active))
        .map_or(1, demiurg_core::RigBone::attachment_count);
    if attach_count > 1 {
        ui.separator();
        ui.label(t(Msg::Attachments));
        let cur = editor.active_attachment;
        ui.horizontal_wrapped(|ui| {
            for i in 0..attach_count {
                let label = layer_label(editor.rig.as_ref(), active, i, t);
                if ui.selectable_label(i == cur, label).clicked() {
                    actions.select_attachment = Some(i);
                }
            }
        });
        ui.checkbox(&mut editor.attach_move_mode, t(Msg::MoveAttachment))
            .on_hover_text(t(Msg::MoveAttachment));
        if cur > 0 {
            if let Some(att) = editor
                .rig
                .as_mut()
                .and_then(|r| r.bones.get_mut(active))
                .and_then(|b| b.extras.get_mut(cur - 1))
            {
                let (b, c) = attachment_offset_fields(ui, att, t);
                begin |= b;
                changed |= c;
            }
        }
    }

    if begin {
        actions.rig_edit_begin = true;
    }
    if changed {
        actions.rig_edit_changed = true;
        editor.rig_dirty = true;
    }
}

/// One labelled row of three axis-coloured `DragValue`s editing `v`. Returns
/// `(begin, changed)`: `begin` = an interaction started this frame (capture the
/// pre-edit undo snapshot), `changed` = a value actually moved.
fn vec3_drag_row(ui: &mut egui::Ui, label: &str, v: &mut [f32; 3], speed: f64) -> (bool, bool) {
    let (mut begin, mut changed) = (false, false);
    ui.horizontal(|ui| {
        ui.label(label);
        for (axis, c) in v.iter_mut().enumerate() {
            ui.colored_label(axis_color(axis), ["x", "y", "z"][axis]);
            let r = ui.add(egui::DragValue::new(c).speed(speed));
            begin |= r.drag_started() || r.gained_focus();
            changed |= r.changed();
        }
    });
    (begin, changed)
}

/// Display label for attachment `i` of bone `bone`: "Base layer" for the
/// primary (`0`), else the extra's editable name (a generic "Layer N" only as a
/// fallback for a nameless extra).
fn layer_label(
    rig: Option<&demiurg_core::Rig>,
    bone: usize,
    i: usize,
    t: &impl Fn(Msg) -> &'static str,
) -> String {
    let base = if i == 0 {
        t(Msg::PrimaryMesh).to_string()
    } else {
        rig.and_then(|r| r.bones.get(bone))
            .and_then(|b| b.extras.get(i - 1))
            .map_or_else(|| format!("{} {i}", t(Msg::Attachment)), |e| e.name.clone())
    };
    // Mark a clip layer with its frame count (e.g. "flame · 8f") — both the
    // type indicator and useful at-a-glance info.
    match rig
        .and_then(|r| r.bones.get(bone))
        .and_then(|b| b.attachment_clip(i))
    {
        Some(c) => format!("{base} · {}f", c.frame_count()),
        None => base,
    }
}

/// The Attachments section (Rig ▸ Sculpt): pick which of the active bone's
/// meshes to sculpt — the primary mesh or an extra attachment — add / remove
/// extras, rename the active extra, and set its local offset (translate /
/// rotate / scale) in the bone's frame. Selection / add / remove are deferred to
/// the host (they swap the working mesh); the name + offset mutate the rig
/// directly, one undo step per interaction (the inline begin/commit-pending pair).
fn attachments_panel(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    actions: &mut UiActions,
    t: &impl Fn(Msg) -> &'static str,
) {
    let active_bone = editor.active_bone;
    let active = editor.active_attachment;
    let Some(count) = editor
        .rig
        .as_ref()
        .and_then(|r| r.bones.get(active_bone))
        .map(demiurg_core::RigBone::attachment_count)
    else {
        return;
    };

    ui.separator();
    ui.label(t(Msg::Attachments));
    for i in 0..count {
        let label = layer_label(editor.rig.as_ref(), active_bone, i, t);
        if ui.selectable_label(i == active, label).clicked() {
            actions.select_attachment = Some(i);
        }
    }
    // Whether the active attachment already draws a clip (then "To clip" is off).
    let active_is_clip = editor
        .rig
        .as_ref()
        .and_then(|r| r.bones.get(active_bone))
        .is_some_and(|b| b.attachment_is_clip(active));
    ui.horizontal(|ui| {
        if ui.button(t(Msg::AddAttachment)).clicked() {
            actions.add_attachment = true;
        }
        if ui
            .add_enabled(active > 0, egui::Button::new(t(Msg::Remove)))
            .clicked()
        {
            actions.remove_attachment = true;
        }
    });
    ui.horizontal(|ui| {
        if ui.button(t(Msg::AddClipLayer)).clicked() {
            actions.add_clip_layer = true;
        }
        // Convert the active mesh attachment into an animated clip (seeded from
        // its current mesh). Disabled if it's already a clip.
        if ui
            .add_enabled(!active_is_clip, egui::Button::new(t(Msg::MakeClipLayer)))
            .clicked()
        {
            actions.make_clip_layer = true;
        }
        if ui.button(t(Msg::ImportClipLayer)).clicked() {
            actions.import_clip_layer = true;
        }
    });

    // The active extra's name + local offset (the primary is fixed at identity).
    let mut rebuild = false;
    if active > 0 {
        if let Some(att) = editor
            .rig
            .as_mut()
            .and_then(|r| r.bones.get_mut(active_bone))
            .and_then(|b| b.extras.get_mut(active - 1))
        {
            // Rename the layer (undoable, but no preview rebuild needed).
            let rn = ui.text_edit_singleline(&mut att.name);
            if rn.gained_focus() {
                actions.rig_edit_begin = true;
            }
            if rn.changed() {
                actions.rig_edit_changed = true;
            }
            let (begin, changed) = attachment_offset_fields(ui, att, t);
            if begin {
                actions.rig_edit_begin = true;
            }
            if changed {
                actions.rig_edit_changed = true;
                rebuild = true;
            }
        }
    }
    // Clip attachments (primary or extra) carry playback params (speed + phase).
    if active_is_clip {
        if let Some(pb) = editor
            .rig
            .as_mut()
            .and_then(|r| r.bones.get_mut(active_bone))
            .and_then(|b| b.attachment_playback_mut(active))
        {
            let (begin, changed) = clip_playback_fields(ui, pb, t);
            if begin {
                actions.rig_edit_begin = true;
            }
            if changed {
                actions.rig_edit_changed = true;
            }
        }
    }
    if rebuild {
        editor.rig_dirty = true;
    }
}

/// Speed (× the rig clip's rate) + start-phase editor for a clip attachment's
/// playback. Returns `(begin, changed)` (one undo step per drag).
fn clip_playback_fields(
    ui: &mut egui::Ui,
    pb: &mut demiurg_core::LayerPlayback,
    t: &impl Fn(Msg) -> &'static str,
) -> (bool, bool) {
    let (mut begin, mut changed) = (false, false);
    #[allow(clippy::cast_precision_loss)] // speed_q8 is small
    let mut speed = pb.speed_q8 as f32 / 256.0;
    ui.horizontal(|ui| {
        ui.label(t(Msg::Speed));
        let r = ui.add(
            egui::DragValue::new(&mut speed)
                .speed(0.01)
                .range(0.0..=16.0),
        );
        begin |= r.drag_started() || r.gained_focus();
        if r.changed() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                pb.speed_q8 = (speed * 256.0).round() as i32;
            }
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label(t(Msg::Phase));
        let r = ui.add(
            egui::DragValue::new(&mut pb.start_phase_ms)
                .speed(1.0)
                .range(0..=600_000),
        );
        begin |= r.drag_started() || r.gained_focus();
        changed |= r.changed();
    });
    (begin, changed)
}

/// Numeric editor for an attachment's local offset — translate / rotate
/// (Euler degrees) / scale rows. Returns `(begin, changed)` (one undo step per
/// drag). Shared by the Sculpt attachments panel and the Skeleton "Move
/// attachment" controls.
fn attachment_offset_fields(
    ui: &mut egui::Ui,
    att: &mut demiurg_core::RigAttachment,
    t: &impl Fn(Msg) -> &'static str,
) -> (bool, bool) {
    let (b0, c0) = vec3_drag_row(ui, t(Msg::Translation), &mut att.offset.t, 0.5);
    // Rotation as Euler degrees (gimbal-limited near ±90° pitch).
    let mut euler = att.offset.r.to_euler().map(f32::to_degrees);
    let (b1, c1) = vec3_drag_row(ui, t(Msg::Rotation), &mut euler, 1.0);
    if c1 {
        att.offset.r = Quat::from_euler(
            euler[0].to_radians(),
            euler[1].to_radians(),
            euler[2].to_radians(),
        );
    }
    let (b2, c2) = vec3_drag_row(ui, t(Msg::Scale), &mut att.offset.s, 0.01);
    (b0 || b1 || b2, c0 || c1 || c2)
}

/// The voxel-editing tools — shown in plain Model mode and in Rig ▸ Sculpt
/// (editing the active bone's mesh): tool picker, paint colour, symmetry,
/// pivot, model size, selection, and the reference-image guide.
#[allow(clippy::cast_precision_loss)] // pivot is centred from small voxel dims
#[allow(clippy::too_many_lines)] // a flat builder of the model-editing sections
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

    // Rig: which of the active bone's meshes the tools sculpt (primary or an
    // extra attachment), plus add/remove + the extra's offset.
    if editor.rig.is_some() {
        attachments_panel(ui, editor, actions, t);
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

    // Rotate the selection 90 degrees about a chosen axis. Acts on the current
    // selection (or a floating layer), so it needs one to do anything.
    ui.separator();
    ui.label(t(Msg::Rotate));
    let can_rotate = !editor.selection.is_empty() || editor.float.is_some();
    ui.horizontal(|ui| {
        for (axis, name) in [(0, "X"), (1, "Y"), (2, "Z")] {
            ui.selectable_value(
                &mut editor.rotate_axis,
                axis,
                egui::RichText::new(name).color(axis_color(axis)),
            );
        }
        if ui
            .add_enabled(can_rotate, egui::Button::new("⟲"))
            .on_hover_text(t(Msg::RotateCcw))
            .clicked()
        {
            actions.rotate_sel = Some(false);
        }
        if ui
            .add_enabled(can_rotate, egui::Button::new("⟳"))
            .on_hover_text(t(Msg::RotateCw))
            .clicked()
        {
            actions.rotate_sel = Some(true);
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
        // A browser can't drop an image onto the window, so offer a paste:
        // "Copy image" in the browser, then this (or Ctrl+V).
        if ui.button(t(Msg::PasteReference)).clicked() {
            actions.paste_reference = true;
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
        // Scale: world voxels per texel. Fit a guide to the model without
        // touching the stored pixels (the sprite reprojects every frame, so no
        // texture rebuild). Drag is multiplicative-ish via a small speed; the
        // field also accepts a typed value.
        ui.horizontal(|ui| {
            ui.label(t(Msg::Scale));
            ui.add(
                egui::DragValue::new(&mut r.scale)
                    .range(0.01..=64.0)
                    .speed(0.02)
                    .max_decimals(3),
            );
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
