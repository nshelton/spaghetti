//! Edge-spring attraction force.
//!
//! Every visible edge acts as a linear spring between its two endpoints.
//! The force on each endpoint is divided by `sqrt(degree)` so hub nodes
//! with many connections don't get yanked across the canvas by the sum of
//! all their edges. Per-edge-kind parameter overrides (`attraction` /
//! `target_distance`) let calls and inheritance have different stiffnesses
//! from the global fallback.
//!
//! On large graphs (≥ [`PARALLEL_THRESHOLD`] nodes) the per-edge loop runs
//! under rayon. Multiple edges can touch the same endpoint, so the parallel
//! path accumulates into per-thread local buffers and reduces at the end
//! to avoid data races on the shared `forces` slice.

use core_ir::EdgeKind;
use glam::Vec2;
use rayon::prelude::*;
use std::any::Any;
use std::collections::HashMap;

use super::{Force, ForceContext, PARALLEL_THRESHOLD};
use crate::EdgeKindParams;

/// Linear-spring attraction along visible edges.
pub struct SpringAttraction {
    /// Whether this force is currently active.
    pub enabled: bool,
    /// Global spring coefficient, used when an edge's kind has no override.
    pub global_attraction: f32,
    /// Global rest length, used when an edge's kind has no override.
    pub global_ideal_length: f32,
    /// Floor for pairwise distance to avoid division-by-near-zero.
    pub min_dist: f32,
    /// Per-edge-kind parameter overrides.
    pub edge_params: HashMap<EdgeKind, EdgeKindParams>,
}

impl SpringAttraction {
    /// Create a new spring-attraction force with the given parameters.
    pub fn new(
        global_attraction: f32,
        global_ideal_length: f32,
        min_dist: f32,
        edge_params: HashMap<EdgeKind, EdgeKindParams>,
        enabled: bool,
    ) -> Self {
        Self {
            enabled,
            global_attraction,
            global_ideal_length,
            min_dist,
            edge_params,
        }
    }

    /// Compute the `(attraction, rest_length)` pair for an edge kind,
    /// falling back to the global defaults if there is no override.
    #[inline]
    fn params_for(&self, kind: EdgeKind) -> (f32, f32) {
        if let Some(ep) = self.edge_params.get(&kind) {
            (ep.attraction, ep.target_distance)
        } else {
            (self.global_attraction, self.global_ideal_length)
        }
    }
}

