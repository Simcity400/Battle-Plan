use super::super::lexical_tokens;
use super::*;
use installed_content_core::parser;
use installed_content_save::Decimal;
use replay_data::{
    rows::ArmyUpgradeDeliveryRow, rows::ArmyUpgradeGroupRow, rows::ArmyUpgradeProducedRow,
    rows::ArmyUpgradeRequestRow, rows::CountryRow, rows::IndustrialOrganisationHistoryRow,
    rows::IndustrialOrganisationRow, rows::LexicalTokenRow, rows::ProductionLineOutputRow,
    rows::Recorded, rows::TableId, rows::TableRows, rows::VariantDefinitionRow,
    rows::VariantModuleRow, rows::VariantStatsRow, rows::VariantUpgradeRow,
};
use replay_frame::frame::TableView;

/// A frame of the line output self-check: whether it read the lines'
/// slot, its ProductionLineOutput rows `(line, variant, organisation, units)`
/// with their census lines, and its history slots.
fn output_frame(
    hour: i32,
    read: bool,
    rows: &[(u64, u64, u64, i64)],
    history: &[(u64, u64, i32)],
) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    if read {
        frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
    }
    *frame.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(
            rows.iter()
                .map(|&(line, variant, mio, units)| ProductionLineOutputRow {
                    line,
                    variant,
                    mio,
                    units,
                    since_hour: 100,
                    exact: true,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(
            rows.iter()
                .map(|&(handle, variant, mio, _)| ProductionLineRow {
                    handle,
                    variant,
                    mio,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::IndustrialOrganisationHistory.index()] =
        TableView::Complete(TableRows::IndustrialOrganisationHistory(
            history
                .iter()
                .map(
                    |&(organisation, variant, units)| IndustrialOrganisationHistoryRow {
                        organisation,
                        variant,
                        units,
                    },
                )
                .collect(),
        ));
    frame
}

/// One branch of `frames` as a replay, for the checks that walk it.
fn output_replay(frames: Vec<DecodedFrame>) -> Replay {
    use replay::spool::Record;
    use replay_data::{
        identity::Identity, identity::Roster, identity::SessionStart, identity::Verification,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Parentage, frame::FrameEncoder,
    };
    let mut records = vec![
        Record::SessionStart(SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 10_000_000,
            sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: Vec::new(),
                mods: Vec::new(),
                coverage_policy: 1,
                language: "l_english".into(),
                recorder_version: "test".into(),
                collector_hash: [3; 32],
                verification: Verification::Verified {
                    launcher: Roster::default(),
                    memory: Roster::default(),
                },
            },
        }),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: Some(frames[0].hour - 1),
            first_hour: frames[0].hour,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    for mut frame in frames {
        // Only the checked tables are recorded; the singletons hold
        // their one row.
        for table in TableId::ALL {
            if !LINE_OUTPUT_TABLES.contains(&table) {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: "outside the line output fixture".into(),
                };
            }
        }
        *frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![replay_data::rows::ClockRow {
                hour: frame.hour,
                ..Default::default()
            }]));
        *frame.tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![replay_data::rows::CampaignRow {
                id: b"campaign".to_vec(),
            }]));
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(b"campaign".to_vec(), &[records]).unwrap()
}

/// With the table empty at the compared hour, because the branch's last
/// line ended and its final rows were dropped at the next publication, the
/// earlier publications are still self-checked against their census.
#[test]
fn line_output_census_is_checked_after_the_last_line_ended() {
    const ORG: u64 = (5 << 32) | 79;
    const TANK: u64 = (9 << 32) | 70;
    let run = |census_keeps_line: bool| {
        let mut ended = output_frame(124, true, &[(2, TANK, ORG, 0)], &[(ORG, TANK, 10)]);
        let TableView::Complete(TableRows::ProductionLineOutput(output)) =
            &mut *ended.tables[TableId::ProductionLineOutput.index()]
        else {
            unreachable!()
        };
        output[0].ended = true;
        if !census_keeps_line {
            *ended.tables[TableId::ProductionLines.index()] =
                TableView::Complete(TableRows::ProductionLines(Vec::new()));
        }
        let frames = vec![
            output_frame(100, true, &[(2, TANK, ORG, 0)], &[(ORG, TANK, 10)]),
            // Line 2's final publication: it ended.
            ended,
            output_frame(148, true, &[], &[(ORG, TANK, 10)]),
        ];
        let compared = frames[2].clone();
        let replay = output_replay(frames);
        let mut report = Report::default();
        compare_line_output(&replay, &compared, &mut report).unwrap();
        report
    };
    let report = run(false);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["ProductionLineOutput census self-check"],
        1
    );
    // The ended line's row left a census that still holds the line.
    let report = run(true);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(
        report.differences[0].starts_with("ProductionLineOutput 2 at hour 124"),
        "{:?}",
        report.differences
    );
    // A branch that never published a row has nothing to check.
    let frames = vec![
        output_frame(100, true, &[], &[(ORG, TANK, 10)]),
        output_frame(124, true, &[], &[(ORG, TANK, 10)]),
    ];
    let compared = frames[1].clone();
    let mut report = Report::default();
    compare_line_output(&output_replay(frames), &compared, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("no production line output was published"))
    );
}

#[test]
fn line_output_country_uses_saved_producer_and_resolved_raw_reference() {
    let saved = production::parse(
        &parser::parse(
            br#"countries={
        MAK={production={military_lines={id={id=1 type=56} cost=0 priority=0 amount=1
            equipment_variant_index={id=9 type=70} requested_factories=0}}}
        DES={production={}}
    }"#,
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let producer = CountryRow {
        id: 5,
        tag: b"MAK".to_vec(),
        ..Default::default()
    };
    let designer = CountryRow {
        id: 6,
        tag: b"DES".to_vec(),
        ..Default::default()
    };
    // CountryLinks resolves raw 5001 to producer slot 5.
    let links = BTreeMap::from([(5001, &producer), (6, &designer)]);
    let line = (1_u64 << 32) | 56;
    let row = ProductionLineOutputRow {
        line,
        country: 5001,
        mio: 0,
        variant: (9 << 32) | 70,
        ..Default::default()
    };
    let run = |rows| {
        let mut frame = output_frame(124, true, &[], &[]);
        *frame.tables[TableId::ProductionLineOutput.index()] =
            TableView::Complete(TableRows::ProductionLineOutput(rows));
        let mut report = Report::default();
        let result = compare_line_output_countries(&frame, &saved, &links, &mut report);
        (result, report)
    };
    let (result, report) = run(vec![
        row.clone(),
        ProductionLineOutputRow {
            line: line + (1 << 32),
            country: 5001,
            ended: true,
            ..Default::default()
        },
    ]);
    result.unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ProductionLineOutput.country"], 1);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("ended production lines"))
    );
    let (result, report) = run(vec![ProductionLineOutputRow {
        country: 6,
        ..row.clone()
    }]);
    result.unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains(".country"));
    // A live row whose line the save does not hold.
    let (result, report) = run(vec![
        row.clone(),
        ProductionLineOutputRow {
            line: line + (1 << 32),
            ..row.clone()
        },
    ]);
    result.unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("absent from saved production"));
    assert!(
        run(vec![ProductionLineOutputRow {
            country: 9999,
            ..row.clone()
        }])
        .0
        .is_err()
    );
    assert_eq!(run(vec![]).1.differences.len(), 1);
    assert_eq!(
        run(vec![ProductionLineOutputRow { ended: true, ..row }])
            .1
            .differences
            .len(),
        1
    );
}

