use super::*;

mod destruction;

// Existing land-product tests keep the Combat Log source explicit.
fn scan(replay: &Replay, branch: u64, tag: &str, cancelled: &AtomicBool) -> io::Result<Scanned> {
    super::scan(replay, branch, tag, LandSource::CombatLog, None, cancelled)
}

#[test]
fn recorded_census_dates_partial_counts_without_selecting_divisions() {
    use replay_data::rows::DivisionCensusRow;
    assert!(!oracle::SELECTED.contains(&TableId::Divisions));
    let base = 60_827_880;
    let mut first = frame(base, vec![], &[]);
    *first.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "not selected".into(),
    };
    *first.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(vec![
            DivisionCensusRow {
                country: GER_ALIAS,
                grouped: 1,
                ungrouped: 1,
            },
            DivisionCensusRow {
                country: GER,
                grouped: 0,
                ungrouped: 0,
            },
        ]));
    let mut unavailable = first.clone();
    unavailable.hour += 1;
    *unavailable.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
        reason: replay_data::cadence::SKIPPED_REASON.into(),
    };
    let mut changed = first.clone();
    changed.hour += 2;
    *changed.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(vec![DivisionCensusRow {
            country: GER,
            grouped: 1,
            ungrouped: 2,
        }]));
    let mut missing = changed.clone();
    missing.hour += 1;
    *missing.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    let mut recovered = changed.clone();
    recovered.hour += 2;
    *recovered.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(vec![DivisionCensusRow {
            country: GER,
            grouped: 1,
            ungrouped: 0,
        }]));
    let scanned = scan(
        &replay_of(vec![first, unavailable, changed, missing, recovered]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        scanned
            .coverage
            .iter()
            .map(|span| (span.first_hour - base, span.last_hour - base, span.coverage))
            .collect::<Vec<_>>(),
        [
            (0, 0, Coverage::Partial { ungrouped: 1 }),
            (1, 1, Coverage::Unknown),
            (2, 2, Coverage::Partial { ungrouped: 2 }),
            (3, 3, Coverage::Unknown),
            (4, 4, Coverage::Logged),
        ]
    );
}

#[test]
fn daily_values_change_at_the_slot_read_even_without_their_own_timing() {
    let base = 60_827_879;
    let mut first = frame(base, vec![], &[]);
    lend_lease(&mut first, vec![exchange(7, 0, 1326, 5)]);
    *first.tables[TableId::SunkConvoys.index()] =
        TableView::Complete(TableRows::SunkConvoys(vec![SunkConvoyRow {
            owner_country: GER,
            killer_country: SOV,
            month: i32::from(OCTOBER),
            convoys: 5,
            ordinal: 0,
        }]));
    let mut carried = first.clone();
    carried.hour += 1;
    carried.metrics = Metrics::default();
    lend_lease(&mut carried, vec![exchange(7, 0, 1326, 99)]);
    let mut read = first.clone();
    read.hour += 24;
    read.metrics = Metrics::default();
    lend_lease(&mut read, vec![exchange(7, 0, 1326, 7)]);
    *read.tables[TableId::SunkConvoys.index()] =
        TableView::Complete(TableRows::SunkConvoys(vec![SunkConvoyRow {
            owner_country: GER,
            killer_country: SOV,
            month: i32::from(OCTOBER),
            convoys: 7,
            ordinal: 0,
        }]));
    *read.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 1326 << 32 | 70,
            equipment_token: 5,
            display_name: b"Seen at slot".to_vec(),
            ..Default::default()
        }]));
    let scanned = scan(
        &replay_of(vec![first.clone(), carried.clone(), read.clone()]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/test_l_english.yml",
        "l_english:\n",
    )]);
    let built = reply(
        &params(),
        &scanned,
        Content {
            catalogue: Some(&catalogue),
            ..Content::default()
        },
        Vec::new(),
    );
    assert_eq!(
        totals(&built, LossSource::LendLeaseSent),
        [(base, raw(5)), (base + 24, raw(7))]
    );
    assert_eq!(
        totals(&built, LossSource::SunkConvoys),
        [(base, raw(5)), (base + 24, raw(7))]
    );
    assert_eq!(last_known_name(&scanned, 1326).as_deref(), Some("Kar98k"));
    let next_definition_read = (read.hour + 1..=read.hour + 24)
        .find(|&hour| replay_data::cadence::is_refresh(TableId::VariantDefinitions, hour))
        .unwrap();
    let mut definitions = read.clone();
    definitions.hour = next_definition_read;
    let scanned = scan(
        &replay_of(vec![first, carried, read, definitions]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        last_known_name(&scanned, 1326).as_deref(),
        Some("Seen at slot")
    );
}
use replay::spool::Record;
use replay_data::{
    identity::Identity, identity::Roster, identity::SampleBoundary, identity::SessionStart,
    identity::Verification, rows::CampaignRow, rows::CombatLogEquipmentRow,
    rows::CombatLogStateRow, rows::CountryLinkRow, rows::CountryRow, rows::LendLeaseEquipmentRow,
    rows::PlayerRow, rows::SliceReadRow, rows::SunkConvoyRow, rows::SunkShipEntry,
    rows::SunkShipsRow, rows::TableRows, rows::VariantDefinitionRow,
};
use replay_frame::{
    branches::BranchReason, branches::BranchStart, branches::Parentage, frame::FrameEncoder,
    frame::Metrics, frame::TableView,
};

const CAMPAIGN: &[u8] = b"losses-test";
/// GER's raw references (both resolve to slot 7) and another country's.
const GER: u32 = 9001;
const GER_ALIAS: u32 = 11;
const SOV: u32 = 9002;
const OCTOBER: u16 = 1943 * 12 + 9;
const NOVEMBER: u16 = 1943 * 12 + 10;

fn session() -> SessionStart {
    SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        identity: Identity {
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
    }
}

