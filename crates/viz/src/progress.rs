//! Progress overlay state and channel message types for background indexing.

use core_ir::Graph;
use layout::LayoutState;

/// Messages sent from the background indexing thread to the UI.
pub enum ProgressMessage {
    /// Update the status text.
    Status(String),
    /// Update the progress bar (current out of total).
    #[allow(dead_code)]
    Progress { current: usize, total: usize },
    /// Append a log line to the progress overlay.
    Log(String),
    /// Indexing completed successfully.
    Done {
        /// The indexed graph.
        graph: Box<Graph>,
        /// Incremental layout state with pre-computed initial positions.
        layout_state: Box<LayoutState>,
    },
    /// Indexing failed with an error.
    Failed(String),
    /// Indexing was cancelled.
    Cancelled,
}

/// UI-side state for the progress overlay.
pub struct ProgressState {
    /// Current status text shown in the overlay.
    pub status: String,
    /// Current progress count.
    pub current: usize,
    /// Total items (for progress bar), if known.
    pub total: Option<usize>,
    /// Scrollable log messages from the background thread.
    pub messages: Vec<String>,
}

impl ProgressState {
    /// Create a new progress state with an initial status message.
    pub fn new(status: &str) -> Self {
        Self {
            status: status.to_string(),
            current: 0,
            total: None,
            messages: Vec::new(),
        }
    }

    /// Apply a progress message, updating the UI state.
    ///
    /// Returns `true` if the operation is still in progress, `false` if
    /// it has completed (Done, Failed, or Cancelled).
    pub fn apply(&mut self, msg: &ProgressMessage) -> bool {
        match msg {
            ProgressMessage::Status(text) => {
                self.status = text.clone();
                true
            }
            ProgressMessage::Progress { current, total } => {
                self.current = *current;
                self.total = Some(*total);
                true
            }
            ProgressMessage::Log(line) => {
                self.messages.push(line.clone());
                true
            }
            ProgressMessage::Done { .. }
            | ProgressMessage::Failed(_)
            | ProgressMessage::Cancelled => false,
        }
    }

    /// Progress fraction in `[0.0, 1.0]`, or `None` if total is unknown.
    pub fn fraction(&self) -> Option<f32> {
        self.total.map(|t| {
            if t == 0 {
                1.0
            } else {
                self.current as f32 / t as f32
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Channel messages propagate correctly to ProgressState.
    #[test]
    fn progress_state_apply() {
        let mut state = ProgressState::new("Starting…");

        assert!(state.apply(&ProgressMessage::Status("Indexing…".into())));
        assert_eq!(state.status, "Indexing…");

        assert!(state.apply(&ProgressMessage::Progress {
            current: 3,
            total: 10
        }));
        assert_eq!(state.current, 3);
        assert_eq!(state.total, Some(10));
        assert!((state.fraction().unwrap() - 0.3).abs() < f32::EPSILON);

        assert!(state.apply(&ProgressMessage::Log("parsed foo.cpp".into())));
        assert_eq!(state.messages.len(), 1);

        // Done terminates
        let graph = Graph::new();
        let layout_state = LayoutState::new(&graph, 42, layout::ForceParams::default());
        let done = ProgressMessage::Done {
            graph: Box::new(graph),
            layout_state: Box::new(layout_state),
        };
        assert!(!state.apply(&done));
    }

    /// Cancellation signal is observed via channel.
    #[test]
    fn cancellation_signal_observed() {
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();

        // Simulate a worker checking cancellation
        assert!(cancel_rx.try_recv().is_err(), "no cancel signal initially");

        cancel_tx.send(()).expect("send cancel");

        assert!(cancel_rx.try_recv().is_ok(), "cancel signal received");
    }
}
