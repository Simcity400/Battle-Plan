//! What a pass of the air registration store did with its state bases, and
//! where a fresh read first differed from a held segment
//! ([`crate::extended::air::RegistrationStore`]).

/// What one pass did with its state bases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
}

/// Where a fresh read first differed from its held segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Difference {
    /// What differed: the demand, the links, the span list, or a span or its
    /// bytes.
    pub what: &'static str,
    /// The span's address and length, for a span.
    pub address: usize,
    pub len: u32,
    /// The first differing byte within the span.
    pub byte: usize,
}
