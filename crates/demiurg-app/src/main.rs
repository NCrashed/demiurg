//! demiurg native editor (M2): open a window, load/edit a `.kv6` voxel
//! model, and see it rendered by roxlap's own renderer. An egui overlay
//! provides the tools, palette, mirror, pivot, and file menu.
//!
//! Usage:
//!   demiurg [--gpu|--cpu] [path.kv6|.vox|.demiurg|.rkc]   # no path -> blank
//!
//! Controls: left mouse applies the active tool (hold to drag-paint); the
//! Place tool falls back to the floor (the volume's bottom face) when the
//! ray hits no voxel, so you can build up from an empty model. With the
//! Select tool, dragging a selected voxel moves the selection in
//! that face's plane (it floats until deselected); `Ctrl`+click (or the
//! Eyedropper tool) picks a colour from the voxel or reference image under
//! the cursor, whichever is nearer; right-mouse drag orbits; middle-mouse (or Shift+right) drag
//! pans the view, `Home` recenters it; wheel and `W`/`S` zoom; arrow keys
//! orbit; the Views panel (or numpad `1`/`3`/`7`, `Ctrl` for the opposite
//! face) snaps to an axis-aligned view. Hotkeys: `1`-`8` pick a tool (`8` is
//! Select), `Ctrl+Z` undo, `Ctrl+Y` / `Ctrl+Shift+Z` redo, `Ctrl+C`
//! copies the selection and `Ctrl+V` pastes it as a floating layer at its
//! original position (settled into the model on deselect), `Delete`
//! removes the selection, `Esc` deselects (settling any pasted layer) or
//! else quits. `Ctrl+S` saves the project (overwriting its path once
//! known); saves run on a background thread (with a spinner) so the UI
//! never freezes, and a periodic autosave to the OS temp dir is recovered
//! on the next launch after a crash. Dragging an image onto the window
//! loads it as a non-destructive reference guide (a model file opens as the
//! model). `DEMIURG_LANG=ru` starts in Russian.
//! The CPU renderer is the default (reliable everywhere); `--gpu` (or
//! `ROXLAP_GPU=1`) opts into the faster GPU backend, whose device creation
//! can hang on some Windows GPUs/drivers.

mod reference;
mod ui;

use reference::Reference;

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use demiurg_core::{Document, Rig, VoxelModel, project};
use demiurg_i18n::{Lang, Msg, tr};
use demiurg_view::{
    KfaView, Line3, ModelView, OrbitCamera, PickHit, RenderMode, ViewDir, pick_voxel,
};
use roxlap_core::opticast::OpticastSettings;
use roxlap_core::sprite::SpriteLighting;
use roxlap_render::{
    FrameParams, ImageFacing, ImageId, ImageSprite, RenderOptions, SceneRenderer, egui,
};
use ui::UiActions;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

/// Packed `0x00RRGGBB` sky/clear colour — a calm muted slate-blue: light
/// enough to read as sky, not the glaring cyan it started as.
const SKY_COLOR: u32 = 0x005a_6b7a;
/// Sprite material colour (`R==G==B` so the cheap shading path applies);
/// a darker grey keeps the model from rendering blown-out bright.
const SPRITE_MATERIAL: u32 = 0x0080_8080;
/// Default canvas size for a new model.
const NEW_DIMS: u32 = 32;
/// The render mode the editor opens in — the voxel grid, whose per-face
/// `side_shades` make voxels easy to read while editing.
const DEFAULT_RENDER_MODE: RenderMode = RenderMode::Voxel;
/// voxlap `setsideshades(top, bot, left, right, up, down)` for the voxel
/// render: leave the top bright and darken the others so top faces pop.
const VOXEL_SIDE_SHADES: [i8; 6] = [0, 28, 16, 16, 16, 28];
/// Redraw cadence — ~60 fps, so the editor doesn't peg the GPU/CPU
/// rendering an idle scene as fast as it can. Pairs with GPU vsync.
const FRAME_DT: Duration = Duration::from_micros(16_667);
/// Voxel-edge wireframe colour (`0xAARRGGBB`): a faint, semi-transparent
/// **light** grey. Light (not dark) so edges lift out of dark shadowed
/// faces — where boundaries otherwise vanish — while staying subtle on
/// already-readable lit faces.
const VOXEL_EDGE_COLOR: u32 = 0x66d4_d8e0;
/// Rotation-axis gizmo (Animate posing): an opaque orange line through the
/// pivot, and a translucent orange ring in the rotation plane.
const GIZMO_AXIS_COLOR: u32 = 0xffff_8c1a;
const GIZMO_RING_COLOR: u32 = 0x99ff_8c1a;
/// How often a background autosave is written while there are unsaved
/// changes — a crash-recovery snapshot, not "the save".
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(20);

/// The on-disk format a save writes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SaveFormat {
    Project,
    Kv6,
    Vxl,
    Vox,
    /// A rigged character (`.rkc`). Encoded from the rig, not a model —
    /// handled directly in [`App::start_save`].
    Rkc,
}

impl SaveFormat {
    /// Serialize `model` to this format's bytes.
    fn encode(self, model: &VoxelModel) -> Vec<u8> {
        match self {
            SaveFormat::Project => project::to_bytes(model),
            SaveFormat::Kv6 => model.to_kv6_bytes(),
            SaveFormat::Vxl => model.to_vxl_bytes(),
            SaveFormat::Vox => model.to_vox_bytes(),
            SaveFormat::Rkc => unreachable!("rkc is encoded from the rig in start_save"),
        }
    }

    /// `(filter label, extension, default file name)` for a save dialog.
    fn dialog_spec(self) -> (&'static str, &'static str, &'static str) {
        match self {
            SaveFormat::Project => ("demiurg", "demiurg", "model.demiurg"),
            SaveFormat::Kv6 => ("kv6", "kv6", "model.kv6"),
            SaveFormat::Vxl => ("vxl", "vxl", "model.vxl"),
            SaveFormat::Vox => ("vox", "vox", "model.vox"),
            SaveFormat::Rkc => ("character", "rkc", "character.rkc"),
        }
    }
}

/// A save running on a worker thread, so a slow serialize/write never
/// freezes the UI — the OS would otherwise flag the window as hung and
/// offer to kill it, taking the unsaved model with it. The result arrives
/// over `rx`; the model is snapshotted (cloned) into the worker so the
/// main loop keeps rendering (and the save spinner animates) meanwhile.
struct PendingSave {
    rx: Receiver<std::io::Result<()>>,
    path: PathBuf,
    format: SaveFormat,
    /// User-initiated (modal + mark saved on success) vs a silent autosave.
    user: bool,
}

/// What a pending file dialog will do with the path it returns.
#[derive(Clone, Copy)]
enum DialogKind {
    OpenKv6,
    OpenVox,
    OpenProject,
    OpenReference,
    OpenCharacter,
    Save(SaveFormat),
}

/// A native file dialog running on a worker thread. The dialog (an XDG
/// portal / OS dialog) blocks its thread, not the event loop, so the
/// window keeps pumping events and the OS doesn't flag it as hung. The
/// chosen path (or `None` if cancelled) arrives over `rx`.
struct PendingDialog {
    rx: Receiver<Option<PathBuf>>,
    kind: DialogKind,
}

/// Run a native file dialog (blocking) and return the chosen path. Called
/// on a worker thread off the main loop (except macOS, whose `AppKit`
/// panels must run on the main thread).
fn run_dialog(kind: DialogKind, dir: Option<PathBuf>, name: Option<&str>) -> Option<PathBuf> {
    match kind {
        DialogKind::OpenKv6 => rfd::FileDialog::new()
            .add_filter("kv6", &["kv6"])
            .pick_file(),
        DialogKind::OpenVox => rfd::FileDialog::new()
            .add_filter("vox", &["vox"])
            .pick_file(),
        DialogKind::OpenProject => rfd::FileDialog::new()
            .add_filter("demiurg", &["demiurg"])
            .pick_file(),
        DialogKind::OpenReference => rfd::FileDialog::new()
            .add_filter(
                "image",
                &["png", "jpg", "jpeg", "bmp", "gif", "tga", "webp"],
            )
            .pick_file(),
        DialogKind::OpenCharacter => rfd::FileDialog::new()
            .add_filter("character", &["rkc"])
            .pick_file(),
        DialogKind::Save(format) => {
            let (filter, ext, default) = format.dialog_spec();
            let mut dialog = rfd::FileDialog::new()
                .add_filter(filter, &[ext])
                .set_file_name(name.unwrap_or(default));
            if let Some(dir) = dir {
                dialog = dialog.set_directory(dir);
            }
            dialog.save_file()
        }
    }
}

/// Path of the background autosave (a crash-recovery snapshot in the OS
/// temp dir, loaded on the next start if it survived a crash).
fn autosave_path() -> PathBuf {
    std::env::temp_dir().join("demiurg-autosave.demiurg")
}

/// Load the autosave file as a model, or `None` if it's missing/corrupt.
fn recover_autosave(path: &Path) -> Option<VoxelModel> {
    project::from_bytes(&std::fs::read(path).ok()?).ok()
}

/// The active editing tool.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Place,
    Erase,
    Paint,
    Eyedropper,
    Box,
    Sphere,
    Fill,
    /// Pick voxels (click / marquee) into a selection for delete, copy,
    /// paste. Doesn't edit geometry itself.
    Select,
}

/// Which aspect of a rigged character is being edited (the Rig panel's
/// sub-mode). Meaningless when no rig is loaded.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RigMode {
    /// Edit the active bone's voxel mesh with the usual tools.
    Sculpt,
    /// Edit the skeleton: each bone's hinge (parent, joint, axis). The rig
    /// renders at its rest pose.
    Skeleton,
    /// Preview the posed rig playing its clip (read-only).
    Animate,
}

/// How a selection gesture combines with the existing selection: replace
/// it, add to it (Shift), or remove from it (Alt).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SelMode {
    Replace,
    Add,
    Remove,
}

/// An in-progress marquee drag (Select tool): the screen-pixel anchor and
/// how the dragged region combines with the current selection.
struct Marquee {
    start: (f64, f64),
    mode: SelMode,
}

/// A floating layer: voxels lifted above the model, rendered on top but
/// **not** part of the document until committed — so they can sit on, or
/// be dragged over, other voxels without overwriting them. Created by
/// paste (copies, `lifted_from` empty) or by grabbing a selection to move
/// it (`lifted_from` = the source cells, cleared on commit). `cells` are
/// absolute voxel coordinates (`i32`, so a drag can carry them out of
/// bounds and back) plus colour. Committing clears `lifted_from` and
/// writes the in-bounds `cells`, both in one undo step.
struct FloatLayer {
    cells: Vec<([i32; 3], u32)>,
    lifted_from: Vec<[u32; 3]>,
}

/// An in-progress move drag (Select tool): grab a selected voxel's face
/// and slide the floating layer in that face's plane, in whole voxels.
struct DragMove {
    /// Voxel axis of the grabbed face normal (0/1/2); motion is locked out
    /// of this axis, so the layer slides in the perpendicular plane.
    axis: usize,
    /// World coordinate of the drag plane along `axis` (the grabbed face).
    plane_coord: f64,
    /// World point where the grab ray met the plane (the slide origin).
    anchor: [f64; 3],
    /// Float cells when the drag began; the live layer is this offset by
    /// the current integer delta (so the move never drifts).
    base: Vec<([i32; 3], u32)>,
    /// Whether the selection has been lifted into a float yet — deferred
    /// to the first real movement, so a plain click doesn't lift.
    lifted: bool,
    /// Last applied voxel delta, to skip redundant rebuilds.
    last_delta: [i32; 3],
}

/// An in-progress drag of the reference layer (Move mode): slides it in its
/// own plane, offsetting from where it sat when the drag began.
struct RefDrag {
    /// The reference plane's normal axis (0/1/2).
    axis: usize,
    /// World coordinate of the reference plane along `axis`.
    plane_coord: f64,
    /// World point where the grab ray met the plane.
    anchor: [f64; 3],
    /// In-plane offsets when the drag began.
    base_u: i32,
    base_v: i32,
}

/// An in-progress drag of a rig bone (Skeleton mode): moves the bone in a
/// screen-parallel plane through its pivot. The world delta is applied to
/// the bone's parent-side velcro `p[1]` (a child) or `rig.root` (the root).
struct BoneDrag {
    /// Bone being moved (index into `rig.bones`).
    bone: usize,
    /// Drag plane through the bone pivot: a point on it + its (unit) normal,
    /// the camera forward at grab time.
    plane_point: [f64; 3],
    plane_normal: [f64; 3],
    /// World point where the grab ray met the plane.
    anchor: [f64; 3],
    /// The value being edited at grab time — `p[1]` for a child, `rig.root`
    /// for the root bone.
    base: [f32; 3],
    /// The parent bone's world basis `[s, h, f]`, to map a world delta into
    /// the velcro's parent-local space. `None` for the root (world space).
    parent_basis: Option<[[f64; 3]; 3]>,
}

/// An in-progress viewport pose (Animate mode): rotates the active bone about
/// its (fixed) hinge axis to follow the cursor, writing the resulting angle
/// into the selected keyframe (`frmval[key][bone]`). The pivot, axis and base
/// angle are frozen at grab time — recomputing them from the moving pose would
/// make the drag wander.
struct PoseDrag {
    /// Bone being rotated (index into `rig.bones`).
    bone: usize,
    /// Keyframe receiving the angle, and its clip.
    key: usize,
    clip: usize,
    /// World pivot (the bone's solved joint position) — the rotation centre.
    pivot: [f64; 3],
    /// World rotation axis (the hinge axis in the parent's frame), unit length.
    axis: [f64; 3],
    /// Reference vector `anchor - pivot` (in the drag plane) at grab time, the
    /// zero of the swept angle.
    ref0: [f64; 3],
    /// The key's angle at grab time; the swept angle is added to this.
    base: i16,
}

