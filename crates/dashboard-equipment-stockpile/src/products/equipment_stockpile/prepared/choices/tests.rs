use super::*;
use dashboard_core::test_fixtures::{replay, stockpile_frame};

// A synthetic authored result already held in the content binding's cache.
fn resolved_content() -> Content {
    let kinds = catalogue::KeyedResults::default();
    kinds
        .get_or_init(&"mod_equipment_1".to_owned(), |_| {
            Ok(Kind {
                key: "mod_equipment".into(),
                domain: Domain::Land,
                requires_available: false,
            })
        })
        .unwrap();
    Content {
        database: Some(Arc::new(Database::default())),
        kinds: Some(Arc::new(kinds)),
    }
}

fn classification_frame(variants: bool) -> DecodedFrame {
    use replay_data::{rows::LogisticsEquipmentRow, rows::TableRows};
    let mut frame = stockpile_frame(
        dashboard_core::test_fixtures::slot_hour(TableId::Stockpiles),
        true,
        &[(10, 100)],
        Some(&[]),
    );
    frame.metrics.table_ns[TableId::Stockpiles.index()] = 1;
    frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = 1;
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![]));
    if !variants {
        *frame.tables[TableId::Stockpiles.index()] =
            TableView::Complete(TableRows::Stockpiles(vec![]));
        *frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![]));
    }
    *frame.tables[TableId::LogisticsEquipment.index()] =
        TableView::Complete(TableRows::LogisticsEquipment(vec![LogisticsEquipmentRow {
            country: 11,
            archetype: 1,
            token: 1,
            balance_raw: -5,
            stock_raw: 7,
            available_raw: replay_data::rows::Recorded::Value(Some(3)),
        }]));
    frame
}

#[test]
fn repeated_reads_reclassify_learned_logistics_and_variant_names() {
    use replay_data::{rows::LexicalTokenRow, rows::TableRows};
    for variants in [false, true] {
        let content = resolved_content();
        let mut frame = classification_frame(variants);
        let first = frame.hour;
        let format = replay(vec![frame.clone()]).format;
        let flag = AtomicBool::new(false);
        let actual = dashboard_base::prepared::series::tests::Fixture::new();
        let expected = dashboard_base::prepared::series::tests::Fixture::new();
        let mut prepared = Prepared::default();
        let mut reference = Prepared::default();
        for (state, fixture) in [(&mut prepared, &actual), (&mut reference, &expected)] {
            state.bind_content(&content);
            state.observe(format, &frame).unwrap();
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        assert_eq!(prepared.choices.completeness["GER"][1..], [false, false]);
        // Restoring discards runtime caches; published groups and continuation
        // state still match preparation that keeps them.
        reference = serde_json::from_slice(&serde_json::to_vec(&reference).unwrap()).unwrap();
        reference.bind_content(&content);
        reference.choices.whole_projection = true;
        frame.hour += 1;
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"mod_equipment_1".to_vec(),
            }]));
        prepared.observe(format, &frame).unwrap();
        reference.observe(format, &frame).unwrap();
        assert_eq!(prepared.holders["GER"].tokens[&1], "mod_equipment_1");
        assert_eq!(prepared.choices.completeness["GER"], [true; 3]);
        let group = series::field_key("type", "mod_equipment");
        assert!(prepared.holders["GER"].groups.contains_key(&group));
        let projected = prepared.numbers.projected;
        let groups = prepared.choices.work.history_visits;
        frame.hour += 1;
        prepared.observe(format, &frame).unwrap();
        reference.observe(format, &frame).unwrap();
        assert_eq!(prepared.numbers.projected, projected);
        assert_eq!(
            prepared.choices.work.history_visits, groups,
            "equal supplying reads project no group"
        );
        for (state, fixture) in [(&mut prepared, &actual), (&mut reference, &expected)] {
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        dashboard_base::prepared::series::tests::assert_payloads_equal(
            &actual.payloads(),
            &expected.payloads(),
            "learned classifications with repeating numeric runs",
        );
        assert_eq!(
            serde_json::to_value(&prepared).unwrap(),
            serde_json::to_value(&reference).unwrap(),
        );
        let keys = prepared.numbers.group_keys("GER", &group);
        for (slot, expected) in [Some(if variants { 100 } else { 0 }), Some(-5), Some(8)]
            .into_iter()
            .enumerate()
        {
            let mut reader = Reader::new(
                &actual.store,
                &actual.key,
                format!("stockpile-1-group-{}", MEASURES[slot]),
                &flag,
            )
            .unwrap();
            for hour in first + 1..=first + 2 {
                assert_eq!(
                    reader
                        .value_at(keys[slot], hour)
                        .unwrap()
                        .map(|run| run.value),
                    expected,
                    "{} at {hour}, variants={variants}",
                    MEASURES[slot],
                );
            }
            let complete = prepared.numbers.complete_keys("GER")[slot];
            assert_eq!(
                reader
                    .value_at(complete, first + 1)
                    .unwrap()
                    .map(|run| run.value),
                Some(0),
            );
        }
        // Losing and recovering coverage changes completeness even though
        // every numeric operand returns with exactly its previous value.
        let logistics = frame.tables[TableId::LogisticsEquipment.index()].clone();
        frame.hour += 1;
        *frame.tables[TableId::LogisticsEquipment.index()] = TableView::Unavailable {
            reason: "synthetic missing logistics read".into(),
        };
        prepared.observe(format, &frame).unwrap();
        assert_eq!(prepared.choices.completeness["GER"][1..], [false, false]);
        frame.hour += 1;
        frame.tables[TableId::LogisticsEquipment.index()] = logistics;
        prepared.observe(format, &frame).unwrap();
        assert_eq!(prepared.choices.completeness["GER"], [true; 3]);
    }
}

