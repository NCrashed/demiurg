//! The animated **voxel-clip** document — demiurg's "GIF/MP4 for voxels".
//!
//! A [`ClipDoc`] is a fixed-bounding-box sequence of editable voxel frames
//! with per-frame timing and a [`LoopMode`]. It is the editor-side, *dense*
//! analogue of roxlap's on-disk [`VoxelClip`](roxlap_formats::voxel_clip)
//! (`.rvc`, magic `RVCL`): every frame is a plain [`VoxelModel`] the existing
//! sculpt tools edit, and the clip *compiles* to a `.rvc` exactly the way a
//! [`VoxelModel`] compiles to `.kv6` — through roxlap's own encoder, so what
//! you author is byte-for-byte what the engine plays.
//!
//! ```text
//! edit frame i (VoxelModel) ──to_kv6──┐
//!                                      ├─ VoxelClip::from_kv6_frames_auto ─ .rvc
//! edit frame i+1 …          ──to_kv6──┘
//! ```
//!
//! Like `.kv6`, `.rvc` stores only **surface** voxels per frame, so an
//! enclosed interior voxel does not survive a `to_rvc` → `from_rvc` round-trip
//! (a property of the format). The lossless editor source is the `.demiurg`
//! project, which stores each frame's dense buffer verbatim (see
//! [`crate::project`]).
//!
//! **Frame source (forward-compat).** Today every frame is hand-sculpted: its
//! [`ClipFrame::model`] is edited directly. The encode / preview / timeline
//! paths only ever read `frame.model`, so a future procedural generator (an
//! embedded Rhai script producing flame / smoke / energy frames) slots in as a
//! source that *fills* `model` before encoding — nothing downstream changes.
//! That generator is a separate feature; this module deliberately keeps the
//! frame the single source of truth so it can be added without disruption.

use roxlap_formats::voxel_clip::{self, DecodeError, ParseError, VoxelClip};

use crate::VoxelModel;

pub use roxlap_formats::voxel_clip::LoopMode;

/// Default frame duration for a fresh clip (~12.5 fps — a comfortable default
/// for hand-animated effects; the artist tunes it).
pub const DEFAULT_FRAME_MS: u32 = 80;

/// Default world size of one voxel for a fresh clip (matches roxlap's sprite
/// default; carried through to the `.rvc`).
pub const DEFAULT_VOXEL_WORLD_SIZE: f32 = 1.0;

/// Keyframe-spacing cap handed to the auto encoder. `0` = fully cost-driven
/// (only frame 0 and "scene-change" frames become keyframes — smallest file).
/// The editor always [`decode`](VoxelClip::decode)s the whole clip up front, so
/// in-file seek points are not needed for preview; we favour size.
const DEFAULT_MAX_KEYFRAME_GAP: u32 = 0;

/// One frame of a [`ClipDoc`]: a dense, editable voxel model plus its on-screen
/// duration.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipFrame {
    /// The frame's voxels. Its dims always equal the owning [`ClipDoc::dims`]
    /// (clips are fixed-bbox); the sculpt tools edit this in place.
    pub model: VoxelModel,
    /// On-screen time of this frame in ms. `None` ⇒ the clip's
    /// [`ClipDoc::default_frame_ms`].
    pub duration_ms: Option<u32>,
}

impl ClipFrame {
    /// A frame wrapping `model`, using the clip default duration.
    #[must_use]
    pub fn new(model: VoxelModel) -> Self {
        Self {
            model,
            duration_ms: None,
        }
    }
}

/// An animated voxel clip under edit: a fixed-bbox sequence of frames. Always
/// holds **at least one** frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipDoc {
    pub name: String,
    /// Shared bounding box of every frame `(xsiz, ysiz, zsiz)`.
    pub dims: [u32; 3],
    /// Pivot in voxel units, shared by every frame (the engine rotates the
    /// clip about this point).
    pub pivot: [f32; 3],
    /// World size of one voxel, carried to the `.rvc`.
    pub voxel_world_size: f32,
    /// How playback advances past the last frame.
    pub loop_mode: LoopMode,
    /// Frame duration (ms) used when a frame's own `duration_ms` is `None`.
    pub default_frame_ms: u32,
    /// The frames, in playback order. Length ≥ 1.
    pub frames: Vec<ClipFrame>,
}

impl ClipDoc {
    /// A new clip of the given dims with a single empty frame, centre pivot.
    #[must_use]
    pub fn new(dims: [u32; 3]) -> Self {
        let model = VoxelModel::new(dims[0], dims[1], dims[2]);
        let pivot = model.pivot;
        Self {
            name: String::new(),
            dims,
            pivot,
            voxel_world_size: DEFAULT_VOXEL_WORLD_SIZE,
            loop_mode: LoopMode::Loop,
            default_frame_ms: DEFAULT_FRAME_MS,
            frames: vec![ClipFrame::new(model)],
        }
    }

