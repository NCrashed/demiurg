//! Editable rigged-character document: a skeleton of bones, each carrying
//! an **editable** voxel mesh, plus named animation clips. Converts to and
//! from the engine's on-disk [`Character`] container (the `.rkc` format),
//! which stores each bone's mesh as a compiled `KV6`.
//!
//! The editor edits a [`Rig`] (one [`VoxelModel`] per bone, with the
//! existing tools); rendering and saving go through [`Rig::to_character`].

use roxlap_formats::character::{
    self, Attachment, Bone as CharBone, Character, Clip, ClipData, ClipPlayback, MeshRef,
};
use roxlap_formats::kfa::{Hinge, Point3, Seq};
use roxlap_formats::xform::BoneXform;

use crate::VoxelModel;
use crate::clip::ClipDoc;

/// Re-export the engine's per-attachment clip playback params (rate + phase).
pub use roxlap_formats::character::ClipPlayback as LayerPlayback;
/// Re-export so editor code can name the keyframe transform type.
pub use roxlap_formats::xform::{BoneXform as KeyXform, Quat};

/// An editable rig: an ordered list of bones (each an editable mesh + a
/// hinge) and named animation clips.
#[derive(Debug, Clone)]
pub struct Rig {
    pub name: String,
    /// World placement of the root bone (passed to the engine sprite).
    pub root: [f32; 3],
    /// Bones in canonical order — position is the bone index used by every
    /// clip's `frmval` column, by `kfaval`, and by `Hinge::parent`.
    pub bones: Vec<RigBone>,
    /// Named animation clips (reused from the engine container).
    pub clips: Vec<Clip>,
}

/// One bone: a name, its primary editable mesh, its hinge, and any extra
/// attachments. `hinge.parent` indexes [`Rig::bones`] (`-1` = root).
///
/// `model` is the bone's primary attachment, drawn at the bone origin
/// (identity offset) — the mesh the existing tools sculpt. `extras` are
/// additional meshes hung off the same bone, each at its own
/// [`RigAttachment::offset`] (e.g. an accessory beside the main mesh). They
/// map to the engine's per-bone attachment list ([`Bone::attachments`]).
#[derive(Debug, Clone)]
pub struct RigBone {
    pub name: String,
    pub model: VoxelModel,
    pub hinge: Hinge,
    pub extras: Vec<RigAttachment>,
    /// When `Some`, the bone's **primary** attachment draws this animated voxel
    /// clip instead of [`Self::model`] (which is then an unused placeholder).
    /// Any attachment — primary or extra — can be a mesh or a clip.
    pub primary_clip: Option<ClipDoc>,
    /// Playback rate + phase for the primary when it is a clip; ignored for a
    /// mesh primary.
    pub primary_playback: ClipPlayback,
}

/// An extra attachment on a bone, positioned by `offset` in the bone's frame
/// (on top of the bone's solved world transform). The primary attachment is the
/// bone's own [`RigBone::model`] / [`RigBone::primary_clip`] at the identity
/// offset; these ride alongside it. Each extra draws either a static mesh
/// ([`Self::model`]) or an animated voxel clip ([`Self::clip`]).
#[derive(Debug, Clone)]
pub struct RigAttachment {
    pub model: VoxelModel,
    /// When `Some`, this attachment draws the clip instead of [`Self::model`].
    pub clip: Option<ClipDoc>,
    /// Playback rate + phase when this attachment is a clip; ignored for a mesh.
    pub playback: ClipPlayback,
    /// Local TRS relative to the bone (engine `local_offset`).
    pub offset: BoneXform,
    /// Artist-facing layer name. Editor metadata (the engine `Attachment` has
    /// no name); persisted in a `DLAY` extra-chunk that round-trips through
    /// `.rkc` (and `.demiurg`), so it survives save/load.
    pub name: String,
}

impl RigAttachment {
    /// A static-mesh extra at `offset` named `name`.
    #[must_use]
    pub fn mesh(model: VoxelModel, offset: BoneXform, name: String) -> Self {
        Self {
            model,
            clip: None,
            playback: ClipPlayback::default(),
            offset,
            name,
        }
    }
}

impl RigBone {
    /// A mesh-primary bone with no clip on its primary attachment — the common
    /// constructor used everywhere a bone is built from a [`VoxelModel`].
    #[must_use]
    pub fn mesh(name: String, model: VoxelModel, hinge: Hinge, extras: Vec<RigAttachment>) -> Self {
        Self {
            name,
            model,
            hinge,
            extras,
            primary_clip: None,
            primary_playback: ClipPlayback::default(),
        }
    }

    /// Whether attachment `i` (`0` = primary, `1..` = extras) draws a clip.
    #[must_use]
    pub fn attachment_is_clip(&self, i: usize) -> bool {
        if i == 0 {
            self.primary_clip.is_some()
        } else {
            self.extras.get(i - 1).is_some_and(|e| e.clip.is_some())
        }
    }

    /// The clip of attachment `i`, or `None` when it's a mesh / out of range.
    #[must_use]
    pub fn attachment_clip(&self, i: usize) -> Option<&ClipDoc> {
        if i == 0 {
            self.primary_clip.as_ref()
        } else {
            self.extras.get(i - 1).and_then(|e| e.clip.as_ref())
        }
    }

    /// Mutable [`Self::attachment_clip`].
    pub fn attachment_clip_mut(&mut self, i: usize) -> Option<&mut ClipDoc> {
        if i == 0 {
            self.primary_clip.as_mut()
        } else {
            self.extras.get_mut(i - 1).and_then(|e| e.clip.as_mut())
        }
    }

    /// Mutable playback params of attachment `i`, or `None` when it's a mesh /
    /// out of range (only a clip attachment has editable playback).
    pub fn attachment_playback_mut(&mut self, i: usize) -> Option<&mut ClipPlayback> {
        if i == 0 {
            self.primary_clip
                .is_some()
                .then_some(&mut self.primary_playback)
        } else {
            self.extras
                .get_mut(i - 1)
                .filter(|e| e.clip.is_some())
                .map(|e| &mut e.playback)
        }
    }

    /// The playback params of attachment `i` (default for a mesh / out of range).
    #[must_use]
    pub fn attachment_playback(&self, i: usize) -> ClipPlayback {
        if i == 0 {
            self.primary_playback
        } else {
            self.extras
                .get(i - 1)
                .map_or_else(ClipPlayback::default, |e| e.playback)
        }
    }
    /// Number of attachments: the primary mesh plus every extra.
    #[must_use]
    pub fn attachment_count(&self) -> usize {
        1 + self.extras.len()
    }

    /// The mesh of attachment `i` — `0` is the primary [`Self::model`], `1..`
    /// index into [`Self::extras`]. `None` if out of range.
    #[must_use]
    pub fn attachment_model(&self, i: usize) -> Option<&VoxelModel> {
        if i == 0 {
            Some(&self.model)
        } else {
            self.extras.get(i - 1).map(|e| &e.model)
        }
    }

    /// Mutable [`Self::attachment_model`].
    pub fn attachment_model_mut(&mut self, i: usize) -> Option<&mut VoxelModel> {
        if i == 0 {
            Some(&mut self.model)
        } else {
            self.extras.get_mut(i - 1).map(|e| &mut e.model)
        }
    }

    /// Append a new extra attachment (a small default mesh at the identity
    /// offset) and return its attachment index (`attachment_count() - 1`).
    pub fn add_extra(&mut self) -> usize {
        let name = format!("layer {}", self.extras.len() + 1);
        self.extras.push(RigAttachment::mesh(
            default_bone_model(),
            BoneXform::IDENTITY,
            name,
        ));
        self.attachment_count() - 1
    }

    /// Make attachment `i` (`0` = primary, `1..` = an extra) draw `clip` instead
    /// of its mesh. The placeholder mesh is left in place (harmless). Returns
    /// `false` for an out-of-range index.
    pub fn set_attachment_clip(&mut self, i: usize, clip: ClipDoc) -> bool {
        if i == 0 {
            self.primary_clip = Some(clip);
            true
        } else if let Some(ex) = self.extras.get_mut(i - 1) {
            ex.clip = Some(clip);
            true
        } else {
            false
        }
    }

    /// Append a new extra attachment that draws `clip` (at the identity offset)
    /// and return its attachment index.
    pub fn add_clip_extra(&mut self, clip: ClipDoc) -> usize {
        let name = format!("layer {}", self.extras.len() + 1);
        let mut att = RigAttachment::mesh(VoxelModel::new(1, 1, 1), BoneXform::IDENTITY, name);
        att.clip = Some(clip);
        self.extras.push(att);
        self.attachment_count() - 1
    }

    /// Remove extra attachment `i` (an attachment index `1..`). Returns
    /// `false` for the primary (`0`) or an out-of-range index — the primary
    /// can't be removed.
    pub fn remove_extra(&mut self, i: usize) -> bool {
        if i == 0 || i > self.extras.len() {
            return false;
        }
        self.extras.remove(i - 1);
        true
    }
}

/// One animation keyframe in **normalized** form: an absolute timestamp and a
/// full-skeleton pose (one [`BoneXform`] per bone — translation, quaternion
/// rotation, scale).
///
/// This is the editor-facing view of a skeletal clip. The on-disk clip is
/// denormalized into two index-joined tables (`frmval` rows + a `seq` of
/// `{tim, frm}` entries with a trailing `!target` loop marker); the
/// `clip_*` / `*_keyframe*` methods on [`Rig`] read and re-bake those tables
/// from a sorted list of `Keyframe`s, so callers never touch the indices or
/// the loop marker directly.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    /// Absolute time of the key, in milliseconds.
    pub tim: i32,
    /// Per-bone local transform (length `== bones.len()`).
    pub xforms: Vec<BoneXform>,
}

