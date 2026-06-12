# demiurg — design

`demiurg` is the voxel **asset editor** for the [roxlap](../roxlap) voxel engine
and the [monada](../monada) game framework built on top of it. It is the tool
where you author **models** (`.kv6`), **skeletal animations** (`.kfa`), and
**voxel-video** clips (`.vvid`), and immediately preview them **rendered by the
actual engine** — because the viewport *is* roxlap, not an approximation.

First concrete target: author the piece set, board, and capture animations for
**monada chess 2.0**.

The editor runs both as a native desktop app and as WASM in the browser, from a
single codebase, and saves directly into the engine's formats.

---

## 1. Scope & relationships

demiurg is an **asset-authoring** tool. It is deliberately *not* the
WC3-World-Editor-class map/logic tool that `monada-editor` (monada DESIGN.md §3.5,
M5) will be. The two are different layers and compose cleanly:

```
demiurg            →  produces .kv6 / .kfa / .vvid + palettes
monada-editor      →  imports those assets, places terrain, declares archetypes,
                      wires triggers/scripts, test-plays
monada-chess       →  ships the assets demiurg authored inside chess.monada
```

**Dependency rule:** demiurg depends on **roxlap only** (path deps to
`roxlap-formats`, `roxlap-render`, `roxlap-scene`, `roxlap-core`, and the new
`roxlap-voxvideo`). It does **not** pull in monada (`rhai`, `quinn`, `tokio`,
fixed-point sim) — the editor authors art, it does not run the deterministic
simulation.

**Voxel-video lives in roxlap.** The codec is a new crate `roxlap-voxvideo`
(the engine must *decode* `.vvid` at runtime, so the format is owned engine-side).
demiurg imports it to *encode* and *preview*. monada's existing `monada-voxvideo`
becomes a thin re-export of `roxlap-voxvideo` — one source of truth for the
format.

---

## 2. Why this is cheap to build

Most of the hard parts already exist in roxlap and are reused verbatim:

| Need | Reused from roxlap |
|---|---|
| Read/write `.kv6` `.kvx` `.vxl` `.kfa` | `roxlap_formats::{kv6,kvx,vxl,kfa}::{parse,serialize}` |
| Surface extraction + face `vis`/normal `dir` bits | `roxlap_formats::Kv6::from_fn` |
| Voxel edit primitives | `roxlap_formats::edit::{set_cube,set_sphere,set_rect,delslab,insslab}` |
| Render a model exactly as the game will | `roxlap_render::SceneRenderer` (CPU `softbuffer` / GPU `wgpu`, auto-fallback) |
| Sprite + skeletal-animation render path | `SceneRenderer::{set_sprites, set_kfa_sprites, update_kfa_poses}` |
| Screen→world picking | `SceneRenderer::{view_ray, pick, pick_depth}` |
| Native + WASM windowing parity | `SceneRenderer::new` accepts any `HasWindowHandle + HasDisplayHandle` (winit / canvas) |
| egui-over-framebuffer compositing | pattern proven in `monada-host` |

The editor is mostly *document model + tools + UI* glued onto these.

---

## 3. Crate layout

A cargo workspace mirroring roxlap/monada conventions (`workspace.package`,
shared lints, `pedantic` clippy).

```
demiurg-core    Document model, edit commands, undo/redo, format conversion.
                NO UI, NO windowing. wasm-safe, unit-tested.
                deps: roxlap-formats, roxlap-voxvideo

demiurg-view    Viewport: wraps SceneRenderer, builds Scene/SpriteSet from the
                document, orbit camera, voxel-precise picking, gizmos/overlays.
                deps: demiurg-core, roxlap-render, roxlap-scene, roxlap-core

demiurg-ui      egui panels: tool palette, color palette editor, timeline,
                menus, inspector. Takes &mut Document + &egui::Context.
                NO winit — platform-agnostic.
                deps: demiurg-core, demiurg-view

demiurg-app     Native binary: winit event loop, window, egui composited onto
                the roxlap framebuffer, rfd file dialogs, CLI args.
                deps: demiurg-ui, demiurg-view, demiurg-core

demiurg-web     wasm32 entry: canvas via winit-web / wasm-bindgen, trunk build,
                File System Access API (save) + file input (load).
                deps: demiurg-ui, demiurg-view, demiurg-core

demiurg-cli     (optional, later) headless: png→kv6, bake .vvid, validate.
                For CI / batch asset pipelines.
                deps: demiurg-core
```

The key invariant: **all editing logic lives in `demiurg-core` with no window/UI
dependency**, so it is identical across native and wasm and fully testable
headless. The hosts (`-app`, `-web`) stay thin.

---

## 4. Document model (`demiurg-core`)

`.kv6` stores **only surface voxels** (each with face-visibility bits `vis` and a
normal-table index `dir`) — awkward to edit directly. So the editor keeps a
**dense editable volume** and treats `.kv6` as a *compiled export*.

```rust
struct VoxelModel {
    dims:    UVec3,           // xsiz, ysiz, zsiz
    pivot:   Vec3,            // xpiv/ypiv/zpiv — monada rotates the piece about this
    voxels:  Vec<u32>,        // dense x*y*z, 0 = empty, else 0x80RRGGBB
    palette: Option<Palette>, // optional 256-color palette; truecolor otherwise
}
```

- **Import** `from_kv6`: place surface voxels as-is (a "shell" — fine for editing).
- **Export / compile** `to_kv6`: call
  `roxlap_formats::Kv6::from_fn(dims, |x,y,z| self.get(x,y,z))`, which reuses the
  engine's *exact* surface-extraction and `vis`/`dir` computation, then
  `kv6::serialize`. **What you painted is byte-for-byte what the engine renders.**