    /// Number of frames (always ≥ 1).
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// An empty model matching the clip's dims + pivot — the shape every frame
    /// must have.
    #[must_use]
    fn blank_frame_model(&self) -> VoxelModel {
        let mut m = VoxelModel::new(self.dims[0], self.dims[1], self.dims[2]);
        m.pivot = self.pivot;
        m
    }

    /// Append a new empty frame; returns its index.
    pub fn add_frame(&mut self) -> usize {
        self.frames.push(ClipFrame::new(self.blank_frame_model()));
        self.frames.len() - 1
    }

    /// Insert a copy of frame `i` right after it (same voxels + duration);
    /// returns the new frame's index. Out-of-range `i` ⇒ append a blank frame.
    pub fn duplicate_frame(&mut self, i: usize) -> usize {
        let Some(src) = self.frames.get(i) else {
            return self.add_frame();
        };
        let copy = src.clone();
        self.frames.insert(i + 1, copy);
        i + 1
    }

    /// Remove frame `i`. Refuses to remove the last remaining frame (a clip is
    /// never empty) and ignores an out-of-range index; returns whether a frame
    /// was removed.
    pub fn remove_frame(&mut self, i: usize) -> bool {
        if self.frames.len() <= 1 || i >= self.frames.len() {
            return false;
        }
        self.frames.remove(i);
        true
    }

    /// Move frame `from` to index `to` (clamped), shifting the rest. No-op for
    /// an out-of-range `from`.
    pub fn move_frame(&mut self, from: usize, to: usize) {
        if from >= self.frames.len() {
            return;
        }
        let to = to.min(self.frames.len() - 1);
        if from == to {
            return;
        }
        let f = self.frames.remove(from);
        self.frames.insert(to, f);
    }

    /// Per-frame durations in ms, resolving each frame's `None` to
    /// [`Self::default_frame_ms`]. Parallel to [`Self::frames`].
    #[must_use]
    pub fn durations(&self) -> Vec<u32> {
        self.frames
            .iter()
            .map(|f| f.duration_ms.unwrap_or(self.default_frame_ms))
            .collect()
    }

    /// Total loop length in ms (sum of resolved frame durations), saturating.
    #[must_use]
    pub fn total_ms(&self) -> u32 {
        self.durations()
            .iter()
            .fold(0u32, |acc, &d| acc.saturating_add(d))
    }

    /// The frame index to show after `elapsed_ms` of playback, honouring the
    /// clip's [`LoopMode`] and per-frame durations — the scrub/play math,
    /// delegated to roxlap so the editor and engine agree exactly.
    #[must_use]
    pub fn frame_at(&self, elapsed_ms: u32) -> usize {
        voxel_clip::frame_at(&self.durations(), self.loop_mode, elapsed_ms)
    }

    /// Resize every frame to `dims` (origin-anchored, like
    /// [`VoxelModel::resized`]) and adopt them as the clip's new bbox. Pivot is
    /// clamped to the new box.
    pub fn resize_all(&mut self, dims: [u32; 3]) {
        for f in &mut self.frames {
            f.model = f.model.resized(dims);
        }
        self.dims = dims;
        self.pivot = self.frames.first().map_or(self.pivot, |f| f.model.pivot);
    }

    /// Crop every frame to the **union** of all frames' occupied bounds, so the
    /// clip's fixed bbox tightens without any frame losing content. No-op if
    /// every frame is empty.
    pub fn crop_all(&mut self) {
        let Some((min, max)) = self.union_bounds() else {
            return;
        };
        let dims = [
            max[0] - min[0] + 1,
            max[1] - min[1] + 1,
            max[2] - min[2] + 1,
        ];
        for f in &mut self.frames {
            let mut out = VoxelModel::new(dims[0], dims[1], dims[2]);
            out.palette = f.model.palette;
            #[allow(clippy::cast_precision_loss)] // dims are tiny; f32 is exact
            {
                out.pivot = [
                    (self.pivot[0] - min[0] as f32).clamp(0.0, dims[0] as f32),
                    (self.pivot[1] - min[1] as f32).clamp(0.0, dims[1] as f32),
                    (self.pivot[2] - min[2] as f32).clamp(0.0, dims[2] as f32),
                ];
            }
            for (x, y, z, col) in f.model.occupied() {
                out.set(x - min[0], y - min[1], z - min[2], col);
            }
            f.model = out;
        }
        self.dims = dims;
        self.pivot = self.frames.first().map_or(self.pivot, |f| f.model.pivot);
    }

