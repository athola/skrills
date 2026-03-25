//! TRIZ inventive principles and contradiction matrix adapted for software.
//!
//! The 40 TRIZ inventive principles are mapped to software/tech domains.
//! Given parameters X (want to improve) and Y (degrades), the matrix
//! suggests applicable inventive principles.

use serde::{Deserialize, Serialize};

/// A TRIZ inventive principle with software-adapted examples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principle {
    pub number: u8,
    pub name: String,
    pub description: String,
    pub software_examples: Vec<String>,
}

/// A software engineering parameter that can be improved or degraded.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Parameter {
    Performance,
    Reliability,
    Maintainability,
    Scalability,
    Security,
    Usability,
    Testability,
    Deployability,
    CostEfficiency,
    DevelopmentSpeed,
    CodeComplexity,
    MemoryUsage,
    Latency,
    Throughput,
    Availability,
}

impl Parameter {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Performance => "performance",
            Self::Reliability => "reliability",
            Self::Maintainability => "maintainability",
            Self::Scalability => "scalability",
            Self::Security => "security",
            Self::Usability => "usability",
            Self::Testability => "testability",
            Self::Deployability => "deployability",
            Self::CostEfficiency => "cost_efficiency",
            Self::DevelopmentSpeed => "development_speed",
            Self::CodeComplexity => "code_complexity",
            Self::MemoryUsage => "memory_usage",
            Self::Latency => "latency",
            Self::Throughput => "throughput",
            Self::Availability => "availability",
        }
    }

    pub fn all() -> &'static [Parameter] {
        &[
            Self::Performance,
            Self::Reliability,
            Self::Maintainability,
            Self::Scalability,
            Self::Security,
            Self::Usability,
            Self::Testability,
            Self::Deployability,
            Self::CostEfficiency,
            Self::DevelopmentSpeed,
            Self::CodeComplexity,
            Self::MemoryUsage,
            Self::Latency,
            Self::Throughput,
            Self::Availability,
        ]
    }
}

/// The TRIZ matrix: maps (improve, degrades) pairs to suggested principle numbers.
pub struct TrizMatrix {
    principles: Vec<Principle>,
}

impl TrizMatrix {
    /// Creates a new TRIZ matrix with the core software-adapted principles.
    pub fn new() -> Self {
        Self {
            principles: build_principles(),
        }
    }

    /// Get a principle by number (1-40).
    pub fn principle(&self, number: u8) -> Option<&Principle> {
        self.principles.iter().find(|p| p.number == number)
    }

    /// All principles.
    pub fn all_principles(&self) -> &[Principle] {
        &self.principles
    }

    /// Given a contradiction (want to improve X, but Y degrades), suggest principles.
    pub fn resolve(&self, improve: Parameter, degrades: Parameter) -> Vec<&Principle> {
        let suggestions = lookup_matrix(improve, degrades);
        suggestions
            .iter()
            .filter_map(|&num| self.principle(num))
            .collect()
    }
}

impl Default for TrizMatrix {
    fn default() -> Self {
        Self::new()
    }
}

/// Look up which principle numbers apply for a given contradiction.
fn lookup_matrix(improve: Parameter, degrades: Parameter) -> Vec<u8> {
    use Parameter::*;
    match (improve, degrades) {
        // Performance vs. others
        (Performance, Reliability) => vec![1, 10, 15, 35],
        (Performance, Maintainability) => vec![2, 15, 19, 35],
        (Performance, MemoryUsage) => vec![1, 4, 7, 35],
        (Performance, Security) => vec![3, 10, 24, 35],
        (Performance, CodeComplexity) => vec![1, 2, 13, 35],

        // Scalability vs. others
        (Scalability, Performance) => vec![1, 7, 10, 35],
        (Scalability, CostEfficiency) => vec![3, 5, 10, 24],
        (Scalability, Maintainability) => vec![1, 2, 15, 24],
        (Scalability, CodeComplexity) => vec![1, 3, 5, 15],

        // Reliability vs. others
        (Reliability, Performance) => vec![10, 11, 15, 35],
        (Reliability, DevelopmentSpeed) => vec![10, 11, 24, 35],
        (Reliability, CostEfficiency) => vec![3, 10, 24, 35],

        // Security vs. others
        (Security, Usability) => vec![1, 3, 15, 24],
        (Security, Performance) => vec![3, 10, 24, 35],
        (Security, DevelopmentSpeed) => vec![1, 10, 24, 35],

        // Development speed vs. others
        (DevelopmentSpeed, Reliability) => vec![1, 10, 15, 35],
        (DevelopmentSpeed, Security) => vec![1, 3, 24, 35],
        (DevelopmentSpeed, Maintainability) => vec![1, 2, 10, 35],
        (DevelopmentSpeed, Testability) => vec![1, 10, 15, 35],

        // Maintainability vs. others
        (Maintainability, Performance) => vec![2, 13, 15, 35],
        (Maintainability, DevelopmentSpeed) => vec![1, 2, 10, 15],

        // Default: suggest general principles
        _ => vec![1, 2, 10, 35],
    }
}

