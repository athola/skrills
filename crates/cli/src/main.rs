//! Command-line interface for the `skrills` application.
//!
//! This crate serves as the main entry point for the executable, delegating
//! its core functionality to the `skrills-core` crate.

fn main() -> anyhow::Result<()> {
    skrills_server::run()
}
