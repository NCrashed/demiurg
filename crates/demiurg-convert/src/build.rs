//! Manifest → document: assembling a [`Rig`] (or a bare [`VoxelModel`]) from a
//! parsed [`Manifest`].
//!
//! The mapping onto the engine's rig model, and the traps it hides:
//!
//! * A bone's mesh **pivot is its joint** — the hinge attaches the child by
//!   its own pivot (`p[0] = 0`) to the parent-side anchor (`p[1] = joint`).
//! * `htype` must stay `0`. The solver applies the animated transform only for
//!   `htype == 0`; anything else silently pins the bone to its rest pose.
//! * The hinge axis is used on **both** sides (`v[0] == v[1]`), so the rest
//!   rotation is the identity and a keyframe quaternion is the bone's whole
//!   orientation. The axis is then just a frame convention, not a constraint —
//!   rotation is a free quaternion, not a hinge angle.
//! * The hinge range is opened to the full `i16` span, because the editor
//!   treats `vmin == vmax` as a locked (unposeable) bone.
//! * Two sign / frame conventions are translated so the manifest can mean what
//!   an exporter expects: the hinge anchor is the **negation** of `joint`, and
//!   a keyframe's `t` absorbs the arc the solver would otherwise swing the
//!   bone's head through when `r` turns. Both are documented at their code.
//! * **Root bones cannot be animated.** `solve_kfa_limbs` gives a bone with
//!   `parent < 0` the sprite's own basis and ignores its keyframe entirely, so
//!   a non-identity root pose would silently do nothing — the converter
//!   rejects it instead. Rigs that need root motion put a dummy bone on top
//!   (Blender's usual `root` / COG bone maps straight onto this).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use demiurg_core::VoxelModel;
use demiurg_core::clip::{ClipDoc, ClipFrame, LoopMode};
use demiurg_core::rig::{LayerPlayback as ClipPlayback, Rig, RigBone};
use demiurg_core::vox;
use roxlap_formats::kfa::{Hinge, Point3};
use roxlap_formats::xform::{BoneXform, Quat};

use crate::manifest::{BoneSpec, ClipSpec, Manifest, MeshSpec, VERSION, VoxelClipSpec};

/// How far a keyframe may sit from the identity before it counts as an actual
/// pose. Baked exports carry float noise, so an exact comparison would flag a
/// rest key as animated.
const IDENTITY_EPS: f32 = 1e-6;

