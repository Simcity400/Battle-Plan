use super::*;
use replay::spool::Record;
use replay_data::{
    identity::CaptureMode, identity::Roster, identity::SampleBoundary, identity::SessionStart,
    identity::Verification, rows::CampaignRow, rows::CountryLinkRow, rows::CountryRow,
    rows::SunkShipsRow, rows::TableRows,
};
use replay_frame::{
    branches::BranchReason, branches::BranchStart, branches::Parentage, frame::FrameEncoder,
    frame::Metrics, frame::TableView,
};

fn ship(name: &[u8], hour: i32) -> SunkShipEntry {
    SunkShipEntry {
        ship_name: name.into(),
        sink_hour: hour,
        victim_country: 1,
        attacker_country: 2,
        class_token: 77,
        ..Default::default()
    }
}
fn tags() -> HashMap<u32, String> {
    [(1, "opaque victim".into()), (2, "opaque attacker".into())].into()
}

#[test]
fn retention_progress_matches_oracle_and_revisits_only_extended_or_new_breaks() {
    let defines = RetentionDefines {
        values: Some([Ok(1), Ok(2)]),
    };
    let mut progress = RetentionProgress::new(defines.clone());
    let first = month_start(1940 * 12).unwrap();
    let mut scan = Scanned::default();
    for step in 0..20 {
        let from = first + step * 20000;
        scan.ship_breaks.push(HourSpan {
            first_hour: from,
            last_hour: from + 10000,
        });
        scan.convoy_breaks.push(HourSpan {
            first_hour: from,
            last_hour: from + 10000,
        });
        assert_eq!(progress.update(&scan), &defines.evaluate(&scan));
    }
    work::WORK.with(|work| *work.borrow_mut() = work::Work::default());
    for _ in 0..100 {
        progress.update(&scan);
    }
    assert_eq!(work::WORK.with(|work| work.borrow().retention_spans), 0);
    scan.ship_breaks.last_mut().unwrap().last_hour += 24;
    scan.convoy_breaks.last_mut().unwrap().last_hour += 24;
    let actual = progress.update(&scan).clone();
    assert_eq!(
        work::WORK.with(|work| work.borrow().retention_spans),
        2,
        "old break history is never revisited when the trailing break extends"
    );
    assert_eq!(actual, defines.evaluate(&scan));
    for values in [
        [Ok(0), Ok(-1)],
        [Err("synthetic mod define missing".into()), Ok(2)],
        [Ok(i32::MAX), Ok(0)],
    ] {
        let defines = RetentionDefines {
            values: Some(values),
        };
        let mut progress = RetentionProgress::new(defines.clone());
        assert_eq!(
            progress.update(&scan),
            &defines.evaluate(&scan),
            "refused content keeps the oracle warning order"
        );
    }
}

impl Tracker {
    fn read_ships(&mut self, out: &mut Scanned, entries: &[SunkShipEntry], hour: i32) {
        let view = TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: entries.to_vec(),
        }]))
        .into();
        self.ships(out, &view, hour)
    }
}

#[test]
fn sinkings_whole_entry_multiset_handles_swap_erase_duplicates_and_all_fields() {
    let a = ship(b"A", 1);
    let b = ship(b"B", 2);
    let c = ship(b"C", 3);
    let mut track = Tracker::default();
    let mut out = Scanned::default();
    track.read_ships(&mut out, &[a.clone(), a.clone(), b.clone()], 10);
    track.read_ships(
        &mut out,
        &[b.clone(), a.clone(), a, c.clone(), c.clone()],
        11,
    );
    assert_eq!(out.ships.len(), 5);
    track.read_ships(&mut out, &[c.clone(), b, c.clone()], 12);
    assert_eq!(out.ships.len(), 5, "swap-erased positions are not births");
    // Fields omitted by the old consumer still distinguish complete entries.
    let mut variants = Vec::new();
    macro_rules! changed {
        ($field:ident, $value:expr) => {{
            let mut entry = c.clone();
            entry.$field = $value;
            variants.push(entry);
        }};
    }
    changed!(attribution_name, b"other".to_vec());
    changed!(class_name, b"other".to_vec());
    changed!(attribution_class_name, b"other".to_vec());
    changed!(victim_country, 3);
    changed!(attacker_country, 3);
    changed!(class_token, 88);
    changed!(attribution_class_token, 88);
    changed!(attribution_class_present, 1);
    changed!(model_level, 1);
    changed!(victim_equipment_variant, 99);
    changed!(damage_source_reference, 99);
    changed!(province, 99);
    changed!(province_present, 1);
    changed!(battle, 99);
    changed!(cause_record, 1);
    changed!(assisted_kill, 1);
    let keys: BTreeSet<_> = variants.iter().map(ShipKey::from).collect();
    assert_eq!(keys.len(), variants.len());
    assert!(!keys.contains(&ShipKey::from(&c)));
    track.read_ships(&mut out, &variants, 13);
    assert_eq!(out.ships.len(), 5 + variants.len());
}

