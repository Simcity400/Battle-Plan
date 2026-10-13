use super::*;
use replay::{
    frame::{Metrics, SharedView, TableView},
    rows::{CampaignRow, CountryLinkRow, CountryRow, ProvinceRow, StateRow, TableRows},
};
use std::sync::atomic::Ordering;

fn frame(hour: i32, countries: u32, provinces: u32) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "outside territory fixture".into(),
    });
    tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: b"analytics-test".to_vec(),
        }]));
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..countries)
            .map(|slot| CountryRow {
                id: 100_000 + slot,
                canonical_slot: slot,
                tag: format!("realm {slot:05}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..countries)
            .map(|slot| CountryLinkRow {
                id: 100_000 + slot,
                canonical_slot: slot,
            })
            .collect(),
    ));
    tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=countries)
            .map(|id| StateRow {
                id,
                owner: 100_000 + (id - 1) % countries,
                controller: 100_000 + id % countries,
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        (1..=provinces)
            .map(|id| ProvinceRow {
                id,
                owner: 100_000 + (id - 1) % countries,
                controller: 100_000 + id % countries,
                victory_points: if id == 1 { -7 } else { id as i32 },
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

fn replay(frames: Vec<DecodedFrame>) -> Replay {
    Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, frames)],
    )
    .unwrap()
}
fn areas(count: u32) -> ProvinceAreas {
    ProvinceAreas {
        land: (0..=count).map(|id| i64::from(id) * 10).collect(),
    }
}

fn raster_areas(count: u32) -> ProvinceAreas {
    let stride = (count as usize * 3).div_ceil(4) * 4;
    let mut bitmap = vec![0; 54 + stride];
    bitmap[..2].copy_from_slice(b"BM");
    bitmap[2..6].copy_from_slice(&((54 + stride) as u32).to_le_bytes());
    bitmap[10..14].copy_from_slice(&54u32.to_le_bytes());
    bitmap[14..18].copy_from_slice(&40u32.to_le_bytes());
    bitmap[18..22].copy_from_slice(&(count as i32).to_le_bytes());
    bitmap[22..26].copy_from_slice(&1i32.to_le_bytes());
    bitmap[26..28].copy_from_slice(&1u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24u16.to_le_bytes());
    let mut definitions = String::from("0;0;0;0;land\n");
    for id in 1..=count {
        let [blue, green, red, _] = id.to_le_bytes();
        definitions.push_str(&format!("{id};{red};{green};{blue};land\n"));
        let at = 54 + (id as usize - 1) * 3;
        bitmap[at..at + 3].copy_from_slice(&[blue, green, red]);
    }
    ProvinceAreas::from_assets(definitions.as_bytes(), &bitmap).unwrap()
}

#[test]
fn modded_extra_states_and_provinces_with_a_renamed_country_use_the_same_raster_ids() {
    let mut decoded = frame(100, 3, 8);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *decoded.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"Renamed realm".to_vec();
    }
    let areas = raster_areas(8);
    let scanned = scan(
        &replay(vec![decoded]),
        1,
        Some(&areas),
        &AtomicBool::new(false),
    )
    .unwrap();
    let series = requested_series(
        &scanned,
        &selected(&["Renamed realm"]),
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    assert_eq!(
        series[0].runs[0].values,
        [
            Some(1),
            Some(2),
            Some(9),
            Some(2),
            Some(1),
            Some(3),
            Some(4),
            Some(3)
        ]
    );
}

#[test]
fn shared_indexed_lineage_matches_the_country_lookup_for_opaque_and_unresolved_tags() {
    let countries = [
        CountryRow {
            id: 8,
            canonical_slot: 2,
            tag: b"opaque tag".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 9,
            canonical_slot: 2,
            tag: b"opaque tag".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 10,
            canonical_slot: 4,
            tag: b"unresolved".to_vec(),
            ..Default::default()
        },
    ];
    let links = [
        CountryLinkRow {
            id: 8,
            canonical_slot: 1,
        },
        CountryLinkRow {
            id: 9,
            canonical_slot: 2,
        },
    ];
    let slots = links
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let indexed = crate::metadata::tagged_countries_indexed(&countries, &slots);
    for tag in ["opaque tag", "unresolved"] {
        assert_eq!(
            indexed[tag.as_bytes()],
            crate::metadata::tagged_country(&countries, &links, tag)
        );
    }
}

#[test]
fn metadata_only_and_concurrent_selections_share_a_cached_branch_without_game_content() {
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "territory-fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay(vec![frame(100, 3, 8)])),
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LoadContext { content: None };
    let info = Territory::load(&source, &selected(&[]), &context, &AtomicBool::new(false)).unwrap();
    assert!(info.area_reason.unwrap().contains("game and user folders"));
    assert!(info.warnings.is_empty());
    assert_eq!(info.world_totals[0].values[0], Some(3));
    assert_eq!(source.state::<Cache>().lock().unwrap().scans.len(), 1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = ["realm 00000", "realm 00001", "realm 00002"]
            .into_iter()
            .map(|tag| {
                let source = &source;
                let context = &context;
                scope.spawn(move || {
                    Territory::load(source, &selected(&[tag]), context, &AtomicBool::new(false))
                        .unwrap()
                })
            })
            .collect();
        for handle in handles {
            let reply = handle.join().unwrap();
            assert_eq!(reply.series.len(), 1);
            assert!(reply.area_reason.is_some());
            assert!(reply.warnings.is_empty());
        }
    });
    assert_eq!(source.state::<Cache>().lock().unwrap().scans.len(), 1);
}

