//! `MagicaVoxel` `.vox` import / export.
//!
//! `.vox` is a popular external authoring format, so this is an interop
//! bridge (not an engine format). Import goes through the `dot_vox`
//! parser, which handles the real-world chunk variants (scene graph,
//! multiple models) and supplies the default palette; we take the first
//! model. Export is hand-rolled — a single `MAIN` chunk with `SIZE`,
//! `XYZI`, and `RGBA` children.
//!
//! `MagicaVoxel` is **z-up**, demiurg/voxlap is **z-down**, so the height
//! axis is flipped on the way in and out (the model stays upright).
//! Colours travel through a 256-entry palette: the dense grid stores
//! packed `0x80RRGGBB`, so on import we resolve each voxel's palette index
//! to a colour, and on export we build a palette from the distinct colours
//! used.

use std::collections::HashMap;
use std::fmt;

use crate::VoxelModel;

/// A `.vox` parse failure.
#[derive(Debug)]
pub enum VoxError {
    /// The bytes are not a valid `.vox` file.
    Parse(String),
    /// The file contains no voxel model.
    Empty,
}

impl fmt::Display for VoxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoxError::Parse(e) => write!(f, "invalid .vox: {e}"),
            VoxError::Empty => write!(f, ".vox has no model"),
        }
    }
}

impl std::error::Error for VoxError {}

/// Parse `.vox` bytes into a dense model: the first model in the file,
/// with its palette colours, and `z` flipped from `MagicaVoxel`'s z-up to
/// demiurg's z-down so it stays upright.
///
/// # Errors
/// [`VoxError`] if the bytes aren't a valid `.vox` or hold no model.
pub fn parse(bytes: &[u8]) -> Result<VoxelModel, VoxError> {
    let data = dot_vox::load_bytes(bytes).map_err(|e| VoxError::Parse(e.to_string()))?;
    let model = data.models.first().ok_or(VoxError::Empty)?;
    let (sx, sy, sz) = (model.size.x, model.size.y, model.size.z);
    let mut out = VoxelModel::new(sx, sy, sz);
    for v in &model.voxels {
        let c = data.palette[v.i as usize];
        let packed = 0x8000_0000 | (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b);
        // z-up -> z-down: flip the height axis (voxels are within `size`,
        // so `v.z < sz` and the subtraction never underflows).
        let z = sz - 1 - u32::from(v.z);
        out.set(u32::from(v.x), u32::from(v.y), z, packed);
    }
    Ok(out)
}

/// Serialize the model to a single-model `.vox` (z flipped back to z-up).
/// Distinct colours map to palette indices `1..=255`; further colours, and
/// any axis beyond 256 voxels, are clamped — `.vox` coordinates are one
/// byte. Editor models comfortably fit both limits.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // dims clamped into 0..256
pub fn serialize(model: &VoxelModel) -> Vec<u8> {
    let (dx, dy, dz) = model.dims();
    let (sx, sy, sz) = (dx.min(256), dy.min(256), dz.min(256));

    // Palette: distinct colours -> indices 1..=255. The RGBA chunk stores
    // entry `c - 1` for colour index `c` (the `MagicaVoxel` convention).
    let mut index_of: HashMap<u32, u8> = HashMap::new();
    let mut rgba = [0u8; 256 * 4];
    for (slot, &col) in model.used_colors().iter().take(255).enumerate() {
        index_of.insert(col, (slot + 1) as u8); // colour index 1..=255
        rgba[slot * 4] = (col >> 16) as u8;
        rgba[slot * 4 + 1] = (col >> 8) as u8;
        rgba[slot * 4 + 2] = col as u8;
        rgba[slot * 4 + 3] = 0xff;
    }
    let fallback = index_of.len().clamp(1, 255) as u8; // colours past the 255th

    // XYZI: solid voxels, z flipped to z-up, coordinates clamped to a byte.
    let mut xyzi = Vec::new();
    let mut count: u32 = 0;
    for (x, y, z, col) in model.occupied() {
        if x >= sx || y >= sy || z >= sz {
            continue;
        }
        let idx = index_of.get(&col).copied().unwrap_or(fallback);
        let vz = (sz - 1 - z) as u8; // z-down -> z-up
        xyzi.extend_from_slice(&[x as u8, y as u8, vz, idx]);
        count += 1;
    }
    let mut xyzi_content = count.to_le_bytes().to_vec();
    xyzi_content.extend_from_slice(&xyzi);

    let mut size_content = Vec::with_capacity(12);
    size_content.extend_from_slice(&(sx as i32).to_le_bytes());
    size_content.extend_from_slice(&(sy as i32).to_le_bytes());
    size_content.extend_from_slice(&(sz as i32).to_le_bytes());

    let mut children = Vec::new();
    write_chunk(&mut children, *b"SIZE", &size_content);
    write_chunk(&mut children, *b"XYZI", &xyzi_content);
    write_chunk(&mut children, *b"RGBA", &rgba);

    let mut out = Vec::new();
    out.extend_from_slice(b"VOX ");
    out.extend_from_slice(&150i32.to_le_bytes());
    write_chunk_with_children(&mut out, *b"MAIN", &[], &children);
    out
}

/// Append a leaf chunk (no children).
fn write_chunk(out: &mut Vec<u8>, id: [u8; 4], content: &[u8]) {
    write_chunk_with_children(out, id, content, &[]);
}

/// Append a chunk: id, content length, children length, content, children
/// (all lengths little-endian `i32`).
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // chunk sizes are small
fn write_chunk_with_children(out: &mut Vec<u8>, id: [u8; 4], content: &[u8], children: &[u8]) {
    out.extend_from_slice(&id);
    out.extend_from_slice(&(content.len() as i32).to_le_bytes());
    out.extend_from_slice(&(children.len() as i32).to_le_bytes());
    out.extend_from_slice(content);
    out.extend_from_slice(children);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_model_through_vox() {
        let mut m = VoxelModel::new(4, 5, 6);
        m.set(0, 0, 0, 0x80ff_0000);
        m.set(1, 2, 3, 0x8000_ff00);
        m.set(3, 4, 5, 0x8012_3456);
        let back = parse(&serialize(&m)).expect("re-parse");
        assert_eq!(back.dims(), (4, 5, 6));
        assert_eq!(back.occupied_count(), 3);
        assert_eq!(back.get(0, 0, 0), 0x80ff_0000);
        assert_eq!(back.get(1, 2, 3), 0x8000_ff00);
        assert_eq!(
            back.get(3, 4, 5),
            0x8012_3456,
            "colours survive the palette"
        );
    }

    #[test]
    fn flips_z_so_the_model_stays_upright() {
        // z is down, so z=0 is the top. It must round-trip back to the top,
        // not flip to the bottom (z = sz-1).
        let mut m = VoxelModel::new(2, 2, 4);
        m.set(0, 0, 0, 0x80ff_ffff);
        let back = parse(&serialize(&m)).expect("re-parse");
        assert_eq!(back.get(0, 0, 0), 0x80ff_ffff, "top stays on top");
        assert_eq!(back.get(0, 0, 3), 0, "not flipped to the bottom");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"definitely not a vox file").is_err());
    }
}
