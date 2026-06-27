//! Procedural clip generation — an embedded [Rhai](https://rhai.rs) script that
//! fills a clip's frames, for dynamic effects (flame, smoke, energy) that are
//! awkward to hand-sculpt.
//!
//! The script body runs **once per frame** with these globals in scope:
//! `frame` (0-based index), `frames` (total), `t` (`frame/frames`, `0.0..1.0`),
//! and `w` / `h` / `d` (the clip dims). It builds the frame by calling flat host
//! functions — the imperative-globals API:
//!
//! - `set(x,y,z, col)` — write a voxel (`col == 0` clears; out-of-range ignored).
//!   `get(x,y,z) -> col`. `sphere(cx,cy,cz, r, col)`, `box(x0,y0,z0, x1,y1,z1, col)`.
//! - `rgb(r,g,b) -> col`, `hsv(h,s,v) -> col` — packed `0x80RRGGBB` colours
//!   (`r,g,b` are `0..255` ints; `h,s,v` are `0.0..1.0` floats).
//! - `noise(x,y,z) -> 0.0..1.0` — a stable value-noise field (same every frame,
//!   so animate by feeding `t` into a coordinate). `rand() -> 0.0..1.0` and
//!   `rand(a,b)` — a per-frame PRNG. Rhai's built-ins cover `sin`/`cos`/`sqrt`…
//!
//! Voxel coordinates are **integers** (`i64`); sample `noise` with floats, e.g.
//! `noise(x.to_float()*0.15, y.to_float()*0.15, z.to_float()*0.15 + t*2.0)`.
//!
//! Generation is deterministic: the same [`ClipGenerator::seed`] yields the same
//! clip. Rhai is sandboxed (no I/O), and an operation cap stops runaway loops.

use serde::{Deserialize, Serialize};

#[cfg(feature = "generator")]
use crate::VoxelModel;

/// A procedural generator attached to a clip: the Rhai source, how many frames
/// to produce, and the seed driving `rand()` / `noise()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipGenerator {
    pub script: String,
    pub frame_count: u32,
    pub seed: u64,
}

impl ClipGenerator {
    /// A generator with the [`DEMO_SCRIPT`] and sensible defaults — the starting
    /// point for "make this clip procedural".
    #[must_use]
    pub fn demo() -> Self {
        Self {
            script: DEMO_SCRIPT.to_string(),
            frame_count: 16,
            seed: 1,
        }
    }
}

/// Why [`generate`] failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenError {
    /// The script did not compile (syntax / parse error).
    Compile(String),
    /// The script errored at runtime (including hitting the operation cap).
    Runtime(String),
}

impl std::fmt::Display for GenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compile(e) => write!(f, "script error: {e}"),
            Self::Runtime(e) => write!(f, "generation error: {e}"),
        }
    }
}

impl std::error::Error for GenError {}

/// A minimal starter effect: a colour-cycling sphere that pulses with `t` — it
/// exercises `set` / `rgb` / `sphere` / `t` so it doubles as an API example.
pub const DEMO_SCRIPT: &str = r"// A pulsing, colour-cycling blob.
// Globals: frame, frames, t (0..1), w, h, d.
let cx = w / 2;
let cy = h / 2;
let cz = d / 2;
let r = 2.0 + 2.5 * (1.0 + sin(t * 6.2831853)) ;  // pulse 2..7
let hue = t;                                       // cycle the hue over the loop
sphere(cx, cy, cz, r.to_int(), hsv(hue, 0.9, 1.0));
";

// ---- the engine (feature-gated) -----------------------------------------

