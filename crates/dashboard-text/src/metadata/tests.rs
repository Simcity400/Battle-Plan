use crate::test_fixtures::{CAMPAIGN, records, scale_frame, source};
use replay::campaign::Replay;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::TableId, rows::TableRows, rows::tables,
};
use replay_frame::{
    frame::FrameDecoder, frame::FrameEncoder, frame::SharedView, frame::TableUpdate,
    frame::TableView,
};
use std::{
    io,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

#[test]
fn country_keys_iter_borrows_retained_operands_in_native_order() {
    let mut rows: Vec<_> = [
        (9, 2, b"Z".as_slice()),
        (3, 5, &[0xff, 0]),
        (4, 7, b"Z".as_slice()),
        (2, 8, b"".as_slice()),
    ]
    .into_iter()
    .map(|(id, canonical_slot, tag)| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    })
    .collect();
    let keys = super::CountryKeys::read(&rows);
    rows[0].tag = b"changed".to_vec();
    let expected = vec![
        (9, 2, b"Z".as_slice()),
        (3, 5, &[0xff, 0]),
        (4, 7, b"Z".as_slice()),
        (2, 8, b"".as_slice()),
    ];
    assert_eq!(keys.iter().collect::<Vec<_>>(), expected);
    assert_eq!(keys.iter().collect::<Vec<_>>(), expected);
    assert_eq!(
        keys.row_visits(),
        0,
        "the view reads no native country rows"
    );
    assert!(super::CountryKeys::default().iter().next().is_none());
}

#[test]
fn identity_track_advance_preserves_known_holder_and_closes_at_gaps() {
    let mut track = super::IdentityTrack::default();
    track.advance(9);
    assert!(!track.step(Some((3, 7)), 10));
    for hour in 11..=20 {
        track.advance(hour);
    }
    track = serde_json::from_slice(&serde_json::to_vec(&track).unwrap()).unwrap();
    assert!(track.step(Some((3, 7)), 21));
    assert!(!track.step(Some((3, 8)), 22));
    track.advance(23);
    assert!(!track.step(Some((3, 8)), 25));
    assert!(!track.step(None, 26));
    track.advance(27);
    assert!(!track.step(Some((3, 8)), 28));
    track.advance(29);
    assert!(track.step(Some((3, 8)), 30));
}

#[test]
fn initial_catalogue_has_no_countries_or_player_when_its_block_is_unreadable() {
    let mut first = source(1, 100, Some(1));
    *first.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![
            replay_data::rows::LocalPlayerRow {
                country: 11,
                observer_country: 0,
            },
        ]));
    let healthy = Replay::from_records(
        recorder_derive::writers(),
        CAMPAIGN.into(),
        &[records(1, vec![first, scale_frame(1, 101, 5000)])],
    )
    .unwrap();
    let healthy = Arc::new(healthy);
    let path = Path::new("memory.battleplan");
    let good = super::read(&healthy, path, &AtomicBool::new(false)).unwrap();
    assert_eq!(good.countries.len(), 1);
    assert_eq!(good.player_country_id.as_deref(), Some("7"));
    let damaged =
        Arc::new(Replay::open(healthy.damaged_block(&healthy.index[0]).unwrap()).unwrap());
    let metadata = super::read(&damaged, path, &AtomicBool::new(false)).unwrap();
    assert!(metadata.countries.is_empty());
    assert_eq!(metadata.player_country_id, None);
    assert!(
        metadata
            .warnings
            .iter()
            .any(|line| line.contains("is unreadable"))
    );
}

