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
from mathutils import Matrix, Quaternion, Vector
from bpy_extras.io_utils import ExportHelper

from . import anim, axes, bundle, rig, skin, voxelize

# Refuse a grid larger than this on a side. `.kv6` addresses voxels in a
# byte-oriented layout meant for sprite-sized models, and a 512³ bone is
# someone having set `voxels_per_unit` two orders of magnitude too high —
# better a clear error than ten minutes of nearest-point queries.
MAX_BONE_DIM = 256

CONVERTER = bundle.CONVERTER


def find_converter(explicit=None):
    """The `demiurg-convert` binary, or `None` if there isn't one.

    An explicit path from the preferences wins — someone building the
    workspace wants their own build, not the bundled one. Then the binary
    shipped inside the addon, then `PATH`.
    """
    if explicit:
        path = bpy.path.abspath(explicit)
        return path if os.path.isfile(path) else None
    return bundle.bundled_converter() or shutil.which(CONVERTER)


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


def is_voxel_clip(obj):
    """Whether `obj` is marked to be baked frame by frame.

    Through `getattr` because the flag is registered by the addon, while
    `build_manifest` is also driven straight from scripts — a missing property
    just means "not flagged", not a crash.
    """
    return bool(getattr(obj, "demiurg_voxel_clip", False))


def _bone_meshes(armature):
    """Mesh children of `armature`, split by how they are attached: the ones
    parented to a bone (grouped by bone), and the ones an armature modifier
    deforms (which have to be cut up by weight — see [`skin`])."""
    by_bone = {}
    by_clip_bone = {}
    skinned = []
    for obj in armature.children:
        if obj.type != "MESH":
            continue
        if obj.parent_type == "BONE" and obj.parent_bone:
            # Flagged meshes take the flipbook path; the rest stay rigid.
            # `getattr` because the flag is registered by the addon, and
            # `build_manifest` is also driven straight from scripts.
            target = by_clip_bone if is_voxel_clip(obj) else by_bone
            target.setdefault(obj.parent_bone, []).append(obj)
        elif any(m.type == "ARMATURE" for m in obj.modifiers):
            skinned.append(obj)
    return by_bone, by_clip_bone, skinned


class _RestPose:
    """Evaluate meshes against the armature's rest pose, not the current frame.

    Geometry is the bind shape; the pose belongs in the clips. Without this the
    export bakes in whatever frame the timeline happened to sit on — an
    armature modifier deforms the evaluated mesh, and a bone-parented object's
    world matrix carries its bone's pose — so a character exported mid-stride
    comes out permanently mid-stride, and then the clips pose it again on top.
    """

    def __init__(self, context, armature):
        self.context = context
        self.armature = armature

    def __enter__(self):
        if self.armature is not None:
            self.saved = self.armature.data.pose_position
            self.armature.data.pose_position = "REST"
            self.context.view_layer.update()  # so the depsgraph has it
        return self

    def __exit__(self, *_):
        if self.armature is not None:
            self.armature.data.pose_position = self.saved
            self.context.view_layer.update()
        return False


def _triangles(context, objects, to_space, bone_names=None):
    """Every triangle of `objects` as `(verts, tris, tri_colors, tri_bones)`,
    with the vertices mapped through `to_space` (world → armature).

    `tri_bones` is empty unless `bone_names` is given, in which case each
    triangle is attributed to the bone weighting it most. It is filled here,
    against the same running triangle index the soup is built with, so several
    objects can be merged without the two ever disagreeing.

    Modifiers are applied — the evaluated mesh is what the artist sees, and a
    mirror or subsurf they forgot to apply should not change the export.
    """
    depsgraph = context.evaluated_depsgraph_get()
    verts = []
    tris = []
    tri_colors = {}
    tri_bones = {}
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
            base_vertex = len(verts)
            base_tri = len(tris)
            verts.extend(matrix @ v.co for v in mesh.vertices)
            if bone_names:
                tri_bones.update(skin.triangle_bones(mesh, obj, bone_names, base_tri))
            for tri in mesh.loop_triangles:
                slot = mesh.polygons[tri.polygon_index].material_index
                material = None
                if slot < len(obj.material_slots):
                    material = obj.material_slots[slot].material
                key = material.name if material is not None else None
                if key not in palette:
                    palette[key] = voxelize.material_hex(material)
                tri_colors[len(tris)] = palette[key]
                tris.append(tuple(base_vertex + i for i in tri.vertices))
        finally:
            evaluated.to_mesh_clear()
    return verts, tris, tri_colors, tri_bones