impl Rig {
    /// Compile to an engine [`Character`]: each bone's mesh becomes a `KV6`,
    /// carried as a single static attachment (`MeshRef::Static(i)` at the
    /// identity offset — the editor models one static mesh per bone; the
    /// engine's animated voxel clips aren't authored here yet).
    #[must_use]
    pub fn to_character(&self) -> Character {
        // Each bone compiles to its primary mesh (a static attachment at the
        // identity offset) followed by one static attachment per extra (at the
        // extra's `offset`). Meshes are pooled across all bones; an attachment
        // references its mesh by the pool index.
        // Meshes and clips are pooled across all bones; an attachment references
        // its source by the pool index. Each attachment is either a static mesh
        // (`MeshRef::Static`) or an animated voxel clip (`MeshRef::Clip` +
        // `VCLP`), carrying its `local_offset` and (for a clip) playback params.
        let mut meshes = Vec::new();
        let mut voxel_clips = Vec::new();
        let mut bones = Vec::new();
        let push_clip = |voxel_clips: &mut Vec<_>, clip: &ClipDoc, offset, playback| {
            let id = voxel_clips.len();
            voxel_clips.push(clip.to_voxel_clip());
            let mut att = Attachment::clip(id);
            att.local_offset = offset;
            att.playback = playback;
            att
        };
        for b in &self.bones {
            let mut attachments = Vec::with_capacity(1 + b.extras.len());
            // Primary attachment (identity offset): a clip or the bone mesh.
            if let Some(clip) = &b.primary_clip {
                attachments.push(push_clip(
                    &mut voxel_clips,
                    clip,
                    BoneXform::IDENTITY,
                    b.primary_playback,
                ));
            } else {
                let id = meshes.len();
                meshes.push(b.model.to_kv6());
                attachments.push(Attachment::static_mesh(id));
            }
            // Extras, each at its own offset.
            for ex in &b.extras {
                if let Some(clip) = &ex.clip {
                    attachments.push(push_clip(&mut voxel_clips, clip, ex.offset, ex.playback));
                } else {
                    let id = meshes.len();
                    meshes.push(ex.model.to_kv6());
                    let mut att = Attachment::static_mesh(id);
                    att.local_offset = ex.offset;
                    attachments.push(att);
                }
            }
            bones.push(CharBone {
                name: b.name.clone(),
                attachments,
                hinge: b.hinge,
            });
        }
        // Layer names ride along in a `DLAY` extra-chunk (the engine has no
        // attachment name) — a postcard `Vec<String>` of every extra's name in
        // bone-major order, matching how `from_character` reads them back.
        let extra_names: Vec<String> = self
            .bones
            .iter()
            .flat_map(|b| b.extras.iter().map(|e| e.name.clone()))
            .collect();
        let mut extra_chunks = Vec::new();
        if !extra_names.is_empty() {
            if let Ok(payload) = postcard::to_allocvec(&extra_names) {
                extra_chunks.push((DLAY_TAG, payload));
            }
        }
        Character {
            name: self.name.clone(),
            root: self.root,
            meshes,
            bones,
            clips: self.clips.clone(),
            voxel_clips,
            extra_chunks,
        }
    }

    /// Build from an engine [`Character`], decompiling each bone's attachments
    /// to editable meshes / clips.
    ///
    /// Attachment `0` is the bone's primary (a mesh in [`RigBone::model`] or a
    /// clip in [`RigBone::primary_clip`]); the rest become `extras`, each
    /// carrying its own offset, name, and (for a clip) playback. A bone with no
    /// attachments gets an empty editable mesh.
    ///
    /// # Errors
    /// A message if a static attachment's mesh index, or a clip attachment's
    /// clip index, is out of range or undecodable.
    pub fn from_character(c: &Character) -> Result<Self, String> {
        // Layer names from the `DLAY` extra-chunk (bone-major order over the
        // extras, written by `to_character`); empty/absent for a foreign `.rkc`,
        // then extras get a default `layer N` name.
        let names: Vec<String> = c
            .extra_chunks
            .iter()
            .find(|(tag, _)| *tag == DLAY_TAG)
            .and_then(|(_, payload)| postcard::from_bytes(payload).ok())
            .unwrap_or_default();
        let mut names = names.into_iter();
        let bones = c
            .bones
            .iter()
            .map(|b| {
                let mut bone = RigBone::mesh(
                    b.name.clone(),
                    // Default for a bone with no attachments (overwritten below
                    // if attachment 0 is a static mesh).
                    VoxelModel::new(1, 1, 1),
                    b.hinge,
                    Vec::new(),
                );
                for (idx, a) in b.attachments.iter().enumerate() {
                    // Resolve the attachment to (mesh, optional clip).
                    let (model, clip) = match a.target {
                        MeshRef::Static(i) => {
                            let kv6 = c.meshes.get(i).ok_or_else(|| {
                                format!("bone {:?}: mesh index {i} out of range", b.name)
                            })?;
                            (VoxelModel::from_kv6(kv6), None)
                        }
                        MeshRef::Clip(i) => {
                            let vc = c.voxel_clips.get(i).ok_or_else(|| {
                                format!("bone {:?}: clip index {i} out of range", b.name)
                            })?;
                            let doc = ClipDoc::from_voxel_clip(vc).map_err(|e| {
                                format!("bone {:?}: clip {i} undecodable: {e:?}", b.name)
                            })?;
                            // A clip attachment keeps an empty placeholder mesh.
                            (VoxelModel::new(1, 1, 1), Some(doc))
                        }
                    };
                    if idx == 0 {
                        bone.model = model;
                        bone.primary_clip = clip;
                        bone.primary_playback = a.playback;
                    } else {
                        let name = names
                            .next()
                            .unwrap_or_else(|| format!("layer {}", bone.extras.len() + 1));
                        bone.extras.push(RigAttachment {
                            model,
                            clip,
                            playback: a.playback,
                            offset: a.local_offset,
                            name,
                        });
                    }
                }
                Ok(bone)
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            name: c.name.clone(),
            root: c.root,
            bones,
            clips: c.clips.clone(),
        })
    }

    /// Serialize to `.rkc` bytes (via [`Self::to_character`]).
    #[must_use]
    pub fn to_rkc_bytes(&self) -> Vec<u8> {
        character::serialize(&self.to_character())
    }

    /// Parse `.rkc` bytes into an editable rig.
    ///
    /// # Errors
    /// A message if the bytes aren't a valid `.rkc`, or a mesh can't be
    /// resolved.
    pub fn from_rkc_bytes(bytes: &[u8]) -> Result<Self, String> {
        let c = character::parse(bytes).map_err(|e| e.to_string())?;
        Self::from_character(&c)
    }

    /// A fresh rig with a single root bone carrying `model` (or a default cube
    /// when `model` is `None`) and no animation clips — the starting point for
    /// "New rig" / "Convert model to rig". Add child bones (or a dummy root)
    /// and a clip to animate it.
    #[must_use]
    pub fn single_bone(name: impl Into<String>, model: Option<VoxelModel>) -> Self {
        Self {
            name: name.into(),
            root: [0.0, 0.0, 0.0],
            bones: vec![RigBone::mesh(
                "root".to_string(),
                model.unwrap_or_else(default_bone_model),
                free_hinge(-1, Z_AXIS, ZERO),
                Vec::new(),
            )],
            clips: Vec::new(),
        }
    }

    /// Append a new bone (parented to `parent`, `-1` = root) and return its
    /// index. The new bone gets a small visible box mesh, a default hinge,
    /// and a fresh `0` animation column in every skeletal clip so that
    /// `frmval[*].len()` stays equal to `bones.len()`.
    pub fn add_bone(&mut self, parent: i32) -> usize {
        let n = self.bones.len();
        // Offset the new bone's joint to the +X edge of its parent so it
        // appears *beside* the parent rather than buried inside its mesh
        // (a coincident bone at the origin looks like nothing was added).
        let joint = usize::try_from(parent)
            .ok()
            .and_then(|p| self.bones.get(p))
            .map_or(ZERO, |p| {
                #[allow(clippy::cast_precision_loss)] // mesh dims are tiny
                let x = p.model.dims().0 as f32;
                Point3 { x, y: 0.0, z: 0.0 }
            });
        self.bones.push(RigBone::mesh(
            format!("bone {n}"),
            default_bone_model(),
            free_hinge(parent, Z_AXIS, joint),
            Vec::new(),
        ));
        for clip in &mut self.clips {
            if let ClipData::Skeletal { frmval, .. } = &mut clip.data {
                for row in frmval {
                    row.push(BoneXform::IDENTITY);
                }
            }
        }
        n
    }

    /// Append `model` as a new child bone of `parent` (`-1` = root), named
    /// `name`, with a free Z-hinge whose parent-side joint sits at `joint`
    /// (parent-local, relative to the parent's mesh pivot), and a fresh `0`
    /// column in every skeletal clip so `frmval[*].len()` stays equal to
    /// `bones.len()`. Returns the new index.
    ///
    /// Unlike [`Self::add_bone`] the caller supplies the whole mesh, its pivot,
    /// and the joint — used to extract a carved-out selection into its own
    /// bone: with the joint at the cut and the mesh pivot pre-offset to match,
    /// the piece keeps its exact place at rest and rotates about the seam (the
    /// artist then tunes both).
    pub fn add_child_mesh(
        &mut self,
        parent: i32,
        name: String,
        model: VoxelModel,
        joint: [f32; 3],
    ) -> usize {
        let n = self.bones.len();
        let joint = Point3 {
            x: joint[0],
            y: joint[1],
            z: joint[2],
        };
        self.bones.push(RigBone::mesh(
            name,
            model,
            free_hinge(parent, Z_AXIS, joint),
            Vec::new(),
        ));
        for clip in &mut self.clips {
            if let ClipData::Skeletal { frmval, .. } = &mut clip.data {
                for row in frmval {
                    row.push(BoneXform::IDENTITY);
                }
            }
        }
        n
    }