#[test]
fn catalogue_campaign_failure_becomes_a_gap_and_tries_the_next_block() {
    let next_hour = 200;
    let recording = |first| {
        Replay::from_records(
            recorder_derive::writers(),
            CAMPAIGN.into(),
            &[records(1, vec![first, source(1, next_hour, Some(2))])],
        )
        .unwrap()
    };
    let mut foreign = source(1, 100, Some(1));
    *foreign.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![replay_data::rows::CampaignRow {
            id: b"different synthetic campaign".to_vec(),
        }]));
    let replay = Arc::new(recording(foreign));
    let recording = recording(source(1, 100, Some(1)));
    assert_eq!(recording.index.len(), 2);
    let metadata = super::read(
        &replay,
        Path::new("memory.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(metadata.countries.len(), 1);
    assert_eq!(metadata.countries[0].observed_hour, next_hour);
    assert_eq!(metadata.branches[0].gaps.len(), 1);
    let gap = &metadata.branches[0].gaps[0];
    assert_eq!((gap.first_hour, gap.last_hour), (100, 100));
    assert!(
        gap.reason
            .contains("frame campaign identity differs from replay header")
    );
    let damaged = metadata.damaged.as_ref().unwrap();
    assert_eq!(damaged.parts, 1);
    assert_eq!(
        (damaged.first_hour, damaged.last_hour),
        (Some(100), Some(100))
    );
    assert!(damaged.reasons[0].contains("frame campaign identity differs from replay header"));
    assert_eq!(replay.index.len(), 2);
    assert_eq!(
        replay.read_block(&replay.index[1]).unwrap(),
        recording.read_block(&recording.index[1]).unwrap()
    );
}

#[test]
fn catalogue_checked_allocation_failures_propagate_instead_of_becoming_success() {
    const CHILD: &str = "BATTLEPLAN_METADATA_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "metadata::tests::catalogue_checked_allocation_failures_propagate_instead_of_becoming_success", "--nocapture"])
            .env(CHILD, "1").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    thread_local! {
        static ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static FAIL_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    }
    fn fail_allocation() -> bool {
        let attempt = ATTEMPTS.get() + 1;
        ATTEMPTS.set(attempt);
        FAIL_AT.get() == Some(attempt)
    }
    let mut segment = records(1, vec![source(1, 100, Some(1))]);
    for record in &mut segment {
        if let replay_capture_spool::spool::Record::SessionStart(session) = record {
            session.identity.verification = replay_data::identity::Verification::Unverified {
                reason: "synthetic launcher roster".into(),
                launcher: Default::default(),
                memory: Default::default(),
            };
        }
    }
    let replay = Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[segment]).unwrap());
    let read = || {
        super::read_resident(
            &replay,
            Path::new("memory.battleplan"),
            &AtomicBool::new(false),
        )
    };
    let healthy = read().unwrap();
    assert_eq!(healthy.countries.len(), 1);
    assert_eq!(healthy.countries[0].id, "7");
    assert_eq!(healthy.countries[0].observed_hour, 100);
    assert_eq!(healthy.branches.len(), 1);
    assert_eq!(healthy.branches[0].catalogue_hour, Some(100));
    assert!(healthy.refusals.is_empty());
    assert_eq!(
        healthy.warnings,
        ["The recorder did not verify the content identity. The dashboard reads recorded values only and does not verify local game assets."]
    );
    let expected_refusals = serde_json::json!({
        "warnings": healthy.warnings,
        "reasons": healthy.damaged.as_ref().map_or(&[][..], |damage| damage.reasons.as_slice()),
    });
    let mut complete = serde_json::to_value(&healthy.value).unwrap();
    complete["warnings"] = serde_json::json!([]);
    if healthy.damaged.is_some() {
        complete["damaged"]["reasons"] = serde_json::json!([]);
    }
    let read_refused = || super::with_storage_failure_after(0, read);
    let refused = read_refused().unwrap();
    assert_eq!(serde_json::to_value(&refused.value).unwrap(), complete);
    assert_eq!(
        serde_json::to_value(&refused.refusals).unwrap(),
        expected_refusals
    );

    engine_allocation::set_stop_hook(Some(fail_allocation));
    let healthy = read_refused();
    engine_allocation::set_stop_hook(None);
    let healthy = healthy.unwrap();
    assert_eq!(serde_json::to_value(&healthy.value).unwrap(), complete);
    assert_eq!(
        serde_json::to_value(&healthy.refusals).unwrap(),
        expected_refusals
    );
    let attempts = ATTEMPTS.get();
    assert!(attempts > 0);
    for ordinal in 1..=attempts {
        ATTEMPTS.set(0);
        FAIL_AT.set(Some(ordinal));
        engine_allocation::set_stop_hook(Some(fail_allocation));
        let result = read_refused();
        engine_allocation::set_stop_hook(None);
        assert_eq!(
            result
                .err()
                .expect("mandatory allocation denial must propagate")
                .kind(),
            io::ErrorKind::OutOfMemory,
            "isolated mandatory allocation {ordinal}"
        );
        assert!(
            ATTEMPTS.get() >= ordinal,
            "allocation {ordinal} was not reached"
        );
    }
}

#[test]
fn link_tags_balance_reads_keep_mapping_and_identity_changes_rebuild() {
    let mut sample = scale_frame(1, 100, 24);
    let mut tags = super::LinkTags::default();
    assert!(tags.update(&sample).unwrap());
    assert!(!tags.update(&sample).unwrap());
    let allocation = tags.get(100_000).unwrap().as_ptr();
    let previous = sample.tables[TableId::Countries.index()].clone();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        for country in rows {
            country.political_power_raw = Some(12_345);
        }
    }
    assert!(!SharedView::ptr_eq(
        &previous,
        &sample.tables[TableId::Countries.index()],
    ));
    assert!(!tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000).unwrap().as_ptr(), allocation);
    assert!(
        tags.from
            .as_ref()
            .unwrap()
            .countries
            .as_ref()
            .unwrap()
            .is(&sample.tables[TableId::Countries.index()])
    );
    let countries = std::mem::replace(
        &mut sample.tables[TableId::Countries.index()],
        TableView::Unavailable {
            reason: "taken".into(),
        }
        .into(),
    );
    sample.tables[TableId::Countries.index()] = countries
        .try_unwrap()
        .expect("only the frame holds the rows")
        .into();
    for country in sample.complete::<tables::Countries>().unwrap() {
        assert_eq!(
            tags.get(country.id).unwrap().as_bytes(),
            country.tag.as_slice(),
        );
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"renamed opaque tag".to_vec();
    }
    assert!(tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000).unwrap(), "renamed opaque tag");
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = 24;
    }
    assert!(tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000), None);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *sample.tables[TableId::CountryLinks.index()]
    {
        rows[0].canonical_slot = 1;
    }
    assert!(tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000), tags.get(100_001));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        rows.remove(1);
    }
    assert!(tags.update(&sample).unwrap());
    assert_eq!(tags.get(100_000), None);
}

