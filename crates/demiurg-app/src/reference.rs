//! Reference images: pixel art loaded as a flat, 1-voxel-thick guide the
//! artist traces voxels from. Non-destructive — rendered as a separate
//! grid in the viewport, never part of the document (so it's never saved,
//! exported, or edited). Each opaque pixel becomes one voxel on a chosen
//! grid plane; transparent pixels are dropped.

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
/// neighbour, preserving aspect) so the plane stays within the voxel grid's
/// single render chunk.
const MAX_DIM: u32 = 128;

/// A loaded reference image placed in the voxel grid.
pub struct Reference {
    /// Opaque source pixels: `([col, row], 0x80RRGGBB)`.
    pixels: Vec<([u32; 2], u32)>,
    pub width: u32,
    pub height: u32,
    pub name: String,
    pub axis: RefAxis,
    /// Offset along the plane normal, in voxels.
    pub depth: i32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub visible: bool,
}

impl Reference {
    /// Decode image `bytes` into a reference: opaque pixels become voxels,
    /// transparent ones are dropped. `name` labels it in the panel.
    ///
    /// # Errors
    /// A message if the bytes aren't a decodable image.
    pub fn load(bytes: &[u8], name: String) -> Result<Reference, String> {
        let img = downscale(image::load_from_memory(bytes).map_err(|e| e.to_string())?);
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        let mut pixels = Vec::new();
        for (x, y, px) in rgba.enumerate_pixels() {
            let [r, g, b, a] = px.0;
            if a >= 128 {
                let col = 0x8000_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
                pixels.push(([x, y], col));
            }
        }
        Ok(Reference {
            pixels,
            width,
            height,
            name,
            axis: RefAxis::Front,
            depth: 0,
            flip_h: false,
            flip_v: false,
            visible: true,
        })
    }

    /// Placed voxel cells `([x, y, z], colour)` for the current axis /
    /// depth / flips. Empty when hidden. Image row 0 maps to the top of the
    /// plane (z is down), so it reads upright.
    #[must_use]
    #[allow(clippy::cast_possible_wrap)] // image dimensions are small
    pub fn cells(&self) -> Vec<([i32; 3], u32)> {
        if !self.visible {
            return Vec::new();
        }
        let (w, h) = (self.width as i32, self.height as i32);
        self.pixels
            .iter()
            .map(|&([px, py], col)| {
                let cx = if self.flip_h {
                    w - 1 - px as i32
                } else {
                    px as i32
                };
                let cy = if self.flip_v {
                    h - 1 - py as i32
                } else {
                    py as i32
                };
                let pos = match self.axis {
                    RefAxis::Front => [cx, self.depth, cy],
                    RefAxis::Side => [self.depth, cx, cy],
                    RefAxis::Top => [cx, cy, self.depth],
                };
                (pos, col)
            })
            .collect()
    }
}

/// Whether a path's extension is a supported reference-image format (used
/// for drag-and-drop routing).
#[must_use]
pub fn is_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "bmp" | "gif" | "tga" | "webp")
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
