use replay_rows::value::value_struct;

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
    pub form: u8,
    pub state: Option<i32>,
    pub flags: Option<[u8; 4]>,
    pub country_operand: Option<u32>,
    pub puppet_auxiliary: Option<u64>,
    pub ship: Option<u64>,
    pub ratio: Option<i64>,
    pub name_key: Option<Vec<u8>>,
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
    pub sequence: u64,
    pub conference: Option<u64>,
    pub hour: Option<i32>,
    pub winners: Vec<u32>,
    pub losers: Vec<u32>,
    pub actions: Vec<SettledPeaceAction>,
    pub transfers: Vec<SettledPeaceTransfer>,
    pub subjects: Vec<SettledPeaceSubject>,
    pub failure: Option<Vec<u8>>,
    pub omitted: u64,
}