#[test]
fn a_geographic_walk_builds_the_cached_totals_in_the_same_pass() {
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "territory-fixture".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay(vec![frame(100, 3, 8), frame(101, 3, 8)])),
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LoadContext { content: None };
    let mut hours = Vec::new();
    for _ in 0..2 {
        walk_branch(
            &source,
            1,
            None,
            &context,
            &AtomicBool::new(false),
            |frame| {
                hours.push(frame.hour);
                Ok(())
            },
        )
        .unwrap();
    }
    assert_eq!(hours, [100, 101, 100, 101]);
    let cache = source.state::<Cache>();
    let scanned = cached_scan(&cache, 1, None, || panic!("cached branch rescanned")).unwrap();
    assert_eq!(scanned.world_totals[0].last_hour, 101);
}
fn selected(tags: &[&str]) -> TerritoryParams {
    TerritoryParams {
        branch_id: "1".into(),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        top: None,
    }
}

#[test]
fn independent_owned_and_controlled_counts_are_exact_and_mod_sized() {
    let replay = replay(vec![frame(100, 3, 8)]);
    let scanned = scan(&replay, 1, Some(&areas(8)), &AtomicBool::new(false)).unwrap();
    let series = requested_series(
        &scanned,
        &selected(&["realm 00000"]),
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    // Controller ids 3,6 versus owner ids 1,4,7. No fixed state/province census.
    assert_eq!(
        series[0].runs[0].values,
        [
            Some(1),
            Some(2),
            Some(9),
            Some(90),
            Some(1),
            Some(3),
            Some(4),
            Some(120)
        ]
    );
}

#[test]
fn partial_and_unavailable_tables_leave_gaps_while_complete_empty_tables_prove_zero() {
    let mut partial = frame(101, 2, 4);
    *partial.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::Provinces(vec![]),
        missing: 4,
    };
    let mut unavailable = frame(102, 2, 4);
    *unavailable.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "province read failed".into(),
    };
    let mut empty = frame(103, 2, 0);
    *empty.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![]));
    let scanned = scan(
        &replay(vec![frame(100, 2, 4), partial, unavailable, empty]),
        1,
        Some(&areas(4)),
        &AtomicBool::new(false),
    )
    .unwrap();
    let runs = &scanned.series["realm 00000"].runs;
    assert_eq!(runs[1].first_hour, 101);
    assert_eq!(runs[1].last_hour, 102);
    assert_eq!(runs[1].values[0], Some(1));
    assert_eq!(runs[1].values[1], None);
    assert_eq!(runs[2].values, [Some(0); 8]);
    let no_map = scan(
        &replay(vec![frame(100, 2, 4)]),
        1,
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(no_map.series["realm 00000"].runs[0].values[3], None);
}

#[test]
fn hourly_slot_reads_are_fresh_without_table_timing_and_preserve_the_read_hour() {
    let mut first = frame(100, 2, 4);
    first.metrics = Metrics::default();
    let mut next = first.clone();
    next.hour = 101;
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *next.tables[TableId::Provinces.index()]
    {
        rows[0].victory_points = -8;
    }
    let scanned = scan(
        &replay(vec![first, next]),
        1,
        Some(&areas(4)),
        &AtomicBool::new(false),
    )
    .unwrap();
    let runs = &scanned.series["realm 00000"].runs;
    assert_eq!(runs[0].first_hour, 100);
    assert_eq!(runs[1].first_hour, 101);
    assert_eq!(runs[0].values[6], Some(-4));
    assert_eq!(runs[1].values[6], Some(-5));
    assert_eq!(runs[1].values[0], Some(1));
}

#[test]
fn null_and_unresolved_links_are_distinct_and_unknown_references_never_understate_totals() {
    let mut decoded = frame(100, 2, 4);
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *decoded.tables[TableId::Provinces.index()]
    {
        rows[0].controller = 0;
        rows[1].owner = 999_999;
    }
    let slots = decoded
        .complete::<tables::CountryLinks>()
        .unwrap()
        .iter()
        .map(|link| (link.id, link.canonical_slot as usize))
        .collect();
    let mut totals = Vec::new();
    super::totals(
        replay::format::FormatVersion::CURRENT,
        &decoded,
        Some(&areas(4)),
        &slots,
        2,
        &mut totals,
    );
    assert_eq!(totals[0][1], Some(2));
    assert_eq!(totals[1][1], Some(1));
    assert_eq!(totals[0][5], None);
    assert_eq!(totals[1][5], None);
    assert_eq!(totals[0][4], Some(1));
}

#[test]
fn a_renamed_country_and_reused_tag_break_identity_and_never_bridge_missing_hours() {
    let mut renamed = frame(101, 2, 4);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"Renamed realm".to_vec();
    }
    let mut replaced = renamed.clone();
    replaced.hour = 103;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *replaced.tables[TableId::Countries.index()]
    {
        rows[0].id = 200_000;
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *replaced.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 200_000,
            canonical_slot: 0,
        });
    }
    let mut next = replaced.clone();
    next.hour = 104;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *next.tables[TableId::Countries.index()]
    {
        rows.iter_mut()
            .find(|row| row.tag == b"Renamed realm")
            .unwrap()
            .id = 300_000;
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *next.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 300_000,
            canonical_slot: 0,
        });
    }
    let scanned = scan(
        &replay(vec![frame(100, 2, 4), renamed, replaced, next]),
        1,
        Some(&areas(4)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.series["realm 00000"].runs.len(), 1);
    let runs = &scanned.series["Renamed realm"].runs;
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
            .collect::<Vec<_>>(),
        [(101, 101, true), (103, 103, true), (104, 104, true)]
    );
}

