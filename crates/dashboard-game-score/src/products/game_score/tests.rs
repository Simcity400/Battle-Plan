use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, scale_frame, source};
use replay_data::{
    rows::AirBaseRegistrationRow, rows::FactionRow, rows::GameScoreAirSite,
    rows::GameScoreFleetCount, rows::TableRows,
};
use replay_frame::frame::TableView;

fn input(country: u32) -> CountryGameScoreInputRow {
    CountryGameScoreInputRow {
        country,
        army_count: Some(2),
        navy_count: Some(3),
        land_count: Some(11),
        air_state: CountryGameScoreInputRow::AIR_ABSENT,
        fleets: vec![GameScoreFleetCount {
            fleet: 99,
            task_forces: Some(3),
            error: Vec::new(),
        }],
        ..Default::default()
    }
}

fn multiplier_row() -> GameScoreMultiplierRow {
    GameScoreMultiplierRow {
        total_raw: Some(Q),
        army_raw: Some(Q),
        navy_raw: Some(Q),
        air_raw: Some(Q),
        industry_raw: Some(Q),
        land_raw: Some(Q),
        ..Default::default()
    }
}

fn factories(country: u32) -> CountryFactoriesRow {
    CountryFactoriesRow {
        country,
        naval_total_raw: 100_001,
        civilian_total_raw: 200_099,
        military_total_raw: 300_999,
        ..Default::default()
    }
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    for rows in [
        TableRows::CountryGameScoreInputs(vec![input(9001)]),
        TableRows::AirWingGameScoreStats(Vec::new()),
        TableRows::GameScoreMultipliers(vec![multiplier_row()]),
        TableRows::CountryFactories(vec![factories(9001)]),
        TableRows::AirBaseRegistrations(Vec::new()),
        TableRows::Factions(Vec::new()),
        TableRows::FactionMembers(Vec::new()),
    ] {
        set(&mut frame, rows);
    }
    frame
}

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let id = rows.table();
    *frame.tables[id.index()] = TableView::Complete(rows);
}

fn scan_frames(frames: Vec<DecodedFrame>, localisation: Option<&Localisation>) -> GameScoreReply {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    scan(
        &replay,
        &BranchParams {
            branch_id: "1".into(),
        },
        localisation,
        Err("Faction content unavailable".into()),
        None,
        &mut || content_multipliers(Err("Recorded content unavailable")),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn amount_at(
    reply: &GameScoreReply,
    series: &GameScoreSeries,
    field: &str,
    hour: i32,
) -> GameScoreAmount {
    let runs = match field {
        "army" => &series.army,
        "navy" => &series.navy,
        "air" => &series.air,
        "industry" => &series.industry,
        "land" => &series.land,
        "total" => &series.total,
        _ => panic!("unknown test score field"),
    };
    let run = runs
        .iter()
        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
        .expect("score read at requested hour");
    GameScoreAmount {
        raw: run.value_at(hour),
        missing: run.missing.map(|index| reply.reasons[index].clone()),
        read_each_hour: run.read_each_hour,
    }
}

fn values_at(reply: &GameScoreReply, series: &GameScoreSeries, hour: i32) -> GameScoreValues {
    GameScoreValues {
        army: amount_at(reply, series, "army", hour),
        navy: amount_at(reply, series, "navy", hour),
        air: amount_at(reply, series, "air", hour),
        industry: amount_at(reply, series, "industry", hour),
        land: amount_at(reply, series, "land", hour),
        total: amount_at(reply, series, "total", hour),
    }
}

#[test]
fn game_score_count_arithmetic_keeps_wrapping_scale_and_signed_truncation() {
    // Multiplication by Q is retained before division, even on overflow.
    assert_eq!(count_score(i32::MAX, Q), 30_280_923_962_904);
    assert_ne!(count_score(i32::MAX, Q), i64::from(i32::MAX) * Q);
    assert_eq!(count_score(i32::MIN, i64::MAX), 2_147_483_648);
    assert_eq!(count_score(-7, 100_001), -700_007);
    assert_eq!(air_contribution(i32::MAX, i64::MAX), -2_147_483_647);
    assert_eq!(air_contribution(-3, 100_001), -300_003);
}

#[test]
fn game_score_industry_truncates_and_narrows_each_total_before_transfers() {
    let row = CountryFactoriesRow {
        naval_total_raw: 199_999,
        civilian_total_raw: 199_999,
        military_total_raw: -199_999,
        civilian_to_others: 1,
        civilian_to_overlord: 2,
        military_to_others: 3,
        military_to_overlord: 4,
        // These unrelated economic fields are deliberately not operands.
        naval_levels: 1000,
        military_damaged_raw: 800_000,
        civilian_unavailable: 400,
        consumer_goods: 500,
        ..Default::default()
    };
    assert_eq!(industry_count(&row), -9);
    assert_eq!(
        industry_count(&CountryFactoriesRow {
            naval_total_raw: (i64::from(i32::MAX) + 2) * Q,
            civilian_total_raw: 2 * Q,
            military_to_overlord: i32::MIN,
            ..Default::default()
        }),
        3
    );
}

#[test]
fn game_score_navy_uses_the_wrapped_task_force_count_witness() {
    let mut row = input(9001);
    row.navy_count = Some(-2);
    row.fleets = vec![
        GameScoreFleetCount {
            fleet: 99,
            task_forces: Some(i32::MAX),
            error: Vec::new(),
        };
        2
    ];
    let mut first = frame(100);
    set(
        &mut first,
        TableRows::CountryGameScoreInputs(vec![row.clone()]),
    );
    let reply = scan_frames(vec![first], None);
    let navy = amount_at(&reply, &reply.countries[0].scores, "navy", 100);
    assert_eq!(navy.value(), Some(-200_000));
    assert_ne!(navy.value(), Some(2 * Q));
    // A reply must not accept a count inconsistent with its ordered witnesses.
    row.navy_count = Some(2);
    assert!(replay_data::rows::RowCodec::delta(&row, None).is_err());
}

#[test]
fn game_score_air_preserves_repeated_first_sites_and_wings_without_capacity_gate() {
    let mut row = input(9001);
    row.air_state = CountryGameScoreInputRow::AIR_PRESENT;
    row.first_sites = vec![
        GameScoreAirSite {
            base: 40,
            access: GameScoreAirSite::ACCESS_TRUE,
            error: Vec::new(),
        };
        2
    ];
    row.first_sites.push(GameScoreAirSite {
        base: 41,
        access: GameScoreAirSite::ACCESS_FALSE,
        error: Vec::new(),
    });
    let mut first = frame(100);
    set(&mut first, TableRows::CountryGameScoreInputs(vec![row]));
    set(
        &mut first,
        TableRows::AirBaseRegistrations(vec![
            AirBaseRegistrationRow {
                base: 40,
                slot: 7,
                capacity_raw: 0,
                wings: vec![70, 70],
                ..Default::default()
            },
            // An unrelated/second-list base is not admitted by registration alone.
            AirBaseRegistrationRow {
                base: 42,
                slot: 7,
                capacity_raw: Q,
                wings: vec![71],
                ..Default::default()
            },
        ]),
    );
    set(
        &mut first,
        TableRows::AirWingGameScoreStats(vec![
            AirWingGameScoreStatsRow {
                id: 70,
                planes: Some(3),
                stat_raw: Some(100_001),
                mission_index: Some(0),
                ..Default::default()
            },
            AirWingGameScoreStatsRow {
                id: 71,
                planes: Some(1000),
                stat_raw: Some(100 * Q),
                mission_index: Some(0),
                ..Default::default()
            },
        ]),
    );
    let reply = scan_frames(vec![first], None);
    assert_eq!(
        amount_at(&reply, &reply.countries[0].scores, "air", 100).value(),
        Some(1_200_012)
    );
}

#[test]
fn game_score_known_absent_empty_unreadable_and_missing_wings_are_distinct() {
    let base = frame(100);
    let multipliers = multipliers(Some(&multiplier_row()), "not read");
    let mut row = input(9001);
    assert_eq!(
        air_score(&base, &row, 7, &multipliers.air, None, None).value(),
        Some(0)
    );
    row.air_state = CountryGameScoreInputRow::AIR_PRESENT;
    assert_eq!(
        air_score(&base, &row, 7, &multipliers.air, None, None).value(),
        Some(0)
    );
    row.air_state = CountryGameScoreInputRow::AIR_UNKNOWN;
    row.air_error = b"strategic-air manager unreadable".to_vec();
    let missing = air_score(&base, &row, 7, &multipliers.air, None, None);
    assert_eq!(missing.value(), None);
    assert_eq!(
        missing.missing.unwrap().reason,
        "strategic-air manager unreadable"
    );
    row.air_state = CountryGameScoreInputRow::AIR_PRESENT;
    row.air_error.clear();
    row.first_sites = vec![GameScoreAirSite {
        base: 40,
        access: GameScoreAirSite::ACCESS_TRUE,
        error: Vec::new(),
    }];
    let registration = AirBaseRegistrationRow {
        base: 40,
        slot: 7,
        capacity_raw: 0,
        wings: vec![70],
        ..Default::default()
    };
    let registrations = std::slice::from_ref(&registration);
    let missing = air_score(&base, &row, 7, &multipliers.air, None, Some(registrations));
    assert_eq!(missing.value(), None);
    assert!(
        missing
            .missing
            .unwrap()
            .reason
            .contains("AirWingGameScoreStats")
    );
    row.first_sites[0].access = GameScoreAirSite::ACCESS_UNKNOWN;
    row.first_sites[0].error = b"air access unreadable".to_vec();
    assert_eq!(
        air_score(&base, &row, 7, &multipliers.air, None, Some(registrations))
            .missing
            .unwrap()
            .reason,
        "air access unreadable"
    );
}

#[test]
fn game_score_total_excludes_industry_and_land_and_preserves_native_overflow() {
    assert_eq!(total_score(i64::MAX, 5, -2, Q), 2);
    let first = frame(100);
    let multipliers = multipliers(Some(&multiplier_row()), "not read");
    let values = ScoreContext {
        frame: &first,
        multipliers: &multipliers,
        input_refusal: None,
        wings: None,
        registrations: None,
    }
    .scores(Some(&input(9001)), 7, Some(&factories(9001)));
    assert_eq!(values.army.value(), Some(2 * Q));
    assert_eq!(values.navy.value(), Some(3 * Q));
    assert_eq!(values.industry.value(), Some(6 * Q));
    assert_eq!(values.land.value(), Some(11 * Q));
    assert_eq!(values.total.value(), Some(5 * Q));
}

#[test]
fn game_score_independent_capture_failures_never_supply_partial_total_or_zero() {
    let mut row = input(9001);
    row.army_count = None;
    row.army_error = b"division header unreadable".to_vec();
    row.navy_count = Some(0);
    row.fleets.clear();
    let mut first = frame(100);
    set(&mut first, TableRows::CountryGameScoreInputs(vec![row]));
    let mut multipliers = multiplier_row();
    multipliers.land_raw = None;
    multipliers.land_error = b"PROVINCE_SCORE_MULTIPLIER read failed".to_vec();
    set(
        &mut first,
        TableRows::GameScoreMultipliers(vec![multipliers]),
    );
    let reply = scan_frames(vec![first], None);
    let values = values_at(&reply, &reply.countries[0].scores, 100);
    assert_eq!(values.army.raw, None);
    assert_eq!(
        values.army.missing.as_ref().unwrap().kind,
        GameScoreMissingKind::Recording
    );
    assert_eq!(
        values.army.missing.as_ref().unwrap().reason,
        "division header unreadable"
    );
    assert_eq!(values.navy.value(), Some(0));
    assert_eq!(values.air.value(), Some(0));
    assert_eq!(values.industry.value(), Some(6 * Q));
    assert_eq!(values.land.value(), None);
    assert_eq!(values.total.value(), None);
    assert!(
        values
            .total
            .missing
            .as_ref()
            .unwrap()
            .reason
            .contains("coherent read")
    );
    assert_eq!(
        amount_at(&reply, &reply.multipliers, "land", 100)
            .missing
            .as_ref()
            .unwrap()
            .reason,
        "PROVINCE_SCORE_MULTIPLIER read failed"
    );
}

#[test]
fn game_score_a_missing_hourly_input_table_keeps_industry_available() {
    let first = frame(100);
    let mut unread = first.clone();
    unread.hour = 101;
    *unread.tables[TableId::CountryGameScoreInputs.index()] = TableView::Unavailable {
        reason: "Game score capture disabled".into(),
    };
    let reply = scan_frames(vec![first, unread], None);
    let series = &reply.countries[0].scores;
    assert_eq!(series.total.len(), 2);
    assert_eq!(amount_at(&reply, series, "total", 100).value(), Some(5 * Q));
    assert_eq!(amount_at(&reply, series, "total", 101).value(), None);
    assert_eq!(
        amount_at(&reply, series, "industry", 101).value(),
        Some(6 * Q)
    );
    assert_eq!(
        amount_at(&reply, series, "total", 101)
            .missing
            .as_ref()
            .unwrap()
            .kind,
        GameScoreMissingKind::Recording
    );
}

#[test]
fn game_score_identity_failure_keeps_multiplier_reads_and_breaks_country_runs() {
    let first = frame(100);
    let mut unidentified = first.clone();
    unidentified.hour = 101;
    *unidentified.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "country catalogue unreadable".into(),
    };
    let mut restored = first.clone();
    restored.hour = 102;
    let reply = scan_frames(vec![first, unidentified, restored], None);
    assert_eq!(reply.reads, [100, 101, 102]);
    assert_eq!(reply.multipliers.total.len(), 1);
    let run = &reply.multipliers.total[0];
    assert_eq!((run.first_hour, run.last_hour), (100, 102));
    for amount in values_at(&reply, &reply.multipliers, 101).amounts() {
        assert_eq!(amount.value(), Some(Q));
        assert!(amount.read_each_hour);
    }
    let country = &reply.countries[0];
    assert_eq!(country.scores.total.len(), 2);
    assert_eq!(
        (
            country.scores.total[0].first_hour,
            country.scores.total[0].last_hour
        ),
        (100, 100)
    );
    assert_eq!(
        (
            country.scores.total[1].first_hour,
            country.scores.total[1].last_hour
        ),
        (102, 102)
    );
    assert_eq!(country.identity.len(), 2);
    assert!(
        reply
            .score_warnings
            .iter()
            .any(|warning| warning.kind == GameScoreMissingKind::Recording
                && warning.reason.contains("country identities are unknown")
                && warning.observations == 1)
    );
}

#[test]
fn game_score_content_multipliers_keep_native_rounding_and_missing_defines() {
    let defines = installed_content_core::defines::Defines::new().unwrap();
    defines.load("synthetic.lua", b"NDefines = {NCountry = {COUNTRY_SCORE_MULTIPLIER = 0.000005, ARMY_SCORE_MULTIPLIER = -0.000005, NAVY_SCORE_MULTIPLIER = 1000.00001, AIR_SCORE_MULTIPLIER = 0, INDUSTRY_SCORE_MULTIPLIER = 1.25, PROVINCE_SCORE_MULTIPLIER = 0}}" ).unwrap();
    let multipliers = content_multipliers(Ok(&defines));
    assert_eq!(multipliers.total.value(), Some(1));
    assert_eq!(multipliers.army.value(), Some(-1));
    assert_eq!(multipliers.navy.value(), Some(100_000_000));
    assert_eq!(multipliers.industry.value(), Some(125_000));
    assert!(!multipliers.industry.read_each_hour);
    defines
        .load(
            "missing.lua",
            b"NDefines.NCountry.INDUSTRY_SCORE_MULTIPLIER = nil",
        )
        .unwrap();
    let missing = content_multipliers(Ok(&defines)).industry;
    assert_eq!(missing.value(), None);
    assert_eq!(missing.missing.unwrap().kind, GameScoreMissingKind::Content);
}

#[test]
fn game_score_equal_rereads_are_dated_but_gaps_and_reused_identities_break_runs() {
    let first = frame(100);
    let mut second = first.clone();
    second.hour = 101;
    let mut after_gap = first.clone();
    after_gap.hour = 103;
    let mut reused = after_gap.clone();
    reused.hour = 104;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *reused.tables[TableId::Countries.index()]
    {
        rows[0].id = 9002;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *reused.tables[TableId::CountryLinks.index()]
    {
        rows[1].id = 9002;
    }
    if let TableView::Complete(TableRows::CountryGameScoreInputs(rows)) =
        &mut *reused.tables[TableId::CountryGameScoreInputs.index()]
    {
        rows[0].country = 9002;
    }
    if let TableView::Complete(TableRows::CountryFactories(rows)) =
        &mut *reused.tables[TableId::CountryFactories.index()]
    {
        rows[0].country = 9002;
    }
    let reply = scan_frames(vec![first, second, after_gap, reused], None);
    assert_eq!(reply.reads, [100, 101, 103, 104]);
    let country = &reply.countries[0];
    assert_eq!(
        country
            .scores
            .army
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101), (103, 103), (104, 104)]
    );
    assert!(country.scores.army.iter().all(|run| run.read_each_hour));
    assert_eq!(
        country.identity.last().unwrap().value.native_country,
        Some(9002)
    );
    assert_eq!(
        reply
            .multipliers
            .total
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101), (103, 104)]
    );
}

