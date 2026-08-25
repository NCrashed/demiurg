"""Export a Blender armature + its per-bone meshes to a demiurg `.demiurg`
project (or a `.rkc` character).

The addon does no format writing of its own: it voxelizes, works out the
skeleton, and emits the JSON exchange manifest that `demiurg-convert` turns
into a document. That keeps the wire formats written once, in Rust — see
`crates/demiurg-convert` for the schema and the traps it guards.

Nothing here imports `bpy` at module scope except the submodules `register()`
pulls in, so the pure parts (`axes`, `rig`, and the grid math in `voxelize`)
can be imported and tested outside Blender — see `blender/tests`.
"""

bl_info = {
    "name": "demiurg rig export",
    "author": "Anton Gushcha",
    "version": (0, 1, 0),
    "blender": (3, 6, 0),
    "location": "File > Export > demiurg rig (.demiurg)",
    "description": "Voxelize an armature's per-bone meshes and export a demiurg project",
    "category": "Import-Export",
    "doc_url": "https://github.com/NCrashed/demiurg",
}


def register():
    # Imported here rather than at module scope so the pure submodules stay
    # importable without Blender.
    from . import operator

    operator.register()


def unregister():
    from . import operator

    operator.unregister()
