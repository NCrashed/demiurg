"""The Blender side: read the scene, voxelize, run `demiurg-convert`.

Everything that touches `bpy` lives here. The export itself is a plain function
([`export_document`]) rather than operator-only code, so a headless script can
drive it — which is how it is tested (`blender/tests/headless_export.py`).
"""

import json
import os
import shutil
import subprocess
import tempfile

import bpy
from bpy.props import BoolProperty, FloatProperty, IntProperty, StringProperty
from bpy_extras.io_utils import ExportHelper

from . import axes, rig, voxelize

# Refuse a grid larger than this on a side. `.kv6` addresses voxels in a
# byte-oriented layout meant for sprite-sized models, and a 512³ bone is
# someone having set `voxels_per_unit` two orders of magnitude too high —
# better a clear error than ten minutes of nearest-point queries.
MAX_BONE_DIM = 256

CONVERTER = "demiurg-convert"


def find_converter(explicit=None):
    """The `demiurg-convert` binary: an explicit path if given, else whatever
    is on `PATH`. `None` when it can't be found."""
    if explicit:
        path = bpy.path.abspath(explicit)
        return path if os.path.isfile(path) else None
    return shutil.which(CONVERTER)


def _pick_armature(context):
    """The armature to export: the active object, else a selected one, else the
    scene's only one. `None` when the scene has no armature (a bare-model
    export) or when the choice is ambiguous."""
    obj = context.active_object
    if obj is not None and obj.type == "ARMATURE":
        return obj
    selected = [o for o in context.selected_objects if o.type == "ARMATURE"]
    if len(selected) == 1:
        return selected[0]
    in_scene = [o for o in context.scene.objects if o.type == "ARMATURE"]
    if len(in_scene) == 1:
        return in_scene[0]
    return None


def _bone_meshes(armature):
    """Mesh objects grouped by the bone they hang off, plus the skinned meshes
    that were skipped.

    Only *bone parenting* attaches a mesh to a bone here. A mesh deformed by an
    armature modifier has smooth weights spread over several bones, and voxlap
    draws each bone as one rigid sprite — cutting such a mesh into per-bone
    chunks is a separate job, so those are reported rather than silently
    dropped or wrongly assigned.
    """
    by_bone = {}
    skinned = []
    for obj in armature.children:
        if obj.type != "MESH":
            continue
        if obj.parent_type == "BONE" and obj.parent_bone:
            by_bone.setdefault(obj.parent_bone, []).append(obj)
        elif any(m.type == "ARMATURE" for m in obj.modifiers):
            skinned.append(obj.name)
    return by_bone, skinned


def _triangles(context, objects, to_space):
    """Every triangle of `objects` as `(verts, tris, tri_colors)`, with the
    vertices mapped through `to_space` (world → armature).

    Modifiers are applied — the evaluated mesh is what the artist sees, and a
    mirror or subsurf they forgot to apply should not change the export.
    """
    depsgraph = context.evaluated_depsgraph_get()
    verts = []
    tris = []
    tri_colors = {}
    palette = {}
    for obj in objects:
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        if mesh is None:
            continue
        try:
            if hasattr(mesh, "calc_loop_triangles"):
                mesh.calc_loop_triangles()
            matrix = to_space @ evaluated.matrix_world
            base = len(verts)
            verts.extend(matrix @ v.co for v in mesh.vertices)
            for tri in mesh.loop_triangles:
                slot = mesh.polygons[tri.polygon_index].material_index
                material = None
                if slot < len(obj.material_slots):
                    material = obj.material_slots[slot].material
                key = material.name if material is not None else None
                if key not in palette:
                    palette[key] = voxelize.material_hex(material)
                tri_colors[len(tris)] = palette[key]
                tris.append(tuple(base + i for i in tri.vertices))
        finally:
            evaluated.to_mesh_clear()
    return verts, tris, tri_colors