/// The in-bounds cells of a float layer as a selection set (for the
/// highlight while it floats).
fn float_selection(cells: &[([i32; 3], u32)], dims: (u32, u32, u32)) -> HashSet<[u32; 3]> {
    cells
        .iter()
        .filter_map(|(p, _)| in_bounds(*p, dims))
        .collect()
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Intersect ray `(o, d)` with the plane through `point` with normal `n`.
/// `None` if parallel or behind the camera.
fn ray_plane(o: [f64; 3], d: [f64; 3], point: [f64; 3], n: [f64; 3]) -> Option<[f64; 3]> {
    let denom = dot3(d, n);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = dot3([point[0] - o[0], point[1] - o[1], point[2] - o[2]], n) / denom;
    (t > 0.0).then(|| [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t])
}

/// Two orthonormal vectors spanning the plane perpendicular to unit `n`
/// (for drawing a ring / spokes in that plane). `n` is assumed normalized.
fn plane_basis(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    // Cross `n` with whichever world axis is least parallel to it, so `u` is
    // well-conditioned; `w = n × u` completes the right-handed frame.
    let seed = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let mut u = cross3(n, seed);
    let len = dot3(u, u).sqrt().max(1e-9);
    u = [u[0] / len, u[1] / len, u[2] / len];
    (u, cross3(n, u))
}

fn point_dist_2d(p: [f64; 2], a: [f64; 2]) -> f64 {
    ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt()
}

/// Distance from point `p` to the segment `[a, b]` in 2D (screen pixels).
fn point_seg_dist_2d(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    if len2 < 1e-9 {
        return point_dist_2d(p, a); // degenerate (zero-length) segment
    }
    let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0);
    point_dist_2d(p, [a[0] + ab[0] * t, a[1] + ab[1] * t])
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Signed rotation from `ref0` to `r` about unit `axis`, in KFA hinge units
/// (full circle = 65536). Both vectors are expected in the plane normal to
/// `axis`. `None` when either is ~zero (cursor on the pivot — the angle is
/// noise). The sign is right-handed about `axis`.
fn hinge_sweep(axis: [f64; 3], ref0: [f64; 3], r: [f64; 3]) -> Option<f64> {
    if dot3(r, r) < 1e-6 || dot3(ref0, ref0) < 1e-6 {
        return None;
    }
    let theta = dot3(axis, cross3(ref0, r)).atan2(dot3(ref0, r));
    Some(theta * 65536.0 / std::f64::consts::TAU)
}

/// Snap a cursor ray to a whole-voxel offset within a drag plane: meet the
/// ray `(o, d)` with the plane perpendicular to `axis` at `plane_coord`,
/// then round the in-plane displacement from `anchor` to whole voxels
/// (the `axis` component stays 0). `None` if the ray is parallel to the
/// plane or meets it behind the camera.
#[allow(clippy::cast_possible_truncation)] // the delta is a small voxel count
fn plane_drag_delta(
    o: [f64; 3],
    d: [f64; 3],
    axis: usize,
    plane_coord: f64,
    anchor: [f64; 3],
) -> Option<[i32; 3]> {
    if d[axis].abs() < 1e-9 {
        return None;
    }
    let t = (plane_coord - o[axis]) / d[axis];
    if t <= 0.0 {
        return None;
    }
    let mut delta = [0i32; 3];
    for j in 0..3 {
        if j != axis {
            delta[j] = (o[j] + d[j] * t - anchor[j]).round() as i32;
        }
    }
    Some(delta)
}

/// The bottom-layer cell a cursor ray meets on the model's floor plane
/// (voxel `z = dz`, the volume's bottom face in voxlap's z-down world), or
/// `None` if the ray is parallel to the floor, meets it behind the camera,
/// or lands outside the `dx`×`dy` footprint. Used to seed Place-tool
/// voxels when nothing solid is under the cursor.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // cell guarded into [0, dim)
fn floor_cell(
    o: [f64; 3],
    d: [f64; 3],
    pivot: [f32; 3],
    dims: (u32, u32, u32),
) -> Option<[u32; 3]> {
    let (dx, dy, dz) = dims;
    if dz == 0 || d[2].abs() < 1e-9 {
        return None;
    }
    let plane = f64::from(dz) - f64::from(pivot[2]); // world z of voxel-z = dz
    let t = (plane - o[2]) / d[2];
    if t <= 0.0 {
        return None;
    }
    // World hit -> voxel space (add the pivot back).
    let vx = o[0] + d[0] * t + f64::from(pivot[0]);
    let vy = o[1] + d[1] * t + f64::from(pivot[1]);
    if vx < 0.0 || vy < 0.0 {
        return None;
    }
    let (cx, cy) = (vx as u32, vy as u32);
    if cx >= dx || cy >= dy {
        return None;
    }
    Some([cx, cy, dz - 1]) // the bottom layer sits on the floor
}

/// The selection mode implied by the held modifiers (Ctrl is reserved for
/// the eyedropper, so it doesn't appear here).
fn sel_mode(m: ModifiersState) -> SelMode {
    if m.shift_key() {
        SelMode::Add
    } else if m.alt_key() {
        SelMode::Remove
    } else {
        SelMode::Replace
    }
}

impl Tool {
    /// Tools that drag-paint (a held-drag applies them along the path,
    /// coalesced into one undo step). Only recolouring qualifies: it
    /// leaves geometry untouched, so a drag can't "tunnel" along the
    /// click ray the way place/erase/sphere would (each removed or added
    /// front voxel re-exposes the one behind it, cascading through the
    /// model). Those stay click-once.
    fn is_continuous(self) -> bool {
        matches!(self, Tool::Paint)
    }
}

/// The mutable editor document + tool state the UI drives.
#[allow(clippy::struct_excessive_bools)] // independent view/edit toggles, not a state enum
struct Editor {
    document: Document,
    tool: Tool,
    /// Current voxlap-packed `0x80RRGGBB` paint colour.
    color: u32,
    /// Sphere-tool radius in voxels.
    radius: i32,
    /// First corner of an in-progress box (set on the first click).
    box_anchor: Option<[i32; 3]>,
    /// Distinct colours used in the model, for the "colours in model"
    /// palette. Refreshed whenever the model changes.
    model_palette: Vec<u32>,
    /// UI language.
    lang: Lang,
    /// Directional sprite lighting (lightmode 1) on; off renders flat.
    lighting: bool,
    /// Draw the reference bounding box / floor grid / origin axes.
    show_grid: bool,
    /// Overlay a wireframe on exposed voxel faces, so boundaries read even
    /// in flat-shaded shadow (there is no ambient occlusion / light bake).
    show_edges: bool,
    /// Mirror the viewport horizontally to correct roxlap's left-handed
    /// (X-mirrored) render. On by default; pure view setting, never saved.
    flip_x: bool,
    /// CPU ray-plane density (voxlap `anginc`): `1.0` is baseline, `< 1`
    /// supersamples the angular fan (more ray planes, smoother thin
    /// geometry), `> 1` coarsens it. Adjusted live with `[` / `]`. Pure
    /// view/diagnostic setting, never saved; CPU backend only.
    anginc: f32,
    /// Sprite vs voxel-grid render.
    render_mode: RenderMode,
    /// Target dimensions edited in the Size panel (the "Resize" button
    /// applies them); kept in sync with the model on structural changes.
    resize_dims: [u32; 3],
    /// The currently selected voxel cells (Select tool). Operated on by
    /// delete / copy / paste; pruned to bounds after structural edits.
    selection: HashSet<[u32; 3]>,
    /// Copied voxels at their **absolute** source positions, so a paste
    /// lands where they came from: `(position, colour)`.
    clipboard: Vec<([i32; 3], u32)>,
    /// The pasted voxels currently floating above the model, if any (see
    /// [`FloatLayer`]); committed into the model on deselect.
    float: Option<FloatLayer>,
    /// A reference image rendered as a non-destructive guide layer, if any.
    reference: Option<Reference>,
    /// "Move reference" mode: left-drag slides the reference in its plane
    /// instead of applying the tool.
    ref_move_mode: bool,
    /// The reference image changed (loaded / replaced / removed): the sprite
    /// texture must be re-uploaded. Placement changes don't set this — they
    /// only move the quad, which is rebuilt every frame.
    ref_image_dirty: bool,
    /// When editing a rigged character: the rig being edited. `document`
    /// holds the working copy of [`active_bone`]'s mesh; the other bones
    /// keep their last-committed mesh here. `None` = plain model editing.
    rig: Option<Rig>,
    /// The bone whose mesh `document` currently edits (index into
    /// `rig.bones`). Meaningless when `rig` is `None`.
    active_bone: usize,
    /// Which aspect of the rig is being edited (Sculpt / Skeleton /
    /// Animate). Meaningless when `rig` is `None`.
    rig_mode: RigMode,
    /// Animate mode: whether the timeline is playing (advancing time each
    /// frame) or paused (holding the current pose). Default `true` to match
    /// the historic auto-play behaviour.
    anim_playing: bool,
    /// Animate mode: which clip the timeline previews (index into
    /// `rig.clips`). Clamped to the clip count on use.
    active_clip: usize,
    /// Animate mode: the selected keyframe (index into the active clip's
    /// sorted keyframes), or `None`. Transient view state, not part of the rig
    /// or its undo; clamped / reset when the clip changes or a key is removed.
    selected_key: Option<usize>,
    /// The rig's skeleton changed (a hinge edit): the posed preview must be
    /// rebuilt from `rig`.
    rig_dirty: bool,
    /// The viewport scene needs a rebuild from the model.
    dirty: bool,
    /// Whole-rig undo / redo snapshots (rig mode only). Each entry is the
    /// full rig + active bone *before* one edit. Plain model editing uses the
    /// per-bone [`Document`] history instead; this stays empty there.
    rig_undo: Vec<RigSnapshot>,
    rig_redo: Vec<RigSnapshot>,
    /// Pre-edit snapshot for an in-progress sculpt stroke, committed to
    /// `rig_undo` on stroke end only if the stroke changed anything.
    rig_pending: Option<RigSnapshot>,
}

/// One whole-rig undo entry: the rig (every bone's mesh + hinge, clips, root)
/// and which bone was active, captured before an edit.
struct RigSnapshot {
    rig: Rig,
    active_bone: usize,
}

/// Cap on the rig undo/redo depth (each entry clones the whole rig).
const RIG_UNDO_DEPTH: usize = 100;

impl Editor {
    /// The model as the viewport should show it: the document model with
    /// the floating layer (if any) composited on top. Borrows the document
    /// model when nothing floats (the common case); clones and overlays
    /// only while a float is active.
    #[allow(clippy::cast_sign_loss)] // negative float cells are filtered out first
    fn display_model(&self) -> Cow<'_, VoxelModel> {
        let Some(layer) = &self.float else {
            return Cow::Borrowed(self.document.model());
        };
        let mut composite = self.document.model().clone();
        // Clear the cells the layer was lifted from, so a moved selection
        // leaves a hole instead of a ghost copy.
        for &p in &layer.lifted_from {
            composite.set(p[0], p[1], p[2], 0);
        }
        let (dx, dy, dz) = composite.dims();
        for &(pos, col) in &layer.cells {
            if pos[0] >= 0 && pos[1] >= 0 && pos[2] >= 0 {
                let (px, py, pz) = (pos[0] as u32, pos[1] as u32, pos[2] as u32);
                if px < dx && py < dy && pz < dz {
                    composite.set(px, py, pz, col);
                }
            }
        }
        Cow::Owned(composite)
    }
}

impl Editor {
    fn new(model: VoxelModel) -> Self {
        let lang = std::env::var("DEMIURG_LANG")
            .ok()
            .and_then(|c| Lang::from_code(&c))
            .unwrap_or_default();
        let model_palette = model.used_colors();
        let (dx, dy, dz) = model.dims();
        Self {
            document: Document::new(model),
            tool: Tool::Place,
            color: 0x80c8_c8c8,
            radius: 2,
            box_anchor: None,
            model_palette,
            lang,
            lighting: true,
            show_grid: true,
            show_edges: true,
            flip_x: true,
            anginc: 1.0,
            render_mode: DEFAULT_RENDER_MODE,
            resize_dims: [dx, dy, dz],
            selection: HashSet::new(),
            clipboard: Vec::new(),
            float: None,
            reference: None,
            ref_move_mode: false,
            ref_image_dirty: false,
            rig: None,
            active_bone: 0,
            rig_mode: RigMode::Sculpt,
            anim_playing: true,
            active_clip: 0,
            selected_key: None,
            rig_dirty: false,
            dirty: false,
            rig_undo: Vec::new(),
            rig_redo: Vec::new(),
            rig_pending: None,
        }
    }

    /// Snapshot the current true rig state: the rig with the active bone's
    /// *working* mesh folded in (in Sculpt the live mesh lives in `document`;
    /// in Skeleton / Animate it's already committed into `rig`). `None`
    /// outside rig mode.
    fn rig_state(&self) -> Option<RigSnapshot> {
        let mut rig = self.rig.clone()?;
        if self.rig_mode == RigMode::Sculpt {
            if let Some(b) = rig.bones.get_mut(self.active_bone) {
                b.model = self.document.model().clone();
            }
        }
        Some(RigSnapshot {
            rig,
            active_bone: self.active_bone,
        })
    }

    /// Push a captured pre-edit snapshot onto the undo stack (clearing redo,
    /// capping depth). Pair with [`Self::rig_state`] for ops that may turn out
    /// to be no-ops (push only on success).
    fn rig_push_undo(&mut self, snap: RigSnapshot) {
        self.rig_redo.clear();
        self.rig_undo.push(snap);
        if self.rig_undo.len() > RIG_UNDO_DEPTH {
            self.rig_undo.remove(0);
        }
    }

    /// Record the pre-edit rig state for undo. Call *before* a mutation that is
    /// certain to change the rig (structural op, bone drag, sculpt stroke).
    /// No-op outside rig mode.
    fn rig_checkpoint(&mut self) {
        if let Some(snap) = self.rig_state() {
            self.rig_push_undo(snap);
        }
    }

    /// Capture a pending pre-edit snapshot at the start of an inline hinge
    /// interaction (a Skeleton-panel drag / focus). Committed by
    /// [`Self::rig_commit_pending`] only once a value actually changes, so a
    /// click that edits nothing leaves no undo step.
    fn rig_begin_edit(&mut self) {
        self.rig_pending = self.rig_state();
    }

    /// Commit the pending inline-edit snapshot as one undo step (if any).
    fn rig_commit_pending(&mut self) {
        if let Some(snap) = self.rig_pending.take() {
            self.rig_push_undo(snap);
        }
    }

    /// Whether an undo/redo is available — the rig stack in rig mode, the
    /// document history in plain model mode.
    fn can_undo(&self) -> bool {
        if self.rig.is_some() {
            !self.rig_undo.is_empty()
        } else {
            self.document.can_undo()
        }
    }

    fn can_redo(&self) -> bool {
        if self.rig.is_some() {
            !self.rig_redo.is_empty()
        } else {
            self.document.can_redo()
        }
    }

    /// Load image `bytes` as the reference layer (replacing any current one);
    /// the sprite texture re-uploads next frame.
    fn set_reference(&mut self, bytes: &[u8], name: String) {
        match Reference::load(bytes, name) {
            Ok(r) => {
                self.reference = Some(r);
                self.ref_move_mode = false; // start in normal editing
                self.ref_image_dirty = true;
            }
            Err(e) => eprintln!("demiurg: reference image: {e}"),
        }
    }

    /// Recompute the model-colour palette (after any edit / load).
    fn refresh_palette(&mut self) {
        self.model_palette = self.document.model().used_colors();
    }

    /// Mirror the model dimensions into the Size-panel target fields
    /// (after a load / crop / grow).
    fn sync_resize_dims(&mut self) {
        let (x, y, z) = self.document.dims();
        self.resize_dims = [x, y, z];
    }

    /// Apply the active tool at a resolved pick.
    #[allow(clippy::cast_possible_wrap)] // voxel coords are far below i32::MAX
    fn apply(&mut self, hit: PickHit) -> bool {
        let color = self.color;
        let changed = match self.tool {
            Tool::Place => match in_bounds(hit.place, self.document.dims()) {
                Some(p) => self.document.set_voxel(p, color),
                None => false,
            },
            Tool::Erase => self.document.erase_voxel(hit.voxel),
            Tool::Paint => self.document.paint_voxel(hit.voxel, color),
            Tool::Eyedropper => {
                self.color = self
                    .document
                    .model()
                    .get(hit.voxel[0], hit.voxel[1], hit.voxel[2]);
                false
            }
            Tool::Sphere => self.document.fill_sphere(
                [
                    hit.voxel[0] as i32,
                    hit.voxel[1] as i32,
                    hit.voxel[2] as i32,
                ],
                self.radius,
                color,
            ),
            Tool::Fill => self.document.flood_fill(hit.voxel, color),
            // Select is handled by the marquee path, never reaches apply.
            Tool::Select => false,
            Tool::Box => match self.box_anchor.take() {
                None => {
                    self.box_anchor = Some(hit.place);
                    false
                }
                Some(anchor) => {
                    let dims = self.document.dims();
                    self.document.fill_rect(
                        clamp_cell(anchor, dims),
                        clamp_cell(hit.place, dims),
                        color,
                    )
                }
            },
        };
        if changed {
            self.dirty = true;
        }
        changed
    }
}

#[allow(clippy::similar_names, clippy::too_many_lines)] // linear startup setup
/// Read a `--key value` / `--key=value` CLI flag as `f64`.
fn flag_f64(key: &str) -> Option<f64> {
    let args: Vec<String> = std::env::args().collect();
    let eq = format!("{key}=");
    for (i, a) in args.iter().enumerate() {
        if let Some(v) = a.strip_prefix(&eq) {
            return v.parse().ok();
        }
        if a == key {
            return args.get(i + 1).and_then(|s| s.parse().ok());
        }
    }
    None
}

/// Read a `--key value` / `--key=value` CLI flag as a string.
fn flag_str(key: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let eq = format!("{key}=");
    for (i, a) in args.iter().enumerate() {
        if let Some(v) = a.strip_prefix(&eq) {
            return Some(v.to_string());
        }
        if a == key {
            return args.get(i + 1).cloned();
        }
    }
    None
}

/// Whether a bare `--key` flag is present.
fn has_flag(key: &str) -> bool {
    std::env::args().any(|a| a == key)
}

/// Write a packed `0x00RRGGBB` framebuffer as a PNG (RGB).
#[allow(clippy::cast_possible_truncation)]
fn save_png(path: &str, fb: &[u32], width: u32, height: u32) {
    let mut img = image::RgbImage::new(width, height);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let v = fb[(y * width + x) as usize];
        *px = image::Rgb([
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
        ]);
    }
    match img.save(path) {
        Ok(()) => eprintln!("demiurg: wrote {path} ({width}x{height})"),
        Err(e) => eprintln!("demiurg: save {path}: {e}"),
    }
}

