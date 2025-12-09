//! Agent adapters for reading/writing native configuration formats.

mod claude;
mod codex;
pub mod traits;

pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use traits::{AgentAdapter, FieldSupport};