/// Run `g`'s script once per frame and collect the resulting models — the
/// runtime behind [`ClipDoc::regenerate`](crate::clip::ClipDoc::regenerate).
///
/// Each frame starts empty (`dims` / `pivot`); the script fills it via the host
/// API. `noise()` uses a fixed seed (a stable field across frames); `rand()` is
/// reseeded per frame from `g.seed`, so the clip is reproducible.
///
/// # Errors
/// [`GenError::Compile`] if the script doesn't parse; [`GenError::Runtime`] if a
/// frame's evaluation errors (a real error, or hitting the operation cap).
#[cfg(feature = "generator")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
pub fn generate(
    dims: [u32; 3],
    pivot: [f32; 3],
    g: &ClipGenerator,
) -> Result<Vec<VoxelModel>, GenError> {
    use std::cell::RefCell;
    use std::rc::Rc;

    use rhai::{Engine, Scope};

    let n = g.frame_count.max(1);
    let seed = g.seed;

    let mut engine = Engine::new();
    // Guard against runaway scripts (an infinite loop hits this and returns a
    // runtime error instead of hanging the editor).
    engine.set_max_operations(20_000_000);
    engine.set_max_call_levels(64);
    engine.set_max_string_size(0); // strings unused; keep them from ballooning

    // The working frame + the per-frame PRNG state, shared with the host fns.
    let frame = Rc::new(RefCell::new(VoxelModel::new(dims[0], dims[1], dims[2])));
    let prng = Rc::new(RefCell::new(0u64));

    register_api(&mut engine, &frame, &prng, seed);

    let ast = engine
        .compile(&g.script)
        .map_err(|e| GenError::Compile(e.to_string()))?;

    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        {
            let mut m = frame.borrow_mut();
            *m = VoxelModel::new(dims[0], dims[1], dims[2]);
            m.pivot = pivot;
        }
        *prng.borrow_mut() = splitmix64(seed ^ splitmix64(u64::from(i)));

        let mut scope = Scope::new();
        scope.push_constant("frame", i64::from(i));
        scope.push_constant("frames", i64::from(n));
        let t = if n > 1 {
            f64::from(i) / f64::from(n)
        } else {
            0.0
        };
        scope.push_constant("t", t);
        scope.push_constant("w", i64::from(dims[0]));
        scope.push_constant("h", i64::from(dims[1]));
        scope.push_constant("d", i64::from(dims[2]));

        engine
            .run_ast_with_scope(&mut scope, &ast)
            .map_err(|e| GenError::Runtime(e.to_string()))?;

        out.push(frame.borrow().clone());
    }
    Ok(out)
}