/// An hour with GER (slot 7) played by a human, observed prunes, and the
/// given Combat Log rows read now for the countries in `read`.
fn frame(hour: i32, log: Vec<CombatLogEquipmentRow>, read: &[u32]) -> DecodedFrame {
    let mut tables: [TableView; TableId::ALL.len()] =
        std::array::from_fn(|_| TableView::Unavailable {
            reason: "outside the losses fixture".into(),
        });
    let mut put = |table: TableId, rows: TableRows| {
        tables[table.index()] = TableView::Complete(rows);
    };
    put(
        TableId::Campaign,
        TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.into(),
        }]),
    );
    put(
        TableId::Countries,
        TableRows::Countries(vec![
            CountryRow {
                id: GER,
                canonical_slot: 7,
                tag: b"GER".to_vec(),
                ..CountryRow::default()
            },
            CountryRow {
                id: SOV,
                canonical_slot: 8,
                tag: b"SOV".to_vec(),
                ..CountryRow::default()
            },
        ]),
    );
    put(
        TableId::CountryLinks,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: GER_ALIAS,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: GER,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: SOV,
                canonical_slot: 8,
            },
        ]),
    );
    put(
        TableId::Divisions,
        TableRows::Divisions(vec![replay_data::rows::DivisionRow {
            id: 1 << 32 | 51,
            roster_owner: GER,
            logical_owner: GER,
            orders_group: 1 << 32 | 53,
            ..replay_data::rows::DivisionRow::default()
        }]),
    );
    put(
        TableId::Players,
        TableRows::Players(vec![PlayerRow {
            id: 1,
            country: GER,
            ..PlayerRow::default()
        }]),
    );
    put(
        TableId::CombatLogState,
        TableRows::CombatLogState(vec![CombatLogStateRow {
            viewing_country_raw: GER as i32,
            prune_observed: 1,
        }]),
    );
    put(
        TableId::SimulationFeatures,
        TableRows::SimulationFeatures(vec![replay_data::rows::SimulationFeatureRow {
            dlc_mask: engine_dlc::dlc_feature_state::EQUIPMENT_LOSSES_MASK,
            human_ai: 0,
            weather_suspend: replay_data::rows::Recorded::Value(None),
            instant_decisions: replay_data::rows::Recorded::Value(Some(0)),
            runtime_scalars: Default::default(),
            movement_defines: Default::default(),
        }]),
    );
    put(
        TableId::SliceReads,
        TableRows::SliceReads(
            read.iter()
                .map(|&owner| SliceReadRow {
                    table: TableId::CombatLogEquipment.index() as u32,
                    owner,
                    sample_hour: hour,
                })
                .collect(),
        ),
    );
    put(
        TableId::CombatLogEquipment,
        TableRows::CombatLogEquipment(log),
    );
    put(
        TableId::VariantDefinitions,
        TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 1326 << 32 | 70,
            equipment_token: 5,
            display_name: b"Kar98k".to_vec(),
            ..VariantDefinitionRow::default()
        }]),
    );
    put(
        TableId::LexicalTokens,
        TableRows::LexicalTokens(vec![replay_data::rows::LexicalTokenRow {
            token: 5,
            literal: b"mod_rifle_1".to_vec(),
        }]),
    );
    DecodedFrame {
        branch: 1,
        hour,
        tables: tables.map(Into::into),
        metrics: Metrics {
            table_ns: std::array::from_fn(|index| {
                u64::from(matches!(
                    TableId::ALL[index],
                    TableId::LendLeaseEquipment
                        | TableId::SunkConvoys
                        | TableId::VariantDefinitions
                ))
            }),
            ..Metrics::default()
        },
    }
}

fn populate_census(frame: &mut DecodedFrame) {
    if !matches!(&*frame.tables[TableId::DivisionCensus.index()], TableView::Unavailable { reason } if reason == "outside the losses fixture")
    {
        return;
    }
    let Ok(divisions) = frame.complete::<tables::Divisions>() else {
        *frame.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
            reason: "unread census".into(),
        };
        return;
    };
    let Ok(countries) = frame.complete::<tables::Countries>() else {
        *frame.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
            reason: "unread country census".into(),
        };
        return;
    };
    let mut rows: BTreeMap<u32, replay_data::rows::DivisionCensusRow> = countries
        .iter()
        .map(|row| {
            (
                row.id,
                replay_data::rows::DivisionCensusRow {
                    country: row.id,
                    ..Default::default()
                },
            )
        })
        .collect();
    for division in divisions {
        let country = if (division.expeditionary_owner as i32) > 0 {
            division.expeditionary_owner
        } else {
            division.roster_owner
        };
        let row = rows
            .entry(country)
            .or_insert_with(|| replay_data::rows::DivisionCensusRow {
                country,
                ..Default::default()
            });
        if division.orders_group == 0 {
            row.ungrouped += 1;
        } else {
            row.grouped += 1;
        }
    }
    *frame.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(rows.into_values().collect()));
}

fn replay_of(frames: Vec<DecodedFrame>) -> Replay {
    let mut records = vec![
        Record::SessionStart(session()),
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
    records.extend(frames.into_iter().map(|mut frame| {
        populate_census(&mut frame);
        Record::Frame(encoder.encode(frame, false).unwrap())
    }));
    Replay::from_records(CAMPAIGN.into(), &[records]).unwrap()
}

fn log(country: u32, group: u32, kind: u8, month: u16, amount: i64) -> CombatLogEquipmentRow {
    CombatLogEquipmentRow {
        country,
        group,
        kind,
        month,
        variant: 1326,
        date: 60_827_868,
        amount_raw: amount * AMOUNT_SCALE,
    }
}

fn params() -> LossesParams {
    LossesParams {
        branch_id: "1".into(),
        tag: "GER".into(),
        land_source: LandSource::CombatLog,
        country: None,
    }
}

fn totals(reply: &LossesReply, source: LossSource) -> Vec<(i32, String)> {
    let cause = reply
        .causes
        .iter()
        .find(|cause| cause.source == source)
        .expect("cause present");
    cause
        .leaves
        .iter()
        .flat_map(|leaf| leaf.points.iter().map(|p| (p.hour, p.total_raw.clone())))
        .collect()
}

fn raw(amount: i64) -> String {
    (amount * AMOUNT_SCALE).to_string()
}

fn last_known_name(scanned: &Scanned, variant: u32) -> Option<String> {
    let catalogue = dashboard_text::catalogue::tests::loaded("l_english:\n");
    scanned
        .variants
        .names(
            (u64::from(variant) << 32) | u64::from(VARIANT_TYPE),
            Some(&catalogue),
        )
        .into_iter()
        .rev()
        .find(|name| name.known)
        .and_then(|name| name.name)
}

#[test]
fn design_names_follow_reads_and_native_suffix_without_crossing_unavailable_hours() {
    let base = 60_827_880;
    let mut first = frame(base, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER]);
    let mut carried = frame(base + 1, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER]);
    carried.metrics = Metrics::default();
    let mut renamed = frame(base + 2, vec![log(GER, 1, 0, OCTOBER, 4)], &[GER]);
    let mut unread = frame(base + 3, vec![log(GER, 1, 0, OCTOBER, 4)], &[GER]);
    let mut recovered = frame(base + 4, vec![log(GER, 1, 0, OCTOBER, 5)], &[GER]);
    for frame in [&mut first, &mut carried, &mut renamed, &mut recovered] {
        *frame.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(Vec::new()));
        *frame.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(TableRows::VariantUpgrades(Vec::new()));
    }
    for (frame, name, suffix, append) in [
        (&mut carried, "Not read", "", false),
        (&mut renamed, "Custom design", "Mk II", true),
        (&mut recovered, "Final name", "", false),
    ] {
        let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        else {
            unreachable!();
        };
        rows[0].display_name = name.as_bytes().to_vec();
        rows[0].variant_name = suffix.as_bytes().to_vec();
        rows[0].append_variant_name = u8::from(append);
    }
    *unread.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
        reason: "failed naming read".into(),
    };
    let mut replay = replay_of(vec![first, carried, renamed, unread, recovered]);
    replay.gaps.push(replay_frame::branches::Gap {
        branch: 1,
        first_hour: base + 5,
        last_hour: base + 6,
        reason: "trailing gap".into(),
    });
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/test_l_english.yml",
        "l_english:\n",
    )]);
    let built = reply(
        &params(),
        &scanned,
        Content {
            catalogue: Some(&catalogue),
            ..Content::default()
        },
        Vec::new(),
    );
    let names = &built.designs[0].names;
    assert_eq!(
        names
            .iter()
            .map(|name| (name.first_hour, name.name.as_deref(), name.known))
            .collect::<Vec<_>>(),
        [
            (base, Some("Kar98k"), true),
            (base + 2, Some("Custom design Mk II"), true),
            (base + 3, None, false),
            (base + 4, Some("Final name"), true),
            (base + 5, None, false),
        ]
    );
    assert!(names[0].design.is_some());
    assert_ne!(names[0].design, names[1].design);
    assert_eq!(names[2].design, None);
    assert_eq!(
        totals(&built, LossSource::CombatLog(0)),
        [(base, raw(3)), (base + 2, raw(4)), (base + 4, raw(5))]
    );
}