#[test]
fn organisations_compare_order_key_texts_owner_and_history_units() {
    let source = br#"countries={ GER={ production={ industrial_organisations={
        GER_a_organization={ id={ id=5 type=79 } name="" icon="GFX_idea_a" size=0
            history={ equipment={ id=9 type=70 } data={ date=1936.1.1.12 units=2116 } } }
        GER_b_organization={ id={ id=6 type=79 } name="GER_b_renamed" icon="" } } } } }"#;
    let saved = production::parse(&parser::parse(source).unwrap().items).unwrap();
    let organisation = |id: u64| (id << 32) | 79;
    let variant = (9 << 32) | 70;
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 22,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    let rows = vec![
        IndustrialOrganisationRow {
            organisation: organisation(5),
            country: 11,
            ordinal: 0,
            owner: 11,
            token: 17,
            name: Vec::new(),
            icon: b"GFX_idea_a".to_vec(),
        },
        IndustrialOrganisationRow {
            organisation: organisation(6),
            country: 11,
            ordinal: 1,
            owner: 11,
            token: 18,
            name: b"GER_b_renamed".to_vec(),
            icon: Vec::new(),
        },
    ];
    let history = |units| {
        vec![IndustrialOrganisationHistoryRow {
            organisation: organisation(5),
            variant,
            units,
        }]
    };
    *frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(rows.clone()));
    *frame.tables[TableId::IndustrialOrganisationHistory.index()] =
        TableView::Complete(TableRows::IndustrialOrganisationHistory(history(2116)));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 17,
                literal: b"GER_a_organization".to_vec(),
            },
            LexicalTokenRow {
                token: 18,
                literal: b"GER_b_organization".to_vec(),
            },
        ]));
    let country = CountryRow {
        id: 11,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(11, &country)]);
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    compare_organisations(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["IndustrialOrganisations"], 2);
    assert_eq!(report.compared_tables["IndustrialOrganisationHistory"], 1);
    // Units, order, key, texts and an unsaved history slot all differ.
    let mut changed = rows;
    changed[0].ordinal = 1;
    changed[0].icon = b"GFX_idea_b".to_vec();
    changed[1].ordinal = 0;
    changed[1].token = 17;
    let mut extra = history(2117);
    extra.push(IndustrialOrganisationHistoryRow {
        organisation: organisation(6),
        variant,
        units: 1,
    });
    *frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(changed));
    *frame.tables[TableId::IndustrialOrganisationHistory.index()] =
        TableView::Complete(TableRows::IndustrialOrganisationHistory(extra));
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    compare_organisations(&frame, &saved, &links, &tokens, &mut report).unwrap();
    let labels: Vec<_> = report
        .differences
        .iter()
        .map(|difference| difference.split(": replay").next().unwrap())
        .collect();
    assert_eq!(
        labels,
        [
            "organisation 21474836559.ordinal",
            "organisation 21474836559.icon",
            "organisation 21474836559.history 38654705734.units",
            "organisation 25769803855.ordinal",
            "organisation 25769803855.key",
            "organisation 25769803855.history 38654705734",
        ]
    );
}

fn pool(text: &[u8]) -> production::Pool {
    production::parse_pool(&parser::parse(text).unwrap().items).unwrap()
}

const DEPLOYMENT_SAVE: &[u8] = br#"countries={
GER={
production={ production_licenses={
available={ id=5061 type=70 } available={ id=90 type=70 }
owned_license={ lended_cic=-1 required_cic=3 owner="GER" giver="GER"
                start_date="1936.1.1.1" parent={ id=1441 type=70 }
                equipment={ id=5061 type=70 } }
} }
deployment={
military_deployment_conveyor={
    id={ id=433 type=64 }
    division_template_id={ id=2526 type=52 }
    name="Panzer"
    amount=-1
    location=6103
    priority=1
    role=armor
    closed=no
    military_deployment_line={
        id={ id=7067 type=63 }
        division_name={ type=0 override="Named" }
        amount=1
        military_deployment={
            army_manpower_value={ }
            army_manpower_need={ }
            max_manpower=0
        }
    }
    military_deployment_line={
        id={ id=7066 type=63 }
        division_name={ type=0 name_order=4 }
        amount=2
        military_deployment={
            training=0.00001
            max_training=0.98799
            army_manpower_value={ value={ tag="GER" value=10620 } }
            army_manpower_need={ }
            equipment={ equipment={ id={ id=5553 type=70 } amount=16 } allow_zero_entries=no }
            max_manpower=10620
        }
    }
}
}
}
}"#;

/// One saved queue plus the captured rows that should match it exactly.
type DeploymentContext = (
    production::Pack,
    Vec<CountryRow>,
    Vec<replay_data::rows::DeploymentConveyorRow>,
    Vec<replay_data::rows::DeploymentLineRow>,
    Vec<replay_data::rows::DeploymentManpowerRow>,
    Vec<replay_data::rows::DeploymentEquipmentRow>,
);

fn deployment_context() -> DeploymentContext {
    let saved = production::parse(&parser::parse(DEPLOYMENT_SAVE).unwrap().items).unwrap();
    let countries = vec![CountryRow {
        id: 9,
        canonical_slot: 9,
        tag: b"GER".to_vec(),
        ..Default::default()
    }];
    let conveyor = (433_u64 << 32) | 64;
    let line = (7066_u64 << 32) | 63;
    let untrained = (7067_u64 << 32) | 63;
    let conveyors = vec![replay_data::rows::DeploymentConveyorRow {
        handle: conveyor,
        country: 9,
        ordinal: 0,
        template: (2526_u64 << 32) | 52,
        amount: -1,
        location: 6103,
        priority: 1,
        role_token: 11,
        closed: 0,
        name: b"Panzer".to_vec(),
        exile_country: None,
    }];
    let lines = vec![
        replay_data::rows::DeploymentLineRow {
            handle: untrained,
            conveyor,
            country: 9,
            ordinal: 0,
            name_order: 0,
            amount: 1,
            training_raw: 0,
            max_training_raw: 0,
            max_manpower: 0,
            allow_zero_entries: 0,
            equipment_allocations: 0,
            distributable_priority: 1,
            halting_reason_present: 0,
            halting_reason: 0,
            custom_name: b"Named".to_vec(),
            custom_flag_a: 1,
            custom_flag_b: 0,
        },
        replay_data::rows::DeploymentLineRow {
            handle: line,
            conveyor,
            country: 9,
            ordinal: 1,
            name_order: 4,
            amount: 2,
            training_raw: 1,
            max_training_raw: 98_799,
            max_manpower: 10_620,
            allow_zero_entries: 0,
            equipment_allocations: 1,
            distributable_priority: 1,
            halting_reason_present: 0,
            halting_reason: 0,
            custom_name: Vec::new(),
            custom_flag_a: 1,
            custom_flag_b: 0,
        },
    ];
    let manpower = vec![replay_data::rows::DeploymentManpowerRow {
        line,
        side: 0,
        ordinal: 0,
        source_country: 9,
        amount: 10_620,
    }];
    let equipment = vec![replay_data::rows::DeploymentEquipmentRow {
        line,
        ordinal: 0,
        variant_handle: (5553_u64 << 32) | 70,
        amount_raw: 1_600_000,
    }];
    (saved, countries, conveyors, lines, manpower, equipment)
}

