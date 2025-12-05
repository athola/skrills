# Gateway Testing and Coverage

Integration tests, located under [`crates/gateway/tests`](crates/gateway/tests), cover the entire gateway path, from autoloading to Codex forwarding. These tests are a reference when implementing new features.

## Covered Areas

- **Gateway Flow** ([`gateway_flow_integration.rs`](crates/gateway/tests/gateway_flow_integration.rs)): This suite covers skill loading, context building, caching, concurrency, large request processing, dry-run functionality, and authentication for the readiness endpoint.
- **TLS Validation** ([`tls_validation_integration.rs`](crates/gateway/tests/tls_validation_integration.rs)): Covers critical TLS aspects, including server and client certificate checks, expiration handling, hostname validation, certificate chain processing, and timeouts.
- **Metrics Collection** ([`metrics_collection_integration.rs`](crates/gateway/tests/metrics_collection_integration.rs)): Verifies request and error counters, latency histograms, concurrency accuracy, and metrics endpoint structure.
- **Error Recovery** ([`error_recovery_integration.rs`](crates/gateway/tests/error_recovery_integration.rs)): Focuses on scenarios like invalid skills, Codex API failures, timeouts, network partitions, memory pressure, circuit breaker patterns, and graceful degradation.

Shared fixtures in [`crates/gateway/tests/common/`](crates/gateway/tests/common/) provide helpers for managing temporary directories, constructing configuration builders, setting up mock servers, and handling test data.

## CI Pipeline Notes

The [`integration-tests.yml`](.github/workflows/integration-tests.yml) workflow in our CI pipeline runs the test suite, with path-based filtering, parallel job execution, cached builds, scheduled daily runs, and Codecov reporting. Specific test types can also be triggered on demand.

## Adding New Coverage

- Put general helpers in the `common/` directory and prioritize reusing existing builders for consistency.
- Include test cases for negative scenarios (e.g., timeouts, malformed skills, authentication failures) as well as happy paths.
- Maintain precise metrics assertions, as alerting reliability depends on their stability and accuracy.