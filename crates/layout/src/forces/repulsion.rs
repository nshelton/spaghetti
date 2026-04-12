//! Grid-based Coulomb repulsion between visible nodes.
//!
//! Every active node pushes every other active node away with a force that
//! falls off as `strength / dist²`. Instead of an O(N²) all-pairs loop, the
//! force builds a spatial grid with cell size equal to the cutoff distance,
//! then only considers the 3×3 neighbourhood of each node's cell. Pairs
//! farther apart than the cutoff contribute zero anyway, so the grid makes
//! the work linear in practice.
//!
//! On large graphs (≥ [`PARALLEL_THRESHOLD`] nodes) the per-node inner loop
//! runs under rayon. Each iteration produces a single Vec2 into an
//! independent output slot, so there are no write conflicts.

use glam::Vec2;
use rayon::prelude::*;
use std::any::Any;
use std::collections::HashMap;

use super::{Force, ForceContext, PARALLEL_THRESHOLD};

/// Grid-based point-Coulomb repulsion.
///
/// `strength`, `cutoff`, and `min_dist` map directly to the legacy
/// [`crate::ForceParams`] fields `repulsion`, `repulsion_cutoff`, and
/// `min_dist`.
pub struct Repulsion {
    /// Whether this force is currently active.
    pub enabled: bool,
    /// Coulomb-style repulsion coefficient applied per interacting pair.
    pub strength: f32,
    /// Maximum distance considered for repulsion. Pairs farther apart than
    /// this are skipped entirely. Also sets the spatial grid cell size.
    pub cutoff: f32,
    /// Floor for pairwise distance to avoid `1 / dist²` blow-ups when two
    /// nodes are nearly coincident.
    pub min_dist: f32,
}

impl Repulsion {
    /// Create a new repulsion force with the given parameters.
    pub fn new(strength: f32, cutoff: f32, min_dist: f32, enabled: bool) -> Self {
        Self {
            enabled,
            strength,
            cutoff,
            min_dist,
        }
    }
}

