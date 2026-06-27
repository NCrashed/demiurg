//! The `.demiurg` project format: a **lossless** work-in-progress snapshot of
//! the editor's document — either a bare [`VoxelModel`] or a full rigged,
//! animated [`Rig`].
//!
//! Unlike `.kv6` (surface-only, the engine export), a project keeps the full
//! dense volume — including enclosed interior voxels — plus the pivot and
//! palette, so an edit session round-trips exactly. A rigged document is stored
//! as its `.rkc` bytes (the engine container, which carries the skeleton, every
//! bone's mesh, and the animation clips) so the project never loses the rig —
//! and stays lossless as `.rkc` gains richer animation. The wire format is
//! [`postcard`] (compact, no_std-friendly); the schema is the [`Doc`] enum,
//! versioned by its layout.

use serde::{Deserialize, Serialize};

use crate::VoxelModel;
use crate::clip::{self, ClipDoc, ClipFrame};
use crate::rig::Rig;
use roxlap_formats::Rgb6;

/// On-disk top-level document: a bare model, a rigged character (its `.rkc`
/// bytes), or an animated voxel clip. Postcard tags the variant, so the loader
/// knows which it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
enum Doc {
    // Boxed: a `Project` (dense voxel buffer + palette) dwarfs the rig's byte
    // vec, and an unboxed large variant bloats every `Doc`.
    Model(Box<Project>),
    /// A rigged, animated character as serialized `.rkc` bytes.
    Rig(Vec<u8>),
    /// An animated voxel clip — a sequence of dense frames (boxed: a clip's
    /// many dense buffers dwarf the other variants).
    Clip(Box<ClipProject>),
}

/// What a `.demiurg` file decoded to — a model, a rig, or a clip.
// A `VoxelModel` (inline 256-entry palette) dwarfs a `Rig`, but `Loaded` is a
// one-shot return value the caller immediately unwraps — boxing would only add
// indirection at every match site, not save any lasting memory.
#[allow(clippy::large_enum_variant)]
pub enum Loaded {
    Model(VoxelModel),
    Rig(Rig),
    Clip(ClipDoc),
}

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

/// Serializable form of a [`ClipDoc`]: clip-level metadata plus the dense
/// frames. Like [`Project`], frames store raw `[r, g, b]` palette triplets and
/// the full dense voxel buffer, so the WIP round-trips losslessly (interior
/// voxels the `.rvc` export would drop are kept). `dims`/`pivot` are
/// clip-level (every frame shares them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipProject {
    pub name: String,
    pub dims: [u32; 3],
    pub pivot: [f32; 3],
    pub voxel_world_size: f32,
    /// [`crate::clip::loop_mode_to_u8`] tag.
    pub loop_mode: u8,
    pub default_frame_ms: u32,
    pub frames: Vec<ClipFrameProject>,
}

/// One dense frame of a [`ClipProject`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipFrameProject {
    /// 256 `[r, g, b]` entries when present.
    pub palette: Option<Vec<[u8; 3]>>,
    /// Dense voxel buffer, `x + xsiz·(y + ysiz·z)` order, `0` = empty.
    pub voxels: Vec<u32>,
    /// On-screen ms; `None` ⇒ the clip's `default_frame_ms`.
    pub duration_ms: Option<u32>,
}

impl ClipProject {
    /// Snapshot a clip document.
    #[must_use]
    pub fn from_clip(c: &ClipDoc) -> Self {
        let frames = c
            .frames
            .iter()
            .map(|f| ClipFrameProject {
                palette: f
                    .model
                    .palette
                    .as_ref()
                    .map(|p| p.iter().map(|c| [c.r, c.g, c.b]).collect()),
                voxels: f.model.voxels().to_vec(),
                duration_ms: f.duration_ms,
            })
            .collect();
        Self {
            name: c.name.clone(),
            dims: c.dims,
            pivot: c.pivot,
            voxel_world_size: c.voxel_world_size,
            loop_mode: clip::loop_mode_to_u8(c.loop_mode),
            default_frame_ms: c.default_frame_ms,
            frames,
        }
    }

