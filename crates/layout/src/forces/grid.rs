//! Spatial index for 2D point lookups inside a fixed cell neighbourhood.
//!
//! [`SpatialGrid::build`] buckets a set of `(index, position)` pairs
//! into square cells of a given `cell_size`, then [`for_each_in_neighborhood`]
//! visits every candidate index in the 3×3 cell window around a query
//! position.
//!
//! The grid normally uses a dense flat vector indexed by `row * cols + col`,
//! giving O(1) lookups with good cache locality and no hashing overhead.
//! For pathological position spreads — a handful of points far apart,
//! where the flat vector would be enormous and mostly empty — the
//! builder falls back to a sparse [`HashMap`]-backed representation.
//!
//! Both the repulsion and container-overlap forces build one of these
//! per step with different cell sizes and candidate sets.
//!
//! [`for_each_in_neighborhood`]: SpatialGrid::for_each_in_neighborhood

use glam::Vec2;
use std::collections::HashMap;

/// Upper bound on `rows * cols` before the flat layout falls back to
/// the hashmap fallback. 1M cells (≈24 MB of `Vec<usize>` headers at
/// 24 bytes each) is well past what any reasonable graph needs — this
/// threshold only trips when a single far-flung outlier blows up the
/// bounding box.
const FLAT_GRID_CELL_LIMIT: usize = 1_000_000;

/// Spatial index for 2D point lookups inside a fixed cell neighbourhood.
///
/// Construct with [`SpatialGrid::build`], then compute a query cell key
/// with [`SpatialGrid::cell_of`] and visit the 3×3 neighbourhood via
/// [`SpatialGrid::for_each_in_neighborhood`].
pub(crate) enum SpatialGrid {
    /// Dense flat layout. `cells[row * cols + col]` holds the indices
    /// that bucketed into `(col + origin_col, row + origin_row)`.
    Flat {
        cols: usize,
        rows: usize,
        origin_col: i32,
        origin_row: i32,
        inv_cell: f32,
        cells: Vec<Vec<usize>>,
    },
    /// Sparse hashmap fallback keyed on the integer cell coordinate.
    Hashed {
        inv_cell: f32,
        cells: HashMap<(i32, i32), Vec<usize>>,
    },
}

impl SpatialGrid {
    /// Build a grid from an iterator of `(index, position)` pairs.
    ///
    /// `cell_size` sets the granularity and must be strictly positive.
    /// The resulting grid's 3×3 neighbourhood catches every pair of
    /// points within `cell_size` of each other.
    pub(crate) fn build<I>(cell_size: f32, iter: I) -> Self
    where
        I: IntoIterator<Item = (usize, Vec2)>,
    {
        debug_assert!(cell_size > 0.0);
        let inv_cell = 1.0 / cell_size;

        // First pass: compute each candidate's integer cell key and the
        // enclosing cell-space bounding box.
        let mut min_c = i32::MAX;
        let mut max_c = i32::MIN;
        let mut min_r = i32::MAX;
        let mut max_r = i32::MIN;
        let mut candidates: Vec<(usize, i32, i32)> = Vec::new();
        for (i, pos) in iter {
            let c = (pos.x * inv_cell).floor() as i32;
            let r = (pos.y * inv_cell).floor() as i32;
            candidates.push((i, c, r));
            if c < min_c {
                min_c = c;
            }
            if c > max_c {
                max_c = c;
            }
            if r < min_r {
                min_r = r;
            }
            if r > max_r {
                max_r = r;
            }
        }

        if candidates.is_empty() {
            return Self::Hashed {
                inv_cell,
                cells: HashMap::new(),
            };
        }

        // Span in i64 so extreme cell coordinates (from positions near
        // the f32→i32 saturation limits) don't overflow the
        // subtraction before we even get a chance to fall back.
        let cols_span = (max_c as i64) - (min_c as i64) + 1;
        let rows_span = (max_r as i64) - (min_r as i64) + 1;
        let total = cols_span.saturating_mul(rows_span);

        if total > FLAT_GRID_CELL_LIMIT as i64 {
            // Sparse / pathological spread: fall back to hashed.
            let mut cells: HashMap<(i32, i32), Vec<usize>> =
                HashMap::with_capacity(candidates.len() / 4 + 1);
            for (i, c, r) in candidates {
                cells.entry((c, r)).or_default().push(i);
            }
            return Self::Hashed { inv_cell, cells };
        }

        // Spans now fit comfortably in `usize`. Per-candidate
        // `c - min_c` likewise fits in `i32` because the span passed
        // the limit check above.
        let cols = cols_span as usize;
        let rows = rows_span as usize;
        let mut cells: Vec<Vec<usize>> = vec![Vec::new(); cols * rows];
        for (i, c, r) in candidates {
            let lc = (c - min_c) as usize;
            let lr = (r - min_r) as usize;
            cells[lr * cols + lc].push(i);
        }

        Self::Flat {
            cols,
            rows,
            origin_col: min_c,
            origin_row: min_r,
            inv_cell,
            cells,
        }
    }