def _bone_mesh_entry(context, objects, to_space, head_vox, voxels_per_unit, solid):
    """One bone's manifest `mesh`, or `None` when the bone carries no geometry
    (a dummy / helper bone). Raises `ValueError` if the grid is unreasonably
    large."""
    verts, tris, tri_colors = _triangles(context, objects, to_space)
    if not tris:
        return None
    corners = axes.bounds_of([axes.to_voxels(v, voxels_per_unit) for v in verts])
    lo, hi = corners
    origin, dims, pivot = axes.grid_layout(lo, hi, head_vox)
    if max(dims) > MAX_BONE_DIM:
        raise ValueError(
            f"grid is {dims[0]}x{dims[1]}x{dims[2]} voxels, over the {MAX_BONE_DIM} limit; "
            "lower 'Voxels per unit'"
        )
    nearest = voxelize.bvh_nearest(verts, tris)
    filled = voxelize.voxelize(nearest, origin, dims, voxels_per_unit, tri_colors, solid)
    return rig.mesh_entry(dims, pivot, filled)


def build_manifest(context, armature, voxels_per_unit, solid):
    """The manifest for `armature` (or, with `armature=None`, for the selected
    meshes as a bare model). Returns `(manifest, warnings)`."""
    stamp = f"written by the demiurg Blender addon from {bpy.data.filepath or 'an unsaved file'}"
    warnings = []
    if armature is None:
        meshes = [o for o in context.selected_objects if o.type == "MESH"]
        if not meshes:
            raise ValueError("select an armature to export a rig, or a mesh to export a model")
        # No skeleton, so the model's own origin is the pivot: keep the
        # scene's world origin as the reference frame.
        identity = meshes[0].matrix_world.copy()
        identity.identity()
        entry = _bone_mesh_entry(
            context, meshes, identity, (0.0, 0.0, 0.0), voxels_per_unit, solid
        )
        if entry is None:
            raise ValueError("the selected meshes have no faces to voxelize")
        return rig.model_manifest(meshes[0].name, entry, stamp), warnings

    by_bone, skinned = _bone_meshes(armature)
    if skinned:
        warnings.append(
            "skipped armature-deformed mesh(es) "
            + ", ".join(sorted(skinned))
            + ": parent a mesh to a bone (Ctrl+P > Bone) to export it"
        )
    if not by_bone:
        raise ValueError("no mesh is parented to a bone of this armature (Ctrl+P > Bone)")

    bones = armature.data.bones
    to_space = armature.matrix_world.inverted()
    heads = {
        b.name: axes.to_voxels(b.matrix_local.translation, voxels_per_unit) for b in bones
    }
    parent_of = {b.name: (b.parent.name if b.parent else None) for b in bones}

    entries = []
    for name in rig.sorted_bones([b.name for b in bones], parent_of.get):
        parent = parent_of[name]
        try:
            mesh = _bone_mesh_entry(
                context, by_bone.get(name, []), to_space, heads[name], voxels_per_unit, solid
            )
        except ValueError as e:
            raise ValueError(f"bone {name!r}: {e}") from e
        if mesh is None and name not in by_bone:
            # A bone with no mesh is legal (the converter gives it an empty
            # model), but it is usually an oversight worth naming.
            warnings.append(f"bone {name!r} has no mesh parented to it")
        joint = rig.joint_offset(heads[name], heads[parent] if parent else None)
        entries.append(rig.bone_entry(name, parent, joint, mesh))
    return rig.rig_manifest(armature.name, entries, stamp), warnings


