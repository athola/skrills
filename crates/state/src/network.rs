//! Network status detection for offline/cached mode.
//!
//! Provides a quick connectivity check so commands can gracefully
//! degrade when network is unavailable.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::OnceLock;
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

/// Environment switch that overrides the connectivity probe.
///
/// A truthy value (`1`, `true`, `yes`, `on`) forces offline mode. A falsy
/// value (`0`, `false`, `no`, `off`) asserts the network is reachable and
/// skips the probe, for hosts where the probe targets are filtered.
pub const OFFLINE_ENV: &str = "SKRILLS_OFFLINE";

/// Returns true when network-backed features must not make requests.
///
/// The switch is read on every call. The connectivity probe runs only when
/// the switch decides nothing, and at most once per process, so commands that
/// never reach for the network never pay for it.
pub fn is_offline() -> bool {
    static PROBED: OnceLock<NetworkStatus> = OnceLock::new();
    let switch = std::env::var(OFFLINE_ENV).ok();
    is_offline_with(switch.as_deref(), || {
        cached_probe(&PROBED, check_connectivity)
    })
}

fn is_offline_with(switch: Option<&str>, probe: impl FnOnce() -> NetworkStatus) -> bool {
    match switch.map(str::trim) {
        Some(raw)
            if ["1", "true", "yes", "on"]
                .iter()
                .any(|v| raw.eq_ignore_ascii_case(v)) =>
        {
            true
        }
        Some(raw)
            if ["0", "false", "no", "off"]
                .iter()
                .any(|v| raw.eq_ignore_ascii_case(v)) =>
        {
            false
        }
        // `Unknown` means no target resolved: nothing proves an outage.
        _ => probe().is_offline(),
    }
}

fn cached_probe(
    cell: &OnceLock<NetworkStatus>,
    probe: impl FnOnce() -> NetworkStatus,
) -> NetworkStatus {
    *cell.get_or_init(probe)
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

    #[test]
    fn offline_switch_truthy_values_force_offline_without_probing() {
        for raw in ["1", "true", "TRUE", "yes", "On"] {
            let mut probed = false;
            let offline = is_offline_with(Some(raw), || {
                probed = true;
                NetworkStatus::Online
            });
            assert!(offline, "{raw:?} should force offline");
            assert!(!probed, "{raw:?} must not trigger a probe");
        }
    }

    #[test]
    fn offline_switch_falsy_values_force_online_without_probing() {
        for raw in ["0", "false", "FALSE", "no", "Off"] {
            let mut probed = false;
            let offline = is_offline_with(Some(raw), || {
                probed = true;
                NetworkStatus::Offline
            });
            assert!(!offline, "{raw:?} should force online");
            assert!(!probed, "{raw:?} must not trigger a probe");
        }
    }

    #[test]
    fn unset_empty_or_unrecognized_switch_defers_to_the_probe() {
        for raw in [None, Some(""), Some("maybe")] {
            assert!(is_offline_with(raw, || NetworkStatus::Offline), "{raw:?}");
            assert!(!is_offline_with(raw, || NetworkStatus::Online), "{raw:?}");
        }
    }

    /// An undetermined status is not proof of an outage, so a request is
    /// still attempted and fails with its own error.
    #[test]
    fn an_unknown_probe_status_is_not_offline() {
        assert!(!is_offline_with(None, || NetworkStatus::Unknown));
    }

    #[test]
    fn the_probe_runs_once_per_cell() {
        let cell = OnceLock::new();
        let mut calls = 0;
        let first = cached_probe(&cell, || {
            calls += 1;
            NetworkStatus::Offline
        });
        let second = cached_probe(&cell, || {
            calls += 1;
            NetworkStatus::Online
        });
        assert_eq!(
            (first, second, calls),
            (NetworkStatus::Offline, NetworkStatus::Offline, 1)
        );
    }

    #[test]
    fn is_offline_reads_the_environment_on_every_call() {
        let _guard = skrills_test_utils::env_guard();
        {
            let _on = skrills_test_utils::set_env_var(OFFLINE_ENV, Some("1"));
            assert!(is_offline());
        }
        let _off = skrills_test_utils::set_env_var(OFFLINE_ENV, Some("0"));
        assert!(!is_offline());
    }
}
