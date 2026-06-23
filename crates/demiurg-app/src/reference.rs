//! Reference images: pixel art loaded as a flat guide the artist traces
//! voxels from. Non-destructive — never part of the document (so it's never
//! saved, exported, or edited).
//!
//! Drawn in the viewport as a flat, world-placed image sprite
//! (`roxlap_render::SceneRenderer::draw_images`), so the model occludes the
//! parts behind it and it stays undistorted from any angle — unlike the
//! earlier voxel-slab (top "film", clipped negatives) and egui-overlay (no
//! depth test, affine warp) attempts. [`Reference::placement`] turns the
//! plane / depth / offset / flip state into the sprite's world geometry.

use std::path::Path;

use image::DynamicImage;

/// Which grid plane the reference sits on (and the axis its `depth`
/// offsets along).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RefAxis {
    /// X×Z plane (a front view), offset along Y.
    Front,
    /// Y×Z plane (a side view), offset along X.
    Side,
    /// X×Y plane (a top view), offset along Z.
    Top,
}

/// Largest reference side kept; bigger images are downscaled (nearest
/// neighbour, preserving aspect) to keep the overlay texture modest.
const MAX_DIM: u32 = 512;

/// A loaded reference image placed on a grid plane.
pub struct Reference {
    /// Straight (un-premultiplied) RGBA pixels, row-major — uploaded as the
    /// image-sprite texture (`SceneRenderer::upload_image`).
    rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub name: String,
    pub axis: RefAxis,
    /// Offset along the plane normal, in voxels.
    pub depth: i32,
    /// In-plane offset along the plane's two axes (set by the mouse drag).
    pub offset_u: i32,
    pub offset_v: i32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub visible: bool,
    /// Drawing opacity in `0.0..=1.0` (scales the sprite's texel alpha), so a
    /// bright reference can be dimmed to a faint guide. `1.0` = as loaded.
    pub opacity: f32,
    /// World size of one texel, in voxels. `1.0` = 1 texel maps to 1 voxel
    /// (the default); raise it to blow a small guide up to the model's size,
    /// lower it to shrink an oversized one. Affects only how the sprite is
    /// projected, not the stored pixels.
    pub scale: f32,
}

impl Reference {
    /// Decode encoded image `bytes` (any supported codec) into a reference.
    /// `name` labels it in the panel.
    ///
    /// # Errors
    /// A message if the bytes aren't a decodable image.
    pub fn load(bytes: &[u8], name: String) -> Result<Reference, String> {
        Ok(Self::from_image(
            image::load_from_memory(bytes).map_err(|e| e.to_string())?,
            name,
        ))
    }