def export_document(context, filepath, voxels_per_unit=10.0, solid=True,
                    converter=None, keep_manifest=False):
    """Voxelize the scene and write `filepath` through `demiurg-convert`.

    Returns `(summary, warnings)`. Raises `ValueError` for a scene the exporter
    can't make sense of and `RuntimeError` when the converter is missing or
    rejects the manifest — its message names the bone or clip at fault.
    """
    binary = find_converter(converter)
    if binary is None:
        raise RuntimeError(
            f"{CONVERTER} not found: set its path in the addon preferences, "
            "or put it on PATH"
        )
    armature = _pick_armature(context)
    manifest, warnings = build_manifest(context, armature, voxels_per_unit, solid)

    if keep_manifest:
        manifest_path = os.path.splitext(filepath)[0] + ".json"
    else:
        handle, manifest_path = tempfile.mkstemp(suffix=".json", prefix="demiurg-")
        os.close(handle)
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
    try:
        done = subprocess.run(
            [binary, manifest_path, "-o", filepath],
            capture_output=True,
            text=True,
            check=False,
        )
    finally:
        if not keep_manifest:
            os.unlink(manifest_path)
    # The converter reports on stderr and distinguishes success by exit code.
    message = (done.stderr or done.stdout).strip()
    if done.returncode != 0:
        raise RuntimeError(message or f"{CONVERTER} failed ({done.returncode})")
    return message, warnings


class DemiurgExportPreferences(bpy.types.AddonPreferences):
    """Where the converter lives. Blank means "look on PATH"."""

    bl_idname = __package__

    converter_path: StringProperty(
        name="demiurg-convert",
        description="Path to the demiurg-convert binary (blank: search PATH)",
        subtype="FILE_PATH",
        default="",
    )

    def draw(self, context):
        layout = self.layout
        layout.prop(self, "converter_path")
        if find_converter(self.converter_path) is None:
            layout.label(text=f"{CONVERTER} not found", icon="ERROR")


class DEMIURG_OT_export_rig(bpy.types.Operator, ExportHelper):
    """Voxelize the armature's per-bone meshes and export a demiurg project"""

    bl_idname = "export_scene.demiurg_rig"
    bl_label = "Export demiurg rig"
    bl_options = {"REGISTER", "UNDO"}

    filename_ext = ".demiurg"
    filter_glob: StringProperty(default="*.demiurg;*.rkc", options={"HIDDEN"})

    voxels_per_unit: FloatProperty(
        name="Voxels per unit",
        description="Voxel resolution: how many voxels one Blender unit becomes",
        default=10.0,
        min=0.01,
        max=float(MAX_BONE_DIM),
    )
    solid: BoolProperty(
        name="Fill interior",
        description=(
            "Fill voxels inside the mesh, not just its surface. Needs closed "
            "geometry; turn it off for open or non-manifold meshes"
        ),
        default=True,
    )
    keep_manifest: BoolProperty(
        name="Keep manifest",
        description="Write the intermediate JSON next to the output, for debugging",
        default=False,
    )

    def execute(self, context):
        prefs = context.preferences.addons.get(__package__)
        converter = prefs.preferences.converter_path if prefs else None
        try:
            summary, warnings = export_document(
                context,
                self.filepath,
                voxels_per_unit=self.voxels_per_unit,
                solid=self.solid,
                converter=converter,
                keep_manifest=self.keep_manifest,
            )
        except (ValueError, RuntimeError) as e:
            self.report({"ERROR"}, str(e))
            return {"CANCELLED"}
        for warning in warnings:
            self.report({"WARNING"}, warning)
        self.report({"INFO"}, summary)
        return {"FINISHED"}


def menu_func_export(self, context):
    self.layout.operator(DEMIURG_OT_export_rig.bl_idname, text="demiurg rig (.demiurg)")


_CLASSES = (DemiurgExportPreferences, DEMIURG_OT_export_rig)


def register():
    for cls in _CLASSES:
        bpy.utils.register_class(cls)
    bpy.types.TOPBAR_MT_file_export.append(menu_func_export)


def unregister():
    bpy.types.TOPBAR_MT_file_export.remove(menu_func_export)
    for cls in reversed(_CLASSES):
        bpy.utils.unregister_class(cls)
