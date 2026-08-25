//! The JSON exchange manifest a DCC exporter (the Blender addon) writes.
//!
//! The manifest is deliberately dumb: plain JSON a Python script builds with
//! the standard library, carrying only what the `.demiurg` document model
//! actually holds. Every geometric decision (voxelization, weight-based
//! segmentation of a skinned mesh into rigid per-bone chunks, baking F-curves
//! into per-frame poses) happens on the exporter side — this crate is the
//! format writer, not a converter of art.
//!
//! Two shapes share the file, discriminated by `format`:
//!
//! * `"demiurg-model"` — a single [`MeshSpec`] under `mesh`, written as a bare
//!   model project (this is the `.vox` bridge plus the pivot `.vox` can't
//!   carry).
//! * `"demiurg-rig"` — `bones` + `clips`, written as a rigged character.
//!
//! Unknown fields are rejected: a typo in a hand- or script-written manifest
//! should fail loudly at the converter, not silently export a bone with a
//! default pivot. Forward compatibility rides on [`Manifest::version`].

use std::collections::BTreeMap;

use serde::Deserialize;

/// The manifest schema version this build understands.
pub const VERSION: u32 = 1;

/// `format` value of a rigged-character manifest.
pub const FORMAT_RIG: &str = "demiurg-rig";
/// `format` value of a bare-model manifest.
pub const FORMAT_MODEL: &str = "demiurg-model";

/// A parsed exchange manifest.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// [`FORMAT_RIG`] or [`FORMAT_MODEL`].
    pub format: String,
    /// Schema version — must be [`VERSION`].
    pub version: u32,
    /// Free-text note, ignored by the converter. Somewhere for an exporter to
    /// stamp its own version and the source `.blend`, so a manifest found on
    /// disk a year later says where it came from.
    #[serde(default, rename = "_comment")]
    pub comment: Option<String>,
    /// Document name (the character name stored in the `.rkc`).
    #[serde(default)]
    pub name: String,
    /// World placement of the root bone. Rig manifests only.
    #[serde(default)]
    pub root: [f32; 3],
    /// Bones in export order. Rig manifests only.
    #[serde(default)]
    pub bones: Vec<BoneSpec>,
    /// Animation clips. Rig manifests only.
    #[serde(default)]
    pub clips: Vec<ClipSpec>,
    /// The model. Bare-model manifests only.
    #[serde(default)]
    pub mesh: Option<MeshSpec>,
}

/// One bone: its mesh, where it hangs off its parent, and the axis its rest
/// frame is built from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoneSpec {
    /// Unique within the manifest — poses reference bones by this name.
    pub name: String,
    /// Parent bone name; `null` / absent makes this a root bone.
    #[serde(default)]
    pub parent: Option<String>,
    /// Where this bone's pivot sits, measured from the **parent's** pivot, in
    /// voxels. The child attaches by its own mesh pivot, so put each bone's
    /// pivot on its head and this is just the difference of the two heads.
    /// (The engine's hinge anchor is the negation of this; the converter
    /// flips it.)
    #[serde(default)]
    pub joint: [f32; 3],
    /// The hinge axis. With a full-TRS keyframe this only picks the rest
    /// frame, and using the same axis on both sides (which the converter does)
    /// makes the rest rotation the identity — so the default is almost always
    /// right, and the pose is whatever the keyframe quaternion says.
    #[serde(default = "default_axis")]
    pub axis: [f32; 3],
    /// The bone's mesh. Absent means an empty 1×1×1 model — a dummy / helper
    /// bone that carries no geometry.
    #[serde(default)]
    pub mesh: Option<MeshSpec>,
}

/// A voxel mesh, either inline or read from a `.vox` file beside the manifest.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshSpec {
    /// Grid dimensions. Required for an inline mesh, rejected with `vox_file`
    /// (the file carries its own).
    #[serde(default)]
    pub dims: Option<[u32; 3]>,
    /// Pivot in voxel units — the point the bone rotates about, and the anchor
    /// the parent's `joint` attaches to. Defaults to the grid centre (for
    /// `vox_file`, to whatever the import produced).
    #[serde(default)]
    pub pivot: Option<[f32; 3]>,
    /// Occupied voxels, `[x, y, z, "rrggbb"]`. Empty is legal (an invisible
    /// bone).
    #[serde(default)]
    pub voxels: Vec<Voxel>,
    /// Path to a `.vox` to import instead of `voxels`, resolved relative to
    /// the manifest's own directory. Lets the exporter hand voxelization to
    /// `MagicaVoxel` / vengi and only describe the rig here.
    #[serde(default)]
    pub vox_file: Option<String>,
}

/// One inline voxel: `[x, y, z, "rrggbb"]`.
#[derive(Debug, Clone, Deserialize)]
pub struct Voxel(pub u32, pub u32, pub u32, pub String);

/// One animation clip: a name, a baked keyframe list, and how it ends.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipSpec {
    /// Clip name the engine plays it by (`"walk"`, `"idle"`).
    pub name: String,
    /// `true` (the default) returns to the first key after the last segment;
    /// `false` plays once and holds the last pose.
    #[serde(rename = "loop", default = "default_true")]
    pub loops: bool,
    /// Playback duration in ms — the time of the trailing loop marker, i.e.
    /// when a cyclic clip is back at its first key. Absent leaves the
    /// converter's default tail after the last key, which is right for a
    /// one-shot but wrong for a cycle: a loop should set this to the action's
    /// full length so the last segment has the correct duration.
    #[serde(default)]
    pub length_ms: Option<i32>,
    /// Keyframes, each a full-skeleton pose. Order doesn't matter (sorted by
    /// time), but two keys at the same time is a last-one-wins overwrite.
    pub keys: Vec<KeySpec>,
}

/// One keyframe: an absolute time plus the bones that move at it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeySpec {
    /// Absolute time on the clip's clock, in milliseconds.
    pub t: i32,
    /// Per-bone local transform, keyed by bone name. Bones left out of the map
    /// are the identity at this key — but note the format stores whole-skeleton
    /// poses, so an omitted bone is *posed to rest*, not "unchanged since the
    /// previous key". Bake every animated bone into every key.
    #[serde(default)]
    pub pose: BTreeMap<String, XformSpec>,
}

/// A bone's local transform at a keyframe.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XformSpec {
    /// Translation in voxel units, added to the bone's joint anchor.
    #[serde(default)]
    pub t: [f32; 3],
    /// Rotation quaternion as `[x, y, z, w]` — scalar **last**, matching
    /// neither of Blender's orderings, so the exporter must reorder.
    #[serde(default = "default_quat")]
    pub r: [f32; 4],
    /// Non-uniform scale along the bone's local axes.
    #[serde(default = "default_scale")]
    pub s: [f32; 3],
}

impl Manifest {
    /// Parse manifest JSON.
    ///
    /// # Errors
    /// [`serde_json::Error`] if the bytes aren't valid JSON or don't match the
    /// schema (including an unknown field — see the module docs).
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// The default hinge axis: `+Z`. Must be non-zero — the solver runs it through
/// `genperp`, and a zero axis collapses the limb to an invisible point.
fn default_axis() -> [f32; 3] {
    [0.0, 0.0, 1.0]
}

/// The identity rotation, `[x, y, z, w]`.
fn default_quat() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

/// Unit scale.
fn default_scale() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

/// `#[serde(default)]` for a `bool` field that defaults to `true`.
fn default_true() -> bool {
    true
}
