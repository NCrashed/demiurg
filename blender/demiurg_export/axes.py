"""Blender space to demiurg voxel space.

Blender is Z-up and measures in metres; demiurg (voxlap) is **Z-down** and
measures in voxels. The axes are otherwise the same, so the whole conversion is
a scale by `voxels_per_unit` plus a flip of the height axis — the same flip
`demiurg-core` applies when it imports a `.vox`
(`crates/demiurg-core/src/vox.rs`).

Flipping one axis reverses chirality, so a rotation carried across it comes
back negated: mirroring turns a rotation by `θ` about `a` into one by `-θ`
about the mirrored `a`. [`quat_to_voxels`] is where that lands.

Pure — no `bpy`, no `mathutils`. Points are any indexable of three floats
(`mathutils.Vector` included).
"""

from math import ceil, floor

# Bounds land on exact voxel boundaries all the time — a 0.4 m box at 10
# voxels/unit is 4 voxels wide — but only to within float error, and
# `10 * (0.5 * 0.4)` is 2.0000000298 in Blender's f32. Rounded straight, that
# grows the grid by a voxel on *both* sides and shifts the pivot off where the
# artist put it. A snap this much smaller than a voxel can't drop real
# geometry.
_SNAP = 1e-4


def to_voxels(p, voxels_per_unit):
    """A Blender-space point as demiurg voxel coordinates (floats)."""
    s = voxels_per_unit
    return (p[0] * s, p[1] * s, -p[2] * s)


def bounds_of(points):
    """`(min, max)` corners of `points`. `None` for an empty sequence."""
    it = iter(points)
    try:
        first = next(it)
    except StopIteration:
        return None
    lo = [first[0], first[1], first[2]]
    hi = list(lo)
    for p in it:
        for i in range(3):
            lo[i] = min(lo[i], p[i])
            hi[i] = max(hi[i], p[i])
    return tuple(lo), tuple(hi)


def grid_layout(lo, hi, pivot):
    """Lay a voxel grid over the box `lo..hi` (voxel-space floats), covering
    `pivot` too.

    Returns `(origin, dims, pivot_in_grid)`: the integer grid origin in voxel
    space, the grid size, and where the pivot lands inside it. The pivot is
    forced inside the box because demiurg clamps a pivot to `[0, dim]` — a bone
    whose head sits outside its own mesh would otherwise silently rotate about
    a different point than the exporter intended.
    """
    origin = tuple(int(floor(min(lo[i], pivot[i]) + _SNAP)) for i in range(3))
    far = tuple(int(ceil(max(hi[i], pivot[i]) - _SNAP)) for i in range(3))
    dims = tuple(max(1, far[i] - origin[i]) for i in range(3))
    in_grid = tuple(pivot[i] - origin[i] for i in range(3))
    return origin, dims, in_grid


def voxel_center(origin, i, j, k):
    """Voxel-space centre of grid cell `(i, j, k)` — cells are unit cubes, so
    the centre is half a voxel past the corner."""
    return (origin[0] + i + 0.5, origin[1] + j + 0.5, origin[2] + k + 0.5)


def quat_to_voxels(q):
    """A Blender quaternion, `(w, x, y, z)`, as the manifest's `[x, y, z, w]`.

    Two changes at once, both easy to miss: Blender puts the scalar **first**
    and the manifest puts it **last**, and the height flip mirrors the
    rotation. A mirror maps a rotation by `θ` about `a` to one by `-θ` about
    the mirrored axis; writing `q = (sin(θ/2)·a, cos(θ/2))` and mirroring z,
    that is `(-x, -y, z, w)`. Get either half wrong and the animation plays
    backwards, or mirrored, or both — which looks like a rigging mistake
    rather than an axis one.
    """
    w, x, y, z = q
    return [-x, -y, z, w]


def from_voxels(p, voxels_per_unit):
    """Inverse of [`to_voxels`] — a voxel-space point back in Blender space.
    Used to ask the mesh what colour sits at a voxel centre."""
    s = voxels_per_unit
    return (p[0] / s, p[1] / s, -p[2] / s)
