use super::*;
use dashboard_core::test_fixtures::{replay, slot_hour, stockpile_frame};
use replay::{
    frame::TableView,
    rows::{LogisticsEquipmentRow, TableRows},
};

#[test]
fn numeric_projection_visits_only_keys_whose_interval_changed() {
    let ids: BTreeSet<u64> = (0..200).collect();
    let mut numbers = Numbers::default();
    let mut rows = ReadRuns::new();
    rows.read(10, ids.iter().map(|&id| (id, Some(7))));
    numbers.advance(10).unwrap();
    numbers.values("mod holder", 0, 10, &mut rows, &ids, &ids);
    assert_eq!(numbers.projected, 200);
    for hour in 11..=20 {
        rows.read(
            hour,
            ids.iter()
                .map(|&id| (id, Some(if id == 0 { i64::from(hour) } else { 7 }))),
        );
        numbers.advance(hour).unwrap();
        let before = numbers.projected;
        numbers.values("mod holder", 0, hour, &mut rows, &ids, &BTreeSet::new());
        assert_eq!(numbers.projected - before, 1);
        assert!(rows.runs.values().all(|runs| runs.len() == 1));
    }
    // Missing rows become known absence once, and do not keep revisiting
    // retired keys on subsequent continuing reads.
    rows.read(21, [(0, Some(21))]);
    numbers.advance(21).unwrap();
    numbers.values("mod holder", 0, 21, &mut rows, &ids, &BTreeSet::new());
    assert_eq!(rows.runs.len(), 1);
    rows.read(22, [(0, Some(22))]);
    numbers.advance(22).unwrap();
    let before = numbers.projected;
    numbers.values("mod holder", 0, 22, &mut rows, &ids, &BTreeSet::new());
    assert_eq!(numbers.projected - before, 1);
}

#[test]
fn repeated_numeric_reads_skip_numeric_projection_and_evaluate_groups() {
    let first = slot_hour(TableId::Stockpiles);
    let mut frame = stockpile_frame(first, true, &[(10, 100)], Some(&[(10, 100, 1, 0)]));
    frame.metrics.table_ns[TableId::Stockpiles.index()] = 1;
    frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = 1;
    *frame.tables[TableId::LogisticsEquipment.index()] =
        TableView::Complete(TableRows::LogisticsEquipment(vec![LogisticsEquipmentRow {
            country: 11,
            archetype: 1,
            token: 1,
            balance_raw: -5,
            stock_raw: 7,
            available_raw: replay::rows::Recorded::Value(Some(3)),
        }]));
    let format = replay(vec![frame.clone()]).format;
    let mut prepared = Prepared::default();
    prepared.observe(format, &frame).unwrap();
    let before = prepared.choices.work.history_visits;
    let projected = prepared.numbers.projected;
    for offset in 1..=10 {
        frame.hour = first + offset;
        // Whole-table observation exercises the projection even when every
        // numeric value repeats; reads still have their own dated events.
        prepared.observe(format, &frame).unwrap();
    }
    assert_eq!(prepared.numbers.projected, projected);
    assert!(prepared.choices.work.history_visits > before);
}

#[test]
fn stockpile_observer_work_is_flat_over_carried_hours_and_retired_country_aliases() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{self, Work},
    };
    let first = slot_hour(TableId::Stockpiles);
    let initial = stockpile_frame(first, true, &[(10, 100), (20, 0)], Some(&[]));
    let format = replay(vec![initial.clone()]).format;
    observer_tests::assert_flat(
        observer_tests::scaled_work(10, |hours| {
            let mut prepared = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            prepared.observe_changed(format, &initial, &tracker.observe(&initial, format))?;
            let mut carried = initial.clone();
            carried.metrics.table_ns.fill(0);
            observer_tests::unchanged_frames(
                prepared.work,
                &carried,
                hours as i32,
                format,
                |frame, changes| {
                    prepared.observe_changed(format, frame, changes)?;
                    Ok(prepared.work)
                },
            )
        })
        .unwrap(),
    );
    observer_tests::assert_flat(
        observer_tests::scaled_work(10, |history| {
            let mut prepared = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            let mut frame = initial.clone();
            for offset in 0..history {
                frame.hour = first + offset as i32;
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.iter_mut()
                        .find(|row| row.canonical_slot == 7)
                        .unwrap()
                        .tag = format!("old stockpile alias {offset}").into_bytes();
                }
                prepared.observe_changed(format, &frame, &tracker.observe(&frame, format))?;
            }
            let before = prepared.work;
            frame.hour += 1;
            if let TableView::Complete(TableRows::Stockpiles(rows)) =
                &mut *frame.tables[TableId::Stockpiles.index()]
            {
                rows[0].amount_raw += 1;
            }
            prepared.observe_changed(format, &frame, &tracker.observe(&frame, format))?;
            Ok(Work {
                row_visits: prepared.work.row_visits - before.row_visits,
                history_visits: prepared.work.history_visits - before.history_visits,
                ..Default::default()
            })
        })
        .unwrap(),
    );
}

#[test]
fn unchanged_supplying_reads_preserve_dates_and_diagnostics_across_resume_and_append() {
    let first = slot_hour(TableId::Stockpiles);
    let mut initial = stockpile_frame(first, true, &[(10, 100)], Some(&[(10, 100, 0, 0)]));
    initial.metrics.table_ns.fill(0);
    *initial.tables[TableId::LogisticsEquipment.index()] =
        TableView::Complete(TableRows::LogisticsEquipment(vec![LogisticsEquipmentRow {
            country: 11,
            archetype: 1,
            token: 1,
            balance_raw: -5,
            stock_raw: 7,
            available_raw: replay::rows::Recorded::Value(None),
        }]));
    let frames: Vec<_> = (0..=100)
        .map(|offset| {
            let mut frame = initial.clone();
            frame.hour += offset;
            frame
        })
        .collect();
    let flag = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "stockpile-equal-reads".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay(frames.clone())),
        &flag,
    )
    .unwrap();
    let context = LoadContext { content: None };
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut prepared = Prepared::default();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for (index, batch) in frames.chunks(35).enumerate() {
        for frame in batch {
            prepared
                .observe_changed(
                    source.replay_ref().format,
                    frame,
                    &tracker.observe(frame, source.replay_ref().format),
                )
                .unwrap();
        }
        let mut writer = if index == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        tracker.invalidate();
    }
    let params = CountryParams {
        branch_id: "1".into(),
        tag: "GER".into(),
    };
    let oracle = EquipmentStockpile::load(&source, &params, &context, &flag).unwrap();
    assert!(oracle.reads.len() > 1);
    assert!(oracle.logistics_reads.len() > 1);
    let mut actual = read_window(
        &source,
        serde_json::from_value(serde_json::json!({
            "branchId":"1", "tag":"GER", "from":first, "to":first + 100,
        }))
        .unwrap(),
        &context,
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap()
    .unwrap();
    actual.as_object_mut().unwrap().remove("prepared");
    assert_eq!(actual, serde_json::to_value(oracle).unwrap());
}

