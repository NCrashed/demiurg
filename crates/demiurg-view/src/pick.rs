//! Voxel picking: turn a world-space ray (from
//! `SceneRenderer::view_ray`) into the voxel under the cursor.
//!
//! The engine's own `pick` reads the scene-grid z-buffer and is
//! transparent to sprites, so a model viewer needs its own ray-march.
//! The model is drawn as one sprite at the world origin with an identity
//! basis, so world↔voxel is a pure translation by the pivot
//! (`voxel = world + pivot`); we confirm this matches the renderer by
//! the model sitting centred on the orbit point. From there it is a
//! standard Amanatides–Woo DDA over the dense grid.

use demiurg_core::VoxelModel;
use glam::DVec3;

/// A resolved voxel pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickHit {
    /// The solid voxel the ray hit — the paint / erase target.
    pub voxel: [u32; 3],
    /// The face the ray entered through, pointing back toward the ray.
    pub normal: [i32; 3],
    /// The empty cell against that face (`voxel + normal`) — the place
    /// target. May be out of bounds (place would be a no-op there).
    pub place: [i32; 3],
}

/// March `origin + t·dir` (world space) through `model` and return the
/// first solid voxel, or `None` if the ray misses all geometry.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)] // grid coords are small and guarded into range before every cast
pub fn pick_voxel(model: &VoxelModel, origin: DVec3, dir: DVec3) -> Option<PickHit> {
    let (nx, ny, nz) = model.dims();
    if nx == 0 || ny == 0 || nz == 0 {
        return None;
    }
    let idim = [i64::from(nx), i64::from(ny), i64::from(nz)];
    let fdim = [f64::from(nx), f64::from(ny), f64::from(nz)];
    let pivot = [
        f64::from(model.pivot[0]),
        f64::from(model.pivot[1]),
        f64::from(model.pivot[2]),
    ];

    // World -> voxel space (sprite at origin, identity basis).
    let o = [
        origin.x + pivot[0],
        origin.y + pivot[1],
        origin.z + pivot[2],
    ];
    let d = [dir.x, dir.y, dir.z];

    let (t_enter, t_exit, enter_axis) = ray_box(o, d, fdim)?;
    if t_exit < 0.0 {
        return None;
    }
    let t0 = t_enter.max(0.0);

    let mut cell = [0i64; 3];
    for a in 0..3 {
        let p = o[a] + d[a] * t0;
        cell[a] = (p.floor() as i64).clamp(0, idim[a] - 1);
    }

    let mut step = [0i64; 3];
    let mut t_max = [f64::INFINITY; 3];
    let mut t_delta = [f64::INFINITY; 3];
    for a in 0..3 {
        if d[a] > 0.0 {
            step[a] = 1;
            t_max[a] = (cell[a] as f64 + 1.0 - o[a]) / d[a];
            t_delta[a] = 1.0 / d[a];
        } else if d[a] < 0.0 {
            step[a] = -1;
            t_max[a] = (cell[a] as f64 - o[a]) / d[a];
            t_delta[a] = -1.0 / d[a];
        }
    }

    // Entry-face normal points opposite the ray's travel on that axis.
    let mut normal = [0i32; 3];
    normal[enter_axis] = if d[enter_axis] >= 0.0 { -1 } else { 1 };

    loop {
        if model.get(cell[0] as u32, cell[1] as u32, cell[2] as u32) != 0 {
            let voxel = [cell[0] as u32, cell[1] as u32, cell[2] as u32];
            let place = [
                cell[0] as i32 + normal[0],
                cell[1] as i32 + normal[1],
                cell[2] as i32 + normal[2],
            ];
            return Some(PickHit {
                voxel,
                normal,
                place,
            });
        }

        // Advance into the neighbouring cell with the nearest crossing.
        let axis = if t_max[0] < t_max[1] {
            if t_max[0] < t_max[2] { 0 } else { 2 }
        } else if t_max[1] < t_max[2] {
            1
        } else {
            2
        };
        cell[axis] += step[axis];
        if cell[axis] < 0 || cell[axis] >= idim[axis] {
            return None;
        }
        normal = [0, 0, 0];
        normal[axis] = -step[axis] as i32;
        t_max[axis] += t_delta[axis];
    }
}

/// Slab-method ray vs `[0, dims]` box. Returns `(t_enter, t_exit,
/// enter_axis)` or `None` if the ray misses.
fn ray_box(o: [f64; 3], d: [f64; 3], dims: [f64; 3]) -> Option<(f64, f64, usize)> {
    let mut t_enter = f64::NEG_INFINITY;
    let mut t_exit = f64::INFINITY;
    let mut enter_axis = 0usize;
    for a in 0..3 {
        if d[a].abs() < 1e-12 {
            // Parallel to this slab: must already be inside it.
            if o[a] < 0.0 || o[a] > dims[a] {
                return None;
            }
        } else {
            let inv = 1.0 / d[a];
            let mut t1 = (0.0 - o[a]) * inv;
            let mut t2 = (dims[a] - o[a]) * inv;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            if t1 > t_enter {
                t_enter = t1;
                enter_axis = a;
            }
            if t2 < t_exit {
                t_exit = t2;
            }
            if t_enter > t_exit {
                return None;
            }
        }
    }
    Some((t_enter, t_exit, enter_axis))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_with_center_voxel() -> VoxelModel {
        // 8³, pivot [4,4,4]; voxel (4,4,4) has world centre ~ (0.5,0.5,0.5).
        let mut m = VoxelModel::new(8, 8, 8);
        m.set(4, 4, 4, 0x80ff_0000);
        m
    }

    #[test]
    fn hits_voxel_marching_plus_z() {
        let m = model_with_center_voxel();
        let hit = pick_voxel(&m, DVec3::new(0.5, 0.5, -10.0), DVec3::new(0.0, 0.0, 1.0))
            .expect("ray should hit");
        assert_eq!(hit.voxel, [4, 4, 4]);
        assert_eq!(hit.normal, [0, 0, -1], "entered from the -z face");
        assert_eq!(
            hit.place,
            [4, 4, 3],
            "place target is the empty -z neighbour"
        );
    }

    #[test]
    fn hits_voxel_marching_minus_x() {
        let m = model_with_center_voxel();
        let hit = pick_voxel(&m, DVec3::new(10.0, 0.5, 0.5), DVec3::new(-1.0, 0.0, 0.0))
            .expect("ray should hit");
        assert_eq!(hit.voxel, [4, 4, 4]);
        assert_eq!(hit.normal, [1, 0, 0], "entered from the +x face");
        assert_eq!(hit.place, [5, 4, 4]);
    }

    #[test]
    fn ray_beside_geometry_misses() {
        let m = model_with_center_voxel();
        assert!(pick_voxel(&m, DVec3::new(40.0, 40.0, -10.0), DVec3::new(0.0, 0.0, 1.0)).is_none());
    }

    #[test]
    fn empty_model_misses() {
        let m = VoxelModel::new(8, 8, 8);
        assert!(pick_voxel(&m, DVec3::new(0.0, 0.0, -10.0), DVec3::new(0.0, 0.0, 1.0)).is_none());
    }
}