def _voxel_field(context, objects, to_space, voxels_per_unit, solid, bone_names=None,
                 limit=MAX_BONE_DIM):
    """Voxelize `objects` into **armature-space** voxel coordinates.

    Returns `{bone name or None: {(x, y, z): "rrggbb"}}`. Without `bone_names`
    everything lands under `None` — the caller already knows which bone it
    asked about. With them, a skinned mesh comes back already divided, from a
    single pass over the whole character rather than one pass per bone.

    Coordinates are global on purpose: a bone's grid can only be laid out once
    its voxels are known, and a skinned bone's voxels aren't known until the
    split has happened.
    """
    verts, tris, tri_colors, tri_bones = _triangles(context, objects, to_space, bone_names)
    if not tris:
        return {}
    lo, hi = axes.bounds_of([axes.to_voxels(v, voxels_per_unit) for v in verts])
    origin, dims, _ = axes.grid_layout(lo, hi, lo)
    if max(dims) > limit:
        raise ValueError(
            f"grid is {dims[0]}x{dims[1]}x{dims[2]} voxels, over the {limit} limit; "
            "lower 'Voxels per unit'"
        )
    values = {i: (color, tri_bones.get(i)) for i, color in tri_colors.items()}
    nearest = voxelize.bvh_nearest(verts, tris)
    filled = voxelize.voxelize(
        nearest, origin, dims, voxels_per_unit, values, solid,
        default=(voxelize.DEFAULT_COLOR, None),
    )
    out = {}
    for (i, j, k), (color, bone) in filled.items():
        position = (origin[0] + i, origin[1] + j, origin[2] + k)
        out.setdefault(bone, {})[position] = color
    return out


def _clip_sample_frames(context, clip_fps):
    """Scene frames to sample a deforming mesh at, and the ms each one holds.

    Independent of the scene's frame rate on purpose: this is the knob that
    decides what a deforming bone costs, since every sample is a whole voxel
    grid. Sampling every rendered frame of a 24 fps action would quadruple the
    file for motion the eye reads the same.
    """
    scene = context.scene
    fps = scene.render.fps / scene.render.fps_base
    start, end = scene.frame_start, scene.frame_end
    span_s = max(0.0, (end - start) / fps)
    count = max(1, int(round(span_s * clip_fps)))
    step = (end - start) / count if count else 0.0
    frames = [start + i * step for i in range(count)]
    return frames, max(1, int(round(1000.0 / clip_fps)))


def _bone_frame_transform(armature, bone_name):
    """`(inverse delta rotation, posed head)` of a bone at the current frame.

    A clip's frames are stored in the bone's own frame, so whatever the
    skeleton does to the bone has to come *out* of the geometry — otherwise
    playback applies it a second time and a carried slime swings twice as far.
    """
    pose_bone = armature.pose.bones.get(bone_name)
    if pose_bone is None:
        return Quaternion(), Vector((0.0, 0.0, 0.0))
    delta = pose_bone.matrix @ pose_bone.bone.matrix_local.inverted()
    return delta.to_quaternion().inverted(), pose_bone.matrix.translation.copy()


