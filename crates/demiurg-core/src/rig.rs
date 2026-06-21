//! Editable rigged-character document: a skeleton of bones, each carrying
//! an **editable** voxel mesh, plus named animation clips. Converts to and
//! from the engine's on-disk [`Character`] container (the `.rkc` format),
//! which stores each bone's mesh as a compiled `KV6`.
//!
//! The editor edits a [`Rig`] (one [`VoxelModel`] per bone, with the
//! existing tools); rendering and saving go through [`Rig::to_character`].

use roxlap_formats::character::{self, Bone as CharBone, Character, Clip, ClipData, MeshRef};
use roxlap_formats::kfa::{Hinge, Point3};

use crate::VoxelModel;

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

/// One bone: a name, its editable voxel mesh, and its hinge. `hinge.parent`
/// indexes [`Rig::bones`] (`-1` = root).
#[derive(Debug, Clone)]
pub struct RigBone {
    pub name: String,
    pub model: VoxelModel,
    pub hinge: Hinge,
}

impl Rig {
    /// Compile to an engine [`Character`]: each bone's mesh becomes a `KV6`
    /// (one mesh per bone, referenced `MeshRef::Static(i)`).
    #[must_use]
    pub fn to_character(&self) -> Character {
        let meshes = self.bones.iter().map(|b| b.model.to_kv6()).collect();
        let bones = self
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| CharBone {
                name: b.name.clone(),
                mesh: MeshRef::Static(i),
                hinge: b.hinge,
            })
            .collect();
        Character {
            name: self.name.clone(),
            root: self.root,
            meshes,
            bones,
            clips: self.clips.clone(),
            extra_chunks: Vec::new(),
        }
    }

    /// Build from an engine [`Character`], decompiling each bone's `KV6` to
    /// an editable [`VoxelModel`].
    ///
    /// # Errors
    /// A message if a bone's static mesh index is out of range.
    pub fn from_character(c: &Character) -> Result<Self, String> {
        let bones = c
            .bones
            .iter()
            .map(|b| {
                let MeshRef::Static(i) = b.mesh;
                let kv6 = c
                    .meshes
                    .get(i)
                    .ok_or_else(|| format!("bone {:?}: mesh index {i} out of range", b.name))?;
                Ok(RigBone {
                    name: b.name.clone(),
                    model: VoxelModel::from_kv6(kv6),
                    hinge: b.hinge,
                })
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
        self.bones.push(RigBone {
            name: format!("bone {n}"),
            model: default_bone_model(),
            hinge: default_hinge(parent, joint),
        });
        for clip in &mut self.clips {
            if let ClipData::Skeletal { frmval, .. } = &mut clip.data {
                for row in frmval {
                    row.push(0);
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
        self.bones.push(RigBone { name, model, hinge });
        for clip in &mut self.clips {
            if let ClipData::Skeletal { frmval, .. } = &mut clip.data {
                for row in frmval {
                    let v = row.get(i).copied().unwrap_or(0);
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
/// (the child mesh sits centred there), no rotation range.
fn default_hinge(parent: i32, joint: Point3) -> Hinge {
    Hinge {
        parent,
        // p[0] = child-side attach (its own pivot); p[1] = parent-side joint.
        p: [ZERO, joint],
        // A valid (non-zero) rotation axis — see Z_AXIS.
        v: [Z_AXIS, Z_AXIS],
        vmin: 0,
        vmax: 0,
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
        RigBone {
            name: name.to_string(),
            model,
            hinge: Hinge {
                parent,
                p: [zero, zero],
                v: [zero, zero],
                vmin: 0,
                vmax: 0,
                htype: 0,
                filler: [0; 7],
            },
        }
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

    use roxlap_formats::character::{Clip, ClipData};
    use roxlap_formats::kfa::Seq;

    /// A 2-frame skeletal clip with `nbones` columns, each cell set to a
    /// recognisable `frame*10 + bone` marker.
    fn clip(nbones: usize) -> Clip {
        let frmval = (0..2)
            .map(|f| {
                (0..nbones)
                    .map(|b| i16::try_from(f * 10 + b).unwrap())
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

    fn skeletal(clip: &Clip) -> &Vec<Vec<i16>> {
        match &clip.data {
            ClipData::Skeletal { frmval, .. } => frmval,
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
}