#[test]
fn repeating_reads_reclassify_a_changed_content_binding() {
    use replay_data::{rows::LexicalTokenRow, rows::TableRows};
    for variants in [false, true] {
        let mut frame = classification_frame(variants);
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"mod_equipment_1".to_vec(),
            }]));
        let format = replay(vec![frame.clone()]).format;
        let mut prepared = Prepared::default();
        prepared.observe(format, &frame).unwrap();
        let content = resolved_content();
        prepared.bind_content(&content);
        // A new binding cannot be skipped as a quiet span or a repeated row read.
        assert!(
            !prepared
                .advance_quiet(frame.hour + 1, frame.hour + 2)
                .unwrap()
        );
        frame.hour += 1;
        prepared.observe(format, &frame).unwrap();
        assert_eq!(prepared.choices.completeness["GER"], [true; 3]);
        assert!(
            prepared.holders["GER"]
                .groups
                .contains_key(&series::field_key("type", "mod_equipment"))
        );
        let projected = prepared.numbers.projected;
        frame.hour += 1;
        prepared.bind_content(&content);
        prepared.observe(format, &frame).unwrap();
        assert_eq!(prepared.numbers.projected, projected);
    }
}

#[test]
fn logistics_coverage_loss_closes_known_totals_with_unchanged_completeness_across_resume() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay_data::{rows::LexicalTokenRow, rows::LogisticsEquipmentRow, rows::TableRows};

    let content = resolved_content();
    let mut initial = classification_frame(false);
    let first = initial.hour;
    *initial.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1,
            literal: b"mod_equipment_1".to_vec(),
        }]));
    if let TableView::Complete(TableRows::LogisticsEquipment(rows)) =
        &mut *initial.tables[TableId::LogisticsEquipment.index()]
    {
        rows.push(LogisticsEquipmentRow {
            country: 11,
            archetype: 2,
            token: 2,
            balance_raw: -9,
            stock_raw: 4,
            available_raw: replay_data::rows::Recorded::Value(Some(2)),
        });
    }
    let frames: Vec<_> = (0..=4)
        .map(|offset| {
            let mut frame = initial.clone();
            frame.hour += offset;
            if offset > 0 {
                frame.metrics.table_ns.fill(0);
            }
            if (1..=2).contains(&offset) {
                *frame.tables[TableId::LogisticsEquipment.index()] = TableView::Unavailable {
                    reason: "synthetic coverage loss without a supplying read".into(),
                };
            } else if offset == 3 {
                frame.metrics.table_ns[TableId::LogisticsEquipment.index()] = 1;
            }
            frame
        })
        .collect();
    let format = replay(frames.clone()).format;
    let flag = AtomicBool::new(false);
    let uninterrupted = dashboard_base::prepared::series::tests::Fixture::new();
    let restored = dashboard_base::prepared::series::tests::Fixture::new();
    let mut prepared = Prepared::default();
    let mut resumed = Prepared::default();
    prepared.bind_content(&content);
    resumed.bind_content(&content);
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        let changes = tracker.observe(frame, format);
        if (first + 1..=first + 2).contains(&frame.hour) {
            assert!(!changes.slot_read[TableId::LogisticsEquipment.index()]);
            assert!(!changes.slot_read[TableId::Stockpiles.index()]);
            assert!(!changes.slot_read[TableId::VariantDefinitions.index()]);
        }
        for (path, state, fixture) in [
            ("uninterrupted", &mut prepared, &uninterrupted),
            ("restored", &mut resumed, &restored),
        ] {
            state.observe_changed(format, frame, &changes).unwrap();
            assert_eq!(
                state.choices.completeness["GER"],
                [true, false, false],
                "{path} completeness at {}",
                frame.hour,
            );
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        dashboard_base::prepared::series::tests::assert_payloads_equal(
            &uninterrupted.payloads(),
            &restored.payloads(),
            "logistics coverage loss, uninterrupted versus restore after every hour",
        );
        assert_eq!(
            serde_json::to_value(&prepared).unwrap(),
            serde_json::to_value(&resumed).unwrap(),
            "continuation state at {}",
            frame.hour,
        );
        resumed = serde_json::from_slice(&serde_json::to_vec(&resumed).unwrap()).unwrap();
        resumed.bind_content(&content);
    }
    let group = series::field_key("type", "mod_equipment");
    let keys = prepared.numbers.group_keys("GER", &group);
    for fixture in [&uninterrupted, &restored] {
        for (slot, value) in [(1, -5), (2, 8)] {
            let mut reader = Reader::new(
                &fixture.store,
                &fixture.key,
                format!("stockpile-1-group-{}", MEASURES[slot]),
                &flag,
            )
            .unwrap();
            for offset in 0..=4 {
                assert_eq!(
                    reader
                        .value_at(keys[slot], first + offset)
                        .unwrap()
                        .map(|run| run.value),
                    (!(1..=2).contains(&offset)).then_some(value),
                    "{} at {}",
                    MEASURES[slot],
                    first + offset,
                );
            }
        }
    }
}