#[test]
fn legacy_accepts_nonempty_token_changes_and_classifies_earlier_stock_by_the_final_token() {
    let first = slot_hour(TableId::Stockpiles);
    let initial = stockpile_frame(first, true, &[(10, 500_000)], Some(&[]));
    let mut changed = stockpile_frame(first + 1, true, &[(10, 700_000)], Some(&[]));
    changed.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *changed.tables[TableId::VariantDefinitions.index()]
    {
        rows.iter_mut()
            .find(|row| row.id == 10)
            .unwrap()
            .equipment_token = 2;
    }
    let scanned = scan(
        &replay(vec![initial, changed]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.variants.token(10), "destroyer_1");
    let names = &scanned.variants.versions[&10];
    assert_eq!(names[0].raw.as_ref().unwrap().token, "infantry_equipment_1");
    assert_eq!(names[1].raw.as_ref().unwrap().token, "destroyer_1");
    assert_eq!(
        scanned.stock.runs[&10][0].amount_raw.as_deref(),
        Some("500000")
    );
}

fn frames() -> Vec<DecodedFrame> {
    let first = slot_hour(TableId::Stockpiles);
    (0..12)
        .filter(|step| *step != 6)
        .map(|step| {
            let held = if step == 3 {
                Vec::new()
            } else {
                vec![(10, 100_000 + i64::from(step)), (20, 0)]
            };
            let mut frame = stockpile_frame(
                first + step,
                step % 2 == 0,
                &held,
                Some(&[(10, 360_045, if step == 2 { 0 } else { 2_250_000 }, 0)]),
            );
            frame.metrics.table_ns[TableId::Stockpiles.index()] = u64::from(step % 2 == 0);
            frame.metrics.table_ns[TableId::ProductionLines.index()] = u64::from(step % 2 == 0);
            frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = u64::from(step % 2 == 0);
            frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
            *frame.tables[TableId::LogisticsEquipment.index()] =
                TableView::Complete(TableRows::LogisticsEquipment(if step == 8 {
                    vec![]
                } else {
                    vec![LogisticsEquipmentRow {
                        country: 11,
                        archetype: 1,
                        token: 1,
                        balance_raw: -i64::from(step),
                        stock_raw: 100_000,
                        available_raw: replay::rows::Recorded::Value(if step == 4 {
                            None
                        } else {
                            Some(110_000)
                        }),
                    }]
                }));
            if step == 5 {
                *frame.tables[TableId::Stockpiles.index()] = TableView::Unavailable {
                    reason: "stock-only refusal".into(),
                };
            }
            if step >= 10
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut().find(|row| row.tag == b"GER").unwrap().id = 9_001;
            }
            if step == 4 {
                *frame.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
                    reason: "names-only refusal".into(),
                };
            }
            frame
        })
        .collect()
}

#[test]
fn canonical_reply_matches_legacy_after_two_commits_resume_append_and_gaps() {
    let frames = frames();
    let recording = replay(frames.clone());
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "stockpile-prepared".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(recording),
        &AtomicBool::new(false),
    )
    .unwrap();
    let cancelled = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut prepared = Prepared::default();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for (index, batch) in frames.chunks(4).enumerate() {
        for frame in batch {
            prepared
                .observe_changed(
                    source.replay_ref().format,
                    frame,
                    &tracker.observe(frame, source.replay_ref().format),
                )
                .unwrap();
        }
        let mut writer = if index < 2 {
            fixture.writer()
        } else {
            fixture.append_writer()
        };
        prepared.flush(&mut writer, 1, &cancelled).unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        tracker.invalidate();
    }
    let params = CountryParams {
        branch_id: "1".into(),
        tag: "GER".into(),
    };
    let legacy = EquipmentStockpile::load(&source, &params, &context, &cancelled).unwrap();
    let request = serde_json::from_value(serde_json::json!({"branchId":"1","tag":"GER","from":frames[0].hour,"to":frames.last().unwrap().hour})).unwrap();
    let mut actual = read_window(
        &source,
        request,
        &context,
        &fixture.store,
        &fixture.key,
        &cancelled,
    )
    .unwrap()
    .unwrap();
    actual.as_object_mut().unwrap().remove("prepared");
    assert_eq!(actual, serde_json::to_value(legacy).unwrap());

    // A read in the middle of an unchanged supplying interval keeps both whole
    // endpoints, rather than adopting the publication/viewport boundaries.
    let actor = prepared.numbers.keys[&series::field_key("GER", "stock:20")];
    let mut present = Reader::new(
        &fixture.store,
        &fixture.key,
        "stockpile-1-stock-present".into(),
        &cancelled,
    )
    .unwrap();
    let mut values = Reader::new(
        &fixture.store,
        &fixture.key,
        "stockpile-1-stock".into(),
        &cancelled,
    )
    .unwrap();
    let point = amount_runs(
        &mut present,
        &mut values,
        actor,
        frames[1].hour,
        frames[1].hour,
    )
    .unwrap();
    assert_eq!(point[0].first_hour, frames[0].hour);
    assert_eq!(point[0].last_hour, frames[4].hour);
    assert_eq!(point[0].amount_raw.as_deref(), Some("0"));
}

