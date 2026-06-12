//! The model viewport: turns a [`VoxelModel`] document into the roxlap
//! scene + sprite set the renderer draws, and frames an [`OrbitCamera`]
//! on it.
//!
//! The model is compiled to a `.kv6` (via [`VoxelModel::to_kv6`], the
//! same surface extraction the engine uses) and drawn as a single
//! sprite instance placed with its pivot at the world origin. The scene
//! is otherwise empty — a pure model preview — so the only thing on
//! screen is the model exactly as the game would render it.

mod camera;

pub use camera::OrbitCamera;

use demiurg_core::VoxelModel;
use glam::DVec3;
use roxlap_render::{Sprite, SpriteInstanceDesc, SpriteSet};
use roxlap_scene::Scene;

/// Sprite pivot world position. Kept at the origin; the camera orbits
/// here so the model's pivot is the turntable axis.
const ORIGIN: [f32; 3] = [0.0, 0.0, 0.0];

/// A previewable model: the roxlap scene + sprite set, plus framing
/// metadata for the camera.
pub struct ModelView {
    scene: Scene,
    sprites: SpriteSet,
    /// Largest model dimension in voxels — the camera frames to it.
    extent: f64,
}

impl ModelView {
    /// Build a viewport for `model`.
    #[must_use]
    pub fn new(model: &VoxelModel) -> Self {
        let mut view = Self {
            scene: Scene::new(),
            sprites: SpriteSet {
                models: Vec::new(),
                instances: Vec::new(),
                carve_model: None,
            },
            extent: 1.0,
        };
        view.set_model(model);
        view
    }

    /// Recompile the sprite from `model` (call after edits; M1 only ever
    /// loads, but the editor will lean on this every frame).
    pub fn set_model(&mut self, model: &VoxelModel) {
        let (xsiz, ysiz, zsiz) = model.dims();
        self.extent = f64::from(xsiz.max(ysiz).max(zsiz)).max(1.0);

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

    /// The sprite set to hand to `SceneRenderer::set_sprites`.
    #[must_use]
    pub fn sprites(&self) -> &SpriteSet {
        &self.sprites
    }

    /// The (empty) scene to hand to `SceneRenderer::render`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_one_model_one_instance() {
        let mut m = VoxelModel::new(4, 4, 4);
        m.set(1, 1, 1, 0x80ff_ffff);
        let view = ModelView::new(&m);
        assert_eq!(view.sprites().models.len(), 1);
        assert_eq!(view.sprites().instances.len(), 1);
        assert_eq!(view.sprites().instances[0].model, 0);
    }

    #[test]
    fn framing_camera_basis_is_orthonormal_and_eye_behind_center() {
        let mut m = VoxelModel::new(8, 8, 8);
        m.set(0, 0, 0, 0x80ff_ffff);
        let cam = ModelView::new(&m).framing_camera().to_roxlap();

        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let len = |a: [f64; 3]| dot(a, a).sqrt();

        assert!((len(cam.right) - 1.0).abs() < 1e-9, "right is unit");
        assert!((len(cam.down) - 1.0).abs() < 1e-9, "down is unit");
        assert!((len(cam.forward) - 1.0).abs() < 1e-9, "forward is unit");
        assert!(
            dot(cam.right, cam.forward).abs() < 1e-9,
            "right _|_ forward"
        );
        // Centre is the origin, so the eye sits along -forward: pos points
        // opposite the view direction.
        assert!(dot(cam.pos, cam.forward) < 0.0, "eye is behind the model");
    }
}
