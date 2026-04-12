//! Container-overlap resolution force.
//!
//! Sibling containers that currently overlap get pushed apart along the
//! axis of least penetration, with force proportional to the overlap
//! depth. The push is applied as a rigid-body translation of each
//! container plus all of its descendants, so the inside of a container
//! stays laid out relative to itself while the whole block slides.
//!
//! Containers are grouped by their shared parent (sibling groups) so
//! unrelated containers at different tree levels can't repel each other,
//! only direct siblings can. A grid indexed by maximum container extent
//! keeps the per-group comparison to a 3×3 neighbourhood instead of
//! `O(S²)` pairs.

use glam::Vec2;
use std::any::Any;
use std::collections::HashMap;

use super::{Force, ForceContext};

/// Sibling-container AABB overlap resolution.
pub struct ContainerRepulsion {
    /// Whether this force is currently active.
    pub enabled: bool,
    /// Overlap-depth coefficient. The force pushing each pair apart scales
    /// linearly with `strength * overlap`.
    pub strength: f32,
    /// Sibling groups: containers grouped by their shared parent.
    /// `sibling_groups[i]` is a list of container indices whose direct
    /// parent is the same. Top-level containers form a single group.
    pub sibling_groups: Vec<Vec<usize>>,
}

impl ContainerRepulsion {
    /// Create a new container-repulsion force with precomputed sibling
    /// groups.
    pub fn new(strength: f32, sibling_groups: Vec<Vec<usize>>, enabled: bool) -> Self {
        Self {
            enabled,
            strength,
            sibling_groups,
        }
    }
}