    /// Integer cell coordinate for a world-space position. The result
    /// matches the key space used internally by both variants, so
    /// [`for_each_in_neighborhood`](Self::for_each_in_neighborhood) can
    /// consume it directly.
    #[inline]
    pub(crate) fn cell_of(&self, pos: Vec2) -> (i32, i32) {
        let inv = match self {
            Self::Flat { inv_cell, .. } | Self::Hashed { inv_cell, .. } => *inv_cell,
        };
        let c = (pos.x * inv).floor() as i32;
        let r = (pos.y * inv).floor() as i32;
        (c, r)
    }

    /// Invoke `f` once for every index bucketed into the 3×3 cell
    /// neighbourhood centred on `query_cell`. Empty or out-of-range
    /// cells are silently skipped.
    ///
    /// Local cell arithmetic runs in `i64` so extreme query keys
    /// (produced when callers pass positions near the f32 → i32
    /// saturation limit) can't overflow.
    #[inline]
    pub(crate) fn for_each_in_neighborhood<F: FnMut(usize)>(
        &self,
        query_cell: (i32, i32),
        mut f: F,
    ) {
        let (qc, qr) = query_cell;
        match self {
            Self::Flat {
                cols,
                rows,
                origin_col,
                origin_row,
                cells,
                ..
            } => {
                let cols_i64 = *cols as i64;
                let rows_i64 = *rows as i64;
                let oc = *origin_col as i64;
                let or = *origin_row as i64;
                let qc_i64 = qc as i64;
                let qr_i64 = qr as i64;
                for dc in -1..=1i64 {
                    for dr in -1..=1i64 {
                        let lc = qc_i64 + dc - oc;
                        let lr = qr_i64 + dr - or;
                        if lc < 0 || lr < 0 || lc >= cols_i64 || lr >= rows_i64 {
                            continue;
                        }
                        let idx = lr as usize * *cols + lc as usize;
                        for &j in &cells[idx] {
                            f(j);
                        }
                    }
                }
            }
            Self::Hashed { cells, .. } => {
                for dc in -1..=1i32 {
                    for dr in -1..=1i32 {
                        let key = (qc.wrapping_add(dc), qr.wrapping_add(dr));
                        if let Some(bucket) = cells.get(&key) {
                            for &j in bucket {
                                f(j);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect every index returned by a 3×3 neighbourhood scan so
    /// assertions can compare sets instead of depending on iteration
    /// order.
    fn collect_neighbors(grid: &SpatialGrid, query: (i32, i32)) -> Vec<usize> {
        let mut out = Vec::new();
        grid.for_each_in_neighborhood(query, |j| out.push(j));
        out.sort_unstable();
        out
    }

    #[test]
    fn empty_input_builds_empty_hashed_grid() {
        let grid = SpatialGrid::build(10.0, std::iter::empty());
        assert!(matches!(grid, SpatialGrid::Hashed { .. }));
        assert!(collect_neighbors(&grid, (0, 0)).is_empty());
    }

    #[test]
    fn cell_of_matches_build() {
        let positions = [
            (0usize, Vec2::new(5.0, 5.0)),
            (1usize, Vec2::new(25.0, 15.0)),
        ];
        let grid = SpatialGrid::build(10.0, positions.iter().copied());
        // Node 0 at (5,5) → cell (0, 0). Node 1 at (25,15) → cell (2, 1).
        assert_eq!(grid.cell_of(Vec2::new(5.0, 5.0)), (0, 0));
        assert_eq!(grid.cell_of(Vec2::new(25.0, 15.0)), (2, 1));
    }

    #[test]
    fn flat_grid_returns_nearby_indices() {
        // 5 points arranged so some share a cell and some are in
        // adjacent cells. cell_size = 10.
        let positions = vec![
            (0usize, Vec2::new(1.0, 1.0)),   // cell (0, 0)
            (1usize, Vec2::new(2.0, 2.0)),   // cell (0, 0) — same as node 0
            (2usize, Vec2::new(11.0, 1.0)),  // cell (1, 0) — adjacent
            (3usize, Vec2::new(25.0, 25.0)), // cell (2, 2) — far
            (4usize, Vec2::new(-5.0, 0.0)),  // cell (-1, 0) — adjacent
        ];
        let grid = SpatialGrid::build(10.0, positions.iter().copied());
        assert!(matches!(grid, SpatialGrid::Flat { .. }));

        // 3×3 neighbourhood around (0, 0) sees nodes 0, 1, 2, 4 (all
        // within the 3×3 window) but not node 3 which is two cells away.
        let neighbors = collect_neighbors(&grid, (0, 0));
        assert_eq!(neighbors, vec![0, 1, 2, 4]);

        // Around the far node's cell (2, 2), only node 3 itself.
        let neighbors = collect_neighbors(&grid, (2, 2));
        assert_eq!(neighbors, vec![3]);
    }

    #[test]
    fn flat_grid_respects_bbox_edges() {
        // Nodes all in the same cell. Querying a cell outside the grid
        // bounds should return nothing instead of panicking.
        let positions = [(0usize, Vec2::new(0.0, 0.0)), (1usize, Vec2::new(1.0, 1.0))];
        let grid = SpatialGrid::build(10.0, positions.iter().copied());

        // Query far outside the bbox — both the flat and hashed variants
        // must silently return no hits.
        assert!(collect_neighbors(&grid, (100, 100)).is_empty());
        assert!(collect_neighbors(&grid, (-100, -100)).is_empty());
    }

    #[test]
    fn pathological_spread_falls_back_to_hashed() {
        // Two nodes at opposite ends of the world at a fine cell size.
        // Cell-bbox is (1e6 + 1) × (1e6 + 1) ≫ FLAT_GRID_CELL_LIMIT, so
        // the builder must pick the hashed variant.
        let positions = [
            (0usize, Vec2::new(0.0, 0.0)),
            (1usize, Vec2::new(1.0e7, 1.0e7)),
        ];
        let grid = SpatialGrid::build(10.0, positions.iter().copied());
        assert!(matches!(grid, SpatialGrid::Hashed { .. }));

        // Both nodes are still individually findable.
        let near_zero = collect_neighbors(&grid, grid.cell_of(Vec2::ZERO));
        assert_eq!(near_zero, vec![0]);
        let near_far = collect_neighbors(&grid, grid.cell_of(Vec2::new(1.0e7, 1.0e7)));
        assert_eq!(near_far, vec![1]);
    }

    #[test]
    fn hashed_variant_handles_negative_cell_keys() {
        // Hashed grid should tolerate negative cell coordinates without
        // the flat-grid's bbox translation.
        let positions = [
            (0usize, Vec2::new(-1.0e7, 0.0)),
            (1usize, Vec2::new(1.0e7, 0.0)),
        ];
        let grid = SpatialGrid::build(10.0, positions.iter().copied());
        assert!(matches!(grid, SpatialGrid::Hashed { .. }));

        let near_neg = collect_neighbors(&grid, grid.cell_of(Vec2::new(-1.0e7, 0.0)));
        assert_eq!(near_neg, vec![0]);
    }
}