#[test]
fn logistics_representatives_preserve_absence_required_available_and_duplicate_policy() {
    let mut required = classification("required");
    required.kind.as_mut().unwrap().requires_available = true;
    let unknown = Classification {
        token: "unresolved token".into(),
        kind: None,
    };
    let all = BTreeMap::from([
        (1, classification("duplicate")),
        (2, classification("duplicate")),
        (3, required),
        (4, unknown),
    ]);
    let mut retained = Logistics::default();
    for (&id, classification) in &all {
        retained.observe(id, classification.clone());
    }
    assert_eq!(
        retained.representatives[&series::field_key("type", "duplicate")].0,
        2
    );
    for active in [
        BTreeSet::new(),
        BTreeSet::from([1]),
        BTreeSet::from([2, 3]),
        BTreeSet::from([2, 3, 4]),
    ] {
        let current: BTreeMap<_, _> = all
            .iter()
            .filter(|(id, _)| active.contains(*id))
            .map(|(&id, class)| (id, class.clone()))
            .collect();
        for group in retained.groups.keys() {
            let read = |field, id: u64| -> io::Result<Operand> {
                let present = active.contains(&(id as u32));
                Ok(Operand {
                    present,
                    value: Some(if present {
                        match field {
                            5 => -2,
                            6 => 5,
                            7 => 8,
                            _ => 0,
                        }
                    } else {
                        0
                    }),
                })
            };
            let expected = evaluate(group, &BTreeMap::new(), &all, true, true, None, read).unwrap();
            let actual = evaluate(
                group,
                &BTreeMap::new(),
                &current,
                true,
                true,
                Some(&retained),
                read,
            )
            .unwrap();
            assert_eq!(actual, expected, "{group} / {active:?}");
        }
    }
    let group = series::field_key("type", "required");
    let absent = evaluate(
        &group,
        &BTreeMap::new(),
        &BTreeMap::new(),
        true,
        true,
        Some(&retained),
        |_, _| {
            Ok(Operand {
                value: Some(0),
                present: false,
            })
        },
    )
    .unwrap();
    assert_eq!(absent[1], Some(0));
    assert_eq!(absent[2], None);
    let unknown = series::field_key("type", &format!("{UNRESOLVED_MARK}unresolved token"));
    assert_eq!(
        evaluate(
            &unknown,
            &BTreeMap::new(),
            &BTreeMap::new(),
            true,
            true,
            Some(&retained),
            |_, _| Ok(Operand {
                value: Some(0),
                present: false
            })
        )
        .unwrap()[0],
        None
    );
}

#[test]
fn logistics_choices_per_read_work_is_flat_over_retired_id_history() {
    use dashboard_base::prepared::observer_tests::{self, Work};
    observer_tests::assert_flat(
        observer_tests::scaled_work(10, |history| {
            let mut choices = Choices::default();
            let mut holder = Holder::default();
            let mut numbers = Numbers::default();
            let names = VariantCatalogue::default();
            let mut scanned = ScannedStockpile::default();
            for offset in 0..history {
                let id = offset as u32 + 1;
                let hour = offset as i32;
                holder.logistics.insert(id);
                scanned.balance = ReadRuns::new();
                scanned.logistics_stock = ReadRuns::new();
                scanned.available = ReadRuns::new();
                scanned.stock.read(hour, []);
                scanned.balance.read(hour, [(id, Some(0))]);
                scanned.logistics_stock.read(hour, [(id, Some(0))]);
                scanned.available.read(hour, [(id, Some(0))]);
                scanned.logistics_reads.push(hour);
                choices.observe("GER", &mut holder, &scanned, &names, hour, &mut numbers)?;
            }
            let before = choices.work;
            let hour = history as i32;
            let id = history as u32;
            scanned.stock.read(hour, []);
            scanned.balance.read(hour, [(id, Some(0))]);
            scanned.logistics_stock.read(hour, [(id, Some(0))]);
            scanned.available.read(hour, [(id, Some(0))]);
            scanned.logistics_reads.push(hour);
            choices.observe("GER", &mut holder, &scanned, &names, hour, &mut numbers)?;
            Ok(Work {
                history_visits: choices.work.history_visits - before.history_visits,
                ..Default::default()
            })
        })
        .unwrap(),
    );
}