#[test]
fn game_score_content_fallback_requires_disabled_capture() {
    let historical = replay_data::format::FormatVersion::read(
        TableId::CountryGameScoreInputs.last_change().introduced - 1,
    )
    .unwrap();
    let mut old = frame(100);
    for table in [
        TableId::CountryGameScoreInputs,
        TableId::GameScoreMultipliers,
        TableId::AirWingGameScoreStats,
    ] {
        *old.tables[table.index()] = TableView::Unavailable {
            reason: historical.table_refusal(table).unwrap(),
        };
    }
    let captured = frame(101);
    let mut refused = frame(102);
    for table in [
        TableId::CountryGameScoreInputs,
        TableId::GameScoreMultipliers,
    ] {
        *refused.tables[table.index()] = TableView::Unavailable {
            reason: replay_data::format::GAME_SCORE_DISABLED_REASON.into(),
        };
    }
    let mut failed = frame(103);
    *failed.tables[TableId::GameScoreMultipliers.index()] = TableView::Unavailable {
        reason: "multiplier native read failed".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![old.clone(), captured, refused, failed])],
    )
    .unwrap();
    let mut content_reads = 0;
    let mut old_defines = || {
        content_reads += 1;
        let defines = installed_content_core::defines::Defines::new().unwrap();
        defines.load("synthetic.lua", b"NDefines = {NCountry = {COUNTRY_SCORE_MULTIPLIER = 1, ARMY_SCORE_MULTIPLIER = 1, NAVY_SCORE_MULTIPLIER = 1, AIR_SCORE_MULTIPLIER = 1, INDUSTRY_SCORE_MULTIPLIER = 1.25, PROVINCE_SCORE_MULTIPLIER = 1}}").unwrap();
        content_multipliers(Ok(&defines))
    };
    let reply = scan(
        &replay,
        &BranchParams {
            branch_id: "1".into(),
        },
        None,
        Err("Faction content unavailable".into()),
        None,
        &mut old_defines,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(content_reads, 1);
    let series = &reply.countries[0].scores;
    assert!(amount_at(&reply, series, "industry", 100).raw.is_none());
    assert!(
        amount_at(&reply, series, "army", 100)
            .missing
            .as_ref()
            .unwrap()
            .reason
            .contains("not recorded at format")
    );
    assert_eq!(amount_at(&reply, series, "total", 101).value(), Some(5 * Q));
    assert_eq!(
        amount_at(&reply, series, "industry", 102).value(),
        Some(750_000)
    );
    assert!(!amount_at(&reply, &reply.multipliers, "industry", 102).read_each_hour);
    assert!(
        amount_at(&reply, series, "army", 102)
            .missing
            .as_ref()
            .unwrap()
            .reason
            .contains("disabled at attachment")
    );
    assert!(amount_at(&reply, series, "industry", 103).raw.is_none());
}

fn appearance(country: u32, cosmetic: &[u8]) -> replay_data::rows::CountryAppearanceRow {
    replay_data::rows::CountryAppearanceRow {
        country,
        cosmetic_tag: cosmetic.to_vec(),
        red: 0.25_f32.to_bits(),
        green: 0.5_f32.to_bits(),
        blue: 0.75_f32.to_bits(),
        ..Default::default()
    }
}

fn identity_tables(frame: &mut DecodedFrame, country: u32, cosmetic: &[u8]) {
    set(
        frame,
        TableRows::CountryAppearance(vec![appearance(country, cosmetic)]),
    );
    set(
        frame,
        TableRows::Politics(vec![replay_data::rows::PoliticsRow {
            country,
            ..Default::default()
        }]),
    );
    set(frame, TableRows::PoliticalParties(Vec::new()));
    set(frame, TableRows::Autonomy(Vec::new()));
}

