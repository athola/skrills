//! Custom serde impls reserved for the wire-format crate.
//!
//! # Why this module exists
//!
//! `TASK-003` (cold-window v0.8.0 plan) calls for `#[serde(tag = "kind")]`
//! on every snapshot enum so that a future variant carrying a payload —
//! shipped in v0.9.0 alongside the gRPC follow-up — can be added without
//! a wire-breaking change. Today every variant is unit-only and
//! serializes as a bare lowercase string (e.g. `"warning"`).
//!
//! Adding `#[serde(tag = "kind")]` right now would flip the on-the-wire
//! shape from `"warning"` to `{"kind":"warning"}`, which breaks the
//! cold-window parity test fixture and any persisted dispatcher state.
//! The compromise (per PR #218 review N1) is **read-tolerant
//! deserialization**:
//!
//! - **Serialize** keeps emitting bare strings via the derived impl
//!   (`#[serde(rename_all = "...")]`). Wire format does not change.
//! - **Deserialize** here accepts BOTH the current bare-string form
//!   AND the future `{"kind":"..."}` tagged form, so a v0.9.0 producer
//!   that emits the new shape can be read by a v0.8.x consumer.
//!
//! When v0.9.0 introduces a variant with a payload, the writer side
//! switches to a tagged emission for that variant only; pre-existing
//! unit variants continue to emit bare strings. Both forms round-trip.
//!
//! # Module shape
//!
//! Each enum gets a `Visitor` that handles bare strings and `{"kind":
//! "..."}` maps with one shared discriminator-resolution helper.

use core::fmt;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::Deserialize;

use crate::types::{HealthStatus, HintCategory, ResearchChannel, Severity};

/// Read either a bare string or a `{"kind": "..."}` map and resolve
/// the discriminator into `T` via `from_str`. Forward-compatible with
/// extra fields next to `kind` (they are skipped, mirroring serde's
/// `flatten` discipline for tagged unions).
fn deserialize_kind_or_string<'de, D, T, F>(
    deserializer: D,
    expected: &'static str,
    from_str: F,
    valid: &'static [&'static str],
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: 'static,
    F: Fn(&str) -> Option<T> + 'static,
{
    struct KindVisitor<T, F> {
        expected: &'static str,
        from_str: F,
        valid: &'static [&'static str],
        _marker: core::marker::PhantomData<fn() -> T>,
    }

    impl<'de, T, F> Visitor<'de> for KindVisitor<T, F>
    where
        F: Fn(&str) -> Option<T>,
    {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            write!(
                formatter,
                "{} (string variant or {{\"kind\": ...}})",
                self.expected
            )
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<T, E> {
            (self.from_str)(value).ok_or_else(|| de::Error::unknown_variant(value, self.valid))
        }

        fn visit_string<E: de::Error>(self, value: String) -> Result<T, E> {
            self.visit_str(&value)
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<T, A::Error> {
            let mut kind: Option<String> = None;
            while let Some(key) = map.next_key::<String>()? {
                if key == "kind" {
                    if kind.is_some() {
                        return Err(de::Error::duplicate_field("kind"));
                    }
                    kind = Some(map.next_value()?);
                } else {
                    // Forward-compat: ignore unknown payload fields so a
                    // v0.9.0 variant carrying additional data still
                    // deserializes its discriminator successfully.
                    let _: de::IgnoredAny = map.next_value()?;
                }
            }
            let kind = kind.ok_or_else(|| de::Error::missing_field("kind"))?;
            (self.from_str)(&kind).ok_or_else(|| de::Error::unknown_variant(&kind, self.valid))
        }
    }

    deserializer.deserialize_any(KindVisitor {
        expected,
        from_str,
        valid,
        _marker: core::marker::PhantomData::<fn() -> T>,
    })
}

// ---------- Severity ----------

const SEVERITY_VARIANTS: &[&str] = &["warning", "caution", "advisory", "status"];

fn severity_from_str(s: &str) -> Option<Severity> {
    match s {
        "warning" => Some(Severity::Warning),
        "caution" => Some(Severity::Caution),
        "advisory" => Some(Severity::Advisory),
        "status" => Some(Severity::Status),
        _ => None,
    }
}

impl<'de> Deserialize<'de> for Severity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_kind_or_string(
            deserializer,
            "Severity",
            severity_from_str,
            SEVERITY_VARIANTS,
        )
    }
}

// ---------- HintCategory ----------

const HINT_CATEGORY_VARIANTS: &[&str] =
    &["token", "validation", "redundancy", "sync-drift", "quality"];

fn hint_category_from_str(s: &str) -> Option<HintCategory> {
    match s {
        "token" => Some(HintCategory::Token),
        "validation" => Some(HintCategory::Validation),
        "redundancy" => Some(HintCategory::Redundancy),
        "sync-drift" => Some(HintCategory::SyncDrift),
        "quality" => Some(HintCategory::Quality),
        _ => None,
    }
}

impl<'de> Deserialize<'de> for HintCategory {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_kind_or_string(
            deserializer,
            "HintCategory",
            hint_category_from_str,
            HINT_CATEGORY_VARIANTS,
        )
    }
}

// ---------- ResearchChannel ----------

const RESEARCH_CHANNEL_VARIANTS: &[&str] = &["git-hub", "hacker-news", "lobsters", "paper", "triz"];

fn research_channel_from_str(s: &str) -> Option<ResearchChannel> {
    match s {
        "git-hub" => Some(ResearchChannel::GitHub),
        "hacker-news" => Some(ResearchChannel::HackerNews),
        "lobsters" => Some(ResearchChannel::Lobsters),
        "paper" => Some(ResearchChannel::Paper),
        "triz" => Some(ResearchChannel::Triz),
        _ => None,
    }
}

impl<'de> Deserialize<'de> for ResearchChannel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_kind_or_string(
            deserializer,
            "ResearchChannel",
            research_channel_from_str,
            RESEARCH_CHANNEL_VARIANTS,
        )
    }
}

// ---------- HealthStatus ----------

const HEALTH_STATUS_VARIANTS: &[&str] = &["ok", "warn", "error", "unknown"];

fn health_status_from_str(s: &str) -> Option<HealthStatus> {
    match s {
        "ok" => Some(HealthStatus::Ok),
        "warn" => Some(HealthStatus::Warn),
        "error" => Some(HealthStatus::Error),
        "unknown" => Some(HealthStatus::Unknown),
        _ => None,
    }
}

impl<'de> Deserialize<'de> for HealthStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_kind_or_string(
            deserializer,
            "HealthStatus",
            health_status_from_str,
            HEALTH_STATUS_VARIANTS,
        )
    }
}
