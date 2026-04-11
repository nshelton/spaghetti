//! Settings persistence: load/save layout and rendering parameters to disk as JSON.

use std::collections::HashMap;
use std::path::PathBuf;

use layout::ForceParams;
use serde::{Deserialize, Serialize};

/// Persisted application settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    /// Force-directed layout parameters.
    pub force_params: ForceParams,
    /// Rendering parameters (colors, circle mode, etc.).
    #[serde(default)]
    pub render: RenderSettings,
}

/// An RGB color stored as `[r, g, b]` for serde compatibility.
pub type Rgb = [u8; 3];

/// Persisted rendering settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderSettings {
    /// Node colors keyed by `SymbolKind` debug name (e.g. "Class", "Method").
    pub node_colors: HashMap<String, Rgb>,
    /// Edge colors keyed by `EdgeKind` debug name (e.g. "Calls", "Inherits").
    pub edge_colors: HashMap<String, Rgb>,
    /// Whether to draw nodes as circles instead of labeled rectangles.
    pub circle_mode: bool,
    /// Circle radius in world-space units.
    pub circle_radius: f32,
    /// Edge opacity (0.0 = invisible, 1.0 = fully opaque).
    #[serde(default = "default_edge_opacity")]
    pub edge_opacity: f32,
    /// Node opacity (0.0 = invisible, 1.0 = fully opaque).
    #[serde(default = "default_node_opacity")]
    pub node_opacity: f32,
}

fn default_edge_opacity() -> f32 {
    0.6
}

fn default_node_opacity() -> f32 {
    1.0
}

impl Default for RenderSettings {
    fn default() -> Self {
        let node_colors = HashMap::from([
            ("Class".into(), [30, 55, 80]),
            ("Struct".into(), [25, 70, 50]),
            ("Function".into(), [80, 45, 25]),
            ("Method".into(), [60, 45, 80]),
            ("Field".into(), [70, 70, 35]),
            ("Namespace".into(), [45, 45, 45]),
            ("TemplateInstantiation".into(), [80, 35, 60]),
            ("TranslationUnit".into(), [35, 35, 35]),
        ]);

        let edge_colors = HashMap::from([
            ("Calls".into(), [220, 180, 80]),
            ("Inherits".into(), [100, 200, 100]),
            ("Contains".into(), [150, 150, 150]),
            ("Overrides".into(), [180, 120, 220]),
            ("ReadsField".into(), [100, 180, 220]),
            ("WritesField".into(), [220, 100, 100]),
            ("Includes".into(), [160, 160, 160]),
            ("Instantiates".into(), [200, 140, 100]),
            ("HasType".into(), [140, 140, 200]),
        ]);

        Self {
            node_colors,
            edge_colors,
            circle_mode: false,
            circle_radius: 5.0,
            edge_opacity: default_edge_opacity(),
            node_opacity: default_node_opacity(),
        }
    }
}

impl RenderSettings {
    /// Look up the node color for a symbol kind, falling back to a default grey.
    pub fn node_color(&self, kind: core_ir::SymbolKind) -> egui::Color32 {
        let key = format!("{kind:?}");
        let rgb = self.node_colors.get(&key).copied().unwrap_or([50, 50, 50]);
        egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }

    /// Look up the edge color for an edge kind, falling back to a default grey.
    pub fn edge_color(&self, kind: core_ir::EdgeKind) -> egui::Color32 {
        let key = format!("{kind:?}");
        let rgb = self
            .edge_colors
            .get(&key)
            .copied()
            .unwrap_or([128, 128, 128]);
        egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }
}

impl AppSettings {
    /// Path to the settings file (next to the binary).
    pub fn path() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("spaghetti_settings.json")))
    }

    /// Load from disk, returning defaults if file is missing or corrupt.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
                tracing::warn!("failed to parse settings: {e}, using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Save to disk. Logs a warning on failure (never panics).
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            tracing::warn!("could not determine settings path");
            return;
        };
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!("failed to write settings: {e}");
                }
            }
            Err(e) => tracing::warn!("failed to serialize settings: {e}"),
        }
    }
}
