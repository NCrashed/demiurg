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
    /// In-plane offset along the plane's two axes (set by the mouse drag).
    pub offset_u: i32,
    pub offset_v: i32,
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
            offset_u: 0,
            offset_v: 0,
            flip_h: false,
            flip_v: false,
            visible: true,
        })
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
                let cx = self.offset_u
                    + if self.flip_h {
                        w - 1 - px as i32
                    } else {
                        px as i32
                    };
                let cy = self.offset_v
                    + if self.flip_v {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make(pixels: Vec<([u32; 2], u32)>, w: u32, h: u32) -> Reference {
        Reference {
            pixels,
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
        }
    }

    #[test]
    fn front_plane_maps_columns_to_x_rows_to_z_with_offset() {
        let mut r = make(vec![([0, 0], 0x80ff_ffff), ([2, 3], 0x8011_2233)], 4, 5);
        r.depth = 1;
        r.offset_u = 10;
        r.offset_v = 20;
        let cells = r.cells();
        // col -> x + offset_u, row -> z + offset_v, depth along y.
        assert!(cells.contains(&([10, 1, 20], 0x80ff_ffff)));
        assert!(cells.contains(&([12, 1, 23], 0x8011_2233)));
    }

    #[test]
    fn axes_are_normal_u_v_per_plane() {
        let mut r = make(vec![], 1, 1);
        r.axis = RefAxis::Front;
        assert_eq!(r.axes(), (1, 0, 2));
        r.axis = RefAxis::Side;
        assert_eq!(r.axes(), (0, 1, 2));
        r.axis = RefAxis::Top;
        assert_eq!(r.axes(), (2, 0, 1));
    }

    #[test]
    fn flip_h_mirrors_columns() {
        let mut r = make(vec![([0, 0], 0x80ff_0000)], 4, 1);
        r.flip_h = true;
        // col 0 of a width-4 image mirrors to x = 3.
        assert_eq!(r.cells(), vec![([3, 0, 0], 0x80ff_0000)]);
    }

    #[test]
    fn hidden_reference_has_no_cells() {
        let mut r = make(vec![([0, 0], 0x80ff_ffff)], 1, 1);
        r.visible = false;
        assert!(r.cells().is_empty());
    }
}
