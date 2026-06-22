//! KFA (skeletal) rig preview — the editor's animation view.
//!
//! Owns an editable [`Rig`] (the live document) and the [`KfaSprite`]s built
//! from it, advances its baked clip, and emits skeleton gizmo lines. The
//! host (`demiurg-app`) hands the sprites to
//! `SceneRenderer::{set_kfa_sprites, update_kfa_poses}` each frame.
//!
//! Until rig authoring exists, [`demo_rig`] seeds a synthetic two-bone rig.

use demiurg_core::{Rig, RigBone, VoxelModel};
use glam::DVec3;
use roxlap_core::kfa_draw::solve_kfa_limbs;
use roxlap_formats::character::{Clip, ClipData};
use roxlap_formats::kfa::{Hinge, KfaSprite, Point3, Seq};

use crate::{Line3, OrbitCamera};

/// Colour of the skeleton gizmo (always-on-top yellow, like the hover box).
const BONE_COLOR: u32 = 0xffff_e600;
/// Colour of the active (selected) bone in the gizmo — bright cyan, thicker,
/// so the bone being posed stands out from the yellow skeleton.
const ACTIVE_BONE_COLOR: u32 = 0xff00_e5ff;

/// A previewable KFA rig: the editable source [`Rig`] plus the live
/// [`KfaSprite`]s built from it.
pub struct KfaView {
    rig: Rig,
    kfas: Vec<KfaSprite>,
}

impl KfaView {
    /// Build a view from `rig`, baking in `clip` (a `Skeletal` clip index,
    /// or `None` for the rest pose).
    #[must_use]
    pub fn from_rig(rig: Rig, clip: Option<usize>) -> Self {
        let kfas = vec![rig.to_character().to_kfa_sprite(clip)];
        Self { rig, kfas }
    }

    /// Parse an `.rkc` rigged-character file into a view. Plays the first
    /// clip if any (rest pose otherwise) — a stand-in until the timeline
    /// drives playback.
    ///
    /// # Errors
    /// A message if the bytes aren't a valid `.rkc` container.
    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        let rig = Rig::from_rkc_bytes(bytes)?;
        let clip = (!rig.clips.is_empty()).then_some(0);
        Ok(Self::from_rig(rig, clip))
    }

    /// The sprites to hand to `SceneRenderer::set_kfa_sprites` /
    /// `update_kfa_poses`.
    pub fn kfas_mut(&mut self) -> &mut [KfaSprite] {
        &mut self.kfas
    }

    /// World pose of bone `i` from the last solve: its pivot position and
    /// orthonormal basis `[s, h, f]`. `None` if out of range. Used to drag
    /// a bone in the viewport (the pivot gives the drag plane; the parent's
    /// basis maps a world delta into the hinge's local velcro space).
    #[must_use]
    pub fn limb_pose(&self, i: usize) -> Option<([f32; 3], [[f32; 3]; 3])> {
        let sprite = self.kfas.first()?.limbs.get(i)?;
        Some((sprite.p, [sprite.s, sprite.h, sprite.f]))
    }

    /// A camera framed on the rig — orbits the root, far enough out to hold
    /// the largest bone mesh.
    #[must_use]
    pub fn framing_camera(&self) -> OrbitCamera {
        let extent = self
            .rig
            .bones
            .iter()
            .map(|b| {
                let (x, y, z) = b.model.dims();
                x.max(y).max(z)
            })
            .max()
            .unwrap_or(1);
        let r = self.rig.root;
        let center = DVec3::new(f64::from(r[0]), f64::from(r[1]), f64::from(r[2]));
        OrbitCamera::framing(center, f64::from(extent) * 3.0)
    }

    /// The playhead position (ms) of the baked clip. `0` with no sprite.
    #[must_use]
    pub fn time(&self) -> i32 {
        self.kfas.first().map_or(0, |k| k.kfatim)
    }

    /// Seek the playhead to `ms` (clamped to `≥ 0`). The pose updates on the
    /// next [`Self::advance`] (which re-resolves from `kfatim`); pass `0` as
    /// the delta there to re-pose in place without advancing time.
    pub fn set_time(&mut self, ms: i32) {
        if let Some(k) = self.kfas.first_mut() {
            k.kfatim = ms.max(0);
        }
    }

    /// The clip's loop length (ms): the last sequence entry's timestamp (the
    /// `!target` loop marker). `0` when there is no animation.
    #[must_use]
    pub fn duration(&self) -> i32 {
        self.kfas
            .first()
            .and_then(|k| k.seq.iter().map(|s| s.tim).max())
            .unwrap_or(0)
    }

    /// Timestamps (ms) of every sequence entry — the keyframe ticks for the
    /// timeline. Empty when there is no animation.
    #[must_use]
    pub fn seq_times(&self) -> Vec<i32> {
        self.kfas
            .first()
            .map(|k| k.seq.iter().map(|s| s.tim).collect())
            .unwrap_or_default()
    }

    /// The pose currently displayed: the per-bone hinge angles
    /// (`KfaSprite::kfaval`) resolved at the playhead by the last
    /// [`Self::advance`]. This is what "key the current pose" snapshots into a
    /// new keyframe — the values the viewport is showing, not the rest pose.
    /// Empty if there is no sprite.
    #[must_use]
    pub fn pose_angles(&self) -> Vec<i16> {
        self.kfas
            .first()
            .map(|k| k.kfaval.clone())
            .unwrap_or_default()
    }

    /// Advance the baked animation by `dt_ms` and re-solve bone transforms,
    /// so [`Self::bone_lines`] reads the current pose.
    pub fn advance(&mut self, dt_ms: i32) {
        for k in &mut self.kfas {
            k.animsprite(dt_ms);
            solve_kfa_limbs(k);
        }
    }

    /// Skeleton gizmo: a segment from each non-root bone's pivot to its
    /// parent's pivot (reads the already-solved limb transforms). Drawn
    /// always-on-top so the skeleton stays visible through the meshes. The
    /// bone at `active` (if any) is drawn highlighted (cyan + thicker) so the
    /// selection / posing target is visible in the viewport.
    #[must_use]
    #[allow(clippy::cast_sign_loss)] // parent >= 0 is checked before the cast
    pub fn bone_lines(&self, active: Option<usize>) -> Vec<Line3> {
        let mut lines = Vec::new();
        for k in &self.kfas {
            for (i, bone) in self.rig.bones.iter().enumerate() {
                let parent = bone.hinge.parent;
                if parent < 0 {
                    continue;
                }
                let a = k.limbs[i].p;
                let b = k.limbs[parent as usize].p;
                let hot = active == Some(i);
                lines.push(Line3 {
                    a: [f64::from(a[0]), f64::from(a[1]), f64::from(a[2])],
                    b: [f64::from(b[0]), f64::from(b[1]), f64::from(b[2])],
                    color: if hot { ACTIVE_BONE_COLOR } else { BONE_COLOR },
                    width_px: if hot { 3.5 } else { 2.0 },
                    depth_test: false,
                });
            }
        }
        lines
    }
}