#[test]
fn sinkings_gap_recovers_event_hours_without_recounting_old_survivors() {
    let mut track = Tracker::default();
    let mut out = Scanned::default();
    let old = ship(b"old", 1);
    track.read_ships(&mut out, &[old.clone(), ship(b"pruned", 2)], 10);
    out.missing(&mut track, 11, 40);
    track.read_ships(
        &mut out,
        &[
            old,
            ship(b"last-read-hour birth", 10),
            ship(b"gap", 25),
            ship(b"gap", 25),
        ],
        41,
    );
    assert_eq!(
        out.ships
            .iter()
            .map(|e| e.entry.sink_hour)
            .collect::<Vec<_>>(),
        [1, 2, 10, 25, 25]
    );
    assert_eq!(out.gap_recoveries, 1);
    assert_eq!(
        out.ship_unread,
        [HourSpan {
            first_hour: 11,
            last_hour: 40
        }]
    );
    let mut malformed = ship(b"bad flag", 40);
    malformed.assisted_kill = 2;
    track.read_ships(
        &mut out,
        &[ship(b"future", 100), malformed, ship(b"valid", 41)],
        41,
    );
    assert_eq!(out.ships.last().unwrap().entry.ship_name, b"valid");
    assert_eq!(out.refused_entries.values().sum::<usize>(), 2);
}

#[test]
fn sinkings_unread_spans_merge_without_losing_separate_breaks() {
    let mut spans = vec![
        HourSpan {
            first_hour: 3,
            last_hour: 5,
        },
        HourSpan {
            first_hour: 34,
            last_hour: 40,
        },
    ];
    append_span(&mut spans, 11, 40);
    assert_eq!(
        spans,
        [
            HourSpan {
                first_hour: 3,
                last_hour: 5
            },
            HourSpan {
                first_hour: 11,
                last_hour: 40
            }
        ]
    );
    append_span(&mut spans, 7, 8);
    assert_eq!(spans.len(), 3);
    append_span(&mut spans, 6, 10);
    assert_eq!(
        spans,
        [HourSpan {
            first_hour: 3,
            last_hour: 40
        }]
    );
}

#[test]
fn sinkings_convoy_delta_keeps_signed_corrections_month_read_and_unknown_owner() {
    let mut track = Tracker::default();
    let mut out = Scanned::default();
    let row = |owner, count| SunkConvoyRow {
        month: 1940 * 12,
        killer_country: 2,
        owner_country: owner,
        convoys: count,
        ordinal: 0,
    };
    track.convoys(&mut out, &[row(1, 5), row(3, 2)], 10, &tags());
    track.convoys(&mut out, &[row(1, 7), row(3, 2)], 20, &tags());
    track.convoys(&mut out, &[row(1, 6)], 30, &tags());
    assert_eq!(
        out.convoys
            .iter()
            .map(|e| (e.read_hour, e.count, e.baseline))
            .collect::<Vec<_>>(),
        [
            (10, 5, true),
            (10, 2, true),
            (20, 2, false),
            (30, -1, false)
        ]
    );
    assert!(out.convoys.iter().all(|e| e.month == 1940 * 12));
    assert!(out.convoys[1].owner.is_none());
    track.convoys(&mut out, &[row(1, 0)], 31, &tags());
    let correction = out.convoys.last().unwrap();
    assert_eq!((correction.count, correction.read_hour), (-6, 31));
    track.convoys(&mut out, &[row(1, -1), row(3, 4)], 32, &tags());
    assert_eq!(
        out.convoys.last().unwrap().count,
        2,
        "corrections do not abort another bucket's deltas"
    );
    assert_eq!(out.convoys[out.convoys.len() - 2].count, -1);
    track.convoys(&mut out, &[row(1, -1), row(3, 5)], 33, &tags());
    assert!(out.refused_entries.is_empty());
    assert_eq!(out.convoy_reads, [10, 20, 30, 31, 32, 33]);
    assert_eq!(
        out.convoys
            .iter()
            .filter(|e| e.owner_raw == 1)
            .map(|e| e.count)
            .sum::<i64>(),
        -1
    );
}

