//! demiurg native editor (M2): open a window, load/edit a `.kv6` voxel
//! model, and see it rendered by roxlap's own renderer. An egui overlay
//! provides the tools, palette, mirror, pivot, and file menu.
//!
//! Usage:
//!   demiurg [path.kv6 | path.demiurg]   # no path -> a blank canvas
//!
//! Controls: left mouse applies the active tool (hold to drag-paint);
//! with the Select tool, dragging a selected voxel moves the selection in
//! that face's plane (it floats until deselected); `Ctrl`+click eyedrops a
//! colour; right-mouse drag orbits; wheel and `W`/`S` zoom; arrow keys
//! orbit. Hotkeys: `1`-`8` pick a tool (`8` is
//! Select), `Ctrl+Z` undo, `Ctrl+Y` / `Ctrl+Shift+Z` redo, `Ctrl+C`
//! copies the selection and `Ctrl+V` pastes it as a floating layer at its
//! original position (settled into the model on deselect), `Delete`
//! removes the selection, `Esc` deselects (settling any pasted layer) or
//! else quits. `DEMIURG_LANG=ru` starts in Russian. The GPU backend is
//! used by default; `ROXLAP_GPU=0` forces the CPU renderer.

mod ui;

use std::borrow::Cow;
use std::collections::HashSet;
use std::process::exit;
use std::sync::Arc;
use std::time::{Duration, Instant};

use demiurg_core::{Document, VoxelModel, project};
use demiurg_i18n::{Lang, Msg, tr};
use demiurg_view::{Line3, ModelView, OrbitCamera, PickHit, RenderMode, pick_voxel};
use roxlap_core::opticast::OpticastSettings;
use roxlap_core::sprite::SpriteLighting;
use roxlap_render::{FrameParams, RenderOptions, SceneRenderer, egui};
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

/// The in-bounds cells of a float layer as a selection set (for the
/// highlight while it floats).
fn float_selection(cells: &[([i32; 3], u32)], dims: (u32, u32, u32)) -> HashSet<[u32; 3]> {
    cells
        .iter()
        .filter_map(|(p, _)| in_bounds(*p, dims))
        .collect()
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
    /// The viewport scene needs a rebuild from the model.
    dirty: bool,
}

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
            render_mode: DEFAULT_RENDER_MODE,
            resize_dims: [dx, dy, dz],
            selection: HashSet::new(),
            clipboard: Vec::new(),
            float: None,
            dirty: false,
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
    fn apply(&mut self, hit: PickHit) {
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
    }
}