#[test]
fn game_score_identity_names_colours_and_factions_use_the_row_hour() {
    let mut localisation: Localisation = [
        ("GER".to_owned(), "Original name".to_owned()),
        ("COS".to_owned(), "Changed name".to_owned()),
    ]
    .into_iter()
    .collect();
    localisation.language = "english".into();
    let mut first = frame(100);
    identity_tables(&mut first, 9001, b"");
    set(
        &mut first,
        TableRows::Factions(vec![FactionRow {
            id: 88,
            name: b"First faction".to_vec(),
            ..Default::default()
        }]),
    );
    set(
        &mut first,
        TableRows::FactionMembers(vec![FactionMemberRow {
            country: 11,
            faction: 88,
            ordinal: 3,
        }]),
    );
    let mut second = first.clone();
    second.hour = 101;
    identity_tables(&mut second, 9001, b"COS");
    set(&mut second, TableRows::FactionMembers(Vec::new()));
    let mut unread = second.clone();
    unread.hour = 102;
    *unread.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "faction read failed".into(),
    };
    *unread.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "appearance read failed".into(),
    };
    let reply = scan_frames(vec![first, second, unread], Some(&localisation));
    let identities = &reply.countries[0].identity;
    assert_eq!(identities.len(), 3);
    assert_eq!(identities[0].first_hour, 100);
    assert_eq!(identities[0].value.name.as_deref(), Some("Original name"));
    assert_eq!(identities[0].value.color, Some([63, 127, 191]));
    let faction = identities[0].value.faction.as_ref().unwrap();
    assert_eq!(faction.id, "88");
    assert_eq!(faction.name.as_deref(), Some("First faction"));
    assert_eq!(faction.ordinal, 3);
    assert_eq!(identities[1].first_hour, 101);
    assert_eq!(identities[1].value.name.as_deref(), Some("Changed name"));
    assert!(identities[1].value.faction.is_none());
    assert!(identities[1].value.faction_missing.is_none());
    assert!(identities[2].value.name.is_none());
    assert_eq!(identities[2].value.color, None);
    assert_eq!(
        identities[2].value.color_missing.as_ref().unwrap().kind,
        GameScoreMissingKind::Recording
    );
    assert!(identities[2].value.faction.is_none());
    assert_eq!(
        identities[2].value.faction_missing.as_ref().unwrap().kind,
        GameScoreMissingKind::Recording
    );
    assert_eq!(
        amount_at(&reply, &reply.countries[0].scores, "total", 102).value(),
        Some(5 * Q)
    );
}

#[test]
fn game_score_flag_only_changes_do_not_split_identity_runs() {
    let localisation: Localisation = [("GER".into(), "Recorded name".into())]
        .into_iter()
        .collect();
    let mut first = frame(100);
    identity_tables(&mut first, 9001, b"");
    let mut second = first.clone();
    second.hour = 101;
    set(
        &mut second,
        TableRows::Politics(vec![replay_data::rows::PoliticsRow {
            country: 9001,
            ruling_party: Some(77),
            ..Default::default()
        }]),
    );
    set(
        &mut second,
        TableRows::PoliticalParties(vec![replay_data::rows::PoliticalPartyRow {
            country: 9001,
            id: 77,
            token: b"synthetic_ideology".to_vec(),
            ..Default::default()
        }]),
    );
    let reply = scan_frames(vec![first, second], Some(&localisation));
    assert_eq!(reply.countries[0].identity.len(), 1);
    assert_eq!(reply.countries[0].identity[0].last_hour, 101);
}

#[test]
fn game_score_faction_unresolved_membership_is_unknown_for_the_unmatched_country() {
    let mut first = frame(100);
    set(
        &mut first,
        TableRows::Factions(vec![FactionRow {
            id: 88,
            name: Vec::new(),
            ..Default::default()
        }]),
    );
    set(
        &mut first,
        TableRows::FactionMembers(vec![FactionMemberRow {
            country: 9999,
            faction: 88,
            ordinal: 0,
        }]),
    );
    let reply = scan_frames(vec![first], None);
    let identity = &reply.countries[0].identity[0].value;
    assert!(identity.faction.is_none());
    assert!(
        identity
            .faction_missing
            .as_ref()
            .unwrap()
            .reason
            .contains("unresolved or ambiguous")
    );
}

#[test]
fn game_score_known_no_faction_does_not_need_faction_name_rows() {
    let mut first = frame(100);
    *first.tables[TableId::Factions.index()] = TableView::Unavailable {
        reason: "faction names unreadable".into(),
    };
    let mut member = first.clone();
    member.hour = 101;
    set(
        &mut member,
        TableRows::FactionMembers(vec![FactionMemberRow {
            country: 9001,
            faction: 88,
            ordinal: 3,
        }]),
    );
    let reply = scan_frames(vec![first, member], None);
    let identities = &reply.countries[0].identity;
    assert!(identities[0].value.faction.is_none());
    assert!(identities[0].value.faction_missing.is_none());
    let known_member = identities[1].value.faction.as_ref().unwrap();
    assert_eq!(known_member.id, "88");
    assert_eq!(known_member.ordinal, 3);
    assert!(known_member.name.is_none());
    assert_eq!(
        known_member.name_missing.as_ref().unwrap().kind,
        GameScoreMissingKind::Recording
    );
    assert!(identities[1].value.faction_missing.is_none());
}

fn faction_frame(hour: i32, literal: &[u8], template: Option<(u32, &[u8])>) -> DecodedFrame {
    let mut frame = frame(hour);
    identity_tables(&mut frame, 9001, b"");
    set(
        &mut frame,
        TableRows::FactionMembers(vec![FactionMemberRow {
            country: 9001,
            faction: 88,
            ordinal: 0,
        }]),
    );
    set(
        &mut frame,
        TableRows::Factions(vec![FactionRow {
            id: 88,
            name: literal.to_vec(),
            template_token: template.map(|(token, _)| token),
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::LexicalTokens(
            template
                .into_iter()
                .map(|(token, literal)| replay_data::rows::LexicalTokenRow {
                    token,
                    literal: literal.to_vec(),
                })
                .collect(),
        ),
    );
    frame
}

#[test]
fn game_score_faction_names_share_literal_and_bound_template_rules_at_each_hour() {
    let templates = dashboard_text::catalogue::tests::loaded_templates(
        "demo={name={localization_key=NAME ID=token:17}}",
    );
    let localisation =
        dashboard_text::catalogue::tests::loaded("l_english:\n NAME:0 \"Pact $ID$\"\n");
    let literal = faction_frame(100, b"NAME", Some((17, b"demo")));
    let mut first = faction_frame(101, b"", Some((17, b"demo")));
    let mut second = faction_frame(102, b"", Some((17, b"demo")));
    for (frame, token) in [(&mut first, 31), (&mut second, 47)] {
        if let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut *frame.tables[TableId::LexicalTokens.index()]
        {
            rows.push(replay_data::rows::LexicalTokenRow {
                token,
                literal: b"17".to_vec(),
            });
        }
    }
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![literal, first, second])]).unwrap();
    let reply = scan(
        &replay,
        &BranchParams {
            branch_id: "1".into(),
        },
        Some(&localisation),
        Ok(templates.as_ref()),
        None,
        &mut || content_multipliers(Err("unused")),
        &AtomicBool::new(false),
    )
    .unwrap();
    let identities = &reply.countries[0].identity;
    assert_eq!(
        identities
            .iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.value.faction.as_ref().unwrap().name.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            (100, 100, Some("NAME")),
            (101, 101, Some("Pact 31")),
            (102, 102, Some("Pact 47"))
        ]
    );
    assert!(
        identities
            .iter()
            .all(|run| run.value.faction.as_ref().unwrap().name_missing.is_none())
    );
}

#[test]
fn game_score_faction_names_preserve_known_empty_text() {
    let templates = dashboard_text::catalogue::tests::loaded_templates(
        "empty={} localised_empty={name=EMPTY_NAME}",
    );
    let localisation = dashboard_text::catalogue::tests::loaded(
        "l_english:\n EMPTY_NAME:0 \"\"\n NAME:0 \"NAME\"\n",
    );
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                faction_frame(100, b"", None),
                faction_frame(101, b"", Some((17, b"empty"))),
                faction_frame(102, b"", Some((18, b"localised_empty"))),
                faction_frame(103, b"NAME", None),
            ],
        )],
    )
    .unwrap();
    let reply = scan(
        &replay,
        &BranchParams {
            branch_id: "1".into(),
        },
        Some(&localisation),
        Ok(templates.as_ref()),
        None,
        &mut || content_multipliers(Err("unused")),
        &AtomicBool::new(false),
    )
    .unwrap();
    let identities = &reply.countries[0].identity;
    assert_eq!(identities.len(), 2);
    assert_eq!(
        (identities[0].first_hour, identities[0].last_hour),
        (100, 102)
    );
    assert_eq!(
        identities[0]
            .value
            .faction
            .as_ref()
            .unwrap()
            .name
            .as_deref(),
        Some("")
    );
    assert_eq!(
        identities[1]
            .value
            .faction
            .as_ref()
            .unwrap()
            .name
            .as_deref(),
        Some("NAME")
    );
    assert!(
        identities
            .iter()
            .all(|run| run.value.faction.as_ref().unwrap().name_missing.is_none())
    );
}

#[test]
fn game_score_faction_name_failures_keep_stable_recording_and_content_reasons() {
    let templates = dashboard_text::catalogue::tests::loaded_templates("demo={name=NAME}");
    let localisation = dashboard_text::catalogue::tests::loaded("l_english:\n NAME:0 \"Pact\"\n");
    let mut first = faction_frame(100, b"", Some((17, b"demo")));
    let mut second = first.clone();
    second.hour = 101;
    for frame in [&mut first, &mut second] {
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: format!("missing table baseline at hour {}", frame.hour),
        };
    }
    let missing_template = faction_frame(102, b"", Some((18, b"missing_template")));
    let mut missing_argument = faction_frame(103, b"", Some((19, b"token_argument")));
    set(
        &mut missing_argument,
        TableRows::LexicalTokens(vec![replay_data::rows::LexicalTokenRow {
            token: 19,
            literal: b"token_argument".to_vec(),
        }]),
    );
    let templates_with_argument = dashboard_text::catalogue::tests::loaded_templates(
        "demo={name=NAME} token_argument={name={localization_key=NAME ID=token:absent}}",
    );
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![first, second, missing_template, missing_argument],
        )],
    )
    .unwrap();
    let reply = scan(
        &replay,
        &BranchParams {
            branch_id: "1".into(),
        },
        Some(&localisation),
        Ok(templates_with_argument.as_ref()),
        None,
        &mut || content_multipliers(Err("unused")),
        &AtomicBool::new(false),
    )
    .unwrap();
    let identities = &reply.countries[0].identity;
    assert_eq!(identities.len(), 3);
    assert_eq!(
        (identities[0].first_hour, identities[0].last_hour),
        (100, 101)
    );
    let missing = identities[0]
        .value
        .faction
        .as_ref()
        .unwrap()
        .name_missing
        .as_ref()
        .unwrap();
    assert_eq!(
        (missing.kind.clone(), missing.reason.as_str()),
        (
            GameScoreMissingKind::Recording,
            "LexicalTokens baseline missing"
        )
    );
    let missing = identities[1]
        .value
        .faction
        .as_ref()
        .unwrap()
        .name_missing
        .as_ref()
        .unwrap();
    assert_eq!(missing.kind, GameScoreMissingKind::Content);
    assert!(missing.reason.contains("missing_template"));
    let missing = identities[2]
        .value
        .faction
        .as_ref()
        .unwrap()
        .name_missing
        .as_ref()
        .unwrap();
    assert_eq!(missing.kind, GameScoreMissingKind::Recording);
    assert!(missing.reason.contains("absent from LexicalTokens"));
    assert_eq!(
        amount_at(&reply, &reply.countries[0].scores, "total", 103).value(),
        Some(5 * Q)
    );
    // A known template still needs the localisation content; no key-as-name guess.
    let frame = faction_frame(104, b"", Some((17, b"demo")));
    let names = faction_names_at(
        &frame,
        None,
        &localisation,
        "english",
        Ok(templates.as_ref()),
        &FactionLexical::new(&frame),
    )
    .unwrap();
    assert_eq!(
        names[&88].1.as_ref().unwrap().kind,
        GameScoreMissingKind::Content
    );
}

