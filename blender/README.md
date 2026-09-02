# demiurg rig export (Blender addon)

Voxelizes an armature's per-bone meshes and writes a `.demiurg` project.

The addon writes no binary formats itself. It voxelizes, works out the
skeleton, and emits the JSON exchange manifest that **`demiurg-convert`** turns
into a document — the same shape Voxelity Pro uses when it shells out to
`vengi-voxconvert`. The wire formats stay written once, in Rust, and the addon
can't drift out of step with them.

## Install

**From a release zip** — one file, nothing to configure:

*Edit ▸ Preferences ▸ Add-ons ▸ Install from Disk* → pick
`demiurg_export-<version>-<platform>.zip` → enable it. The converter it needs
is inside the zip; its preferences should say *using the bundled … build*.

Blender 4.2+ installs it as an extension, older builds as a legacy add-on.

**From the repo**, if you're working on demiurg itself:

```sh
cargo build --release -p demiurg-convert
cd blender && zip -r ~/demiurg_export.zip demiurg_export
```

Install that the same way, then point **demiurg-convert** in its preferences at
`target/release/demiurg-convert` — an explicit path wins over the bundled
binary, so your own build is what runs. Leaving it blank falls back to the
bundled one, then to `PATH`.

## Giving it to an artist

**From CI**, for a zip covering every platform. Each converter is compiled on
its own runner — Linux, Windows, and both Mac architectures — so nothing is
cross-compiled and nothing is missing.

| Where | What you get |
| --- | --- |
| Tag `vX.Y.Z` | Attached to the GitHub release, its version stamped from the tag |
| *Blender addon* workflow ▸ **Run workflow** | The same zip off any branch, as a run artifact |
| Any push or PR touching the addon | Built and checked, so a broken package fails the PR rather than the release |

**Locally**, when you want it now and know who it's for:

```sh
scripts/package-blender-addon.sh --cross windows
```

Builds the converter for this machine *and* for Windows, lays both inside the
addon under `bin/<platform>/`, and writes `dist/demiurg_export-<version>.zip`.
Send that one file; the addon picks the binary for whatever the artist is on.

Drop `--cross windows` for a host-only zip (then the file name carries the
platform, so a folder of releases stays readable).

The Linux converter is linked **statically against musl**. A glibc build
carries an ELF interpreter path that NixOS does not have — it refuses with
*"cannot run dynamically linked executables intended for generic linux
environments"* — and a glibc version floor an older distro would trip over.
Neither applies to a static binary, so the machine that built it stops
mattering.

The Windows build is a real cross-compile from Linux: the target's std comes
from the pinned toolchain, the linker from mingw-w64, and everything links
statically, so the `.exe` imports nothing but system DLLs — no runtime
redistributable for the artist to install. macOS has no cross path from Linux;
build the converter on a Mac and fold it in (or let CI do it):

```sh
scripts/package-blender-addon.sh --with-bin macos-arm64=/path/to/demiurg-convert
```

Platform folders are named `<system>-<arch>` — `linux-x86_64`,
`windows-x86_64`, `macos-arm64` — matching what
[`bundle.platform_tag`](demiurg_export/bundle.py) computes at runtime. Several
can live in one zip.

## Exporting

Select the armature, then *File ▸ Export ▸ demiurg rig (.demiurg)*. Name the
file `.rkc` instead to write the engine character directly.