    /// Delete bone `i`, keeping the rig consistent. Children of `i` are
    /// reparented to `i`'s parent so the subtree survives; every
    /// `hinge.parent` index is remapped for the removal, and column `i` is
    /// dropped from every skeletal clip's `frmval` rows.
    ///
    /// No-op (returns `false`) when there are fewer than two bones, when `i`
    /// is out of range, or when `i` is a root (`parent == -1`) — the rig must
    /// always keep a root.
    pub fn delete_bone(&mut self, i: usize) -> bool {
        if self.bones.len() < 2 || i >= self.bones.len() {
            return false;
        }
        let parent = self.bones[i].hinge.parent;
        if parent < 0 {
            return false; // never delete a root bone
        }
        // Reparent children of `i` to `i`'s parent, then shift indices for
        // every parent that pointed past the removed slot.
        let removed = i32::try_from(i).unwrap_or(i32::MAX); // bone counts are tiny
        for (j, b) in self.bones.iter_mut().enumerate() {
            if j == i {
                continue;
            }
            let p = b.hinge.parent;
            if p == removed {
                b.hinge.parent = parent;
            } else if p > removed {
                b.hinge.parent = p - 1;
            }
        }
        self.bones.remove(i);
        for clip in &mut self.clips {
            if let ClipData::Skeletal { frmval, .. } = &mut clip.data {
                for row in frmval {
                    if i < row.len() {
                        row.remove(i);
                    }
                }
            }
        }
        true
    }

    /// Duplicate bone `i`, cloning its mesh and hinge, and return the new
    /// index. The copy's joint is offset along +X by the mesh width so it
    /// sits beside the original instead of exactly on top of it. Each clip
    /// gains a new column copied from bone `i`'s, so the duplicate animates
    /// identically (and `frmval[*].len()` stays correct).
    ///
    /// A **child** bone is copied as a sibling (same parent), offsetting its
    /// parent-side joint `p[1]`. A **root** can't be offset that way (roots
    /// ignore `p[1]`) and a second root just confuses the rig, so a root's
    /// copy is parented to the original root as a child — a visible, editable
    /// copy rather than an overlapping duplicate root.
    ///
    /// Returns `None` if `i` is out of range.
    pub fn duplicate_bone(&mut self, i: usize) -> Option<usize> {
        let src = self.bones.get(i)?;
        let model = src.model.clone();
        let extras = src.extras.clone();
        let src_primary_clip = src.primary_clip.clone();
        let src_primary_playback = src.primary_playback;
        let mut hinge = src.hinge;
        let name = format!("{} copy", src.name);
        #[allow(clippy::cast_precision_loss)] // mesh dims are tiny
        let dx = model.dims().0 as f32;
        if hinge.parent >= 0 {
            // Child: sibling copy, parent-side joint nudged off the original.
            hinge.p[1].x += dx;
        } else {
            // Root: re-parent the copy under the original root and offset it
            // like any child (p[0] = own pivot, p[1] = joint in root space).
            hinge.parent = i32::try_from(i).unwrap_or(-1);
            hinge.p = [
                ZERO,
                Point3 {
                    x: dx,
                    y: 0.0,
                    z: 0.0,
                },
            ];
        }
        let new = self.bones.len();
        let mut bone = RigBone::mesh(name, model, hinge, extras);
        bone.primary_clip = src_primary_clip;
        bone.primary_playback = src_primary_playback;
        self.bones.push(bone);
        for clip in &mut self.clips {
            if let ClipData::Skeletal { frmval, .. } = &mut clip.data {
                for row in frmval {
                    let v = row.get(i).copied().unwrap_or(BoneXform::IDENTITY);
                    row.push(v);
                }
            }
        }
        Some(new)
    }

    /// Move bone `from` to index `to`, keeping the rig consistent: every
    /// `hinge.parent` index is remapped through the permutation, and each
    /// clip's `frmval` columns are reordered the same way. Bone order is
    /// purely organisational (the limb solver topologically sorts hinges
    /// itself), so any permutation is valid as long as the indices follow.
    ///
    /// No-op (returns `false`) if either index is out of range or `from == to`.
    pub fn move_bone(&mut self, from: usize, to: usize) -> bool {
        let n = self.bones.len();
        if from >= n || to >= n || from == to {
            return false;
        }
        let b = self.bones.remove(from);
        self.bones.insert(to, b);
        // Remap parent indices through the same move.
        for bone in &mut self.bones {
            if let Ok(p) = usize::try_from(bone.hinge.parent) {
                bone.hinge.parent = i32::try_from(remap_index(p, from, to)).unwrap_or(-1);
            }
        }
        // Reorder every clip's per-bone columns identically.
        for clip in &mut self.clips {
            if let ClipData::Skeletal { frmval, .. } = &mut clip.data {
                for row in frmval {
                    if from < row.len() && to < row.len() {
                        let v = row.remove(from);
                        row.insert(to, v);
                    }
                }
            }
        }
        true
    }

    // --- Rig-building helpers (multi-DOF joints) ----------------------------

    /// Add a 3-DOF "ball" joint under `parent`: a chain of three 1-DOF rotator
    /// bones (axes X -> Y -> Z), the last carrying a default visible mesh.
    /// Returns the leaf (visible) bone's index.
    ///
    /// The format animates exactly one angle per bone about a fixed axis, so a
    /// multi-axis joint is built from one bone per axis (the standard voxlap
    /// trick). The two upper rotators are zero-length with empty (invisible)
    /// meshes and exist only to compose rotation; posing all three gives a full
    /// 3-axis orientation. Every clip gains three fresh `0` columns so
    /// `frmval[*].len()` stays equal to `bones.len()`.
    pub fn add_axis_joint(&mut self, parent: i32) -> usize {
        let n = self.bones.len();
        // Offset the joint to the parent's +X edge (like `add_bone`) so it sits
        // beside the parent; only the first rotator's parent-side velcro
        // carries the offset, the rest are coincident.
        let joint = usize::try_from(parent)
            .ok()
            .and_then(|p| self.bones.get(p))
            .map_or(ZERO, |p| {
                #[allow(clippy::cast_precision_loss)] // mesh dims are tiny
                let x = p.model.dims().0 as f32;
                Point3 { x, y: 0.0, z: 0.0 }
            });
        let leaf = format!("bone {}", n + 2);
        let rot_x = i32::try_from(n).unwrap_or(-1);
        let rot_y = i32::try_from(n + 1).unwrap_or(-1);
        self.bones.push(RigBone::mesh(
            format!("{leaf} rotX"),
            empty_bone_model(),
            free_hinge(parent, X_AXIS, joint),
            Vec::new(),
        ));
        self.bones.push(RigBone::mesh(
            format!("{leaf} rotY"),
            empty_bone_model(),
            free_hinge(rot_x, Y_AXIS, ZERO),
            Vec::new(),
        ));
        self.bones.push(RigBone::mesh(
            leaf,
            default_bone_model(),
            free_hinge(rot_y, Z_AXIS, ZERO),
            Vec::new(),
        ));
        for clip in &mut self.clips {
            if let ClipData::Skeletal { frmval, .. } = &mut clip.data {
                for row in frmval {
                    row.extend_from_slice(&[BoneXform::IDENTITY; 3]);
                }
            }
        }
        n + 2 // the leaf
    }

    /// Insert an empty "dummy" root above the current root so the old root
    /// becomes an animatable child (a keyframed clip can't pose a `parent < 0`
    /// bone — the solver takes the root orientation from the sprite basis, not
    /// `frmval`). The old root is re-parented to the new dummy at the same
    /// spot, with a free rotation range. Returns the dummy's index, or `None`
    /// if the rig has no root.
    pub fn add_dummy_root(&mut self) -> Option<usize> {
        let old_root = self.bones.iter().position(|b| b.hinge.parent < 0)?;
        let dummy = self.bones.len();
        let dummy_parent = i32::try_from(dummy).unwrap_or(-1);
        // Re-parent the old root onto the dummy, coincident, animatable.
        let h = &mut self.bones[old_root].hinge;
        h.parent = dummy_parent;
        h.p = [ZERO, ZERO];
        h.vmin = i16::MIN;
        h.vmax = i16::MAX;
        self.bones.push(RigBone::mesh(
            "origin".to_string(),
            empty_bone_model(),
            free_hinge(-1, Z_AXIS, ZERO),
            Vec::new(),
        ));
        for clip in &mut self.clips {
            if let ClipData::Skeletal { frmval, .. } = &mut clip.data {
                for row in frmval {
                    row.push(BoneXform::IDENTITY);
                }
            }
        }
        Some(dummy)
    }

    // --- Clip management (Animate mode) -------------------------------------

    /// Append a new skeletal clip named `name` with a single rest-pose key at
    /// t=0 (all-zero angles, one column per bone) and a loop marker at
    /// [`DEFAULT_TAIL_MS`], and return its index. The lone key makes the clip
    /// immediately previewable / editable rather than an empty timeline.
    pub fn add_clip(&mut self, name: String) -> usize {
        self.clips.push(Clip {
            name,
            data: ClipData::Skeletal {
                frmval: vec![vec![BoneXform::IDENTITY; self.bones.len()]],
                seq: vec![
                    Seq { tim: 0, frm: 0 },
                    Seq {
                        tim: DEFAULT_TAIL_MS,
                        frm: !0, // loop back to entry 0
                    },
                ],
            },
        });
        self.clips.len() - 1
    }

