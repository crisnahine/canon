//! Convention types and the arithmetic that decides when a pattern is real.
//!
//! This crate is pure data. It performs no I/O, spawns no processes, and reads
//! no environment, which is what makes the derivation rules unit-testable
//! without a repository on disk. Every crate above it may depend on this one;
//! this one depends on nothing but `serde`.
//!
//! The two ideas worth knowing before reading further:
//!
//! - A [`Convention`] is a claim about shape, carrying the evidence that
//!   produced it. It is never a claim about correctness.
//! - [`Confidence`] refuses to produce a value at all when the sample is too
//!   small or the agreement too weak. A convention engine that guesses is worse
//!   than one that stays quiet, because a wrong rule gets followed.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::float_cmp
    )
)]

mod confidence;
mod convention;
mod settings;

pub use confidence::Confidence;
pub use convention::{Convention, Enforcement, Evidence, ROLLUP_SUFFIX, Scope, enforcement_for};
pub use settings::{Settings, SettingsError};

/// Bytes of convention text injected per write.
///
/// Headroom rather than a limit that binds. Measured across a 13,456-file
/// workspace, real paths spend 262 to 486 bytes of it, and raising the budget
/// to 4,000 produced byte-identical output: nothing was being dropped. What
/// bounds the block is how many conventions exist for a path, not how many
/// fit, so this is set well above what any scope currently uses and will
/// absorb new convention kinds without needing to move.
///
/// It is not unlimited. The block competes with the user's own instruction for
/// attention, and a page of derived shape would start reading as though it
/// outranked what was actually asked for.
pub const INJECTION_BUDGET: usize = 3_000;

/// Hard ceiling on anything this process writes to the protocol channel.
///
/// The host truncates oversized hook output. Truncation of a JSON document is
/// indistinguishable from a crash on the receiving end, so canon refuses to
/// emit past this rather than letting the host cut a value in half.
pub const HOOK_OUTPUT_CAP: usize = 16_384;
