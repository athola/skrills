//! Command-line interface for the `skrills` application.
//!
//! This is the entry point for the `skrills` executable. It delegates to
//! the `skrills-server` crate.

fn main() -> anyhow::Result<()> {
    skrills_server::run()
}
