"""Cut an armature-deformed mesh into rigid per-bone chunks.

voxlap draws each bone as one whole kv6 sprite, so there is no such thing as a
voxel influenced by two bones: a smoothly weighted skin has to be *divided*,
and each voxel handed to exactly one bone. This picks the bone with the most
influence over each triangle, and the voxelizer then hands every voxel the bone
of the triangle nearest to it.

The result is a rigid rig. At a joint that bends far, the two chunks pull apart
or intersect — no amount of cleverness here fixes that, because the engine has
nowhere to put a voxel that is half one bone and half another. Modelling each
limb as its own object, parented to its bone, avoids the question entirely and
is what voxel characters usually do anyway.

The weight arithmetic is duck-typed rather than `bpy`-typed, so it can be
tested with stand-ins — see `blender/tests/test_pure.py`.
"""


def dominant_bone(weights):
    """The bone with the most influence, or `None` when nothing weighs in.

    Ties go to the alphabetically first name, so an export is reproducible
    rather than depending on dictionary order.
    """
    best_name = None
    best_weight = 0.0
    for name in sorted(weights):
        weight = weights[name]
        if weight > best_weight:
            best_weight = weight
            best_name = name
    return best_name


def vertex_weights(vertex, group_names):
    """One vertex's `{bone: weight}`, keeping only groups that name a bone."""
    weights = {}
    for entry in vertex.groups:
        name = group_names.get(entry.group)
        if name is not None and entry.weight > 0.0:
            weights[name] = weights.get(name, 0.0) + entry.weight
    return weights


def triangle_bones(mesh, obj, bone_names, base=0):
    """`{triangle index: bone name}` for `mesh`, by summed vertex weight.

    Summing the three corners' weights (rather than voting on each corner's
    own winner) puts a triangle straddling a joint on the side that actually
    holds more of it. `base` offsets the indices, so several objects can share
    one triangle soup.

    Triangles with no weight at all are left out — the caller decides what to
    do with geometry the artist never bound to anything.
    """
    group_names = {i: g.name for i, g in enumerate(obj.vertex_groups) if g.name in bone_names}
    if not group_names:
        return {}
    per_vertex = {}
    owners = {}
    for index, tri in enumerate(mesh.loop_triangles):
        totals = {}
        for vertex_index in tri.vertices:
            weights = per_vertex.get(vertex_index)
            if weights is None:
                weights = vertex_weights(mesh.vertices[vertex_index], group_names)
                per_vertex[vertex_index] = weights
            for name, weight in weights.items():
                totals[name] = totals.get(name, 0.0) + weight
        owner = dominant_bone(totals)
        if owner is not None:
            owners[base + index] = owner
    return owners


def nearest_bone(voxel, heads):
    """The bone whose head is closest to `voxel`, or `None` with no bones.

    Where unweighted geometry goes. Dropping it would lose part of the model
    with nothing on screen to explain why, and putting it all on one bone would
    smear a stray face across the character.
    """
    best_name = None
    best_distance = None
    for name in sorted(heads):
        head = heads[name]
        distance = sum((voxel[i] - head[i]) ** 2 for i in range(3))
        if best_distance is None or distance < best_distance:
            best_distance = distance
            best_name = name
    return best_name
