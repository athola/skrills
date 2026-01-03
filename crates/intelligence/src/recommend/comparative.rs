//! Comparative evaluation for skill effectiveness and deviation scoring.
//!
//! This module compares actual skill-assisted work against expected outcomes
//! to identify underperforming skills and highlight proven effective ones.

use crate::usage::{SkillUsageEvent, UsageAnalytics};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============================================================================
// Skill Categories and Expected Outcomes
// ============================================================================

/// Functional category of a skill.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SkillCategory {
    /// Testing-related skills (test generation, coverage, assertions).
    Testing,
    /// Debugging and troubleshooting skills.
    Debugging,
    /// Documentation and explanation skills.
    Documentation,
    /// Code refactoring and cleanup skills.
    Refactoring,
    /// Performance optimization skills.
    Performance,
    /// Security-focused skills.
    Security,
    /// General-purpose skills with no specific expectations.
    #[default]
    General,
}

/// Metrics used to measure session outcomes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutcomeMetrics {
    /// Expected/actual session duration in milliseconds.
    pub session_duration_ms: Option<f64>,
    /// Expected/actual retry or error rate (0.0 - 1.0).
    pub retry_rate: Option<f64>,
    /// Keywords indicating success.
    pub success_indicators: Vec<String>,
    /// Keywords indicating failure.
    pub failure_indicators: Vec<String>,
    /// Number of sessions this metric is based on.
    pub sessions_analyzed: usize,
}

/// Expected outcomes for a skill category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedOutcome {
    /// The skill category.
    pub category: SkillCategory,
    /// Baseline metrics for this category.
    pub baseline_metrics: OutcomeMetrics,
}

// ============================================================================
// Deviation and Effectiveness Scoring
// ============================================================================

/// Evidence supporting a deviation score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviationEvidence {
    /// Number of sessions analyzed.
    pub sessions_analyzed: usize,
    /// Actual observed metrics.
    pub actual_metrics: OutcomeMetrics,
    /// Expected baseline metrics.
    pub expected_metrics: OutcomeMetrics,
    /// Specific deviations detected.
    pub deviations: Vec<String>,
}

/// Deviation score comparing actual vs expected outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviationScore {
    /// Skill URI being evaluated.
    pub skill_uri: String,
    /// Deviation value (-1.0 to 1.0, negative = worse than expected).
    pub deviation: f64,
    /// Confidence in the measurement (0.0 - 1.0).
    pub confidence: f64,
    /// Evidence supporting this score.
    pub evidence: DeviationEvidence,
}

/// Effectiveness metric comparing skill-assisted vs non-skill sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectivenessMetric {
    /// Skill URI being evaluated.
    pub skill_uri: String,
    /// Outcomes from sessions that used this skill.
    pub with_skill_outcomes: OutcomeMetrics,
    /// Outcomes from comparable sessions without this skill.
    pub without_skill_outcomes: OutcomeMetrics,
    /// Improvement factor (> 1.0 = improvement, < 1.0 = regression).
    pub improvement_factor: f64,
}

// ============================================================================
// Category Inference
// ============================================================================

/// Keywords used to infer skill categories.
const TESTING_KEYWORDS: &[&str] = &[
    "test", "testing", "tdd", "bdd", "coverage", "spec", "assert", "mock", "pytest", "jest",
];
const DEBUGGING_KEYWORDS: &[&str] = &[
    "debug",
    "debugging",
    "troubleshoot",
    "diagnose",
    "fix",
    "error",
    "bug",
    "trace",
];
const DOCUMENTATION_KEYWORDS: &[&str] = &[
    "doc",
    "docs",
    "documentation",
    "readme",
    "comment",
    "explain",
    "guide",
    "tutorial",
];
const REFACTORING_KEYWORDS: &[&str] = &[
    "refactor",
    "refactoring",
    "clean",
    "cleanup",
    "simplify",
    "restructure",
    "rename",
];
const PERFORMANCE_KEYWORDS: &[&str] = &[
    "perf",
    "performance",
    "optimize",
    "optimization",
    "fast",
    "speed",
    "benchmark",
    "cache",
];
const SECURITY_KEYWORDS: &[&str] = &[
    "security",
    "secure",
    "auth",
    "authentication",
    "vulnerability",
    "audit",
    "penetration",
];