#[test]
fn sinkings_signed_bucket_endpoints_include_initial_negatives_and_native_wrap() {
    let mut track = Tracker::default();
    let mut out = Scanned::default();
    let row = |count| SunkConvoyRow {
        month: 1940 * 12,
        killer_country: 2,
        owner_country: 1,
        convoys: count,
        ordinal: 0,
    };
    for (hour, amount) in [(10, -3), (20, -3), (30, i32::MAX), (40, i32::MIN)] {
        track.convoys(&mut out, &[row(amount)], hour, &tags());
        assert_eq!(
            out.convoys.iter().map(|event| event.count).sum::<i64>(),
            i64::from(amount)
        );
    }
    assert_eq!(out.convoys[0].count, -3);
    assert_eq!(out.convoys.last().unwrap().count, -4_294_967_295);
    assert_eq!(out.convoy_reads, [10, 20, 30, 40]);
}

#[test]
fn sinkings_equal_ship_views_still_record_every_supplying_read() {
    let mut track = Tracker::default();
    let mut out = Scanned::default();
    let view = TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow::default()])).into();
    for hour in [10, 34, 58] {
        track.ships(&mut out, &view, hour);
    }
    assert_eq!(out.ship_reads, [10, 34, 58]);
    assert!(out.ships.is_empty());
}

fn frame(hour: i32, countries: u32) -> DecodedFrame {
    let mut table: [TableView; TableId::ALL.len()] =
        std::array::from_fn(|_| TableView::Unavailable {
            reason: "not used in synthetic naval fixture".into(),
        });
    table[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: b"synthetic naval".to_vec(),
        }]));
    table[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=countries)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod country {id:04}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    table[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=countries)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    table[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow::default()]));
    table[TableId::SunkConvoys.index()] = TableView::Complete(TableRows::SunkConvoys(vec![]));
    DecodedFrame {
        branch: 1,
        hour,
        tables: table.map(Into::into),
        metrics: Metrics {
            table_ns: std::array::from_fn(|i| {
                u64::from(matches!(
                    TableId::ALL[i],
                    TableId::SunkShips | TableId::SunkConvoys
                ))
            }),
            ..Default::default()
        },
    }
}
fn replay_of(frames: Vec<DecodedFrame>) -> Replay {
    let session = SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        capture_mode: CaptureMode::Full,
        identity: replay_data::identity::Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    };
    let mut records = vec![
        Record::SessionStart(session),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: None,
            first_hour: frames[0].hour,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    let mut branch = 1;
    for frame in frames {
        if frame.branch != branch {
            branch = frame.branch;
            records.push(Record::BranchStart(BranchStart {
                id: branch,
                session: 1,
                loaded_hour: None,
                first_hour: frame.hour,
                parent: Parentage::Unknown,
                reason: BranchReason::LoadSignal,
            }));
            encoder = FrameEncoder::new();
        }
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(b"synthetic naval".to_vec(), &[records]).unwrap()
}

#[test]
fn sinkings_unchanged_view_reconsiders_a_previously_future_event() {
    let view: SharedView = TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
        entries: vec![ship(b"valid", 10), ship(b"future", 12)],
    }]))
    .into();
    let mut track = Tracker::default();
    let mut out = Scanned::default();
    track.ships(&mut out, &view, 10);
    track.ships(&mut out, &view, 11);
    assert_eq!(out.ships.len(), 1);
    track.ships(&mut out, &view, 12);
    track.ships(&mut out, &view, 13);
    assert_eq!(out.ships.len(), 2);
    assert_eq!(out.refused_entries.values().sum::<usize>(), 1);
}

