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


def voxelize(nearest, origin, dims, voxels_per_unit, tri_values, solid=True, default=None):
    """Fill a `dims` grid at `origin` (voxel space) by asking `nearest` about
    each voxel centre.

    `tri_values` maps a triangle index to whatever the caller wants recorded
    for a voxel the triangle claims — a `"rrggbb"` for a plain mesh, or a
    `(colour, bone)` pair when a skinned mesh is being cut into per-bone
    chunks. Returns `{(i, j, k): value}` for the filled voxels only; the
    manifest carries a sparse list and a bone's grid is mostly empty.
    """
    if default is None:
        default = DEFAULT_COLOR
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
                out[(i, j, k)] = tri_values.get(index, default)
    return out


def _is_behind(point, location, normal):
    """Whether `point` lies on the back side of the surface at `location` —
    inside, for a closed mesh. The nearest surface point of an interior point
    is *outward* from it, so it agrees with the outward normal."""
    if normal is None:
        return False
    d = (location[0] - point[0], location[1] - point[1], location[2] - point[2])
    return d[0] * normal[0] + d[1] * normal[1] + d[2] * normal[2] > 0.0


def lattice_pitch(coords, minimum_samples=8):
    """The spacing of `coords` when they sit on a regular lattice, else `None`.

    An already-voxelized mesh — a Voxelity modifier, a remesh — has its
    vertices on a grid, and every gap between distinct coordinates is a whole
    multiple of one spacing. Exporting such a mesh onto a grid of a different
    size resamples one lattice through the other and chews the edges, so it is
    worth spotting and reporting.

    Pure, so the fiddly part is testable: a smooth mesh must not be mistaken
    for a lattice, or the export would nag about a number that means nothing.
    """
    distinct = sorted({round(c, 5) for c in coords})
    if len(distinct) < minimum_samples:
        return None
    gaps = sorted({round(b - a, 5) for a, b in zip(distinct, distinct[1:]) if b - a > 1e-5})
    if not gaps:
        return None
    smallest = gaps[0]
    if all(abs(g / smallest - round(g / smallest)) < 0.01 for g in gaps):
        return smallest
    return None


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


def _principled(material):
    """The material's Principled BSDF node, or `None`."""
    node_tree = getattr(material, "node_tree", None)
    for node in getattr(node_tree, "nodes", ()) if node_tree is not None else ():
        if getattr(node, "type", "") == "BSDF_PRINCIPLED":
            return node
    return None


def _socket(node, name):
    """A node input's value, or `None` when it is absent or driven by a graph
    (a linked socket's `default_value` is a stale leftover, not what renders)."""
    socket = node.inputs.get(name) if node is not None else None
    if socket is None or getattr(socket, "is_linked", False):
        return None
    return socket.default_value


def material_effect(material):
    """How a material composites: `(alpha, mode)` or `None` for solid.

    Read off the Principled BSDF, since that is where an artist sets it:

    * **Alpha** below 1 is the direct statement — a slime at 0.86 is
      `blend` at 220.
    * **Transmission** with a full alpha is the other way people author glass;
      taken as `1 - transmission` so it means the same thing.
    * **Emission strength** above zero on an otherwise solid material is a
      glow, which the engine draws `add`itively.

    Only one of the three wins, in that order: guessing at a blend of two
    physical effects the format cannot represent would be worse than picking
    the one the artist most likely meant.
    """
    node = _principled(material)
    if node is None:
        return None
    alpha = _socket(node, "Alpha")
    if alpha is not None and alpha < 1.0:
        return _quantize(alpha), "blend"
    # "Transmission Weight" in 4.x, "Transmission" before it.
    transmission = _socket(node, "Transmission Weight")
    if transmission is None:
        transmission = _socket(node, "Transmission")
    if transmission:
        return _quantize(1.0 - transmission), "blend"
    strength = _socket(node, "Emission Strength")
    if strength:
        return _quantize(min(1.0, strength)), "add"
    return None


def _quantize(value):
    """A 0..1 factor as the manifest's 0..255."""
    return int(round(min(max(value, 0.0), 1.0) * 255.0))


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
