use super::*;

fn scan(replay: &Replay, branch: u64, tag: &str, cancelled: &AtomicBool) -> io::Result<Scanned> {
    super::super::scan(
        replay,
        branch,
        tag,
        LandSource::Destruction,
        None,
        cancelled,
    )
}
use replay_data::{rows::EquipmentDestructionCause as NativeCause, rows::EquipmentDestructionRow};

fn removal(
    country: u32,
    cause: NativeCause,
    variant: u32,
    amount_raw: i64,
) -> EquipmentDestructionRow {
    EquipmentDestructionRow {
        country,
        cause,
        variant: u64::from(variant) << 32 | u64::from(VARIANT_TYPE),
        amount_raw,
        ..Default::default()
    }
}

fn drain(frame: &mut DecodedFrame, mut rows: Vec<EquipmentDestructionRow>) {
    rows.sort_by_key(|row| (row.country, row.cause as u8, row.variant));
    *frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(rows));
}

fn first(hour: i32) -> DecodedFrame {
    let mut frame = frame(hour, vec![], &[]);
    *frame.tables[TableId::EquipmentDestruction.index()] = TableView::Unavailable {
        reason: EQUIPMENT_DESTRUCTION_PARTIAL_REASON.into(),
    };
    frame
}

fn destruction_params() -> LossesParams {
    LossesParams {
        land_source: LandSource::Destruction,
        ..params()
    }
}

#[test]
fn native_loss_selection_joins_renames_without_a_reused_tag_and_keeps_gap_uncertainty() {
    let base = 60_827_880;
    let country = LossCountry {
        slot: 7,
        country_id: GER,
        origin_hour: base + 1,
    };
    let mut original = frame(base + 1, vec![], &[]);
    drain(
        &mut original,
        vec![removal(GER, NativeCause::LandCombat, 1326, 100_001)],
    );
    let mut renamed = frame(base + 2, vec![], &[]);
    *renamed.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: GER,
            canonical_slot: 7,
            tag: b"renamed".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: SOV,
            canonical_slot: 8,
            tag: b"GER".to_vec(),
            ..Default::default()
        },
    ]));
    drain(
        &mut renamed,
        vec![
            removal(GER_ALIAS, NativeCause::LandCombat, 1326, -50_001),
            removal(SOV, NativeCause::LandCombat, 1326, 90_000_000),
        ],
    );
    let mut unread = renamed.clone();
    unread.hour = base + 3;
    *unread.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "identity gap".into(),
    };
    // An empty complete drain proves zero even without country identities.
    drain(
        &mut unread,
        vec![removal(GER, NativeCause::LandCombat, 1326, 12_345)],
    );
    let mut resumed = renamed.clone();
    resumed.hour = base + 4;
    drain(
        &mut resumed,
        vec![removal(GER, NativeCause::LandCombat, 1326, 25_000)],
    );
    let countries = resumed.complete::<tables::Countries>().unwrap();
    let links = resumed.complete::<tables::CountryLinks>().unwrap();
    assert_eq!(
        selected_country(countries, links, "GER", Some(country))
            .found()
            .unwrap()
            .0
            .id,
        GER
    );
    assert!(matches!(
        selected_country(
            countries,
            links,
            "GER",
            Some(LossCountry {
                country_id: SOV,
                ..country
            })
        ),
        TaggedCountry::Absent
    ));
    let replay = replay_of(vec![first(base), original, renamed, unread, resumed]);
    let scanned = super::super::scan(
        &replay,
        1,
        "GER",
        LandSource::Destruction,
        Some(country),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = reply(
        &LossesParams {
            country: Some(country),
            ..destruction_params()
        },
        &scanned,
        Content::default(),
        vec![],
    );
    let source = LossSource::Destruction(NativeCause::LandCombat);
    assert_eq!(
        totals(&reply, source),
        [
            (base + 1, "100001".into()),
            (base + 2, "50000".into()),
            (base + 4, "75000".into()),
        ]
    );
    assert_eq!(
        cause(&reply, source).leaves[0].lower_bound_from,
        Some(base + 3)
    );
    assert!(
        scanned
            .destruction_unknown
            .iter()
            .any(|span| span.reason == UnknownReason::DestructionUnresolved
                && span.first_hour == base + 3)
    );
    assert_eq!(scanned.destruction_counted_from, Some(base + 1));
}

