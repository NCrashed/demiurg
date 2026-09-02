# Changelog

All notable changes to demiurg are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/). The release CI extracts the section
matching a `vX.Y.Z` tag as the GitHub release notes.

## [Unreleased]

### Fixed

- **The bundled Linux converter would not start on NixOS.** Built against the
  runner's glibc, it carried an ELF interpreter path NixOS does not have, so
  Blender reported "cannot run dynamically linked executables intended for
  generic linux environments" and the export died at the last step. It is now
  linked statically against musl — no interpreter, no glibc version floor — so
  the distro that built it stops mattering, which also retires the
  older-runner pin that was standing in for the same problem. CI checks the
  shipped binary really is static, since a dropped `--target` would undo it
  silently.

### Added

- **Several meshes on one bone export as layers.** They used to be voxelized
  into the bone's single grid, which works until something is held away from
  its bone: a sword in a hand stretched that grid to span both, most of it
  empty. Each extra object now becomes its own attachment — a grid of its own
  size, placed by an offset, named after the object, with the object's origin
  as the pivot it rotates about. A layer can be a flipbook too, so a flame can
  ride a staff.

  The manifest gained `layers` on a bone, `--shot` composes them at their
  offsets (the KFA limb path draws one mesh per bone and nothing else, so an
  exported layer would otherwise be in the file and missing from every
  screenshot of it), and the export summary counts them — it previously
  reported a sword-wielding character as unchanged by the sword.

## [0.16.0] - 2026-09-02

The other half of shading: a rig's transparency is now editable in demiurg
itself, not only in Blender. 0.15.0 gave rigs somewhere to keep per-colour
materials; this opens the panel that edits them.

### Added

- **A rig's transparency is editable in demiurg.** The materials panel was
  hidden whenever a rig was open, because the `.rkc` export had nowhere to keep
  the table — which the `DMAT` chunk fixed, leaving the gate behind. It now
  shows for a rig, and the working mesh mirrors the rig's table on load and
  lifts it back on commit, the same shape the clip editor has always used. The
  posed preview keeps its own copy of the rig, so a slider move now flags it
  for a rebuild and the character in the viewport changes as you drag rather
  than at the next bone switch.

  Inside a rig the table being edited is always the **rig's**, including while
  a clip attachment is open: the rig-level map is what the assembled character
  renders with, so editing a clip's own table there would show a translucency
  the posed rig never has.

## [0.15.0] - 2026-09-02

Shading. A character authored see-through in Blender arrived solid, because a
rig had nowhere to keep per-colour materials and the engine's container has no
material channel. It now carries them in an extra-chunk, the addon reads them
off the Principled BSDF, and both renderers composite — including the headless
`--shot`, so it stays checkable.

Two fixes for what a real slime turned up: a mesh deformed by an armature
modifier can be baked as a flipbook, and a Geometry Nodes modifier that eats
vertex weights is now named as the cause instead of quietly producing a
nonsense per-bone split.

### Added

- **Transparency and glow survive the trip.** A rig had nowhere to put
  per-colour materials — the editor's transparency was for plain models only,
  and the `.rkc` container has no material channel — so a slime authored
  see-through in Blender arrived solid. Rigs now carry a colour → material map
  that rides in a `DMAT` extra-chunk, the way layer names and easing already
  do, which means it round-trips through both `.rkc` and `.demiurg` and
  survives a re-save by an older build.

  The addon reads it off the Principled BSDF: **Alpha** below 1 becomes
  `blend`, **Transmission** with a full alpha becomes `blend` at `1 -
  transmission`, and **Emission Strength** on an otherwise solid material
  becomes `add`. One wins — the engine has a single channel per colour, and
  blending effects it cannot represent would be a guess. Materials key by
  *colour*, since that is what the renderer indexes, so two Blender materials
  sharing a base colour are reported rather than silently reconciled.

  Both renderers show it: the limb sprites carry the colour map, the compose
  pass registers attachments with it, and the headless `--shot` composites —
  a file that carried transparency nothing rendered would be worse than one
  that did not carry it at all.

### Fixed

