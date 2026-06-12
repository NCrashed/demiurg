//! Smoke test against a real `.kv6` authored by the voxlap/slab6
//! toolchain (not synthesised by us), proving the editor bridge handles
//! engine-produced files: parse → dense model → recompile → reparse, and
//! the occupied voxel set is stable across that second round-trip.
//!
//! The first parse may carry interior voxels the original tool stored;
//! our `to_kv6` re-extracts the surface, so we compare the *second*
//! generation against the third (both surface-only) for an exact match —
//! the editor's own save/reload cycle.

use demiurg_core::VoxelModel;

const COCO: &[u8] = include_bytes!("fixtures/coco.kv6");

#[test]
#[allow(clippy::float_cmp)] // pivot is bit-identical across an f32 serialize round-trip
fn real_kv6_loads_and_recompiles() {
    let model = VoxelModel::from_kv6_bytes(COCO).expect("coco.kv6 parses");

    let (xsiz, ysiz, zsiz) = model.dims();
    assert!(xsiz > 0 && ysiz > 0 && zsiz > 0, "non-degenerate dims");
    assert!(model.occupied_count() > 0, "model has voxels");

    // Editor save → reload must be a fixed point on the occupied set.
    let reloaded = VoxelModel::from_kv6_bytes(&model.to_kv6_bytes()).expect("re-parses");
    assert_eq!(reloaded.dims(), model.dims());

    let again = VoxelModel::from_kv6_bytes(&reloaded.to_kv6_bytes()).expect("re-parses again");
    let a: Vec<_> = reloaded.occupied().collect();
    let b: Vec<_> = again.occupied().collect();
    assert_eq!(a, b, "save/reload is stable once surface-extracted");
    assert_eq!(reloaded.pivot, model.pivot, "pivot preserved");
}