    /// Build a reference from a raw, row-major RGBA8 buffer (e.g. the system
    /// clipboard's straight-RGBA image, pasted in as a reference).
    ///
    /// # Errors
    /// A message if `rgba` isn't `width * height * 4` bytes.
    pub fn from_rgba(
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        name: String,
    ) -> Result<Reference, String> {
        let buf = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| "pixel buffer doesn't match its dimensions".to_string())?;
        Ok(Self::from_image(DynamicImage::ImageRgba8(buf), name))
    }

    /// Shared constructor: downscale to [`MAX_DIM`] and place at the defaults
    /// (Front plane, no offset/flip, fully opaque, 1 texel = 1 voxel).
    fn from_image(img: DynamicImage, name: String) -> Reference {
        let rgba = downscale(img).to_rgba8();
        let (width, height) = rgba.dimensions();
        Reference {
            rgba: rgba.into_raw(),
            width,
            height,
            name,
            axis: RefAxis::Front,
            depth: 0,
            offset_u: 0,
            offset_v: 0,
            flip_h: false,
            flip_v: false,
            visible: true,
            opacity: 1.0,
            scale: 1.0,
        }
    }

    /// The straight-RGBA pixel buffer, for `SceneRenderer::upload_image`.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The `0x80RRGGBB` voxel colour of texel `(col, row)`, or `None` if it's
    /// out of bounds or too transparent to sample (an eyedrop should fall
    /// through transparent pixels to whatever's behind them).
    #[must_use]
    pub fn texel(&self, col: u32, row: u32) -> Option<u32> {
        if col >= self.width || row >= self.height {
            return None;
        }
        let i = ((row * self.width + col) * 4) as usize;
        let px = self.rgba.get(i..i + 4)?;
        if px[3] < 8 {
            return None; // effectively transparent
        }
        Some(0x8000_0000 | (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]))
    }

    /// The plane's `(normal, u, v)` voxel-axis indices. `u`/`v` are the
    /// in-plane axes the image's columns/rows and the offsets run along.
    #[must_use]
    pub fn axes(&self) -> (usize, usize, usize) {
        match self.axis {
            RefAxis::Front => (1, 0, 2), // normal Y; u = X, v = Z
            RefAxis::Side => (0, 1, 2),  // normal X; u = Y, v = Z
            RefAxis::Top => (2, 0, 1),   // normal Z; u = X, v = Y
        }
    }

    /// World geometry for the image sprite: the top-left corner (image texel
    /// `(0, 0)`), the world `u`/`v` directions its columns/rows run along,
    /// and its size (1 texel = `scale` voxels). Placed at the plane's `depth`
    /// and in-plane `offset`, shifted by `-pivot` to align with the model.
    /// Flips move the corner to the opposite edge and reverse the axis, so the
    /// sprite stays a positive-size quad.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // image dims + voxel coords are small
    pub fn placement(&self, pivot: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3], [f32; 2]) {
        let (w, h) = (
            self.width as f32 * self.scale,
            self.height as f32 * self.scale,
        );
        // Corner voxel (column 0, row 0) and the world column/row axes.
        let (corner, mut u, mut v) = match self.axis {
            RefAxis::Front => ([self.offset_u, self.depth, self.offset_v], X, Z),
            RefAxis::Side => ([self.depth, self.offset_u, self.offset_v], Y, Z),
            RefAxis::Top => ([self.offset_u, self.offset_v, self.depth], X, Y),
        };
        let mut origin = [
            corner[0] as f32 - pivot[0],
            corner[1] as f32 - pivot[1],
            corner[2] as f32 - pivot[2],
        ];
        if self.flip_h {
            origin = add(origin, scale(u, w));
            u = scale(u, -1.0);
        }
        if self.flip_v {
            origin = add(origin, scale(v, h));
            v = scale(v, -1.0);
        }
        (origin, u, v, [w, h])
    }
}

const X: [f32; 3] = [1.0, 0.0, 0.0];
const Y: [f32; 3] = [0.0, 1.0, 0.0];
const Z: [f32; 3] = [0.0, 0.0, 1.0];

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// Whether a path's extension is a document (model / rig) format demiurg
/// opens as the working document. Used to route drag-and-drop: a dropped
/// file is a document if its extension says so, otherwise it's treated as a
/// reference image. The reference decoder ([`Reference::load`]) sniffs the
/// bytes, so a dropped image needn't carry an image extension at all — which
/// is what lets a `.webp` (or anything) dragged straight out of a browser,
/// often a temp file with an odd or missing extension, load as a reference.
#[must_use]
pub fn is_document(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("demiurg" | "rkc" | "kv6" | "vox")
    )
}