#[test]
fn link_tags_incomplete_reads_are_errors_and_recovery_rebuilds_even_the_same_views() {
    let complete = source(1, 100, Some(1));
    let mut tags = super::LinkTags::default();
    for table in [TableId::Countries, TableId::CountryLinks] {
        for truncated in [false, true] {
            assert!(tags.update(&complete).unwrap());
            let mut failed = complete.clone();
            let rows = match &*failed.tables[table.index()] {
                TableView::Complete(rows) => rows.clone(),
                _ => unreachable!(),
            };
            *failed.tables[table.index()] = if truncated {
                TableView::Truncated { rows, missing: 1 }
            } else {
                TableView::Unavailable {
                    reason: "fixture".into(),
                }
            };
            assert!(tags.update(&failed).is_err());
            assert_eq!(tags.get(11).unwrap(), "GER");
            assert!(tags.from.is_none());
        }
    }
    assert!(tags.update(&complete).unwrap());
    assert!(!tags.update(&complete).unwrap());
}

#[test]
fn link_tags_aliases_join_slots_without_requiring_a_countries_own_link() {
    let mut sample = source(1, 100, Some(1));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *sample.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 9002,
            canonical_slot: 8,
            tag: b"GER".to_vec(),
            ..Default::default()
        });
    }
    *sample.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(11, 7), (12, 7), (13, 8), (14, 9), (9001, 7), (9002, 8)]
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .to_vec(),
    ));
    let mut tags = super::LinkTags::default();
    assert!(tags.update(&sample).unwrap());
    for raw in [11, 12, 13] {
        assert_eq!(tags.get(raw).unwrap(), "GER");
    }
    assert_eq!(tags.get(14), None);
    assert_eq!(tags.get(9001).unwrap(), "GER");
    assert_eq!(tags.get(9002).unwrap(), "GER");
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *sample.tables[TableId::CountryLinks.index()]
    {
        rows.truncate(4);
    }
    assert!(tags.update(&sample).unwrap());
    for raw in [11, 12, 13] {
        assert_eq!(tags.get(raw).unwrap(), "GER");
    }
    assert_eq!(tags.get(9001), None);
    assert_eq!(tags.get(9002), None);
}

#[test]
fn link_tags_contradictory_raw_links_and_conflicting_slots_never_reach_a_scan() {
    for duplicate_slot in [false, true] {
        let mut sample = source(1, 100, Some(1));
        let table = if duplicate_slot {
            TableId::Countries
        } else {
            TableId::CountryLinks
        };
        let mut wire = FrameEncoder::new().encode(sample.clone(), false).unwrap();
        if duplicate_slot {
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *sample.tables[TableId::Countries.index()]
            {
                rows.push(CountryRow {
                    id: 9002,
                    canonical_slot: 7,
                    tag: b"other".to_vec(),
                    ..Default::default()
                });
            }
        } else if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *sample.tables[TableId::CountryLinks.index()]
        {
            rows.insert(
                1,
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 8,
                },
            );
        }
        let TableView::Complete(rows) = &*sample.tables[table.index()] else {
            unreachable!();
        };
        wire.tables[table.index()] = TableUpdate::Snapshot(rows.clone());
        let error = FrameEncoder::new().encode(sample, false).unwrap_err();
        let reason = if duplicate_slot {
            "duplicate canonical slots"
        } else {
            "unique ascending keys"
        };
        assert!(error.to_string().contains(reason), "{error}");
        let decoded = FrameDecoder::new().apply(&wire).unwrap();
        assert!(matches!(
            &*decoded.tables[table.index()],
            TableView::Unavailable { reason: why } if why.contains(reason)
        ));
        assert!(super::LinkTags::default().update(&decoded).is_err());
    }
}

#[test]
fn metadata_partial_marker_keeps_the_failure_separate_from_known_and_unknown_hour_gaps() {
    use replay::campaign::SeekResult;
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::Record;
    use replay_frame::branches::Gap;
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
        let replay = Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records]).unwrap());
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
            recorder_derive::writers(),
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
            recorder_derive::writers(),
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
    let replay = Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![bad])]).unwrap());
    let metadata = crate::metadata::read(&replay, path, &AtomicBool::new(false)).unwrap();
    assert!(metadata.countries.is_empty());
    assert!(
        metadata
            .warnings
            .iter()
            .any(|warning| warning.contains("missing or conflicting country links mappings"))
    );
}

#[test]
fn a_branch_without_a_complete_census_is_catalogued_from_a_later_block() {
    let mut first = source(1, 100, Some(1));
    *first.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "census failed".into(),
    };
    let path = Path::new("memory.battleplan");
    let flag = AtomicBool::new(false);
    let early =
        Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![first])]).unwrap());
    let previous = crate::metadata::read_resident(&early, path, &flag).unwrap();
    assert!(previous.countries.is_empty());
    assert_eq!(previous.branches[0].catalogue_hour, None);
    let later = Arc::new(
        early
            .append(recorder_derive::writers(), &[crate::test_fixtures::continuation_records(
                &early,
                1,
                vec![source(1, 101, Some(1))],
            )])
            .unwrap(),
    );
    assert_eq!(later.index.len(), 2);
    for metadata in [
        crate::metadata::read(&later, path, &flag).unwrap(),
        crate::metadata::read_append(&later, path, &flag, &previous, &|block| {
            early.index.contains(block)
        })
            .unwrap()
            .into_wire()
            .unwrap(),
    ] {
        assert_eq!(metadata.branches[0].catalogue_hour, Some(101));
        assert_eq!(metadata.countries.len(), 1);
        assert_eq!(metadata.countries[0].observed_hour, 101);
    }
}

