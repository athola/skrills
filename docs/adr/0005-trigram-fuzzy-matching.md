# ADR 0005: Trigram-Based Fuzzy Matching for Skill Search

- Status: Accepted
- Date: 2025-12-29

## Context

The skrills intelligence crate needs to match user queries to available skills.
The existing keyword-based matching in `crates/intelligence/src/usage/analytics.rs`
has significant limitations:

- Requires exact keyword matches
- No typo tolerance ("databas" won't match "database")
- No substring matching ("test" won't match "testing" as similar)
- No fuzzy matching for abbreviations or variations

This limits discoverability when users don't remember exact skill names or make
minor spelling mistakes in their queries.

**Research origin:** [GitHub Issue #24](https://github.com/athola/skrills/issues/24)
investigated Zoekt (Go-based trigram search engine) as a potential solution.

## Decision

Implement trigram-based similarity matching using the `trigram` Rust crate
(v0.4.x) rather than Zoekt or other alternatives.

### Implementation Details

1. **Dependency**: Add `trigram = "0.4"` to `crates/intelligence/Cargo.toml`

2. **Similarity Module**: Create `crates/intelligence/src/recommend/similarity.rs`
   with functions for:
   - `skill_similarity(skill: &SkillMeta, query: &str) -> f64`
   - `find_similar_skills(skills, query, threshold) -> Vec<Match>`

3. **MCP Tool**: Expose as `skill_search` tool with parameters:
   - `query` (required): Search string
   - `threshold` (optional): Minimum similarity score (default: 0.3)
   - `limit` (optional): Maximum results (default: 10)
   - `include_description` (optional, deferred): Match against descriptions

4. **Integration**: Weight trigram similarity alongside existing signals
   (dependency, usage, context) in `SmartRecommendation` scoring.

## Rationale

### Why `trigram` Crate

| Criterion | `trigram` | Zoekt | tantivy | Custom |
|-----------|-----------|-------|---------|--------|
| Pure Rust | Yes | No (Go) | Yes | Yes |
| Dependencies | 0 | FFI/subprocess | Heavy | 0 |
| Complexity | Simple | Complex | Complex | Medium |
| Performance | O(n*m) | Fast index | Fast index | Varies |
| Maintenance | Low | High | Medium | High |

The `trigram` crate provides:
- PostgreSQL pg_trgm-compatible similarity algorithm
- Simple API: `similarity(a, b) -> f64`
- Zero dependencies
- Apache 2.0 license

### Performance Analysis

| Collection Size | Approach | Estimated Time |
|-----------------|----------|----------------|
| < 100 skills | Direct comparison | < 1ms |
| 100-500 skills | Direct comparison | 1-5ms |
| 500-1000 skills | Consider index | 5-20ms |
| > 1000 skills | Index required | Varies |

Typical skrills deployments have 10-100 skills, well within direct comparison
range. A trigram inverted index can be added later if scaling requires it.

### Why Not Zoekt

Zoekt was the original research target but rejected because:
- Adds Go runtime dependency
- Requires subprocess or HTTP communication
- Complex deployment for simple string similarity
- Overkill for our scale (code search vs. skill name matching)

### Why Not Embedding-Based Search

Semantic search via embeddings was deferred because:
- Requires ML model (significant dependency)
- More complex than needed for skill name matching
- Can be Phase 2 enhancement for description-based search

## Alternatives Considered

1. **Zoekt as a Service**: Run Zoekt as subprocess/service.
   - Rejected: Complex deployment, Go dependency.

2. **tantivy**: Full-text search engine with trigram support.
   - Rejected: Heavy dependency, overkill for our use case.

3. **ngram-search crate**: Trie-based trigram indexing.
   - Rejected: Less mature, more complex API.

4. **Custom implementation**: Build zoekt-style index in Rust.
   - Rejected: Development/maintenance burden when crate solves it.

5. **No fuzzy matching**: Keep exact keyword matching.
   - Rejected: Poor user experience for skill discovery.

## Consequences

### Positive

- Users can find skills with typos or partial names
- Improved skill discoverability
- Simple implementation with minimal dependencies
- Performance suitable for typical deployments

### Negative

- O(n*m) complexity for each query (acceptable at current scale)
- May need indexing if skill collections grow beyond 1000
- `include_description` parameter deferred (tracked in Issue #68)

## Implementation Status

- [x] Add `trigram` dependency
- [x] Create `similarity.rs` module
- [x] Implement `fuzzy_match_skills` function
- [x] Add `skill_search` MCP tool
- [x] Input validation and error handling
- [ ] Description matching (Issue #68)
- [ ] Trigram index for large collections (future)

## Related

- [Issue #24](https://github.com/athola/skrills/issues/24): Original research ticket
- [Issue #68](https://github.com/athola/skrills/issues/68): Description caching enhancement
- [ADR 0004](0004-intelligence-crate-versioning.md): Intelligence crate versioning

## References

- [Zoekt GitHub](https://github.com/sourcegraph/zoekt) - Research target
- [trigram crate](https://crates.io/crates/trigram) - Chosen implementation
- [PostgreSQL pg_trgm](https://www.postgresql.org/docs/current/pgtrgm.html) - Algorithm reference
