//! demiurg native viewer (M1): open a window, load a `.kv6`, and orbit
//! around it rendered by roxlap's own renderer — what you see is exactly
//! what the engine draws.
//!
//! Usage:
//!   demiurg [path.kv6]      # no path -> a built-in demo model
//!
//! Controls: arrow keys orbit, `W`/`S` zoom, mouse-drag orbits, scroll
//! zooms, `Esc` quits. `ROXLAP_GPU=1` tries the wgpu backend (CPU
//! otherwise; roxlap falls back automatically if GPU init fails).

use std::process::exit;
use std::sync::Arc;

use demiurg_core::VoxelModel;
use demiurg_view::{ModelView, OrbitCamera};
use roxlap_core::opticast::OpticastSettings;
use roxlap_core::sprite::SpriteLighting;
use roxlap_render::{FrameParams, RenderOptions, SceneRenderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// Packed `0x00RRGGBB` sky/clear colour (matches the monada host).
const SKY_COLOR: u32 = 0x0099_b3d9;

fn main() {
    let path = std::env::args().nth(1);
    let model = if let Some(p) = &path {
        load_model(p)
    } else {
        eprintln!("demiurg: no .kv6 given; showing a demo model (pass a path to view a file)");
        demo_model()
    };

    let view = ModelView::new(&model);
    let camera = view.framing_camera();
    let mut app = App {
        window: None,
        renderer: None,
        view,
        camera,
        lighting: SpriteLighting::default_oracle(),
        keys: Keys::default(),
        dragging: false,
        last_drag: None,
        // DEMIURG_CAPTURE=<path.ppm>: render one frame, write it, exit.
        // A headless smoke hook (CPU backend only); also a seed for
        // future screenshot tests.
        capture: std::env::var("DEMIURG_CAPTURE").ok(),
        done: false,
    };

    let event_loop = EventLoop::new().expect("winit: create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut app).expect("winit: run_app");
}

/// Read and parse a `.kv6`, or exit with a message.
fn load_model(path: &str) -> VoxelModel {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("demiurg: cannot read {path}: {e}");
        exit(2);
    });
    VoxelModel::from_kv6_bytes(&bytes).unwrap_or_else(|e| {
        eprintln!("demiurg: {path}: {e}");
        exit(2);
    })
}

/// A 16³ colour-gradient cube shell, so `demiurg` shows something with
/// no file argument.
#[allow(clippy::cast_possible_truncation)] // channel math is bounded to 0..=255
fn demo_model() -> VoxelModel {
    const N: u32 = 16;
    let mut m = VoxelModel::new(N, N, N);
    for z in 0..N {
        for y in 0..N {
            for x in 0..N {
                let on_shell = x == 0 || y == 0 || z == 0 || x == N - 1 || y == N - 1 || z == N - 1;
                if !on_shell {
                    continue;
                }
                let r = (x * 255 / (N - 1)) as u8;
                let g = (y * 255 / (N - 1)) as u8;
                let b = (z * 255 / (N - 1)) as u8;
                let col = 0x8000_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
                m.set(x, y, z, col);
            }
        }
    }
    m
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
    lighting: SpriteLighting<'static>,
    keys: Keys,
    dragging: bool,
    last_drag: Option<(f64, f64)>,
    capture: Option<String>,
    done: bool,
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

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.step_camera();

        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }

        let camera = self.camera.to_roxlap();
        let settings = OpticastSettings::for_oracle_framebuffer(size.width, size.height);
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
            // Required (Some) for the CPU backend to draw sprites.
            sprite_lighting: Some(&self.lighting),
        };

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        renderer.set_sprites(self.view.sprites());
        if self.capture.is_some() {
            renderer.request_capture();
        }
        renderer.render(self.view.scene_mut(), &camera, &frame);
        renderer.present();

        if let Some(path) = self.capture.take() {
            match renderer.take_capture() {
                Some((pixels, w, h)) => {
                    if let Err(e) = write_ppm(&path, &pixels, w, h) {
                        eprintln!("demiurg: capture write failed: {e}");
                    } else {
                        let lit = pixels
                            .iter()
                            .filter(|&&p| p & 0x00ff_ffff != SKY_COLOR)
                            .count();
                        eprintln!("demiurg: captured {w}x{h} to {path} ({lit} non-sky pixels)");
                    }
                }
                None => eprintln!("demiurg: capture unavailable (GPU backend?)"),
            }
            self.done = true;
            return;
        }

        window.request_redraw();
    }
}

/// Write packed `0x00RRGGBB` pixels as a binary PPM (P6) — no deps.
fn write_ppm(path: &str, pixels: &[u32], width: u32, height: u32) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut buf = Vec::with_capacity(pixels.len() * 3 + 32);
    write!(buf, "P6\n{width} {height}\n255\n")?;
    for &p in pixels {
        // 0x00RRGGBB little-endian = [BB, GG, RR, 00]
        let [b, g, r, _] = p.to_le_bytes();
        buf.extend_from_slice(&[r, g, b]);
    }
    std::fs::write(path, buf)
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("demiurg")
            .with_inner_size(LogicalSize::new(960.0, 720.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("winit: create_window"),
        );

        let want_gpu = std::env::var_os("ROXLAP_GPU").is_some_and(|v| v != "0" && !v.is_empty());
        let opts = RenderOptions {
            want_gpu,
            ..RenderOptions::default()
        };
        let size = window.inner_size();
        let renderer = SceneRenderer::new(window.clone(), (size.width, size.height), &opts);
        match renderer.adapter_info() {
            Some(info) => eprintln!("demiurg: GPU backend - {info}"),
            None => eprintln!("demiurg: CPU backend"),
        }

        self.renderer = Some(renderer);
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        ..
                    },
                ..
            } => {
                let pressed = state == ElementState::Pressed;
                match code {
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
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
                if !self.dragging {
                    self.last_drag = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } if self.dragging => {
                if let Some((lx, ly)) = self.last_drag {
                    self.camera
                        .orbit((position.x - lx) * 0.01, -(position.y - ly) * 0.01, 0.0);
                }
                self.last_drag = Some((position.x, position.y));
            }
            WindowEvent::MouseWheel { delta, .. } => {
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.done {
            event_loop.exit();
        }
    }
}
