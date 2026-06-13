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
use roxlap_core::Camera;

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

/// Project a world point to framebuffer pixels (physical), or `None` if
/// it is at/behind the camera plane.
///
/// Exact inverse of roxlap's `setcamera` pixel ray
/// (`dir = (x-hx)·right + (y-hy)·down + hz·forward`, with
/// `hx = hy = w/2`, `hz = w/2` from `OpticastSettings::for_oracle_
/// framebuffer`): for a point at relative offset `rel = P − cam.pos`,
/// `x = hx + hz·(rel·right)/(rel·forward)` and likewise for `y`. The
/// basis is orthonormal so the dot products read off the components.
#[must_use]
pub fn project_to_screen(
    camera: &Camera,
    width: f64,
    height: f64,
    world: [f64; 3],
) -> Option<(f64, f64)> {
    let rel = [
        world[0] - camera.pos[0],
        world[1] - camera.pos[1],
        world[2] - camera.pos[2],
    ];
    let dot = |a: [f64; 3]| a[0] * rel[0] + a[1] * rel[1] + a[2] * rel[2];
    let f = dot(camera.forward);
    if f <= 1e-6 {
        return None;
    }
    let (hx, hy, hz) = (width * 0.5, height * 0.5, width * 0.5);
    Some((
        hx + hz * dot(camera.right) / f,
        hy + hz * dot(camera.down) / f,
    ))
}