#[test]
fn sinkings_changed_views_count_only_added_refused_multiplicities() {
    let mut track = Tracker::default();
    let mut out = Scanned::default();
    let mut malformed = ship(b"persistent malformed", 10);
    malformed.cause_record = 2;
    let future = ship(b"persistent future", 30);
    track.read_ships(&mut out, &[malformed.clone(), future.clone()], 10);
    track.read_ships(
        &mut out,
        &[future.clone(), malformed.clone(), ship(b"valid", 11)],
        11,
    );
    assert_eq!(out.refused_entries.values().sum::<usize>(), 2);
    track.read_ships(&mut out, &[malformed.clone(), malformed, future], 12);
    assert_eq!(
        out.refused_entries.values().sum::<usize>(),
        3,
        "one added malformed duplicate"
    );
}

#[test]
fn sinkings_retention_risk_begins_right_after_the_previous_read() {
    use dashboard_text::catalogue::tests::folder_catalogue;
    let (cat, _folder) = folder_catalogue(&[(
        "common/defines/mod.lua",
        "NDefines={NNavy={NAVAL_COMBAT_RESULT_TIMEOUT_YEARS=1, CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS=3}}",
    )]);
    let first = month_start(1940 * 12).unwrap();
    let mut out = Scanned {
        last_ship_read: Some(first),
        last_convoy_read: Some(first),
        ..Default::default()
    };
    out.missing(&mut Tracker::default(), first + 24, first + 8760);
    assert_eq!(out.ship_unread[0].first_hour, first + 24);
    assert_eq!(out.ship_breaks[0].first_hour, first + 1);
    assert_eq!(out.convoy_breaks[0].first_hour, first + 1);
    let retained = retention(&out, Some(&cat));
    assert_eq!(
        retained.ship_loss,
        [HistoryLoss {
            first_hour: first + 1,
            last_hour: first + 1,
            known_at: first + 8761
        }]
    );
}

#[test]
fn sinkings_attribution_bridges_agreeing_runs_and_bounds_references_without_history() {
    let run = |first, last, tag: &str, identity| TagRun {
        first,
        last,
        tag: tag.into(),
        identity,
    };
    let history = HashMap::from([(
        9,
        vec![run(10, 12, "holder", (1, 1)), run(20, 22, "holder", (1, 1))],
    )]);
    let candidates = BTreeMap::from([(99, BTreeSet::from([(1, 1), (3, 3)]))]);
    assert_eq!(
        attribution(9, 15, &history, &candidates),
        (Some("holder".into()), BTreeSet::new())
    );
    assert_eq!(
        attribution(99, 15, &history, &candidates),
        (None, BTreeSet::from([(1, 1), (3, 3)]))
    );
    assert_eq!(
        attribution(100, 15, &history, &candidates),
        (None, BTreeSet::new())
    );
}

#[test]
fn sinkings_convoy_attribution_bridges_an_unread_country_frame() {
    let first = 60_800_000;
    let before = frame(first, 2);
    let mut unread = before.clone();
    unread.hour += 1;
    *unread.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "synthetic country gap".into(),
    };
    *unread.tables[TableId::SunkConvoys.index()] =
        TableView::Complete(TableRows::SunkConvoys(vec![SunkConvoyRow {
            month: month(first).unwrap(),
            owner_country: 1,
            killer_country: 2,
            convoys: 3,
            ordinal: 0,
        }]));
    let mut after = before.clone();
    after.hour += 2;
    let scanned = scan(
        &replay_of(vec![before, unread, after]),
        1,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        scanned.convoys[0].owner.as_deref(),
        Some("mod country 0001")
    );
    assert_eq!(
        scanned.convoys[0].killer.as_deref(),
        Some("mod country 0002")
    );
    assert!(
        scanned.uncertain_convoy_owners.is_empty() && scanned.uncertain_convoy_killers.is_empty()
    );
    assert!(
        scanned.unresolved.is_empty(),
        "resolved gap references add no unresolved warning"
    );
}

#[test]
fn sinkings_cache_retains_the_last_two_branches_and_rebuilds_an_evicted_branch() {
    let frames = (1..=3)
        .map(|branch| {
            let mut frame = frame(60_800_000, 2);
            frame.branch = branch;
            frame
        })
        .collect();
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "cache-retention".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay_of(frames)),
        &AtomicBool::new(false),
    )
    .unwrap();
    let flag = AtomicBool::new(false);
    let metadata = source.metadata();
    assert_eq!(metadata.branches.len(), 3);
    assert!(
        metadata
            .branches
            .iter()
            .all(|branch| branch.first_hour == 60_800_000 && branch.last_hour == 60_800_000)
    );
    let oldest = cached(&source, 1, &flag).unwrap();
    cached(&source, 2, &flag).unwrap();
    let latest = cached(&source, 3, &flag).unwrap();
    assert!(!Arc::ptr_eq(&oldest, &cached(&source, 1, &flag).unwrap()));
    assert!(Arc::ptr_eq(&latest, &cached(&source, 3, &flag).unwrap()));
}

