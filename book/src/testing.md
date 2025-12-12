# Testing and Coverage

Integration tests, located under `crates/server/tests/`, cover the MCP server functionality, skill discovery, autoloading, and sync operations.

## Covered Areas

- **MCP Server Flow**: Tests cover skill loading, context building, caching, and request handling.
- **Runtime Tools**: Tests verify runtime options, pinning, and configuration management.
- **Sync Operations**: Tests validate cross-agent sync functionality between Claude and Codex.
- **Subagent Integration**: Tests cover subagent service integration and backend communication.

Shared test utilities are available in the test modules for managing temporary directories, constructing test configurations, and handling test data.

## CI Pipeline Notes

The [integration-tests.yml](.github/workflows/integration-tests.yml) workflow in our CI pipeline runs the test suite with path-based filtering, parallel job execution, cached builds, scheduled daily runs, and Codecov reporting.

## Adding New Coverage

- Prioritize reusing existing test helpers for consistency.
- Include test cases for negative scenarios (e.g., timeouts, malformed skills) as well as happy paths.
- Use `tempfile::tempdir()` for filesystem isolation in tests.
