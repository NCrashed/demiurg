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
//! `(x, y, z) − pivot`, matching [`pick_voxel`] and the world-space
//! gizmo lines ([`voxel_box_lines_3d`]), so picking and the hover box
//! line up in either mode.

mod bake;
mod camera;
mod kfa;
mod pick;

pub use bake::bake_clip;
pub use camera::{OrbitCamera, ViewDir};
pub use kfa::{KfaView, demo_rig, demo_rkc_bytes};
pub use pick::{
    AXIS_COLORS, PickHit, marquee_voxels, pick_voxel, project_to_screen, reference_lines_3d,
    selection_lines_3d, voxel_box_lines_3d, voxel_edge_lines_3d,
};
pub use roxlap_render::Line3;

use demiurg_core::VoxelModel;
use glam::{DVec3, IVec3};
use roxlap_formats::kv6::Kv6;
use roxlap_formats::{Rgb, VoxColor};
use roxlap_render::{Material, Sprite, SpriteInstanceDesc, SpriteSet};
use roxlap_scene::{GridId, GridTransform, Scene};

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

/// A previewable model: a **persistent** roxlap scene (one grid, reused
/// across edits) + sprite set, plus framing metadata for the camera.
///
/// The scene is kept across frames on purpose: `SceneRenderer` caches the
/// uploaded ("resident") scene and only re-uploads chunks whose version
/// changed. Building a fresh `Scene` each edit would be silently ignored
/// (its new grid id isn't the one the renderer tracks), leaving the
/// preview stale. So edits mutate the one grid's chunk in place and bump
/// its version.
pub struct ModelView {
    scene: Scene,
    /// The single persistent grid (voxel mode populates its chunk;
    /// sprite mode leaves it empty).
    grid_id: GridId,
    sprites: SpriteSet,
    /// The model compiled to a `.kv6`, kept only in sprite mode **when the
    /// model has translucent materials** — then `sprites` is left empty and
    /// the host registers this kv6 via
    /// `SceneRenderer::add_sprite_model_with_materials` (the `set_sprites`
    /// path carries no material map). `None` for the opaque sprite path
    /// (drawn straight from `sprites`) and in voxel mode.
    sprite_kv6: Option<Kv6>,
    /// Renderer material palette derived from the model: `(id, material)`
    /// to install via `define_material`, plus a `0xRRGGBB`→id colour map for
    /// `set_terrain_materials` (voxel mode) / `add_sprite_model_with_materials`
    /// (sprite mode). Empty for an all-opaque model.
    material_defs: Vec<(u8, Material)>,
    material_map: Vec<(u32, u8)>,
    /// Largest model dimension in voxels — the camera frames to it.
    extent: f64,
}

impl ModelView {
    /// Build a viewport for `model` in `mode`.
    #[must_use]
    pub fn new(model: &VoxelModel, mode: RenderMode) -> Self {
        let mut scene = Scene::new();
        let grid_id = scene.add_grid(GridTransform::identity());
        let mut view = Self {
            scene,
            grid_id,
            sprites: empty_sprite_set(),
            sprite_kv6: None,
            material_defs: Vec::new(),
            material_map: Vec::new(),
            extent: 1.0,
        };
        view.set_model(model, mode);
        view
    }

    /// Refresh the scene from `model` for `mode` (after edits, a load, or
    /// a mode switch). Reuses the persistent grid — see the type docs.
    pub fn set_model(&mut self, model: &VoxelModel, mode: RenderMode) {
        let (xsiz, ysiz, zsiz) = model.dims();
        self.extent = f64::from(xsiz.max(ysiz).max(zsiz)).max(1.0);

        // Renderer material palette for this model (empty when all-opaque).
        let (defs, map) = model.material_palette();
        self.material_defs = defs;
        self.material_map = map;

        // Keep the grid aligned to -pivot so a voxel (x, y, z) sits at world
        // (x, y, z) - pivot, matching the picker, in both modes.
        let p = model.pivot;
        let neg = DVec3::new(-f64::from(p[0]), -f64::from(p[1]), -f64::from(p[2]));
        if let Some(grid) = self.scene.grid_mut(self.grid_id) {
            grid.transform = GridTransform::at(neg);
        }

        match mode {
            RenderMode::Sprite => {
                self.drop_grid_chunk();
                let kv6 = model.to_kv6();
                if self.material_map.is_empty() {
                    // Opaque: the plain `set_sprites` path, unchanged.
                    self.sprite_kv6 = None;
                    self.sprites = SpriteSet {
                        models: vec![Sprite::axis_aligned(kv6, ORIGIN)],
                        instances: vec![SpriteInstanceDesc {
                            model: 0,
                            pos: ORIGIN,
                        }],
                        carve_model: None,
                    };
                } else {
                    // Translucent: hand the kv6 to the host to register with
                    // its material map (`set_sprites` has no material param),
                    // and leave `sprites` empty so it isn't double-drawn opaque.
                    self.sprites = empty_sprite_set();
                    self.sprite_kv6 = Some(kv6);
                }
            }
            RenderMode::Voxel => {
                self.sprites = empty_sprite_set();
                self.sprite_kv6 = None;
                self.rebuild_grid_chunk(model);
            }
        }
    }