fn main() {
    // The CPU renderer is the default (it's reliable everywhere); `--gpu`
    // opts into the GPU backend, `--cpu` forces CPU. The first non-flag
    // argument is the file to open.
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Dev utility: write a sample `.rkc` (the synthetic demo rig) and exit,
    // with no window — a quick way to mint a file for the load path.
    if let Some(path) = std::env::var_os("DEMIURG_KFA_DUMP") {
        match std::fs::write(&path, demiurg_view::demo_rkc_bytes()) {
            Ok(()) => eprintln!("demiurg: wrote {}", path.to_string_lossy()),
            Err(e) => eprintln!("demiurg: DEMIURG_KFA_DUMP {}: {e}", path.to_string_lossy()),
        }
        return;
    }

    let force_cpu = args.iter().any(|a| a == "--cpu");
    let force_gpu = args.iter().any(|a| a == "--gpu");
    // The first non-flag argument is the file to open. Flags that take a
    // separate value token (`--shot out.png`, `--dist 24`) must not have
    // that value mistaken for the file path.
    const VALUE_FLAGS: &[&str] = &[
        "--shot", "--cx", "--cy", "--cz", "--yaw", "--pitch", "--dist", "--width", "--height",
        "--anginc",
    ];
    let arg = {
        let mut found = None;
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a.starts_with('-') {
                if VALUE_FLAGS.contains(&a.as_str()) {
                    i += 1; // also skip this flag's value token
                }
                i += 1;
                continue;
            }
            found = Some(a.clone());
            break;
        }
        found
    };
    let autosave = autosave_path();

    // An `.rkc` argument (or DEMIURG_KFA) opens a rigged character for
    // editing; a `.demiurg` is an editable project (its path drives Ctrl+S);
    // a `.kv6` / `.vox` opens as a fresh model. With no argument, recover an
    // autosave if one survived a previous crash.
    let rkc_arg = arg
        .as_deref()
        .filter(|p| p.to_ascii_lowercase().ends_with(".rkc"))
        .map(str::to_string);
    let startup_rig: Option<Rig> = if let Some(p) = &rkc_arg {
        match std::fs::read(p).map(|b| Rig::from_rkc_bytes(&b)) {
            Ok(Ok(rig)) if !rig.bones.is_empty() => Some(rig),
            Ok(Ok(_)) => {
                eprintln!("demiurg: {p}: character has no bones");
                None
            }
            Ok(Err(e)) => {
                eprintln!("demiurg: {p}: {e}");
                None
            }
            Err(e) => {
                eprintln!("demiurg: read {p}: {e}");
                None
            }
        }
    } else if std::env::var_os("DEMIURG_KFA").is_some() {
        Some(demiurg_view::demo_rig())
    } else {
        None
    };

    let (model, project_path, doc_name, recovered) = if let Some(rig) = &startup_rig {
        // Rig mode starts editing the first bone's mesh.
        let name = rkc_arg.as_deref().and_then(|p| stem_of(Path::new(p)));
        (rig.bones[0].model.clone(), None, name, false)
    } else if rkc_arg.is_some() {
        (new_model(), None, None, false) // .rkc given but failed to load
    } else if let Some(p) = &arg {
        let proj = p.ends_with(".demiurg").then(|| PathBuf::from(p));
        (load_any(p), proj, stem_of(Path::new(p)), false)
    } else if let Some(m) = recover_autosave(&autosave) {
        eprintln!(
            "demiurg: recovered unsaved work from {}",
            autosave.display()
        );
        (m, None, None, true)
    } else {
        eprintln!("demiurg: blank canvas (pass a .kv6 / .demiurg / .rkc path to open one)");
        (new_model(), None, None, false)
    };

    let mut view = ModelView::new(&model, DEFAULT_RENDER_MODE);
    let mut camera = view.framing_camera();

    // Headless screenshot mode: `--shot <out.png>` renders the loaded
    // model on the CPU from the given camera and exits (no window). The
    // camera overrides mirror the in-app `P`-key dump exactly, so a bad
    // angle found in the GUI can be reproduced here verbatim:
    //   --cx/--cy/--cz (look-at), --yaw/--pitch (radians), --dist,
    //   --width/--height (default 900x700), --no-flip (disable the X flip).
    if let Some(path) = flag_str("--shot") {
        if let Some(v) = flag_f64("--cx") {
            camera.center.x = v;
        }
        if let Some(v) = flag_f64("--cy") {
            camera.center.y = v;
        }
        if let Some(v) = flag_f64("--cz") {
            camera.center.z = v;
        }
        if let Some(v) = flag_f64("--yaw") {
            camera.yaw = v;
        }
        if let Some(v) = flag_f64("--pitch") {
            camera.pitch = v;
        }
        if let Some(v) = flag_f64("--dist") {
            camera.dist = v;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let w = flag_f64("--width").map_or(900u32, |v| v as u32);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let h = flag_f64("--height").map_or(700u32, |v| v as u32);
        let flip = !has_flag("--no-flip");
        #[allow(clippy::cast_possible_truncation)]
        let anginc = flag_f64("--anginc").map_or(1.0f32, |v| v as f32);
        eprintln!(
            "demiurg: shot cam --cx {:.4} --cy {:.4} --cz {:.4} --yaw {:.5} --pitch {:.5} --dist {:.3} ({}x{}, flip_x={flip}, anginc={anginc})",
            camera.center.x,
            camera.center.y,
            camera.center.z,
            camera.yaw,
            camera.pitch,
            camera.dist,
            w,
            h
        );
        let fb = view.render_cpu(&camera, w, h, VOXEL_SIDE_SHADES, SKY_COLOR, flip, anginc);
        save_png(&path, &fb, w, h);
        return;
    }

    let mut editor = Editor::new(model);
    editor.rig = startup_rig; // edits the active bone (0); None = plain model
    if recovered {
        editor.document.mark_unsaved(); // recovered work has no save point yet
    }
    let mut app = App {
        window: None,
        renderer: None,
        view,
        camera,
        editor,
        egui_ctx: egui::Context::default(),
        egui_state: None,
        keys: Keys::default(),
        modifiers: ModifiersState::empty(),
        orbiting: false,
        panning: false,
        painting: false,
        last_paint: None,
        cursor: (0.0, 0.0),
        last_drag: None,
        doc_name,
        project_path,
        pending_save: None,
        pending_dialog: None,
        autosave_path: autosave,
        next_autosave: Instant::now() + AUTOSAVE_INTERVAL,
        recovered,
        last_title: None,
        confirm_quit: false,
        marquee: None,
        drag: None,
        ref_drag: None,
        bone_drag: None,
        pose_drag: None,
        last_tool: Tool::Place,
        force_cpu,
        force_gpu,
        ref_image: None,
        // The posed-rig preview (KfaView) is dormant in this slice — rig
        // editing renders the active bone as a plain model. A later Edit /
        // Animate toggle revives it.
        kfa: None,
        next_frame: Instant::now(),
    };

    let event_loop = EventLoop::new().expect("winit: create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app).expect("winit: run_app");
}

/// Load a `.kv6`, `.vox`, or `.demiurg` by extension, or exit with a
/// message.
fn load_any(path: &str) -> VoxelModel {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("demiurg: cannot read {path}: {e}");
        exit(2);
    });
    let is_vox = Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vox"));
    let model = if path.ends_with(".demiurg") {
        project::from_bytes(&bytes).map_err(|e| e.to_string())
    } else if is_vox {
        VoxelModel::from_vox_bytes(&bytes).map_err(|e| e.to_string())
    } else {
        VoxelModel::from_kv6_bytes(&bytes).map_err(|e| e.to_string())
    };
    model.unwrap_or_else(|e| {
        eprintln!("demiurg: {path}: {e}");
        exit(2);
    })
}

/// A blank canvas with a single seed voxel at the centre, so the place
/// tool has a face to build on.
fn new_model() -> VoxelModel {
    let mut m = VoxelModel::new(NEW_DIMS, NEW_DIMS, NEW_DIMS);
    let c = NEW_DIMS / 2;
    m.set(c, c, c, 0x80c8_c8c8);
    m
}

/// Clamp an `i32` cell into `[0, dims)` per axis.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)] // dims are small; clamped non-negative below the dim
fn clamp_cell(p: [i32; 3], dims: (u32, u32, u32)) -> [u32; 3] {
    let d = [dims.0 as i32, dims.1 as i32, dims.2 as i32];
    [
        p[0].clamp(0, d[0] - 1) as u32,
        p[1].clamp(0, d[1] - 1) as u32,
        p[2].clamp(0, d[2] - 1) as u32,
    ]
}

/// `Some` voxel coord if `p` is in `[0, dims)`, else `None`.
#[allow(clippy::cast_sign_loss)] // guarded non-negative before the cast
fn in_bounds(p: [i32; 3], dims: (u32, u32, u32)) -> Option<[u32; 3]> {
    let (dx, dy, dz) = dims;
    if p[0] >= 0
        && p[1] >= 0
        && p[2] >= 0
        && (p[0] as u32) < dx
        && (p[1] as u32) < dy
        && (p[2] as u32) < dz
    {
        Some([p[0] as u32, p[1] as u32, p[2] as u32])
    } else {
        None
    }
}

/// Held-key state, applied per frame for smooth orbiting.
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // a held-key input map, not a state flag soup
struct Keys {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    zoom_in: bool,
    zoom_out: bool,
}

#[allow(clippy::struct_excessive_bools)] // independent input/UI flags, not a state enum
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<SceneRenderer>,
    view: ModelView,
    camera: OrbitCamera,
    editor: Editor,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    keys: Keys,
    modifiers: ModifiersState,
    /// Right-mouse drag orbits the camera (left mouse edits).
    orbiting: bool,
    /// Middle-mouse (or Shift+right) drag pans the camera's look-at point.
    panning: bool,
    /// Left mouse held with a continuous tool: an open drag-paint stroke.
    painting: bool,
    /// Last cell painted this stroke, to skip redundant re-applies.
    last_paint: Option<[i32; 3]>,
    cursor: (f64, f64),
    last_drag: Option<(f64, f64)>,
    /// File stem of the open document (for the title), or `None` if new.
    doc_name: Option<String>,
    /// Path of the open `.demiurg` project, if any — `Ctrl+S` overwrites it
    /// without a dialog.
    project_path: Option<PathBuf>,
    /// A background save in progress, else `None`.
    pending_save: Option<PendingSave>,
    /// A native file dialog running off the event loop, else `None`.
    pending_dialog: Option<PendingDialog>,
    /// Where the periodic background autosave is written.
    autosave_path: PathBuf,
    /// When the next autosave is due.
    next_autosave: Instant,
    /// Show the "recovered from autosave" banner until dismissed.
    recovered: bool,
    /// Last window title set, to avoid redundant `set_title` calls.
    last_title: Option<String>,
    /// The unsaved-changes quit modal is showing.
    confirm_quit: bool,
    /// An in-progress selection marquee drag (Select tool), else `None`.
    marquee: Option<Marquee>,
    /// An in-progress move drag of the floating layer, else `None`.
    drag: Option<DragMove>,
    /// An in-progress drag of the reference layer (Move mode), else `None`.
    ref_drag: Option<RefDrag>,
    bone_drag: Option<BoneDrag>,
    /// An in-progress viewport pose (Animate mode), else `None`.
    pose_drag: Option<PoseDrag>,
    /// The active tool last frame, to detect a tool switch (keyboard or
    /// UI) and settle a floating layer when the user leaves Select.
    last_tool: Tool,
    /// Force the CPU renderer (`--cpu`); skips GPU device creation.
    force_cpu: bool,
    /// Opt into the GPU renderer (`--gpu`); CPU is the default.
    force_gpu: bool,
    /// The uploaded reference-image texture (`upload_image`), drawn each
    /// frame as a world-placed sprite. `None` when no reference is loaded.
    ref_image: Option<ImageId>,
    /// KFA rig preview (first slice of the animation editor): an animated
    /// skeletal character drawn via `set_kfa_sprites`/`update_kfa_poses`.
    /// Seeded with a synthetic rig when `DEMIURG_KFA` is set.
    kfa: Option<KfaView>,
    /// When the next frame should render (drives the ~60 fps cap).
    next_frame: Instant,
}

impl App {
    /// Apply held keys to the camera once per frame.
    fn step_camera(&mut self) {
        let axis = |neg: bool, pos: bool| f64::from(i32::from(pos) - i32::from(neg));
        let dyaw = axis(self.keys.left, self.keys.right) * 0.03;
        let dpitch = axis(self.keys.down, self.keys.up) * 0.02;
        let dzoom = axis(self.keys.zoom_in, self.keys.zoom_out) * (self.camera.dist * 0.04);
        if dyaw != 0.0 || dpitch != 0.0 || dzoom != 0.0 {
            self.camera.orbit(dyaw, dpitch, dzoom);
        }
    }

    /// Pan the camera by a cursor delta in pixels, so the grabbed point
    /// tracks the cursor. The world-per-pixel scale uses the renderer's
    /// ~90° horizontal FOV (focal = width / 2) at the look-at distance, so
    /// the pan feels 1:1 with the drag; negated so the scene follows.
    fn pan_camera(&mut self, dx: f64, dy: f64) {
        let width = self
            .window
            .as_ref()
            .map_or(1.0, |w| f64::from(w.inner_size().width).max(1.0));
        let world_per_px = self.camera.dist / (width * 0.5);
        self.camera.pan(-dx * world_per_px, -dy * world_per_px);
    }

    /// The cell the active tool would affect under the cursor (place
    /// target for Place, hit voxel otherwise), or `None` over a panel /
    /// on a miss.
    #[allow(clippy::cast_possible_wrap)] // voxel coords are far below i32::MAX
    fn hover_cell(&self) -> Option<[i32; 3]> {
        if self.egui_ctx.is_pointer_over_egui() {
            return None;
        }
        let hit = self.tool_pick()?;
        Some(if matches!(self.editor.tool, Tool::Place) {
            hit.place
        } else {
            [
                hit.voxel[0] as i32,
                hit.voxel[1] as i32,
                hit.voxel[2] as i32,
            ]
        })
    }

    /// World-space lines for `draw_lines`: the reference grid / box /
    /// axes (when enabled) and the hover wire box. The engine depth-tests
    /// them against the rendered frame, so the model occludes them.
    fn scene_lines(&self) -> Vec<Line3> {
        let pivot = self.editor.document.pivot();
        let mut lines = Vec::new();
        // The editing gizmos (grid box, voxel edges, selection, hover cube)
        // belong to the document model — skip them in the Animate preview,
        // which renders the posed rig instead.
        if self.kfa.is_none() {
            if self.editor.show_grid {
                lines.extend(demiurg_view::reference_lines_3d(
                    pivot,
                    self.editor.document.dims(),
                ));
            }
            if self.editor.show_edges {
                lines.extend(demiurg_view::voxel_edge_lines_3d(
                    self.editor.document.model(),
                    pivot,
                    VOXEL_EDGE_COLOR,
                    1.0,
                ));
            }
            if !self.editor.selection.is_empty() {
                let cells: Vec<[u32; 3]> = self.editor.selection.iter().copied().collect();
                lines.extend(demiurg_view::selection_lines_3d(pivot, &cells));
            }
            // No hover box mid-drag: the selection outline already tracks the
            // moving layer, and the hover would pick the model under it.
            if self.drag.is_none() && self.ref_drag.is_none() {
                if let Some(cell) = self.hover_cell() {
                    lines.extend(demiurg_view::voxel_box_lines_3d(pivot, cell));
                }
            }
        }
        if let Some(kfa) = &self.kfa {
            lines.extend(kfa.bone_lines(Some(self.editor.active_bone)));
            lines.extend(self.pose_gizmo_lines());
        }
        lines
    }

    /// Cursor position to cast rays through. With the viewport flip on, the
    /// display is mirrored but the engine's projection isn't, so a click at
    /// window-x corresponds to the engine's `width - x`.
    fn ray_cursor(&self) -> (f64, f64) {
        if self.editor.flip_x {
            if let Some(w) = self.window.as_ref() {
                return (
                    f64::from(w.inner_size().width) - self.cursor.0,
                    self.cursor.1,
                );
            }
        }
        self.cursor
    }

    /// The voxel under the cursor, if any.
    fn pointer_pick(&self) -> Option<PickHit> {
        let cam = self.camera.to_roxlap();
        let (cx, cy) = self.ray_cursor();
        let ray = self.renderer.as_ref()?.view_ray(&cam, cx, cy)?;
        pick_voxel(self.editor.document.model(), ray.origin, ray.dir)
    }

    /// The cursor ray as world `(origin, dir)` component arrays.
    fn pointer_ray(&self) -> Option<([f64; 3], [f64; 3])> {
        let cam = self.camera.to_roxlap();
        let (cx, cy) = self.ray_cursor();
        let r = self.renderer.as_ref()?.view_ray(&cam, cx, cy)?;
        Some((
            [r.origin.x, r.origin.y, r.origin.z],
            [r.dir.x, r.dir.y, r.dir.z],
        ))
    }

    /// The bone whose gizmo segment is nearest the cursor on screen, within a
    /// pixel threshold — for click-to-select in the Animate viewport. Projects
    /// each bone's solved pivot (and its parent's, for the segment) to engine
    /// screen space via the renderer's `project_point` — the same projection
    /// the gizmo lines are drawn with, and the inverse of `view_ray` that
    /// `ray_cursor` is calibrated against — then compares in pixels. `None`
    /// when the click is far from every bone (the selection then stays put) or
    /// nothing projects (before the first frame / behind the camera).
    fn pick_bone_screen(&self) -> Option<usize> {
        const MAX_PX: f64 = 32.0;
        let cam = self.camera.to_roxlap();
        let renderer = self.renderer.as_ref()?;
        let kfa = self.kfa.as_ref()?;
        let rig = self.editor.rig.as_ref()?;
        let (cx, cy) = self.ray_cursor();
        let cursor = [cx, cy];
        let proj = |w: [f32; 3]| {
            renderer
                .project_point(&cam, w)
                .map(|(x, y)| [f64::from(x), f64::from(y)])
        };
        let mut best: Option<(usize, f64)> = None;
        for (i, bone) in rig.bones.iter().enumerate() {
            let Some((p, _)) = kfa.limb_pose(i) else {
                continue;
            };
            let Some(a) = proj(p) else {
                continue;
            };
            // Measure to the bone's gizmo segment (pivot → parent pivot); a
            // root (or an unprojectable parent) falls back to its pivot point.
            let dist = if bone.hinge.parent >= 0 {
                #[allow(clippy::cast_sign_loss)] // parent >= 0 checked
                let parent = bone.hinge.parent as usize;
                match kfa.limb_pose(parent).and_then(|(pp, _)| proj(pp)) {
                    Some(b) => point_seg_dist_2d(cursor, a, b),
                    None => point_dist_2d(cursor, a),
                }
            } else {
                point_dist_2d(cursor, a)
            };
            if best.is_none_or(|(_, bd)| dist < bd) {
                best = Some((i, dist));
            }
        }
        best.filter(|&(_, d)| d <= MAX_PX).map(|(i, _)| i)
    }

    /// The cursor ray for the **KFA sprite** scene (posed rig / bones), as
    /// world `(origin, dir)`. The sprite render path is a 180° rotation of
    /// the voxel-grid convention that [`Self::ray_cursor`] is calibrated for
    /// — *both* screen axes are inverted — so a bone drag built on the plain
    /// pointer ray tracks the wrong way on X and Y. Compensate by mirroring
    /// the cursor in both axes:
    /// - X: opposite of `ray_cursor` (mirror when flip is **off**), since
    ///   `flip_x` already mirrors the displayed X.
    /// - Y: always (flip never touches Y).
    fn bone_pointer_ray(&self) -> Option<([f64; 3], [f64; 3])> {
        let cam = self.camera.to_roxlap();
        let w = self.window.as_ref()?.inner_size();
        let cx = if self.editor.flip_x {
            self.cursor.0
        } else {
            f64::from(w.width) - self.cursor.0
        };
        let cy = f64::from(w.height) - self.cursor.1;
        let r = self.renderer.as_ref()?.view_ray(&cam, cx, cy)?;
        Some((
            [r.origin.x, r.origin.y, r.origin.z],
            [r.dir.x, r.dir.y, r.dir.z],
        ))
    }

