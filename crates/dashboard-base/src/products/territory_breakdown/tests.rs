use super::*;
use replay_data::{
    rows::CampaignRow, rows::CountryLinkRow, rows::CountryRow, rows::ProvinceRow, rows::StateRow,
    rows::TableId, rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::TableView};

#[test]
fn territory_breakdown_country_operand_refresh_retains_the_link_map() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut read = frame(100, 3);
    read.metrics = Default::default();
    let region_inputs = regions(3);
    let mut prepared = Prepared {
        regions: Some(regions(3)),
        ..Default::default()
    };
    prepared.scan.format = format;
    let mut oracle = Scan {
        format,
        ..Default::default()
    };
    let mut builder = Builder::default();
    let mut tracker = FrameChangeTracker::new();
    prepared
        .observe_with(&read, None, Some(&tracker.observe(&read, format)))
        .unwrap();
    observe(&mut oracle, &read, &region_inputs, None, &mut builder, None).unwrap();
    let slots = Arc::clone(prepared.builder.lineage.as_ref().unwrap().reads.slots());
    for hour in 101..=102 {
        read.hour = hour;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *read.tables[TableId::Countries.index()]
        {
            rows[1].tag = format!("recorded region alias {hour}").into_bytes();
        }
        prepared
            .observe_with(&read, None, Some(&tracker.observe(&read, format)))
            .unwrap();
        observe(&mut oracle, &read, &region_inputs, None, &mut builder, None).unwrap();
        assert!(Arc::ptr_eq(
            &slots,
            prepared.builder.lineage.as_ref().unwrap().reads.slots()
        ));
    }
    read.hour = 103;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *read.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 200_000,
            canonical_slot: 2,
        });
    }
    prepared
        .observe_with(&read, None, Some(&tracker.observe(&read, format)))
        .unwrap();
    observe(&mut oracle, &read, &region_inputs, None, &mut builder, None).unwrap();
    let changed = Arc::clone(prepared.builder.lineage.as_ref().unwrap().reads.slots());
    assert!(!Arc::ptr_eq(&slots, &changed));
    assert_eq!(changed.get(&200_000), Some(&2));
    prepared.builder.seal(&mut prepared.scan);
    observer_tests::assert_reply_parity(&prepared.scan, &oracle).unwrap();
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    prepared.regions = Some(regions(3));
    tracker.invalidate();
    read.hour = 104;
    prepared
        .observe_with(&read, None, Some(&tracker.observe(&read, format)))
        .unwrap();
    observe(&mut oracle, &read, &region_inputs, None, &mut builder, None).unwrap();
    assert!(!Arc::ptr_eq(
        &changed,
        prepared.builder.lineage.as_ref().unwrap().reads.slots()
    ));
    prepared.builder.seal(&mut prepared.scan);
    observer_tests::assert_reply_parity(&prepared.scan, &oracle).unwrap();
}