#[test]
fn unknown_presence_zero_absence_and_name_refusals_survive_resume() {
    let first = slot_hour(TableId::Stockpiles);
    let flag = AtomicBool::new(false);
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut prepared = Prepared::default();
    let unknown = stockpile_frame(first, true, &[(10, 0)], Some(&[(10, 100_000, 0, 0)]));
    prepared
        .observe(replay(vec![unknown.clone()]).format, &unknown)
        .unwrap();
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    let mut empty = stockpile_frame(first + 1, true, &[], Some(&[]));
    empty.metrics.table_ns[TableId::Stockpiles.index()] = 1;
    empty.metrics.table_ns[TableId::ProductionLines.index()] = 1;
    prepared
        .observe(replay(vec![empty.clone()]).format, &empty)
        .unwrap();
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let actor = prepared.numbers.keys[&series::field_key("GER", "daily:10")];
    let mut present = Reader::new(
        &fixture.store,
        &fixture.key,
        "stockpile-1-daily-present".into(),
        &flag,
    )
    .unwrap();
    let mut amounts = Reader::new(
        &fixture.store,
        &fixture.key,
        "stockpile-1-daily".into(),
        &flag,
    )
    .unwrap();
    assert_eq!(
        amount_runs(&mut present, &mut amounts, actor, first, first).unwrap()[0].amount_raw,
        None
    );
    assert!(
        amount_runs(&mut present, &mut amounts, actor, first + 1, first + 1)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        amounts.value_at(actor, first + 1).unwrap().unwrap().value,
        0
    );
    let raw = equipment::RawVariant {
        refusals: vec![
            ("token".into(), "unread dictionary".into()),
            ("design".into(), "unread modules".into()),
        ],
        ..Default::default()
    };
    let value = serde_json::to_value(&raw).unwrap();
    assert_eq!(
        serde_json::from_value::<equipment::RawVariant>(value.clone()).unwrap(),
        raw
    );
    let mut invalid = value;
    invalid["refusals"] = serde_json::json!([["invented operand", "unread"]]);
    assert!(serde_json::from_value::<equipment::RawVariant>(invalid).is_err());
}

#[test]
fn naming_pages_keep_the_incoming_version_without_counting_an_extra_change() {
    let first = slot_hour(TableId::Stockpiles);
    let frames: Vec<_> = (0..3)
        .map(|step| {
            let mut frame = stockpile_frame(first + step, true, &[(10, 100_000)], Some(&[]));
            frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
            if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
                &mut *frame.tables[TableId::VariantDefinitions.index()]
            {
                rows.iter_mut()
                    .find(|row| row.id == 10)
                    .unwrap()
                    .display_name = format!("synthetic-name-{step}").into_bytes();
            }
            frame
        })
        .collect();
    let flag = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "stockpile-names".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay(frames.clone())),
        &flag,
    )
    .unwrap();
    // Literal display names require the same recorded/viewer language context
    // as the product; otherwise all three labels are refused and coalesced.
    let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[]);
    let content = dashboard_core::config::ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.join("user"),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    dashboard_base::catalogue::set_test_catalogue(&source, &content, Arc::new(catalogue));
    let context = LoadContext {
        content: Some(content),
    };
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut prepared = Prepared::default();
    for frame in &frames {
        prepared.observe(source.replay_ref().format, frame).unwrap();
    }
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    for cursor in 0..3 {
        let request=serde_json::from_value(serde_json::json!({"branchId":"1","tag":"GER","from":first,"to":first+2,"kind":"names-page","variant":"10","cursor":cursor.to_string(),"limit":1})).unwrap();
        let page = read_window(
            &source,
            request,
            &context,
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap()
        .unwrap();
        assert_eq!(page["first"], cursor.to_string());
        assert_eq!(page["total"], "3");
        assert_eq!(page["rows"].as_array().unwrap().len(), 1);
        assert_eq!(page["rows"][0]["firstHour"], first + cursor);
        assert_eq!(page["rows"][0]["name"], format!("synthetic-name-{cursor}"));
        assert_eq!(page["incoming"]["firstHour"], first);
        assert_eq!(page["incoming"]["name"], "synthetic-name-0");
    }
}

#[test]
fn native_alias_summary_keeps_explicit_unknown_rows_distinct_from_absent_zero() {
    let display = |first, last| {
        let mut unknown = dashboard_base::prepared_chart::ChartBucket::hour(first, -1.0, true);
        unknown.last_hour = last;
        let presence = dashboard_base::prepared_chart::ChartSummary {
            approximate: Vec::new(),
            from: 0.0,
            to: 3.0,
            width: 1,
            buckets: vec![Some(unknown)],
            runs: None,
        };
        let value = dashboard_base::prepared_chart::ChartSummary {
            buckets: vec![None],
            ..presence.clone()
        };
        serde_json::json!({"prepared":StockpileDisplay{from:0.0,to:3.0,stock_recorded:true,lines_recorded:false,balance_recorded:false,ai_recorded:false,
            operands:vec![StockpileOperandSummary{field:"stock".into(),id:"1".into(),key:"1".into(),values:value,presence:presence.clone(),samples:vec![Default::default()],row_samples:vec![[Some(-1);4]]}],groups:vec![],coverage:vec![presence],nonzero_logistics:vec![],choices:BTreeMap::new(),designs:BTreeMap::new()}})
    };
    let joined = alias_display(&[display(0, 1), display(2, 3)])
        .unwrap()
        .unwrap();
    assert_eq!(joined.operands[0].row_samples[0], [Some(-1); 4]);
    assert!(joined.operands[0].values.buckets[0].is_none());
}

#[test]
#[ignore = "lead-owned storage measurement: 5000 countries over one recorded year"]
fn measured_5000_country_year_stockpile_bytes() {
    use dashboard_core::test_fixtures::{scale_frame, scale_tag};
    use replay::rows::{ProductionLineRow, StockpileRow};
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let format = replay(vec![scale_frame(1, 0, 1)]).format;
    let mut prepared = Prepared::default();
    let started = std::time::Instant::now();
    let mut bytes = 0_u64;
    for hour in 0..365 * 24 {
        let mut frame = scale_frame(1, hour, 5_000);
        *frame.tables[TableId::Stockpiles.index()] = TableView::Complete(TableRows::Stockpiles(
            (0..5_000)
                .map(|index| StockpileRow {
                    country: 100_000 + index,
                    ordinal: 0,
                    variant_handle: 10,
                    amount_raw: i64::from(index + hour as u32 / 24) * 100_000,
                })
                .collect(),
        ));
        *frame.tables[TableId::ProductionLines.index()] =
            TableView::Complete(TableRows::ProductionLines(
                (0..5_000)
                    .map(|index| ProductionLineRow {
                        handle: u64::from(index) + 1,
                        country: 100_000 + index,
                        variant: 10,
                        cost_raw: 50_000,
                        speed_raw: 100_000,
                        kind: engine::cequipment_production_line::KIND_MILITARY,
                        ..Default::default()
                    })
                    .collect(),
            ));
        frame.metrics.table_ns[TableId::Stockpiles.index()] = u64::from(hour % 24 == 0);
        frame.metrics.table_ns[TableId::ProductionLines.index()] = u64::from(hour % 24 == 0);
        prepared.observe(format, &frame).unwrap();
        if (hour + 1) % (30 * 24) == 0 || hour == 365 * 24 - 1 {
            let mut writer = fixture.writer();
            prepared.flush(&mut writer, 1, &flag).unwrap();
            bytes = u64::try_from(i128::from(bytes) + writer.payload_bytes_change()).unwrap();
            writer.commit().unwrap();
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        }
    }
    assert_eq!(prepared.holders.len(), 5_000);
    assert!(
        prepared
            .holders
            .contains_key(std::str::from_utf8(&scale_tag(4_999)).unwrap())
    );
    let checkpoint = serde_json::to_vec(&prepared).unwrap().len();
    eprintln!(
        "stockpile countries=5000 hours=8760 payload_bytes={bytes} bytes_per_country_year={} checkpoint_bytes={checkpoint} elapsed_ms={}",
        (bytes as f64 + checkpoint as f64) / 5_000.0,
        started.elapsed().as_millis()
    );
}