/// Infer the skill category from the skill name/path.
pub fn infer_skill_category(skill_name: &str) -> SkillCategory {
    let lower = skill_name.to_lowercase();

    // Check each category's keywords
    for kw in TESTING_KEYWORDS {
        if lower.contains(kw) {
            return SkillCategory::Testing;
        }
    }
    for kw in DEBUGGING_KEYWORDS {
        if lower.contains(kw) {
            return SkillCategory::Debugging;
        }
    }
    for kw in DOCUMENTATION_KEYWORDS {
        if lower.contains(kw) {
            return SkillCategory::Documentation;
        }
    }
    for kw in REFACTORING_KEYWORDS {
        if lower.contains(kw) {
            return SkillCategory::Refactoring;
        }
    }
    for kw in PERFORMANCE_KEYWORDS {
        if lower.contains(kw) {
            return SkillCategory::Performance;
        }
    }
    for kw in SECURITY_KEYWORDS {
        if lower.contains(kw) {
            return SkillCategory::Security;
        }
    }

    SkillCategory::General
}

/// Get baseline expected outcomes for a skill category.
pub fn get_baseline_expectations(category: SkillCategory) -> ExpectedOutcome {
    let baseline_metrics = match category {
        SkillCategory::Testing => OutcomeMetrics {
            session_duration_ms: Some(120_000.0), // 2 minutes
            retry_rate: Some(0.15),               // 15% retry rate
            success_indicators: vec![
                "tests pass".to_string(),
                "coverage".to_string(),
                "assertions".to_string(),
                "green".to_string(),
            ],
            failure_indicators: vec![
                "tests fail".to_string(),
                "timeout".to_string(),
                "assertion failed".to_string(),
            ],
            sessions_analyzed: 0,
        },
        SkillCategory::Debugging => OutcomeMetrics {
            session_duration_ms: Some(300_000.0), // 5 minutes
            retry_rate: Some(0.25),               // 25% retry rate
            success_indicators: vec![
                "fixed".to_string(),
                "resolved".to_string(),
                "working".to_string(),
                "found the issue".to_string(),
            ],
            failure_indicators: vec![
                "still broken".to_string(),
                "error persists".to_string(),
                "crash".to_string(),
            ],
            sessions_analyzed: 0,
        },
        SkillCategory::Documentation => OutcomeMetrics {
            session_duration_ms: Some(180_000.0), // 3 minutes
            retry_rate: Some(0.10),               // 10% retry rate
            success_indicators: vec![
                "documented".to_string(),
                "examples".to_string(),
                "clear".to_string(),
            ],
            failure_indicators: vec![
                "unclear".to_string(),
                "missing".to_string(),
                "incomplete".to_string(),
            ],
            sessions_analyzed: 0,
        },
        SkillCategory::Refactoring => OutcomeMetrics {
            session_duration_ms: Some(240_000.0), // 4 minutes
            retry_rate: Some(0.20),               // 20% retry rate
            success_indicators: vec![
                "cleaner".to_string(),
                "simplified".to_string(),
                "improved".to_string(),
            ],
            failure_indicators: vec![
                "broke".to_string(),
                "regression".to_string(),
                "more complex".to_string(),
            ],
            sessions_analyzed: 0,
        },
        SkillCategory::Performance => OutcomeMetrics {
            session_duration_ms: Some(360_000.0), // 6 minutes
            retry_rate: Some(0.30),               // 30% retry rate
            success_indicators: vec![
                "faster".to_string(),
                "optimized".to_string(),
                "benchmark".to_string(),
                "improved".to_string(),
            ],
            failure_indicators: vec![
                "slower".to_string(),
                "regression".to_string(),
                "timeout".to_string(),
            ],
            sessions_analyzed: 0,
        },
        SkillCategory::Security => OutcomeMetrics {
            session_duration_ms: Some(300_000.0), // 5 minutes
            retry_rate: Some(0.20),               // 20% retry rate
            success_indicators: vec![
                "secure".to_string(),
                "patched".to_string(),
                "vulnerability fixed".to_string(),
            ],
            failure_indicators: vec![
                "vulnerable".to_string(),
                "exploit".to_string(),
                "insecure".to_string(),
            ],
            sessions_analyzed: 0,
        },
        SkillCategory::General => OutcomeMetrics {
            session_duration_ms: None, // No expectation
            retry_rate: None,          // No expectation
            success_indicators: vec![],
            failure_indicators: vec![],
            sessions_analyzed: 0,
        },
    };

    ExpectedOutcome {
        category,
        baseline_metrics,
    }
}

