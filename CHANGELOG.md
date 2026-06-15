# Changelog

All notable changes to demiurg are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows
[Semantic Versioning](https://semver.org/). The release CI extracts the section
matching a `vX.Y.Z` tag as the GitHub release notes.

## [Unreleased]

### Added

- Voxel-edge overlay (View ▸ Voxel edges, on by default): a light wireframe on
  exposed voxel faces so boundaries read even on flat-shaded faces in shadow,
  where coplanar voxels would otherwise blend into one patch (there is no
  ambient occlusion / light baking).

### Fixed

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

[Unreleased]: https://github.com/NCrashed/demiurg/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/NCrashed/demiurg/releases/tag/v0.1.0
