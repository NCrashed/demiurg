//! The editable document: a [`VoxelModel`] wrapped in an undo/redo
//! history.
//!
//! Every mutation goes through [`Document::commit`], which diffs the
//! requested cells against the current model, applies only the changes,
//! and records the before/after of each touched voxel as one undo step.
//! Tools are thin: they generate a list of `(position, colour)` cells
//! ([`rect_cells`], [`sphere_cells`], [`flood_fill_cells`], or a single
//! voxel) and hand it to the document. Mirror planes are applied inside
//! `commit`, so a mirrored edit is still one undo step.

use std::collections::{BTreeMap, HashSet};

use crate::VoxelModel;

/// One voxel's before/after within an [`Edit`].
#[derive(Debug, Clone, Copy)]
struct Delta {
    pos: [u32; 3],
    old: u32,
    new: u32,
}

/// One undoable step, plus a unique monotonic id used for the "modified
/// since save" check. Voxel edits store per-voxel deltas; structural ones
/// (resize / crop / grow, which change dimensions) store the whole model
/// from before the edit.
#[derive(Debug, Clone)]
enum Edit {
    Voxels { deltas: Vec<Delta>, id: u64 },
    // Boxed: a `VoxelModel` (with its 256-entry palette) is much larger
    // than the `Voxels` variant.
    Replace { before: Box<VoxelModel>, id: u64 },
}

impl Edit {
    fn id(&self) -> u64 {
        match self {
            Edit::Voxels { id, .. } | Edit::Replace { id, .. } => *id,
        }
    }
}

/// A voxel model plus its edit history. Read the model with
/// [`model`](Self::model); mutate it through the edit methods, which are
/// all undoable.
pub struct Document {
    model: VoxelModel,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// An open stroke (drag-paint): commits accumulate here, keeping each
    /// voxel's *original* pre-stroke value, until [`end_stroke`] folds
    /// them into a single undo step.
    stroke: Option<BTreeMap<[u32; 3], (u32, u32)>>,
    /// Next edit id to hand out; ids are unique and monotonic.
    next_id: u64,
    /// Edit id of the state last saved (0 = the empty initial state).
    saved_id: u64,
    /// Mirror planes about the model centre (x, y, z). An edit is
    /// duplicated across each enabled plane within the same undo step.
    pub mirror: [bool; 3],
}