#[test]
fn sinkings_scan_supplies_the_branch_country_histories_without_another_walk() {
    let first = 60_800_000;
    let mut renamed = frame(first + 1, 2);
    *renamed.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: b"renamed".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 2,
            canonical_slot: 2,
            tag: b"mod country 0002".to_vec(),
            ..Default::default()
        },
    ]));
    let restored = frame(first + 2, 2);
    let replay = replay_of(vec![
        frame(first, 2),
        renamed,
        restored,
        frame(first + 6, 2),
    ]);
    let flag = AtomicBool::new(false);
    let walked = country_history::scan_histories(&replay, 1, false, &[], &flag).unwrap();
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "shared-histories".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let scanned = cached(&source, 1, &flag).unwrap();
    let kept = country_history::branch_histories(&source, 1, false, &[], &flag).unwrap();
    assert!(Arc::ptr_eq(&scanned.histories, &kept));
    let identities = |histories: &RecordedHistories| {
        histories
            .countries
            .iter()
            .map(|(tag, country)| (tag.clone(), country.identities.to_vec()))
            .collect::<Vec<_>>()
    };
    assert_eq!(identities(&kept), identities(&walked));
    assert_eq!(kept.identity_reads, walked.identity_reads);
}

#[test]
fn sinkings_real_decoder_dates_fresh_reads_and_country_identity_at_event_hour() {
    let first = 60_800_000;
    let mut initial = frame(first, 2);
    *initial.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            replay_data::rows::LexicalTokenRow {
                token: 77,
                literal: b"first_class".to_vec(),
            },
        ]));
    let mut frames = vec![initial];
    let mut fresh = frames[0].clone();
    fresh.hour += 1;
    *fresh.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![ship(b"dated", first)],
        }]));
    frames.push(fresh.clone());
    let mut held = fresh.clone();
    held.hour += 1;
    held.metrics = Metrics::default();
    // A mutated carried view is not a read and cannot add a new event.
    *held.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![ship(b"dated", first), ship(b"held", first + 2)],
        }]));
    frames.push(held);
    let mut recover = fresh;
    recover.hour += 10;
    *recover.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![ship(b"dated", first), ship(b"gap", first + 5)],
        }]));
    let mut tail = recover.clone();
    tail.hour += 1;
    tail.metrics = Metrics::default();
    *recover.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            replay_data::rows::LexicalTokenRow {
                token: 77,
                literal: b"first_class".to_vec(),
            },
            replay_data::rows::LexicalTokenRow {
                token: 88,
                literal: b"later_mod_class".to_vec(),
            },
        ]));
    frames.push(recover);
    frames.push(tail);
    let replay = replay_of(frames);
    let out = scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(out.ships.len(), 2);
    assert_eq!(
        out.tokens.get(&88).map(String::as_str),
        Some("later_mod_class")
    );
    assert_eq!(out.ships[0].victim.as_deref(), Some("mod country 0001"));
    assert_eq!(
        out.ships[1].victim.as_deref(),
        Some("mod country 0001"),
        "unchanged holders on both sides establish gap attribution"
    );
    assert_eq!(out.identity("mod country 0001", first + 5), Some((1, 1)));
    assert_eq!(
        out.ship_unread,
        [HourSpan {
            first_hour: first + 3,
            last_hour: first + 10
        }]
    );
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "synthetic".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        &cached(&source, 1, &AtomicBool::new(false)).unwrap(),
        &cached(&source, 1, &AtomicBool::new(false)).unwrap()
    ));
}

