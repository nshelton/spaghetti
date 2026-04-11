//! Decomposed application state for the spaghetti visualizer.
//!
//! Each sub-module owns a cohesive slice of state, reducing merge conflicts
//! when multiple agents edit different panels concurrently.

pub mod console_state;
pub mod filter_state;
pub mod graph_state;
pub mod indexing;
pub mod interaction;
pub mod render_state;
pub mod simulation;

pub use console_state::ConsoleState;
pub use filter_state::FilterState;
pub use graph_state::GraphState;
pub use indexing::IndexingState;
pub use interaction::InteractionState;
pub use render_state::RenderState;
pub use simulation::SimulationState;
