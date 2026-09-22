# Dependency Monitoring

This document tracks important dependency considerations and monitoring tasks for the skrills project.

## Dependency: `getrandom` Consolidation

**Status**: Monitoring for v0.4.0
**Impact**: Reduce binary size by ~50KB
**Current State** (as of 2026-09-21):

- rustls version: `0.23.45`
- getrandom versions in tree:
  - `getrandom@0.4.3`: Used by `rand` 0.10, `uuid` and `tempfile`
  - `getrandom@0.3.4`: Used by `rand_core` 0.9
  - `getrandom@0.2.17`: Used by `ring`
- rustls does NOT directly depend on getrandom. It reaches 0.2 through `ring`.

The tree went from two copies to three since the last review, so consolidation
now depends on `ring` and `rand_core` moving, not on rustls alone.

### Monitoring Plan

Track the following for `getrandom` consolidation:

1. **Monitor rustls releases**: Check if newer versions consolidate to single getrandom version
2. **Check hyper-rustls updates**: Version 0.27.9 currently in use
3. **Review tokio-rustls**: Version 0.26.5 currently in use
4. **Binary size impact**: Measure actual size reduction when consolidation occurs

### Action Items

- [ ] Review rustls 0.24+ changelog when available for getrandom consolidation
- [ ] Test binary size before/after any rustls ecosystem updates
- [ ] Update this document when consolidation is achieved

## Monitoring Commands

```bash
# Check current getrandom versions
cargo tree -i getrandom

# Check rustls version and dependencies
cargo tree -p rustls

# Check binary size
ls -lh target/release/skrills
```