    /// The bounding box `(min, max)` covering occupied voxels across *all*
    /// frames, or `None` if every frame is empty.
    #[must_use]
    fn union_bounds(&self) -> Option<([u32; 3], [u32; 3])> {
        let mut acc: Option<([u32; 3], [u32; 3])> = None;
        for f in &self.frames {
            if let Some((min, max)) = f.model.content_bounds() {
                acc = Some(match acc {
                    None => (min, max),
                    Some((amin, amax)) => (
                        [
                            amin[0].min(min[0]),
                            amin[1].min(min[1]),
                            amin[2].min(min[2]),
                        ],
                        [
                            amax[0].max(max[0]),
                            amax[1].max(max[1]),
                            amax[2].max(max[2]),
                        ],
                    ),
                });
            }
        }
        acc
    }

    /// Compile the clip to a roxlap [`VoxelClip`] (auto keyframe/delta), the
    /// in-memory `.rvc`. Frame 0's pivot becomes the clip pivot.
    ///
    /// # Panics
    /// Never in practice: the clip invariant (≥ 1 frame, uniform dims) is what
    /// the encoder requires.
    #[must_use]
    pub fn to_voxel_clip(&self) -> VoxelClip {
        let kv6s: Vec<_> = self.frames.iter().map(|f| f.model.to_kv6()).collect();
        let durations = self.durations();
        VoxelClip::from_kv6_frames_auto(
            &kv6s,
            self.voxel_world_size,
            self.loop_mode,
            &durations,
            self.default_frame_ms,
            DEFAULT_MAX_KEYFRAME_GAP,
        )
        .expect("clip always has ≥1 uniform-dims frame")
    }

    /// Serialize to `.rvc` bytes — the engine-played export.
    #[must_use]
    pub fn to_rvc_bytes(&self) -> Vec<u8> {
        self.to_voxel_clip().serialize()
    }

    /// Parse `.rvc` bytes back into an (editable, surface-only) clip document.
    ///
    /// # Errors
    /// [`ClipLoadError::Parse`] if the bytes are not a valid `.rvc`;
    /// [`ClipLoadError::Decode`] if its frame stream cannot be reconstructed.
    pub fn from_rvc_bytes(bytes: &[u8]) -> Result<Self, ClipLoadError> {
        let clip = VoxelClip::parse(bytes).map_err(ClipLoadError::Parse)?;
        let decoded = clip.decode().map_err(ClipLoadError::Decode)?;
        let durations = decoded.durations.clone();
        let frames = decoded
            .frames
            .iter()
            .enumerate()
            .map(|(i, vf)| {
                let kv6 = vf.to_kv6(decoded.dims, decoded.pivot);
                ClipFrame {
                    model: VoxelModel::from_kv6(&kv6),
                    duration_ms: durations.get(i).copied(),
                }
            })
            .collect();
        Ok(Self {
            name: String::new(),
            dims: decoded.dims,
            pivot: decoded.pivot,
            voxel_world_size: decoded.voxel_world_size,
            loop_mode: decoded.loop_mode,
            default_frame_ms: clip.default_frame_ms,
            frames,
        })
    }
}

/// Stable `u8` tag for a [`LoopMode`], for the `.demiurg` project schema
/// (roxlap's own mapping is private). Mirrors the on-disk `.rvc` ordering.
#[must_use]
pub fn loop_mode_to_u8(m: LoopMode) -> u8 {
    match m {
        LoopMode::Loop => 0,
        LoopMode::Once => 1,
        LoopMode::PingPong => 2,
    }
}

/// Inverse of [`loop_mode_to_u8`]; unknown values fall back to
/// [`LoopMode::Loop`].
#[must_use]
pub fn loop_mode_from_u8(v: u8) -> LoopMode {
    match v {
        1 => LoopMode::Once,
        2 => LoopMode::PingPong,
        _ => LoopMode::Loop,
    }
}

/// Why [`ClipDoc::from_rvc_bytes`] could not load a clip.
#[derive(Debug)]
pub enum ClipLoadError {
    Parse(ParseError),
    Decode(DecodeError),
}

impl std::fmt::Display for ClipLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "not a valid .rvc clip: {e:?}"),
            Self::Decode(e) => write!(f, ".rvc clip could not be decoded: {e:?}"),
        }
    }
}

