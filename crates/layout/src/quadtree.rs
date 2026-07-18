//! Barnes-Hut quadtree for O(n log n) truncated-Coulomb repulsion.
//!
//! Matches the pairwise formula exactly at leaves (`repulsion / d²` within
//! `cutoff`, `min_dist` floor, coincident points skipped) and approximates
//! sufficiently far cells by their center of mass. Cells lying entirely
//! outside the cutoff radius are pruned exactly, so the truncation behaviour
//! of the old grid implementation is preserved.

use glam::Vec2;

/// A cell is approximated as a point mass when `cell_size / distance < θ`.
/// Must stay below 1/√2 ≈ 0.707 so a cell containing the query point itself
/// is always recursed into (its COM can be at most `size·√2/2` away).
const THETA_SQ: f32 = 0.7 * 0.7;

/// Maximum tree depth — bounds subdivision for near-coincident points.
const MAX_DEPTH: u32 = 24;

/// Points a leaf holds before splitting.
const LEAF_CAP: u32 = 8;

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Node {
    /// Cell center.
    center: Vec2,
    /// Cell half extent.
    half: f32,
    /// Sum of member positions during build; center of mass after.
    com: Vec2,
    /// Number of points in this subtree.
    count: u32,
    /// Index of the first of 4 contiguous children, or `NONE` for a leaf.
    children: u32,
    /// Head of the leaf's intrusive point list (`NONE` = empty).
    first_point: u32,
    /// Subdivision depth of this cell.
    depth: u32,
}

/// Flat-array Barnes-Hut quadtree, rebuilt each simulation step.
pub struct QuadTree {
    nodes: Vec<Node>,
    /// Intrusive linked list: `next_point[p]` = next point in the same leaf.
    next_point: Vec<u32>,
}

impl QuadTree {
    /// Build over all non-hidden points.
    pub fn build(positions: &[Vec2], hidden: &[bool]) -> Self {
        let mut mn = Vec2::splat(f32::INFINITY);
        let mut mx = Vec2::splat(f32::NEG_INFINITY);
        for (i, p) in positions.iter().enumerate() {
            if !hidden[i] {
                mn = mn.min(*p);
                mx = mx.max(*p);
            }
        }
        if !mn.x.is_finite() {
            return Self {
                nodes: Vec::new(),
                next_point: Vec::new(),
            };
        }

        let center = (mn + mx) * 0.5;
        let half = ((mx - mn).max_element() * 0.5 + 1.0).max(1.0);
        let mut qt = Self {
            nodes: vec![Node {
                center,
                half,
                com: Vec2::ZERO,
                count: 0,
                children: NONE,
                first_point: NONE,
                depth: 0,
            }],
            next_point: vec![NONE; positions.len()],
        };
        for (i, p) in positions.iter().enumerate() {
            if !hidden[i] {
                qt.insert(i as u32, *p, positions);
            }
        }
        for n in &mut qt.nodes {
            if n.count > 0 {
                n.com /= n.count as f32;
            }
        }
        qt
    }

    fn insert(&mut self, point: u32, pos: Vec2, positions: &[Vec2]) {
        let mut idx = 0usize;
        loop {
            self.nodes[idx].com += pos;
            self.nodes[idx].count += 1;
            let children = self.nodes[idx].children;
            if children != NONE {
                let q = quadrant(self.nodes[idx].center, pos);
                idx = (children + q) as usize;
                continue;
            }
            self.next_point[point as usize] = self.nodes[idx].first_point;
            self.nodes[idx].first_point = point;
            if self.nodes[idx].count > LEAF_CAP && self.nodes[idx].depth < MAX_DEPTH {
                self.split(idx, positions);
            }
            return;
        }
    }

    /// Turn a leaf into an interior node, redistributing its points.
    fn split(&mut self, idx: usize, positions: &[Vec2]) {
        let Node {
            center,
            half,
            depth,
            first_point,
            ..
        } = self.nodes[idx];
        let child_base = self.nodes.len() as u32;
        let h2 = half * 0.5;
        for q in 0..4u32 {
            let offset = Vec2::new(
                if q & 1 == 1 { h2 } else { -h2 },
                if q & 2 == 2 { h2 } else { -h2 },
            );
            self.nodes.push(Node {
                center: center + offset,
                half: h2,
                com: Vec2::ZERO,
                count: 0,
                children: NONE,
                first_point: NONE,
                depth: depth + 1,
            });
        }
        self.nodes[idx].children = child_base;
        self.nodes[idx].first_point = NONE;

        let mut p = first_point;
        while p != NONE {
            let next = self.next_point[p as usize];
            let pos = positions[p as usize];
            let c = (child_base + quadrant(center, pos)) as usize;
            self.nodes[c].com += pos;
            self.nodes[c].count += 1;
            self.next_point[p as usize] = self.nodes[c].first_point;
            self.nodes[c].first_point = p;
            p = next;
        }
    }

    /// Repulsion force on point `i`, matching the pairwise truncated-Coulomb
    /// formula: `repulsion / d²` within `cutoff`, `min_dist` floor.
    pub fn force_at(
        &self,
        i: usize,
        positions: &[Vec2],
        cutoff_sq: f32,
        repulsion: f32,
        min_dist: f32,
    ) -> Vec2 {
        if self.nodes.is_empty() {
            return Vec2::ZERO;
        }
        let pos_i = positions[i];
        let mut force = Vec2::ZERO;

        // DFS with a fixed stack: each pop pushes ≤ 4 children and depth is
        // capped at MAX_DEPTH, so 3·MAX_DEPTH + 4 slots suffice.
        let mut stack = [0u32; 3 * MAX_DEPTH as usize + 4];
        let mut sp = 1usize;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            if n.count == 0 {
                continue;
            }

            // Exact cutoff prune: nearest possible point of the cell box.
            let bx = ((pos_i.x - n.center.x).abs() - n.half).max(0.0);
            let by = ((pos_i.y - n.center.y).abs() - n.half).max(0.0);
            if bx * bx + by * by > cutoff_sq {
                continue;
            }

            if n.children == NONE {
                // Leaf: exact pairwise forces.
                let mut p = n.first_point;
                while p != NONE {
                    let j = p as usize;
                    p = self.next_point[j];
                    if j == i {
                        continue;
                    }
                    let delta = pos_i - positions[j];
                    let dist_sq = delta.length_squared();
                    if dist_sq > cutoff_sq || dist_sq < 1e-10 {
                        continue;
                    }
                    let dist = dist_sq.sqrt().max(min_dist);
                    force += delta.normalize_or_zero() * (repulsion / (dist * dist));
                }
                continue;
            }

            let delta = pos_i - n.com;
            let dist_sq = delta.length_squared();
            let size = n.half * 2.0;
            if size * size < THETA_SQ * dist_sq {
                // Far cell: all members as one point mass at the COM.
                let dist = dist_sq.sqrt().max(min_dist);
                force += delta.normalize_or_zero() * (repulsion * n.count as f32 / (dist * dist));
            } else {
                let c = n.children;
                stack[sp] = c;
                stack[sp + 1] = c + 1;
                stack[sp + 2] = c + 2;
                stack[sp + 3] = c + 3;
                sp += 4;
            }
        }
        force
    }
}

/// Quadrant of `pos` relative to `center`: bit 0 = +x, bit 1 = +y.
fn quadrant(center: Vec2, pos: Vec2) -> u32 {
    (pos.x >= center.x) as u32 | (((pos.y >= center.y) as u32) << 1)
}