    /// A synthetic hit on the model's floor under the cursor (see
    /// [`floor_cell`]): its `place` is the bottom-layer cell, so the Place
    /// tool can seed voxels on the floor when there's nothing solid to
    /// click (e.g. a model emptied of its last voxel).
    #[allow(clippy::cast_possible_wrap)] // voxel coords are far below i32::MAX
    fn floor_pick(&self) -> Option<PickHit> {
        let (o, d) = self.pointer_ray()?;
        let cell = floor_cell(
            o,
            d,
            self.editor.document.pivot(),
            self.editor.document.dims(),
        )?;
        Some(PickHit {
            voxel: cell,
            normal: [0, 0, -1],
            place: [cell[0] as i32, cell[1] as i32, cell[2] as i32],
            t: f64::INFINITY, // synthetic floor hit; never ordered against a surface
        })
    }

    /// The pick a tool acts on: a real voxel hit, or — for the Place tool
    /// only — the floor cell under the cursor, so a model can be built
    /// from nothing.
    fn tool_pick(&self) -> Option<PickHit> {
        match self.pointer_pick() {
            None if self.editor.tool == Tool::Place => self.floor_pick(),
            other => other,
        }
    }

    /// Pick against the **composite** (model + floating layer), so a
    /// floating voxel can be grabbed even though it isn't in the document.
    fn grab_pick(&self) -> Option<PickHit> {
        let cam = self.camera.to_roxlap();
        let (cx, cy) = self.ray_cursor();
        let ray = self.renderer.as_ref()?.view_ray(&cam, cx, cy)?;
        pick_voxel(&self.editor.display_model(), ray.origin, ray.dir)
    }

    /// If the cursor grabbed a *selected* voxel's face (plain click, no
    /// Shift/Alt), arm a move drag in that face's plane and return `true`.
    /// The selection isn't lifted yet — that waits for the first move.
    #[allow(clippy::cast_precision_loss)] // voxel coords are tiny; f64 is exact
    fn try_begin_drag(&mut self) -> bool {
        if self.editor.selection.is_empty()
            || self.modifiers.shift_key()
            || self.modifiers.alt_key()
        {
            return false;
        }
        let Some(hit) = self.grab_pick() else {
            return false;
        };
        if !self.editor.selection.contains(&hit.voxel) {
            return false; // grabbing an unselected voxel starts a new selection
        }
        // The grabbed face: its normal picks the locked axis, and the face
        // boundary in world space is the drag plane.
        let axis = (0..3).find(|&a| hit.normal[a] != 0).unwrap_or(0);
        let pivot = self.editor.document.pivot();
        let face = f64::from(hit.voxel[axis]) + f64::from(i32::from(hit.normal[axis] > 0));
        let plane_coord = face - f64::from(pivot[axis]);
        let Some((o, d)) = self.pointer_ray() else {
            return false;
        };
        if d[axis].abs() < 1e-9 {
            return false; // looking edge-on: no stable plane intersection
        }
        let t = (plane_coord - o[axis]) / d[axis];
        if t <= 0.0 {
            return false;
        }
        let anchor = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
        // (The move bakes into the mesh in `commit_float`, which checkpoints.)
        // Drag an existing float directly; a model selection lifts lazily.
        let (base, lifted) = match &self.editor.float {
            Some(f) => (f.cells.clone(), true),
            None => (Vec::new(), false),
        };
        self.drag = Some(DragMove {
            axis,
            plane_coord,
            anchor,
            base,
            lifted,
            last_delta: [0; 3],
        });
        true
    }

    /// Lift the selected model voxels into a floating layer (recording
    /// where they came from, to clear on commit) and return them. No-op
    /// returning empty if nothing occupied is selected.
    #[allow(clippy::cast_possible_wrap)] // selection coords are far below i32::MAX
    fn lift_selection_to_float(&mut self) -> Vec<([i32; 3], u32)> {
        let model = self.editor.document.model();
        let cells: Vec<([i32; 3], u32)> = self
            .editor
            .selection
            .iter()
            .filter_map(|&c| {
                let col = model.get(c[0], c[1], c[2]);
                (col != 0).then_some(([c[0] as i32, c[1] as i32, c[2] as i32], col))
            })
            .collect();
        if cells.is_empty() {
            return Vec::new();
        }
        let lifted_from = self.editor.selection.iter().copied().collect();
        self.editor.float = Some(FloatLayer {
            cells: cells.clone(),
            lifted_from,
        });
        self.editor.dirty = true;
        cells
    }

    /// Update the move drag from the current cursor: snap the cursor ray
    /// to a whole-voxel offset in the drag plane and offset the floating
    /// layer (lifting the selection on the first real move).
    fn update_drag(&mut self) {
        let Some(drag) = &self.drag else {
            return;
        };
        let (axis, plane_coord, anchor, last_delta, lifted) = (
            drag.axis,
            drag.plane_coord,
            drag.anchor,
            drag.last_delta,
            drag.lifted,
        );
        let Some((o, d)) = self.pointer_ray() else {
            return;
        };
        let Some(delta) = plane_drag_delta(o, d, axis, plane_coord, anchor) else {
            return;
        };
        if delta == last_delta {
            return;
        }

        // Lift the model selection into a float on the first real move.
        if !lifted {
            if delta == [0, 0, 0] {
                return;
            }
            let base = self.lift_selection_to_float();
            if base.is_empty() {
                return;
            }
            if let Some(drag) = self.drag.as_mut() {
                drag.base = base;
                drag.lifted = true;
            }
        }

        let Some(drag) = &self.drag else {
            return;
        };
        let moved: Vec<([i32; 3], u32)> = drag
            .base
            .iter()
            .map(|&(p, col)| ([p[0] + delta[0], p[1] + delta[1], p[2] + delta[2]], col))
            .collect();
        let dims = self.editor.document.dims();
        if let Some(f) = self.editor.float.as_mut() {
            f.cells = moved;
        }
        if let Some(f) = &self.editor.float {
            self.editor.selection = float_selection(&f.cells, dims);
        }
        if let Some(drag) = self.drag.as_mut() {
            drag.last_delta = delta;
        }
        self.editor.dirty = true;
    }

    /// Begin dragging the reference layer in its own plane (Move mode).
    fn begin_ref_drag(&mut self) {
        let Some(r) = &self.editor.reference else {
            return;
        };
        let (axis, _, _) = r.axes();
        let (depth, base_u, base_v) = (r.depth, r.offset_u, r.offset_v);
        let pivot = self.editor.document.pivot();
        // The reference sits on voxel coord `depth` along the normal axis;
        // grab the plane through the voxel centres (+0.5) for a 1:1 feel.
        let plane_coord = f64::from(depth) + 0.5 - f64::from(pivot[axis]);
        let Some((o, d)) = self.pointer_ray() else {
            return;
        };
        if d[axis].abs() < 1e-9 {
            return;
        }
        let t = (plane_coord - o[axis]) / d[axis];
        if t <= 0.0 {
            return;
        }
        let anchor = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
        self.ref_drag = Some(RefDrag {
            axis,
            plane_coord,
            anchor,
            base_u,
            base_v,
        });
    }

    /// Update the reference drag: snap the cursor ray to a whole-voxel
    /// in-plane offset and apply it to the reference.
    fn update_ref_drag(&mut self) {
        let Some(drag) = &self.ref_drag else {
            return;
        };
        let (axis, plane_coord, anchor, base_u, base_v) = (
            drag.axis,
            drag.plane_coord,
            drag.anchor,
            drag.base_u,
            drag.base_v,
        );
        let Some((o, d)) = self.pointer_ray() else {
            return;
        };
        let Some(delta) = plane_drag_delta(o, d, axis, plane_coord, anchor) else {
            return;
        };
        if let Some(r) = &mut self.editor.reference {
            let (_, u_axis, v_axis) = r.axes();
            // Only the in-plane offset changes — the overlay reprojects every
            // frame, so no texture rebuild (reference_dirty) is needed.
            r.offset_u = base_u + delta[u_axis];
            r.offset_v = base_v + delta[v_axis];
        }
    }

    /// Begin dragging the active bone (Skeleton mode): grab a screen-parallel
    /// plane through its pivot and remember the value being moved (`p[1]` for
    /// a child bone, `rig.root` for the root) and the parent's basis.
    fn begin_bone_drag(&mut self) {
        let bone = self.editor.active_bone;
        let Some((o, d)) = self.bone_pointer_ray() else {
            return;
        };
        let normal = self.camera.to_roxlap().forward;
        let Some(kfa) = &self.kfa else {
            return;
        };
        let Some((pivot, _)) = kfa.limb_pose(bone) else {
            return;
        };
        let Some(rig) = &self.editor.rig else {
            return;
        };
        let Some(b) = rig.bones.get(bone) else {
            return;
        };
        let (base, parent_basis) = if b.hinge.parent < 0 {
            (rig.root, None)
        } else {
            let p1 = b.hinge.p[1];
            #[allow(clippy::cast_sign_loss)] // parent >= 0 checked above
            let pb = kfa
                .limb_pose(b.hinge.parent as usize)
                .map(|(_, m)| m.map(|a| [f64::from(a[0]), f64::from(a[1]), f64::from(a[2])]));
            ([p1.x, p1.y, p1.z], pb)
        };
        let plane_point = [
            f64::from(pivot[0]),
            f64::from(pivot[1]),
            f64::from(pivot[2]),
        ];
        let Some(anchor) = ray_plane(o, d, plane_point, normal) else {
            return;
        };
        // The drag will move the bone — record the pre-drag rig for undo.
        self.editor.rig_checkpoint();
        self.bone_drag = Some(BoneDrag {
            bone,
            plane_point,
            plane_normal: normal,
            anchor,
            base,
            parent_basis,
        });
    }

    /// Update a bone drag: move the bone to follow the cursor in its plane,
    /// applying the world delta to `p[1]` (parent-local) or `rig.root`.
    #[allow(clippy::cast_possible_truncation)] // world deltas are small
    fn update_bone_drag(&mut self) {
        let Some(drag) = &self.bone_drag else {
            return;
        };
        let (bone, plane_point, normal, anchor, base, parent_basis) = (
            drag.bone,
            drag.plane_point,
            drag.plane_normal,
            drag.anchor,
            drag.base,
            drag.parent_basis,
        );
        let Some((o, d)) = self.bone_pointer_ray() else {
            return;
        };
        let Some(cur) = ray_plane(o, d, plane_point, normal) else {
            return;
        };
        let wd = [cur[0] - anchor[0], cur[1] - anchor[1], cur[2] - anchor[2]];
        // World delta -> parent-local (the velcro is in the parent's space);
        // the basis is orthonormal, so the inverse is its transpose (dots).
        let local = match parent_basis {
            Some(m) => [dot3(wd, m[0]), dot3(wd, m[1]), dot3(wd, m[2])],
            None => wd,
        };
        let new = [
            base[0] + local[0] as f32,
            base[1] + local[1] as f32,
            base[2] + local[2] as f32,
        ];
        if let Some(rig) = &mut self.editor.rig {
            match rig.bones.get(bone).map(|b| b.hinge.parent) {
                Some(p) if p < 0 => rig.root = new,
                Some(_) => {
                    let h = &mut rig.bones[bone].hinge;
                    h.p[1].x = new[0];
                    h.p[1].y = new[1];
                    h.p[1].z = new[2];
                }
                None => {}
            }
        }
        self.editor.rig_dirty = true;
    }

    /// Begin a viewport pose (Animate mode): grab the active bone and rotate it
    /// about its hinge axis, writing into the selected keyframe. Bails (no drag,
    /// gesture falls through to nothing) unless we're in Animate with a key
    /// selected and the active bone is poseable — a child (`parent >= 0`, roots
    /// aren't keyframed) with a non-empty range (`vmin < vmax`, locked bones
    /// The active world pivot and (unit) rotation axis of bone `bone` — the
    /// fixed point and axis a viewport pose rotates it about. The pivot is the
    /// bone's solved joint position; the axis is the parent-side velcro axis
    /// `hinge.v[1]` mapped to world by the **parent's** solved basis (the hinge
    /// axis is fixed in the parent's frame). `None` if the bone isn't poseable
    /// (root / locked), isn't solved, or the axis degenerates. Shared by the
    /// posing gesture and the axis gizmo.
    #[allow(clippy::cast_sign_loss)] // parent >= 0 once is_poseable passed
    fn bone_pose_axis(&self, bone: usize) -> Option<([f64; 3], [f64; 3])> {
        let kfa = self.kfa.as_ref()?;
        let rig = self.editor.rig.as_ref()?;
        if !rig.is_poseable(bone) {
            return None;
        }
        let b = rig.bones.get(bone)?;
        let (pivot, _) = kfa.limb_pose(bone)?;
        // The hinge axis is fixed in the parent's frame; map it to world by the
        // parent's solved basis. A child of the (dummy) root uses the sprite
        // basis (≈ identity), which limb_pose returns for the root too.
        let (_, pb) = kfa.limb_pose(b.hinge.parent as usize)?;
        let va = b.hinge.v[1];
        let world = [
            f64::from(va.x) * f64::from(pb[0][0])
                + f64::from(va.y) * f64::from(pb[1][0])
                + f64::from(va.z) * f64::from(pb[2][0]),
            f64::from(va.x) * f64::from(pb[0][1])
                + f64::from(va.y) * f64::from(pb[1][1])
                + f64::from(va.z) * f64::from(pb[2][1]),
            f64::from(va.x) * f64::from(pb[0][2])
                + f64::from(va.y) * f64::from(pb[1][2])
                + f64::from(va.z) * f64::from(pb[2][2]),
        ];
        let len = dot3(world, world).sqrt();
        if len < 1e-9 {
            return None; // degenerate axis
        }
        let axis = [world[0] / len, world[1] / len, world[2] / len];
        let piv = [
            f64::from(pivot[0]),
            f64::from(pivot[1]),
            f64::from(pivot[2]),
        ];
        Some((piv, axis))
    }

    /// Gizmo lines for the active bone's rotation axis (Animate mode): a line
    /// through the pivot along the hinge axis plus a ring in the rotation
    /// plane, so the user can see which axis a viewport pose rotates about
    /// before grabbing. Empty unless Animate + the active bone is poseable.
    /// Sized to the bone's mesh so the ring tracks the limb it rotates.
    fn pose_gizmo_lines(&self) -> Vec<Line3> {
        let mut lines = Vec::new();
        if self.editor.rig_mode != RigMode::Animate {
            return lines;
        }
        let bone = self.editor.active_bone;
        let Some((p, n)) = self.bone_pose_axis(bone) else {
            return lines;
        };
        // Radius ~ the bone's mesh size, with a floor so a tiny / empty mesh
        // still shows a visible gizmo.
        let r = self
            .editor
            .rig
            .as_ref()
            .and_then(|rig| rig.bones.get(bone))
            .map_or(8.0, |b| {
                let (x, y, z) = b.model.dims();
                f64::from(x.max(y).max(z)).max(4.0)
            });
        let (u, w) = plane_basis(n);
        // Axis line through the pivot (both directions), full diameter.
        lines.push(Line3 {
            a: [p[0] - n[0] * r, p[1] - n[1] * r, p[2] - n[2] * r],
            b: [p[0] + n[0] * r, p[1] + n[1] * r, p[2] + n[2] * r],
            color: GIZMO_AXIS_COLOR,
            width_px: 2.0,
            depth_test: false,
        });
        // Ring in the rotation plane, as a closed polyline.
        const SEG: usize = 32;
        let pt = |ang: f64| {
            let (s, c) = ang.sin_cos();
            [
                p[0] + (u[0] * c + w[0] * s) * r,
                p[1] + (u[1] * c + w[1] * s) * r,
                p[2] + (u[2] * c + w[2] * s) * r,
            ]
        };
        for i in 0..SEG {
            #[allow(clippy::cast_precision_loss)] // i <= 32
            let a0 = (i as f64) / (SEG as f64) * std::f64::consts::TAU;
            #[allow(clippy::cast_precision_loss)]
            let a1 = ((i + 1) as f64) / (SEG as f64) * std::f64::consts::TAU;
            lines.push(Line3 {
                a: pt(a0),
                b: pt(a1),
                color: GIZMO_RING_COLOR,
                width_px: 1.5,
                depth_test: false,
            });
        }
        lines
    }

    /// can't move). The pivot, world axis and base angle are captured once here.
    fn begin_pose_drag(&mut self) {
        if self.editor.rig_mode != RigMode::Animate {
            return;
        }
        let Some(key) = self.editor.selected_key else {
            return;
        };
        let clip = self.editor.active_clip;
        let bone = self.editor.active_bone;
        let Some((o, d)) = self.bone_pointer_ray() else {
            return;
        };
        let Some((piv, axis)) = self.bone_pose_axis(bone) else {
            return;
        };
        let Some(anchor) = ray_plane(o, d, piv, axis) else {
            return;
        };
        let ref0 = [anchor[0] - piv[0], anchor[1] - piv[1], anchor[2] - piv[2]];
        let base = self
            .editor
            .rig
            .as_ref()
            .and_then(|rig| {
                rig.clip_keyframes(clip)
                    .get(key)
                    .and_then(|kf| kf.angles.get(bone).copied())
            })
            .unwrap_or(0);
        // Posing pauses; undo is captured lazily (begin_edit now, commit on the
        // first real change) so a click that doesn't move adds no step.
        self.editor.anim_playing = false;
        self.editor.rig_begin_edit();
        self.pose_drag = Some(PoseDrag {
            bone,
            key,
            clip,
            pivot: piv,
            axis,
            ref0,
            base,
        });
    }