#[test]
fn a_design_name_is_not_backfilled_before_its_first_read() {
    let base = 60_827_880;
    let mut first = frame(base, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER]);
    *first.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
        reason: "first naming read unavailable".into(),
    };
    let later = frame(base + 1, vec![log(GER, 1, 0, OCTOBER, 4)], &[GER]);
    let scanned = scan(
        &replay_of(vec![first, later]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/test_l_english.yml",
        "l_english:\n",
    )]);
    let built = reply(
        &params(),
        &scanned,
        Content {
            catalogue: Some(&catalogue),
            ..Content::default()
        },
        Vec::new(),
    );
    assert_eq!(built.designs[0].names[0].first_hour, base + 1);
    assert_eq!(built.designs[0].names[0].name.as_deref(), Some("Kar98k"));
    assert_eq!(
        totals(&built, LossSource::CombatLog(0)),
        [(base, raw(3)), (base + 1, raw(4))]
    );
}

#[test]
fn land_losses_keep_each_entrys_last_value_through_windows_and_prunes() {
    let base = 60_827_880;
    let replay = replay_of(vec![
        // Full read: October 3, November 1, recovered 1; SOV's rows ignored.
        frame(
            base,
            vec![
                log(GER, 1, 0, OCTOBER, 3),
                log(GER, 1, 0, NOVEMBER, 1),
                log(
                    GER,
                    1,
                    engine_land_inputs::ncombat_log_cmanager::RECOVERED_KIND,
                    NOVEMBER,
                    1,
                ),
                log(SOV, 5, 0, NOVEMBER, 40),
            ],
            &[GER, SOV],
        ),
        // Carried (not read now): no change, even though the value differs.
        frame(base + 1, vec![log(GER, 1, 0, NOVEMBER, 99)], &[]),
        // Window read under the alias: October has left the table (it
        // stopped changing), November grew to 4, a new group appeared.
        frame(
            base + 24,
            vec![
                log(GER_ALIAS, 1, 0, NOVEMBER, 4),
                log(GER_ALIAS, 2, 3, NOVEMBER, 2),
            ],
            &[GER_ALIAS],
        ),
        // Group 1's log was deleted; its rows are gone: nothing is undone.
        frame(base + 48, vec![log(GER, 2, 3, NOVEMBER, 2)], &[GER]),
    ]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/test_l_english.yml",
        "l_english:\n",
    )]);
    let reply = reply(
        &params(),
        &scanned,
        Content {
            catalogue: Some(&catalogue),
            ..Content::default()
        },
        Vec::new(),
    );
    assert_eq!(
        totals(&reply, LossSource::CombatLog(0)),
        [(base, raw(4)), (base + 24, raw(7))]
    );
    assert_eq!(
        totals(&reply, LossSource::CombatLog(3)),
        [(base + 24, raw(2))]
    );
    let recovered = reply
        .causes
        .iter()
        .find(|cause| cause.source == LossSource::Recovered)
        .unwrap();
    assert!(!recovered.loss);
    assert_eq!(totals(&reply, LossSource::Recovered), [(base, raw(1))]);
    // Names come from content keys. Without content, only the executable's
    // own keys are known; the interface's are not guessed.
    let combat = &reply.causes[0];
    assert_eq!(combat.name.key, None);
    assert!(combat.name.text.is_none());
    let attrition = reply
        .causes
        .iter()
        .find(|cause| cause.source == LossSource::CombatLog(2))
        .unwrap();
    assert_eq!(
        attrition.name.key.as_deref(),
        Some("COMBAT_LOG_EQ_LOST_ATTR_COMBAT")
    );
    // Game sentences and templates are not names: the app labels them.
    assert_eq!(attrition.name.text.as_deref(), Some("Attrition: in combat"));
    for (source, label) in [
        (LossSource::SunkShips, "Sunk ships"),
        (LossSource::SunkConvoys, "Sunk convoys"),
        (LossSource::LendLeaseSunk, "Lend-lease sunk in transit"),
    ] {
        assert_eq!(cause(&reply, source).name.text.as_deref(), Some(label));
    }
    assert_eq!(attrition.group.as_ref().unwrap().key, None);
    // Unavailable destruction keeps each air cause unknown, never zero.
    for source in EquipmentDestructionCause::ALL
        .into_iter()
        .filter(|cause| cause.is_air())
    {
        let air = cause(&reply, LossSource::Destruction(source));
        assert!(!air.unknown_spans.is_empty() && air.leaves.is_empty());
    }
    // The design is recorded, its type unknown without content.
    assert_eq!(reply.designs[0].names[0].name.as_deref(), Some("Kar98k"));
    assert_eq!(reply.designs[0].equipment.as_deref(), Some("mod_rifle_1"));
    assert!(reply.designs[0].archetype.is_none());
}

#[test]
fn hours_the_game_did_not_log_the_country_are_not_logged_not_zero() {
    let base = 60_827_880;
    let mut observer = frame(base + 1, vec![], &[]);
    *observer.tables[TableId::CombatLogState.index()] =
        TableView::Complete(TableRows::CombatLogState(vec![CombatLogStateRow {
            viewing_country_raw: 0,
            prune_observed: 1,
        }]));
    let mut other_player = frame(base + 2, vec![], &[]);
    *other_player.tables[TableId::Players.index()] =
        TableView::Complete(TableRows::Players(vec![PlayerRow {
            id: 1,
            country: SOV,
            ..PlayerRow::default()
        }]));
    let mut unobserved = frame(base + 3, vec![], &[]);
    *unobserved.tables[TableId::CombatLogState.index()] =
        TableView::Complete(TableRows::CombatLogState(vec![CombatLogStateRow {
            viewing_country_raw: GER as i32,
            prune_observed: 0,
        }]));
    let mut missing = frame(base + 4, vec![], &[]);
    *missing.tables[TableId::Players.index()] = TableView::Unavailable {
        reason: "missing".into(),
    };
    let replay = replay_of(vec![
        frame(base, vec![], &[]),
        observer,
        other_player,
        unobserved,
        missing,
        frame(base + 6, vec![], &[]),
    ]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let spans: Vec<_> = scanned
        .coverage
        .iter()
        .map(|span| (span.first_hour, span.last_hour, span.coverage))
        .collect();
    assert_eq!(
        spans,
        [
            (base, base + 1, Coverage::Logged),
            (base + 2, base + 2, Coverage::NotLogged),
            (base + 3, base + 3, Coverage::PruneUnobserved),
            // The unread frame and missing hour merge as unknown.
            (base + 4, base + 5, Coverage::Unknown),
            (base + 6, base + 6, Coverage::Logged),
        ]
    );
    let reply = reply(&params(), &scanned, Content::default(), Vec::new());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("unknown, not zero"))
    );
}