/// A synthetic two-bone rig (a body with a swinging arm) built from demiurg
/// voxel models. Temporary seed until rig authoring lands.
#[must_use]
pub fn demo_rig() -> Rig {
    let body = box_model(6, 4, 16, 0x8033_cc55); // green
    let arm = box_model(4, 3, 10, 0x80cc_4433); // red

    let zero = Point3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let z_axis = Point3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    let shoulder = Point3 {
        x: 6.0,
        y: 0.0,
        z: 0.0,
    }; // body-side velcro, +x of body centre

    Rig {
        name: "demo".to_string(),
        root: [0.0, 0.0, 0.0],
        bones: vec![
            RigBone {
                name: "body".to_string(),
                model: body,
                hinge: Hinge {
                    parent: -1,
                    p: [zero, zero],
                    v: [z_axis, z_axis],
                    vmin: 0,
                    vmax: 0,
                    htype: 0,
                    filler: [0; 7],
                },
            },
            RigBone {
                name: "arm".to_string(),
                model: arm,
                hinge: Hinge {
                    parent: 0,
                    p: [zero, shoulder],
                    v: [z_axis, z_axis],
                    vmin: i16::MIN, // free hinge
                    vmax: i16::MAX,
                    htype: 0,
                    filler: [0; 7],
                },
            },
        ],
        clips: vec![Clip {
            name: "swing".to_string(),
            data: ClipData::Skeletal {
                frmval: vec![vec![0, 0], vec![0, 16000], vec![0, 0], vec![0, -16000]],
                seq: vec![
                    Seq { tim: 0, frm: 0 },
                    Seq { tim: 500, frm: 1 },
                    Seq { tim: 1000, frm: 2 },
                    Seq { tim: 1500, frm: 3 },
                    Seq { tim: 2000, frm: !0 }, // loop back to frame 0
                ],
            },
        }],
    }
}

/// The synthetic [`demo_rig`] serialized as `.rkc` bytes — a sample rig for
/// testing the load path (see `DEMIURG_KFA_DUMP`).
#[must_use]
pub fn demo_rkc_bytes() -> Vec<u8> {
    demo_rig().to_rkc_bytes()
}