fn main() {
    let path = std::env::args().nth(1);
    let model = if let Some(p) = &path {
        load_any(p)
    } else {
        eprintln!("demiurg: blank canvas (pass a .kv6 or .demiurg path to open one)");
        new_model()
    };

    let view = ModelView::new(&model, DEFAULT_RENDER_MODE);
    let camera = view.framing_camera();
    let doc_name = path
        .as_deref()
        .and_then(|p| stem_of(std::path::Path::new(p)));
    let mut app = App {
        window: None,
        renderer: None,
        view,
        camera,
        editor: Editor::new(model),
        egui_ctx: egui::Context::default(),
        egui_state: None,
        keys: Keys::default(),
        modifiers: ModifiersState::empty(),
        orbiting: false,
        painting: false,
        last_paint: None,
        cursor: (0.0, 0.0),
        last_drag: None,
        doc_name,
        last_title: None,
        confirm_quit: false,
        marquee: None,
        drag: None,
        last_tool: Tool::Place,
        next_frame: Instant::now(),
    };

    let event_loop = EventLoop::new().expect("winit: create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app).expect("winit: run_app");
}

/// Load a `.kv6` or `.demiurg` by extension, or exit with a message.
fn load_any(path: &str) -> VoxelModel {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("demiurg: cannot read {path}: {e}");
        exit(2);
    });
    let model = if path.ends_with(".demiurg") {
        project::from_bytes(&bytes).map_err(|e| e.to_string())
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
    /// Left mouse held with a continuous tool: an open drag-paint stroke.
    painting: bool,
    /// Last cell painted this stroke, to skip redundant re-applies.
    last_paint: Option<[i32; 3]>,
    cursor: (f64, f64),
    last_drag: Option<(f64, f64)>,
    /// File stem of the open document (for the title), or `None` if new.
    doc_name: Option<String>,
    /// Last window title set, to avoid redundant `set_title` calls.
    last_title: Option<String>,
    /// The unsaved-changes quit modal is showing.
    confirm_quit: bool,
    /// An in-progress selection marquee drag (Select tool), else `None`.
    marquee: Option<Marquee>,
    /// An in-progress move drag of the floating layer, else `None`.
    drag: Option<DragMove>,
    /// The active tool last frame, to detect a tool switch (keyboard or
    /// UI) and settle a floating layer when the user leaves Select.
    last_tool: Tool,
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

    /// The cell the active tool would affect under the cursor (place
    /// target for Place, hit voxel otherwise), or `None` over a panel /
    /// on a miss.
    #[allow(clippy::cast_possible_wrap)] // voxel coords are far below i32::MAX
    fn hover_cell(&self) -> Option<[i32; 3]> {
        if self.egui_ctx.is_pointer_over_egui() {
            return None;
        }
        let hit = self.pointer_pick()?;
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
        if self.editor.show_grid {
            lines.extend(demiurg_view::reference_lines_3d(
                pivot,
                self.editor.document.dims(),
            ));
        }
        if !self.editor.selection.is_empty() {
            let cells: Vec<[u32; 3]> = self.editor.selection.iter().copied().collect();
            lines.extend(demiurg_view::selection_lines_3d(pivot, &cells));
        }
        // No hover box mid-drag: the selection outline already tracks the
        // moving layer, and the hover would pick the model under it.
        if self.drag.is_none() {
            if let Some(cell) = self.hover_cell() {
                lines.extend(demiurg_view::voxel_box_lines_3d(pivot, cell));
            }
        }
        lines
    }

    /// The voxel under the cursor, if any.
    fn pointer_pick(&self) -> Option<PickHit> {
        let cam = self.camera.to_roxlap();
        let ray = self
            .renderer
            .as_ref()?
            .view_ray(&cam, self.cursor.0, self.cursor.1)?;
        pick_voxel(self.editor.document.model(), ray.origin, ray.dir)
    }

    /// The cursor ray as world `(origin, dir)` component arrays.
    fn pointer_ray(&self) -> Option<([f64; 3], [f64; 3])> {
        let cam = self.camera.to_roxlap();
        let r = self
            .renderer
            .as_ref()?
            .view_ray(&cam, self.cursor.0, self.cursor.1)?;
        Some((
            [r.origin.x, r.origin.y, r.origin.z],
            [r.dir.x, r.dir.y, r.dir.z],
        ))
    }

    /// Pick against the **composite** (model + floating layer), so a
    /// floating voxel can be grabbed even though it isn't in the document.
    fn grab_pick(&self) -> Option<PickHit> {
        let cam = self.camera.to_roxlap();
        let ray = self
            .renderer
            .as_ref()?
            .view_ray(&cam, self.cursor.0, self.cursor.1)?;
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

    /// Left-button press: a quick eyedropper (Ctrl), a selection marquee
    /// (Select tool), a drag-paint stroke (continuous tools), or a
    /// click-once tool.
    fn begin_paint(&mut self) {
        if self.confirm_quit {
            return; // don't edit behind the quit modal
        }
        // Ctrl+click is a quick eyedropper, whatever the active tool.
        if self.modifiers.control_key() {
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
            self.editor.document.begin_stroke();
            self.painting = true;
            self.last_paint = None;
            self.paint_step();
        } else if let Some(hit) = self.pointer_pick() {
            self.editor.apply(hit);
        }
    }

    /// Ctrl+click eyedropper: adopt the colour of the voxel under the
    /// cursor (ignores empty space).
    fn pick_color_under_cursor(&mut self) {
        if let Some(hit) = self.pointer_pick() {
            let c = self
                .editor
                .document
                .model()
                .get(hit.voxel[0], hit.voxel[1], hit.voxel[2]);
            if c != 0 {
                self.editor.color = c;
            }
        }
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
        if self.drag.take().is_some() {
            return; // the moved layer stays floating until deselect
        }
        if self.marquee.is_some() {
            self.finalize_marquee();
            return;
        }
        if self.painting {
            self.editor.document.end_stroke();
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
            demiurg_view::marquee_voxels(
                self.editor.document.model(),
                &cam,
                f64::from(size.width),
                f64::from(size.height),
                [m.start, self.cursor],
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
        if self.editor.document.set_cells(cells) {
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
            self.editor.document.set_cells(cells);
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
            event_loop.exit();
        }
    }

    fn do_undo(&mut self) {
        if self.editor.document.undo() {
            self.editor.dirty = true;
        }
    }

    fn do_redo(&mut self) {
        if self.editor.document.redo() {
            self.editor.dirty = true;
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
        let show_quit = self.confirm_quit;
        let editor = &mut self.editor;
        let mut actions = UiActions::default();
        let out = ctx.run_ui(raw, |ui| {
            ui::build(ui, editor, &mut actions, show_quit, marquee);
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
        // Saving serializes the document model, so bake a floating layer
        // in first or it would be silently left out of the file.
        if a.save_kv6 || a.save_vxl || a.save_project {
            self.commit_float();
        }
        if a.new_model {
            self.load_model(new_model());
            self.doc_name = None;
        }
        if a.open_kv6 {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("kv6", &["kv6"])
                .pick_file()
            {
                match std::fs::read(&path).map(|b| VoxelModel::from_kv6_bytes(&b)) {
                    Ok(Ok(m)) => {
                        self.load_model(m);
                        self.doc_name = stem_of(&path);
                    }
                    Ok(Err(e)) => eprintln!("demiurg: {}: {e}", path.display()),
                    Err(e) => eprintln!("demiurg: read {}: {e}", path.display()),
                }
            }
        }
        if a.open_project {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("demiurg", &["demiurg"])
                .pick_file()
            {
                match std::fs::read(&path).map(|b| project::from_bytes(&b)) {
                    Ok(Ok(m)) => {
                        self.load_model(m);
                        self.doc_name = stem_of(&path);
                    }
                    Ok(Err(e)) => eprintln!("demiurg: {}: {e}", path.display()),
                    Err(e) => eprintln!("demiurg: read {}: {e}", path.display()),
                }
            }
        }
        if a.save_kv6 {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("kv6", &["kv6"])
                .set_file_name("model.kv6")
                .save_file()
            {
                let bytes = self.editor.document.model().to_kv6_bytes();
                self.on_saved(&path, std::fs::write(&path, bytes));
            }
        }
        if a.save_vxl {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("vxl", &["vxl"])
                .set_file_name("model.vxl")
                .save_file()
            {
                let bytes = self.editor.document.model().to_vxl_bytes();
                self.on_saved(&path, std::fs::write(&path, bytes));
            }
        }
        if a.save_project {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("demiurg", &["demiurg"])
                .set_file_name("model.demiurg")
                .save_file()
            {
                let bytes = project::to_bytes(self.editor.document.model());
                self.on_saved(&path, std::fs::write(&path, bytes));
            }
        }
    }

    /// React to a save attempt: on success, clear the modified flag and
    /// adopt the file name; on failure, just log.
    fn on_saved(&mut self, path: &std::path::Path, result: std::io::Result<()>) {
        match result {
            Ok(()) => {
                eprintln!("demiurg: saved {}", path.display());
                self.editor.document.mark_saved();
                self.doc_name = stem_of(path);
            }
            Err(e) => eprintln!("demiurg: write {} failed: {e}", path.display()),
        }
    }

    /// Replace the document model, rebuild the sprite, refresh the
    /// palette, and reframe.
    fn load_model(&mut self, model: VoxelModel) {
        self.editor.document.replace_model(model);
        self.view
            .set_model(self.editor.document.model(), self.editor.render_mode);
        self.editor.refresh_palette();
        self.editor.sync_resize_dims();
        self.editor.selection.clear();
        self.editor.float = None; // a loaded model starts with no float
        self.drag = None;
        self.camera = self.view.framing_camera();
        self.editor.dirty = false;
    }

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
            event_loop.exit();
            return;
        }
        if actions.quit_cancel {
            self.confirm_quit = false;
        }
        // A tool switch (keyboard or a panel click) away from a floating
        // layer settles it into the model.
        if self.editor.tool != self.last_tool {
            self.commit_float();
            self.last_tool = self.editor.tool;
        }
        if self.editor.dirty {
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
        self.update_title();

        let settings = OpticastSettings::for_oracle_framebuffer(size.width, size.height);
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

        // Editor lines (uses the pick ray from the last frame's
        // projection — fine at redraw cadence). Built before the mutable
        // renderer borrow.
        let lines = self.scene_lines();

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        renderer.set_sprites(self.view.sprites());
        renderer.render(self.view.scene_mut(), &camera, &frame);
        // Depth-tested editor gizmos land in the framebuffer; paint_egui
        // then draws the panels on top.
        renderer.draw_lines(&camera, &lines);
        renderer.paint_egui(&jobs, &textures, ppp);
        // Next redraw is scheduled by `about_to_wait` at the frame cap.
    }

    /// Dispatch a key press/release (camera holds, tool hotkeys, undo).
    fn on_key(&mut self, event_loop: &ActiveEventLoop, code: KeyCode, pressed: bool) {
        let ctrl = self.modifiers.control_key();
        let shift = self.modifiers.shift_key();
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
            KeyCode::Delete | KeyCode::Backspace if pressed => self.delete_selection(),
            KeyCode::ArrowLeft => self.keys.left = pressed,
            KeyCode::ArrowRight => self.keys.right = pressed,
            KeyCode::ArrowUp => self.keys.up = pressed,
            KeyCode::ArrowDown => self.keys.down = pressed,
            KeyCode::KeyW => self.keys.zoom_in = pressed,
            KeyCode::KeyS => self.keys.zoom_out = pressed,
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

        // GPU backend by default (roxlap falls back to CPU if init
        // fails); set ROXLAP_GPU=0 to force the CPU renderer.
        let want_gpu = std::env::var("ROXLAP_GPU").map_or(true, |v| v != "0");
        let mut opts = RenderOptions {
            want_gpu,
            // The empty (sprite-only) scene's background comes from the
            // construction-time clear colour, so set it here too — not
            // just FrameParams.sky_color (which feeds sky-miss / GPU).
            clear_sky: SKY_COLOR,
            ..RenderOptions::default()
        };
        // vsync-cap the GPU present (Fifo) so it doesn't render the idle
        // editor scene flat-out; the ~60 fps frame timer caps the CPU
        // path the same way.
        opts.gpu.uncapped_present = false;
        let size = window.inner_size();
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
                self.orbiting = state == ElementState::Pressed;
                if !self.orbiting {
                    self.last_drag = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                if self.drag.is_some() {
                    self.update_drag();
                }
                if self.painting {
                    self.paint_step();
                }
                if self.orbiting {
                    if let Some((lx, ly)) = self.last_drag {
                        self.camera
                            .orbit((position.x - lx) * 0.01, -(position.y - ly) * 0.01, 0.0);
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