#[test]
fn ships_convoys_and_lend_lease_join_as_their_own_causes() {
    let base = 60_827_880;
    let ship = |name: &[u8], sunk: i32, victim: u32| SunkShipEntry {
        ship_name: name.to_vec(),
        victim_country: victim,
        sink_hour: sunk,
        victim_equipment_variant: 6019 << 32 | 70,
        ..SunkShipEntry::default()
    };
    let with = |hour: i32, ships: Vec<SunkShipEntry>, convoys: i32, sunk: i64, sent: i64| {
        let mut frame = frame(hour, vec![], &[]);
        *frame.tables[TableId::SunkShips.index()] =
            TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow { entries: ships }]));
        *frame.tables[TableId::SunkConvoys.index()] =
            TableView::Complete(TableRows::SunkConvoys(vec![
                SunkConvoyRow {
                    month: 23_325,
                    killer_country: GER,
                    owner_country: SOV,
                    convoys: 50,
                    ordinal: 0,
                },
                SunkConvoyRow {
                    month: 23_325,
                    killer_country: SOV,
                    owner_country: GER,
                    convoys,
                    ordinal: 1,
                },
            ]));
        *frame.tables[TableId::LendLeaseEquipment.index()] =
            TableView::Complete(TableRows::LendLeaseEquipment(vec![
                LendLeaseEquipmentRow {
                    giver: GER,
                    exchange: 97730,
                    kind: 0,
                    variant: 1326,
                    receiver: SOV,
                    amount_raw: sent,
                },
                LendLeaseEquipmentRow {
                    giver: GER,
                    exchange: 97730,
                    kind: 1,
                    variant: 1326,
                    receiver: SOV,
                    amount_raw: sunk,
                },
            ]));
        frame
    };
    let replay = replay_of(vec![
        with(
            base,
            vec![ship(b"Emden", base - 10, GER)],
            2,
            100_000,
            500_000,
        ),
        // Two identical anonymous entries at once count twice; a victim of
        // another country does not count.
        with(
            base + 24,
            vec![
                ship(b"Emden", base - 10, GER),
                ship(b"", base + 5, GER),
                ship(b"", base + 5, GER),
                ship(b"Kirov", base + 6, SOV),
            ],
            5,
            300_000,
            900_000,
        ),
    ]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(&params(), &scanned, Content::default(), Vec::new());
    assert_eq!(
        totals(&reply, LossSource::SunkShips),
        [(base - 10, raw(1)), (base + 5, raw(3))]
    );
    assert_eq!(
        totals(&reply, LossSource::SunkConvoys),
        [(base, raw(2)), (base + 24, raw(5))]
    );
    assert_eq!(
        totals(&reply, LossSource::LendLeaseSunk),
        [
            (base, "100000".to_owned()),
            (base + 24, "300000".to_owned())
        ]
    );
    let sent = reply
        .causes
        .iter()
        .find(|cause| cause.source == LossSource::LendLeaseSent)
        .unwrap();
    assert!(!sent.loss, "a transfer, not a loss");
    assert_eq!(sent.name.key.as_deref(), Some("LOGISTICS_NEED_LEND_LEASE"));
    // The exchange stayed: nothing is a lower bound.
    assert!(
        sent.leaves
            .iter()
            .all(|leaf| leaf.lower_bound_from.is_none())
    );
    assert_eq!(
        spans(&sent.unknown_spans),
        [(base + 1, base + 23, UnknownReason::Unread)]
    );
}

