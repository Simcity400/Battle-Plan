use super::exported_table_ticks;
use replay_data::rows::TableId;

#[test]
fn a_retried_country_intel_walk_keeps_its_cost() {
    assert_eq!(exported_table_ticks(TableId::Subjects, true, 7), Some(7));
    assert_eq!(exported_table_ticks(TableId::Subjects, false, 7), None);
    assert_eq!(
        exported_table_ticks(TableId::CountryIntel, false, 9),
        Some(9)
    );
    assert_eq!(
        exported_table_ticks(TableId::CountryIntel, true, 0),
        Some(0)
    );
    assert_eq!(exported_table_ticks(TableId::CountryIntel, false, 0), None);
}
