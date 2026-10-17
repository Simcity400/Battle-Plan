use super::*;
use replay::{
    frame::TableView,
    rows::{CombatLogEquipmentRow, LendLeaseEquipmentRow, SliceReadRow, TableId, TableRows},
};

const GER: u32 = 4;
const SOV: u32 = 9;
/// 1943.10.30.11 in the save's text form.
const DATE: i32 = 60_827_938;
const SEPTEMBER: i32 = DATE - 24 * 30;
const AUGUST: i32 = DATE - 24 * 61;

#[test]
fn census_compares_all_saved_divisions_groups_and_expeditionary_owners() {
    use replay::rows::DivisionCensusRow;
    let units = "units={ division={ id={id=1 type=51} logical_country=GER strength=1 organisation=1 } division={ id={id=2 type=51} logical_country=GER expeditionary_owner=SOV strength=1 organisation=1 } }";
    let saved = save_with(
        &[],
        &format!(
            "{units} theatres={{ theatre={{ id={{id=1 type=56}} orders_group={{ id={{id=1 type=53}} name=\"Army\" field_marshal_group=no motorization_level=0 member={{unit={{id=1 type=51}}}} }} }} }}"
        ),
    );
    let mut recorded = frame(vec![], &[]);
    let rows = vec![
        DivisionCensusRow {
            country: GER,
            grouped: 0,
            ungrouped: 0,
        },
        DivisionCensusRow {
            country: SOV,
            grouped: 0,
            ungrouped: 1,
        },
        DivisionCensusRow {
            country: 14,
            grouped: 1,
            ungrouped: 0,
        },
    ];
    *recorded.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(rows.clone()));
    let countries = links();
    let links = BTreeMap::from([
        (GER, &countries[0]),
        (SOV, &countries[1]),
        (14, &countries[0]),
    ]);
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["DivisionCensus"], 2);
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|text| text.contains("grouped and ungrouped counts compared"))
    );
    let mut wrong = rows.clone();
    wrong[1].grouped = 1;
    wrong[1].ungrouped = 0;
    *recorded.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(wrong));
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    // No definitions mean no grouped divisions, even when totals match.
    let saved = save_with(&[], units);
    assert!(saved.naval_history.theatre_countries.is_some());
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    // Both countries' grouped and ungrouped fields differ, never waived.
    assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
    for tag in ["GER.", "SOV."] {
        assert_eq!(
            report
                .differences
                .iter()
                .filter(|text| text.starts_with(tag))
                .count(),
            2
        );
    }
    *recorded.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(rows));
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .all(|text| text.starts_with("GER."))
    );
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|text| text.contains("only the named certified omitted members qualify"))
    );
    assert!(report.limitations.keys().any(|text| {
        text.contains("the army save writer omits the parent reference")
            && text.contains("the formation save writer")
    }));
    let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &mut *recorded.tables[TableId::DivisionCensus.index()]
    else {
        unreachable!()
    };
    rows[2].grouped = 0;
    rows[2].ungrouped = 1;
    rows[1].ungrouped += 1;
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .any(|text| text.contains(".total"))
    );
    *recorded.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    assert!(division_census(&recorded, &saved, &links, &mut Report::default()).is_err());
}

#[test]
fn an_empty_saved_group_still_supplies_membership_data() {
    use replay::rows::DivisionCensusRow;
    let saved = save_with(
        &[],
        "units={ division={ id={id=1 type=51} logical_country=GER strength=1 organisation=1 } } theatres={ theatre={ id={id=1 type=56} orders_group={ id={id=1 type=53} name=\"Empty army\" field_marshal_group=no motorization_level=0 } } }",
    );
    let mut recorded = frame(vec![], &[]);
    *recorded.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(vec![
            DivisionCensusRow {
                country: GER,
                grouped: 1,
                ungrouped: 0,
            },
            DivisionCensusRow {
                country: SOV,
                grouped: 0,
                ungrouped: 0,
            },
        ]));
    let countries = links();
    let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        !report
            .limitations
            .values()
            .flatten()
            .any(|text| text.contains("only total divisions compared"))
    );
}

