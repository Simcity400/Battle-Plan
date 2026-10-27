//! What a pass of the air registration store did with its state bases
//! ([`crate::extended::air::RegistrationStore`]).

/// What one pass did with its state bases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
}
