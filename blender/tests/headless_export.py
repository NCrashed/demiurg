"""Build a two-bone scene, export it, and report — the addon's end-to-end test.

Run it from the repo root:

    blender --background --python blender/tests/headless_export.py -- \
        --out /tmp/hero.demiurg --converter ./target/debug/demiurg-convert

Prints `RESULT: OK` (and the converter's summary) on success, `RESULT: FAIL`
with the reason otherwise — Blender swallows a non-zero exit from a `--python`
script, so the marker line is what a caller should grep for.
"""

import json
import os
import sys
import traceback

from math import radians

import bpy
from mathutils import Quaternion, Vector

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from demiurg_export import axes as demiurg_axes  # noqa: E402
from demiurg_export import operator as demiurg_op  # noqa: E402


def clear_scene():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for block in (bpy.data.meshes, bpy.data.armatures, bpy.data.materials):
        for item in list(block):
            block.remove(item)


def colored(name, rgba, alpha=1.0):
    """A material coloured the way an artist would: on the Principled BSDF,
    which is what the exporter reads (the viewport `diffuse_color` is only a
    fallback for materials with no node tree). `alpha` below 1 makes it
    translucent, which has to reach the file as a blend material."""
    material = bpy.data.materials.new(name)
    material.diffuse_color = rgba
    material.use_nodes = True
    for node in material.node_tree.nodes:
        if node.type == "BSDF_PRINCIPLED":
            node.inputs["Base Color"].default_value = rgba
            node.inputs["Alpha"].default_value = alpha
    return material


def build_scene():
    """A torso bone with an arm bone off its top, each carrying a box.

    Blender is Z-up here — the exporter is what flips it, so the arm bone runs
    *downward* from the shoulder like a real arm and should come out of the
    export hanging the same way.
    """
    clear_scene()
    armature_data = bpy.data.armatures.new("skeleton")
    armature = bpy.data.objects.new("hero", armature_data)
    bpy.context.collection.objects.link(armature)
    bpy.context.view_layer.objects.active = armature

    bpy.ops.object.mode_set(mode="EDIT")
    torso = armature_data.edit_bones.new("torso")
    torso.head = (0.0, 0.0, 0.0)
    torso.tail = (0.0, 0.0, 1.0)
    arm = armature_data.edit_bones.new("arm")
    arm.head = (0.35, 0.0, 0.9)
    arm.tail = (0.35, 0.0, 0.3)
    arm.parent = torso
    # A third bone down the chain: rotating the arm has to carry it, which is
    # what makes the exported rotation checkable rather than merely plausible.
    hand = armature_data.edit_bones.new("hand")
    hand.head = (0.35, 0.0, 0.3)
    hand.tail = (0.35, 0.0, 0.1)
    hand.parent = arm
    bpy.ops.object.mode_set(mode="OBJECT")

    def box(name, center, size, material):
        bpy.ops.mesh.primitive_cube_add(size=1.0, location=center)
        obj = bpy.context.object
        obj.name = name
        obj.scale = Vector(size)
        obj.data.materials.append(material)
        return obj

    body = box("body", (0.0, 0.0, 0.5), (0.4, 0.3, 1.0), colored("blue", (0.1, 0.25, 0.7, 1.0)))
    # Translucent on purpose: the arm's material has to arrive as a blend.
    limb = box(
        "limb", (0.35, 0.0, 0.6), (0.15, 0.15, 0.6),
        colored("orange", (0.8, 0.35, 0.05, 1.0), alpha=0.5),
    )
    fist = box("fist", (0.35, 0.0, 0.2), (0.2, 0.2, 0.2), colored("pale", (0.85, 0.7, 0.5, 1.0)))

    for obj, bone in ((body, "torso"), (limb, "arm"), (fist, "hand")):
        world = obj.matrix_world.copy()
        obj.parent = armature
        obj.parent_type = "BONE"
        obj.parent_bone = bone
        bpy.context.view_layer.update()
        obj.matrix_world = world  # bone parenting hangs off the tail; stay put

    bpy.context.view_layer.objects.active = armature
    bpy.context.view_layer.update()
    add_wave_action(armature)
    return armature