#[test]
fn game_score_faction_literals_check_viewer_language_without_naming_content() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![faction_frame(100, b"Recorded literal", None)],
        )],
    )
    .unwrap();
    for (language, expected) in [(None, Some("Recorded literal")), (Some("french"), None)] {
        let reply = scan(
            &replay,
            &BranchParams {
                branch_id: "1".into(),
            },
            None,
            Err("Faction content unavailable".into()),
            language,
            &mut || content_multipliers(Err("unused")),
            &AtomicBool::new(false),
        )
        .unwrap();
        let faction = reply.countries[0].identity[0]
            .value
            .faction
            .as_ref()
            .unwrap();
        assert_eq!(faction.name.as_deref(), expected);
        if expected.is_none() {
            let missing = faction.name_missing.as_ref().unwrap();
            assert_eq!(missing.kind, GameScoreMissingKind::Content);
            assert!(missing.reason.contains("viewer's language is french"));
        }
    }
}

#[test]
fn game_score_static_subject_name_does_not_need_unused_overlord_naming_rows() {
    let mut first = frame(100);
    identity_tables(&mut first, 9001, b"COS");
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        rows.push(replay_data::rows::CountryRow {
            id: 9002,
            canonical_slot: 8,
            tag: b"OVR".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *first.tables[TableId::CountryLinks.index()]
    {
        rows.push(replay_data::rows::CountryLinkRow {
            id: 9002,
            canonical_slot: 8,
        });
    }
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *first.tables[TableId::CountryAppearance.index()]
    {
        rows[0].overlord = 9002;
    }
    let localisation: Localisation = [("COS".to_owned(), "Plain subject name".to_owned())]
        .into_iter()
        .collect();
    let shared_names = CountryNames::read(&first, &localisation);
    assert_eq!(shared_names.name(7).unwrap(), "Plain subject name");
    let reply = scan_frames(vec![first], Some(&localisation));
    let country = reply
        .countries
        .iter()
        .find(|country| country.tag == "GER")
        .unwrap();
    assert_eq!(
        country.identity[0].value.name.as_deref(),
        Some("Plain subject name")
    );
    assert!(country.identity[0].value.name_missing.is_none());
}

#[test]
fn game_score_names_are_read_again_only_for_changed_operands_or_slot_zero() {
    let localisation = Localisation::default();
    let mut first = frame(100);
    identity_tables(&mut first, 9001, b"");
    let countries = first.complete::<tables::Countries>().unwrap();
    let names = HourNames::read(&first, countries, &localisation);
    let mut balance = first.clone();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *balance.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(2);
    }
    assert!(names.agrees(&balance));
    let mut cosmetic = first.clone();
    identity_tables(&mut cosmetic, 9001, b"COS");
    assert!(!names.agrees(&cosmetic));
    // Slot zero's refusal names its hour, so its subject's name is read every hour.
    let mut subject = first.clone();
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *subject.tables[TableId::CountryLinks.index()]
    {
        rows[0].canonical_slot = 0;
    }
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *subject.tables[TableId::CountryAppearance.index()]
    {
        rows[0].overlord = 11;
    }
    let countries = subject.complete::<tables::Countries>().unwrap();
    let names = HourNames::read(&subject, countries, &localisation);
    assert!(!names.agrees(&subject));
}

fn scale_score_frame(count: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, 100, count);
    // Ordinary countries use positive slots; zero has separate naming operands.
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.canonical_slot += 1;
        }
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        for row in rows {
            row.canonical_slot += 1;
        }
    }
    for rows in [
        TableRows::CountryGameScoreInputs((0..count).map(|index| input(100_000 + index)).collect()),
        TableRows::CountryFactories((0..count).map(|index| factories(100_000 + index)).collect()),
        TableRows::CountryAppearance(
            (0..count)
                .map(|index| appearance(100_000 + index, b""))
                .collect(),
        ),
        TableRows::Politics(
            (0..count)
                .map(|index| replay_data::rows::PoliticsRow {
                    country: 100_000 + index,
                    ..Default::default()
                })
                .collect(),
        ),
        TableRows::GameScoreMultipliers(vec![multiplier_row()]),
        TableRows::AirWingGameScoreStats(Vec::new()),
        TableRows::AirBaseRegistrations(Vec::new()),
        TableRows::Factions(Vec::new()),
        TableRows::FactionMembers(Vec::new()),
        TableRows::PoliticalParties(Vec::new()),
        TableRows::Autonomy(Vec::new()),
    ] {
        set(&mut frame, rows);
    }
    frame
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn game_score_keeps_all_five_thousand_countries_without_player_admission() {
    let first = scale_score_frame(5000);
    let mut second = first.clone();
    second.hour = 101;
    let reply = scan_frames(vec![first, second], None);
    assert_eq!(reply.countries.len(), 5000);
    assert_eq!(reply.reads, [100, 101]);
    for country in &reply.countries {
        assert_eq!(country.scores.total.len(), 1);
        assert_eq!(
            (
                country.scores.total[0].first_hour,
                country.scores.total[0].last_hour
            ),
            (100, 101)
        );
        assert_eq!(
            amount_at(&reply, &country.scores, "total", 101).value(),
            Some(5 * Q)
        );
    }
    assert!(dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_some());
}

#[test]
fn game_score_equal_rereads_do_not_expand_sparse_histories() {
    let first = frame(0);
    let mut changed = first.clone();
    set(
        &mut changed,
        TableRows::CountryGameScoreInputs(vec![CountryGameScoreInputRow {
            army_count: Some(3),
            ..input(9001)
        }]),
    );
    let hours = 3;
    let frames = (0..hours)
        .map(|hour| {
            let mut frame = if hour == 0 {
                first.clone()
            } else {
                changed.clone()
            };
            frame.hour = hour;
            frame
        })
        .collect();
    let reply = scan_frames(frames, None);
    assert_eq!(reply.reads.len(), hours as usize);
    assert_eq!(reply.reads.last(), Some(&(hours - 1)));
    assert_eq!(reply.countries[0].scores.total.len(), 1);
    assert_eq!(reply.countries[0].scores.total[0].last_hour, hours - 1);
    assert_eq!(
        reply.countries[0].scores.army[0].deltas.as_deref(),
        Some("1:1")
    );
    assert_eq!(
        reply.countries[0].scores.total[0].deltas.as_deref(),
        Some("1:1")
    );
    assert_eq!(
        amount_at(&reply, &reply.countries[0].scores, "total", hours - 1).value(),
        Some(6 * Q)
    );
    assert!(reply.countries[0].scores.total[0].read_each_hour);
    assert!(dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_some());
}

#[test]
fn game_score_field_runs_share_reasons_without_repeating_unchanged_fields() {
    let mut series = GameScoreSeries::default();
    let mut reasons = ReasonTable::default();
    let mut budget = 0;
    let mut values = unknown_values("one unreadable operand");
    values.army = GameScoreAmount::known(Q, true);
    values.land = GameScoreAmount::known(0, true);
    series
        .push(100, &values, false, &mut reasons, &mut budget)
        .unwrap();
    values.army = GameScoreAmount::known(2 * Q, true);
    series
        .push(101, &values, false, &mut reasons, &mut budget)
        .unwrap();
    assert_eq!(reasons.entries.len(), 1);
    assert_eq!(series.army.len(), 1);
    assert_eq!(series.army[0].value_at(101), Some(2 * Q));
    assert_eq!(series.land[0].value_at(101), Some(0));
    assert!(series.land[0].deltas.is_none());
    for runs in [&series.navy, &series.air, &series.industry, &series.total] {
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].missing, Some(0));
        assert_eq!(runs[0].last_hour, 101);
        assert!(!serde_json::to_string(&runs[0]).unwrap().contains("null"));
    }
}

