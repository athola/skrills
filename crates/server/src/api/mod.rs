//! REST API endpoints for the skrills visualization dashboard.

#[cfg(feature = "http-transport")]
pub mod metrics;
#[cfg(feature = "http-transport")]
pub mod skills;

#[cfg(feature = "http-transport")]
pub use metrics::metrics_routes;
#[cfg(feature = "http-transport")]
pub use skills::skills_routes;
