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
#[ignore = "read-only G2 recording; BATTLE_PLAN_COMPAT_RECORDINGS supplies its path"]
fn compatibility_real_g2_product_impact() {
    use crate::{
        products::{self, LoadContext, Product},
        scope::{Context, RecordedScopes, Target, Unresolved},
    };
    use installed_content::{
        localisation::Localisation,
        scope::{Keyword, Scope},
        tag_aliases::TagAliases,
    };
    let path = std::env::var("BATTLE_PLAN_COMPAT_RECORDINGS").or_else(|_| std::env::var("HOI4_COMPAT_RECORDINGS")).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = crate::Source::open(
        crate::SourceKey {
            source_id: "g2".into(),
            revision: "read-only".into(),
        },
        path.into(),
        &cancelled,
    )
    .unwrap();
    let replay = source.replay();
    assert_eq!(replay.format.version(), 94);
    let replay::container::SeekResult::Frame(frame) = replay.seek(2, 60827796).unwrap() else {
        panic!("missing G2 comparison hour")
    };
    let countries = frame.complete::<replay::rows::tables::Countries>().unwrap();
    let country = countries
        .iter()
        .find(|country| {
            country.canonical_slot > 0
                && country.tag.len() == 3
                && country.tag.iter().all(u8::is_ascii_alphabetic)
        })
        .unwrap();
    let tag = std::str::from_utf8(&country.tag).unwrap();
    let localisation = Localisation::default();
    let styles = products::country_styles::HourStyles::read(&frame, Some(&localisation)).unwrap();
    let coloured = countries
        .iter()
        .filter(|country| {
            let tag = std::str::from_utf8(&country.tag).unwrap();
            styles.get(tag).is_some_and(|style| style.color.is_some())
        })
        .count();
    assert!(coloured > 0);
    let flags = products::country_flags::choices(&frame).unwrap();
    assert!(!flags.is_empty());
    let appearances = frame
        .complete::<replay::rows::tables::CountryAppearance>()
        .unwrap();
    for flag in &flags {
        let fallback = appearances
            .iter()
            .find(|row| row.country == flag.raw)
            .map(|row| std::str::from_utf8(&row.name_tag).unwrap())
            .filter(|original| !original.is_empty())
            .unwrap_or(&flag.tag);
        assert_eq!(flag.candidates.last().map(String::as_str), Some(fallback));
    }
    let names = installed_content::country_names::CountryNames::read(&frame, &localisation);
    assert!(names.operands(country.canonical_slot).unwrap().is_ok());
    let scopes = RecordedScopes::read(&frame, Arc::new(TagAliases::default()));
    assert_eq!(scopes.resolve_tag(tag).unwrap(), country.canonical_slot);
    let context = Context::country(country.canonical_slot);
    assert!(matches!(
        scopes.resolve(&Scope::Keyword(Keyword::Overlord), &context),
        Ok(_) | Err(Unresolved::Nothing(_))
    ));
    assert_eq!(
        scopes
            .resolve(&Scope::Keyword(Keyword::Root), &context)
            .unwrap(),
        Target::Country(country.canonical_slot)
    );
    let context = LoadContext { content: None };
    let measure = products::country_measure::CountryMeasure::load(
        &source,
        &products::country_measure::MeasureParams {
            branch_id: "2".into(),
            measure: products::country_measure::Measure::CommandPower,
            tags: vec![tag.into()],
        },
        &context,
        &cancelled,
    )
    .unwrap();
    assert!(measure.series.iter().any(|series| !series.runs.is_empty()));
    let owners = products::land_owners::land_owners(&replay, None, &cancelled).unwrap();
    println!(
        "G2_PRODUCT country-styles: {} colour rows; country naming operands retained (localisation not loaded in this check); country-flags: {} native cosmetic/original choices; OVERLORD retained",
        coloured,
        flags.len()
    );
    println!(
        "G2_PRODUCT command-power: {} runs; land-owners: {} countries",
        measure
            .series
            .iter()
            .map(|series| series.runs.len())
            .sum::<usize>(),
        owners.countries.len()
    );
    for table in [
        replay::rows::TableId::CountryAppearance,
        replay::rows::TableId::DivisionTemplates,
        replay::rows::TableId::Decisions,
        replay::rows::TableId::DivisionMovement,
    ] {
        assert!(matches!(
            &*frame.tables[table.index()],
            TableView::Complete(_)
        ));
    }
}

