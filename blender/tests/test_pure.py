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

from demiurg_export import axes, bundle, rig, skin, voxelize  # noqa: E402


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

    def test_quaternion_reorders_and_mirrors(self):
        # Blender is (w, x, y, z); the manifest is [x, y, z, w].
        self.assertEqual(axes.quat_to_voxels((1.0, 0.0, 0.0, 0.0)), [0.0, 0.0, 0.0, 1.0])
        # The height flip mirrors the rotation: the components about the two
        # unflipped axes change sign, the one about z does not.
        self.assertEqual(axes.quat_to_voxels((0.5, 0.1, 0.2, 0.3)), [-0.1, -0.2, 0.3, 0.5])

    def test_a_turn_about_the_height_axis_survives_the_flip(self):
        # Mirroring negates a rotation's angle, so a turn about z has to come
        # back as the same turn — otherwise every yaw plays backwards. Rotating
        # +x by 90 degrees about +z gives +y in Blender; in demiurg, where z
        # points the other way, the same visual turn takes +x to -y.
        from math import cos, sin, radians

        half = radians(90) / 2
        x, y, z, w = axes.quat_to_voxels((cos(half), 0.0, 0.0, sin(half)))
        self.assertAlmostEqual(z, sin(half))
        self.assertAlmostEqual(w, cos(half))
        self.assertEqual((x, y), (0.0, -0.0))


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


class TestLatticePitch(unittest.TestCase):
    """Spotting a mesh that is already voxelized."""

    def test_a_grid_reports_its_spacing(self):
        # Voxelity's default: blocks every 0.05 units, with gaps where the
        # shape is hollow — every gap still a whole multiple.
        coords = [i * 0.05 for i in range(12)] + [i * 0.05 for i in range(20, 26)]
        self.assertAlmostEqual(voxelize.lattice_pitch(coords), 0.05, places=5)

    def test_a_smooth_mesh_is_not_a_lattice(self):
        # A UV sphere's x coordinates are irregular; reporting a pitch here
        # would nag the artist about a number that means nothing.
        from math import cos, pi

        coords = [cos(pi * i / 30) for i in range(31)]
        self.assertIsNone(voxelize.lattice_pitch(coords))

    def test_too_few_samples_says_nothing(self):
        self.assertIsNone(voxelize.lattice_pitch([0.0, 0.1, 0.2]))
        self.assertIsNone(voxelize.lattice_pitch([]))

    def test_a_single_plane_says_nothing(self):
        # Every coordinate identical: no gaps to measure.
        self.assertIsNone(voxelize.lattice_pitch([0.5] * 20))


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