#[test]
fn game_score_table_missing_reasons_do_not_depend_on_hour_or_truncation_count() {
    let mut first = frame(100);
    let mut later = frame(200);
    for table in [
        TableId::CountryGameScoreInputs,
        TableId::GameScoreMultipliers,
        TableId::AirBaseRegistrations,
        TableId::AirWingGameScoreStats,
        TableId::CountryFactories,
        TableId::CountryAppearance,
    ] {
        for (frame, missing) in [(&mut first, 1), (&mut later, 20)] {
            *frame.tables[table.index()] = TableView::Truncated {
                rows: TableRows::empty(table),
                missing,
            };
        }
        assert_eq!(
            table_missing_reason(&first, table),
            Some(format!("{table:?} is truncated"))
        );
        assert_eq!(
            table_missing_reason(&first, table),
            table_missing_reason(&later, table)
        );
        for frame in [&mut first, &mut later] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: format!("missing table baseline at hour {}", frame.hour),
            };
        }
        assert_eq!(
            table_missing_reason(&first, table),
            Some(format!("{table:?} baseline missing"))
        );
        assert_eq!(
            table_missing_reason(&first, table),
            table_missing_reason(&later, table)
        );
        for (frame, missing) in [(&mut first, 1), (&mut later, 20)] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: format!("truncated at keyframe: {missing} rows not read"),
            };
        }
        assert_eq!(
            table_missing_reason(&first, table),
            Some(format!("{table:?} is truncated"))
        );
        assert_eq!(
            table_missing_reason(&first, table),
            table_missing_reason(&later, table)
        );
        for frame in [&mut first, &mut later] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: replay_data::format::GAME_SCORE_DISABLED_REASON.into(),
            };
        }
        assert_eq!(
            table_missing_reason(&first, table).as_deref(),
            Some(replay_data::format::GAME_SCORE_DISABLED_REASON)
        );
        assert_eq!(
            table_missing_reason(&first, table),
            table_missing_reason(&later, table)
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn game_score_disabled_across_block_boundaries_keeps_one_run_and_one_reason() {
    // One initial block and two appended block windows exercise both boundaries.
    let hours = replay_frame::KEYFRAME_INTERVAL as i32 + 2;
    let countries = 2;
    let mut first = scale_score_frame(countries);
    first.hour = 0;
    for table in [
        TableId::CountryGameScoreInputs,
        TableId::AirWingGameScoreStats,
        TableId::GameScoreMultipliers,
        TableId::CountryAppearance,
    ] {
        *first.tables[table.index()] = TableView::Unavailable {
            reason: replay_data::format::GAME_SCORE_DISABLED_REASON.into(),
        };
    }
    let initial = records(1, vec![first.clone()]);
    let context = initial[..2].to_vec();
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[initial]).unwrap();
    // Append one native block window at a time instead of retaining every
    // synthetic hourly table-update array before the production scan.
    for first_hour in (1..hours).step_by(replay_frame::KEYFRAME_INTERVAL as usize) {
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        let mut segment = context.clone();
        for hour in first_hour..(first_hour + replay_frame::KEYFRAME_INTERVAL as i32).min(hours) {
            let mut frame = first.clone();
            frame.hour = hour;
            segment.push(replay::spool::Record::Frame(
                encoder.encode(frame, false).unwrap(),
            ));
        }
        replay = replay.append(&[segment]).unwrap();
    }
    assert!(replay.index.len() >= 3);
    let defines = installed_content_core::defines::Defines::new().unwrap();
    defines.load("synthetic.lua", b"NDefines = {NCountry = {COUNTRY_SCORE_MULTIPLIER = 1, ARMY_SCORE_MULTIPLIER = 1, NAVY_SCORE_MULTIPLIER = 1, AIR_SCORE_MULTIPLIER = 1, INDUSTRY_SCORE_MULTIPLIER = 1, PROVINCE_SCORE_MULTIPLIER = 1}}").unwrap();
    let fallback = content_multipliers(Ok(&defines));
    let reply = scan(
        &replay,
        &BranchParams {
            branch_id: "1".into(),
        },
        None,
        Err("Faction content unavailable".into()),
        None,
        &mut || fallback.clone(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.reads, (0..hours).collect::<Vec<_>>());
    assert_eq!(reply.countries.len(), countries as usize);
    assert_eq!(
        reply.reasons,
        [recording_missing(
            replay_data::format::GAME_SCORE_DISABLED_REASON
        )]
    );
    for runs in [
        &reply.multipliers.army,
        &reply.multipliers.navy,
        &reply.multipliers.air,
        &reply.multipliers.industry,
        &reply.multipliers.land,
        &reply.multipliers.total,
    ] {
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].first_hour, runs[0].last_hour), (0, hours - 1));
        assert_eq!(runs[0].value_at(hours - 1), Some(Q));
        assert!(runs[0].deltas.is_none());
    }
    for country in &reply.countries {
        assert_eq!(country.identity.len(), 1);
        assert_eq!(
            (
                country.identity[0].first_hour,
                country.identity[0].last_hour
            ),
            (0, hours - 1)
        );
        for runs in [
            &country.scores.army,
            &country.scores.navy,
            &country.scores.air,
            &country.scores.industry,
            &country.scores.land,
            &country.scores.total,
        ] {
            assert_eq!(runs.len(), 1);
            assert_eq!((runs[0].first_hour, runs[0].last_hour), (0, hours - 1));
            assert!(runs[0].deltas.is_none());
        }
        for hour in [0, 24, hours / 2, hours - 1] {
            let values = values_at(&reply, &country.scores, hour);
            assert_eq!(values.industry.value(), Some(6 * Q));
            for amount in [
                &values.army,
                &values.navy,
                &values.air,
                &values.land,
                &values.total,
            ] {
                assert_eq!(
                    amount.missing.as_ref().unwrap().reason,
                    replay_data::format::GAME_SCORE_DISABLED_REASON
                );
            }
        }
    }
    let disabled_warnings: Vec<_> = reply
        .score_warnings
        .iter()
        .filter(|warning| warning.reason == replay_data::format::GAME_SCORE_DISABLED_REASON)
        .collect();
    assert_eq!(disabled_warnings.len(), 1);
    assert_eq!(
        disabled_warnings[0].observations,
        u64::from(countries) * 6 * hours as u64
    );
    assert!(dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_some());
}

#[test]
fn game_score_missing_tables_keep_runs_when_other_tables_change() {
    let mut first = frame(100);
    let mut second = frame(101);
    for frame in [&mut first, &mut second] {
        *frame.tables[TableId::CountryGameScoreInputs.index()] = TableView::Unavailable {
            reason: "score inputs unreadable".into(),
        };
        *frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
            reason: "appearance unreadable".into(),
        };
    }
    set(
        &mut second,
        TableRows::AirBaseRegistrations(vec![AirBaseRegistrationRow {
            base: 40,
            slot: 7,
            capacity_raw: Q,
            wings: Vec::new(),
            ..Default::default()
        }]),
    );
    let reply = scan_frames(vec![first, second], None);
    let country = &reply.countries[0];
    assert_eq!(country.identity.len(), 1);
    assert_eq!(
        country.identity[0]
            .value
            .color_missing
            .as_ref()
            .unwrap()
            .reason,
        "appearance unreadable"
    );
    assert_eq!(
        reply.reasons,
        [recording_missing("score inputs unreadable")]
    );
    for runs in [
        &country.scores.army,
        &country.scores.navy,
        &country.scores.air,
        &country.scores.industry,
        &country.scores.land,
        &country.scores.total,
    ] {
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].first_hour, runs[0].last_hour), (100, 101));
    }
}

fn campaign_values(country: i32, hour: i32) -> GameScoreValues {
    let day = i64::from(hour / 24);
    let sample = i64::from(hour / 6);
    let army = (i64::from(country) + day) * 15_007;
    let navy = 300_011 + day / 7 * Q + i64::from(country);
    let air = 1_000_007 + (sample * sample % 97) * 701 - (sample % 11) * 113;
    GameScoreValues {
        army: GameScoreAmount::known(army, true),
        navy: GameScoreAmount::known(navy, true),
        air: GameScoreAmount::known(air, true),
        industry: GameScoreAmount::known(600_019 + day * 23_009 + day % 3 * 17, true),
        land: GameScoreAmount::known((i64::from(country) + day % 2) * 10_003, true),
        total: GameScoreAmount::known(total_score(army, navy, air, Q), true),
    }
}

#[test]
fn game_score_sparse_five_year_campaign_for_seventy_countries_fits_and_round_trips() {
    let hours = 24 * 366 * 5;
    let mut reasons = ReasonTable::default();
    let mut budget = 0;
    let mut countries = Vec::new();
    for country in 0..70 {
        let mut scores = GameScoreSeries::default();
        scores
            .push(
                0,
                &campaign_values(country, 0),
                false,
                &mut reasons,
                &mut budget,
            )
            .unwrap();
        for hour in (6..hours).step_by(6) {
            // The synthetic intervening hourly rereads are equal; fill their span
            // directly instead of pushing thirteen million fixture observations.
            for runs in [
                &mut scores.army,
                &mut scores.navy,
                &mut scores.air,
                &mut scores.industry,
                &mut scores.land,
                &mut scores.total,
            ] {
                runs.last_mut().unwrap().last_hour = hour - 1;
            }
            scores
                .push(
                    hour,
                    &campaign_values(country, hour),
                    false,
                    &mut reasons,
                    &mut budget,
                )
                .unwrap();
        }
        for runs in [
            &mut scores.army,
            &mut scores.navy,
            &mut scores.air,
            &mut scores.industry,
            &mut scores.land,
            &mut scores.total,
        ] {
            assert_eq!(runs.len(), 1);
            assert!(runs[0].deltas.is_some());
            runs[0].last_hour = hours - 1;
        }
        countries.push(GameScoreCountry {
            tag: format!("SYN{country}"),
            identity: vec![GameScoreRun {
                first_hour: 0,
                last_hour: hours - 1,
                value: GameScoreIdentity {
                    canonical_slot: Some(country as u32),
                    native_country: Some(100_000 + country as u32),
                    name: Some(format!("Synthetic country {country}")),
                    name_missing: None,
                    color: Some([12, 34, 56]),
                    color_missing: None,
                    faction: None,
                    faction_missing: None,
                },
            }],
            scores,
        });
    }
    let mut multiplier_series = GameScoreSeries::default();
    multiplier_series
        .push(
            0,
            &multipliers(Some(&multiplier_row()), "not read"),
            false,
            &mut reasons,
            &mut budget,
        )
        .unwrap();
    for runs in [
        &mut multiplier_series.army,
        &mut multiplier_series.navy,
        &mut multiplier_series.air,
        &mut multiplier_series.industry,
        &mut multiplier_series.land,
        &mut multiplier_series.total,
    ] {
        runs[0].last_hour = hours - 1;
    }
    let reply = GameScoreReply {
        branch_id: "1".into(),
        scale: Q,
        reads: (0..hours).collect(),
        countries,
        multipliers: multiplier_series,
        reasons: reasons.entries,
        labels: score_labels(),
        hints: BTreeMap::new(),
        warnings: Vec::new(),
        score_warnings: Vec::new(),
    };
    assert!(dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_some());
    for country in [0, 35, 69] {
        for hour in [0, 5, 6, 23, 24, 25, 168, 721, hours - 7, hours - 1] {
            assert_eq!(
                values_at(&reply, &reply.countries[country as usize].scores, hour),
                campaign_values(country, hour)
            );
        }
    }
}

#[test]
fn game_score_bad_tag_refuses_only_its_country_and_counts_the_reason() {
    let mut first = scale_score_frame(2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        rows[0].tag = vec![0xff];
    }
    let reply = scan_frames(vec![first], None);
    assert_eq!(reply.countries.len(), 1);
    assert_eq!(
        amount_at(&reply, &reply.countries[0].scores, "total", 100).value(),
        Some(5 * Q)
    );
    assert!(reply.score_warnings.iter().any(|warning| warning.kind
        == GameScoreMissingKind::Recording
        && warning.reason.contains("UTF-8")
        && warning.reason.contains("ff")
        && warning.observations == 1));
}

#[test]
fn game_score_orphan_operand_counts_its_missing_country_identity() {
    let mut first = frame(100);
    set(
        &mut first,
        TableRows::CountryGameScoreInputs(vec![input(0), input(9001)]),
    );
    let reply = scan_frames(vec![first], None);
    assert_eq!(reply.countries.len(), 1);
    assert_eq!(
        amount_at(&reply, &reply.countries[0].scores, "total", 100).value(),
        Some(5 * Q)
    );
    assert!(reply.score_warnings.iter().any(|warning| warning.kind == GameScoreMissingKind::Recording
        && warning.reason == "CountryGameScoreInputs country raw reference 0 has no CountryLinks identity at this read"
        && warning.observations == 1));
}