def _clip_segments(context, windows, clip_fps):
    """What to sample, as `[(action, frames, window_ms)]`.

    With no windows — a blob with no skeleton — that is the scene's range,
    once. With them, it is each action over its own frame range, and the
    flipbook is the concatenation: this is what lets one bone deform
    differently per action, since the rig playhead is what picks the frame and
    each action owns its own stretch of the timeline.
    """
    if not windows:
        frames, frame_ms = _clip_sample_frames(context, clip_fps)
        return [(None, frames, len(frames) * frame_ms)]
    scene = context.scene
    fps = scene.render.fps / scene.render.fps_base
    segments = []
    for window in windows:
        start, end = window["frame_start"], window["frame_end"]
        span_s = max(0.0, (end - start) / fps)
        count = max(1, int(round(span_s * clip_fps)))
        step = (end - start) / count if count else 0.0
        segments.append(
            (window["action"], [start + i * step for i in range(count)], window["length_ms"])
        )
    return segments


def _bake_voxel_clip(context, objects, armature, bone_name, voxels_per_unit, solid,
                     clip_fps, windows=None):
    """Voxelize `objects` once per sampled frame into one shared grid.

    Returns a manifest `clip`, or `None` if the mesh has no faces. The grid
    covers the union of every frame's bounds, because the format shares
    `dims`/`pivot` across frames — a per-frame fit would make the blob jump
    around its own pivot.
    """
    scene = context.scene
    saved = scene.frame_current
    segments = _clip_segments(context, windows, clip_fps)
    to_space = armature.matrix_world.inverted() if armature else Matrix()

    # Pass one: gather each sample's geometry, in the bone's own frame.
    samples = []
    durations = []
    try:
        for action, sample_frames, window_ms in segments:
            swap = anim.action_swap(armature, action) if action is not None else None
            if swap is not None:
                swap.__enter__()
            try:
                taken = 0
                for value in sample_frames:
                    scene.frame_set(int(value), subframe=float(value) - int(value))
                    inverse_delta, head = (
                        _bone_frame_transform(armature, bone_name)
                        if armature
                        else (Quaternion(), Vector((0.0, 0.0, 0.0)))
                    )
                    verts, tris, tri_colors, _ = _triangles(context, objects, to_space)
                    if not tris:
                        continue
                    local = [inverse_delta @ (v - head) for v in verts]
                    samples.append((local, tris, tri_colors))
                    taken += 1
                # Durations that sum to the window exactly, so the next
                # action's window starts on this action's last frame ending.
                durations.extend(rig.split_evenly(window_ms, taken))
            finally:
                if swap is not None:
                    swap.__exit__(None, None, None)
    finally:
        scene.frame_set(saved)
    if not samples:
        return None

    # One grid over every sample, with the bone's head — the pivot — inside it.
    corners = [
        axes.bounds_of([axes.to_voxels(v, voxels_per_unit) for v in local])
        for local, _, _ in samples
    ]
    lo = tuple(min(c[0][i] for c in corners) for i in range(3))
    hi = tuple(max(c[1][i] for c in corners) for i in range(3))
    origin, dims, pivot = axes.grid_layout(lo, hi, (0.0, 0.0, 0.0))
    if max(dims) > MAX_BONE_DIM:
        raise ValueError(
            f"clip grid is {dims[0]}x{dims[1]}x{dims[2]} voxels, over the {MAX_BONE_DIM} "
            "limit; lower 'Voxels per unit'"
        )

    # Pass two: fill that grid, sample by sample.
    frames = []
    for local, tris, tri_colors in samples:
        nearest = voxelize.bvh_nearest(local, tris)
        frames.append(
            voxelize.voxelize(nearest, origin, dims, voxels_per_unit, tri_colors, solid)
        )
    return rig.voxel_clip_entry(
        dims, pivot, frames, max(1, durations[0] if durations else 1), durations=durations
    )


