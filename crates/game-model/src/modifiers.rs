#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct RequestedOperand {
    pub raw: Option<i64>,
    pub present: bool,
    pub effective: i64,
}
