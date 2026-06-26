//! Editor-side document model for demiurg, and the bridge to roxlap's
//! on-disk voxel formats.
//!
//! `.kv6` stores only the **surface** voxels of a model (each tagged
//! with face-visibility bits and a normal-table index), which is the
//! right shape for the engine to draw but an awkward one to *edit*. So
//! the editor works on a dense [`VoxelModel`] — a full `xsiz·ysiz·zsiz`
//! grid of colours — and treats `.kv6` as a *compiled export*:
//!
//! - [`VoxelModel::to_kv6`] runs roxlap's own
//!   [`Kv6::from_fn`](roxlap_formats::kv6::Kv6::from_fn), so the surface
//!   extraction and the `vis` / `dir` face bits are computed by the exact
//!   engine code that renders them — what you paint is byte-for-byte what
//!   the engine shows.
//! - [`VoxelModel::from_kv6`] walks the `xlen` / `ylen` run tables back
//!   into the dense grid.
//!
//! Because `.kv6` is surface-only, an enclosed interior voxel does **not**
//! survive a `to_kv6` → `from_kv6` round-trip (see the
//! `kv6_drops_enclosed_interior_voxels` test). That loss is a property of
//! the format, not a bug; the lossless editor source is the forthcoming
//! `.demiurg` project file (M2), not `.kv6`.

use roxlap_formats::Rgb6;
use roxlap_formats::kv6::{self, Kv6};
use roxlap_formats::vxl::{self, Vxl};

pub mod edit;
pub mod project;
pub mod rig;
pub mod vox;

pub use edit::Document;
pub use rig::{KeyXform, Keyframe, Quat, Rig, RigAttachment, RigBone};

/// A dense, editable voxel model: the in-memory document the editor
/// mutates and previews.
///
/// Colours are voxlap-packed `0x80RRGGBB` words (the high byte is a
/// brightness flag, not alpha). The value `0` means "empty" — a real
/// voxel always has the `0x80` brightness bit set, so it is never `0`.
#[derive(Debug, Clone, PartialEq)]
pub struct VoxelModel {
    xsiz: u32,
    ysiz: u32,
    zsiz: u32,
    /// Pivot in voxel units (`xpiv`/`ypiv`/`zpiv`). monada rotates a
    /// placed model about this point, so it is part of the document.
    pub pivot: [f32; 3],
    /// Optional 6-bit-per-channel palette, carried through to the
    /// exported `.kv6` (`"SPal"` section).
    pub palette: Option<[Rgb6; 256]>,
    /// Dense grid, indexed `x + xsiz·(y + ysiz·z)`. Length is
    /// `xsiz·ysiz·zsiz`. `0` = empty.
    voxels: Vec<u32>,
}

/// Clamp a pivot to `[0, dim]` per axis.
#[allow(clippy::cast_precision_loss)] // dims are tiny; f32 is exact
fn clamp_pivot(p: [f32; 3], dims: [u32; 3]) -> [f32; 3] {
    [
        p[0].clamp(0.0, dims[0] as f32),
        p[1].clamp(0.0, dims[1] as f32),
        p[2].clamp(0.0, dims[2] as f32),
    ]
}

/// A voxel coordinate as `f32` (exact for editor-sized models).
#[allow(clippy::cast_precision_loss)]
fn voxel_f32(v: u32) -> f32 {
    v as f32
}

impl VoxelModel {
    /// An empty model of the given dimensions, pivot at the geometric
    /// centre.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // dimensions are tiny; f32 is exact here
    pub fn new(xsiz: u32, ysiz: u32, zsiz: u32) -> Self {
        let len = xsiz as usize * ysiz as usize * zsiz as usize;
        Self {
            xsiz,
            ysiz,
            zsiz,
            pivot: [xsiz as f32 * 0.5, ysiz as f32 * 0.5, zsiz as f32 * 0.5],
            palette: None,
            voxels: vec![0; len],
        }
    }

