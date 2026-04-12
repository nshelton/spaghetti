//! Directory-affinity force: cluster nodes that share filesystem directories.
//!
//! Nodes whose source locations share a directory prefix at some depth are
//! pulled toward the centroid of their group. Deeper (more specific)
//! prefixes get the full strength; shallower levels decay by
//! `falloff.powi(depth)` so same-directory attraction dominates over
//! same-grandparent.
//!
//! The directory grouping is precomputed once at construction time and
//! owned by this force — before the refactor it lived on `LayoutState`
//! as `dir_groups` and `max_dir_depth`.

use glam::Vec2;
use std::any::Any;

use super::{Force, ForceContext};

/// Directory-affinity clustering force.
pub struct LocationAffinity {
    /// Whether this force is currently active.
    pub enabled: bool,
    /// Base strength at the deepest directory level.
    pub strength: f32,
    /// Per-level decay. `0.5` means each level up halves the attraction.
    pub falloff: f32,
    /// `dir_groups[depth][group_idx]` is a list of node indices sharing the
    /// same directory prefix at `depth`. Groups with fewer than two members
    /// are not stored (a singleton group produces no force).
    pub dir_groups: Vec<Vec<Vec<usize>>>,
    /// Maximum directory depth across all nodes (cached for force scaling).
    pub max_dir_depth: usize,
}

impl LocationAffinity {
    /// Create a new location-affinity force from precomputed directory
    /// groups. `max_dir_depth` should equal
    /// `dir_groups.len().saturating_sub(1)` — the caller typically computes
    /// this alongside the groups themselves.
    pub fn new(
        strength: f32,
        falloff: f32,
        dir_groups: Vec<Vec<Vec<usize>>>,
        max_dir_depth: usize,
        enabled: bool,
    ) -> Self {
        Self {
            enabled,
            strength,
            falloff,
            dir_groups,
            max_dir_depth,
        }
    }
}

impl Force for LocationAffinity {
    fn name(&self) -> &str {
        "location_affinity"
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]) {
        if self.strength <= 0.0 || self.dir_groups.is_empty() {
            return;
        }
        let max_d = self.max_dir_depth;
        for (depth, groups_at_depth) in self.dir_groups.iter().enumerate() {
            // Deeper = more specific = stronger. Scale so the deepest
            // level gets full strength and shallower levels decay.
            let level_scale = if max_d > 0 {
                self.falloff.powi((max_d - depth) as i32)
            } else {
                1.0
            };
            let strength = self.strength * level_scale;
            if strength < 1e-6 {
                continue;
            }

            for group in groups_at_depth {
                // Compute centroid of visible nodes in this group.
                let mut centroid = Vec2::ZERO;
                let mut count = 0u32;
                for &idx in group {
                    if ctx.active[idx] {
                        centroid += ctx.positions[idx];
                        count += 1;
                    }
                }
                if count < 2 {
                    continue;
                }
                centroid /= count as f32;

                for &idx in group {
                    if ctx.active[idx] {
                        forces[idx] += (centroid - ctx.positions[idx]) * strength;
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

#[cfg(test)]
mod tests {
    use super::*;
    use core_ir::EdgeKind;
    use std::collections::HashSet;

    fn mk_ctx<'a>(
        positions: &'a [Vec2],
        active: &'a [bool],
        sizes: &'a [Vec2],
        degrees: &'a [f32],
        edge_pairs: &'a [(usize, usize, EdgeKind)],
        visible_edge_kinds: &'a [EdgeKind],
        children_of: &'a [Vec<usize>],
        containers: &'a HashSet<usize>,
        expanded: &'a HashSet<usize>,
        toplevel: &'a HashSet<usize>,
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
    fn empty_groups_is_noop() {
        let positions = vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)];
        let active = vec![true, true];
        let sizes = vec![Vec2::ZERO; 2];
        let degrees = vec![1.0; 2];
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible: Vec<EdgeKind> = vec![];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();

        let ctx = mk_ctx(
            &positions,
            &active,
            &sizes,
            &degrees,
            &edge_pairs,
            &visible,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        LocationAffinity::new(1.0, 0.5, vec![], 0, true).apply(&ctx, &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn nodes_in_same_group_pulled_to_centroid() {
        // Two nodes forming one group, symmetric around the origin.
        let positions = vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)];
        let active = vec![true, true];
        let sizes = vec![Vec2::ZERO; 2];
        let degrees = vec![1.0; 2];
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible: Vec<EdgeKind> = vec![];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();

        let dir_groups = vec![vec![vec![0usize, 1usize]]];
        let ctx = mk_ctx(
            &positions,
            &active,
            &sizes,
            &degrees,
            &edge_pairs,
            &visible,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        // max_dir_depth = 0, so level_scale = 1.0 and strength = 0.5 applies
        // fully. Nodes pulled toward centroid (origin) by 0.5 * displacement.
        LocationAffinity::new(0.5, 0.5, dir_groups, 0, true).apply(&ctx, &mut forces);
        assert_eq!(forces[0], Vec2::new(-5.0, 0.0));
        assert_eq!(forces[1], Vec2::new(5.0, 0.0));
    }

    #[test]
    fn hidden_nodes_ignored_in_centroid_and_application() {
        // Three nodes, node 0 is hidden. Centroid computed from 1 and 2
        // only, and node 0 receives no force despite being listed in
        // the group.
        let positions = vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(-10.0, 0.0),
        ];
        let active = vec![false, true, true];
        let sizes = vec![Vec2::ZERO; 3];
        let degrees = vec![1.0; 3];
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible: Vec<EdgeKind> = vec![];
        let children_of: Vec<Vec<usize>> = vec![vec![]; 3];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();

        let dir_groups = vec![vec![vec![0usize, 1usize, 2usize]]];
        let ctx = mk_ctx(
            &positions,
            &active,
            &sizes,
            &degrees,
            &edge_pairs,
            &visible,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
        );
        let mut forces = vec![Vec2::ZERO; 3];
        LocationAffinity::new(1.0, 0.5, dir_groups, 0, true).apply(&ctx, &mut forces);

        assert_eq!(forces[0], Vec2::ZERO);
        // Centroid of visible = (0, 0).
        assert_eq!(forces[1], Vec2::new(-10.0, 0.0));
        assert_eq!(forces[2], Vec2::new(10.0, 0.0));
    }

    #[test]
    fn falloff_weakens_shallow_levels() {
        // One group at depth 0, one at depth 1; max depth = 1.
        // Shallower (depth 0) level gets strength * falloff^1.
        let positions = vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)];
        let active = vec![true, true];
        let sizes = vec![Vec2::ZERO; 2];
        let degrees = vec![1.0; 2];
        let edge_pairs: Vec<(usize, usize, EdgeKind)> = vec![];
        let visible: Vec<EdgeKind> = vec![];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();

        // Only the shallow group present — force should be scaled by falloff.
        let dir_groups = vec![vec![vec![0usize, 1usize]], vec![]];
        let ctx = mk_ctx(
            &positions,
            &active,
            &sizes,
            &degrees,
            &edge_pairs,
            &visible,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        LocationAffinity::new(1.0, 0.5, dir_groups, 1, true).apply(&ctx, &mut forces);

        // Level scale at depth 0 with max_d=1 is falloff^(1-0) = 0.5.
        // Expected force: 0.5 * (centroid - position) = 0.5 * (-10, 0) = (-5, 0).
        assert_eq!(forces[0], Vec2::new(-5.0, 0.0));
        assert_eq!(forces[1], Vec2::new(5.0, 0.0));
    }
}