- **A deforming mesh bound by an armature modifier can now be baked as a
  flipbook.** Marking one was refused with "parent it to a bone", which is the
  wrong answer for the commonest way to build a soft character: an object
  parented to the armature with weights. It is now baked whole onto the root
  bone — everything the armature did to it is already in the frames, so there
  is nothing left for the skeleton to move, which the export says plainly.

- **A Geometry Nodes modifier silently produced a nonsense per-bone split.**
  A voxelizer or remesh rebuilds the geometry and does not carry vertex groups
  across, so a mesh fully weighted in the outliner arrives at the exporter with
  no weights at all. Every voxel then fell to the nearest-bone fallback and the
  character came apart along nothing in particular. The export now checks the
  *evaluated* mesh and names the cause, pointing at the flag that doesn't need
  weights.

## [0.14.0] - 2026-08-28

Geometry that changes shape. A skeleton moves rigid chunks and cannot reshape
one, so anything that squashes — a slime, cloth, a wobbling blob — now exports
as a per-frame voxel flipbook the engine plays, including different geometry
per action. `--shot` composes it, so it can still be checked without opening
the editor.

### Added

- **Meshes that deform export as per-frame voxel flipbooks.** A skeleton moves
  rigid chunks and cannot reshape one, so a slime baked at rest and then pushed
  around by bones came out wrong no matter how the rig was built. A bone can
  now carry an animated voxel clip instead of a mesh — the engine has had the
  attachment kind all along — and the Blender addon bakes one from any
  deformation Blender evaluates: shape keys, lattices, sims, drivers. Tick
  **Voxelize per frame** on the object; on its own it exports as a one-bone
  flipbook, parented to a bone it deforms while the rest of the skeleton stays
  rigid, so a hard-shelled character with a soft belly is one export.

  **Clip fps** is the size knob, independent of the scene's frame rate: every
  sample is a whole voxel grid, so it is what decides the cost. A bone's frames
  are stored in its own frame with its skeletal pose removed, so a carried blob
  is not posed twice. The manifest gained a `clip` on a bone (mutually
  exclusive with `mesh`, and rejected by name if both appear), and the summary
  reports clip frames, since a clip bone's voxels live in its frames rather
  than its placeholder model.

  **A deforming bone can hold different geometry per action.** One bone has one
  flipbook and the rig playhead picks its frame, so a rig with both a deforming
  bone and several actions gets those actions laid out on separate windows of
  one timeline — `walk` on 0…1000 ms, `idle` on 1000…1800 — with the flipbook
  concatenated to cover them. Each action reaches its own geometry and still
  cycles inside its own window, since a clip's loop marker returns to its own
  first key rather than to zero. `--shot` and `--dump-pose` take `--time`
  relative to the chosen clip, so the layout stays an implementation detail.
  Rigs without a deforming bone, or with one action, are laid out at 0 as
  before.

  An action a file holds but the armature cannot play — a shape key's, a
  material's — is now skipped instead of aborting the export when Blender
  refuses the binding.

- **`--shot` composes a deforming bone's current frame.** A clip bone's limb
  sprite is deliberately empty — the frames are drawn on top — so without this
  a slime rendered as nothing at all, which would have made the whole feature
  unverifiable. The frame-picking arithmetic moved into `demiurg-core`, shared
  by the viewport and the headless shot so a screenshot cannot disagree with
  the editor.

## [0.13.0] - 2026-08-28

A bridge from Blender: model and animate a character there, export it, open it
here. The addon voxelizes an armature — meshes parented to bones, or one
skinned mesh cut into rigid per-bone chunks — bakes its actions into clips, and
hands the result to a new `demiurg-convert` binary that writes the file. The
editor gained the tools that made it checkable: `--shot` renders a posed rig,
and `--dump-pose` says where each bone actually ended up.

### Added