class TestMaterialEffect(unittest.TestCase):
    """How a Blender material becomes a demiurg blend mode."""

    @staticmethod
    def principled(**values):
        class Socket:
            def __init__(self, value, linked=False):
                self.default_value = value
                self.is_linked = linked

        class Node:
            def __init__(self, inputs):
                self.type = "BSDF_PRINCIPLED"
                self.inputs = inputs

        class Material:
            def __init__(self, node):
                self.node_tree = type("T", (), {"nodes": [node]})()
                self.diffuse_color = (0.5, 0.5, 0.5, 1.0)

        inputs = {k.replace("_", " "): Socket(v) for k, v in values.items()}
        return Material(Node(inputs))

    def test_alpha_below_one_is_a_blend(self):
        # The slime case: Alpha 0.864 in the picker.
        self.assertEqual(
            voxelize.material_effect(self.principled(Alpha=0.864)), (220, "blend")
        )

    def test_a_solid_material_has_no_effect(self):
        self.assertIsNone(voxelize.material_effect(self.principled(Alpha=1.0)))
        self.assertIsNone(voxelize.material_effect(None))

    def test_transmission_is_the_other_way_to_author_glass(self):
        # Alpha untouched, transmission turned up: same intent, so it maps to
        # the same mode with the opacity inverted.
        effect = voxelize.material_effect(
            self.principled(Alpha=1.0, Transmission_Weight=0.75)
        )
        self.assertEqual(effect, (64, "blend"))

    def test_alpha_and_transmission_stack(self):
        # They are not two ways of saying the same thing: alpha is how much of
        # the surface is there, transmission how much light passes through what
        # is there. Taking only alpha is how a slime authored at 0.86 alpha and
        # 0.5 transmission — plainly see-through in Blender — exported at
        # 220/255 and rendered solid.
        effect = voxelize.material_effect(
            self.principled(Alpha=0.5, Transmission_Weight=0.9)
        )
        self.assertEqual(effect, (13, "blend"))

    def test_the_slime_that_exported_solid(self):
        effect = voxelize.material_effect(
            self.principled(Alpha=0.8636, Transmission_Weight=0.5)
        )
        self.assertEqual(effect, (110, "blend"))

    def test_emission_on_a_solid_material_glows(self):
        effect = voxelize.material_effect(
            self.principled(Alpha=1.0, Emission_Strength=0.6)
        )
        self.assertEqual(effect, (153, "add"))

    def test_a_linked_socket_is_not_read(self):
        # A driven or textured input's `default_value` is a stale leftover, not
        # what renders.
        class Socket:
            default_value = 0.2
            is_linked = True

        class Node:
            type = "BSDF_PRINCIPLED"
            inputs = {"Alpha": Socket()}

        class Material:
            node_tree = type("T", (), {"nodes": [Node()]})()

        self.assertIsNone(voxelize.material_effect(Material()))


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

    def test_a_key_only_carries_what_moved(self):
        # A bone that only turns writes `{"r": ...}`. Emitting all three every
        # time would triple a baked clip's JSON for nothing — the converter
        # fills the rest with the identity.
        self.assertEqual(rig.xform_entry((0.0, 0.0, 0.0), (0.0, 0.0, 0.0, 1.0), (1.0, 1.0, 1.0)), {})
        self.assertEqual(
            rig.xform_entry((0.0, 0.0, 0.0), (0.5, 0.0, 0.0, 0.866_025), (1.0, 1.0, 1.0)),
            {"r": [0.5, 0.0, 0.0, 0.86603]},
        )
        self.assertEqual(
            rig.xform_entry((1.234_567_8, 0.0, 0.0), None, None), {"t": [1.23457, 0.0, 0.0]}
        )
        self.assertEqual(rig.xform_entry(None, None, (2.0, 1.0, 1.0)), {"s": [2.0, 1.0, 1.0]})

    def test_a_negated_quaternion_is_still_the_identity(self):
        # `-q` and `q` are the same rotation, so a rest bone keyed as either
        # must drop out of the pose.
        self.assertEqual(rig.xform_entry(None, (0.0, 0.0, 0.0, -1.0), None), {})

    def test_a_clip_carries_its_length_and_loop(self):
        keys = [rig.key_entry(0, {}), rig.key_entry(500, {"arm": {"r": [0.0, 0.0, 0.0, 1.0]}})]
        clip = rig.clip_entry("wave", keys, 1000, loops=True)
        self.assertEqual(clip["name"], "wave")
        self.assertEqual(clip["length_ms"], 1000)
        self.assertIs(clip["loop"], True)
        self.assertEqual(len(clip["keys"]), 2)

    def test_a_root_bone_carries_no_parent_key(self):
        entry = rig.bone_entry("torso", None, (0.0, 0.0, 0.0))
        self.assertNotIn("parent", entry)
        self.assertNotIn("mesh", entry)
        self.assertIn("parent", rig.bone_entry("arm", "torso", (1.0, 0.0, 0.0)))


class Group:
    def __init__(self, group, weight):
        self.group = group
        self.weight = weight


class Vertex:
    def __init__(self, *groups):
        self.groups = [Group(g, w) for g, w in groups]


class Triangle:
    def __init__(self, *vertices):
        self.vertices = vertices


class FakeMesh:
    def __init__(self, vertices, triangles):
        self.vertices = vertices
        self.loop_triangles = triangles


class FakeObject:
    def __init__(self, group_names):
        self.vertex_groups = [type("G", (), {"name": n})() for n in group_names]


