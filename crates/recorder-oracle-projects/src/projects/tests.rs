use super::*;
use replay_data::{
    rows::NukeRow, rows::ProjectBreakthroughRow, rows::ProjectContext, rows::ProjectHistoryRow,
    rows::ProjectProgramRow, rows::ProjectRow, rows::ProjectState, rows::RaidCooldownRow,
    rows::RaidCountryRow, rows::RaidRow, rows::RaidScope, rows::RaidSource, rows::RaidTargetRow,
    rows::RaidTargetStatusRow, rows::TableId, rows::TableRows,
};
use replay_frame::frame::TableView;

fn scientist_pack() -> characters_saved::Pack {
    characters_saved::parse(
        &content_reader_parser::parser::parse(
            br#"character_manager={ historical={
                character={ id={ id=265 type=73 } token="a" country="GER"
                    scientist={ traits={ "t1" "t2" }
                        skills={ specialization_nuclear={ level=2 }
                                 specialization_naval={ experience=219.18 } } } }
                character={ id={ id=266 type=73 } token="b"
                    scientist={ skills={ specialization_nuclear={ } } } }
            }} countries={}"#,
        )
        .unwrap()
        .items,
    )
    .unwrap()
}

fn scientist_frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: replay_data::cadence::refresh_slot(TableId::ProjectPrograms).unwrap() as i32,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Default::default(),
    };
    *frame.tables[TableId::Scientists.index()] = TableView::Complete(TableRows::Scientists(vec![
        replay_data::rows::ScientistRow {
            character: 265,
            country: 7,
            ordinal: 0,
            trait_count: 2,
            skill_count: 2,
            active: replay_data::rows::Recorded::Value(Some(0)),
        },
    ]));
    *frame.tables[TableId::ScientistDaily.index()] =
        TableView::Complete(TableRows::ScientistDaily(vec![
            replay_data::rows::ScientistDailyRow {
                character: 265,
                country: 7,
                injury_days: replay_data::rows::Recorded::Value(Some(0)),
                template_visibility: replay_data::rows::Recorded::Value(Some(0)),
                template_origin: replay_data::rows::Recorded::Value(None),
            },
        ]));
    *frame.tables[TableId::ScientistSkills.index()] =
        TableView::Complete(TableRows::ScientistSkills(vec![
            replay_data::rows::ScientistSkillRow {
                character: 265,
                ordinal: 0,
                token: 0x56c6,
                level: 2,
                experience_raw: 0,
            },
            replay_data::rows::ScientistSkillRow {
                character: 265,
                ordinal: 1,
                token: 0x56c7,
                level: 0,
                experience_raw: 21_918_000,
            },
        ]));
    frame
}

fn run_scientists(frame: &DecodedFrame) -> Report {
    let germany = CountryRow {
        id: 7,
        canonical_slot: 1,
        tag: b"GER".to_vec(),
        ..CountryRow::default()
    };
    let links = BTreeMap::from([(7, &germany)]);
    let tokens: TokenNames<'_> = BTreeMap::from([
        (0x56c6, b"specialization_nuclear".as_slice()),
        (0x56c7, b"specialization_naval"),
    ]);
    let mut report = Report::default();
    compare_scientists(frame, &scientist_pack(), &links, &tokens, &mut report).unwrap();
    report
}

#[test]
fn gap_research_grants_scientist_oracle_compares_injury_without_inventing_negative_save_fields() {
    let mut frame = scientist_frame();
    let TableView::Complete(TableRows::ScientistDaily(rows)) =
        &mut *frame.tables[TableId::ScientistDaily.index()]
    else {
        unreachable!()
    };
    rows[0].injury_days = replay_data::rows::Recorded::Value(Some(8));
    let report = run_scientists(&frame);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("injury days"))
    );
    let TableView::Complete(TableRows::ScientistDaily(rows)) =
        &mut *frame.tables[TableId::ScientistDaily.index()]
    else {
        unreachable!()
    };
    rows[0].injury_days = replay_data::rows::Recorded::Value(Some(-8));
    assert!(run_scientists(&frame).differences.is_empty());
    let TableView::Complete(TableRows::ScientistDaily(rows)) =
        &mut *frame.tables[TableId::ScientistDaily.index()]
    else {
        unreachable!()
    };
    rows[0].injury_days = replay_data::rows::Recorded::NotRecorded;
    frame.hour += 1;
    assert!(run_scientists(&frame).outcome().is_ok());
}

#[test]
fn current_daily_observation_cannot_omit_a_rostered_scientist() {
    let mut frame = scientist_frame();
    *frame.tables[TableId::ScientistDaily.index()] =
        TableView::Complete(TableRows::ScientistDaily(Vec::new()));
    assert!(
        run_scientists(&frame)
            .differences
            .iter()
            .any(|difference| difference.contains("daily observation"))
    );
}

#[test]
fn a_matching_scientist_roster_compares_counts_tokens_and_levels() {
    let report = run_scientists(&scientist_frame());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["Scientists"], 1);
    assert_eq!(report.compared_tables["ScientistSkills"], 2);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("specialization level"))
    );
}

#[test]
fn scientist_active_oracle_checks_truth_and_preserves_older_rows() {
    let mut frame = scientist_frame();
    let TableView::Complete(TableRows::Scientists(rows)) =
        &mut *frame.tables[TableId::Scientists.index()]
    else {
        unreachable!()
    };
    rows[0].active = replay_data::rows::Recorded::Value(Some(1));
    let report = run_scientists(&frame);
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("scientist 265.active"));
    let TableView::Complete(TableRows::Scientists(rows)) =
        &mut *frame.tables[TableId::Scientists.index()]
    else {
        unreachable!()
    };
    rows[0].active = replay_data::rows::Recorded::Value(Some(0));
    assert!(run_scientists(&frame).differences.is_empty());
    let TableView::Complete(TableRows::Scientists(rows)) =
        &mut *frame.tables[TableId::Scientists.index()]
    else {
        unreachable!()
    };
    rows[0].active = replay_data::rows::Recorded::Value(None);
    let report = run_scientists(&frame);
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("capture refused"));
    assert_eq!(report.compared_tables["Scientists"], 1);
    assert_eq!(report.compared_tables["ScientistSkills"], 2);
    let TableView::Complete(TableRows::Scientists(rows)) =
        &mut *frame.tables[TableId::Scientists.index()]
    else {
        unreachable!()
    };
    rows[0].active = replay_data::rows::Recorded::NotRecorded;
    assert!(
        run_scientists(&frame)
            .limitations
            .contains_key("Scientists.active not recorded")
    );
}

#[test]
fn count_token_and_level_disagreements_are_each_reported() {
    for (mutate, expected) in [
        (
            (|frame: &mut DecodedFrame| {
                let TableView::Complete(TableRows::Scientists(rows)) =
                    &mut *frame.tables[TableId::Scientists.index()]
                else {
                    unreachable!()
                };
                rows[0].trait_count = 1;
            }) as fn(&mut DecodedFrame),
            "trait_count",
        ),
        (
            |frame| {
                let TableView::Complete(TableRows::ScientistSkills(rows)) =
                    &mut *frame.tables[TableId::ScientistSkills.index()]
                else {
                    unreachable!()
                };
                rows[0].level = 3;
            },
            "level",
        ),
        (
            |frame| {
                let TableView::Complete(TableRows::ScientistSkills(rows)) =
                    &mut *frame.tables[TableId::ScientistSkills.index()]
                else {
                    unreachable!()
                };
                rows[1].level = 1;
            },
            "level",
        ),
        (
            |frame| {
                let TableView::Complete(TableRows::ScientistSkills(rows)) =
                    &mut *frame.tables[TableId::ScientistSkills.index()]
                else {
                    unreachable!()
                };
                rows[1].token = 0x56c6;
            },
            "specialization[1]",
        ),
        (
            |frame| {
                let TableView::Complete(TableRows::ScientistSkills(rows)) =
                    &mut *frame.tables[TableId::ScientistSkills.index()]
                else {
                    unreachable!()
                };
                rows[1].experience_raw = 1;
            },
            "experience",
        ),
        (
            |frame| {
                let TableView::Complete(TableRows::ScientistSkills(rows)) =
                    &mut *frame.tables[TableId::ScientistSkills.index()]
                else {
                    unreachable!()
                };
                rows[0].experience_raw = 5;
            },
            "experience",
        ),
        (
            |frame| {
                let TableView::Complete(TableRows::Scientists(rows)) =
                    &mut *frame.tables[TableId::Scientists.index()]
                else {
                    unreachable!()
                };
                rows[0].country = 0;
            },
            "roster country",
        ),
    ] {
        let mut frame = scientist_frame();
        mutate(&mut frame);
        let report = run_scientists(&frame);
        let text = report.differences.join("\n");
        assert!(text.contains(expected), "{expected} missing from {text}");
    }
}