| Option | What it does |
| --- | --- |
| **Voxels per unit** | Resolution: how many voxels one Blender unit becomes. 10 means a 2 m character is 20 voxels tall. |
| **Fill interior** | Fill inside the mesh, not just its surface. Needs closed geometry — turn it off for open or non-manifold meshes. |
| **Animation** | Bake actions into clips. |
| **Detect deforming meshes** | Bake a mesh that squashes as a per-frame flipbook, since bones cannot carry one. See [Meshes that deform](#meshes-that-deform). |
| **All actions** | Export every action in the file, not just the ones this armature uses. |
| **Keep manifest** | Write the intermediate JSON next to the output. The first thing to look at when a result surprises you. |

Selecting a plain mesh with no armature exports it as a bare model — worth
using over the `.vox` route, which carries no pivot.

## What the scene has to look like

Either way of attaching a mesh works, and they can be mixed:

**One mesh per bone, parented to it** (select the mesh, then the armature,
*Ctrl+P ▸ Bone*). The engine draws each bone as one rigid sprite, so this maps
straight across, and it is how voxel characters are usually built anyway.

Several meshes on one bone are fine: the first by name becomes the bone's own
mesh and the rest become **layers** — separate attachments, each keeping a grid
of its own size and placed by an offset. That matters for anything held away
from its bone: folding a sword into the hand's mesh would stretch that grid to
span both, most of it empty. A layer takes the object's name, and its origin
becomes the pivot it rotates about.

**One skinned mesh, bound with an armature modifier.** Since a bone is drawn as
one rigid sprite, there is no such thing as a voxel shared between two bones:
the mesh is **cut up**, each voxel going to the bone that weighs most on the
triangle nearest to it. Everything is kept — geometry the artist never weighted
goes to the closest bone, with a warning saying how much.

The cut is rigid, and that is the catch: **at a joint that bends far, the two
chunks pull apart on the outside and overlap on the inside.** Nothing in the
exporter can fix it, because the format has nowhere to put a voxel that is half
one bone and half another. Modelling each limb as its own object sidesteps the
question; so does keeping bends modest.

Geometry is always read at the **rest pose**, whatever frame the timeline sits
on — the pose belongs in the clips, and baking it into the voxels as well would
apply it twice. The exception is a mesh marked to deform, below, whose whole
point is that its geometry changes.

## Meshes that deform

A skeleton moves rigid chunks; it cannot reshape one. Anything that *squashes*
— a slime, a wobbling blob, cloth — needs its geometry voxelized **every
frame** instead, and the engine plays the result as a flipbook.

**The export usually works this out on its own.** With **Detect deforming
meshes** on (the default) it watches each mesh over two frames and bakes it as a
flipbook when it changes in a way bones cannot carry — its vertex count varies,
or it keeps moving with the armature held at rest. It says which meshes it did
that to, so the decision is visible rather than silent. Untick the option to
force everything rigid.

Tick **Voxelize per frame** in *Object Properties ▸ demiurg* to state it
outright instead of relying on the test. Either way it works whichever way the
mesh is attached:

| How the mesh is attached | What the flag does |
| --- | --- |
| Nothing (no armature) | Exports as a one-bone rig that is nothing but the flipbook |
| Parented to a bone | That bone plays the flipbook; the rest of the skeleton stays rigid |
| Armature modifier + weights | Baked whole onto the root bone — the deformation is already in the frames, so the skeleton no longer moves it |

So a hard-shelled character with a soft belly is one export.

**If a Geometry Nodes modifier rebuilds the mesh** — a voxelizer, a remesh, a
scatter — the vertex groups do not survive it. The evaluated mesh reaches the
exporter with no weights at all, the per-bone split has nothing to go on, and
the result is a nonsense division across bones. The export says so by name when
it happens; the answer is the flipbook path, which does not need weights. (This
is also the case detection catches for you: a voxelizer downstream of an
armature changes its vertex count as the bones squash it.)

**If something else already voxelized the mesh**, match **Voxels per unit** to
its voxel size (`1 / size`) or the two grids fight: a coarser export grid
samples one block in every two or four and chews the edges. The export measures
the mesh's own lattice and tells you the number to use. Or drop the other
voxelizer and let this one do it once.

The deformation can come from anywhere Blender evaluates — shape keys, a
lattice, a cloth sim, drivers — because the exporter samples the result rather
than reading any one mechanism.

| Setting | What it does |
| --- | --- |
| **Detect deforming meshes** (export option) | Finds them itself; on by default |
| **Voxelize per frame** (per object) | Marks the mesh as a flipbook outright |
| **Clip fps** (export option) | How often to sample it |

**Clip fps is the size knob.** Every sample is a whole voxel grid, so this
decides what a deforming mesh costs — 12 fps reads as smooth for most motion
and stores half of what 24 does. It is deliberately independent of the scene's
frame rate: sampling every rendered frame usually spends file size on motion
the eye can't tell apart.

### Deforming differently per action

A bone holds one flipbook, and the rig's playhead is what picks a frame from
it. So when a rig has both a deforming bone and more than one action, the
exporter lays the actions on **separate windows of one timeline** — `walk` on
0…1000 ms, `idle` on 1000…1800 — and concatenates the flipbook to cover both.
Each action then reaches its own geometry, and still cycles inside its own
window, because a clip's loop marker returns to its own first key rather than
to zero.

The export warns when it does this. The visible consequences: a clip's
keyframes no longer start at 0 (the editor's timeline shows `idle` beginning at
1000 ms), and **the host must seek to a clip's own start** when it switches
clips — automatic from the second cycle on, but not for the first. `--shot` and
`--dump-pose` take `--time` relative to the chosen clip, so the layout stays
invisible there.

For the geometry to actually differ per action, the deformation has to *depend
on* what the action drives — a driver reading a bone, a lattice or cloth the
armature moves. A shape key animated on the mesh by itself evaluates the same
under every action, so every window would hold the same shapes.

Rigs without a deforming bone, or with a single action, are laid out as before,
starting at 0.

The bake covers the **scene frame range** with whatever animation is active
(or each action's own range, when windowed), and the last frame is left out as
a loop's duplicate of the first.

If a bone's mesh is marked to deform, that replaces its rigid geometry rather
than adding to it: the engine draws one thing per bone.

Bones with no mesh export as empty — fine for a root or control bone, and
reported so a forgotten mesh doesn't pass silently.

## Animation

Actions become clips, **baked one key per frame**. The clip format stores
whole-skeleton poses interpolated linearly, with no per-bone curves, so a
Bezier F-curve could not survive any other way. Nothing is read out of the
action's channels either: each action is assigned in turn and the scene is
stepped frame by frame, so constraints, drivers, and however the running
Blender version stores an action all arrive already evaluated.

Which actions get exported: the armature's **active action** plus any its
**NLA strips** reference. A file with neither — a pose keyed and never put on a
track — falls back to every action in the file, with a warning saying so. Tick
**All actions** to force that.

An action whose last frame repeats its first has that key dropped and its
length shortened to match, so a cycle authored the usual way loops without a
one-frame stutter.

**A root bone cannot be animated.** The engine hands a root the sprite's own
transform and ignores its keyframe, so the converter rejects it by name rather
than exporting a rig that quietly doesn't move. Give the skeleton a dummy root
above whatever you want to animate — Blender's usual `root` / COG bone is
exactly that.

Scale is exported from the bone's own scale channel. Non-uniform scale
combined with rotation is approximate; uniform scale is exact.

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
- **Transparency and glow come from the same node**, read in this order:

  | Principled input | Exported as |
  | --- | --- |
  | **Alpha** below 1 | `blend` — front-to-back compositing, for glass and slime |
  | **Transmission** above 0 | `blend`, multiplied into the alpha above |
  | **Emission Strength** above 0 on a solid material | `add` — order-independent glow |

  Alpha and transmission **stack**, because they say different things: alpha is
  how much of the surface is there, transmission how much light passes through
  what is there, so what reaches the eye is `alpha × (1 - transmission)`. A
  slime at alpha 0.86 and transmission 0.5 exports at 110/255, not 220.

  Emission only applies when neither of the other two did — the engine has one
  blend mode per colour, so a material that is both translucent and glowing has
  to be one or the other. Roughness, metallic, and specular have no equivalent
  and are ignored.

  **Materials are keyed by colour, not by material.** Two Blender materials
  sharing a base colour cannot composite differently — the renderer indexes by
  colour. The export warns and keeps the first.
- **Modifiers are applied.** The evaluated mesh is exported, so a mirror or
  subsurf you forgot to apply still counts.

## Not exported yet

Textures — a textured mesh exports as its material's flat colour.

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

# cuts a smoothly weighted mesh in two and checks where the seam landed
blender --background --python blender/tests/headless_skin.py -- \
    --out /tmp/skinned.demiurg --converter ./target/debug/demiurg-convert

# bakes a squashing blob and checks the frames really differ, at the asked rate
blender --background --python blender/tests/headless_deform.py -- \
    --out /tmp/slime.demiurg --converter ./target/debug/demiurg-convert --fps 8

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
demiurg /tmp/hero.demiurg --shot /tmp/pose.png --clip wave --time 500 --dist 24
```

For animation, check it against Blender numerically instead. Export with
`--expected`, which writes where Blender puts every bone on every frame, then
compare that against the solver:

```sh
blender --background --python blender/tests/headless_export.py -- \
    --out /tmp/hero.demiurg --converter ./target/debug/demiurg-convert \
    --expected /tmp/expected.json
python3 blender/tests/compare_poses.py --expected /tmp/expected.json \
    --rig /tmp/hero.demiurg --demiurg ./target/debug/demiurg
```

It reports the worst disagreement in voxels and names any bone that drifts — a
mirrored quaternion or a bad joint shows up as a number instead of a render
you have to squint at. (`demiurg --dump-pose` is what it reads; that flag is
useful on its own when a limb ends up somewhere unexpected.)