#[test]
fn territory_breakdown_shared_driver_defers_regions_and_publishes_exact_endpoints() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let format = replay_data::format::FormatVersion::CURRENT;
    let work = observer_tests::scaled_work(10, |hours| {
        let mut read = frame(100, 64);
        read.metrics = Default::default();
        let mut prepared = Prepared {
            regions: Some(regions(64)),
            ..Default::default()
        };
        prepared.scan.format = format;
        let changes = FrameChangeTracker::new().observe(&read, format);
        prepared.observe_with(&read, None, Some(&changes))?;
        let delta = observer_tests::unchanged_frames(
            prepared.builder.work,
            &read,
            hours as i32,
            format,
            |frame, changes| {
                prepared.observe_with(frame, None, Some(changes))?;
                Ok(prepared.builder.work)
            },
        )?;
        assert_eq!(delta, observer_tests::Work::default());
        prepared.builder.seal(&mut prepared.scan);
        let mut oracle = Scan {
            format,
            ..Default::default()
        };
        let mut builder = Builder::default();
        for hour in 100..=100 + hours as i32 {
            read.hour = hour;
            observe(&mut oracle, &read, &regions(64), None, &mut builder, None)?;
        }
        observer_tests::assert_reply_parity(&prepared.scan, &oracle)?;
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(work);
}

#[test]
fn territory_breakdown_one_changed_province_skips_closed_region_history() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let format = replay_data::format::FormatVersion::CURRENT;
    let work = observer_tests::scaled_work(10, |history| {
        let mut read = frame(100, 2);
        read.metrics = Default::default();
        let mut prepared = Prepared {
            regions: Some(regions(2)),
            ..Default::default()
        };
        prepared.scan.format = format;
        let mut tracker = FrameChangeTracker::new();
        for index in 0..history {
            read.hour = 100 + index as i32;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *read.tables[TableId::Countries.index()]
            {
                rows[0].tag = format!("closed region alias {index}").into_bytes();
            }
            prepared.observe_with(&read, None, Some(&tracker.observe(&read, format)))?;
        }
        let before = prepared.builder.work;
        read.hour += 1;
        if let TableView::Complete(TableRows::Provinces(rows)) =
            &mut *read.tables[TableId::Provinces.index()]
        {
            rows[0].victory_points += 1;
        }
        prepared.observe_with(&read, None, Some(&tracker.observe(&read, format)))?;
        let delta = observer_tests::Work {
            row_visits: prepared.builder.work.row_visits - before.row_visits,
            history_visits: prepared.builder.work.history_visits - before.history_visits,
            emitted: prepared.builder.work.emitted - before.emitted,
            ..Default::default()
        };
        assert_eq!(delta.row_visits, 2);
        assert_eq!(delta.history_visits, 1);
        assert_eq!(delta.emitted, 1);
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(work);
}

#[test]
#[cfg(feature = "slow-tests")]
fn territory_breakdown_shared_driver_keeps_all_5000_country_region_keys_without_hourly_work() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut read = frame(100, 5_000);
    read.metrics = Default::default();
    let mut prepared = Prepared {
        regions: Some(regions(5_000)),
        ..Default::default()
    };
    prepared.scan.format = format;
    prepared
        .observe_with(
            &read,
            None,
            Some(&FrameChangeTracker::new().observe(&read, format)),
        )
        .unwrap();
    let delta = observer_tests::unchanged_frames(
        prepared.builder.work,
        &read,
        100,
        format,
        |frame, changes| {
            prepared.observe_with(frame, None, Some(changes))?;
            Ok(prepared.builder.work)
        },
    )
    .unwrap();
    assert_eq!(delta, observer_tests::Work::default());
    prepared.builder.seal(&mut prepared.scan);
    assert_eq!(prepared.builder.active.len(), 5_000);
    assert!(
        prepared
            .scan
            .series
            .iter()
            .all(|series| series.runs.last().unwrap().last_hour == 200)
    );
}

#[test]
fn territory_breakdown_prepared_reply_matches_after_two_commits_resume_and_append() {
    let mut fixture = crate::prepared::series::tests::Fixture::new();
    let cancelled = AtomicBool::new(false);
    let mut prepared = Prepared {
        regions: Some(regions(3)),
        ..Default::default()
    };
    let mut reference = Scan::default();
    let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
    let mut builder = Builder::default();
    for (commit, hours) in [vec![100, 101], vec![102, 104], vec![105, 106]]
        .into_iter()
        .enumerate()
    {
        for hour in hours {
            let mut frame = frame(hour, 3);
            if hour >= 104
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].tag = b"renamed realm".to_vec();
            }
            observe(
                &mut reference,
                &frame,
                &regions(3),
                None,
                &mut builder,
                None,
            )
            .unwrap();
            let changes = tracker.observe(&frame, Default::default());
            prepared.observe_with(&frame, None, Some(&changes)).unwrap();
        }
        let mut writer = if commit == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared.flush(&mut writer, 1, &cancelled).unwrap();
        writer
            .write_at(
                "regional-continuation",
                0,
                serde_json::to_vec(&prepared).unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_reader(
            fixture
                .store
                .read(&fixture.key, "regional-continuation", 0)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        prepared.regions = Some(regions(3));
        tracker.invalidate();
        let totals = || super::super::territory::TerritoryReply {
            branch_id: "1".into(),
            world_totals: vec![],
            series: vec![],
            area_unit: None,
            area_reason: None,
            warnings: vec![],
        };
        let actual = Prepared::reply(
            &fixture.store,
            &fixture.key,
            1,
            100,
            106,
            totals(),
            &cancelled,
        )
        .unwrap();
        let mut expected = build_reply(&reference, &regions(3), totals()).unwrap();
        expected.series.sort_unstable_by(|a, b| {
            (&a.tag, a.region, &a.missing).cmp(&(&b.tag, b.region, &b.missing))
        });
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "commit {commit}"
        );
    }
}

