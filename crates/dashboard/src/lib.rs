//! Terminal UI dashboard for skrills.
//!
//! Provides a real-time terminal dashboard showing:
//! - Discovered skills with validation status
//! - Recent activity and metrics
//! - Sync status across CLI tools
//!
//! # Usage
//!
//! ```no_run
//! use skrills_dashboard::Dashboard;
//! use std::path::PathBuf;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let dashboard = Dashboard::new(vec![])?;
//!     dashboard.run().await
//! }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod app;
mod events;
mod ui;

pub use app::{App, Dashboard};
pub use events::Event;