#[test]
fn army_group_members_do_not_duplicate_child_army_members() {
    let text = |army_group| {
        format!(
            "units={{ division={{ id={{id=1 type=51}} logical_country=GER strength=1 organisation=1 }} }} theatres={{ theatre={{ id={{id=1 type=56}} orders_group={{ id={{id=1 type=53}} name=\"Army\" field_marshal_group=no motorization_level=0 member={{unit={{id=1 type=51}}}} }} orders_group={{ id={{id=2 type=53}} name=\"Parent\" field_marshal_group={army_group} motorization_level=0 member={{unit={{id=1 type=51}}}} }} }} }}"
        )
    };
    let saved = save_with(&[], &text("yes"));
    let counts = saved_division_counts(&saved, &BTreeSet::new()).unwrap();
    assert_eq!(counts[b"GER".as_slice()], (1, 0, 0));
    assert_eq!(counts[b"SOV".as_slice()], (0, 0, 0));
    let saved = save_with(&[], &text("no"));
    assert_eq!(
        saved_division_counts(&saved, &BTreeSet::new())
            .unwrap_err()
            .to_string(),
        "saved division belongs to multiple orders groups"
    );
}

/// One saved division has a reciprocally observed group in the omitted
/// slot-zero theatre; the other remains independently ungrouped.
fn omitted_group_census_fixture() -> (DecodedFrame, Save) {
    use replay::rows::{
        DivisionCensusRow, DivisionRow, LandTheatreRow, OrdersGroupMemberRow, OrdersGroupRow,
        Recorded, ReservedCountryNamingRow,
    };
    let saved = save_with(
        &[],
        "units={ division={ id={id=1 type=51} logical_country=GER strength=1 organisation=1 } division={ id={id=2 type=51} logical_country=GER strength=1 organisation=1 } }",
    );
    let mut recorded = frame(vec![], &[]);
    let theatre = (10_u64 << 32) | 67;
    let group = (9_u64 << 32) | 53;
    let mut set = |rows: TableRows| {
        let index = rows.table().index();
        *recorded.tables[index] = TableView::Complete(rows);
    };
    set(TableRows::ReservedCountryNaming(vec![
        ReservedCountryNamingRow {
            tag: b"ZERO".to_vec(),
            land_theatre_count: Recorded::Value(Some(1)),
            country_array: Recorded::Value(Some(vec![0, GER, SOV])),
            ..Default::default()
        },
    ]));
    set(TableRows::LandTheatres(vec![LandTheatreRow {
        id: theatre,
        owner: 0,
        ordinal: 0,
        volunteers_theatre: 0,
    }]));
    set(TableRows::OrdersGroups(vec![OrdersGroupRow {
        handle: group,
        theatre,
        theatre_ordinal: Some(0),
        ..Default::default()
    }]));
    set(TableRows::OrdersGroupMembers(vec![OrdersGroupMemberRow {
        group,
        ordinal: 0,
        unit: (1_u64 << 32) | 51,
    }]));
    set(TableRows::Divisions(vec![
        DivisionRow {
            id: (1_u64 << 32) | 51,
            roster_owner: GER,
            orders_group: group,
            ..Default::default()
        },
        DivisionRow {
            id: (2_u64 << 32) | 51,
            roster_owner: GER,
            ..Default::default()
        },
    ]));
    set(TableRows::DivisionCensus(vec![
        DivisionCensusRow {
            country: GER,
            grouped: 1,
            ungrouped: 1,
        },
        DivisionCensusRow {
            country: SOV,
            grouped: 0,
            ungrouped: 0,
        },
    ]));
    (recorded, saved)
}