/// Register the imperative-globals host API on `engine`.
#[cfg(feature = "generator")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::many_single_char_names
)]
fn register_api(
    engine: &mut rhai::Engine,
    frame: &std::rc::Rc<std::cell::RefCell<VoxelModel>>,
    prng: &std::rc::Rc<std::cell::RefCell<u64>>,
    seed: u64,
) {
    // Coordinate → Option<(u32,u32,u32)> within the frame, for the geometry fns.
    let in_bounds = |m: &VoxelModel, x: i64, y: i64, z: i64| -> Option<(u32, u32, u32)> {
        let (dx, dy, dz) = m.dims();
        let (x, y, z) = (
            u32::try_from(x).ok()?,
            u32::try_from(y).ok()?,
            u32::try_from(z).ok()?,
        );
        (x < dx && y < dy && z < dz).then_some((x, y, z))
    };

    // set(x,y,z,col): write a voxel (col==0 clears; out-of-range ignored).
    {
        let f = frame.clone();
        engine.register_fn("set", move |x: i64, y: i64, z: i64, col: i64| {
            let mut m = f.borrow_mut();
            if let Some((x, y, z)) = in_bounds(&m, x, y, z) {
                m.set(x, y, z, col as u32);
            }
        });
    }
    // get(x,y,z): the voxel colour, or 0.
    {
        let f = frame.clone();
        engine.register_fn("get", move |x: i64, y: i64, z: i64| -> i64 {
            let m = f.borrow();
            in_bounds(&m, x, y, z).map_or(0, |(x, y, z)| i64::from(m.get(x, y, z)))
        });
    }
    // sphere(cx,cy,cz,r,col): solid filled sphere of integer radius r.
    {
        let f = frame.clone();
        engine.register_fn(
            "sphere",
            move |cx: i64, cy: i64, cz: i64, r: i64, col: i64| {
                if r < 0 {
                    return;
                }
                let mut m = f.borrow_mut();
                let rr = r * r;
                for dz in -r..=r {
                    for dy in -r..=r {
                        for dx in -r..=r {
                            if dx * dx + dy * dy + dz * dz <= rr {
                                if let Some((x, y, z)) = in_bounds(&m, cx + dx, cy + dy, cz + dz) {
                                    m.set(x, y, z, col as u32);
                                }
                            }
                        }
                    }
                }
            },
        );
    }
    // box(x0,y0,z0,x1,y1,z1,col): inclusive axis-aligned box fill.
    {
        let f = frame.clone();
        engine.register_fn(
            "box",
            move |x0: i64, y0: i64, z0: i64, x1: i64, y1: i64, z1: i64, col: i64| {
                let mut m = f.borrow_mut();
                for z in z0.min(z1)..=z0.max(z1) {
                    for y in y0.min(y1)..=y0.max(y1) {
                        for x in x0.min(x1)..=x0.max(x1) {
                            if let Some((x, y, z)) = in_bounds(&m, x, y, z) {
                                m.set(x, y, z, col as u32);
                            }
                        }
                    }
                }
            },
        );
    }
    // rgb(r,g,b): pack a voxlap 0x80RRGGBB colour (channels clamped 0..255).
    engine.register_fn("rgb", |r: i64, g: i64, b: i64| -> i64 {
        let c = |v: i64| v.clamp(0, 255) as u32;
        i64::from(0x8000_0000u32 | (c(r) << 16) | (c(g) << 8) | c(b))
    });
    // hsv(h,s,v): hue/sat/value in 0..1 → packed colour.
    engine.register_fn("hsv", |h: f64, s: f64, v: f64| -> i64 {
        let (r, g, b) = hsv_to_rgb(h, s, v);
        i64::from(0x8000_0000u32 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b))
    });
    // noise(x,y,z): stable value-noise field in 0..1 (fixed seed across frames).
    engine.register_fn("noise", move |x: f64, y: f64, z: f64| -> f64 {
        value_noise(seed, x, y, z)
    });
    // rand(): next per-frame PRNG draw in 0..1.
    {
        let p = prng.clone();
        engine.register_fn("rand", move || -> f64 {
            let mut s = p.borrow_mut();
            *s = splitmix64(*s);
            (*s >> 11) as f64 / 9_007_199_254_740_992.0 // 2^53
        });
    }
    // rand(a,b): a per-frame draw in [a,b).
    {
        let p = prng.clone();
        engine.register_fn("rand", move |a: f64, b: f64| -> f64 {
            let mut s = p.borrow_mut();
            *s = splitmix64(*s);
            let u = (*s >> 11) as f64 / 9_007_199_254_740_992.0;
            a + (b - a) * u
        });
    }
}

// ---- deterministic noise / PRNG primitives (no extra deps) ----------------

/// splitmix64 — a fast, well-mixed 64-bit hash / PRNG step.
#[cfg(feature = "generator")]
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hashed lattice value in `0.0..1.0` for integer corner `(ix,iy,iz)`.
#[cfg(feature = "generator")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn corner(seed: u64, ix: i64, iy: i64, iz: i64) -> f64 {
    let mut h = seed;
    h = splitmix64(h ^ (ix as u64).wrapping_mul(0xA076_1D64_78BD_642F));
    h = splitmix64(h ^ (iy as u64).wrapping_mul(0xE703_7ED1_A0B4_28DB));
    h = splitmix64(h ^ (iz as u64).wrapping_mul(0x8EBC_6AF0_9C88_C6E3));
    (h >> 11) as f64 / 9_007_199_254_740_992.0 // 2^53
}