fn deployment_frame(
    conveyors: Vec<replay_data::rows::DeploymentConveyorRow>,
    lines: Vec<replay_data::rows::DeploymentLineRow>,
    manpower: Vec<replay_data::rows::DeploymentManpowerRow>,
    equipment: Vec<replay_data::rows::DeploymentEquipmentRow>,
    licenses: Vec<replay_data::rows::ProductionLicenseRow>,
    records: Vec<replay_data::rows::ProductionLicenseRecordRow>,
) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 22,
        metrics: replay_frame::frame::Metrics::default(),
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "outside deployment comparison".into(),
        })
        .map(Into::into),
    };
    *frame.tables[TableId::DeploymentConveyors.index()] =
        TableView::Complete(TableRows::DeploymentConveyors(conveyors));
    *frame.tables[TableId::DeploymentLines.index()] =
        TableView::Complete(TableRows::DeploymentLines(lines));
    *frame.tables[TableId::DeploymentManpower.index()] =
        TableView::Complete(TableRows::DeploymentManpower(manpower));
    *frame.tables[TableId::DeploymentEquipment.index()] =
        TableView::Complete(TableRows::DeploymentEquipment(equipment));
    *frame.tables[TableId::ProductionLicenses.index()] =
        TableView::Complete(TableRows::ProductionLicenses(licenses));
    *frame.tables[TableId::ProductionLicenseRecords.index()] =
        TableView::Complete(TableRows::ProductionLicenseRecords(records));
    frame
}

#[test]
fn equipment_rosters_compare_in_saved_order_and_absent_foreign_is_empty() {
    let saved = production::parse(
        &parser::parse(
            br#"countries={GER={production={available_equipments={
                equipment={id=5061 type=70} equipment={id=5061 type=70}
            }}}}"#,
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let countries = [CountryRow {
        id: 9,
        canonical_slot: 9,
        tag: b"GER".to_vec(),
        ..Default::default()
    }];
    let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
    let row = |roster_kind, ordinal, id: u64| replay_data::rows::AvailableEquipmentVariantRow {
        country: 9,
        roster_kind,
        ordinal,
        variant: (id << 32) | 70,
    };
    let run = |rows| {
        let mut frame = deployment_frame(vec![], vec![], vec![], vec![], vec![], vec![]);
        *frame.tables[TableId::AvailableEquipmentVariants.index()] =
            TableView::Complete(TableRows::AvailableEquipmentVariants(rows));
        let mut report = Report::default();
        compare_available_variants(&frame, &saved, &links, &mut report).map(|()| report)
    };
    let report = run(vec![row(0, 0, 5061), row(0, 1, 5061)]).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["AvailableEquipmentVariants"], 2);
    // A different variant, and a foreign entry the save does not hold.
    let report = run(vec![row(0, 0, 5061), row(0, 1, 90), row(1, 0, 90)]).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(run(vec![row(0, 1, 5061)]).is_err());
}

#[test]
fn licences_and_the_deployment_queue_compare_exactly_against_a_real_save_shape() {
    let (saved, countries, mut conveyors, lines, manpower, equipment) = deployment_context();
    conveyors[0].name.extend_from_slice(b"\0tail");
    let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
    let licenses = vec![
        replay_data::rows::ProductionLicenseRow {
            country: 9,
            ordinal: 0,
            variant: (5061_u64 << 32) | 70,
        },
        replay_data::rows::ProductionLicenseRow {
            country: 9,
            ordinal: 1,
            variant: (90_u64 << 32) | 70,
        },
    ];
    let records = vec![replay_data::rows::ProductionLicenseRecordRow {
        owner: 9,
        ordinal: 0,
        giver: 9,
        lended_cic: -1,
        required_cic: 3,
        start_date: installed_content_save::GameDate {
            year: 1936,
            month: 1,
            day: 1,
            hour: 1,
        }
        .engine_hour()
        .unwrap(),
        parent: (1441_u64 << 32) | 70,
        operand_raw: 100_000,
        equipment: vec![(5061_u64 << 32) | 70],
    }];
    let frame = deployment_frame(conveyors, lines, manpower, equipment, licenses, records);
    let tokens: TokenNames<'_> = [(11_u32, b"armor".as_slice())].into_iter().collect();
    let mut report = Report::default();
    compare_licenses(&frame, &saved, &links, &mut report).unwrap();
    compare_license_records(&frame, &saved, &links, &mut report).unwrap();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ProductionLicenses"], 2);
    assert_eq!(report.compared_tables["ProductionLicenseRecords"], 1);
    assert_eq!(report.compared_tables["DeploymentConveyors"], 1);
    // Only the trained line's two words reach the decimal comparator; the
    // untrained line's omitted words are compared against zero instead.
    assert_eq!(report.scalar_checks, 2);
    // Both deployment gaps are now recorded and compared, so neither
    // limitation may remain for this queue.
    assert!(
        !report
            .limitations
            .keys()
            .any(|text| text.contains("custom division name")
                || text.contains("government-in-exile")),
        "{:?}",
        report.limitations
    );
}