    /// Update a pose drag: sweep the active bone from `ref0` to the cursor's
    /// in-plane vector and write `base + swept` into the keyframe. The first
    /// real change commits the pending undo snapshot (one drag = one step).
    #[allow(clippy::cast_possible_truncation)] // the angle is clamped into i16 by the rig
    fn update_pose_drag(&mut self) {
        let Some(drag) = &self.pose_drag else {
            return;
        };
        let (bone, key, clip, pivot, axis, ref0, base) = (
            drag.bone, drag.key, drag.clip, drag.pivot, drag.axis, drag.ref0, drag.base,
        );
        let Some((o, d)) = self.bone_pointer_ray() else {
            return;
        };
        let Some(cur) = ray_plane(o, d, pivot, axis) else {
            return;
        };
        let r = [cur[0] - pivot[0], cur[1] - pivot[1], cur[2] - pivot[2]];
        let Some(sweep) = hinge_sweep(axis, ref0, r) else {
            return; // cursor on the pivot — the angle is noise
        };
        // roxlap's sprite render / stored rig frame is left-handed (the same
        // chirality issue as bone_pointer_ray, Gotcha #2): a right-handed world
        // sweep turns the bone the wrong way on screen, so negate it. This is a
        // FIXED flip (view-independent) — the rig's parent basis carries the
        // handedness, not the camera.
        let delta = -sweep;
        let new = (f64::from(base) + delta).round();
        let new = new.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
        // Skip frames that don't change the stored value, so a stationary press
        // commits no undo step and writes nothing (set_keyframe_angle clamps, so
        // compare against the post-clamp stored angle).
        let stored = self.editor.rig.as_ref().and_then(|rg| {
            rg.clip_keyframes(clip)
                .get(key)
                .and_then(|k| k.angles.get(bone).copied())
        });
        if stored == Some(new) {
            return;
        }
        // First real motion: fold the pre-drag snapshot into one undo step.
        self.editor.rig_commit_pending();
        if self
            .editor
            .rig
            .as_mut()
            .is_some_and(|rg| rg.set_keyframe_angle(clip, key, bone, new))
        {
            self.editor.rig_dirty = true;
        }
    }

    /// Left-button press: a quick eyedropper (Ctrl), a selection marquee
    /// (Select tool), a drag-paint stroke (continuous tools), or a
    /// click-once tool.
    fn begin_paint(&mut self) {
        if self.confirm_quit || self.busy() {
            return; // don't edit behind the quit / saving / dialog modal
        }
        if self.editor.rig.is_some() && self.editor.rig_mode != RigMode::Sculpt {
            // Skeleton: left-drag repositions the active bone. Animate:
            // left-drag rotates the active bone into the selected keyframe.
            if self.editor.rig_mode == RigMode::Skeleton {
                self.begin_bone_drag();
            } else {
                // Animate: a click near a bone first selects it, then starts
                // posing it — so the bone list isn't the only way to choose
                // what to rotate. A click far from every bone keeps the current
                // selection (and just poses it, if poseable).
                if let Some(i) = self.pick_bone_screen() {
                    self.select_bone(i);
                }
                self.begin_pose_drag();
            }
            return;
        }
        // "Move reference" mode: left-drag slides the reference, not the tool.
        if self.editor.ref_move_mode && self.editor.reference.is_some() {
            self.begin_ref_drag();
            return;
        }
        // Ctrl+click is a quick eyedropper, whatever the active tool — as is
        // the Eyedropper tool itself. Both sample the model or the reference,
        // whichever is nearer (see pick_color_under_cursor).
        if self.modifiers.control_key() || self.editor.tool == Tool::Eyedropper {
            self.pick_color_under_cursor();
            return;
        }
        // The Select tool: grabbing a selected voxel starts a move drag;
        // anything else settles any floating layer and starts a marquee.
        if self.editor.tool == Tool::Select {
            if self.try_begin_drag() {
                return;
            }
            self.commit_float();
            self.marquee = Some(Marquee {
                start: self.cursor,
                mode: sel_mode(self.modifiers),
            });
            return;
        }
        if self.editor.tool.is_continuous() {
            // Capture the pre-stroke rig; committed on end_paint iff the stroke
            // changed anything (rig mode only — None in plain model editing).
            self.editor.rig_pending = self.editor.rig_state();
            self.editor.document.begin_stroke();
            self.painting = true;
            self.last_paint = None;
            self.paint_step();
        } else if let Some(hit) = self.tool_pick() {
            let snap = self.editor.rig_state();
            if self.editor.apply(hit) {
                if let Some(s) = snap {
                    self.editor.rig_push_undo(s);
                }
            }
        }
    }

    /// Eyedropper (the tool, or a Ctrl+click from any tool): adopt the colour
    /// under the cursor — sampling whichever of the model voxel or the
    /// reference image is nearer along the ray, so you can pick colours
    /// straight off a reference (and the model still occludes it).
    fn pick_color_under_cursor(&mut self) {
        let voxel = self.pointer_pick().and_then(|h| {
            let c = self
                .editor
                .document
                .model()
                .get(h.voxel[0], h.voxel[1], h.voxel[2]);
            (c != 0).then_some((h.t, c))
        });
        let reference = self.reference_pick();
        let color = match (reference, voxel) {
            (Some((tr, cr)), Some((tv, cv))) => Some(if tr <= tv { cr } else { cv }),
            (Some((_, cr)), None) => Some(cr),
            (None, Some((_, cv))) => Some(cv),
            (None, None) => None,
        };
        if let Some(c) = color {
            self.editor.color = c;
        }
    }