    /// Drop the grid's voxel chunk (sprite mode); `refresh_dirty` evicts
    /// it from the resident scene next frame.
    fn drop_grid_chunk(&mut self) {
        if let Some(grid) = self.scene.grid_mut(self.grid_id) {
            grid.chunks.remove(&IVec3::ZERO);
        }
    }

    /// Rebuild the grid's single chunk from `model` and bump its version
    /// so the renderer re-uploads it. Models larger than one chunk
    /// (`CHUNK_SIZE_XY` / `CHUNK_SIZE_Z`) are clipped to it.
    #[allow(clippy::cast_possible_wrap)] // voxel coords are small, well within i32
    fn rebuild_grid_chunk(&mut self, model: &VoxelModel) {
        let Some(grid) = self.scene.grid_mut(self.grid_id) else {
            return;
        };
        grid.chunks.remove(&IVec3::ZERO);
        let chunk = grid.ensure_chunk(IVec3::ZERO);
        for (x, y, z, col) in model.occupied() {
            roxlap_formats::edit::set_cube(
                chunk,
                x as i32,
                y as i32,
                z as i32,
                Some(VoxColor(col)),
            );
        }
        // `chunk_versions` survives the remove above, so this strictly
        // increases the version → `refresh_dirty` re-uploads the chunk.
        grid.bump_chunk_version(IVec3::ZERO);
    }

    /// The sprite set to hand to `SceneRenderer::set_sprites`.
    #[must_use]
    pub fn sprites(&self) -> &SpriteSet {
        &self.sprites
    }

    /// The renderer material palette `(id, material)` to install via
    /// `SceneRenderer::define_material`. Empty for an all-opaque model.
    #[must_use]
    pub fn material_defs(&self) -> &[(u8, Material)] {
        &self.material_defs
    }

    /// The `0xRRGGBB`→material-id colour map for `set_terrain_materials`
    /// (voxel mode) / `add_sprite_model_with_materials` (sprite mode). Empty
    /// for an all-opaque model.
    #[must_use]
    pub fn material_map(&self) -> &[(u32, u8)] {
        &self.material_map
    }

    /// Empty the previewed model — drop the grid's voxel chunk and clear the
    /// sprite set + material palette. Used when the clip preview is driven by
    /// roxlap's clip player instead of the per-frame model, so the editor's
    /// own grid/sprites don't draw on top of the played clip.
    pub fn clear_scene(&mut self) {
        self.drop_grid_chunk();
        self.sprites = empty_sprite_set();
        self.sprite_kv6 = None;
        self.material_defs.clear();
        self.material_map.clear();
    }

    /// In **sprite** mode with translucent materials, the model's compiled
    /// `.kv6` for the host to register via
    /// `SceneRenderer::add_sprite_model_with_materials` (paired with
    /// [`material_map`](Self::material_map)). `None` for the opaque sprite
    /// path (drawn from [`sprites`](Self::sprites)) and in voxel mode.
    #[must_use]
    pub fn sprite_kv6(&self) -> Option<&Kv6> {
        self.sprite_kv6.as_ref()
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

    /// Headless CPU render of the scene to a packed `0x00RRGGBB`
    /// framebuffer (row-major, `width x height`), for offscreen
    /// screenshots / oracle-style diagnostics with no window. This is
    /// the voxel-grid path only — sprites and editor gizmos are not
    /// drawn. It mirrors `roxlap_render`'s CPU `render` so a shot matches
    /// what the live viewport draws for the same camera + `side_shades`.
    ///
    /// `side_shades` is voxlap's `setsideshades` (pass `[0; 6]` to
    /// disable). `flip_x` mirrors the result horizontally to match the
    /// viewport's "Flip X" correction.
    #[must_use]
    // A headless-render facade that mirrors roxlap's CPU `render` inputs
    // (camera, dims, shades, sky, flip, ray density); bundling them into a
    // struct would just shuffle the same fields around.
    #[allow(clippy::too_many_arguments)]
    pub fn render_cpu(
        &mut self,
        camera: &OrbitCamera,
        width: u32,
        height: u32,
        side_shades: [i8; 6],
        sky_color: u32,
        flip_x: bool,
        anginc: f32,
    ) -> Vec<u32> {
        use roxlap_core::OpticastSettings;
        use roxlap_formats::material::MaterialTable;
        use roxlap_scene::render::{CpuFog, render_scene_composed_with_materials};

        let cam = camera.to_roxlap();
        let pixels = (width as usize) * (height as usize);
        let mut fb = vec![sky_color; pixels];
        let mut zb = vec![f32::INFINITY; pixels];

        let mut settings = OpticastSettings::for_oracle_framebuffer(width, height);
        // Ray-plane density: anginc < 1 supersamples the angular fan
        // (more ray planes), anginc > 1 coarsens it. 1.0 is the baseline.
        settings.anginc = anginc.max(0.05);

        // The DDA scratch pool is owned by the renderer now (roxlap RF.1); we
        // pass per-frame fog + per-face shading by value instead. Fog is off
        // (matches the old `pool.set_fog(0, 0)`); `sky_color` carries the
        // skycast colour, and `treat_z_max_as_air` is the renderer default.
        let fog = CpuFog {
            color: sky_color & 0x00FF_FFFF,
            max_scan_dist: 0,
            side_shades,
        };
        // Compose translucent voxels (voxel mode) the same way the live
        // viewport does, so a `--shot` screenshot matches the window. The
        // global palette is built from the model's material defs; an
        // all-opaque model passes `None` and renders byte-for-byte as before.
        let mut table = MaterialTable::new();
        for &(id, mat) in &self.material_defs {
            table.set(id, mat);
        }
        let materials = (!self.material_defs.is_empty()).then_some(&table);
        // roxlap 0.23 keys colour→material maps by `Rgb` (0x00RRGGBB); our
        // stored keys are already stripped to that packing.
        let terrain_materials: Vec<(Rgb, u8)> = self
            .material_map
            .iter()
            .map(|&(c, id)| (Rgb(c), id))
            .collect();
        render_scene_composed_with_materials(
            &mut fb,
            &mut zb,
            width as usize,
            width,
            height,
            fog,
            &mut self.scene,
            &cam,
            &settings,
            sky_color,
            None,
            materials,
            &terrain_materials,
            // Editor `--shot` renders unlit (baked shade only); no dynamic
            // light rig, no sprite-cast terrain shadows.
            roxlap_core::CpuLights::default(),
            None,
        );

        if flip_x {
            for row in fb.chunks_mut(width as usize) {
                row.reverse();
            }
        }
        fb
    }
}

fn empty_sprite_set() -> SpriteSet {
    SpriteSet {
        models: Vec::new(),
        instances: Vec::new(),
        carve_model: None,
    }
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
        // Opaque: no material data, the host draws straight from `sprites`.
        assert!(view.sprite_kv6().is_none());
        assert!(view.material_map().is_empty());
    }

