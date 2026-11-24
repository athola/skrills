//! Network status detection for offline/cached mode.
//!
//! Provides a quick connectivity check so commands can gracefully
//! degrade when network is unavailable.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

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

/// Known endpoints to probe for connectivity. They are IP literals, so the
/// probe needs no DNS: it is a TCP connect to each.
const PROBE_TARGETS: &[&str] = &["1.1.1.1:443", "8.8.8.8:443"];

/// How long one probe answer stands before the next call probes again, so a
/// long-running server that started offline (or hit a blip) recovers, and
/// one that loses the network notices.
pub const PROBE_TTL: Duration = Duration::from_secs(30);

/// Proxy variables that send HTTPS through a proxy. With one set, a direct
/// TCP connect proves nothing about whether requests can succeed.
const PROXY_ENV: &[&str] = &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"];

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
/// skips the probe, for hosts where the probe targets are filtered. Any
/// other value is ignored with a warning.
pub const OFFLINE_ENV: &str = "SKRILLS_OFFLINE";

/// What put the process in offline mode, so an error can say what to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflineSource {
    /// `SKRILLS_OFFLINE` is truthy; `--offline` sets it too.
    Switch,
    /// The connectivity probe reached none of its targets.
    Probe,
}

impl std::fmt::Display for OfflineSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OfflineSource::Switch => write!(f, "{OFFLINE_ENV} or --offline is set"),
            OfflineSource::Probe => write!(
                f,
                "the connectivity probe reached no network (set {OFFLINE_ENV}=0 if that is wrong)"
            ),
        }
    }
}

/// Returns true when network-backed features must not make requests.
///
/// See [`offline_source`].
pub fn is_offline() -> bool {
    offline_source().is_some()
}

/// Why network-backed features must not make requests, or `None` when they
/// may.
///
/// The switch is read on every call. The connectivity probe runs only when
/// the switch decides nothing, is skipped (status unknown) when a proxy is
/// configured, and its answer is reused for [`PROBE_TTL`], so commands that
/// never reach for the network never pay for it. The probe can block for a
/// few seconds, so async callers run this off the async workers.
pub fn offline_source() -> Option<OfflineSource> {
    static PROBED: Mutex<Option<(Instant, NetworkStatus)>> = Mutex::new(None);
    let switch = std::env::var(OFFLINE_ENV).ok();
    offline_source_with(switch.as_deref(), || {
        cached_probe(&PROBED, Instant::now(), PROBE_TTL, || {
            probe_unless_proxied(|name| std::env::var(name).ok(), check_connectivity)
        })
    })
}

/// True only when `SKRILLS_OFFLINE` (or `--offline`) asks for offline mode.
///
/// For requests that may go to a local endpoint, where a failed probe of the
/// public internet proves nothing.
pub fn offline_switch_is_on() -> bool {
    let switch = std::env::var(OFFLINE_ENV).ok();
    parse_switch(switch.as_deref()) == Switch::On
}

#[derive(Debug, PartialEq, Eq)]
enum Switch {
    On,
    Off,
    Unset,
}

fn parse_switch(switch: Option<&str>) -> Switch {
    let Some(raw) = switch.map(str::trim) else {
        return Switch::Unset;
    };
    match raw.to_ascii_lowercase().as_str() {
        "" => Switch::Unset,
        "1" | "true" | "yes" | "on" => Switch::On,
        "0" | "false" | "no" | "off" => Switch::Off,
        _ => {
            static WARNED: Once = Once::new();
            WARNED.call_once(|| {
                tracing::warn!(
                    value = raw,
                    "{OFFLINE_ENV} is not one of 1/true/yes/on or 0/false/no/off; ignoring it"
                );
            });
            Switch::Unset
        }
    }
}

fn offline_source_with(
    switch: Option<&str>,
    probe: impl FnOnce() -> NetworkStatus,
) -> Option<OfflineSource> {
    match parse_switch(switch) {
        Switch::On => Some(OfflineSource::Switch),
        Switch::Off => None,
        // `Unknown` means nothing proved an outage.
        Switch::Unset => probe().is_offline().then_some(OfflineSource::Probe),
    }
}