#[test]
fn territory_region_names_keep_successful_empty_text_distinct_from_failure() {
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "map/strategicregions/mod.txt",
            "strategic_region={id=11 name=EMPTY} strategic_region={id=12} strategic_region={id=13 name=ABSENT} strategic_region={id=14 name=REFUSED}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n EMPTY:0 \"\"\n REFUSED:0 \"a\u{a4}b\"\n",
        ),
    ]);
    let regions = Regions::read(&catalogue).unwrap();
    for (region, name) in regions.names[..2].iter().zip(["", ""]) {
        assert_eq!(region.name.as_deref(), Some(name));
        assert!(region.missing.is_none());
    }
    for region in &regions.names[2..] {
        assert!(region.name.is_none());
        assert!(region.missing.is_some());
    }
    assert!(
        regions
            .warnings
            .iter()
            .any(|warning| { warning.starts_with("2 strategic region names unavailable") })
    );
}

fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "outside fixture".into(),
    });
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: b"analytics-test".to_vec(),
        }]));
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("realm {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=count)
            .map(|id| StateRow {
                id,
                owner: id,
                controller: id,
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        (1..=count)
            .map(|id| ProvinceRow {
                id,
                state: id,
                owner: id,
                controller: id,
                victory_points: -7,
                ..Default::default()
            })
            .collect(),
    ));
    let mut metrics = Metrics::default();
    metrics.table_ns[TableId::States.index()] = 1;
    metrics.table_ns[TableId::Provinces.index()] = 1;
    DecodedFrame {
        branch: 1,
        hour,
        tables: tables.map(Into::into),
        metrics,
    }
}
fn regions(count: u32) -> Regions {
    Regions {
        provinces: (1..=count).map(|id| (id, Some(id))).collect(),
        states: (1..=count)
            .map(|id| {
                (
                    id,
                    vec![StatePart {
                        region: Some(id),
                        numerator: 1,
                        denominator: 1,
                    }],
                )
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn every_country_region_and_signed_value_survives_without_a_count_ceiling() {
    let mut scan = Scan::default();
    let mut builder = Builder::default();
    observe(
        &mut scan,
        &frame(100, 5_000),
        &regions(5_000),
        None,
        &mut builder,
        None,
    )
    .unwrap();
    assert_eq!(scan.series.len(), 5_000);
    for series in &scan.series {
        assert!(series.tag.is_some());
        assert_eq!(
            series.runs[0].values,
            [
                Some("1".into()),
                Some("-7".into()),
                None,
                Some("1".into()),
                Some("-7".into()),
                None
            ]
        );
        assert_eq!(
            series.runs[0].state_shares[0].as_ref().unwrap(),
            &[TerritoryStateShare {
                numerator: "1".into(),
                denominator: 1
            }]
        );
    }
}
#[test]
fn gaps_and_reused_tags_start_new_stretches_without_inventing_zeroes() {
    let mut scan = Scan::default();
    let mut builder = Builder::default();
    let regions = regions(1);
    observe(
        &mut scan,
        &frame(100, 1),
        &regions,
        None,
        &mut builder,
        None,
    )
    .unwrap();
    observe(
        &mut scan,
        &frame(102, 1),
        &regions,
        None,
        &mut builder,
        None,
    )
    .unwrap();
    let mut renamed = frame(103, 1);
    *renamed.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 2,
            canonical_slot: 1,
            tag: b"realm 1".to_vec(),
            ..Default::default()
        }]));
    *renamed.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 1,
            },
        ]));
    observe(&mut scan, &renamed, &regions, None, &mut builder, None).unwrap();
    let runs = &scan.series[0].runs;
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
            .collect::<Vec<_>>(),
        [(100, 100, true), (102, 102, true), (103, 103, true)]
    );
}
#[test]
fn multi_region_states_split_exactly_while_unknown_holders_keep_named_parts() {
    let mut frame = frame(100, 1);
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        ProvinceRow {
            id: 1,
            state: 1,
            owner: 1,
            controller: 1,
            ..Default::default()
        },
        ProvinceRow {
            id: 2,
            state: 1,
            owner: 0,
            controller: 99,
            victory_points: -9,
            ..Default::default()
        },
    ]));
    let mut scan = Scan::default();
    let mut regions = regions(2);
    regions
        .states
        .insert(1, state_parts(&[1, 2], &regions.provinces).unwrap());
    observe(
        &mut scan,
        &frame,
        &regions,
        None,
        &mut Builder::default(),
        None,
    )
    .unwrap();
    for region in [1, 2] {
        let state = scan
            .series
            .iter()
            .find(|series| {
                series.tag.as_deref() == Some("realm 1") && series.region == Some(region)
            })
            .unwrap();
        assert_eq!(
            state.runs[0].state_shares[0].as_ref().unwrap(),
            &[TerritoryStateShare {
                numerator: "1".into(),
                denominator: 2
            }]
        );
        assert!(state.missing.is_none());
    }
    let unknown = scan
        .series
        .iter()
        .find(|series| {
            series.tag.is_none()
                && series
                    .missing
                    .as_deref()
                    .is_some_and(|m| m.starts_with("Country"))
        })
        .unwrap();
    assert_eq!(unknown.runs[0].values[1].as_deref(), Some("-9"));
    assert!(
        scan.series
            .iter()
            .any(|series| series.tag.is_none() && series.missing.is_none())
    );
}