#[test]
fn metadata_compatibility_opens_format94_and_surfaces_every_named_gap() {
    let mut frame = source(1, 1000, Some(1));
    for table in [
        TableId::CountryAppearance,
        TableId::DivisionTemplates,
        TableId::Decisions,
        TableId::DivisionMovement,
    ] {
        // Empty snapshots have the same wire framing in both contracts.
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    let mut records = records(1, vec![frame]);
    let replay::spool::Record::SessionStart(session) = &mut records[0] else {
        unreachable!();
    };
    session.identity.coverage_policy = 3;
    let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    let mut bytes = replay.bytes().to_vec();
    bytes[4..6].copy_from_slice(&94_u16.to_le_bytes());
    let replay = Arc::new(Replay::open(bytes).unwrap());
    let metadata = crate::metadata::read(
        &replay,
        Path::new("format94.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(metadata.format, 94);
    assert_eq!(metadata.countries.len(), 1);
    assert_eq!(metadata.countries[0].tag, "GER");
    let expected = replay.compatibility_gaps();
    assert_eq!(expected.len(), 4);
    assert_eq!(metadata.warnings, expected);
    for table in [
        TableId::CountryAppearance,
        TableId::DivisionTemplates,
        TableId::Decisions,
        TableId::DivisionMovement,
    ] {
        assert!(metadata.warnings.iter().any(|warning| {
            warning.starts_with(&format!("{table:?}."))
                && warning.contains("this recording is format 94")
        }));
    }
}

#[test]
fn metadata_partial_marker_keeps_the_failure_separate_from_known_and_unknown_hour_gaps() {
    use replay::{
        branches::Gap,
        container::SeekResult,
        spool::{ControlRecord, Record},
    };
    for failed_hour in [Some(1017), None] {
        let mut records = records(1, vec![source(1, 1000, Some(1))]);
        if let Some(hour) = failed_hour {
            records.push(Record::Gap(Gap {
                branch: 1,
                first_hour: hour,
                last_hour: hour,
                reason: "collector allocation failed".into(),
            }));
        }
        records.push(Record::Control(ControlRecord::Partial {
            session: 1,
            reason: "collector allocation failed".into(),
            failed_hour,
        }));
        records.push(Record::Control(ControlRecord::Detach {
            session: 1,
            last_hour: Some(1000),
        }));
        let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records]).unwrap());
        let metadata = crate::metadata::read(
            &replay,
            Path::new("partial.battleplan"),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            metadata.partial,
            Some(crate::PartialRecording {
                session_id: "1".into(),
                reason: "collector allocation failed".into(),
                failed_hour,
            })
        );
        assert_eq!(
            metadata.branches[0].gaps.len(),
            usize::from(failed_hour.is_some())
        );
        assert_eq!(metadata.branches[0].last_hour, failed_hour.unwrap_or(1000));
        assert!(matches!(
            replay.seek(1, 1000).unwrap(),
            SeekResult::Frame(_)
        ));
        if failed_hour.is_some() {
            assert_eq!(metadata.branches[0].gaps[0].first_hour, 1017);
            assert_eq!(
                metadata.branches[0].gaps[0].reason,
                "collector allocation failed"
            );
            assert!(matches!(
                replay.seek(1, 1017).unwrap(),
                SeekResult::Gap { .. }
            ));
        }
    }
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 1000, Some(1))])],
        )
        .unwrap(),
    );
    let metadata = crate::metadata::read(
        &replay,
        Path::new("healthy.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(metadata.partial, None);
}

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
    // Intel can move off its slot independently of the other daily walks.
    let sibling = TableId::LogisticsEquipment;
    assert_eq!(daily_slot_of(sibling), slot);
    assert!(!super::slot_read(&timed, sibling, hour + 5));
    let mut sibling_timed = replay::frame::Metrics::default();
    sibling_timed.table_ns[sibling.index()] = 1;
    for sampled_hour in [hour, hour + 5] {
        assert!(!super::slot_read(
            &sibling_timed,
            TableId::CountryIntel,
            sampled_hour
        ));
    }
}

