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

**From CI**, for a zip covering every platform. Tagging `vX.Y.Z` attaches it to
the GitHub release; the *Release* workflow's **Run workflow** button builds the
same zip off any branch and leaves it as a run artifact, which is what to use
when someone needs a build of current master. Each converter is compiled on its
own runner — Linux, Windows, and both Mac architectures — so nothing is
cross-compiled and nothing is missing.

**Locally**, when you want it now and know who it's for:

```sh
scripts/package-blender-addon.sh --cross windows
```

Builds the converter for this machine *and* for Windows, lays both inside the
addon under `bin/<platform>/`, and writes `dist/demiurg_export-<version>.zip`.
Send that one file; the addon picks the binary for whatever the artist is on.

Drop `--cross windows` for a host-only zip (then the file name carries the
platform, so a folder of releases stays readable).

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
| **All actions** | Export every action in the file, not just the ones this armature uses. |
| **Keep manifest** | Write the intermediate JSON next to the output. The first thing to look at when a result surprises you. |

Selecting a plain mesh with no armature exports it as a bare model — worth
using over the `.vox` route, which carries no pivot.

## What the scene has to look like

Either way of attaching a mesh works, and they can be mixed:

**One mesh per bone, parented to it** (select the mesh, then the armature,
*Ctrl+P ▸ Bone*). The engine draws each bone as one rigid sprite, so this maps
straight across, and it is how voxel characters are usually built anyway.

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
apply it twice.

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
- **Modifiers are applied.** The evaluated mesh is exported, so a mirror or
  subsurf you forgot to apply still counts.

## Not exported yet

Extra attachment layers and transparency materials.

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