#[test]
#[ignore = "lead-owned real-recording storage and token-stability measurement"]
fn measured_real_recording_stockpile_bytes_and_token_stability() {
    use replay::container::ReplayStream;
    use std::{io::Write, path::PathBuf, time::Instant};
    let required =
        |name: &str| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("Set {name}")));
    let root = required("BATTLE_PLAN_STOCKPILE_PROFILE_DIR");
    assert!(
        root.is_absolute()
            && !root
                .components()
                .any(|part| part == std::path::Component::ParentDir)
    );
    assert_eq!(required("LOCALAPPDATA"), root.join("localappdata"));
    assert_eq!(required("APPDATA"), root.join("appdata"));
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    assert!(
        !root.starts_with(repository),
        "profile output must stay outside Git"
    );
    for path in [required("LOCALAPPDATA"), required("APPDATA")] {
        std::fs::create_dir_all(&path).unwrap();
        assert!(path.canonicalize().unwrap().starts_with(&root));
    }
    engine::executable_tables::initialize_for_tests().unwrap();
    let recording = required("BATTLE_PLAN_STOCKPILE_PROFILE_RECORDING");
    let flag = AtomicBool::new(false);
    let source = Source::open(
        dashboard_base::source::SourceKey {
            source_id: "stockpile-profile".into(),
            revision: "1".into(),
        },
        recording.clone(),
        &flag,
    )
    .unwrap();
    let options = dashboard_core::config::ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: required("BATTLE_PLAN_GAME_DIR"),
            user_dir: required("BATTLE_PLAN_USER_DIR"),
            cache_dir: root.join("content-cache"),
        },
        allow_unverified: false,
    };
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(uuid::Uuid::new_v4()).unwrap();
    let mut writer = Some(store.replace(key.clone()).unwrap());
    let mut branches = BTreeMap::<u64, Prepared>::new();
    let mut tokens = BTreeMap::<(u64, u64), String>::new();
    let mut countries = BTreeSet::new();
    let mut hours = BTreeMap::<u64, u64>::new();
    let mut country_hours = 0_u64;
    let mut token_changes = 0;
    let mut bytes = 0_u64;
    let mut frame_ns = 0_u128;
    let output = root.join("stockpile-profile.jsonl");
    let mut log = io::BufWriter::new(std::fs::File::create(&output).unwrap());
    let mut stream = ReplayStream::new(&TableId::ALL).unwrap();
    let started = Instant::now();
    while stream.next_block(source.replay_ref(), |frame| {
        let prepared = branches.entry(frame.branch).or_default();
        prepared.bind_content(&Prepared::content(&source, Some(&options)));
        let observed = Instant::now();
        prepared.observe(source.replay_ref().format, frame)?;
        frame_ns += observed.elapsed().as_nanos();
        *hours.entry(frame.branch).or_default() += 1;
        country_hours += frame.complete::<tables::Countries>().map_or(0, |rows| rows.len() as u64);
        countries.extend(prepared.holders.keys().map(|tag| (frame.branch, tag.clone())));
        for &id in prepared.names.versions.keys() {
            let token = prepared.names.token(id);
            if token.is_empty() { continue; }
            if let Some(previous) = tokens.insert((frame.branch, id), token.to_owned()).filter(|previous| previous != token) {
                token_changes += 1;
                serde_json::to_writer(&mut log, &serde_json::json!({"branch": frame.branch.to_string(), "hour":frame.hour, "variant":id.to_string(), "oldToken":previous, "newToken":token}))?;
                log.write_all(b"\n")?;
            }
        }
        Ok(())
    }).unwrap() {
        if branches.values().map(Prepared::bytes).sum::<usize>() >= 64 * 1024 * 1024 {
            let mut commit = writer.take().unwrap();
            for (&branch, prepared) in &mut branches { prepared.flush(&mut commit, branch, &flag).unwrap(); }
            bytes = u64::try_from(i128::from(bytes) + commit.payload_bytes_change()).unwrap(); commit.commit().unwrap();
            branches = serde_json::from_slice(&serde_json::to_vec(&branches).unwrap()).unwrap();
            writer = Some(store.extend(&key).unwrap());
        }
    }
    let mut commit = writer.take().unwrap();
    for (&branch, prepared) in &mut branches {
        prepared.flush(&mut commit, branch, &flag).unwrap();
    }
    bytes = u64::try_from(i128::from(bytes) + commit.payload_bytes_change()).unwrap();
    commit.commit().unwrap();
    let context = LoadContext {
        content: Some(options),
    };
    let mut query_timings = Vec::new();
    for (&branch, prepared) in &branches {
        let Some(tag) = prepared
            .holders
            .keys()
            .find(|tag| tag.as_str() == "ENG")
            .or_else(|| prepared.holders.keys().next())
        else {
            continue;
        };
        let (first, last) = prepared.choices.metadata().bounds();
        let country = CountryParams {
            branch_id: branch.to_string(),
            tag: tag.clone(),
        };
        let oracle_started = Instant::now();
        let legacy = EquipmentStockpile::load(&source, &country, &context, &flag).unwrap();
        let legacy_ms = oracle_started.elapsed().as_millis();
        for (kind, from, to, width) in [
            ("exact-branch", first, last, None),
            ("date", last, last, Some(1)),
            ("dense", first, last, Some(8)),
        ] {
            let request=serde_json::from_value(serde_json::json!({"branchId":branch.to_string(),"tag":tag,"from":from,"to":to,"width":width,"groups":prepared.holders[tag].groups.keys().collect::<Vec<_>>()})).unwrap();
            let query_started = Instant::now();
            let mut result = read_window(&source, request, &context, &store, &key, &flag)
                .unwrap()
                .unwrap();
            let elapsed = query_started.elapsed().as_micros();
            let reply_bytes = serde_json::to_vec(&result).unwrap().len();
            if kind == "exact-branch" {
                result.as_object_mut().unwrap().remove("prepared");
                assert_eq!(
                    result,
                    serde_json::to_value(&legacy).unwrap(),
                    "real canonical stockpile reply differs"
                );
            }
            query_timings.push(serde_json::json!({"branch":branch.to_string(),"tag":tag,"kind":kind,"micros":elapsed.to_string(),"replyBytes":reply_bytes,"legacyMillis":legacy_ms.to_string()}));
        }
    }
    let checkpoint = serde_json::to_vec(&branches).unwrap().len();
    let total_bytes = bytes + checkpoint as u64;
    let report = serde_json::json!({"countries": countries.len(), "branchHours": hours, "tokenChanges":token_changes,
        "observedCountryHours":country_hours,"queries":query_timings, "seriesAndCatalogueBytes":bytes,"totalPreparedBytes":total_bytes, "bytesPerCountryYear":total_bytes as f64 * (365 * 24) as f64 / country_hours as f64,
        "checkpointBytes":checkpoint, "frameNanoseconds":frame_ns.to_string(), "elapsedMs":started.elapsed().as_millis().to_string()});
    serde_json::to_writer(&mut log, &report).unwrap();
    log.write_all(b"\n").unwrap();
    log.flush().unwrap();
    eprintln!("stockpile profile {}: {report}", output.display());
    store.clear().unwrap();
}