#[test]
fn metadata_catalogue_lists_every_country_once_across_branches_within_the_reply_budget() {
    const COUNTRIES: u32 = 65_535;
    let branches: Vec<_> = (1..=3)
        .map(|branch| records(branch, vec![scale_frame(branch, 100, COUNTRIES)]))
        .collect();
    let replay = Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &branches).unwrap());
    let metadata = crate::metadata::read(
        &replay,
        Path::new("scale.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
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
        presentation_generation: 0,
    };
    let bytes = serde_json::to_vec(&reply).unwrap().len();
    assert!(
        bytes < crate::transport::MAX_REPLY_BYTES / 2,
        "{bytes} bytes for {COUNTRIES} countries"
    );
}

#[test]
fn a_tag_or_slot_is_found_absent_or_unresolved() {
    use crate::metadata::{TaggedCountry, slotted_country, tagged_country};
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
    assert_eq!(
        slotted_country(&countries, &links, 7),
        TaggedCountry::Found(&countries[1], 7)
    );
    assert_eq!(slotted_country(&countries, &links, 5), TaggedCountry::Absent);
    assert_eq!(
        slotted_country(&countries, &links[..1], 7),
        TaggedCountry::Unresolved,
        "a row in the slot whose link is missing is unresolved, not absent"
    );
}

fn daily_slot_of(table: TableId) -> i32 {
    let replay_data::cadence::Cadence::Daily(slot) = replay_data::cadence::cadence(table) else {
        panic!("{table:?} is read in a daily slot");
    };
    slot as i32
}