def _mesh_entry(voxels, head_vox):
    """A bone's manifest `mesh` from its voxels in armature space, or `None`
    when it has none. Raises `ValueError` if the bone's own grid is too large
    for the format."""
    if not voxels:
        return None
    lo = tuple(min(p[i] for p in voxels) for i in range(3))
    # A voxel at index `i` fills the cell `[i, i + 1)`, so the far corner is
    # one past the last index.
    hi = tuple(max(p[i] for p in voxels) + 1 for i in range(3))
    origin, dims, pivot = axes.grid_layout(lo, hi, head_vox)
    if max(dims) > MAX_BONE_DIM:
        raise ValueError(
            f"grid is {dims[0]}x{dims[1]}x{dims[2]} voxels, over the {MAX_BONE_DIM} limit; "
            "lower 'Voxels per unit'"
        )
    local = {
        (p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]): color
        for p, color in voxels.items()
    }
    return rig.mesh_entry(dims, pivot, local)


def build_manifest(context, armature, voxels_per_unit, solid,
                   export_animation=True, all_actions=False, clip_fps=12.0):
    """The manifest for `armature` (or, with `armature=None`, for the selected
    meshes as a model or a lone deforming blob). Returns `(manifest, warnings)`."""
    stamp = f"written by the demiurg Blender addon from {bpy.data.filepath or 'an unsaved file'}"
    warnings = []
    if armature is None:
        meshes = [o for o in context.selected_objects if o.type == "MESH"]
        if not meshes:
            raise ValueError("select an armature to export a rig, or a mesh to export a model")
        deforming = [o for o in meshes if is_voxel_clip(o)]
        if deforming:
            # A blob with no skeleton at all. A bare model holds one static
            # mesh, so wrap the flipbook in the smallest rig that can carry
            # it: one bone, no animation of its own.
            if len(deforming) != len(meshes):
                warnings.append(
                    "exported only the meshes marked 'Voxelize per frame'; "
                    "a document is either a flipbook or a static model"
                )
            entry = _bake_voxel_clip(
                context, deforming, None, None, voxels_per_unit, solid, clip_fps
            )
            if entry is None:
                raise ValueError("the selected meshes have no faces to voxelize")
            bone = rig.bone_entry(deforming[0].name, None, (0.0, 0.0, 0.0))
            bone["clip"] = entry
            return rig.rig_manifest(deforming[0].name, [bone], None, stamp), warnings
        # No skeleton, so the model's own origin is the pivot: keep the
        # scene's world origin as the reference frame.
        identity = meshes[0].matrix_world.copy()
        identity.identity()
        field = _voxel_field(context, meshes, identity, voxels_per_unit, solid)
        entry = _mesh_entry(field.get(None, {}), (0.0, 0.0, 0.0))
        if entry is None:
            raise ValueError("the selected meshes have no faces to voxelize")
        return rig.model_manifest(meshes[0].name, entry, stamp), warnings

    by_bone, by_clip_bone, skinned = _bone_meshes(armature)
    flagged_skin = [o.name for o in skinned if is_voxel_clip(o)]
    if flagged_skin:
        warnings.append(
            "marked 'Voxelize per frame' but bound by an armature modifier: "
            + ", ".join(sorted(flagged_skin))
            + " — parent it to a bone (Ctrl+P > Bone) to bake it as a flipbook; "
            "cut into rigid chunks instead"
        )
    if not by_bone and not by_clip_bone and not skinned:
        raise ValueError(
            "no mesh is attached to this armature: parent one to a bone "
            "(Ctrl+P > Bone), or bind it with an armature modifier"
        )

    bones = armature.data.bones
    to_space = armature.matrix_world.inverted()
    heads = {
        b.name: axes.to_voxels(b.matrix_local.translation, voxels_per_unit) for b in bones
    }
    parent_of = {b.name: (b.parent.name if b.parent else None) for b in bones}
    joints = {
        b.name: rig.joint_offset(
            heads[b.name], heads[parent_of[b.name]] if parent_of[b.name] else None
        )
        for b in bones
    }

    # Clips first, because a deforming bone's flipbook has to follow their
    # layout. When several actions share a rig that has one, they are laid out
    # on separate windows of one timeline — the playhead is what picks a
    # flipbook frame, so the actions have to be distinguishable by time.
    clips = []
    windows = []
    if export_animation:
        actions, _ = anim.armature_actions(armature, all_actions)
        windowed = bool(by_clip_bone) and len(actions) > 1
        clips, clip_warnings, windows = anim.build_clips(
            context, armature, voxels_per_unit, joints, all_actions, windowed
        )
        warnings.extend(clip_warnings)
        if windowed and len(windows) > 1:
            warnings.append(
                f"laid {len(windows)} actions on separate windows of one timeline so "
                + ", ".join(sorted(by_clip_bone))
                + " can deform differently in each; the host seeks to a clip's own start"
            )

    # Skinned meshes are cut up in one pass over the whole character: a bone's
    # share isn't known until the split has run, so it can't be done per bone.
    skin_voxels = {}
    orphans = {}
    parented_voxels = {}
    with _RestPose(context, armature):
        if skinned:
            field = _voxel_field(
                context, skinned, to_space, voxels_per_unit, solid,
                bone_names=set(heads), limit=MAX_BONE_DIM * 4,
            )
            orphans = field.pop(None, {})
            skin_voxels = field
        for name, objects in by_bone.items():
            parented_voxels[name] = _voxel_field(
                context, objects, to_space, voxels_per_unit, solid, limit=MAX_BONE_DIM
            ).get(None, {})

    if skinned:
        if orphans:
            # Geometry the artist never weighted to a bone. Dropping it would
            # lose part of the model with nothing on screen to say why.
            for position, color in orphans.items():
                owner = skin.nearest_bone(position, heads)
                if owner is not None:
                    skin_voxels.setdefault(owner, {})[position] = color
            warnings.append(
                f"{len(orphans)} voxel(s) of {', '.join(sorted(o.name for o in skinned))} "
                "have no armature weights; gave them to the nearest bone"
            )
        warnings.append(
            "cut armature-deformed mesh(es) "
            + ", ".join(sorted(o.name for o in skinned))
            + " into rigid per-bone chunks; joints that bend far will show seams"
        )

    entries = []
    for name in rig.sorted_bones([b.name for b in bones], parent_of.get):
        parent = parent_of[name]
        joint = joints[name]
        try:
            # A deforming bone replaces its geometry every frame, so it has a
            # flipbook where a rigid one has a mesh — never both, since the
            # engine draws one primary attachment.
            if name in by_clip_bone:
                if name in by_bone or name in skin_voxels:
                    warnings.append(
                        f"bone {name!r} also has rigid geometry, which the flipbook replaces"
                    )
                entry = rig.bone_entry(name, parent, joint)
                clip = _bake_voxel_clip(
                    context, by_clip_bone[name], armature, name,
                    voxels_per_unit, solid, clip_fps, windows,
                )
                if clip is None:
                    warnings.append(f"bone {name!r}: nothing to voxelize per frame")
                else:
                    entry["clip"] = clip
                entries.append(entry)
                continue

            # A bone can have both a chunk cut out of the skin and its own
            # parented mesh (an accessory, say). The parented one wins where
            # they overlap, being the more deliberate of the two.
            voxels = dict(skin_voxels.get(name, {}))
            voxels.update(parented_voxels.get(name, {}))
            mesh = _mesh_entry(voxels, heads[name])
        except ValueError as e:
            raise ValueError(f"bone {name!r}: {e}") from e
        if mesh is None:
            # A bone with no mesh is legal (the converter gives it an empty
            # model), but it is usually an oversight worth naming.
            warnings.append(f"bone {name!r} has no geometry")
        entries.append(rig.bone_entry(name, parent, joint, mesh))

    return rig.rig_manifest(armature.name, entries, clips, stamp), warnings