#[test]
fn fixed_territory_choices_project_cached_native_aliases_and_keep_absence_apart() {
    let countries = |hour, holders: &[(u32, u32, &[u8])]| {
        let mut decoded = frame(hour, 2, 4);
        *decoded.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            holders
                .iter()
                .map(|&(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
        *decoded.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                [(100_000, 0), (100_001, 1), (200_000, 2), (300_000, 3)]
                    .into_iter()
                    .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .collect(),
            ));
        if !holders.iter().any(|holder| holder.0 == 100_000) {
            if let TableView::Complete(TableRows::States(rows)) =
                &mut *decoded.tables[TableId::States.index()]
            {
                for row in rows {
                    row.owner = 100_001;
                    row.controller = 100_001;
                }
            }
            if let TableView::Complete(TableRows::Provinces(rows)) =
                &mut *decoded.tables[TableId::Provinces.index()]
            {
                for row in rows {
                    row.owner = 100_001;
                    row.controller = 100_001;
                }
            }
        }
        decoded
    };
    let replay = replay(vec![
        countries(100, &[(100_000, 0, b"picked"), (100_001, 1, b"other")]),
        countries(101, &[(100_000, 0, b"alias"), (100_001, 1, b"other")]),
        countries(
            103,
            &[
                (100_000, 0, b"alias"),
                (100_001, 1, b"other"),
                (200_000, 2, b"picked"),
            ],
        ),
        countries(
            104,
            &[
                (100_001, 1, b"other"),
                (200_000, 2, b"picked"),
                (300_000, 3, b"alias"),
            ],
        ),
    ]);
    let cancelled = AtomicBool::new(false);
    let scanned = scan(&replay, 1, Some(&areas(4)), &cancelled).unwrap();
    let histories =
        super::super::country_history::scan_histories(&replay, 1, false, &[], &cancelled).unwrap();
    let series = requested_series(
        &scanned,
        &selected(&["picked", "not released"]),
        &cancelled,
        Some(&histories),
    )
    .unwrap();
    assert_eq!(
        series
            .iter()
            .map(|series| series.tag.as_str())
            .collect::<Vec<_>>(),
        ["alias", "picked", "not released"]
    );
    let alias = series.iter().find(|series| series.tag == "alias").unwrap();
    assert_eq!(
        alias
            .identities
            .as_ref()
            .unwrap()
            .iter()
            .map(|identity| (identity.first_hour, identity.last_hour, identity.country_id))
            .collect::<Vec<_>>(),
        [
            (101, 101, 100_000),
            (103, 103, 100_000),
            (104, 104, 300_000)
        ]
    );
    assert_eq!(
        alias
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (101, 101), (103, 103), (104, 104)]
    );
    let picked = series.iter().find(|series| series.tag == "picked").unwrap();
    assert_eq!(
        picked
            .identities
            .as_ref()
            .unwrap()
            .iter()
            .map(|identity| (identity.first_hour, identity.last_hour, identity.country_id))
            .collect::<Vec<_>>(),
        [(100, 100, 100_000), (103, 104, 200_000)]
    );
    assert_eq!(
        picked
            .runs
            .iter()
            .find(|run| run.first_hour == 101)
            .unwrap()
            .values,
        [Some(0); 8]
    );
    let absent = series
        .iter()
        .find(|series| series.tag == "not released")
        .unwrap();
    assert_eq!(absent.identities, Some(Vec::new()));
    assert!(absent.segments.is_empty());
    assert!(absent.runs.iter().all(|run| run.values == [Some(0); 8]));
    assert!(
        requested_series(
            &scanned,
            &selected(&["picked"]),
            &AtomicBool::new(true),
            Some(&histories)
        )
        .is_err()
    );
}