- **A Blender addon** (`blender/`) that voxelizes an armature's per-bone meshes
  and exports a `.demiurg` project, animation included. It writes only the JSON
  manifest and shells out to `demiurg-convert`, so nothing about the wire
  formats is duplicated in Python. Meshes are voxelized in armature space by
  nearest-surface queries (surface plus, optionally, the interior), colours come
  from the Principled BSDF converted linear → sRGB, and the height axis is
  flipped from Blender's Z-up to voxlap's Z-down. Actions are baked one key per
  frame by stepping the scene and reading each bone's evaluated matrix — the
  clip format stores whole-skeleton poses interpolated linearly, so curves have
  to be sampled, and sampling also means constraints, drivers, and whatever an
  action stores internally all arrive evaluated. A mesh bound with an armature
  modifier is cut into rigid per-bone chunks — the engine draws a bone as one
  sprite, so a voxel cannot be shared, and each goes to the bone weighing most
  on the triangle nearest to it (unweighted geometry to the closest bone,
  reported rather than dropped). The cut is rigid by nature: a joint that bends
  far shows a seam, which the README says plainly. Verified against Blender
  numerically — every bone on every frame of a baked clip lands within 0.0001
  voxels of where Blender puts it, and a smoothly weighted column comes apart
  at the joint with every voxel kept.

- **A workflow that builds the Blender addon for every platform.** Four jobs
  compile `demiurg-convert` natively — Linux, Windows, and both Mac
  architectures — and a packaging job folds them into one zip that installs
  anywhere. It runs on any push or pull request that touches what goes into the
  zip, so a broken package fails there rather than at release time; on demand
  for a build off current master; and from the release pipeline, which reuses
  it through `workflow_call` instead of keeping a second copy in step. The zip
  is checked before upload: manifest present, Python compiles, and a converter
  for each of the four platforms. CI also runs the addon's Blender-free tests
  on every push.

  A release stamps its tag into the extension's manifest and the zip's name
  (`--version`), so an installed addon reports the release it came from instead
  of a version drifting on its own. The manifest in git carries the workspace
  version for local builds, and a release build leaves the tree clean.

- **`scripts/package-blender-addon.sh`** builds a single zip an artist installs
  and uses — the `demiurg-convert` binary rides inside the addon under
  `bin/<platform>/`, so there is no second download and no path to set. The
  addon prefers an explicit path from its preferences (a developer's own
  build), then the bundled binary, then `PATH`, and says which one it will run.
  `--cross windows` cross-compiles a Windows converter from Linux (mingw-w64
  linker, everything static, so the `.exe` imports only system DLLs and needs
  no redistributable); other platforms fold in with `--with-bin`, and one zip
  can carry several. The `x86_64-pc-windows-gnu` target is pinned in
  `rust-toolchain.toml` for it.

- **`--dump-pose`** prints where the solver puts each bone (position + basis)
  at a given `--clip` / `--time`, one line per bone. A screenshot says an
  exported animation looks wrong; this says which bone is off and by how much,
  which is what makes `blender/tests/compare_poses.py` possible.