#[test]
fn supplying_read_batches_match_each_tables_own_read_rule() {
    use replay_data::format::FormatVersion;
    use replay_frame::frame::Metrics;
    let versions = [
        FormatVersion::CURRENT,
        FormatVersion::read(replay_data::rows::compatibility::CADENCE_FORMAT - 1).unwrap(),
    ];
    for format in versions {
        for hour in [-1, 0, 4, 17, 23, replay_data::cadence::STAGGER_PERIOD] {
            for timed in [
                None,
                Some(TableId::CountryIntel),
                Some(TableId::CountryFuel),
                Some(TableId::LexicalTokens),
                Some(TableId::ResearchInputs),
            ] {
                let mut metrics = Metrics::default();
                if let Some(table) = timed {
                    metrics.table_ns[table.index()] = 1;
                }
                let flags = super::slot_reads(format, &metrics, hour);
                for table in TableId::ALL {
                    assert_eq!(
                        flags[table.index()],
                        scalar_read_rule(format, &metrics, table, hour),
                        "{format:?}, {table:?} at {hour}, timed {timed:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_slot_hour_reads_its_tables_even_when_another_table_is_charged() {
    let slot = daily_slot_of(TableId::CountryFuel);
    let day = 1000 * replay_data::cadence::STAGGER_PERIOD;
    let mut charged = replay_frame::frame::Metrics::default();
    charged.table_ns[TableId::ProvinceSupply.index()] = 1_051_200;
    for table in [TableId::CountryFuel, TableId::CountryExperience] {
        assert!(super::slot_read(
            replay_data::format::FormatVersion::CURRENT,
            &charged,
            table,
            day + slot
        ));
        assert!(!super::slot_read(
            replay_data::format::FormatVersion::CURRENT,
            &charged,
            table,
            day + slot + 1
        ));
    }
    let mut full = replay_frame::frame::Metrics::default();
    full.table_ns[TableId::CountryExperience.index()] = 72_100;
    assert!(super::slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &full,
        TableId::CountryFuel,
        day + slot + 9
    ));
}

#[test]
fn a_recording_dates_tables_by_the_cadence_of_its_format() {
    use replay_data::format::FormatVersion;
    let previous =
        FormatVersion::read(replay_data::rows::compatibility::CADENCE_FORMAT - 1).unwrap();
    let day = 1000 * replay_data::cadence::STAGGER_PERIOD;
    let untimed = replay_frame::frame::Metrics::default();
    for (table, slot) in [(TableId::WorldTension, 18), (TableId::Projects, 4)] {
        for hour in day..day + replay_data::cadence::STAGGER_PERIOD {
            assert_eq!(
                super::slot_read(previous, &untimed, table, hour),
                hour == day + slot,
                "{table:?} at {hour}"
            );
            assert!(super::slot_read(
                FormatVersion::CURRENT,
                &untimed,
                table,
                hour
            ));
        }
    }
    let mut sibling = replay_frame::frame::Metrics::default();
    sibling.table_ns[TableId::TheatreAreas.index()] = 1;
    assert!(super::slot_read(
        previous,
        &sibling,
        TableId::WorldTension,
        day + 3
    ));
    let mut other = replay_frame::frame::Metrics::default();
    other.table_ns[TableId::ProvinceNames.index()] = 1;
    assert!(!super::slot_read(
        previous,
        &other,
        TableId::WorldTension,
        day + 3
    ));
}

#[test]
fn country_intel_is_dated_only_by_its_measured_walk() {
    let slot = daily_slot_of(TableId::CountryIntel);
    let hour = 1000 * replay_data::cadence::STAGGER_PERIOD + slot;
    assert!(!super::slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &replay_frame::frame::Metrics::default(),
        TableId::CountryIntel,
        hour
    ));
    let mut timed = replay_frame::frame::Metrics::default();
    timed.table_ns[TableId::CountryIntel.index()] = 1;
    assert!(super::slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &timed,
        TableId::CountryIntel,
        hour + 5
    ));
    let sibling = TableId::LogisticsEquipment;
    assert_eq!(daily_slot_of(sibling), slot);
    assert!(!super::slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &timed,
        sibling,
        hour + 5
    ));
    let mut sibling_timed = replay_frame::frame::Metrics::default();
    sibling_timed.table_ns[sibling.index()] = 1;
    for sampled_hour in [hour, hour + 5] {
        assert!(!super::slot_read(
            replay_data::format::FormatVersion::CURRENT,
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
        let mut metrics = replay_frame::frame::Metrics::default();
        for table in tables {
            metrics.table_ns[table.index()] = 1;
        }
        metrics
    };
    let slot = slot_hour(TableId::LogisticsEquipment);
    assert_eq!(slot, slot_hour(TableId::CountryIntel));
    let deferred = walked(&[TableId::LogisticsEquipment]);
    assert!(slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &deferred,
        TableId::LogisticsEquipment,
        slot
    ));
    assert!(!slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &deferred,
        TableId::CountryIntel,
        slot
    ));
    let retry = walked(&[TableId::CountryIntel]);
    assert!(slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &retry,
        TableId::CountryIntel,
        slot + 1
    ));
    assert!(!slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &retry,
        TableId::LogisticsEquipment,
        slot + 1
    ));

    let fuel = slot_hour(TableId::CountryFuel);
    let shared = walked(&[TableId::CountryFuelConsumers]);
    assert!(slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &shared,
        TableId::CountryFuel,
        fuel
    ));
    assert!(!slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &walked(&[]),
        TableId::CountryFuel,
        fuel + 1
    ));
    let off_slot = fuel + 3;
    assert!(slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &shared,
        TableId::CountryFuel,
        off_slot
    ));
    for table in [
        TableId::Stockpiles,
        TableId::ProductionLines,
        TableId::LogisticsEquipment,
        TableId::CountryIntel,
    ] {
        assert!(
            !slot_read(
                replay_data::format::FormatVersion::CURRENT,
                &shared,
                table,
                off_slot
            ),
            "{table:?}"
        );
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
        assert!(
            slot_read(
                replay_data::format::FormatVersion::CURRENT,
                &full,
                table,
                off_slot
            ),
            "{table:?}"
        );
    }
    let stock = slot_hour(TableId::Stockpiles);
    assert!(slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &walked(&[TableId::DeploymentLines]),
        TableId::ProductionLines,
        stock
    ));
    assert!(!slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &walked(&[]),
        TableId::Stockpiles,
        stock + 1
    ));
    for table in [TableId::CountryResources, TableId::ResourceExchanges] {
        assert!(slot_read(
            replay_data::format::FormatVersion::CURRENT,
            &walked(&[]),
            table,
            fuel
        ));
        assert!(slot_read(
            replay_data::format::FormatVersion::CURRENT,
            &walked(&[]),
            table,
            fuel + 2
        ));
    }
    assert!(slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &walked(&[]),
        TableId::CountryFuel,
        fuel
    ));
    assert!(slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &walked(&[]),
        TableId::LogisticsEquipment,
        slot
    ));
}