#[test]
fn game_score_plain_headers_keep_localised_sort_text_only_as_hints() {
    let localisation: Localisation = [
        (
            "ENDGAME_SORT_ARMY".to_owned(),
            "Synthetic sort force tooltip".to_owned(),
        ),
        (
            "ENDGAME_SORT_PROVINCE".to_owned(),
            "Synthetic sort land tooltip".to_owned(),
        ),
        (
            "SCORE_CALC_SCORE_MULT".to_owned(),
            "Multiplier: $MULT$".to_owned(),
        ),
        ("ENDGAME_SORT_NAVY".to_owned(), "[ROOT.GetName]".to_owned()),
    ]
    .into_iter()
    .collect();
    let mut warnings = BTreeMap::new();
    let hints = score_hints(Some(&localisation), &mut warnings);
    assert_eq!(score_labels()["army"], "Army");
    assert_eq!(score_labels()["total"], "Total score");
    assert_eq!(hints["army"], "Synthetic sort force tooltip");
    assert_eq!(hints["land"], "Synthetic sort land tooltip");
    assert!(!hints.contains_key("navy"));
    assert!(!hints.contains_key("total"));
    assert!(!hints.contains_key("multiplier"));
    assert!(
        warnings
            .keys()
            .any(|(_, reason)| reason.contains("ENDGAME_SORT_NAVY"))
    );
    assert!(
        !warnings
            .keys()
            .any(|(_, reason)| reason.contains("SCORE_CALC_SCORE_MULT"))
    );
    assert!(
        !warnings
            .keys()
            .any(|(_, reason)| reason.contains("ENDGAME_SORT_FACTION"))
    );
}

#[test]
fn game_score_sort_hints_preserve_empty_whitespace_and_key_equal_text() {
    let localisation = dashboard_text::catalogue::tests::loaded(
        "l_english:\n ENDGAME_SORT_ARMY:0 \"\"\n ENDGAME_SORT_AIR:0 \"   \"\n ENDGAME_SORT_PROVINCE:0 \"ENDGAME_SORT_PROVINCE\"\n",
    );
    let mut warnings = BTreeMap::new();
    let hints = score_hints(Some(&localisation), &mut warnings);
    assert_eq!(hints["army"], "");
    assert_eq!(hints["air"], "   ");
    assert_eq!(hints["land"], "ENDGAME_SORT_PROVINCE");
    for key in [
        "ENDGAME_SORT_ARMY",
        "ENDGAME_SORT_AIR",
        "ENDGAME_SORT_PROVINCE",
    ] {
        assert!(warnings.keys().all(|(_, reason)| !reason.contains(key)));
    }
    assert!(!hints.contains_key("total"));
    assert!(
        warnings
            .keys()
            .any(|(_, reason)| reason.contains("ENDGAME_SORT_SCORE"))
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn game_score_oversized_reply_refuses_by_transport_budget_name() {
    let mut first = scale_score_frame(5000);
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *first.tables[TableId::CountryAppearance.index()]
    {
        for row in rows {
            row.name_tag = b"SYN".to_vec();
        }
    }
    let localisation: Localisation = [(
        "SYN".to_owned(),
        "x".repeat(MAX_PRODUCT_REPLY_BYTES / 5000 + 1),
    )]
    .into_iter()
    .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first])]).unwrap();
    let error = scan(
        &replay,
        &BranchParams {
            branch_id: "1".into(),
        },
        Some(&localisation),
        Err("Faction content unavailable".into()),
        None,
        &mut || content_multipliers(Err("Recorded content unavailable")),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "game_score reply exceeds MAX_PRODUCT_REPLY_BYTES"
    );
}

fn prepared_naming() -> Naming {
    let mut fallback = Localisation::default();
    fallback.language = "english".into();
    Naming {
        localisation: Err("Recorded content unavailable".into()),
        templates: Err("Faction content unavailable".into()),
        fallback,
        warnings: Vec::new(),
    }
}
fn assert_prepared_score_at(
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    reference: &GameScoreReply,
    hour: i32,
) {
    let value = prepared::query(
        "series-state",
        serde_json::json!({ "branchId": "1",
        "family": {"kind": "game-score"}, "reads": [{"hour": hour}] }),
        &fixture.store,
        &fixture.key,
        &AtomicBool::new(false),
    )
    .unwrap();
    let actual: prepared::GameScoreStateReply = serde_json::from_value(value).unwrap();
    let recorded = reference.reads.contains(&hour);
    assert_eq!(
        actual.reply.reads,
        if recorded { vec![hour] } else { Vec::new() }
    );
    assert_eq!(actual.reply.score_warnings, reference.score_warnings);
    assert_eq!(actual.reply.labels, reference.labels);
    assert_eq!(actual.reply.hints, reference.hints);
    if recorded {
        assert_eq!(
            values_at(&actual.reply, &actual.reply.multipliers, hour),
            values_at(reference, &reference.multipliers, hour)
        );
    }
    let expected: Vec<_> = reference
        .countries
        .iter()
        .filter(|country| {
            country
                .identity
                .iter()
                .any(|run| run.first_hour <= hour && hour <= run.last_hour)
        })
        .collect();
    assert_eq!(actual.reply.countries.len(), expected.len());
    for (actual_country, expected_country) in actual.reply.countries.iter().zip(expected) {
        assert_eq!(actual_country.tag, expected_country.tag);
        assert_eq!(
            values_at(&actual.reply, &actual_country.scores, hour),
            values_at(reference, &expected_country.scores, hour)
        );
        let identity = expected_country
            .identity
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .unwrap();
        assert_eq!(actual_country.identity[0].value, identity.value);
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_game_score_matches_history_after_commits_resume_append_gaps_and_identity_changes() {
    let mut first = frame(100);
    let mut second = frame(101);
    let mut missing = frame(102);
    *missing.tables[TableId::CountryGameScoreInputs.index()] = TableView::Unavailable {
        reason: "synthetic score inputs unavailable".into(),
    };
    let mut replacement = frame(104);
    for input_frame in [&mut first, &mut second, &mut replacement] {
        identity_tables(input_frame, 9001, b"GER");
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *replacement.tables[TableId::Countries.index()]
    {
        rows[0].id += 1;
    }
    set(
        &mut second,
        TableRows::CountryGameScoreInputs(vec![CountryGameScoreInputRow {
            army_count: Some(17),
            ..input(9001)
        }]),
    );
    let frames = [first, second, missing, replacement];
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let naming = prepared_naming();
    let mut state = prepared::Prepared::default();
    for end in [1, 3, 4] {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..end].to_vec())]).unwrap();
        let begin = match end {
            1 => 0,
            3 => 1,
            _ => 3,
        };
        let mut observer = naming.observer(&replay);
        let mut changes = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        for frame in &frames[begin..end] {
            state
                .observe(
                    frame,
                    &replay,
                    &mut observer,
                    &naming,
                    &changes.observe(frame, replay.format),
                )
                .unwrap();
        }
        let mut writer = if end == 4 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        state
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        let saved = serde_json::to_vec(&state).unwrap();
        writer
            .write_at("score-test-resume", 0, saved.as_slice())
            .unwrap();
        writer.commit().unwrap();
        let reference = scan_frames(frames[..end].to_vec(), None);
        for hour in 100..=frames[end - 1].hour {
            assert_prepared_score_at(&fixture, &reference, hour);
        }
        state = serde_json::from_reader(
            fixture
                .store
                .read(&fixture.key, "score-test-resume", 0)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_game_score_five_thousand_countries_retains_only_open_facts() {
    let first = scale_score_frame(5000);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first.clone()])]).unwrap();
    let naming = prepared_naming();
    let mut observer = naming.observer(&replay);
    let mut state = prepared::Prepared::default();
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut changes = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    state
        .observe(
            &first,
            &replay,
            &mut observer,
            &naming,
            &changes.observe(&first, replay.format),
        )
        .unwrap();
    let mut writer = fixture.writer();
    state
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    let first_bytes = serde_json::to_vec(&state).unwrap().len();
    for hour in 101..=164 {
        let mut next = first.clone();
        next.hour = hour;
        state
            .observe(
                &next,
                &replay,
                &mut observer,
                &naming,
                &changes.observe(&next, replay.format),
            )
            .unwrap();
    }
    let mut writer = fixture.writer();
    state
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    let later_bytes = serde_json::to_vec(&state).unwrap().len();
    eprintln!("game-score continuation: {first_bytes} -> {later_bytes} bytes at 5000 countries");
    // Counters/partial buckets may change; retaining an hourly row per key cannot pass.
    assert!(later_bytes < first_bytes * 2);
    let reference = scan_frames(
        (100..=164)
            .map(|hour| {
                let mut frame = first.clone();
                frame.hour = hour;
                frame
            })
            .collect(),
        None,
    );
    assert_prepared_score_at(&fixture, &reference, 100);
    assert_prepared_score_at(&fixture, &reference, 164);
}

#[test]
fn prepared_game_score_unchanged_work_is_flat_over_hours_and_accumulated_holders() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let measure = |history: usize, hours: i32| -> io::Result<observer_tests::Work> {
        let mut first = frame(100);
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first.clone()])]).unwrap();
        let naming = prepared_naming();
        let mut observer = naming.observer(&replay);
        let mut state = prepared::Prepared::default();
        let mut changes = FrameChangeTracker::new();
        for index in 0..history {
            first.hour = 100 + index as i32;
            identity_tables(&mut first, 9001, format!("H{index:03}").as_bytes());
            state.observe(
                &first,
                &replay,
                &mut observer,
                &naming,
                &changes.observe(&first, replay.format),
            )?;
        }
        let work = observer_tests::unchanged_frames(
            state.work(),
            &first,
            hours,
            replay.format,
            |frame, changes| {
                state.observe(frame, &replay, &mut observer, &naming, changes)?;
                Ok(state.work())
            },
        )?;
        assert_eq!(
            work,
            observer_tests::Work::default(),
            "unchanged keys or history were visited"
        );
        let before = state.work();
        let mut baseline = first.clone();
        baseline.hour += hours;
        changes.observe(&baseline, replay.format);
        first.hour += hours + 1;
        if let TableView::Complete(TableRows::CountryGameScoreInputs(rows)) =
            &mut *first.tables[TableId::CountryGameScoreInputs.index()]
        {
            rows[0].army_count = Some(27);
        }
        state.observe(
            &first,
            &replay,
            &mut observer,
            &naming,
            &changes.observe(&first, replay.format),
        )?;
        let after = state.work();
        Ok(observer_tests::Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        })
    };
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |hours| measure(1, hours as i32)).unwrap(),
    );
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |history| measure(history, 8)).unwrap(),
    );
}

#[test]
fn prepared_game_score_balance_only_changes_reuse_native_operands_and_warning_counts() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let first = frame(100);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first.clone()])]).unwrap();
    let naming = prepared_naming();
    let mut observer = naming.observer(&replay);
    let mut prepared = prepared::Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    prepared
        .observe(
            &first,
            &replay,
            &mut observer,
            &naming,
            &tracker.observe(&first, replay.format),
        )
        .unwrap();
    let work = prepared.work();
    let mut next = first.clone();
    for hour in 101..=110 {
        next.hour = hour;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *next.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(i64::from(hour));
        }
        prepared
            .observe(
                &next,
                &replay,
                &mut observer,
                &naming,
                &tracker.observe(&next, replay.format),
            )
            .unwrap();
        assert_eq!(prepared.work(), work);
        assert_eq!(observer.current.as_ref().unwrap().projected, 100);
    }
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut writer = fixture.writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    let reference = scan_frames(
        (100..=110)
            .map(|hour| {
                let mut frame = first.clone();
                frame.hour = hour;
                frame
            })
            .collect(),
        None,
    );
    assert_prepared_score_at(&fixture, &reference, 110);
}