#[test]
fn omitted_group_split_qualifies_only_certified_members_and_keeps_total_and_other_members_exact() {
    use replay::rows::{
        DivisionCensusRow, DivisionRow, LandTheatreRow, OrdersGroupMemberRow, OrdersGroupRow,
    };
    let (mut recorded, saved) = omitted_group_census_fixture();
    let countries = links();
    let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|text| text.contains("1 certified omitted-group members"))
    );
    let group = (11_u64 << 32) | 53;
    let theatre = (11_u64 << 32) | 67;
    if let TableView::Complete(TableRows::LandTheatres(rows)) =
        &mut *recorded.tables[TableId::LandTheatres.index()]
    {
        rows.push(LandTheatreRow {
            id: theatre,
            owner: GER,
            ordinal: 0,
            volunteers_theatre: 0,
        });
    } else {
        unreachable!();
    }
    if let TableView::Complete(TableRows::OrdersGroups(rows)) =
        &mut *recorded.tables[TableId::OrdersGroups.index()]
    {
        rows.push(OrdersGroupRow {
            handle: group,
            theatre,
            theatre_ordinal: Some(0),
            ..Default::default()
        });
    } else {
        unreachable!();
    }
    if let TableView::Complete(TableRows::OrdersGroupMembers(rows)) =
        &mut *recorded.tables[TableId::OrdersGroupMembers.index()]
    {
        rows.push(OrdersGroupMemberRow {
            group,
            ordinal: 0,
            unit: (2_u64 << 32) | 51,
        });
    } else {
        unreachable!();
    }
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *recorded.tables[TableId::Divisions.index()]
    {
        rows[1] = DivisionRow {
            orders_group: group,
            ..rows[1].clone()
        };
    } else {
        unreachable!();
    }
    *recorded.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(vec![
            DivisionCensusRow {
                country: GER,
                grouped: 2,
                ungrouped: 0,
            },
            DivisionCensusRow {
                country: SOV,
                grouped: 0,
                ungrouped: 0,
            },
        ]));
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        !report
            .differences
            .iter()
            .any(|text| text.contains(".total"))
    );
    if let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &mut *recorded.tables[TableId::DivisionCensus.index()]
    {
        rows[0].ungrouped = 1;
    } else {
        unreachable!();
    }
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|text| text.contains(".total"))
    );
}

#[test]
fn omitted_group_qualification_refusals_preserve_total_and_strict_split_diagnostics() {
    for missing_members in [false, true] {
        let (mut recorded, saved) = omitted_group_census_fixture();
        if missing_members {
            *recorded.tables[TableId::OrdersGroupMembers.index()] = TableView::Unavailable {
                reason: "member census refused".into(),
            };
        } else {
            let TableView::Complete(TableRows::Divisions(rows)) =
                &mut *recorded.tables[TableId::Divisions.index()]
            else {
                unreachable!();
            };
            rows[0].orders_group = 0;
        }
        let TableView::Complete(TableRows::DivisionCensus(rows)) =
            &mut *recorded.tables[TableId::DivisionCensus.index()]
        else {
            unreachable!();
        };
        rows[0].grouped = 2;
        let countries = links();
        let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
        let mut report = Report::default();
        division_census(&recorded, &saved, &links, &mut report).unwrap();
        assert!(
            report
                .failed_comparisons
                .contains_key("DivisionCensus omitted-group qualification")
        );
        for field in [".total", ".grouped", ".ungrouped"] {
            assert!(report.differences.iter().any(|text| text.contains(field)));
        }
    }
}

#[test]
fn recorded_grouped_count_cannot_fall_below_the_certified_omitted_members() {
    let (mut recorded, saved) = omitted_group_census_fixture();
    let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &mut *recorded.tables[TableId::DivisionCensus.index()]
    else {
        unreachable!();
    };
    rows[0].grouped = 0;
    rows[0].ungrouped = 2;
    let countries = links();
    let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .any(|text| text.contains("at least 1"))
    );
    assert!(
        !report
            .differences
            .iter()
            .any(|text| text.contains(".total"))
    );
}

#[test]
fn serialized_definitions_cannot_be_qualified_as_omitted_group_members() {
    let (recorded, mut saved) = omitted_group_census_fixture();
    theatre_groups(&mut saved, b"GER", &[9]); // Same full group identity, serialized empty.
    let countries = links();
    let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
    assert!(
        !report
            .limitations
            .values()
            .flatten()
            .any(|text| text.contains("certified omitted-group members"))
    );
    let group = &mut saved
        .naval_history
        .theatre_countries
        .as_mut()
        .unwrap()
        .get_mut(b"GER".as_slice())
        .unwrap()
        .land[0]
        .definitions[0];
    group.members.push((1_u64 << 32) | 51);
    let mut report = Report::default();
    division_census(&recorded, &saved, &links, &mut report).unwrap();
    assert!(report.differences.is_empty());
}

