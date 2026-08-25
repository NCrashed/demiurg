# demiurg rig export (Blender addon)

Voxelizes an armature's per-bone meshes and writes a `.demiurg` project.

The addon writes no binary formats itself. It voxelizes, works out the
skeleton, and emits the JSON exchange manifest that **`demiurg-convert`** turns
into a document — the same shape Voxelity Pro uses when it shells out to
`vengi-voxconvert`. The wire formats stay written once, in Rust, and the addon
can't drift out of step with them.

## Install

Build the converter first — the addon runs it:

```sh
cargo build --release -p demiurg-convert     # target/release/demiurg-convert
```

Then zip the addon and install it (Blender 4.2+ takes it as an extension; older
builds as a legacy add-on):

```sh
cd blender && zip -r ~/demiurg_export.zip demiurg_export
```

*Edit ▸ Preferences ▸ Add-ons ▸ Install from Disk* → pick the zip → enable it.
Expand its preferences and point **demiurg-convert** at the binary, or leave it
blank if it's on `PATH`.

## Exporting

Select the armature, then *File ▸ Export ▸ demiurg rig (.demiurg)*. Name the
file `.rkc` instead to write the engine character directly.

| Option | What it does |
| --- | --- |
| **Voxels per unit** | Resolution: how many voxels one Blender unit becomes. 10 means a 2 m character is 20 voxels tall. |
| **Fill interior** | Fill inside the mesh, not just its surface. Needs closed geometry — turn it off for open or non-manifold meshes. |
| **Keep manifest** | Write the intermediate JSON next to the output. The first thing to look at when a result surprises you. |

Selecting a plain mesh with no armature exports it as a bare model — worth
using over the `.vox` route, which carries no pivot.

## What the scene has to look like

**One mesh object per bone, parented to that bone** (select the mesh, then the
armature, *Ctrl+P ▸ Bone*). Each bone is drawn by the engine as one rigid
sprite, so that is the unit of geometry.

A mesh deformed by an *armature modifier* is **skipped, with a warning**. Its
weights are spread smoothly across several bones, and cutting such a mesh into
rigid per-bone chunks is a separate job — see the roadmap below. Voxel
characters are usually built as separate objects anyway, which is exactly what
this wants.

Bones with no mesh export as empty — fine for a root or control bone, and
reported so a forgotten mesh doesn't pass silently.

## Conventions worth knowing

- **+Z flips.** Blender is Z-up; demiurg (voxlap) is Z-down. The exporter does
  the flip, so a model comes out upright — but it is why a hand-written
  manifest puts the head at *low* z.
- **A bone's pivot is its joint.** Each bone's mesh pivot goes at the bone
  head, and a child's `joint` is the offset between the two heads.
- **Meshes are voxelized in armature space**, not bone-local space: the
  manifest has no per-bone rest rotation, so a bone's rest orientation lives in
  its voxels.
- **Colour comes from the Principled BSDF's Base Color**, falling back to the
  viewport display colour for a material with no nodes. Textures are not
  sampled — a textured mesh exports flat. Colours are converted linear → sRGB,
  so what you see in the picker is what lands in the file.
- **Modifiers are applied.** The evaluated mesh is exported, so a mirror or
  subsurf you forgot to apply still counts.

## Not exported yet

Animation, skinned meshes, extra attachment layers, and transparency
materials. Rest pose and geometry only, for now.

## Testing

The Blender-free parts run under any interpreter:

```sh
python3 blender/tests/test_pure.py
```

The rest needs Blender, headless:

```sh
# builds a two-bone scene and exports it
blender --background --python blender/tests/headless_export.py -- \
    --out /tmp/hero.demiurg --converter ./target/debug/demiurg-convert

# installs the zip and drives the real operator
cd blender && zip -qr /tmp/demiurg_export.zip demiurg_export && cd ..
blender --background --python blender/tests/headless_install.py -- \
    --zip /tmp/demiurg_export.zip --out /tmp/hero.demiurg \
    --converter ./target/debug/demiurg-convert
```

Both print `RESULT: OK` or `RESULT: FAIL` — Blender swallows a script's exit
code, so that marker line is the verdict.

Then look at what came out, without opening the editor:

```sh
demiurg /tmp/hero.demiurg --shot /tmp/pose.png --yaw 1.2 --dist 24
```