#[test]
fn unchanged_tables_compress_but_unavailable_tables_cannot_reuse_a_known_measure() {
    let first = frame(100, 1);
    let mut same = first.clone();
    same.hour = 101;
    let mut stale = same.clone();
    stale.hour = 102;
    *stale.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "unreadable states".into(),
    };
    let mut scan = Scan::default();
    let mut builder = Builder::default();
    let regions = regions(1);
    for read in [&first, &same, &stale] {
        observe(&mut scan, read, &regions, None, &mut builder, None).unwrap();
    }
    let runs = &scan.series[0].runs;
    assert_eq!((runs[0].first_hour, runs[0].last_hour), (100, 101));
    assert_eq!(runs[1].state_shares[0], None);
    assert_eq!(runs[1].values[0].as_deref(), Some("1"));
}

#[test]
fn native_content_roster_keeps_repeats_and_unattributed_shares_without_a_province_read() {
    let mut regions = regions(2);
    regions
        .states
        .insert(1, state_parts(&[1, 1, 2, 99], &regions.provinces).unwrap());
    let mut read = frame(100, 1);
    *read.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "province read failed".into(),
    };
    let mut scan = Scan::default();
    observe(
        &mut scan,
        &read,
        &regions,
        None,
        &mut Builder::default(),
        None,
    )
    .unwrap();
    let shares: BTreeMap<_, _> = scan
        .series
        .iter()
        .map(|series| (series.region, series.runs[0].state_shares[0].clone()))
        .collect();
    for (region, numerator) in [(Some(1), "2"), (Some(2), "1"), (None, "1")] {
        assert_eq!(
            shares[&region].as_ref().unwrap(),
            &[TerritoryStateShare {
                numerator: numerator.into(),
                denominator: 4
            }]
        );
    }
    assert!(
        scan.series
            .iter()
            .all(|series| series.runs[0].values[0].is_none())
    );
}