#[test]
fn a_saved_scientist_with_a_country_must_be_recorded_and_an_extra_row_is_reported() {
    let report = run_scientists(&scientist_frame());
    assert!(report.differences.is_empty());
    let mut frame = scientist_frame();
    let TableView::Complete(TableRows::Scientists(rows)) =
        &mut *frame.tables[TableId::Scientists.index()]
    else {
        unreachable!()
    };
    rows.push(replay_data::rows::ScientistRow {
        character: 266,
        country: 7,
        ordinal: 1,
        trait_count: 0,
        skill_count: 1,
        active: replay_data::rows::Recorded::Value(Some(0)),
    });
    let TableView::Complete(TableRows::ScientistDaily(daily)) =
        &mut *frame.tables[TableId::ScientistDaily.index()]
    else {
        unreachable!()
    };
    daily.push(replay_data::rows::ScientistDailyRow {
        character: 266,
        country: 7,
        ..Default::default()
    });
    let TableView::Complete(TableRows::ScientistSkills(skills)) =
        &mut *frame.tables[TableId::ScientistSkills.index()]
    else {
        unreachable!()
    };
    skills.push(replay_data::rows::ScientistSkillRow {
        character: 266,
        ordinal: 0,
        token: 0x56c6,
        level: 0,
        experience_raw: 0,
    });
    let report = run_scientists(&frame);
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("scientist 266.roster country"))
    );
}

#[test]
fn an_unresolved_roster_country_reference_fails_the_group_instead_of_comparing() {
    let mut frame = scientist_frame();
    let TableView::Complete(TableRows::Scientists(rows)) =
        &mut *frame.tables[TableId::Scientists.index()]
    else {
        unreachable!()
    };
    rows[0].country = 9;
    let germany = CountryRow {
        id: 7,
        canonical_slot: 1,
        tag: b"GER".to_vec(),
        ..CountryRow::default()
    };
    let links = BTreeMap::from([(7, &germany)]);
    let mut report = Report::default();
    assert!(
        compare_scientists(
            &frame,
            &scientist_pack(),
            &links,
            &BTreeMap::new(),
            &mut report
        )
        .is_err()
    );
}

