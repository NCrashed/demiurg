"""Build a two-bone scene, export it, and report — the addon's end-to-end test.

Run it from the repo root:

    blender --background --python blender/tests/headless_export.py -- \
        --out /tmp/hero.demiurg --converter ./target/debug/demiurg-convert

Prints `RESULT: OK` (and the converter's summary) on success, `RESULT: FAIL`
with the reason otherwise — Blender swallows a non-zero exit from a `--python`
script, so the marker line is what a caller should grep for.
"""

import os
import sys
import traceback

import bpy
from mathutils import Vector

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from demiurg_export import operator as demiurg_op  # noqa: E402


def clear_scene():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for block in (bpy.data.meshes, bpy.data.armatures, bpy.data.materials):
        for item in list(block):
            block.remove(item)


def colored(name, rgba):
    """A material coloured the way an artist would: on the Principled BSDF,
    which is what the exporter reads (the viewport `diffuse_color` is only a
    fallback for materials with no node tree)."""
    material = bpy.data.materials.new(name)
    material.diffuse_color = rgba
    material.use_nodes = True
    for node in material.node_tree.nodes:
        if node.type == "BSDF_PRINCIPLED":
            node.inputs["Base Color"].default_value = rgba
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
    bpy.ops.object.mode_set(mode="OBJECT")

    def box(name, center, size, material):
        bpy.ops.mesh.primitive_cube_add(size=1.0, location=center)
        obj = bpy.context.object
        obj.name = name
        obj.scale = Vector(size)
        obj.data.materials.append(material)
        return obj

    body = box("body", (0.0, 0.0, 0.5), (0.4, 0.3, 1.0), colored("blue", (0.1, 0.25, 0.7, 1.0)))
    limb = box("limb", (0.35, 0.0, 0.6), (0.15, 0.15, 0.6), colored("orange", (0.8, 0.35, 0.05, 1.0)))

    for obj, bone in ((body, "torso"), (limb, "arm")):
        world = obj.matrix_world.copy()
        obj.parent = armature
        obj.parent_type = "BONE"
        obj.parent_bone = bone
        bpy.context.view_layer.update()
        obj.matrix_world = world  # bone parenting hangs off the tail; stay put

    bpy.context.view_layer.objects.active = armature
    bpy.context.view_layer.update()
    return armature


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    out = "/tmp/demiurg-blender-test.demiurg"
    converter = None
    for i, a in enumerate(argv):
        if a == "--out":
            out = argv[i + 1]
        elif a == "--converter":
            converter = os.path.abspath(argv[i + 1])

    build_scene()
    # Registering exercises the real install path (operator + preferences), not
    # just the export function this script then calls directly.
    import demiurg_export

    demiurg_export.register()
    assert hasattr(bpy.ops.export_scene, "demiurg_rig"), "operator did not register"

    summary, warnings = demiurg_op.export_document(
        bpy.context, out, voxels_per_unit=10.0, solid=True,
        converter=converter, keep_manifest=True,
    )
    for w in warnings:
        print(f"WARNING: {w}")
    print(f"RESULT: OK {summary}")


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 — the marker line is the test's verdict
        traceback.print_exc()
        print("RESULT: FAIL")