// ============================================================================
// Deviation Computation
// ============================================================================

/// Minimum sessions required for reliable deviation scoring.
const MIN_SESSIONS_FOR_DEVIATION: usize = 3;

/// Compute deviation score for a skill based on usage analytics.
pub fn compute_deviation_score(
    skill_uri: &str,
    analytics: &UsageAnalytics,
    session_events: &[SkillUsageEvent],
) -> Option<DeviationScore> {
    // Filter events for this skill
    let skill_events: Vec<_> = session_events
        .iter()
        .filter(|e| e.skill_path == skill_uri)
        .collect();

    if skill_events.len() < MIN_SESSIONS_FOR_DEVIATION {
        return None; // Insufficient data
    }

    // Infer category and get baseline
    let category = infer_skill_category(skill_uri);
    let expected = get_baseline_expectations(category);

    // Compute actual metrics from events
    let actual_metrics = compute_actual_metrics(&skill_events, analytics);

    // Calculate deviation components
    let mut deviation_components = Vec::new();
    let mut total_deviation = 0.0;
    let mut component_count = 0;

    // Duration deviation
    if let (Some(expected_duration), Some(actual_duration)) = (
        expected.baseline_metrics.session_duration_ms,
        actual_metrics.session_duration_ms,
    ) {
        let duration_ratio = actual_duration / expected_duration;
        // Longer than expected is slightly negative, much shorter is concerning
        let duration_deviation = if duration_ratio > 2.0 {
            -0.3 // Taking too long
        } else if duration_ratio < 0.3 {
            -0.2 // Suspiciously fast (might be abandoned)
        } else {
            0.1 // Within expected range
        };

        deviation_components.push(format!(
            "Duration ratio: {:.2}x expected (deviation: {:.2})",
            duration_ratio, duration_deviation
        ));
        total_deviation += duration_deviation;
        component_count += 1;
    }

    // Retry rate deviation
    if let (Some(expected_retry), Some(actual_retry)) = (
        expected.baseline_metrics.retry_rate,
        actual_metrics.retry_rate,
    ) {
        let retry_deviation = expected_retry - actual_retry; // Lower retry = positive
        deviation_components.push(format!(
            "Retry rate: {:.1}% (expected {:.1}%, deviation: {:.2})",
            actual_retry * 100.0,
            expected_retry * 100.0,
            retry_deviation
        ));
        total_deviation += retry_deviation;
        component_count += 1;
    }

    // Keyword matching
    let success_matches = count_keyword_matches(
        &actual_metrics.success_indicators,
        &expected.baseline_metrics.success_indicators,
    );
    let failure_matches = count_keyword_matches(
        &actual_metrics.failure_indicators,
        &expected.baseline_metrics.failure_indicators,
    );

    if !expected.baseline_metrics.success_indicators.is_empty() {
        let keyword_deviation = (success_matches as f64 * 0.1) - (failure_matches as f64 * 0.15);
        deviation_components.push(format!(
            "Keywords: {} success, {} failure (deviation: {:.2})",
            success_matches, failure_matches, keyword_deviation
        ));
        total_deviation += keyword_deviation;
        component_count += 1;
    }

    // Normalize deviation to [-1.0, 1.0]
    let normalized_deviation = if component_count > 0 {
        (total_deviation / component_count as f64).clamp(-1.0, 1.0)
    } else {
        0.0
    };

    // Calculate confidence based on sample size
    let confidence = (skill_events.len() as f64 / 20.0).min(1.0);

    Some(DeviationScore {
        skill_uri: skill_uri.to_string(),
        deviation: normalized_deviation,
        confidence,
        evidence: DeviationEvidence {
            sessions_analyzed: skill_events.len(),
            actual_metrics,
            expected_metrics: expected.baseline_metrics,
            deviations: deviation_components,
        },
    })
}

