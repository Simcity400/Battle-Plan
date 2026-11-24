use super::*;
use replay_data::rows::{CountryStabilityInputRow, TableRows};
use replay_frame::frame::TableView;

#[test]
fn an_exile_is_kept_with_its_identity_missing_when_countries_are_unread_and_an_unread_host_is_counted() {
    let mut frame = dashboard_base::test_fixtures::source(1, 100, Some(1_000));
    let rows = vec![
        CountryStabilityInputRow { country: 9001, exile_host: Recorded::Value(Some(9002)), ..Default::default() },
        CountryStabilityInputRow { country: 9003, exile_host: Recorded::NotRecorded, ..Default::default() },
    ];
    frame.tables[TableId::CountryStabilityInputs.index()] = TableView::Complete(TableRows::CountryStabilityInputs(rows)).into();
    *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "links unreadable".into() };
    let (exiles, refusals) = exile_rows(&frame, |_, _| true);
    assert_eq!(exiles.iter().map(|row| (row.country, row.exile.tag.as_deref(), row.exile.host.as_deref())).collect::<Vec<_>>(), [(9001, None, None)]);
    assert_eq!(refusals[0], 1, "the unread host leaves membership incomplete");
}

#[test]
fn an_exile_keeps_its_slot_and_deployed_air_from_the_links_when_only_country_names_are_unread() {
    use replay_data::rows::{CountryDeployedManpowerRow, CountryLinkRow};
    let mut frame = dashboard_base::test_fixtures::source(1, 100, Some(1_000));
    let rows = vec![CountryStabilityInputRow { country: 9001, exile_host: Recorded::Value(Some(9002)), ..Default::default() }];
    frame.tables[TableId::CountryStabilityInputs.index()] = TableView::Complete(TableRows::CountryStabilityInputs(rows)).into();
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "names unreadable".into() };
    let links = vec![CountryLinkRow { id: 9001, canonical_slot: 5 }, CountryLinkRow { id: 9002, canonical_slot: 6 }];
    frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links)).into();
    let deployed = vec![CountryDeployedManpowerRow { slot: 5, exile_air: Recorded::Value(3), ..Default::default() }];
    frame.tables[TableId::CountryDeployedManpower.index()] = TableView::Complete(TableRows::CountryDeployedManpower(deployed)).into();
    let (exiles, _) = exile_rows(&frame, |_, _| true);
    assert_eq!(exiles.iter().map(|row| (row.slot, row.exile.tag.as_deref(), row.exile.deployed_air)).collect::<Vec<_>>(), [(Some(5), None, Some(3))]);
}

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
