//! KFA (skeletal) rig preview — the first slice of the animation editor.
//!
//! Builds a renderable [`KfaSprite`] from a roxlap
//! [`Character`](roxlap_formats::character::Character) (the on-disk rigged-
//! character container), advances its baked clip, and emits skeleton gizmo
//! lines. The host (`demiurg-app`) hands the sprites to
//! `SceneRenderer::{set_kfa_sprites, update_kfa_poses}` each frame.
//!
//! Until rig authoring exists, [`demo_character`] seeds a synthetic two-bone
//! rig (round-tripped through `character::serialize`/`parse` to exercise the
//! whole engine path).

use demiurg_core::VoxelModel;
use roxlap_core::kfa_draw::solve_kfa_limbs;
use roxlap_formats::character::{self, Bone, Character, Clip, ClipData, MeshRef};
use roxlap_formats::kfa::{Hinge, KfaSprite, Point3, Seq};

use crate::Line3;

/// Colour of the skeleton gizmo (always-on-top yellow, like the hover box).
const BONE_COLOR: u32 = 0xffff_e600;

/// A previewable KFA rig: the source [`Character`] plus the live
/// [`KfaSprite`]s built from it.
pub struct KfaView {
    character: Character,
    kfas: Vec<KfaSprite>,
}

impl KfaView {
    /// Build a view from `character`, baking in `clip` (a `Skeletal` clip
    /// index, or `None` for the rest pose).
    #[must_use]
    pub fn from_character(character: Character, clip: Option<usize>) -> Self {
        let kfas = vec![character.to_kfa_sprite(clip)];
        Self { character, kfas }
    }

    /// The sprites to hand to `SceneRenderer::set_kfa_sprites` /
    /// `update_kfa_poses`.
    pub fn kfas_mut(&mut self) -> &mut [KfaSprite] {
        &mut self.kfas
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
    /// always-on-top so the skeleton stays visible through the meshes.
    #[must_use]
    #[allow(clippy::cast_sign_loss)] // parent >= 0 is checked before the cast
    pub fn bone_lines(&self) -> Vec<Line3> {
        let mut lines = Vec::new();
        for k in &self.kfas {
            for (i, bone) in self.character.bones.iter().enumerate() {
                let parent = bone.hinge.parent;
                if parent < 0 {
                    continue;
                }
                let a = k.limbs[i].p;
                let b = k.limbs[parent as usize].p;
                lines.push(Line3 {
                    a: [f64::from(a[0]), f64::from(a[1]), f64::from(a[2])],
                    b: [f64::from(b[0]), f64::from(b[1]), f64::from(b[2])],
                    color: BONE_COLOR,
                    width_px: 2.0,
                    depth_test: false,
                });
            }
        }
        lines
    }
}

/// A synthetic two-bone rig (a body with a swinging arm) built from demiurg
/// voxel models and **round-tripped through the engine container** — proving
/// `VoxelModel` → `Kv6` → `Character` → `serialize`/`parse` → `to_kfa_sprite`
/// end to end. Temporary seed until rig authoring lands.
///
/// # Panics
/// If the synthetic character fails to round-trip (a bug in the container).
#[must_use]
pub fn demo_character() -> Character {
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

    let character = Character {
        name: "demo".to_string(),
        root: [0.0, 0.0, 0.0],
        meshes: vec![body.to_kv6(), arm.to_kv6()],
        bones: vec![
            Bone {
                name: "body".to_string(),
                mesh: MeshRef::Static(0),
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
            Bone {
                name: "arm".to_string(),
                mesh: MeshRef::Static(1),
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
        extra_chunks: Vec::new(),
    };

    // Round-trip through the container so the demo also exercises the format.
    let bytes = character::serialize(&character);
    character::parse(&bytes).expect("demo character round-trips through the container")
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