    /// Sample the reference image under the cursor: intersect the cursor ray
    /// with the (visible) reference plane and, if it lands on an opaque texel
    /// within the image, return `(ray distance, 0x80RRGGBB colour)`.
    #[allow(clippy::many_single_char_names)] // ray/plane math: o, d, n, t, u, v
    fn reference_pick(&self) -> Option<(f64, u32)> {
        let r = self.editor.reference.as_ref()?;
        if !r.visible {
            return None;
        }
        let (o, d) = self.pointer_ray()?;
        let (origin, uf, vf, size) = r.placement(self.editor.document.pivot());
        let origin = [
            f64::from(origin[0]),
            f64::from(origin[1]),
            f64::from(origin[2]),
        ];
        let u = [f64::from(uf[0]), f64::from(uf[1]), f64::from(uf[2])];
        let v = [f64::from(vf[0]), f64::from(vf[1]), f64::from(vf[2])];
        let n = cross3(u, v); // plane normal (axis-aligned unit vector)
        let denom = dot3(d, n);
        if denom.abs() < 1e-9 {
            return None; // ray parallel to the plane
        }
        let rel0 = [origin[0] - o[0], origin[1] - o[1], origin[2] - o[2]];
        let t = dot3(rel0, n) / denom;
        if t <= 0.0 {
            return None; // plane is behind the camera
        }
        let hit = [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
        let rel = [hit[0] - origin[0], hit[1] - origin[1], hit[2] - origin[2]];
        // `u`/`v` are unit, sized 1 texel = 1 voxel, so the dot products are
        // the column/row directly (and already account for any flip).
        let (cu, cv) = (dot3(rel, u), dot3(rel, v));
        if cu < 0.0 || cu >= f64::from(size[0]) || cv < 0.0 || cv >= f64::from(size[1]) {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // bounds-checked above
        let color = r.texel(cu as u32, cv as u32)?;
        Some((t, color))
    }

    /// Apply the active tool at the current cursor, skipping the cell we
    /// last painted this stroke.
    #[allow(clippy::cast_possible_wrap)] // voxel coords are far below i32::MAX
    fn paint_step(&mut self) {
        let Some(hit) = self.pointer_pick() else {
            return;
        };
        let cell = if matches!(self.editor.tool, Tool::Place) {
            hit.place
        } else {
            [
                hit.voxel[0] as i32,
                hit.voxel[1] as i32,
                hit.voxel[2] as i32,
            ]
        };
        if self.last_paint == Some(cell) {
            return;
        }
        self.last_paint = Some(cell);
        self.editor.apply(hit);
    }

    /// Left-button release: end a move drag (the layer keeps floating
    /// until deselected), resolve a marquee, or close the drag-paint
    /// stroke (one undo step).
    fn end_paint(&mut self) {
        if self.bone_drag.take().is_some() {
            return; // bone repositioned
        }
        if self.pose_drag.take().is_some() {
            // A click that never moved leaves an uncommitted pre-edit snapshot;
            // drop it so the next unrelated edit doesn't absorb it. (A real
            // drag already committed it on its first change.)
            self.editor.rig_pending = None;
            return; // bone posed into the keyframe
        }
        if self.ref_drag.take().is_some() {
            return; // reference repositioned
        }
        if self.drag.take().is_some() {
            return; // the moved layer stays floating until deselect
        }
        if self.marquee.is_some() {
            self.finalize_marquee();
            return;
        }
        if self.painting {
            let changed = self.editor.document.end_stroke();
            // Commit the pre-stroke rig snapshot only if the stroke edited
            // something (drop it otherwise, so empty strokes add no undo step).
            if let Some(snap) = self.editor.rig_pending.take() {
                if changed {
                    self.editor.rig_push_undo(snap);
                }
            }
            self.painting = false;
            self.last_paint = None;
        }
    }

    /// Resolve a finished marquee drag into a selection change. A
    /// negligible drag is a single click (pick one voxel); a real drag
    /// uses the screen rectangle (select-through, ignores occlusion).
    fn finalize_marquee(&mut self) {
        let Some(m) = self.marquee.take() else {
            return;
        };
        let (sx, sy) = m.start;
        let (cx, cy) = self.cursor;
        let cells = if (cx - sx).abs() < 4.0 && (cy - sy).abs() < 4.0 {
            self.pointer_pick()
                .map(|h| vec![h.voxel])
                .unwrap_or_default()
        } else if let Some(window) = &self.window {
            let size = window.inner_size();
            let cam = self.camera.to_roxlap();
            let w = f64::from(size.width);
            // The pick is in the engine's (unflipped) screen space, so mirror
            // the rectangle's X when the viewport flip is on.
            let rect = if self.editor.flip_x {
                [
                    (w - m.start.0, m.start.1),
                    (w - self.cursor.0, self.cursor.1),
                ]
            } else {
                [m.start, self.cursor]
            };
            demiurg_view::marquee_voxels(
                self.editor.document.model(),
                &cam,
                w,
                f64::from(size.height),
                rect,
            )
        } else {
            Vec::new()
        };
        let sel = &mut self.editor.selection;
        match m.mode {
            SelMode::Replace => {
                sel.clear();
                sel.extend(cells);
            }
            SelMode::Add => sel.extend(cells),
            SelMode::Remove => {
                for c in &cells {
                    sel.remove(c);
                }
            }
        }
    }

    /// Delete the selection. A floating (pasted) layer is just discarded
    /// — it was never in the model. Otherwise the selected model voxels
    /// are cleared as one undo step.
    fn delete_selection(&mut self) {
        if self.editor.float.take().is_some() {
            self.editor.selection.clear();
            self.editor.dirty = true;
            return;
        }
        if self.editor.selection.is_empty() {
            return;
        }
        let cells: Vec<([u32; 3], u32)> = self.editor.selection.iter().map(|&c| (c, 0)).collect();
        let snap = self.editor.rig_state();
        if self.editor.document.set_cells(cells) {
            if let Some(s) = snap {
                self.editor.rig_push_undo(s);
            }
            self.editor.dirty = true;
        }
        self.editor.selection.clear();
    }

    /// Copy the occupied selected voxels to the clipboard at their
    /// absolute positions, so a paste lands back where they came from.
    /// Settles any floating layer first, so the copy reads real voxels.
    #[allow(clippy::cast_possible_wrap)] // voxel coords are far below i32::MAX
    fn copy_selection(&mut self) {
        self.commit_float();
        let model = self.editor.document.model();
        let occ: Vec<([i32; 3], u32)> = self
            .editor
            .selection
            .iter()
            .filter_map(|&c| {
                let col = model.get(c[0], c[1], c[2]);
                (col != 0).then_some(([c[0] as i32, c[1] as i32, c[2] as i32], col))
            })
            .collect();
        if !occ.is_empty() {
            self.editor.clipboard = occ;
        }
    }

    /// Paste the clipboard as a **floating layer** at its original
    /// positions (see [`FloatLayer`]): the voxels overlay the model but
    /// aren't written until the layer is deselected, so they can be moved
    /// without clobbering anything. Any existing float is settled first.
    fn paste_clipboard(&mut self) {
        if self.editor.clipboard.is_empty() {
            return;
        }
        self.commit_float();
        let cells = self.editor.clipboard.clone();
        self.editor.selection = float_selection(&cells, self.editor.document.dims());
        // A paste only adds voxels — nothing to clear on commit.
        self.editor.float = Some(FloatLayer {
            cells,
            lifted_from: Vec::new(),
        });
        self.editor.dirty = true; // rebuild the composite with the layer
    }

    /// Bake the floating layer into the model and drop it: clear the cells
    /// it was lifted from and write its in-bounds cells, all as one undo
    /// step (so a move is a single, reversible edit). No-op when nothing
    /// floats.
    fn commit_float(&mut self) {
        let Some(f) = self.editor.float.take() else {
            return;
        };
        // The composite changes either way (the layer is baked in or, if
        // fully out of bounds, simply removed), so a rebuild is due.
        self.editor.dirty = true;
        let dims = self.editor.document.dims();
        // Clears first, then writes — set_cells keeps the last value per
        // cell, so a moved voxel overwrites its own cleared source.
        let mut cells: Vec<([u32; 3], u32)> = f.lifted_from.iter().map(|&p| (p, 0)).collect();
        cells.extend(
            f.cells
                .iter()
                .filter_map(|(p, col)| in_bounds(*p, dims).map(|v| (v, *col))),
        );
        if !cells.is_empty() {
            // Baking a float (paste / moved selection) edits the mesh — record
            // the pre-bake rig for undo (no-op outside rig mode).
            let snap = self.editor.rig_state();
            if self.editor.document.set_cells(cells) {
                if let Some(s) = snap {
                    self.editor.rig_push_undo(s);
                }
            }
        }
    }

    /// Deselect: settle any floating layer into the model and clear the
    /// selection highlight and any in-progress drag.
    fn deselect(&mut self) {
        self.drag = None;
        self.commit_float();
        self.editor.selection.clear();
    }

    /// Drop selection cells that fell out of bounds (e.g. after a resize
    /// or an undo that shrank the model).
    fn prune_selection(&mut self) {
        let (dx, dy, dz) = self.editor.document.dims();
        self.editor
            .selection
            .retain(|c| c[0] < dx && c[1] < dy && c[2] < dz);
    }

    /// Refresh the window title: `demiurg — <name>`, with a trailing `*`
    /// while the document has unsaved changes. Only calls `set_title`
    /// when the text actually changes.
    fn update_title(&mut self) {
        let name = self
            .doc_name
            .clone()
            .unwrap_or_else(|| tr(self.editor.lang, Msg::Untitled).to_string());
        let star = if self.editor.document.is_modified() {
            " *"
        } else {
            ""
        };
        let title = format!("demiurg — {name}{star}");
        if self.last_title.as_deref() != Some(title.as_str()) {
            if let Some(window) = &self.window {
                window.set_title(&title);
            }
            self.last_title = Some(title);
        }
    }

    /// Quit, or — if there are unsaved changes — raise the in-app
    /// confirmation modal (shown by the UI next frame). A floating layer
    /// is settled first, so it counts as unsaved work.
    fn request_exit(&mut self, event_loop: &ActiveEventLoop) {
        self.commit_float();
        if self.editor.document.is_modified() {
            self.confirm_quit = true;
        } else {
            self.do_exit(event_loop);
        }
    }

    fn do_undo(&mut self) {
        if self.editor.rig.is_some() {
            self.rig_undo();
        } else if self.editor.document.undo() {
            self.editor.dirty = true;
        }
    }

    fn do_redo(&mut self) {
        if self.editor.rig.is_some() {
            self.rig_redo();
        } else if self.editor.document.redo() {
            self.editor.dirty = true;
        }
    }

    /// Undo the last rig edit: swap the current state onto the redo stack and
    /// restore the previous snapshot (rig + active bone), then refresh the
    /// working bone (Sculpt) / posed preview (Skeleton, Animate).
    fn rig_undo(&mut self) {
        let Some(prev) = self.editor.rig_undo.pop() else {
            return;
        };
        if let Some(cur) = self.editor.rig_state() {
            self.editor.rig_redo.push(cur);
        }
        self.restore_rig_snapshot(prev);
    }

    /// Redo the last undone rig edit (mirror of [`Self::rig_undo`]).
    fn rig_redo(&mut self) {
        let Some(next) = self.editor.rig_redo.pop() else {
            return;
        };
        if let Some(cur) = self.editor.rig_state() {
            self.editor.rig_undo.push(cur);
        }
        self.restore_rig_snapshot(next);
    }

    /// Install a rig snapshot as the live state and resync the view. Keeps the
    /// camera steady (an undo shouldn't jump the viewport).
    fn restore_rig_snapshot(&mut self, snap: RigSnapshot) {
        self.editor.rig = Some(snap.rig);
        self.editor.active_bone = snap.active_bone;
        self.editor.rig_dirty = true; // rebuild the posed preview if showing
        if self.editor.rig_mode == RigMode::Sculpt {
            self.load_active_bone(false);
        }
    }

    /// Build the egui frame and tessellate it (borrows only egui state +
    /// the editor, never the renderer).
    fn run_ui(
        &mut self,
        window: &Window,
        marquee: Option<[(f64, f64); 2]>,
    ) -> (
        Vec<egui::ClippedPrimitive>,
        egui::TexturesDelta,
        f32,
        UiActions,
    ) {
        let ctx = self.egui_ctx.clone();
        let raw = self
            .egui_state
            .as_mut()
            .expect("egui state")
            .take_egui_input(window);
        let modals = ui::Modals {
            quit_confirm: self.confirm_quit,
            saving: self.saving(),
            recovered: self.recovered,
        };
        let timeline = ui::Timeline {
            time: self.kfa.as_ref().map_or(0, KfaView::time),
        };
        let editor = &mut self.editor;
        let mut actions = UiActions::default();
        let out = ctx.run_ui(raw, |ui| {
            ui::build(ui, editor, &mut actions, modals, marquee, timeline);
        });
        self.egui_state
            .as_mut()
            .expect("egui state")
            .handle_platform_output(window, out.platform_output);
        let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
        (jobs, out.textures_delta, out.pixels_per_point, actions)
    }

    /// Run the deferred menu actions (file dialogs, undo/redo).
    #[allow(clippy::too_many_lines)] // a flat dispatch over the menu items
    fn apply_actions(&mut self, a: &UiActions) {
        if a.undo {
            self.do_undo();
        }
        if a.redo {
            self.do_redo();
        }
        if a.delete_sel {
            self.delete_selection();
        }
        if a.copy_sel {
            self.copy_selection();
        }
        if a.paste_sel {
            self.paste_clipboard();
        }
        if let Some(dir) = a.set_view {
            self.camera.set_view(dir);
        }
        if a.recovered_ok {
            self.recovered = false;
        }
        if a.new_model {
            self.load_model(new_model());
            self.doc_name = None;
        }
        if a.open_kv6 {
            self.open_dialog(DialogKind::OpenKv6);
        }
        if a.open_vox {
            self.open_dialog(DialogKind::OpenVox);
        }
        if a.open_project {
            self.open_dialog(DialogKind::OpenProject);
        }
        if a.open_reference {
            self.open_dialog(DialogKind::OpenReference);
        }
        if a.open_character {
            self.open_dialog(DialogKind::OpenCharacter);
        }
        if a.remove_reference {
            self.editor.reference = None;
            self.editor.ref_move_mode = false;
            self.editor.ref_image_dirty = true;
        }
        if a.save {
            self.save_project();
        }
        if a.save_as {
            self.save_to(SaveFormat::Project, true);
        }
        if a.export_kv6 {
            self.save_to(SaveFormat::Kv6, true);
        }
        if a.export_vxl {
            self.save_to(SaveFormat::Vxl, true);
        }
        if a.export_vox {
            self.save_to(SaveFormat::Vox, true);
        }
        if a.export_rkc {
            self.save_to(SaveFormat::Rkc, true);
        }
        if let Some(mode) = a.set_rig_mode {
            self.set_rig_mode(mode);
        }
        if a.toggle_play {
            self.editor.anim_playing = !self.editor.anim_playing;
        }
        if let Some(ms) = a.seek {
            // Scrubbing implies pause; the next redraw re-poses at the new time.
            self.editor.anim_playing = false;
            if let Some(kfa) = &mut self.kfa {
                kfa.set_time(ms);
            }
        }
        if let Some(i) = a.select_clip {
            self.select_clip(i);
        }
        if a.add_clip {
            self.add_clip();
        }
        if let Some(i) = a.delete_clip {
            self.delete_clip(i);
        }
        if let Some(sel) = a.select_key {
            self.set_selected_key(Some(sel));
        }
        if a.add_key {
            self.add_key();
        }
        if a.delete_key {
            self.delete_key();
        }
        if let Some(i) = a.select_bone {
            self.select_bone(i);
        }
        if a.add_bone {
            self.add_bone();
        }
        if a.add_axis_joint {
            self.add_axis_joint();
        }
        if a.add_dummy_root {
            self.add_dummy_root();
        }
        if let Some(i) = a.duplicate_bone {
            self.duplicate_bone(i);
        }
        if let Some((from, to)) = a.move_bone {
            self.move_bone(from, to);
        }
        if let Some(i) = a.delete_bone {
            self.delete_bone(i);
        }
        // Inline Skeleton-panel hinge edits: capture the pre-edit rig when an
        // interaction starts, commit it as one undo step when a value changes.
        if a.rig_edit_begin {
            self.editor.rig_begin_edit();
        }
        if a.rig_edit_changed {
            self.editor.rig_commit_pending();
        }
        if let Some((i, name)) = &a.rename_clip {
            self.rename_clip(*i, name.clone());
        }
        // Timeline drags (tick retime / angle edit). Undo is handled by the
        // begin/commit-pending pair above (one drag = one step), so these only
        // mutate; they must run *after* the commit so the captured snapshot is
        // the pre-edit rig.
        if let Some((k, ms)) = a.move_key {
            self.move_key(k, ms);
        }
        if let Some((k, bone, v)) = a.set_key_angle {
            self.set_key_angle(k, bone, v);
        }
        if let Some(axis) = a.set_bone_axis {
            self.set_bone_axis(axis);
        }
    }

    /// Set the active bone's rotation axis to a principal axis (one undo step).
    fn set_bone_axis(&mut self, axis: usize) {
        if self.editor.rig.is_none() || axis > 2 {
            return;
        }
        self.editor.rig_checkpoint();
        if let Some(rig) = self.editor.rig.as_mut() {
            if let Some(b) = rig.bones.get_mut(self.editor.active_bone) {
                let mut unit = [0.0f32; 3];
                unit[axis] = 1.0;
                b.hinge.v[0].x = unit[0];
                b.hinge.v[0].y = unit[1];
                b.hinge.v[0].z = unit[2];
                b.hinge.v[1] = b.hinge.v[0]; // mirror the axis to the parent side
            }
        }
        self.editor.rig_dirty = true;
    }

    /// `Ctrl+S` / Save: settle a float, then overwrite the known project
    /// path (background write) or pop a Save dialog.
    fn save_project(&mut self) {
        self.save_to(SaveFormat::Project, false);
    }

    /// Save `format`: overwrite the open project path directly when known
    /// and `ask` is false (Ctrl+S), else pick a path via a dialog (Save
    /// As / Export). Both the dialog and the write run off the event loop.
    fn save_to(&mut self, format: SaveFormat, ask: bool) {
        self.commit_float();
        if !ask {
            if let Some(path) = self.project_path.clone() {
                self.start_save(path, SaveFormat::Project, true);
                return;
            }
        }
        self.open_dialog(DialogKind::Save(format));
    }

    /// Launch a native file dialog on a worker thread (so it can't freeze
    /// the window), to be collected by [`poll_dialog`](Self::poll_dialog).
    fn open_dialog(&mut self, kind: DialogKind) {
        if self.busy() {
            return; // one dialog / user save at a time
        }
        // Default a project Save-As to the open file's folder + name.
        let (dir, name) = match (kind, &self.project_path) {
            (DialogKind::Save(SaveFormat::Project), Some(p)) => (
                p.parent().map(Path::to_path_buf),
                p.file_name().map(|n| n.to_string_lossy().into_owned()),
            ),
            _ => (None, None),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        #[cfg(not(target_os = "macos"))]
        std::thread::spawn(move || {
            let _ = tx.send(run_dialog(kind, dir, name.as_deref()));
        });
        // `AppKit` panels must run on the main thread; native macOS apps stay
        // responsive during their own modal, so the sync block is fine there.
        #[cfg(target_os = "macos")]
        {
            let _ = tx.send(run_dialog(kind, dir, name.as_deref()));
        }
        self.pending_dialog = Some(PendingDialog { rx, kind });
    }

    /// Collect a finished file dialog and act on the chosen path: load a
    /// model (open) or kick off a background save.
    fn poll_dialog(&mut self) {
        let path = match self.pending_dialog.as_ref().map(|d| d.rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => return, // none / still open
            Some(Ok(p)) => p,
            Some(Err(TryRecvError::Disconnected)) => None, // worker stopped
        };
        let kind = self.pending_dialog.take().expect("pending dialog").kind;
        let Some(path) = path else {
            return; // cancelled
        };
        match kind {
            DialogKind::OpenKv6 => {
                match std::fs::read(&path).map(|b| VoxelModel::from_kv6_bytes(&b)) {
                    Ok(Ok(m)) => {
                        self.load_model(m); // a .kv6 has no project path
                        self.doc_name = stem_of(&path);
                    }
                    Ok(Err(e)) => eprintln!("demiurg: {}: {e}", path.display()),
                    Err(e) => eprintln!("demiurg: read {}: {e}", path.display()),
                }
            }
            DialogKind::OpenVox => {
                match std::fs::read(&path).map(|b| VoxelModel::from_vox_bytes(&b)) {
                    Ok(Ok(m)) => {
                        self.load_model(m); // imported .vox has no project path
                        self.doc_name = stem_of(&path);
                    }
                    Ok(Err(e)) => eprintln!("demiurg: {}: {e}", path.display()),
                    Err(e) => eprintln!("demiurg: read {}: {e}", path.display()),
                }
            }
            DialogKind::OpenProject => {
                match std::fs::read(&path).map(|b| project::from_bytes(&b)) {
                    Ok(Ok(m)) => {
                        self.load_model(m);
                        self.doc_name = stem_of(&path);
                        self.project_path = Some(path); // Ctrl+S overwrites it
                    }
                    Ok(Err(e)) => eprintln!("demiurg: {}: {e}", path.display()),
                    Err(e) => eprintln!("demiurg: read {}: {e}", path.display()),
                }
            }
            DialogKind::OpenReference => self.load_reference(&path),
            DialogKind::OpenCharacter => self.load_character(&path),
            DialogKind::Save(format) => self.start_save(path, format, true),
        }
    }

    /// Spawn a background save of a snapshot of the current model. One at a
    /// time: if a save is already running this is dropped (a later save or
    /// autosave retries), so the worker count stays bounded.
    fn start_save(&mut self, path: PathBuf, format: SaveFormat, user: bool) {
        if self.pending_save.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let job_path = path.clone();
        if format == SaveFormat::Rkc {
            // Snapshot the rig (with the active bone's pending edits folded
            // in) and serialize off the event loop.
            self.commit_active_bone();
            let Some(rig) = self.editor.rig.clone() else {
                return; // not in rig mode
            };
            std::thread::spawn(move || {
                let _ = tx.send(std::fs::write(&job_path, rig.to_rkc_bytes()));
            });
        } else {
            let model = self.editor.document.model().clone();
            std::thread::spawn(move || {
                let _ = tx.send(std::fs::write(&job_path, format.encode(&model)));
            });
        }
        self.pending_save = Some(PendingSave {
            rx,
            path,
            format,
            user,
        });
    }

    /// Whether a *user* save is in flight (its modal blocks editing). An
    /// autosave runs silently and doesn't block.
    fn saving(&self) -> bool {
        self.pending_save.as_ref().is_some_and(|p| p.user)
    }

    /// Whether a modal operation (user save or open file dialog) is in
    /// flight, during which editing input is ignored.
    fn busy(&self) -> bool {
        self.saving() || self.pending_dialog.is_some()
    }

    /// Collect a finished background save: mark the project saved / adopt
    /// its path on success, or log the failure.
    fn poll_save(&mut self) {
        let result = match self.pending_save.as_ref().map(|p| p.rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => return, // none / still running
            Some(Ok(r)) => r,
            Some(Err(TryRecvError::Disconnected)) => {
                Err(std::io::Error::other("save worker stopped before writing"))
            }
        };
        let done = self.pending_save.take().expect("pending save");
        match (result, done.user) {
            (Ok(()), true) => {
                eprintln!("demiurg: saved {}", done.path.display());
                if done.format == SaveFormat::Project {
                    self.editor.document.mark_saved();
                    self.doc_name = stem_of(&done.path);
                    self.project_path = Some(done.path);
                }
            }
            (Ok(()), false) => {} // silent autosave success
            (Err(e), true) => eprintln!("demiurg: save {} failed: {e}", done.path.display()),
            (Err(e), false) => eprintln!("demiurg: autosave failed: {e}"),
        }
    }

    /// Write a background autosave if one is due and there are unsaved
    /// changes — a crash-recovery snapshot, never marked as "the save".
    fn maybe_autosave(&mut self) {
        if Instant::now() < self.next_autosave {
            return;
        }
        self.next_autosave = Instant::now() + AUTOSAVE_INTERVAL;
        // Skip while a save is in flight (start_save would drop it) or a
        // dialog is open (its result may be a save we mustn't pre-empt).
        if self.pending_save.is_some()
            || self.pending_dialog.is_some()
            || !self.editor.document.is_modified()
        {
            return;
        }
        let path = self.autosave_path.clone();
        self.start_save(path, SaveFormat::Project, false);
    }

    /// Drop the autosave file (a clean exit means the work is saved or
    /// deliberately discarded, so there's nothing to recover next time).
    fn clear_autosave(&self) {
        let _ = std::fs::remove_file(&self.autosave_path);
    }

    /// Exit cleanly: remove the crash-recovery autosave, then quit.
    fn do_exit(&self, event_loop: &ActiveEventLoop) {
        self.clear_autosave();
        event_loop.exit();
    }

    /// Read an image file and install it as the reference layer.
    fn load_reference(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(bytes) => {
                let name = stem_of(path).unwrap_or_else(|| "reference".to_string());
                self.editor.set_reference(&bytes, name);
            }
            Err(e) => eprintln!("demiurg: read {}: {e}", path.display()),
        }
    }

    /// Read a `.rkc` rigged-character file and open it for editing (rig
    /// mode), starting on the first bone.
    fn load_character(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(bytes) => match Rig::from_rkc_bytes(&bytes) {
                Ok(rig) if rig.bones.is_empty() => {
                    eprintln!("demiurg: {}: character has no bones", path.display());
                }
                Ok(rig) => {
                    self.enter_rig(rig);
                    self.doc_name = stem_of(path);
                }
                Err(e) => eprintln!("demiurg: {}: {e}", path.display()),
            },
            Err(e) => eprintln!("demiurg: read {}: {e}", path.display()),
        }
    }

    /// Route a dropped file: an image becomes the reference layer; an
    /// `.rkc` opens as the KFA rig; a `.kv6` / `.vox` / `.demiurg` opens as
    /// the model; anything else is ignored.
    fn on_dropped_file(&mut self, path: &Path) {
        if reference::is_image(path) {
            self.load_reference(path);
            return;
        }
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("rkc"))
        {
            self.load_character(path);
            return;
        }
        let read = || std::fs::read(path).map_err(|e| e.to_string());
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let parsed = match ext.as_deref() {
            Some("demiurg") => {
                read().and_then(|b| project::from_bytes(&b).map_err(|e| e.to_string()))
            }
            Some("vox") => {
                read().and_then(|b| VoxelModel::from_vox_bytes(&b).map_err(|e| e.to_string()))
            }
            Some("kv6") => {
                read().and_then(|b| VoxelModel::from_kv6_bytes(&b).map_err(|e| e.to_string()))
            }
            _ => {
                eprintln!("demiurg: ignored dropped file {}", path.display());
                return;
            }
        };
        match parsed {
            Ok(m) => {
                self.load_model(m);
                self.doc_name = stem_of(path);
                if ext.as_deref() == Some("demiurg") {
                    self.project_path = Some(path.to_path_buf());
                }
            }
            Err(e) => eprintln!("demiurg: {}: {e}", path.display()),
        }
    }

    /// Replace the document model, rebuild the sprite, refresh the
    /// palette, and reframe.
    fn load_model(&mut self, model: VoxelModel) {
        self.editor.rig = None; // leaving rig mode for a plain model
        self.editor.rig_mode = RigMode::Sculpt;
        self.kfa = None;
        self.editor.document.replace_model(model);
        self.view
            .set_model(self.editor.document.model(), self.editor.render_mode);
        self.editor.refresh_palette();
        self.editor.sync_resize_dims();
        self.editor.selection.clear();
        self.editor.float = None; // a loaded model starts with no float
        self.editor.reference = None; // a stale guide wouldn't align; drop it
        self.editor.ref_move_mode = false;
        self.editor.ref_image_dirty = true;
        self.drag = None;
        self.project_path = None; // callers re-set it for a `.demiurg` open
        self.camera = self.view.framing_camera();
        self.editor.dirty = false;
    }

    /// Enter rig-edit mode (Sculpt) on the first bone.
    fn enter_rig(&mut self, rig: Rig) {
        self.editor.rig = Some(rig);
        self.editor.active_bone = 0;
        self.editor.rig_mode = RigMode::Sculpt;
        self.editor.anim_playing = true;
        self.editor.active_clip = 0;
        self.editor.selected_key = None;
        self.editor.rig_undo.clear();
        self.editor.rig_redo.clear();
        self.editor.rig_pending = None;
        self.kfa = None;
        self.load_active_bone(true);
    }

    /// Switch the rig sub-mode, doing the scene swap each implies: Sculpt
    /// edits the active bone's mesh; Skeleton and Animate show the posed
    /// rig (rest pose / playing). No-op outside rig mode.
    fn set_rig_mode(&mut self, mode: RigMode) {
        if self.editor.rig.is_none() || self.editor.rig_mode == mode {
            return;
        }
        if self.editor.rig_mode == RigMode::Sculpt {
            self.commit_active_bone(); // fold the bone's edits back in
        }
        self.editor.rig_mode = mode;
        if mode == RigMode::Sculpt {
            self.kfa = None;
            self.load_active_bone(true); // restores the active bone + camera
        } else {
            self.rebuild_rig_preview();
            if let Some(kfa) = &self.kfa {
                self.camera = kfa.framing_camera();
            }
            // Empty the static scene so only the posed rig renders.
            self.view
                .set_model(&VoxelModel::new(1, 1, 1), self.editor.render_mode);
        }
    }

