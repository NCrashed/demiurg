# demiurg

Voxel **asset editor** for the [roxlap](../roxlap) voxel engine and the
[monada](../monada) game framework.

Author voxel **models** (`.kv6`), **skeletal animations** (`.kfa`), and
**voxel-video** clips (`.vvid`), and preview them **rendered by the actual
engine** — the viewport *is* roxlap. Runs natively and as WASM in the browser
from one codebase, and saves directly into the engine's formats.

First target: the piece set, board, and animations for **monada chess 2.0**.

See [DESIGN.md](./DESIGN.md) for architecture.

## Status

Design phase. Implementation starts at M0 (workspace skeleton + `demiurg-core`
kv6 round-trip). See DESIGN.md §9 for the roadmap.

## Layout (planned)

```
demiurg-core    document model, edit commands, undo/redo, format conversion (no UI)
demiurg-view    viewport: roxlap SceneRenderer wrapper, camera, picking
demiurg-ui      egui panels: tools, palette, timeline
demiurg-app     native binary (winit + egui over roxlap framebuffer)
demiurg-web     wasm32 build (trunk + canvas)
demiurg-cli     (optional) headless asset pipeline
```

## Dependencies

roxlap only — `roxlap-formats`, `roxlap-render`, `roxlap-scene`, `roxlap-core`,
and `roxlap-voxvideo` (the `.vvid` codec, new). No monada dependency.

## License

MIT OR Apache-2.0