- **`demiurg-convert`: a JSON manifest to `.demiurg` / `.rkc`.** The bridge a
  DCC exporter (the Blender addon) shells out to, so the wire formats stay
  written in one language: the exporter emits JSON — per-bone voxel meshes
  (inline or a `.vox` beside the manifest), the skeleton, and baked animation
  clips — and this binary assembles the rig. Clip keys carry full TRS
  (translation, quaternion, scale) per bone, which the `.rkc` clip format has
  stored since roxlap 0.30, so a Blender action maps onto it directly once it
  is baked to one whole-skeleton pose per key. Validation is the point of the
  tool: duplicate bone names, parent cycles, out-of-bounds voxels, misspelled
  fields, and poses on a root bone (which the solver silently ignores, because
  a root takes the sprite's own basis) are all rejected by name instead of
  exporting a subtly broken rig.

- **`--shot` renders a posed rig**, picked with `--clip <name|index>` and
  `--time <ms>`. It used to draw only the active bone's mesh, which for a
  rigged document meant one lonely body part — useless for checking an
  animation. The headless path now solves every limb at the playhead and draws
  them all (`KfaView::render_cpu`), frames the camera on the rig, and logs
  which clip and time it rendered. An unknown `--clip` lists the real ones
  instead of quietly falling back to the rest pose. This is how a DCC
  exporter's output gets compared against its source, frame by frame. The limb
  path draws one mesh per bone: extra attachment layers, clip layers, and gizmo
  lines belong to the windowed compose pass and are not in the shot.

### Fixed

- **A keyframe rotation swung a bone's head around its parent's pivot** instead
  of turning the bone in place: the solver puts a bone at `t + r · anchor`, so
  rotating a shoulder walked the whole arm along an arc of radius `joint` and
  off the body. The converter now cancels that arc, so `r` means what every DCC
  tool (and every animator) means by rotating a bone — spin about its own
  joint — and `t` stays a plain offset from it. Asserted through the real
  solver, since the difference is invisible in the stored values.

- **A manifest's `joint` hung every limb off the wrong side of its parent.**
  The solver places a child at `parent + (p[0] - p[1])`, so writing the joint
  into the hinge anchor as-is mirrored it through the parent's pivot — an
  exported arm ended up detached below the body instead of at the shoulder.
  The converter negates it, keeping `joint` meaning what the schema says (where
  the child's pivot lands, measured from the parent's), and a test now asserts
  the *solved* offset rather than the field, since the sign reads perfectly
  either way in the struct.

- **A rig or clip `.demiurg` given on the command line failed to open.** The
  startup path decoded projects as bare models only, so `demiurg hero.demiurg`
  exited with "this `.demiurg` holds a rig, not a bare model" for exactly the
  files File ▸ Open handled fine. The CLI now routes all three document kinds
  the way the menu does, and also accepts a `.rvc` clip.

## [0.12.1] - 2026-07-21

### Fixed

- **GPU backend crashed on startup when the egui font atlas arrived late.**
  `roxlap-gpu` skips egui work while no surface frame is pending, so the first
  scene upload could discard the font-atlas allocation even though egui
  considered it delivered; a later partial atlas update then panicked in
  `egui-wgpu` because the texture no longer existed. The editor now retains one
  up-to-date full image per egui texture and replays it before each partial
  update, so the UI renders on the GPU backend (`ROXLAP_GPU=1`) regardless of
  how long surface startup takes. Memory stays bounded to a single atlas copy
  (partials are folded into the retained full instead of kept as history). The
  CPU backend was unaffected.

## [0.12.0] - 2026-07-21

Track roxlap 0.30 and make the editor installable with Nix. No new editor
features - a dependency + packaging release. The editor renders and behaves
exactly as 0.11.0; the new emissive / dynamic-light hooks roxlap 0.30 adds are
left at their defaults.

### Changed

- **Updated to roxlap 0.30** (from 0.17). Tracks the colour-newtype family
  (`VoxColor` / `Rgb` / `OverlayColor`), the `#[non_exhaustive]` `FrameParams`
  built via `FrameParams::new`, `RenderOptions.backend: BackendPreference`
  (replacing `want_gpu`), the `emissive` field on `Material`, and the wider
  `render_scene_composed_with_materials` signature (dynamic-light rig + sprite
  occluder). No behavioural change to the editor; the new emissive/lighting
  hooks are left at their defaults.

### Added

- **Nix package + app output.** `nix profile add .` (or `nix build` / `nix run`)
  now installs the editor. The binary is wrapped with the runtime library path
  the render backends dlopen, so it runs outside a dev shell.

## [0.11.0] - 2026-06-28

Transparency: give voxels a blend mode and opacity, so models, clips, and
procedural effects can be glass, glowing energy, or soft smoke instead of solid.
Requires roxlap 0.17 (per-voxel clip materials + the volumetric blend mode).

### Added

- **Per-colour materials.** Each colour used in a model gets a blend mode and an
  opacity in the tool panel's new "Materials" section, composited live in the
  preview:
  - **Alpha** — front-to-back glass / water.
  - **Additive** — order-independent glow, for spells / fire / energy.
  - **Volumetric** — depth-weighted opacity, for soft smoke / fog.

  Opaque is the default, so an all-opaque model renders exactly as before.
  Material edits are undoable (a slider drag is one step) and saved in the
  `.demiurg` project.
- **Clip transparency.** A clip carries one material table shared by every
  frame; converting a translucent model to a clip keeps its materials. While a
  clip is **playing**, the preview runs through the engine's real clip player so
  the transparency matches what the game shows.
- **Translucent procedural presets.** The Rhai generator gains a
  `material(col, mode, alpha)` call so a script can declare its own materials —
  the **Smoke** preset is now volumetric fog and the **Energy** preset an
  additive-glow sphere.

### Notes

- Materials are kept in the lossless `.demiurg` project only; the `.kv6` and
  `.rvc` exports have no material channel yet, so they load back as opaque (the
  same boundary as enclosed interior voxels). Projects saved before this release
  load unchanged (all-opaque).

## [0.10.0] - 2026-06-28

Bring skeletal animation and voxel clips closer together: bake a rig into a
clip, and ease its motion.

### Added

- **Bake a rig's animation into a clip.** "Bake to clip" (Animate ▸ clips
  panel, with a frame-count field) samples a skeletal clip's poses and
  rasterizes every bone attachment — static mesh or animated clip layer — into a
  voxel-flipbook clip, opened as a new clip document to preview / tweak / export
  or drop back onto a bone. The bake matches the viewport exactly (same posing +
  voxel placement) and bounds the result to a fixed box (capped at 256³).