/// Why a manifest could not be turned into a document.
#[derive(Debug)]
pub enum ConvertError {
    /// `format` is neither of the two known values.
    UnknownFormat(String),
    /// `version` is not [`VERSION`].
    UnsupportedVersion(u32),
    /// A rig manifest with an empty `bones` list.
    NoBones,
    /// More bones than a hinge's `i32` parent index can address. Unreachable
    /// with real art, but the cast has to be total.
    TooManyBones(usize),
    /// A model manifest with no `mesh`.
    NoMesh,
    /// Two bones share a name (poses reference bones by name).
    DuplicateBone(String),
    /// A bone's `parent` names a bone that isn't in the manifest.
    UnknownParent {
        /// The bone carrying the bad reference.
        bone: String,
        /// The name it pointed at.
        parent: String,
    },
    /// The parent chain from this bone never reaches a root.
    ParentCycle(String),
    /// A mesh gave both `vox_file` and inline `dims`/`voxels`, or an inline
    /// mesh gave no `dims`.
    MeshSource(String),
    /// A bone gave both a rigid `mesh` and a per-frame `clip`. The engine
    /// draws one primary attachment, so only one can win.
    MeshAndClip(String),
    /// A voxel clip with no frames.
    EmptyClip(String),
    /// A voxel clip's `loop` is not one of the three the format has.
    BadLoopMode {
        /// The bone the clip belongs to.
        bone: String,
        /// The offending value.
        value: String,
    },
    /// A voxel's colour is not a 6-digit hex string.
    BadColor {
        /// Bone (or `"mesh"`) the voxel belongs to.
        at: String,
        /// The offending value.
        value: String,
    },
    /// An inline voxel sits outside the mesh's `dims`.
    VoxelOutOfBounds {
        /// Bone (or `"mesh"`) the voxel belongs to.
        at: String,
        /// The offending coordinate.
        pos: [u32; 3],
        /// The grid it had to fit in.
        dims: [u32; 3],
    },
    /// A bone's hinge axis is (near) zero — the solver's `genperp` would
    /// collapse the limb to an invisible point.
    DegenerateAxis(String),
    /// A pose names a bone the manifest doesn't define.
    UnknownPoseBone {
        /// The clip the pose belongs to.
        clip: String,
        /// The name it pointed at.
        bone: String,
    },
    /// A keyframe poses a root bone — see the module docs.
    RootAnimated {
        /// The clip the pose belongs to.
        clip: String,
        /// The root bone it tried to move.
        bone: String,
    },
    /// A `vox_file` could not be read.
    VoxRead {
        /// The path, as resolved against the manifest's directory.
        path: PathBuf,
        /// The OS error.
        err: std::io::Error,
    },
    /// A `vox_file` is not a valid `.vox`.
    VoxParse {
        /// The path, as resolved against the manifest's directory.
        path: PathBuf,
        /// The parser's message.
        err: String,
    },
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat(g) => write!(
                f,
                "unknown format {g:?}: expected \"demiurg-rig\" or \"demiurg-model\""
            ),
            Self::UnsupportedVersion(v) => {
                write!(
                    f,
                    "manifest version {v} is not supported (this build reads version {VERSION})"
                )
            }
            Self::NoBones => write!(f, "a rig manifest needs at least one bone"),
            Self::TooManyBones(n) => write!(f, "{n} bones is more than the format can index"),
            Self::NoMesh => write!(f, "a model manifest needs a \"mesh\""),
            Self::DuplicateBone(n) => {
                write!(f, "two bones are named {n:?}; bone names must be unique")
            }
            Self::UnknownParent { bone, parent } => {
                write!(
                    f,
                    "bone {bone:?} has parent {parent:?}, which is not in the manifest"
                )
            }
            Self::ParentCycle(n) => write!(f, "bone {n:?} is part of a parent cycle"),
            Self::MeshSource(at) => write!(
                f,
                "{at}: give either \"vox_file\" or inline \"dims\" (+ \"voxels\"), not both or neither"
            ),
            Self::MeshAndClip(n) => write!(
                f,
                "bone {n:?} has both a \"mesh\" and a \"clip\"; a bone draws one or the other"
            ),
            Self::EmptyClip(n) => write!(f, "bone {n:?}: a \"clip\" needs at least one frame"),
            Self::BadLoopMode { bone, value } => write!(
                f,
                "bone {bone:?}: loop {value:?} is not \"loop\", \"once\", or \"pingpong\""
            ),
            Self::BadColor { at, value } => {
                write!(
                    f,
                    "{at}: {value:?} is not a 6-digit hex colour like \"ff8800\""
                )
            }
            Self::VoxelOutOfBounds { at, pos, dims } => write!(
                f,
                "{at}: voxel [{}, {}, {}] is outside dims [{}, {}, {}]",
                pos[0], pos[1], pos[2], dims[0], dims[1], dims[2]
            ),
            Self::DegenerateAxis(n) => {
                write!(
                    f,
                    "bone {n:?} has a zero-length axis; it must be a direction vector"
                )
            }
            Self::UnknownPoseBone { clip, bone } => {
                write!(
                    f,
                    "clip {clip:?} poses bone {bone:?}, which is not in the manifest"
                )
            }
            Self::RootAnimated { clip, bone } => write!(
                f,
                "clip {clip:?} poses root bone {bone:?}, but the solver ignores a root bone's \
                 keyframe (it takes the sprite's own basis). Parent it to a dummy root bone."
            ),
            Self::VoxRead { path, err } => write!(f, "read {}: {err}", path.display()),
            Self::VoxParse { path, err } => write!(f, "{}: {err}", path.display()),
        }
    }
}

impl std::error::Error for ConvertError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::VoxRead { err, .. } => Some(err),
            _ => None,
        }
    }
}