#[test]
fn logistics_representatives_match_full_evaluator_after_two_commits_resume_and_append() {
    let flag = AtomicBool::new(false);
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut choices = Choices::default();
    let mut numbers = Numbers::default();
    let mut holder = Holder::default();
    let names = VariantCatalogue::default();
    let mut scanned = ScannedStockpile::default();
    let mut expected = Vec::new();
    for hour in [0, 1, 2, 4, 5, 6, 7, 8] {
        choices.advance(hour).unwrap();
        numbers.advance(hour).unwrap();
        holder.tokens.entry(1).or_insert_with(|| "token 2".into());
        if hour == 4 {
            holder.tokens.insert(2, "token 3".into());
        }
        if hour == 8 {
            holder.tokens.insert(3, "third named type".into());
        }
        let ids: &[u32] = match hour {
            0 | 2 => &[1],
            1 | 4 => &[2],
            5 => &[1, 2, 3],
            6 => &[],
            7 => &[1, 2],
            _ => &[2, 3],
        };
        holder.logistics.extend(ids.iter().copied());
        scanned.stock.read(hour, []);
        scanned
            .balance
            .read(hour, ids.iter().map(|&id| (id, Some(i64::from(id)))));
        scanned
            .logistics_stock
            .read(hour, ids.iter().map(|&id| (id, Some(7))));
        scanned.available.read(
            hour,
            ids.iter().filter(|_| hour != 2).map(|&id| (id, Some(8))),
        );
        scanned.logistics_reads.push(hour);
        choices
            .observe("GER", &mut holder, &scanned, &names, hour, &mut numbers)
            .unwrap();
        let all: BTreeMap<_, _> = holder
            .logistics
            .iter()
            .map(|&id| (id, choices.classify_logistics(&holder, id)))
            .collect();
        for group in holder.groups.keys() {
            let values = evaluate(
                group,
                &BTreeMap::new(),
                &all,
                true,
                true,
                None,
                |field, id| {
                    Ok(match field {
                        0 => operand(&scanned.stock, id, hour),
                        5 => operand(&scanned.balance, id as u32, hour),
                        6 => operand(&scanned.logistics_stock, id as u32, hour),
                        7 => operand(&scanned.available, id as u32, hour),
                        _ => unreachable!(),
                    })
                },
            )
            .unwrap();
            expected.push((hour, group.clone(), values));
        }
        numbers
            .observe(
                "GER",
                hour,
                &mut scanned,
                &holder,
                &BTreeSet::new(),
                &BTreeSet::new(),
            )
            .unwrap();
        if [1, 4, 8].contains(&hour) {
            let mut writer = if hour == 8 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            numbers.flush(&mut writer, 1, &flag).unwrap();
            choices.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            choices = serde_json::from_slice(&serde_json::to_vec(&choices).unwrap()).unwrap();
            numbers = serde_json::from_slice(&serde_json::to_vec(&numbers).unwrap()).unwrap();
        }
    }
    for slot in 0..4 {
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            format!("stockpile-1-group-{}", MEASURES[slot]),
            &flag,
        )
        .unwrap();
        for (hour, group, values) in &expected {
            let id = numbers.keys[&series::field_key("GER", &group_key(group, MEASURES[slot]))];
            assert_eq!(
                reader.value_at(id, *hour).unwrap().map(|run| run.value),
                values[slot],
                "{group} at {hour}, {}",
                MEASURES[slot]
            );
        }
    }
}