#[test]
fn destruction_totals_are_read_dated_and_equal_hourly_drains_count_again() {
    let base = 60_827_880;
    let mut one = frame(base + 1, vec![], &[]);
    drain(
        &mut one,
        vec![
            removal(GER_ALIAS, NativeCause::LandCombat, 1326, 100_001),
            removal(GER, NativeCause::LandCombat, 1326, 50_000),
            removal(SOV, NativeCause::LandCombat, 1326, 90_000_000),
        ],
    );
    let mut equal = one.clone();
    equal.hour += 1;
    let mut empty = frame(base + 3, vec![], &[]);
    drain(&mut empty, vec![]);
    let replay = replay_of(vec![first(base), one, equal, empty]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    let source = LossSource::Destruction(EquipmentDestructionCause::LandCombat);
    assert_eq!(
        totals(&reply, source),
        [(base + 1, "150001".into()), (base + 2, "300002".into()),]
    );
    assert_eq!(reply.destruction_coverage.len(), 1);
    assert_eq!(reply.destruction_counted_from, Some(base + 1));
    assert_eq!(
        reply.destruction_coverage[0],
        DestructionCoverageSpan {
            first_hour: base + 1,
            last_hour: base + 3,
            reason: None,
        }
    );
    assert!(reply.coverage.is_empty());
    assert_eq!(reply.retained_months, None);
    assert!(reply.land_lower_bound.is_none());
    assert_eq!(cause(&reply, source).leaves[0].lower_bound_from, None);
}

#[test]
fn partial_first_interval_precedes_counting() {
    let base = 60_827_880;
    let scanned = scan(
        &replay_of(vec![first(base)]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    assert_eq!(reply.destruction_counted_from, None);
    assert!(reply.destruction_coverage.is_empty());
    for source in EquipmentDestructionCause::ALL {
        let source = cause(&reply, LossSource::Destruction(source));
        assert!(source.leaves.is_empty());
        assert!(source.unknown_spans.is_empty());
    }
}

#[test]
fn first_complete_drain_after_startup_proves_every_hooked_cause_zero() {
    let base = 60_827_880;
    let mut complete = frame(base, vec![], &[]);
    drain(&mut complete, vec![]);
    let scanned = scan(
        &replay_of(vec![first(base - 1), complete]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    assert_eq!(reply.destruction_counted_from, Some(base));
    assert!(
        reply
            .causes
            .iter()
            .filter(|cause| matches!(cause.source, LossSource::Destruction(_)))
            .all(|cause| cause.leaves.is_empty() && cause.unknown_spans.is_empty())
    );
}

#[test]
fn complete_empty_drain_needs_no_country_links_to_prove_zero() {
    let base = 60_827_880;
    let mut complete = frame(base, vec![], &[]);
    drain(&mut complete, vec![]);
    *complete.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "unread links".into(),
    };
    let scanned = scan(
        &replay_of(vec![complete]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.destruction_counted_from, Some(base));
    assert!(scanned.destruction_unknown.is_empty());
    assert_eq!(scanned.destruction_coverage[0].reason, None);
}

#[test]
fn withheld_destruction_keeps_its_reason_for_the_rest_of_the_branch() {
    let base = 60_827_880;
    let mut recorded = frame(base + 1, vec![], &[]);
    drain(
        &mut recorded,
        vec![removal(GER, NativeCause::AirAccident, 1326, AMOUNT_SCALE)],
    );
    let mut overflow = frame(base + 2, vec![], &[]);
    *overflow.tables[TableId::EquipmentDestruction.index()] = TableView::Unavailable {
        reason: "equipment destruction event log overflow".into(),
    };
    let mut later = recorded.clone();
    later.hour = base + 3;
    let scanned = scan(
        &replay_of(vec![first(base), recorded, overflow, later]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    let source = LossSource::Destruction(EquipmentDestructionCause::AirAccident);
    assert_eq!(totals(&reply, source), [(base + 1, raw(1))]);
    let span = cause(&reply, source)
        .unknown_spans
        .iter()
        .find(|span| span.reason == UnknownReason::DestructionWithheld)
        .unwrap();
    assert_eq!((span.first_hour, span.last_hour), (base + 2, base + 3));
    assert_eq!(
        span.detail.as_deref(),
        Some("equipment destruction event log overflow")
    );
    assert_eq!(
        reply.destruction_coverage.last().unwrap().last_hour,
        base + 3
    );
    assert_eq!(
        reply.destruction_coverage.last().unwrap().reason,
        span.detail
    );
}

#[test]
fn a_truncated_first_drain_keeps_its_reason_and_withholds_the_branch() {
    let base = 60_827_880;
    let mut truncated = first(base);
    *truncated.tables[TableId::EquipmentDestruction.index()] = TableView::Truncated {
        rows: TableRows::EquipmentDestruction(vec![removal(
            GER,
            NativeCause::Attrition,
            1326,
            AMOUNT_SCALE,
        )]),
        missing: 2,
    };
    let mut later = frame(base + 1, vec![], &[]);
    drain(
        &mut later,
        vec![removal(GER, NativeCause::Attrition, 1326, 9 * AMOUNT_SCALE)],
    );
    let scanned = scan(
        &replay_of(vec![truncated, later]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    let source = cause(
        &reply,
        LossSource::Destruction(EquipmentDestructionCause::Attrition),
    );
    assert!(source.leaves.is_empty());
    assert!(source.unknown_spans.iter().any(|span| span.reason
        == UnknownReason::DestructionWithheld
        && span.first_hour == base
        && span.last_hour == base + 1
        && span.detail.as_deref() == Some(DESTRUCTION_TRUNCATED)));
    assert!(
        reply.destruction_coverage.iter().all(|span| span
            .reason
            .as_ref()
            .unwrap()
            .contains("truncated"))
    );
}

#[test]
fn unknown_caller_is_its_own_cause_and_full_design_handles_are_validated() {
    let base = 60_827_880;
    let mut recorded = frame(base + 1, vec![], &[]);
    drain(
        &mut recorded,
        vec![removal(GER, NativeCause::Unknown, 0xf001, 7)],
    );
    let scanned = scan(
        &replay_of(vec![first(base), recorded.clone()]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    let source = LossSource::Destruction(EquipmentDestructionCause::Unknown);
    assert_eq!(totals(&reply, source), [(base + 1, "7".into())]);
    assert_eq!(
        cause(&reply, source).name.text.as_deref(),
        Some("Unrecognised destruction caller")
    );
    assert_eq!(reply.designs[0].variant, 0xf001);
    let TableView::Complete(TableRows::EquipmentDestruction(rows)) =
        &mut *recorded.tables[TableId::EquipmentDestruction.index()]
    else {
        unreachable!()
    };
    rows[0].variant += 1;
    assert!(
        scan(
            &replay_of(vec![first(base), recorded]),
            1,
            "GER",
            &AtomicBool::new(false)
        )
        .unwrap_err()
        .to_string()
        .contains("unsupported reference type")
    );
}

#[test]
fn unknown_callers_keep_proven_land_and_air_and_refuse_unsupported_domains() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!(
        "hoi4_losses_unknown_domain_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let equipment = root.join("common/units/equipment");
    std::fs::create_dir_all(&equipment).unwrap();
    std::fs::write(equipment.join("synthetic.txt"),
        "equipments = { mod_rifle = { is_archetype = yes type = infantry } mod_rifle_1 = { archetype = mod_rifle } mod_air = { is_archetype = yes type = fighter } mod_air_1 = { archetype = mod_air } mod_hull = { is_archetype = yes type = screen_ship } mod_hull_1 = { archetype = mod_hull } }").unwrap();
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let types = installed_content_core::equipment::Database::load_complete(&layers).unwrap();
    assert_eq!(archetype_of(&types, "mod_air_1").unwrap().2, Domain::Air);
    let base = 60_827_880;
    let mut first = first(base);
    *first.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            (1326..=1329)
                .map(|variant| VariantDefinitionRow {
                    id: variant << 32 | u64::from(VARIANT_TYPE),
                    equipment_token: (variant - 1321) as u32,
                    ..Default::default()
                })
                .collect(),
        ));
    *first.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (5, "mod_rifle_1"),
            (6, "mod_air_1"),
            (7, "not_authored"),
            (8, "mod_hull_1"),
        ]
        .into_iter()
        .map(|(token, literal)| replay_data::rows::LexicalTokenRow {
            token,
            literal: literal.as_bytes().to_vec(),
        })
        .collect(),
    ));
    let mut recorded = first.clone();
    recorded.hour += 1;
    drain(
        &mut recorded,
        vec![
            removal(GER, NativeCause::Unknown, 1326, 3 * AMOUNT_SCALE),
            removal(GER, NativeCause::Unknown, 1327, 5 * AMOUNT_SCALE),
            removal(GER, NativeCause::Unknown, 1328, 7 * AMOUNT_SCALE),
            removal(GER, NativeCause::AirAccident, 1327, AMOUNT_SCALE),
            removal(
                GER,
                NativeCause::ScriptedStockpileFraction,
                1326,
                3 * AMOUNT_SCALE,
            ),
            removal(
                GER,
                NativeCause::ScriptedStockpileFraction,
                1327,
                -5 * AMOUNT_SCALE,
            ),
            removal(
                GER,
                NativeCause::ScriptedStockpileFraction,
                1328,
                7 * AMOUNT_SCALE,
            ),
            removal(
                GER,
                NativeCause::ScriptedStockpileFraction,
                1329,
                2 * AMOUNT_SCALE,
            ),
        ],
    );
    let scanned = scan(
        &replay_of(vec![first, recorded]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let content = Content {
        archetypes: Some(&types),
        ..Content::default()
    };
    let combat = reply(&params(), &scanned, content, vec![]);
    let source = LossSource::Destruction(EquipmentDestructionCause::Unknown);
    assert_eq!(
        totals(&combat, source),
        [(base + 1, raw(3)), (base + 1, raw(5))]
    );
    assert_eq!(cause(&combat, source).leaves.len(), 2);
    assert_eq!(
        cause(&combat, source)
            .leaves
            .iter()
            .filter_map(|leaf| leaf.points.last())
            .map(|point| point.total_raw.parse::<i128>().unwrap())
            .sum::<i128>(),
        8 * i128::from(AMOUNT_SCALE)
    );
    let unknown = cause(&combat, source);
    assert_eq!(
        combat.designs[unknown.leaves[0].design.unwrap() as usize].variant,
        1326
    );
    assert!(unknown.unknown_spans.iter().any(|span| {
        span.reason == UnknownReason::DestructionDomain
            && span
                .detail
                .as_ref()
                .unwrap()
                .contains("does not resolve this destruction equipment's domain")
    }));
    assert_eq!(unknown.leaves[0].lower_bound_from, Some(base + 1));
    assert!(
        !cause(
            &combat,
            LossSource::Destruction(EquipmentDestructionCause::AirAccident)
        )
        .unknown_spans
        .iter()
        .any(|span| span.reason == UnknownReason::DestructionDomain)
    );
    let destruction = reply(&destruction_params(), &scanned, content, vec![]);
    let unknown = cause(&destruction, source);
    assert_eq!(unknown.leaves.len(), 3);
    assert!(
        !unknown
            .unknown_spans
            .iter()
            .any(|span| span.reason == UnknownReason::DestructionDomain)
    );
    let stockpile = LossSource::Destruction(NativeCause::ScriptedStockpileFraction);
    // The alternate land source omits known land stockpile writers, keeps exact
    // air/naval corrections and exposes only this mixed cause's unresolved arm.
    assert_eq!(
        totals(&combat, stockpile),
        [(base + 1, raw(-5)), (base + 1, raw(2))]
    );
    assert_eq!(cause(&combat, stockpile).leaves.len(), 2);
    assert!(
        cause(&combat, stockpile)
            .unknown_spans
            .iter()
            .any(|span| span.reason == UnknownReason::DestructionDomain)
    );
    assert_eq!(cause(&destruction, stockpile).leaves.len(), 4);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn air_sources_are_the_same_in_both_land_modes_and_non_losses_stay_excluded() {
    let base = 60_827_880;
    let mut recorded = frame(
        base + 1,
        vec![
            log(GER, 1, 0, OCTOBER, 8),
            log(
                GER,
                1,
                engine_land_inputs::ncombat_log_cmanager::RECOVERED_KIND,
                OCTOBER,
                90,
            ),
        ],
        &[GER],
    );
    lend_lease(
        &mut recorded,
        vec![exchange(
            7,
            engine_resources::clend_lease_exchange::DELIVERED_KIND,
            1326,
            80,
        )],
    );
    let mut rows = vec![removal(GER, NativeCause::LandCombat, 1326, AMOUNT_SCALE)];
    rows.extend(
        EquipmentDestructionCause::ALL
            .into_iter()
            .filter(|cause| cause.is_air())
            .map(|cause| {
                removal(
                    GER,
                    NativeCause::try_from(cause as u8).unwrap(),
                    1326,
                    AMOUNT_SCALE,
                )
            }),
    );
    drain(&mut recorded, rows);
    let replay = replay_of(vec![first(base), recorded]);
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let destruction = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    let combat_scan = super::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let combat = reply(
        &params(),
        &combat_scan,
        Content {
            retained_months: Some(12),
            ..Content::default()
        },
        vec![],
    );
    for cause in EquipmentDestructionCause::ALL
        .into_iter()
        .filter(|cause| cause.is_air())
    {
        let source = LossSource::Destruction(cause);
        assert_eq!(totals(&destruction, source), [(base + 1, raw(1))]);
        assert_eq!(
            super::cause(&destruction, source),
            super::cause(&combat, source)
        );
    }
    assert_eq!(
        totals(&combat, LossSource::CombatLog(0)),
        [(base + 1, raw(8))]
    );
    assert!(combat.land_lower_bound.is_none());
    assert!(!combat.coverage.is_empty());
    assert!(!destruction.causes.iter().any(|cause| matches!(
        cause.source,
        LossSource::CombatLog(_) | LossSource::Recovered
    )));
    assert!(!combat.causes.iter().any(
        |cause| cause.source == LossSource::Destruction(EquipmentDestructionCause::LandCombat)
    ));
    for reply in [&destruction, &combat] {
        assert!(!super::cause(reply, LossSource::LendLeaseSent).loss);
        let shown: i128 = reply
            .causes
            .iter()
            .filter(|cause| cause.loss)
            .flat_map(|cause| &cause.leaves)
            .filter_map(|leaf| leaf.points.last())
            .map(|point| point.total_raw.parse::<i128>().unwrap())
            .sum();
        assert_eq!(
            shown,
            if reply.land_source == LandSource::Destruction {
                8
            } else {
                15
            } * i128::from(AMOUNT_SCALE)
        );
    }
    assert!(!super::cause(&combat, LossSource::Recovered).loss);
}

#[test]
fn destruction_gaps_and_unresolved_raw_owners_are_named_unknown_spans() {
    let base = 60_827_880;
    let mut complete = frame(base + 1, vec![], &[]);
    drain(&mut complete, vec![]);
    let mut recorded = frame(base + 3, vec![], &[]);
    drain(
        &mut recorded,
        vec![removal(99999, NativeCause::Attrition, 1326, AMOUNT_SCALE)],
    );
    let scanned = scan(
        &replay_of(vec![first(base), complete, recorded]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    let cause = cause(
        &reply,
        LossSource::Destruction(EquipmentDestructionCause::Attrition),
    );
    assert!(cause.leaves.is_empty());
    assert!(cause.unknown_spans.iter().any(|span| span.reason == UnknownReason::DestructionGap && span.first_hour == base + 2));
    assert!(
        cause
            .unknown_spans
            .iter()
            .any(|span| span.reason == UnknownReason::DestructionUnresolved
                && span.detail.as_ref().unwrap().contains("CountryLink"))
    );
}

#[test]
fn land_source_defaults_to_every_destruction_and_invalid_values_are_refused() {
    let params: LossesParams =
        serde_json::from_value(serde_json::json!({"branchId":"1", "tag":"opaque_tag"})).unwrap();
    assert_eq!(params.land_source, LandSource::Destruction);
    assert!(
        serde_json::from_value::<LossesParams>(
            serde_json::json!({"branchId":"1", "tag":"opaque_tag", "landSource":"guessed"})
        )
        .is_err()
    );
    assert_eq!(
        serde_json::to_value(LossSource::Destruction(
            EquipmentDestructionCause::AirMissionAa
        ))
        .unwrap(),
        serde_json::json!({"kind":"destruction", "reason":"air_mission_aa"})
    );
}

#[test]
fn changing_unresolved_owner_counts_merge_one_persistent_reason() {
    let base = 60_827_880;
    let mut frames = vec![first(base)];
    for offset in 1..=64 {
        let mut frame = frame(base + offset, vec![], &[]);
        let mut rows: Vec<_> = (0..offset)
            .map(|raw| removal(100_000 + raw as u32, NativeCause::Attrition, 1326, 1))
            .collect();
        rows.push(removal(GER, NativeCause::AirAccident, 1326, 1));
        drain(&mut frame, rows);
        frames.push(frame);
    }
    let mut recovered = frame(base + 65, vec![], &[]);
    drain(
        &mut recovered,
        vec![removal(GER, NativeCause::AirAccident, 1326, 1)],
    );
    frames.push(recovered);
    let scanned = scan(&replay_of(frames), 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.destruction_unknown.len(), 1);
    assert_eq!(scanned.destruction_coverage.len(), 2);
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    for cause in reply
        .causes
        .iter()
        .filter(|cause| matches!(cause.source, LossSource::Destruction(_)))
    {
        assert_eq!(cause.unknown_spans.len(), 1);
        let span = &cause.unknown_spans[0];
        assert_eq!((span.first_hour, span.last_hour), (base + 1, base + 65));
        assert_eq!(span.reason, UnknownReason::DestructionUnresolved);
        assert!(
            !span
                .detail
                .as_ref()
                .unwrap()
                .contains(|ch: char| ch.is_ascii_digit())
        );
        assert!(
            cause
                .leaves
                .iter()
                .all(|leaf| leaf.lower_bound_from == Some(base + 1))
        );
    }
}

#[test]
fn each_scan_keeps_only_its_land_source_and_shared_destruction() {
    assert!(!selected(LandSource::Destruction).contains(&TableId::CombatLogEquipment));
    assert!(!selected(LandSource::Destruction).contains(&TableId::DivisionCensus));
    assert!(selected(LandSource::CombatLog).contains(&TableId::CombatLogEquipment));
    let base = 60_827_880;
    let mut frame = frame(base, vec![log(GER, 1, 0, OCTOBER, 8)], &[GER]);
    drain(
        &mut frame,
        vec![
            removal(GER, NativeCause::LandCombat, 1326, AMOUNT_SCALE),
            removal(GER, NativeCause::AirAccident, 1326, AMOUNT_SCALE),
            removal(GER, NativeCause::Unknown, 1326, AMOUNT_SCALE),
        ],
    );
    let replay = replay_of(vec![frame]);
    let destruction = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert!(
        !destruction
            .events
            .keys()
            .any(|(source, _)| matches!(source, LossSource::CombatLog(_) | LossSource::Recovered))
    );
    let combat = super::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert!(
        !combat
            .events
            .keys()
            .any(|(source, _)| *source == LossSource::Destruction(NativeCause::LandCombat))
    );
    assert!(
        combat
            .events
            .keys()
            .any(|(source, _)| *source == LossSource::Destruction(NativeCause::Unknown))
    );
    assert_eq!(
        destruction.events[&(
            LossSource::Destruction(NativeCause::AirAccident),
            Some(1326)
        )],
        combat.events[&(
            LossSource::Destruction(NativeCause::AirAccident),
            Some(1326)
        )]
    );
}

#[test]
fn unrequested_destruction_land_is_not_added_to_combat_log() {
    let mut frame = frame(60_827_880, vec![], &[]);
    drain(
        &mut frame,
        vec![removal(GER, NativeCause::LandCombat, 1326, 1)],
    );
    let links = frame.complete::<tables::CountryLinks>().unwrap();
    let mut scanned = Scanned::default();
    let mut state = State {
        land_source: LandSource::CombatLog,
        ..State::default()
    };
    destruction(
        &mut scanned,
        &mut state,
        &frame,
        Some(links),
        tagged_country(frame.complete::<tables::Countries>().unwrap(), links, "GER"),
    )
    .unwrap();
    assert!(scanned.events.is_empty());
}

#[test]
fn unrequested_combat_log_is_not_added_to_destruction() {
    let mut frame = frame(60_827_880, vec![log(GER, 1, 0, OCTOBER, 1)], &[GER]);
    drain(&mut frame, vec![]);
    let mut scanned = Scanned::default();
    let mut state = State {
        land_source: LandSource::Destruction,
        ..State::default()
    };
    observe(&mut scanned, &mut state, &frame, "GER").unwrap();
    assert!(scanned.events.is_empty());
    assert!(scanned.coverage.is_empty());
}

#[test]
fn load_and_discontinuity_branches_each_choose_their_own_counting_start() {
    let base = 60_827_880;
    let mut records = vec![Record::SessionStart(session())];
    for (branch, reason) in [
        (1, BranchReason::SessionStart),
        (2, BranchReason::LoadSignal),
        (3, BranchReason::HourDiscontinuity),
    ] {
        let hour = base + branch as i32 * 10;
        records.push(Record::BranchStart(BranchStart {
            id: branch,
            session: 1,
            loaded_hour: (reason == BranchReason::LoadSignal).then_some(hour),
            first_hour: hour,
            parent: Parentage::Unknown,
            reason,
        }));
        let mut encoder = FrameEncoder::new();
        let mut startup = first(hour);
        startup.branch = branch;
        records.push(Record::Frame(encoder.encode(startup, false).unwrap()));
        let mut complete = frame(hour + 1, vec![], &[]);
        complete.branch = branch;
        drain(
            &mut complete,
            vec![removal(GER, NativeCause::AirAccident, 1326, 1)],
        );
        records.push(Record::Frame(encoder.encode(complete, false).unwrap()));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records]).unwrap();
    for (branch, start) in [(1, base + 11), (2, base + 21), (3, base + 31)] {
        let scanned = scan(&replay, branch, "GER", &AtomicBool::new(false)).unwrap();
        assert_eq!(scanned.destruction_counted_from, Some(start));
        assert!(scanned.destruction_unknown.is_empty());
        assert_eq!(scanned.destruction_coverage[0].first_hour, start);
        assert_eq!(
            scanned.events[&(
                LossSource::Destruction(NativeCause::AirAccident),
                Some(1326)
            )],
            [(start, 1)]
        );
    }
}

fn without_requested_country(frame: &mut DecodedFrame) {
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    rows.retain(|row| row.canonical_slot != 7);
    let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    rows.retain(|row| row.canonical_slot != 7);
}

#[test]
fn countries_appearing_and_disappearing_do_not_create_owner_unknowns() {
    let base = 60_827_880;
    for appears in [true, false] {
        let mut frames = vec![first(base)];
        for offset in 1..=4 {
            let present = (offset >= 3) == appears;
            let mut hour = frame(base + offset, vec![], &[]);
            let mut rows = vec![removal(SOV, NativeCause::AirAccident, 1326, 100)];
            if present {
                rows.push(removal(GER_ALIAS, NativeCause::AirAccident, 1326, 1));
            } else {
                without_requested_country(&mut hour);
                // Even an unresolved raw owner cannot belong to an absent tag.
                rows.push(removal(999_999, NativeCause::Attrition, 1326, 100));
            }
            drain(&mut hour, rows);
            frames.push(hour);
        }
        let replay = replay_of(frames);
        for land_source in [LandSource::Destruction, LandSource::CombatLog] {
            let scanned = super::super::scan(
                &replay,
                1,
                "GER",
                land_source,
                None,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(scanned.destruction_unknown.is_empty());
            assert!(scanned.unresolved.is_empty());
            assert_eq!(scanned.destruction_counted_from, Some(base + 1));
            assert_eq!(scanned.destruction_coverage.len(), 1);
            assert_eq!(scanned.destruction_coverage[0].reason, None);
            let reply = reply(
                &LossesParams {
                    land_source,
                    ..destruction_params()
                },
                &scanned,
                Content {
                    retained_months: Some(12),
                    ..Content::default()
                },
                vec![],
            );
            let source = LossSource::Destruction(NativeCause::AirAccident);
            let from = base + if appears { 3 } else { 1 };
            assert_eq!(
                totals(&reply, source),
                [(from, "1".into()), (from + 1, "2".into())]
            );
            for cause in reply
                .causes
                .iter()
                .filter(|cause| matches!(cause.source, LossSource::Destruction(_)))
            {
                assert!(cause.unknown_spans.is_empty());
                assert!(
                    cause
                        .leaves
                        .iter()
                        .all(|leaf| leaf.lower_bound_from.is_none())
                );
            }
        }
    }
}

#[test]
fn unresolved_tag_and_incomplete_country_tables_still_create_owner_unknowns() {
    let base = 60_827_880;
    for incomplete in [None, Some(TableId::Countries), Some(TableId::CountryLinks)] {
        let mut hour = frame(base + 1, vec![], &[]);
        drain(
            &mut hour,
            vec![removal(SOV, NativeCause::AirAccident, 1326, 100)],
        );
        if let Some(table) = incomplete {
            *hour.tables[table.index()] = TableView::Unavailable {
                reason: "unread".into(),
            };
        } else {
            *hour.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                    id: SOV,
                    canonical_slot: 8,
                }]));
        }
        // Replay admission requires Countries; exercise the observer's
        // incomplete-table fallback directly without inventing a valid replay.
        let mut scanned = Scanned::default();
        let mut state = State::default();
        observe(&mut scanned, &mut state, &first(base), "GER").unwrap();
        observe(&mut scanned, &mut state, &hour, "GER").unwrap();
        assert_eq!(scanned.destruction_unknown.len(), 1);
        assert_eq!(
            scanned.destruction_unknown[0].reason,
            UnknownReason::DestructionUnresolved
        );
    }
}

#[test]
fn distinct_owner_and_gap_details_survive_and_repeated_details_merge() {
    let base = 60_827_880;
    let mut initial = frame(base + 1, vec![], &[]);
    drain(
        &mut initial,
        vec![removal(GER, NativeCause::AirAccident, 1326, 1)],
    );
    let mut no_links = initial.clone();
    no_links.hour = base + 2;
    *no_links.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    let mut no_owner = initial.clone();
    no_owner.hour = base + 3;
    drain(
        &mut no_owner,
        vec![removal(999_999, NativeCause::Attrition, 1326, 1)],
    );
    let mut repeated_owner = no_owner.clone();
    repeated_owner.hour += 1;
    // Hour 5 is an implicit gap, followed by a deliberately partial interval.
    let partial = first(base + 6);
    let repeated_partial = first(base + 7);
    let mut recovered = initial.clone();
    recovered.hour = base + 9;
    let scanned = scan(
        &replay_of(vec![
            first(base),
            initial,
            no_links,
            no_owner,
            repeated_owner,
            partial,
            repeated_partial,
            recovered,
        ]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(scanned.destruction_unknown.len(), 4);
    assert_eq!(scanned.destruction_unknown_index.len(), 4);
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    for cause in reply
        .causes
        .iter()
        .filter(|cause| matches!(cause.source, LossSource::Destruction(_)))
    {
        let spans = &cause.unknown_spans;
        assert_eq!(spans.len(), 4);
        assert!(spans.iter().all(|span| span.last_hour == base + 9));
        for reason in [
            UnknownReason::DestructionUnresolved,
            UnknownReason::DestructionGap,
        ] {
            let details: BTreeSet<_> = spans
                .iter()
                .filter(|span| span.reason == reason)
                .map(|span| span.detail.as_deref())
                .collect();
            assert_eq!(details.len(), 2);
        }
        assert!(
            cause
                .leaves
                .iter()
                .all(|leaf| leaf.lower_bound_from == Some(base + 2))
        );
    }
}

#[test]
fn recording_causes_keep_their_writer_domains() {
    for cause in NativeCause::ALL {
        assert_eq!(
            LossSource::Destruction(cause).domain(),
            if matches!(
                cause,
                NativeCause::Unknown | NativeCause::ScriptedStockpileFraction
            ) {
                LossDomain::Other
            } else if cause.is_air() {
                LossDomain::Air
            } else {
                LossDomain::Land
            }
        );
    }
    assert_eq!(LossSource::SunkShips.domain(), LossDomain::Naval);
    assert_eq!(LossSource::SunkConvoys.domain(), LossDomain::Naval);
    assert_eq!(LossSource::CombatLog(0).domain(), LossDomain::Land);
    assert_eq!(LossSource::LendLeaseSunk.domain(), LossDomain::Other);
}

#[test]
fn every_supplemental_cause_and_native_gain_survives_the_product() {
    let base = 60_827_880;
    let mut hour = frame(base, vec![], &[]);
    let causes: Vec<_> = NativeCause::ALL
        .into_iter()
        .filter(|cause| (*cause as u8) >= NativeCause::DivisionDestroyed as u8)
        .collect();
    drain(
        &mut hour,
        causes
            .iter()
            .map(|&cause| removal(GER_ALIAS, cause, 1326, -100_001))
            .collect(),
    );
    let scanned = scan(&replay_of(vec![hour]), 1, "GER", &AtomicBool::new(false)).unwrap();
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    assert!(reply.land_lower_bound.is_none());
    for cause in causes {
        let source = LossSource::Destruction(cause);
        assert_eq!(totals(&reply, source), [(base, "-100001".into())]);
        assert!(super::cause(&reply, source).name.text.is_some());
        assert!(super::cause(&reply, source).unknown_spans.is_empty());
    }
}

#[test]
fn historical_amounts_remain_visible_without_claiming_supplemental_coverage() {
    let base = 60_827_880;
    let mut one = frame(base, vec![], &[]);
    let mut historical = removal(GER, NativeCause::LandCombat, 1326, 123_456);
    historical.coverage = replay_data::rows::Recorded::NotRecorded;
    historical.unavailable_reason = replay_data::rows::Recorded::NotRecorded;
    drain(&mut one, vec![historical]);
    let mut empty = frame(base + 1, vec![], &[]);
    drain(&mut empty, vec![]);
    let mut scanned = Scanned::default();
    let mut state = State {
        destruction_coverage_not_recorded: true,
        ..Default::default()
    };
    for frame in [one, empty] {
        observe(&mut scanned, &mut state, &frame, "GER").unwrap();
    }
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    let source = LossSource::Destruction(NativeCause::LandCombat);
    assert_eq!(totals(&reply, source), [(base, "123456".into())]);
    assert_eq!(reply.destruction_coverage.len(), 1);
    assert_eq!(reply.destruction_coverage[0].last_hour, base + 1);
    assert_eq!(
        reply.destruction_coverage[0].reason.as_deref(),
        Some("EquipmentDestruction supplemental coverage not recorded")
    );
    assert_eq!(cause(&reply, source).unknown_spans[0].last_hour, base + 1);
}

#[test]
fn supplemental_unavailable_marker_is_not_an_owner_and_keeps_cumulative_uncertainty() {
    use replay_data::rows::EquipmentDestructionCoverage;
    let base = 60_827_880;
    let mut one = frame(base, vec![], &[]);
    drain(
        &mut one,
        vec![EquipmentDestructionRow {
            coverage: EquipmentDestructionCoverage::SupplementalUnavailable.into(),
            unavailable_reason: "Supplemental capture hooks refused".to_owned().into(),
            ..Default::default()
        }],
    );
    let mut two = frame(base + 1, vec![], &[]);
    drain(
        &mut two,
        vec![removal(GER, NativeCause::AirAccident, 1326, -AMOUNT_SCALE)],
    );
    let scanned = scan(
        &replay_of(vec![one, two]),
        1,
        "GER",
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = reply(&destruction_params(), &scanned, Content::default(), vec![]);
    assert!(scanned.unresolved.is_empty());
    assert_eq!(reply.destruction_counted_from, Some(base));
    assert_eq!(
        reply.destruction_coverage[0].reason.as_deref(),
        Some("Supplemental capture hooks refused")
    );
    assert_eq!(reply.destruction_coverage[1].reason, None);
    let air = cause(&reply, LossSource::Destruction(NativeCause::AirAccident));
    assert_eq!(air.leaves[0].points[0].total_raw, "-100000");
    assert_eq!(air.leaves[0].lower_bound_from, Some(base));
    assert_eq!(air.unknown_spans[0].first_hour, base);
    assert_eq!(air.unknown_spans[0].last_hour, base + 1);
    assert_eq!(
        air.unknown_spans[0].reason,
        UnknownReason::DestructionWithheld
    );
    assert!(
        !reply
            .warnings
            .iter()
            .any(|warning| warning.contains("country references"))
    );
}

#[test]
fn supplemental_failure_cannot_add_losses_to_an_absent_country() {
    let base = 60_827_880;
    let mut hour = frame(base, vec![], &[]);
    drain(
        &mut hour,
        vec![EquipmentDestructionRow {
            coverage: replay_data::rows::EquipmentDestructionCoverage::SupplementalUnavailable
                .into(),
            unavailable_reason: "Supplemental capture hooks refused".to_owned().into(),
            ..Default::default()
        }],
    );
    let scanned = scan(&replay_of(vec![hour]), 1, "ABSENT", &AtomicBool::new(false)).unwrap();
    assert!(scanned.destruction_unknown.is_empty());
    assert!(scanned.unresolved.is_empty());
    assert_eq!(scanned.destruction_coverage[0].reason, None);
}