/// A solid box of `col`, pivot at its centre (so the sprite places it
/// centred on the bone root).
#[allow(clippy::cast_precision_loss)] // box dims are tiny
fn box_model(x: u32, y: u32, z: u32, col: u32) -> VoxelModel {
    let mut m = VoxelModel::new(x, y, z);
    for zz in 0..z {
        for yy in 0..y {
            for xx in 0..x {
                m.set(xx, yy, zz, col);
            }
        }
    }
    m.pivot = [x as f32 / 2.0, y as f32 / 2.0, z as f32 / 2.0];
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_parses_a_serialized_rig() {
        let view = KfaView::load(&demo_rkc_bytes()).expect("loads a valid .rkc");
        assert_eq!(view.rig.bones.len(), 2, "body + arm");
        assert_eq!(view.kfas.len(), 1, "one assembled sprite");
    }

    #[test]
    fn load_rejects_garbage() {
        assert!(KfaView::load(b"not an rkc file").is_err());
    }

    #[test]
    fn timeline_reads_the_baked_clip() {
        let mut view = KfaView::from_rig(demo_rig(), Some(0));
        assert_eq!(view.duration(), 2000, "loop length = last seq tim");
        assert_eq!(view.seq_times(), vec![0, 500, 1000, 1500, 2000]);

        // Seek, then re-pose in place (dt == 0): the playhead holds at 750.
        view.set_time(750);
        view.advance(0);
        assert_eq!(view.time(), 750);

        // set_time clamps below zero.
        view.set_time(-100);
        assert_eq!(view.time(), 0);
    }

    #[test]
    fn timeline_is_empty_for_the_rest_pose() {
        let view = KfaView::from_rig(demo_rig(), None);
        assert_eq!(view.duration(), 0);
        assert!(view.seq_times().is_empty());
    }

    #[test]
    fn empty_mesh_rotator_chain_bakes_solves_and_round_trips() {
        // A 3-axis joint is a chain of zero-length, empty-mesh "rotator" bones
        // (one per principal axis) carrying a visible leaf. Verify the format
        // handles empty (zero-voxel) meshes through bake -> solve -> .rkc.
        let axis = |x: f32, y: f32, z: f32| Point3 { x, y, z };
        let zero = axis(0.0, 0.0, 0.0);
        let hinge = |parent: i32, v: Point3| Hinge {
            parent,
            p: [zero, zero], // zero-length: child pivot == parent joint
            v: [v, v],
            vmin: i16::MIN,
            vmax: i16::MAX,
            htype: 0,
            filler: [0; 7],
        };
        let rotator = |name: &str, parent: i32, v: Point3| RigBone {
            name: name.to_string(),
            model: VoxelModel::new(1, 1, 1), // empty: zero voxels -> invisible
            hinge: hinge(parent, v),
        };
        let rig = Rig {
            name: "joint".to_string(),
            root: [0.0; 3],
            bones: vec![
                rotator("root", -1, axis(0.0, 0.0, 1.0)),
                rotator("rotX", 0, axis(1.0, 0.0, 0.0)),
                rotator("rotY", 1, axis(0.0, 1.0, 0.0)),
                RigBone {
                    name: "leaf".to_string(),
                    model: box_model(3, 3, 8, 0x80ff_ffff),
                    hinge: hinge(2, axis(0.0, 0.0, 1.0)),
                },
            ],
            clips: vec![Clip {
                name: "c".to_string(),
                data: ClipData::Skeletal {
                    // ~44 deg on each rotator axis.
                    frmval: vec![vec![0, 8000, 8000, 8000]],
                    seq: vec![Seq { tim: 0, frm: 0 }, Seq { tim: 500, frm: !0 }],
                },
            }],
        };
        // Round-trips through .rkc with empty meshes (zero-voxel kv6).
        let back = Rig::from_rkc_bytes(&rig.to_rkc_bytes()).expect("empty meshes round-trip");
        assert_eq!(back.bones.len(), 4);
        // Bakes + solves without panic; the leaf gets a finite pose.
        let mut view = KfaView::from_rig(rig, Some(0));
        view.advance(0);
        let (p, basis) = view.limb_pose(3).expect("leaf is posed");
        assert!(p.iter().all(|c| c.is_finite()), "leaf pivot finite: {p:?}");
        assert!(
            basis.iter().flatten().all(|c| c.is_finite()),
            "leaf basis finite (empty rotators didn't break the solve)"
        );
    }

    #[test]
    fn pose_angles_read_the_resolved_pose_at_the_playhead() {
        let mut view = KfaView::from_rig(demo_rig(), Some(0));
        // Seek to t=500 (demo frame 1) and re-pose in place; the arm hinge
        // (bone 1) should resolve to that frame's value, the root (bone 0)
        // stays untouched at 0.
        view.set_time(500);
        view.advance(0);
        assert_eq!(view.pose_angles(), vec![0, 16000]);
    }
}