fn raid_fixture() -> (DecodedFrame, saved::Pack, CountryRow) {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: replay_data::cadence::refresh_slot(TableId::ProjectPrograms).unwrap() as i32,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Default::default(),
    };
    let source = br"countries={GER={nukes={{amount=90071992547.40993 nukes_ready=-1}{amount=0 nukes_ready=0}} program_status={project_pool={project={}} breakthrough={}}}}
        raids={targets={target={valid=yes target={state=291} detected={ 1 0 }}} countries={{tag=GER priority=-1 raid_instance={{id={id=4294967295 type=84} phase=PREPARED prep_time=-2 distance=22.1 target={state=291}}} target_cooldowns={{target={state=291} type=oilfield_raid cooldown=3}}}}} peace_conference={}";
    let pack = saved::parse(&content_reader_parser::parser::parse(source).unwrap().items).unwrap();
    *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(vec![
        NukeRow {
            country: 102,
            nuke_type: 0,
            amount_raw: 9_007_199_254_740_993,
            ready: -1,
        },
        NukeRow {
            country: 102,
            nuke_type: 1,
            amount_raw: 0,
            ready: 0,
        },
    ]));
    *frame.tables[TableId::RaidCountries.index()] =
        TableView::Complete(TableRows::RaidCountries(vec![RaidCountryRow {
            canonical_slot: 0,
            country: 102,
            ..Default::default()
        }]));
    *frame.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(vec![RaidRow {
        reference_type: 84,
        id: u32::MAX,
        parent_slot: 0,
        ordinal: 0,
        phase: 3,
        outcome: 0,
        show_for: Vec::new(),
        unit: 0,
        air_wing: 0,
        travelled: Some(2_210_000),
        preparation_hours: -2,
        source: RaidSource {
            kind: None,
            kind_present: 0,
            province: 123,
            ship_reference_type: 51,
            ship_reference_id: u32::MAX,
            country: 102,
            building_type: 0x4c17,
            building_location: -1,
            building_country: 0,
        },
    }]));
    *frame.tables[TableId::RaidEquipment.index()] =
        TableView::Complete(TableRows::RaidEquipment(vec![
            replay_data::rows::RaidEquipmentRow {
                reference_type: 84,
                id: u32::MAX,
                phase: 3,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::RaidTargets.index()] =
        TableView::Complete(TableRows::RaidTargets(vec![RaidTargetRow {
            reference_type: 84,
            id: u32::MAX,
            scope: RaidScope {
                state: 291,
                ..Default::default()
            },
            raid_type: 0,
            detected: 0,
        }]));
    *frame.tables[TableId::RaidCooldowns.index()] =
        TableView::Complete(TableRows::RaidCooldowns(vec![RaidCooldownRow {
            owner_slot: 0,
            ordinal: 0,
            raid_type: 0x7777,
            days: 3,
            scope: RaidScope {
                state: 291,
                ..Default::default()
            },
        }]));
    *frame.tables[TableId::RaidTargetStatuses.index()] =
        TableView::Complete(TableRows::RaidTargetStatuses(vec![RaidTargetStatusRow {
            ordinal: 0,
            valid: 1,
            scope: RaidScope {
                state: 291,
                ..Default::default()
            },
            detected: vec![1, 0],
        }]));
    let country = CountryRow {
        id: 102,
        canonical_slot: 0,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    (frame, pack, country)
}

#[test]
fn raid_target_fields_are_compared_against_the_saved_target_block() {
    let (frame, pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    let mutate = |edit: &dyn Fn(&mut RaidTargetRow)| {
        let mut frame = frame.clone();
        let TableView::Complete(TableRows::RaidTargets(rows)) =
            &mut *frame.tables[TableId::RaidTargets.index()]
        else {
            panic!("fixture holds complete raid targets");
        };
        edit(&mut rows[0]);
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        report.differences
    };
    assert!(mutate(&|row| row.scope.state = 292)[0].contains("state"));
    assert!(mutate(&|row| row.detected = 1)[0].contains("detected"));
    assert!(mutate(&|row| row.scope.province = 7)[0].contains("province"));
    assert!(mutate(&|row| row.scope.leader_province = 7)[0].contains("leader_province"));
    assert!(mutate(&|row| row.scope.leader_reference = 7)[0].contains("leader"));
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

#[test]
fn raid_show_for_skips_the_entries_without_a_country_as_the_save_does() {
    let (frame, mut pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    pack.raids.as_mut().unwrap()[0].instances[0].show_for = vec![b"GER".to_vec()];
    let run = |show_for: Vec<u32>| {
        let mut frame = frame.clone();
        let TableView::Complete(TableRows::Raids(rows)) =
            &mut *frame.tables[TableId::Raids.index()]
        else {
            panic!("fixture holds complete raids");
        };
        rows[0].show_for = show_for;
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        report
    };
    let report = run(vec![102, 0]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.limitations["the save writes no show_for entry without a country; those entries are not compared"],
        ["Raid 18446744069414584404.show_for"]
    );
    assert!(run(vec![u32::MAX, 102]).differences.is_empty());
    assert!(run(vec![102]).limitations.keys().all(|reason| !reason.contains("show_for")));
    assert!(run(vec![102, 102]).differences[0].contains("show_for"));
    assert!(run(vec![0]).differences[0].contains("show_for"));
}

#[test]
fn raids_compare_the_travelled_distance_with_the_saved_one() {
    let (frame, pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    let run = |travelled| {
        let mut frame = frame.clone();
        let TableView::Complete(TableRows::Raids(rows)) =
            &mut *frame.tables[TableId::Raids.index()]
        else {
            panic!("fixture holds complete raids");
        };
        rows[0].travelled = travelled;
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        report
    };
    assert!(run(Some(2_210_000)).differences.is_empty());
    let mut prepared = frame.clone();
    let TableView::Complete(TableRows::Raids(rows)) = &mut *prepared.tables[TableId::Raids.index()]
    else {
        panic!("fixture holds complete raids");
    };
    rows[0].preparation_hours = -1;
    let mut report = Report::default();
    compare(&prepared, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert!(
        report.differences[0].contains("preparation_hours"),
        "{:?}",
        report.differences
    );
    assert!(run(Some(2_210_001)).differences[0].contains("travelled"));
    assert!(run(Some(0)).differences[0].contains("travelled"));
    let unread = run(None);
    assert!(unread.differences.is_empty());
    assert!(
        unread
            .limitations
            .keys()
            .any(|reason| reason.contains("travelled"))
    );
}

#[test]
fn a_building_source_compares_template_and_location() {
    let (mut frame, mut pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    let tokens: TokenNames<'static> = BTreeMap::from([(77, b"supply_node".as_slice())]);
    pack.raids.as_mut().unwrap()[0].instances[0].source = Some(saved::RaidSource {
        kind: 5,
        province: None,
        ship: None,
        country: Some(b"GER".to_vec()),
        building_template: Some(b"supply_node".to_vec()),
        building_location: Some(7301),
    });
    let run = |frame: &mut DecodedFrame, source: RaidSource| {
        let TableView::Complete(TableRows::Raids(rows)) =
            &mut *frame.tables[TableId::Raids.index()]
        else {
            unreachable!()
        };
        rows[0].source = source;
        let mut report = Report::default();
        compare(frame, &pack, &countries, &tokens, &mut report).unwrap();
        report.differences
    };
    let building = RaidSource {
        kind: Some(5),
        kind_present: 1,
        country: 102,
        building_type: 77,
        building_location: 7301,
        ..Default::default()
    };
    assert!(run(&mut frame, building.clone()).is_empty());
    let moved = run(
        &mut frame,
        RaidSource {
            building_location: 7302,
            ..building
        },
    );
    assert!(
        moved.len() == 1 && moved[0].contains("source_building_location"),
        "{moved:?}"
    );
    let unset = run(
        &mut frame,
        RaidSource {
            building_type: 0x4c17,
            ..building
        },
    );
    assert!(
        unset.len() == 1 && unset[0].contains("source_building"),
        "{unset:?}"
    );
}

#[test]
fn tooltip_operand_oracle_checks_references_ordinals_and_missing_census() {
    use engine_native_air::raid_tooltip_operands::*;
    use replay_data::rows::RaidTooltipOperandRow;
    let (mut frame, pack, _) = raid_fixture();
    let raid = u64::from(u32::MAX) << 32 | 84;
    let rows = vec![
        RaidTooltipOperandRow {
            raid,
            ordinal: 0,
            kind: REFERENCES,
            key: 0,
            raw: 0,
        },
        RaidTooltipOperandRow {
            raid,
            ordinal: 1,
            kind: REFERENCES,
            key: 1,
            raw: 0,
        },
    ];
    let compare = |frame: &DecodedFrame| {
        let mut report = Report::default();
        compare_raid_tooltip_operands(
            frame,
            &pack,
            &BTreeMap::new(),
            &Default::default(),
            &mut report,
        )
        .unwrap();
        report
    };
    *frame.tables[TableId::RaidTooltipOperands.index()] =
        TableView::Complete(TableRows::RaidTooltipOperands(rows.clone()));
    assert!(compare(&frame).differences.is_empty());
    let mut wrong = rows;
    wrong[1].raw = 1;
    wrong[1].ordinal = 3;
    *frame.tables[TableId::RaidTooltipOperands.index()] =
        TableView::Complete(TableRows::RaidTooltipOperands(wrong));
    assert_eq!(compare(&frame).differences.len(), 2);
    *frame.tables[TableId::RaidTooltipOperands.index()] =
        TableView::Complete(TableRows::RaidTooltipOperands(Vec::new()));
    assert_eq!(compare(&frame).differences.len(), 2);
}

#[test]
fn tooltip_operand_oracle_compares_wing_name_bytes_and_reports_capture_failures() {
    use engine_native_air::raid_tooltip_operands::*;
    use replay_data::rows::RaidTooltipOperandRow;
    let (mut frame, mut pack, _) = raid_fixture();
    let raid = pack.raids.as_mut().unwrap()[0]
        .instances
        .first_mut()
        .unwrap();
    raid.id = (6_u64 << 32) | 84;
    let id = raid.id;
    let wing = (7_u64 << 32) | 69;
    raid.air_wing = Some(wing);
    let name = b"A\x80BCDEFG\xffZ";
    let mut air = content_reader_save::air::Pack::default();
    air.wings.push(content_reader_save::air::Wing {
        id: wing,
        pool_type: b"fixture_plane".to_vec(),
        name: Some(name.to_vec()),
        accidents: None,
        experience: content_reader_save::Decimal::parse(b"0").unwrap(),
        equipment: content_reader_save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid: None,
    });
    let run = |frame: &mut DecodedFrame,
               air: &content_reader_save::air::Pack,
               name_rows: Vec<(u32, i64)>| {
        let mut rows = vec![
            (REFERENCES, 0, 0),
            (REFERENCES, 1, wing as i64),
            (NAME, ARMY_RESOLVED, 0),
            (NAME, WING_RESOLVED, 1),
        ];
        rows.extend(
            name_rows
                .into_iter()
                .map(|(key, raw)| (WING_NAME, key, raw)),
        );
        *frame.tables[TableId::RaidTooltipOperands.index()] =
            TableView::Complete(TableRows::RaidTooltipOperands(
                rows.into_iter()
                    .enumerate()
                    .map(|(ordinal, (kind, key, raw))| RaidTooltipOperandRow {
                        raid: id,
                        ordinal: ordinal as u32,
                        kind,
                        key,
                        raw,
                    })
                    .collect(),
            ));
        let mut report = Report::default();
        compare_raid_tooltip_operands(frame, &pack, &BTreeMap::new(), air, &mut report).unwrap();
        report
    };
    let chunks = |name: &[u8]| {
        let mut rows = vec![(u32::MAX, name.len() as i64)];
        rows.extend(name.chunks(8).enumerate().map(|(index, chunk)| {
            let mut padded = [0; 8];
            padded[..chunk.len()].copy_from_slice(chunk);
            (index as u32, i64::from_le_bytes(padded))
        }));
        rows
    };
    let exact = chunks(name);
    let report = run(&mut frame, &air, exact.clone());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        !report
            .limitations
            .keys()
            .any(|reason| reason.contains("wing name"))
    );
    let mut mismatch = exact.clone();
    mismatch[1].1 ^= 1;
    assert_eq!(run(&mut frame, &air, mismatch).differences.len(), 1);
    let mut length = exact.clone();
    length[0].1 += 1;
    assert_eq!(run(&mut frame, &air, length).differences.len(), 1);
    let mut missing = exact.clone();
    missing.pop();
    assert_eq!(run(&mut frame, &air, missing).differences.len(), 1);
    let mut padding = exact;
    padding.last_mut().unwrap().1 |= 1 << 24;
    assert_eq!(run(&mut frame, &air, padding).differences.len(), 1);
    for code in -5..=-1 {
        let report = run(&mut frame, &air, vec![(u32::MAX, code)]);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let failures: Vec<_> = report
            .limitations
            .iter()
            .filter(|(reason, _)| reason.contains("raid wing name unavailable"))
            .collect();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].1.len(), 1);
        assert!(failures[0].0.contains(wing_name_error(code)));
    }
    assert_eq!(run(&mut frame, &air, Vec::new()).differences.len(), 3);
    air.wings[0].name = Some(Vec::new());
    assert!(run(&mut frame, &air, chunks(b"")).differences.is_empty());
    air.wings[0].name = None;
    let report = run(&mut frame, &air, chunks(b""));
    assert_eq!(report.differences.len(), 1);
    air.wings.push(air.wings[0].clone());
    let duplicate = run(&mut frame, &air, chunks(b""));
    assert_eq!(duplicate.differences.len(), 1);
    assert!(format!("{:?}", duplicate.differences).contains("duplicate saved wing ID"));

    let mut second = pack.raids.as_ref().unwrap()[0].instances[0].clone();
    second.id = (8_u64 << 32) | 84;
    second.air_wing = Some((8_u64 << 32) | 69);
    pack.raids.as_mut().unwrap()[0]
        .instances
        .push(second.clone());
    let mut other_wing = air.wings[0].clone();
    other_wing.id = second.air_wing.unwrap();
    other_wing.name = Some(b"Other".to_vec());
    air.wings.push(other_wing);
    let TableView::Complete(TableRows::RaidTooltipOperands(rows)) =
        &mut *frame.tables[TableId::RaidTooltipOperands.index()]
    else {
        panic!("missing name rows")
    };
    let mut other_rows = rows.clone();
    other_rows.retain(|row| row.kind != WING_NAME);
    for row in &mut other_rows {
        row.raid = second.id;
        if row.kind == REFERENCES && row.key == 1 {
            row.raw = second.air_wing.unwrap() as i64;
        }
    }
    other_rows.extend(
        chunks(b"Wrong")
            .into_iter()
            .enumerate()
            .map(|(index, (key, raw))| RaidTooltipOperandRow {
                raid: second.id,
                ordinal: 4 + index as u32,
                kind: WING_NAME,
                key,
                raw,
            }),
    );
    rows.extend(other_rows);
    let mut isolated = Report::default();
    compare_raid_tooltip_operands(&frame, &pack, &BTreeMap::new(), &air, &mut isolated).unwrap();
    assert_eq!(isolated.differences.len(), 2, "{:?}", isolated.differences);
}

#[test]
fn raid_equipment_needs_a_row_for_every_saved_recorded_raid() {
    let (mut frame, mut pack, _) = raid_fixture();
    let raid = &mut pack.raids.as_mut().unwrap()[0].instances[0];
    raid.nukes = 2;
    raid.equipment = Some(content_reader_save::production::Pool {
        equipment: vec![content_reader_save::production::Equipment {
            variant: (41 << 32) | 70,
            amount: content_reader_save::Decimal::parse(b"5").unwrap(),
        }],
        allow_zero_entries: Some(false),
    });
    *frame.tables[TableId::RaidEquipment.index()] =
        TableView::Complete(TableRows::RaidEquipment(Vec::new()));
    let mut report = Report::default();
    compare_raid_equipment(&frame, &pack, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("absent from replay"));
}

#[test]
fn raid_censuses_follow_the_readers_rules_over_the_saved_raids() {
    use content_reader_save::{GameDate, SavedDate};
    use replay_data::{rows::RaidEquipmentRow, rows::RaidSpeedInputRow};
    let (mut frame, pack, _) = raid_fixture();
    frame.hour = GameDate::parse(b"1943.4.22.7")
        .unwrap()
        .engine_hour()
        .unwrap();
    let cases = [
        (3, 0, None, true, false),
        (4, 0, None, true, true),
        (5, 3, Some("1943.4.22.6"), true, true),
        (5, 3, Some("1943.4.22.5"), false, false),
        (5, 5, Some("1943.4.22.7"), false, false),
        (5, 3, None, false, false),
    ];
    for (phase, outcome, end, equipment, speed) in cases {
        let mut pack = pack.clone();
        let raid = &mut pack.raids.as_mut().unwrap()[0].instances[0];
        raid.phase = phase;
        raid.outcome = outcome;
        raid.end_date = end.map(|text| SavedDate::parse(text.as_bytes()).unwrap());
        let end_hour = end.map(|text| {
            GameDate::parse(text.as_bytes())
                .unwrap()
                .engine_hour()
                .unwrap()
        });
        let run = |equipment_row: bool, speed_row: bool, recorded_end: Option<i32>| {
            let mut frame = frame.clone();
            *frame.tables[TableId::RaidEquipment.index()] =
                TableView::Complete(TableRows::RaidEquipment(
                    std::iter::once(RaidEquipmentRow {
                        reference_type: 84,
                        id: u32::MAX,
                        phase,
                        end_hour: recorded_end,
                        ..Default::default()
                    })
                    .filter(|_| equipment_row)
                    .collect(),
                ));
            *frame.tables[TableId::RaidSpeedInputs.index()] =
                TableView::Complete(TableRows::RaidSpeedInputs(
                    std::iter::once(RaidSpeedInputRow {
                        reference_type: 84,
                        id: u32::MAX,
                        phase,
                        outcome,
                        end_hour: recorded_end,
                        ..Default::default()
                    })
                    .filter(|_| speed_row)
                    .collect(),
                ));
            let mut equipment_report = Report::default();
            compare_raid_equipment(&frame, &pack, &mut equipment_report).unwrap();
            let mut speed_report = Report::default();
            compare_raid_speed_inputs(&frame, &pack, &mut speed_report).unwrap();
            (equipment_report.differences, speed_report.differences)
        };
        let case = format!("phase {phase} outcome {outcome} end {end:?}");
        let (equipment_differences, speed_differences) = run(equipment, speed, end_hour);
        assert!(
            equipment_differences.is_empty() && speed_differences.is_empty(),
            "{case}: {equipment_differences:?} {speed_differences:?}"
        );
        let (equipment_differences, speed_differences) = run(!equipment, !speed, end_hour);
        assert_eq!(
            (equipment_differences.len(), speed_differences.len()),
            (1, 1),
            "{case}: {equipment_differences:?} {speed_differences:?}"
        );
        if equipment && speed {
            let shifted = end_hour.map_or(Some(0), |hour| Some(hour + 1));
            let (equipment_differences, speed_differences) = run(true, true, shifted);
            assert!(
                equipment_differences[0].contains("end_hour")
                    && speed_differences[0].contains("end_hour"),
                "{case}: {equipment_differences:?} {speed_differences:?}"
            );
        }
    }
}

#[test]
fn raid_equipment_compares_the_saved_pool_and_nukes() {
    use replay_data::rows::RaidEquipmentRow;
    let (mut frame, mut pack, _) = raid_fixture();
    let raid = &mut pack.raids.as_mut().unwrap()[0].instances[0];
    raid.nukes = 1;
    raid.equipment = Some(content_reader_save::production::Pool {
        equipment: vec![content_reader_save::production::Equipment {
            variant: (41 << 32) | 70,
            amount: content_reader_save::Decimal::parse(b"5").unwrap(),
        }],
        allow_zero_entries: Some(false),
    });
    let run = |frame: &mut DecodedFrame, amounts: Vec<i64>, nukes| {
        *frame.tables[TableId::RaidEquipment.index()] =
            TableView::Complete(TableRows::RaidEquipment(vec![RaidEquipmentRow {
                reference_type: 84,
                id: u32::MAX,
                phase: 3,
                variants: vec![(41 << 32) | 70, (42 << 32) | 70][..amounts.len()].to_vec(),
                amounts,
                nukes,
                ..Default::default()
            }]));
        let mut report = Report::default();
        compare_raid_equipment(frame, &pack, &mut report).unwrap();
        report.differences
    };
    assert!(run(&mut frame, vec![500_000, 0], 1).is_empty());
    assert!(run(&mut frame, vec![500_001], 1)[0].contains("amount"));
    assert!(run(&mut frame, vec![500_000], 0)[0].contains("nukes"));
}

const RIFLE: u64 = (41 << 32) | 70;
const TRUCK: u64 = (42 << 32) | 70;
const BOUND: i64 = 0x53e2_d620_fb40;

fn totals_fixture(items: &[(u64, &str)], definitions: bool) -> (DecodedFrame, saved::Pack) {
    use content_reader_save::{
        Decimal,
        production::{Equipment, Pool},
    };
    use replay_data::rows::VariantDefinitionRow;
    let (mut frame, mut pack, _) = raid_fixture();
    pack.raids.as_mut().unwrap()[0].instances[0].equipment = Some(Pool {
        equipment: items
            .iter()
            .map(|&(variant, amount)| Equipment {
                variant,
                amount: Decimal::parse(amount.as_bytes()).unwrap(),
            })
            .collect(),
        allow_zero_entries: Some(false),
    });
    if definitions {
        *frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(
                [RIFLE, TRUCK]
                    .map(|id| VariantDefinitionRow {
                        id,
                        archetype_token: 0x3309,
                        ..Default::default()
                    })
                    .to_vec(),
            ));
    }
    (frame, pack)
}

fn run_totals(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    items: &[(u64, i64)],
    totals: &[(u32, i64)],
) -> Report {
    let mut frame = frame.clone();
    *frame.tables[TableId::RaidEquipment.index()] =
        TableView::Complete(TableRows::RaidEquipment(vec![
            replay_data::rows::RaidEquipmentRow {
                reference_type: 84,
                id: u32::MAX,
                phase: 3,
                variants: items.iter().map(|&(variant, _)| variant).collect(),
                amounts: items.iter().map(|&(_, amount)| amount).collect(),
                archetype_tokens: totals.iter().map(|&(token, _)| token).collect(),
                archetype_amounts: totals.iter().map(|&(_, total)| total).collect(),
                ..Default::default()
            },
        ]));
    let mut report = Report::default();
    compare_raid_equipment(&frame, pack, &mut report).unwrap();
    report
}

#[test]
fn raid_equipment_totals_match_the_saved_sums_by_archetype() {
    let items = [(RIFLE, 500_000), (TRUCK, 200_000)];
    let (frame, pack) = totals_fixture(&[(RIFLE, "5"), (TRUCK, "2")], true);
    let report = run_totals(&frame, &pack, &items, &[(0x3309, 700_000), (0x330a, 0)]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        !report
            .compared_tables
            .contains_key("RaidEquipment.archetype_amounts"),
        "{:?}",
        report.compared_tables
    );
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("1 totals consistent with saved variant sums")),
        "{:?}",
        report.limitations
    );
    let (mut frame, pack) = totals_fixture(&[(RIFLE, "5"), (TRUCK, "2")], false);
    let report = run_totals(&frame, &pack, &items, &[(0x3309, 1)]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no VariantDefinitions row"))
    );
    *frame.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let report = run_totals(&frame, &pack, &items, &[(0x3309, 1)]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("VariantDefinitions is not complete"))
    );
}

#[test]
fn raid_equipment_totals_that_disagree_away_from_the_bound_are_differences() {
    let items = [(RIFLE, 500_000), (TRUCK, 200_000)];
    let (frame, pack) = totals_fixture(&[(RIFLE, "5"), (TRUCK, "2")], true);
    let differences = |totals: &[(u32, i64)]| run_totals(&frame, &pack, &items, totals).differences;
    assert!(differences(&[])[0].contains("absent"));
    assert!(differences(&[(0x3309, 700_000), (0x3309, 700_000)])[0].contains("repeated"));
    assert!(differences(&[(0x3309, BOUND + 1)])[0].contains("archetype[13065]"));
    let off_by_one = run_totals(&frame, &pack, &items, &[(0x3309, 700_001)]);
    assert!(
        off_by_one.differences.is_empty(),
        "{:?}",
        off_by_one.differences
    );
    assert!(
        off_by_one
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("archetype[13065]")),
        "{:?}",
        off_by_one.limitations
    );
    let extra = run_totals(&frame, &pack, &items, &[(0x3309, 700_000), (0x330a, 1)]);
    assert!(extra.differences.is_empty(), "{:?}", extra.differences);
    assert!(
        extra
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("no saved variant")),
        "{:?}",
        extra.limitations
    );
}

