# Release Artifacts

This document specifies the naming conventions and internal structure for `skrills` release artifacts. Following these guidelines is crucial to allow installation scripts to find and extract the correct assets for each target platform.

## Naming Convention

Release archives must follow this naming pattern:
`skrills-<target>.<extension>`

- `<target>` is a supported target triple (e.g., `x86_64-unknown-linux-gnu`), indicating the specific platform.
- `<extension>` will be `.tar.gz` for Unix-like operating systems and `.zip` for Windows environments.

### Supported Targets
- `x88_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

## Archive Structure

The executable binary must be at the root level of the archive and named `skrills` (or `skrills.exe` for Windows systems).

## Repository

The default repository for releases is `athola/skrills`. This default can be overridden by setting the `SKRILLS_GH_REPO` environment variable.