#[test]
fn sinkings_records_native_holders_through_tag_switch_reuse_and_held_country_views() {
    let first = 60_800_000;
    let old_tag = "opaque old tag";
    let new_tag = "opaque changed tag";
    let mut before = frame(first, 2);
    *before.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: old_tag.as_bytes().to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 2,
            canonical_slot: 2,
            tag: b"second country".to_vec(),
            ..Default::default()
        },
    ]));
    let mut after = before.clone();
    after.hour = first + 1;
    *after.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: new_tag.as_bytes().to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 2,
            canonical_slot: 2,
            tag: old_tag.as_bytes().to_vec(),
            ..Default::default()
        },
    ]));
    let mut replacement_loss = ship(b"replacement country ship", first + 1);
    replacement_loss.victim_country = 2;
    *after.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![ship(b"original country ship", first), replacement_loss],
        }]));
    let mut held = after.clone();
    held.hour = first + 2;
    held.metrics = Metrics::default();
    let out = scan(
        &replay_of(vec![before, after, held]),
        1,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(out.identity(old_tag, first), Some((1, 1)));
    assert_eq!(out.identity(old_tag, first + 1), Some((2, 2)));
    assert_eq!(out.identity(new_tag, first + 2), Some((1, 1)));
    assert_eq!(out.identity(new_tag, first), None);
    assert_eq!(out.country_identities[old_tag].len(), 2);
    assert_eq!(out.country_identities[old_tag][1].last_hour, first + 2);
    let original = out
        .ships
        .iter()
        .find(|event| event.entry.sink_hour == first)
        .unwrap();
    assert_eq!(original.victim.as_deref(), Some(old_tag));
    assert_eq!(
        out.identity(original.victim.as_ref().unwrap(), original.entry.sink_hour),
        Some((1, 1))
    );
    let replacement = out
        .ships
        .iter()
        .find(|event| event.entry.sink_hour == first + 1)
        .unwrap();
    assert_eq!(replacement.victim.as_deref(), Some(old_tag));
    assert_eq!(
        out.identity(
            replacement.victim.as_ref().unwrap(),
            replacement.entry.sink_hour
        ),
        Some((2, 2))
    );
}

#[test]
fn sinkings_retained_entries_name_countries_at_the_supplying_read_only() {
    let first = 60_800_000;
    let mut before = frame(first, 3);
    let retained = ship(b"retained", first - 24);
    let mut unresolved = ship(b"unresolved at supplying read", first - 12);
    unresolved.victim_country = 99;
    *before.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![retained.clone(), unresolved.clone()],
        }]));
    let mut after = before.clone();
    after.hour += 1;
    *after.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=3)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("later mod tag {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *after.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 3,
                canonical_slot: 3,
            },
            CountryLinkRow {
                id: 99,
                canonical_slot: 3,
            },
        ]));
    *after.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![retained, unresolved, ship(b"earlier history wins", first)],
        }]));
    let scanned = scan(&replay_of(vec![before, after]), 1, &AtomicBool::new(false)).unwrap();
    let retained = &scanned.ships[0];
    assert_eq!(retained.entry.sink_hour, first - 24);
    assert_eq!(retained.victim.as_deref(), Some("mod country 0001"));
    assert_eq!(retained.attacker.as_deref(), Some("mod country 0002"));
    assert_eq!(retained.victim_as_of, Some(first));
    assert_eq!(retained.attacker_as_of, Some(first));
    assert_eq!(
        scanned.identity(retained.victim.as_ref().unwrap(), first),
        Some((1, 1))
    );
    assert!(!scanned.uncertain_victims.contains_key(&(1, 1)));
    let unresolved = &scanned.ships[1];
    assert!(unresolved.victim.is_none());
    assert_eq!(
        unresolved.victim_as_of, None,
        "a later alias cannot name the supplying read"
    );
    let earlier = &scanned.ships[2];
    assert_eq!(earlier.read_hour, first + 1);
    assert_eq!(earlier.victim.as_deref(), Some("mod country 0001"));
    assert_eq!(earlier.attacker.as_deref(), Some("mod country 0002"));
    assert_eq!(earlier.victim_as_of, None);
    assert_eq!(earlier.attacker_as_of, None);
}