    /// Dimensions `(xsiz, ysiz, zsiz)` in voxels.
    #[must_use]
    pub fn dims(&self) -> (u32, u32, u32) {
        (self.xsiz, self.ysiz, self.zsiz)
    }

    /// Colour at `(x, y, z)`, or `0` if empty or out of bounds.
    #[must_use]
    pub fn get(&self, x: u32, y: u32, z: u32) -> u32 {
        self.index(x, y, z).map_or(0, |i| self.voxels[i])
    }

    /// Set the colour at `(x, y, z)` (`0` clears the voxel). Returns
    /// `false` if the coordinate is out of bounds.
    pub fn set(&mut self, x: u32, y: u32, z: u32, col: u32) -> bool {
        match self.index(x, y, z) {
            Some(i) => {
                self.voxels[i] = col;
                true
            }
            None => false,
        }
    }

    /// Number of non-empty voxels.
    #[must_use]
    pub fn occupied_count(&self) -> usize {
        self.voxels.iter().filter(|&&c| c != 0).count()
    }

    /// Iterate occupied voxels as `(x, y, z, col)` in storage order
    /// (x fastest, then y, then z).
    pub fn occupied(&self) -> impl Iterator<Item = (u32, u32, u32, u32)> + '_ {
        (0..self.zsiz).flat_map(move |z| {
            (0..self.ysiz).flat_map(move |y| {
                (0..self.xsiz).filter_map(move |x| {
                    let col = self.get(x, y, z);
                    (col != 0).then_some((x, y, z, col))
                })
            })
        })
    }

    fn index(&self, x: u32, y: u32, z: u32) -> Option<usize> {
        if x >= self.xsiz || y >= self.ysiz || z >= self.zsiz {
            return None;
        }
        Some(x as usize + self.xsiz as usize * (y as usize + self.ysiz as usize * z as usize))
    }

    /// Rebuild a dense model from a parsed `.kv6`, walking the
    /// `xlen` / `ylen` run tables.
    #[must_use]
    pub fn from_kv6(kv6: &Kv6) -> Self {
        let mut model = Self::new(kv6.xsiz, kv6.ysiz, kv6.zsiz);
        model.pivot = [kv6.xpiv, kv6.ypiv, kv6.zpiv];
        model.palette = kv6.palette;

        // Voxels are stored column-major: for each x, then each y, the
        // next `ylen[x][y]` records belong to column (x, y), each
        // carrying its own z.
        let mut cursor = 0usize;
        for x in 0..kv6.xsiz {
            for y in 0..kv6.ysiz {
                let count = kv6.ylen[x as usize][y as usize];
                for _ in 0..count {
                    let v = kv6.voxels[cursor];
                    cursor += 1;
                    model.set(x, y, u32::from(v.z), v.col);
                }
            }
        }
        model
    }

    /// Compile the model into a `.kv6`, reusing roxlap's surface
    /// extraction. Uses `from_fn_shaded`, so each surface voxel gets a
    /// real `dir` (surface normal) + `vis` (exposed faces) — the sprite
    /// render shades the model's form instead of flat. Pivot and palette
    /// are carried over from the document.
    #[must_use]
    pub fn to_kv6(&self) -> Kv6 {
        let mut kv6 = Kv6::from_fn_shaded(self.xsiz, self.ysiz, self.zsiz, |x, y, z| {
            let col = self.get(x, y, z);
            (col != 0).then_some(col)
        });
        kv6.xpiv = self.pivot[0];
        kv6.ypiv = self.pivot[1];
        kv6.zpiv = self.pivot[2];
        kv6.palette = self.palette;
        kv6
    }

    /// Compile and serialize to `.kv6` bytes.
    #[must_use]
    pub fn to_kv6_bytes(&self) -> Vec<u8> {
        kv6::serialize(&self.to_kv6())
    }

    /// Parse `.kv6` bytes into a dense model.
    ///
    /// # Errors
    /// Returns [`kv6::ParseError`] if the bytes are not a valid `.kv6`.
    pub fn from_kv6_bytes(bytes: &[u8]) -> Result<Self, kv6::ParseError> {
        kv6::parse(bytes).map(|kv6| Self::from_kv6(&kv6))
    }

    /// Compile and serialize the model to `.vxl` bytes — a small voxlap
    /// world holding just this model. The world is square (`vsid` = next
    /// power of two ≥ the larger horizontal dimension); z is voxlap
    /// z-down, `0..256` (models taller than 256 are clipped).
    #[must_use]
    pub fn to_vxl_bytes(&self) -> Vec<u8> {
        let vsid = self.xsiz.max(self.ysiz).max(1).next_power_of_two();
        let vxl = Vxl::from_dense(vsid, |x, y, z| {
            let col = self.get(x, y, z);
            (col != 0).then_some(col)
        });
        vxl::serialize(&vxl)
    }

    /// Serialize the model to `MagicaVoxel` `.vox` bytes (a single model;
    /// z-up, palette-based). See [`vox`] for the conversion details.
    #[must_use]
    pub fn to_vox_bytes(&self) -> Vec<u8> {
        vox::serialize(self)
    }

    /// Parse `MagicaVoxel` `.vox` bytes into a dense model (the first model
    /// in the file, flipped to z-down).
    ///
    /// # Errors
    /// Returns [`vox::VoxError`] if the bytes are not a valid `.vox`.
    pub fn from_vox_bytes(bytes: &[u8]) -> Result<Self, vox::VoxError> {
        vox::parse(bytes)
    }

    /// The raw dense voxel buffer (`0` = empty), indexed
    /// `x + xsiz·(y + ysiz·z)`. Used by the `.demiurg` project codec.
    #[must_use]
    pub fn voxels(&self) -> &[u32] {
        &self.voxels
    }

    /// Bounding box `(min, max)` (inclusive) of occupied voxels, or
    /// `None` if the model is empty.
    #[must_use]
    pub fn content_bounds(&self) -> Option<([u32; 3], [u32; 3])> {
        let mut min = [u32::MAX; 3];
        let mut max = [0u32; 3];
        let mut any = false;
        for (x, y, z, _) in self.occupied() {
            any = true;
            for (i, v) in [x, y, z].into_iter().enumerate() {
                min[i] = min[i].min(v);
                max[i] = max[i].max(v);
            }
        }
        any.then_some((min, max))
    }

    /// A copy cropped to the occupied bounding box, with the pivot shifted
    /// to track the same voxels. `None` if the model is empty.
    #[must_use]
    pub fn cropped(&self) -> Option<VoxelModel> {
        let (min, max) = self.content_bounds()?;
        let dims = [
            max[0] - min[0] + 1,
            max[1] - min[1] + 1,
            max[2] - min[2] + 1,
        ];
        let mut out = VoxelModel::new(dims[0], dims[1], dims[2]);
        out.palette = self.palette;
        out.pivot = clamp_pivot(
            [
                self.pivot[0] - voxel_f32(min[0]),
                self.pivot[1] - voxel_f32(min[1]),
                self.pivot[2] - voxel_f32(min[2]),
            ],
            dims,
        );
        for (x, y, z, col) in self.occupied() {
            out.set(x - min[0], y - min[1], z - min[2], col);
        }
        Some(out)
    }

    /// A copy resized to `dims`, anchored at the origin: voxels keep their
    /// coordinates, out-of-range ones are dropped, new space is empty.
    #[must_use]
    pub fn resized(&self, dims: [u32; 3]) -> VoxelModel {
        let mut out = VoxelModel::new(dims[0], dims[1], dims[2]);
        out.palette = self.palette;
        out.pivot = clamp_pivot(self.pivot, dims);
        for (x, y, z, col) in self.occupied() {
            out.set(x, y, z, col); // set ignores out-of-range coords
        }
        out
    }

    /// A copy grown by one voxel along `axis` (0=x, 1=y, 2=z). `positive`
    /// adds the layer at the far end (coordinates unchanged); otherwise at
    /// the near end (coordinates +1 on that axis, pivot tracks).
    #[must_use]
    pub fn grown(&self, axis: usize, positive: bool) -> VoxelModel {
        let mut dims = [self.xsiz, self.ysiz, self.zsiz];
        dims[axis] += 1;
        let mut shift = [0u32; 3];
        let mut pivot = self.pivot;
        if !positive {
            shift[axis] = 1;
            pivot[axis] += 1.0;
        }
        let mut out = VoxelModel::new(dims[0], dims[1], dims[2]);
        out.palette = self.palette;
        out.pivot = clamp_pivot(pivot, dims);
        for (x, y, z, col) in self.occupied() {
            out.set(x + shift[0], y + shift[1], z + shift[2], col);
        }
        out
    }

    /// The distinct non-empty voxel colours in the model, ascending.
    /// Drives the editor's "colours used in this model" palette.
    #[must_use]
    pub fn used_colors(&self) -> Vec<u32> {
        let mut set: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        for &c in &self.voxels {
            if c != 0 {
                set.insert(c);
            }
        }
        set.into_iter().collect()
    }

    /// Reconstruct a model from raw parts (e.g. a loaded `.demiurg`
    /// project). Returns `None` if `voxels.len()` does not equal
    /// `xsiz·ysiz·zsiz`.
    #[must_use]
    pub fn from_parts(
        xsiz: u32,
        ysiz: u32,
        zsiz: u32,
        pivot: [f32; 3],
        palette: Option<[Rgb6; 256]>,
        voxels: Vec<u32>,
    ) -> Option<Self> {
        if voxels.len() != xsiz as usize * ysiz as usize * zsiz as usize {
            return None;
        }
        Some(Self {
            xsiz,
            ysiz,
            zsiz,
            pivot,
            palette,
            voxels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Occupied voxels as an order-independent map, for comparison.
    fn occ(m: &VoxelModel) -> BTreeMap<(u32, u32, u32), u32> {
        m.occupied().map(|(x, y, z, c)| ((x, y, z), c)).collect()
    }

    #[test]
    fn surface_model_round_trips_through_kv6() {
        // A 3³ "plus": six arms around an empty centre — every voxel is
        // exposed, so kv6's surface-only storage loses nothing.
        let mut m = VoxelModel::new(3, 3, 3);
        m.set(1, 1, 0, 0x8000_0000 | 0x00ff_0000);
        m.set(1, 1, 2, 0x8000_0000 | 0x0000_ff00);
        m.set(0, 1, 1, 0x8000_0000 | 0x0000_00ff);
        m.set(2, 1, 1, 0x8000_0000 | 0x00ff_ff00);
        m.set(1, 0, 1, 0x8000_0000 | 0x00ff_00ff);
        m.set(1, 2, 1, 0x8000_0000 | 0x0000_ffff);

        let back = VoxelModel::from_kv6_bytes(&m.to_kv6_bytes()).unwrap();
        assert_eq!(occ(&m), occ(&back));
    }

    #[test]
    #[allow(clippy::float_cmp)] // pivot literals are exact in f32 and round-trip losslessly
    fn pivot_and_palette_survive_round_trip() {
        let mut m = VoxelModel::new(4, 4, 4);
        m.set(0, 0, 0, 0x8012_3456);
        m.pivot = [1.5, 2.0, 0.25];
        let mut pal = [Rgb6::default(); 256];
        pal[7] = Rgb6 {
            r: 12,
            g: 34,
            b: 56,
        };
        pal[255] = Rgb6 { r: 63, g: 0, b: 1 };
        m.palette = Some(pal);

        let back = VoxelModel::from_kv6_bytes(&m.to_kv6_bytes()).unwrap();
        assert_eq!(back.pivot, [1.5, 2.0, 0.25]);
        assert_eq!(back.palette, Some(pal));
    }

    #[test]
    fn kv6_drops_enclosed_interior_voxels() {
        // Documents the format property: a solid 3³ cube keeps its
        // 26-voxel shell but drops the single enclosed centre voxel.
        let mut m = VoxelModel::new(3, 3, 3);
        for z in 0..3 {
            for y in 0..3 {
                for x in 0..3 {
                    m.set(x, y, z, 0x8080_8080);
                }
            }
        }
        assert_eq!(m.occupied_count(), 27);

        let back = VoxelModel::from_kv6_bytes(&m.to_kv6_bytes()).unwrap();
        assert_eq!(back.occupied_count(), 26);
        assert_eq!(back.get(1, 1, 1), 0, "enclosed centre voxel is dropped");
    }

    #[test]
    fn exports_a_vxl_that_round_trips() {
        let mut m = VoxelModel::new(4, 4, 8);
        m.set(1, 1, 5, 0x80ff_0000);
        m.set(2, 3, 0, 0x8000_ff00);
        let vxl = roxlap_formats::vxl::parse(&m.to_vxl_bytes()).expect("vxl parses");
        assert!(
            vxl.voxel_color(1, 1, 5).is_some(),
            "a set voxel is in the vxl"
        );
        assert!(vxl.voxel_color(2, 3, 0).is_some());
        assert!(vxl.voxel_color(0, 0, 0).is_none(), "an empty cell is air");
    }

    #[test]
    fn used_colors_are_distinct_and_sorted() {
        let mut m = VoxelModel::new(4, 4, 4);
        m.set(0, 0, 0, 0x8000_00ff);
        m.set(1, 0, 0, 0x80ff_0000);
        m.set(2, 0, 0, 0x80ff_0000); // duplicate colour
        assert_eq!(m.used_colors(), vec![0x8000_00ff, 0x80ff_0000]);
        assert!(VoxelModel::new(2, 2, 2).used_colors().is_empty());
    }

    #[test]
    fn cropped_tightens_to_content() {
        let mut m = VoxelModel::new(8, 8, 8);
        m.set(3, 4, 5, 0x80ff_0000);
        m.set(4, 4, 5, 0x8000_ff00);
        let c = m.cropped().expect("has content");
        assert_eq!(c.dims(), (2, 1, 1));
        assert_eq!(c.get(0, 0, 0), 0x80ff_0000);
        assert_eq!(c.get(1, 0, 0), 0x8000_ff00);
        assert!(
            VoxelModel::new(4, 4, 4).cropped().is_none(),
            "empty has no content"
        );
    }

    #[test]
    fn resized_keeps_overlap_and_drops_rest() {
        let mut m = VoxelModel::new(4, 4, 4);
        m.set(3, 3, 3, 0x80ff_0000); // outside the new box
        m.set(0, 0, 0, 0x8000_ff00);
        let r = m.resized([2, 2, 2]);
        assert_eq!(r.dims(), (2, 2, 2));
        assert_eq!(r.get(0, 0, 0), 0x8000_ff00, "kept");
        assert_eq!(r.occupied_count(), 1, "(3,3,3) dropped");
    }

    #[test]
    fn grown_adds_a_layer_each_way() {
        let mut m = VoxelModel::new(2, 2, 2);
        m.set(0, 0, 0, 0x80ff_0000);
        let pos = m.grown(0, true); // far end: coords unchanged
        assert_eq!(pos.dims(), (3, 2, 2));
        assert_eq!(pos.get(0, 0, 0), 0x80ff_0000);
        let neg = m.grown(0, false); // near end: shifted +1
        assert_eq!(neg.dims(), (3, 2, 2));
        assert_eq!(neg.get(1, 0, 0), 0x80ff_0000, "shifted to make room");
        assert_eq!(neg.get(0, 0, 0), 0, "new empty layer at x=0");
    }

    #[test]
    fn out_of_bounds_access_is_safe() {
        let mut m = VoxelModel::new(2, 2, 2);
        assert!(!m.set(2, 0, 0, 0x8011_2233));
        assert_eq!(m.get(9, 9, 9), 0);
        assert_eq!(m.occupied_count(), 0);
    }
}