/// Compute actual metrics from skill usage events.
fn compute_actual_metrics(
    events: &[&SkillUsageEvent],
    _analytics: &UsageAnalytics, // Reserved for future behavioral analysis integration
) -> OutcomeMetrics {
    if events.is_empty() {
        return OutcomeMetrics::default();
    }

    // Group by session and calculate duration
    let mut session_durations: HashMap<&str, (u64, u64)> = HashMap::new();
    for event in events {
        let entry = session_durations
            .entry(&event.session_id)
            .or_insert((u64::MAX, 0));
        entry.0 = entry.0.min(event.timestamp);
        entry.1 = entry.1.max(event.timestamp);
    }

    let avg_duration_ms: f64 = if !session_durations.is_empty() {
        session_durations
            .values()
            .map(|(start, end)| (end.saturating_sub(*start) * 1000) as f64)
            .sum::<f64>()
            / session_durations.len() as f64
    } else {
        0.0
    };

    // Estimate retry rate from frequency (higher frequency in same session = retries)
    let total_events = events.len();
    let unique_sessions = session_durations.len();
    let events_per_session = total_events as f64 / unique_sessions.max(1) as f64;
    let retry_rate = ((events_per_session - 1.0) / 5.0).clamp(0.0, 1.0);

    // Collect keywords from prompt contexts
    let mut success_indicators = Vec::new();
    let mut failure_indicators = Vec::new();

    for event in events {
        if let Some(ref context) = event.prompt_context {
            let lower = context.to_lowercase();
            // Check for common success/failure patterns
            if lower.contains("success")
                || lower.contains("pass")
                || lower.contains("working")
                || lower.contains("fixed")
            {
                success_indicators.push(context.chars().take(50).collect());
            }
            if lower.contains("error")
                || lower.contains("fail")
                || lower.contains("broken")
                || lower.contains("crash")
            {
                failure_indicators.push(context.chars().take(50).collect());
            }
        }
    }

    OutcomeMetrics {
        session_duration_ms: Some(avg_duration_ms),
        retry_rate: Some(retry_rate),
        success_indicators,
        failure_indicators,
        sessions_analyzed: unique_sessions,
    }
}

/// Count how many keywords from `actual` match keywords in `expected`.
fn count_keyword_matches(actual: &[String], expected: &[String]) -> usize {
    let mut matches = 0;
    for actual_kw in actual {
        let lower = actual_kw.to_lowercase();
        for expected_kw in expected {
            if lower.contains(&expected_kw.to_lowercase()) {
                matches += 1;
                break;
            }
        }
    }
    matches
}

// ============================================================================
// Effectiveness Computation
// ============================================================================

/// Minimum sessions required for effectiveness comparison.
const MIN_SESSIONS_FOR_EFFECTIVENESS: usize = 5;

