//! Bake a rig's **skeletal** animation into a voxel-flipbook clip — "render"
//! the posed rig to voxels frame by frame so a `.kfa`-style animation becomes a
//! standalone [`ClipDoc`] (`.rvc`), cheap to play at runtime without a rig.
//!
//! At each sampled time the rig is posed (the same solve the viewport uses);
//! every bone attachment — a static mesh or an animated clip layer — has its
//! voxels mapped to world space via the exact convention the renderer uses
//! (`compose_attachment` for the world basis + pivot, then the
//! `dda_sprite` voxel→world map `world = pos + (v - pivot)·[s,h,f]`), so the
//! bake matches the preview. The world voxels across all frames define one
//! fixed bounding box; each frame is splatted (nearest-cell) into it.

use demiurg_core::{ClipDoc, ClipFrame, KeyXform, LayerPlayback, LoopMode, Rig, VoxelModel};
use roxlap_core::kfa_draw::compose_attachment;

use crate::KfaView;

/// Per-axis cap on a baked clip's dimensions — a posed rig can sweep a large
/// volume; refuse rather than allocate something absurd.
const MAX_BAKE_DIM: u32 = 256;

/// One attachment's posed world transform: world basis columns `(s, h, f)` and
/// world pivot `pos`, as produced by [`compose_attachment`].
type PosedXform = ([f32; 3], [f32; 3], [f32; 3], [f32; 3]);

/// Bake the rig's skeletal clip `clip_index` into a [`ClipDoc`] of `frame_count`
/// frames (uniformly sampled over the clip's length).
///
/// # Errors
/// A message if the rig has no clip / the clip is empty of voxels, or if the
/// posed content would exceed [`MAX_BAKE_DIM`] on some axis.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
pub fn bake_clip(rig: &Rig, clip_index: usize, frame_count: u32) -> Result<ClipDoc, String> {
    if clip_index >= rig.clips.len() {
        return Err("the rig has no such clip to bake".into());
    }
    let n = frame_count.max(1);
    let mut view = KfaView::from_rig(rig.clone(), Some(clip_index));
    let dur = view.duration().max(0); // ms; 0 for a static (poseless) clip

    // Pass 1: pose each sample, map every attachment's voxels to world, and
    // track the global bounding box. World voxels are kept per frame for pass 2.
    let mut frames_world: Vec<Vec<([f32; 3], u32)>> = Vec::with_capacity(n as usize);
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut any = false;

    let sample_count = if dur > 0 { n } else { 1 }; // static clip ⇒ a single frame
    for i in 0..sample_count {
        let t = if dur > 0 {
            (i64::from(i) * i64::from(dur) / i64::from(n)) as i32
        } else {
            0
        };
        view.set_time(t);
        view.advance(0); // re-pose in place at the playhead
        let rig_t = view.time();

        let mut voxels = Vec::new();
        for (bi, bone) in rig.bones.iter().enumerate() {
            let Some((bp, [bs, bh, bf])) = view.limb_pose(bi) else {
                continue;
            };
            // Primary attachment (identity offset): a clip layer or the mesh.
            let prim = compose_attachment(bs, bh, bf, bp, &KeyXform::IDENTITY);
            let prim_model = clip_or_mesh(
                &bone.model,
                bone.primary_clip.as_ref(),
                bone.primary_playback,
                rig_t,
            );
            splat(prim_model, prim, &mut voxels, &mut lo, &mut hi, &mut any);
            // Extras, each at its own offset.
            for ex in &bone.extras {
                let xf = compose_attachment(bs, bh, bf, bp, &ex.offset);
                let m = clip_or_mesh(&ex.model, ex.clip.as_ref(), ex.playback, rig_t);
                splat(m, xf, &mut voxels, &mut lo, &mut hi, &mut any);
            }
        }
        frames_world.push(voxels);
    }

    if !any {
        return Err("nothing to bake — the rig has no voxels in this clip".into());
    }

    // Fixed bounding box for the whole clip: cells span floor(lo)..=floor(hi).
    let base = [lo[0].floor(), lo[1].floor(), lo[2].floor()];
    let dims = [
        ((hi[0] - base[0]).floor() as u32) + 1,
        ((hi[1] - base[1]).floor() as u32) + 1,
        ((hi[2] - base[2]).floor() as u32) + 1,
    ];
    if dims.iter().any(|&d| d > MAX_BAKE_DIM) {
        return Err(format!(
            "baked clip is too large ({}×{}×{}, max {MAX_BAKE_DIM}); reduce the rig or its scale",
            dims[0], dims[1], dims[2]
        ));
    }

    // The clip rotates about the rig root, expressed in the grid's voxel frame.
    let pivot = [
        rig.root[0] - base[0],
        rig.root[1] - base[1],
        rig.root[2] - base[2],
    ];

    // Pass 2: splat each frame's world voxels into a dense model.
    let frames: Vec<ClipFrame> = frames_world
        .iter()
        .map(|fw| {
            let mut m = VoxelModel::new(dims[0], dims[1], dims[2]);
            m.pivot = pivot;
            for &(w, col) in fw {
                let cx = (w[0] - base[0]).floor() as i32;
                let cy = (w[1] - base[1]).floor() as i32;
                let cz = (w[2] - base[2]).floor() as i32;
                if cx >= 0 && cy >= 0 && cz >= 0 {
                    m.set(cx as u32, cy as u32, cz as u32, col);
                }
            }
            ClipFrame::new(m)
        })
        .collect();

    let default_frame_ms = if dur > 0 { (dur as u32 / n).max(1) } else { 80 };
    let mut clip = ClipDoc::new(dims);
    clip.name = format!("{} baked", rig.name);
    clip.pivot = pivot;
    clip.default_frame_ms = default_frame_ms;
    clip.loop_mode = LoopMode::Loop; // skeletal clips loop
    clip.frames = frames;
    Ok(clip)
}

