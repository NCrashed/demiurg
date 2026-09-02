//! KFA (skeletal) rig preview — the editor's animation view.
//!
//! Owns an editable [`Rig`] (the live document) and the [`KfaSprite`]s built
//! from it, advances its baked clip, and emits skeleton gizmo lines. The
//! host (`demiurg-app`) hands the sprites to
//! `SceneRenderer::{set_kfa_sprites, update_kfa_poses}` each frame.
//!
//! Until rig authoring exists, [`demo_rig`] seeds a synthetic two-bone rig.

use demiurg_core::{Easing, Rig, RigBone, VoxelModel};
use glam::DVec3;
use roxlap_core::kfa_draw::solve_kfa_limbs;
use roxlap_formats::OverlayColor;
use roxlap_formats::character::{Clip, ClipData};
use roxlap_formats::kfa::{Hinge, KfaSprite, Point3, Seq};
use roxlap_formats::sprite::Sprite;
use roxlap_formats::xform::BoneXform;

use crate::{Line3, OrbitCamera};

/// Colour of the skeleton gizmo (always-on-top yellow, like the hover box).
const BONE_COLOR: OverlayColor = OverlayColor(0xffff_e600);
/// Colour of the active (selected) bone in the gizmo — bright cyan, thicker,
/// so the bone being posed stands out from the yellow skeleton.
const ACTIVE_BONE_COLOR: OverlayColor = OverlayColor(0xff00_e5ff);

/// A previewable KFA rig: the editable source [`Rig`] plus the live
/// [`KfaSprite`]s built from it.
pub struct KfaView {
    rig: Rig,
    kfas: Vec<KfaSprite>,
    /// The previewed clip index (for per-clip easing); `None` = rest pose.
    clip: Option<usize>,
}

