//! `demiurg-convert`: turn a JSON exchange manifest into a `.demiurg` project
//! or a `.rkc` character.
//!
//! This is the bridge a DCC exporter shells out to — the same shape Voxelity
//! Pro uses for `vengi-voxconvert`. The alternative, re-implementing the wire
//! formats in Python, means duplicating both postcard and roxlap's `.rkc`
//! container in a second language and keeping them in step forever; here the
//! Rust workspace stays the single source of truth and the addon only has to
//! write JSON.
//!
//! ```no_run
//! use std::path::Path;
//! use demiurg_convert::{convert, Output};
//!
//! let json = std::fs::read("hero.json")?;
//! let out = convert(&json, Path::new("."), Output::Demiurg)?;
//! std::fs::write("hero.demiurg", &out.bytes)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! See [`manifest`] for the schema and [`build`] for how it maps onto the
//! engine's rig model (and which of that model's traps the converter guards).

pub mod build;
pub mod manifest;

use std::fmt;
use std::path::Path;

use demiurg_core::project;

pub use build::ConvertError;
pub use manifest::Manifest;

/// Which document to encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// The editor's `.demiurg` project — lossless, re-openable, what an artist
    /// continues working on.
    Demiurg,
    /// The engine's `.rkc` character — rigs only.
    Rkc,
}

impl Output {
    /// Pick the output kind from a file name's extension. `None` for anything
    /// but `.demiurg` / `.rkc`.
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        match path
            .extension()
            .and_then(|e| e.to_str())?
            .to_ascii_lowercase()
            .as_str()
        {
            "demiurg" => Some(Self::Demiurg),
            "rkc" => Some(Self::Rkc),
            _ => None,
        }
    }
}

/// A converted document plus what went into it.
#[derive(Debug, Clone)]
pub struct Converted {
    /// The encoded file.
    pub bytes: Vec<u8>,
    /// Counts worth echoing back to the exporter's log.
    pub stats: Stats,
}

/// What the manifest actually produced — an exporter that silently drops half a
/// rig should be caught by reading one line of output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Bones in the rig (`0` for a bare model).
    pub bones: usize,
    /// Animation clips.
    pub clips: usize,
    /// Keyframes across every clip (excluding the trailing loop markers).
    pub keys: usize,
    /// Occupied voxels across every mesh **and** every voxel-clip frame.
    pub voxels: usize,
    /// Frames across every bone's voxel clip — the multiplier that makes a
    /// deforming bone cost what it does, so it is worth seeing.
    pub clip_frames: usize,
    /// Extra attachments across every bone.
    pub layers: usize,
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} bones, {} clips, {} keys, {} voxels",
            self.bones, self.clips, self.keys, self.voxels
        )?;
        if self.clip_frames > 0 {
            write!(f, " in {} voxel-clip frames", self.clip_frames)?;
        }
        if self.layers > 0 {
            write!(f, ", {} layers", self.layers)?;
        }
        Ok(())
    }
}

/// Why a conversion failed.
#[derive(Debug)]
pub enum Error {
    /// The manifest is not valid JSON, or doesn't match the schema.
    Json(serde_json::Error),
    /// The manifest is well-formed but doesn't describe a valid document.
    Convert(ConvertError),
    /// A bare model was asked to be written as `.rkc`, which only holds rigs.
    ModelToRkc,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid manifest: {e}"),
            Self::Convert(e) => write!(f, "{e}"),
            Self::ModelToRkc => write!(
                f,
                ".rkc holds a rigged character; write a bare model as .demiurg \
                 (or wrap it in a one-bone rig)"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            Self::Convert(e) => Some(e),
            Self::ModelToRkc => None,
        }
    }
}

/// Parse a manifest and encode it as `out`. `base_dir` is the manifest's own
/// directory — `vox_file` references resolve against it.
///
/// # Errors
/// [`Error`] if the JSON is malformed, the document is invalid (the message
/// names the bone or clip at fault), or a bare model was aimed at `.rkc`.
pub fn convert(json: &[u8], base_dir: &Path, out: Output) -> Result<Converted, Error> {
    let m = Manifest::from_json(json).map_err(Error::Json)?;
    if m.format == manifest::FORMAT_MODEL {
        if out == Output::Rkc {
            return Err(Error::ModelToRkc);
        }
        let model = build::build_model(&m, base_dir).map_err(Error::Convert)?;
        let stats = Stats {
            voxels: model.occupied_count(),
            ..Stats::default()
        };
        return Ok(Converted {
            bytes: project::to_bytes(&model),
            stats,
        });
    }
    let rig = build::build_rig(&m, base_dir).map_err(Error::Convert)?;
    let stats = Stats {
        bones: rig.bones.len(),
        clips: rig.clips.len(),
        keys: (0..rig.clips.len())
            .map(|i| rig.clip_keyframes(i).len())
            .sum(),
        // A clip bone's `model` is an unused placeholder, so its voxels live
        // in the frames — count those instead, or a deforming character would
        // report as empty.
        // Extras count too — a bone's layers are geometry the file carries,
        // and a summary that ignored them would report a sword-wielding
        // character as unchanged by the sword.
        voxels: rig
            .bones
            .iter()
            .map(|b| {
                let primary: usize = match &b.primary_clip {
                    Some(c) => c.frames.iter().map(|f| f.model.occupied_count()).sum(),
                    None => b.model.occupied_count(),
                };
                let layers: usize = b
                    .extras
                    .iter()
                    .map(|e| match &e.clip {
                        Some(c) => c.frames.iter().map(|f| f.model.occupied_count()).sum(),
                        None => e.model.occupied_count(),
                    })
                    .sum();
                primary + layers
            })
            .sum(),
        layers: rig.bones.iter().map(|b| b.extras.len()).sum(),
        clip_frames: rig
            .bones
            .iter()
            .map(|b| {
                let primary = b.primary_clip.as_ref().map_or(0, |c| c.frames.len());
                let layers: usize = b
                    .extras
                    .iter()
                    .filter_map(|e| e.clip.as_ref())
                    .map(|c| c.frames.len())
                    .sum();
                primary + layers
            })
            .sum(),
    };
    let bytes = match out {
        Output::Demiurg => project::to_bytes_rig(&rig),
        Output::Rkc => rig.to_rkc_bytes(),
    };
    Ok(Converted { bytes, stats })
}
