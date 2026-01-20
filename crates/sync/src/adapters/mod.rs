//! Agent adapters for reading/writing native configuration formats.

mod claude;
mod codex;
mod copilot;
pub mod traits;
pub(crate) mod utils;

pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use copilot::CopilotAdapter;
pub use traits::{AgentAdapter, FieldSupport};