impl Document {
    /// Wrap a model in a fresh (empty) history.
    #[must_use]
    pub fn new(model: VoxelModel) -> Self {
        Self {
            model,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
            next_id: 1,
            saved_id: 0,
            mirror: [false; 3],
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Id of the current state — the top undo entry's, or `0` when the
    /// history is empty.
    fn current_state_id(&self) -> u64 {
        self.undo.last().map_or(0, Edit::id)
    }

    /// Replace the model as one undoable step (resize / crop / grow). The
    /// pre-edit model is stored so undo restores it.
    fn apply_structural(&mut self, new_model: VoxelModel) {
        self.stroke = None;
        let before = std::mem::replace(&mut self.model, new_model);
        let id = self.alloc_id();
        self.undo.push(Edit::Replace {
            before: Box::new(before),
            id,
        });
        self.redo.clear();
    }

    /// Crop the model to its occupied bounding box. Returns `false` if it
    /// is empty or already tight.
    pub fn crop_to_content(&mut self) -> bool {
        let Some(cropped) = self.model.cropped() else {
            return false;
        };
        if cropped.dims() == self.model.dims() {
            return false;
        }
        self.apply_structural(cropped);
        true
    }

    /// Resize the model to `dims` (origin-anchored). Returns `false` for a
    /// zero/unchanged size.
    pub fn resize(&mut self, dims: [u32; 3]) -> bool {
        if dims.contains(&0) || (dims[0], dims[1], dims[2]) == self.model.dims() {
            return false;
        }
        let resized = self.model.resized(dims);
        self.apply_structural(resized);
        true
    }

    /// Grow the model by one voxel along `axis` (0=x, 1=y, 2=z), at the
    /// far end (`positive`) or near end. Returns `false` for a bad axis.
    pub fn grow(&mut self, axis: usize, positive: bool) -> bool {
        if axis > 2 {
            return false;
        }
        let grown = self.model.grown(axis, positive);
        self.apply_structural(grown);
        true
    }

    /// Mark the current state as saved (call after a successful save).
    pub fn mark_saved(&mut self) {
        self.saved_id = self.current_state_id();
    }

    /// Whether the document differs from the last saved state. Robust to
    /// undo/redo: undoing back to the saved edit reads as unmodified,
    /// and a different edit at the same depth reads as modified (ids are
    /// unique, not positional).
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.current_state_id() != self.saved_id
    }

    /// Begin a drag-paint stroke: subsequent edits coalesce into one undo
    /// step until [`end_stroke`](Self::end_stroke). No-op if already open.
    pub fn begin_stroke(&mut self) {
        if self.stroke.is_none() {
            self.stroke = Some(BTreeMap::new());
            self.redo.clear();
        }
    }

    /// End the current stroke, pushing it as a single undo step. Returns
    /// `true` if the stroke changed anything.
    pub fn end_stroke(&mut self) -> bool {
        let Some(map) = self.stroke.take() else {
            return false;
        };
        if map.is_empty() {
            return false;
        }
        let deltas = map
            .into_iter()
            .map(|(pos, (old, new))| Delta { pos, old, new })
            .collect();
        let id = self.alloc_id();
        self.undo.push(Edit::Voxels { deltas, id });
        true
    }

    /// The current model.
    #[must_use]
    pub fn model(&self) -> &VoxelModel {
        &self.model
    }

    /// Replace the model and clear history (e.g. after a load / resize).
    /// The fresh state counts as saved (unmodified).
    pub fn replace_model(&mut self, model: VoxelModel) {
        self.model = model;
        self.undo.clear();
        self.redo.clear();
        self.stroke = None;
        self.saved_id = 0;
    }

    /// Model dimensions `(xsiz, ysiz, zsiz)`.
    #[must_use]
    pub fn dims(&self) -> (u32, u32, u32) {
        self.model.dims()
    }

    /// The model pivot (voxel units).
    #[must_use]
    pub fn pivot(&self) -> [f32; 3] {
        self.model.pivot
    }

    /// Set the model pivot. Not part of the undo history (it is a cheap,
    /// always-reversible scalar the inspector edits live).
    pub fn set_pivot(&mut self, pivot: [f32; 3]) {
        self.model.pivot = pivot;
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Set one voxel to `col` (`0` clears it).
    pub fn set_voxel(&mut self, pos: [u32; 3], col: u32) -> bool {
        self.commit([(pos, col)])
    }

    /// Clear one voxel.
    pub fn erase_voxel(&mut self, pos: [u32; 3]) -> bool {
        self.commit([(pos, 0)])
    }

    /// Recolour a voxel **only if it is already occupied** (the paint
    /// tool: never creates geometry).
    pub fn paint_voxel(&mut self, pos: [u32; 3], col: u32) -> bool {
        if self.model.get(pos[0], pos[1], pos[2]) == 0 {
            return false;
        }
        self.commit([(pos, col)])
    }

    /// Fill an axis-aligned box (inclusive) with `col` (`0` erases).
    pub fn fill_rect(&mut self, a: [u32; 3], b: [u32; 3], col: u32) -> bool {
        self.commit(rect_cells(a, b, col))
    }

    /// Fill a solid sphere with `col` (`0` erases).
    pub fn fill_sphere(&mut self, center: [i32; 3], radius: i32, col: u32) -> bool {
        self.commit(sphere_cells(center, radius, col))
    }

    /// Flood-fill the 6-connected region of voxels matching the colour
    /// at `start`, recolouring them to `col`.
    pub fn flood_fill(&mut self, start: [u32; 3], col: u32) -> bool {
        self.commit(flood_fill_cells(&self.model, start, col))
    }

    /// Undo the last edit. Returns `false` if the history is empty.
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        match edit {
            Edit::Voxels { deltas, id } => {
                for d in &deltas {
                    self.model.set(d.pos[0], d.pos[1], d.pos[2], d.old);
                }
                self.redo.push(Edit::Voxels { deltas, id });
            }
            Edit::Replace { before, id } => {
                let cur = std::mem::replace(&mut self.model, *before);
                self.redo.push(Edit::Replace {
                    before: Box::new(cur),
                    id,
                });
            }
        }
        true
    }

    /// Redo the last undone edit. Returns `false` if there is nothing to
    /// redo.
    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        match edit {
            Edit::Voxels { deltas, id } => {
                for d in &deltas {
                    self.model.set(d.pos[0], d.pos[1], d.pos[2], d.new);
                }
                self.undo.push(Edit::Voxels { deltas, id });
            }
            Edit::Replace { before, id } => {
                let cur = std::mem::replace(&mut self.model, *before);
                self.undo.push(Edit::Replace {
                    before: Box::new(cur),
                    id,
                });
            }
        }
        true
    }

