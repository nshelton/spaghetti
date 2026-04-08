//! 2D camera with pan/zoom and hit-testing.
//!
//! Node size is **fixed in world space** — a node always occupies `NODE_WIDTH × NODE_HEIGHT`
//! world units regardless of zoom. Zoom only changes how large those world units appear on
//! screen.

use core_ir::SymbolId;
use glam::Vec2 as GVec2;
use layout::Positions;

/// Width of a node in world-space units.
pub const NODE_WIDTH: f32 = 120.0;
/// Height of a node in world-space units.
pub const NODE_HEIGHT: f32 = 30.0;

/// Minimum allowed zoom level.
const ZOOM_MIN: f32 = 0.1;
/// Maximum allowed zoom level.
const ZOOM_MAX: f32 = 10.0;

/// 2D camera managing pan (offset) and zoom for the canvas.
pub struct Camera2D {
    /// Pan offset in world-space units.
    pub offset: egui::Vec2,
    /// Zoom factor, clamped to [`ZOOM_MIN`, `ZOOM_MAX`].
    pub zoom: f32,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            offset: egui::Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera2D {
    /// Transform a world position to a screen position.
    ///
    /// `canvas_center` is the pixel center of the drawing area.
    pub fn world_to_screen(&self, world: GVec2, canvas_center: egui::Pos2) -> egui::Pos2 {
        let x = canvas_center.x + (world.x + self.offset.x) * self.zoom;
        let y = canvas_center.y + (world.y + self.offset.y) * self.zoom;
        egui::Pos2::new(x, y)
    }

    /// Transform a screen position back to a world position.
    ///
    /// This is the mathematical inverse of [`world_to_screen`](Self::world_to_screen).
    pub fn screen_to_world(&self, screen: egui::Pos2, canvas_center: egui::Pos2) -> GVec2 {
        let x = (screen.x - canvas_center.x) / self.zoom - self.offset.x;
        let y = (screen.y - canvas_center.y) / self.zoom - self.offset.y;
        GVec2::new(x, y)
    }

    /// Apply a zoom delta, keeping the zoom clamped to [0.1, 10.0].
    pub fn apply_zoom(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    }

    /// Set offset and zoom so all node positions fit within the viewport.
    ///
    /// `viewport_size` is the pixel dimensions of the drawing area. Adds 10%
    /// padding on each side. Does nothing when `positions` is empty.
    pub fn fit_to_bounds(&mut self, positions: &Positions, viewport_size: egui::Vec2) {
        if positions.0.is_empty() {
            return;
        }

        let mut min = GVec2::splat(f32::INFINITY);
        let mut max = GVec2::splat(f32::NEG_INFINITY);
        for pos in positions.0.values() {
            min = min.min(*pos - GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
            max = max.max(*pos + GVec2::new(NODE_WIDTH / 2.0, NODE_HEIGHT / 2.0));
        }

        let center = (min + max) * 0.5;
        let extent = max - min;

        // 10% padding on each side → usable area is 80% of viewport
        let usable = viewport_size * 0.8;
        let zoom = if extent.x > 0.0 && extent.y > 0.0 {
            (usable.x / extent.x).min(usable.y / extent.y)
        } else {
            1.0
        };

        self.zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        self.offset = egui::Vec2::new(-center.x, -center.y);
    }
}

/// Identify which node (if any) is under `pointer_screen`.
///
/// This is a **pure function** — it has no GUI dependencies beyond the position types.
/// Node bounding boxes are axis-aligned rectangles of `NODE_WIDTH × NODE_HEIGHT` in world
/// space.
///
/// Returns `None` when `pointer_screen` is `None` or no node is hit.
pub fn hit_test(
    camera: &Camera2D,
    positions: &Positions,
    pointer_screen: Option<egui::Pos2>,
    canvas_center: egui::Pos2,
) -> Option<SymbolId> {
    let pointer = pointer_screen?;
    let world = camera.screen_to_world(pointer, canvas_center);

    let half_w = NODE_WIDTH / 2.0;
    let half_h = NODE_HEIGHT / 2.0;

    for (id, pos) in &positions.0 {
        if (world.x - pos.x).abs() < half_w && (world.y - pos.y).abs() < half_h {
            return Some(*id);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    /// Helper: canvas center at (400, 300).
    fn center() -> egui::Pos2 {
        egui::Pos2::new(400.0, 300.0)
    }

    /// 1. Roundtrip: world_to_screen then screen_to_world returns the original point.
    #[test]
    fn roundtrip_world_screen_world() {
        let cam = Camera2D {
            offset: egui::Vec2::new(10.0, -20.0),
            zoom: 2.5,
        };
        let original = GVec2::new(42.0, -17.0);
        let screen = cam.world_to_screen(original, center());
        let back = cam.screen_to_world(screen, center());
        assert!((back.x - original.x).abs() < 1e-4, "x mismatch");
        assert!((back.y - original.y).abs() < 1e-4, "y mismatch");
    }

    /// 2. Identity: at zoom=1 offset=0, world (0,0) maps to canvas center.
    #[test]
    fn identity_at_default_camera() {
        let cam = Camera2D::default();
        let screen = cam.world_to_screen(GVec2::ZERO, center());
        assert!((screen.x - center().x).abs() < 1e-4);
        assert!((screen.y - center().y).abs() < 1e-4);
    }

    /// 3. Zoom scaling: doubling zoom doubles the screen offset from center.
    #[test]
    fn zoom_scales_screen_offset() {
        let world = GVec2::new(50.0, 50.0);

        let cam1 = Camera2D {
            zoom: 1.0,
            ..Default::default()
        };
        let cam2 = Camera2D {
            zoom: 2.0,
            ..Default::default()
        };

        let s1 = cam1.world_to_screen(world, center());
        let s2 = cam2.world_to_screen(world, center());

        let dx1 = s1.x - center().x;
        let dx2 = s2.x - center().x;
        assert!((dx2 - 2.0 * dx1).abs() < 1e-4, "x offset should double");

        let dy1 = s1.y - center().y;
        let dy2 = s2.y - center().y;
        assert!((dy2 - 2.0 * dy1).abs() < 1e-4, "y offset should double");
    }

    /// 4. Hit-test hits a node at the node's center.
    #[test]
    fn hit_test_on_node_center() {
        let cam = Camera2D::default();
        let node_world = GVec2::new(100.0, 50.0);
        let id = SymbolId(1);

        let mut map = IndexMap::new();
        map.insert(id, node_world);
        let positions = Positions(map);

        let screen = cam.world_to_screen(node_world, center());
        let result = hit_test(&cam, &positions, Some(screen), center());
        assert_eq!(result, Some(id));
    }

    /// 5. Hit-test misses when pointer is outside the node.
    #[test]
    fn hit_test_miss_outside_node() {
        let cam = Camera2D::default();
        let node_world = GVec2::new(100.0, 50.0);
        let id = SymbolId(1);

        let mut map = IndexMap::new();
        map.insert(id, node_world);
        let positions = Positions(map);

        // Place pointer well outside node bounds (200 units away in world space)
        let far = cam.world_to_screen(GVec2::new(300.0, 50.0), center());
        let result = hit_test(&cam, &positions, Some(far), center());
        assert_eq!(result, None);
    }

    /// 6. Hit-test works correctly at a non-default zoom level.
    #[test]
    fn hit_test_at_zoomed_level() {
        let cam = Camera2D {
            zoom: 3.0,
            ..Default::default()
        };
        let node_world = GVec2::new(0.0, 0.0);
        let id = SymbolId(42);

        let mut map = IndexMap::new();
        map.insert(id, node_world);
        let positions = Positions(map);

        // Slightly inside the node boundary (half_w = 60, so 59 units in world space)
        let near_edge = cam.world_to_screen(GVec2::new(59.0, 14.0), center());
        assert_eq!(
            hit_test(&cam, &positions, Some(near_edge), center()),
            Some(id)
        );

        // Just outside (61 units in world space, past the 60-unit half-width)
        let outside = cam.world_to_screen(GVec2::new(61.0, 0.0), center());
        assert_eq!(hit_test(&cam, &positions, Some(outside), center()), None);
    }

    /// 7. Hit-test returns None when pointer is None.
    #[test]
    fn hit_test_none_pointer() {
        let cam = Camera2D::default();
        let positions = Positions(IndexMap::new());
        assert_eq!(hit_test(&cam, &positions, None, center()), None);
    }

    /// 8. fit_to_bounds centers the graph and sets zoom so all nodes are visible.
    #[test]
    fn fit_to_bounds_centers_and_zooms() {
        let mut cam = Camera2D::default();
        let mut map = IndexMap::new();
        map.insert(SymbolId(1), GVec2::new(-100.0, -50.0));
        map.insert(SymbolId(2), GVec2::new(100.0, 50.0));
        let positions = Positions(map);

        let viewport = egui::Vec2::new(800.0, 600.0);
        cam.fit_to_bounds(&positions, viewport);

        // Center of AABB is (0,0), so offset should be ~(0,0).
        assert!(cam.offset.x.abs() < 1e-4, "offset.x = {}", cam.offset.x);
        assert!(cam.offset.y.abs() < 1e-4, "offset.y = {}", cam.offset.y);

        // Both extremes should map to within the viewport.
        let canvas_center = egui::Pos2::new(400.0, 300.0);
        let s1 = cam.world_to_screen(GVec2::new(-100.0, -50.0), canvas_center);
        let s2 = cam.world_to_screen(GVec2::new(100.0, 50.0), canvas_center);
        assert!(s1.x >= 0.0 && s1.x <= 800.0, "s1.x out of viewport");
        assert!(s2.x >= 0.0 && s2.x <= 800.0, "s2.x out of viewport");
        assert!(s1.y >= 0.0 && s1.y <= 600.0, "s1.y out of viewport");
        assert!(s2.y >= 0.0 && s2.y <= 600.0, "s2.y out of viewport");
    }

    /// 9. fit_to_bounds does nothing on empty positions.
    #[test]
    fn fit_to_bounds_empty_is_noop() {
        let mut cam = Camera2D {
            offset: egui::Vec2::new(5.0, 10.0),
            zoom: 2.0,
        };
        let positions = Positions(IndexMap::new());
        cam.fit_to_bounds(&positions, egui::Vec2::new(800.0, 600.0));
        assert!((cam.zoom - 2.0).abs() < 1e-4);
        assert!((cam.offset.x - 5.0).abs() < 1e-4);
    }
}