- **Keyframe easing.** A skeletal clip can interpolate with an easing curve —
  Linear, In, Out, or In-out — chosen per clip in the Animate panel, for smooth
  motion instead of the engine's robotic linear blend. The curve is saved with
  the rig (`.rkc` / `.demiurg`), shows live in the preview, and is carried into a
  bake.

### Fixed

- A bone whose primary attachment is an animated clip but that also carries a
  static-mesh extra no longer draws that extra twice in the posed-rig preview.

## [0.9.0] - 2026-06-27

Procedural clips: script a flame, smoke, or energy effect instead of sculpting
it frame by frame.

### Added

- **Procedural clip generator (Rhai).** A clip can be driven by an embedded
  [Rhai](https://rhai.rs) script that fills its frames — ideal for dynamic
  effects that are tedious to animate by hand:
  - **Imperative API** — the script body runs once per frame with globals
    (`frame`, `frames`, `t`, `w`/`h`/`d`) and flat helpers: `set` / `get`,
    `sphere`, `box`, `rgb` / `hsv`, a stable `noise` field, fractal `fbm`, and a
    per-frame `rand`.
  - **Presets** — Flame, Smoke, Energy, Plasma, and Sparkle, loadable as a
    starting point (and a live API example).
  - **Editing** — a floating, syntax-highlighted script window with an API
    cheat-sheet, frame-count and seed controls, and an **Auto** toggle that
    regenerates a short moment after edits settle for a near-live preview.
  - **Deterministic & safe** — the same seed yields the same clip; scripts are
    sandboxed (no I/O) and capped against runaway loops.
  - The script + parameters are saved in the `.demiurg` project; a procedural
    clip exports to `.rvc` (and drops onto bones as a clip layer) like any other.

## [0.8.0] - 2026-06-27

Animated voxel clips: author "GIF/MP4 for voxels" as their own documents, and
hang them off bones as animated layers.

### Added

- **Animated voxel clips (`.rvc`) — a new document type.** A clip is a flipbook
  of voxel frames, sculpted with the usual tools and rendered by the engine:
  - **Frames** — add / duplicate / delete / reorder, each with its own duration
    (or a clip-wide default), plus a loop mode (loop / once / ping-pong).
  - **Playback** — play / pause and step with `Space` / `,` / `.`, scrub a
    bottom frame timeline; the playhead and the edited frame are one, so you
    stop on any frame and sculpt it.
  - **Onion-skinning** — ghost the previous (cool) and next (warm) frames while
    sculpting, so motion registers; the ghosts are never picked or edited.
  - **Crop warning** — when the bounding box dwarfs the content the clip warns
    and offers a crop-to-content that tightens every frame at once.
  - **Files** — New clip, Open / Export `.rvc` (the engine's clip codec), and
    lossless `.demiurg` clip projects that keep every frame's interior voxels.
- **Clips as bone layers.** Any bone attachment — a bone's base mesh or an extra
  layer — can be an animated clip instead of a static mesh:
  - **Author in place** — "To clip" turns the current mesh into a clip (its
    first frame), "+ Clip layer" adds an empty one, and "Import .rvc" drops an
    authored clip onto the bone; selecting a clip layer brings up the full clip
    editor (frames, timeline, onion-skin) to sculpt it inside the rig.
  - **Plays on the posed rig** — a clip layer animates in the Animate preview
    along the rig's playhead, with per-layer **speed** and **phase** so the same
    clip can run fast on one bone and slow / offset on another.
  - **Round-trips** through `.rkc` (the engine character container) and
    `.demiurg`; clip layers show their frame count in the Layers list.
- **Select all** — `Ctrl+A` selects every occupied voxel of the current model
  and switches to the Select tool, ready to delete / copy / move.

## [0.7.0] - 2026-06-27

Layers: build a bone out of several meshes, each sculpted, placed and named on
its own.

### Added

- **Layers — multiple meshes per bone.** A bone can now carry several meshes (a
  base mesh plus extra layers), each one:
  - sculpted on its own — pick the active layer in the Layers panel and the
    voxel tools edit it;
  - placed by its own offset — translate / rotate / scale it numerically, or
    drag it against the posed bone with "Move layer" (Skeleton mode);
  - named — rename it in the panel (names are saved with the project);
  - drawn together in the Skeleton / Animate preview.
- **Extract to layer** — carve the current selection into a new layer on the
  same bone (it stays exactly where it was), the layer counterpart of "Extract
  to bone".
- **Copy / paste across bones and layers** — copying voxels from one mesh and
  pasting into another (a different bone, or a layer) now lands the paste inside
  the target instead of off its edge.

### Changed

- Bumped roxlap to **0.15.0** — its animated-voxel-clips character container and
  renderer are what layers build on (a bone's layers map to the engine's
  per-bone attachment list).

## [0.6.0] - 2026-06-23

Rigging quality-of-life: slice a model into a skeleton faster, place each bone's
pivot where it belongs, and bring reference art in from anywhere.

### Added

- **Extract a selection into a child bone** — in a rig's Sculpt mode, select a
  region and "Extract to bone" carves it out of the current mesh into a new child
  bone, left exactly where it was. Its joint defaults to the centre of the cut,
  so it rotates about where it joins (a shoulder / hip). The fast way to slice a
  whole model into a skeleton.
- **Edit a bone's mesh pivot in Skeleton mode** — the point a bone's mesh sits on
  and rotates about is now editable while the skeleton is in view: numeric X/Y/Z
  fields + Center, or a "Move pivot" toggle to drag the mesh against its joint.
- **Rotate the selection 90°** — a Rotate panel turns the selected voxels a
  quarter turn clockwise or counter-clockwise about a chosen X/Y/Z axis. The
  result floats so it can be nudged into place before it settles.
- **Paste a reference image from the clipboard** (`Ctrl+V`, or the Reference
  panel button) — copy an image in a browser or any app and paste it straight in
  as a tracing guide, since a browser can't drop one onto the window.
- **Scale a reference image** — a Scale control sizes a guide to the model
  without re-importing it.

### Fixed

- A bone's voxels could suddenly be replaced by another bone's (most visibly an
  extracted bone reverting to the one it was cut from) when the background
  autosave fired while a different bone was selected in Skeleton or Animate.

## [0.5.0] - 2026-06-22

Skeletal animation (preview): rig a model into bones and animate it with
keyframes, posed right in the viewport and saved to roxlap's `.rkc` rigged-
character format. Formats and UI may still change.

### Added

- **Skeletal animation editor** — build a rigged character (a skeleton of bones,
  each carrying its own voxel mesh) and animate it:
  - **Rig** a model: File ▸ New rig (one root bone) or Convert to rig (wrap the
    current model). Bones can be added, duplicated, reordered and deleted, with
    3-axis ball joints and a dummy root for full-body motion.
  - Three **sub-modes**: Sculpt (edit the active bone's mesh with the usual voxel
    tools), Skeleton (set each bone's joint / parent / rotation axis, or drag a
    bone in the viewport to place it), and Animate (preview and pose the clip).
  - **Pose in the viewport**: click a bone to select it, then left-drag to
    transform it on the selected keyframe — `R` / `G` / `S` switch the gizmo
    between rotate (trackball / ring), move and scale. Each keyframe stores a
    full per-bone transform (translation + rotation + scale).
  - **Timeline** (bottom bar): play / pause with `Space`, step keyframes with
    `,` / `.`, add / delete / copy / cut / paste keyframes, and drag a tick to
    retime it; a pose inspector edits the selected key's move / rotate / scale
    numerically.
  - **Clips** (left panel): add, rename and delete animation clips, set each
    clip's length, and toggle whether it loops.
  - **Export** to `.rkc` (File ▸ Export character); a `.demiurg` project stores
    the full rig too, and opening or dropping a `.rkc` loads it.
- **Recent files**: File ▸ Open recent reopens a recently used document, and the
  file dialog now remembers the last folder you used.
- The menu bar shows a build stamp — `demiurg <version> · <commit>` — so you
  can tell which version and git commit a binary was built from (selectable to
  copy into a bug report). Source-tarball builds with no git show `unknown`.

### Changed

- Bumped roxlap to 0.13.0, which adds the `roxlap_formats::character` rigged-
  character container (per-keyframe translation + rotation + scale) the
  animation editor builds on.

### Fixed

- Editing a rig or animation now marks the project unsaved (the title `*` and
  the quit guard), so posing / keyframing work can't be lost by quitting a
  document that still looked saved.

## [0.4.0] - 2026-06-16

Reference images: trace voxels over loaded pixel art, drawn in the viewport as a
flat, depth-tested guide on roxlap 0.12's world-placed image sprites.

### Added

- Reference images: load pixel art (PNG / BMP / JPG / GIF / TGA / WEBP) as a
  flat guide to trace voxels from — via File ▸ Open reference image or by
  dragging an image onto the window. It's non-destructive (never
  saved/exported/edited): place it on the Front / Side / Top plane, set its
  depth, flip it, hide it, or remove it. The Reference panel's **Move** toggle
  lets you drag it into position on the grid with the mouse (left-drag slides it
  in its plane, whole-voxel snap). It's drawn as a flat, world-placed image
  sprite (roxlap 0.12 `draw_images`), so the model occludes the parts behind it
  and it stays undistorted from any angle, with an **Opacity** slider to dim a
  too-bright reference to a faint guide. The eyedropper (the tool, or `Ctrl`
  +click from any tool) picks colours straight off the reference image —
  whichever of the model voxel or the reference is nearer the cursor wins.
  Dropping a `.kv6` / `.vox` / `.demiurg` file opens it as the model. The tool
  panel now scrolls so every section stays reachable.

### Changed

- Bumped roxlap to 0.12.0, for the world-placed 2D image sprites
  (`SceneRenderer::upload_image` / `draw_images`) that draw reference images.

## [0.3.0] - 2026-06-15

MagicaVoxel `.vox` interop, and the CPU renderer is now the default to dodge a
Windows GPU-init hang.

### Added

- MagicaVoxel `.vox` import and export (File ▸ Open .vox / Export .vox, and a
  `.vox` path argument). Import uses the `dot_vox` parser (handles real-world
  files) and takes the first model; export writes a single model. The height
  axis is flipped between MagicaVoxel's z-up and demiurg's z-down so models stay
  upright, and colours map through a 256-entry palette. `.vox` has no pivot, so
  import centres it.

### Fixed

- The 0.2.0 white-window fix wasn't enough: on some Windows GPUs/drivers wgpu
  *device creation itself* hangs (before the first frame), which a synchronous
  call can't be timed out of. The CPU renderer is now the **default** (reliable
  everywhere); the GPU backend is opt-in via `--gpu` or `ROXLAP_GPU=1` (`--cpu` /
  `ROXLAP_GPU=0` force CPU).