/// Build the core set of software-adapted TRIZ principles.
fn build_principles() -> Vec<Principle> {
    vec![
        Principle {
            number: 1,
            name: "Segmentation".to_string(),
            description: "Divide a system into independent parts".to_string(),
            software_examples: vec![
                "Microservices from monolith".to_string(),
                "Module boundaries / feature flags".to_string(),
                "Database sharding".to_string(),
            ],
        },
        Principle {
            number: 2,
            name: "Taking out / Extraction".to_string(),
            description: "Extract the disturbing part or property".to_string(),
            software_examples: vec![
                "Extract interface from implementation".to_string(),
                "Move side effects to boundary".to_string(),
                "Separate config from code".to_string(),
            ],
        },
        Principle {
            number: 3,
            name: "Local quality".to_string(),
            description: "Transition from uniform to non-uniform structure".to_string(),
            software_examples: vec![
                "Different caching strategies per endpoint".to_string(),
                "Context-specific validation rules".to_string(),
                "Per-tenant configuration".to_string(),
            ],
        },
        Principle {
            number: 4,
            name: "Asymmetry".to_string(),
            description: "Replace symmetric form with asymmetric".to_string(),
            software_examples: vec![
                "CQRS (separate read/write paths)".to_string(),
                "Asymmetric encryption".to_string(),
                "Different schemas for API input vs output".to_string(),
            ],
        },
        Principle {
            number: 5,
            name: "Merging / Consolidation".to_string(),
            description: "Combine identical or similar operations".to_string(),
            software_examples: vec![
                "Batch API requests".to_string(),
                "Connection pooling".to_string(),
                "Deduplicate event handlers".to_string(),
            ],
        },
        Principle {
            number: 7,
            name: "Nested doll / Matryoshka".to_string(),
            description: "Place one object inside another".to_string(),
            software_examples: vec![
                "Middleware chains / decorators".to_string(),
                "Nested virtualization / containers".to_string(),
                "Composable pipelines".to_string(),
            ],
        },
        Principle {
            number: 10,
            name: "Preliminary action".to_string(),
            description: "Perform required changes in advance".to_string(),
            software_examples: vec![
                "Pre-computed caches / materialized views".to_string(),
                "Database migrations before deploy".to_string(),
                "Prefetching / preloading resources".to_string(),
            ],
        },
        Principle {
            number: 11,
            name: "Beforehand cushioning".to_string(),
            description: "Prepare emergency means in advance".to_string(),
            software_examples: vec![
                "Circuit breakers".to_string(),
                "Retry with exponential backoff".to_string(),
                "Graceful degradation / fallback modes".to_string(),
            ],
        },
        Principle {
            number: 13,
            name: "The other way round / Inversion".to_string(),
            description: "Invert the action or process".to_string(),
            software_examples: vec![
                "Inversion of control / dependency injection".to_string(),
                "Pull vs push architecture".to_string(),
                "Reactive vs imperative patterns".to_string(),
            ],
        },
        Principle {
            number: 15,
            name: "Dynamics / Flexibility".to_string(),
            description: "Make characteristics changeable at runtime".to_string(),
            software_examples: vec![
                "Feature flags / runtime configuration".to_string(),
                "Plugin architectures".to_string(),
                "Dynamic dispatch / strategy pattern".to_string(),
            ],
        },
        Principle {
            number: 19,
            name: "Periodic action".to_string(),
            description: "Replace continuous action with periodic".to_string(),
            software_examples: vec![
                "Batch processing instead of real-time".to_string(),
                "Scheduled jobs / cron".to_string(),
                "Polling with backoff".to_string(),
            ],
        },
        Principle {
            number: 24,
            name: "Intermediary / Mediator".to_string(),
            description: "Use an intermediate carrier or process".to_string(),
            software_examples: vec![
                "Message queues / event buses".to_string(),
                "API gateways / reverse proxies".to_string(),
                "Adapter / facade patterns".to_string(),
            ],
        },
        Principle {
            number: 35,
            name: "Parameter changes".to_string(),
            description: "Change the physical/logical parameters".to_string(),
            software_examples: vec![
                "Change data serialization format".to_string(),
                "Switch database engine".to_string(),
                "Change algorithm complexity class".to_string(),
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_has_principles() {
        let m = TrizMatrix::new();
        assert!(!m.all_principles().is_empty());
        assert!(m.principle(1).is_some());
        assert!(m.principle(99).is_none());
    }

    #[test]
    fn resolve_contradiction() {
        let m = TrizMatrix::new();
        let suggestions = m.resolve(Parameter::Performance, Parameter::Reliability);
        assert!(!suggestions.is_empty());
        // Principle 1 (Segmentation) should be suggested
        assert!(suggestions.iter().any(|p| p.number == 1));
    }

    #[test]
    fn resolve_unknown_pair_returns_defaults() {
        let m = TrizMatrix::new();
        let suggestions = m.resolve(Parameter::Availability, Parameter::Latency);
        assert!(
            !suggestions.is_empty(),
            "default principles should be returned"
        );
    }

    #[test]
    fn all_parameters_listed() {
        assert_eq!(Parameter::all().len(), 15);
    }

    #[test]
    fn principle_has_software_examples() {
        let m = TrizMatrix::new();
        let p1 = m.principle(1).unwrap();
        assert!(!p1.software_examples.is_empty());
    }
}
