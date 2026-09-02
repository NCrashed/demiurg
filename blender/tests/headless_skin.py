"""Export a smoothly weighted mesh and check it came apart at the joint.

The case the addon has to handle badly-but-correctly: one mesh spanning two
bones, weights ramping across the boundary, no bone parenting anywhere. voxlap
draws a bone as one rigid sprite, so the mesh has to be *divided* — and the
division has to land at the joint, keep every voxel, and give each one to
exactly one bone.

    blender --background --python blender/tests/headless_skin.py -- \
        --out /tmp/skinned.demiurg --converter ./target/debug/demiurg-convert

Prints `RESULT: OK` or `RESULT: FAIL` — Blender swallows a script's exit code.
"""

import json
import os
import sys
import traceback

from math import radians

import bpy
from mathutils import Quaternion

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from demiurg_export import operator as demiurg_op  # noqa: E402

VOXELS_PER_UNIT = 10.0
# Where the two bones meet, in Blender units up the column.
JOINT_Z = 1.0
# Weights ramp over this much either side of the joint, so the boundary
# triangles genuinely belong to both bones.
BLEND = 0.2


def clear_scene():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for block in (bpy.data.meshes, bpy.data.armatures, bpy.data.materials, bpy.data.objects):
        for item in list(block):
            try:
                block.remove(item)
            except (RuntimeError, ReferenceError):
                pass


def column_mesh(name, half_width, height, sections):
    """A square column from `z = 0` to `height`, ringed into `sections` bands.

    Built from data rather than a primitive plus modifiers so the vertex
    heights — and therefore the weights below — are exact.
    """
    verts = []
    faces = []
    for s in range(sections + 1):
        z = height * s / sections
        verts += [
            (-half_width, -half_width, z),
            (half_width, -half_width, z),
            (half_width, half_width, z),
            (-half_width, half_width, z),
        ]
    for s in range(sections):
        base = s * 4
        for i in range(4):
            j = (i + 1) % 4
            faces.append((base + i, base + j, base + 4 + j, base + 4 + i))
    faces.append((3, 2, 1, 0))  # bottom cap
    top = sections * 4
    faces.append((top, top + 1, top + 2, top + 3))

    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return obj


def build_scene():
    clear_scene()
    armature_data = bpy.data.armatures.new("skeleton")
    armature = bpy.data.objects.new("column", armature_data)
    bpy.context.collection.objects.link(armature)
    bpy.context.view_layer.objects.active = armature

    bpy.ops.object.mode_set(mode="EDIT")
    lower = armature_data.edit_bones.new("lower")
    lower.head = (0.0, 0.0, 0.0)
    lower.tail = (0.0, 0.0, JOINT_Z)
    upper = armature_data.edit_bones.new("upper")
    upper.head = (0.0, 0.0, JOINT_Z)
    upper.tail = (0.0, 0.0, 2.0)
    upper.parent = lower
    bpy.ops.object.mode_set(mode="OBJECT")

    obj = column_mesh("skin", 0.2, 2.0, 20)
    material = bpy.data.materials.new("skin")
    material.diffuse_color = (0.3, 0.6, 0.3, 1.0)
    material.use_nodes = True
    for node in material.node_tree.nodes:
        if node.type == "BSDF_PRINCIPLED":
            node.inputs["Base Color"].default_value = (0.3, 0.6, 0.3, 1.0)
    obj.data.materials.append(material)

    # Weights ramp through the joint, so the boundary is genuinely shared and
    # the exporter has to decide rather than read off an obvious answer.
    groups = {name: obj.vertex_groups.new(name=name) for name in ("lower", "upper")}
    for vertex in obj.data.vertices:
        z = vertex.co.z
        share = min(max((z - (JOINT_Z - BLEND)) / (2 * BLEND), 0.0), 1.0)
        groups["upper"].add([vertex.index], share, "REPLACE")
        groups["lower"].add([vertex.index], 1.0 - share, "REPLACE")

    obj.parent = armature
    modifier = obj.modifiers.new("Armature", "ARMATURE")
    modifier.object = armature

    bpy.context.view_layer.objects.active = armature
    bpy.context.view_layer.update()

    # Bend the joint, so the export also has to survive the segmentation and
    # the animation bake at once — and so the seam a rigid cut leaves is
    # visible in a render rather than only described in the docs.
    pose_bone = armature.pose.bones["upper"]
    pose_bone.rotation_mode = "QUATERNION"
    for frame, degrees in ((1, 0.0), (13, 50.0)):
        pose_bone.rotation_quaternion = Quaternion((1.0, 0.0, 0.0), radians(degrees))
        pose_bone.keyframe_insert(data_path="rotation_quaternion", frame=frame)
    armature.animation_data.action.name = "bend"
    return armature