    /// Rename clip `i`. Returns `false` if `i` is out of range.
    pub fn rename_clip(&mut self, i: usize, name: String) -> bool {
        match self.clips.get_mut(i) {
            Some(c) => {
                c.name = name;
                true
            }
            None => false,
        }
    }

    /// Delete clip `i` (the rig may end up with no clips). Returns `false` if
    /// `i` is out of range.
    pub fn remove_clip(&mut self, i: usize) -> bool {
        if i >= self.clips.len() {
            return false;
        }
        self.clips.remove(i);
        true
    }

    // --- Keyframe authoring (Animate mode) ----------------------------------
    //
    // These expose a skeletal clip as a sorted list of [`Keyframe`]s and
    // re-bake the denormalized `seq`/`frmval` tables (incl. the `!0` loop
    // marker) from that list on every edit. The first edit normalizes the
    // clip: `frmval` rows line up 1:1 with keyframes in `tim` order, and the
    // single trailing `seq` entry always loops back to entry 0.

    /// Whether bone `bone` can be posed (keyframed): a child (`parent >= 0` —
    /// roots take their orientation from the sprite basis, never a keyframe)
    /// with a non-empty hinge range (`vmin < vmax` — a `vmin == vmax` hinge is
    /// locked). The viewport posing gesture and its angle editor share this rule.
    #[must_use]
    pub fn is_poseable(&self, bone: usize) -> bool {
        self.bones
            .get(bone)
            .is_some_and(|b| b.hinge.parent >= 0 && b.hinge.vmin < b.hinge.vmax)
    }

    /// The clip's keyframes in time order (empty if `clip` is out of range or
    /// not a skeletal clip). The loop marker is excluded — these are the real,
    /// poseable keys.
    #[must_use]
    pub fn clip_keyframes(&self, clip: usize) -> Vec<Keyframe> {
        self.read_clip(clip)
            .map(|(kfs, _, _)| kfs)
            .unwrap_or_default()
    }

    /// The clip's loop length in ms: the timestamp of the trailing loop marker
    /// (== the playback duration). `0` if `clip` is out of range / not skeletal.
    #[must_use]
    pub fn clip_loop_tim(&self, clip: usize) -> i32 {
        self.read_clip(clip).map_or(0, |(_, lt, _)| lt)
    }

    /// Insert a key at `tim` (clamped `>= 0`) with `xforms` as the pose
    /// (resized to `bones.len()` with identity transforms), and return its
    /// index in the sorted key list. A key already at exactly `tim` is
    /// overwritten in place. `None` if `clip` is out of range / not skeletal.
    pub fn add_keyframe(
        &mut self,
        clip: usize,
        tim: i32,
        mut xforms: Vec<BoneXform>,
    ) -> Option<usize> {
        let (mut kfs, loop_tim, loops) = self.read_clip(clip)?;
        let tim = tim.max(0);
        xforms.resize(self.bones.len(), BoneXform::IDENTITY);
        if let Some(existing) = kfs.iter().position(|k| k.tim == tim) {
            kfs[existing].xforms = xforms;
        } else {
            kfs.push(Keyframe { tim, xforms });
        }
        kfs.sort_by_key(|k| k.tim);
        let idx = kfs.iter().position(|k| k.tim == tim)?;
        self.write_clip(clip, kfs, loop_tim, loops);
        Some(idx)
    }

    /// Overwrite key `k`'s entire pose (resized like [`Self::add_keyframe`]).
    /// Returns `false` if the clip / key is out of range.
    pub fn set_keyframe_pose(&mut self, clip: usize, k: usize, mut xforms: Vec<BoneXform>) -> bool {
        let Some((mut kfs, loop_tim, loops)) = self.read_clip(clip) else {
            return false;
        };
        if k >= kfs.len() {
            return false;
        }
        xforms.resize(self.bones.len(), BoneXform::IDENTITY);
        kfs[k].xforms = xforms;
        self.write_clip(clip, kfs, loop_tim, loops);
        true
    }

    /// Set one bone's **rotation** in key `k` to a pure hinge rotation about
    /// its axis by `v` (Q15, clamped to `vmin..=vmax`), keeping the bone's
    /// translation and scale. This is the 1-DOF angle control (slider / tick
    /// drag); free rotation comes from the viewport pose gesture. Returns
    /// `false` if the clip / key / bone is out of range.
    pub fn set_keyframe_angle(&mut self, clip: usize, k: usize, bone: usize, v: i16) -> bool {
        let Some((mut kfs, loop_tim, loops)) = self.read_clip(clip) else {
            return false;
        };
        let clamped = clamp_angle(v, self.bones.get(bone));
        let axis = self.bones.get(bone).map(|b| b.hinge.v[0]);
        let (Some(kf), Some(axis)) = (kfs.get_mut(k), axis) else {
            return false;
        };
        let Some(slot) = kf.xforms.get_mut(bone) else {
            return false;
        };
        slot.r = BoneXform::from_hinge_angle([axis.x, axis.y, axis.z], clamped).r;
        self.write_clip(clip, kfs, loop_tim, loops);
        true
    }

    /// Set bone `bone`'s translation in key `k`, keeping its rotation and
    /// scale. Returns `false` if the clip / key / bone is out of range.
    pub fn set_keyframe_translation(
        &mut self,
        clip: usize,
        k: usize,
        bone: usize,
        t: [f32; 3],
    ) -> bool {
        self.edit_keyframe_xform(clip, k, bone, |x| x.t = t)
    }

    /// Set bone `bone`'s scale in key `k`, keeping its translation and
    /// rotation. Returns `false` if the clip / key / bone is out of range.
    pub fn set_keyframe_scale(&mut self, clip: usize, k: usize, bone: usize, s: [f32; 3]) -> bool {
        self.edit_keyframe_xform(clip, k, bone, |x| x.s = s)
    }

    /// Set bone `bone`'s full rotation in key `k`, keeping its translation and
    /// scale — free 3-DOF, unlike [`Self::set_keyframe_angle`] (which is the
    /// 1-DOF hinge case). Returns `false` if the clip / key / bone is out of
    /// range.
    pub fn set_keyframe_rotation(&mut self, clip: usize, k: usize, bone: usize, r: Quat) -> bool {
        self.edit_keyframe_xform(clip, k, bone, |x| x.r = r)
    }

    /// Apply `edit` to bone `bone`'s transform in key `k`, then re-bake.
    /// Returns `false` if the clip / key / bone is out of range.
    fn edit_keyframe_xform(
        &mut self,
        clip: usize,
        k: usize,
        bone: usize,
        edit: impl FnOnce(&mut BoneXform),
    ) -> bool {
        let Some((mut kfs, loop_tim, loops)) = self.read_clip(clip) else {
            return false;
        };
        let Some(slot) = kfs.get_mut(k).and_then(|kf| kf.xforms.get_mut(bone)) else {
            return false;
        };
        edit(slot);
        self.write_clip(clip, kfs, loop_tim, loops);
        true
    }

    /// Retime key `k` to `new_tim` (clamped `>= 0`), re-sort, and return its
    /// new index. No-op (`None`) if the clip / key is out of range or another
    /// key already sits at `new_tim`.
    pub fn move_keyframe(&mut self, clip: usize, k: usize, new_tim: i32) -> Option<usize> {
        let (mut kfs, loop_tim, loops) = self.read_clip(clip)?;
        if k >= kfs.len() {
            return None;
        }
        let new_tim = new_tim.max(0);
        if kfs
            .iter()
            .enumerate()
            .any(|(i, kf)| i != k && kf.tim == new_tim)
        {
            return None; // would collide with an existing key
        }
        kfs[k].tim = new_tim;
        kfs.sort_by_key(|kf| kf.tim);
        let idx = kfs.iter().position(|kf| kf.tim == new_tim)?;
        self.write_clip(clip, kfs, loop_tim, loops);
        Some(idx)
    }

    /// Delete key `k`. Refuses (`false`) to remove the last remaining key (an
    /// empty clip would have nothing to loop to) or an out-of-range index.
    pub fn remove_keyframe(&mut self, clip: usize, k: usize) -> bool {
        let Some((mut kfs, loop_tim, loops)) = self.read_clip(clip) else {
            return false;
        };
        if k >= kfs.len() || kfs.len() <= 1 {
            return false;
        }
        kfs.remove(k);
        self.write_clip(clip, kfs, loop_tim, loops);
        true
    }

    /// Whether `clip` loops (the trailing marker returns to the start) versus
    /// playing once and holding the last frame. `false` if out of range / not
    /// skeletal.
    #[must_use]
    pub fn clip_loops(&self, clip: usize) -> bool {
        self.read_clip(clip).is_some_and(|(_, _, loops)| loops)
    }

    /// Set `clip`'s length (the trailing marker's time / playback duration) to
    /// `ms`, keeping the keyframes and loop mode. `ms` is clamped strictly
    /// after the last key (a marker at/before it would leave no final segment).
    /// `false` if the clip is out of range / not skeletal.
    pub fn set_clip_length(&mut self, clip: usize, ms: i32) -> bool {
        let Some((kfs, _, loops)) = self.read_clip(clip) else {
            return false;
        };
        // write_clip enforces `loop_tim > last key` itself; pass the request as
        // the desired marker time and let it clamp.
        self.write_clip(clip, kfs, ms, loops);
        true
    }

    /// Set whether `clip` loops back to the start (`true`) or plays once and
    /// holds its last frame (`false`), keeping keyframes and length. `false`
    /// if the clip is out of range / not skeletal.
    pub fn set_clip_loops(&mut self, clip: usize, loops: bool) -> bool {
        let Some((kfs, loop_tim, _)) = self.read_clip(clip) else {
            return false;
        };
        self.write_clip(clip, kfs, loop_tim, loops);
        true
    }

