//! Gentle pull toward the centroid of all visible nodes.
//!
//! Gravity keeps disconnected components from drifting to infinity.
//! Each step it computes the centroid of every active node and adds
//! `strength * (centroid - position)` to every active node's force.

use glam::Vec2;
use std::any::Any;

use super::{Force, ForceContext};

/// Gentle centroid-attraction force. A `strength` of `0.0` (or negative)
/// makes `apply` a no-op, as does `enabled == false`.
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
    use crate::forces::test_utils::TestCtx;

    #[test]
    fn disabled_strength_is_noop() {
        let tc = TestCtx::new(vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)]);
        let mut forces = vec![Vec2::ZERO; 2];
        Gravity::new(0.0, true).apply(&tc.view(), &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn pulls_nodes_toward_centroid() {
        // Two nodes symmetric around origin: centroid is origin, so both
        // get pulled toward origin with equal and opposite forces.
        let tc = TestCtx::new(vec![Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)]);
        let mut forces = vec![Vec2::ZERO; 2];
        Gravity::new(0.5, true).apply(&tc.view(), &mut forces);
        assert_eq!(forces[0], Vec2::new(-5.0, 0.0));
        assert_eq!(forces[1], Vec2::new(5.0, 0.0));
    }

    #[test]
    fn hidden_nodes_are_skipped() {
        // Node 0 is hidden; centroid is computed from nodes 1 and 2 only.
        let mut tc = TestCtx::new(vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(-2.0, 0.0),
        ]);
        tc.active[0] = false;
        let mut forces = vec![Vec2::ZERO; 3];
        Gravity::new(1.0, true).apply(&tc.view(), &mut forces);

        assert_eq!(forces[0], Vec2::ZERO);
        // Centroid = (0, 0); each visible node pulled toward origin by strength=1.
        assert_eq!(forces[1], Vec2::new(-2.0, 0.0));
        assert_eq!(forces[2], Vec2::new(2.0, 0.0));
    }
}
