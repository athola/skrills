//! Agent adapters for reading/writing native configuration formats.

mod claude;
mod codex;
mod codex_toml;
mod copilot;
mod cursor;
pub(crate) mod json_config;
#[cfg(test)]
mod tests_common;
pub mod traits;
pub(crate) mod utils;

pub use claude::ClaudeAdapter;
pub use codex::CodexAdapter;
pub use copilot::CopilotAdapter;
pub use cursor::CursorAdapter;
pub use traits::{AgentAdapter, FieldSupport};
