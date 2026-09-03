//! Construction helpers for rmcp result types.
//!
//! rmcp 1.x marks [`CallToolResult`] `#[non_exhaustive]`, so it can no longer
//! be built with a struct expression from outside the rmcp crate. Its
//! constructors cover the common shapes but none takes structured content
//! alongside caller-chosen text content, which is what nearly every tool here
//! returns. The fields stay public, so setting them after construction is the
//! supported route.

use rmcp::model::{CallToolResult, Content};
use serde_json::Value;

/// Builds a tool result with both text content and a structured payload.
///
/// `is_error` picks between the tool-level success and error constructors,
/// matching what the call sites previously passed as `is_error: Some(..)`.
pub(crate) fn tool_result(
    content: Vec<Content>,
    structured_content: Option<Value>,
    is_error: bool,
) -> CallToolResult {
    let mut result = if is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    };
    result.structured_content = structured_content;
    result
}
