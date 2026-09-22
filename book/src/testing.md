# Testing and Coverage

Tests are in three places. Unit tests are beside the code in `#[cfg(test)]` modules. Integration tests are under each crate's `tests/` directory: `crates/server/tests/` covers the MCP server, HTTP transport and dashboard, `crates/sync/tests/` covers cross-agent sync, `crates/subagents/tests/` covers the subagent service, and `crates/cli/tests/` drives the real `skrills` binary. Characterization tests for the shell scripts in `scripts/` are under `tests/unit/` and run under pytest.

```bash
make test            # every Rust test in the workspace
make test-unit       # #[cfg(test)] modules only
make test-integration
make test-scripts    # pytest tests/unit; needs pytest on PATH
```

## Covered Areas

- **MCP Server Flow**: Tests cover skill loading, request handling, and response formatting.
- **Validation**: Tests verify skill validation for Claude Code and Codex CLI compatibility.
- **Analysis**: Tests cover token counting and dependency analysis.
- **Sync Operations**: Tests validate cross-agent sync functionality between Claude and Codex.
- **Subagent Integration**: Tests cover subagent service integration and backend communication.
- **Dashboard Navigation**: Tests verify keyboard navigation edge cases including empty skill lists and lazy-loading boundaries.
- **CLI Backend Resolution**: Tests cover `multi-cli-agent` backend selection, binary probing, and fallback ordering.
- **HTTP Transport**: Tests verify port binding, TLS configuration, CORS setup, auth middleware, port fallback exhaustion, and `Host` validation (403 for a host outside the allow-list).
- **CLI Dispatch**: Tests under `crates/cli/tests/` spawn the built binary, which is the only way to catch a defect in `run()` itself, such as a missing dispatch arm or a signal disposition that stops the process waiting on children.
- **Lint Scripts**: pytest cases run each lint in a temporary tree and assert its exit code, including exit 2 when ripgrep itself fails, so a gate cannot fail open.

Shared test utilities are available in the test modules for managing temporary directories, constructing test configurations, and handling test data.

## CI Pipeline Notes

The [integration-tests.yml](.github/workflows/integration-tests.yml) workflow in our CI pipeline runs the test suite with path-based filtering, parallel job execution, cached builds, scheduled daily runs, and Codecov reporting.

## Adding New Coverage

- Prioritize reusing existing test helpers for consistency.
- Include test cases for negative scenarios (e.g., invalid frontmatter, missing fields) as well as happy paths.
- Use `tempfile::tempdir()` for filesystem isolation in tests.