def add_wave_action(armature):
    """Swing the arm bone and back over 24 frames.

    Keyed through `keyframe_insert` rather than by building fcurves by hand, so
    Blender decides how the action stores its channels — which is the whole
    reason the exporter samples poses instead of reading curves.
    """
    pose_bone = armature.pose.bones["arm"]
    pose_bone.rotation_mode = "QUATERNION"
    for frame, degrees in ((1, 0.0), (13, 60.0), (25, 0.0)):
        pose_bone.rotation_quaternion = Quaternion((1.0, 0.0, 0.0), radians(degrees))
        pose_bone.keyframe_insert(data_path="rotation_quaternion", frame=frame)
    armature.animation_data.action.name = "wave"
    return armature.animation_data.action


def dump_expected_poses(armature, action, voxels_per_unit, path):
    """Write where Blender itself puts each bone, per frame, in voxel space.

    `compare_poses.py` checks these against `demiurg --dump-pose`, which turns
    "the animation looks about right" into a number per bone per frame.
    Positions are relative to the root bone, so a global offset can't paper
    over a real error.
    """
    scene = bpy.context.scene
    fps = scene.render.fps / scene.render.fps_base
    start, end = (int(round(v)) for v in action.frame_range)
    root = next(b for b in armature.pose.bones if b.bone.parent is None)

    frames = []
    saved = scene.frame_current
    for frame in range(start, end + 1):
        scene.frame_set(frame)
        origin = root.matrix.translation
        bones = {
            b.name: list(demiurg_axes.to_voxels(b.matrix.translation - origin, voxels_per_unit))
            for b in armature.pose.bones
        }
        frames.append({"t_ms": round((frame - start) / fps * 1000.0), "bones": bones})
    scene.frame_set(saved)

    with open(path, "w", encoding="utf-8") as f:
        json.dump({"clip": action.name, "fps": fps, "frames": frames}, f, indent=1)
    return path


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    out = "/tmp/demiurg-blender-test.demiurg"
    converter = None
    expected = None
    for i, a in enumerate(argv):
        if a == "--out":
            out = argv[i + 1]
        elif a == "--converter":
            converter = os.path.abspath(argv[i + 1])
        elif a == "--expected":
            expected = argv[i + 1]

    armature = build_scene()
    # Registering exercises the real install path (operator + preferences), not
    # just the export function this script then calls directly.
    import demiurg_export

    demiurg_export.register()
    assert hasattr(bpy.ops.export_scene, "demiurg_rig"), "operator did not register"

    # Export from a posed frame on purpose. Geometry has to come out at rest —
    # if the exporter reads the evaluated mesh as-is, the pose bakes into the
    # voxels and the clips then pose them a second time.
    bpy.context.scene.frame_set(13)

    summary, warnings = demiurg_op.export_document(
        bpy.context, out, voxels_per_unit=10.0, solid=True,
        converter=converter, keep_manifest=True,
    )
    for w in warnings:
        print(f"WARNING: {w}")
    with open(os.path.splitext(out)[0] + ".json", encoding="utf-8") as f:
        manifest = json.load(f)
    materials = manifest.get("materials", [])
    print(f"MATERIALS: {materials}")
    assert len(materials) == 1, f"expected the translucent arm's material, got {materials}"
    entry = materials[0]
    assert entry["mode"] == "blend", entry
    assert abs(entry["alpha"] - 128) <= 1, f"alpha 0.5 should land near 128, got {entry}"

    if expected:
        dump_expected_poses(armature, armature.animation_data.action, 10.0, expected)
        print(f"EXPECTED: wrote {expected}")
    print(f"RESULT: OK {summary}")


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 — the marker line is the test's verdict
        traceback.print_exc()
        print("RESULT: FAIL")