    /// Rebuild a clip document. Returns `None` if any frame's buffer length
    /// disagrees with `dims`, or the clip has no frames (corrupt project).
    #[must_use]
    pub fn into_clip(self) -> Option<ClipDoc> {
        if self.frames.is_empty() {
            return None;
        }
        let mut frames = Vec::with_capacity(self.frames.len());
        for f in self.frames {
            let palette = f.palette.map(|entries| {
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
            let model = VoxelModel::from_parts(
                self.dims[0],
                self.dims[1],
                self.dims[2],
                self.pivot,
                palette,
                f.voxels,
            )?;
            frames.push(ClipFrame {
                model,
                duration_ms: f.duration_ms,
            });
        }
        Some(ClipDoc {
            name: self.name,
            dims: self.dims,
            pivot: self.pivot,
            voxel_world_size: self.voxel_world_size,
            loop_mode: clip::loop_mode_from_u8(self.loop_mode),
            default_frame_ms: self.default_frame_ms,
            frames,
        })
    }
}

/// Serialize a bare model to `.demiurg` bytes.
///
/// # Panics
/// Never in practice: postcard serialization of plain data into a
/// growable `Vec` has no failure path.
#[must_use]
pub fn to_bytes(model: &VoxelModel) -> Vec<u8> {
    // Plain-data serialization into a growable Vec cannot fail.
    postcard::to_allocvec(&Doc::Model(Box::new(Project::from_model(model))))
        .expect("postcard serialize")
}

/// Serialize a rigged character to `.demiurg` bytes (its `.rkc` encoding).
///
/// # Panics
/// Never in practice: postcard serialization into a growable `Vec` has no
/// failure path.
#[must_use]
pub fn to_bytes_rig(rig: &Rig) -> Vec<u8> {
    postcard::to_allocvec(&Doc::Rig(rig.to_rkc_bytes())).expect("postcard serialize")
}

/// Serialize an animated voxel clip to `.demiurg` bytes (lossless dense frames).
///
/// # Panics
/// Never in practice: postcard serialization into a growable `Vec` has no
/// failure path.
#[must_use]
pub fn to_bytes_clip(clip: &ClipDoc) -> Vec<u8> {
    postcard::to_allocvec(&Doc::Clip(Box::new(ClipProject::from_clip(clip))))
        .expect("postcard serialize")
}

/// Parse `.demiurg` bytes into a model or a rig.
///
/// # Errors
/// [`LoadError::Decode`] if the bytes are not a valid project,
/// [`LoadError::DimsMismatch`] if a model's voxel buffer length disagrees with
/// its stored dimensions, [`LoadError::Rig`] if an embedded rig fails to parse.
pub fn from_bytes(bytes: &[u8]) -> Result<Loaded, LoadError> {
    match postcard::from_bytes::<Doc>(bytes).map_err(LoadError::Decode)? {
        Doc::Model(p) => (*p)
            .into_model()
            .map(Loaded::Model)
            .ok_or(LoadError::DimsMismatch),
        Doc::Rig(rkc) => Rig::from_rkc_bytes(&rkc)
            .map(Loaded::Rig)
            .map_err(LoadError::Rig),
        Doc::Clip(c) => (*c)
            .into_clip()
            .map(Loaded::Clip)
            .ok_or(LoadError::DimsMismatch),
    }
}

/// Parse `.demiurg` bytes that are expected to be a bare model (legacy / CLI /
/// autosave paths that don't handle rigs). A rig project is an error.
///
/// # Errors
/// As [`from_bytes`], plus [`LoadError::ExpectedModel`] if the file holds a rig.
pub fn from_bytes_model(bytes: &[u8]) -> Result<VoxelModel, LoadError> {
    match from_bytes(bytes)? {
        Loaded::Model(m) => Ok(m),
        Loaded::Rig(_) | Loaded::Clip(_) => Err(LoadError::ExpectedModel),
    }
}

/// Failure modes of [`from_bytes`].
#[derive(Debug)]
pub enum LoadError {
    Decode(postcard::Error),
    DimsMismatch,
    /// An embedded rig (`.rkc` bytes) failed to parse.
    Rig(String),
    /// A model was expected but the file holds a rig (see [`from_bytes_model`]).
    ExpectedModel,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "not a valid .demiurg project: {e}"),
            Self::DimsMismatch => write!(f, "project voxel buffer does not match its dimensions"),
            Self::Rig(e) => write!(f, "project's embedded rig is invalid: {e}"),
            Self::ExpectedModel => write!(f, "this .demiurg holds a rig, not a bare model"),
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

        let back = from_bytes_model(&to_bytes(&m)).expect("round-trips");
        assert_eq!(back.occupied_count(), 27, "interior voxel survives");
        assert_eq!(back.get(1, 1, 1), 0x8080_8080);
        assert_eq!(back.dims(), (3, 3, 3));
        assert_eq!(back.pivot, m.pivot);
        assert_eq!(back.palette, m.palette);
    }

    #[test]
    #[allow(clippy::float_cmp)] // pivot/size literals are exact in f32
    fn clip_round_trips_losslessly_including_interior() {
        use crate::clip::{ClipDoc, LoopMode};

        // A solid 3³ first frame: its enclosed centre voxel would vanish
        // through `.rvc` (surface-only) but must survive the `.demiurg` WIP.
        let mut clip = ClipDoc::new([3, 3, 3]);
        for z in 0..3 {
            for y in 0..3 {
                for x in 0..3 {
                    clip.frames[0].model.set(x, y, z, 0x8080_8080);
                }
            }
        }
        let f1 = clip.add_frame();
        clip.frames[f1].model.set(0, 0, 0, 0x80ff_0000);
        clip.frames[f1].duration_ms = Some(33);
        clip.loop_mode = LoopMode::PingPong;
        clip.default_frame_ms = 50;
        clip.name = "spark".into();

        let Loaded::Clip(back) = from_bytes(&to_bytes_clip(&clip)).expect("round-trips") else {
            panic!("expected a clip");
        };
        assert_eq!(back.name, "spark");
        assert_eq!(back.dims, [3, 3, 3]);
        assert_eq!(back.frame_count(), 2);
        assert_eq!(back.loop_mode, LoopMode::PingPong);
        assert_eq!(back.durations(), vec![50, 33]);
        assert_eq!(
            back.frames[0].model.occupied_count(),
            27,
            "interior voxel survives the project round-trip"
        );
        assert_eq!(back.frames[0].model.get(1, 1, 1), 0x8080_8080);
    }

    #[test]
    fn rejects_truncated_buffer() {
        let doc = Doc::Model(Box::new(Project {
            dims: [2, 2, 2],
            pivot: [1.0, 1.0, 1.0],
            palette: None,
            voxels: vec![0; 3], // should be 8
        }));
        let bytes = postcard::to_allocvec(&doc).unwrap();
        assert!(matches!(from_bytes(&bytes), Err(LoadError::DimsMismatch)));
    }
}
