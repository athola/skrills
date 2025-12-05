# Rate Limiting Configuration

This document describes the rate limiting functionality in the `skrills` gateway.

## Current Implementation

The gateway uses a **fixed-window** rate limiter with per-IP tracking. This provides basic abuse protection with low overhead.

### Memory Management

The rate limiter uses a background cleanup task to prevent unbounded memory growth:

- **Cleanup Frequency**: Every `window * 2` (default: 120 seconds for a 60-second window)
- **Retention Policy**: Entries are kept for `window * 2` to handle requests near window boundaries
- **Automatic**: Cleanup runs in the background when rate limiting is enabled
- **Implementation**: Located in `crates/gateway/src/rate_limit.rs:82-94, 120-143`

### Configuration

```toml
[rate_limit]
enabled = false  # Rate limiting is disabled by default
max_requests = 100  # Maximum requests per window
window_seconds = 60  # Time window in seconds
```

When enabled, the rate limiter:
- Tracks requests per IP address (using `X-Forwarded-For` if available)
- Returns HTTP 429 with `Retry-After` and `X-RateLimit-*` headers when limits are exceeded
- Automatically cleans up old entries every 120 seconds (for default 60-second window)

---

## Future Enhancements

This section describes planned improvements to the rate limiting system.

## Motivation

Rate limiting is crucial for `skrills` gateway stability. It prevents:
- **Resource Exhaustion**: Abusive or buggy clients from over-consuming CPU, memory, or upstream service quotas.
- **Unfair Resource Allocation**: Unfair resource sharing in multi-tenant gateway environments.
- **Cost Overruns**: Unexpected costs from excessive usage of expensive operations (e.g., embedding generation).

## Recommended Design (Token Bucket)

The **Token Bucket** algorithm is recommended for its effectiveness in allowing traffic bursts, smoothing request rates, and clear mapping to request costs.

**Config sketch**:
```toml
[rate_limit]
enabled = true
algorithm = "token_bucket"

[rate_limit.global]
capacity = 1000
refill_rate = 100

[rate_limit.per_client]
identifier = "api_key" # Specifies the identifier for per-client limits: "api_key" or "ip_address".
capacity = 100
refill_rate = 10

[rate_limit.operations.costs]
"call-tool/autoload-snippet" = 5 # Example: an expensive operation.
"list-tools" = 1 # Example: a relatively cheap operation.
```

## Operational Hooks

- Issue HTTP 429 "Too Many Requests" responses, with `Retry-After` and `X-RateLimit-*` headers.
- Integrate counters for rate limit hits, exposing metrics in Prometheus for alerting.
- Record actor IDs (e.g., API key prefix or IP address), current bucket state, and operation names in audit events for detailed tracking.

## Testing and Rollout

- Deploy with conservative per-client limits. Closely monitor p95 latency and HTTP 429 response rates.
- Test boundary cases, including empty token buckets, burst traffic at window edges, and mixed-cost operations.
- Conduct a staged rollout, protected by a feature flag. Ensure dashboards are implemented and operational before enforcing rate limits.