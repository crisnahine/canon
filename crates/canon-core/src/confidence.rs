//! When agreement between files becomes a convention, and when it stays noise.

use serde::{Deserialize, Serialize};

/// How strongly the repository agrees with a claim, in the range 0.5 to 1.0.
///
/// Constructed only through [`Confidence::derive`] and
/// [`Confidence::derive_counted`], both of which return `None` rather than a
/// weak value. There is deliberately no `Confidence::new`: every value in the
/// system has passed the sample-size and agreement gates.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Confidence(f32);

impl Confidence {
    /// Below this ratio the majority is not a convention, it is a coin flip
    /// with a lean. Set at four in five: three in five is a real split that
    /// two new files could reverse.
    pub const FLOOR: f32 = 0.8;

    /// Sample size below which agreement is meaningless.
    ///
    /// Three files that match are frequently one file copied twice. Five is the
    /// point where a shared shape is more likely deliberate than incidental.
    pub const MIN_FILES: usize = 5;

    /// Agreement strong enough that a deterministic check may block a write.
    ///
    /// Total agreement only. Anything less has a counterexample already in the
    /// repository, and blocking a write that matches an existing file is the
    /// fastest way to get a tool uninstalled.
    pub const BLOCKING: f32 = 1.0;

    /// Build from a weighted majority over a sample, or refuse.
    ///
    /// `agreeing` and `total` are recency-weighted sums, not counts, so a file
    /// touched last week outvotes one untouched since 2019. `sample` is the raw
    /// file count and gates on [`Self::MIN_FILES`] before any weighting.
    ///
    /// Returns `None` when the sample is too small, the weights are degenerate,
    /// or agreement falls below [`Self::FLOOR`].
    #[must_use]
    pub fn derive_counted(agreeing: f32, total: f32, sample: usize) -> Option<Self> {
        if sample < Self::MIN_FILES || total <= 0.0 || !agreeing.is_finite() || !total.is_finite() {
            return None;
        }
        let ratio = agreeing / total;
        if (Self::required_for(sample)..=1.0).contains(&ratio) { Some(Self(ratio)) } else { None }
    }

    /// The agreement a sample of this size has to clear.
    ///
    /// The floor is a floor for a directory, not for a repository. A rule over
    /// thirty files that holds four times in five is a real local habit. The
    /// same ratio over four thousand files is eight hundred counterexamples,
    /// and it is what a rule looks like when it has been derived from one kind
    /// of file and applied to every kind.
    ///
    /// Measured: `shape.public-arity` over every `.rb` file in a Rails API sat
    /// at 3232/3957, a hair above the floor. It came from single-purpose
    /// service objects and was then applied to models, controllers and
    /// migrations, where it produced the only complaint on a migration written
    /// with the `up`/`down` pair the framework asks for.
    #[must_use]
    pub fn required_for(sample: usize) -> f32 {
        match sample {
            0..=50 => Self::FLOOR,
            51..=500 => 0.85,
            501..=5_000 => 0.90,
            _ => 0.95,
        }
    }

    /// Build from unweighted counts. Convenience for facts with no recency
    /// dimension, such as directory layout.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn derive(agreeing: usize, total: usize) -> Option<Self> {
        Self::derive_counted(agreeing as f32, total as f32, total)
    }

    /// The underlying ratio.
    #[must_use]
    pub fn value(self) -> f32 {
        self.0
    }

    /// Whether this convention may ever block a write rather than advise.
    ///
    /// Necessary but not sufficient: the check must also be deterministic. See
    /// [`crate::Enforcement`].
    #[must_use]
    pub fn is_blocking_grade(self) -> bool {
        self.0 >= Self::BLOCKING
    }

    /// Two decimal places, for rendering into an injected block.
    #[must_use]
    pub fn render(self) -> String {
        format!("{:.2}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_agreement_over_a_large_sample_is_confident() {
        let c = Confidence::derive(52, 52).unwrap();
        assert_eq!(c.value(), 1.0);
        assert!(c.is_blocking_grade());
    }

    #[test]
    fn a_sample_smaller_than_the_gate_never_produces_a_convention() {
        for n in 0..Confidence::MIN_FILES {
            assert!(Confidence::derive(n, n).is_none(), "{n} files must not be a convention");
        }
        assert!(Confidence::derive(Confidence::MIN_FILES, Confidence::MIN_FILES).is_some());
    }

    #[test]
    fn a_bare_majority_is_not_a_convention() {
        assert!(Confidence::derive(6, 10).is_none());
        assert!(Confidence::derive(7, 10).is_none());
        assert!(Confidence::derive(8, 10).is_some(), "the floor is inclusive");
    }

    #[test]
    fn near_total_agreement_is_not_blocking_grade() {
        // 51 of 52 means a counterexample exists in the repo. Advisory only.
        let c = Confidence::derive(51, 52).unwrap();
        assert!(!c.is_blocking_grade());
    }

    #[test]
    fn recency_weight_can_carry_a_minority_of_files() {
        // Four recent files outweigh six stale ones. The sample gate still
        // applies to the raw count, which is why `sample` is passed separately.
        assert!(Confidence::derive_counted(9.0, 10.0, 10).is_some());
        assert!(Confidence::derive_counted(9.0, 10.0, 4).is_none());
    }

    #[test]
    fn degenerate_weights_are_refused_rather_than_producing_nan() {
        assert!(Confidence::derive_counted(0.0, 0.0, 10).is_none());
        assert!(Confidence::derive_counted(f32::NAN, 10.0, 10).is_none());
        assert!(Confidence::derive_counted(1.0, f32::INFINITY, 10).is_none());
        // Agreement above the total is a caller bug; refuse rather than clamp,
        // so the bug surfaces as a missing convention instead of a false one.
        assert!(Confidence::derive_counted(11.0, 10.0, 10).is_none());
    }

    #[test]
    fn a_wide_scope_has_to_clear_a_higher_bar_than_a_narrow_one() {
        // Issue #8. The widest scopes cleared the floor by the smallest margin
        // and produced the most messages. 0.81 over four thousand files is
        // eight hundred counterexamples.
        assert!(Confidence::derive(24, 30).is_some(), "0.80 over 30 files stands");
        assert!(Confidence::derive(3232, 3957).is_none(), "0.81 over 3957 must not");
        assert!(Confidence::derive(3800, 3957).is_some(), "0.96 over 3957 does");
    }

    #[test]
    fn the_bar_rises_with_the_sample_and_never_exceeds_total_agreement() {
        let bars: Vec<f32> =
            [10, 100, 1_000, 10_000].iter().map(|n| Confidence::required_for(*n)).collect();
        for pair in bars.windows(2) {
            assert!(pair[1] >= pair[0], "the bar must not fall as the sample grows: {bars:?}");
        }
        assert!(bars.iter().all(|b| *b <= 1.0));
    }

    #[test]
    fn rendering_is_two_decimals() {
        assert_eq!(Confidence::derive(47, 52).unwrap().render(), "0.90");
        assert_eq!(Confidence::derive(52, 52).unwrap().render(), "1.00");
    }
}
