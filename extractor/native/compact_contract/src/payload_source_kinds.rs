//! Which SOURCE a payload's rows were read from, for the two payloads whose
//! source codebook the producer and every consumer wrote down separately.
//!
//! THE DEFECT (lane TV, row R18). The collector assigned the codes, the
//! recorder host's authority check redeclared them, and the fold declared them
//! a third time to attach names -- three crates, one codebook, nothing typing
//! the value. Exchanging the producer's two constants compiles, and scan data
//! is reported as bridge data with every count intact.
//!
//! Both crates already depend on this one, so the column belongs here.

crate::cause_tables::coded_cause_names! {
    repr = u32, roster = pub COMPACT_FRD_DECISION_TARGET_SOURCES;

    /// The timed-targeted decision list, whose rows carry their source at
    /// offset `0xE8`.
    COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED = 1 => "active_targeted_timed",
    /// The plain targeted decision list, at offset `0x100`.
    COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED = 2 => "active_targeted",
}

crate::cause_tables::coded_cause_names! {
    repr = u32, roster = pub COMPACT_PROJECT_SOURCES;

    /// Read from the project pool by the scan.
    COMPACT_PROJECT_SOURCE_SCAN = 1 => "scan",
    /// Read through the bridge, which owes bridge rows.
    COMPACT_PROJECT_SOURCE_BRIDGE = 2 => "bridge",
}

/// `0` is the producer's "no source" and is not a member of either column: a
/// payload carrying it was never read, and both consumers refuse it. Named so
/// the producer's early-out tests against it rather than a bare `0`.
pub const COMPACT_SOURCE_KIND_ABSENT: u32 = 0;
