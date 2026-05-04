//! Agent adapters for reading/writing native configuration formats.

mod claude;
mod codex;
mod copilot;
mod cursor;
#[cfg(test)]
mod tests_common;
pub mod traits;
pub(crate) mod utils;

pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use copilot::CopilotAdapter;
pub use cursor::CursorAdapter;
pub use traits::{AgentAdapter, FieldSupport};