#[test]
fn recorded_gap_and_partial_reasons_and_campaign_identity_stay_exact() {
    use replay_recording::control::ControlRecord;
    use replay_capture_spool::spool::Record;
    use replay_frame::branches::Gap;
    let campaign = vec![0xa5; 1024 * 1024];
    let reason = "synthetic gap".to_owned() + &"\u{e9}".repeat(1024 * 1024);
    let mut input = records(1, vec![source(1, 100, Some(1))]);
    input.retain(|record| !matches!(record, Record::Frame(_)));
    input.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 100,
        last_hour: 101,
        reason: reason.clone(),
    }));
    input.push(Record::Control(ControlRecord::Partial {
        session: 1,
        reason: reason.clone(),
        failed_hour: None,
    }));
    let replay = Arc::new(Replay::from_records(recorder_derive::writers(), campaign.clone(), &[input]).unwrap());
    let metadata = super::read(
        &replay,
        Path::new("memory.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(metadata.branches[0].gaps[0].reason, reason);
    assert_eq!(metadata.partial.as_ref().unwrap().reason, reason);
    assert_eq!(
        super::partial_recording(replay.partial().unwrap())
            .unwrap()
            .unwrap()
            .reason,
        reason
    );
    assert_eq!(replay.gaps[0].reason, reason);
    assert_eq!(replay.campaign_id, campaign);
    assert_eq!(metadata.campaign_key.len(), campaign.len() * 2);
    assert!(
        metadata
            .campaign_key
            .as_bytes()
            .chunks_exact(2)
            .all(|pair| pair == b"a5")
    );
}

#[test]
fn damage_after_the_initial_catalogue_reports_a_gap_and_preserves_good_data() {
    let recording = Replay::from_records(
        recorder_derive::writers(),
        CAMPAIGN.into(),
        &[
            records(1, vec![source(1, 100, Some(1))]),
            records(2, vec![source(2, 200, Some(2))]),
        ],
    )
    .unwrap();
    let expected = match recording.seek(1, 100).unwrap() {
        replay::campaign::SeekResult::Frame(frame) => frame,
        other => panic!("{other:?}"),
    };
    let replay =
        Arc::new(Replay::open(recording.damaged_block(&recording.index[1]).unwrap()).unwrap());
    let metadata = super::read(
        &replay,
        Path::new("modified.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    let damaged = metadata.damaged.as_ref().unwrap();
    assert_eq!(damaged.parts, 1);
    assert_eq!((damaged.first_hour, damaged.last_hour), (Some(200), Some(200)));
    assert!(!damaged.reasons.is_empty());
    assert!(!metadata.warnings.is_empty());
    assert_eq!(metadata.branches[1].gaps.len(), 1);
    assert!(replay.seek(2, 200).is_err());
    let replay::campaign::SeekResult::Frame(actual) = replay.seek(1, 100).unwrap() else {
        panic!("good hour lost")
    };
    assert_eq!(actual, expected);
    assert!(
        metadata
            .countries
            .iter()
            .all(|country| country.observed_branch_id == "1")
    );
}

#[test]
fn malformed_table_after_the_catalogue_is_found_by_playback_not_by_opening() {
    let input = records(
        1,
        vec![source(1, 100, Some(1)), source(1, 101, Some(1))],
    );
    let recording = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), std::slice::from_ref(&input)).unwrap();
    let mut malformed = input;
    let replay_capture_spool::spool::Record::Frame(frame) = &mut malformed[3] else {
        unreachable!();
    };
    frame.tables[TableId::Countries.index()] = TableUpdate::Unchanged { baseline: 99 };
    let replay = Arc::new(Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[malformed]).unwrap());
    let metadata = super::read(
        &replay,
        Path::new("table.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(metadata.damaged.is_none());
    assert!(metadata.branches[0].gaps.is_empty());
    let replay::campaign::SeekResult::Frame(actual) = replay.seek(1, 101).unwrap() else {
        panic!("table damage lost entire hour")
    };
    let replay::campaign::SeekResult::Frame(expected) = recording.seek(1, 101).unwrap() else {
        panic!("fixture hour unavailable")
    };
    assert!(
        matches!(&*actual.tables[TableId::Countries.index()], TableView::Unavailable { reason } if reason.contains("baseline"))
    );
    let derived = recorder_derive::writers().derived_from(TableId::Countries);
    let TableView::Complete(TableRows::TagHolders(holders)) =
        &*actual.tables[TableId::TagHolders.index()]
    else {
        panic!("tag holders: {:?}", actual.tables[TableId::TagHolders.index()]);
    };
    assert_eq!(
        holders,
        &[replay_data::rows::TagHolderRow {
            number: replay_data::rows::TagHolderRow::UNKNOWN,
            epoch: 1,
            slot: 0,
            id: replay_data::rows::TagHolderRow::NONE,
            tag: replay_data::rows::TagHolderRow::REASONS[0].as_bytes().to_vec(),
        }],
        "the holder before is not carried over hours its countries were not read"
    );
    assert!(derived.contains(&TableId::TagHolders.index()));
    for table in TableId::ALL
        .into_iter()
        .filter(|table| *table != TableId::Countries && !derived.contains(&table.index()))
    {
        assert_eq!(
            actual.tables[table.index()],
            expected.tables[table.index()],
            "{table:?}"
        );
    }
}

#[test]
fn link_tags_projected_matches_native_bytes_unchanged_inputs_aliases_and_recovery() {
    use super::{CountryKeys, LinkTags, link_slots};
    let mut native = LinkTags::default();
    let mut projected = LinkTags::default();
    let first = crate::test_fixtures::world_frame(100, true);
    let mut balance = first.clone();
    balance.hour = 101;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *balance.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(77);
    }
    let mut alias = balance.clone();
    alias.hour = 102;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *alias.tables[TableId::CountryLinks.index()]
    {
        let at = rows.partition_point(|row| row.id < 99);
        rows.insert(
            at,
            replay_data::rows::CountryLinkRow {
                id: 99,
                canonical_slot: 9,
            },
        );
    }
    let mut rename = alias.clone();
    rename.hour = 103;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *rename.tables[TableId::Countries.index()]
    {
        rows.iter_mut()
            .find(|row| row.canonical_slot == 9)
            .unwrap()
            .tag = b"RENAMED".to_vec();
        rows.iter_mut()
            .find(|row| row.canonical_slot == 8)
            .unwrap()
            .tag = vec![0xff];
        rows.iter_mut()
            .find(|row| row.canonical_slot == 7)
            .unwrap()
            .tag
            .clear();
    }
    let mut same = rename.clone();
    same.hour = 104;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *same.tables[TableId::CountryLinks.index()]
    {
        let retained = rows.clone();
        *rows = retained;
    }
    for (index, frame) in [&first, &balance, &alias, &rename, &same]
        .into_iter()
        .enumerate()
    {
        let countries = CountryKeys::read(
            frame
                .complete::<replay_data::rows::tables::Countries>()
                .unwrap(),
        );
        let links = link_slots(
            frame
                .complete::<replay_data::rows::tables::CountryLinks>()
                .unwrap(),
        );
        let expected = native.update(frame).unwrap();
        assert_eq!(
            projected
                .update_projected(Some(&countries), Some(&links))
                .unwrap(),
            expected
        );
        assert_eq!(expected, matches!(index, 0 | 2 | 3));
        assert_eq!(
            serde_json::to_vec(&projected).unwrap(),
            serde_json::to_vec(&native).unwrap()
        );
        assert!(
            !projected
                .update_projected(Some(&countries), Some(&links))
                .unwrap()
        );
    }
    assert_eq!(projected.get(99).map(String::as_str), Some("RENAMED"));
    let mut missing = same.clone();
    *missing.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "missing fixture countries".into(),
    };
    assert!(native.update(&missing).is_err());
    let links = link_slots(
        same.complete::<replay_data::rows::tables::CountryLinks>()
            .unwrap(),
    );
    assert!(projected.update_projected(None, Some(&links)).is_err());
    assert_eq!(
        serde_json::to_vec(&projected).unwrap(),
        serde_json::to_vec(&native).unwrap()
    );
    let countries = CountryKeys::read(
        same.complete::<replay_data::rows::tables::Countries>()
            .unwrap(),
    );
    assert!(native.update(&same).unwrap());
    assert!(
        projected
            .update_projected(Some(&countries), Some(&links))
            .unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&projected).unwrap(),
        serde_json::to_vec(&native).unwrap()
    );
    assert!(projected.update_projected(Some(&countries), None).is_err());
    assert!(
        projected
            .update_projected(Some(&countries), Some(&links))
            .unwrap()
    );
}

