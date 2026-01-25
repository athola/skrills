# Release Artifacts

This document specifies the naming conventions and internal structure for `skrills` release artifacts. These guidelines ensure installation scripts can correctly locate and extract assets for each target platform.

## Naming Convention

Release archives must follow this naming pattern:
`skrills-<target>.<extension>`

- `<target>`: Supported target triple (e.g., `x86_64-unknown-linux-gnu`).
- `<extension>`: `.tar.gz` for Unix-like systems, `.zip` for Windows.

### Supported Targets
- `x88_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

## Archive Structure

The executable binary (`skrills` or `skrills.exe`) must be at the root level of the archive.

## Repository

Releases are hosted at `athola/skrills`. Override this default by setting the `SKRILLS_GH_REPO` environment variable.