/// Read-only check of an external text save with the oracle's parser and
/// census attribution. Set BATTLE_PLAN_CENSUS_SAVE to its path before running.
#[test]
#[ignore]
fn division_census_from_text_save() {
    let path = std::env::var_os("BATTLE_PLAN_CENSUS_SAVE").or_else(|| std::env::var_os("HOI4_CENSUS_SAVE")).expect("set BATTLE_PLAN_CENSUS_SAVE");
    let bytes = std::fs::read(&path).unwrap();
    let saved = installed_content_save::parse(&bytes).unwrap();
    let mut memberships: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    let mut army_memberships: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    let mut groups = [0; 2];
    for group in saved
        .naval_history
        .theatre_countries
        .iter()
        .flat_map(|countries| countries.values())
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
    {
        groups[usize::from(group.is_army_group)] += 1;
        for &member in &group.members {
            if saved.divisions.contains_key(&member) {
                memberships.entry(member).or_default().insert(group.id);
                if !group.is_army_group {
                    army_memberships.entry(member).or_default().insert(group.id);
                }
            }
        }
    }
    println!(
        "save={path:?} countries={} divisions={} armies={} army_groups={}",
        saved.countries.len(),
        saved.divisions.len(),
        groups[0],
        groups[1],
    );
    for (label, memberships) in [
        ("all groups", &memberships),
        ("armies only", &army_memberships),
    ] {
        let duplicates: Vec<_> = memberships
            .iter()
            .filter(|(_, groups)| groups.len() > 1)
            .collect();
        println!(
            "{label}: duplicate divisions={} {duplicates:?}",
            duplicates.len()
        );
    }
    for (tag, (grouped, ungrouped, _)) in saved_division_counts(&saved, &BTreeSet::new()).unwrap() {
        println!(
            "{}: grouped={grouped} ungrouped={ungrouped} total={}",
            String::from_utf8_lossy(tag),
            u64::from(grouped) + u64::from(ungrouped),
        );
    }
}

fn frame(rows: Vec<CombatLogEquipmentRow>, read: &[u32]) -> DecodedFrame {
    let reads: Vec<SliceReadRow> = read
        .iter()
        .map(|&owner| SliceReadRow {
            table: TableId::CombatLogEquipment.index() as u32,
            owner,
            sample_hour: DATE,
        })
        .collect();
    let tables: [TableView; TableId::ALL.len()] = std::array::from_fn(|index| {
        let table = TableId::ALL[index];
        match table {
            TableId::CombatLogEquipment => {
                TableView::Complete(TableRows::CombatLogEquipment(rows.clone()))
            }
            TableId::SliceReads => TableView::Complete(TableRows::SliceReads(reads.clone())),
            _ => TableView::Complete(TableRows::empty(table)),
        }
    });
    DecodedFrame {
        branch: 1,
        hour: DATE,
        metrics: Default::default(),
        tables: tables.map(Into::into),
    }
}

fn row_at(date: i32, kind: u8, variant: u32, amount_raw: i64) -> CombatLogEquipmentRow {
    CombatLogEquipmentRow {
        country: GER,
        group: 12793,
        kind,
        month: month_bucket(date) as u16,
        variant,
        date,
        amount_raw,
    }
}

fn row(kind: u8, variant: u32, amount_raw: i64) -> CombatLogEquipmentRow {
    row_at(DATE, kind, variant, amount_raw)
}

/// One saved month entry: its key, reason, date and (variant, amount) pairs.
fn month_at(key: &str, reason: i32, date: &str, pool: &[(u32, &str)]) -> String {
    let entries: String = pool
        .iter()
        .map(|(variant, amount)| {
            format!("equipment={{ id={{ id={variant} type=70 }} amount={amount} }} ")
        })
        .collect();
    format!(
        "{key}={{ equipment={{ {entries}allow_zero_entries=no }} reason={reason} date=\"{date}\" }} "
    )
}

fn month(key: &str, reason: i32, pool: &[(u32, &str)]) -> String {
    month_at(key, reason, "1943.10.30.11", pool)
}

fn save_with(months: &[String], extra: &str) -> Save {
    let text = format!(
        "HOI4txt\ndate=\"1943.10.30.11\" game_unique_id=\"campaign\" version=\"build\" game_unique_seed=-42 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13\ncountries={{ GER={{ {extra} }} SOV={{ }} }}\nstates={{}}\nprovinces={{}}\ncombat_log={{ tag=\"GER\" log={{ group={{ id=12793 type=53 }} {} }} }}\ncombat_log={{ tag=\"SOV\" log={{ group={{ id=500 type=53 }} {} }} }}\n",
        months.concat(),
        month("equipment", 0, &[(9, "1")]),
    );
    installed_content_save::parse(text.as_bytes()).unwrap()
}

fn save(months: &[String]) -> Save {
    save_with(months, "")
}