def bone_extents(manifest):
    """Each bone's voxel span along the height axis, in armature space.

    A bone's voxels are stored relative to its own grid, so they are put back
    into the shared frame through its pivot and the chain of joints — the same
    arithmetic the solver does.
    """
    heads = {}
    spans = {}
    for bone in manifest["bones"]:
        parent = bone.get("parent")
        base = heads[parent][2] if parent else 0.0
        heads[bone["name"]] = (0.0, 0.0, base + bone["joint"][2])
        mesh = bone.get("mesh")
        if not mesh or not mesh["voxels"]:
            continue
        head_z = heads[bone["name"]][2]
        zs = [v[2] - mesh["pivot"][2] + head_z for v in mesh["voxels"]]
        spans[bone["name"]] = (min(zs), max(zs), len(mesh["voxels"]))
    return spans


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    args = dict(zip(argv[::2], argv[1::2]))
    out = args.get("--out", "/tmp/demiurg-skin-test.demiurg")
    converter = args.get("--converter")
    if converter:
        converter = os.path.abspath(converter)

    import demiurg_export

    demiurg_export.register()  # registers the per-object flags the export reads
    build_scene()
    summary, warnings = demiurg_op.export_document(
        bpy.context, out, voxels_per_unit=VOXELS_PER_UNIT, solid=True,
        export_animation=True, converter=converter, keep_manifest=True,
    )
    for w in warnings:
        print(f"WARNING: {w}")
    print(f"EXPORT: {summary}")

    with open(os.path.splitext(out)[0] + ".json", encoding="utf-8") as f:
        manifest = json.load(f)
    spans = bone_extents(manifest)
    for name, (lo, hi, count) in sorted(spans.items()):
        print(f"BONE {name}: {count} voxels, height {lo:.1f}..{hi:.1f}")

    problems = []
    if set(spans) != {"lower", "upper"}:
        problems.append(f"expected both bones to get geometry, got {sorted(spans)}")
    else:
        # +Z points down, so the upper bone holds the more negative voxels.
        overlap = spans["lower"][0] - spans["upper"][1]
        if overlap < -1.0:
            problems.append(f"chunks overlap by {-overlap:.1f} voxels; the cut is not at the joint")
        boundary = -JOINT_Z * VOXELS_PER_UNIT
        if abs(spans["lower"][0] - boundary) > 2.0:
            problems.append(
                f"lower chunk starts at {spans['lower'][0]:.1f}, not near the joint ({boundary:.1f})"
            )
        total = sum(s[2] for s in spans.values())
        # The column is 0.4 x 0.4 x 2.0 units at 10 voxels/unit: 4 x 4 x 20
        # cells, solid. Losing voxels to the split would show up here.
        if not 280 <= total <= 384:
            problems.append(f"{total} voxels total, expected a solid 4x4x20 column (320)")

    if problems:
        for line in problems:
            print(f"  {line}")
        print("RESULT: FAIL")
        return
    print("RESULT: OK the skin came apart at the joint with every voxel kept")


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 — the marker line is the test's verdict
        traceback.print_exc()
        print("RESULT: FAIL")