/// Assemble a rigged character from a `"demiurg-rig"` manifest. `base_dir` is
/// the manifest's own directory — `vox_file` paths resolve against it.
///
/// # Errors
/// [`ConvertError`] for any schema or referential problem; the message names
/// the bone or clip at fault.
pub fn build_rig(m: &Manifest, base_dir: &Path) -> Result<Rig, ConvertError> {
    check_header(m)?;
    if m.bones.is_empty() {
        return Err(ConvertError::NoBones);
    }
    // Checked once so every `parent` cast below is total.
    if i32::try_from(m.bones.len()).is_err() {
        return Err(ConvertError::TooManyBones(m.bones.len()));
    }
    let index = bone_index(&m.bones)?;
    check_acyclic(&m.bones, &index)?;

    let mut bones = Vec::with_capacity(m.bones.len());
    for spec in &m.bones {
        // `bone_index` proved every parent name resolves, and the count fits.
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let parent = spec.parent.as_ref().map_or(-1, |p| index[p] as i32);
        if spec.mesh.is_some() && spec.clip.is_some() {
            return Err(ConvertError::MeshAndClip(spec.name.clone()));
        }
        // A clip primary leaves `model` an unused placeholder — the engine
        // reads the flipbook instead — so don't spend a grid on it.
        let model = if spec.clip.is_some() {
            VoxelModel::new(1, 1, 1)
        } else {
            build_mesh(
                spec.mesh.as_ref(),
                &format!("bone {:?}", spec.name),
                base_dir,
            )?
        };
        let axis =
            unit_axis(spec.axis).ok_or_else(|| ConvertError::DegenerateAxis(spec.name.clone()))?;
        let mut bone = RigBone::mesh(
            spec.name.clone(),
            model,
            Hinge {
                parent,
                // The child attaches by its own pivot (`p[0] = 0`) to the
                // parent-side anchor. Negated, because the solver places the
                // child at `parent + (p[0] - p[1])` — so storing the joint
                // as-is would hang every limb off the opposite side of its
                // parent. The manifest's `joint` means what an exporter
                // expects: where the child's pivot ends up, measured from the
                // parent's.
                p: [pt3([0.0, 0.0, 0.0]), pt3(neg(spec.joint))],
                // Same axis on both sides ⇒ identity rest rotation.
                v: [pt3(axis), pt3(axis)],
                // A full range: `vmin == vmax` reads as a locked bone.
                vmin: i16::MIN,
                vmax: i16::MAX,
                // Anything but 0 makes the solver drop the animation.
                htype: 0,
                filler: [0; 7],
            },
            Vec::new(),
        );
        if let Some(clip) = &spec.clip {
            bone.primary_clip = Some(build_voxel_clip(clip, &spec.name)?);
            bone.primary_playback = playback_of(clip);
        }
        bones.push(bone);
    }

    let mut rig = Rig {
        name: m.name.clone(),
        root: m.root,
        bones,
        clips: Vec::new(),
        clip_easing: Vec::new(),
    };
    for spec in &m.clips {
        add_clip(&mut rig, spec, &index)?;
    }
    Ok(rig)
}

/// Assemble a bare model from a `"demiurg-model"` manifest.
///
/// # Errors
/// [`ConvertError`] if the header is wrong, `mesh` is missing, or the mesh
/// itself is malformed.
pub fn build_model(m: &Manifest, base_dir: &Path) -> Result<VoxelModel, ConvertError> {
    check_header(m)?;
    let spec = m.mesh.as_ref().ok_or(ConvertError::NoMesh)?;
    build_mesh(Some(spec), "mesh", base_dir)
}