#[test]
fn changed_words_missing_rows_and_unresolved_roles_are_all_reported() {
    let (saved, countries, mut conveyors, mut lines, mut manpower, mut equipment) =
        deployment_context();
    let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
    conveyors[0].amount += 1;
    conveyors[0].name = b"Panzers".to_vec();
    lines[0].training_raw = 1;
    lines[1].max_training_raw -= 1;
    lines[1].name_order = 5;
    manpower[0].amount = 0;
    equipment[0].amount_raw = 0;
    let frame = deployment_frame(
        conveyors,
        lines,
        manpower,
        equipment,
        vec![replay_data::rows::ProductionLicenseRow {
            country: 9,
            ordinal: 0,
            variant: (5061_u64 << 32) | 70,
        }],
        Vec::new(),
    );
    let mut report = Report::default();
    compare_licenses(&frame, &saved, &links, &mut report).unwrap();
    // One licence short: the count differs and the retained entry matches.
    assert_eq!(report.differences.len(), 1);
    // An empty licence-record table against one saved record: count only.
    let mut report = Report::default();
    compare_license_records(&frame, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    let tokens: TokenNames<'_> = TokenNames::new();
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    // name, amount, max_training, name_order, a changed manpower count and
    // a zero-amount entry the save still carries, plus the untrained line
    // whose omitted training no longer reads as zero.
    assert_eq!(report.differences.len(), 7, "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|text| text.contains("conveyor role tokens"))
    );
}

/// Every licence-record field must be able to fail on its own; a comparator
/// that only counted rows would pass this with the wrong values.
#[test]
fn every_licence_record_field_can_differ_on_its_own() {
    let (saved, countries, ..) = deployment_context();
    let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
    let good = replay_data::rows::ProductionLicenseRecordRow {
        owner: 9,
        ordinal: 0,
        giver: 9,
        lended_cic: -1,
        required_cic: 3,
        start_date: installed_content_save::GameDate {
            year: 1936,
            month: 1,
            day: 1,
            hour: 1,
        }
        .engine_hour()
        .unwrap(),
        parent: (1441_u64 << 32) | 70,
        operand_raw: 100_000,
        equipment: vec![(5061_u64 << 32) | 70],
    };
    type Change = fn(&mut replay_data::rows::ProductionLicenseRecordRow);
    let mutate: [(&str, Change); 6] = [
        ("lended_cic", |row| row.lended_cic += 1),
        ("required_cic", |row| row.required_cic += 1),
        ("start_date", |row| row.start_date += 1),
        ("parent", |row| row.parent += 1),
        ("equipment", |row| row.equipment.push(0)),
        ("giver", |row| row.giver = 0),
    ];
    for (name, change) in mutate {
        let mut row = good.clone();
        change(&mut row);
        let frame = deployment_frame(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![row],
        );
        let mut report = Report::default();
        let result = compare_license_records(&frame, &saved, &links, &mut report);
        // A zero giver has no country tag at all, which is an error, not a
        // silent pass; every other change is exactly one difference.
        if name == "giver" {
            assert!(result.is_err(), "{name}");
        } else {
            result.unwrap();
            assert_eq!(
                report.differences.len(),
                1,
                "{name}: {:?}",
                report.differences
            );
        }
    }
    let frame = deployment_frame(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![good],
    );
    let mut report = Report::default();
    compare_license_records(&frame, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ProductionLicenseRecords"], 1);
}

/// The game omits the order word only when it is zero, so a nonzero
/// captured order against an omitted saved one is a real difference.
#[test]
fn an_omitted_name_order_is_compared_against_zero_not_waived() {
    let (saved, countries, conveyors, mut lines, manpower, equipment) = deployment_context();
    let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
    let tokens: TokenNames<'_> = [(11_u32, b"armor".as_slice())].into_iter().collect();
    // The untrained line's saved division_name carries an override and no
    // order word, and the captured row already holds zero.
    assert_eq!(lines[0].name_order, 0);
    lines[0].name_order = -1;
    let frame = deployment_frame(
        conveyors,
        lines,
        manpower,
        equipment,
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("name_order"));
}

/// Gated exile and flagged custom-name emission: present values compare
/// exactly, omitted values prove empty/clear, and any mismatch is a real
/// difference rather than a printed limitation.
#[test]
fn exile_tag_present_and_omitted_compare_with_gated_emission() {
    const SAVE: &[u8] = br#"countries={
GER={ deployment={
military_deployment_conveyor={
    id={ id=1 type=64 }
    division_template_id={ id=1 type=52 }
    name="A" amount=1 location=0 priority=1 role=infantry closed=no
    government_in_exile_tag="ETH"
}
military_deployment_conveyor={
    id={ id=2 type=64 }
    division_template_id={ id=1 type=52 }
    name="B" amount=1 location=0 priority=1 role=infantry closed=no
}
} } }"#;
    let saved = production::parse(&parser::parse(SAVE).unwrap().items).unwrap();
    let countries = [
        CountryRow {
            id: 9,
            canonical_slot: 9,
            tag: b"GER".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 10,
            canonical_slot: 10,
            tag: b"ETH".to_vec(),
            ..Default::default()
        },
    ];
    let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
    let tokens: TokenNames<'_> = [(7_u32, b"infantry".as_slice())].into_iter().collect();
    let conveyors = vec![
        replay_data::rows::DeploymentConveyorRow {
            handle: (1_u64 << 32) | 64,
            country: 9,
            ordinal: 0,
            template: (1_u64 << 32) | 52,
            amount: 1,
            location: 0,
            priority: 1,
            role_token: 7,
            closed: 0,
            name: b"A".to_vec(),
            exile_country: Some(10),
        },
        replay_data::rows::DeploymentConveyorRow {
            handle: (2_u64 << 32) | 64,
            country: 9,
            ordinal: 1,
            template: (1_u64 << 32) | 52,
            amount: 1,
            location: 0,
            priority: 1,
            role_token: 7,
            closed: 0,
            name: b"B".to_vec(),
            exile_country: None,
        },
    ];
    let frame = deployment_frame(
        conveyors.clone(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    // Present saved tag against an omitted captured reference is a difference.
    let mut missing = conveyors.clone();
    missing[0].exile_country = None;
    let frame = deployment_frame(
        missing,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("government_in_exile_tag"));
    // Omitted saved tag against a present captured reference is a difference.
    let mut extra = conveyors;
    extra[1].exile_country = Some(10);
    let frame = deployment_frame(
        extra,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("government_in_exile_tag"));
}

/// Custom names keep raw bytes plus both flag bytes: the string is gated on
/// nonzero length, 0x3936 on the set flag (saved as
/// override_set_programmatically), and 0x38e2 omits when set.
#[test]
fn custom_name_and_programmatic_flags_compare_with_flagged_emission() {
    const SAVE: &[u8] = br#"countries={
GER={ deployment={
military_deployment_conveyor={
    id={ id=1 type=64 }
    division_template_id={ id=1 type=52 }
    name="A" amount=1 location=0 priority=1 role=infantry closed=no
    military_deployment_line={
        id={ id=10 type=63 }
        division_name={ type=0 override="Named" override_set_programmatically=yes }
        amount=1
        military_deployment={ army_manpower_value={ } army_manpower_need={ } }
    }
    military_deployment_line={
        id={ id=11 type=63 }
        division_name={ type=0 name_order=4 }
        amount=1
        military_deployment={ army_manpower_value={ } army_manpower_need={ } }
    }
}
} } }"#;
    let saved = production::parse(&parser::parse(SAVE).unwrap().items).unwrap();
    let countries = [CountryRow {
        id: 9,
        canonical_slot: 9,
        tag: b"GER".to_vec(),
        ..Default::default()
    }];
    let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
    let tokens: TokenNames<'_> = [(7_u32, b"infantry".as_slice())].into_iter().collect();
    let conveyor = (1_u64 << 32) | 64;
    let conveyors = vec![replay_data::rows::DeploymentConveyorRow {
        handle: conveyor,
        country: 9,
        ordinal: 0,
        template: (1_u64 << 32) | 52,
        amount: 1,
        location: 0,
        priority: 1,
        role_token: 7,
        closed: 0,
        name: b"A".to_vec(),
        exile_country: None,
    }];
    let lines = vec![
        replay_data::rows::DeploymentLineRow {
            handle: (10_u64 << 32) | 63,
            conveyor,
            country: 9,
            ordinal: 0,
            name_order: 0,
            amount: 1,
            distributable_priority: 1,
            custom_name: b"Named".to_vec(),
            custom_flag_a: 1,
            custom_flag_b: 1,
            ..Default::default()
        },
        replay_data::rows::DeploymentLineRow {
            handle: (11_u64 << 32) | 63,
            conveyor,
            country: 9,
            ordinal: 1,
            name_order: 4,
            amount: 1,
            distributable_priority: 1,
            custom_name: Vec::new(),
            custom_flag_a: 1,
            custom_flag_b: 0,
            ..Default::default()
        },
    ];
    let frame = deployment_frame(
        conveyors.clone(),
        lines.clone(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    // Omitted saved override against present captured bytes is a difference.
    let mut extra = lines.clone();
    extra[1].custom_name = b"Other".to_vec();
    let frame = deployment_frame(
        conveyors.clone(),
        extra,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains(".override"));
    // Present saved override against empty captured bytes is a difference
    // (presence plus content, both under the same override label).
    let mut missing = lines.clone();
    missing[0].custom_name.clear();
    let frame = deployment_frame(
        conveyors.clone(),
        missing,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .all(|text| text.contains(".override"))
    );
    // Present programmatic flag against a clear captured flag is a difference.
    let mut cleared = lines.clone();
    cleared[0].custom_flag_b = 0;
    let frame = deployment_frame(
        conveyors.clone(),
        cleared,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("override_set_programmatically"));
    // Omitted programmatic flag against a set captured flag is a difference.
    let mut set = lines.clone();
    set[1].custom_flag_b = 1;
    let frame = deployment_frame(
        conveyors.clone(),
        set,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("override_set_programmatically"));
    // A clear flag without its `no` field is a real difference.
    let mut cleared_a = lines;
    cleared_a[0].custom_flag_a = 0;
    let frame = deployment_frame(
        conveyors,
        cleared_a,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut report = Report::default();
    compare_deployment(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("custom_flag_a"));
    let saved_clear = production::parse(
        &parser::parse(
            String::from_utf8(SAVE.to_vec())
                .unwrap()
                .replace("type=0 override=", "type=0 is_name_ordered=no override=")
                .as_bytes(),
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let mut report = Report::default();
    compare_deployment(&frame, &saved_clear, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

#[test]
fn duplicate_handles_and_unavailable_deployment_tables_are_errors() {
    let (saved, countries, conveyors, lines, manpower, equipment) = deployment_context();
    let links: BTreeMap<u32, &CountryRow> = countries.iter().map(|row| (row.id, row)).collect();
    let tokens: TokenNames<'_> = TokenNames::new();
    let mut duplicated = lines.clone();
    duplicated.push(lines[0].clone());
    let frame = deployment_frame(
        conveyors.clone(),
        duplicated,
        manpower.clone(),
        equipment.clone(),
        Vec::new(),
        Vec::new(),
    );
    assert!(compare_deployment(&frame, &saved, &links, &tokens, &mut Report::default()).is_err());

    let mut frame = deployment_frame(
        conveyors,
        lines,
        manpower,
        equipment,
        Vec::new(),
        Vec::new(),
    );
    *frame.tables[TableId::DeploymentManpower.index()] = TableView::Unavailable {
        reason: "manpower container vtable mismatch".into(),
    };
    assert!(compare_deployment(&frame, &saved, &links, &tokens, &mut Report::default()).is_err());
    assert!(compare_licenses(&frame, &saved, &links, &mut Report::default()).is_ok());
}

#[test]
fn zero_omission_does_not_shift_duplicate_nonzero_bank_matches() {
    let saved = pool(
        b"equipment={id={id=1 type=70} amount=-0.00001}
          equipment={id={id=1 type=70} amount=2}
          allow_zero_entries=no",
    );
    let rows = [
        (0, 70, 0),
        (1, 0x1_0000_0046, -1),
        (2, 71, 0),
        (3, 0x1_0000_0046, 200_000),
    ];
    let mut report = Report::default();
    assert_eq!(compare_pool(rows, &saved, "pool", &mut report).unwrap(), 2);
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 2);
    assert_eq!(report.limitations.len(), 1);
    let mut changed = rows;
    changed[1].2 = 200_000;
    changed[3].2 = -1;
    let mut report = Report::default();
    compare_pool(changed, &saved, "pool", &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn zero_preserving_pool_checks_full_handles_and_zero_membership() {
    let saved = pool(
        b"equipment={id={id=4294967295 type=70} amount=0}
          equipment={id={id=0 type=70} amount=-92233720368547.75808}
          allow_zero_entries=yes",
    );
    let mut report = Report::default();
    compare_pool(
        [(0, 0xffff_ffff_0000_0046, 0), (1, 70, i64::MIN)],
        &saved,
        "pool",
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert!(report.limitations.is_empty());
    // Keyed alignment reports the one missing entry once instead of
    // shifting every later position into a variant and an amount difference.
    let mut report = Report::default();
    compare_pool([(0, 70, i64::MIN)], &saved, "pool", &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("saved[0]"));
}

#[test]
fn removed_entries_do_not_shift_later_banks_into_false_differences() {
    // One removed entry used to inflate every following position into two
    // differences; only the removal and the real amount change remain.
    let saved = pool(
        b"equipment={id={id=1 type=70} amount=1}
          equipment={id={id=3 type=70} amount=3}
          equipment={id={id=4 type=70} amount=9}
          allow_zero_entries=yes",
    );
    let rows = [
        (0, 0x1_0000_0046, 100_000),
        (1, 0x2_0000_0046, 200_000),
        (2, 0x3_0000_0046, 300_000),
        (3, 0x4_0000_0046, 400_000),
    ];
    let mut report = Report::default();
    assert_eq!(compare_pool(rows, &saved, "pool", &mut report).unwrap(), 3);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("visible_equipment[1].variant"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains("visible_equipment[3].amount"))
    );
}

#[test]
fn a_reordered_native_pool_is_one_explicit_order_difference() {
    let saved = pool(
        b"equipment={id={id=1 type=70} amount=1}
          equipment={id={id=2 type=70} amount=2}
          allow_zero_entries=yes",
    );
    let rows = [(0, 0x2_0000_0046, 200_000), (1, 0x1_0000_0046, 100_000)];
    let mut report = Report::default();
    assert_eq!(compare_pool(rows, &saved, "pool", &mut report).unwrap(), 2);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("visible_equipment.order"));
}

#[test]
fn absent_policy_and_broken_native_order_are_explicit_errors() {
    let mut saved = production::Pool {
        equipment: vec![production::Equipment {
            variant: 70,
            amount: Decimal::parse(b"1").unwrap(),
        }],
        allow_zero_entries: None,
    };
    let mut report = Report::default();
    assert!(compare_pool([(0, 70, 100_000)], &saved, "pool", &mut report).is_err());
    saved.allow_zero_entries = Some(false);
    for rows in [[(1, 70, 100_000)], [(0, 0, 100_000)]] {
        assert!(compare_pool(rows, &saved, "pool", &mut report).is_err());
    }
}

#[test]
fn base_line_compares_omitted_zero_and_signed_fixed5_without_narrowing() {
    let doc = parser::parse(
        b"id={id=4294967295 type=56} cost=92233720368547.75807 priority=-1 amount=-2147483648 produced=0.00001",
    )
    .unwrap();
    let expected = production::parse_line_base(&doc.items).unwrap();
    // The saved produced progress has no recorded counterpart: it is retired.
    assert!(expected.produced.is_some());
    let mut actual = ProductionLineRow {
        handle: 0xffff_ffff_0000_0038,
        cost_raw: i64::MAX,
        priority: -1,
        amount: i32::MIN,
        ..Default::default()
    };
    let mut report = Report::default();
    compare_line_base(&actual, &expected, "line", &mut report);
    assert!(report.differences.is_empty());
    assert_eq!(report.scalar_checks, 2);
    actual.speed_raw = -1;
    actual.cost_raw -= 1;
    actual.active_factories = -1;
    compare_line_base(&actual, &expected, "line", &mut report);
    assert_eq!(report.differences.len(), 3);
}

#[test]
fn efficiency_averages_the_saved_prefix_clamped_by_the_recorded_maximum() {
    let doc = parser::parse(
        b"id={id=7 type=56} cost=1 active_factories=3 priority=0 amount=-1 \
          requested_factories=3 equipment_variant_index={id=9 type=70} \
          factory_efficiencies={ 53.00000 50.00000 10.00000 10.00000 }",
    )
    .unwrap();
    let expected = &production::parse_line(0, &doc.items).unwrap();
    assert_eq!(
        expected.factory_efficiencies,
        [53 << 15, 50 << 15, 10 << 15, 10 << 15]
    );
    let mut actual = ProductionLineRow {
        max_factories: 150,
        efficiency_raw: (113 << 15) / 3,
        ..Default::default()
    };
    let mut report = Report::default();
    compare_efficiency(&actual, expected, "line", &mut report);
    assert!(report.differences.is_empty());
    // A maximum below the active factories clamps the average.
    actual.max_factories = 2;
    actual.efficiency_raw = (103 << 15) / 2;
    compare_efficiency(&actual, expected, "line", &mut report);
    assert!(report.differences.is_empty());
    actual.efficiency_raw += 1;
    compare_efficiency(&actual, expected, "line", &mut report);
    assert_eq!(report.differences.len(), 1);
    // More factories than saved entries is a difference, not a guess.
    actual.max_factories = 5;
    let mut short = expected.clone();
    short.base.active_factories = Some(5);
    compare_efficiency(&actual, &short, "line", &mut report);
    assert_eq!(report.differences.len(), 2);
    // No active factories: zero, whatever the array holds.
    short.base.active_factories = None;
    actual.efficiency_raw = 0;
    compare_efficiency(&actual, &short, "line", &mut report);
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn modifier_oracle_compares_every_saved_factory_occurrence_and_keeps_runtime_operands_as_limitations()
 {
    use replay_data::rows::ProductionLineModifiersRow;
    let saved = production::parse(
        &parser::parse(
            br#"countries={MAK={production={military_lines={
        id={id=1 type=56} cost=0 priority=0 amount=1 active_factories=1
        equipment_variant_index={id=9 type=70} requested_factories=1
        industrial_manufacturer={id=7 type=127}
        factory_efficiencies={53.00000 -1.00000 10.00000 10.00000}
    }}}}"#,
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let producer = CountryRow {
        id: 5,
        tag: b"MAK".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(5001, &producer)]);
    let row = ProductionLineModifiersRow {
        line: (1 << 32) | 56,
        country: 5001,
        variant: (9 << 32) | 70,
        organisation: (7 << 32) | 127,
        active_factories: 1,
        factory_efficiencies: vec![53 << 15, -(1 << 15), 10 << 15, 10 << 15],
        organisation_dirty: true,
        bonuses: [i64::MIN; 7],
        resource_penalty_raw: i64::MAX,
        ..Default::default()
    };
    let check = |rows| {
        let mut frame = output_frame(124, true, &[], &[]);
        *frame.tables[TableId::ProductionLineModifiers.index()] =
            TableView::Complete(TableRows::ProductionLineModifiers(rows));
        let mut report = Report::default();
        compare_modifiers(&frame, &saved, &links, &mut report).unwrap();
        report
    };
    let report = check(vec![row.clone()]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ProductionLineModifiers"], 1);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("MIO cache/dirty state"))
    );
    // Inactive entries, repeats and native order are compared, not the average.
    for factories in [
        vec![53 << 15],
        vec![53 << 15, 10 << 15, -(1 << 15), 10 << 15],
    ] {
        let report = check(vec![ProductionLineModifiersRow {
            factory_efficiencies: factories,
            ..row.clone()
        }]);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("factory_efficiencies"));
    }
    let report = check(vec![ProductionLineModifiersRow {
        variant: (8 << 32) | 70,
        ..row.clone()
    }]);
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("variant"));
    let report = check(vec![ProductionLineModifiersRow {
        active_factories: 2,
        ..row.clone()
    }]);
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("active_factories"));
    let mut extra = row.clone();
    extra.line = (2 << 32) | 56;
    assert_eq!(check(vec![row, extra]).differences.len(), 1);
    assert_eq!(check(vec![]).differences.len(), 1);
}

#[test]
fn icon_presence_follows_the_active_byte_not_the_string() {
    let mut report = Report::default();
    icon_field(&mut report, "inactive", 0, b"", None);
    icon_field(&mut report, "inactive text", 0, b"GFX_x", None);
    icon_field(&mut report, "active empty", 1, b"", Some(b""));
    icon_field(&mut report, "sentinel", 0xfe, b".", Some(b"."));
    icon_field(&mut report, "nul", 1, b"GFX_a\0b", Some(b"GFX_a"));
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.len(), 1);
    icon_field(&mut report, "missing", 1, b"GFX_x", None);
    icon_field(&mut report, "unexpected", 0, b"", Some(b"GFX_x"));
    icon_field(&mut report, "changed", 1, b"GFX_x", Some(b"GFX_y"));
    assert_eq!(report.differences.len(), 3);
}

#[test]
fn saved_string_presence_uses_native_length_and_visible_prefix() {
    let mut report = Report::default();
    name_field(&mut report, "empty", b"", None);
    name_field(&mut report, "bytes", &[0xff, b'A'], Some(&[0xff, b'A']));
    name_field(&mut report, "nul", b"A\0suffix", Some(b"A"));
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.len(), 1);
    name_field(&mut report, "missing", b"A", None);
    name_field(&mut report, "unexpected", b"", Some(b""));
    name_field(&mut report, "changed", b"A\0suffix", Some(b"B"));
    assert_eq!(report.differences.len(), 3);
}

#[test]
fn lexical_writers_keep_exact_bytes_and_scope_unproved_nul_literals() {
    let token_bytes = [b'a', 0xff, 0, b'z'];
    let exact_bytes = [b'a', 0xff];
    let tokens = BTreeMap::from([
        (0, b"".as_slice()),
        (17, exact_bytes.as_slice()),
        (18, token_bytes.as_slice()),
        (0x167, b"0.75".as_slice()),
    ]);
    let mut report = Report::default();
    for writer in [TokenWriter::Token, TokenWriter::String] {
        compare_token(&tokens, 17, &[b'a', 0xff], "exact", writer, &mut report).unwrap();
        compare_token(&tokens, 18, b"unproved output", "nul", writer, &mut report).unwrap();
    }
    compare_token(&tokens, 0, b"", "zero", TokenWriter::Token, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.limitations.len(), 1);
    assert!(
        compare_token(
            &tokens,
            0x167,
            b"0.75",
            "numeric",
            TokenWriter::Token,
            &mut report
        )
        .is_err()
    );
    compare_token(
        &tokens,
        0x167,
        b"0.75",
        "upgrade",
        TokenWriter::String,
        &mut report,
    )
    .unwrap();
    compare_token(
        &tokens,
        u32::MAX,
        b"",
        "missing",
        TokenWriter::String,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("missing (token 4294967295)"))
    );
    compare_token(
        &tokens,
        17,
        &[b'a', 0xfe],
        "changed",
        TokenWriter::Token,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
}

/// The archetype comes from the content: the type itself when it is an
/// archetype, else its `archetype`, else its parent's. A type the content
/// declares twice is the later declaration, reset and read again when
/// names match under the lexer's fold.
#[test]
fn variant_archetypes_follow_the_content_equipment_types() {
    let (equipment, _) = installed_content_core::equipment::fixture::load(&[(
        "common/units/equipment/planes.txt",
        "equipments = {
            airframe = { is_archetype = yes }
            airframe_1 = { archetype = airframe }
            airframe_2 = { parent = airframe_1 }
            twice = { is_archetype = yes }
            Twice = { archetype = airframe }
        }",
    )]);
    let tokens: TokenNames<'static> = BTreeMap::from([
        (1, b"AIRFRAME".as_slice()),
        (2, b"airframe_1".as_slice()),
        (3, b"airframe_2".as_slice()),
        (4, b"twice".as_slice()),
        (5, b"unknown".as_slice()),
    ]);
    let run = |rows: Vec<(u32, u32)>| {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 22,
            metrics: Default::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(
                rows.into_iter()
                    .enumerate()
                    .map(
                        |(id, (equipment_token, archetype_token))| VariantDefinitionRow {
                            id: id as u64 + 1,
                            equipment_token,
                            archetype_token,
                            ..Default::default()
                        },
                    )
                    .collect(),
            ));
        let mut report = Report::default();
        compare_variant_archetypes(&frame, &equipment, &tokens, &mut report).unwrap();
        report
    };
    let report = run(vec![(1, 1), (2, 1), (3, 1), (4, 1)]);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["VariantDefinitions.archetype_token"],
        4
    );
    assert!(report.limitations.is_empty(), "{:?}", report.limitations);
    // A wrong archetype and a type the content lacks.
    let report = run(vec![(3, 2), (5, 1)]);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
}

