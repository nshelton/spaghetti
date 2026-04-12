//! Gentle pull toward the centroid of all visible nodes.
//!
//! Gravity keeps disconnected components from drifting away to infinity.
//! It is the simplest force in the pipeline: compute the centroid of every
//! active node once per step, then add `(centroid - position) * strength`
//! to each active node's force accumulator.

use glam::Vec2;
use std::any::Any;

use super::{Force, ForceContext};

/// Gentle centroid-attraction force.
///
/// Adding `strength * (centroid - position)` to every active node biases the
/// whole graph toward its own center of mass, preventing disconnected
/// components from drifting apart. A `strength` of `0.0` (or a negative
/// value) makes the force a no-op, as does `enabled == false`.
pub struct Gravity {
    /// Whether this force is currently toggled on in the pipeline.
    pub enabled: bool,
    /// Pull coefficient. Multiplied into `(centroid - position)` per node.
    pub strength: f32,
}

impl Gravity {
    /// Create a new gravity force with the given strength and enabled flag.
    pub fn new(strength: f32, enabled: bool) -> Self {
        Self { enabled, strength }
    }
}

impl Force for Gravity {
    fn name(&self) -> &str {
        "gravity"
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]) {
        // A non-positive strength is an effective no-op and matches the
        // original inline gravity code's `gravity > 0.0` gating.
        if self.strength <= 0.0 {
            return;
        }

        // Centroid of all active (visible) nodes.
        let mut centroid = Vec2::ZERO;
        let mut count = 0u32;
        for (i, &active) in ctx.active.iter().enumerate() {
            if active {
                centroid += ctx.positions[i];
                count += 1;
            }
        }
        if count == 0 {
            return;
        }
        centroid /= count as f32;

        // Pull every active node toward that centroid.
        for (i, force) in forces.iter_mut().enumerate().take(ctx.node_count) {
            if ctx.active[i] {
                *force += (centroid - ctx.positions[i]) * self.strength;
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
        let positions = vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)];
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

        Gravity::new(0.0, true).apply(&ctx, &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn pulls_nodes_toward_centroid() {
        // Two nodes symmetric around origin: centroid is origin, so both get
        // pulled toward origin with equal and opposite forces.
        let positions = vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)];
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

        Gravity::new(0.5, true).apply(&ctx, &mut forces);
        assert_eq!(forces[0], Vec2::new(-5.0, 0.0));
        assert_eq!(forces[1], Vec2::new(5.0, 0.0));
    }

    #[test]
    fn hidden_nodes_are_skipped() {
        // Node 0 is hidden; centroid is computed from nodes 1 and 2 only.
        let positions = vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(-2.0, 0.0),
        ];
        let active = vec![false, true, true];
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

        Gravity::new(1.0, true).apply(&ctx, &mut forces);
        // Hidden node must not receive any force even though it's far away.
        assert_eq!(forces[0], Vec2::ZERO);
        // Centroid = (0, 0); each visible node pulled toward origin by strength=1.
        assert_eq!(forces[1], Vec2::new(-2.0, 0.0));
        assert_eq!(forces[2], Vec2::new(2.0, 0.0));
    }
}
