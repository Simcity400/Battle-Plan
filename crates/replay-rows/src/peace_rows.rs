//! Finalizer events, not resident conference snapshots. Rows appear once in
//! the next hourly drain; their hour is the guarded finalizer-time read.
use crate::value::value_struct;

/// A try-only drain preserves the owned log for the next hourly boundary.
pub const PEACE_DRAIN_BUSY: &str = "peace finalizer log busy at drain";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SettledPeaceAction {
    pub country: u32,
    pub turn: u32,
    pub ordinal: u32,
    pub token: u32,
    pub taker: u32,
    pub giver: u32,
    pub negotiator: u32,
    pub status: i32,
    pub cost: i32,
    /// Native layout discriminator, never an action-token catalogue.
    pub form: u8,
    pub state: Option<i32>,
    pub flags: Option<[u8; 4]>,
    pub country_operand: Option<u32>,
    /// Opaque stored word; must never be followed as a live pointer.
    pub puppet_auxiliary: Option<u64>,
    /// None outside navy; Some(0) for a null native selection.
    pub ship: Option<u64>,
    pub ratio: Option<i64>,
    pub name_key: Option<Vec<u8>>,
    /// Cached native type name in the recording client's language, not a demand label.
    pub cached_name: Option<Vec<u8>>,
}
value_struct!(SettledPeaceAction {
    country: u32, turn: u32, ordinal: u32, token: u32, taker: u32,
    giver: u32, negotiator: u32, status: i32, cost: i32, form: u8,
    state: Option<i32>, flags: Option<[u8; 4]>, country_operand: Option<u32>,
    puppet_auxiliary: Option<u64>, ship: Option<u64>, ratio: Option<i64>, name_key: Option<Vec<u8>>,
    cached_name: Option<Vec<u8>>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SettledPeaceTransfer {
    pub state: i32,
    pub taker: u32,
    /// Former owner is not retained by these native outcome lists.
    pub automatic: bool,
}
value_struct!(SettledPeaceTransfer {
    state: i32,
    taker: u32,
    automatic: bool
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SettledPeaceSubject {
    pub overlord: u32,
    pub subject: u32,
}
value_struct!(SettledPeaceSubject {
    overlord: u32,
    subject: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PeaceSettlementRow {
    /// Observation sequence, also identifying failed identity reads.
    pub sequence: u64,
    /// The PeaceConferences full reference, absent if identity failed.
    pub conference: Option<u64>,
    pub hour: Option<i32>,
    pub winners: Vec<u32>,
    pub losers: Vec<u32>,
    pub actions: Vec<SettledPeaceAction>,
    pub transfers: Vec<SettledPeaceTransfer>,
    pub subjects: Vec<SettledPeaceSubject>,
    /// First failure; Some means unavailable, even with a copied prefix.
    pub failure: Option<Vec<u8>>,
    /// Lower bound on unread payload occurrences, distinct from lost events.
    pub omitted: u64,
}
