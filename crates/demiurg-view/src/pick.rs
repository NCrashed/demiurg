//! Voxel picking and the editor's reference/hover line geometry.
//!
//! Picking ([`pick_voxel`]) ray-marches the dense model directly (the
//! engine's own pick reads the scene-grid z-buffer and is transparent to
//! sprites). The model is placed so a voxel `(x, y, z)` sits at world
//! `(x, y, z) − pivot`, matching the renderer.
//!
//! The reference grid / box / axes ([`reference_lines_3d`]) and the hover
//! wire box ([`voxel_box_lines_3d`]) are returned as **world-space**
//! [`Line3`]s for `SceneRenderer::draw_lines`, which projects and
//! depth-tests them against the rendered frame — so the model occludes
//! lines behind it. No screen projection here anymore.

use demiurg_core::VoxelModel;
use glam::DVec3;
use roxlap_render::Line3;

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

// ---- Reference / hover line geometry (world space, for draw_lines) ----

/// Axis gizmo colours (`0xAARRGGBB`): X red, Y green, Z blue. Shared with
/// the editor's tool panel so an artist maps panel axes to the viewport.
pub const AXIS_COLORS: [u32; 3] = [0xffe0_5a5a, 0xff6e_c86e, 0xff6e_96eb];

/// Corner indices of a unit/box wireframe: index `i = x | y<<1 | z<<2`.
const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7), // along x
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7), // along y
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7), // along z
];

/// The 12 edges of the world-space box `[lo, hi]` as `Line3`s.
fn box_lines(
    lo: [f64; 3],
    hi: [f64; 3],
    color: u32,
    width_px: f32,
    depth_test: bool,
) -> Vec<Line3> {
    let corner = |i: usize| {
        [
            if i & 1 == 0 { lo[0] } else { hi[0] },
            if i & 2 == 0 { lo[1] } else { hi[1] },
            if i & 4 == 0 { lo[2] } else { hi[2] },
        ]
    };
    BOX_EDGES
        .iter()
        .map(|&(a, b)| Line3 {
            a: corner(a),
            b: corner(b),
            color,
            width_px,
            depth_test,
        })
        .collect()
}

/// Wire box around voxel `cell` (always-on-top yellow), so the targeted
/// voxel is visible even when it would be occluded.
#[must_use]
pub fn voxel_box_lines_3d(pivot: [f32; 3], cell: [i32; 3]) -> Vec<Line3> {
    let pv = [
        f64::from(pivot[0]),
        f64::from(pivot[1]),
        f64::from(pivot[2]),
    ];
    let lo = [
        f64::from(cell[0]) - pv[0],
        f64::from(cell[1]) - pv[1],
        f64::from(cell[2]) - pv[2],
    ];
    let hi = [lo[0] + 1.0, lo[1] + 1.0, lo[2] + 1.0];
    box_lines(lo, hi, 0xffff_e600, 1.5, false)
}

/// The reference overlay as world-space `Line3`s: the volume bounding box,
/// a per-voxel floor grid (max-z face), and X/Y/Z origin axes. All
/// depth-tested, so the model occludes the parts behind it.
#[must_use]
pub fn reference_lines_3d(pivot: [f32; 3], dims: (u32, u32, u32)) -> Vec<Line3> {
    let pv = [
        f64::from(pivot[0]),
        f64::from(pivot[1]),
        f64::from(pivot[2]),
    ];
    let (dx, dy, dz) = (f64::from(dims.0), f64::from(dims.1), f64::from(dims.2));
    // voxel-space -> world.
    let w = |p: [f64; 3]| [p[0] - pv[0], p[1] - pv[1], p[2] - pv[2]];
    let line = |a: [f64; 3], b: [f64; 3], color: u32, width: f32| Line3 {
        a: w(a),
        b: w(b),
        color,
        width_px: width,
        depth_test: true,
    };

    let mut lines = box_lines(w([0.0, 0.0, 0.0]), w([dx, dy, dz]), 0xc0c8_cdde, 1.0, true);

    // Floor grid on the max-z face (z is down → this is the bottom).
    for x in 0..=dims.0 {
        lines.push(line(
            [f64::from(x), 0.0, dz],
            [f64::from(x), dy, dz],
            0x70a0_a8b8,
            1.0,
        ));
    }
    for y in 0..=dims.1 {
        lines.push(line(
            [0.0, f64::from(y), dz],
            [dx, f64::from(y), dz],
            0x70a0_a8b8,
            1.0,
        ));
    }

    // Origin axes (X red / Y green / Z blue), matching `AXIS_COLORS`.
    lines.push(line([0.0; 3], [dx, 0.0, 0.0], AXIS_COLORS[0], 1.5));
    lines.push(line([0.0; 3], [0.0, dy, 0.0], AXIS_COLORS[1], 1.5));
    lines.push(line([0.0; 3], [0.0, 0.0, dz], AXIS_COLORS[2], 1.5));

    lines
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

    #[test]
    fn reference_and_hover_line_counts() {
        // box (12) + floor grid (dx+1 + dy+1) + 3 axes.
        let refs = reference_lines_3d([4.0, 4.0, 4.0], (8, 8, 8));
        assert_eq!(refs.len(), 12 + (9 + 9) + 3);
        assert_eq!(voxel_box_lines_3d([4.0, 4.0, 4.0], [1, 2, 3]).len(), 12);
        assert!(
            voxel_box_lines_3d([0.0; 3], [0, 0, 0])
                .iter()
                .all(|l| !l.depth_test),
            "hover box is always-on-top"
        );
    }
}