#[test]
fn raid_equipment_totals_at_the_clamp_bound_are_a_limitation() {
    let at_bound = |report: &Report| {
        assert!(
            report
                .differences
                .iter()
                .all(|difference| !difference.contains("archetype")),
            "{:?}",
            report.differences
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("reaches the bound"))
        );
    };
    let (frame, pack) = totals_fixture(&[(RIFLE, "922337202"), (TRUCK, "2")], true);
    at_bound(&run_totals(
        &frame,
        &pack,
        &[(RIFLE, BOUND), (TRUCK, 200_000)],
        &[(0x3309, 1)],
    ));
    let (frame, pack) = totals_fixture(&[(RIFLE, "5"), (TRUCK, "2")], true);
    let items = [(RIFLE, 500_000), (TRUCK, 200_000)];
    at_bound(&run_totals(&frame, &pack, &items, &[(0x3309, BOUND)]));
    at_bound(&run_totals(&frame, &pack, &items, &[(0x3309, -BOUND)]));
    at_bound(&run_totals(
        &frame,
        &pack,
        &items,
        &[(0x3309, 700_000), (0x330a, BOUND)],
    ));
    let half = BOUND / 2;
    for (saved, entry) in [("461168601", half), ("-461168601", -half)] {
        let (frame, pack) = totals_fixture(&[(RIFLE, saved), (TRUCK, saved)], true);
        assert_eq!(2 * entry, BOUND * entry.signum());
        at_bound(&run_totals(
            &frame,
            &pack,
            &[(RIFLE, entry), (TRUCK, entry)],
            &[(0x3309, entry)],
        ));
    }
    let (frame, pack) = totals_fixture(&[(RIFLE, "600000000"), (TRUCK, "600000000")], true);
    at_bound(&run_totals(
        &frame,
        &pack,
        &[(RIFLE, 60_000_000_000_000), (TRUCK, 60_000_000_000_000)],
        &[(0x3309, 1)],
    ));
}