#[test]
fn hourly_top_membership_is_cut_to_changes_with_byte_order_ties_and_cancellation() {
    let mut scanned = Scan::default();
    for (tag, first, last, value) in [
        ("a realm", 100, 104, 5),
        ("Z realm", 100, 101, 5),
        ("Z realm", 103, 104, 6),
    ] {
        scanned
            .series
            .entry(tag.into())
            .or_default()
            .runs
            .push(TerritoryRun {
                first_hour: first,
                last_hour: last,
                starts_segment: true,
                values: [Some(value); 8],
            });
    }
    let params = TerritoryParams {
        branch_id: "1".into(),
        tags: vec![],
        top: Some(TerritoryTop {
            count: 1,
            measure: TerritoryMeasure::States,
            rule: TerritoryRule::Controlled,
        }),
    };
    let series = requested_series(&scanned, &params, &AtomicBool::new(false), None).unwrap();
    assert_eq!(series[0].tag, "Z realm");
    assert_eq!(series[0].runs.len(), 2);
    assert_eq!(series[1].runs[0].first_hour, 102);
    assert_eq!(series[1].runs[0].last_hour, 102);
    assert_eq!(
        requested_series(&scanned, &params, &AtomicBool::new(true), None)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert!(
        Territory::validate(&TerritoryParams {
            tags: vec!["a realm".into()],
            ..params
        })
        .is_err()
    );
}

#[test]
#[ignore = "large synthetic recording"]
fn scale_replay_5000_countries_13000_provinces_preserves_every_country() {
    const HOURS: i32 = 24;
    for (countries, changing) in [1_500, 5_000]
        .into_iter()
        .flat_map(|count| [(count, false), (count, true)])
    {
        let replay = replay(
            (100..100 + HOURS)
                .map(|hour| {
                    let mut decoded = frame(hour, countries, 13_000);
                    if changing
                        && let TableView::Complete(TableRows::Provinces(rows)) =
                            &mut *decoded.tables[TableId::Provinces.index()]
                    {
                        rows[0].controller = 100_000 + ((hour - 100) as u32 % 2);
                    }
                    decoded
                })
                .collect(),
        );
        let areas = raster_areas(13_000);
        let scanned = scan(&replay, 1, Some(&areas), &AtomicBool::new(false)).unwrap();
        let params = TerritoryParams {
            branch_id: "1".into(),
            tags: vec![],
            top: Some(TerritoryTop {
                count: 10,
                measure: TerritoryMeasure::Provinces,
                rule: TerritoryRule::Controlled,
            }),
        };
        let series = requested_series(&scanned, &params, &AtomicBool::new(false), None).unwrap();
        let reply = TerritoryReply {
            world_totals: scanned.world_totals.clone(),
            branch_id: "1".into(),
            series,
            area_unit: Some(ProvinceAreas::UNIT.into()),
            area_reason: None,
            warnings: vec![],
        };
        let json = serde_json::to_vec(&reply).unwrap();
        assert_eq!(scanned.series.len(), countries as usize);
        for hour in 100..100 + HOURS {
            let recorded = reply
                .series
                .iter()
                .filter(|series| {
                    series
                        .runs
                        .iter()
                        .any(|run| run.first_hour <= hour && hour <= run.last_hour)
                })
                .count();
            assert_eq!(recorded, 10, "top membership at {hour}");
        }
        assert!(json.len() < MAX_PRODUCT_REPLY_BYTES);
        let run_count = reply
            .series
            .iter()
            .map(|series| series.runs.len())
            .sum::<usize>();
        if changing {
            assert!(run_count >= HOURS as usize);
        } else {
            assert_eq!(run_count, 10);
        }
        let all = requested_series(
            &scanned,
            &TerritoryParams {
                tags: scanned.series.keys().map(|tag| tag.to_string()).collect(),
                top: None,
                ..params
            },
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        assert_eq!(all.len(), countries as usize);
        for hour in 100..100 + HOURS {
            let values = all
                .iter()
                .map(|series| {
                    series
                        .runs
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                        .and_then(|run| run.values[1])
                })
                .collect::<Option<Vec<_>>>()
                .expect("every country has a recorded province count");
            assert_eq!(values.into_iter().sum::<i64>(), 13_000);
        }
    }
}

#[test]
fn reply_budget_counts_actual_json_without_a_private_conservative_limit() {
    let mut scanned = Scan::default();
    scanned.series.insert(
        "history".into(),
        Track {
            runs: (0..82_000)
                .map(|hour| TerritoryRun {
                    first_hour: hour,
                    last_hour: hour,
                    starts_segment: false,
                    values: [Some(i64::from(hour)); 8],
                })
                .collect(),
            ..Default::default()
        },
    );
    let mut tags: Vec<_> = (0..3_000)
        .map(|index| format!("{index:04}{}", "\u{1}".repeat(124)))
        .collect();
    tags.push("history".into());
    let params = TerritoryParams {
        branch_id: "1".into(),
        tags,
        top: None,
    };
    assert!(Territory::validate(&params).is_ok());
    // The old per-run estimate refused this reply although its JSON fits.
    let series = requested_series(&scanned, &params, &AtomicBool::new(false), None).unwrap();
    assert_eq!(series.last().unwrap().runs.len(), 82_000);
    assert!(serde_json::to_vec(&series).unwrap().len() < MAX_PRODUCT_REPLY_BYTES);
    let mut params = params;
    params
        .tags
        .extend((3_000..30_000).map(|index| format!("{index:05}{}", "\u{1}".repeat(123))));
    assert!(Territory::validate(&params).is_ok());
    assert!(
        requested_series(&scanned, &params, &AtomicBool::new(false), None)
            .unwrap_err()
            .to_string()
            .contains("transport size budget")
    );
}

#[test]
fn shared_lineage_interns_tags_once_and_equal_totals_extend_tracks_without_new_runs() {
    let mut decoded = frame(100, 3, 8);
    let mut scanned = Scan::default();
    let mut builder = Builder::default();
    observe(&mut scanned, &decoded, Some(&areas(8)), &mut builder).unwrap();
    let entries = builder.lineage.as_ref().unwrap().entries.as_ptr();
    let tag = Arc::clone(scanned.series.get_key_value("realm 00000").unwrap().0);
    decoded.hour = 101;
    observe(&mut scanned, &decoded, Some(&areas(8)), &mut builder).unwrap();
    assert_eq!(entries, builder.lineage.as_ref().unwrap().entries.as_ptr());
    assert_eq!(scanned.world_totals.len(), 1);
    assert!(
        scanned
            .series
            .values()
            .all(|track| track.runs.len() == 1 && track.runs[0].last_hour == 101)
    );
    // A gap still breaks equal values, even with the same allocations.
    decoded.hour = 103;
    observe(&mut scanned, &decoded, Some(&areas(8)), &mut builder).unwrap();
    assert!(
        scanned
            .series
            .values()
            .all(|track| track.runs.len() == 2 && track.runs[1].starts_segment)
    );
    // Replacing only the links rebuilds the index but keeps the interned tag.
    let links = decoded.complete::<tables::CountryLinks>().unwrap().to_vec();
    decoded.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links)).into();
    decoded.hour = 104;
    observe(&mut scanned, &decoded, Some(&areas(8)), &mut builder).unwrap();
    assert_ne!(entries, builder.lineage.as_ref().unwrap().entries.as_ptr());
    assert!(Arc::ptr_eq(
        &tag,
        scanned.series.get_key_value("realm 00000").unwrap().0
    ));
}