Because `.kv6` is lossy (surface-only, palette quantization), WIP is saved in a
**lossless project format `.demiurg`** (postcard or json of `VoxelModel`).
`.kv6` is the export artifact for the engine; `.demiurg` is the editable source.

Dense storage is the right call for kv6-sized models (chess pieces ≈ 32–64³). A
sparse/chunked backing (sharing `roxlap-scene`'s `Vxl` chunks) is a later option
if very large models appear; guard the dense allocation with a dimension cap.

---

## 5. Live preview — "rendered by the engine"

```
edit → (debounce) → VoxelModel::to_kv6 → Sprite → SpriteSet → renderer.set_sprites
```

For chess-piece-sized models the rebuild is microseconds, so the viewport shows
*exactly* what monada chess will render. Extensions reuse the same render paths:

- **Skeletal animation:** `renderer.set_kfa_sprites` with the rig + current frame.
- **Voxel-video:** decode the current frame to a `Kv6`, feed the same
  `set_sprites` path while scrubbing the timeline.

No determinism/fixed-point in the editor (it authors art, it does not run the
lockstep sim). Faithful output is guaranteed structurally: the engine's own
`serialize`/`Kv6::from_fn` produce the bytes.

---

## 6. Editing: commands, undo, tools

Every mutation is a `Command` with `apply`/`undo` (storing the before-image of the
affected AABB). Tools emit commands onto an undo/redo stack.

**MVP tool set (kv6, no animation):**

- voxel place / erase (raycast → hit face → adjacent empty cell, MagicaVoxel-style)
- box / rect fill & erase
- sphere brush
- paint (recolor in place) · eyedropper · line · 3D flood-fill
- mirror modes X/Y/Z
- pivot editing · canvas resize/crop
- 256-color palette editor

**Picking:** a small voxel DDA over the dense volume in `demiurg-view` gives
precise "which face / which adjacent empty cell" for placement; `SceneRenderer`'s
`pick`/`view_ray` handles selection highlight. **Camera:** an `OrbitCamera`
(orbit/zoom around the model bbox, plus quick front/side/top views), following
monada's camera pattern.

---

## 7. roxlap-voxvideo — the `.vvid` format

"MP4 for voxels": a stream of frames stored as **compressed grid diffs**, not
vector motion of sprites. Lives in roxlap (engine decodes at runtime). Sketch
(full design doc to land in roxlap before implementation):

```
header  { magic, version, dims, fps, loop_flag, global_palette[256], keyframe_interval }
frames:
  I-frame (keyframe): full grid as RLE slabs (same model as kv6/vxl)
  P-frame (delta):    per-column span diffs vs the previous reconstructed grid
                      + a dirty AABB to bound application
container: zstd over the whole stream
```

- **Per-column span diffs** match the slab/RLE model of kv6/vxl, are cheap to
  apply, and reuse roxlap's existing `expandrle` / `compilerle` primitives.
- Periodic I-frames enable seek/scrub.
- Decode reconstructs frame N from the last keyframe; the engine renders each
  frame as an ordinary `Kv6` sprite (existing path).
- Global palette shared across the clip; deltas carry only geometry + color index.

Open design questions deferred to the roxlap-voxvideo design doc: adaptive vs
fixed keyframe cadence, palette deltas (sidecar vs inline), and whole-grid vs
chunked keyframes for large clips.

---

## 8. Persistence & WASM

**Native (`demiurg-app`):** `rfd` dialogs; read/write `.kv6`/`.kfa`/`.vvid` via
roxlap serialize; `.demiurg` project file for lossless WIP.

**Web (`demiurg-web`):** winit-web canvas; roxlap-render runs on
WebGPU/WebGL2 (`wgpu`) with the `softbuffer` CPU fallback — both already exercised
in `roxlap-web`. Built with `trunk`. Saving "directly to the format" uses the
File System Access API where available, otherwise a blob download; loading via a
file input. Single-threaded first; SharedArrayBuffer multi-thread optional (the
path exists in roxlap, see roxlap `PORTING-WASM.md`).

---

## 9. Roadmap

| Milestone | Content |
|---|---|
| **M0** | Workspace skeleton; `demiurg-core`: `VoxelModel` + kv6 import/export round-trip, headless tests |
| **M1** | `demiurg-view` + `demiurg-app`: open a window, load `.kv6`, orbit camera, engine preview (read-only viewer) |
| **M2** | **kv6 editor, no animation**: place/erase/paint/box/sphere, undo/redo, palette, pivot, save kv6 + `.demiurg` project |
| **M3** | `demiurg-web`: the same editor in the browser via trunk |
| **M4** | `.kfa` animation: rig (hinges/joints), pose frames, sequence timeline, preview via `set_kfa_sprites` |
| **M5** | `roxlap-voxvideo` codec + voxel-video timeline in demiurg (record / bake / scrub / export `.vvid`) |
| **M6** | Polish: monada chess 2.0 piece set, palettes, batch CLI |

**M2 is the first usable product** — the "kv6 sprite editor without animation"
that bootstraps the chess 2.0 art.

---

## 10. Integration with monada chess 2.0

demiurg exports the piece KV6s (6 types, palette-swapped per side), board tiles,
and capture animations (`.kfa` or `.vvid`). `monada-format` packs them into
`chess.monada`; `monada-chess` ships them. demiurg's engine-faithful preview is
the guarantee that pieces look correct *before* they enter the game build.
