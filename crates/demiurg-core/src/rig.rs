//! Editable rigged-character document: a skeleton of bones, each carrying
//! an **editable** voxel mesh, plus named animation clips. Converts to and
//! from the engine's on-disk [`Character`] container (the `.rkc` format),
//! which stores each bone's mesh as a compiled `KV6`.
//!
//! The editor edits a [`Rig`] (one [`VoxelModel`] per bone, with the
//! existing tools); rendering and saving go through [`Rig::to_character`].

use roxlap_formats::character::{self, Bone as CharBone, Character, Clip, MeshRef};
use roxlap_formats::kfa::Hinge;

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