    /// Apply a batch of `(position, colour)` cells as one undo step:
    /// expand mirror planes, dedup (last write wins), keep only the
    /// cells that actually change an in-bounds voxel. Returns `true` if
    /// anything changed.
    fn commit<I: IntoIterator<Item = ([u32; 3], u32)>>(&mut self, cells: I) -> bool {
        let dims = self.model.dims();
        let mut map: BTreeMap<[u32; 3], u32> = BTreeMap::new();
        for (pos, col) in cells {
            for mirrored in mirror_positions(pos, dims, self.mirror) {
                map.insert(mirrored, col);
            }
        }

        // Inside a stroke, accumulate into the open map (keeping the
        // first-seen `old` per voxel) instead of pushing an undo step.
        if let Some(stroke) = self.stroke.as_mut() {
            let mut any = false;
            for (pos, new) in map {
                let old = self.model.get(pos[0], pos[1], pos[2]);
                if old != new && self.model.set(pos[0], pos[1], pos[2], new) {
                    any = true;
                    stroke
                        .entry(pos)
                        .and_modify(|e| e.1 = new)
                        .or_insert((old, new));
                }
            }
            return any;
        }

        let mut deltas = Vec::new();
        for (pos, new) in map {
            let old = self.model.get(pos[0], pos[1], pos[2]);
            if old != new && self.model.set(pos[0], pos[1], pos[2], new) {
                deltas.push(Delta { pos, old, new });
            }
        }
        if deltas.is_empty() {
            return false;
        }
        let id = self.alloc_id();
        self.undo.push(Edit::Voxels { deltas, id });
        self.redo.clear();
        true
    }
}

/// All mirror images of `pos` across the enabled centre planes
/// (including `pos` itself); 1, 2, 4, or 8 positions.
fn mirror_positions(pos: [u32; 3], dims: (u32, u32, u32), mirror: [bool; 3]) -> Vec<[u32; 3]> {
    let dims = [dims.0, dims.1, dims.2];
    let mut out = vec![pos];
    for axis in 0..3 {
        if !mirror[axis] {
            continue;
        }
        for i in 0..out.len() {
            let mut p = out[i];
            if p[axis] < dims[axis] {
                p[axis] = dims[axis] - 1 - p[axis];
                out.push(p);
            }
        }
    }
    out
}

/// Cells of an inclusive axis-aligned box from `a` to `b` (order-free).
#[must_use]
pub fn rect_cells(a: [u32; 3], b: [u32; 3], col: u32) -> Vec<([u32; 3], u32)> {
    let lo = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
    let hi = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
    let mut cells = Vec::new();
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                cells.push(([x, y, z], col));
            }
        }
    }
    cells
}

/// Cells of a solid sphere (centre, radius in voxels). Negative
/// coordinates are dropped; the document bounds-checks the upper edge.
#[must_use]
#[allow(clippy::cast_sign_loss)] // coords are guarded non-negative before the cast
pub fn sphere_cells(center: [i32; 3], radius: i32, col: u32) -> Vec<([u32; 3], u32)> {
    let mut cells = Vec::new();
    if radius < 0 {
        return cells;
    }
    let r2 = radius * radius;
    for dz in -radius..=radius {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy + dz * dz > r2 {
                    continue;
                }
                let (x, y, z) = (center[0] + dx, center[1] + dy, center[2] + dz);
                if x >= 0 && y >= 0 && z >= 0 {
                    cells.push(([x as u32, y as u32, z as u32], col));
                }
            }
        }
    }
    cells
}