/// Compute effectiveness by comparing sessions with and without this skill.
pub fn compute_effectiveness(
    skill_uri: &str,
    analytics: &UsageAnalytics,
    session_events: &[SkillUsageEvent],
) -> Option<EffectivenessMetric> {
    // Collect sessions that used this skill
    let skill_sessions: std::collections::HashSet<_> = session_events
        .iter()
        .filter(|e| e.skill_path == skill_uri)
        .map(|e| e.session_id.as_str())
        .collect();

    // Collect all sessions
    let all_sessions: std::collections::HashSet<_> = session_events
        .iter()
        .map(|e| e.session_id.as_str())
        .collect();

    // Sessions without this skill
    let without_skill_sessions: Vec<_> =
        all_sessions.difference(&skill_sessions).cloned().collect();

    // Need minimum samples for both groups
    if skill_sessions.len() < MIN_SESSIONS_FOR_EFFECTIVENESS
        || without_skill_sessions.len() < MIN_SESSIONS_FOR_EFFECTIVENESS
    {
        return None;
    }

    // Get events for each group
    let with_skill_events: Vec<_> = session_events
        .iter()
        .filter(|e| skill_sessions.contains(e.session_id.as_str()))
        .collect();

    let without_skill_events: Vec<_> = session_events
        .iter()
        .filter(|e| without_skill_sessions.contains(&e.session_id.as_str()))
        .collect();

    // Compute metrics for each group
    let with_skill_metrics = compute_actual_metrics(&with_skill_events, analytics);
    let without_skill_metrics = compute_actual_metrics(&without_skill_events, analytics);

    // Calculate improvement factor
    // Lower retry rate is better, so invert the comparison
    let improvement_factor = if let (Some(with_retry), Some(without_retry)) = (
        with_skill_metrics.retry_rate,
        without_skill_metrics.retry_rate,
    ) {
        if with_retry > 0.0 && without_retry > 0.001 {
            // Both have retries - calculate ratio (higher = better)
            without_retry / with_retry
        } else if with_retry > 0.0 {
            // Baseline has no retries but skill introduces some - slight negative
            0.5
        } else {
            // No retries with skill = good improvement
            1.5
        }
    } else {
        1.0 // No data, assume neutral
    };

    Some(EffectivenessMetric {
        skill_uri: skill_uri.to_string(),
        with_skill_outcomes: with_skill_metrics,
        without_skill_outcomes: without_skill_metrics,
        improvement_factor: improvement_factor.clamp(0.1, 10.0),
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_skill_category_testing() {
        assert_eq!(
            infer_skill_category("pytest-helper"),
            SkillCategory::Testing
        );
        assert_eq!(infer_skill_category("tdd-workflow"), SkillCategory::Testing);
        assert_eq!(
            infer_skill_category("unit-test-generator"),
            SkillCategory::Testing
        );
    }

    #[test]
    fn test_infer_skill_category_debugging() {
        assert_eq!(
            infer_skill_category("debug-assistant"),
            SkillCategory::Debugging
        );
        assert_eq!(
            infer_skill_category("troubleshoot-errors"),
            SkillCategory::Debugging
        );
    }

    #[test]
    fn test_infer_skill_category_general() {
        assert_eq!(
            infer_skill_category("random-helper"),
            SkillCategory::General
        );
        assert_eq!(infer_skill_category("my-skill"), SkillCategory::General);
    }

    #[test]
    fn test_get_baseline_expectations() {
        let testing_baseline = get_baseline_expectations(SkillCategory::Testing);
        assert_eq!(testing_baseline.category, SkillCategory::Testing);
        assert!(testing_baseline
            .baseline_metrics
            .session_duration_ms
            .is_some());
        assert!(testing_baseline.baseline_metrics.retry_rate.is_some());
        assert!(!testing_baseline
            .baseline_metrics
            .success_indicators
            .is_empty());

        let general_baseline = get_baseline_expectations(SkillCategory::General);
        assert_eq!(general_baseline.category, SkillCategory::General);
        assert!(general_baseline
            .baseline_metrics
            .session_duration_ms
            .is_none());
    }

    #[test]
    fn test_count_keyword_matches() {
        let actual = vec!["tests pass".to_string(), "all working".to_string()];
        let expected = vec![
            "pass".to_string(),
            "working".to_string(),
            "green".to_string(),
        ];

        assert_eq!(count_keyword_matches(&actual, &expected), 2);
    }

    #[test]
    fn test_compute_deviation_score_insufficient_data() {
        let analytics = UsageAnalytics::default();
        let events = vec![SkillUsageEvent {
            timestamp: 1000,
            skill_path: "test-skill".to_string(),
            session_id: "s1".to_string(),
            prompt_context: None,
        }];

        let result = compute_deviation_score("test-skill", &analytics, &events);
        assert!(result.is_none()); // Only 1 event, need at least 3
    }

    #[test]
    fn test_compute_deviation_score_with_data() {
        let analytics = UsageAnalytics::default();
        let events = vec![
            SkillUsageEvent {
                timestamp: 1000,
                skill_path: "pytest-helper".to_string(),
                session_id: "s1".to_string(),
                prompt_context: Some("run tests".to_string()),
            },
            SkillUsageEvent {
                timestamp: 2000,
                skill_path: "pytest-helper".to_string(),
                session_id: "s2".to_string(),
                prompt_context: Some("tests pass!".to_string()),
            },
            SkillUsageEvent {
                timestamp: 3000,
                skill_path: "pytest-helper".to_string(),
                session_id: "s3".to_string(),
                prompt_context: Some("all tests passing".to_string()),
            },
        ];

        let result = compute_deviation_score("pytest-helper", &analytics, &events);
        assert!(result.is_some());

        let score = result.unwrap();
        assert_eq!(score.skill_uri, "pytest-helper");
        assert!(score.confidence > 0.0);
        assert!(score.deviation >= -1.0 && score.deviation <= 1.0);
    }

    #[test]
    fn test_compute_effectiveness_insufficient_data() {
        let analytics = UsageAnalytics::default();
        let events = vec![
            SkillUsageEvent {
                timestamp: 1000,
                skill_path: "my-skill".to_string(),
                session_id: "s1".to_string(),
                prompt_context: None,
            },
            SkillUsageEvent {
                timestamp: 2000,
                skill_path: "other-skill".to_string(),
                session_id: "s2".to_string(),
                prompt_context: None,
            },
        ];

        let result = compute_effectiveness("my-skill", &analytics, &events);
        assert!(result.is_none()); // Not enough sessions in either group
    }
}