/// Variants the live #63 recording (LR63B_O1, hour 60827795) held,
/// with their recorded equipment and archetype literals: their types
/// exist only as `duplicate_archetypes` copies of the vanilla airframe
/// and chassis types (content trimmed to the expansion's fields). The
/// oracle found each "absent from the content" before the expansion.
#[test]
fn duplicated_archetype_variants_of_the_live_save_match_the_content() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (equipment, _) = installed_content_core::equipment::fixture::load(&[
        (
            "common/units/equipment/plane_airframes.txt",
            "equipments = {
small_plane_airframe = { is_archetype = yes type = fighter }
small_plane_airframe_0 = { archetype = small_plane_airframe }
small_plane_airframe_1 = { archetype = small_plane_airframe parent = small_plane_airframe_0 }
light_tank_chassis = { is_archetype = yes type = armor }
light_tank_chassis_1 = { archetype = light_tank_chassis }
}",
        ),
        (
            "common/units/equipment/x_plane_airframes.txt",
            "duplicate_archetypes = {
small_plane_cas_airframe = { archetype = small_plane_airframe type = cas }
small_plane_naval_bomber_airframe = { archetype = small_plane_airframe type = naval_bomber }
light_tank_destroyer_chassis = { archetype = light_tank_chassis type = { armor anti_tank } }
}",
        ),
    ]);
    assert!(equipment.lost.is_none(), "{:?}", equipment.lost);
    let literals: [&[u8]; 7] = [
        b"small_plane_cas_airframe",
        b"small_plane_cas_airframe_0",
        b"small_plane_cas_airframe_1",
        b"small_plane_naval_bomber_airframe",
        b"small_plane_naval_bomber_airframe_1",
        b"light_tank_destroyer_chassis",
        b"light_tank_destroyer_chassis_1",
    ];
    let tokens: TokenNames<'static> = (1..).zip(literals).collect();
    // (variant, equipment token, archetype token) as recorded.
    let rows = [
        (5_896_990_097_478, 2, 1),
        (5_905_580_032_070, 3, 1),
        (5_909_874_999_366, 5, 4),
        (6_987_911_790_662, 7, 6),
    ];
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 60_827_795,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            rows.into_iter()
                .map(
                    |(id, equipment_token, archetype_token)| VariantDefinitionRow {
                        id,
                        equipment_token,
                        archetype_token,
                        ..Default::default()
                    },
                )
                .collect(),
        ));
    let mut report = Report::default();
    compare_variant_archetypes(&frame, &equipment, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["VariantDefinitions.archetype_token"],
        4
    );
    // A copy recorded under its source archetype is still a difference.
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 5_896_990_097_478,
            equipment_token: 2,
            archetype_token: 4,
            ..Default::default()
        }]));
    let mut report = Report::default();
    compare_variant_archetypes(&frame, &equipment, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn variant_version_oracle_matches_raw_byte_zero_omission_and_old_absence() {
    let variant = (1_u64 << 32) | 70;
    let country = CountryRow {
        id: 1,
        tag: b"SYN".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(1, &country)]);
    let tokens = TokenNames::from([(17, b"synthetic".as_slice())]);
    let run = |version, saved_version: &str| {
        let source = format!(
            "equipments={{synthetic={{id={{id=1 type=70}} creator=SYN origin=SYN {saved_version}}}}}"
        );
        let saved = production::parse(&parser::parse(source.as_bytes()).unwrap().items).unwrap();
        let mut frame = output_frame(22, false, &[], &[]);
        *frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: variant,
                equipment_token: 17,
                creator_link: 1,
                origin_link: 1,
                append_variant_name: 1,
                version,
                ..Default::default()
            }]));
        *frame.tables[TableId::VariantStats.index()] =
            TableView::Complete(TableRows::VariantStats(vec![VariantStatsRow {
                variant,
                ..Default::default()
            }]));
        let mut report = Report::default();
        compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
        report
    };
    for (version, saved) in [(255, "version=255"), (0, "")] {
        let report = run(Recorded::Value(version), saved);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
    }
    for (version, saved) in [(254, "version=255"), (0, "version=0")] {
        let report = run(Recorded::Value(version), saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("version"));
    }
    let report = run(Recorded::NotRecorded, "version=255");
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason == "VariantDefinitions.version not recorded")
    );
}

