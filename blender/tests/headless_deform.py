"""Export a mesh that deforms, and check the geometry really changes per frame.

The case a skeleton cannot express: a blob that squashes and stretches. Bones
move rigid chunks, so this has to come out as a per-frame voxel flipbook
instead — and the frames have to differ, at the rate the exporter was asked
for, no more.

    blender --background --python blender/tests/headless_deform.py -- \
        --out /tmp/slime.demiurg --converter ./target/debug/demiurg-convert

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
# Overridable with `--fps`, which is the point of the knob: it is what decides
# how much a deforming mesh costs.
CLIP_FPS = 8.0
FRAME_END = 24  # one second at 24 fps


def clear_scene():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for block in (bpy.data.meshes, bpy.data.armatures, bpy.data.materials, bpy.data.objects):
        for item in list(block):
            try:
                block.remove(item)
            except (RuntimeError, ReferenceError):
                pass


def build_scene():
    """A ball that squashes flat and springs back, driven by a shape key.

    A shape key rather than an armature on purpose: the deformation must come
    from something the skeleton has no way to represent, or the test would pass
    for the wrong reason.
    """
    clear_scene()
    scene = bpy.context.scene
    scene.frame_start = 1
    scene.frame_end = FRAME_END

    bpy.ops.mesh.primitive_uv_sphere_add(radius=0.5, location=(0.0, 0.0, 0.5))
    obj = bpy.context.object
    obj.name = "slime"

    material = bpy.data.materials.new("slime")
    material.diffuse_color = (0.15, 0.7, 0.3, 1.0)
    material.use_nodes = True
    for node in material.node_tree.nodes:
        if node.type == "BSDF_PRINCIPLED":
            node.inputs["Base Color"].default_value = (0.15, 0.7, 0.3, 1.0)
    obj.data.materials.append(material)

    obj.shape_key_add(name="Basis", from_mix=False)
    squashed = obj.shape_key_add(name="Squash", from_mix=False)
    for point in squashed.data:
        # Flatten toward the floor and bulge outward — the classic squash.
        point.co.z = point.co.z * 0.35 + 0.15
        point.co.x *= 1.45
        point.co.y *= 1.45

    squashed.value = 0.0
    squashed.keyframe_insert(data_path="value", frame=1)
    squashed.value = 1.0
    squashed.keyframe_insert(data_path="value", frame=FRAME_END // 2)
    squashed.value = 0.0
    squashed.keyframe_insert(data_path="value", frame=FRAME_END)

    # The flag an artist ticks in Object Properties ▸ demiurg.
    obj.demiurg_voxel_clip = True
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    return obj


def build_two_action_scene():
    """The same blob on a two-bone rig, deforming *differently* per action.

    `squash` flattens it; `stretch` pulls it tall. One flipbook per bone is all
    the format has, so the exporter has to lay the actions on separate windows
    of one timeline and concatenate — otherwise only one deformation survives.
    """
    obj = build_scene()  # the squashing blob, with its shape key already keyed
    scene = bpy.context.scene

    armature_data = bpy.data.armatures.new("skeleton")
    armature = bpy.data.objects.new("blob_rig", armature_data)
    bpy.context.collection.objects.link(armature)
    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.mode_set(mode="EDIT")
    root = armature_data.edit_bones.new("root")
    root.head, root.tail = (0.0, 0.0, 0.0), (0.0, 0.0, 0.4)
    body = armature_data.edit_bones.new("body")
    body.head, body.tail = (0.0, 0.0, 0.4), (0.0, 0.0, 1.0)
    body.parent = root
    bpy.ops.object.mode_set(mode="OBJECT")

    world = obj.matrix_world.copy()
    obj.parent = armature
    obj.parent_type = "BONE"
    obj.parent_bone = "body"
    bpy.context.view_layer.update()
    obj.matrix_world = world

    # Drive the squash from the bone instead of keying it on the mesh. That is
    # what makes the deformation *depend on which action plays* — a shape key
    # animated on its own would evaluate the same under every action, and the
    # windows would hold identical geometry however well they were laid out.
    obj.data.shape_keys.animation_data_clear()
    squashed = obj.data.shape_keys.key_blocks["Squash"]
    fcurve = squashed.driver_add("value")
    driver = fcurve.driver
    driver.type = "SCRIPTED"
    variable = driver.variables.new()
    variable.name = "rot"
    variable.type = "TRANSFORMS"
    target = variable.targets[0]
    target.id = armature
    target.bone_target = "body"
    target.transform_type = "ROT_X"
    target.transform_space = "LOCAL_SPACE"
    # Only a positive bend squashes; a negative one leaves the ball round.
    driver.expression = "max(0.0, rot) * 2.0"

    pose_bone = armature.pose.bones["body"]
    pose_bone.rotation_mode = "QUATERNION"

    def bend(name, degrees):
        armature.animation_data_clear()
        for frame, value in ((1, 0.0), (FRAME_END // 2, degrees), (FRAME_END, 0.0)):
            pose_bone.rotation_quaternion = Quaternion((1.0, 0.0, 0.0), radians(value))
            pose_bone.keyframe_insert(data_path="rotation_quaternion", frame=frame)
        action = armature.animation_data.action
        action.name = name
        action.use_fake_user = True
        return action

    squash = bend("squash", 35.0)  # bends +X, so the driver flattens the blob
    stretch = bend("stretch", -35.0)  # bends -X, so it stays round
    armature.animation_data_clear()
    anim_data = armature.animation_data_create()
    track = anim_data.nla_tracks.new()
    track.strips.new("squash", 1, squash)
    track = anim_data.nla_tracks.new()
    track.strips.new("stretch", 1, stretch)

    bpy.context.view_layer.objects.active = armature
    scene.frame_set(scene.frame_start)
    return armature


def check_two_actions(manifest):
    """The windows must be disjoint, and the flipbook must cover both."""
    problems = []
    clips = manifest.get("clips", [])
    if len(clips) < 2:
        return [f"expected two clips, got {[c['name'] for c in clips]}"]
    starts = [(c["name"], c["keys"][0]["t"], c["length_ms"]) for c in clips]
    for name, start, length in starts:
        print(f"CLIP {name}: window {start}..{length} ms")
    # Laid end to end: the second starts where the first ends.
    ordered = sorted(starts, key=lambda s: s[1])
    if ordered[0][1] != 0:
        problems.append(f"first clip starts at {ordered[0][1]}, expected 0")
    if ordered[1][1] < ordered[0][2]:
        problems.append(
            f"windows overlap: {ordered[0][0]} ends at {ordered[0][2]}, "
            f"{ordered[1][0]} starts at {ordered[1][1]}"
        )

    clip_bones = [b for b in manifest["bones"] if "clip" in b]
    if not clip_bones:
        return problems + ["no bone carries a flipbook"]
    flip = clip_bones[0]["clip"]
    total = sum(f["duration_ms"] for f in flip["frames"])
    print(f"FLIPBOOK {len(flip['frames'])} frames, {total} ms total")

    # The windows must hold *different* geometry, or all this layout bought
    # nothing: find the frame in the middle of each window and compare.
    def frame_at(ms):
        elapsed = 0
        for i, f in enumerate(flip["frames"]):
            elapsed += f["duration_ms"]
            if ms < elapsed:
                return i
        return len(flip["frames"]) - 1

    first_mid = frame_at((ordered[0][1] + ordered[0][2]) // 2)
    second_mid = frame_at((ordered[1][1] + ordered[1][2]) // 2)
    counts = [len(f["voxels"]) for f in flip["frames"]]
    print(f"FLIPBOOK {ordered[0][0]} mid frame {first_mid} = {counts[first_mid]} voxels, "
          f"{ordered[1][0]} mid frame {second_mid} = {counts[second_mid]} voxels")
    if first_mid == second_mid:
        problems.append("both windows land on the same flipbook frame")
    elif counts[first_mid] == counts[second_mid]:
        problems.append(
            f"the two windows hold the same geometry ({counts[first_mid]} voxels each); "
            "the per-action deformation did not survive"
        )
    # The flipbook has to span both windows, or the second action would wrap
    # around and show the first one's geometry.
    if total < ordered[1][2] - 1:
        problems.append(
            f"flipbook covers {total} ms but the timeline runs to {ordered[1][2]} ms"
        )
    return problems


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    args = dict(zip(argv[::2], argv[1::2]))
    out = args.get("--out", "/tmp/demiurg-deform-test.demiurg")
    converter = args.get("--converter")
    if converter:
        converter = os.path.abspath(converter)
    global CLIP_FPS
    CLIP_FPS = float(args.get("--fps", CLIP_FPS))

    import demiurg_export

    demiurg_export.register()  # the per-object flag lives on the registered type
    two_actions = args.get("--actions") == "2"
    if two_actions:
        build_two_action_scene()
    else:
        build_scene()

    summary, warnings = demiurg_op.export_document(
        bpy.context, out, voxels_per_unit=VOXELS_PER_UNIT, solid=True,
        export_animation=two_actions, all_actions=two_actions, clip_fps=CLIP_FPS,
        converter=converter, keep_manifest=True,
    )
    for w in warnings:
        print(f"WARNING: {w}")
    print(f"EXPORT: {summary}")

    with open(os.path.splitext(out)[0] + ".json", encoding="utf-8") as f:
        manifest = json.load(f)

    if two_actions:
        problems = check_two_actions(manifest)
        if problems:
            for line in problems:
                print(f"  {line}")
            print("RESULT: FAIL")
            return
        print("RESULT: OK each action got its own window of the flipbook")
        return

    problems = []
    bones = manifest.get("bones", [])
    if len(bones) != 1 or "clip" not in bones[0]:
        problems.append(f"expected one bone carrying a clip, got {bones and bones[0].keys()}")
    else:
        clip = bones[0]["clip"]
        frames = clip["frames"]
        counts = [len(f["voxels"]) for f in frames]
        print(f"CLIP dims={clip['dims']} frame_ms={clip['frame_ms']} frames={len(frames)}")
        print(f"CLIP voxels per frame: {counts}")

        # Sampled at the rate asked for, not the scene's. Frames 1..24 at 24
        # fps span 23 intervals, and the last is excluded (it is the loop's
        # duplicate of the first), so allow one either side of the ideal.
        scene = bpy.context.scene
        span_s = (scene.frame_end - scene.frame_start) / (
            scene.render.fps / scene.render.fps_base
        )
        want = span_s * CLIP_FPS
        if abs(len(frames) - want) > 1.0:
            problems.append(f"{len(frames)} frames, expected about {want:.1f} at {CLIP_FPS} fps")
        want_ms = round(1000.0 / CLIP_FPS)
        if clip["frame_ms"] != want_ms:
            problems.append(f"frame_ms {clip['frame_ms']}, expected {want_ms} at {CLIP_FPS} fps")
        # The squash is a real shape change: flattened, the ball is wider and
        # shorter, so the voxel count has to move.
        if max(counts) - min(counts) < 0.1 * max(counts):
            problems.append(f"frames barely differ ({counts}); the deformation was not baked")
        # Every frame shares one grid — that is the format's rule, and a frame
        # outside it would have been rejected by the converter, but check the
        # shape actually changed rather than just the count.
        first = {tuple(v[:3]) for v in frames[0]["voxels"]}
        middle = {tuple(v[:3]) for v in frames[len(frames) // 2]["voxels"]}
        if first == middle:
            problems.append("the extreme frames hold identical voxels")

    if problems:
        for line in problems:
            print(f"  {line}")
        print("RESULT: FAIL")
        return
    print("RESULT: OK the blob was baked frame by frame")


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 — the marker line is the test's verdict
        traceback.print_exc()
        print("RESULT: FAIL")