/// 3-D value noise in `0.0..1.0`: trilinear interpolation of hashed lattice
/// corners with a smoothstep fade — deterministic in `seed`.
#[cfg(feature = "generator")]
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn value_noise(seed: u64, x: f64, y: f64, z: f64) -> f64 {
    let fade = |t: f64| t * t * (3.0 - 2.0 * t);
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;

    let (xi, yi, zi) = (x.floor(), y.floor(), z.floor());
    let (fx, fy, fz) = (fade(x - xi), fade(y - yi), fade(z - zi));
    let (ix, iy, iz) = (xi as i64, yi as i64, zi as i64);

    let c = |dx: i64, dy: i64, dz: i64| corner(seed, ix + dx, iy + dy, iz + dz);
    let x00 = lerp(c(0, 0, 0), c(1, 0, 0), fx);
    let x10 = lerp(c(0, 1, 0), c(1, 1, 0), fx);
    let x01 = lerp(c(0, 0, 1), c(1, 0, 1), fx);
    let x11 = lerp(c(0, 1, 1), c(1, 1, 1), fx);
    lerp(lerp(x00, x10, fy), lerp(x01, x11, fy), fz)
}

/// HSV (each `0.0..1.0`) → 8-bit RGB.
#[cfg(feature = "generator")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let q = |f: f64| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (q(r), q(g), q(b))
}

#[cfg(all(test, feature = "generator"))]
#[allow(clippy::cast_possible_truncation)] // tiny test indices
mod tests {
    use super::*;

    fn make(script: &str, frame_count: u32, seed: u64) -> ClipGenerator {
        ClipGenerator {
            script: script.to_string(),
            frame_count,
            seed,
        }
    }

    #[test]
    fn frame_index_and_voxels_track_the_globals() {
        // Each frame paints a voxel at (frame, 0, 0) — verifies `frame`, `set`,
        // `rgb`, and the per-frame frame count.
        let frames = generate(
            [8, 8, 8],
            [4.0, 4.0, 4.0],
            &make("set(frame, 0, 0, rgb(255, 0, 0));", 4, 1),
        )
        .expect("runs");
        assert_eq!(frames.len(), 4);
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.occupied_count(), 1);
            assert_eq!(f.get(i as u32, 0, 0), 0x80ff_0000);
        }
    }

    #[test]
    fn same_seed_is_deterministic() {
        let g = make("set((rand() * 8.0).to_int(), 0, 0, rgb(1, 2, 3));", 3, 42);
        let a = generate([8, 8, 8], [0.0; 3], &g).expect("runs");
        let b = generate([8, 8, 8], [0.0; 3], &g).expect("runs");
        assert_eq!(a, b, "same seed ⇒ identical frames");
    }

    #[test]
    fn sphere_and_bounds_are_safe() {
        // A sphere centred outside the grid only fills the in-range part; no panic.
        let frames = generate(
            [4, 4, 4],
            [0.0; 3],
            &make("sphere(0, 0, 0, 10, rgb(9,9,9));", 1, 0),
        )
        .expect("runs");
        assert_eq!(frames[0].occupied_count(), 4 * 4 * 4, "whole grid filled");
    }

    #[test]
    fn syntax_error_is_a_compile_error() {
        let r = generate([2, 2, 2], [0.0; 3], &make("set(", 1, 0));
        assert!(matches!(r, Err(GenError::Compile(_))));
    }

    #[test]
    fn runaway_loop_hits_the_op_cap() {
        // An infinite loop must error (op cap), not hang the editor.
        let r = generate(
            [2, 2, 2],
            [0.0; 3],
            &make("let i = 0; while true { i += 1; }", 1, 0),
        );
        assert!(matches!(r, Err(GenError::Runtime(_))));
    }

    #[test]
    fn demo_script_fills_every_frame() {
        let frames = generate([16, 16, 16], [8.0; 3], &ClipGenerator::demo()).expect("runs");
        assert_eq!(frames.len(), 16);
        assert!(frames.iter().all(|f| f.occupied_count() > 0));
    }
}