fn theatre_groups(save: &mut Save, tag: &[u8], groups: &[u32]) {
    use installed_content_save::naval_history::{LandTheatre, OrdersGroup, TheatreCountry};
    save.naval_history.theatre_countries = Some(BTreeMap::from([(
        tag.to_vec(),
        TheatreCountry {
            land: vec![LandTheatre {
                id: 1 << 32 | 56,
                volunteers_country: None,
                groups: vec![],
                units: vec![],
                area_representatives: vec![],
                definitions: groups
                    .iter()
                    .map(|&group| OrdersGroup {
                        id: u64::from(group) << 32 | u64::from(GROUP_TYPE),
                        ..OrdersGroup::default()
                    })
                    .collect(),
            }],
            air: vec![],
        },
    )]));
}

fn links() -> [CountryRow; 2] {
    [
        CountryRow {
            tag: b"GER".to_vec(),
            ..CountryRow::default()
        },
        CountryRow {
            tag: b"SOV".to_vec(),
            ..CountryRow::default()
        },
    ]
}

fn run(frame: &DecodedFrame, save: &Save, full: bool) -> io::Result<Report> {
    let countries = links();
    let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
    let mut report = Report {
        forced_full: full,
        ..Report::default()
    };
    compare(frame, save, &links, &mut report)?;
    Ok(report)
}

#[test]
fn matching_logs_have_no_differences_and_zeros_may_be_omitted() {
    let frame = frame(
        vec![
            row(0, 1326, 300_000),
            row(0, 1327, 0),
            row(2, 1326, 50_000),
            row(RECOVERED_KIND, 1846, 100_000),
            CombatLogEquipmentRow {
                country: SOV,
                group: 500,
                ..row(0, 9, 100_000)
            },
        ],
        &[],
    );
    let save = save(&[
        month("equipment", 0, &[(1326, "3")]),
        month("equipment", 2, &[(1326, "0.5")]),
        month("enemy_equipment", 0, &[(5300, "2")]),
        month("equipment_recovered", 0, &[(1846, "1")]),
    ]);
    let report = run(&frame, &save, true).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CombatLogEquipment"], 4);
}

#[test]
fn a_missing_log_of_a_still_existing_group_is_a_difference() {
    let frame = frame(
        vec![CombatLogEquipmentRow {
            group: 999,
            ..row(0, 1326, 100_000)
        }],
        &[GER],
    );
    let mut save = save(&[]);
    // Reference ids are global: a transfer to another country is not deletion.
    theatre_groups(&mut save, b"SOV", &[999]);
    let report = run(&frame, &save, false).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("group 999")),
        "{:?}",
        report.differences
    );
}

#[test]
fn every_mismatch_is_a_difference() {
    let frame = frame(
        vec![
            row(0, 1326, 300_001),
            row(1, 1326, 100_000),
            row(RECOVERED_KIND, 1846, 5),
        ],
        &[],
    );
    let save = save(&[
        month("equipment", 0, &[(1326, "3"), (1327, "1")]),
        month("equipment", 2, &[(1326, "1")]),
    ]);
    let report = run(&frame, &save, true).unwrap();
    let differences = report.differences.join("\n");
    // GER's four plus SOV's saved month, never recorded.
    assert_eq!(report.differences.len(), 6, "{differences}");
    assert!(differences.contains("[kind 0]") && differences.contains("amount_raw"));
    assert!(differences.contains("[variant 1327]"));
    assert!(differences.contains("[kind 1]") && differences.contains("[kind 2]"));
    assert!(differences.contains("[kind 4]"));
    assert!(differences.contains("SOV.combat_log[group 500]"));
}

#[test]
fn a_sliced_hour_compares_the_countries_read_and_their_last_two_months() {
    // GER was read now: its window (September, October) is recorded; its
    // August entry is not expected. SOV was not read: not compared.
    let frame = frame(
        vec![
            row_at(SEPTEMBER, 0, 1326, 200_000),
            row_at(DATE, 0, 1326, 300_000),
        ],
        &[GER],
    );
    let save = save(&[
        month_at("equipment", 0, "1943.8.30.11", &[(1326, "1")]),
        month_at("equipment", 0, "1943.9.30.11", &[(1326, "2")]),
        month("equipment", 0, &[(1326, "3")]),
    ]);
    let report = run(&frame, &save, false).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CombatLogEquipment"], 2);
    // The same rows at a full hour miss August and SOV.
    let report = run(&frame, &save, true).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    let _ = AUGUST;
}