    /// Switch the previewed Animate clip. Re-bakes the new clip's `seq` /
    /// `frmval` into a fresh sprite (playhead reset to 0). No-op outside a
    /// rig or for an out-of-range index.
    fn select_clip(&mut self, i: usize) {
        let Some(rig) = &self.editor.rig else {
            return;
        };
        if i >= rig.clips.len() || i == self.editor.active_clip {
            return;
        }
        self.editor.active_clip = i;
        self.editor.selected_key = None; // selection is per-clip
        if self.editor.rig_mode == RigMode::Animate {
            self.rebuild_rig_preview(); // fresh sprite starts at time 0
        }
    }

    /// Append a new clip, make it active, and preview it. One undo step.
    fn add_clip(&mut self) {
        let snap = self.editor.rig_state();
        let idx = self.editor.rig.as_mut().map(|r| {
            let name = format!("clip {}", r.clips.len() + 1);
            r.add_clip(name)
        });
        if let Some(idx) = idx {
            if let Some(snap) = snap {
                self.editor.rig_push_undo(snap);
            }
            self.editor.active_clip = idx;
            self.editor.selected_key = None;
            if self.editor.rig_mode == RigMode::Animate {
                self.rebuild_rig_preview();
            }
        }
    }

    /// Rename clip `i`. Undo is the active begin/commit-pending step (a text
    /// edit); the rename doesn't change the baked pose, so no re-bake.
    fn rename_clip(&mut self, i: usize, name: String) {
        if let Some(r) = self.editor.rig.as_mut() {
            r.rename_clip(i, name);
        }
    }

    /// Delete clip `i`, clamping the active clip / clearing the selection. One
    /// undo step (on success).
    fn delete_clip(&mut self, i: usize) {
        let snap = self.editor.rig_state();
        let removed = self.editor.rig.as_mut().is_some_and(|r| r.remove_clip(i));
        if removed {
            if let Some(snap) = snap {
                self.editor.rig_push_undo(snap);
            }
            let n = self.editor.rig.as_ref().map_or(0, |r| r.clips.len());
            if self.editor.active_clip >= n {
                self.editor.active_clip = n.saturating_sub(1);
            }
            self.editor.selected_key = None;
            if self.editor.rig_mode == RigMode::Animate {
                self.rebuild_rig_preview();
            }
        }
    }

    /// (Re)build the posed-rig preview from the current rig — rest pose in
    /// Skeleton mode, the active clip in Animate. Keeps the camera.
    fn rebuild_rig_preview(&mut self) {
        let Some(rig) = &self.editor.rig else {
            return;
        };
        let clip = match self.editor.rig_mode {
            // Clamp the chosen clip to the current count (deletes / reorders
            // elsewhere can leave `active_clip` stale).
            RigMode::Animate if !rig.clips.is_empty() => {
                Some(self.editor.active_clip.min(rig.clips.len() - 1))
            }
            _ => None, // Skeleton (or no clips): rest pose
        };
        self.kfa = Some(KfaView::from_rig(rig.clone(), clip));
    }

    /// Re-bake the posed preview but keep the playhead where it was (a fresh
    /// sprite otherwise restarts at time 0). Used after an authoring edit / undo
    /// so the user stays parked on the key/time they were editing. In Skeleton
    /// mode (rest pose, time 0) this is a no-op beyond the rebuild.
    fn rebuild_rig_preview_keep_time(&mut self) {
        let t = self.kfa.as_ref().map(KfaView::time);
        self.rebuild_rig_preview();
        if let (Some(t), Some(kfa)) = (t, self.kfa.as_mut()) {
            kfa.set_time(t);
            kfa.advance(0); // re-pose in place at the preserved time
        }
    }

    /// Select a keyframe in the active clip (clamped to its key count; a stale
    /// index clears the selection). Pure view state, no rig change / undo.
    fn set_selected_key(&mut self, sel: Option<usize>) {
        let clip = self.editor.active_clip;
        let sel = sel.filter(|&k| {
            self.editor
                .rig
                .as_ref()
                .is_some_and(|r| k < r.clip_keyframes(clip).len())
        });
        self.editor.selected_key = sel;
        // Snap the playhead to the selected key's time and pause, so the
        // viewport shows exactly the pose being edited (posing rotates against
        // the shown pose; an interpolated in-between would be non-WYSIWYG).
        if let Some(k) = sel {
            let tim = self
                .editor
                .rig
                .as_ref()
                .and_then(|r| r.clip_keyframes(clip).get(k).map(|kf| kf.tim));
            if let (Some(tim), Some(kfa)) = (tim, self.kfa.as_mut()) {
                kfa.set_time(tim);
                self.editor.anim_playing = false;
                self.editor.rig_dirty = true; // re-pose the preview at the key
            }
        }
    }

    /// Add a keyframe at the playhead from the currently displayed pose
    /// ("key the current pose"), select it, and pause. One undo step (pushed
    /// only on success). Animate mode only.
    fn add_key(&mut self) {
        let Some(kfa) = self.kfa.as_ref() else {
            return;
        };
        if self.editor.rig_mode != RigMode::Animate {
            return;
        }
        let pose = kfa.pose_angles();
        let t = kfa.time();
        let clip = self.editor.active_clip;
        let snap = self.editor.rig_state();
        let idx = self
            .editor
            .rig
            .as_mut()
            .and_then(|r| r.add_keyframe(clip, t, pose));
        if let Some(idx) = idx {
            if let Some(snap) = snap {
                self.editor.rig_push_undo(snap);
            }
            self.editor.selected_key = Some(idx);
            self.editor.anim_playing = false;
            self.editor.rig_dirty = true;
        }
    }

    /// Delete the selected keyframe (one undo step, on success). Keeps the
    /// selection within the new key count (or clears it).
    fn delete_key(&mut self) {
        let Some(k) = self.editor.selected_key else {
            return;
        };
        let clip = self.editor.active_clip;
        let snap = self.editor.rig_state();
        let removed = self
            .editor
            .rig
            .as_mut()
            .is_some_and(|r| r.remove_keyframe(clip, k));
        if removed {
            if let Some(snap) = snap {
                self.editor.rig_push_undo(snap);
            }
            let n = self
                .editor
                .rig
                .as_ref()
                .map_or(0, |r| r.clip_keyframes(clip).len());
            self.editor.selected_key = (n > 0).then(|| k.min(n - 1));
            self.editor.rig_dirty = true;
        }
    }

    /// Retime keyframe `k` to `ms` (a tick drag). Undo is the active
    /// begin/commit-pending step, so this only mutates + follows the selection
    /// to the key's new index.
    fn move_key(&mut self, k: usize, ms: i32) {
        let clip = self.editor.active_clip;
        let new_idx = self
            .editor
            .rig
            .as_mut()
            .and_then(|r| r.move_keyframe(clip, k, ms));
        if let Some(new_idx) = new_idx {
            self.editor.selected_key = Some(new_idx);
            self.editor.anim_playing = false;
            self.editor.rig_dirty = true;
        }
    }

    /// Set bone `bone`'s angle in keyframe `k` (an angle-editor drag). Undo is
    /// the active begin/commit-pending step; this only mutates.
    fn set_key_angle(&mut self, k: usize, bone: usize, v: i16) {
        let clip = self.editor.active_clip;
        let changed = self
            .editor
            .rig
            .as_mut()
            .is_some_and(|r| r.set_keyframe_angle(clip, k, bone, v));
        if changed {
            self.editor.rig_dirty = true;
        }
    }

    /// Load the active bone's mesh into the document for editing. `reframe`
    /// re-centres the camera (on a bone switch); undo/redo pass `false` to
    /// keep the view steady. (Leaves `editor.rig` in place — only the working
    /// model changes.)
    fn load_active_bone(&mut self, reframe: bool) {
        let Some(rig) = &self.editor.rig else {
            return;
        };
        let model = rig.bones[self.editor.active_bone].model.clone();
        self.editor.document.replace_model(model);
        self.view
            .set_model(self.editor.document.model(), self.editor.render_mode);
        self.editor.refresh_palette();
        self.editor.sync_resize_dims();
        self.editor.selection.clear();
        self.editor.float = None;
        self.drag = None;
        if reframe {
            self.camera = self.view.framing_camera();
        }
        self.editor.dirty = false;
    }

    /// Write the document's working model back into the active bone, so the
    /// rig reflects the edits (called before switching bones or saving).
    fn commit_active_bone(&mut self) {
        if let Some(rig) = &mut self.editor.rig {
            if let Some(bone) = rig.bones.get_mut(self.editor.active_bone) {
                bone.model = self.editor.document.model().clone();
            }
        }
    }

    /// Switch which bone is active. In Sculpt it swaps the working mesh; in
    /// Skeleton / Animate it only records the choice (the preview is shared).
    fn select_bone(&mut self, i: usize) {
        if self.editor.rig.is_none() || i == self.editor.active_bone {
            return;
        }
        if self.editor.rig_mode != RigMode::Sculpt {
            self.editor.active_bone = i;
            return;
        }
        self.commit_active_bone();
        self.editor.active_bone = i;
        self.load_active_bone(true);
    }

    /// Add a new bone as a child of the active bone and make it active.
    /// No-op outside rig mode. In Sculpt the active bone's edits are folded
    /// back first, then the new (blank) bone is loaded for editing; in
    /// Skeleton / Animate the posed preview is rebuilt (via `rig_dirty`).
    fn add_bone(&mut self) {
        if self.editor.rig.is_none() {
            return;
        }
        self.editor.rig_checkpoint();
        if self.editor.rig_mode == RigMode::Sculpt {
            self.commit_active_bone();
        }
        let parent = i32::try_from(self.editor.active_bone).unwrap_or(-1);
        let rig = self.editor.rig.as_mut().expect("rig present");
        let new_idx = rig.add_bone(parent);
        self.editor.active_bone = new_idx;
        self.editor.rig_dirty = true;
        if self.editor.rig_mode == RigMode::Sculpt {
            self.load_active_bone(true);
        }
    }

    /// Add a 3-axis (ball) joint under the active bone and select its visible
    /// leaf. Same scene handling as [`Self::add_bone`].
    fn add_axis_joint(&mut self) {
        if self.editor.rig.is_none() {
            return;
        }
        self.editor.rig_checkpoint();
        if self.editor.rig_mode == RigMode::Sculpt {
            self.commit_active_bone();
        }
        let parent = i32::try_from(self.editor.active_bone).unwrap_or(-1);
        let leaf = self
            .editor
            .rig
            .as_mut()
            .expect("rig present")
            .add_axis_joint(parent);
        self.editor.active_bone = leaf;
        self.editor.rig_dirty = true;
        if self.editor.rig_mode == RigMode::Sculpt {
            self.load_active_bone(true);
        }
    }

    /// Wrap the rig's root in a dummy root so the old root becomes animatable.
    /// One undo step (on success); rebuilds the posed preview.
    fn add_dummy_root(&mut self) {
        let snap = self.editor.rig_state();
        let wrapped = self
            .editor
            .rig
            .as_mut()
            .and_then(Rig::add_dummy_root)
            .is_some();
        if wrapped {
            if let Some(snap) = snap {
                self.editor.rig_push_undo(snap);
            }
            self.editor.rig_dirty = true;
        }
    }

    /// Duplicate bone `i` (sibling copy of its mesh + hinge) and make the
    /// copy active. Mirrors [`Self::add_bone`]'s mode handling: commit the
    /// active mesh first in Sculpt, then load the copy for editing; otherwise
    /// the posed preview rebuilds via `rig_dirty`.
    fn duplicate_bone(&mut self, i: usize) {
        if self.editor.rig.is_none() {
            return;
        }
        let Some(snap) = self.editor.rig_state() else {
            return;
        };
        if self.editor.rig_mode == RigMode::Sculpt {
            self.commit_active_bone();
        }
        let rig = self.editor.rig.as_mut().expect("rig present");
        let Some(new_idx) = rig.duplicate_bone(i) else {
            return; // no-op: drop the snapshot, no checkpoint
        };
        self.editor.rig_push_undo(snap);
        self.editor.active_bone = new_idx;
        self.editor.rig_dirty = true;
        if self.editor.rig_mode == RigMode::Sculpt {
            self.load_active_bone(true);
        }
    }

    /// Delete bone `i`, keeping clips and parent indices consistent. No-op
    /// when the rig refuses it (last bone, or a root). Clamps the active
    /// bone, then reloads it (Sculpt) or rebuilds the preview (via
    /// `rig_dirty`).
    fn delete_bone(&mut self, i: usize) {
        let Some(snap) = self.editor.rig_state() else {
            return;
        };
        let Some(rig) = self.editor.rig.as_mut() else {
            return;
        };
        if !rig.delete_bone(i) {
            return; // refused (root / last bone): no checkpoint
        }
        self.editor.rig_push_undo(snap);
        let rig = self.editor.rig.as_mut().expect("rig present");
        let last = rig.bones.len() - 1;
        if self.editor.active_bone > last {
            self.editor.active_bone = last;
        } else if self.editor.active_bone > i {
            // A bone before the active one was removed — indices shifted down.
            self.editor.active_bone -= 1;
        }
        self.editor.rig_dirty = true;
        if self.editor.rig_mode == RigMode::Sculpt {
            self.load_active_bone(true);
        }
    }

    /// Move bone `from` to index `to` and keep `active_bone` pointing at the
    /// same bone. Only the ordering changes (not the meshes), so Sculpt needs
    /// no reload; the posed preview rebuilds via `rig_dirty`.
    fn move_bone(&mut self, from: usize, to: usize) {
        let Some(snap) = self.editor.rig_state() else {
            return;
        };
        let Some(rig) = self.editor.rig.as_mut() else {
            return;
        };
        if !rig.move_bone(from, to) {
            return; // no-op: drop the snapshot, no checkpoint
        }
        self.editor.rig_push_undo(snap);
        if self.editor.active_bone == from {
            self.editor.active_bone = to;
        }
        self.editor.rig_dirty = true;
    }

    #[allow(clippy::too_many_lines)] // the per-frame sequence reads better unsplit
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.step_camera();

        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let camera = self.camera.to_roxlap();

        // While dragging a marquee, the live screen rectangle (anchor ->
        // current cursor) is drawn by the UI as a 2D overlay.
        let marquee = self.marquee.as_ref().map(|m| [m.start, self.cursor]);
        let (jobs, textures, ppp, actions) = self.run_ui(&window, marquee);
        self.apply_actions(&actions);
        if actions.quit_confirm {
            self.do_exit(event_loop);
            return;
        }
        if actions.quit_cancel {
            self.confirm_quit = false;
        }
        // Off-loop I/O bookkeeping: collect a finished file dialog and any
        // finished save, then write a periodic autosave so a crash leaves
        // something to recover.
        self.poll_dialog();
        self.poll_save();
        self.maybe_autosave();
        // A tool switch (keyboard or a panel click) away from a floating
        // layer settles it into the model.
        if self.editor.tool != self.last_tool {
            self.commit_float();
            self.last_tool = self.editor.tool;
        }
        // In Animate mode the static scene is empty (the posed rig renders
        // via the KFA path), so don't repopulate it from the document.
        if self.editor.dirty && self.kfa.is_none() {
            // Render the document with the floating layer composited on
            // top (a borrow when nothing floats, a clone while it does).
            let mode = self.editor.render_mode;
            {
                let display = self.editor.display_model();
                self.view.set_model(&display, mode);
            }
            self.editor.refresh_palette();
            self.prune_selection();
            self.editor.dirty = false;
        }
        // A skeleton edit (Skeleton mode) changed the rig — rebuild the
        // posed preview so the rest pose reflects it.
        if self.editor.rig_dirty {
            if self.kfa.is_some() {
                self.rebuild_rig_preview_keep_time();
            }
            self.editor.rig_dirty = false;
        }
        self.update_title();

        let mut settings = OpticastSettings::for_oracle_framebuffer(size.width, size.height);
        // Live ray-plane density (CPU backend); `[` / `]` adjust it.
        settings.anginc = self.editor.anginc;
        // Lit (lightmode 1) by default for directional shading; the View
        // menu can switch it to flat (lightmode 0). `R==G==B` material
        // takes the cheap shading path either way.
        let sprite_lighting = SpriteLighting {
            kv6col: SPRITE_MATERIAL,
            lightmode: u32::from(self.editor.lighting),
            lights: &[],
        };
        // Per-face shading applies to the voxel-grid render only (the
        // sprite path shades per voxel, not per face).
        let side_shades = match self.editor.render_mode {
            RenderMode::Voxel if self.editor.lighting => VOXEL_SIDE_SHADES,
            _ => [0; 6],
        };
        let frame = FrameParams {
            settings: &settings,
            sky_color: SKY_COLOR,
            sky: None,
            fog_color: 0,
            fog_max_scan_dist: 0,
            treat_z_max_as_air: true,
            gpu_mip_scan_dist: 64.0,
            gpu_max_outer_steps: 64,
            gpu_fov_y_rad: 1.2,
            sprite_lighting: Some(&sprite_lighting),
            side_shades,
        };

        // Advance the KFA rig's animation (and re-solve its bones) before
        // building gizmo lines, so the skeleton overlay tracks the pose.
        // Gate the *time delta*, not the call: the per-frame solve must keep
        // running every frame (it poses the limbs + skeleton gizmo), but time
        // only advances while playing in Animate mode. With `dt == 0`,
        // `advance` re-poses in place at the current playhead — so a paused or
        // freshly-scrubbed frame still renders correctly.
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        if let Some(kfa) = &mut self.kfa {
            let dt = if self.editor.rig_mode == RigMode::Animate && self.editor.anim_playing {
                FRAME_DT.as_millis() as i32
            } else {
                0
            };
            kfa.advance(dt);
        }

