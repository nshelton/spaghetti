//! Containment force: pull the direct children of a container toward
//! their shared centroid.
//!
//! Each non-hidden container with at least two visible children computes
//! the centroid of those children and pulls each child toward it by
//! `strength * (centroid - position)`. Top-level containers (namespaces,
//! translation units) use half strength so class-level containment wins
//! the tug-of-war.

use glam::Vec2;
use std::any::Any;

use super::{Force, ForceContext};

/// Per-container children-toward-centroid force.
pub struct Containment {
    /// Whether this force is currently active.
    pub enabled: bool,
    /// Base strength for non-top-level containers. Top-level containers
    /// (namespaces, translation units) get half of this.
    pub strength: f32,
}

impl Containment {
    /// Create a new containment force with the given parameters.
    pub fn new(strength: f32, enabled: bool) -> Self {
        Self { enabled, strength }
    }
}

impl Force for Containment {
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

        for &c in ctx.containers {
            if !ctx.active[c] {
                continue;
            }
            let children = &ctx.children_of[c];
            if children.len() < 2 {
                continue;
            }
            let mut centroid = Vec2::ZERO;
            let mut count = 0u32;
            for &child in children {
                if ctx.active[child] {
                    centroid += ctx.positions[child];
                    count += 1;
                }
            }
            if count < 2 {
                continue;
            }
            centroid /= count as f32;

            let strength = if ctx.toplevel_containers.contains(&c) {
                self.strength * 0.5
            } else {
                self.strength
            };
            for &child in children {
                if ctx.active[child] {
                    forces[child] += (centroid - ctx.positions[child]) * strength;
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
    use crate::forces::test_utils::TestCtx;

    #[test]
    fn singleton_child_receives_no_force() {
        // Container 0 with a single child 1 needs at least two visible
        // children to produce any force.
        let mut tc = TestCtx::new(vec![Vec2::ZERO, Vec2::new(10.0, 0.0)]);
        tc.children_of[0] = vec![1];
        tc.containers.insert(0);

        let mut forces = vec![Vec2::ZERO; 2];
        Containment::new(1.0, true).apply(&tc.view(), &mut forces);
        assert_eq!(forces, vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn two_children_pulled_to_their_centroid() {
        // Container 0 contains children 1 and 2 symmetric around origin.
        let mut tc = TestCtx::new(vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(-10.0, 0.0),
        ]);
        tc.children_of[0] = vec![1, 2];
        tc.containers.insert(0);

        let mut forces = vec![Vec2::ZERO; 3];
        Containment::new(1.0, true).apply(&tc.view(), &mut forces);

        // Centroid of children = origin. Each child pulled toward origin
        // by 1.0 * displacement.
        assert_eq!(forces[0], Vec2::ZERO);
        assert_eq!(forces[1], Vec2::new(-10.0, 0.0));
        assert_eq!(forces[2], Vec2::new(10.0, 0.0));
    }

    #[test]
    fn toplevel_container_uses_half_strength() {
        let mut tc = TestCtx::new(vec![
            Vec2::ZERO,
            Vec2::new(10.0, 0.0),
            Vec2::new(-10.0, 0.0),
        ]);
        tc.children_of[0] = vec![1, 2];
        tc.containers.insert(0);
        tc.toplevel_containers.insert(0);

        let mut forces = vec![Vec2::ZERO; 3];
        Containment::new(1.0, true).apply(&tc.view(), &mut forces);
        // Half strength → 0.5 * displacement.
        assert_eq!(forces[1], Vec2::new(-5.0, 0.0));
        assert_eq!(forces[2], Vec2::new(5.0, 0.0));
    }

    #[test]
    fn hidden_child_excluded_from_centroid_and_application() {
        // Child 3 is hidden and way off to the side; it must not
        // participate in the centroid or receive a force.
        let mut tc = TestCtx::new(vec![
            Vec2::ZERO,
            Vec2::new(10.0, 0.0),
            Vec2::new(-10.0, 0.0),
            Vec2::new(1000.0, 1000.0),
        ]);
        tc.active[3] = false;
        tc.children_of[0] = vec![1, 2, 3];
        tc.containers.insert(0);

        let mut forces = vec![Vec2::ZERO; 4];
        Containment::new(1.0, true).apply(&tc.view(), &mut forces);

        assert_eq!(forces[3], Vec2::ZERO);
        // Centroid computed from 1 and 2 only = (0, 0).
        assert_eq!(forces[1], Vec2::new(-10.0, 0.0));
        assert_eq!(forces[2], Vec2::new(10.0, 0.0));
    }
}