def export_document(context, filepath, voxels_per_unit=10.0, solid=True,
                    export_animation=True, all_actions=False, clip_fps=12.0,
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
    manifest, warnings = build_manifest(
        context, armature, voxels_per_unit, solid, export_animation, all_actions, clip_fps
    )

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
    """Where the converter lives. Blank uses the bundled one, or `PATH`."""

    bl_idname = __package__

    converter_path: StringProperty(
        name="demiurg-convert",
        description=(
            "Path to the demiurg-convert binary. Leave blank to use the one "
            "bundled with this addon, or the first on PATH"
        ),
        subtype="FILE_PATH",
        default="",
    )

    def draw(self, context):
        layout = self.layout
        layout.prop(self, "converter_path")
        # Say which one will actually run: an artist with a release zip should
        # see that it is ready without knowing what a PATH is, and a developer
        # should see immediately when a stale override is shadowing their build.
        found = find_converter(self.converter_path)
        if found is None:
            layout.label(text=f"{CONVERTER} not found — set a path above", icon="ERROR")
        elif self.converter_path:
            layout.label(text="using the path above", icon="CHECKMARK")
        elif found == bundle.bundled_converter():
            layout.label(text=f"using the bundled {bundle.platform_tag()} build", icon="CHECKMARK")
        else:
            layout.label(text=f"using {found}", icon="CHECKMARK")


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
    export_animation: BoolProperty(
        name="Animation",
        description=(
            "Bake actions into clips, sampled once per frame (the clip format "
            "interpolates linearly, so curves have to be baked)"
        ),
        default=True,
    )
    all_actions: BoolProperty(
        name="All actions",
        description=(
            "Export every action in the file, not just the ones this armature "
            "uses (its active action and NLA strips)"
        ),
        default=False,
    )
    clip_fps: FloatProperty(
        name="Clip fps",
        description=(
            "How often to voxelize a mesh marked 'Voxelize per frame'. Every "
            "sample is a whole voxel grid, so this is the knob that decides "
            "what a deforming mesh costs"
        ),
        default=12.0,
        min=1.0,
        max=60.0,
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
                export_animation=self.export_animation,
                all_actions=self.all_actions,
                clip_fps=self.clip_fps,
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


class OBJECT_PT_demiurg(bpy.types.Panel):
    """Per-object export settings, in Object Properties."""

    bl_idname = "OBJECT_PT_demiurg"
    bl_label = "demiurg"
    bl_space_type = "PROPERTIES"
    bl_region_type = "WINDOW"
    bl_context = "object"

    @classmethod
    def poll(cls, context):
        return context.object is not None and context.object.type == "MESH"

    def draw(self, context):
        layout = self.layout
        obj = context.object
        layout.prop(obj, "demiurg_voxel_clip")
        if obj.demiurg_voxel_clip:
            column = layout.column(align=True)
            column.label(text="Voxelized every frame, not once", icon="INFO")
            column.label(text="Costs a full grid per sampled frame")


def menu_func_export(self, context):
    self.layout.operator(DEMIURG_OT_export_rig.bl_idname, text="demiurg rig (.demiurg)")


_CLASSES = (DemiurgExportPreferences, DEMIURG_OT_export_rig, OBJECT_PT_demiurg)


def register():
    # A per-object flag rather than an export option: which mesh deforms is a
    # property of the model, not of one export, and an artist should be able
    # to set it once and forget it.
    bpy.types.Object.demiurg_voxel_clip = BoolProperty(
        name="Voxelize per frame",
        description=(
            "Bake this mesh as a per-frame voxel flipbook instead of one rigid "
            "mesh the skeleton moves. For geometry that deforms — a slime, "
            "cloth, a squashing blob — which bones cannot express"
        ),
        default=False,
    )
    for cls in _CLASSES:
        bpy.utils.register_class(cls)
    bpy.types.TOPBAR_MT_file_export.append(menu_func_export)


def unregister():
    bpy.types.TOPBAR_MT_file_export.remove(menu_func_export)
    for cls in reversed(_CLASSES):
        bpy.utils.unregister_class(cls)
    del bpy.types.Object.demiurg_voxel_clip
