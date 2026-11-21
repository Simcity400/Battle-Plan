#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Row {
    pub token: u32,
    pub set_hour: i32,
    pub value: i16,
    pub remaining_days: i16,
}