#[test]
#[cfg(feature = "slow-tests")]
fn progressive_game_score_sampler_matches_observer_at_five_thousand_countries() {
    let mut frame = scale_score_frame(5000);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.pop(); // A present tag whose native holder is unresolved stays unknown.
    }
    if let TableView::Complete(TableRows::CountryGameScoreInputs(rows)) =
        &mut *frame.tables[TableId::CountryGameScoreInputs.index()]
    {
        for (index, row) in rows.iter_mut().enumerate() {
            row.army_count = (index % 7 != 0).then_some(index as i32 - 2500);
            // An unread count carries its named error, as the recording requires.
            row.army_error = if row.army_count.is_some() {
                Vec::new()
            } else {
                b"fixture unread army".to_vec()
            };
            row.land_count = Some(index as i32);
        }
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]).unwrap();
    let naming = prepared_naming();
    let mut observer = naming.observer(&replay);
    let observed = observer
        .observe(&frame, &replay, &mut || panic!("captured multipliers"))
        .unwrap();
    let samples = sample(&frame, replay.format, &mut || {
        panic!("captured multipliers")
    });
    let values = samples.values();
    assert_eq!(values.len(), 5000 * 6 + 6);
    for (tag, (scores, identity)) in &observed.countries {
        for (field, amount) in PARTS.iter().zip(scores.amounts()) {
            assert_eq!(
                values[&(tag.clone(), (*field).into())],
                (
                    identity.canonical_slot.zip(identity.native_country),
                    amount.raw.map(i128::from),
                )
            );
        }
    }
    for (field, amount) in PARTS.iter().zip(observed.multipliers.amounts()) {
        assert_eq!(
            values[&(String::new(), format!("multiplier:{field}"))],
            (None, amount.raw.map(i128::from))
        );
    }
    assert!(!values.contains_key(&("unseen country".into(), "total".into())));
}

/// The recording codec: each input hour is encoded and decoded as the
/// preparation pipeline reads it, with its recorded updates.
struct Recorded {
    encoder: replay_frame::frame::FrameEncoder,
    decoder: FrameDecoder,
}

impl Recorded {
    fn new() -> Self {
        Self {
            encoder: replay_frame::frame::FrameEncoder::new(),
            decoder: FrameDecoder::new(),
        }
    }
    fn next(
        &mut self,
        input: &DecodedFrame,
        keyframe: bool,
    ) -> (DecodedFrame, std::sync::Arc<replay_frame::frame::Frame>) {
        let wire = self.encoder.encode(input.clone(), keyframe).unwrap();
        let decoded = self.decoder.advance(&wire).unwrap().clone();
        (decoded, std::sync::Arc::new(wire))
    }
}

/// One score preparation fed recorded hours with their changes, as the
/// pipeline feeds them.
struct Lane<'a> {
    observer: ScoreObserver<'a>,
    prepared: prepared::Prepared,
    tracker: dashboard_base::prepared::frame_changes::FrameChangeTracker,
}

impl<'a> Lane<'a> {
    fn new(naming: &'a Naming, replay: &'a Replay) -> Self {
        Self {
            observer: naming.observer(replay),
            prepared: prepared::Prepared::default(),
            tracker: dashboard_base::prepared::frame_changes::FrameChangeTracker::new(),
        }
    }

    /// Observe one recorded hour. Gives the countries whose facts were
    /// computed, whether the hour was read from its changed rows, and the
    /// preparation's country visits, observed countries and emitted keys.
    fn observe(
        &mut self,
        (decoded, wire): &(DecodedFrame, std::sync::Arc<replay_frame::frame::Frame>),
        replay: &Replay,
        naming: &Naming,
    ) -> (usize, usize, [usize; 3]) {
        let before = (
            self.observer.computed,
            self.observer.row_hours,
            self.prepared.work(),
        );
        let changes =
            self.tracker
                .observe_updates(decoded, std::sync::Arc::clone(wire), replay.format);
        self.prepared
            .observe(decoded, replay, &mut self.observer, naming, &changes)
            .unwrap();
        let work = self.prepared.work();
        (
            self.observer.computed - before.0,
            self.observer.row_hours - before.1,
            [
                work.row_visits - before.2.row_visits,
                work.history_visits - before.2.history_visits,
                work.emitted - before.2.emitted,
            ],
        )
    }
}

fn rows<T: replay_data::rows::TableSchema>(frame: &mut DecodedFrame) -> &mut Vec<T::Row> {
    match &mut *frame.tables[T::ID.index()] {
        TableView::Complete(rows) => rows.rows_mut::<T>().expect("fixture rows"),
        _ => unreachable!("fixture tables are complete"),
    }
}

fn site(base: u64, access: u8) -> GameScoreAirSite {
    GameScoreAirSite {
        base,
        access,
        error: Vec::new(),
    }
}

fn wing(id: u64, planes: i32) -> AirWingGameScoreStatsRow {
    AirWingGameScoreStatsRow {
        id,
        planes: Some(planes),
        stat_raw: Some(2 * Q + 7),
        mission_index: Some(0),
        ..Default::default()
    }
}

fn registration(base: u64, slot: u32, wings: Vec<u64>) -> AirBaseRegistrationRow {
    AirBaseRegistrationRow {
        base,
        slot,
        capacity_raw: Q,
        wings,
        ..Default::default()
    }
}

