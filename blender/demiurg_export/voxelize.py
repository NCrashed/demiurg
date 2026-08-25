"""Turn a triangle soup into demiurg voxels.

One nearest-surface query per voxel decides both whether the voxel is filled
and what colour it takes. A voxel is filled when the surface passes through it
(within half a voxel diagonal of its centre) or — with `solid` — when it sits
behind the nearest surface, which for a closed mesh means inside it.

Nearest-point queries are injected as a callable, so the fill rule is testable
without Blender; [`bvh_nearest`] builds the real one from `mathutils`.
"""

from .axes import from_voxels, voxel_center

# Centre sampling: a voxel counts as surface when the surface comes within its
# inscribed sphere. The generous alternative — half a space diagonal, so any
# cube the surface could clip at all — grows every model by a shell of one
# voxel on each side, and a limb four voxels thick coming out six is far more
# visible than the pinhole a steeply diagonal thin wall can leave here (raise
# the resolution, or leave `solid` on, for those).
_REACH = 0.5


def bvh_nearest(verts, tris):
    """A nearest-surface query over `tris` (index triples into `verts`, both in
    Blender space).

    Returns `f(point) -> (location, normal, tri_index, distance)`, with `None`s
    when nothing is in range. Needs `mathutils`, so it is imported here rather
    than at module scope — the rest of this module stays importable outside
    Blender.
    """
    from mathutils.bvhtree import BVHTree

    tree = BVHTree.FromPolygons(
        [tuple(v) for v in verts], [tuple(t) for t in tris], all_triangles=True
    )
    return tree.find_nearest


def voxelize(nearest, origin, dims, voxels_per_unit, tri_colors, solid=True):
    """Fill a `dims` grid at `origin` (voxel space) by asking `nearest` about
    each voxel centre.

    `tri_colors` maps a triangle index to its `"rrggbb"`. Returns
    `{(i, j, k): "rrggbb"}` for the filled voxels only — the manifest carries a
    sparse list, and a bone's grid is mostly empty.
    """
    # The query works in Blender units, where a voxel is `1 / voxels_per_unit`
    # across.
    reach = _REACH / voxels_per_unit
    out = {}
    for k in range(dims[2]):
        for j in range(dims[1]):
            for i in range(dims[0]):
                center = from_voxels(voxel_center(origin, i, j, k), voxels_per_unit)
                location, normal, index, distance = nearest(center)
                if location is None:
                    continue
                if distance > reach and not (solid and _is_behind(center, location, normal)):
                    continue
                out[(i, j, k)] = tri_colors.get(index, "cccccc")
    return out


def _is_behind(point, location, normal):
    """Whether `point` lies on the back side of the surface at `location` —
    inside, for a closed mesh. The nearest surface point of an interior point
    is *outward* from it, so it agrees with the outward normal."""
    if normal is None:
        return False
    d = (location[0] - point[0], location[1] - point[1], location[2] - point[2])
    return d[0] * normal[0] + d[1] * normal[1] + d[2] * normal[2] > 0.0


def linear_to_srgb(c):
    """Blender stores colours linear; the manifest's hex is sRGB, which is what
    a colour picker showed the artist. Skipping this makes every export come
    out visibly too dark."""
    c = min(max(c, 0.0), 1.0)
    if c <= 0.003_130_8:
        return c * 12.92
    return 1.055 * (c ** (1.0 / 2.4)) - 0.055


def to_hex(rgb):
    """A linear `(r, g, b)` as the manifest's `"rrggbb"`."""
    return "".join(f"{round(linear_to_srgb(c) * 255.0):02x}" for c in rgb[:3])


# Fallback for a face with no material — mid grey, visible against any sky.
DEFAULT_COLOR = "cccccc"


def material_hex(material):
    """A material's flat colour as `"rrggbb"`.

    Prefers the Principled BSDF's base colour (what the artist actually set),
    falling back to the viewport display colour. Textures are not sampled — a
    textured mesh exports as its base colour, flat.

    Duck-typed rather than `bpy`-typed, so it can be tested with a stand-in.
    """
    if material is None:
        return DEFAULT_COLOR
    node_tree = getattr(material, "node_tree", None)
    if node_tree is not None:
        for node in getattr(node_tree, "nodes", ()):
            if getattr(node, "type", "") != "BSDF_PRINCIPLED":
                continue
            base = node.inputs.get("Base Color")
            # A linked base colour is a texture or a whole node graph; its
            # `default_value` is a stale leftover, so fall through to the
            # viewport colour instead of exporting a wrong flat colour.
            if base is not None and not getattr(base, "is_linked", False):
                return to_hex(base.default_value)
    diffuse = getattr(material, "diffuse_color", None)
    if diffuse is not None:
        return to_hex(diffuse)
    return DEFAULT_COLOR