        // Editor lines (uses the pick ray from the last frame's
        // projection — fine at redraw cadence). Built before the mutable
        // renderer borrow.
        let lines = self.scene_lines();

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        // Keep the reference sprite texture in sync with the loaded image
        // (re-uploaded only on load / replace / remove, not on every move).
        if self.editor.ref_image_dirty {
            if let Some(id) = self.ref_image.take() {
                renderer.drop_image(id);
            }
            if let Some(r) = &self.editor.reference {
                self.ref_image = Some(renderer.upload_image(r.rgba(), r.width, r.height));
            }
            self.editor.ref_image_dirty = false;
        }
        renderer.set_flip_x(self.editor.flip_x);
        renderer.set_sprites(self.view.sprites());
        // The KFA rig's limb sprites. set_sprites resets the registry each
        // frame, so re-establish the rig after it, then apply the current
        // pose — or clear it (empty set) when there's no preview, so leaving
        // Animate mode doesn't leave the posed rig drawn over the editor.
        // (Re-establishing every frame is wasteful on GPU; a later pass can
        // do set_kfa_sprites only when the set changes.)
        match &mut self.kfa {
            Some(kfa) => {
                renderer.set_kfa_sprites(kfa.kfas_mut());
                renderer.update_kfa_poses(kfa.kfas_mut());
            }
            None => renderer.set_kfa_sprites(&mut []),
        }
        renderer.render(self.view.scene_mut(), &camera, &frame);
        // Depth-tested editor gizmos land in the framebuffer; paint_egui
        // then draws the panels on top.
        renderer.draw_lines(&camera, &lines);
        // The reference image as a flat, depth-tested world sprite: the model
        // occludes the parts behind it, and it stays undistorted from any
        // angle. Drawn after the scene/gizmos, before the egui panels.
        if let (Some(image), Some(r)) = (self.ref_image, self.editor.reference.as_ref()) {
            if r.visible {
                let (origin, u, v, size) = r.placement(self.editor.document.pivot());
                // The tint's high byte scales texel alpha — the opacity slider.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let alpha = (r.opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
                renderer.draw_images(
                    &camera,
                    &[ImageSprite {
                        image,
                        origin,
                        facing: ImageFacing::World { u, v },
                        size,
                        tint: (alpha << 24) | 0x00FF_FFFF,
                        alpha_cutoff: 0.0, // blend all texels (current reference behaviour)
                        depth_test: true,
                        double_sided: true,
                    }],
                );
            }
        }
        renderer.paint_egui(&jobs, &textures, ppp);
        // Next redraw is scheduled by `about_to_wait` at the frame cap.
    }

    /// Dispatch a key press/release (camera holds, tool hotkeys, undo).
    fn on_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, pressed: bool) {
        let ctrl = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();
        if self.busy() {
            return; // keys are blocked behind the saving / dialog modal
        }
        match code {
            KeyCode::Digit1 if pressed => self.editor.tool = Tool::Place,
            KeyCode::Digit2 if pressed => self.editor.tool = Tool::Erase,
            KeyCode::Digit3 if pressed => self.editor.tool = Tool::Paint,
            KeyCode::Digit4 if pressed => self.editor.tool = Tool::Eyedropper,
            KeyCode::Digit5 if pressed => self.editor.tool = Tool::Box,
            KeyCode::Digit6 if pressed => self.editor.tool = Tool::Sphere,
            KeyCode::Digit7 if pressed => self.editor.tool = Tool::Fill,
            KeyCode::Digit8 if pressed => self.editor.tool = Tool::Select,
            KeyCode::KeyZ if pressed && ctrl && !shift => self.do_undo(),
            KeyCode::KeyZ if pressed && ctrl && shift => self.do_redo(),
            KeyCode::KeyY if pressed && ctrl => self.do_redo(),
            KeyCode::KeyC if pressed && ctrl => self.copy_selection(),
            KeyCode::KeyV if pressed && ctrl => self.paste_clipboard(),
            // Ctrl+S overwrites the project file (S without Ctrl zooms out).
            KeyCode::KeyS if ctrl && pressed => self.save_project(),
            KeyCode::Delete | KeyCode::Backspace if pressed => self.delete_selection(),
            // Diagnostic: dump the current camera as `--shot` flags so a
            // problematic angle can be reproduced headlessly. Paste the line
            // after `demiurg <model> --shot out.png`.
            KeyCode::KeyP if pressed => {
                let c = &self.camera;
                eprintln!(
                    "demiurg camera: --cx {:.4} --cy {:.4} --cz {:.4} --yaw {:.5} --pitch {:.5} --dist {:.3}",
                    c.center.x, c.center.y, c.center.z, c.yaw, c.pitch, c.dist
                );
            }
            // Ray-plane density (CPU backend, voxlap `anginc`). `]` adds ray
            // planes (anginc /= sqrt2, supersamples the angular fan), `[`
            // removes them (anginc *= sqrt2, coarsens). Clamped to a sane
            // range; thin-geometry silhouette artifacts shrink as planes rise.
            KeyCode::BracketRight if pressed => {
                self.editor.anginc = (self.editor.anginc / std::f32::consts::SQRT_2).max(0.125);
                eprintln!(
                    "demiurg: ray planes anginc={:.4} ({:.2}x baseline)",
                    self.editor.anginc,
                    1.0 / self.editor.anginc
                );
            }
            KeyCode::BracketLeft if pressed => {
                self.editor.anginc = (self.editor.anginc * std::f32::consts::SQRT_2).min(16.0);
                eprintln!(
                    "demiurg: ray planes anginc={:.4} ({:.2}x baseline)",
                    self.editor.anginc,
                    1.0 / self.editor.anginc
                );
            }
            KeyCode::ArrowLeft => self.keys.left = pressed,
            KeyCode::ArrowRight => self.keys.right = pressed,
            KeyCode::ArrowUp => self.keys.up = pressed,
            KeyCode::ArrowDown => self.keys.down = pressed,
            KeyCode::KeyW => self.keys.zoom_in = pressed,
            KeyCode::KeyS => self.keys.zoom_out = pressed,
            KeyCode::Home if pressed => self.camera.recenter(), // undo a pan
            // Axis views on the numpad (Blender-style); Ctrl flips to the
            // opposite face.
            KeyCode::Numpad1 if pressed => {
                self.camera
                    .set_view(if ctrl { ViewDir::Back } else { ViewDir::Front });
            }
            KeyCode::Numpad3 if pressed => {
                self.camera
                    .set_view(if ctrl { ViewDir::Left } else { ViewDir::Right });
            }
            KeyCode::Numpad7 if pressed => {
                self.camera
                    .set_view(if ctrl { ViewDir::Bottom } else { ViewDir::Top });
            }
            KeyCode::Escape if pressed => {
                if self.confirm_quit {
                    self.confirm_quit = false; // Esc dismisses the modal
                } else if self.editor.float.is_some() || !self.editor.selection.is_empty() {
                    self.deselect(); // Esc first settles a float / clears selection
                } else {
                    self.request_exit(event_loop);
                }
            }
            _ => {}
        }
    }
}

/// A file's stem as an owned `String`, for the window title.
fn stem_of(path: &std::path::Path) -> Option<String> {
    path.file_stem().map(|s| s.to_string_lossy().into_owned())
}

impl ApplicationHandler for App {
    #[allow(clippy::cast_possible_truncation)] // scale_factor f64->f32 is exact in practice
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("demiurg")
            .with_inner_size(LogicalSize::new(1100.0, 760.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("winit: create_window"),
        );

        // CPU renderer by default — it's reliable everywhere. The GPU
        // backend is opt-in (`--gpu` or `ROXLAP_GPU=1`): it's faster, but
        // its device creation can *hang* on some Windows GPUs/drivers (a
        // white frozen window, the OS offering to kill the app), and a
        // synchronous hang can't be timed out, so it isn't the default.
        let want_gpu = if self.force_cpu {
            false
        } else if self.force_gpu {
            true
        } else {
            std::env::var("ROXLAP_GPU").is_ok_and(|v| v == "1")
        };
        let mut opts = RenderOptions {
            want_gpu,
            // The empty (sprite-only) scene's background comes from the
            // construction-time clear colour, so set it here too — not
            // just FrameParams.sky_color (which feeds sky-miss / GPU).
            clear_sky: SKY_COLOR,
            ..RenderOptions::default()
        };
        // Present uncapped (no forced vsync). The ~60 fps `about_to_wait`
        // frame timer already limits how often we render, so the GPU isn't
        // overworked, and this avoids Fifo/vsync present stalls.
        opts.gpu.uncapped_present = true;
        let size = window.inner_size();
        // Logged before creation so a hang here is visible in the console
        // (it pins the freeze to GPU device init).
        eprintln!("demiurg: creating renderer (gpu={want_gpu})...");
        let renderer = SceneRenderer::new(window.clone(), (size.width, size.height), &opts);
        match renderer.adapter_info() {
            Some(info) => eprintln!("demiurg: GPU backend - {info}"),
            None => eprintln!("demiurg: CPU backend"),
        }

        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        ));

        self.renderer = Some(renderer);
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Let egui see the event first; `consumed` means a widget took it
        // (a click on a panel), so we skip camera / editing.
        let consumed = match (self.window.clone(), self.egui_state.as_mut()) {
            (Some(window), Some(state)) => state.on_window_event(&window, &event).consumed,
            _ => false,
        };

        match event {
            WindowEvent::CloseRequested => self.request_exit(event_loop),
            // Drag-and-drop: an image becomes the reference layer; a model
            // file (.kv6/.vox/.demiurg) opens as the model.
            WindowEvent::DroppedFile(path) => self.on_dropped_file(&path),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        ..
                    },
                ..
            } if !consumed => self.on_key(event_loop, code, state == ElementState::Pressed),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !consumed => self.begin_paint(),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.end_paint(),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Right,
                ..
            } => {
                // Right drag orbits, or pans while Shift is held (a pan
                // path for setups without a middle mouse button).
                if state == ElementState::Pressed {
                    if self.modifiers.shift_key() {
                        self.panning = true;
                    } else {
                        self.orbiting = true;
                    }
                } else {
                    self.orbiting = false;
                    self.panning = false;
                    self.last_drag = None;
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Middle,
                ..
            } => {
                self.panning = state == ElementState::Pressed;
                if !self.panning {
                    self.last_drag = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                if self.bone_drag.is_some() {
                    self.update_bone_drag();
                }
                if self.pose_drag.is_some() {
                    self.update_pose_drag();
                }
                if self.ref_drag.is_some() {
                    self.update_ref_drag();
                }
                if self.drag.is_some() {
                    self.update_drag();
                }
                if self.painting {
                    self.paint_step();
                }
                // A mirrored viewport flips the horizontal drag sense, so the
                // camera rotates / pans the way the cursor moves on screen.
                let sx = if self.editor.flip_x { -1.0 } else { 1.0 };
                if self.orbiting {
                    if let Some((lx, ly)) = self.last_drag {
                        self.camera.orbit(
                            sx * (position.x - lx) * 0.01,
                            -(position.y - ly) * 0.01,
                            0.0,
                        );
                    }
                    self.last_drag = Some((position.x, position.y));
                } else if self.panning {
                    if let Some((lx, ly)) = self.last_drag {
                        self.pan_camera(sx * (position.x - lx), position.y - ly);
                    }
                    self.last_drag = Some((position.x, position.y));
                }
            }
            WindowEvent::MouseWheel { delta, .. } if !consumed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                self.camera.orbit(0.0, 0.0, -lines * self.camera.dist * 0.1);
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    /// Drive the redraw loop at ~60 fps: request a frame when due, then
    /// sleep until the next one (or until an input event wakes us).
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_frame {
            self.next_frame = now + FRAME_DT;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_selection_keeps_only_in_bounds_cells() {
        let cells = vec![
            ([0, 0, 0], 0x80ff_0000),
            ([5, 5, 5], 0x8000_ff00),  // out of a 4^3 model
            ([-1, 0, 0], 0x8000_00ff), // negative
        ];
        let sel = float_selection(&cells, (4, 4, 4));
        assert_eq!(sel.len(), 1, "only the in-bounds cell is selectable");
        assert!(sel.contains(&[0, 0, 0]));
    }

    #[test]
    fn display_model_overlays_the_float_but_leaves_the_document_clean() {
        let mut ed = Editor::new(VoxelModel::new(4, 4, 4));
        ed.float = Some(FloatLayer {
            cells: vec![([1, 1, 1], 0x80ff_0000), ([9, 9, 9], 0x8000_ff00)],
            lifted_from: Vec::new(),
        });
        let disp = ed.display_model();
        assert_eq!(disp.get(1, 1, 1), 0x80ff_0000, "in-bounds float shows");
        assert_eq!(disp.get(9, 9, 9), 0, "out-of-bounds float cell is skipped");
        assert_eq!(
            ed.document.model().get(1, 1, 1),
            0,
            "the document model is not mutated by display"
        );
    }

    #[test]
    fn plane_drag_delta_snaps_in_plane_and_locks_the_axis() {
        // Plane perpendicular to z (axis 2) at z=0; a ray straight down +z
        // from (3.4, -1.6, -5) meets it at (3.4, -1.6, 0).
        let delta = plane_drag_delta([3.4, -1.6, -5.0], [0.0, 0.0, 1.0], 2, 0.0, [0.0; 3]);
        assert_eq!(delta, Some([3, -2, 0]), "x/y round, z is locked to 0");

        // A ray parallel to the plane never meets it.
        assert_eq!(
            plane_drag_delta([0.0; 3], [1.0, 0.0, 0.0], 2, 5.0, [0.0; 3]),
            None
        );
        // A plane behind the camera (negative t) is rejected.
        assert_eq!(
            plane_drag_delta([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 2, -3.0, [0.0; 3]),
            None
        );
    }

    #[test]
    fn plane_basis_is_orthonormal_and_perpendicular_to_the_axis() {
        for n in [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.6, -0.8, 0.0]] {
            let (u, w) = plane_basis(n);
            assert!((dot3(u, u) - 1.0).abs() < 1e-9, "u is unit");
            assert!((dot3(w, w) - 1.0).abs() < 1e-9, "w is unit");
            assert!(dot3(u, n).abs() < 1e-9, "u ⟂ n");
            assert!(dot3(w, n).abs() < 1e-9, "w ⟂ n");
            assert!(dot3(u, w).abs() < 1e-9, "u ⟂ w");
        }
    }

    #[test]
    fn point_seg_dist_2d_measures_to_the_nearest_point_on_the_segment() {
        // Perpendicular distance to the segment's interior.
        assert!((point_seg_dist_2d([5.0, 3.0], [0.0, 0.0], [10.0, 0.0]) - 3.0).abs() < 1e-9);
        // Past an endpoint: clamps to the endpoint distance, not the line.
        assert!((point_seg_dist_2d([13.0, 4.0], [0.0, 0.0], [10.0, 0.0]) - 5.0).abs() < 1e-9);
        // A zero-length segment is just point-to-point.
        assert!((point_seg_dist_2d([3.0, 4.0], [0.0, 0.0], [0.0, 0.0]) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn hinge_sweep_is_signed_and_in_hinge_units() {
        let z = [0.0, 0.0, 1.0];
        // ref0 = +x; a quarter turn to +y is +90° = a quarter of 65536.
        let q = hinge_sweep(z, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert!(
            (q - 16384.0).abs() < 1.0,
            "+90deg about +z is +16384, got {q}"
        );
        // The same sweep about -z is the opposite sign (right-handed).
        let qn = hinge_sweep([0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        assert!(
            (qn + 16384.0).abs() < 1.0,
            "flipping the axis flips the sign"
        );
        // A turn the other way (+x -> -y) is -90deg; magnitude is scale-free.
        let h = hinge_sweep(z, [2.0, 0.0, 0.0], [0.0, -5.0, 0.0]).unwrap();
        assert!(
            (h + 16384.0).abs() < 1.0,
            "-90deg is -16384 regardless of length"
        );
        // Cursor on the pivot (a ~zero vector) yields no angle.
        assert_eq!(hinge_sweep(z, [1.0, 0.0, 0.0], [0.0; 3]), None);
    }

    #[test]
    fn floor_cell_maps_a_down_ray_to_the_bottom_layer() {
        // 8^3 model, pivot at the centre: the floor plane is voxel z = 8,
        // i.e. world z = 8 - 4 = 4. A ray straight down (+z) through world
        // (-0.5, 1.5) lands in voxel column (3, 5) -> cell (3, 5, 7).
        let dims = (8, 8, 8);
        let pivot = [4.0, 4.0, 4.0];
        let cell = floor_cell([-0.5, 1.5, -10.0], [0.0, 0.0, 1.0], pivot, dims);
        assert_eq!(cell, Some([3, 5, 7]), "bottom layer is z = dz - 1");

        // Parallel to the floor -> no hit.
        assert_eq!(
            floor_cell([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], pivot, dims),
            None
        );
        // Outside the footprint -> no hit.
        assert_eq!(
            floor_cell([20.0, 0.0, -10.0], [0.0, 0.0, 1.0], pivot, dims),
            None
        );
    }

    #[test]
    fn display_model_leaves_a_hole_where_a_moved_layer_was_lifted() {
        let mut m = VoxelModel::new(4, 4, 4);
        m.set(0, 0, 0, 0x80ff_0000);
        let mut ed = Editor::new(m);
        // Simulate a move of (0,0,0) -> (2,2,2): the layer carries the
        // voxel and remembers its source cell to clear.
        ed.float = Some(FloatLayer {
            cells: vec![([2, 2, 2], 0x80ff_0000)],
            lifted_from: vec![[0, 0, 0]],
        });
        let disp = ed.display_model();
        assert_eq!(
            disp.get(2, 2, 2),
            0x80ff_0000,
            "voxel shows at the new spot"
        );
        assert_eq!(disp.get(0, 0, 0), 0, "and the source cell reads empty");
        assert_eq!(
            ed.document.model().get(0, 0, 0),
            0x80ff_0000,
            "the document still holds the original until commit"
        );
    }
}