/// `Unknown` without probing when `lookup` finds a proxy variable, else the
/// result of `probe`.
fn probe_unless_proxied(
    lookup: impl Fn(&str) -> Option<String>,
    probe: impl FnOnce() -> NetworkStatus,
) -> NetworkStatus {
    let proxied = PROXY_ENV
        .iter()
        .any(|name| lookup(name).is_some_and(|v| !v.trim().is_empty()));
    if proxied {
        NetworkStatus::Unknown
    } else {
        probe()
    }
}

/// The answer stored in `cell` when it is younger than `ttl` at `now`,
/// otherwise a fresh `probe`, stored with `now`.
fn cached_probe(
    cell: &Mutex<Option<(Instant, NetworkStatus)>>,
    now: Instant,
    ttl: Duration,
    probe: impl FnOnce() -> NetworkStatus,
) -> NetworkStatus {
    // A panic while probing leaves no half-written answer to refuse.
    let mut slot = cell.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, status)) = *slot {
        if now.saturating_duration_since(at) < ttl {
            return status;
        }
    }
    let status = probe();
    *slot = Some((now, status));
    status
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

    fn is_offline_with(switch: Option<&str>, probe: impl FnOnce() -> NetworkStatus) -> bool {
        offline_source_with(switch, probe).is_some()
    }

    #[test]
    fn offline_switch_truthy_values_force_offline_without_probing() {
        for raw in ["1", "true", "TRUE", "yes", "On"] {
            let mut probed = false;
            let source = offline_source_with(Some(raw), || {
                probed = true;
                NetworkStatus::Online
            });
            assert_eq!(source, Some(OfflineSource::Switch), "{raw:?}");
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
            assert_eq!(
                offline_source_with(raw, || NetworkStatus::Offline),
                Some(OfflineSource::Probe),
                "{raw:?}"
            );
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
    fn the_offline_source_names_what_to_change() {
        assert!(OfflineSource::Switch.to_string().contains("--offline"));
        assert!(OfflineSource::Probe.to_string().contains("probe"));
    }

    /// Replaces `the_probe_runs_once_per_cell`: the answer is reused only
    /// within the TTL, so a process that started offline recovers.
    #[test]
    fn probe_result_expires_after_its_ttl() {
        let cell = Mutex::new(None);
        let start = Instant::now();
        let ttl = Duration::from_secs(30);
        let mut calls = 0;
        let mut probe_at = |at: Instant, answer: NetworkStatus| {
            cached_probe(&cell, at, ttl, || {
                calls += 1;
                answer
            })
        };

        let first = probe_at(start, NetworkStatus::Offline);
        let within = probe_at(start + Duration::from_secs(29), NetworkStatus::Online);
        let after = probe_at(start + ttl, NetworkStatus::Online);

        assert_eq!(
            (first, within, after),
            (
                NetworkStatus::Offline,
                NetworkStatus::Offline,
                NetworkStatus::Online
            )
        );
        assert_eq!(calls, 2);
    }

    /// On a proxy-only network the direct connect fails, which says nothing
    /// about whether a proxied request would succeed.
    #[test]
    fn a_proxy_env_var_makes_the_probe_unknown_not_offline() {
        for proxy_var in PROXY_ENV {
            let mut probed = false;
            let lookup = |name: &str| (name == *proxy_var).then(|| "http://proxy:3128".to_string());
            let status = probe_unless_proxied(lookup, || {
                probed = true;
                NetworkStatus::Offline
            });

            assert_eq!(status, NetworkStatus::Unknown, "{proxy_var}");
            assert!(!probed, "{proxy_var} must skip the direct probe");
            assert!(!is_offline_with(None, || status));
        }
        let unproxied = probe_unless_proxied(|_| None, || NetworkStatus::Offline);
        assert_eq!(unproxied, NetworkStatus::Offline);
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