#[test]
fn typed_variants_keep_independent_checks_when_lexical_tokens_are_unavailable() {
    let source = br#"equipments={plane={id={id=4294967295 type=70}
        name="A" position="II" creator=GER origin=GER
        modules={slot=module} upgrades={upgrades={{engine 255}}}}}
    "#;
    let saved = production::parse(&parser::parse(source).unwrap().items).unwrap();
    let variant = 0xffff_ffff_0000_0046;
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 22,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: variant,
            equipment_token: 17,
            creator_link: 11,
            origin_link: 11,
            append_variant_name: 0xff,
            display_name: b"A".to_vec(),
            variant_name: b"II".to_vec(),
            classifier_flags: u64::MAX,
            mission_mask: u32::MAX,
            ..Default::default()
        }]));
    *frame.tables[TableId::VariantModules.index()] =
        TableView::Complete(TableRows::VariantModules(vec![
            VariantModuleRow {
                variant,
                source_ordinal: 0,
                slot: u32::MAX,
                definition_token: u32::MAX,
                definition_active: 0,
            },
            VariantModuleRow {
                variant,
                source_ordinal: 1,
                slot: 18,
                definition_token: 19,
                definition_active: 0xff,
            },
        ]));
    *frame.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(TableRows::VariantUpgrades(vec![VariantUpgradeRow {
            variant,
            source_ordinal: 0,
            definition_token: 20,
            level: 255,
        }]));
    *frame.tables[TableId::VariantStats.index()] =
        TableView::Complete(TableRows::VariantStats(vec![VariantStatsRow {
            variant,
            values: [i64::MIN; 10],
        }]));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "lexical writer closure unproved".into(),
    };
    let country = CountryRow {
        id: 11,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(11, &country)]);
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["VariantDefinitions"], 1);
    assert_eq!(report.compared_tables["VariantModules"], 1);
    assert_eq!(report.compared_tables["VariantUpgrades"], 1);
    assert_eq!(report.compared_tables["VariantStats"], 0);
    let unverified = report
        .limitations
        .iter()
        .find(|(reason, _)| reason.contains("this field's saved literal is unverified"))
        .unwrap()
        .1;
    assert_eq!(unverified.len(), 4);

    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 17,
                literal: b"plane".to_vec(),
            },
            LexicalTokenRow {
                token: 18,
                literal: b"slot".to_vec(),
            },
            LexicalTokenRow {
                token: 19,
                literal: b"module".to_vec(),
            },
            LexicalTokenRow {
                token: 20,
                literal: b"engine".to_vec(),
            },
        ]));
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        !report
            .limitations
            .keys()
            .any(|reason| reason.contains("saved literal is unverified"))
    );
    let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    for row in rows {
        row.literal.extend_from_slice(&[0, 0xff]);
    }
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert!(report.differences.is_empty());
    let unverified = report
        .limitations
        .iter()
        .find(|(reason, _)| reason.contains("recorded lexical spelling contains an interior NUL"))
        .unwrap()
        .1;
    assert_eq!(unverified.len(), 4);
    let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    for row in rows {
        if row.token == 17 {
            continue;
        }
        row.literal
            .truncate(row.literal.iter().position(|&byte| byte == 0).unwrap());
        row.literal[0] = b'x';
    }
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 3);
    let unverified = report
        .limitations
        .iter()
        .find(|(reason, _)| reason.contains("recorded lexical spelling contains an interior NUL"))
        .unwrap()
        .1;
    assert_eq!(unverified.len(), 1);
    let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    let row = rows.iter_mut().find(|row| row.token == 17).unwrap();
    row.literal
        .truncate(row.literal.iter().position(|&byte| byte == 0).unwrap());
    row.literal[0] = b'x';
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 4);
    for field in [
        ".equipment_token",
        ".modules[0].slot",
        ".modules[0].definition_token",
        ".upgrades[0].definition_token",
    ] {
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(field)),
            "{field}"
        );
    }
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "lexical writer closure unproved".into(),
    };

    let mut wider_save = saved.clone();
    let mut extra = wider_save.variants.as_ref().unwrap()[&variant].clone();
    extra.id = 70;
    extra.modules = None;
    extra.upgrades = None;
    wider_save
        .variants
        .as_mut()
        .unwrap()
        .insert(extra.id, extra);
    let mut report = Report::default();
    assert!(compare_variants(&frame, &wider_save, &links, &BTreeMap::new(), &mut report).is_err());
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .all(|field| field.contains("variant 70"))
    );

    let TableView::Complete(TableRows::VariantUpgrades(rows)) =
        &mut *frame.tables[TableId::VariantUpgrades.index()]
    else {
        unreachable!()
    };
    rows[0].level = 254;
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains(".level"));

    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 17,
            literal: b"different_equipment".to_vec(),
        }]));
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .any(|field| field.contains(".equipment_token"))
    );
    *frame.tables[TableId::VariantStats.index()] =
        TableView::Complete(TableRows::VariantStats(Vec::new()));
    *frame.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
        reason: "variant identity unreadable".into(),
    };
    let mut report = Report::default();
    let tokens = lexical_tokens(&frame, &mut report).unwrap();
    assert!(compare_variants(&frame, &saved, &links, &tokens, &mut report).is_err());
    assert!(report.failed_comparisons.contains_key("VariantDefinitions"));
    assert!(report.failed_comparisons.contains_key("VariantStats"));
    assert_eq!(report.compared_tables["VariantModules"], 1);
    assert_eq!(report.compared_tables["VariantUpgrades"], 1);
    assert!(
        report
            .differences
            .iter()
            .any(|field| field.contains(".level"))
    );
}