#[test]
fn state_terms_combine_by_denominator_without_rounding_or_lcm_limits() {
    let mut regions = regions(2);
    regions
        .states
        .insert(1, state_parts(&[1, 2], &regions.provinces).unwrap());
    regions
        .states
        .insert(2, state_parts(&[1, 1, 2], &regions.provinces).unwrap());
    let mut read = frame(100, 1);
    *read.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            owner: 1,
            controller: 1,
            ..Default::default()
        },
        StateRow {
            id: 2,
            owner: 1,
            controller: 1,
            ..Default::default()
        },
    ]));
    let mut scan = Scan::default();
    observe(
        &mut scan,
        &read,
        &regions,
        None,
        &mut Builder::default(),
        None,
    )
    .unwrap();
    let first = scan
        .series
        .iter()
        .find(|series| series.region == Some(1))
        .unwrap();
    assert_eq!(
        first.runs[0].state_shares[0].as_ref().unwrap(),
        &[
            TerritoryStateShare {
                numerator: "1".into(),
                denominator: 2
            },
            TerritoryStateShare {
                numerator: "2".into(),
                denominator: 3
            },
        ]
    );
}

#[test]
fn restored_breakdown_closes_a_column_unknown_at_the_first_hour_after_a_commit() {
    use crate::prepared::observer_tests;
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    // States are unreadable at hour 102, the first hour after a commit.
    let frames: Vec<_> = (100..104)
        .map(|hour| {
            let mut frame = frame(hour, 3);
            if hour == 102 {
                *frame.tables[TableId::States.index()] = TableView::Unavailable {
                    reason: "states unavailable".into(),
                };
            }
            frame
        })
        .collect();
    observer_tests::assert_restore_continues(
        format,
        &[&frames[..2], &frames[2..]],
        || Prepared {
            regions: Some(regions(3)),
            ..Default::default()
        },
        |prepared: &mut Prepared, frame, changes, _| {
            prepared.observe(frame, format, None, changes).unwrap()
        },
        |prepared: &mut Prepared, writer, _| {
            prepared.flush(writer, 1, &flag).unwrap();
            writer
                .write_at(
                    "regional-continuation",
                    0,
                    serde_json::to_vec(&*prepared).unwrap().as_slice(),
                )
                .unwrap();
        },
        |fixture| {
            let mut prepared = serde_json::from_reader::<_, Prepared>(
                fixture
                    .store
                    .read(&fixture.key, "regional-continuation", 0)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            // Preparation configures each block's observer before its first frame.
            prepared.regions = Some(regions(3));
            prepared
        },
    );
}

/// Three realms over strategic regions one to four, with a province held by an
/// unresolved link and an unassigned controller. Realm 2's province changes
/// controller at 102, hour 103 is not recorded, realm 1 is renamed at 104,
/// realm 2's native holder is replaced at 105, hour 106 has no province read,
/// and realm 3 is annexed at 107 while it still exists.
fn regional(hour: i32) -> DecodedFrame {
    let mut frame = frame(hour, 3);
    let replaced = hour >= 105;
    let annexed = hour >= 107;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        if hour >= 104 {
            rows[0].tag = b"renamed realm".to_vec();
        }
        if replaced {
            rows[1].id = 2002;
        }
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        if replaced {
            rows[1].id = 2002;
        }
        rows.sort_by_key(|row| row.id);
    }
    let holder = |raw: u32| match raw {
        2 if replaced => 2002,
        3 if annexed => 1,
        raw => raw,
    };
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *frame.tables[TableId::States.index()]
    {
        for row in rows.iter_mut() {
            row.owner = holder(row.owner);
            row.controller = holder(row.controller);
        }
    }
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    {
        for row in rows.iter_mut() {
            row.owner = holder(row.owner);
            row.controller = holder(row.controller);
            if hour >= 102 && row.id == 2 {
                row.controller = 1;
            }
            row.victory_points = (hour + row.id as i32) % 3;
        }
        rows.push(ProvinceRow {
            id: 4,
            state: 1,
            owner: 999,
            controller: 0,
            victory_points: 5,
            ..Default::default()
        });
    }
    if hour == 106 {
        *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        };
    }
    frame
}

/// Hours recorded in each replay block; hour 103 lies between blocks.
const REGIONAL_BLOCKS: [&[i32]; 7] = [
    &[100, 101, 102],
    &[104, 105],
    &[106, 107],
    &[108, 109],
    &[110, 111],
    &[112, 113],
    &[114, 115],
];

fn regional_totals() -> super::super::territory::TerritoryReply {
    super::super::territory::TerritoryReply {
        branch_id: "1".into(),
        world_totals: vec![],
        series: vec![],
        area_unit: None,
        area_reason: None,
        warnings: vec![],
    }
}

