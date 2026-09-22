//! Command-line interface for the `skrills` CLI.
//!
//! This is the main entry point for the `skrills` executable,
//! delegating to [`skrills::run`].

#![deny(unsafe_code)]

fn main() -> anyhow::Result<()> {
    skrills::run()
}
