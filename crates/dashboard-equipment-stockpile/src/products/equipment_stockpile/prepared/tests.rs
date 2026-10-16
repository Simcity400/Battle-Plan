use super::*;
use dashboard_core::test_fixtures::{replay, slot_hour, stockpile_frame};
use replay::{
    frame::TableView,
    rows::{LogisticsEquipmentRow, TableRows},
};

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