## [0.2.0] - 2026-06-15

Editing and save quality-of-life: voxel-edge readability, a proper Save / Save
As flow, non-blocking file I/O, and crash-recovery autosave.

### Added

- Voxel-edge overlay (View ▸ Voxel edges, on by default): a light wireframe on
  exposed voxel faces so boundaries read even on flat-shaded faces in shadow,
  where coplanar voxels would otherwise blend into one patch (there is no
  ambient occlusion / light baking).
- Save / Save As for the project: `Ctrl+S` overwrites the open `.demiurg` file
  without a dialog once its path is known; Save As picks a new path. The kv6 and
  vxl menu entries are now labelled Export.
- File I/O no longer freezes the window (which the OS would flag as hung and
  offer to kill, losing the model): the open/save **dialogs** run on a worker
  thread off the event loop, and **saves** serialize/write on a worker thread
  too, with a "Saving…" spinner.
- Background autosave: while there are unsaved changes the project is snapshotted
  to the OS temp dir every 20 s; on the next launch a surviving autosave (after a
  crash) is loaded automatically with a "Recovered work" banner. A clean exit
  removes it.

### Fixed

- Startup could open a white, frozen window on some Windows GPUs/drivers/remote
  sessions: the forced Fifo (vsync) present mode could stall `present()`
  indefinitely. Present is now uncapped — the ~60 fps frame timer already caps
  GPU load — and `--cpu` was added as an escape hatch (alongside `ROXLAP_GPU=0`)
  when GPU device creation itself hangs.