#[test]
fn sinkings_changed_holder_across_gap_marks_only_the_possible_countries() {
    let first = 60_800_000;
    let before = frame(first, 3);
    let mut after = before.clone();
    after.hour += 10;
    *after.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 2,
            canonical_slot: 2,
            tag: b"mod country 0002".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 3,
            canonical_slot: 3,
            tag: b"mod country 0003".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 91,
            canonical_slot: 1,
            tag: b"mod country 0001".to_vec(),
            ..Default::default()
        },
    ]));
    // Keep the old raw reference as an alias, and map the new native id too.
    // mapped_slot requires the holder's own CountryLinks entry.
    *after.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 3,
                canonical_slot: 3,
            },
            CountryLinkRow {
                id: 91,
                canonical_slot: 1,
            },
        ]));
    *after.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![ship(b"ambiguous victim", first + 5)],
        }]));
    let scanned = scan(&replay_of(vec![before, after]), 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.identity("mod country 0001", first), Some((1, 1)));
    assert_eq!(
        scanned.identity("mod country 0001", first + 10),
        Some((1, 91))
    );
    assert_eq!(scanned.identity("mod country 0001", first + 5), None);
    assert!(scanned.ships[0].victim.is_none());
    assert_eq!(
        scanned.ships[0].attacker.as_deref(),
        Some("mod country 0002")
    );
    assert_eq!(
        scanned
            .uncertain_victims
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [(1, 1), (1, 91)]
    );
    assert!(!scanned.uncertain_victims.contains_key(&(3, 3)));
    assert!(scanned.uncertain_attackers.is_empty());
}

fn archive_observed(
    history: &ShipHistory,
    ships: &mut BTreeMap<u32, ShipEvent>,
    reads: &mut Vec<i32>,
) {
    for (&index, event) in history.ship_indices.iter().zip(&history.scan.ships) {
        if let Some(prior) = ships.get_mut(&index) {
            prior.victim = event.victim.clone();
            prior.attacker = event.attacker.clone();
            prior.victim_as_of = event.victim_as_of;
            prior.attacker_as_of = event.attacker_as_of;
        } else {
            ships.insert(index, event.clone());
        }
    }
    reads.extend(&history.scan.ship_reads);
}

#[test]
fn prepared_ship_history_resumes_duplicate_and_pruning_state_without_recovery_births() {
    let mut history = ShipHistory::default();
    let mut archived = BTreeMap::new();
    let mut reads = Vec::new();
    let mut oracle = Scanned::default();
    let mut track = Tracker::default();
    let format = replay_data::format::FormatVersion::default();
    let a = ship(b"same mod name", 1);
    let b = ship(b"same mod name", 2);
    for (hour, entries) in [
        (10, vec![a.clone(), a.clone()]),
        (11, vec![a.clone(), a, b.clone()]),
        (12, vec![b]),
    ] {
        let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
        frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
        frame.tables[TableId::SunkShips.index()] =
            TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
                entries: entries.clone(),
            }]))
            .into();
        history.observe(format, &frame);
        track.read_ships(&mut oracle, &entries, hour);
        archive_observed(&history, &mut archived, &mut reads);
        history.compact();
        let saved = serde_json::to_value(&history).unwrap();
        assert!(saved["scan"].get("ships").is_none());
        assert!(
            saved["open_ships"]
                .as_array()
                .unwrap()
                .iter()
                .all(|event| event.as_object().unwrap().len() == 5)
        );
        history = serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
        history.resume(&frame);
    }
    assert_eq!(archived.len(), 3);
    assert_eq!(reads, oracle.ship_reads);
    for (actual, expected) in archived.values().zip(&oracle.ships) {
        assert_eq!(actual.entry, expected.entry);
        assert_eq!(actual.read_hour, expected.read_hour);
    }
}

#[test]
fn prepared_ship_history_retains_multiset_through_unavailable_resume_and_future_read() {
    let format = replay_data::format::FormatVersion::default();
    let mut history = ShipHistory::default();
    let mut archived = BTreeMap::new();
    let mut reads = Vec::new();
    let mut oracle = Scanned::default();
    let mut track = Tracker::default();
    let mut a = ship(b"stored mod ship", 9);
    a.attribution_name = b"attributed mod name".to_vec();
    a.class_name = b"stored class".to_vec();
    a.attribution_class_name = b"attributed class".to_vec();
    a.model_level = -2;
    a.victim_equipment_variant = u64::MAX;
    a.damage_source_reference = u64::MAX - 1;
    a.province = 99;
    a.province_present = 1;
    a.battle = u64::MAX;
    a.attribution_class_token = 123;
    a.attribution_class_present = 1;
    a.cause_record = 1;
    a.assisted_kill = 1;
    let future = ship(b"future mod ship", 13);
    for (hour, entries) in [
        (10, Some(vec![a.clone(), a.clone(), future.clone()])),
        (11, None),
        (12, Some(vec![a.clone(), future.clone()])),
        (13, Some(vec![future])),
    ] {
        let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
        frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
        if let Some(entries) = entries {
            frame.tables[TableId::SunkShips.index()] =
                TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
                    entries: entries.clone(),
                }]))
                .into();
            track.read_ships(&mut oracle, &entries, hour);
        } else {
            frame.tables[TableId::SunkShips.index()] = TableView::Unavailable {
                reason: "history failed".into(),
            }
            .into();
            oracle.missing(&mut track, hour, hour);
        }
        history.observe(format, &frame);
        archive_observed(&history, &mut archived, &mut reads);
        history.compact();
        let saved = serde_json::to_value(&history).unwrap();
        assert!(saved["scan"].get("ships").is_none());
        assert!(
            saved["open_ships"]
                .as_array()
                .unwrap()
                .iter()
                .all(|event| event.as_object().unwrap().len() == 5)
        );
        history = serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
        history.resume(&frame);
    }
    assert_eq!(archived.len(), 3);
    assert_eq!(reads, oracle.ship_reads);
    for (actual, expected) in archived.values().zip(&oracle.ships) {
        assert_eq!(actual.entry, expected.entry);
        assert_eq!(actual.read_hour, expected.read_hour);
    }
}