impl Force for ContainerRepulsion {
    fn name(&self) -> &str {
        "container_repulsion"
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]) {
        if self.strength <= 0.0 {
            return;
        }
        let cr = self.strength;

        for group in &self.sibling_groups {
            // Collect the expanded, visible containers in this group.
            let live_siblings: Vec<usize> = group
                .iter()
                .copied()
                .filter(|&c| ctx.active[c] && ctx.expanded.contains(&c))
                .collect();
            if live_siblings.len() < 2 {
                continue;
            }

            // Grid-accelerated overlap detection: bucket containers by cell
            // so we only check nearby pairs instead of all O(S²).
            let max_extent = live_siblings
                .iter()
                .map(|&c| ctx.sizes[c].x.max(ctx.sizes[c].y))
                .fold(0.0f32, f32::max);
            // Cell size = largest container extent so overlapping pairs are
            // always in the same or adjacent cells.
            let cell_size = max_extent.max(1.0);
            let inv_cell = 1.0 / cell_size;

            let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
            for &c in &live_siblings {
                let cx = (ctx.positions[c].x * inv_cell).floor() as i32;
                let cy = (ctx.positions[c].y * inv_cell).floor() as i32;
                grid.entry((cx, cy)).or_default().push(c);
            }

            // Check each container against neighbours in a 3×3 cell window.
            for &a in &live_siblings {
                let pos_a = ctx.positions[a];
                let half_a = ctx.sizes[a] * 0.5;
                let ax = (pos_a.x * inv_cell).floor() as i32;
                let ay = (pos_a.y * inv_cell).floor() as i32;

                for dx in -1i32..=1 {
                    for dy in -1i32..=1 {
                        let key = (ax.wrapping_add(dx), ay.wrapping_add(dy));
                        let Some(bucket) = grid.get(&key) else {
                            continue;
                        };
                        for &b in bucket {
                            // Avoid self-pairs and double-counting (a < b).
                            if b <= a {
                                continue;
                            }
                            let pos_b = ctx.positions[b];
                            let half_b = ctx.sizes[b] * 0.5;

                            // AABB overlap test on each axis.
                            let overlap_x = (half_a.x + half_b.x) - (pos_a.x - pos_b.x).abs();
                            let overlap_y = (half_a.y + half_b.y) - (pos_a.y - pos_b.y).abs();

                            if overlap_x <= 0.0 || overlap_y <= 0.0 {
                                continue; // No overlap — skip.
                            }

                            // Push apart along the axis of least overlap
                            // (minimum penetration direction).
                            let delta = pos_a - pos_b;
                            let f = if overlap_x < overlap_y {
                                Vec2::new(delta.x.signum() * overlap_x * cr, 0.0)
                            } else {
                                Vec2::new(0.0, delta.y.signum() * overlap_y * cr)
                            };

                            // Rigid-body: move container + all descendants.
                            apply_force_to_subtree(a, f, forces, ctx.children_of, ctx.active);
                            apply_force_to_subtree(b, -f, forces, ctx.children_of, ctx.active);
                        }
                    }
                }
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

/// Apply a force to a node and all its descendants (rigid-body
/// translation). Walks the subtree via `children_of` without allocating.
/// Descendants whose `active` flag is `false` are skipped.
fn apply_force_to_subtree(
    root: usize,
    force: Vec2,
    forces: &mut [Vec2],
    children_of: &[Vec<usize>],
    active: &[bool],
) {
    forces[root] += force;
    // Use a manual stack to avoid recursion overhead.
    let mut stack: Vec<usize> = children_of[root].clone();
    while let Some(node) = stack.pop() {
        if active[node] {
            forces[node] += force;
            stack.extend_from_slice(&children_of[node]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_ir::EdgeKind;
    use std::collections::HashSet;

    fn mk_ctx<'a>(
        positions: &'a [Vec2],
        active: &'a [bool],
        sizes: &'a [Vec2],
        children_of: &'a [Vec<usize>],
        containers: &'a HashSet<usize>,
        expanded: &'a HashSet<usize>,
        toplevel: &'a HashSet<usize>,
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
    fn non_overlapping_siblings_unaffected() {
        // Two sibling containers 100 apart with half-size 10 — no overlap.
        let positions = vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)];
        let active = vec![true, true];
        let sizes = vec![Vec2::new(20.0, 20.0); 2];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::from([0, 1]);
        let expanded = HashSet::from([0, 1]);
        let toplevel = HashSet::new();
        let degrees = vec![1.0; 2];
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible: Vec<EdgeKind> = vec![];

        let ctx = mk_ctx(
            &positions,
            &active,
            &sizes,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
            &degrees,
            &edge_pairs,
            &visible,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        let sibling_groups = vec![vec![0, 1]];
        ContainerRepulsion::new(1.0, sibling_groups, true).apply(&ctx, &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn overlapping_siblings_pushed_apart() {
        // Two expanded containers overlapping along the x axis.
        // a at -5, b at +5, each 20 wide → centres 10 apart, extents
        // touch across the full overlap zone.
        let positions = vec![Vec2::new(-5.0, 0.0), Vec2::new(5.0, 0.0)];
        let active = vec![true, true];
        let sizes = vec![Vec2::new(20.0, 20.0); 2];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::from([0, 1]);
        let expanded = HashSet::from([0, 1]);
        let toplevel = HashSet::new();
        let degrees = vec![1.0; 2];
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible: Vec<EdgeKind> = vec![];

        let ctx = mk_ctx(
            &positions,
            &active,
            &sizes,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
            &degrees,
            &edge_pairs,
            &visible,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        let sibling_groups = vec![vec![0, 1]];
        ContainerRepulsion::new(1.0, sibling_groups, true).apply(&ctx, &mut forces);
        // Container a (at -x) pushed in -x, b (at +x) pushed in +x.
        assert!(forces[0].x < 0.0);
        assert!(forces[1].x > 0.0);
        // Newton's third law.
        assert!((forces[0] + forces[1]).length() < 1e-4);
    }

    #[test]
    fn collapsed_siblings_do_not_participate() {
        // Same overlapping setup as the previous test, but neither
        // container is expanded — force should be a no-op.
        let positions = vec![Vec2::new(-5.0, 0.0), Vec2::new(5.0, 0.0)];
        let active = vec![true, true];
        let sizes = vec![Vec2::new(20.0, 20.0); 2];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::from([0, 1]);
        let expanded = HashSet::new(); // nothing expanded
        let toplevel = HashSet::new();
        let degrees = vec![1.0; 2];
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible: Vec<EdgeKind> = vec![];

        let ctx = mk_ctx(
            &positions,
            &active,
            &sizes,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
            &degrees,
            &edge_pairs,
            &visible,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        let sibling_groups = vec![vec![0, 1]];
        ContainerRepulsion::new(1.0, sibling_groups, true).apply(&ctx, &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn force_applied_to_container_and_descendants() {
        // Container 0 has child 2; container 1 has child 3. Overlapping
        // siblings should have their subtrees translated rigidly.
        let positions = vec![
            Vec2::new(-5.0, 0.0),
            Vec2::new(5.0, 0.0),
            Vec2::new(-5.0, 0.0),
            Vec2::new(5.0, 0.0),
        ];
        let active = vec![true; 4];
        let sizes = vec![Vec2::new(20.0, 20.0); 4];
        let children_of: Vec<Vec<usize>> = vec![vec![2], vec![3], vec![], vec![]];
        let containers = HashSet::from([0, 1]);
        let expanded = HashSet::from([0, 1]);
        let toplevel = HashSet::new();
        let degrees = vec![1.0; 4];
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible: Vec<EdgeKind> = vec![];

        let ctx = mk_ctx(
            &positions,
            &active,
            &sizes,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
            &degrees,
            &edge_pairs,
            &visible,
        );
        let mut forces = vec![Vec2::ZERO; 4];
        let sibling_groups = vec![vec![0, 1]];
        ContainerRepulsion::new(1.0, sibling_groups, true).apply(&ctx, &mut forces);

        // Container and its child get identical force vectors (rigid body).
        assert_eq!(forces[0], forces[2]);
        assert_eq!(forces[1], forces[3]);
        // And the two subtrees get opposite pushes.
        assert!(forces[0].x < 0.0);
        assert!(forces[1].x > 0.0);
    }
}