/// The 12 edges of voxel `cell` projected to framebuffer pixels, as
/// `[start, end]` segments. Edges with an endpoint behind the camera are
/// dropped. Uses the same world↔voxel mapping as [`pick_voxel`]
/// (`world = voxel − pivot`, sprite at the origin), so the wire box lines
/// up exactly with the rendered voxel.
#[must_use]
#[allow(clippy::cast_sign_loss)] // i,j,k are 0/1 so the corner index is non-negative
pub fn voxel_screen_edges(
    camera: &Camera,
    width: f64,
    height: f64,
    pivot: [f32; 3],
    cell: [i32; 3],
) -> Vec<[(f64, f64); 2]> {
    const EDGES: [(usize, usize); 12] = [
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

    let pv = [
        f64::from(pivot[0]),
        f64::from(pivot[1]),
        f64::from(pivot[2]),
    ];
    // Corner c = (i, j, k) with index i + 2j + 4k; world = (cell + c) - pivot.
    let mut pts: [Option<(f64, f64)>; 8] = [None; 8];
    for k in 0..2i32 {
        for j in 0..2i32 {
            for i in 0..2i32 {
                let world = [
                    f64::from(cell[0] + i) - pv[0],
                    f64::from(cell[1] + j) - pv[1],
                    f64::from(cell[2] + k) - pv[2],
                ];
                pts[(i + 2 * j + 4 * k) as usize] = project_to_screen(camera, width, height, world);
            }
        }
    }

    let mut out = Vec::with_capacity(12);
    for (a, b) in EDGES {
        if let (Some(pa), Some(pb)) = (pts[a], pts[b]) {
            out.push([pa, pb]);
        }
    }
    out
}

/// Reference overlay geometry, in framebuffer pixels: the volume
/// bounding box, a floor grid on the origin plane, and the X/Y/Z axes
/// from the `(0,0,0)` corner. Segments with an endpoint behind the
/// camera are dropped. Uses the same world↔voxel mapping as
/// [`pick_voxel`], so it lines up with the model.
/// A projected segment plus its view-space depth (distance along the
/// camera forward axis), so callers can fade segments behind the model.
pub type DepthSeg = ([(f64, f64); 2], f64);

pub struct ReferenceLines {
    /// The 12 edges of the `[0, dims]` volume box.
    pub box_edges: Vec<DepthSeg>,
    /// Per-voxel grid lines on the floor (max-z face; z is down).
    pub floor_grid: Vec<DepthSeg>,
    /// X, Y, Z axes from the origin corner (caller colours them).
    pub axes: [Option<DepthSeg>; 3],
    /// View-space depth of the model centre — segments deeper than this
    /// are behind the model.
    pub center_depth: f64,
}

/// Build the [`ReferenceLines`] for a model of `dims` with `pivot`.
#[must_use]
pub fn reference_lines(
    camera: &Camera,
    width: f64,
    height: f64,
    pivot: [f32; 3],
    dims: (u32, u32, u32),
) -> ReferenceLines {
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (0, 2),
        (1, 3),
        (4, 6),
        (5, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    let pv = [
        f64::from(pivot[0]),
        f64::from(pivot[1]),
        f64::from(pivot[2]),
    ];
    let (dx, dy, dz) = (f64::from(dims.0), f64::from(dims.1), f64::from(dims.2));
    // voxel-space point -> world (= voxel - pivot).
    let world = |p: [f64; 3]| [p[0] - pv[0], p[1] - pv[1], p[2] - pv[2]];
    let project = |p: [f64; 3]| project_to_screen(camera, width, height, world(p));
    // View-space depth: distance of a voxel-space point along forward.
    let depth = |p: [f64; 3]| {
        let w = world(p);
        (w[0] - camera.pos[0]) * camera.forward[0]
            + (w[1] - camera.pos[1]) * camera.forward[1]
            + (w[2] - camera.pos[2]) * camera.forward[2]
    };
    let seg = |a: [f64; 3], b: [f64; 3]| match (project(a), project(b)) {
        (Some(pa), Some(pb)) => Some(([pa, pb], 0.5 * (depth(a) + depth(b)))),
        _ => None,
    };
    let center_depth = depth([dx * 0.5, dy * 0.5, dz * 0.5]);

    let corners = [
        [0.0, 0.0, 0.0],
        [dx, 0.0, 0.0],
        [0.0, dy, 0.0],
        [dx, dy, 0.0],
        [0.0, 0.0, dz],
        [dx, 0.0, dz],
        [0.0, dy, dz],
        [dx, dy, dz],
    ];
    let box_edges = EDGES
        .iter()
        .filter_map(|&(a, b)| seg(corners[a], corners[b]))
        .collect();

    // Floor: the max-z face (z is down in the voxlap world, so this is
    // the bottom of the volume).
    let mut floor_grid = Vec::new();
    for x in 0..=dims.0 {
        if let Some(s) = seg([f64::from(x), 0.0, dz], [f64::from(x), dy, dz]) {
            floor_grid.push(s);
        }
    }
    for y in 0..=dims.1 {
        if let Some(s) = seg([0.0, f64::from(y), dz], [dx, f64::from(y), dz]) {
            floor_grid.push(s);
        }
    }

    let axes = [
        seg([0.0, 0.0, 0.0], [dx, 0.0, 0.0]),
        seg([0.0, 0.0, 0.0], [0.0, dy, 0.0]),
        seg([0.0, 0.0, 0.0], [0.0, 0.0, dz]),
    ];

    ReferenceLines {
        box_edges,
        floor_grid,
        axes,
        center_depth,
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

    #[test]
    #[allow(clippy::float_cmp)] // exact-centre projection is an exact value
    fn projection_centres_a_forward_point_and_drops_behind() {
        // Camera at origin looking +y; right=+x, down=+z.
        let cam = Camera {
            pos: [0.0, 0.0, 0.0],
            right: [1.0, 0.0, 0.0],
            down: [0.0, 0.0, 1.0],
            forward: [0.0, 1.0, 0.0],
        };
        let (x, y) = project_to_screen(&cam, 100.0, 80.0, [0.0, 10.0, 0.0]).unwrap();
        assert!((x - 50.0).abs() < 1e-9, "hx centre"); // width/2
        assert!((y - 40.0).abs() < 1e-9, "hy centre"); // height/2

        let (xr, _) = project_to_screen(&cam, 100.0, 80.0, [2.0, 10.0, 0.0]).unwrap();
        assert!(xr > 50.0, "a +x point lands right of centre");

        assert!(
            project_to_screen(&cam, 100.0, 80.0, [0.0, -10.0, 0.0]).is_none(),
            "behind the camera projects to None"
        );
    }

    #[test]
    fn voxel_edges_visible_when_in_front() {
        let cam = Camera {
            pos: [0.0, -20.0, 0.0],
            right: [1.0, 0.0, 0.0],
            down: [0.0, 0.0, 1.0],
            forward: [0.0, 1.0, 0.0],
        };
        let edges = voxel_screen_edges(&cam, 100.0, 100.0, [4.0, 4.0, 4.0], [4, 4, 4]);
        assert_eq!(edges.len(), 12, "all 12 edges in front of the camera");
    }
}