#[test]
fn filtered_metadata_country_joins_record_their_source_once() {
    let country = replay_data::rows::CountryRow {
        id: 999,
        canonical_slot: 1,
        tag: b"SYN".to_vec(),
        ..Default::default()
    };
    let (_, refusals) = crate::triggers::refusal::capture(|| {
        assert!(super::link_slot(&[], 999).is_none());
        assert!(super::mapped_slot(&country, &[]).is_err());
        super::tagged_countries_indexed(&[country], &std::collections::HashMap::new());
        assert!(super::LinkTags::default().get(999).is_none());
    });
    assert_eq!(refusals.len(), 1);
    assert!(refusals.first().unwrap().names("CountryLinks"));
}

fn scalar_read_rule(
    format: replay_data::format::FormatVersion,
    metrics: &replay_frame::frame::Metrics,
    table: TableId,
    hour: i32,
) -> bool {
    if table == TableId::CountryIntel {
        return metrics.table_ns[table.index()] != 0;
    }
    let cadence = replay_data::cadence::cadence_in(table, format);
    cadence.is_refresh(hour)
        || TableId::ALL.into_iter().any(|other| {
            other != TableId::CountryIntel
                && replay_data::cadence::cadence_in(other, format) == cadence
                && metrics.table_ns[other.index()] != 0
        })
}

#[test]
fn scalar_supplying_reads_share_one_projection_across_holders() {
    use replay_data::format::FormatVersion;
    use replay_frame::frame::Metrics;
    super::SLOT_READS.with_borrow_mut(|held| *held = None);
    let before = super::slot_read_batches();
    let format = FormatVersion::CURRENT;
    let mut metrics = Metrics::default();
    metrics.table_ns[TableId::CountryFuel.index()] = 1;
    let hour = daily_slot_of(TableId::CountryFuel) + 1;
    let expected = scalar_read_rule(format, &metrics, TableId::CountryFuel, hour);
    for _ in 0..24 {
        assert_eq!(
            super::slot_read(format, &metrics, TableId::CountryFuel, hour),
            expected
        );
    }
    assert_eq!(
        super::slot_read_batches() - before,
        1,
        "one timed-table projection, independent of holder count"
    );
    for (at, timed) in [
        (hour, TableId::LexicalTokens),
        (hour + 1, TableId::CountryIntel),
        (hour + 3, TableId::CountryFuel),
    ] {
        metrics.table_ns.fill(0);
        metrics.table_ns[timed.index()] = replay_frame::frame::READ_TIME_NOT_RECORDED;
        for table in TableId::ALL {
            assert_eq!(
                super::slot_read(format, &metrics, table, at),
                scalar_read_rule(format, &metrics, table, at)
            );
        }
    }
    let previous = FormatVersion::read(format.version() - 1).unwrap();
    for table in TableId::ALL {
        assert_eq!(
            super::slot_read(previous, &metrics, table, hour),
            scalar_read_rule(previous, &metrics, table, hour)
        );
    }
    assert_eq!(super::slot_read_batches() - before, 5);
}

