//! Edge-spring attraction force.
//!
//! Every visible edge acts as a linear spring between its two endpoints.
//! Each endpoint's force is divided by `sqrt(degree)` so hub nodes with
//! many connections don't get yanked across the canvas by the sum of
//! all their edges. Per-edge-kind parameter overrides (`attraction` /
//! `target_distance`) let call and inheritance edges have different
//! stiffnesses from the global fallback.
//!
//! The edge loop is serial and writes directly into the shared `forces`
//! accumulator. Per-edge work is tiny, so even on large graphs the
//! single-threaded cost is a few milliseconds — well under the point
//! where rayon's task-spawn + per-thread buffer allocation overhead
//! starts paying off.

use core_ir::EdgeKind;
use glam::Vec2;
use std::any::Any;
use std::collections::HashMap;

use super::{Force, ForceContext};
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

    /// `(attraction, rest_length)` for an edge kind, falling back to the
    /// global defaults when no override exists.
    #[inline]
    fn params_for(&self, kind: EdgeKind) -> (f32, f32) {
        if let Some(ep) = self.edge_params.get(&kind) {
            (ep.attraction, ep.target_distance)
        } else {
            (self.global_attraction, self.global_ideal_length)
        }
    }

    /// Accumulate one edge's spring contribution into `forces`. Shared
    /// between the serial and parallel paths so the per-edge math stays
    /// identical.
    #[inline]
    fn accumulate_edge(
        &self,
        ctx: &ForceContext,
        forces: &mut [Vec2],
        from: usize,
        to: usize,
        kind: EdgeKind,
    ) {
        if !ctx.visible_edge_kinds.contains(&kind) {
            return;
        }
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

impl Force for SpringAttraction {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply(&self, ctx: &ForceContext, forces: &mut [Vec2]) {
        // Serial single-pass accumulation into the shared `forces`
        // slice. Per-edge work is tiny (~50-100 ns), so even at 200k
        // edges this is <20 ms single-thread — well below the point
        // where rayon's task-spawn + per-thread buffer allocation
        // starts paying off. The previous `par_iter().fold(|| vec![...])`
        // path allocated one N-sized `Vec<Vec2>` per rayon chunk, which
        // at 57k nodes × ~16 chunks was ~16 MB of allocator churn per
        // step.
        for &(from, to, kind) in ctx.edge_pairs {
            self.accumulate_edge(ctx, forces, from, to, kind);
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
    use crate::forces::test_utils::TestCtx;

    fn empty_edge_params() -> HashMap<EdgeKind, EdgeKindParams> {
        HashMap::new()
    }

    #[test]
    fn long_edge_pulls_endpoints_together() {
        // Two nodes 100 units apart with a rest length of 10: the spring
        // should pull them toward each other.
        let mut tc = TestCtx::new(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)]);
        tc.edge_pairs.push((0, 1, EdgeKind::Calls));
        tc.visible_edge_kinds.push(EdgeKind::Calls);

        let mut forces = vec![Vec2::ZERO; 2];
        SpringAttraction::new(0.1, 10.0, 1.0, empty_edge_params(), true)
            .apply(&tc.view(), &mut forces);

        assert!(forces[0].x > 0.0);
        assert!(forces[1].x < 0.0);
        assert!((forces[0] + forces[1]).length() < 1e-4);
    }

    #[test]
    fn short_edge_pushes_endpoints_apart() {
        // Two nodes 2 units apart with a rest length of 20: the spring
        // should push them away from each other.
        let mut tc = TestCtx::new(vec![Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0)]);
        tc.edge_pairs.push((0, 1, EdgeKind::Calls));
        tc.visible_edge_kinds.push(EdgeKind::Calls);

        let mut forces = vec![Vec2::ZERO; 2];
        SpringAttraction::new(0.1, 20.0, 1.0, empty_edge_params(), true)
            .apply(&tc.view(), &mut forces);

        assert!(forces[0].x < 0.0);
        assert!(forces[1].x > 0.0);
    }

    #[test]
    fn invisible_edge_kind_is_skipped() {
        let mut tc = TestCtx::new(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)]);
        tc.edge_pairs.push((0, 1, EdgeKind::Calls));
        // `Calls` is NOT in the visible list — the edge should be ignored.
        tc.visible_edge_kinds.push(EdgeKind::Inherits);

        let mut forces = vec![Vec2::ZERO; 2];
        SpringAttraction::new(0.1, 10.0, 1.0, empty_edge_params(), true)
            .apply(&tc.view(), &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn hidden_endpoint_skips_edge() {
        let mut tc = TestCtx::new(vec![Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)]);
        tc.active[1] = false;
        tc.edge_pairs.push((0, 1, EdgeKind::Calls));
        tc.visible_edge_kinds.push(EdgeKind::Calls);

        let mut forces = vec![Vec2::ZERO; 2];
        SpringAttraction::new(0.1, 10.0, 1.0, empty_edge_params(), true)
            .apply(&tc.view(), &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn edge_kind_override_takes_precedence() {
        // Global params would give zero force (rest_len equals distance).
        // The per-kind override changes the rest length so the spring pulls.
        let mut tc = TestCtx::new(vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)]);
        tc.edge_pairs.push((0, 1, EdgeKind::Inherits));
        tc.visible_edge_kinds.push(EdgeKind::Inherits);

        let mut edge_params = HashMap::new();
        edge_params.insert(
            EdgeKind::Inherits,
            EdgeKindParams {
                target_distance: 2.0,
                attraction: 0.5,
            },
        );

        let mut forces = vec![Vec2::ZERO; 2];
        SpringAttraction::new(0.1, 10.0, 1.0, edge_params, true).apply(&tc.view(), &mut forces);
        assert!(forces[0].x > 0.0);
        assert!(forces[1].x < 0.0);
    }
}