impl Force for Repulsion {
    fn name(&self) -> &str {
        "repulsion"
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]) {
        if self.strength == 0.0 || self.cutoff <= 0.0 {
            return;
        }

        let cutoff = self.cutoff;
        let cutoff_sq = cutoff * cutoff;
        let inv_cutoff = 1.0 / cutoff;
        let strength = self.strength;
        let min_dist = self.min_dist;
        let len = ctx.node_count;

        // Cell key for every position (including hidden — indices remain
        // stable, but hidden nodes don't get added to the grid below).
        let cell_keys: Vec<(i32, i32)> = ctx
            .positions
            .iter()
            .map(|pos| {
                let cx = (pos.x * inv_cutoff).floor() as i32;
                let cy = (pos.y * inv_cutoff).floor() as i32;
                (cx, cy)
            })
            .collect();

        // Bucket only active nodes. Hidden nodes aren't in any cell, so they
        // can't attract force contributions from their neighbours.
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::with_capacity(len / 4 + 1);
        for (i, &key) in cell_keys.iter().enumerate() {
            if ctx.active[i] {
                grid.entry(key).or_default().push(i);
            }
        }

        let positions = ctx.positions;
        let grid_ref = &grid;
        let active = ctx.active;

        // Per-node repulsion. Each node writes into a single output slot, so
        // the inner loop parallelises cleanly.
        if len >= PARALLEL_THRESHOLD {
            let contribs: Vec<Vec2> = (0..len)
                .into_par_iter()
                .map(|i| {
                    if !active[i] {
                        return Vec2::ZERO;
                    }
                    compute_repulsion_for_node(
                        i, positions, grid_ref, &cell_keys, cutoff_sq, strength, min_dist,
                    )
                })
                .collect();
            for (force, contrib) in forces.iter_mut().zip(contribs.iter()) {
                *force += *contrib;
            }
        } else {
            for (i, force) in forces.iter_mut().enumerate().take(len) {
                if !active[i] {
                    continue;
                }
                *force += compute_repulsion_for_node(
                    i, positions, grid_ref, &cell_keys, cutoff_sq, strength, min_dist,
                );
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Compute point-based Coulomb repulsive force on node `i` from its 3×3
/// grid neighbourhood. Center-to-center distance, no size awareness.
#[allow(clippy::too_many_arguments)]
fn compute_repulsion_for_node(
    i: usize,
    positions: &[Vec2],
    grid: &HashMap<(i32, i32), Vec<usize>>,
    cell_keys: &[(i32, i32)],
    cutoff_sq: f32,
    strength: f32,
    min_dist: f32,
) -> Vec2 {
    let pos_i = positions[i];
    let (cx, cy) = cell_keys[i];
    let mut force = Vec2::ZERO;

    // Scan 3×3 neighbourhood (including own cell).
    for dx in -1..=1i32 {
        for dy in -1..=1i32 {
            let nx = cx.wrapping_add(dx);
            let ny = cy.wrapping_add(dy);
            if let Some(cell) = grid.get(&(nx, ny)) {
                for &j in cell {
                    if j == i {
                        continue;
                    }
                    let delta = pos_i - positions[j];
                    let dist_sq = delta.length_squared();
                    if dist_sq > cutoff_sq || dist_sq < 1e-10 {
                        continue;
                    }
                    let dist = dist_sq.sqrt().max(min_dist);
                    force += delta.normalize_or_zero() * (strength / (dist * dist));
                }
            }
        }
    }
    force
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_ir::EdgeKind;
    use std::collections::HashSet;

    fn mk_ctx<'a>(
        positions: &'a [Vec2],
        active: &'a [bool],
        containers: &'a HashSet<usize>,
        expanded: &'a HashSet<usize>,
        toplevel: &'a HashSet<usize>,
        children_of: &'a [Vec<usize>],
        sizes: &'a [Vec2],
        degrees: &'a [f32],
        edge_pairs: &'a [(usize, usize, EdgeKind)],
        visible_edge_kinds: &'a [EdgeKind],
    ) -> ForceContext<'a> {
        ForceContext {
            positions,
            sizes,
            degrees,
            active,
            edge_pairs,
            visible_edge_kinds,
            children_of,
            containers,
            expanded,
            toplevel_containers: toplevel,
            node_count: positions.len(),
        }
    }

    #[test]
    fn disabled_strength_is_noop() {
        let positions = vec![Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0)];
        let active = vec![true, true];
        let sizes = vec![Vec2::ZERO; 2];
        let degrees = vec![1.0; 2];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible_edge_kinds: Vec<EdgeKind> = vec![];

        let ctx = mk_ctx(
            &positions,
            &active,
            &containers,
            &expanded,
            &toplevel,
            &children_of,
            &sizes,
            &degrees,
            &edge_pairs,
            &visible_edge_kinds,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        Repulsion::new(0.0, 500.0, 1.0, true).apply(&ctx, &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn two_nodes_push_apart_symmetrically() {
        let positions = vec![Vec2::new(5.0, 0.0), Vec2::new(-5.0, 0.0)];
        let active = vec![true, true];
        let sizes = vec![Vec2::ZERO; 2];
        let degrees = vec![1.0; 2];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible_edge_kinds: Vec<EdgeKind> = vec![];

        let ctx = mk_ctx(
            &positions,
            &active,
            &containers,
            &expanded,
            &toplevel,
            &children_of,
            &sizes,
            &degrees,
            &edge_pairs,
            &visible_edge_kinds,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        Repulsion::new(1000.0, 500.0, 1.0, true).apply(&ctx, &mut forces);
        // Node 0 at +x gets pushed in +x, node 1 at -x gets pushed in -x.
        assert!(forces[0].x > 0.0 && forces[0].y == 0.0);
        assert!(forces[1].x < 0.0 && forces[1].y == 0.0);
        // Newton's third law: forces are equal and opposite.
        assert!((forces[0] + forces[1]).length() < 1e-4);
    }

    #[test]
    fn hidden_nodes_do_not_interact() {
        // Node 1 is hidden — it should neither push nor be pushed.
        let positions = vec![
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(-1.0, 0.0),
        ];
        let active = vec![true, false, true];
        let sizes = vec![Vec2::ZERO; 3];
        let degrees = vec![1.0; 3];
        let children_of: Vec<Vec<usize>> = vec![vec![]; 3];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible_edge_kinds: Vec<EdgeKind> = vec![];

        let ctx = mk_ctx(
            &positions,
            &active,
            &containers,
            &expanded,
            &toplevel,
            &children_of,
            &sizes,
            &degrees,
            &edge_pairs,
            &visible_edge_kinds,
        );
        let mut forces = vec![Vec2::ZERO; 3];
        Repulsion::new(1000.0, 500.0, 1.0, true).apply(&ctx, &mut forces);
        assert_eq!(forces[1], Vec2::ZERO, "hidden node received force");
        // The two visible nodes still push each other.
        assert!(forces[0].x > 0.0);
        assert!(forces[2].x < 0.0);
    }

    #[test]
    fn cutoff_respected() {
        // Two nodes far apart (farther than cutoff) should feel no force.
        let positions = vec![Vec2::new(10000.0, 0.0), Vec2::new(-10000.0, 0.0)];
        let active = vec![true, true];
        let sizes = vec![Vec2::ZERO; 2];
        let degrees = vec![1.0; 2];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible_edge_kinds: Vec<EdgeKind> = vec![];

        let ctx = mk_ctx(
            &positions,
            &active,
            &containers,
            &expanded,
            &toplevel,
            &children_of,
            &sizes,
            &degrees,
            &edge_pairs,
            &visible_edge_kinds,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        Repulsion::new(1000.0, 500.0, 1.0, true).apply(&ctx, &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }
}