class TestSkin(unittest.TestCase):
    def test_the_heaviest_bone_wins(self):
        self.assertEqual(skin.dominant_bone({"arm": 0.7, "torso": 0.3}), "arm")
        self.assertIsNone(skin.dominant_bone({}))
        self.assertIsNone(skin.dominant_bone({"arm": 0.0}))

    def test_a_tie_resolves_the_same_way_every_time(self):
        # Otherwise re-exporting an unchanged scene could hand a boundary voxel
        # to a different bone and produce a different file.
        self.assertEqual(skin.dominant_bone({"b": 0.5, "a": 0.5}), "a")
        self.assertEqual(skin.dominant_bone({"a": 0.5, "b": 0.5}), "a")

    def test_only_groups_that_name_a_bone_count(self):
        # Vertex groups are also used for masks, modifiers, and shape keys; one
        # named "outline" must not become a bone.
        vertex = Vertex((0, 0.6), (1, 0.4), (2, 0.9))
        weights = skin.vertex_weights(vertex, {0: "arm", 1: "torso"})
        self.assertEqual(weights, {"arm": 0.6, "torso": 0.4})

    def test_a_triangle_goes_to_the_bone_holding_most_of_it(self):
        # Two corners lean torso, one leans arm — summing beats voting, because
        # a triangle straddling a joint belongs where its area is.
        mesh = FakeMesh(
            [
                Vertex((0, 0.9), (1, 0.1)),
                Vertex((0, 0.8), (1, 0.2)),
                Vertex((0, 0.1), (1, 0.9)),
            ],
            [Triangle(0, 1, 2)],
        )
        obj = FakeObject(["torso", "arm"])
        self.assertEqual(skin.triangle_bones(mesh, obj, {"torso", "arm"}), {0: "torso"})

    def test_unweighted_triangles_are_left_for_the_caller(self):
        mesh = FakeMesh([Vertex(), Vertex(), Vertex()], [Triangle(0, 1, 2)])
        self.assertEqual(skin.triangle_bones(mesh, FakeObject(["torso"]), {"torso"}), {})

    def test_indices_can_be_offset_for_a_merged_soup(self):
        mesh = FakeMesh([Vertex((0, 1.0))] * 3, [Triangle(0, 1, 2)])
        obj = FakeObject(["torso"])
        self.assertEqual(skin.triangle_bones(mesh, obj, {"torso"}, base=7), {7: "torso"})

    def test_a_mesh_with_no_bone_groups_claims_nothing(self):
        mesh = FakeMesh([Vertex((0, 1.0))] * 3, [Triangle(0, 1, 2)])
        self.assertEqual(skin.triangle_bones(mesh, FakeObject(["mask"]), {"torso"}), {})

    def test_orphan_voxels_go_to_the_closest_bone(self):
        heads = {"torso": (0.0, 0.0, 0.0), "arm": (10.0, 0.0, 0.0)}
        self.assertEqual(skin.nearest_bone((1.0, 0.0, 0.0), heads), "torso")
        self.assertEqual(skin.nearest_bone((9.0, 1.0, 0.0), heads), "arm")
        self.assertIsNone(skin.nearest_bone((0.0, 0.0, 0.0), {}))


class TestBundle(unittest.TestCase):
    """Which bundled binary this machine picks.

    Worth testing every branch from one machine: whoever builds a release zip
    is rarely on the same OS as whoever installs it, so a wrong folder name
    would only ever surface as "demiurg-convert not found" on someone else's
    computer.
    """

    def test_platform_folder_names(self):
        self.assertEqual(bundle.platform_tag("linux", "x86_64"), "linux-x86_64")
        self.assertEqual(bundle.platform_tag("win32", "AMD64"), "windows-x86_64")
        self.assertEqual(bundle.platform_tag("darwin", "arm64"), "macos-arm64")
        self.assertEqual(bundle.platform_tag("darwin", "x86_64"), "macos-x86_64")
        # Linux reports aarch64 where macOS says arm64; both are one folder.
        self.assertEqual(bundle.platform_tag("linux", "aarch64"), "linux-arm64")
        # An unknown platform still yields a usable name rather than blowing up
        # — it just won't match a folder, and the addon falls back to PATH.
        self.assertEqual(bundle.platform_tag("freebsd14", "riscv64"), "linux-riscv64")

    def test_the_binary_is_exe_only_on_windows(self):
        self.assertEqual(bundle.converter_name("win32"), "demiurg-convert.exe")
        self.assertEqual(bundle.converter_name("linux"), "demiurg-convert")
        self.assertEqual(bundle.converter_name("darwin"), "demiurg-convert")

    def test_no_bundle_is_not_an_error(self):
        # A repo checkout has no `bin/`; the addon must fall through to the
        # preferences path or PATH rather than fail.
        self.assertIsNone(bundle.bundled_converter(root="/nonexistent"))

    def test_a_bundled_binary_is_found_and_made_executable(self):
        import stat
        import tempfile

        with tempfile.TemporaryDirectory() as root:
            binary = os.path.join(root, "bin", bundle.platform_tag(), bundle.converter_name())
            os.makedirs(os.path.dirname(binary))
            with open(binary, "w", encoding="utf-8") as f:
                f.write("#!/bin/sh\n")
            os.chmod(binary, 0o644)  # as some unzip routes leave it
            found = bundle.bundled_converter(root=root)
            self.assertEqual(found, binary)
            self.assertTrue(os.stat(found).st_mode & stat.S_IXUSR, "restored the executable bit")


if __name__ == "__main__":
    unittest.main(verbosity=2)