impl Force for SpringAttraction {
    fn name(&self) -> &str {
        "spring_attraction"
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]) {
        if ctx.edge_pairs.is_empty() {
            return;
        }
        let len = ctx.node_count;

        if len >= PARALLEL_THRESHOLD {
            // Parallel path: each rayon worker accumulates into its own
            // thread-local buffer, then we reduce the buffers together.
            // This avoids data races on the shared `forces` slice caused by
            // multiple edges touching the same endpoint.
            let local_forces: Vec<Vec2> = ctx
                .edge_pairs
                .par_iter()
                .fold(
                    || vec![Vec2::ZERO; len],
                    |mut local, &(from, to, kind)| {
                        self.accumulate_edge(ctx, &mut local, from, to, kind);
                        local
                    },
                )
                .reduce(
                    || vec![Vec2::ZERO; len],
                    |mut acc, partial| {
                        for (a, b) in acc.iter_mut().zip(partial.iter()) {
                            *a += *b;
                        }
                        acc
                    },
                );
            for (force, contrib) in forces.iter_mut().zip(local_forces.iter()) {
                *force += *contrib;
            }
        } else {
            // Serial path: write directly into `forces`.
            for &(from, to, kind) in ctx.edge_pairs {
                self.accumulate_edge(ctx, forces, from, to, kind);
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

impl SpringAttraction {
    /// Accumulate one edge's spring contribution into `forces`.
    /// Shared between the serial and parallel paths so the per-edge math
    /// is guaranteed identical.
    #[inline]
    fn accumulate_edge(
        &self,
        ctx: &ForceContext,
        forces: &mut [Vec2],
        from: usize,
        to: usize,
        kind: EdgeKind,
    ) {
        // Edge-kind filter: only edges the UI is rendering exert a force.
        if !ctx.visible_edge_kinds.contains(&kind) {
            return;
        }
        // Hidden endpoints don't participate.
        if !ctx.active[from] || !ctx.active[to] {
            return;
        }

        let (attr, rest_len) = self.params_for(kind);
        let delta = ctx.positions[to] - ctx.positions[from];
        let dist = delta.length().max(self.min_dist);
        let displacement = attr * (dist - rest_len);
        let dir = delta.normalize_or_zero();
        // Scale by 1/sqrt(degree) at each endpoint so hubs stay calm.
        forces[from] += dir * displacement / ctx.degrees[from].sqrt();
        forces[to] -= dir * displacement / ctx.degrees[to].sqrt();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn mk_ctx<'a>(
        positions: &'a [Vec2],
        active: &'a [bool],
        edge_pairs: &'a [(usize, usize, EdgeKind)],
        visible_edge_kinds: &'a [EdgeKind],
        degrees: &'a [f32],
        sizes: &'a [Vec2],
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

    fn empty_edge_params() -> HashMap<EdgeKind, EdgeKindParams> {
        HashMap::new()
    }

    #[test]
    fn long_edge_pulls_endpoints_together() {
        // Two nodes 100 units apart with a rest length of 10: the spring
        // should pull them toward each other.
        let positions = vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)];
        let active = vec![true, true];
        let degrees = vec![1.0; 2];
        let sizes = vec![Vec2::ZERO; 2];
        let edge_pairs = vec![(0usize, 1usize, EdgeKind::Calls)];
        let visible = vec![EdgeKind::Calls];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();

        let ctx = mk_ctx(
            &positions,
            &active,
            &edge_pairs,
            &visible,
            &degrees,
            &sizes,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        SpringAttraction::new(0.1, 10.0, 1.0, empty_edge_params(), true).apply(&ctx, &mut forces);

        // Node 0 (at -50) is pulled toward +x; node 1 (at +50) toward -x.
        assert!(forces[0].x > 0.0);
        assert!(forces[1].x < 0.0);
        // Newton's third law.
        assert!((forces[0] + forces[1]).length() < 1e-4);
    }

    #[test]
    fn short_edge_pushes_endpoints_apart() {
        // Two nodes only 2 units apart with a rest length of 20: spring
        // should push them away from each other.
        let positions = vec![Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0)];
        let active = vec![true, true];
        let degrees = vec![1.0; 2];
        let sizes = vec![Vec2::ZERO; 2];
        let edge_pairs = vec![(0usize, 1usize, EdgeKind::Calls)];
        let visible = vec![EdgeKind::Calls];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();

        let ctx = mk_ctx(
            &positions,
            &active,
            &edge_pairs,
            &visible,
            &degrees,
            &sizes,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        SpringAttraction::new(0.1, 20.0, 1.0, empty_edge_params(), true).apply(&ctx, &mut forces);

        // Node 0 at -1 pushed further in -x; node 1 at +1 pushed in +x.
        assert!(forces[0].x < 0.0);
        assert!(forces[1].x > 0.0);
    }

    #[test]
    fn invisible_edge_kind_is_skipped() {
        let positions = vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)];
        let active = vec![true, true];
        let degrees = vec![1.0; 2];
        let sizes = vec![Vec2::ZERO; 2];
        let edge_pairs = vec![(0usize, 1usize, EdgeKind::Calls)];
        // `Calls` is NOT in the visible list — the edge should be ignored.
        let visible = vec![EdgeKind::Inherits];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();

        let ctx = mk_ctx(
            &positions,
            &active,
            &edge_pairs,
            &visible,
            &degrees,
            &sizes,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        SpringAttraction::new(0.1, 10.0, 1.0, empty_edge_params(), true).apply(&ctx, &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn hidden_endpoint_skips_edge() {
        let positions = vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)];
        // Node 1 is hidden.
        let active = vec![true, false];
        let degrees = vec![1.0; 2];
        let sizes = vec![Vec2::ZERO; 2];
        let edge_pairs = vec![(0usize, 1usize, EdgeKind::Calls)];
        let visible = vec![EdgeKind::Calls];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();

        let ctx = mk_ctx(
            &positions,
            &active,
            &edge_pairs,
            &visible,
            &degrees,
            &sizes,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        SpringAttraction::new(0.1, 10.0, 1.0, empty_edge_params(), true).apply(&ctx, &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn edge_kind_override_takes_precedence() {
        // Global params would give zero force (rest_len equals distance).
        // Per-kind override changes the rest length so the spring pulls.
        let positions = vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)];
        let active = vec![true, true];
        let degrees = vec![1.0; 2];
        let sizes = vec![Vec2::ZERO; 2];
        let edge_pairs = vec![(0usize, 1usize, EdgeKind::Inherits)];
        let visible = vec![EdgeKind::Inherits];
        let children_of: Vec<Vec<usize>> = vec![vec![], vec![]];
        let containers = HashSet::new();
        let expanded = HashSet::new();
        let toplevel = HashSet::new();

        let mut edge_params = HashMap::new();
        edge_params.insert(
            EdgeKind::Inherits,
            EdgeKindParams {
                target_distance: 2.0,
                attraction: 0.5,
            },
        );

        let ctx = mk_ctx(
            &positions,
            &active,
            &edge_pairs,
            &visible,
            &degrees,
            &sizes,
            &children_of,
            &containers,
            &expanded,
            &toplevel,
        );
        let mut forces = vec![Vec2::ZERO; 2];
        // Global rest length = 10, matching the actual distance, so the
        // global params would produce zero force. The Inherits override
        // should pull the endpoints together.
        SpringAttraction::new(0.1, 10.0, 1.0, edge_params, true).apply(&ctx, &mut forces);
        assert!(forces[0].x > 0.0);
        assert!(forces[1].x < 0.0);
    }
}
