//! What a pass of the air registration store did with its state bases
//! ([`crate::extended::air::RegistrationStore`]).

/// What one pass did with its state bases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
}

/// What one pass did with the AirBases census of its state bases: the
/// manager's state base slot arrays and each state base's row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    /// Slot arrays carried and read.
    pub arrays: [u64; 2],
    /// State base rows (identity, host, state, province, capacity, kind)
    /// carried and read.
    pub rows: [u64; 2],
    /// Spans the census read from the game and spans it spliced.
    pub spans: [u64; 2],
}