    /// Read a skeletal clip as `(sorted keyframes, loop_tim, loops)`. The loop
    /// marker (`frm < 0`) is dropped from the key list; only real frames
    /// (`frm >= 0`) become keys. `loops` is whether the trailing marker loops
    /// back to the start (`frm == !0`) versus a self-jump (`frm == !own_index`)
    /// that plays once and holds the last frame — see [`Self::write_clip`].
    fn read_clip(&self, clip: usize) -> Option<(Vec<Keyframe>, i32, bool)> {
        let ClipData::Skeletal { frmval, seq } = &self.clips.get(clip)?.data else {
            return None;
        };
        let mut kfs: Vec<Keyframe> = seq
            .iter()
            .filter(|s| s.frm >= 0)
            .filter_map(|s| {
                let row = frmval.get(usize::try_from(s.frm).ok()?)?;
                Some(Keyframe {
                    tim: s.tim,
                    xforms: row.clone(),
                })
            })
            .collect();
        kfs.sort_by_key(|k| k.tim);
        let loop_tim = seq.iter().map(|s| s.tim).max().unwrap_or(0);
        // The trailing marker (the negative-`frm` entry at the latest time)
        // loops iff it jumps back to entry 0 (`frm == !0 == -1`); any other
        // self-jump marker holds the last frame.
        let loops = seq
            .iter()
            .filter(|s| s.frm < 0)
            .max_by_key(|s| s.tim)
            .is_none_or(|s| s.frm == !0);
        Some((kfs, loop_tim, loops))
    }

    /// Re-bake `clip`'s `seq`/`frmval` from a keyframe list: `frmval` rows in
    /// `tim` order, a 1:1 `seq` (`frm == row index`), and a trailing marker at
    /// `loop_tim`. The marker is kept strictly after the last key (extended by
    /// [`DEFAULT_TAIL_MS`] when a key would meet or pass it) so there's always
    /// a final segment. `loops` chooses the marker: `frm == !0` loops back to
    /// the start (a return-to-start segment); otherwise a self-jump
    /// (`frm == !own_index`) plays the clip once and holds the last frame (the
    /// engine breaks Phase-1 advance on a self-jump and skips the Phase-2 blend
    /// — see `KfaSprite::animsprite`).
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // key counts are tiny
    fn write_clip(&mut self, clip: usize, mut kfs: Vec<Keyframe>, prev_loop: i32, loops: bool) {
        kfs.sort_by_key(|k| k.tim);
        let max_tim = kfs.iter().map(|k| k.tim).max().unwrap_or(0);
        let loop_tim = if prev_loop > max_tim {
            prev_loop
        } else {
            max_tim + DEFAULT_TAIL_MS
        };
        let frmval: Vec<Vec<BoneXform>> = kfs.iter().map(|k| k.xforms.clone()).collect();
        let mut seq: Vec<Seq> = kfs
            .iter()
            .enumerate()
            .map(|(i, k)| Seq {
                tim: k.tim,
                frm: i as i32,
            })
            .collect();
        let marker_idx = seq.len() as i32; // the marker's own index in `seq`
        seq.push(Seq {
            tim: loop_tim,
            frm: if loops { !0 } else { !marker_idx },
        });
        if let Some(c) = self.clips.get_mut(clip) {
            c.data = ClipData::Skeletal { frmval, seq };
        }
    }
}

/// Clamp an i16 hinge angle to a bone's `vmin..=vmax` range. A missing bone
/// (or one with `vmin == vmax`) pins the value; the bounds are normalized so a
/// malformed `vmin > vmax` can't panic `clamp`.
fn clamp_angle(v: i16, bone: Option<&RigBone>) -> i16 {
    let (a, b) = bone.map_or((i16::MIN, i16::MAX), |bn| (bn.hinge.vmin, bn.hinge.vmax));
    v.clamp(a.min(b), a.max(b))
}

/// New index of an element originally at `old` after the element at `from` is
/// removed and re-inserted at `to` (a `Vec::remove` + `Vec::insert`). The
/// moved element lands on `to`; the span between `from` and `to` shifts by one
/// toward the vacated slot; everything else is unchanged.
fn remap_index(old: usize, from: usize, to: usize) -> usize {
    if old == from {
        to
    } else if from < to && old > from && old <= to {
        old - 1
    } else if from > to && old >= to && old < from {
        old + 1
    } else {
        old
    }
}

/// When a new/retimed keyframe meets or passes the loop marker, push the
/// marker this many ms beyond the last key so there's always a visible
/// return-to-start segment (matches the demo clip's inter-key spacing).
const DEFAULT_TAIL_MS: i32 = 500;

/// Tag of the `.rkc` extra-chunk that carries the editor's per-layer names (a
/// postcard `Vec<String>` in bone-major order). Unknown to the engine, so it's
/// round-tripped verbatim via [`Character::extra_chunks`].
const DLAY_TAG: [u8; 4] = *b"DLAY";

/// The origin point, reused for default hinge endpoints.
const ZERO: Point3 = Point3 {
    x: 0.0,
    y: 0.0,
    z: 0.0,
};

/// The +Z unit axis. A hinge's rotation axis (`v`) must be a non-zero unit
/// vector: the limb solver runs it through `genperp`, and a zero axis yields
/// a degenerate basis that collapses the limb to a point (invisible).
const Z_AXIS: Point3 = Point3 {
    x: 0.0,
    y: 0.0,
    z: 1.0,
};

/// The +X unit axis (a 3-axis joint's first rotator).
const X_AXIS: Point3 = Point3 {
    x: 1.0,
    y: 0.0,
    z: 0.0,
};

/// The +Y unit axis (a 3-axis joint's second rotator).
const Y_AXIS: Point3 = Point3 {
    x: 0.0,
    y: 1.0,
    z: 0.0,
};

/// An empty 1x1x1 mesh (zero voxels) for an invisible "rotator" bone — the
/// zero-length helper bones of a 3-axis joint carry no geometry.
fn empty_bone_model() -> VoxelModel {
    VoxelModel::new(1, 1, 1)
}

/// A small, visible default mesh for a freshly added bone: a solid box of
/// the centre voxels in a 3×3×3 grid.
fn default_bone_model() -> VoxelModel {
    let mut model = VoxelModel::new(3, 3, 3);
    for z in 0..3 {
        for y in 0..3 {
            for x in 0..3 {
                model.set(x, y, z, 0x80c0_c0c0);
            }
        }
    }
    model
}

