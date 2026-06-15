# demiurg

Voxel **asset editor** for the [roxlap](https://github.com/NCrashed/roxlap) voxel
engine and the monada game framework.

Author voxel **models** and preview them **rendered by the actual engine** — the
viewport *is* roxlap, so what you paint is byte-for-byte what the game shows.
Save directly into the engine's formats. Native today; the browser/WASM build is
designed but not yet shipped (one codebase, see [DESIGN.md](./DESIGN.md)).

First target: the piece set, board, and animations for **monada chess 2.0**.

## Status

**v0.1.0 — first release for artists.** A working native model editor
(DESIGN.md milestone M2): tools, selection + move, palette, mirror, pivot,
resizing, undo/redo, project save and engine-format export. Animation (`.kfa`,
M4) and voxel-video (`.vvid`, M5) are next. See the
[CHANGELOG](./CHANGELOG.md).

## Install

Pre-built **Windows** binaries are attached to each
[release](https://github.com/NCrashed/demiurg/releases). Download
`demiurg-<version>-windows-x64.exe` and run it.

## Build from source

The toolchain is pinned in `rust-toolchain.toml` (a nightly shared with roxlap
for the future wasm-threads path; native builds behave like stable).

With [Nix](https://nixos.org) (provides the toolchain and the Linux render libs):

```sh
nix develop --command cargo run -p demiurg-app -- model.kv6
```

Or with a matching rustup toolchain (rustup auto-installs the pinned nightly):

```sh
cargo run -p demiurg-app -- model.kv6      # or no path for a blank canvas
```

On Linux the viewport needs the usual windowing/render libs (`libxkbcommon`,
`wayland`, X11, `vulkan-loader`); the Nix devshell supplies them.

## Usage

```
demiurg [path.kv6 | path.demiurg]    # no path -> a blank canvas
```

- **Tools** `1`–`8`: place, erase, paint, eyedropper, box, sphere, flood fill,
  select. Left mouse applies the tool; `Ctrl`+click eyedrops a colour.
- **Select** (`8`): click or drag a marquee; `Shift` adds, `Alt` removes. Drag a
  selected voxel's face to move the selection; `Delete`, `Ctrl+C`/`Ctrl+V`
  copy/paste, `Esc` deselects.
- **Camera**: right-drag orbits, middle-drag (or `Shift`+right) pans, wheel /
  `W`/`S` zoom, `Home` recenters; the **Views** panel and numpad `1`/`3`/`7`
  snap to axis views (`Ctrl` for the opposite face).
- **Edit**: `Ctrl+Z` undo, `Ctrl+Y` / `Ctrl+Shift+Z` redo.
- **Render**: GPU by default; `ROXLAP_GPU=0` forces the CPU renderer. Switch
  sprite/voxel preview in the View menu.
- **Language**: `DEMIURG_LANG=ru`, or the Language menu (English / Русский).

## Layout

```
demiurg-core    document model, edit commands, undo/redo, format conversion (no UI)
demiurg-i18n    UI message catalogue + translations (no_std, no deps)
demiurg-view    viewport: roxlap SceneRenderer bridge, orbit camera, picking
demiurg-app     native binary (winit + egui over the roxlap framebuffer)
```

## Formats

- `.demiurg` — lossless editor project (the source of truth).
- `.kv6` — engine sprite export (surface voxels; how monada draws pieces).
- `.vxl` — voxlap world export.

## Dependencies

roxlap only — `roxlap-formats`, `roxlap-render`, `roxlap-scene`, `roxlap-core`
(crates.io 0.9.0). No monada dependency. See [DESIGN.md](./DESIGN.md) for the
architecture and roadmap.

## License

MIT OR Apache-2.0