#[test]
fn unwound_raid_pool_saturation_is_reported_as_a_limitation_not_a_difference() {
    let clamp = |value: i128| value.clamp(-i128::from(BOUND), i128::from(BOUND)) as i64;
    let apply = |entries: &mut BTreeMap<u64, i64>, total: &mut i64, variant: u64, delta: i64| {
        let entry = entries.entry(variant).or_default();
        *entry = clamp(i128::from(*entry) + i128::from(delta));
        *total = clamp(i128::from(*total) + i128::from(delta));
    };

    let three_quarters = BOUND / 4 * 3;
    let mut entries = BTreeMap::new();
    let mut total = 0;
    apply(&mut entries, &mut total, RIFLE, three_quarters);
    apply(&mut entries, &mut total, TRUCK, three_quarters);
    assert_eq!(total, BOUND);
    apply(&mut entries, &mut total, RIFLE, -three_quarters);
    assert_eq!(entries[&RIFLE], 0);
    assert_eq!(entries[&TRUCK], three_quarters);
    assert_eq!(total, BOUND / 4);

    let (frame, pack) = totals_fixture(&[(TRUCK, "691752901.5")], true);
    let report = run_totals(
        &frame,
        &pack,
        &[(RIFLE, entries[&RIFLE]), (TRUCK, entries[&TRUCK])],
        &[(0x3309, total)],
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        !report
            .compared_tables
            .contains_key("RaidEquipment.archetype_amounts"),
        "{:?}",
        report.compared_tables
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("unwound saturation")),
        "{:?}",
        report.limitations
    );

    let false_match = run_totals(
        &frame,
        &pack,
        &[(RIFLE, entries[&RIFLE]), (TRUCK, entries[&TRUCK])],
        &[(0x3309, three_quarters)],
    );
    assert!(
        false_match.differences.is_empty(),
        "{:?}",
        false_match.differences
    );
    assert!(
        !false_match
            .compared_tables
            .contains_key("RaidEquipment.archetype_amounts"),
        "{:?}",
        false_match.compared_tables
    );

    apply(&mut entries, &mut total, TRUCK, -three_quarters);
    assert_eq!(entries[&TRUCK], 0);
    assert_eq!(total, -(BOUND / 2));
    let (frame, pack) = totals_fixture(&[], true);
    let report = run_totals(
        &frame,
        &pack,
        &[(RIFLE, 0), (TRUCK, 0)],
        &[(0x3309, total)],
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("unwound saturation")),
        "{:?}",
        report.limitations
    );

    let (frame, pack) = totals_fixture(&[(RIFLE, "5"), (TRUCK, "2")], true);
    let items = [(RIFLE, 500_000), (TRUCK, 200_000)];
    for impossible in [BOUND + 1, -BOUND - 1] {
        let report = run_totals(&frame, &pack, &items, &[(0x3309, impossible)]);
        assert_eq!(report.differences.len(), 1, "{impossible}: {:?}", report.differences);
    }
}

