//! Activation manager models for network upgrades and voting.

use reth_codecs::{add_arbitrary_tests, Compact};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Represents a validator's vote on a network upgrade proposal.
///
/// Validators can explicitly vote in favor of an upgrade (`Aye`),
/// against an upgrade (`Nay`), or can abstain from voting (`Absent`).
///
/// Votes are included in block proposals via the `NetworkUpgradePayload`
/// in the Non-Deterministic Data (NDD) transaction. These votes are then
/// tracked by the activation manager to calculate support thresholds.
///
/// The default vote is `Nay`, indicating that validators must explicitly
/// opt-in to upgrades rather than being opted-in by default.

#[derive(Default, Debug, Clone, Copy, Eq, PartialEq, Compact)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(any(test, feature = "reth-codec"), reth_codecs::add_arbitrary_tests(compact))]
pub enum Vote {
    /// Vote in favor of the upgrade. An `Aye` vote contributes to the signaling
    /// threshold calculations.
    Aye,

    /// Vote against the upgrade. A `Nay` vote allows validators to signal
    /// opposition while still being counted in voting statistics.
    Nay,

    /// Explicit abstention from voting. An `Absent` vote functions the same as
    /// `Nay` in quorum calculations, but communicates the validator's intent to
    /// abstain rather than actively oppose the upgrade. It still counts as
    /// participation in the voting process.
    #[default]
    Absent,
}

/// Represents a protocol version in the Botanix blockchain.
///
/// A runtime version consists of two components:
/// - Major version (`0`): Incremented for hard fork or breaking changes
/// - Minor version (`1`): Incremented for non-breaking changes
///
/// Runtime versions are comparable, with lexicographic ordering:
/// - First compare major versions
/// - If major versions are equal, compare minor versions
///
/// This ordered relationship is crucial for the upgrade process, as it ensures that:
/// 1. Nodes can determine whether a version is an upgrade or downgrade
/// 2. The activation manager can enforce one-way upgrade progression
/// 3. Historical blocks during sync can be validated against appropriate version thresholds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeVersion(
    /// Major version component, incremented for breaking changes (hard fork)
    pub MajorVersion,
    /// Minor version component, incremented for non-breaking changes (soft fork)
    pub MinorVersion,
);

impl RuntimeVersion {
    /// Creates a new runtime version from major and minor components.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self(MajorVersion(major), MinorVersion(minor))
    }
}

impl PartialOrd for RuntimeVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RuntimeVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.0.cmp(&other.0) {
            Ordering::Equal => self.1.cmp(&other.1),
            ordering => ordering,
        }
    }
}

impl From<(u16, u16)> for RuntimeVersion {
    fn from((major, minor): (u16, u16)) -> Self {
        Self::new(major, minor)
    }
}

/// Represents a major version component of a runtime version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MajorVersion(pub u16);

/// Represents a minor version component of a runtime version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MinorVersion(pub u16);

#[test]
fn runtime_version_ordering() {
    let v0_0 = RuntimeVersion::new(0, 0);
    let v0_1 = RuntimeVersion::new(0, 1);
    let v0_10 = RuntimeVersion::new(0, 10);
    let v1_0 = RuntimeVersion::new(1, 0);
    let v2_0 = RuntimeVersion::new(2, 0);

    assert_eq!(v0_0, v0_0);
    assert_eq!(v0_1, v0_1);
    assert_eq!(v0_10, v0_10);
    assert_eq!(v1_0, v1_0);
    assert_eq!(v2_0, v2_0);

    assert!(v0_0 < v0_1);
    assert!(v0_1 < v0_10);
    assert!(v0_10 < v1_0);
    assert!(v1_0 < v2_0);

    assert!(v2_0 > v1_0);
    assert!(v1_0 > v0_10);
    assert!(v0_10 > v0_1);
    assert!(v0_1 > v0_0);
}
