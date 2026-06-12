//! The `.demiurg` project format: a **lossless** snapshot of a
//! [`VoxelModel`] for work-in-progress saves.
//!
//! Unlike `.kv6` (surface-only, the engine export), a project keeps the
//! full dense volume — including enclosed interior voxels — plus the
//! pivot and palette, so an edit session round-trips exactly. The wire
//! format is [`postcard`] (compact, no_std-friendly); the schema is the
//! [`Project`] struct, versioned by its field layout.

use serde::{Deserialize, Serialize};

use crate::VoxelModel;
use roxlap_formats::Rgb6;

/// Serializable form of a [`VoxelModel`]. Palette entries are stored as
/// raw `[r, g, b]` triplets (6-bit channels, as on disk) so the schema
/// does not depend on roxlap's `Rgb6` deriving serde.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub dims: [u32; 3],
    pub pivot: [f32; 3],
    /// 256 `[r, g, b]` entries when present.
    pub palette: Option<Vec<[u8; 3]>>,
    /// Dense voxel buffer, `x + xsiz·(y + ysiz·z)` order, `0` = empty.
    pub voxels: Vec<u32>,
}

impl Project {
    /// Snapshot a model.
    #[must_use]
    pub fn from_model(model: &VoxelModel) -> Self {
        let (x, y, z) = model.dims();
        let palette = model
            .palette
            .as_ref()
            .map(|p| p.iter().map(|c| [c.r, c.g, c.b]).collect());
        Self {
            dims: [x, y, z],
            pivot: model.pivot,
            palette,
            voxels: model.voxels().to_vec(),
        }
    }

    /// Rebuild a model. Returns `None` if `voxels` length disagrees with
    /// `dims` (a corrupt or truncated project).
    #[must_use]
    pub fn into_model(self) -> Option<VoxelModel> {
        let palette = self.palette.map(|entries| {
            let mut arr = [Rgb6::default(); 256];
            for (slot, c) in arr.iter_mut().zip(entries.iter()) {
                *slot = Rgb6 {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                };
            }
            arr
        });
        VoxelModel::from_parts(
            self.dims[0],
            self.dims[1],
            self.dims[2],
            self.pivot,
            palette,
            self.voxels,
        )
    }
}

/// Serialize a model to `.demiurg` bytes.
///
/// # Panics
/// Never in practice: postcard serialization of plain data into a
/// growable `Vec` has no failure path.
#[must_use]
pub fn to_bytes(model: &VoxelModel) -> Vec<u8> {
    // Plain-data serialization into a growable Vec cannot fail.
    postcard::to_allocvec(&Project::from_model(model)).expect("postcard serialize")
}

/// Parse `.demiurg` bytes back into a model.
///
/// # Errors
/// [`LoadError::Decode`] if the bytes are not a valid project,
/// [`LoadError::DimsMismatch`] if the voxel buffer length disagrees with
/// the stored dimensions.
pub fn from_bytes(bytes: &[u8]) -> Result<VoxelModel, LoadError> {
    let project: Project = postcard::from_bytes(bytes).map_err(LoadError::Decode)?;
    project.into_model().ok_or(LoadError::DimsMismatch)
}

/// Failure modes of [`from_bytes`].
#[derive(Debug)]
pub enum LoadError {
    Decode(postcard::Error),
    DimsMismatch,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "not a valid .demiurg project: {e}"),
            Self::DimsMismatch => write!(f, "project voxel buffer does not match its dimensions"),
        }
    }
}

impl std::error::Error for LoadError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::float_cmp)] // pivot literals are exact in f32 and round-trip losslessly
    fn round_trip_preserves_everything_including_interior() {
        // Solid 3³ cube: the centre voxel is enclosed — `.kv6` would drop
        // it, but the project must keep it.
        let mut m = VoxelModel::new(3, 3, 3);
        for z in 0..3 {
            for y in 0..3 {
                for x in 0..3 {
                    m.set(x, y, z, 0x8080_8080);
                }
            }
        }
        m.pivot = [1.25, 0.5, 2.75];
        let mut pal = [Rgb6::default(); 256];
        pal[42] = Rgb6 { r: 1, g: 2, b: 3 };
        m.palette = Some(pal);
        assert_eq!(m.occupied_count(), 27);

        let back = from_bytes(&to_bytes(&m)).expect("round-trips");
        assert_eq!(back.occupied_count(), 27, "interior voxel survives");
        assert_eq!(back.get(1, 1, 1), 0x8080_8080);
        assert_eq!(back.dims(), (3, 3, 3));
        assert_eq!(back.pivot, m.pivot);
        assert_eq!(back.palette, m.palette);
    }

    #[test]
    fn rejects_truncated_buffer() {
        let project = Project {
            dims: [2, 2, 2],
            pivot: [1.0, 1.0, 1.0],
            palette: None,
            voxels: vec![0; 3], // should be 8
        };
        let bytes = postcard::to_allocvec(&project).unwrap();
        assert!(matches!(from_bytes(&bytes), Err(LoadError::DimsMismatch)));
    }
}