/// A modded content stack: its own equipment type and archetype, which no
/// vanilla list knows, name the type level.
#[test]
fn a_modded_equipment_type_resolves_through_the_content_stack() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_losses_modded_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let equipment = root.join("common/units/equipment");
    std::fs::create_dir_all(&equipment).unwrap();
    std::fs::write(
        equipment.join("mod_rifles.txt"),
        "equipments = { mod_rifle = { is_archetype = yes } mod_rifle_1 = { archetype = mod_rifle } }",
    )
    .unwrap();
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let archetypes = installed_content_core::equipment::Database::load_complete(&layers).unwrap();
    let base = 60_827_880;
    let replay = replay_of(vec![frame(base, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER])]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(
        &params(),
        &scanned,
        Content {
            archetypes: Some(&archetypes),
            retained_months: Some(12),
            ..Content::default()
        },
        Vec::new(),
    );
    assert_eq!(reply.designs[0].archetype.as_deref(), Some("mod_rifle"));
    assert_eq!(reply.retained_months, Some(12));
    // A variant the content does not declare keeps its type unknown.
    let empty = installed_content_core::equipment::Database::default();
    let reply = super::reply(
        &params(),
        &scanned,
        Content {
            archetypes: Some(&empty),
            ..Content::default()
        },
        Vec::new(),
    );
    assert!(reply.designs[0].archetype.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

fn cause(reply: &LossesReply, source: LossSource) -> &LossCause {
    reply
        .causes
        .iter()
        .find(|cause| cause.source == source)
        .expect("cause present")
}

fn spans(spans: &[UnknownSpan]) -> Vec<(i32, i32, UnknownReason)> {
    spans
        .iter()
        .map(|span| (span.first_hour, span.last_hour, span.reason))
        .collect()
}

/// Codex #160 round 1: a record the branch did not read is never an empty
/// table or a flat total.
#[test]
fn each_record_lists_the_hours_it_was_not_read() {
    let base = 60_827_880;
    let mut failed = frame(base + 1, vec![], &[]);
    *failed.tables[TableId::CombatLogEquipment.index()] = TableView::Unavailable {
        reason: "failed".into(),
    };
    let mut convoys = frame(base + 2, vec![], &[]);
    *convoys.tables[TableId::SunkConvoys.index()] =
        TableView::Complete(TableRows::SunkConvoys(Vec::new()));
    let replay = replay_of(vec![
        frame(base, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER]),
        failed,
        convoys,
    ]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    // The land hour whose log failed is unknown, not logged.
    assert_eq!(
        scanned
            .coverage
            .iter()
            .map(|span| (span.first_hour, span.last_hour, span.coverage))
            .collect::<Vec<_>>(),
        [
            (base, base, Coverage::Logged),
            (base + 1, base + 1, Coverage::Unknown),
            (base + 2, base + 2, Coverage::Logged),
        ]
    );
    let reply = reply(&params(), &scanned, Content::default(), Vec::new());
    let unread = |from, to| vec![(from, to, UnknownReason::Unread)];
    for source in [LossSource::CombatLog(0), LossSource::Recovered] {
        assert_eq!(
            spans(&cause(&reply, source).unknown_spans),
            unread(base + 1, base + 1)
        );
    }
    // This fixture records no ships or lend-lease at all: unknown throughout.
    for source in [
        LossSource::SunkShips,
        LossSource::LendLeaseSunk,
        LossSource::LendLeaseSent,
    ] {
        assert_eq!(
            spans(&cause(&reply, source).unknown_spans),
            unread(base, base + 2)
        );
    }
    assert_eq!(
        spans(&cause(&reply, LossSource::SunkConvoys).unknown_spans),
        unread(base, base + 1)
    );
}

fn lend_lease(frame: &mut DecodedFrame, rows: Vec<LendLeaseEquipmentRow>) {
    *frame.tables[TableId::LendLeaseEquipment.index()] =
        TableView::Complete(TableRows::LendLeaseEquipment(rows));
}

fn exchange(exchange: u32, kind: u8, variant: u32, amount: i64) -> LendLeaseEquipmentRow {
    LendLeaseEquipmentRow {
        giver: GER,
        exchange,
        kind,
        variant,
        receiver: SOV,
        amount_raw: amount * AMOUNT_SCALE,
    }
}

/// Codex #160 round 1: an exchange that ends after its last read loses what
/// it moved since, so its totals are lower bounds from then on.
#[test]
fn an_exchange_that_ends_after_its_last_read_makes_its_totals_lower_bounds() {
    let base = 60_827_880;
    let mut first = frame(base, vec![], &[]);
    lend_lease(
        &mut first,
        vec![exchange(7, 0, 1326, 100), exchange(8, 0, 1846, 5)],
    );
    // Carried hours show the same read, but have no daily walk timing.
    let mut carried = frame(base + 1, vec![], &[]);
    lend_lease(
        &mut carried,
        vec![exchange(7, 0, 1326, 100), exchange(8, 0, 1846, 5)],
    );
    carried.metrics = Metrics::default();
    // The next read: exchange 7 has ended and been deleted.
    let mut next = frame(base + 24, vec![], &[]);
    lend_lease(&mut next, vec![exchange(8, 0, 1846, 9)]);
    let later = frame(base + 30, vec![], &[]);
    let replay = replay_of(vec![first, carried, next, later]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(&params(), &scanned, Content::default(), Vec::new());
    let sent = cause(&reply, LossSource::LendLeaseSent);
    assert_eq!(
        spans(&sent.unknown_spans),
        [
            (base + 1, base + 30, UnknownReason::EndedAfterLastRead),
            (base + 2, base + 23, UnknownReason::Unread),
            (base + 25, base + 30, UnknownReason::Unread),
        ]
    );
    let bound = |variant: u32| {
        sent.leaves
            .iter()
            .find(|leaf| reply.designs[leaf.design.unwrap() as usize].variant == variant)
            .unwrap()
            .lower_bound_from
    };
    assert_eq!(bound(1326), Some(base + 1));
    assert_eq!(bound(1846), None);
    // The total is not undone by the deleted exchange.
    assert_eq!(
        totals(&reply, LossSource::LendLeaseSent),
        [(base, raw(100)), (base, raw(5)), (base + 24, raw(9)),]
    );
}

#[test]
fn a_retention_below_one_is_refused_by_name_and_an_unknown_one_warned() {
    let base = 60_827_880;
    let replay = replay_of(vec![frame(base, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER])]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let with = |months| {
        reply(
            &params(),
            &scanned,
            Content {
                retained_months: months,
                ..Content::default()
            },
            Vec::new(),
        )
    };
    let refused = with(Some(0));
    for source in [LossSource::CombatLog(0), LossSource::Recovered] {
        let land = cause(&refused, source);
        assert!(land.leaves.is_empty());
        assert!(
            land.unknown
                .as_deref()
                .is_some_and(|reason| reason.contains("NGame.COMBAT_LOG_MAX_MONTHS is 0"))
        );
    }
    assert!(cause(&refused, LossSource::SunkShips).unknown.is_none());
    let kept = with(Some(1));
    assert_eq!(totals(&kept, LossSource::CombatLog(0)), [(base, raw(3))]);
    assert!(kept.warnings.is_empty());
    assert!(
        with(None)
            .warnings
            .iter()
            .any(|warning| warning.contains("COMBAT_LOG_MAX_MONTHS) is unknown"))
    );
}

#[test]
fn a_combat_log_value_that_falls_is_a_signed_correction() {
    let base = 60_827_880;
    let replay = replay_of(vec![
        frame(base, vec![log(GER, 1, 0, OCTOBER, 5)], &[GER]),
        frame(base + 24, vec![log(GER, 1, 0, OCTOBER, 2)], &[GER]),
    ]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(
        &params(),
        &scanned,
        Content {
            retained_months: Some(12),
            ..Content::default()
        },
        Vec::new(),
    );
    assert_eq!(
        totals(&reply, LossSource::CombatLog(0)),
        [(base, raw(5)), (base + 24, raw(2))]
    );
    assert_eq!(
        reply.warnings,
        [
            "Some hours have recorded gaps, failed blocks/frames, unread division census, unresolved owners or missing writer inputs; land-loss coverage is unknown there."
        ]
    );
}

/// Codex #160 round 1: the column titles come from the content's interface,
/// so a mod that renames them names the causes.
#[test]
fn interface_authored_cause_names_come_from_the_contents_gui() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!(
        "hoi4_losses_gui_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let interface = root.join("interface");
    std::fs::create_dir_all(&interface).unwrap();
    std::fs::write(
        interface.join("combatlogview.gui"),
        r#"guiTypes = {
    containerWindowType = {
        name = "combat_log_equipment_tab"
        instantTextBoxType = { name = "title_lost_in_combat" text = "MOD_LOST_IN_BATTLE" }
        instantTextBoxType = { name = "title_recovered" text = "MOD_SALVAGED" }
        instantTextBoxType = { name = "title_lost_by_attrition" text = "MOD_WORN_OUT" }
    }
}"#,
    )
    .unwrap();
    std::fs::write(
        interface.join("naval_losses.gui"),
        r#"guiTypes = {
    containerWindowType = {
        name = "navallosseswindow"
        instantTextboxType = { name = "total_ships" pdx_tooltip = "MOD_SHIPS_LOST" }
    }
}"#,
    )
    .unwrap();
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let mut warnings = Vec::new();
    let guis = installed_content_core::gui::native::read_layers(&layers).unwrap();
    let keys = InterfaceKeys::read(&guis, &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        keys,
        InterfaceKeys {
            combat: Some("MOD_LOST_IN_BATTLE".into()),
            recovered: Some("MOD_SALVAGED".into()),
            attrition: Some("MOD_WORN_OUT".into()),
            ships_lost: Some("MOD_SHIPS_LOST".into()),
        }
    );
    let replay = replay_of(vec![frame(60_827_880, vec![], &[])]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(
        &params(),
        &scanned,
        Content {
            keys: Some(&keys),
            ..Content::default()
        },
        Vec::new(),
    );
    let key = |source| cause(&reply, source).name.key.clone();
    assert_eq!(
        key(LossSource::CombatLog(0)).as_deref(),
        Some("MOD_LOST_IN_BATTLE")
    );
    assert_eq!(key(LossSource::Recovered).as_deref(), Some("MOD_SALVAGED"));
    assert_eq!(
        key(LossSource::SunkShips).as_deref(),
        Some("MOD_SHIPS_LOST")
    );
    assert_eq!(
        cause(&reply, LossSource::CombatLog(3))
            .group
            .as_ref()
            .unwrap()
            .key
            .as_deref(),
        Some("MOD_WORN_OUT")
    );
    let destruction = super::reply(
        &LossesParams {
            land_source: LandSource::Destruction,
            ..params()
        },
        &scanned,
        Content {
            keys: Some(&keys),
            ..Content::default()
        },
        vec![],
    );
    assert_eq!(
        cause(
            &destruction,
            LossSource::Destruction(EquipmentDestructionCause::LandCombat)
        )
        .name
        .key
        .as_deref(),
        Some("MOD_LOST_IN_BATTLE")
    );
    assert_eq!(
        cause(
            &destruction,
            LossSource::Destruction(EquipmentDestructionCause::Attrition)
        )
        .name
        .key
        .as_deref(),
        Some("MOD_WORN_OUT")
    );
    assert_eq!(
        cause(
            &destruction,
            LossSource::Destruction(EquipmentDestructionCause::Garrison)
        )
        .name
        .text
        .as_deref(),
        Some("Resistance attacks on garrisons")
    );
    assert_eq!(
        cause(
            &destruction,
            LossSource::Destruction(EquipmentDestructionCause::AirAccident)
        )
        .name
        .text
        .as_deref(),
        Some("Air accidents")
    );
    // A content stack without the window leaves the names null, warned.
    let bare = installed_content_core::ResolvedLayers::from_folders(&[(&interface, &[][..])]).unwrap();
    let mut warnings = Vec::new();
    assert_eq!(
        InterfaceKeys::read(
            &installed_content_core::gui::native::read_layers(&bare).unwrap(),
            &mut warnings,
        ),
        InterfaceKeys::default()
    );
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_cancelled_scan_stops() {
    let replay = replay_of(vec![frame(60_827_880, vec![], &[])]);
    assert!(scan(&replay, 1, "GER", &AtomicBool::new(true)).is_err());
}

#[test]
fn daily_records_only_change_at_the_read_that_saw_them() {
    let base = 60_827_880;
    let mut first = frame(base, vec![], &[]);
    lend_lease(&mut first, vec![exchange(7, 0, 1326, 5)]);
    *first.tables[TableId::SunkConvoys.index()] =
        TableView::Complete(TableRows::SunkConvoys(vec![SunkConvoyRow {
            owner_country: GER,
            killer_country: SOV,
            month: 23_325,
            convoys: 5,
            ordinal: 0,
        }]));
    let mut carried = first.clone();
    carried.hour += 1;
    carried.metrics = Metrics::default();
    // A changed carried value is not a newly witnessed value.
    lend_lease(&mut carried, vec![exchange(7, 0, 1326, 99)]);
    *carried.tables[TableId::SunkConvoys.index()] =
        TableView::Complete(TableRows::SunkConvoys(vec![SunkConvoyRow {
            owner_country: GER,
            killer_country: SOV,
            month: 23_325,
            convoys: 99,
            ordinal: 0,
        }]));
    *carried.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 1326 << 32 | 70,
            equipment_token: 5,
            display_name: b"Unwitnessed".to_vec(),
            ..VariantDefinitionRow::default()
        }]));
    let replay = replay_of(vec![first, carried]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(&params(), &scanned, Content::default(), vec![]);
    assert_eq!(totals(&reply, LossSource::SunkConvoys), [(base, raw(5))]);
    assert_eq!(totals(&reply, LossSource::LendLeaseSent), [(base, raw(5))]);
    assert_eq!(last_known_name(&scanned, 1326).as_deref(), Some("Kar98k"));
}