/// Prepare the given commits through the production observer, resuming its
/// published continuation between commits as preparation does.
fn prepare_regional(
    fixture: &crate::prepared::series::tests::Fixture,
    commits: &[&[i32]],
    format: replay_data::format::FormatVersion,
) {
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared {
        regions: Some(regions(4)),
        ..Default::default()
    };
    let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
    for hours in commits {
        for &hour in *hours {
            let frame = regional(hour);
            let changes = tracker.observe(&frame, format);
            prepared.observe(&frame, format, None, &changes).unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer
            .write_at(
                "regional-continuation",
                0,
                serde_json::to_vec(&prepared).unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_reader(
            fixture
                .store
                .read(&fixture.key, "regional-continuation", 0)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        prepared.regions = Some(regions(4));
        tracker.invalidate();
    }
}

/// Everything a regional reply dates at `hour`. A transient read knows a run
/// only at the hour itself, so run extents, segment starts and observed spans
/// beyond the hour are not compared.
/// A series the frame does not reach is absent: preparation's zero runs for a
/// holder without land there, and its unknown runs at an unread hour, carry no
/// value the frame lacks, while `observed` says which columns the hour proves.
fn at_hour(reply: &TerritoryBreakdownReply, hour: i32) -> serde_json::Value {
    let series: Vec<_> = reply
        .series
        .iter()
        .filter_map(|series| {
            let run = series
                .runs
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour)?;
            let evidence = run.values.iter().flatten().any(|value| value != "0")
                || run
                    .state_shares
                    .iter()
                    .flatten()
                    .any(|terms| !terms.is_empty());
            evidence.then(|| {
                serde_json::json!([
                    series.tag,
                    series.region,
                    series.missing,
                    run.values,
                    run.state_shares
                ])
            })
        })
        .collect();
    // Observed columns at the hour itself: a prepared reply for one hour is
    // clipped to it, while a continuous scan's spans run on.
    let observed: Vec<Vec<HourSpan>> = reply
        .observed
        .iter()
        .map(|spans| {
            spans
                .iter()
                .filter(|span| span.first_hour <= hour && hour <= span.last_hour)
                .map(|_| HourSpan {
                    first_hour: hour,
                    last_hour: hour,
                })
                .collect()
        })
        .collect();
    serde_json::json!({
        "series": series,
        "observed": observed,
        "regions": reply.regions,
        "countries": reply.countries,
        "worldTotals": reply.world_totals,
        "warnings": reply.warnings,
    })
}

#[test]
fn regional_hours_answer_beyond_preparation_from_one_frame_as_preparation_does() {
    use crate::test_fixtures::{CAMPAIGN, continuation_records, records};
    let mut replay = replay::container::Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            REGIONAL_BLOCKS[0]
                .iter()
                .map(|&hour| regional(hour))
                .collect(),
        )],
    )
    .unwrap();
    for block in &REGIONAL_BLOCKS[1..] {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                block.iter().map(|&hour| regional(hour)).collect(),
            )])
            .unwrap();
    }
    assert_eq!(replay.index.len(), REGIONAL_BLOCKS.len());
    let format = replay.format;
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "regional-progressive".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let flag = AtomicBool::new(false);
    let cold = |hour: i32| {
        sample_reply(
            read_frame(&source, 1, hour, &flag).unwrap().as_deref(),
            format,
            &regions(4),
            None,
            regional_totals(),
        )
        .unwrap()
    };
    // A published prefix through hour 107, in two commits.
    let prefix = crate::prepared::series::tests::Fixture::new();
    prepare_regional(
        &prefix,
        &[REGIONAL_BLOCKS[0], &[104, 105, 106, 107]],
        format,
    );
    // The whole branch, in three commits.
    let full = crate::prepared::series::tests::Fixture::new();
    prepare_regional(
        &full,
        &[
            REGIONAL_BLOCKS[0],
            &[104, 105, 106, 107],
            &[108, 109, 110, 111, 112, 113, 114, 115],
        ],
        format,
    );
    let prepared = |fixture: &crate::prepared::series::tests::Fixture, hour: i32| {
        Prepared::reply(
            &fixture.store,
            &fixture.key,
            1,
            hour,
            hour,
            regional_totals(),
            &flag,
        )
        .unwrap()
    };
    for hour in 100..=115 {
        let direct = cold(hour);
        let expected = prepared(&full, hour);
        let joined = if hour <= 107 {
            prepared(&prefix, hour)
        } else {
            direct.clone()
        };
        assert_eq!(
            at_hour(&direct, hour),
            at_hour(&expected, hour),
            "cold {hour}"
        );
        assert_eq!(
            at_hour(&joined, hour),
            at_hour(&expected, hour),
            "prefix and tail {hour}"
        );
    }

    // Cold reads answer with values, never pending.
    let late = cold(115);
    assert!(late.series.iter().any(|series| {
        series.tag.as_deref() == Some("renamed realm")
            && series.region == Some(3)
            && series.runs[0].values[0].as_deref() == Some("1")
    }));
    assert!(
        late.series
            .iter()
            .any(|series| series.tag.is_none() && series.region == Some(4)),
        "unresolved and unassigned holders keep their parts"
    );
    let replaced = cold(105);
    assert!(replaced.series.iter().any(|series| {
        series.tag.as_deref() == Some("realm 2") && series.runs[0].values[3].as_deref() == Some("1")
    }));
    assert!(
        replaced
            .series
            .iter()
            .all(|series| series.tag.as_deref() != Some("realm 1")),
        "the renamed holder appears only under its recorded tag"
    );
    // An annexed realm that still exists has a zero run when prepared, and no
    // series on a direct read: absence, never an invented value.
    assert!(
        cold(107)
            .series
            .iter()
            .all(|series| series.tag.as_deref() != Some("realm 3"))
    );
    assert!(prepared(&full, 107).series.iter().any(|series| {
        series.tag.as_deref() == Some("realm 3")
            && series
                .runs
                .iter()
                .any(|run| run.first_hour <= 107 && run.values[0].as_deref() == Some("0"))
    }));
    // An unread province table proves only the state columns.
    let unread = cold(106);
    assert!(unread.observed[1].is_empty() && unread.observed[5].is_empty());
    assert_eq!(
        unread.observed[0],
        vec![HourSpan {
            first_hour: 106,
            last_hour: 106
        }]
    );
    // The unrecorded hour reads nothing.
    let gap = cold(103);
    assert!(gap.series.is_empty());
    assert!(gap.observed.iter().all(Vec::is_empty));
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        read_frame(&source, 1, 115, &cancelled).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn regional_sample_matches_the_continuous_scan_for_5000_countries() {
    const COUNT: u32 = 5_000;
    let format = replay_data::format::FormatVersion::CURRENT;
    let inputs = regions(COUNT);
    let frames: Vec<_> = (100..=102)
        .map(|hour| {
            let mut frame = frame(hour, COUNT);
            if hour >= 101 {
                if let TableView::Complete(TableRows::Provinces(rows)) =
                    &mut *frame.tables[TableId::Provinces.index()]
                {
                    for row in rows.iter_mut().filter(|row| row.id % 7 == 0) {
                        row.controller = row.id % COUNT + 1;
                    }
                }
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].tag = b"renamed realm".to_vec();
                }
            }
            frame
        })
        .collect();
    let mut oracle = Scan {
        format,
        ..Default::default()
    };
    let mut builder = Builder::default();
    for frame in &frames {
        observe(&mut oracle, frame, &inputs, None, &mut builder, None).unwrap();
    }
    oracle.series.sort_unstable_by(|a, b| {
        (&a.tag, a.region, &a.missing).cmp(&(&b.tag, b.region, &b.missing))
    });
    let scanned = build_reply(&oracle, &inputs, regional_totals()).unwrap();
    for frame in &frames {
        let sampled = sample_reply(Some(frame), format, &inputs, None, regional_totals()).unwrap();
        assert!(sampled.series.len() >= COUNT as usize);
        assert_eq!(
            at_hour(&sampled, frame.hour),
            at_hour(&scanned, frame.hour),
            "hour {}",
            frame.hour
        );
    }
}