/// Append one clip, baking its keys into the rig's `seq`/`frmval` tables.
fn add_clip(
    rig: &mut Rig,
    spec: &ClipSpec,
    index: &BTreeMap<String, usize>,
) -> Result<(), ConvertError> {
    let n = rig.bones.len();
    let ci = rig.add_clip(spec.name.clone());
    // `add_clip` seeds a rest key at t=0 so the timeline is never empty; a
    // manifest key at t=0 overwrites it, otherwise it is dropped below.
    let mut seeded = true;
    for key in &spec.keys {
        let mut xforms = vec![BoneXform::IDENTITY; n];
        for (bone, x) in &key.pose {
            let i = *index
                .get(bone)
                .ok_or_else(|| ConvertError::UnknownPoseBone {
                    clip: spec.name.clone(),
                    bone: bone.clone(),
                })?;
            let r = Quat {
                x: x.r[0],
                y: x.r[1],
                z: x.r[2],
                w: x.r[3],
            }
            .normalize();
            // The solver puts the bone at `t + r · anchor`, so a rotation
            // drags the bone's head around its PARENT's pivot on an arc of
            // radius `joint` — a shoulder rotation would walk the arm off the
            // body. Every DCC (and every animator) means "spin about my own
            // joint", so cancel the arc: with `t += joint - r · joint` the
            // head stays at `joint + t` whatever `r` does, and `r` is left as
            // a pure rotation about it.
            let joint = neg(pt_arr(rig.bones[i].hinge.p[1]));
            let swung = r.rotate(joint);
            let xform = BoneXform {
                t: [
                    x.t[0] + joint[0] - swung[0],
                    x.t[1] + joint[1] - swung[1],
                    x.t[2] + joint[2] - swung[2],
                ],
                r,
                s: x.s,
            };
            if rig.bones[i].hinge.parent < 0 && !is_identity(&xform) {
                return Err(ConvertError::RootAnimated {
                    clip: spec.name.clone(),
                    bone: bone.clone(),
                });
            }
            xforms[i] = xform;
        }
        // `add_keyframe` clamps a negative time to 0, so that also lands on
        // the seed key.
        if key.t <= 0 {
            seeded = false;
        }
        rig.add_keyframe(ci, key.t, xforms);
    }
    if seeded && !spec.keys.is_empty() {
        rig.remove_keyframe(ci, 0);
    }
    if let Some(ms) = spec.length_ms {
        rig.set_clip_length(ci, ms);
    }
    rig.set_clip_loops(ci, spec.loops);
    Ok(())
}

/// Build a bone's per-frame flipbook.
///
/// Every frame shares the clip's grid and pivot — the engine's rule, not a
/// simplification here — so the frames differ only in which voxels are filled.
fn build_voxel_clip(spec: &VoxelClipSpec, bone: &str) -> Result<ClipDoc, ConvertError> {
    if spec.frames.is_empty() {
        return Err(ConvertError::EmptyClip(bone.to_string()));
    }
    let loop_mode = match spec.loop_mode.as_str() {
        "loop" => LoopMode::Loop,
        "once" => LoopMode::Once,
        "pingpong" => LoopMode::PingPong,
        other => {
            return Err(ConvertError::BadLoopMode {
                bone: bone.to_string(),
                value: other.to_string(),
            });
        }
    };
    let mut doc = ClipDoc::new(spec.dims);
    doc.name = bone.to_string();
    doc.loop_mode = loop_mode;
    doc.default_frame_ms = spec.frame_ms;
    if let Some(p) = spec.pivot {
        doc.pivot = p;
    }
    let at = format!("bone {bone:?} clip");
    let mut frames = Vec::with_capacity(spec.frames.len());
    for f in &spec.frames {
        let mut model = VoxelModel::new(spec.dims[0], spec.dims[1], spec.dims[2]);
        model.pivot = doc.pivot;
        for v in &f.voxels {
            let col = parse_color(&v.3).ok_or_else(|| ConvertError::BadColor {
                at: at.clone(),
                value: v.3.clone(),
            })?;
            if !model.set(v.0, v.1, v.2, col) {
                return Err(ConvertError::VoxelOutOfBounds {
                    at: at.clone(),
                    pos: [v.0, v.1, v.2],
                    dims: spec.dims,
                });
            }
        }
        frames.push(ClipFrame {
            model,
            duration_ms: f.duration_ms,
        });
    }
    doc.frames = frames;
    Ok(doc)
}

/// A clip's playback params. Speed is Q8 fixed point in the engine.
fn playback_of(spec: &VoxelClipSpec) -> ClipPlayback {
    #[allow(clippy::cast_possible_truncation)] // a sane rate is far inside i32
    let speed_q8 = (spec.speed * 256.0).round() as i32;
    ClipPlayback {
        speed_q8,
        start_phase_ms: spec.phase_ms,
    }
}

