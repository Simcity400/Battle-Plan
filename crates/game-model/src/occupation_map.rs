#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Markers {
    /// Present only when zero resistance and nonpositive compliance demand the
    /// cached target for the map's suppression predicate.
    pub stored_target: Option<i64>,
    /// The matched occupation group's retained country identity, also its saved
    /// group key. None proves no matching native group, including for empty lists.
    pub core_country: Option<u32>,
    /// State occurrences followed by core occurrences, before the display's stable sort.
    pub resistance: Vec<Vec<u8>>,
    pub compliance: Vec<Vec<u8>>,
}