#[test]
fn raid_equipment_structural_checks_run_even_when_variant_definitions_are_missing() {
    use replay_data::rows::VariantDefinitionRow;
    let items = [(RIFLE, 500_000), (TRUCK, 200_000)];
    let (mut frame, pack) = totals_fixture(&[(RIFLE, "5"), (TRUCK, "2")], false);
    *frame.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    for pair in [
        [(0x3309, BOUND + 1), (0x3309, 100)],
        [(0x3309, 100), (0x3309, BOUND + 1)],
    ] {
        let unavailable = run_totals(&frame, &pack, &items, &pair);
        assert_eq!(
            unavailable.differences.len(),
            2,
            "{pair:?}: {:?}",
            unavailable.differences
        );
        assert!(
            unavailable
                .differences
                .iter()
                .any(|d| d.contains("within clamp bound")),
            "{pair:?}: {:?}",
            unavailable.differences
        );
        assert!(
            unavailable
                .differences
                .iter()
                .any(|d| d.contains("repeated")),
            "{pair:?}: {:?}",
            unavailable.differences
        );
        assert!(
            unavailable
                .limitations
                .keys()
                .any(|reason| reason.contains("VariantDefinitions is not complete")),
            "{pair:?}: {:?}",
            unavailable.limitations
        );
    }

    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: RIFLE,
            archetype_token: 0x3309,
            ..Default::default()
        }]));
    let partial = run_totals(
        &frame,
        &pack,
        &items,
        &[(0x330a, 100), (0x330a, BOUND + 1)],
    );
    assert_eq!(partial.differences.len(), 3, "{:?}", partial.differences);
    assert!(
        partial
            .differences
            .iter()
            .any(|d| d.contains("within clamp bound")),
        "{:?}",
        partial.differences
    );
    assert!(
        partial.differences.iter().any(|d| d.contains("repeated")),
        "{:?}",
        partial.differences
    );
    assert!(
        partial.differences.iter().any(|d| {
            d.contains("archetype[13065]")
                && d.contains("absent")
                && d.contains("present")
                && !d.contains("500000")
        }),
        "{:?}",
        partial.differences
    );
    assert!(
        partial
            .limitations
            .keys()
            .any(|reason| reason.contains("no VariantDefinitions row")),
        "{:?}",
        partial.limitations
    );
}

#[test]
fn building_flags_follow_the_saved_reference_branch() {
    let tokens: TokenNames<'static> = BTreeMap::from([(9, b"bunker".as_slice())]);
    let check = |flags, country, location| {
        let scope = RaidScope {
            building_type: 9,
            building_location: location as u32,
            building_flags: flags,
            building_country: country,
            ..Default::default()
        };
        let saved = saved::RaidTarget {
            building_template: Some(b"bunker".to_vec()),
            building_location: Some(i64::from(location)),
            ..Default::default()
        };
        let mut report = Report::default();
        compare_scope(&mut report, "scope", &tokens, &scope, &saved);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("the building location flags"))
        );
        report.differences.len()
    };
    assert_eq!(check(1, 0, 7), 0);
    assert_eq!(check(2, 0, 7), 0);
    assert_eq!(check(1, 5, 7), 0);
    assert_eq!(check(2, 5, 7), 1);
    assert_eq!(check(2, 5, -1), 0);
    assert_eq!(check(3, 5, -1), 1);
}

#[test]
fn raid_target_statuses_join_on_their_native_ordinal() {
    let (mut frame, pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    let run = |frame: &DecodedFrame| {
        let mut report = Report::default();
        compare(frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        report
    };
    let report = run(&frame);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["RaidTargetStatuses"], 1);
    let TableView::Complete(TableRows::RaidTargetStatuses(rows)) =
        &mut *frame.tables[TableId::RaidTargetStatuses.index()]
    else {
        unreachable!()
    };
    rows[0].ordinal = 1;
    let report = run(&frame);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert_eq!(report.compared_tables["RaidTargetStatuses"], 0);
}

#[test]
fn raid_path_verdict_uses_only_its_dispatchers_search() {
    assert_eq!(path_verdict(Some(5), &Ok(Some(5))), PathVerdict::Match);
    assert_eq!(path_verdict(None, &Ok(None)), PathVerdict::Match);
    assert_eq!(path_verdict(Some(5), &Ok(Some(4))), PathVerdict::Difference);
    assert_eq!(path_verdict(Some(5), &Ok(None)), PathVerdict::Difference);
    assert_eq!(path_verdict(None, &Ok(Some(0))), PathVerdict::Difference);
    assert_eq!(
        path_verdict(Some(5), &Err("undecidable".into())),
        PathVerdict::Undecided
    );
}

#[test]
fn raid_memo_oracle_checks_each_dispatcher_instead_of_either_path() {
    use replay_data::rows::{RaidPathDispatcher, RaidPathDistanceRow};
    let graph = content_reader_map::province_graph::ProvinceGraph {
        edges: BTreeMap::from([(1, vec![(2, Some(50_000))]), (2, vec![(3, Some(50_000))])]),
        ..Default::default()
    };
    let mut geometry = content_reader_map::province_geometry::ProvinceGeometry::default();
    geometry.width = 10;
    geometry.height = 1;
    geometry.centroids = BTreeMap::from([(1, [0, 0]), (2, [1, 0]), (3, [2, 0])]);
    geometry.land = BTreeSet::from([1, 3]);
    let mut frame = scientist_frame();
    let mut rows = vec![
        RaidPathDistanceRow {
            dispatcher: RaidPathDispatcher::Land,
            source: 1,
            target: 3,
            distance: 0,
            ok: 0,
        },
        RaidPathDistanceRow {
            dispatcher: RaidPathDispatcher::Naval,
            source: 1,
            target: 3,
            distance: 100_000,
            ok: 1,
        },
    ];
    frame.tables[TableId::RaidPathDistances.index()] =
        TableView::Complete(TableRows::RaidPathDistances(rows.clone())).into();
    let mut report = Report::default();
    compare_raid_paths(&frame, &(graph.clone(), geometry.clone()), &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["RaidPathDistances"], 2);
    rows[0].distance = 100_000;
    rows[0].ok = 1;
    frame.tables[TableId::RaidPathDistances.index()] =
        TableView::Complete(TableRows::RaidPathDistances(rows)).into();
    let mut report = Report::default();
    compare_raid_paths(&frame, &(graph, geometry), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn comparison_keeps_fixed_precision_and_does_not_certify_omitted_raid_source() {
    let (mut frame, pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("suppresses the whole saved source"))
    );
    let TableView::Complete(TableRows::Nukes(rows)) = &mut *frame.tables[TableId::Nukes.index()]
    else {
        unreachable!()
    };
    rows[0].amount_raw -= 1;
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    *frame.tables[TableId::Raids.index()] = TableView::Unavailable {
        reason: "source unavailable".into(),
    };
    assert!(
        compare(
            &frame,
            &pack,
            &countries,
            &BTreeMap::new(),
            &mut Report::default()
        )
        .is_err()
    );
}

#[test]
fn unavailable_tables_keep_independent_comparisons_and_differences() {
    let (mut frame, pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    for table in [TableId::Nukes, TableId::Projects, TableId::RaidCountries] {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: format!("{table:?} source unavailable"),
        };
    }
    let TableView::Complete(TableRows::Raids(rows)) = &mut *frame.tables[TableId::Raids.index()]
    else {
        unreachable!()
    };
    rows[0].phase += 1;
    let mut report = Report::default();
    assert!(compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).is_err());
    assert_eq!(report.failed_comparisons.len(), 3);
    for table in ["Nukes", "Projects", "RaidCountries"] {
        assert!(
            report
                .failed_comparisons
                .contains_key(&format!("projects {table}"))
        );
        assert!(!report.compared_tables.contains_key(table));
    }
    for (table, count) in [
        ("ProjectPrograms", 0),
        ("ProjectHistory", 0),
        ("ProjectBreakthroughs", 0),
        ("Raids", 1),
        ("PeaceConferences", 0),
    ] {
        assert_eq!(report.compared_tables.get(table), Some(&count), "{table}");
    }
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("phase"));
}

