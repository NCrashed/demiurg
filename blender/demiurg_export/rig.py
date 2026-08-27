"""Assemble the JSON exchange manifest `demiurg-convert` consumes.

Pure — everything here takes plain numbers and dicts, so the manifest layout
can be tested without Blender. See `crates/demiurg-convert/src/manifest.rs` for
the schema this has to match, and `examples/two-bone-wave.json` for a
hand-written one.

Two conventions are easy to get wrong and are settled here:

* A bone's **mesh pivot is its joint**, so each bone's pivot goes at its head.
* A child's `joint` is measured from the **parent's pivot** — not from the
  parent grid's corner — so it is just the difference of the two heads in voxel
  space.

Meshes are voxelized in **armature space**, not bone-local space. The manifest
has no per-bone rest rotation (the converter builds hinges whose rest is the
identity), so a bone's rest orientation has to live in its voxels; keeping
every bone in one shared frame does that for free and makes the joints plain
subtractions.
"""

FORMAT_RIG = "demiurg-rig"
FORMAT_MODEL = "demiurg-model"
VERSION = 1


def voxel_list(voxels):
    """`{(i, j, k): "rrggbb"}` as the manifest's `[[x, y, z, "rrggbb"], ...]`,
    in a stable order so re-exporting an unchanged scene produces an identical
    file (diffable, and kind to version control)."""
    return [[i, j, k, color] for (i, j, k), color in sorted(voxels.items())]


def mesh_entry(dims, pivot, voxels):
    """A manifest `mesh`."""
    return {
        "dims": [int(d) for d in dims],
        "pivot": [round(float(p), 4) for p in pivot],
        "voxels": voxel_list(voxels),
    }


def bone_entry(name, parent, joint, mesh=None):
    """A manifest `bone`. `parent` is a bone name or `None` for a root; `joint`
    is the head offset from the parent's pivot, in voxels."""
    entry = {"name": name}
    if parent is not None:
        entry["parent"] = parent
    entry["joint"] = [round(float(c), 4) for c in joint]
    if mesh is not None:
        entry["mesh"] = mesh
    return entry


def joint_offset(child_head_vox, parent_head_vox):
    """A child bone's `joint`: where its head sits relative to its parent's."""
    if parent_head_vox is None:
        return (0.0, 0.0, 0.0)
    return tuple(child_head_vox[i] - parent_head_vox[i] for i in range(3))


def xform_entry(translation=None, rotation=None, scale=None, places=5):
    """A bone's transform at a keyframe, with anything at its default left
    out — a bone that only turns writes `{"r": [...]}`. Rounded, because five
    decimals of a voxel is far past what the renderer can show and full f64
    noise would make every re-export a diff."""
    entry = {}
    if translation is not None and any(abs(c) > 1e-6 for c in translation):
        entry["t"] = [round(float(c), places) for c in translation]
    if rotation is not None and abs(abs(float(rotation[3])) - 1.0) > 1e-6:
        entry["r"] = [round(float(c), places) for c in rotation]
    if scale is not None and any(abs(float(c) - 1.0) > 1e-6 for c in scale):
        entry["s"] = [round(float(c), places) for c in scale]
    return entry


def key_entry(time_ms, pose):
    """One keyframe. `pose` maps bone name to [`xform_entry`]; bones left out
    are at rest, which is what the format stores anyway (whole-skeleton poses,
    not per-bone deltas)."""
    return {"t": int(time_ms), "pose": pose}


def clip_entry(name, keys, length_ms, loops=True):
    """One animation clip."""
    return {
        "name": name,
        "loop": bool(loops),
        "length_ms": int(length_ms),
        "keys": keys,
    }


def rig_manifest(name, bones, clips=None, comment=None):
    """The whole document: a `demiurg-rig` manifest around `bones`."""
    doc = {
        "format": FORMAT_RIG,
        "version": VERSION,
        "name": name,
        "root": [0.0, 0.0, 0.0],
        "bones": bones,
    }
    if clips:
        doc["clips"] = clips
    if comment:
        doc["_comment"] = comment
    return doc


def model_manifest(name, mesh, comment=None):
    """A bare model — what a lone mesh with no armature exports as. Worth
    having over the `.vox` route because `.vox` carries no pivot."""
    doc = {"format": FORMAT_MODEL, "version": VERSION, "name": name, "mesh": mesh}
    if comment:
        doc["_comment"] = comment
    return doc


def sorted_bones(names, parent_of):
    """Bone names with every parent ahead of its children.

    The converter accepts any order, but a parents-first file reads like the
    skeleton it describes, and the editor lists bones in file order. Bones
    whose parent chain is broken keep their relative order at the end rather
    than vanishing.
    """
    remaining = list(names)
    placed = []
    seen = set()
    while remaining:
        progressed = False
        held = []
        for n in remaining:
            p = parent_of(n)
            if p is None or p in seen:
                placed.append(n)
                seen.add(n)
                progressed = True
            else:
                held.append(n)
        remaining = held
        if not progressed:
            # A cycle, or a parent outside `names`; the converter reports it
            # properly, so just pass the rest through.
            placed.extend(remaining)
            break
    return placed
