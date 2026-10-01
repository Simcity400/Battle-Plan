use crate::test_fixtures::{CAMPAIGN, records, scale_frame, source};
use replay::{
    container::Replay,
    frame::TableView,
    rows::{CountryLinkRow, CountryRow, TableId, TableRows},
};
use std::{
    io,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

#[test]
fn metadata_skips_an_unavailable_first_census_warns_on_bad_links_and_stops_when_cancelled() {
    let mut first = source(1, 100, Some(1));
    *first.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "census failed".into(),
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![first, source(1, 101, Some(1))])],
        )
        .unwrap(),
    );
    let path = Path::new("memory.battleplan");
    let metadata = crate::metadata::read(&replay, path, &AtomicBool::new(false)).unwrap();
    assert_eq!(metadata.branches[0].first_hour, 100);
    assert_eq!(metadata.countries.len(), 1);
    assert_eq!(metadata.countries[0].observed_hour, 101);
    assert_eq!(metadata.countries[0].tag, "GER");
    assert_eq!(metadata.player_country_id, None);
    assert_eq!(
        crate::metadata::read(&replay, path, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );

    let mut bad = source(1, 100, Some(1));
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *bad.tables[TableId::CountryLinks.index()]
    {
        links[1].canonical_slot = 8;
    }
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![bad])]).unwrap());
    let metadata = crate::metadata::read(&replay, path, &AtomicBool::new(false)).unwrap();
    assert!(metadata.countries.is_empty());
    assert!(
        metadata
            .warnings
            .iter()
            .any(|warning| warning.contains("CountryLinks"))
    );
}

#[test]
fn metadata_catalogue_lists_every_country_once_across_branches_within_the_reply_budget() {
    const COUNTRIES: u32 = 65_535;
    let branches: Vec<_> = (1..=3)
        .map(|branch| records(branch, vec![scale_frame(branch, 100, COUNTRIES)]))
        .collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &branches).unwrap());
    let metadata = crate::metadata::read(
        &replay,
        Path::new("scale.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    // Not countries x branches: each country once, at its first branch.
    assert_eq!(metadata.countries.len(), COUNTRIES as usize);
    assert!(
        metadata
            .countries
            .iter()
            .all(|country| country.observed_branch_id == "1")
    );
    let reply = crate::transport::SourceReply {
        id: "lease".into(),
        path: metadata.path.clone(),
        revision: "revision".into(),
        metadata,
    };
    let bytes = serde_json::to_vec(&reply).unwrap().len();
    assert!(
        bytes < crate::transport::MAX_REPLY_BYTES / 2,
        "{bytes} bytes for {COUNTRIES} countries"
    );
}

#[test]
fn a_tag_is_found_absent_or_unresolved() {
    use crate::metadata::{TaggedCountry, tagged_country};
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    let countries = [country(21, 8, b"ITA"), country(9001, 7, b"GER")];
    let links = [
        CountryLinkRow {
            id: 21,
            canonical_slot: 8,
        },
        CountryLinkRow {
            id: 9001,
            canonical_slot: 7,
        },
    ];
    assert_eq!(
        tagged_country(&countries, &links, "GER"),
        TaggedCountry::Found(&countries[1], 7)
    );
    assert_eq!(
        tagged_country(&countries, &links, "FRA"),
        TaggedCountry::Absent
    );
    assert_eq!(
        tagged_country(&countries, &links[..1], "GER"),
        TaggedCountry::Unresolved
    );
    assert_eq!(tagged_country(&countries, &links[..1], "GER").found(), None);
}

fn daily_slot_of(table: TableId) -> i32 {
    let replay::cadence::Cadence::Daily(slot) = replay::cadence::cadence(table) else {
        panic!("{table:?} is read in a daily slot");
    };
    slot as i32
}

#[test]
fn a_slot_hour_reads_its_tables_even_when_another_table_is_charged() {
    let slot = daily_slot_of(TableId::CountryFuel);
    let day = 1000 * replay::cadence::STAGGER_PERIOD;
    // Part Q's pattern: the shared economy read is charged to the hourly
    // ProvinceSupply, so no table of the slot is timed at its slot hour.
    let mut charged = replay::frame::Metrics::default();
    charged.table_ns[TableId::ProvinceSupply.index()] = 1_051_200;
    for table in [TableId::CountryFuel, TableId::CountryExperience] {
        assert!(super::slot_read(&charged, table, day + slot));
        assert!(!super::slot_read(&charged, table, day + slot + 1));
    }
    // A full hour times the slot's tables off their slot hour.
    let mut full = replay::frame::Metrics::default();
    full.table_ns[TableId::CountryExperience.index()] = 72_100;
    assert!(super::slot_read(
        &full,
        TableId::CountryFuel,
        day + slot + 9
    ));
}

#[test]
fn country_intel_is_dated_only_by_its_measured_walk() {
    let slot = daily_slot_of(TableId::CountryIntel);
    let hour = 1000 * replay::cadence::STAGGER_PERIOD + slot;
    assert!(!super::slot_read(
        &replay::frame::Metrics::default(),
        TableId::CountryIntel,
        hour
    ));
    let mut timed = replay::frame::Metrics::default();
    timed.table_ns[TableId::CountryIntel.index()] = 1;
    assert!(super::slot_read(&timed, TableId::CountryIntel, hour + 5));
}