#[test]
fn human_membership_does_not_prove_ungrouped_divisions_are_logged() {
    let mut frame = frame(60_827_880, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER]);
    *frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![replay_data::rows::DivisionRow {
            id: 1 << 32 | 51,
            roster_owner: GER,
            logical_owner: GER,
            orders_group: 0,
            ..replay_data::rows::DivisionRow::default()
        }]));
    let scanned = scan(&replay_of(vec![frame]), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.coverage[0].coverage, Coverage::NotLogged);
    // Coverage explains the game's retention; it never excludes captured rows.
    assert_eq!(
        scanned.events[&(LossSource::CombatLog(0), Some(1326))],
        [(60_827_880, 3 * i128::from(AMOUNT_SCALE))]
    );
}

#[test]
fn unread_writer_feature_cannot_prove_logged_coverage() {
    let mut frame = frame(60_827_880, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER]);
    *frame.tables[TableId::SimulationFeatures.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    let scanned = scan(&replay_of(vec![frame]), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.coverage[0].coverage, Coverage::Unknown);
    let reply = reply(&params(), &scanned, Content::default(), vec![]);
    assert_eq!(
        totals(&reply, LossSource::CombatLog(0)),
        [(60_827_880, raw(3))]
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("writer inputs"))
    );
}

#[test]
fn a_disabled_writer_feature_is_not_logged_without_excluding_recorded_totals() {
    let mut frame = frame(60_827_880, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER]);
    *frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            replay_data::rows::SimulationFeatureRow {
                // Other runtime features do not enable this writer.
                dlc_mask: !engine_dlc::dlc_feature_state::EQUIPMENT_LOSSES_MASK,
                human_ai: 0,
                weather_suspend: replay_data::rows::Recorded::Value(None),
                instant_decisions: replay_data::rows::Recorded::Value(Some(0)),
                runtime_scalars: Default::default(),
                movement_defines: Default::default(),
            },
        ]));
    let scanned = scan(&replay_of(vec![frame]), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.coverage[0].coverage, Coverage::NotLogged);
    let reply = reply(&params(), &scanned, Content::default(), vec![]);
    assert_eq!(
        totals(&reply, LossSource::CombatLog(0)),
        [(60_827_880, raw(3))]
    );
}

#[test]
fn mixed_division_membership_is_partial_and_unread_census_is_unknown() {
    let mut mixed = frame(60_827_880, vec![], &[]);
    *mixed.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        replay_data::rows::DivisionRow {
            id: 1 << 32 | 51,
            roster_owner: GER,
            orders_group: 1 << 32 | 53,
            ..Default::default()
        },
        replay_data::rows::DivisionRow {
            id: 2 << 32 | 51,
            roster_owner: GER_ALIAS,
            ..Default::default()
        },
    ]));
    let mut missing = mixed.clone();
    missing.hour += 1;
    *missing.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    let scanned = scan(
        &replay_of(vec![mixed, missing]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.coverage.len(), 2);
    assert_eq!(
        scanned.coverage[0].coverage,
        Coverage::Partial { ungrouped: 1 }
    );
    assert_eq!(scanned.coverage[0].last_hour, 60_827_880);
    assert_eq!(scanned.coverage[1].coverage, Coverage::Unknown);
    assert_eq!(scanned.coverage[1].last_hour, 60_827_881);
}