/// The model to draw for an attachment at rig time `rig_t`: a clip layer's
/// current frame, or the static mesh.
fn clip_or_mesh<'a>(
    mesh: &'a VoxelModel,
    clip: Option<&'a ClipDoc>,
    playback: LayerPlayback,
    rig_t: i32,
) -> &'a VoxelModel {
    let Some(clip) = clip else {
        return mesh;
    };
    // Advance the layer's clip by the rig playhead × its playback (Q8 speed + ms
    // phase) — the same mapping the live preview uses.
    let elapsed = (i64::from(rig_t) * i64::from(playback.speed_q8) / 256
        + i64::from(playback.start_phase_ms))
    .max(0);
    let frame = clip.frame_at(u32::try_from(elapsed).unwrap_or(u32::MAX));
    clip.frames.get(frame).map_or(mesh, |f| &f.model)
}

/// Map every occupied voxel of `model` to world via the posed transform and
/// push `(world, colour)`, updating the running bounding box.
#[allow(clippy::cast_precision_loss, clippy::many_single_char_names)]
fn splat(
    model: &VoxelModel,
    (s, h, f, pos): PosedXform,
    out: &mut Vec<([f32; 3], u32)>,
    lo: &mut [f32; 3],
    hi: &mut [f32; 3],
    any: &mut bool,
) {
    let pv = model.pivot;
    for (vx, vy, vz, col) in model.occupied() {
        // Voxel centre in the model's voxel frame, relative to its pivot.
        let l = [
            vx as f32 + 0.5 - pv[0],
            vy as f32 + 0.5 - pv[1],
            vz as f32 + 0.5 - pv[2],
        ];
        let w = [
            pos[0] + l[0] * s[0] + l[1] * h[0] + l[2] * f[0],
            pos[1] + l[0] * s[1] + l[1] * h[1] + l[2] * f[1],
            pos[2] + l[0] * s[2] + l[1] * h[2] + l[2] * f[2],
        ];
        for k in 0..3 {
            lo[k] = lo[k].min(w[k]);
            hi[k] = hi[k].max(w[k]);
        }
        out.push((w, col));
        *any = true;
    }
}

#[cfg(test)]
mod tests {
    use super::bake_clip;
    use crate::demo_rig;

    #[test]
    fn bakes_a_rig_clip_into_animated_frames() {
        // `demo_rig` is a two-bone arm with a "swing" skeletal clip.
        let rig = demo_rig();
        let clip = bake_clip(&rig, 0, 8).expect("bakes");
        assert_eq!(clip.frame_count(), 8);
        // Every frame holds the posed rig's voxels…
        assert!(clip.frames.iter().all(|f| f.model.occupied_count() > 0));
        // …and the arm swings, so consecutive frames are not all identical.
        assert!(
            clip.frames.windows(2).any(|w| w[0].model != w[1].model),
            "baked frames should differ as the rig animates"
        );
    }

    #[test]
    fn rejects_an_out_of_range_clip() {
        assert!(bake_clip(&demo_rig(), 99, 4).is_err());
    }
}
