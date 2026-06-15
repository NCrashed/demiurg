# Changelog

All notable changes to demiurg are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/). The release CI extracts the section
matching a `vX.Y.Z` tag as the GitHub release notes.

## [Unreleased]

### Added

- Reference images: load pixel art (PNG / BMP / JPG / GIF / TGA / WEBP) as a
  flat, 1-voxel-thick guide to trace voxels from — via File ▸ Open reference
  image or by dragging an image onto the window. It's non-destructive (a
  separate render layer, never saved/exported/edited): place it on the Front /
  Side / Top plane, set its depth, flip it, hide it, or remove it. The Reference
  panel's **Move** toggle lets you drag it into position on the grid with the
  mouse (left-drag slides it in its plane, whole-voxel snap). Dropping a
  `.kv6` / `.vox` / `.demiurg` file opens it as the model. The tool panel now
  scrolls so every section stays reachable.

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

[Unreleased]: https://github.com/NCrashed/demiurg/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/NCrashed/demiurg/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/NCrashed/demiurg/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/NCrashed/demiurg/releases/tag/v0.1.0