    #[test]
    fn translucent_voxels_change_the_cpu_render() {
        use roxlap_render::Material;
        // A solid slab the camera looks through.
        let mut m = VoxelModel::new(6, 6, 6);
        for y in 0..6 {
            for x in 0..6 {
                m.set(x, y, 3, 0x80ff_0000);
            }
        }
        let (w, h) = (120, 100);
        let sky = 0x0020_3040;
        let render = |model: &VoxelModel| {
            let mut view = ModelView::new(model, RenderMode::Voxel);
            let cam = view.framing_camera();
            view.render_cpu(&cam, w, h, [0; 6], sky, false, 1.0)
        };

        let opaque = render(&m);
        // Same geometry, now glass: the slab lets the sky behind it through,
        // so the framebuffer must differ from the opaque render.
        m.set_material(0x80ff_0000, Material::alpha_blend(48));
        let glass = render(&m);
        assert_ne!(opaque, glass, "materials must affect the CPU render");
        // The render is deterministic — opaque renders twice are identical.
        assert_eq!(
            opaque,
            render(&{
                let mut m2 = m.clone();
                m2.set_material(0x80ff_0000, Material::OPAQUE);
                m2
            })
        );
    }

    #[test]
    fn sprite_mode_with_materials_defers_to_the_host() {
        use roxlap_render::Material;
        let mut m = VoxelModel::new(4, 4, 4);
        m.set(1, 1, 1, 0x80ff_ffff);
        m.set_material(0x80ff_ffff, Material::alpha_blend(120));
        let view = ModelView::new(&m, RenderMode::Sprite);
        // The `set_sprites` set is empty (no opaque double-draw); the host
        // registers `sprite_kv6` with the material map instead.
        assert!(view.sprites().instances.is_empty());
        assert!(view.sprite_kv6().is_some());
        assert_eq!(view.material_map().len(), 1);
        assert_eq!(view.material_defs().len(), 1);
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
        assert_eq!(view.scene_mut().grid_count(), 1, "one model grid");
    }

    #[test]
    fn voxel_edits_reuse_the_grid_and_show_up() {
        // The renderer caches by grid id, so an edit must reuse the same
        // model grid and land in its chunk — not spawn a throwaway scene.
        let mut m = VoxelModel::new(8, 8, 8);
        m.set(1, 1, 1, 0x80ff_0000);
        let mut view = ModelView::new(&m, RenderMode::Voxel);
        let gid0 = view.grid_id;
        let count0 = view.scene_mut().grid_count();

        m.set(5, 4, 3, 0x8000_ff00); // add a voxel
        view.set_model(&m, RenderMode::Voxel);

        assert_eq!(view.grid_id, gid0, "same persistent model grid reused");
        assert_eq!(
            view.scene_mut().grid_count(),
            count0,
            "no extra grid spawned"
        );
        let grid = view.scene_mut().grid_mut(gid0).expect("model grid");
        assert!(
            grid.voxel_solid(IVec3::new(5, 4, 3)),
            "the new voxel reached the grid chunk"
        );
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