impl KfaView {
    /// Build a view from `rig`, baking in `clip` (a `Skeletal` clip index,
    /// or `None` for the rest pose).
    #[must_use]
    pub fn from_rig(rig: Rig, clip: Option<usize>) -> Self {
        // The KFA limb path draws exactly one mesh per bone — the first *static*
        // attachment `to_kfa_sprite` finds. For a bone whose primary is a clip,
        // that would be the first static *extra*, drawn (wrongly) at the bone
        // origin and again (correctly) by the host's compose pass — a double
        // draw. Build the limb sprites from an extras-stripped rig so each limb
        // is just the primary (a mesh, or an empty limb for a clip primary);
        // extras + clip layers are drawn solely by the compose pass. Stripping
        // extras doesn't affect the solve (it reads hinges, not meshes).
        let mut skel = rig.clone();
        for b in &mut skel.bones {
            b.extras.clear();
        }
        let mut kfas = vec![skel.to_character().to_kfa_sprite(clip)];
        // Hand every limb the rig's colour→material map. The renderers
        // classify a sprite's voxels by its own map, so setting it here is all
        // it takes for a translucent bone to composite down both paths — the
        // host's KFA pass and the headless shot — instead of one of them
        // quietly drawing it solid.
        let map = material_map_of(&rig);
        if !map.is_empty() {
            for k in &mut kfas {
                for limb in &mut k.limbs {
                    limb.material_map.clone_from(&map);
                }
            }
        }
        Self { rig, kfas, clip }
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

    /// The rig's `(id, material)` palette to install via
    /// `SceneRenderer::define_material`. Empty for an all-opaque rig.
    #[must_use]
    pub fn material_defs(&self) -> demiurg_core::MaterialDefs {
        self.rig.material_palette().0
    }

    /// The rig's `0xRRGGBB`→material-id map, for
    /// `SceneRenderer::add_sprite_model_with_materials`. Empty for an
    /// all-opaque rig.
    #[must_use]
    pub fn material_map(&self) -> demiurg_core::MaterialColorMap {
        self.rig.material_palette().1
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

    /// The pose currently displayed: the per-bone local transforms
    /// (`KfaSprite::kfaval`) resolved at the playhead by the last
    /// [`Self::advance`]. This is what "key the current pose" snapshots into a
    /// new keyframe — the values the viewport is showing, not the rest pose.
    /// Empty if there is no sprite.
    #[must_use]
    pub fn pose_xforms(&self) -> Vec<BoneXform> {
        self.kfas
            .first()
            .map(|k| k.kfaval.clone())
            .unwrap_or_default()
    }

    /// Advance the baked animation by `dt_ms` and re-solve bone transforms,
    /// so [`Self::bone_lines`] reads the current pose. The engine interpolates
    /// linearly; when the clip carries a non-linear [`Easing`], the linear pose
    /// is overridden with the eased one before solving.
    pub fn advance(&mut self, dt_ms: i32) {
        // `animsprite` advances the playhead (and writes a linear pose).
        for k in &mut self.kfas {
            k.animsprite(dt_ms);
        }
        if let Some(ci) = self.clip {
            let easing = self.rig.clip_easing(ci);
            if easing != Easing::Linear {
                let t = self.kfas.first().map(|k| k.kfatim);
                if let Some(t) = t {
                    let pose = eased_pose(&self.rig, ci, t, easing);
                    if !pose.is_empty() {
                        if let Some(k) = self.kfas.first_mut() {
                            k.kfaval = pose;
                        }
                    }
                }
            }
        }
        for k in &mut self.kfas {
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

    /// Headless CPU render of the **posed rig** to a packed `0x00RRGGBB`
    /// framebuffer (row-major, `width x height`) — the rig counterpart of
    /// [`ModelView::render_cpu`](crate::ModelView::render_cpu), for offscreen
    /// screenshots with no window.
    ///
    /// Re-poses at the current playhead first (see [`Self::set_time`]), then
    /// draws every solved limb, so the result is the skeleton as posed — not
    /// one bone's mesh. `flip_x` mirrors the result to match the viewport's
    /// "Flip X" correction.
    ///
    /// A window-bound `SceneRenderer` isn't available here, so this draws the
    /// limb sprites directly. That means the limb path only: one mesh per bone
    /// (its primary), and no gizmo lines, terrain grid, or extra attachment
    /// layers — those are composed by the host's render pass.
    #[must_use]
    pub fn render_cpu(
        &mut self,
        camera: &OrbitCamera,
        width: u32,
        height: u32,
        sky_color: u32,
        flip_x: bool,
        anginc: f32,
    ) -> Vec<u32> {
        use roxlap_core::OpticastSettings;
        use roxlap_core::camera_math;
        use roxlap_core::dda_sprite::{SpriteShade, draw_sprite_dda_shaded};
        use roxlap_formats::Rgb;
        use roxlap_formats::material::MaterialTable;

        // Resolve the pose at the playhead without moving it, and solve every
        // limb's world transform from it.
        self.advance(0);

        let mut settings = OpticastSettings::for_oracle_framebuffer(width, height);
        settings.anginc = anginc.max(0.05);
        // The sprite path takes the derived per-frame camera, built from the
        // same pinhole the terrain raycaster uses, so both agree on depth.
        let cam = camera_math::derive(
            &camera.to_roxlap(),
            width,
            height,
            settings.hx,
            settings.hy,
            settings.hz,
        );

        let pixels = (width as usize) * (height as usize);
        let mut fb = vec![sky_color; pixels];
        let mut zb = vec![f32::INFINITY; pixels];

        // The rig's translucent colours, as the renderer wants them: a palette
        // of ids and a colour→id map each sprite classifies its voxels by. An
        // all-opaque rig produces neither and takes the plain path.
        let (defs, color_map) = self.rig.material_palette();
        let mut table = MaterialTable::new();
        for &(id, mat) in &defs {
            table.set(id, mat);
        }
        let material_map: Vec<(Rgb, u8)> = color_map.iter().map(|&(c, id)| (Rgb(c), id)).collect();

        let draw = |fb: &mut Vec<u32>, zb: &mut Vec<f32>, sprite: &Sprite| {
            let shade = (!defs.is_empty()).then(|| SpriteShade {
                materials: &table,
                material: 0,
                alpha_mul: 255,
                tint: 0x00FF_FFFF,
                lights: roxlap_core::CpuLights::default(),
                // A `--shot` renders unlit and casts no shadows, like the
                // model path it sits beside.
                shadow: None,
            });
            let _ = draw_sprite_dda_shaded(
                fb,
                zb,
                width as usize,
                width,
                height,
                &cam,
                &settings,
                sprite,
                shade,
            );
        };
        let time = self.time();
        for k in &self.kfas {
            for limb in &k.limbs {
                draw(&mut fb, &mut zb, limb);
            }
            // A bone whose geometry deforms carries a flipbook instead of a
            // mesh, and its limb sprite is deliberately empty — the frames are
            // composed on top, at the same solved transform. Without this a
            // slime renders as nothing at all.
            for (i, bone) in self.rig.bones.iter().enumerate() {
                let (Some(clip), Some(limb)) = (&bone.primary_clip, k.limbs.get(i)) else {
                    continue;
                };
                let frame = clip.frame_at_playback(bone.primary_playback, time);
                let Some(cf) = clip.frames.get(frame) else {
                    continue;
                };
                let mut sprite = Sprite::axis_aligned(cf.model.to_kv6(), limb.p);
                sprite.s = limb.s;
                sprite.h = limb.h;
                sprite.f = limb.f;
                sprite.material_map.clone_from(&material_map);
                draw(&mut fb, &mut zb, &sprite);
            }
        }

        if flip_x {
            for row in fb.chunks_mut(width as usize) {
                row.reverse();
            }
        }
        fb
    }
}

/// The rig's colour→material map in the renderer's packing, or empty when
/// nothing is translucent.
fn material_map_of(rig: &Rig) -> Vec<(roxlap_formats::Rgb, u8)> {
    rig.material_palette()
        .1
        .iter()
        .map(|&(c, id)| (roxlap_formats::Rgb(c), id))
        .collect()
}

/// Resolve clip `clip`'s pose at time `t` with `easing` applied to the active
/// segment's blend parameter — the eased counterpart of the engine's linear
/// interpolation. Empty when the clip has fewer than two keys (nothing to
/// interpolate; the single-key pose is left as `animsprite` set it).
#[allow(clippy::cast_precision_loss)]
fn eased_pose(rig: &Rig, clip: usize, t: i32, easing: Easing) -> Vec<BoneXform> {
    let keys = rig.clip_keyframes(clip);
    if keys.len() < 2 {
        return Vec::new();
    }
    let loop_len = rig.clip_loop_tim(clip).max(1);
    let t = t.rem_euclid(loop_len);
    // The last key with `tim <= t` starts the active segment.
    let mut i = 0;
    for (j, k) in keys.iter().enumerate() {
        if k.tim <= t {
            i = j;
        } else {
            break;
        }
    }
    // Segment end key + span; the final segment wraps back to key 0 at loop_len.
    let (end, span) = if i + 1 < keys.len() {
        (&keys[i + 1], keys[i + 1].tim - keys[i].tim)
    } else {
        (&keys[0], loop_len - keys[i].tim)
    };
    let u = if span > 0 {
        (t - keys[i].tim) as f32 / span as f32
    } else {
        0.0
    };
    let u = easing.apply(u);
    keys[i]
        .xforms
        .iter()
        .zip(&end.xforms)
        .map(|(a, b)| a.blend(*b, u))
        .collect()
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
            RigBone::mesh(
                "body".to_string(),
                body,
                Hinge {
                    parent: -1,
                    p: [zero, zero],
                    v: [z_axis, z_axis],
                    vmin: 0,
                    vmax: 0,
                    htype: 0,
                    filler: [0; 7],
                },
                Vec::new(),
            ),
            RigBone::mesh(
                "arm".to_string(),
                arm,
                Hinge {
                    parent: 0,
                    p: [zero, shoulder],
                    v: [z_axis, z_axis],
                    vmin: i16::MIN, // free hinge
                    vmax: i16::MAX,
                    htype: 0,
                    filler: [0; 7],
                },
                Vec::new(),
            ),
        ],
        clips: vec![Clip {
            name: "swing".to_string(),
            data: ClipData::Skeletal {
                // The arm (bone 1) swings about +z; the body (root) stays put.
                frmval: [0i16, 16000, 0, -16000]
                    .iter()
                    .map(|&a| {
                        let z = [0.0, 0.0, 1.0];
                        vec![BoneXform::IDENTITY, BoneXform::from_hinge_angle(z, a)]
                    })
                    .collect(),
                seq: vec![
                    Seq { tim: 0, frm: 0 },
                    Seq { tim: 500, frm: 1 },
                    Seq { tim: 1000, frm: 2 },
                    Seq { tim: 1500, frm: 3 },
                    Seq { tim: 2000, frm: !0 }, // loop back to frame 0
                ],
            },
        }],
        clip_easing: Vec::new(),
        materials: std::collections::BTreeMap::new(),
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
        let rotator = |name: &str, parent: i32, v: Point3| {
            RigBone::mesh(
                name.to_string(),
                VoxelModel::new(1, 1, 1), // empty: zero voxels -> invisible
                hinge(parent, v),
                Vec::new(),
            )
        };
        let rig = Rig {
            name: "joint".to_string(),
            root: [0.0; 3],
            bones: vec![
                rotator("root", -1, axis(0.0, 0.0, 1.0)),
                rotator("rotX", 0, axis(1.0, 0.0, 0.0)),
                rotator("rotY", 1, axis(0.0, 1.0, 0.0)),
                RigBone::mesh(
                    "leaf".to_string(),
                    box_model(3, 3, 8, 0x80ff_ffff),
                    hinge(2, axis(0.0, 0.0, 1.0)),
                    Vec::new(),
                ),
            ],
            clips: vec![Clip {
                name: "c".to_string(),
                data: ClipData::Skeletal {
                    // ~44 deg on each rotator's own axis (X, Y, Z).
                    frmval: vec![vec![
                        BoneXform::IDENTITY,
                        BoneXform::from_hinge_angle([1.0, 0.0, 0.0], 8000),
                        BoneXform::from_hinge_angle([0.0, 1.0, 0.0], 8000),
                        BoneXform::from_hinge_angle([0.0, 0.0, 1.0], 8000),
                    ]],
                    seq: vec![Seq { tim: 0, frm: 0 }, Seq { tim: 500, frm: !0 }],
                },
            }],
            clip_easing: Vec::new(),
            materials: std::collections::BTreeMap::new(),
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
    fn rig_round_trips_through_a_demiurg_project() {
        // A rigged document saved as a `.demiurg` project keeps the whole rig —
        // skeleton, meshes, and animation clips — not just a bare model.
        let rig = demo_rig();
        let bytes = demiurg_core::project::to_bytes_rig(&rig);
        match demiurg_core::project::from_bytes(&bytes).expect("decodes") {
            demiurg_core::project::Loaded::Rig(back) => {
                assert_eq!(back.bones.len(), rig.bones.len(), "bones survive");
                assert_eq!(back.clips.len(), rig.clips.len(), "clips survive");
                assert_eq!(back.clip_keyframes(0).len(), rig.clip_keyframes(0).len());
            }
            demiurg_core::project::Loaded::Model(_) | demiurg_core::project::Loaded::Clip(_) => {
                panic!("expected a rig")
            }
        }
    }

    #[test]
    fn pose_xforms_read_the_resolved_pose_at_the_playhead() {
        let mut view = KfaView::from_rig(demo_rig(), Some(0));
        // Seek to t=500 (demo frame 1) and re-pose in place; the arm hinge
        // (bone 1) should resolve to that frame's value (16000 about +z), the
        // root (bone 0) stays at identity.
        view.set_time(500);
        view.advance(0);
        let z = [0.0, 0.0, 1.0];
        let angles: Vec<i16> = view
            .pose_xforms()
            .iter()
            .map(|x| x.hinge_angle(z))
            .collect();
        assert_eq!(angles[0], 0, "root untouched");
        assert!((i32::from(angles[1]) - 16000).abs() <= 1, "arm at frame 1");
    }

    /// Count pixels whose dominant channel is red / green. The demo rig's
    /// bones are a green body and a red arm, so this says which limbs drew —
    /// robust to the per-face shading exact colours aren't.
    fn limb_pixels(fb: &[u32]) -> (usize, usize) {
        let mut red = 0;
        let mut green = 0;
        for px in fb {
            let (r, g, b) = ((px >> 16) & 0xff, (px >> 8) & 0xff, px & 0xff);
            if r > g && r > b {
                red += 1;
            } else if g > r && g > b {
                green += 1;
            }
        }
        (red, green)
    }

    #[test]
    fn the_headless_render_draws_every_limb_posed() {
        // The bug this guards: `--shot` used to render the active bone's mesh
        // alone, so an exported rig looked like one lonely body part.
        let mut view = KfaView::from_rig(demo_rig(), Some(0));
        let cam = view.framing_camera();
        let sky = 0x0020_3040;
        let fb = view.render_cpu(&cam, 200, 200, sky, false, 1.0);

        assert_eq!(fb.len(), 200 * 200);
        let (red, green) = limb_pixels(&fb);
        assert!(green > 0, "the body limb drew");
        assert!(red > 0, "the arm limb drew too, not just bone 0");
    }

    /// A rig whose second bone deforms: a flipbook of two very different
    /// frames instead of a rigid mesh.
    fn clip_rig() -> Rig {
        use demiurg_core::{ClipDoc, ClipFrame};

        let mut rig = Rig::single_bone("root", Some(box_model(4, 4, 4, 0x8033_cc55)));
        let idx = rig.add_bone(0);
        let mut clip = ClipDoc::new([6, 6, 6]);
        clip.default_frame_ms = 100;
        let frame = |fill: u32, span: u32| {
            let mut m = VoxelModel::new(6, 6, 6);
            for z in 0..span {
                for y in 0..span {
                    for x in 0..span {
                        m.set(x, y, z, fill);
                    }
                }
            }
            ClipFrame::new(m)
        };
        // Two frames of wildly different size, so a render that ignored the
        // playhead could not accidentally match.
        clip.frames = vec![frame(0x80cc_4433, 2), frame(0x80cc_4433, 6)];
        rig.bones[idx].primary_clip = Some(clip);
        rig
    }

    #[test]
    fn the_headless_render_draws_a_bone_that_deforms() {
        // A clip bone's limb sprite is deliberately empty — the frames are
        // composed on top. Miss that and a slime renders as nothing, which is
        // exactly what the shot tool exists to rule out.
        let mut view = KfaView::from_rig(clip_rig(), None);
        let cam = view.framing_camera();
        let sky = 0x0020_3040;
        let fb = view.render_cpu(&cam, 200, 200, sky, false, 1.0);
        let (red, green) = limb_pixels(&fb);
        assert!(green > 0, "the rigid bone drew");
        assert!(red > 0, "the deforming bone's current frame drew too");
    }

    #[test]
    fn a_deforming_bone_changes_shape_with_the_playhead() {
        let mut view = KfaView::from_rig(clip_rig(), None);
        let cam = view.framing_camera();
        let sky = 0x0020_3040;
        let shot = |view: &mut KfaView, t: i32| {
            view.set_time(t);
            let fb = view.render_cpu(&cam, 200, 200, sky, false, 1.0);
            let (red, _) = limb_pixels(&fb);
            red
        };
        // Frame 0 is a 2³ corner, frame 1 fills the 6³ grid: the second has to
        // cover far more of the screen.
        let small = shot(&mut view, 0);
        let large = shot(&mut view, 150);
        assert!(
            large > small * 2,
            "the flipbook must advance with the playhead: {small} then {large} pixels"
        );
    }

    #[test]
    fn a_translucent_rig_composites_in_the_headless_render() {
        use roxlap_formats::material::Material;

        // A slab the camera looks through, so what is behind it changes the
        // pixels. The rig path had no materials at all until the `DMAT` chunk,
        // and a file that carries transparency nothing renders is worse than
        // one that doesn't carry it.
        let mut rig = Rig::single_bone("slab", Some(box_model(8, 8, 2, 0x80ff_0000)));
        let sky = 0x0020_3040;
        let render = |rig: &Rig| {
            let mut view = KfaView::from_rig(rig.clone(), None);
            let cam = view.framing_camera();
            view.render_cpu(&cam, 140, 140, sky, false, 1.0)
        };

        let opaque = render(&rig);
        rig.materials.insert(0x80ff_0000, Material::alpha_blend(48));
        let glass = render(&rig);
        assert_ne!(opaque, glass, "the rig's materials must reach the render");

        // And it is the material doing it, not the map merely being present:
        // back to opaque and the pixels come back.
        rig.materials
            .insert(0x80ff_0000, Material::alpha_blend(255));
        assert_ne!(render(&rig), glass, "a different alpha renders differently");
    }

    #[test]
    fn materials_reach_every_limb_sprite() {
        use roxlap_formats::material::Material;

        // The renderers classify a sprite's voxels by the sprite's own map, so
        // a limb without one draws solid however good the palette is.
        let mut rig = Rig::single_bone("slab", Some(box_model(4, 4, 4, 0x80ff_0000)));
        rig.materials.insert(0x80ff_0000, Material::alpha_blend(64));
        let view = KfaView::from_rig(rig, None);
        assert!(!view.material_defs().is_empty(), "the palette is built");
        for limb in &view.kfas[0].limbs {
            assert!(!limb.material_map.is_empty(), "every limb carries the map");
        }
    }

    #[test]
    fn the_headless_render_follows_the_playhead() {
        let mut view = KfaView::from_rig(demo_rig(), Some(0));
        let cam = view.framing_camera();
        let sky = 0x0020_3040;
        let shot = |view: &mut KfaView, t: i32| {
            view.set_time(t);
            view.render_cpu(&cam, 160, 160, sky, false, 1.0)
        };
        // The demo clip swings the arm between frame 0 and frame 1, so the two
        // framebuffers must differ — that is the whole point of `--time`.
        let a = shot(&mut view, 0);
        let b = shot(&mut view, 500);
        assert_ne!(a, b, "the pose at t=500 must not match the pose at t=0");
        // ...and seeking back reproduces the first frame exactly (the render
        // re-poses from the playhead rather than accumulating).
        assert_eq!(a, shot(&mut view, 0));
    }
}
