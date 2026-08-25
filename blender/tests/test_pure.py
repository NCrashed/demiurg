"""Tests for the addon's Blender-free parts.

Run with a plain interpreter — no Blender, no third-party packages:

    python3 blender/tests/test_pure.py

The parts that need `bpy`/`mathutils` (scene reading, the BVH query) are
covered by `headless_export.py`, which runs inside Blender.
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from demiurg_export import axes, rig, voxelize  # noqa: E402


class TestAxes(unittest.TestCase):
    def test_height_axis_flips(self):
        # Blender is Z-up, demiurg is Z-down; x and y are untouched.
        self.assertEqual(axes.to_voxels((1.0, 2.0, 3.0), 10.0), (10.0, 20.0, -30.0))
        self.assertEqual(axes.from_voxels((10.0, 20.0, -30.0), 10.0), (1.0, 2.0, 3.0))

    def test_grid_absorbs_float_noise(self):
        # What a 0.4-unit box at 10 voxels/unit really measures in f32. Rounded
        # naively this is a 6-voxel grid for a 4-voxel box, with the model
        # sitting half a voxel off where the artist put it.
        noise = 2.000_000_029_802_322
        lo = (-noise, -noise, -noise)
        hi = (noise, noise, noise)
        _, dims, _ = axes.grid_layout(lo, hi, (0.0, 0.0, 0.0))
        self.assertEqual(dims, (4, 4, 4))

    def test_grid_keeps_real_fractions(self):
        # A genuinely fractional bound still rounds outward.
        _, dims, _ = axes.grid_layout((-2.3, 0.0, 0.0), (2.3, 1.0, 1.0), (0.0, 0.0, 0.0))
        self.assertEqual(dims, (6, 1, 1))

    def test_grid_covers_a_pivot_outside_the_mesh(self):
        # demiurg clamps a pivot into [0, dim], so a bone whose head sits off
        # its own mesh has to have the grid stretched to reach it — otherwise
        # it silently rotates about a different point.
        origin, dims, pivot = axes.grid_layout((0.0, 0.0, 0.0), (2.0, 2.0, 2.0), (-3.0, 0.0, 5.0))
        self.assertEqual(origin, (-3, 0, 0))
        self.assertEqual(dims, (5, 2, 5))
        self.assertEqual(pivot, (0.0, 0.0, 5.0))
        for i in range(3):
            self.assertGreaterEqual(pivot[i], 0.0)
            self.assertLessEqual(pivot[i], dims[i])

    def test_bounds_of_empty(self):
        self.assertIsNone(axes.bounds_of([]))


def plane_at_z(z_blender):
    """A `nearest` stand-in: an infinite horizontal plane, normal pointing up
    (+Z in Blender), so "behind the surface" means below it."""

    def nearest(point):
        location = (point[0], point[1], z_blender)
        return location, (0.0, 0.0, 1.0), 0, abs(point[2] - z_blender)

    return nearest


class TestVoxelize(unittest.TestCase):
    def test_surface_layer_is_one_voxel_thick(self):
        # 1 voxel per unit, so a voxel is 1.0 Blender units and the reach is
        # 0.5. Grid rows sit at Blender z = -0.5, -1.5, ... (the flip).
        filled = voxelize.voxelize(
            plane_at_z(-0.5), (0, 0, 0), (1, 1, 4), 1.0, {0: "ff0000"}, solid=False
        )
        self.assertEqual(sorted(filled), [(0, 0, 0)])
        self.assertEqual(filled[(0, 0, 0)], "ff0000")

    def test_solid_fills_behind_the_surface(self):
        # Everything below the plane is "inside"; with the height flip, that is
        # increasing k.
        filled = voxelize.voxelize(
            plane_at_z(-0.5), (0, 0, 0), (1, 1, 4), 1.0, {0: "ff0000"}, solid=True
        )
        self.assertEqual(sorted(filled), [(0, 0, 0), (0, 0, 1), (0, 0, 2), (0, 0, 3)])

    def test_nothing_in_range_stays_empty(self):
        def nowhere(_point):
            return None, None, None, None

        self.assertEqual(voxelize.voxelize(nowhere, (0, 0, 0), (2, 2, 2), 1.0, {}), {})

    def test_a_face_without_a_colour_still_draws(self):
        filled = voxelize.voxelize(
            plane_at_z(-0.5), (0, 0, 0), (1, 1, 1), 1.0, {}, solid=False
        )
        self.assertEqual(filled[(0, 0, 0)], voxelize.DEFAULT_COLOR)


class TestColor(unittest.TestCase):
    def test_linear_becomes_srgb(self):
        self.assertEqual(voxelize.to_hex((0.0, 0.0, 0.0)), "000000")
        self.assertEqual(voxelize.to_hex((1.0, 1.0, 1.0)), "ffffff")
        # Blender's default Principled grey, 0.8 linear. Skipping the transfer
        # would export cc — visibly darker than what the artist sees.
        self.assertEqual(voxelize.to_hex((0.8, 0.8, 0.8)), "e7e7e7")

    def test_out_of_range_is_clamped(self):
        self.assertEqual(voxelize.to_hex((-1.0, 2.0, 0.0)), "00ff00")

    def test_material_prefers_the_principled_base_colour(self):
        class Socket:
            def __init__(self, value, linked=False):
                self.default_value = value
                self.is_linked = linked

        class Node:
            def __init__(self, socket):
                self.type = "BSDF_PRINCIPLED"
                self.inputs = {"Base Color": socket}

        class Material:
            def __init__(self, node, diffuse):
                self.node_tree = type("T", (), {"nodes": [node] if node else []})()
                self.diffuse_color = diffuse

        base = Material(Node(Socket((1.0, 0.0, 0.0, 1.0))), (0.0, 1.0, 0.0, 1.0))
        self.assertEqual(voxelize.material_hex(base), "ff0000")

        # A linked base colour is a texture: its `default_value` is a stale
        # leftover, so the viewport colour is the better guess.
        textured = Material(Node(Socket((1.0, 0.0, 0.0, 1.0), linked=True)), (0.0, 1.0, 0.0, 1.0))
        self.assertEqual(voxelize.material_hex(textured), "00ff00")

        self.assertEqual(voxelize.material_hex(Material(None, (0.0, 0.0, 1.0, 1.0))), "0000ff")
        self.assertEqual(voxelize.material_hex(None), voxelize.DEFAULT_COLOR)


class TestRig(unittest.TestCase):
    def test_joint_is_the_offset_between_heads(self):
        self.assertEqual(rig.joint_offset((3.5, 0.0, -9.0), (0.0, 0.0, 0.0)), (3.5, 0.0, -9.0))
        self.assertEqual(rig.joint_offset((3.5, 0.0, -9.0), None), (0.0, 0.0, 0.0))

    def test_voxels_come_out_in_a_stable_order(self):
        # Same scene, same bytes: an export that reshuffles is unreadable in a
        # diff and churns version control for nothing.
        voxels = {(1, 0, 0): "aabbcc", (0, 0, 0): "112233", (0, 1, 0): "445566"}
        self.assertEqual(
            rig.voxel_list(voxels),
            [[0, 0, 0, "112233"], [0, 1, 0, "445566"], [1, 0, 0, "aabbcc"]],
        )

    def test_bones_come_out_parents_first(self):
        parents = {"hand": "arm", "arm": "torso", "torso": None, "head": "torso"}
        order = rig.sorted_bones(["hand", "head", "arm", "torso"], parents.get)
        for name, parent in parents.items():
            if parent:
                self.assertLess(order.index(parent), order.index(name))

    def test_a_parent_cycle_terminates(self):
        # Malformed input is the converter's job to report by name; this must
        # just not hang or lose bones on the way there.
        parents = {"a": "b", "b": "a"}
        self.assertEqual(sorted(rig.sorted_bones(["a", "b"], parents.get)), ["a", "b"])

    def test_a_root_bone_carries_no_parent_key(self):
        entry = rig.bone_entry("torso", None, (0.0, 0.0, 0.0))
        self.assertNotIn("parent", entry)
        self.assertNotIn("mesh", entry)
        self.assertIn("parent", rig.bone_entry("arm", "torso", (1.0, 0.0, 0.0)))


if __name__ == "__main__":
    unittest.main(verbosity=2)