/// Build one mesh. `at` labels it in error messages.
fn build_mesh(
    spec: Option<&MeshSpec>,
    at: &str,
    base_dir: &Path,
) -> Result<VoxelModel, ConvertError> {
    // No mesh at all: a dummy bone, which the engine still needs a (empty)
    // model for.
    let Some(spec) = spec else {
        return Ok(VoxelModel::new(1, 1, 1));
    };
    let mut model = match (&spec.vox_file, spec.dims) {
        (Some(file), None) if spec.voxels.is_empty() => {
            let path = base_dir.join(file);
            let bytes = std::fs::read(&path).map_err(|err| ConvertError::VoxRead {
                path: path.clone(),
                err,
            })?;
            vox::parse(&bytes).map_err(|e| ConvertError::VoxParse {
                path,
                err: e.to_string(),
            })?
        }
        (None, Some(dims)) => {
            let mut model = VoxelModel::new(dims[0], dims[1], dims[2]);
            for v in &spec.voxels {
                let col = parse_color(&v.3).ok_or_else(|| ConvertError::BadColor {
                    at: at.to_string(),
                    value: v.3.clone(),
                })?;
                if !model.set(v.0, v.1, v.2, col) {
                    return Err(ConvertError::VoxelOutOfBounds {
                        at: at.to_string(),
                        pos: [v.0, v.1, v.2],
                        dims,
                    });
                }
            }
            model
        }
        _ => return Err(ConvertError::MeshSource(at.to_string())),
    };
    if let Some(p) = spec.pivot {
        model.pivot = p;
    }
    Ok(model)
}

/// `"rrggbb"` → the packed `0x80RRGGBB` colour word the grid stores. A leading
/// `#` is accepted so a hex literal copied out of a colour picker works.
fn parse_color(hex: &str) -> Option<u32> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16)
        .ok()
        .map(|rgb| 0x8000_0000 | rgb)
}

/// Map bone names to indices, rejecting duplicates and dangling parents.
fn bone_index(bones: &[BoneSpec]) -> Result<BTreeMap<String, usize>, ConvertError> {
    let mut index = BTreeMap::new();
    for (i, b) in bones.iter().enumerate() {
        if index.insert(b.name.clone(), i).is_some() {
            return Err(ConvertError::DuplicateBone(b.name.clone()));
        }
    }
    for b in bones {
        if let Some(p) = &b.parent
            && !index.contains_key(p)
        {
            return Err(ConvertError::UnknownParent {
                bone: b.name.clone(),
                parent: p.clone(),
            });
        }
    }
    Ok(index)
}

/// Every bone's parent chain must reach a root; a cycle would hang the
/// solver's topological walk.
fn check_acyclic(bones: &[BoneSpec], index: &BTreeMap<String, usize>) -> Result<(), ConvertError> {
    for b in bones {
        let mut cur = b;
        // The longest cycle-free chain visits every bone once, so a chain
        // still going after `bones.len()` hops has revisited one.
        for _ in 0..bones.len() {
            let Some(parent) = &cur.parent else {
                break;
            };
            cur = &bones[index[parent]]; // `bone_index` proved this resolves
        }
        if cur.parent.is_some() {
            return Err(ConvertError::ParentCycle(b.name.clone()));
        }
    }
    Ok(())
}

/// Both header fields, checked the same way for either document shape.
fn check_header(m: &Manifest) -> Result<(), ConvertError> {
    if m.version != VERSION {
        return Err(ConvertError::UnsupportedVersion(m.version));
    }
    if m.format != crate::manifest::FORMAT_RIG && m.format != crate::manifest::FORMAT_MODEL {
        return Err(ConvertError::UnknownFormat(m.format.clone()));
    }
    Ok(())
}

/// Normalize an axis, or `None` if it is too short to define a direction.
fn unit_axis(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = v[0].mul_add(v[0], v[1].mul_add(v[1], v[2] * v[2])).sqrt();
    (len > 1e-6).then(|| [v[0] / len, v[1] / len, v[2] / len])
}

/// Whether a transform is the identity up to [`IDENTITY_EPS`]. `-q` is the same
/// rotation as `q`, so the quaternion is compared by `|w|`.
fn is_identity(x: &BoneXform) -> bool {
    x.t.iter().all(|c| c.abs() <= IDENTITY_EPS)
        && x.s.iter().all(|c| (c - 1.0).abs() <= IDENTITY_EPS)
        && (x.r.w.abs() - 1.0).abs() <= IDENTITY_EPS
}

/// Componentwise negation.
fn neg(v: [f32; 3]) -> [f32; 3] {
    [-v[0], -v[1], -v[2]]
}

/// The engine's point type → `[f32; 3]`.
fn pt_arr(p: Point3) -> [f32; 3] {
    [p.x, p.y, p.z]
}

/// `[f32; 3]` → the engine's point type.
fn pt3(v: [f32; 3]) -> Point3 {
    Point3 {
        x: v[0],
        y: v[1],
        z: v[2],
    }
}