#[test]
fn every_tag_is_tagged_in_one_pass_as_tagged_country_tags_it() {
    use crate::metadata::{tagged_countries, tagged_country};
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    // A second GER row the links resolve after one they do not, and a tag no link resolves.
    let countries = [
        country(5, 9, b"GER"),
        country(9001, 7, b"GER"),
        country(21, 8, b"ITA"),
        country(33, 4, b"FRA"),
    ];
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
    let tagged = tagged_countries(&countries, &links);
    assert_eq!(tagged.len(), 3);
    for tag in ["GER", "ITA", "FRA"] {
        assert_eq!(
            tagged[tag.as_bytes()],
            tagged_country(&countries, &links, tag),
            "{tag}"
        );
    }
}

#[test]
fn a_table_is_read_by_its_slot_or_timing_and_a_deferred_intel_walk_proves_nothing_else() {
    use crate::metadata::slot_read;
    use crate::test_fixtures::slot_hour;
    let walked = |tables: &[TableId]| {
        let mut metrics = replay::frame::Metrics::default();
        for table in tables {
            metrics.table_ns[table.index()] = 1;
        }
        metrics
    };
    let slot = slot_hour(TableId::LogisticsEquipment);
    assert_eq!(slot, slot_hour(TableId::CountryIntel));
    // Intel deferred at the slot hour, then retried alone. Its retry cannot
    // date the logistics rows carried from the slot read.
    let deferred = walked(&[TableId::LogisticsEquipment]);
    assert!(slot_read(&deferred, TableId::LogisticsEquipment, slot));
    assert!(!slot_read(&deferred, TableId::CountryIntel, slot));
    let retry = walked(&[TableId::CountryIntel]);
    assert!(slot_read(&retry, TableId::CountryIntel, slot + 1));
    assert!(!slot_read(&retry, TableId::LogisticsEquipment, slot + 1));

    let fuel = slot_hour(TableId::CountryFuel);
    let shared = walked(&[TableId::CountryFuelConsumers]);
    assert!(slot_read(&shared, TableId::CountryFuel, fuel));
    assert!(!slot_read(&walked(&[]), TableId::CountryFuel, fuel + 1));
    // Off-slot timing dates its own slot only. A full read needs evidence
    // for the other slots too, and Intel always needs its own timing.
    let off_slot = fuel + 3;
    assert!(slot_read(&shared, TableId::CountryFuel, off_slot));
    for table in [
        TableId::Stockpiles,
        TableId::ProductionLines,
        TableId::LogisticsEquipment,
        TableId::CountryIntel,
    ] {
        assert!(!slot_read(&shared, table, off_slot), "{table:?}");
    }
    let full = walked(&[
        TableId::CountryFuelConsumers,
        TableId::DeploymentLines,
        TableId::LogisticsEquipment,
        TableId::CountryIntel,
    ]);
    for table in [
        TableId::CountryFuel,
        TableId::Stockpiles,
        TableId::ProductionLines,
        TableId::LogisticsEquipment,
        TableId::CountryIntel,
    ] {
        assert!(slot_read(&full, table, off_slot), "{table:?}");
    }
    let stock = slot_hour(TableId::Stockpiles);
    assert!(slot_read(
        &walked(&[TableId::DeploymentLines]),
        TableId::ProductionLines,
        stock
    ));
    assert!(!slot_read(&walked(&[]), TableId::Stockpiles, stock + 1));
    // Resources are now hourly; a lack of daily-slot timing cannot make them carried.
    for table in [TableId::CountryResources, TableId::ResourceExchanges] {
        assert!(slot_read(&walked(&[]), table, fuel));
        assert!(slot_read(&walked(&[]), table, fuel + 2));
    }
    // The shared economy read can be charged to an hourly anchor instead.
    assert!(slot_read(&walked(&[]), TableId::CountryFuel, fuel));
    assert!(slot_read(&walked(&[]), TableId::LogisticsEquipment, slot));
}
