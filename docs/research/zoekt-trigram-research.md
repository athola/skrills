# Research: Zoekt Trigram Matching for Skrills

**Issue:** [#24](https://github.com/athola/skrills/issues/24)
**Date:** 2025-12-29
**Status:** Research Complete - Implementation Recommended

## Executive Summary

Zoekt is a Go-based code search engine using trigram indexing. While no Rust bindings exist, **pure Rust alternatives provide the same trigram matching capabilities** needed for skrills' skill recommendation system. The `trigram` crate offers PostgreSQL pg_trgm-compatible similarity matching that integrates cleanly with the existing intelligence crate.

**Recommendation:** Implement trigram-based similarity matching using the `trigram` crate to enhance skill recommendations with fuzzy matching, typo tolerance, and substring similarity.

## Current State Analysis

### Existing Keyword Matching (`crates/intelligence/src/usage/analytics.rs`)

The current implementation uses simple keyword extraction:

```rust
fn extract_keywords(prompt: &str) -> Vec<String> {
    prompt
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|s| s.len() >= 3)
        .filter(|s| !is_stop_word(s))
        .map(|s| s.to_string())
        .collect()
}
```

**Limitations:**
- Requires exact keyword matches
- No typo tolerance ("databas" won't match "database")
- No substring matching ("test" won't match "testing" as similar)
- No fuzzy matching for abbreviations or variations

### What Zoekt Provides

Zoekt (Go) offers:
- Trigram-based inverted index for fast substring search
- Regular expression matching
- Symbol-aware ranking
- Multi-repository search

However:
- **No Rust bindings exist**
- Would require FFI, subprocess calls, or running as a separate service
- Overkill for skill name/description matching (designed for code search)

## Rust Alternatives Evaluated

### 1. `trigram` Crate (Recommended)

**Source:** [github.com/ijt/trigram](https://github.com/ijt/trigram)

```rust
use trigram::similarity;

// Returns 0.0 - 1.0 similarity score
similarity("rustacean", "crustacean")  // ~0.7
similarity("color", "colour")          // ~0.44
similarity("database", "databas")      // ~0.8
```

**Pros:**
- Pure Rust, no dependencies
- PostgreSQL pg_trgm compatible
- Simple API: `similarity(a, b) -> f64`
- Includes `find_words_iter()` for fuzzy word search
- Apache 2.0 licensed

**Cons:**
- O(n*m) for each comparison (no index)
- Best for small-to-medium collections

### 2. `ngram-search` Crate

Uses a trie structure for trigram indexing with lookup.

**Pros:**
- Pre-built index for faster repeated queries

**Cons:**
- Less mature
- More complex API

### 3. Tantivy (Full-text Search Engine)

**Pros:**
- Full-featured search engine
- Supports trigram tokenizers

**Cons:**
- Heavy dependency for our use case
- Requires index management
- Overkill for skill matching

### 4. Custom Implementation

Could implement zoekt-style trigram index in Rust.

**Pros:**
- Full control
- Optimized for our use case

**Cons:**
- Development time
- Maintenance burden
- Existing crates solve the problem

## Recommended Approach

### Phase 1: Skill Similarity Matching

Add trigram similarity to `PromptAffinity` matching:

```rust
use trigram::similarity;

const SIMILARITY_THRESHOLD: f64 = 0.3;

fn match_skill_to_prompt(skill_name: &str, prompt_keywords: &[String]) -> f64 {
    prompt_keywords
        .iter()
        .map(|keyword| similarity(skill_name, keyword))
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(0.0)
}
```

### Phase 2: Enhanced Skill Search

Add fuzzy search to the `recommend_skills` flow:

```rust
pub fn fuzzy_match_skills(
    query: &str,
    skills: &[SkillMetadata],
    threshold: f64,
) -> Vec<(SkillMetadata, f64)> {
    skills
        .iter()
        .filter_map(|skill| {
            let name_score = similarity(&skill.name.to_lowercase(), &query.to_lowercase());
            let desc_score = skill.description
                .as_ref()
                .map(|d| find_best_word_match(query, d))
                .unwrap_or(0.0);

            let best_score = name_score.max(desc_score);
            if best_score >= threshold {
                Some((skill.clone(), best_score))
            } else {
                None
            }
        })
        .collect()
}
```

### Phase 3: Trigram Index (Future)

If performance becomes an issue with large skill collections (1000+), implement a trigram inverted index:

```rust
struct TrigramIndex {
    // trigram -> skill IDs containing it
    index: HashMap<[char; 3], HashSet<usize>>,
    skills: Vec<SkillMetadata>,
}

impl TrigramIndex {
    fn search(&self, query: &str) -> Vec<(usize, f64)> {
        // Get candidate skills by trigram intersection
        // Then compute full similarity only on candidates
    }
}
```

## Implementation Plan

### Task 1: Add `trigram` Dependency

```toml
# crates/intelligence/Cargo.toml
[dependencies]
trigram = "0.2"
```

### Task 2: Create Similarity Module

New file: `crates/intelligence/src/similarity.rs`

- `skill_similarity(skill: &SkillMetadata, query: &str) -> f64`
- `find_similar_skills(skills: &[SkillMetadata], query: &str, threshold: f64) -> Vec<Match>`
- Integration with existing `SmartRecommendation` scoring

### Task 3: Enhance Recommendation Engine

Update `crates/intelligence/src/recommendation.rs`:

- Add similarity score as a signal in `SmartRecommendation`
- Weight trigram similarity alongside dependency, usage, and context scores
- Expose fuzzy search through MCP tools

### Task 4: Add Fuzzy Search MCP Tool

New tool: `skill_search`

```json
{
  "name": "skill_search",
  "description": "Fuzzy search for skills by name or description",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string" },
      "threshold": { "type": "number", "default": 0.3 }
    }
  }
}
```

## Performance Considerations

| Collection Size | Approach | Estimated Time |
|-----------------|----------|----------------|
| < 100 skills | Direct `similarity()` | < 1ms |
| 100-500 skills | Direct `similarity()` | 1-5ms |
| 500-1000 skills | Consider trigram index | 5-20ms |
| > 1000 skills | Trigram index required | Varies |

The current skrills use case (typically 10-100 skills) is well within the direct comparison range.

## Alternatives Considered

### Using Zoekt as a Service

**Rejected because:**
- Adds Go runtime dependency
- Requires subprocess or HTTP communication
- Complex deployment for simple string similarity
- Overkill for our scale

### Embedding-Based Semantic Search

**Deferred because:**
- Requires ML model (adds significant dependency)
- More complex than needed for skill name matching
- Could be Phase 4 enhancement for description-based search

## Conclusion

The `trigram` crate provides exactly the functionality needed to enhance skrills' skill recommendations with fuzzy matching. It's:

- Pure Rust with zero dependencies
- Simple API that integrates cleanly
- Performant for our scale (< 100 skills typically)
- Well-tested (pg_trgm compatible)

**Next Steps:**
1. Create implementation branch
2. Add `trigram` dependency
3. Implement similarity module
4. Integrate with recommendation engine
5. Add fuzzy search MCP tool

## References

- [Zoekt GitHub](https://github.com/sourcegraph/zoekt) - Original research target
- [trigram crate](https://crates.io/crates/trigram) - Recommended implementation
- [trigram docs](https://docs.rs/trigram) - API documentation
- [PostgreSQL pg_trgm](https://www.postgresql.org/docs/current/pgtrgm.html) - Algorithm reference
