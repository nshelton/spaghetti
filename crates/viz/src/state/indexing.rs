//! Background indexing state: progress, channels, file dialog.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crate::progress::{ProgressMessage, ProgressState};

/// State related to background indexing and file opening.
#[derive(Default)]
pub struct IndexingState {
    /// Whether a background indexing operation is in progress.
    pub indexing: bool,
    /// Path to the last opened compile_commands.json (for cache clearing).
    pub compile_commands_path: Option<PathBuf>,
    /// Most-recently-opened project paths (newest first, max 5).
    pub recent_projects: Vec<PathBuf>,
    /// Current progress overlay state.
    pub progress_state: Option<ProgressState>,
    /// Channel receiving progress messages from the indexing thread.
    pub progress_rx: Option<Receiver<ProgressMessage>>,
    /// Channel to send cancellation signal to the indexing thread.
    pub cancel_tx: Option<Sender<()>>,
    /// Channel receiving the result of a native file dialog.
    pub pending_file_dialog: Option<Receiver<Option<PathBuf>>>,
}