#[test]
fn dense_totals_count_aliases_once_and_keep_unresolved_slot_identities_unknown() {
    let decoded = frame(100, 2, 4);
    let slots = HashMap::from([(100_000, 3), (100_001, 3), (999_999, 3)]);
    let mut sums = Vec::new();
    totals(
        replay::format::FormatVersion::CURRENT,
        &decoded,
        Some(&areas(4)),
        &slots,
        4,
        &mut sums,
    );
    assert_eq!(
        sums[3],
        [
            Some(2),
            Some(4),
            Some(2),
            Some(100),
            Some(2),
            Some(4),
            Some(2),
            Some(100)
        ]
    );
    let mut decoded = decoded;
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *decoded.tables[TableId::CountryLinks.index()]
    {
        links[0].canonical_slot = u32::MAX;
    }
    let mut scan = Scan::default();
    let mut builder = Builder::default();
    observe(&mut scan, &decoded, None, &mut builder).unwrap();
    assert_eq!(scan.incomplete, 1);
    assert!(builder.current.is_empty());
    assert_eq!(builder.previous.len(), 1);
    assert_eq!(scan.rosters.len(), 1);
    assert_eq!(scan.series.len(), 2);
    assert!(scan.series["realm 00000"].runs.is_empty());
}

#[test]
fn settings_and_widgets_build_the_area_raster_once_per_content_source_even_when_refused() {
    let content = || {
        Arc::new(installed_content::ResolvedContent {
            layers: installed_content::ResolvedLayers::from_folders(&[]).unwrap(),
            stats: Default::default(),
        })
    };
    let resolved = content();
    let cache = Mutex::new(Cache::default());
    let cancelled = AtomicBool::new(false);
    let first = cached_areas(&cache, Arc::clone(&resolved), &cancelled, |_| Ok(areas(4))).unwrap();
    let second = cached_areas(&cache, Arc::clone(&resolved), &cancelled, |_| {
        panic!("raster already built")
    })
    .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let resolved = content();
    let refusal = cached_areas(&cache, Arc::clone(&resolved), &cancelled, |_| {
        Err(io::Error::other("unsupported map"))
    })
    .unwrap_err();
    let repeated = cached_areas(&cache, resolved, &cancelled, |_| {
        panic!("refused raster already tried")
    })
    .unwrap_err();
    assert_eq!(refusal.to_string(), repeated.to_string());
}

#[test]
fn concurrent_scans_release_the_cache_lock_and_keep_the_first_insert() {
    let cache = Mutex::new(Cache::default());
    cached_scan(&cache, 9, None, || {
        assert!(cache.try_lock().is_ok(), "scan holds the cache lock");
        Ok(Scan::default())
    })
    .unwrap();
    cache.lock().unwrap().scans.clear();
    let ready = std::sync::Barrier::new(3);
    let scans = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..3)
            .map(|incomplete| {
                let (cache, ready) = (&cache, &ready);
                scope.spawn(move || {
                    cached_scan(cache, 1, None, || {
                        ready.wait();
                        Ok(Scan {
                            incomplete,
                            ..Default::default()
                        })
                    })
                    .unwrap()
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(cache.lock().unwrap().scans.len(), 1);
    assert!(scans.iter().all(|scan| Arc::ptr_eq(scan, &scans[0])));
    assert!(Arc::ptr_eq(
        &cached_scan(&cache, 1, None, || panic!("cached branch rescanned")).unwrap(),
        &scans[0]
    ));
    assert!(
        cached_scan(&cache, 2, None, || Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "cancelled"
        )))
        .is_err()
    );
    assert_eq!(cache.lock().unwrap().scans.len(), 1);
    cached_scan(&cache, 2, None, || Ok(Scan::default())).unwrap();
    cached_scan(&cache, 3, None, || Ok(Scan::default())).unwrap();
    assert_eq!(
        cache
            .lock()
            .unwrap()
            .scans
            .iter()
            .map(|scan| scan.0)
            .collect::<Vec<_>>(),
        [2, 3]
    );
}

#[test]
fn annexation_and_release_emit_dated_zero_only_with_complete_rosters() {
    let mut absent = frame(101, 2, 4);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *absent.tables[TableId::Countries.index()]
    {
        rows.retain(|row| row.canonical_slot != 1);
    }
    let mut missing = absent.clone();
    missing.hour = 102;
    *missing.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "links failed".into(),
    };
    let mut absent_again = absent.clone();
    absent_again.hour = 103;
    let scanned = scan(
        &replay(vec![
            frame(100, 2, 4),
            absent,
            missing,
            absent_again,
            frame(104, 2, 4),
        ]),
        1,
        Some(&areas(4)),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = requested_series(
        &scanned,
        &selected(&["realm 00001", "not released"]),
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    assert_eq!(
        reply[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.values[0]))
            .collect::<Vec<_>>(),
        [
            (100, 100, Some(1)),
            (101, 101, Some(0)),
            (103, 103, Some(0)),
            (104, 104, Some(1))
        ]
    );
    assert_eq!(
        reply[1]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101), (103, 104)]
    );
    assert!(reply[1].runs.iter().all(|run| run.values == [Some(0); 8]));
    assert!(
        reply[1].segments.is_empty(),
        "zero is not a country identity"
    );
    assert_eq!(
        scanned.incomplete, 3,
        "two unnamed-holder hours and one unread-links hour are incomplete"
    );
    // A row exists but its identity cannot be resolved: it cannot prove zero.
    let mut unresolved = frame(105, 2, 4);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *unresolved.tables[TableId::CountryLinks.index()]
    {
        rows.pop();
    }
    let scanned = scan(&replay(vec![unresolved]), 1, None, &AtomicBool::new(false)).unwrap();
    let reply = requested_series(
        &scanned,
        &selected(&["realm 00001"]),
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    assert!(reply[0].runs.is_empty());
}

#[test]
fn a_disagreeing_slot_identity_makes_one_hour_incomplete_and_breaks_continuity() {
    let mut invalid = frame(101, 3, 6);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *invalid.tables[TableId::CountryLinks.index()]
    {
        rows[0].canonical_slot = 100;
    }
    let scanned = scan(
        &replay(vec![frame(100, 3, 6), invalid, frame(102, 3, 6)]),
        1,
        Some(&areas(6)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.incomplete, 1);
    let runs = &scanned.series["realm 00000"].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1].first_hour, 102);
    assert!(runs[1].starts_segment);
    assert_eq!(scanned.rosters.len(), 1);
}