/// The recording codec: each input hour is encoded and decoded as the
/// preparation pipeline reads it, with its recorded updates.
struct Recorded {
    encoder: replay::frame::FrameEncoder,
    decoder: replay::frame::FrameDecoder,
}
impl Recorded {
    fn new() -> Self {
        Self {
            encoder: replay::frame::FrameEncoder::new(),
            decoder: replay::frame::FrameDecoder::new(),
        }
    }
    fn next(
        &mut self,
        input: &DecodedFrame,
        keyframe: bool,
    ) -> (DecodedFrame, Arc<replay::frame::Frame>) {
        let wire = self.encoder.encode(input.clone(), keyframe).unwrap();
        let decoded = self.decoder.advance(&wire).unwrap().clone();
        (decoded, Arc::new(wire))
    }
}

fn stock_rows(frame: &mut DecodedFrame) -> &mut Vec<replay::rows::StockpileRow> {
    match &mut *frame.tables[TableId::Stockpiles.index()] {
        TableView::Complete(TableRows::Stockpiles(rows)) => rows,
        _ => unreachable!("fixture stock is complete"),
    }
}
fn line_rows(frame: &mut DecodedFrame) -> &mut Vec<replay::rows::ProductionLineRow> {
    match &mut *frame.tables[TableId::ProductionLines.index()] {
        TableView::Complete(TableRows::ProductionLines(rows)) => rows,
        _ => unreachable!("fixture lines are complete"),
    }
}
fn logistics_rows(frame: &mut DecodedFrame) -> &mut Vec<LogisticsEquipmentRow> {
    match &mut *frame.tables[TableId::LogisticsEquipment.index()] {
        TableView::Complete(TableRows::LogisticsEquipment(rows)) => rows,
        _ => unreachable!("fixture logistics are complete"),
    }
}
fn stock_row(
    frame: &mut DecodedFrame,
    country: u32,
    ordinal: u32,
) -> &mut replay::rows::StockpileRow {
    stock_rows(frame)
        .iter_mut()
        .find(|row| (row.country, row.ordinal) == (country, ordinal))
        .expect("fixture stock row")
}
fn line_row(frame: &mut DecodedFrame, handle: u64) -> &mut replay::rows::ProductionLineRow {
    line_rows(frame)
        .iter_mut()
        .find(|row| row.handle == handle)
        .expect("fixture line")
}
fn logistics_row(
    frame: &mut DecodedFrame,
    country: u32,
    archetype: u32,
) -> &mut LogisticsEquipmentRow {
    logistics_rows(frame)
        .iter_mut()
        .find(|row| (row.country, row.archetype) == (country, archetype))
        .expect("fixture logistics row")
}
fn country_row(frame: &mut DecodedFrame, id: u32) -> &mut replay::rows::CountryRow {
    match &mut *frame.tables[TableId::Countries.index()] {
        TableView::Complete(TableRows::Countries(rows)) => rows
            .iter_mut()
            .find(|row| row.id == id)
            .expect("fixture country"),
        _ => unreachable!("fixture countries are complete"),
    }
}
fn variant_row(frame: &mut DecodedFrame, id: u64) -> &mut replay::rows::VariantDefinitionRow {
    match &mut *frame.tables[TableId::VariantDefinitions.index()] {
        TableView::Complete(TableRows::VariantDefinitions(rows)) => rows
            .iter_mut()
            .find(|row| row.id == id)
            .expect("fixture variant"),
        _ => unreachable!("fixture variants are complete"),
    }
}
fn logistics(
    country: u32,
    archetype: u32,
    token: u32,
    balance_raw: i64,
    stock_raw: i64,
) -> LogisticsEquipmentRow {
    LogisticsEquipmentRow {
        country,
        archetype,
        token,
        balance_raw,
        stock_raw,
        available_raw: replay::rows::Recorded::Value(Some(stock_raw.max(0))),
    }
}

/// `count` holders (native references 100,000 and up), each holding two
/// variants, making one of them on a line, and with one logistics row; every
/// hour reads the stockpile and the logistics record.
fn counted_frame(hour: i32, count: u32) -> DecodedFrame {
    use replay::rows::{LexicalTokenRow, ProductionLineRow, StockpileRow};
    let mut frame = dashboard_base::test_fixtures::scale_frame(1, hour, count);
    *frame.tables[TableId::Stockpiles.index()] = TableView::Complete(TableRows::Stockpiles(
        (0..count)
            .flat_map(|index| {
                [10, 20]
                    .into_iter()
                    .zip(0..)
                    .map(move |(variant_handle, ordinal)| StockpileRow {
                        country: 100_000 + index,
                        ordinal,
                        variant_handle,
                        amount_raw: i64::from(index + 1) * 100_000,
                    })
            })
            .collect(),
    ));
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(
            (0..count)
                .map(|index| ProductionLineRow {
                    handle: u64::from(index) + 1,
                    country: 100_000 + index,
                    kind: engine::cequipment_production_line::KIND_MILITARY,
                    variant: 10,
                    speed_raw: 100_000,
                    cost_raw: 50_000,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::LogisticsEquipment.index()] =
        TableView::Complete(TableRows::LogisticsEquipment(
            (0..count)
                .map(|index| logistics(100_000 + index, 1, 1, -5, 7))
                .collect(),
        ));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1,
            literal: b"infantry_equipment_1".to_vec(),
        }]));
    frame.metrics.table_ns[TableId::Stockpiles.index()] = 1;
    frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = 1;
    frame
}

