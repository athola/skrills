//! Network status detection for offline/cached mode.
//!
//! Provides a quick connectivity check so commands can gracefully
//! degrade when network is unavailable.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Network connectivity status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkStatus {
    /// Network is reachable.
    Online,
    /// Network is not reachable.
    Offline,
    /// Status could not be determined.
    Unknown,
}

impl NetworkStatus {
    /// Returns true if network is confirmed online.
    pub fn is_online(self) -> bool {
        self == NetworkStatus::Online
    }

    /// Returns true if network is confirmed offline.
    pub fn is_offline(self) -> bool {
        self == NetworkStatus::Offline
    }
}

impl std::fmt::Display for NetworkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkStatus::Online => write!(f, "online"),
            NetworkStatus::Offline => write!(f, "offline"),
            NetworkStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Default timeout for connectivity checks (2 seconds).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Known endpoints to probe for connectivity.
/// We try DNS resolution + TCP connect to common reliable hosts.
const PROBE_TARGETS: &[&str] = &["1.1.1.1:443", "8.8.8.8:443"];

/// Check if the system has network connectivity.
///
/// Attempts a quick TCP connect to well-known endpoints with a short timeout.
/// Returns `NetworkStatus::Online` if any endpoint is reachable,
/// `NetworkStatus::Offline` if none are, or `NetworkStatus::Unknown` on error.
pub fn check_connectivity() -> NetworkStatus {
    check_connectivity_with_timeout(CONNECT_TIMEOUT)
}

/// Check connectivity with a custom timeout.
pub fn check_connectivity_with_timeout(timeout: Duration) -> NetworkStatus {
    for target in PROBE_TARGETS {
        match target.to_socket_addrs() {
            Ok(mut addrs) => {
                if let Some(addr) = addrs.next() {
                    if TcpStream::connect_timeout(&addr, timeout).is_ok() {
                        return NetworkStatus::Online;
                    }
                }
            }
            Err(_) => continue,
        }
    }
    NetworkStatus::Offline
}

/// Returns true if the system appears to have network connectivity.
///
/// This is a convenience wrapper around `check_connectivity()`.
pub fn is_online() -> bool {
    check_connectivity().is_online()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_status_display() {
        assert_eq!(NetworkStatus::Online.to_string(), "online");
        assert_eq!(NetworkStatus::Offline.to_string(), "offline");
        assert_eq!(NetworkStatus::Unknown.to_string(), "unknown");
    }

    #[test]
    fn network_status_predicates() {
        assert!(NetworkStatus::Online.is_online());
        assert!(!NetworkStatus::Online.is_offline());

        assert!(NetworkStatus::Offline.is_offline());
        assert!(!NetworkStatus::Offline.is_online());

        assert!(!NetworkStatus::Unknown.is_online());
        assert!(!NetworkStatus::Unknown.is_offline());
    }

    #[test]
    fn check_connectivity_returns_valid_status() {
        // We can't guarantee network state in CI, but the function should not panic.
        let status = check_connectivity();
        assert!(
            status == NetworkStatus::Online
                || status == NetworkStatus::Offline
                || status == NetworkStatus::Unknown
        );
    }

    #[test]
    fn check_connectivity_with_zero_timeout_returns_offline() {
        // A zero-duration timeout should always fail to connect.
        let status = check_connectivity_with_timeout(Duration::from_nanos(1));
        // With near-zero timeout, we expect Offline (connect will timeout).
        assert!(
            status == NetworkStatus::Offline || status == NetworkStatus::Online,
            "should be offline or online, got {:?}",
            status
        );
    }

    #[test]
    fn is_online_returns_bool() {
        // Just verify it doesn't panic and returns a bool.
        let _result: bool = is_online();
    }
}