#[test]
fn stockpile_static_content_resolves_once_before_success_and_refusal_observation() {
    use dashboard_base::prepared::observer_tests::{self, Work};
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let frame = stockpile_frame(0, true, &[(10, 100)], Some(&[]));
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "stockpile-static-content".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay(vec![frame.clone()])),
        &AtomicBool::new(false),
    )
    .unwrap();
    for (text, refused) in [
        (
            "equipments = { infantry_equipment = { is_archetype = yes type = infantry } infantry_equipment_1 = { archetype = infantry_equipment } }",
            false,
        ),
        (
            "equipments = { broken = { unsupported_property = yes } }",
            false,
        ),
        ("equipments = { hull = { alias = 7 } later = { } }", true),
        (
            "equipments = { hull = { is_archetype = yes } } duplicate_archetypes = { d = { archetype = \"9\" } }",
            true,
        ),
    ] {
        let (catalogue, folder) =
            catalogue::tests::folder_catalogue(&[("common/units/equipment/mod.txt", text)]);
        let catalogue = Arc::new(catalogue);
        let options = dashboard_core::config::ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: folder.0.clone(),
                user_dir: folder.0.clone(),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        };
        catalogue::set_test_catalogue(&source, &options, catalogue.clone());
        let content = Content::load(&source, Some(&options));
        if refused {
            assert!(content.database.is_none());
        }
        let units = catalogue
            .content
            .layers
            .derived::<catalogue::CachedResult<Database>>();
        let kinds = catalogue
            .content
            .layers
            .derived::<catalogue::KeyedResults<String, Kind>>();
        assert_eq!(units.loads(), 1);
        let baseline = Work {
            content_loads: units.loads(),
            ..Default::default()
        };
        let mut choices = Choices::default();
        choices.bind_content(&content);
        let expected = choices.classification("infantry_equipment_1");
        observer_tests::assert_flat(
            observer_tests::scaled_work(10, |hours| {
                observer_tests::unchanged_frames(
                    baseline,
                    &frame,
                    hours as i32,
                    source.replay_ref().format,
                    |_frame, _changes| {
                        // A new token/holder uses fixed authored facts, including a
                        // previously refused content binding, without re-resolution.
                        choices.bind_content(&content);
                        assert_eq!(choices.classification("infantry_equipment_1"), expected);
                        assert!(
                            choices
                                .classification("absent mod equipment")
                                .kind
                                .is_none()
                        );
                        Ok(Work {
                            content_loads: units.loads(),
                            ..baseline
                        })
                    },
                )
            })
            .unwrap(),
        );
        let rebound = Content::load(&source, Some(&options));
        assert_eq!(units.loads(), baseline.content_loads);
        assert_eq!(rebound.database.is_some(), content.database.is_some());
        if let Some(database) = &content.database {
            assert!(Arc::ptr_eq(database, rebound.database.as_ref().unwrap()));
            assert!(Arc::ptr_eq(
                content.kinds.as_ref().unwrap(),
                rebound.kinds.as_ref().unwrap()
            ));
            assert_eq!(kinds.loads(&"infantry_equipment_1".to_owned()), 1);
            assert_eq!(kinds.loads(&"absent mod equipment".to_owned()), 1);
            for row in &database.equipment {
                if let Ok(token) = row.name_str() {
                    let expected =
                        archetype_of(database, token)
                            .ok()
                            .map(|(key, _, domain)| Kind {
                                requires_available: need_note(database, &key)
                                    .map_or(true, |note| note.is_some()),
                                key,
                                domain,
                            });
                    assert_eq!(choices.classification(token).kind, expected);
                }
            }
        } else {
            let first = equipment::units(&source, &catalogue).unwrap_err();
            assert_eq!(equipment::units(&source, &catalogue).unwrap_err(), first);
        }
    }
}

fn classification(key: &str) -> Classification {
    Classification {
        token: key.into(),
        kind: Some(Kind {
            key: key.into(),
            domain: Domain::Land,
            requires_available: false,
        }),
    }
}

struct Scenario {
    fixture: dashboard_base::prepared::series::tests::Fixture,
    source: Source,
    holder: Holder,
    numbers: Numbers,
    metadata: Metadata,
}
impl Scenario {
    fn new(
        raw: &[[Option<i64>; 2]],
        observed: &[[Option<i128>; 2]],
        final_groups: [&str; 2],
        historical: &[(u64, i32, i32, Classification)],
    ) -> Self {
        assert_eq!(raw.len(), observed.len());
        let fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let flag = AtomicBool::new(false);
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "stockpile-choices".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay(vec![stockpile_frame(0, true, &[], Some(&[]))])),
            &flag,
        )
        .unwrap();
        let holder = Holder {
            variants: BTreeSet::from([1, 2]),
            groups: BTreeMap::from([
                (series::field_key("type", "a"), 0),
                (series::field_key("type", "b"), 0),
                (series::field_key("domain", "land"), 0),
            ]),
            ..Default::default()
        };
        let metadata = Metadata {
            first: Some(0),
            last: raw.len() as i32 - 1,
            current: final_groups
                .into_iter()
                .enumerate()
                .map(|(index, key)| {
                    (
                        index as u64 + 1,
                        ClassifiedSpan {
                            first: 0,
                            last: raw.len() as i32 - 1,
                            classification: classification(key),
                        },
                    )
                })
                .collect(),
        };
        let mut numbers = Numbers::default();
        let mut groups: [Accumulator; 4] = Default::default();
        let mut intervals = Accumulator::default();
        for (id, first, last, classification) in historical {
            intervals
                .event(
                    *id,
                    last + 1,
                    &ClassifiedSpan {
                        first: *first,
                        last: *last,
                        classification: classification.clone(),
                    },
                )
                .unwrap();
        }
        let mut scanned = ScannedStockpile::default();
        for (hour, (raw, observed)) in raw.iter().zip(observed).enumerate() {
            let hour = hour as i32;
            scanned.stock.read(
                hour,
                raw.iter()
                    .enumerate()
                    .map(|(index, value)| (index as u64 + 1, *value)),
            );
            numbers
                .observe(
                    "GER",
                    hour,
                    &mut scanned,
                    &holder,
                    &BTreeSet::new(),
                    &BTreeSet::new(),
                )
                .unwrap();
            for (key, value) in ["a", "b"].into_iter().zip(observed) {
                let actor = allocate(
                    &mut numbers.keys,
                    "GER",
                    &group_key(&series::field_key("type", key), "stock"),
                );
                groups[0].observe(actor, hour, *value, false);
            }
            for group in &mut groups {
                group.through(hour);
            }
            intervals.through(hour);
            if hour % 2 == 1 || hour == metadata.last {
                let mut writer = fixture.writer();
                numbers.flush(&mut writer, 1, &flag).unwrap();
                for (slot, group) in groups.iter_mut().enumerate() {
                    group
                        .flush(
                            &mut writer,
                            &format!("stockpile-1-group-{}", MEASURES[slot]),
                            0,
                            &flag,
                        )
                        .unwrap();
                }
                intervals
                    .flush(&mut writer, "stockpile-1-classification", 0, &flag)
                    .unwrap();
                writer.commit().unwrap();
                numbers = serde_json::from_slice(&serde_json::to_vec(&numbers).unwrap()).unwrap();
                groups = serde_json::from_slice(&serde_json::to_vec(&groups).unwrap()).unwrap();
                intervals =
                    serde_json::from_slice(&serde_json::to_vec(&intervals).unwrap()).unwrap();
            }
        }
        Self {
            fixture,
            source,
            holder,
            numbers,
            metadata,
        }
    }
    fn query<'a>(&'a self, request: &'a Request, flag: &'a AtomicBool) -> Query<'a> {
        Query::new(
            &self.source,
            request,
            &LoadContext { content: None },
            &self.fixture.store,
            &self.fixture.key,
            &self.numbers.keys,
            &self.holder,
            &self.metadata,
            flag,
        )
        .unwrap()
    }
}
fn request(included: &[&str]) -> Request {
    serde_json::from_value(serde_json::json!({"branchId":"1","tag":"GER","from":0,"to":2,"width":1,"included":included})).unwrap()
}