impl std::error::Error for ClipLoadError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn occ(m: &VoxelModel) -> BTreeMap<(u32, u32, u32), u32> {
        m.occupied().map(|(x, y, z, c)| ((x, y, z), c)).collect()
    }

    /// Build a 4³ clip whose frames are exposed (surface-only) so kv6/rvc loses
    /// nothing, with a moving voxel so frames actually differ.
    fn sample_clip() -> ClipDoc {
        let mut clip = ClipDoc::new([4, 4, 4]);
        // frame 0 already exists; paint it, then add two more.
        clip.frames[0].model.set(0, 0, 0, 0x80ff_0000);
        let f1 = clip.add_frame();
        clip.frames[f1].model.set(1, 0, 0, 0x8000_ff00);
        clip.frames[f1].duration_ms = Some(120);
        let f2 = clip.add_frame();
        clip.frames[f2].model.set(2, 0, 0, 0x8000_00ff);
        clip
    }

    #[test]
    fn new_clip_has_one_blank_frame() {
        let clip = ClipDoc::new([8, 8, 8]);
        assert_eq!(clip.frame_count(), 1);
        assert_eq!(clip.frames[0].model.dims(), (8, 8, 8));
        assert_eq!(clip.frames[0].model.occupied_count(), 0);
    }

    #[test]
    fn frame_ops_add_dup_remove_move() {
        let mut clip = ClipDoc::new([2, 2, 2]);
        clip.frames[0].model.set(0, 0, 0, 0x80ff_0000);
        let d = clip.duplicate_frame(0);
        assert_eq!(d, 1);
        assert_eq!(occ(&clip.frames[0].model), occ(&clip.frames[1].model));
        let a = clip.add_frame();
        assert_eq!(a, 2);
        assert_eq!(clip.frames[2].model.occupied_count(), 0);

        // move the blank frame to the front, then back.
        clip.move_frame(2, 0);
        assert_eq!(clip.frames[0].model.occupied_count(), 0);
        clip.move_frame(0, 2);
        assert_eq!(clip.frames[2].model.occupied_count(), 0);

        assert!(clip.remove_frame(2));
        assert_eq!(clip.frame_count(), 2);
    }

    #[test]
    fn cannot_remove_last_frame() {
        let mut clip = ClipDoc::new([2, 2, 2]);
        assert!(!clip.remove_frame(0));
        assert_eq!(clip.frame_count(), 1);
    }

    #[test]
    fn durations_resolve_default_and_override() {
        let mut clip = ClipDoc::new([1, 1, 1]);
        clip.default_frame_ms = 50;
        clip.add_frame();
        clip.frames[1].duration_ms = Some(200);
        assert_eq!(clip.durations(), vec![50, 200]);
        assert_eq!(clip.total_ms(), 250);
    }

    #[test]
    fn frame_at_loops_over_total() {
        let mut clip = ClipDoc::new([1, 1, 1]);
        clip.default_frame_ms = 100;
        clip.add_frame(); // 2 frames, 100ms each, Loop
        assert_eq!(clip.frame_at(0), 0);
        assert_eq!(clip.frame_at(150), 1);
        assert_eq!(clip.frame_at(250), 0, "wraps at 200ms total");
    }

    #[test]
    fn rvc_round_trip_preserves_frames_and_timing() {
        let clip = sample_clip();
        let bytes = clip.to_rvc_bytes();
        let back = ClipDoc::from_rvc_bytes(&bytes).expect("round-trips");

        assert_eq!(back.frame_count(), clip.frame_count());
        assert_eq!(back.dims, clip.dims);
        assert_eq!(back.loop_mode, clip.loop_mode);
        assert_eq!(back.durations(), clip.durations());
        for (a, b) in clip.frames.iter().zip(&back.frames) {
            assert_eq!(occ(&a.model), occ(&b.model), "frame voxels survive");
        }
    }

    #[test]
    fn crop_all_tightens_to_frame_union() {
        // Two frames each with one voxel at different positions: the union
        // bbox spans both, and neither frame loses its voxel.
        let mut clip = ClipDoc::new([8, 8, 8]);
        clip.frames[0].model.set(2, 2, 2, 0x80ff_0000);
        let f1 = clip.add_frame();
        clip.frames[f1].model.set(4, 3, 2, 0x8000_ff00);
        clip.crop_all();
        assert_eq!(clip.dims, [3, 2, 1]);
        assert_eq!(clip.frames[0].model.get(0, 0, 0), 0x80ff_0000);
        assert_eq!(clip.frames[1].model.get(2, 1, 0), 0x8000_ff00);
    }

    #[test]
    fn resize_all_changes_every_frame_bbox() {
        let mut clip = sample_clip();
        clip.resize_all([6, 6, 6]);
        assert_eq!(clip.dims, [6, 6, 6]);
        for f in &clip.frames {
            assert_eq!(f.model.dims(), (6, 6, 6));
        }
        // origin-anchored: the painted voxels keep their coordinates.
        assert_eq!(clip.frames[0].model.get(0, 0, 0), 0x80ff_0000);
    }
}