#[test]
fn coverage_uses_the_writers_expeditionary_or_roster_country() {
    let mut frame = frame(60_827_880, vec![], &[]);
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        // Logical ownership is not the country the Combat Log writer chooses.
        replay_data::rows::DivisionRow {
            id: 1 << 32 | 51,
            roster_owner: GER,
            logical_owner: SOV,
            ..Default::default()
        },
        replay_data::rows::DivisionRow {
            id: 2 << 32 | 51,
            roster_owner: GER,
            expeditionary_owner: SOV,
            orders_group: 1 << 32 | 53,
            ..Default::default()
        },
    ]));
    let scanned = scan(&replay_of(vec![frame]), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.coverage[0].coverage, Coverage::NotLogged);
}

#[test]
fn division_census_tracks_transfers_remapping_deletion_and_unread_recovery() {
    let base = 60_827_880;
    let mut first = frame(base, vec![], &[]);
    let mut rows = vec![
        replay_data::rows::DivisionRow {
            id: 1 << 32 | 51,
            roster_owner: GER,
            orders_group: 1 << 32 | 53,
            ..Default::default()
        },
        replay_data::rows::DivisionRow {
            id: 2 << 32 | 51,
            roster_owner: SOV,
            ..Default::default()
        },
    ];
    *first.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(rows.clone()));
    let mut frames = vec![first.clone()];
    // Numeric deltas of unrelated fields must retain their exact baselines.
    rows[0].hit_points = i64::MAX;
    rows[1].province = 17;
    let mut moved = first.clone();
    moved.hour += 1;
    *moved.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(rows.clone()));
    frames.push(moved.clone());
    rows[0].hit_points = i64::MIN;
    rows[1].expeditionary_owner = GER_ALIAS;
    let mut transferred = moved.clone();
    transferred.hour += 1;
    *transferred.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(rows.clone()));
    frames.push(transferred.clone());
    let mut remapped = transferred.clone();
    remapped.hour += 1;
    *remapped.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: GER_ALIAS,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: GER,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: SOV,
                canonical_slot: 8,
            },
        ]));
    frames.push(remapped.clone());
    rows.pop();
    let mut deleted = remapped.clone();
    deleted.hour += 1;
    *deleted.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(rows));
    frames.push(deleted.clone());
    let mut unread = deleted.clone();
    unread.hour += 1;
    *unread.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    frames.push(unread);
    deleted.hour = base + 6;
    frames.push(deleted);
    let scanned = scan(&replay_of(frames), 1, "GER", &AtomicBool::new(false)).unwrap();
    let states: Vec<_> = scanned
        .coverage
        .iter()
        .map(|span| (span.first_hour - base, span.last_hour - base, span.coverage))
        .collect();
    assert_eq!(
        states,
        [
            (0, 1, Coverage::Logged),
            (2, 2, Coverage::Partial { ungrouped: 1 }),
            (3, 4, Coverage::Logged),
            (5, 5, Coverage::Unknown),
            (6, 6, Coverage::Logged),
        ]
    );
}

#[test]
fn unresolved_division_owner_keeps_the_census_unknown() {
    let mut frame = frame(60_827_880, vec![], &[]);
    *frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![replay_data::rows::DivisionRow {
            id: 1 << 32 | 51,
            roster_owner: 123,
            ..Default::default()
        }]));
    let scanned = scan(&replay_of(vec![frame]), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.coverage[0].coverage, Coverage::Unknown);
}

#[test]
fn closed_feature_gate_stays_not_logged_with_an_unread_census() {
    let mut frame = frame(60_827_880, vec![], &[]);
    *frame.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    *frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            replay_data::rows::SimulationFeatureRow {
                dlc_mask: 0,
                human_ai: 0,
                weather_suspend: replay_data::rows::Recorded::Value(None),
                instant_decisions: replay_data::rows::Recorded::Value(Some(0)),
                runtime_scalars: Default::default(),
                movement_defines: Default::default(),
            },
        ]));
    let scanned = scan(&replay_of(vec![frame]), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.coverage[0].coverage, Coverage::NotLogged);
}

#[test]
fn unread_country_references_are_warned_from_their_slice_reads() {
    let frame = frame(60_827_880, vec![log(1, 1, 0, OCTOBER, 3)], &[1]);
    let scanned = scan(&replay_of(vec![frame]), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.unresolved, BTreeSet::from([1]));
    assert!(scanned.events.is_empty());
}

#[test]
fn a_read_of_an_unresolved_owner_with_no_loss_rows_does_not_warn() {
    let frame = frame(60_827_880, vec![], &[1]);
    let scanned = scan(&replay_of(vec![frame]), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert!(scanned.unresolved.is_empty());
}

fn assert_missing(scanned: &Scanned, first: i32, last: i32) {
    for record in [
        LossRecord::CombatLog,
        LossRecord::SunkShips,
        LossRecord::SunkConvoys,
        LossRecord::LendLease,
    ] {
        assert!(
            scanned.unread.get(&record).is_some_and(|spans| spans
                .iter()
                .any(|span| span.first_hour <= first && span.last_hour >= last)),
            "{record:?}: {:?}",
            scanned.unread
        );
    }
    assert!(
        scanned
            .coverage
            .iter()
            .any(|span| span.coverage == Coverage::Unknown
                && span.first_hour <= first
                && span.last_hour >= last),
        "{:?}",
        scanned.coverage
    );
    assert!(
        scanned
            .coverage
            .windows(2)
            .all(|pair| pair[0].last_hour < pair[1].first_hour)
    );
}

#[test]
fn recorded_gaps_including_a_trailing_gap_are_unread() {
    let base = 60_827_880;
    let mut replay = replay_of(vec![frame(base, vec![], &[]), frame(base + 4, vec![], &[])]);
    for (first_hour, last_hour) in [(base + 1, base + 3), (base + 5, base + 8)] {
        replay.gaps.push(replay_frame::branches::Gap {
            branch: 1,
            first_hour,
            last_hour,
            reason: "dropped".into(),
        });
    }
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_missing(&scanned, base + 1, base + 3);
    assert_missing(&scanned, base + 5, base + 8);
    assert_eq!(scanned.last_hour, Some(base + 8));
}

#[test]
fn a_frame_inside_a_recorded_gap_does_not_supply_losses_or_coverage() {
    let base = 60_827_880;
    let mut hidden = frame(base + 1, vec![log(GER, 1, 0, OCTOBER, 99)], &[GER]);
    let mut later = frame(base + 2, vec![log(GER, 1, 0, OCTOBER, 4)], &[GER]);
    for frame in [&mut hidden, &mut later] {
        let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *frame.tables[TableId::Divisions.index()]
        else {
            unreachable!()
        };
        rows[0].orders_group = 0;
    }
    let mut replay = replay_of(vec![
        frame(base, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER]),
        hidden,
        later,
    ]);
    replay.gaps.push(replay_frame::branches::Gap {
        branch: 1,
        first_hour: base + 1,
        last_hour: base + 1,
        reason: "recorded missing hour".into(),
    });
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_missing(&scanned, base + 1, base + 1);
    assert_eq!(scanned.coverage.last().unwrap().first_hour, base + 2);
    assert_eq!(
        scanned.coverage.last().unwrap().coverage,
        Coverage::NotLogged
    );
    let reply = reply(&params(), &scanned, Content::default(), vec![]);
    assert_eq!(
        totals(&reply, LossSource::CombatLog(0)),
        [(base, raw(3)), (base + 2, raw(4))]
    );
}