#[test]
fn ordinary_and_float_rounded_buckets_return_exact_integer_maxima() {
    for values in [
        [42, 41, 43],
        [
            9_007_199_254_740_992,
            9_007_199_254_740_993,
            9_007_199_254_740_992,
        ],
    ] {
        let scenario = Scenario::new(
            &values.map(|value| [Some(value), Some(0)]),
            &values.map(|value| [Some(i128::from(value)), Some(0)]),
            ["a", "b"],
            &[],
        );
        let flag = AtomicBool::new(false);
        let request = request(&[]);
        let mut query = scenario.query(&request, &flag);
        let peak = query
            .peak(
                &series::field_key("type", "a"),
                &[HourSpan {
                    first_hour: 0,
                    last_hour: 2,
                }],
            )
            .unwrap();
        assert_eq!(peak.raw, values.into_iter().max().unwrap().to_string());
        assert!(peak.stock_drawable);
    }
}

#[test]
fn signed_late_classification_excludes_uncorrected_peaks_and_honours_membership() {
    let scenario = Scenario::new(
        &[
            [Some(-150), Some(200)],
            [Some(-150), Some(200)],
            [Some(0), Some(60)],
        ],
        &[
            [Some(200), Some(-150)],
            [Some(200), Some(-150)],
            [Some(60), Some(0)],
        ],
        ["a", "a"],
        &[(1, 0, 1, classification("b"))],
    );
    let flag = AtomicBool::new(false);
    let request = request(&[]);
    let mut query = scenario.query(&request, &flag);
    let group = series::field_key("type", "a");
    assert_eq!(
        query
            .peak(
                &group,
                &[HourSpan {
                    first_hour: 0,
                    last_hour: 2
                }]
            )
            .unwrap()
            .raw,
        "60"
    );
    assert_eq!(
        query
            .peak(
                &group,
                &[HourSpan {
                    first_hour: 0,
                    last_hour: 1
                }]
            )
            .unwrap()
            .raw,
        "50"
    );
    let summary = query
        .summary(
            &group,
            0,
            (0.0, 2.0, 1),
            &[HourSpan {
                first_hour: 0,
                last_hour: 2,
            }],
        )
        .unwrap();
    assert_eq!(
        summary
            .values
            .buckets
            .iter()
            .flatten()
            .map(|bucket| bucket.max_value)
            .fold(f64::NEG_INFINITY, f64::max),
        60.0
    );
}

#[test]
fn unknown_affected_operands_stay_unknown_and_late_naming_restores_other_groups() {
    let unknown = Classification {
        token: String::new(),
        kind: None,
    };
    let scenario = Scenario::new(
        &[[Some(20), Some(80)], [None, Some(80)], [Some(20), Some(80)]],
        &[[None, None], [None, None], [Some(20), Some(80)]],
        ["a", "b"],
        &[(1, 0, 1, unknown)],
    );
    let flag = AtomicBool::new(false);
    let request = request(&[]);
    let mut query = scenario.query(&request, &flag);
    let within = [HourSpan {
        first_hour: 0,
        last_hour: 1,
    }];
    assert_eq!(
        query
            .runs(&series::field_key("type", "a"), 0, &within)
            .unwrap()
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value))
            .collect::<Vec<_>>(),
        vec![(0, 0, 20)]
    );
    assert_eq!(
        query
            .peak(&series::field_key("type", "b"), &within)
            .unwrap()
            .raw,
        "80"
    );
}

