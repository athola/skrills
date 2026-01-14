//! Common types shared across intelligence modules.

use serde::{Deserialize, Serialize};

/// Confidence score clamped to [0.0, 1.0] range.
///
/// This newtype ensures confidence values are always valid by clamping
/// any input to the valid range during construction.
///
/// # Examples
///
/// ```
/// use skrills_intelligence::Confidence;
///
/// // Normal values are preserved
/// let c = Confidence::new(0.75);
/// assert_eq!(c.value(), 0.75);
///
/// // Values are clamped to valid range
/// let high = Confidence::new(1.5);
/// assert_eq!(high.value(), 1.0);
///
/// let low = Confidence::new(-0.5);
/// assert_eq!(low.value(), 0.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Confidence(f64);

impl Confidence {
    /// Create a new Confidence, clamping the value to [0.0, 1.0].
    #[must_use]
    pub fn new(value: f64) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    /// Get the inner confidence value.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.0
    }

    /// Create a zero confidence score.
    #[must_use]
    pub fn zero() -> Self {
        Self(0.0)
    }

    /// Create a full confidence score (1.0).
    #[must_use]
    pub fn full() -> Self {
        Self(1.0)
    }
}

impl Default for Confidence {
    fn default() -> Self {
        Self(0.0)
    }
}

impl From<f64> for Confidence {
    fn from(value: f64) -> Self {
        Self::new(value)
    }
}

impl From<Confidence> for f64 {
    fn from(conf: Confidence) -> Self {
        conf.0
    }
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_clamps_high_values() {
        let c = Confidence::new(1.5);
        assert_eq!(c.value(), 1.0);
    }

    #[test]
    fn test_confidence_clamps_low_values() {
        let c = Confidence::new(-0.5);
        assert_eq!(c.value(), 0.0);
    }

    #[test]
    fn test_confidence_preserves_valid_values() {
        let c = Confidence::new(0.75);
        assert_eq!(c.value(), 0.75);
    }

    #[test]
    fn test_confidence_edge_cases() {
        assert_eq!(Confidence::new(0.0).value(), 0.0);
        assert_eq!(Confidence::new(1.0).value(), 1.0);
    }

    #[test]
    fn test_confidence_from_f64() {
        let c: Confidence = 0.5.into();
        assert_eq!(c.value(), 0.5);

        let high: Confidence = 2.0.into();
        assert_eq!(high.value(), 1.0);
    }

    #[test]
    fn test_confidence_serde_roundtrip() {
        let c = Confidence::new(0.85);
        let json = serde_json::to_string(&c).unwrap();
        let parsed: Confidence = serde_json::from_str(&json).unwrap();
        assert_eq!(c, parsed);
    }

    #[test]
    fn test_confidence_default() {
        assert_eq!(Confidence::default().value(), 0.0);
    }

    #[test]
    fn test_confidence_display() {
        let c = Confidence::new(0.756);
        assert_eq!(format!("{}", c), "0.76");
    }

    #[test]
    fn test_confidence_constants() {
        assert_eq!(Confidence::zero().value(), 0.0);
        assert_eq!(Confidence::full().value(), 1.0);
    }

    #[test]
    fn test_confidence_ordering() {
        let low = Confidence::new(0.2);
        let high = Confidence::new(0.8);
        assert!(low < high);
    }
}