#[test]
fn typed_army_upgrades_compare_priority_progress_and_ordered_produced_amounts() {
    let source = br"countries={GER={units={division={id={id=4294967295 type=51}
        requests={upgrades={priority=-1
            request={request=-0.00001 total=92233720368547.75807
                equipment_variant_index={id=0 type=70}}
            delivery={status=1 progress=-92233720368547.75808 produced={
                equipment={id={id=0 type=70} amount=-0.00001}
                allow_zero_entries=no}}
        }}
    }}}}";
    let saved = production::parse(&parser::parse(source).unwrap().items).unwrap();
    let army = 0xffff_ffff_0000_0033;
    let country = CountryRow {
        id: 11,
        tag: b"GER".to_vec(),
        ..Default::default()
    };
    let other_country = CountryRow {
        id: 12,
        tag: b"FRA".to_vec(),
        ..Default::default()
    };
    let links = BTreeMap::from([(11, &country), (12, &other_country)]);
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 22,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    *frame.tables[TableId::ArmyUpgradeGroups.index()] =
        TableView::Complete(TableRows::ArmyUpgradeGroups(vec![ArmyUpgradeGroupRow {
            country: 11,
            army,
            priority: u32::MAX,
        }]));
    *frame.tables[TableId::ArmyUpgradeRequests.index()] =
        TableView::Complete(TableRows::ArmyUpgradeRequests(vec![
            ArmyUpgradeRequestRow {
                country: 11,
                army,
                ordinal: 0,
                equipment: 70,
                request_raw: -1,
                total_raw: i64::MAX,
            },
        ]));
    *frame.tables[TableId::ArmyUpgradeDeliveries.index()] =
        TableView::Complete(TableRows::ArmyUpgradeDeliveries(vec![
            ArmyUpgradeDeliveryRow {
                country: 11,
                army,
                ordinal: 0,
                status: 1,
                progress_raw: Some(i64::MIN),
                total_progress_raw: 100_000,
                allow_zero_entries: 0,
            },
        ]));
    *frame.tables[TableId::ArmyUpgradeProduced.index()] =
        TableView::Complete(TableRows::ArmyUpgradeProduced(vec![
            ArmyUpgradeProducedRow {
                country: 11,
                army,
                delivery_ordinal: 0,
                ordinal: 0,
                equipment: 70,
                amount_raw: 0,
            },
            ArmyUpgradeProducedRow {
                country: 11,
                army,
                delivery_ordinal: 0,
                ordinal: 1,
                equipment: 70,
                amount_raw: -1,
            },
        ]));
    let mut report = Report::default();
    compare_army_upgrades(&frame, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    for table in [
        "ArmyUpgradeGroups",
        "ArmyUpgradeRequests",
        "ArmyUpgradeDeliveries",
        "ArmyUpgradeProduced",
    ] {
        assert_eq!(report.compared_tables[table], 1);
    }
    assert_eq!(report.scalar_checks, 5);
    let TableView::Complete(TableRows::ArmyUpgradeGroups(rows)) =
        &mut *frame.tables[TableId::ArmyUpgradeGroups.index()]
    else {
        unreachable!()
    };
    rows[0].priority = 1;
    let TableView::Complete(TableRows::ArmyUpgradeDeliveries(rows)) =
        &mut *frame.tables[TableId::ArmyUpgradeDeliveries.index()]
    else {
        unreachable!()
    };
    rows[0].progress_raw = Some(i64::MIN + 1);
    let TableView::Complete(TableRows::ArmyUpgradeRequests(rows)) =
        &mut *frame.tables[TableId::ArmyUpgradeRequests.index()]
    else {
        unreachable!()
    };
    rows[0].total_raw -= 1;
    let TableView::Complete(TableRows::ArmyUpgradeProduced(rows)) =
        &mut *frame.tables[TableId::ArmyUpgradeProduced.index()]
    else {
        unreachable!()
    };
    rows[1].amount_raw -= 1;
    let mut report = Report::default();
    compare_army_upgrades(&frame, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 4);
    *frame.tables[TableId::ArmyUpgradeRequests.index()] = TableView::Unavailable {
        reason: "request graph unreadable".into(),
    };
    let TableView::Complete(TableRows::ArmyUpgradeGroups(rows)) =
        &mut *frame.tables[TableId::ArmyUpgradeGroups.index()]
    else {
        unreachable!()
    };
    rows[0].country = 12;
    let mut report = Report::default();
    assert!(compare_army_upgrades(&frame, &saved, &links, &mut report).is_err());
    assert_eq!(report.differences.len(), 4);
    assert!(
        report
            .differences
            .iter()
            .any(|field| field.contains(".upgrades.country"))
    );
    assert!(
        report
            .failed_comparisons
            .contains_key("ArmyUpgradeRequests")
    );
    assert!(report.failed_comparisons.contains_key("ArmyUpgradeLinks"));
    assert_eq!(report.compared_tables["ArmyUpgradeGroups"], 1);
    assert_eq!(report.compared_tables["ArmyUpgradeDeliveries"], 1);
    assert_eq!(report.compared_tables["ArmyUpgradeProduced"], 1);
}

#[test]
fn staggered_off_hours_skips_variants_and_prints_refresh_hour() {
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
    compare_variants(
        &frame,
        &production::Pack::default(),
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains(replay_data::cadence::STAGGER_LIMITATION)),
        "{:?}",
        report.limitations.keys()
    );
}