#[test]
fn a_failed_block_marks_its_whole_range_unread() {
    let base = 60_827_880;
    let mut replay = replay_of(vec![
        frame(base, vec![], &[]),
        frame(base + 1, vec![], &[]),
        frame(base + 2, vec![], &[]),
    ]);
    replay.index[0].length = 1;
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_missing(&scanned, base, base + 2);
}

#[test]
fn a_failed_frame_marks_the_rest_of_its_block_unread() {
    let base = 60_827_880;
    let mut first = frame(base, vec![], &[]);
    *first.tables[TableId::Campaign.index()] = TableView::Unavailable {
        reason: "missing".into(),
    };
    let mut replay = replay_of(vec![
        first,
        frame(base + 1, vec![], &[]),
        frame(base + 2, vec![], &[]),
    ]);
    // The first frame has no campaign witness; the next violates the header.
    replay.campaign_id = b"another-campaign".to_vec();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_missing(&scanned, base + 1, base + 2);
    assert_eq!(scanned.coverage[0].first_hour, base);
    assert_eq!(scanned.coverage[0].last_hour, base);
    assert_eq!(scanned.coverage[1].last_hour, base + 2);
}

#[test]
fn a_convoy_count_that_falls_counts_as_a_new_entry_and_warns() {
    let base = 60_827_880;
    let frames = [(base, 5), (base + 1, 2)]
        .into_iter()
        .map(|(hour, convoys)| {
            let mut frame = frame(hour, vec![], &[]);
            *frame.tables[TableId::SunkConvoys.index()] =
                TableView::Complete(TableRows::SunkConvoys(vec![SunkConvoyRow {
                    owner_country: GER,
                    killer_country: SOV,
                    month: 23_325,
                    convoys,
                    ordinal: 0,
                }]));
            frame
        })
        .collect();
    let scanned = scan(&replay_of(frames), 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(&params(), &scanned, Content::default(), vec![]);
    assert_eq!(
        totals(&reply, LossSource::SunkConvoys),
        [(base, raw(5)), (base + 1, raw(7))]
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("SunkConvoys") && warning.contains("new entry"))
    );
}

#[test]
fn alias_owners_keep_independent_convoy_totals_on_repeated_reads() {
    let base = 60_827_880;
    let frames = (0..4)
        .map(|offset| {
            let mut frame = frame(base + offset, vec![], &[]);
            *frame.tables[TableId::SunkConvoys.index()] =
                TableView::Complete(TableRows::SunkConvoys(vec![
                    SunkConvoyRow {
                        owner_country: GER_ALIAS,
                        killer_country: SOV,
                        month: 23_325,
                        convoys: 5,
                        ordinal: 0,
                    },
                    SunkConvoyRow {
                        owner_country: GER,
                        killer_country: SOV,
                        month: 23_325,
                        convoys: 2,
                        ordinal: 0,
                    },
                ]));
            frame
        })
        .collect();
    let scanned = scan(&replay_of(frames), 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(&params(), &scanned, Content::default(), vec![]);
    assert_eq!(totals(&reply, LossSource::SunkConvoys), [(base, raw(7))]);
    assert_eq!(scanned.convoy_drops, 0);
    assert!(
        !reply
            .warnings
            .iter()
            .any(|warning| warning.contains("fell"))
    );
}

#[test]
fn mixed_census_counts_ungrouped_divisions_and_splits_when_the_count_changes() {
    let base = 60_827_880;
    let mut first = frame(base, vec![log(GER, 1, 0, OCTOBER, 3)], &[GER]);
    let mut divisions = vec![
        replay_data::rows::DivisionRow {
            id: 1 << 32 | 51,
            roster_owner: GER,
            orders_group: 1 << 32 | 53,
            ..Default::default()
        },
        replay_data::rows::DivisionRow {
            id: 2 << 32 | 51,
            roster_owner: GER_ALIAS,
            ..Default::default()
        },
        replay_data::rows::DivisionRow {
            id: 3 << 32 | 51,
            roster_owner: GER,
            ..Default::default()
        },
    ];
    *first.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(divisions.clone()));
    let mut same = first.clone();
    same.hour += 1;
    let mut changed = same.clone();
    changed.hour += 1;
    divisions[2].orders_group = 1 << 32 | 53;
    *changed.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(divisions));
    let scanned = scan(
        &replay_of(vec![first, same, changed]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let spans = serde_json::to_value(&scanned.coverage).unwrap();
    assert_eq!(
        spans,
        serde_json::json!([
            { "firstHour": base, "lastHour": base + 1, "coverage": { "partial": { "ungrouped": 2 } } },
            { "firstHour": base + 2, "lastHour": base + 2, "coverage": { "partial": { "ungrouped": 1 } } },
        ])
    );
    let reply = reply(&params(), &scanned, Content::default(), vec![]);
    assert_eq!(totals(&reply, LossSource::CombatLog(0)), [(base, raw(3))]);
}

/// The localiser fills an unknown `$NAME$` with nothing, so a template with a
/// fill-in leaves an incomplete label with empty parentheses.
#[test]
fn templates_with_fill_ins_have_no_complete_text() {
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/english/losses_l_english.yml",
        "\u{feff}l_english:\n FILLED:0 \"Lost Convoys: $NUM|H$ (until today $UNTIL_TODAY|H$)\"\n NAMED:0 \"In Combat\"\n",
    )]);
    let filled = name(Some(&catalogue), Some("FILLED"));
    assert_eq!(
        filled.template.as_deref(),
        Some("Lost Convoys: $NUM|H$ (until today $UNTIL_TODAY|H$)")
    );
    assert_eq!(filled.text, None);
    assert_eq!(
        name(Some(&catalogue), Some("NAMED")).text.as_deref(),
        Some("In Combat")
    );
}

#[test]
fn combat_log_signed_extremes_keep_the_full_i128_difference() {
    let base = 60_827_880;
    let mut one = log(GER, 1, 0, OCTOBER, 1);
    one.amount_raw = i64::MAX;
    let mut two = one.clone();
    two.amount_raw = i64::MIN;
    let recording = replay_of(vec![
        frame(base, vec![one], &[GER]),
        frame(base + 24, vec![two], &[GER]),
    ]);
    let scanned = scan(&recording, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(
        &params(),
        &scanned,
        Content {
            retained_months: Some(12),
            ..Content::default()
        },
        vec![],
    );
    assert_eq!(
        totals(&reply, LossSource::CombatLog(0)),
        [
            (base, i64::MAX.to_string()),
            (base + 24, i64::MIN.to_string())
        ]
    );
    assert_eq!(
        reply.warnings,
        [
            "Some hours have recorded gaps, failed blocks/frames, unread division census, unresolved owners or missing writer inputs; land-loss coverage is unknown there."
        ]
    );
}