/// A country whose last complete read is older is read whole outside a
/// full hour: its older months are compared, never differences for
/// being there.
#[test]
fn a_whole_read_at_a_sliced_hour_compares_its_older_months() {
    let save = save(&[
        month_at("equipment", 0, "1943.8.30.11", &[(1326, "1")]),
        month_at("equipment", 0, "1943.9.30.11", &[(1326, "2")]),
        month("equipment", 0, &[(1326, "3")]),
    ]);
    let whole = |august: i64| {
        frame(
            vec![
                row_at(AUGUST, 0, 1326, august),
                row_at(SEPTEMBER, 0, 1326, 200_000),
                row_at(DATE, 0, 1326, 300_000),
            ],
            &[GER],
        )
    };
    let report = run(&whole(100_000), &save, false).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["CombatLogEquipment"], 3);
    let report = run(&whole(100_001), &save, false).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("amount_raw"));
}

#[test]
fn a_missing_log_without_theatre_definitions_is_a_difference() {
    let frame = frame(
        vec![CombatLogEquipmentRow {
            group: 777,
            ..row(0, 1326, 900_000)
        }],
        &[GER],
    );
    let mut save = save(&[]);
    save.naval_history.theatre_countries = None;
    assert!(save.naval_history.theatre_countries.is_none());
    let report = run(&frame, &save, false).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("group 777"));
}

#[test]
fn a_deleted_logs_final_rows_are_listed_not_compared() {
    let frame = frame(
        vec![
            row(0, 1326, 300_000),
            CombatLogEquipmentRow {
                group: 777,
                ..row(0, 1326, 900_000)
            },
        ],
        &[GER],
    );
    let mut save = save(&[month("equipment", 0, &[(1326, "3")])]);
    // The saved census still contains the live group, but not group 777.
    theatre_groups(&mut save, b"GER", &[12793]);
    let report = run(&frame, &save, false).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("daily prune deleted"))
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("completeness is unproven"))
    );
}

#[test]
fn a_saved_reason_outside_the_arrays_is_refused() {
    let save = save(&[month("equipment", 4, &[(1326, "1")])]);
    assert!(run(&frame(vec![], &[]), &save, true).is_err());
}

#[test]
fn lend_lease_compares_every_saved_amount_and_receiver() {
    let exchange = |sunk: &str| {
        format!(
            "resources={{ lend_lease={{ receiver=\"SOV\" id={{ id=97730 type=4713 }} efficiency=1 efficiency_due_to_lost_convoys=1 total_delivered={{ equipment={{ id={{ id=1723 type=70 }} amount=18 }} allow_zero_entries=no }} equipment_sunk={{ {sunk} allow_zero_entries=no }} }} produced={{}} transfer_overlord_subject={{}} imported={{}} to_use={{ {{}} {{}} {{}} }} to_export={{}} base_export={{}} exported={{}} }}"
        )
    };
    let rows = vec![
        LendLeaseEquipmentRow {
            giver: GER,
            exchange: 97730,
            kind: DELIVERED_KIND,
            variant: 1723,
            receiver: SOV,
            amount_raw: 1_800_000,
        },
        LendLeaseEquipmentRow {
            giver: GER,
            exchange: 97730,
            kind: SUNK_KIND,
            variant: 5157,
            receiver: SOV,
            amount_raw: 0,
        },
    ];
    let tables: [TableView; TableId::ALL.len()] = std::array::from_fn(|index| {
        let table = TableId::ALL[index];
        if table == TableId::LendLeaseEquipment {
            TableView::Complete(TableRows::LendLeaseEquipment(rows.clone()))
        } else {
            TableView::Complete(TableRows::empty(table))
        }
    });
    let frame = DecodedFrame {
        branch: 1,
        hour: DATE,
        metrics: Default::default(),
        tables: tables.map(Into::into),
    };
    let countries = links();
    let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
    let save = save_with(&[], &exchange(""));
    let mut report = Report::default();
    lend_lease(&frame, &save, &links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["LendLeaseEquipment"], 1);
    let save = save_with(
        &[],
        &exchange("equipment={ id={ id=5157 type=70 } amount=0.5 }"),
    );
    let mut report = Report::default();
    lend_lease(&frame, &save, &links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}