#[test]
fn raid_type_word_changes_membership_and_present_source_changes_fail() {
    let (mut frame, mut pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    pack.raids.as_mut().unwrap()[0].instances[0].source = Some(saved::RaidSource {
        kind: 3,
        province: Some(9),
        ship: Some(51),
        country: Some(b"GER".to_vec()),
        building_template: None,
        building_location: None,
    });
    let TableView::Complete(TableRows::Raids(rows)) = &mut *frame.tables[TableId::Raids.index()]
    else {
        unreachable!()
    };
    let row = &mut rows[0];
    row.source = RaidSource {
        kind: Some(3),
        kind_present: 1,
        province: 9,
        ship_reference_type: 51,
        ship_reference_id: 0,
        country: 102,
        building_type: 0x4c17,
        building_location: -1,
        building_country: 0,
    };
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let TableView::Complete(TableRows::Raids(rows)) = &mut *frame.tables[TableId::Raids.index()]
    else {
        unreachable!()
    };
    let row = &mut rows[0];
    row.source.province = 10;
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    let TableView::Complete(TableRows::Raids(rows)) = &mut *frame.tables[TableId::Raids.index()]
    else {
        unreachable!()
    };
    let row = &mut rows[0];
    row.reference_type ^= 1;
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn missing_tokens_limit_fields_while_presence_census_and_native_values_still_compare() {
    let (mut frame, mut pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    let program_id = (7_u64 << 32) | 83;
    let status = pack
        .countries
        .get_mut(b"GER".as_slice())
        .unwrap()
        .status
        .as_mut()
        .unwrap();
    status.programs.insert(
        program_id,
        saved::Program {
            id: program_id,
            country: Some(b"GER".to_vec()),
            province: 1,
            active_project: Some(b"definition".to_vec()),
            scientist: None,
            building: b"building".to_vec(),
            unread_rewards: -2,
            assignments: vec![saved::ScientistAssignment {
                scientist: 0,
                target: 0,
                current: 0,
                timed: false,
            }],
        },
    );
    status.program_order.push(program_id);
    *frame.tables[TableId::RaidFacilities.index()] =
        TableView::Complete(TableRows::RaidFacilities(vec![
            replay_data::rows::RaidFacilityRow {
                country: 102,
                ordinal: 0,
                id: 7,
                owner: 102,
                province: 1,
                active_project: Some(100),
                project_definition: None,
            },
        ]));
    status
        .breakthroughs
        .insert(b"specialization".to_vec(), i32::MIN);
    *frame.tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
            id: 7,
            owner: 102,
            province: 1,
            active_project: Some(100),
            scientist: 0,
            unread_rewards: -2,
            building: b"building".to_vec(),
            facility: replay_data::rows::Recorded::Value(None),
            assignments: replay_data::rows::Recorded::Value(Some(vec![
                replay_data::rows::ProjectScientistAssignment::default(),
            ])),
        }]));
    *frame.tables[TableId::ProjectBreakthroughs.index()] =
        TableView::Complete(TableRows::ProjectBreakthroughs(vec![
            ProjectBreakthroughRow {
                owner: 102,
                specialization: 101,
                points: i32::MIN,
            },
        ]));
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("active_project"))
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("points cannot be matched"))
    );
    let TableView::Complete(TableRows::ProjectPrograms(rows)) =
        &mut *frame.tables[TableId::ProjectPrograms.index()]
    else {
        unreachable!()
    };
    rows[0].unread_rewards = -1;
    rows[0].active_project = None;
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    let tokens = BTreeMap::from([
        (100, b"definition".as_slice()),
        (101, b"wrong_specialization".as_slice()),
    ]);
    let mut report = Report::default();
    compare_breakthroughs(&frame, &pack, &countries, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    *frame.tables[TableId::ProjectBreakthroughs.index()] =
        TableView::Complete(TableRows::ProjectBreakthroughs(Vec::new()));
    let mut report = Report::default();
    compare_breakthroughs(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("count"))
    );
}

#[test]
fn populated_dictionary_checks_project_names_and_breakthrough_points() {
    let (mut frame, mut pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    let source = br"countries={GER={program_status={
        program={id={id=7 type=83} country=GER province=1 scientist={}
            building=building unread_prototype_rewards=-2 project=definition}
        project_pool={project={definition={id={id=9 type=86}
            progress={current_state=1 prototype_state={phase_progress=-0.00001 project_progress=-2 iterations=-3} stopping_state={phase_progress=0}}
            start=yes completed=no program={id=7 type=83}
            history={{context={province=-1 progress=-2} option=option prototype_reward=reward date=1936.1.1.1}}
        }}}
        breakthrough={specialization=-2147483648}
    }}}";
    let mut parsed = saved::parse(&content_reader_parser::parser::parse(source).unwrap().items).unwrap();
    pack.countries.get_mut(b"GER".as_slice()).unwrap().status =
        parsed.countries.remove(b"GER".as_slice()).unwrap().status;
    let definition = 0x1234;
    let reward = 0x1235;
    let option = 0x1236;
    let specialization = i32::MAX as u32;
    *frame.tables[TableId::RaidFacilities.index()] =
        TableView::Complete(TableRows::RaidFacilities(vec![
            replay_data::rows::RaidFacilityRow {
                country: 102,
                ordinal: 0,
                id: 7,
                owner: 102,
                province: 1,
                active_project: Some(definition),
                project_definition: Some(definition),
            },
        ]));
    *frame.tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
            id: 7,
            owner: 102,
            province: 1,
            active_project: Some(definition),
            scientist: 0,
            unread_rewards: -2,
            building: b"building".to_vec(),
            facility: replay_data::rows::Recorded::Value(None),
            assignments: replay_data::rows::Recorded::Value(Some(vec![
                replay_data::rows::ProjectScientistAssignment::default(),
            ])),
        }]));
    *frame.tables[TableId::Projects.index()] =
        TableView::Complete(TableRows::Projects(vec![ProjectRow {
            owner: 102,
            local_id: 9,
            definition_id: definition,
            program_reference: (7_u64 << 32) | 83,
            started: 1,
            completed: 0,
            state: ProjectState {
                current_offset: 0x18,
                state_enums: [1, 2, 3, 4],
                not_completed: 1,
                phase_progress: -1,
                phase_target: -4,
                prototype_progress: -2,
                iterations: -3,
            },
            runtime: replay_data::rows::Recorded::Value(replay_data::rows::ProjectRuntime {
                pending_known: true,
                pending_reward: None,
                pending_ordinal: None,
                variables: Some(
                    replay_data::rows::ProjectVariableStore {
                        seed: 1,
                        entries: Vec::new(),
                    }
                    .into(),
                ),
            }),
        }]));
    *frame.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
            owner: 102,
            project: 9,
            ordinal: 0,
            prototype_reward_handle: reward,
            option_handle: option,
            date: content_reader_save::GameDate::parse(b"1936.1.1.1")
                .unwrap()
                .engine_hour()
                .unwrap(),
            context: ProjectContext {
                province: -1,
                progress: -2,
                ..Default::default()
            },
        }]));
    *frame.tables[TableId::ProjectBreakthroughs.index()] =
        TableView::Complete(TableRows::ProjectBreakthroughs(vec![
            ProjectBreakthroughRow {
                owner: 102,
                specialization,
                points: i32::MIN,
            },
        ]));
    let tokens = BTreeMap::from([
        (definition, b"definition".as_slice()),
        (reward, b"reward".as_slice()),
        (option, b"option".as_slice()),
        (specialization, b"specialization".as_slice()),
        (0x7777, b"oilfield_raid".as_slice()),
    ]);
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        !report
            .limitations
            .keys()
            .any(|reason| reason.contains("token"))
    );
    for table in [
        "ProjectPrograms",
        "Projects",
        "ProjectHistory",
        "ProjectBreakthroughs",
    ] {
        assert_eq!(report.compared_tables.get(table), Some(&1), "{table}");
    }
    for (token, differences, field) in [
        (definition, 4, ".definition"),
        (reward, 1, ".reward"),
        (option, 1, ".option"),
        (specialization, 2, "membership"),
    ] {
        let mut wrong = tokens.clone();
        wrong.insert(token, b"wrong_literal");
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &wrong, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(field))
        );
    }
    let TableView::Complete(TableRows::ProjectBreakthroughs(rows)) =
        &mut *frame.tables[TableId::ProjectBreakthroughs.index()]
    else {
        unreachable!()
    };
    rows[0].points += 1;
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("ProjectBreakthrough"));
    let mut limited = tokens.clone();
    limited.insert(option, b"option\0suffix");
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &limited, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("ProjectBreakthrough"));
    assert_eq!(report.compared_tables.get("ProjectHistory"), Some(&1));
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("interior NUL"))
    );
}

