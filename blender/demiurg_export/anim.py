"""Bake Blender actions into manifest clips.

Sampling, not curve translation: each action is assigned to the armature, the
scene is stepped frame by frame, and `pose_bone.matrix` is read. That way
constraints, drivers, and whatever an action stores internally (fcurves in one
Blender version, slotted layers in the next) all come out evaluated, and the
exporter never has to understand any of it.

The clip format wants exactly that anyway: `frmval` rows are whole-skeleton
poses interpolated linearly, with no sparse per-bone keys, so a Bezier F-curve
has to be sampled to survive at all.

**What a pose means here.** Bone meshes are voxelized in armature space with
the rest orientation baked in, so at rest every bone's basis is the identity
and a bone's posed basis is its *delta* from rest. The manifest wants that
delta relative to the parent's:

    D(B)   = pose_bone.matrix · bone.matrix_local⁻¹     (armature space)
    r(B)   = rot(D(parent))⁻¹ · rot(D(B))
    t(B)   = rot(D(parent))⁻¹ · (head(B) - head(parent)) - joint(B)

with `joint(B)` the same rest offset the bone entry carries. Everything is
computed in Blender space and converted at the end, so the height flip is
applied once, in one place.
"""

import bpy
from mathutils import Quaternion

from . import axes, rig


def armature_actions(armature, all_actions=False):
    """The actions to export, and whether the "everything in the file" fallback
    was used.

    An armature's own actions are its active one plus whatever its NLA strips
    reference. A file with neither — an artist who keyed a pose and never set
    up NLA — would otherwise export a rig with no animation at all, so fall
    back to every action in the file and say so.
    """
    found = []
    anim = armature.animation_data
    if anim is not None:
        if anim.action is not None:
            found.append(anim.action)
        for track in anim.nla_tracks:
            for strip in track.strips:
                if strip.action is not None and strip.action not in found:
                    found.append(strip.action)
    used_fallback = False
    if all_actions or not found:
        used_fallback = not found and not all_actions
        for action in bpy.data.actions:
            if action not in found:
                found.append(action)
    return found, used_fallback


class _ActionSwap:
    """Assign an action for sampling and put everything back afterwards.

    NLA evaluation is muted while sampling: with a stack in place it would win
    over the action just assigned, and every clip would come out as whatever
    the stack evaluates to.
    """

    def __init__(self, armature, action):
        self.armature = armature
        self.action = action

    def __enter__(self):
        anim = self.armature.animation_data
        if anim is None:
            anim = self.armature.animation_data_create()
        self.anim = anim
        self.saved = (anim.action, getattr(anim, "action_slot", None), anim.use_nla)
        anim.use_nla = False
        anim.action = self.action
        # Blender 4.4+ keeps an action's channels in slots; assigning the
        # action alone can leave nothing bound, and every frame samples as rest.
        slots = getattr(self.action, "slots", None)
        if hasattr(anim, "action_slot") and anim.action_slot is None and slots:
            anim.action_slot = slots[0]
        return self

    def __exit__(self, *_):
        action, slot, use_nla = self.saved
        self.anim.action = action
        if slot is not None and hasattr(self.anim, "action_slot"):
            self.anim.action_slot = slot
        self.anim.use_nla = use_nla
        return False


def _pose_snapshot(armature, voxels_per_unit, joints):
    """Every posed bone's manifest transform at the current frame.

    `joints` maps a bone name to its rest offset from its parent, in voxels —
    the same number its bone entry carries.
    """
    deltas = {}
    heads = {}
    for pose_bone in armature.pose.bones:
        bone = pose_bone.bone
        delta = pose_bone.matrix @ bone.matrix_local.inverted()
        deltas[bone.name] = delta.to_quaternion()
        heads[bone.name] = pose_bone.matrix.translation.copy()

    pose = {}
    for pose_bone in armature.pose.bones:
        bone = pose_bone.bone
        parent = bone.parent
        parent_rot = deltas[parent.name] if parent else Quaternion()
        inverse_parent = parent_rot.inverted()

        rotation = axes.quat_to_voxels(inverse_parent @ deltas[bone.name])
        if parent:
            moved = inverse_parent @ (heads[bone.name] - heads[parent.name])
            offset = axes.to_voxels(moved, voxels_per_unit)
            joint = joints.get(bone.name, (0.0, 0.0, 0.0))
            translation = tuple(offset[i] - joint[i] for i in range(3))
        else:
            # A root's own placement is the sprite's, not a keyframe's; the
            # converter rejects an animated root outright, and this keeps a
            # rest root out of the file entirely.
            translation = axes.to_voxels(heads[bone.name] - bone.matrix_local.translation,
                                         voxels_per_unit)
        entry = rig.xform_entry(translation, rotation, pose_bone.scale)
        if entry:
            pose[bone.name] = entry
    return pose


def sample_action(context, armature, action, voxels_per_unit, joints):
    """Bake `action` into a manifest clip, or `None` if it animates nothing.

    Sampled once per frame over the action's range. A last pose identical to
    the first is dropped and the clip length shortened to match, so a cycle
    authored with a duplicated end frame loops without a one-frame stutter.
    """
    scene = context.scene
    fps = scene.render.fps / scene.render.fps_base
    start, end = (int(round(v)) for v in action.frame_range)
    if end < start:
        return None
    saved_frame = scene.frame_current

    poses = []
    try:
        with _ActionSwap(armature, action):
            for frame in range(start, end + 1):
                scene.frame_set(frame)
                poses.append(_pose_snapshot(armature, voxels_per_unit, joints))
    finally:
        scene.frame_set(saved_frame)

    if not any(poses):
        return None  # the action never touches this armature

    frame_span = end - start + 1
    if len(poses) > 1 and poses[-1] == poses[0]:
        poses.pop()
        frame_span -= 1

    def at(index):
        return round((index) / fps * 1000.0)

    keys = [rig.key_entry(at(i), pose) for i, pose in enumerate(poses)]
    return rig.clip_entry(action.name, keys, at(frame_span), loops=True)


def build_clips(context, armature, voxels_per_unit, joints, all_actions=False):
    """Every exportable action as a clip. Returns `(clips, warnings)`."""
    actions, used_fallback = armature_actions(armature, all_actions)
    warnings = []
    if used_fallback and actions:
        warnings.append(
            f"{armature.name!r} has no action assigned and no NLA tracks; exporting all "
            f"{len(actions)} action(s) in the file"
        )
    clips = []
    skipped = []
    for action in actions:
        clip = sample_action(context, armature, action, voxels_per_unit, joints)
        if clip is None:
            skipped.append(action.name)
        else:
            clips.append(clip)
    if skipped:
        warnings.append("skipped action(s) that animate no bone: " + ", ".join(sorted(skipped)))
    return clips, warnings