- Place tool: when the cursor ray hits no voxel it now falls back to the model's
  floor (the volume's bottom face), so you can seed voxels — and rebuild a model
  emptied of its last voxel — instead of having nothing to click.

## [0.1.0] - 2026-06-15

First release for artists — a working native voxel **model** editor (DESIGN.md
milestone M2). The viewport is rendered by the roxlap engine itself, so what you
paint is what the game shows.

### Added

- **Editing tools**: place, erase, paint, eyedropper, box (2 clicks), sphere
  (radius), and flood fill. Paint drag-strokes coalesce into one undo step.
- **Selection** (Select tool): click or screen-rectangle marquee, with `Shift`
  to add and `Alt` to remove. `Ctrl`+click is a quick eyedropper from any tool.
- **Selection operations**: delete, copy, and paste. Paste drops a floating
  layer at the source position; it is written into the model (one undo step)
  only when deselected, so it never clobbers what is underneath.
- **Move**: drag a selected voxel's face to slide the selection in that face's
  plane, in whole voxels, leaving the model untouched until commit.
- **Model sizing**: crop to content, resize to exact dimensions, grow by one
  voxel per direction, edit and centre the pivot.
- **Palette**: colour picker, preset swatches, and a "colours in model" row;
  mirror planes (X/Y/Z) for symmetric edits.
- **Camera**: orbit, pan (middle-mouse or `Shift`+right drag), zoom, `Home`
  recenter, and six axis-aligned view presets (panel buttons or numpad
  `1`/`3`/`7`, `Ctrl` for the opposite face).
- **Rendering**: engine preview as a kv6 sprite (WYSIWYG) or a side-shaded
  voxel grid; GPU backend by default with a CPU fallback (`ROXLAP_GPU=0`);
  ~60 fps vsync cap so an idle scene doesn't peg the GPU.
- **Files**: lossless `.demiurg` project save/load, plus export to `.kv6`
  (engine sprite) and `.vxl` (voxlap world).
- **Localisation**: English and Russian UI (`DEMIURG_LANG=ru`).
- Undo/redo for every edit, with an unsaved-changes guard on quit.

### Notes

- Built against roxlap 0.9.0.
- The browser/WASM build (M3), `.kfa` animation (M4), and voxel-video (M5) are
  designed but not yet implemented — see DESIGN.md §9.

[Unreleased]: https://github.com/NCrashed/demiurg/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/NCrashed/demiurg/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/NCrashed/demiurg/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/NCrashed/demiurg/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/NCrashed/demiurg/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/NCrashed/demiurg/releases/tag/v0.1.0