#[test]
fn optional_token_presence_is_not_inferred_from_zero_or_empty_literal() {
    let tokens = BTreeMap::from([(0, b"".as_slice())]);
    let mut report = Report::default();
    compare_optional_token(&mut report, "active_project", &tokens, Some(0), Some(b""));
    compare_optional_token(&mut report, "active_project", &tokens, None, None);
    assert!(report.differences.is_empty());
    assert!(report.limitations.is_empty());
    compare_optional_token(&mut report, "active_project", &tokens, Some(0), None);
    compare_optional_token(&mut report, "active_project", &tokens, None, Some(b""));
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .all(|difference| difference.contains(".present"))
    );
}

#[test]
fn hourly_facilities_compare_order_duplicates_and_unresolved_project_tokens() {
    use replay_data::rows::RaidFacilityRow;
    let (mut frame, mut pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    let source = br"countries={GER={program_status={
        program={id={id=7 type=83} country=GER province=1 scientist={}
            building=building unread_prototype_rewards=0 project=definition}
        program={id={id=7 type=83} country=GER province=1 scientist={}
            building=building unread_prototype_rewards=0 project=definition}
        project_pool={project={}} breakthrough={}
    }}}";
    let mut parsed = saved::parse(&content_reader_parser::parser::parse(source).unwrap().items).unwrap();
    pack.countries.get_mut(b"GER".as_slice()).unwrap().status =
        parsed.countries.remove(b"GER".as_slice()).unwrap().status;
    let original: Vec<_> = (0..2)
        .map(|ordinal| RaidFacilityRow {
            country: 102,
            ordinal,
            id: 7,
            owner: 102,
            province: 1,
            active_project: Some(0),
            project_definition: None,
        })
        .collect();
    let tokens = BTreeMap::from([(0, b"definition".as_slice())]);
    let mut check = |rows| {
        *frame.tables[TableId::RaidFacilities.index()] =
            TableView::Complete(TableRows::RaidFacilities(rows));
        let mut report = Report::default();
        compare_facilities(&frame, &pack, &countries, &tokens, &mut report).unwrap();
        report
    };
    assert!(check(original.clone()).differences.is_empty());
    for mutate in [
        |r: &mut RaidFacilityRow| r.ordinal = 3,
        |r: &mut RaidFacilityRow| r.id = 8,
        |r: &mut RaidFacilityRow| r.province = 2,
        |r: &mut RaidFacilityRow| r.active_project = None,
        |r: &mut RaidFacilityRow| r.project_definition = Some(0),
    ] {
        let mut rows = original.clone();
        mutate(&mut rows[0]);
        assert!(!check(rows).differences.is_empty());
    }
    let mut missing = original.clone();
    missing.pop();
    assert!(!check(missing).differences.is_empty());
    let mut extra = original;
    extra.push(extra[0].clone());
    assert!(!check(extra).differences.is_empty());
}

#[test]
fn exceptional_lexical_tokens_do_not_claim_a_saved_text_mapping() {
    let tokens = BTreeMap::from([
        (1, b"prefix\0suffix".as_slice()),
        (0x167, b"32768".as_slice()),
    ]);
    let mut report = Report::default();
    compare_token(&mut report, "nul", &tokens, 1, b"prefix");
    compare_token(&mut report, "reserved", &tokens, 0x167, b"32768");
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.len(), 2);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("interior NUL"))
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("fixed15"))
    );
    assert_eq!(tokens[&1], b"prefix\0suffix");
}

#[test]
fn peace_turn_reconstruction_never_picks_arbitrary_tag_order() {
    let mut conference = saved::Conference {
        id: 1,
        ordinal: 0,
        finalized: false,
        history_turns: BTreeMap::new(),
        bids: BTreeMap::new(),
        bid_countries: Vec::new(),
    };
    assert_eq!(reconstructed_turn(&conference), Some(0));
    conference.history_turns.insert(b"GER".to_vec(), 4);
    conference.history_turns.insert(b"ENG".to_vec(), 4);
    assert_eq!(reconstructed_turn(&conference), Some(4));
    conference.history_turns.insert(b"SOV".to_vec(), 5);
    assert_eq!(reconstructed_turn(&conference), None);
}

#[test]
fn facility_flags_must_lie_on_saved_program_provinces() {
    let (mut frame, mut pack, _) = raid_fixture();
    let status = pack
        .countries
        .get_mut(b"GER".as_slice())
        .unwrap()
        .status
        .as_mut()
        .unwrap();
    for (id, province) in [(1, 1), (2, 3)] {
        status.programs.insert(
            id,
            saved::Program {
                id,
                country: None,
                province,
                active_project: None,
                scientist: None,
                building: Vec::new(),
                unread_rewards: 0,
                assignments: vec![saved::ScientistAssignment {
                    scientist: 0,
                    target: 0,
                    current: 0,
                    timed: false,
                }],
            },
        );
    }
    let flag = |province| replay_data::rows::FacilityMapFlagRow { province, flag: 1 };
    *frame.tables[TableId::FacilityMapFlags.index()] =
        TableView::Complete(TableRows::FacilityMapFlags(vec![flag(1), flag(5)]));
    let mut report = Report::default();
    compare_facility_flags(&frame, &pack, &mut report).unwrap();
    assert_eq!(report.compared_tables["FacilityMapFlags"], 2);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("1 saved program provinces unflagged"))
    );
}

#[test]
fn staggered_off_hours_skips_projects_and_prints_refresh_hour() {
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};
    let frame = DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    let mut report = Report::default();
    let _ = compare(
        &frame,
        &saved::Pack::default(),
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        &mut report,
    );
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    let (_, stale) = report
        .limitations
        .iter()
        .find(|(reason, _)| reason.contains(replay_data::cadence::STAGGER_LIMITATION))
        .unwrap_or_else(|| panic!("{:?}", report.limitations.keys()));
    assert_eq!(stale.len(), 2, "{stale:?}");
    assert!(stale[0].starts_with("ProjectPrograms "), "{stale:?}");
    assert!(stale[1].starts_with("ProjectBreakthroughs "), "{stale:?}");
}