/// Downscale so the larger side is at most [`MAX_DIM`], nearest-neighbour
/// to keep pixel art crisp. Smaller images pass through untouched.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // scaled dims are small + positive
fn downscale(img: DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w <= MAX_DIM && h <= MAX_DIM {
        return img;
    }
    let scale = f64::from(MAX_DIM) / f64::from(w.max(h));
    let nw = ((f64::from(w) * scale) as u32).max(1);
    let nh = ((f64::from(h) * scale) as u32).max(1);
    img.resize_exact(nw, nh, image::imageops::FilterType::Nearest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(w: u32, h: u32) -> Reference {
        Reference {
            rgba: vec![0; (w * h * 4) as usize],
            width: w,
            height: h,
            name: "t".to_string(),
            axis: RefAxis::Front,
            depth: 0,
            offset_u: 0,
            offset_v: 0,
            flip_h: false,
            flip_v: false,
            visible: true,
            opacity: 1.0,
            scale: 1.0,
        }
    }

    #[test]
    fn axes_are_normal_u_v_per_plane() {
        let mut r = make(1, 1);
        r.axis = RefAxis::Front;
        assert_eq!(r.axes(), (1, 0, 2));
        r.axis = RefAxis::Side;
        assert_eq!(r.axes(), (0, 1, 2));
        r.axis = RefAxis::Top;
        assert_eq!(r.axes(), (2, 0, 1));
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact integer-valued placement
    fn front_placement_origin_axes_and_size() {
        let mut r = make(4, 6);
        r.depth = 1;
        r.offset_u = 10;
        r.offset_v = 20;
        // Corner (col 0, row 0) -> voxel (10, 1, 20) - pivot(0); columns run
        // +X, rows +Z; size is the image dims.
        let (origin, u, v, size) = r.placement([0.0; 3]);
        assert_eq!(origin, [10.0, 1.0, 20.0]);
        assert_eq!(u, [1.0, 0.0, 0.0]);
        assert_eq!(v, [0.0, 0.0, 1.0]);
        assert_eq!(size, [4.0, 6.0]);
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact integer-valued placement
    fn flips_move_the_corner_and_reverse_the_axis() {
        let mut r = make(4, 6);
        r.flip_h = true;
        r.flip_v = true;
        // Corner shifts to the far edge (+4 in X, +6 in Z) and both axes flip.
        let (origin, u, v, _) = r.placement([0.0; 3]);
        assert_eq!(origin, [4.0, 0.0, 6.0]);
        assert_eq!(u, [-1.0, 0.0, 0.0]);
        assert_eq!(v, [0.0, 0.0, -1.0]);
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact scaled placement
    fn scale_grows_the_sprite_size_and_flip_corner() {
        let mut r = make(4, 6);
        r.scale = 2.0;
        // Size is the image dims times the scale; the corner is unflipped.
        let (origin, _, _, size) = r.placement([0.0; 3]);
        assert_eq!(origin, [0.0, 0.0, 0.0]);
        assert_eq!(size, [8.0, 12.0]);
        // A flip moves the corner to the *scaled* far edge.
        r.flip_h = true;
        r.flip_v = true;
        let (origin, u, v, _) = r.placement([0.0; 3]);
        assert_eq!(origin, [8.0, 0.0, 12.0]);
        assert_eq!(u, [-1.0, 0.0, 0.0]);
        assert_eq!(v, [0.0, 0.0, -1.0]);
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact default scale
    fn from_rgba_keeps_pixels_and_defaults() {
        // 2×1: opaque red, opaque green. Round-trips through the raw-RGBA path.
        let r = Reference::from_rgba(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255], "clip".into())
            .expect("valid buffer");
        assert_eq!((r.width, r.height), (2, 1));
        assert_eq!(r.texel(0, 0), Some(0x80ff_0000));
        assert_eq!(r.texel(1, 0), Some(0x8000_ff00));
        assert_eq!(r.scale, 1.0, "fresh paste is unscaled");
    }

    #[test]
    fn from_rgba_rejects_a_mismatched_buffer() {
        // 2×2 needs 16 bytes; give 4 — must error, not panic.
        assert!(Reference::from_rgba(2, 2, vec![0; 4], "clip".into()).is_err());
    }

    #[test]
    fn texel_samples_opaque_pixels_as_voxel_colours() {
        let mut r = make(2, 1);
        // Pixel (0,0) = opaque red, (1,0) = transparent.
        r.rgba = vec![255, 0, 0, 255, 0, 0, 255, 0];
        assert_eq!(r.texel(0, 0), Some(0x80ff_0000));
        assert_eq!(r.texel(1, 0), None, "transparent texel is not sampled");
        assert_eq!(r.texel(2, 0), None, "out of bounds");
    }
}