/// A neutral hinge with the bone's joint at `joint` in the parent's frame
/// (the child mesh sits centred there), rotating about `axis`. The rotation
/// range is **free** (`i16::MIN..=i16::MAX`) so the bone is animatable out of
/// the box — there is no in-editor control to widen a locked range, and the
/// format only animates this one angle per bone, so a locked default would
/// make a freshly added bone impossible to pose.
fn free_hinge(parent: i32, axis: Point3, joint: Point3) -> Hinge {
    Hinge {
        parent,
        // p[0] = child-side attach (its own pivot); p[1] = parent-side joint.
        p: [ZERO, joint],
        // A valid (non-zero) rotation axis — see Z_AXIS.
        v: [axis, axis],
        vmin: i16::MIN,
        vmax: i16::MAX,
        htype: 0,
        filler: [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxlap_formats::kfa::Point3;

    fn bone(name: &str, parent: i32, fill: u32) -> RigBone {
        let mut model = VoxelModel::new(3, 3, 3);
        model.set(1, 1, 1, fill);
        let zero = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        RigBone::mesh(
            name.to_string(),
            model,
            Hinge {
                parent,
                p: [zero, zero],
                v: [zero, zero],
                vmin: 0,
                vmax: 0,
                htype: 0,
                filler: [0; 7],
            },
            Vec::new(),
        )
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact small values
    fn round_trips_through_character() {
        let rig = Rig {
            name: "t".to_string(),
            root: [1.0, 2.0, 3.0],
            bones: vec![bone("body", -1, 0x80ff_0000), bone("arm", 0, 0x8000_ff00)],
            clips: Vec::new(),
        };
        let back = Rig::from_character(&rig.to_character()).expect("round-trips");
        assert_eq!(back.bones.len(), 2);
        assert_eq!(back.name, "t");
        assert_eq!(back.root, [1.0, 2.0, 3.0]);
        assert_eq!(back.bones[1].name, "arm");
        assert_eq!(back.bones[1].hinge.parent, 0);
        // The decompiled mesh keeps the painted voxel.
        assert_eq!(back.bones[0].model.get(1, 1, 1), 0x80ff_0000);
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact small values
    fn extra_attachments_round_trip_through_rkc() {
        let mut rig = Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones: vec![bone("body", -1, 0x80ff_0000)],
            clips: Vec::new(),
        };
        // Hang an extra mesh (distinct colour) off the bone at an offset.
        let mut extra = VoxelModel::new(2, 2, 2);
        extra.set(0, 0, 0, 0x8000_ff00);
        rig.bones[0].extras.push(RigAttachment::mesh(
            extra,
            BoneXform {
                t: [3.0, 0.0, -1.0],
                ..BoneXform::IDENTITY
            },
            "horn".to_string(),
        ));
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("round-trips");
        assert_eq!(back.bones.len(), 1);
        // Primary mesh survives as the bone's `model`.
        assert_eq!(back.bones[0].model.get(1, 1, 1), 0x80ff_0000);
        // The extra survives with its own mesh + offset + name.
        assert_eq!(back.bones[0].extras.len(), 1);
        assert_eq!(back.bones[0].extras[0].model.get(0, 0, 0), 0x8000_ff00);
        assert_eq!(back.bones[0].extras[0].offset.t, [3.0, 0.0, -1.0]);
        assert_eq!(
            back.bones[0].extras[0].name, "horn",
            "layer name survives .rkc"
        );
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact small offset values
    fn clip_attachments_round_trip_through_rkc() {
        use crate::clip::ClipDoc;

        // A two-frame clip with a moving voxel — distinct enough to verify the
        // frames survive the .rkc VCLP encode/decode.
        let clip_doc = |col: u32| {
            let mut c = ClipDoc::new([4, 4, 4]);
            c.frames[0].model.set(0, 0, 0, col);
            let f = c.add_frame();
            c.frames[f].model.set(1, 0, 0, col);
            c
        };

        let mut rig = Rig {
            name: "fx".to_string(),
            root: [0.0; 3],
            bones: vec![bone("torch", -1, 0x80ff_0000)],
            clips: Vec::new(),
        };
        // Primary becomes a clip with non-default playback (2x speed, phased).
        rig.bones[0].primary_clip = Some(clip_doc(0x8000_00ff));
        rig.bones[0].primary_playback = LayerPlayback {
            speed_q8: 512,
            start_phase_ms: 40,
        };
        // …plus a clip extra at an offset.
        rig.bones[0].extras.push(RigAttachment {
            model: VoxelModel::new(1, 1, 1),
            clip: Some(clip_doc(0x8000_ff00)),
            playback: LayerPlayback::default(),
            offset: BoneXform {
                t: [2.0, 0.0, 0.0],
                ..BoneXform::IDENTITY
            },
            name: "flame".to_string(),
        });

        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("round-trips");
        let b = &back.bones[0];
        // Primary clip survives with its frames + playback.
        assert!(b.attachment_is_clip(0), "primary is a clip");
        let pc = b.primary_clip.as_ref().expect("primary clip");
        assert_eq!(pc.frame_count(), 2);
        assert_eq!(pc.frames[1].model.get(1, 0, 0), 0x8000_00ff);
        assert_eq!(b.primary_playback.speed_q8, 512);
        assert_eq!(b.primary_playback.start_phase_ms, 40);
        // Clip extra survives with its frames + offset + name.
        assert_eq!(b.extras.len(), 1);
        assert!(b.attachment_is_clip(1), "extra is a clip");
        let ec = b.extras[0].clip.as_ref().expect("extra clip");
        assert_eq!(ec.frames[1].model.get(1, 0, 0), 0x8000_ff00);
        assert_eq!(b.extras[0].offset.t, [2.0, 0.0, 0.0]);
        assert_eq!(b.extras[0].name, "flame");
    }

    use roxlap_formats::character::{Clip, ClipData};
    use roxlap_formats::kfa::Seq;

    /// A `BoneXform` carrying a recognisable `i16` marker in its translation `x`,
    /// so column reshuffles (add / delete / reorder bone) stay checkable.
    fn mark(v: i16) -> BoneXform {
        BoneXform {
            t: [f32::from(v), 0.0, 0.0],
            ..BoneXform::IDENTITY
        }
    }

    /// A 2-frame skeletal clip with `nbones` columns, each cell marked
    /// `frame*10 + bone` (in the xform's translation `x`).
    fn clip(nbones: usize) -> Clip {
        let frmval = (0..2)
            .map(|f| {
                (0..nbones)
                    .map(|b| mark(i16::try_from(f * 10 + b).unwrap()))
                    .collect()
            })
            .collect();
        Clip {
            name: "c".to_string(),
            data: ClipData::Skeletal {
                frmval,
                seq: vec![Seq { tim: 0, frm: 0 }],
            },
        }
    }

    /// The clip's column markers as a plain `i16` grid (extracts each cell's
    /// translation `x`), so the bone-management tests can assert on values.
    #[allow(clippy::cast_possible_truncation)]
    fn skeletal(clip: &Clip) -> Vec<Vec<i16>> {
        match &clip.data {
            ClipData::Skeletal { frmval, .. } => frmval
                .iter()
                .map(|row| row.iter().map(|x| x.t[0] as i16).collect())
                .collect(),
            ClipData::Unknown { .. } => panic!("expected skeletal clip"),
        }
    }

    #[test]
    fn add_bone_appends_and_grows_clip_columns() {
        let mut rig = Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones: vec![bone("body", -1, 0x80ff_0000), bone("arm", 0, 0x8000_ff00)],
            clips: vec![clip(2)],
        };
        let idx = rig.add_bone(0);
        assert_eq!(idx, 2);
        assert_eq!(rig.bones.len(), 3);
        assert_eq!(rig.bones[2].hinge.parent, 0);
        // The joint is offset to the parent's +X edge (parent mesh is 3 wide)
        // so the new bone renders beside the parent, not buried inside it.
        assert!(
            rig.bones[2].hinge.p[1].x > 0.0,
            "new bone should be offset from its parent"
        );
        // The rotation axis must be non-zero, or the limb solver collapses the
        // bone's mesh to a point (it renders as nothing in the posed preview).
        let axis = rig.bones[2].hinge.v[0];
        assert!(
            axis.x != 0.0 || axis.y != 0.0 || axis.z != 0.0,
            "new bone needs a non-zero hinge axis"
        );
        // Every clip row grew by one trailing 0 column.
        let frmval = skeletal(&rig.clips[0]);
        assert!(frmval.iter().all(|row| row.len() == 3));
        assert_eq!(frmval[0], vec![0, 1, 0]);
        assert_eq!(frmval[1], vec![10, 11, 0]);
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact offset by the integer mesh width
    fn duplicate_bone_clones_as_sibling_and_copies_clip_column() {
        let mut rig = Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones: vec![bone("body", -1, 0x80ff_0000), bone("arm", 0, 0x8000_ff00)],
            clips: vec![clip(2)],
        };
        let idx = rig.duplicate_bone(1).expect("in range");
        assert_eq!(idx, 2);
        assert_eq!(rig.bones.len(), 3);
        // Same parent as the source (a sibling), name marked as a copy.
        assert_eq!(rig.bones[2].hinge.parent, 0);
        assert_eq!(rig.bones[2].name, "arm copy");
        // Joint nudged off the original (source mesh is 3 wide) so it's visible.
        assert_eq!(rig.bones[2].hinge.p[1].x, 3.0);
        // The new clip column copies the source bone's values (identical motion).
        let frmval = skeletal(&rig.clips[0]);
        assert!(frmval.iter().all(|row| row.len() == 3));
        assert_eq!(frmval[0], vec![0, 1, 1]);
        assert_eq!(frmval[1], vec![10, 11, 11]);
        // Out-of-range is a no-op.
        assert!(rig.duplicate_bone(99).is_none());
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact offset by the integer mesh width
    fn duplicate_root_becomes_an_offset_child_not_a_second_root() {
        let mut rig = Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones: vec![bone("body", -1, 0x80ff_0000), bone("arm", 0, 0x8000_ff00)],
            clips: Vec::new(),
        };
        let idx = rig.duplicate_bone(0).expect("in range");
        assert_eq!(idx, 2);
        // The copy is a child of the original root (index 0), not a new root.
        assert_eq!(rig.bones[2].hinge.parent, 0);
        assert_eq!(rig.bones[2].name, "body copy");
        // Offset via the parent-side joint (root mesh is 3 wide), pivot at zero.
        assert_eq!(rig.bones[2].hinge.p[1].x, 3.0);
        assert_eq!(rig.bones[2].hinge.p[0].x, 0.0);
        // Still exactly one root.
        assert_eq!(rig.bones.iter().filter(|b| b.hinge.parent < 0).count(), 1);
    }

    #[test]
    fn move_bone_remaps_parents_and_reorders_clip_columns() {
        // 0:root -> 1:child -> 2:grandchild.
        let mut rig = Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones: vec![
                bone("root", -1, 1),
                bone("child", 0, 2),
                bone("grand", 1, 3),
            ],
            clips: vec![clip(3)],
        };
        // Move the grandchild (2) to the front (0): order becomes grand, root,
        // child — old indices [0,1,2] map to new [1,2,0].
        assert!(rig.move_bone(2, 0));
        assert_eq!(
            rig.bones
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            ["grand", "root", "child"]
        );
        // Parents follow the permutation: root stays -1; child's parent (old 0)
        // -> 1; grand's parent (old 1) -> 2.
        assert_eq!(rig.bones[0].hinge.parent, 2); // grand -> child (now at 2)
        assert_eq!(rig.bones[1].hinge.parent, -1); // root
        assert_eq!(rig.bones[2].hinge.parent, 1); // child -> root (now at 1)
        // Clip columns reordered the same way: old col 2 now leads.
        let frmval = skeletal(&rig.clips[0]);
        assert!(frmval.iter().all(|row| row.len() == 3));
        assert_eq!(frmval[0], vec![2, 0, 1]); // was [0,1,2]
        assert_eq!(frmval[1], vec![12, 10, 11]); // was [10,11,12]
        // Round-trips (columns stay consistent with bones.len()).
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("consistent");
        assert_eq!(back.bones.len(), 3);
        // No-ops.
        assert!(!rig.move_bone(1, 1));
        assert!(!rig.move_bone(0, 9));
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact pivot literal
    fn add_child_mesh_appends_a_child_and_grows_clip_columns() {
        let mut rig = Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones: vec![bone("body", -1, 0x80ff_0000)],
            clips: vec![clip(1)],
        };
        let mut part = VoxelModel::new(2, 2, 2);
        part.set(0, 0, 0, 0x8000_ff00);
        part.pivot = [1.5, 0.0, -2.0]; // carried verbatim (no clamp)
        let idx = rig.add_child_mesh(0, "arm".to_string(), part, [2.5, 1.0, 3.0]);
        assert_eq!(idx, 1);
        assert_eq!(rig.bones.len(), 2);
        assert_eq!(rig.bones[1].name, "arm");
        assert_eq!(rig.bones[1].hinge.parent, 0);
        // The supplied joint lands on the parent-side velcro p[1].
        assert_eq!(
            rig.bones[1].hinge.p[1],
            Point3 {
                x: 2.5,
                y: 1.0,
                z: 3.0
            }
        );
        assert_eq!(rig.bones[1].model.pivot, [1.5, 0.0, -2.0]);
        // Free range + a real axis, so the new bone is immediately poseable.
        assert_eq!(rig.bones[1].hinge.vmin, i16::MIN);
        assert_eq!(rig.bones[1].hinge.vmax, i16::MAX);
        // Every clip grew one trailing column; reload stays consistent.
        let frmval = skeletal(&rig.clips[0]);
        assert!(frmval.iter().all(|row| row.len() == 2));
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("child stays consistent");
        assert_eq!(back.bones.len(), 2);
    }

    #[test]
    fn delete_bone_reparents_children_and_remaps_parents() {
        // 0:root -> 1:child -> 2:grandchild, and 3:sibling of 1.
        let mut rig = Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones: vec![
                bone("root", -1, 1),
                bone("child", 0, 2),
                bone("grand", 1, 3),
                bone("sib", 0, 4),
            ],
            clips: vec![clip(4)],
        };
        assert!(rig.delete_bone(1));
        assert_eq!(rig.bones.len(), 3);
        // Remaining bones (indices shifted down by one): grand, then sib.
        assert_eq!(rig.bones[0].name, "root");
        assert_eq!(rig.bones[1].name, "grand");
        assert_eq!(rig.bones[2].name, "sib");
        // grand was a child of deleted bone 1 -> reparented to its parent (0).
        assert_eq!(rig.bones[1].hinge.parent, 0);
        // sib's parent (0) is below the removed index -> unchanged.
        assert_eq!(rig.bones[2].hinge.parent, 0);
        // Clip column 1 was dropped from every row.
        let frmval = skeletal(&rig.clips[0]);
        assert!(frmval.iter().all(|row| row.len() == 3));
        assert_eq!(frmval[0], vec![0, 2, 3]);
        assert_eq!(frmval[1], vec![10, 12, 13]);
    }

    #[test]
    fn delete_bone_refuses_root_and_last_bone() {
        let mut rig = Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones: vec![bone("root", -1, 1), bone("arm", 0, 2)],
            clips: Vec::new(),
        };
        assert!(!rig.delete_bone(0), "root must not be deletable");
        assert_eq!(rig.bones.len(), 2);
        assert!(rig.delete_bone(1));
        assert!(!rig.delete_bone(0), "must keep at least the root bone");
        assert_eq!(rig.bones.len(), 1);
    }

    #[test]
    fn add_then_delete_round_trips_through_character() {
        let mut rig = Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones: vec![bone("root", -1, 1)],
            clips: vec![clip(1)],
        };
        rig.add_bone(0);
        // frmval columns must match bones.len() or to/from character breaks.
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("add keeps clips consistent");
        assert_eq!(back.bones.len(), 2);
        rig.delete_bone(1);
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("delete keeps clips consistent");
        assert_eq!(back.bones.len(), 1);
    }

    #[test]
    fn rkc_bytes_round_trip() {
        let rig = Rig {
            name: "r".to_string(),
            root: [0.0; 3],
            bones: vec![bone("only", -1, 0x80ab_cdef)],
            clips: Vec::new(),
        };
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("parses");
        assert_eq!(back.bones.len(), 1);
        assert_eq!(back.bones[0].model.get(1, 1, 1), 0x80ab_cdef);
    }

    // --- Keyframe authoring -------------------------------------------------

    /// A bone with a free hinge (full i16 range) so angles aren't pinned to 0.
    fn free_bone(name: &str, parent: i32) -> RigBone {
        let mut b = bone(name, parent, 0);
        b.hinge.vmin = i16::MIN;
        b.hinge.vmax = i16::MAX;
        b.hinge.v = [Z_AXIS, Z_AXIS]; // a real axis so angle edits resolve
        b
    }

    /// The Q15 hinge angle a keyframe stores for `bone` (about +z, the axis
    /// `free_bone` uses) — the inverse of `set_keyframe_angle`.
    fn key_angle(rig: &Rig, clip: usize, k: usize, bone: usize) -> i16 {
        rig.clip_keyframes(clip)[k].xforms[bone].hinge_angle([0.0, 0.0, 1.0])
    }

    /// Column markers of a keyframe's pose (translation `x` per bone).
    #[allow(clippy::cast_possible_truncation)]
    fn xf_marks(xforms: &[BoneXform]) -> Vec<i16> {
        xforms.iter().map(|x| x.t[0] as i16).collect()
    }

    /// A demo-shaped skeletal clip: two real keys (t=0, t=1000) plus a loop
    /// marker at t=1500. Bone `b`'s angle in frame `f` is `(f*1000 + b*100)`.
    fn anim_clip(nbones: usize) -> Clip {
        let frmval = (0..2)
            .map(|f| {
                (0..nbones)
                    .map(|b| mark(i16::try_from(f * 1000 + b * 100).unwrap()))
                    .collect()
            })
            .collect();
        Clip {
            name: "a".to_string(),
            data: ClipData::Skeletal {
                frmval,
                seq: vec![
                    Seq { tim: 0, frm: 0 },
                    Seq { tim: 1000, frm: 1 },
                    Seq { tim: 1500, frm: !0 }, // loop back to entry 0
                ],
            },
        }
    }

    fn seq_of(clip: &Clip) -> &Vec<Seq> {
        match &clip.data {
            ClipData::Skeletal { seq, .. } => seq,
            ClipData::Unknown { .. } => panic!("expected skeletal clip"),
        }
    }

    fn anim_rig(nbones: usize) -> Rig {
        let bones = (0..nbones)
            .map(|i| free_bone(&format!("b{i}"), if i == 0 { -1 } else { 0 }))
            .collect();
        Rig {
            name: "t".to_string(),
            root: [0.0; 3],
            bones,
            clips: vec![anim_clip(nbones)],
        }
    }

    #[test]
    fn read_clip_exposes_sorted_keyframes_and_loop_tim() {
        let rig = anim_rig(2);
        let kfs = rig.clip_keyframes(0);
        assert_eq!(kfs.len(), 2, "two real keys, marker excluded");
        assert_eq!(kfs[0].tim, 0);
        assert_eq!(kfs[1].tim, 1000);
        assert_eq!(xf_marks(&kfs[1].xforms), vec![1000, 1100]);
        assert_eq!(rig.clip_loop_tim(0), 1500);
        // Out-of-range / non-skeletal clips read as empty.
        assert!(rig.clip_keyframes(9).is_empty());
    }

    #[test]
    fn add_keyframe_inserts_sorted_and_rebakes_marker_last() {
        let mut rig = anim_rig(2);
        let idx = rig
            .add_keyframe(0, 500, vec![mark(0), mark(42)])
            .expect("skeletal");
        assert_eq!(idx, 1, "sorts between t=0 and t=1000");
        let kfs = rig.clip_keyframes(0);
        assert_eq!(
            kfs.iter().map(|k| k.tim).collect::<Vec<_>>(),
            [0, 500, 1000]
        );
        assert_eq!(xf_marks(&kfs[1].xforms), vec![0, 42]);
        // Baked tables: frmval rows line up 1:1 with keys; the single trailing
        // seq entry is the loop marker (frm < 0) at the loop time.
        let seq = seq_of(&rig.clips[0]);
        assert_eq!(seq.len(), 4); // 3 keys + marker
        assert_eq!(seq[3].frm, !0);
        assert_eq!(seq[3].tim, 1500, "loop_tim kept (1500 > last key 1000)");
        assert_eq!(seq.iter().filter(|s| s.frm < 0).count(), 1);
        let frmval = skeletal(&rig.clips[0]);
        assert_eq!(frmval.len(), 3);
        assert!(frmval.iter().all(|r| r.len() == 2));
    }

    #[test]
    fn add_keyframe_at_existing_time_overwrites() {
        let mut rig = anim_rig(2);
        let idx = rig
            .add_keyframe(0, 1000, vec![mark(7), mark(9)])
            .expect("skeletal");
        assert_eq!(idx, 1);
        let kfs = rig.clip_keyframes(0);
        assert_eq!(kfs.len(), 2, "no duplicate timestamp");
        assert_eq!(xf_marks(&kfs[1].xforms), vec![7, 9]);
    }

    #[test]
    fn add_keyframe_past_loop_extends_the_marker() {
        let mut rig = anim_rig(2);
        rig.add_keyframe(0, 2000, vec![mark(0), mark(1)])
            .expect("skeletal");
        // The marker must stay strictly after the last key.
        assert_eq!(rig.clip_loop_tim(0), 2000 + 500);
    }

    #[test]
    fn set_keyframe_angle_clamps_to_bone_range() {
        let mut rig = anim_rig(2);
        // Pin bone 1 to a tiny symmetric range.
        rig.bones[1].hinge.vmin = -100;
        rig.bones[1].hinge.vmax = 100;
        assert!(rig.set_keyframe_angle(0, 0, 1, 5000));
        assert_eq!(key_angle(&rig, 0, 0, 1), 100, "clamped to vmax");
        assert!(rig.set_keyframe_angle(0, 0, 1, -5000));
        assert_eq!(key_angle(&rig, 0, 0, 1), -100, "clamped to vmin");
        // Out-of-range key / bone is a no-op.
        assert!(!rig.set_keyframe_angle(0, 9, 1, 0));
        assert!(!rig.set_keyframe_angle(0, 0, 9, 0));
    }

    #[test]
    fn move_keyframe_resorts_and_returns_new_index() {
        let mut rig = anim_rig(2);
        // Move the first key (t=0) past the second (t=1000) -> it becomes last.
        let idx = rig.move_keyframe(0, 0, 1200).expect("in range");
        assert_eq!(idx, 1);
        let kfs = rig.clip_keyframes(0);
        assert_eq!(kfs.iter().map(|k| k.tim).collect::<Vec<_>>(), [1000, 1200]);
        // The moved key kept its pose (frame 0 markers = [bone0=0, bone1=100]).
        assert_eq!(xf_marks(&kfs[1].xforms), vec![0, 100]);
        // Colliding with an existing key is refused.
        assert!(rig.move_keyframe(0, 0, 1200).is_none());
    }

    #[test]
    fn remove_keyframe_drops_and_keeps_at_least_one() {
        let mut rig = anim_rig(2);
        assert!(rig.remove_keyframe(0, 0));
        let kfs = rig.clip_keyframes(0);
        assert_eq!(kfs.len(), 1);
        assert_eq!(kfs[0].tim, 1000);
        // The last remaining key can't be removed.
        assert!(!rig.remove_keyframe(0, 0));
        assert_eq!(rig.clip_keyframes(0).len(), 1);
    }

    #[test]
    fn add_clip_appends_a_one_key_rest_pose_clip() {
        let mut rig = anim_rig(2);
        assert_eq!(rig.clips.len(), 1);
        let idx = rig.add_clip("walk".to_string());
        assert_eq!(idx, 1);
        assert_eq!(rig.clips.len(), 2);
        assert_eq!(rig.clips[1].name, "walk");
        // One rest-pose key (identity transforms, one column per bone), looping.
        let kfs = rig.clip_keyframes(1);
        assert_eq!(kfs.len(), 1);
        assert_eq!(kfs[0].tim, 0);
        assert_eq!(
            kfs[0].xforms,
            vec![BoneXform::IDENTITY, BoneXform::IDENTITY]
        );
        assert_eq!(rig.clip_loop_tim(1), 500);
        // Round-trips (frmval columns match bones.len()).
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("new clip is consistent");
        assert_eq!(back.clips.len(), 2);
    }

    #[test]
    fn set_clip_loops_toggles_the_trailing_marker() {
        let mut rig = anim_rig(2); // demo clip loops by default
        assert!(rig.clip_loops(0));
        // Turning the loop off swaps the marker to a self-jump; keys + length
        // are preserved.
        let before = rig.clip_keyframes(0);
        let len = rig.clip_loop_tim(0);
        assert!(rig.set_clip_loops(0, false));
        assert!(!rig.clip_loops(0));
        assert_eq!(rig.clip_keyframes(0), before);
        assert_eq!(rig.clip_loop_tim(0), len);
        // The trailing seq marker is now a self-jump (`!own_index`), not `!0`.
        if let ClipData::Skeletal { seq, .. } = &rig.clips[0].data {
            let last = seq.last().unwrap();
            let last_idx = i32::try_from(seq.len() - 1).unwrap();
            assert_eq!(last.frm, !last_idx);
        } else {
            panic!("skeletal");
        }
        // And back on.
        assert!(rig.set_clip_loops(0, true));
        assert!(rig.clip_loops(0));
        // Survives a round-trip.
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("consistent");
        assert!(back.clip_loops(0));
    }

    #[test]
    fn set_clip_length_moves_the_marker_and_clamps_past_the_last_key() {
        let mut rig = anim_rig(2); // keys at 0 and 1000
        assert!(rig.set_clip_length(0, 3000));
        assert_eq!(rig.clip_loop_tim(0), 3000);
        assert_eq!(rig.clip_keyframes(0).len(), 2, "keys untouched");
        // A length at/before the last key is clamped strictly past it.
        assert!(rig.set_clip_length(0, 500));
        assert!(
            rig.clip_loop_tim(0) > 1000,
            "marker stays after the last key"
        );
    }

    #[test]
    fn rename_and_remove_clip() {
        let mut rig = anim_rig(2);
        rig.add_clip("b".to_string()); // clips: ["a", "b"]
        assert!(rig.rename_clip(1, "renamed".to_string()));
        assert_eq!(rig.clips[1].name, "renamed");
        assert!(!rig.rename_clip(9, "x".to_string()));
        // Remove down to zero; out-of-range is a no-op.
        assert!(rig.remove_clip(0));
        assert_eq!(rig.clips.len(), 1);
        assert_eq!(rig.clips[0].name, "renamed");
        assert!(rig.remove_clip(0));
        assert!(rig.clips.is_empty());
        assert!(!rig.remove_clip(0));
    }

    #[test]
    fn add_axis_joint_builds_an_xyz_rotator_chain() {
        let mut rig = anim_rig(2); // bones: 0 root, 1 child
        let leaf = rig.add_axis_joint(1);
        assert_eq!(leaf, 4, "leaf is the last of the 3 added bones");
        assert_eq!(rig.bones.len(), 5);
        // Chain: rotX(2) -> rotY(3) -> leaf(4), under bone 1.
        assert_eq!(rig.bones[2].hinge.parent, 1);
        assert_eq!(rig.bones[3].hinge.parent, 2);
        assert_eq!(rig.bones[4].hinge.parent, 3);
        // One principal axis each.
        assert_eq!(rig.bones[2].hinge.v[0], X_AXIS);
        assert_eq!(rig.bones[3].hinge.v[0], Y_AXIS);
        assert_eq!(rig.bones[4].hinge.v[0], Z_AXIS);
        // Rotators are empty (invisible) and free-range (animatable); the leaf
        // is visible.
        assert!(rig.bones[2].model.used_colors().is_empty());
        assert!(rig.bones[3].model.used_colors().is_empty());
        assert!(!rig.bones[4].model.used_colors().is_empty());
        assert_eq!(rig.bones[2].hinge.vmin, i16::MIN);
        assert_eq!(rig.bones[2].hinge.vmax, i16::MAX);
        // Every clip grew by three columns; still consistent on reload.
        let frmval = skeletal(&rig.clips[0]);
        assert!(frmval.iter().all(|row| row.len() == 5));
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("joint stays consistent");
        assert_eq!(back.bones.len(), 5);
    }

    #[test]
    fn add_dummy_root_makes_the_old_root_animatable() {
        let mut rig = anim_rig(2); // bone 0 is the root
        let dummy = rig.add_dummy_root().expect("has a root");
        assert_eq!(dummy, 2);
        assert_eq!(rig.bones.len(), 3);
        // Exactly one root, and it's the new dummy.
        assert_eq!(rig.bones.iter().filter(|b| b.hinge.parent < 0).count(), 1);
        assert_eq!(rig.bones[2].hinge.parent, -1);
        assert!(rig.bones[2].model.used_colors().is_empty());
        // The old root is now a free-range child of the dummy.
        assert_eq!(rig.bones[0].hinge.parent, 2);
        assert_eq!(rig.bones[0].hinge.vmin, i16::MIN);
        // Clip columns grew by one; reload stays consistent.
        let frmval = skeletal(&rig.clips[0]);
        assert!(frmval.iter().all(|row| row.len() == 3));
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("dummy root is consistent");
        assert_eq!(back.bones.len(), 3);
    }

    #[test]
    #[allow(clippy::float_cmp)] // the test literals are exact in f32
    fn set_keyframe_translation_and_scale_keep_other_components() {
        let mut rig = anim_rig(2);
        // Rotate bone 1, then set its translation + scale — each keeps the rest.
        assert!(rig.set_keyframe_angle(0, 0, 1, 4000));
        assert!(rig.set_keyframe_translation(0, 0, 1, [1.0, 2.0, 3.0]));
        assert!(rig.set_keyframe_scale(0, 0, 1, [2.0, 1.0, 0.5]));
        let x = rig.clip_keyframes(0)[0].xforms[1];
        assert_eq!(x.t, [1.0, 2.0, 3.0]);
        assert_eq!(x.s, [2.0, 1.0, 0.5]);
        assert_eq!(
            key_angle(&rig, 0, 0, 1),
            4000,
            "rotation survived t/s edits"
        );
        // Out-of-range is a no-op.
        assert!(!rig.set_keyframe_translation(0, 9, 1, [0.0; 3]));
        assert!(!rig.set_keyframe_scale(0, 0, 9, [1.0; 3]));
    }

    #[test]
    fn single_bone_builds_a_minimal_animatable_rig() {
        let rig = Rig::single_bone("hero", None);
        assert_eq!(rig.bones.len(), 1);
        assert_eq!(rig.bones[0].hinge.parent, -1, "the lone bone is the root");
        assert!(rig.clips.is_empty(), "no clips until one is added");
        // It round-trips through .rkc, and a clip can be added on top.
        let mut rig = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("consistent");
        rig.add_clip("idle".to_string());
        assert_eq!(rig.clips.len(), 1);
    }

    #[test]
    fn keyframe_edits_round_trip_through_rkc() {
        let mut rig = anim_rig(2);
        rig.add_keyframe(0, 500, vec![mark(0), mark(42)]);
        rig.set_keyframe_angle(0, 1, 1, 1234);
        rig.move_keyframe(0, 2, 1800);
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("edits stay consistent");
        assert_eq!(back.bones.len(), 2);
        // The clip survives the round-trip with the same keyframes.
        assert_eq!(back.clip_keyframes(0), rig.clip_keyframes(0));
    }
}
