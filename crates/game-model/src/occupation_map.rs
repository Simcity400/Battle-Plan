#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Markers {
    pub stored_target: Option<i64>,
    pub core_country: Option<u32>,
    pub resistance: Vec<Vec<u8>>,
    pub compliance: Vec<Vec<u8>>,
}