/// Cells of the 6-connected region of voxels equal to the colour at
/// `start`, recoloured to `col`. Empty if `start` is out of bounds or
/// already `col`.
#[must_use]
pub fn flood_fill_cells(model: &VoxelModel, start: [u32; 3], col: u32) -> Vec<([u32; 3], u32)> {
    let (dx, dy, dz) = model.dims();
    if start[0] >= dx || start[1] >= dy || start[2] >= dz {
        return Vec::new();
    }
    let target = model.get(start[0], start[1], start[2]);
    if target == col {
        return Vec::new();
    }

    let mut seen: HashSet<[u32; 3]> = HashSet::new();
    let mut stack = vec![start];
    let mut cells = Vec::new();
    while let Some(p) = stack.pop() {
        if !seen.insert(p) {
            continue;
        }
        if model.get(p[0], p[1], p[2]) != target {
            continue;
        }
        cells.push((p, col));
        let [x, y, z] = p;
        if x + 1 < dx {
            stack.push([x + 1, y, z]);
        }
        if x > 0 {
            stack.push([x - 1, y, z]);
        }
        if y + 1 < dy {
            stack.push([x, y + 1, z]);
        }
        if y > 0 {
            stack.push([x, y - 1, z]);
        }
        if z + 1 < dz {
            stack.push([x, y, z + 1]);
        }
        if z > 0 {
            stack.push([x, y, z - 1]);
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: u32 = 0x80ff_0000;
    const GREEN: u32 = 0x8000_ff00;
    const BLUE: u32 = 0x8000_00ff;

    fn doc(n: u32) -> Document {
        Document::new(VoxelModel::new(n, n, n))
    }

    #[test]
    fn set_and_undo_redo() {
        let mut d = doc(4);
        assert!(d.set_voxel([1, 1, 1], RED));
        assert_eq!(d.model().get(1, 1, 1), RED);
        assert!(d.can_undo() && !d.can_redo());

        assert!(d.undo());
        assert_eq!(d.model().get(1, 1, 1), 0);
        assert!(!d.can_undo() && d.can_redo());

        assert!(d.redo());
        assert_eq!(d.model().get(1, 1, 1), RED);
    }

    #[test]
    fn no_op_edit_is_not_recorded() {
        let mut d = doc(4);
        assert!(!d.set_voxel([0, 0, 0], 0), "clearing empty changes nothing");
        assert!(!d.can_undo());
    }

    #[test]
    fn paint_only_touches_occupied() {
        let mut d = doc(4);
        assert!(!d.paint_voxel([2, 2, 2], RED), "nothing to paint");
        d.set_voxel([2, 2, 2], RED);
        assert!(d.paint_voxel([2, 2, 2], GREEN));
        assert_eq!(d.model().get(2, 2, 2), GREEN);
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut d = doc(4);
        d.set_voxel([0, 0, 0], RED);
        d.undo();
        assert!(d.can_redo());
        d.set_voxel([1, 1, 1], GREEN);
        assert!(!d.can_redo(), "a fresh edit drops the redo stack");
    }

    #[test]
    fn fill_rect_then_undo_restores_all() {
        let mut d = doc(4);
        assert!(d.fill_rect([0, 0, 0], [1, 1, 1], RED));
        assert_eq!(d.model().occupied_count(), 8);
        d.undo();
        assert_eq!(d.model().occupied_count(), 0);
    }

    #[test]
    fn sphere_is_round_and_bounded() {
        let mut d = doc(7);
        assert!(d.fill_sphere([3, 3, 3], 2, RED));
        // Corner of the bounding box is outside radius 2.
        assert_eq!(d.model().get(1, 1, 1), 0);
        // Centre and axis points are inside.
        assert_eq!(d.model().get(3, 3, 3), RED);
        assert_eq!(d.model().get(5, 3, 3), RED);
    }

    #[test]
    fn flood_fill_recolours_connected_region_only() {
        let mut d = doc(4);
        // Two separate red voxels; flood from one must not reach the other.
        d.fill_rect([0, 0, 0], [1, 0, 0], RED); // (0,0,0)-(1,0,0) connected
        d.set_voxel([3, 3, 3], RED); // isolated
        assert!(d.flood_fill([0, 0, 0], GREEN));
        assert_eq!(d.model().get(0, 0, 0), GREEN);
        assert_eq!(d.model().get(1, 0, 0), GREEN);
        assert_eq!(d.model().get(3, 3, 3), RED, "disconnected voxel untouched");
    }

    #[test]
    fn stroke_coalesces_into_one_undo_step() {
        let mut d = doc(4);
        d.begin_stroke();
        d.set_voxel([0, 0, 0], RED);
        d.set_voxel([1, 0, 0], RED);
        d.set_voxel([2, 0, 0], RED);
        assert!(d.end_stroke());
        assert_eq!(d.model().occupied_count(), 3);

        assert!(d.undo());
        assert_eq!(d.model().occupied_count(), 0, "one undo reverts the stroke");
        assert!(!d.can_undo());
    }

    #[test]
    fn stroke_keeps_pre_stroke_value_when_a_voxel_is_repainted() {
        let mut d = doc(4);
        d.set_voxel([0, 0, 0], RED); // committed before the stroke
        d.begin_stroke();
        d.paint_voxel([0, 0, 0], GREEN);
        d.paint_voxel([0, 0, 0], BLUE);
        d.end_stroke();
        assert_eq!(d.model().get(0, 0, 0), BLUE);

        d.undo();
        assert_eq!(
            d.model().get(0, 0, 0),
            RED,
            "undo restores the pre-stroke colour"
        );
    }

    #[test]
    fn empty_stroke_records_nothing() {
        let mut d = doc(4);
        d.begin_stroke();
        assert!(!d.set_voxel([0, 0, 0], 0)); // clearing empty: no change
        assert!(!d.end_stroke());
        assert!(!d.can_undo());
    }

    #[test]
    fn modified_flag_tracks_the_save_point() {
        let mut d = doc(4);
        assert!(!d.is_modified(), "fresh document is unmodified");

        d.set_voxel([0, 0, 0], RED);
        assert!(d.is_modified(), "an edit modifies it");

        d.mark_saved();
        assert!(!d.is_modified(), "saving clears it");

        d.undo();
        assert!(d.is_modified(), "undoing past the saved state modifies it");
        d.redo();
        assert!(
            !d.is_modified(),
            "redoing back to saved is unmodified again"
        );

        // A different edit at the same depth must read as modified.
        d.set_voxel([1, 0, 0], GREEN);
        d.mark_saved();
        d.undo();
        d.set_voxel([2, 0, 0], BLUE); // new branch, same undo depth as saved
        assert!(
            d.is_modified(),
            "a divergent edit at the saved depth is modified"
        );
    }

    #[test]
    fn structural_resize_is_undoable() {
        let mut d = doc(8);
        d.set_voxel([2, 3, 4], RED);
        assert!(d.crop_to_content());
        assert_eq!(d.model().dims(), (1, 1, 1), "cropped to the single voxel");
        assert!(d.is_modified());

        assert!(d.undo());
        assert_eq!(d.model().dims(), (8, 8, 8), "undo restores pre-crop dims");
        assert_eq!(d.model().get(2, 3, 4), RED, "and the voxel");

        assert!(d.redo());
        assert_eq!(d.model().dims(), (1, 1, 1), "redo re-crops");
    }

    #[test]
    fn grow_then_undo_round_trips() {
        let mut d = doc(2);
        d.set_voxel([0, 0, 0], RED);
        assert!(d.grow(0, false)); // near-end x: shifts content +1
        assert_eq!(d.model().dims(), (3, 2, 2));
        assert_eq!(d.model().get(1, 0, 0), RED);
        d.undo();
        assert_eq!(d.model().dims(), (2, 2, 2));
        assert_eq!(d.model().get(0, 0, 0), RED);
    }

    #[test]
    fn replace_model_is_unmodified() {
        let mut d = doc(4);
        d.set_voxel([0, 0, 0], RED);
        assert!(d.is_modified());
        d.replace_model(VoxelModel::new(2, 2, 2));
        assert!(!d.is_modified(), "a freshly loaded model is unmodified");
    }

    #[test]
    fn mirror_x_duplicates_across_centre() {
        let mut d = doc(4); // centre plane maps x -> 3-x
        d.mirror = [true, false, false];
        assert!(d.set_voxel([0, 1, 1], RED));
        assert_eq!(d.model().get(0, 1, 1), RED);
        assert_eq!(d.model().get(3, 1, 1), RED, "mirrored copy placed");
        // Single undo reverts both halves.
        d.undo();
        assert_eq!(d.model().occupied_count(), 0);
    }
}