#[test]
fn holder_catch_up_steps_one_span_and_keeps_hourly_saved_state() {
    let first = slot_hour(TableId::Stockpiles);
    let frame = stockpile_frame(first, true, &[(10, 100), (20, 0)], Some(&[]));
    let format = replay::format::FormatVersion::CURRENT;
    let input = FrameRows::new(&frame);
    for hours in [10, 1_000] {
        let mut hourly = ScanState::default();
        hourly.observe(format, &input, input.identities.get("GER").copied());
        let mut spans = hourly.clone();
        for hour in first + 1..=first + hours {
            hourly.advance(hour);
        }
        spans.catch_up(Some(first + hours));
        hourly.seal();
        spans.seal();
        assert_eq!(
            serde_json::to_vec(&spans).unwrap(),
            serde_json::to_vec(&hourly).unwrap()
        );
        assert_eq!(spans.advances, 1, "a holder steps the entire span once");
        let mut restored: ScanState =
            serde_json::from_slice(&serde_json::to_vec(&spans).unwrap()).unwrap();
        restored.catch_up(Some(first + hours));
        assert_eq!(restored.advances, 0, "a restored holder is already current");
    }
}

#[test]
fn holder_catch_up_keeps_identity_changes_absence_returns_and_recording_gaps() {
    let first = slot_hour(TableId::Stockpiles);
    let mut frame = stockpile_frame(first, true, &[(10, 100), (20, 0)], Some(&[]));
    let format = replay::format::FormatVersion::CURRENT;
    let input = FrameRows::new(&frame);
    let original = input.identities["GER"];
    let replacement = (original.0, original.1 + 1);
    let returned = (original.0, original.1 + 2);
    let mut hourly = ScanState::default();
    hourly.observe(format, &input, Some(original));
    let mut spans = hourly.clone();
    for (through, at, identity, continues) in [
        (4, 5, Some(replacement), false),
        (7, 8, Some(replacement), true),
        (10, 11, None, false),
        (13, 14, Some(returned), false),
        (16, 19, Some(returned), false),
        (21, 22, Some((original.0 + 1, original.1 + 3)), false),
    ] {
        if let Some(from) = hourly.advanced {
            for hour in from + 1..=first + through {
                hourly.advance(hour);
            }
        }
        spans.catch_up(Some(first + through));
        for state in [&mut hourly, &mut spans] {
            state.seal();
        }
        assert_eq!(
            serde_json::to_vec(&hourly).unwrap(),
            serde_json::to_vec(&spans).unwrap(),
            "catch-up through {through}"
        );
        let mut track = spans.track.clone();
        assert_eq!(
            track.step(identity, first + at),
            continues,
            "holder at {at}"
        );
        frame.hour = first + at;
        frame.metrics.table_ns.fill(0);
        let input = FrameRows::new(&frame);
        for state in [&mut hourly, &mut spans] {
            state.observe(format, &input, identity);
            state.seal();
        }
        assert_eq!(
            serde_json::to_vec(&hourly).unwrap(),
            serde_json::to_vec(&spans).unwrap(),
            "observation at {at}"
        );
    }
}

fn assert_changed_rows_observe_only_their_holder(count: u32) {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests::Work};
    let format = replay::format::FormatVersion::CURRENT;
    let mut recorded = Recorded::new();
    let mut tracker = FrameChangeTracker::new();
    let mut step = |prepared: &mut Prepared, frame: &DecodedFrame| {
        let before = (prepared.work, prepared.row_hours);
        let (decoded, wire) = recorded.next(frame, false);
        let changes = tracker.observe_updates(&decoded, wire, format);
        prepared
            .observe_changed(format, &decoded, &changes)
            .unwrap();
        let work = Work {
            row_visits: prepared.work.row_visits - before.0.row_visits,
            history_visits: prepared.work.history_visits - before.0.history_visits,
            ..Default::default()
        };
        (work, prepared.row_hours - before.1)
    };
    let mut prepared = Prepared::default();
    let mut frame = counted_frame(100, count);
    let (first, _) = step(&mut prepared, &frame);
    assert_eq!(
        first.history_visits, count as usize,
        "the first hour reads every holder"
    );
    let holder = |index: u32| 100_000 + index;

    // A read of unchanged rows repeats every holder's read without visiting it.
    frame.hour = 101;
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1, "the hour is observed from its changed rows");
    assert_eq!((work.row_visits, work.history_visits), (0, 0));

    frame.hour = 102;
    stock_row(&mut frame, holder(count / 2), 1).amount_raw += 1;
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1);
    assert_eq!(work.history_visits, 1, "only its holder is observed again");
    assert_eq!(
        work.row_visits, 4,
        "only its holder's two stock rows, line and logistics row are read"
    );

    // A line field and a country field no holder reads.
    frame.hour = 103;
    line_row(&mut frame, u64::from(count / 3) + 1).efficiency_raw += 7;
    country_row(&mut frame, holder(count / 4)).political_power_raw = Some(7);
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1);
    assert_eq!(
        (work.row_visits, work.history_visits),
        (0, 0),
        "nothing a holder reads changed"
    );

    frame.hour = 104;
    line_row(&mut frame, u64::from(count / 5) + 1).speed_raw += 1;
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1);
    assert_eq!((work.row_visits, work.history_visits), (4, 1));

    frame.hour = 105;
    logistics_row(&mut frame, holder(count / 6), 1).balance_raw -= 1;
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1);
    assert_eq!((work.row_visits, work.history_visits), (4, 1));

    // A changed row an hour does not read waits for its holder's next read.
    frame.hour = 106;
    frame.metrics.table_ns.fill(0);
    stock_row(&mut frame, holder(count / 7), 0).amount_raw += 1;
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1);
    assert_eq!((work.row_visits, work.history_visits), (0, 0));
    frame.hour = 107;
    frame.metrics.table_ns[TableId::Stockpiles.index()] = 1;
    frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = 1;
    let (work, rows) = step(&mut prepared, &frame);
    assert_eq!(rows, 1);
    assert_eq!((work.row_visits, work.history_visits), (4, 1));
}