#[test]
fn a_country_added_at_a_higher_slot_grows_totals_and_keeps_existing_tracks() {
    let mut added = frame(101, 4, 4);
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *added.tables[TableId::Provinces.index()]
    {
        rows.push(ProvinceRow {
            id: 5,
            controller: 100_003,
            owner: 100_003,
            victory_points: 10,
            ..Default::default()
        });
    }
    let scanned = scan(
        &replay(vec![frame(100, 3, 4), added]),
        1,
        Some(&areas(5)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.incomplete, 0);
    assert_eq!(scanned.series.len(), 4);
    for tag in ["realm 00000", "realm 00001", "realm 00002"] {
        assert_eq!(
            scanned.series[tag].segments,
            [HourSpan {
                first_hour: 100,
                last_hour: 101
            }]
        );
        assert!(
            !scanned.series[tag].runs.last().unwrap().starts_segment
                || scanned.series[tag].runs.len() == 1
        );
    }
    let totals_at = |hour| {
        scanned
            .series
            .values()
            .map(|track| {
                track
                    .runs
                    .iter()
                    .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                    .map_or(0, |run| run.values[1].unwrap())
            })
            .sum::<i64>()
    };
    assert_eq!((totals_at(100), totals_at(101)), (4, 5));
    assert_eq!(scanned.series["realm 00003"].runs[0].values[1], Some(2));
}

#[test]
fn a_province_outside_the_area_vector_counts_as_an_incomplete_read() {
    let scanned = scan(
        &replay(vec![frame(100, 2, 4), frame(101, 2, 4)]),
        1,
        Some(&areas(3)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.incomplete, 2);
    let runs = &scanned.series["realm 00000"].runs;
    assert_eq!(runs[0].values[1], Some(2));
    assert_eq!(runs[0].values[3], None);
    let no_map = scan(
        &replay(vec![frame(100, 2, 4)]),
        1,
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(no_map.incomplete, 0, "a refused map has its own reason");
}

#[test]
fn an_unowned_out_of_range_province_warns_only_for_affected_hours() {
    let mut decoded = frame(100, 2, 4);
    if let TableView::Complete(TableRows::Provinces(provinces)) =
        &mut *decoded.tables[TableId::Provinces.index()]
    {
        let province = provinces.last_mut().unwrap();
        province.id = 5;
        province.owner = 0;
        province.controller = 0;
    }
    let map = areas(4);
    let mut scanned = Scan::default();
    let mut builder = Builder::default();
    for hour in 100..=101 {
        decoded.hour = hour;
        observe(&mut scanned, &decoded, Some(&map), &mut builder).unwrap();
    }
    assert_eq!(scanned.incomplete, 2);
    if let TableView::Complete(TableRows::Provinces(provinces)) =
        &mut *decoded.tables[TableId::Provinces.index()]
    {
        provinces.last_mut().unwrap().id = 4;
    }
    decoded.hour = 102;
    observe(&mut scanned, &decoded, Some(&map), &mut builder).unwrap();
    assert_eq!(
        scanned.incomplete, 2,
        "the repaired read keeps equal totals"
    );
    assert!(scanned.series.values().all(|track| {
        track.runs.len() == 1
            && track.runs[0].last_hour == 102
            && track.runs[0].values.iter().all(Option::is_some)
    }));
}

#[test]
fn raster_builds_release_the_cache_lock_keep_first_insert_and_never_cache_cancellation() {
    let cache = Mutex::new(Cache::default());
    let resolved = Arc::new(installed_content::ResolvedContent {
        layers: installed_content::ResolvedLayers::from_folders(&[]).unwrap(),
        stats: Default::default(),
    });
    let cancelled = AtomicBool::new(false);
    cached_areas(&cache, Arc::clone(&resolved), &cancelled, |_| {
        assert!(cache.try_lock().is_ok(), "raster holds cache lock");
        Ok(areas(4))
    })
    .unwrap();
    cache.lock().unwrap().areas = None;
    let ready = std::sync::Barrier::new(3);
    let results = std::thread::scope(|scope| {
        (0..3)
            .map(|count| {
                let (cache, resolved, cancelled, ready) =
                    (&cache, Arc::clone(&resolved), &cancelled, &ready);
                scope.spawn(move || {
                    cached_areas(cache, resolved, cancelled, |_| {
                        ready.wait();
                        Ok(areas(count))
                    })
                    .unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        results
            .iter()
            .all(|result| Arc::ptr_eq(result, &results[0]))
    );
    cache.lock().unwrap().areas = None;
    assert_eq!(
        cached_areas(&cache, Arc::clone(&resolved), &cancelled, |_| {
            cancelled.store(true, Ordering::Release);
            Ok(areas(4))
        })
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
    assert!(cache.lock().unwrap().areas.is_none());
    cancelled.store(false, Ordering::Release);
    assert_eq!(
        cached_areas(&cache, Arc::clone(&resolved), &cancelled, |_| Err(
            io::Error::new(io::ErrorKind::Interrupted, "cancelled")
        ))
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
    assert!(cache.lock().unwrap().areas.is_none());
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        cached_areas(&cache, resolved, &cancelled, |_| panic!(
            "cancelled before raster"
        ))
        .unwrap_err()
        .kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn the_world_total_includes_unheld_land_and_never_uses_only_selected_countries() {
    let mut decoded = frame(100, 2, 4);
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *decoded.tables[TableId::Provinces.index()]
    {
        rows[0].controller = 0;
        rows[0].owner = 0;
    }
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *decoded.tables[TableId::States.index()]
    {
        rows[0].controller = 0;
        rows[0].owner = 0;
    }
    let scanned = scan(
        &replay(vec![decoded]),
        1,
        Some(&areas(4)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        scanned.world_totals[0].values,
        [
            Some(2),
            Some(4),
            Some(2),
            Some(100),
            Some(2),
            Some(4),
            Some(2),
            Some(100)
        ]
    );
    let countries = requested_series(
        &scanned,
        &selected(&["realm 00000"]),
        &AtomicBool::new(false),
        None,
    )
    .unwrap();
    assert!(countries[0].runs[0].values[1].unwrap() < scanned.world_totals[0].values[1].unwrap());
}

#[test]
fn a_world_share_has_no_partial_rule_when_a_holder_or_a_read_is_unknown() {
    let mut unresolved = frame(100, 2, 4);
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *unresolved.tables[TableId::Provinces.index()]
    {
        rows[0].controller = 999999;
    }
    let mut unread = frame(101, 2, 4);
    *unread.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unread provinces".into(),
    };
    let scanned = scan(
        &replay(vec![unresolved, unread]),
        1,
        Some(&areas(4)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.world_totals[0].values[1..4], [None; 3]);
    assert_eq!(scanned.world_totals[0].values[5], Some(4));
    assert_eq!(scanned.world_totals[1].values[1..4], [None; 3]);
    assert_eq!(scanned.world_totals[1].values[5..8], [None; 3]);
}

#[test]
fn overflowing_territory_and_world_totals_have_counted_warnings() {
    let scanned = scan(
        &replay(vec![frame(100, 2, 4)]),
        1,
        Some(&ProvinceAreas {
            land: vec![0, i64::MAX, i64::MAX, i64::MAX, i64::MAX],
        }),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(scanned.overflowed > 0);
    assert!(scanned.world_totals[0].values[3].is_none());
    let mut warnings = Vec::new();
    scan_warning(&scanned, &mut warnings);
    assert!(warnings.iter().any(|warning| warning.starts_with(&format!("{} territory totals", scanned.overflowed))));
}

#[test]
fn a_safe_signed_world_total_does_not_hide_unsafe_country_candidates() {
    let mut countries = [[Some(0); 8]; 2];
    countries[0][2] = Some(9_007_199_254_740_992);
    countries[1][2] = Some(-9_007_199_254_740_992);
    let mut world = [Some(0); 8];
    assert_eq!(
        sanitise_totals([true; 8], &mut countries, &mut world, false),
        2
    );
    assert_eq!(
        world[2], None,
        "a share cannot rank an unknown country into Rest"
    );
    assert_eq!(world[0], Some(0));
    assert_eq!(countries[0][2], None);
    assert_eq!(countries[1][2], None);
}

#[test]
fn a_conflicting_holder_alias_cannot_supply_a_partial_world_ring() {
    let mut decoded = frame(100, 2, 4);
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *decoded.tables[TableId::CountryLinks.index()]
    {
        links.push(CountryLinkRow {
            id: 999999,
            canonical_slot: 0,
        });
        links.push(CountryLinkRow {
            id: 999999,
            canonical_slot: 1,
        });
    }
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *decoded.tables[TableId::Provinces.index()]
    {
        rows[0].controller = 999999;
    }
    let mut scanned = Scan::default();
    observe(
        &mut scanned,
        &decoded,
        Some(&areas(4)),
        &mut Builder::default(),
    )
    .unwrap();
    assert_eq!(scanned.world_totals[0].values[1..4], [None; 3]);
    assert_eq!(scanned.world_totals[0].values[5], Some(4));
}

#[test]
fn an_orphan_canonical_holder_without_a_named_country_cannot_supply_a_world_share() {
    let mut decoded = frame(100, 2, 4);
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *decoded.tables[TableId::CountryLinks.index()]
    {
        links.push(CountryLinkRow {
            id: 999999,
            canonical_slot: 2,
        });
    }
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *decoded.tables[TableId::Provinces.index()]
    {
        rows[0].controller = 999999;
    }
    let scanned = scan(
        &replay(vec![decoded]),
        1,
        Some(&areas(4)),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.world_totals[0].values[1..4], [None; 3]);
    assert_eq!(scanned.world_totals[0].values[0], Some(2));
    assert_eq!(scanned.world_totals[0].values[5], Some(4));
    assert!(
        scanned
            .series
            .values()
            .all(|track| track.runs[0].values[1].is_none())
    );
}

#[test]
fn recorded_country_slots_and_histories_grow_past_the_former_limits() {
    let mut decoded = frame(100, 65_537, 0);
    let states = decoded.tables[TableId::States.index()].clone();
    let empty: SharedView = TableView::Complete(TableRows::States(vec![])).into();
    let mut scanned = Scan::default();
    let mut builder = Builder::default();
    for hour in 100..116 {
        decoded.hour = hour;
        decoded.tables[TableId::States.index()] = if hour % 2 == 0 {
            states.clone()
        } else {
            empty.clone()
        };
        observe(&mut scanned, &decoded, None, &mut builder).unwrap();
    }
    assert_eq!(scanned.incomplete, 0);
    assert_eq!(scanned.series.len(), 65_537);
    assert_eq!(builder.previous.len(), 65_537);
    assert!(
        scanned
            .series
            .values()
            .map(|track| track.runs.len())
            .sum::<usize>()
            > 1_000_000
    );
    let params = TerritoryParams {
        branch_id: "1".into(),
        tags: (0..10_001).map(|slot| format!("realm {slot:05}")).collect(),
        top: None,
    };
    assert!(Territory::validate(&params).is_ok());
    let reply = requested_series(&scanned, &params, &AtomicBool::new(false), None).unwrap();
    assert_eq!(reply.len(), 10_001);
    assert!(reply.iter().all(|series| series.runs.len() == 16));
    let top = TerritoryParams {
        branch_id: "1".into(),
        tags: vec![],
        top: Some(TerritoryTop {
            count: 31,
            measure: TerritoryMeasure::States,
            rule: TerritoryRule::Controlled,
        }),
    };
    assert!(Territory::validate(&top).is_ok());
    let top_scan = Scan {
        series: scanned
            .series
            .iter()
            .take(31)
            .map(|(tag, track)| {
                (
                    Arc::clone(tag),
                    Track {
                        runs: vec![track.runs[0].clone()],
                        ..Default::default()
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    assert_eq!(
        requested_series(&top_scan, &top, &AtomicBool::new(false), None)
            .unwrap()
            .len(),
        31
    );
}

#[test]
fn sparse_recorded_slots_use_dense_totals_without_changing_identity() {
    let mut decoded = frame(100, 2, 4);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *decoded.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = u32::MAX;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *decoded.tables[TableId::CountryLinks.index()]
    {
        rows[0].canonical_slot = u32::MAX;
    }
    let mut scanned = Scan::default();
    let mut builder = Builder::default();
    observe(&mut scanned, &decoded, Some(&areas(4)), &mut builder).unwrap();
    assert_eq!(scanned.incomplete, 0);
    assert_eq!(builder.previous.len(), 2);
    let (identity, _) = scanned.series["realm 00000"].last.unwrap();
    assert_eq!(identity.0, u32::MAX);
    assert_eq!(scanned.series["realm 00000"].runs[0].values[1], Some(2));
    assert_eq!(scanned.world_totals[0].values[1], Some(4));
}

#[test]
fn territory_refuses_two_tags_for_one_canonical_slot() {
    let mut decoded = frame(100, 2, 4);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *decoded.tables[TableId::Countries.index()]
    {
        rows[1].canonical_slot = 0;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *decoded.tables[TableId::CountryLinks.index()]
    {
        rows[1].canonical_slot = 0;
    }
    // Recorded frames refuse duplicate canonical slots (replay frame validation), so
    // this defensive path is reached only by observing an unvalidated frame.
    let mut scanned = Scan::default();
    observe(&mut scanned, &decoded, None, &mut Builder::default()).unwrap();
    assert!(scanned.series.values().all(|track| track.runs.is_empty()));
    assert_eq!(scanned.world_totals[0].values, [None; 8]);
    let mut warnings = Vec::new();
    scan_warning(&scanned, &mut warnings);
    assert!(
        warnings
            .iter()
            .any(|reason| reason.contains("more than one tag"))
    );
}

#[test]
fn territory_totals_cache_tracks_measured_fields_lineage_and_unavailable_reads() {
    let mut decoded = frame(100, 2, 4);
    let mut scanned = Scan::default();
    let mut builder = Builder::default();
    observe(&mut scanned, &decoded, None, &mut builder).unwrap();
    decoded.hour += 1;
    observe(&mut scanned, &decoded, None, &mut builder).unwrap();
    assert_eq!(
        builder.aggregations, 1,
        "shared unchanged views reuse the aggregate"
    );
    let mut countries = decoded.complete::<tables::Countries>().unwrap().to_vec();
    countries[0].stability_raw += 1;
    let mut provinces = decoded.complete::<tables::Provinces>().unwrap().to_vec();
    provinces[0].move_gate ^= 1;
    decoded.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries)).into();
    decoded.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(provinces)).into();
    decoded.hour += 1;
    observe(&mut scanned, &decoded, None, &mut builder).unwrap();
    assert_eq!(
        builder.aggregations, 1,
        "hourly balances and gates in new allocations reuse the aggregate"
    );
    decoded.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![])).into();
    decoded.hour += 1;
    observe(&mut scanned, &decoded, None, &mut builder).unwrap();
    assert_eq!(builder.aggregations, 2);
    assert_eq!(builder.previous[0][1], Some(0));
    decoded.tables[TableId::CountryLinks.index()] =
        frame(100, 2, 4).tables[TableId::CountryLinks.index()].clone();
    decoded.hour += 1;
    observe(&mut scanned, &decoded, None, &mut builder).unwrap();
    assert_eq!(
        builder.aggregations, 3,
        "lineage changes invalidate dense-slot totals"
    );
    decoded.metrics = Metrics::default();
    decoded.hour += 1;
    assert!(
        slot_read(
            replay::format::FormatVersion::CURRENT,
            &decoded.metrics,
            TableId::Provinces,
            decoded.hour
        ),
        "hourly reads stay fresh without a timer"
    );
    decoded.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unread province table".into(),
    }
    .into();
    observe(&mut scanned, &decoded, None, &mut builder).unwrap();
    assert_eq!(builder.aggregations, 4);
    assert_eq!(
        builder.previous[0][1], None,
        "an unavailable read cannot reuse earlier totals"
    );
    decoded.hour += 1;
    observe(&mut scanned, &decoded, None, &mut builder).unwrap();
    assert_eq!(
        builder.aggregations, 4,
        "an unchanged unavailable view reuses its gap"
    );
    decoded.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![])).into();
    decoded.hour += 1;
    observe(&mut scanned, &decoded, None, &mut builder).unwrap();
    assert_eq!(builder.aggregations, 5);
    assert_eq!(builder.previous[0][1], Some(0));
}

#[test]
fn territory_top_accepts_the_full_ui_safe_integer_count() {
    let params: TerritoryParams = serde_json::from_value(serde_json::json!({
        "branchId": "1", "tags": [], "top": { "count": MAX_EXACT_JS_INTEGER, "measure": "states", "rule": "controlled" },
    })).unwrap();
    assert_eq!(params.top.as_ref().unwrap().count, MAX_EXACT_JS_INTEGER);
    Territory::validate(&params).unwrap();
}
