//! Settings persistence: load/save layout parameters to disk as JSON.

use std::path::PathBuf;

use layout::ForceParams;
use serde::{Deserialize, Serialize};

/// Persisted application settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    /// Force-directed layout parameters.
    pub force_params: ForceParams,
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