#[test]
fn included_domain_merges_correlated_type_changes_instead_of_summing_maxima() {
    let scenario = Scenario::new(
        &[
            [Some(100), Some(0)],
            [Some(0), Some(100)],
            [Some(40), Some(40)],
        ],
        &[
            [Some(100), Some(0)],
            [Some(0), Some(100)],
            [Some(40), Some(40)],
        ],
        ["a", "b"],
        &[],
    );
    let flag = AtomicBool::new(false);
    let request = request(&["a", "b"]);
    let mut query = scenario.query(&request, &flag);
    let within = [HourSpan {
        first_hour: 0,
        last_hour: 2,
    }];
    let group = series::field_key("domain", "land");
    assert_eq!(query.peak(&group, &within).unwrap().raw, "100");
    assert_eq!(
        query
            .summary(&group, 0, (0.0, 2.0, 1), &within)
            .unwrap()
            .values
            .buckets[0]
            .unwrap()
            .max_value,
        100.0
    );
}

#[test]
fn token_changes_affect_both_groups_and_unnamed_intervals_affect_completeness() {
    let history = BTreeMap::from([(
        1,
        vec![
            ClassifiedSpan {
                first: 0,
                last: 1,
                classification: classification("a"),
            },
            ClassifiedSpan {
                first: 2,
                last: 3,
                classification: Classification {
                    token: String::new(),
                    kind: None,
                },
            },
            ClassifiedSpan {
                first: 4,
                last: 5,
                classification: classification("b"),
            },
        ],
    )]);
    let final_names = BTreeMap::from([(1, classification("b"))]);
    for (group, expected) in [
        ("a", vec![(0, 3)]),
        ("b", vec![(0, 3)]),
        ("c", vec![(2, 3)]),
    ] {
        assert_eq!(
            affected(&series::field_key("type", group), &history, &final_names)
                .iter()
                .map(|span| (span.first_hour, span.last_hour))
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn empty_logistics_spelling_is_not_the_variant_handle_fallback() {
    let empty = Classification {
        token: String::new(),
        kind: None,
    };
    assert_eq!(empty.groups(None)[0], series::field_key("type", ":"));
    assert_eq!(empty.groups(Some(7))[0], series::field_key("type", ":7"));
}

#[test]
fn display_selection_reads_exact_members_but_dense_types_use_only_aggregates() {
    let scenario = Scenario::new(
        &[[Some(20), Some(80)]],
        &[[Some(20), Some(80)]],
        ["a", "b"],
        &[],
    );
    let flag = AtomicBool::new(false);
    let request = request(&[]);
    let query = scenario.query(&request, &flag);
    let type_a = [series::field_key("type", "a")];
    assert!(query.relevant(0, 1, &type_a, false));
    assert!(!query.relevant(0, 2, &type_a, false));
    assert!(!query.relevant(0, 1, &type_a, true));
    assert!(!query.relevant(0, 2, &type_a, true));
    let variant = [series::field_key("variant", "1")];
    assert!(query.relevant(0, 1, &variant, true));
    assert!(!query.relevant(0, 2, &variant, true));
}

#[test]
fn one_pass_group_totals_are_each_groups_own_evaluation() {
    let unresolved = |token: &str| Classification {
        token: token.into(),
        kind: None,
    };
    let mut truck = classification("truck");
    truck.kind.as_mut().unwrap().requires_available = true;
    let mut fighter = classification("fighter");
    fighter.kind.as_mut().unwrap().domain = Domain::Air;
    let variants = BTreeMap::from([
        (1, classification("infantry")),
        (2, classification("infantry")),
        (3, fighter.clone()),
        (4, truck.clone()),
        (5, unresolved("mod_gun")),
        (6, unresolved("")),
    ]);
    // Two rows of one archetype, of which the last is its representative.
    let logistics = BTreeMap::from([
        (11, classification("infantry")),
        (12, classification("infantry")),
        (13, truck),
        (14, fighter),
        (15, unresolved("mod_type")),
    ]);
    let mut retained = Logistics::default();
    for (&id, classification) in &logistics {
        retained.observe(id, classification.clone());
    }
    let mut groups: BTreeSet<String> = variants
        .iter()
        .flat_map(|(&id, classification)| classification.groups(Some(id)))
        .chain(
            logistics
                .values()
                .flat_map(|classification| classification.groups(None)),
        )
        .collect();
    groups.insert(series::field_key("type", "absent"));
    groups.insert(series::field_key("domain", "naval"));
    groups.insert(series::field_key("domain", "unresolved"));
    for scenario in 0..256_u64 {
        let mix = |field: u64, id: u64| {
            (scenario * 1_000_003 + field * 7_919 + id * 104_729)
                .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                >> 33
        };
        let read = |field: usize, id: u64| {
            let hash = mix(field as u64, id);
            Operand {
                present: hash % 3 != 0,
                value: match hash % 5 {
                    0 => None,
                    1 => Some(0),
                    _ => Some(i128::from(hash % 1_000) - 500),
                },
            }
        };
        let current: BTreeMap<u64, Classification> = variants
            .iter()
            .filter(|(id, _)| mix(97, **id) % 4 != 0)
            .map(|(&id, classification)| (id, classification.clone()))
            .collect();
        let current_logistics: BTreeMap<u32, Classification> = logistics
            .iter()
            .filter(|(id, _)| mix(98, u64::from(**id)) % 3 != 0)
            .map(|(&id, classification)| (id, classification.clone()))
            .collect();
        let variant_groups: Vec<_> = current
            .iter()
            .map(|(&id, classification)| (id, classification, classification.groups(Some(id))))
            .collect();
        let members: Vec<_> = variant_groups
            .iter()
            .map(|(id, classification, groups)| (*id, *classification, groups))
            .collect();
        let logistic_members: Vec<_> = current_logistics
            .iter()
            .map(|(&id, classification)| (id, classification, classification.groups(None)))
            .collect();
        let (stock_known, balance_known) = (scenario & 1 != 0, scenario & 2 != 0);
        let measured = evaluate_groups(
            &groups,
            &members,
            &logistic_members,
            stock_known,
            balance_known,
            &retained,
            read,
        )
        .unwrap();
        for (group, values) in groups.iter().zip(&measured) {
            let expected = evaluate(
                group,
                &current,
                &current_logistics,
                stock_known,
                balance_known,
                Some(&retained),
                |field, id| Ok(read(field, id)),
            )
            .unwrap();
            assert_eq!(*values, expected, "{group} in scenario {scenario}");
        }
    }
}

#[test]
fn changed_stock_projects_only_its_type_and_domain_and_keeps_all_group_bytes() {
    let fixtures = [
        dashboard_base::prepared::series::tests::Fixture::new(),
        dashboard_base::prepared::series::tests::Fixture::new(),
    ];
    let flag = AtomicBool::new(false);
    let mut choices = [Choices::default(), Choices::default()];
    choices[1].whole_projection = true;
    let mut numbers = [Numbers::default(), Numbers::default()];
    let mut holders = [Holder::default(), Holder::default()];
    let mut scans = [ScannedStockpile::default(), ScannedStockpile::default()];
    for choices in &mut choices {
        for (id, key, domain) in [
            (10, "synthetic_a", Domain::Land),
            (20, "synthetic_b", Domain::Air),
        ] {
            let class = Classification {
                token: key.into(),
                kind: Some(Kind {
                    key: key.into(),
                    domain,
                    requires_available: false,
                }),
            };
            choices.variant_groups.insert(id, class.groups(Some(id)));
            choices.metadata.current.insert(
                id,
                ClassifiedSpan {
                    first: 0,
                    last: 0,
                    classification: class,
                },
            );
        }
    }
    for hour in [0, 1, 2, 3, 4, 6] {
        let mut work = [0; 2];
        for index in 0..2 {
            choices[index].advance(hour).unwrap();
            numbers[index].advance(hour).unwrap();
            holders[index].variants.extend([10, 20]);
            if hour == 3 {
                scans[index].stock.close();
            } else {
                scans[index].stock.read(
                    hour,
                    [
                        (10, (hour != 4).then_some(if hour < 2 { 10 } else { 50 })),
                        (20, Some(5)),
                    ],
                );
            }
            scans[index].daily.read(hour, []);
            let before = choices[index].work.history_visits;
            choices[index]
                .observe(
                    "mod/tag",
                    &mut holders[index],
                    &scans[index],
                    &VariantCatalogue::default(),
                    hour,
                    &mut numbers[index],
                )
                .unwrap();
            work[index] = choices[index].work.history_visits - before;
            numbers[index]
                .observe(
                    "mod/tag",
                    hour,
                    &mut scans[index],
                    &holders[index],
                    &BTreeSet::new(),
                    &BTreeSet::new(),
                )
                .unwrap();
        }
        if hour == 1 {
            assert_eq!(work[0], 0);
            assert!(work[1] > 0);
        }
        if hour == 2 {
            assert_eq!(work, [2, 4], "only the changed stock's type and domain");
        }
        if [2, 6].contains(&hour) {
            for index in 0..2 {
                let mut writer = fixtures[index].writer();
                choices[index].flush(&mut writer, 1, &flag).unwrap();
                numbers[index].flush(&mut writer, 1, &flag).unwrap();
                writer.commit().unwrap();
            }
            dashboard_base::prepared::series::tests::assert_payloads_equal(
                &fixtures[0].payloads(),
                &fixtures[1].payloads(),
                "changed stock groups",
            );
            assert_eq!(
                serde_json::to_vec(&choices[0]).unwrap(),
                serde_json::to_vec(&choices[1]).unwrap()
            );
        }
    }
}