#[test]
fn shared_sinking_observer_matches_oracle_after_commits_resume_append_and_country_scale() {
    use dashboard_base::prepared::series::{self, tests::Fixture};
    let flag = AtomicBool::new(false);
    let mut first = frame(10, 12);
    let mut second = frame(11, 12);
    let mut missing = frame(13, 12);
    let mut appended = frame(17, 12);
    let a = ship(b"duplicate retained ship", 9);
    let b = ship(b"new observed ship", 10);
    let c = ship(b"gap sinking", 14);
    for (input, entries) in [
        (&mut first, vec![a.clone(), a.clone()]),
        (&mut second, vec![a.clone(), b.clone()]),
        (&mut appended, vec![b, c]),
    ] {
        input.tables[TableId::SunkShips.index()] =
            TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow { entries }])).into();
    }
    missing.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "identity unavailable".into(),
    }
    .into();
    missing.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "identity unavailable".into(),
    }
    .into();
    missing.tables[TableId::SunkShips.index()] = TableView::Unavailable {
        reason: "history unavailable".into(),
    }
    .into();
    let mut countries = appended.complete::<tables::Countries>().unwrap().to_vec();
    countries[0].id = 6001;
    countries[0].tag = b"renamed mod holder".to_vec();
    countries.sort_by_key(|row| row.id);
    appended.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries)).into();
    let mut links = appended
        .complete::<tables::CountryLinks>()
        .unwrap()
        .to_vec();
    links.push(CountryLinkRow {
        id: 6001,
        canonical_slot: 1,
    });
    appended.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links)).into();
    let inputs = vec![first, second, missing, appended];
    let replay = replay_of(inputs.clone());
    let fixture = Fixture::new();
    let mut history = ShipHistory::new(10);
    let mut archived = BTreeMap::new();
    let mut reads = Vec::new();
    let mut convoy_reads = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        history.observe(replay.format, input);
        history.finish(input.hour);
        archive_observed(&history, &mut archived, &mut reads);
        convoy_reads.extend(&history.scan.convoy_reads);
        history.compact();
        let mut writer = fixture.writer();
        series::write_reply(&mut writer, "observer", 0, &history).unwrap();
        writer.commit().unwrap();
        history = series::read_reply(&fixture.store, &fixture.key, "observer", 0)
            .unwrap()
            .unwrap();
        history.resume(input);
        if index == 1 || index + 1 == inputs.len() {
            let oracle = scan(&replay_of(inputs[..=index].to_vec()), 1, &flag).unwrap();
            let mut actual = history.scan.metadata();
            actual.ship_reads = reads.clone();
            actual.convoy_reads = convoy_reads.clone();
            assert_eq!(
                serde_json::to_value(archived.values().collect::<Vec<_>>()).unwrap(),
                serde_json::to_value(&oracle.ships).unwrap()
            );
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                serde_json::to_value(&oracle).unwrap()
            );
        }
    }
    assert_eq!(archived.len(), 4);
    assert_eq!(archived[&0].victim_as_of, Some(10));
    assert!(archived.last_key_value().unwrap().1.victim.is_none());
    assert_eq!(history.scan.country_candidates[&1].len(), 2);
}
