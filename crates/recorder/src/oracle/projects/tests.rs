use super::*;
use replay::{
    frame::TableView,
    rows::{
        NukeRow, ProjectBreakthroughRow, ProjectContext, ProjectHistoryRow, ProjectProgramRow,
        ProjectRow, ProjectState, RaidCooldownRow, RaidCountryRow, RaidRow, RaidScope, RaidSource,
        RaidTargetRow, RaidTargetStatusRow, TableId, TableRows,
    },
};

fn scientist_pack() -> characters_saved::Pack {
    characters_saved::parse(
        &installed_content::parser::parse(
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
        hour: replay::cadence::refresh_slot(TableId::Projects).unwrap() as i32,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Default::default(),
    };
    *frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::Scientists(vec![replay::rows::ScientistRow {
            character: 265,
            country: 7,
            ordinal: 0,
            trait_count: 2,
            skill_count: 2,
            active: replay::rows::Recorded::Value(Some(0)),
        }]));
    *frame.tables[TableId::ScientistSkills.index()] =
        TableView::Complete(TableRows::ScientistSkills(vec![
            replay::rows::ScientistSkillRow {
                character: 265,
                ordinal: 0,
                token: 0x56c6,
                level: 2,
                experience_raw: 0,
            },
            replay::rows::ScientistSkillRow {
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
fn a_matching_scientist_roster_compares_counts_tokens_and_levels() {
    let report = run_scientists(&scientist_frame());
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["Scientists"], 1);
    assert_eq!(report.compared_tables["ScientistSkills"], 2);
    // The naval record's omitted level proves only a nonpositive value.
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
    rows[0].active = replay::rows::Recorded::Value(Some(1));
    let report = run_scientists(&frame);
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("scientist 265.active"));
    let TableView::Complete(TableRows::Scientists(rows)) =
        &mut *frame.tables[TableId::Scientists.index()]
    else {
        unreachable!()
    };
    rows[0].active = replay::rows::Recorded::Value(Some(0));
    assert!(run_scientists(&frame).differences.is_empty());
    let TableView::Complete(TableRows::Scientists(rows)) =
        &mut *frame.tables[TableId::Scientists.index()]
    else {
        unreachable!()
    };
    rows[0].active = replay::rows::Recorded::Value(None);
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
    rows[0].active = replay::rows::Recorded::NotRecorded;
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
                // A positive experience where the writer omitted the field.
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
                // Zero is the engine's own "no country", which the save omits.
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
    // Character 266 has no saved country, so no roster row is required.
    let report = run_scientists(&scientist_frame());
    assert!(report.differences.is_empty());
    // Recording it anyway is compared against its saved character, which
    // has no country tag at all.
    let mut frame = scientist_frame();
    let TableView::Complete(TableRows::Scientists(rows)) =
        &mut *frame.tables[TableId::Scientists.index()]
    else {
        unreachable!()
    };
    rows.push(replay::rows::ScientistRow {
        character: 266,
        country: 7,
        ordinal: 1,
        trait_count: 0,
        skill_count: 1,
        active: replay::rows::Recorded::Value(Some(0)),
    });
    let TableView::Complete(TableRows::ScientistSkills(skills)) =
        &mut *frame.tables[TableId::ScientistSkills.index()]
    else {
        unreachable!()
    };
    skills.push(replay::rows::ScientistSkillRow {
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
        // The project family's refresh hour.
        hour: replay::cadence::refresh_slot(TableId::Projects).unwrap() as i32,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Default::default(),
    };
    let source = br"countries={GER={nukes={{amount=90071992547.40993 nukes_ready=-1}{amount=0 nukes_ready=0}} program_status={project_pool={project={}} breakthrough={}}}}
        raids={targets={target={valid=yes target={state=291} detected={ 1 0 }}} countries={{tag=GER priority=-1 raid_instance={{id={id=4294967295 type=84} phase=PREPARED prep_time=-2 distance=22.1 target={state=291}}} target_cooldowns={{target={state=291} type=oilfield_raid cooldown=3}}}}} peace_conference={}";
    let pack = saved::parse(&installed_content::parser::parse(source).unwrap().items).unwrap();
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
    // The prepared raid's pool, which is empty in the save.
    *frame.tables[TableId::RaidEquipment.index()] =
        TableView::Complete(TableRows::RaidEquipment(vec![
            replay::rows::RaidEquipmentRow {
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

/// Every RaidTargets field the save carries is compared: a changed target
/// arm, raid type or detected byte has to show as a difference.
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
    // An unchanged row is silent, so the assertions above cannot pass by
    // comparing nothing at all.
    let mut report = Report::default();
    compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

/// The travelled distance compares exactly with the saved `distance`
/// (raw / 100000); an absent one is zero, and an unread one is named.
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
    // The saved prep_time is compared exactly.
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

/// A building source compares its template and location with the saved
/// `raid_source.building` (the live save's supply node 7301); an unset
/// reference needs none saved.
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
            ..building.clone()
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
    use engine::raid_tooltip_operands::*;
    use replay::rows::RaidTooltipOperandRow;
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
    let mut wrong = rows.clone();
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
    use engine::raid_tooltip_operands::*;
    use replay::rows::RaidTooltipOperandRow;
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
    let mut air = installed_content::save::air::Pack::default();
    air.wings.push(installed_content::save::air::Wing {
        id: wing,
        pool_type: b"fixture_plane".to_vec(),
        name: Some(name.to_vec()),
        accidents: None,
        experience: installed_content::save::Decimal::parse(b"0").unwrap(),
        equipment: installed_content::save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid: None,
    });
    let run = |frame: &mut DecodedFrame,
               air: &installed_content::save::air::Pack,
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

/// A complete but empty RaidEquipment beside a saved prepared raid with
/// equipment and nukes is a difference: the census is checked from the
/// save's side too, not only from the recorded rows.
#[test]
fn raid_equipment_needs_a_row_for_every_saved_recorded_raid() {
    let (mut frame, mut pack, _) = raid_fixture();
    let raid = &mut pack.raids.as_mut().unwrap()[0].instances[0];
    raid.nukes = 2;
    raid.equipment = Some(installed_content::save::production::Pool {
        equipment: vec![installed_content::save::production::Equipment {
            variant: (41 << 32) | 70,
            amount: installed_content::save::Decimal::parse(b"5").unwrap(),
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

/// RaidEquipment and RaidSpeedInputs derive their censuses from the saved
/// phase, outcome and `end_date` by the readers' rules: prepared raids
/// only in RaidEquipment, raids in progress in both, and an ended, not
/// cancelled raid in both while its end hour is at most an hour before the
/// frame's. A missing row and a row for a raid outside the census are each
/// a difference, as is a recorded end hour other than the saved one.
#[test]
fn raid_censuses_follow_the_readers_rules_over_the_saved_raids() {
    use installed_content::save::{GameDate, SavedDate};
    use replay::rows::{RaidEquipmentRow, RaidSpeedInputRow};
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

/// The raid pool's visible entries and nuke count compare with the saved
/// `equipment` pool and `nukes`.
#[test]
fn raid_equipment_compares_the_saved_pool_and_nukes() {
    use replay::rows::RaidEquipmentRow;
    let (mut frame, mut pack, _) = raid_fixture();
    let raid = &mut pack.raids.as_mut().unwrap()[0].instances[0];
    raid.nukes = 1;
    raid.equipment = Some(installed_content::save::production::Pool {
        equipment: vec![installed_content::save::production::Equipment {
            variant: (41 << 32) | 70,
            amount: installed_content::save::Decimal::parse(b"5").unwrap(),
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
    // A hidden zero entry is not saved.
    assert!(run(&mut frame, vec![500_000, 0], 1).is_empty());
    assert!(run(&mut frame, vec![500_001], 1)[0].contains("amount"));
    assert!(run(&mut frame, vec![500_000], 0)[0].contains("nukes"));
}

const RIFLE: u64 = (41 << 32) | 70;
const TRUCK: u64 = (42 << 32) | 70;
/// Variant addition's clamp bound `L`, raw.
const BOUND: i64 = 0x53e2_d620_fb40;

/// The prepared fixture raid with a saved pool of `items` (variant, save
/// literal), both variants of archetype 0x3309 when `definitions`.
fn totals_fixture(items: &[(u64, &str)], definitions: bool) -> (DecodedFrame, saved::Pack) {
    use installed_content::save::{
        Decimal,
        production::{Equipment, Pool},
    };
    use replay::rows::VariantDefinitionRow;
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

/// The fixture raid's RaidEquipment row with `items` and `totals`.
fn run_totals(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    items: &[(u64, i64)],
    totals: &[(u32, i64)],
) -> Report {
    let mut frame = frame.clone();
    *frame.tables[TableId::RaidEquipment.index()] =
        TableView::Complete(TableRows::RaidEquipment(vec![
            replay::rows::RaidEquipmentRow {
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

/// A raid pool's archetype total is its saved amounts summed by the
/// variants' VariantDefinitions archetype, exactly; a zero total needs no
/// saved variant. A saved variant without a definition row, or an
/// incomplete VariantDefinitions, leaves the totals a limitation.
#[test]
fn raid_equipment_totals_match_the_saved_sums_by_archetype() {
    let items = [(RIFLE, 500_000), (TRUCK, 200_000)];
    let (frame, pack) = totals_fixture(&[(RIFLE, "5"), (TRUCK, "2")], true);
    let report = run_totals(&frame, &pack, &items, &[(0x3309, 700_000), (0x330a, 0)]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["RaidEquipment.archetype_amounts"], 1);
    // Without a definition the totals are a limitation, not compared.
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

/// Away from the clamp bound every disagreement is a difference: a wrong
/// total, a nonzero total without a saved variant, a missing total and a
/// repeated archetype. So is the one case the comparison cannot see, a
/// clamp that later unwound (3/4 L added to two variants, the first
/// removed: native total 1/4 L beside saved entries of 3/4 L).
#[test]
fn raid_equipment_totals_that_disagree_away_from_the_bound_are_differences() {
    let items = [(RIFLE, 500_000), (TRUCK, 200_000)];
    let (frame, pack) = totals_fixture(&[(RIFLE, "5"), (TRUCK, "2")], true);
    let differences = |totals: &[(u32, i64)]| run_totals(&frame, &pack, &items, totals).differences;
    let wrong = differences(&[(0x3309, 700_001)]);
    assert!(
        wrong.len() == 1 && wrong[0].contains("archetype[13065]"),
        "{wrong:?}"
    );
    assert!(differences(&[(0x3309, 700_000), (0x330a, 1)])[0].contains("no saved variant"));
    assert!(differences(&[])[0].contains("absent"));
    assert!(differences(&[(0x3309, 700_000), (0x3309, 700_000)])[0].contains("repeated"));
    let (frame, pack) = totals_fixture(&[(TRUCK, "691752901.5")], true);
    let unwound = run_totals(
        &frame,
        &pack,
        &[(RIFLE, 0), (TRUCK, BOUND / 4 * 3)],
        &[(0x3309, BOUND / 4)],
    );
    assert_eq!(unwound.differences.len(), 1, "{:?}", unwound.differences);
}

/// A total, a contributing saved entry or the saved sum at the clamp
/// bound leaves the total uncompared, with the stated limitation and no
/// difference.
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
    // A saved entry at the bound; the recorded total is not.
    let (frame, pack) = totals_fixture(&[(RIFLE, "922337202"), (TRUCK, "2")], true);
    at_bound(&run_totals(
        &frame,
        &pack,
        &[(RIFLE, BOUND), (TRUCK, 200_000)],
        &[(0x3309, 1)],
    ));
    // The recorded total at either bound, the entries below it.
    let (frame, pack) = totals_fixture(&[(RIFLE, "5"), (TRUCK, "2")], true);
    let items = [(RIFLE, 500_000), (TRUCK, 200_000)];
    at_bound(&run_totals(&frame, &pack, &items, &[(0x3309, BOUND)]));
    at_bound(&run_totals(&frame, &pack, &items, &[(0x3309, -BOUND)]));
    // A total at the bound with no saved variant of its archetype.
    at_bound(&run_totals(
        &frame,
        &pack,
        &items,
        &[(0x3309, 700_000), (0x330a, BOUND)],
    ));
    // Entries below the bound whose sum is exactly +L or -L: 3/4 L added
    // to two variants, then 1/4 L removed from each, leaves entries of
    // 1/2 L summing to L beside a native total of 1/2 L.
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
    // Entries below the bound whose sum passes it.
    let (frame, pack) = totals_fixture(&[(RIFLE, "600000000"), (TRUCK, "600000000")], true);
    at_bound(&run_totals(
        &frame,
        &pack,
        &[(RIFLE, 60_000_000_000_000), (TRUCK, 60_000_000_000_000)],
        &[(0x3309, 1)],
    ));
}

/// Bit 0 of a building's status flags is checked where the writer's
/// branch fixes it: location -1 needs it clear, a province or state
/// location with a positive country override needs it set.
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
    // Province and state references without an override agree either way.
    assert_eq!(check(1, 0, 7), 0);
    assert_eq!(check(2, 0, 7), 0);
    // A positive override is saved with location -1 unless bit 0 is set.
    assert_eq!(check(1, 5, 7), 0);
    assert_eq!(check(2, 5, 7), 1);
    assert_eq!(check(2, 5, -1), 0);
    assert_eq!(check(3, 5, -1), 1);
}

/// Target statuses join on their native ordinal, the saved entry's
/// position: a shifted ordinal is two membership differences, not a
/// silent comparison with whatever entry happens to be at that index.
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

/// One decided search reproducing the entry is a match; a difference
/// needs both searches decided; an undecidable search with no match
/// certifies nothing either way.
#[test]
fn raid_path_verdicts_need_both_searches_to_differ() {
    use installed_content::province_graph::RaidPath::{Land, Naval};
    let failed = || Err("undecidable".to_owned());
    assert_eq!(
        path_verdict(Some(5), &[(Naval, failed()), (Land, Ok(Some(5)))]),
        PathVerdict::Match
    );
    assert_eq!(
        path_verdict(None, &[(Naval, Ok(Some(4))), (Land, Ok(None))]),
        PathVerdict::Match
    );
    assert_eq!(
        path_verdict(Some(5), &[(Naval, Ok(Some(4))), (Land, Ok(None))]),
        PathVerdict::Difference
    );
    assert_eq!(
        path_verdict(Some(5), &[(Naval, failed()), (Land, Ok(Some(4)))]),
        PathVerdict::Undecided
    );
    assert_eq!(
        path_verdict(Some(5), &[(Naval, failed()), (Land, failed())]),
        PathVerdict::Undecided
    );
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
        },
    );
    status.program_order.push(program_id);
    *frame.tables[TableId::RaidFacilities.index()] =
        TableView::Complete(TableRows::RaidFacilities(vec![
            replay::rows::RaidFacilityRow {
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
    let mut parsed = saved::parse(&installed_content::parser::parse(source).unwrap().items).unwrap();
    pack.countries.get_mut(b"GER".as_slice()).unwrap().status =
        parsed.countries.remove(b"GER".as_slice()).unwrap().status;
    let definition = 0x1234;
    let reward = 0x1235;
    let option = 0x1236;
    let specialization = i32::MAX as u32;
    *frame.tables[TableId::RaidFacilities.index()] =
        TableView::Complete(TableRows::RaidFacilities(vec![
            replay::rows::RaidFacilityRow {
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
        }]));
    *frame.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
            owner: 102,
            project: 9,
            ordinal: 0,
            prototype_reward_handle: reward,
            option_handle: option,
            date: installed_content::save::GameDate::parse(b"1936.1.1.1")
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
    use replay::rows::RaidFacilityRow;
    let (mut frame, mut pack, country) = raid_fixture();
    let countries = BTreeMap::from([(102, &country)]);
    let source = br"countries={GER={program_status={
        program={id={id=7 type=83} country=GER province=1 scientist={}
            building=building unread_prototype_rewards=0 project=definition}
        program={id={id=7 type=83} country=GER province=1 scientist={}
            building=building unread_prototype_rewards=0 project=definition}
        project_pool={project={}} breakthrough={}
    }}}";
    let mut parsed = saved::parse(&installed_content::parser::parse(source).unwrap().items).unwrap();
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
            },
        );
    }
    let flag = |province| replay::rows::FacilityMapFlagRow { province, flag: 1 };
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
    use replay::{
        frame::{Metrics, TableView},
        rows::{TableId, TableRows},
    };
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
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains(replay::cadence::STAGGER_LIMITATION)),
        "{:?}",
        report.limitations.keys()
    );
}