#[test]
fn country_key_changes_use_field_masks_and_snapshots_keep_full_comparison() {
    use super::CountryKeys;
    let count = 8;
    let mut input = scale_frame(1, 100, count);
    let baseline = input.complete::<tables::Countries>().unwrap().to_vec();
    let keys = CountryKeys::read(&baseline);
    let mut encoder = FrameEncoder::new();
    encoder.encode(input.clone(), true).unwrap();
    for (offset, edit) in ["balance", "tag", "slot", "delete"].into_iter().enumerate() {
        input.hour += 1;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *input.tables[TableId::Countries.index()]
        {
            match edit {
                "balance" => rows[0].political_power_raw = Some(123),
                "tag" => rows[0].tag = b"mod changed country".to_vec(),
                "slot" => rows[0].canonical_slot += count,
                "delete" => {
                    rows.remove(0);
                }
                _ => unreachable!(),
            }
        }
        let wire = encoder.encode(input.clone(), false).unwrap();
        let rows = input.complete::<tables::Countries>().unwrap();
        let delta = match &wire.tables[TableId::Countries.index()] {
            TableUpdate::Delta {
                changed, deleted, ..
            } => Some((changed.as_slice(), deleted.as_slice())),
            _ => panic!("fixture countries must encode a delta"),
        };
        let before = keys.row_visits();
        assert_eq!(keys.same_changes(rows, delta), offset == 0);
        if offset == 0 {
            assert_eq!(
                keys.row_visits() - before,
                1,
                "one mask must not compare {count} countries"
            );
        }
        assert_eq!(keys.same_changes(rows, delta), keys.same(rows));
    }
    let before = keys.row_visits();
    assert!(keys.same_changes(&baseline, Some((&[], &[]))));
    assert_eq!(
        keys.row_visits(),
        before,
        "an equal fresh acquisition visits no country key"
    );
    assert!(keys.same_changes(&baseline, None));
    assert_eq!(
        keys.row_visits() - before,
        count as usize,
        "a snapshot rechecks every key"
    );
}

#[test]
fn every_wire_vector_string_and_summary_copy_fails_by_name_under_real_denied_growth() {
    let replay = Arc::new(
        Replay::from_records(
            recorder_derive::writers(),
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 100, Some(1))])],
        )
        .unwrap(),
    );
    let mut resident = super::read_resident(
        &replay,
        Path::new("wire.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    resident.value.branches[0].fork = Some(crate::BranchFork {
        branch_id: "parent".into(),
        hour: 100,
    });
    resident.value.branches[0].joins.push(crate::BranchJoin {
        hour: 101,
        segment_id: "segment".into(),
        parent_id: "earlier".into(),
        evidence: crate::JoinEvidence::Unconfirmed,
        choices: vec![crate::JoinChoice {
            segment_id: "choice".into(),
            first_hour: 90,
            last_hour: 100,
        }],
    });
    resident.value.branches[0].gaps.push(crate::RecordingGap {
        first_hour: 101,
        last_hour: 101,
        reason: "gap".into(),
    });
    resident.value.branches[0].event_losses = Some(vec![crate::RecordingEventLoss {
        after_hour: 99,
        hour: 100,
        reason: "loss".into(),
    }]);
    resident.value.player_country_id = Some("7".into());
    resident.value.partial = Some(crate::PartialRecording {
        session_id: "1".into(),
        reason: "partial".into(),
        failed_hour: Some(101),
    });
    let mut warnings = super::Warnings::new().unwrap();
    let mut reasons = super::Warnings::new().unwrap();
    warnings.warn("stored warning").unwrap();
    reasons.warn("stored reason").unwrap();
    super::with_storage_failure_after(0, || {
        warnings.warn("refused warning").unwrap();
        reasons.warn("refused reason").unwrap();
    });
    let (values, refused) = warnings.finish();
    resident.value.warnings = values;
    resident.refusals.warnings = refused;
    let (values, refused) = reasons.finish();
    resident.value.damaged = Some(crate::DamagedRecording {
        parts: 1,
        first_hour: Some(101),
        last_hour: Some(101),
        reasons: values,
    });
    resident.refusals.reasons = refused;
    let (healthy, allocations) = crate::test_allocator::deny_after(usize::MAX, || resident.wire());
    let healthy = healthy.unwrap();
    assert!(allocations > 0);
    for allowed in 0..allocations {
        let (failed, _) = crate::test_allocator::deny_after(allowed, || resident.wire());
        let error = failed.unwrap_err();
        assert!(
            matches!(error, super::MetadataError::WireAllocationFailed),
            "allocation {allowed}"
        );
        assert_eq!(error.to_string(), "Metadata wire allocation failed");
    }
    assert_eq!(resident.wire().unwrap(), healthy);
    let (moved, attempts) = crate::test_allocator::deny_after(0, || resident.into_wire());
    assert_eq!(attempts, 0);
    assert_eq!(moved.unwrap(), healthy);
}

#[test]
fn a_restored_wire_vector_without_a_spare_slot_returns_a_named_failure_without_allocating() {
    let replay = Arc::new(
        Replay::from_records(
            recorder_derive::writers(),
            CAMPAIGN.into(),
            &[records(1, vec![source(1, 100, Some(1))])],
        )
        .unwrap(),
    );
    let mut resident = super::read_resident(
        &replay,
        Path::new("wire.battleplan"),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut warnings = super::Warnings::new().unwrap();
    warnings.warn("retained").unwrap();
    super::with_storage_failure_after(0, || warnings.warn("refused").unwrap());
    let (mut values, refused) = warnings.finish();
    values.shrink_to_fit();
    resident.value.warnings = values;
    resident.refusals.warnings = refused;
    let (result, attempts) = crate::test_allocator::deny_after(0, || resident.into_wire());
    assert!(attempts > 0);
    assert!(matches!(
        result,
        Err(super::MetadataError::WireAllocationFailed)
    ));
}