/// `count` countries: every tenth flies one wing from its own base, and the
/// first five share a faction.
fn row_score_frame(count: u32) -> DecodedFrame {
    let mut frame = scale_score_frame(count);
    let air: Vec<u32> = (0..count).step_by(10).collect();
    for &index in &air {
        let row = &mut rows::<tables::CountryGameScoreInputs>(&mut frame)[index as usize];
        row.air_state = CountryGameScoreInputRow::AIR_PRESENT;
        row.first_sites = vec![site(u64::from(index) + 500, GameScoreAirSite::ACCESS_TRUE)];
    }
    set(
        &mut frame,
        TableRows::AirBaseRegistrations(
            air.iter()
                .map(|&index| {
                    registration(
                        u64::from(index) + 500,
                        index + 1,
                        vec![u64::from(index) + 7000],
                    )
                })
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableRows::AirWingGameScoreStats(
            air.iter()
                .map(|&index| wing(u64::from(index) + 7000, 10))
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableRows::Factions(vec![FactionRow {
            id: 1,
            name: b"Allies".to_vec(),
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::FactionMembers(
            (0..count.min(5))
                .map(|index| FactionMemberRow {
                    country: 100_000 + index,
                    faction: 1,
                    ordinal: index,
                })
                .collect(),
        ),
    );
    frame
}

fn assert_one_country_hour_observes_only_it(count: u32) {
    let mut input = row_score_frame(count);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![input.clone()])]).unwrap();
    let naming = prepared_naming();
    let mut lane = Lane::new(&naming, &replay);
    let mut recorded = Recorded::new();
    let (computed, _, work) = lane.observe(&recorded.next(&input, false), &replay, &naming);
    assert_eq!(
        computed, count as usize,
        "the first hour reads every country"
    );
    assert_eq!(work[1], count as usize + 1, "with the multipliers");
    let mut next = |change: &dyn Fn(&mut DecodedFrame)| {
        input.hour += 1;
        change(&mut input);
        lane.observe(&recorded.next(&input, false), &replay, &naming)
    };
    let index = |part: u32| (count / part) as usize;
    // One country computed, read from changed rows, and observed alone.
    let one = (1, 1, [1, 1, 7]);
    assert_eq!(
        next(&|frame| {
            rows::<tables::CountryGameScoreInputs>(frame)[index(2)].army_count = Some(40);
        }),
        one,
        "an army change observes only its country"
    );
    assert_eq!(
        next(&|frame| rows::<tables::AirWingGameScoreStats>(frame)[1].planes = Some(11)),
        one,
        "a wing's planes observe only the country flying it"
    );
    assert_eq!(
        next(&|frame| {
            rows::<tables::CountryFactories>(frame)[index(3)].military_used += 1;
            rows::<tables::CountryGameScoreInputs>(frame)[index(3)].fleets[0].fleet += 1;
            rows::<tables::AirWingGameScoreStats>(frame)[0].mission_index = Some(1);
            rows::<tables::Politics>(frame)[index(3)].last_election += 1;
            rows::<tables::Factions>(frame)[0].color_red += 1;
        }),
        (0, 1, [0, 0, 0]),
        "fields no score part reads observe no country"
    );
    assert_eq!(
        next(&|frame| {
            rows::<tables::CountryAppearance>(frame)[index(4)].red = 0.5_f32.to_bits();
        }),
        one,
        "a colour change observes only its country"
    );
    assert_eq!(
        next(&|frame| {
            let hour = i64::from(frame.hour);
            for row in rows::<tables::Countries>(frame) {
                row.political_power_raw = Some(hour);
            }
        }),
        (0, 1, [0, 0, 0]),
        "every country's balance changing observes none"
    );
    assert_eq!(
        next(&|frame| {
            let members = rows::<tables::FactionMembers>(frame);
            members.push(FactionMemberRow {
                country: 100_007,
                faction: 1,
                ordinal: 5,
            });
            members.sort_by_key(|row| row.country);
        }),
        one,
        "joining a faction observes only the joining country"
    );
    assert_eq!(
        next(&|frame| {
            rows::<tables::CountryAppearance>(frame)[index(5)].cosmetic_tag = b"COS".to_vec();
        }),
        (1, 1, [0, 0, 0]),
        "a naming change names only its country again, unchanged without localisation"
    );
}

#[test]
fn prepared_game_score_one_changed_country_observes_only_that_country() {
    assert_one_country_hour_observes_only_it(200);
}

#[test]
#[cfg(feature = "slow-tests")]
fn prepared_game_score_one_changed_country_observes_only_it_at_five_thousand_countries() {
    assert_one_country_hour_observes_only_it(5000);
}

/// Six countries: a subject named after its overlord, two air forces with
/// an inaccessible base, and a faction one member joins through an alias link.
fn parity_frame() -> DecodedFrame {
    let mut frame = frame(100);
    let tags: [&[u8]; 6] = [b"AAA", b"BBB", b"CCC", b"DDD", b"EEE", b"FFF"];
    set(
        &mut frame,
        TableRows::Countries(
            (1..=6)
                .map(|slot| CountryRow {
                    id: 1000 + slot,
                    canonical_slot: slot,
                    tag: tags[slot as usize - 1].to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let mut links: Vec<_> = (1..=6)
        .map(|slot| CountryLinkRow {
            id: 1000 + slot,
            canonical_slot: slot,
        })
        .collect();
    links.push(CountryLinkRow {
        id: 2006,
        canonical_slot: 6,
    });
    set(&mut frame, TableRows::CountryLinks(links));
    set(
        &mut frame,
        TableRows::CountryGameScoreInputs((1..=6).map(|slot| input(1000 + slot)).collect()),
    );
    set(
        &mut frame,
        TableRows::CountryFactories((1..=6).map(|slot| factories(1000 + slot)).collect()),
    );
    let mut appearances: Vec<_> = (1..=6).map(|slot| appearance(1000 + slot, b"")).collect();
    appearances[2].overlord = 1001;
    set(&mut frame, TableRows::CountryAppearance(appearances));
    set(
        &mut frame,
        TableRows::Politics(
            (1..=6)
                .map(|slot| replay_data::rows::PoliticsRow {
                    country: 1000 + slot,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(&mut frame, TableRows::PoliticalParties(Vec::new()));
    set(
        &mut frame,
        TableRows::Autonomy(vec![replay_data::rows::AutonomyRow {
            subject: 1003,
            state: b"autonomy_puppet".to_vec(),
            ..Default::default()
        }]),
    );
    let inputs = rows::<tables::CountryGameScoreInputs>(&mut frame);
    inputs[3].air_state = CountryGameScoreInputRow::AIR_PRESENT;
    inputs[3].first_sites = vec![
        site(900, GameScoreAirSite::ACCESS_TRUE),
        site(901, GameScoreAirSite::ACCESS_FALSE),
    ];
    inputs[4].air_state = CountryGameScoreInputRow::AIR_PRESENT;
    inputs[4].first_sites = vec![site(902, GameScoreAirSite::ACCESS_TRUE)];
    set(
        &mut frame,
        TableRows::AirBaseRegistrations(vec![
            registration(900, 4, vec![70, 71]),
            registration(901, 4, vec![72]),
            registration(902, 5, vec![73]),
        ]),
    );
    set(
        &mut frame,
        TableRows::AirWingGameScoreStats(vec![wing(70, 5), wing(71, 6), wing(72, 7), wing(73, 8)]),
    );
    set(
        &mut frame,
        TableRows::Factions(vec![
            FactionRow {
                id: 10,
                name: b"Axis".to_vec(),
                ..Default::default()
            },
            FactionRow {
                id: 11,
                name: b"Allies".to_vec(),
                ..Default::default()
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 1001,
                faction: 10,
                ordinal: 0,
            },
            FactionMemberRow {
                country: 1003,
                faction: 10,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 2006,
                faction: 11,
                ordinal: 0,
            },
        ]),
    );
    frame
}

#[test]
fn prepared_game_score_changed_rows_match_whole_tables_and_history_across_keyframe_gap_and_resume()
{
    use dashboard_base::prepared::series::tests::{Fixture, assert_payloads_equal};
    let mut localisation: Localisation = [
        ("AAA", "Alpha"),
        ("AAA_ADJ", "Alphan"),
        ("COSA", "Cosmetic Alpha"),
        ("COSA_ADJ", "Cosmetic Alphan"),
        ("BBB", "Beta"),
        ("BBB_neutrality", "Neutral Beta"),
        ("CCC_AAA_autonomy_puppet", "$OVERLORDADJ$ Gamma"),
        ("DDD", "Delta"),
        ("EEE", "Epsilon"),
        ("FFF", "Phi"),
        ("GGG", "Gimel"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    localisation.language = "english".into();
    let mut hourly = parity_frame();
    let mut inputs = Vec::new();
    for hour in 100..=126 {
        hourly.hour = hour;
        let frame = &mut hourly;
        match hour {
            101 => rows::<tables::AirWingGameScoreStats>(frame)[0].planes = Some(10),
            102 => rows::<tables::CountryGameScoreInputs>(frame)[0].army_count = Some(9),
            // Fields no score part reads.
            103 => {
                rows::<tables::CountryFactories>(frame)[1].military_used += 1;
                rows::<tables::Politics>(frame)[2].last_election += 1;
                rows::<tables::AirWingGameScoreStats>(frame)[0].mission_index = Some(1);
            }
            // The overlord's adjective names its subject.
            104 => rows::<tables::CountryAppearance>(frame)[0].cosmetic_tag = b"COSA".to_vec(),
            105 => {
                rows::<tables::Politics>(frame)[1].ruling_party = Some(5);
                set(
                    frame,
                    TableRows::PoliticalParties(vec![replay_data::rows::PoliticalPartyRow {
                        country: 1002,
                        id: 5,
                        token: b"neutrality".to_vec(),
                        ..Default::default()
                    }]),
                );
            }
            // A wing moves base and is reinforced in the same hour.
            106 => {
                let registrations = rows::<tables::AirBaseRegistrations>(frame);
                registrations[0].wings = vec![70];
                registrations[2].wings = vec![71, 73];
                rows::<tables::AirWingGameScoreStats>(frame)[1].planes = Some(7);
            }
            // A holder's total becomes unknown, then known again.
            107 => {
                let row = &mut rows::<tables::CountryGameScoreInputs>(frame)[4];
                row.army_count = None;
                row.army_error = b"fixture unread army".to_vec();
            }
            108 => {
                let row = &mut rows::<tables::CountryGameScoreInputs>(frame)[4];
                row.army_count = Some(4);
                row.army_error = Vec::new();
            }
            109 => rows::<tables::CountryGameScoreInputs>(frame)[3].army_count = Some(12),
            110 => rows::<tables::Factions>(frame)[0].name = b"Axis Powers".to_vec(),
            111 => rows::<tables::FactionMembers>(frame).retain(|row| row.country != 2006),
            // An input row no link names, then gone, beside a colour change.
            112 => rows::<tables::CountryGameScoreInputs>(frame).push(input(3000)),
            113 => {
                rows::<tables::CountryGameScoreInputs>(frame).retain(|row| row.country != 3000);
                rows::<tables::CountryAppearance>(frame)[1].green = 0.125_f32.to_bits();
            }
            114 => continue,
            115 => rows::<tables::CountryGameScoreInputs>(frame)[2].land_count = Some(20),
            // A wing at an inaccessible base.
            116 => rows::<tables::AirWingGameScoreStats>(frame)[2].planes = Some(10),
            117 => {
                let hour = i64::from(hour);
                for row in rows::<tables::Countries>(frame) {
                    row.political_power_raw = Some(hour);
                }
            }
            118 => rows::<tables::CountryGameScoreInputs>(frame)[0].army_count = Some(10),
            // An unresolved membership leaves every non-member unknown.
            119 => rows::<tables::FactionMembers>(frame).push(FactionMemberRow {
                country: 4000,
                faction: 11,
                ordinal: 1,
            }),
            120 => rows::<tables::FactionMembers>(frame).retain(|row| row.country != 4000),
            121 => {
                rows::<tables::Countries>(frame).push(CountryRow {
                    id: 1007,
                    canonical_slot: 7,
                    tag: b"GGG".to_vec(),
                    ..Default::default()
                });
                let links = rows::<tables::CountryLinks>(frame);
                links.push(CountryLinkRow {
                    id: 1007,
                    canonical_slot: 7,
                });
                links.sort_by_key(|row| row.id);
                rows::<tables::CountryGameScoreInputs>(frame).push(input(1007));
                rows::<tables::CountryFactories>(frame).push(factories(1007));
                rows::<tables::CountryAppearance>(frame).push(appearance(1007, b""));
                rows::<tables::Politics>(frame).push(replay_data::rows::PoliticsRow {
                    country: 1007,
                    ..Default::default()
                });
            }
            122 => rows::<tables::CountryGameScoreInputs>(frame)[6].army_count = Some(1),
            123 => rows::<tables::GameScoreMultipliers>(frame)[0].army_raw = Some(2 * Q),
            125 => {
                rows::<tables::CountryGameScoreInputs>(frame)[3].first_sites[1].access =
                    GameScoreAirSite::ACCESS_TRUE;
            }
            126 => rows::<tables::CountryLinks>(frame).retain(|row| row.id != 2006),
            _ => {}
        }
        inputs.push((hourly.clone(), hour == 109));
    }
    let frames: Vec<_> = inputs.iter().map(|(frame, _)| frame.clone()).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let naming = Naming {
        localisation: Ok(std::sync::Arc::new(localisation.clone())),
        templates: Err("Faction content unavailable".into()),
        fallback: prepared_naming().fallback,
        warnings: Vec::new(),
    };
    let fixtures = [Fixture::new(), Fixture::new()];
    // Observed from changed rows, and from the whole tables every hour.
    let mut lanes = [Lane::new(&naming, &replay), Lane::new(&naming, &replay)];
    lanes[1].observer.whole_tables = true;
    let mut recorded = Recorded::new();
    let mut row_hours = 0;
    for (index, (input, keyframe)) in inputs.iter().enumerate() {
        let hour = input.hour;
        let read = recorded.next(input, *keyframe);
        for lane in &mut lanes {
            lane.observe(&read, &replay, &naming);
        }
        if ![108, 117, 126].contains(&hour) {
            continue;
        }
        for (lane, fixture) in lanes.iter_mut().zip(&fixtures) {
            let mut writer = fixture.writer();
            lane.prepared
                .flush(&mut writer, 1, &AtomicBool::new(false))
                .unwrap();
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &fixtures[0].payloads(),
            &fixtures[1].payloads(),
            &format!("changed rows vs whole tables through hour {hour}"),
        );
        let saved = lanes
            .each_ref()
            .map(|lane| serde_json::to_vec(&lane.prepared).unwrap());
        assert!(
            saved[0] == saved[1],
            "continuations differ through hour {hour}"
        );
        let reference = scan_frames(frames[..=index].to_vec(), Some(&localisation));
        for read in 100..=hour {
            assert_prepared_score_at(&fixtures[0], &reference, read);
        }
        // Resume both from what they published.
        row_hours += lanes[0].observer.row_hours;
        for (lane, saved) in lanes.iter_mut().zip(&saved) {
            let whole_tables = lane.observer.whole_tables;
            lane.prepared = serde_json::from_slice(saved).unwrap();
            lane.observer = naming.observer(&replay);
            lane.observer.whole_tables = whole_tables;
            lane.tracker.invalidate();
        }
    }
    assert!(
        row_hours >= 15,
        "most hours are read from their changed rows, not {row_hours}"
    );
    assert_eq!(lanes[1].observer.row_hours, 0);
}

#[test]
fn game_score_change_masks_name_only_recorded_fields() {
    // A mask naming a field its table lacks reads every field.
    for mask in [
        *HOLDER_FIELDS,
        *INPUT_FIELDS,
        *FACTORY_FIELDS,
        *WING_FIELDS,
        *REGISTRATION_FIELDS,
        *COLOUR_FIELDS,
        *APPEARANCE_NAME_FIELDS,
        *RULING_PARTY_FIELDS,
        *PARTY_TOKEN_FIELDS,
        *AUTONOMY_FIELDS,
        *FACTION_NAME_FIELDS,
    ] {
        assert_ne!(mask, u64::MAX);
    }
}
