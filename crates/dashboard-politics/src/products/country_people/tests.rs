use super::*;

#[test]
fn election_rules_keep_native_calendar_and_cache() {
    use game_model::calendar::HOUR_EPOCH;
    let january_31 = HOUR_EPOCH + 1936 * 365 * 24 + 30 * 24 + 7;
    assert_eq!(election_date(january_31, 1), HOUR_EPOCH + 1936 * 365 * 24 + 61 * 24 + 7);
    for last in [engine_native_memory::cgame_date::NULL_RAW_HOUR, engine_native_politics::cpolitical_status::RESET_LAST_ELECTION] {
        assert_eq!(next_election(last, Some(48), Some(Some(12345))), Some(12345));
        assert_eq!(next_election(last, Some(48), Some(None)), None);
    }
}
