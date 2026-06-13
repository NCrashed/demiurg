//! The model viewport: turns a [`VoxelModel`] document into the roxlap
//! scene the renderer draws, and frames an [`OrbitCamera`] on it.
//!
//! Two render representations, switchable per [`RenderMode`]:
//!
//! - [`RenderMode::Sprite`] — the model compiled to a `.kv6`
//!   ([`VoxelModel::to_kv6`]) and drawn as one sprite at the world
//!   origin. This is how monada draws pieces (WYSIWYG).
//! - [`RenderMode::Voxel`] — the model packed into a one-chunk voxel
//!   grid (a `Vxl`) and rendered via the scene path, which applies
//!   voxlap's per-face `side_shades` (top faces shade differently from
//!   sides) — easier to read while editing.
//!
//! Both place the model so a voxel `(x, y, z)` sits at world
//! `(x, y, z) − pivot`, matching [`pick_voxel`] / [`voxel_screen_edges`],
//! so picking and the hover box line up in either mode.

mod camera;
mod pick;

pub use camera::OrbitCamera;
pub use pick::{PickHit, pick_voxel, project_to_screen, voxel_screen_edges};

use demiurg_core::VoxelModel;
use glam::{DVec3, IVec3};
use roxlap_render::{Sprite, SpriteInstanceDesc, SpriteSet};
use roxlap_scene::{GridTransform, Scene};

/// Sprite pivot world position. Kept at the origin; the camera orbits
/// here so the model's pivot is the turntable axis.
const ORIGIN: [f32; 3] = [0.0, 0.0, 0.0];

/// How the editor draws the model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderMode {
    /// One kv6 sprite — matches the in-game render.
    Sprite,
    /// A voxel grid — gets per-face side shading for easier editing.
    Voxel,
}

/// A previewable model: the roxlap scene + sprite set, plus framing
/// metadata for the camera.
pub struct ModelView {
    scene: Scene,
    sprites: SpriteSet,
    /// Largest model dimension in voxels — the camera frames to it.
    extent: f64,
}

impl ModelView {
    /// Build a viewport for `model` in `mode`.
    #[must_use]
    pub fn new(model: &VoxelModel, mode: RenderMode) -> Self {
        let mut view = Self {
            scene: Scene::new(),
            sprites: empty_sprite_set(),
            extent: 1.0,
        };
        view.set_model(model, mode);
        view
    }

    /// Rebuild the scene from `model` for `mode` (called after edits, a
    /// load, or a mode switch).
    pub fn set_model(&mut self, model: &VoxelModel, mode: RenderMode) {
        let (xsiz, ysiz, zsiz) = model.dims();
        self.extent = f64::from(xsiz.max(ysiz).max(zsiz)).max(1.0);

        match mode {
            RenderMode::Sprite => {
                self.scene = Scene::new();
                let sprite = Sprite::axis_aligned(model.to_kv6(), ORIGIN);
                self.sprites = SpriteSet {
                    models: vec![sprite],
                    instances: vec![SpriteInstanceDesc {
                        model: 0,
                        pos: ORIGIN,
                    }],
                    carve_model: None,
                };
            }
            RenderMode::Voxel => {
                self.sprites = empty_sprite_set();
                self.scene = build_voxel_scene(model);
            }
        }
    }

    /// The sprite set to hand to `SceneRenderer::set_sprites`.
    #[must_use]
    pub fn sprites(&self) -> &SpriteSet {
        &self.sprites
    }

    /// The scene to hand to `SceneRenderer::render`.
    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }

    /// An orbit camera framed on the model: far enough out that the
    /// whole model sits inside the renderer's ~90° horizontal FOV.
    #[must_use]
    pub fn framing_camera(&self) -> OrbitCamera {
        OrbitCamera::framing(DVec3::from_array([0.0, 0.0, 0.0]), self.extent * 1.6)
    }
}

fn empty_sprite_set() -> SpriteSet {
    SpriteSet {
        models: Vec::new(),
        instances: Vec::new(),
        carve_model: None,
    }
}

/// Pack `model` into a single-chunk voxel grid placed at `-pivot`, so a
/// voxel `(x, y, z)` lands at world `(x, y, z) − pivot` (matching the
/// sprite path and the picker). Models larger than one chunk
/// (`CHUNK_SIZE_XY` / `CHUNK_SIZE_Z`) are clipped to it.
#[allow(clippy::cast_possible_wrap)] // voxel coords are small, well within i32
fn build_voxel_scene(model: &VoxelModel) -> Scene {
    let p = model.pivot;
    let mut scene = Scene::new();
    let id = scene.add_grid(GridTransform::at(DVec3::new(
        -f64::from(p[0]),
        -f64::from(p[1]),
        -f64::from(p[2]),
    )));
    let grid = scene.grid_mut(id).expect("grid just added");
    let chunk = grid.ensure_chunk(IVec3::ZERO);
    for (x, y, z, col) in model.occupied() {
        roxlap_formats::edit::set_cube(chunk, x as i32, y as i32, z as i32, Some(col));
    }
    scene
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_mode_builds_one_instance() {
        let mut m = VoxelModel::new(4, 4, 4);
        m.set(1, 1, 1, 0x80ff_ffff);
        let view = ModelView::new(&m, RenderMode::Sprite);
        assert_eq!(view.sprites().models.len(), 1);
        assert_eq!(view.sprites().instances.len(), 1);
    }

    #[test]
    fn voxel_mode_builds_a_grid_and_no_sprites() {
        let mut m = VoxelModel::new(4, 4, 4);
        m.set(1, 1, 1, 0x80ff_ffff);
        let mut view = ModelView::new(&m, RenderMode::Voxel);
        assert!(
            view.sprites().instances.is_empty(),
            "no sprites in voxel mode"
        );
        assert_eq!(view.scene_mut().grid_count(), 1, "one grid built");
    }

    #[test]
    fn framing_camera_basis_is_orthonormal_and_eye_behind_center() {
        let mut m = VoxelModel::new(8, 8, 8);
        m.set(0, 0, 0, 0x80ff_ffff);
        let cam = ModelView::new(&m, RenderMode::Sprite)
            .framing_camera()
            .to_roxlap();

        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let len = |a: [f64; 3]| dot(a, a).sqrt();

        assert!((len(cam.right) - 1.0).abs() < 1e-9, "right is unit");
        assert!((len(cam.down) - 1.0).abs() < 1e-9, "down is unit");
        assert!((len(cam.forward) - 1.0).abs() < 1e-9, "forward is unit");
        assert!(
            dot(cam.right, cam.forward).abs() < 1e-9,
            "right _|_ forward"
        );
        assert!(dot(cam.pos, cam.forward) < 0.0, "eye is behind the model");
    }
}
