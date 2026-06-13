//! demiurg native editor (M2): open a window, load/edit a `.kv6` voxel
//! model, and see it rendered by roxlap's own renderer. An egui overlay
//! provides the tools, palette, mirror, pivot, and file menu.
//!
//! Usage:
//!   demiurg [path.kv6 | path.demiurg]   # no path -> a blank canvas
//!
//! Controls: left mouse applies the active tool (hold to drag-paint);
//! right-mouse drag orbits; wheel and `W`/`S` zoom; arrow keys orbit.
//! Hotkeys: `1`-`7` pick a tool, `Ctrl+Z` undo, `Ctrl+Y` / `Ctrl+Shift+Z`
//! redo, `Esc` quits. `DEMIURG_LANG=ru` starts in Russian.
//! `ROXLAP_GPU=1` tries the wgpu backend.

mod ui;

use std::process::exit;
use std::sync::Arc;

use demiurg_core::{Document, VoxelModel, project};
use demiurg_i18n::Lang;
use demiurg_view::{ModelView, OrbitCamera, PickHit, pick_voxel};
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
    /// The viewport sprite needs a rebuild from the model.
    dirty: bool,
}

impl Editor {
    fn new(model: VoxelModel) -> Self {
        let lang = std::env::var("DEMIURG_LANG")
            .ok()
            .and_then(|c| Lang::from_code(&c))
            .unwrap_or_default();
        let model_palette = model.used_colors();
        Self {
            document: Document::new(model),
            tool: Tool::Place,
            color: 0x80c8_c8c8,
            radius: 2,
            box_anchor: None,
            model_palette,
            lang,
            lighting: true,
            dirty: false,
        }
    }

    /// Recompute the model-colour palette (after any edit / load).
    fn refresh_palette(&mut self) {
        self.model_palette = self.document.model().used_colors();
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

    let view = ModelView::new(&model);
    let camera = view.framing_camera();
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

    /// The voxel under the cursor, if any.
    fn pointer_pick(&self) -> Option<PickHit> {
        let cam = self.camera.to_roxlap();
        let ray = self
            .renderer
            .as_ref()?
            .view_ray(&cam, self.cursor.0, self.cursor.1)?;
        pick_voxel(self.editor.document.model(), ray.origin, ray.dir)
    }

    /// Left-button press: start a drag-paint stroke (continuous tools) or
    /// apply a click-once tool.
    fn begin_paint(&mut self) {
        if self.editor.tool.is_continuous() {
            self.editor.document.begin_stroke();
            self.painting = true;
            self.last_paint = None;
            self.paint_step();
        } else if let Some(hit) = self.pointer_pick() {
            self.editor.apply(hit);
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

    /// Left-button release: close the drag-paint stroke (one undo step).
    fn end_paint(&mut self) {
        if self.painting {
            self.editor.document.end_stroke();
            self.painting = false;
            self.last_paint = None;
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
        let editor = &mut self.editor;
        let mut actions = UiActions::default();
        let out = ctx.run(raw, |c| ui::build(c, editor, &mut actions));
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
        if a.new_model {
            self.load_model(new_model());
        }
        if a.open_kv6 {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("kv6", &["kv6"])
                .pick_file()
            {
                match std::fs::read(&path).map(|b| VoxelModel::from_kv6_bytes(&b)) {
                    Ok(Ok(m)) => self.load_model(m),
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
                    Ok(Ok(m)) => self.load_model(m),
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
                report_write(&path, std::fs::write(&path, bytes));
            }
        }
        if a.save_project {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("demiurg", &["demiurg"])
                .set_file_name("model.demiurg")
                .save_file()
            {
                let bytes = project::to_bytes(self.editor.document.model());
                report_write(&path, std::fs::write(&path, bytes));
            }
        }
    }

    /// Replace the document model, rebuild the sprite, refresh the
    /// palette, and reframe.
    fn load_model(&mut self, model: VoxelModel) {
        self.editor.document.replace_model(model);
        self.view.set_model(self.editor.document.model());
        self.editor.refresh_palette();
        self.camera = self.view.framing_camera();
        self.editor.dirty = false;
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.step_camera();

        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }

        let (jobs, textures, ppp, actions) = self.run_ui(&window);
        self.apply_actions(&actions);
        if self.editor.dirty {
            self.view.set_model(self.editor.document.model());
            self.editor.refresh_palette();
            self.editor.dirty = false;
        }

        let camera = self.camera.to_roxlap();
        let settings = OpticastSettings::for_oracle_framebuffer(size.width, size.height);
        // Lit (lightmode 1) by default for directional shading; the View
        // menu can switch it to flat (lightmode 0). `R==G==B` material
        // takes the cheap shading path either way.
        let sprite_lighting = SpriteLighting {
            kv6col: SPRITE_MATERIAL,
            lightmode: u32::from(self.editor.lighting),
            lights: &[],
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
            side_shades: [0; 6],
        };

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        renderer.set_sprites(self.view.sprites());
        renderer.render(self.view.scene_mut(), &camera, &frame);
        renderer.paint_egui(&jobs, &textures, ppp);

        window.request_redraw();
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
            KeyCode::KeyZ if pressed && ctrl && !shift => self.do_undo(),
            KeyCode::KeyZ if pressed && ctrl && shift => self.do_redo(),
            KeyCode::KeyY if pressed && ctrl => self.do_redo(),
            KeyCode::ArrowLeft => self.keys.left = pressed,
            KeyCode::ArrowRight => self.keys.right = pressed,
            KeyCode::ArrowUp => self.keys.up = pressed,
            KeyCode::ArrowDown => self.keys.down = pressed,
            KeyCode::KeyW => self.keys.zoom_in = pressed,
            KeyCode::KeyS => self.keys.zoom_out = pressed,
            KeyCode::Escape if pressed => event_loop.exit(),
            _ => {}
        }
    }
}

/// Log the result of a file write.
fn report_write(path: &std::path::Path, result: std::io::Result<()>) {
    match result {
        Ok(()) => eprintln!("demiurg: saved {}", path.display()),
        Err(e) => eprintln!("demiurg: write {} failed: {e}", path.display()),
    }
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

        let want_gpu = std::env::var_os("ROXLAP_GPU").is_some_and(|v| v != "0" && !v.is_empty());
        let opts = RenderOptions {
            want_gpu,
            // The empty (sprite-only) scene's background comes from the
            // construction-time clear colour, so set it here too — not
            // just FrameParams.sky_color (which feeds sky-miss / GPU).
            clear_sky: SKY_COLOR,
            ..RenderOptions::default()
        };
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
            WindowEvent::CloseRequested => event_loop.exit(),
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
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
}
