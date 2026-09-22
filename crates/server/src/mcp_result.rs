//! Construction helpers for rmcp result types.
//!
//! rmcp marks [`CallToolResult`] `#[non_exhaustive]`, so it cannot be built
//! with a struct expression from outside the rmcp crate. Its constructors
//! cover the common shapes but none takes structured content alongside
//! caller-chosen text content, which is what nearly every tool here returns.
//! The fields stay public, so setting them after construction is the
//! supported route.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

/// Builds a successful tool result with text content and a structured payload.
pub(crate) fn tool_ok(
    content: Vec<ContentBlock>,
    structured_content: Option<Value>,
) -> CallToolResult {
    let mut result = CallToolResult::success(content);
    result.structured_content = structured_content;
    result
}

/// Builds a tool-level error result with text content and a structured payload.
pub(crate) fn tool_err(
    content: Vec<ContentBlock>,
    structured_content: Option<Value>,
) -> CallToolResult {
    let mut result = CallToolResult::error(content);
    result.structured_content = structured_content;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_ok_marks_the_result_as_not_an_error() {
        let result = tool_ok(vec![ContentBlock::text("done")], Some(json!({"count": 1})));

        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.structured_content, Some(json!({"count": 1})));
    }

    #[test]
    fn tool_err_marks_the_result_as_an_error() {
        let result = tool_err(vec![ContentBlock::text("failed")], None);

        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.structured_content, None);
    }
}
