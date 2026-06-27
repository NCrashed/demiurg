# Changelog

All notable changes to demiurg are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/). The release CI extracts the section
matching a `vX.Y.Z` tag as the GitHub release notes.

## [Unreleased]

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
