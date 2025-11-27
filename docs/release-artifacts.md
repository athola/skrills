# Release Artifact Naming and Structure

This document details the naming conventions and internal structure for release artifacts. Adhering to these guidelines allows installation scripts to correctly identify and extract the appropriate assets for each target platform.

Install scripts select a release asset whose filename contains the target triple. Therefore, archives should be published following this pattern:

```
skrills-<target>.tar.gz
```

Where `<target>` is one of the supported target triples:
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

Inside each archive, the binary must be placed in the archive root and named `skrills` (or `skrills.exe` for Windows builds).

The default release repository is `athola/skrills`. This can be overridden by setting the `SKRILLS_GH_REPO` environment variable if necessary.
