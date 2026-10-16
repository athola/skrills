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
/// We try DNS resolution and TCP connect to common reliable hosts.
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
    probe(PROBE_TARGETS, timeout)
}

/// Probes `targets`: `Online` when one accepts a TCP connection, `Offline`
/// when at least one resolved and none accepted, and `Unknown` when none
/// resolved, since then nothing was actually tried.
fn probe(targets: &[&str], timeout: Duration) -> NetworkStatus {
    let mut tried = false;
    for target in targets {
        let addrs = match target.to_socket_addrs() {
            Ok(addrs) => addrs,
            // An unresolvable target was never tried; it decides nothing.
            Err(_) => continue,
        };
        for addr in addrs {
            tried = true;
            if TcpStream::connect_timeout(&addr, timeout).is_ok() {
                return NetworkStatus::Online;
            }
        }
    }
    if tried {
        NetworkStatus::Offline
    } else {
        NetworkStatus::Unknown
    }
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

    /// A reachable address is online.
    #[test]
    fn a_reachable_target_is_online() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target = listener.local_addr().unwrap().to_string();

        let status = probe(&[target.as_str()], Duration::from_secs(2));

        assert_eq!(status, NetworkStatus::Online);
    }

    /// Resolved addresses that all refuse the connection are offline.
    #[test]
    fn resolved_targets_that_refuse_are_offline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target = listener.local_addr().unwrap().to_string();
        drop(listener);

        let status = probe(&[target.as_str()], Duration::from_secs(2));

        assert_eq!(status, NetworkStatus::Offline);
    }

    /// No target resolved, so nothing was probed: the status is unknown,
    /// as the docs say, not offline.
    #[test]
    fn targets_that_do_not_resolve_are_unknown() {
        let status = probe(&["not a socket address"], Duration::from_secs(2));

        assert_eq!(status, NetworkStatus::Unknown);
    }
}