#[test]
fn stockpile_hour_changing_one_holder_observes_only_that_holder() {
    assert_changed_rows_observe_only_their_holder(200);
}

#[test]
#[cfg(feature = "slow-tests")]
fn stockpile_hour_changing_one_of_5000_holders_observes_only_that_holder() {
    assert_changed_rows_observe_only_their_holder(5000);
}

#[test]
fn stockpile_row_masks_name_every_field_they_read() {
    assert_eq!(HOLDER_FIELDS.count_ones(), 2);
    assert_eq!(LINE_FIELDS.count_ones(), 5);
    let efficiency = (0..64)
        .position(|field| TableId::ProductionLines.field_name(field) == Some("efficiency_raw"))
        .unwrap();
    assert_eq!(
        *LINE_FIELDS & (1 << efficiency),
        0,
        "no holder reads a line's efficiency"
    );
}

/// Germany (canonical slot 7) through links 11 and 9001, Italy (21, slot 8),
/// France (31, slot 9) and Sweden (41, slot 10), each through one link.
fn holders_frame(hour: i32) -> DecodedFrame {
    use engine::cequipment_production_line::{KIND_MILITARY, KIND_NAVAL};
    use replay::rows::{
        CountryLinkRow, CountryRow, LexicalTokenRow, ProductionLineRow, StockpileRow,
        VariantDefinitionRow,
    };
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        [
            (21, 8, "ITA"),
            (31, 9, "FRA"),
            (41, 10, "SWE"),
            (9001, 7, "GER"),
        ]
        .into_iter()
        .map(|(id, canonical_slot, tag)| CountryRow {
            id,
            canonical_slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        })
        .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(11, 7), (21, 8), (31, 9), (41, 10), (9001, 7)]
            .into_iter()
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    let stock = |country, ordinal, variant_handle, amount_raw| StockpileRow {
        country,
        ordinal,
        variant_handle,
        amount_raw,
    };
    *frame.tables[TableId::Stockpiles.index()] = TableView::Complete(TableRows::Stockpiles(vec![
        stock(11, 0, 10, 100_000),
        stock(11, 1, 20, 5),
        stock(21, 0, 10, 999),
        stock(31, 0, 20, 7_000),
        stock(9001, 0, 30, 3),
    ]));
    let line = |handle, country, variant, speed_raw, cost_raw, kind| ProductionLineRow {
        handle,
        country,
        kind,
        variant,
        speed_raw,
        cost_raw,
        ..Default::default()
    };
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(vec![
            line(1, 9001, 10, 360_045, 2_250_000, KIND_MILITARY),
            line(2, 21, 10, 100_000, 50_000, KIND_MILITARY),
            line(3, 11, 20, 100_000, 0, KIND_MILITARY),
            line(4, 31, 30, 50_000, 10_000, KIND_NAVAL),
        ]));
    *frame.tables[TableId::LogisticsEquipment.index()] =
        TableView::Complete(TableRows::LogisticsEquipment(vec![
            logistics(21, 1, 1, -5, 7),
            logistics(9001, 1, 1, 3, 9),
            logistics(9001, 2, 2, 0, 0),
        ]));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [(1, "infantry_equipment_1"), (2, "destroyer_1")]
            .into_iter()
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
    ));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            [(10, 1, ""), (20, 1, "Gewehr 43"), (30, 2, "")]
                .into_iter()
                .map(|(id, equipment_token, name)| VariantDefinitionRow {
                    id,
                    equipment_token,
                    creator_link: 9001,
                    display_name: name.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

/// A prepared window of `tag` through `to`, with the given width and groups.
fn window(
    source: &Source,
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    tag: &str,
    to: i32,
    width: Option<u32>,
    groups: &[String],
) -> serde_json::Value {
    let mut request = serde_json::json!({
        "branchId": "1", "tag": tag, "from": 100, "to": to, "groups": groups,
    });
    if let Some(width) = width {
        request["width"] = width.into();
    }
    read_window(
        source,
        serde_json::from_value(request).unwrap(),
        &LoadContext { content: None },
        &fixture.store,
        &fixture.key,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap()
}

/// What an observer continues from. A closed run set's frontier is never read
/// again: its next read starts anew, and carrying or advancing it does nothing,
/// so a holder stepped through hours it was not visited may hold any.
fn continuation(observer: &Prepared) -> serde_json::Value {
    fn closed(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                if fields.get("carried") == Some(&serde_json::Value::Bool(false)) {
                    fields.insert("frontier".into(), serde_json::Value::Null);
                }
                fields.values_mut().for_each(closed);
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(closed),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(observer).unwrap();
    closed(&mut value);
    value
}

#[test]
fn stockpile_rows_observation_matches_whole_tables_and_legacy_across_keyframe_gap_and_resume() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use engine::cequipment_production_line::KIND_NAVAL;
    use replay::rows::{ProductionLineRow, StockpileRow};
    const TAGS: [&str; 5] = ["GER", "ITA", "FRA", "SWE", "NOR"];
    const PUBLISHED: [i32; 4] = [108, 121, 135, 150];
    let mut frame = holders_frame(100);
    let mut inputs = Vec::new();
    let mut stashed = None;
    for hour in 100..=150 {
        frame.hour = hour;
        frame.metrics.table_ns.fill(0);
        frame.metrics.table_ns[TableId::Countries.index()] = 1;
        frame.metrics.table_ns[TableId::Stockpiles.index()] = u64::from(hour % 4 == 0);
        frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = u64::from(hour % 6 == 0);
        frame.metrics.table_ns[TableId::VariantDefinitions.index()] = u64::from(hour % 10 == 0);
        match hour {
            // Changed before its read, read at the next one.
            101 | 145 => stock_row(&mut frame, 11, 0).amount_raw += 1,
            // Fields no holder reads.
            102 | 131 => country_row(&mut frame, 21).political_power_raw = Some(i64::from(hour)),
            103 => line_row(&mut frame, 1).efficiency_raw += 1,
            105 => {
                stock_row(&mut frame, 21, 0).amount_raw += 10;
                line_row(&mut frame, 2).speed_raw += 1_000;
            }
            106 => {
                stock_rows(&mut frame).push(StockpileRow {
                    country: 31,
                    ordinal: 1,
                    variant_handle: 10,
                    amount_raw: 50,
                });
                stock_rows(&mut frame).sort_by_key(|row| (row.country, row.ordinal));
            }
            107 => {
                stock_rows(&mut frame).push(StockpileRow {
                    country: 41,
                    ordinal: 0,
                    variant_handle: 10,
                    amount_raw: 1,
                });
                stock_rows(&mut frame).sort_by_key(|row| (row.country, row.ordinal));
            }
            110 => variant_row(&mut frame, 20).display_name = b"Gewehr 43/1".to_vec(),
            111 => logistics_row(&mut frame, 9001, 1).balance_raw -= 4,
            // A line moves from Germany's other link to Italy.
            112 => line_row(&mut frame, 3).country = 21,
            113 => stock_rows(&mut frame).retain(|row| (row.country, row.ordinal) != (21, 0)),
            // Italy is AI-controlled at this read.
            114 => logistics_rows(&mut frame).retain(|row| row.country != 21),
            115 => stock_row(&mut frame, 31, 0).amount_raw += 1,
            116 => stock_row(&mut frame, 9001, 0).amount_raw += 2,
            117 => {
                line_rows(&mut frame).push(ProductionLineRow {
                    handle: 5,
                    country: 31,
                    variant: 20,
                    speed_raw: 1,
                    cost_raw: 1,
                    ..Default::default()
                });
                line_rows(&mut frame).sort_by_key(|row| row.handle);
            }
            // At its read, variant 10 changes type: its holders change groups.
            118 => variant_row(&mut frame, 10).equipment_token = 2,
            119 => continue,
            121 => {
                stashed = Some(std::mem::take(stock_rows(&mut frame)));
                *frame.tables[TableId::Stockpiles.index()] = TableView::Unavailable {
                    reason: "synthetic missing stock".into(),
                };
            }
            122 => {
                *frame.tables[TableId::Stockpiles.index()] =
                    TableView::Complete(TableRows::Stockpiles(stashed.take().unwrap()));
                stock_row(&mut frame, 11, 1).amount_raw += 1;
            }
            // A holder renamed: the holders change.
            123 => country_row(&mut frame, 41).tag = b"NOR".to_vec(),
            124 => line_rows(&mut frame).retain(|row| row.handle != 2),
            126 => {
                logistics_row(&mut frame, 9001, 2).available_raw =
                    replay::rows::Recorded::Value(None);
            }
            127 => {
                logistics_rows(&mut frame).push(logistics(21, 1, 1, -9, 2));
                logistics_rows(&mut frame).sort_by_key(|row| (row.country, row.archetype));
            }
            // A total that does not fit: its count repeats with every read.
            129 => {
                stock_rows(&mut frame).push(StockpileRow {
                    country: 9001,
                    ordinal: 1,
                    variant_handle: 10,
                    amount_raw: i64::MAX,
                });
                stock_rows(&mut frame).sort_by_key(|row| (row.country, row.ordinal));
            }
            133 => line_row(&mut frame, 1).cost_raw = 0,
            137 => stock_row(&mut frame, 41, 0).amount_raw += 5,
            138 => line_row(&mut frame, 5).kind = KIND_NAVAL,
            140 => stock_rows(&mut frame).retain(|row| row.country != 41),
            _ => {}
        }
        inputs.push((frame.clone(), hour == 115));
    }
    let flag = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "stockpile-rows".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay(
            inputs.iter().map(|(frame, _)| frame.clone()).collect(),
        )),
        &flag,
    )
    .unwrap();
    let format = source.replay_ref().format;
    let fixtures = [
        dashboard_base::prepared::series::tests::Fixture::new(),
        dashboard_base::prepared::series::tests::Fixture::new(),
    ];
    // Observed from changed rows, and from the whole tables every hour.
    let mut observers = [
        Prepared::default(),
        Prepared {
            whole_tables: true,
            ..Default::default()
        },
    ];
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut recorded = Recorded::new();
    for (input, keyframe) in &inputs {
        let (decoded, wire) = recorded.next(input, *keyframe);
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let changes = tracker.observe_updates(&decoded, Arc::clone(&wire), format);
            observer
                .observe_changed(format, &decoded, &changes)
                .unwrap();
        }
        if !PUBLISHED.contains(&decoded.hour) {
            continue;
        }
        for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
            let mut writer = fixture.writer();
            observer.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        dashboard_base::prepared::series::tests::assert_payloads_equal(
            &fixtures[0].payloads(),
            &fixtures[1].payloads(),
            &format!("published through hour {}", decoded.hour),
        );
        assert_eq!(
            continuation(&observers[0]),
            continuation(&observers[1]),
            "continuation through hour {}",
            decoded.hour
        );
        for tag in TAGS {
            let groups: Vec<String> = observers[1]
                .holders
                .get(tag)
                .map(|holder| holder.groups.keys().cloned().collect())
                .unwrap_or_default();
            for (width, groups) in [(None, &[][..]), (Some(4), &[][..]), (Some(4), &groups[..])] {
                assert_eq!(
                    window(&source, &fixtures[0], tag, decoded.hour, width, groups),
                    window(&source, &fixtures[1], tag, decoded.hour, width, groups),
                    "{tag} through hour {} at width {width:?}",
                    decoded.hour
                );
            }
        }
        // Resume both from what they published.
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let (whole_tables, row_hours) = (observer.whole_tables, observer.row_hours);
            *observer = serde_json::from_slice(&serde_json::to_vec(&*observer).unwrap()).unwrap();
            observer.whole_tables = whole_tables;
            observer.row_hours = row_hours;
            tracker.invalidate();
        }
    }
    assert!(
        observers[0].row_hours >= 30,
        "most hours are observed from their changed rows, not {}",
        observers[0].row_hours
    );
    assert_eq!(observers[1].row_hours, 0);
    let context = LoadContext { content: None };
    for tag in TAGS {
        let legacy = EquipmentStockpile::load(
            &source,
            &CountryParams {
                branch_id: "1".into(),
                tag: tag.into(),
            },
            &context,
            &flag,
        )
        .unwrap();
        let mut actual = window(&source, &fixtures[0], tag, 150, None, &[]);
        actual.as_object_mut().unwrap().remove("prepared");
        assert_eq!(actual, serde_json::to_value(legacy).unwrap(), "{tag}");
    }
}
