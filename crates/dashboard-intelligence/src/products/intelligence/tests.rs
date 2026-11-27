use super::{
    decryption::{CipherState, Observation, attempt},
    placements::{CAPTURED, MISSION, OPERATION, target},
};
use replay_data::rows::{OperativeMembershipRow, Recorded};

#[test]
fn shared_defines_give_captivity_its_mod_duration_or_name_the_missing_days() {
    let assignment = |changes| {
        let (_machine, source, load) = dashboard_base::test_fixtures::recorded_mod_defines(
            "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NOperatives={OPERATIVE_CAPTURE_DURATION_IN_DAYS=3}}",
            changes,
            &[("localisation/probe_l_english.yml", "\u{feff}l_english:\n DURATION_DAYS:0 \"$NUM$ ticks\"\n OPERATIVE_CAPTURED_BY:0 \"$CAPTURER$\"\n")],
        );
        let content = super::operative_content::content(&source, &load).unwrap();
        let epoch = game_model::calendar::HOUR_EPOCH;
        let row = OperativeMembershipRow { assignment: CAPTURED, captured_hour: epoch, ..Default::default() };
        content.assignment(&row, epoch + 24, None, Some("Probe"))
    };
    assert_eq!(assignment("NDefines.NOperatives.OPERATIVE_CAPTURE_DURATION_IN_DAYS=29").duration.as_deref(), Some("28 ticks"));
    let refused = assignment("NDefines.NOperatives=nil");
    assert!(refused.duration.is_none() && refused.progress.is_none());
    assert_eq!(refused.missing.as_deref(), Some("NDefines.NOperatives.OPERATIVE_CAPTURE_DURATION_IN_DAYS is unavailable: missing NDefines.NOperatives.OPERATIVE_CAPTURE_DURATION_IN_DAYS"));
}

const fn state(amount: i64, decrypting: bool, bonus_active: bool, broken: Option<bool>) -> Option<CipherState> {
    Some(CipherState { amount, decrypting, bonus_active, broken })
}

const fn at(hour: i32, upper_bound: bool) -> Option<Observation> {
    Some(Observation { hour, upper_bound })
}

#[test]
fn a_cipher_attempt_starts_when_work_starts_and_breaks_when_the_cipher_is_broken() {
    let states = [
        (10, 19, state(0, false, false, Some(false))),
        (20, 29, state(0, true, false, Some(false))),
        (30, 39, state(500, true, false, Some(false))),
        (40, 49, state(900, true, false, Some(true))),
    ];
    assert_eq!(attempt(&states), (at(20, true), at(40, false)));
    assert_eq!(attempt(&[(5, 9, state(900, true, true, Some(true)))]), (at(5, true), at(5, true)));
    assert_eq!(attempt(&[(5, 9, state(0, false, false, Some(false)))]), (None, None));
}

#[test]
fn a_bonus_ending_or_progress_dropping_to_nothing_ends_an_attempt_and_the_next_start_is_exact() {
    let states = [
        (0, 9, state(100, true, false, Some(false))),
        (10, 19, state(1000, false, true, Some(true))),
        (20, 29, state(0, false, false, Some(false))),
        (30, 39, state(0, true, false, Some(false))),
    ];
    assert_eq!(attempt(&states), (at(30, false), None));
    let dropped = [(0, 9, state(300, true, false, Some(false))), (10, 19, state(0, false, false, Some(false)))];
    assert_eq!(attempt(&dropped), (None, None));
}

#[test]
fn hours_not_known_end_continuity_so_the_attempt_after_them_is_dated_by_its_first_hour_known() {
    let states = [
        (0, 9, state(0, true, false, Some(false))),
        (10, 19, None),
        (20, 29, state(400, true, false, None)),
        (30, 39, state(900, true, false, Some(true))),
    ];
    assert_eq!(attempt(&states), (at(20, true), at(30, true)));
    let apart = [(0, 9, state(0, true, false, Some(false))), (15, 19, state(100, true, false, Some(false)))];
    assert_eq!(attempt(&apart), (at(15, true), None));
}

fn member(assignment: u32, kind: u8, variant: Option<u32>, operation: Option<u64>, target: Recorded<Option<u32>>) -> OperativeMembershipRow {
    OperativeMembershipRow {
        id: 7,
        owner_country: 1,
        roster_country: 1,
        roster_kind: kind,
        assignment,
        action_variant: variant,
        operation: Recorded::Value(operation),
        target_country: target,
        ..OperativeMembershipRow::default()
    }
}

#[test]
fn an_operative_is_placed_only_while_free_on_the_active_list_at_a_mission_or_operation_with_a_target_its_own_country_included() {
    assert_eq!(target(&member(MISSION, 0, Some(2), None, Recorded::Value(Some(9)))), Some(9));
    assert_eq!(target(&member(OPERATION, 0, None, Some(55), Recorded::Value(Some(9)))), Some(9));
    assert_eq!(target(&member(MISSION, 0, Some(0), None, Recorded::Value(Some(9)))), None);
    assert_eq!(target(&member(OPERATION, 0, None, Some(0), Recorded::Value(Some(9)))), None);
    assert_eq!(target(&member(CAPTURED, 0, Some(2), None, Recorded::Value(Some(9)))), None);
    assert_eq!(target(&member(MISSION, 2, Some(2), None, Recorded::Value(Some(9)))), None);
    assert_eq!(target(&member(MISSION, 0, Some(2), None, Recorded::NotRecorded)), None);
    assert_eq!(target(&member(MISSION, 0, Some(2), None, Recorded::Value(Some(1)))), Some(1));
}


#[test]
fn captivity_and_cooldown_days_round_as_the_game_does_and_progress_is_never_clamped() {
    use super::operative_content::{
        capture_progress, capture_valid_through, cooldown_progress, cooldown_remaining,
        cooldown_valid_through, native_progress,
    };
    let epoch = game_model::calendar::HOUR_EPOCH;
    assert_eq!(capture_progress(epoch + 23, epoch + 24, 17), (16, 1.0 / 17.0));
    assert_eq!(capture_progress(epoch + 1, epoch + 23, 17), (17, 0.0));
    assert_eq!(capture_progress(epoch + 48, epoch + 24, 17), (18, 0.0));
    assert_eq!(capture_progress(epoch, epoch + 24 * 20, 17), (-3, 20.0 / 17.0));
    assert_eq!(capture_progress(epoch - 23, epoch, 17), (17, 0.0));
    assert_eq!(cooldown_progress(3, epoch + 2, epoch + 50, epoch + 1), (3, 0.0));
    assert_eq!(cooldown_progress(2, epoch + 2, epoch + 50, epoch + 27), (1, 0.5));
    assert_eq!(cooldown_progress(3, epoch + 2, epoch + 50, epoch + 99), (-1, 1.0));
    assert_eq!(cooldown_progress(3, epoch + 50, epoch + 2, epoch), (1, 0.0));
    assert_eq!(cooldown_valid_through(0, epoch + 50, epoch), i32::MAX);
    for delta in -72..=72 {
        let now = epoch + delta;
        let through = capture_valid_through(now);
        assert!(through >= now);
        assert_eq!(capture_progress(epoch, through, 17), capture_progress(epoch, now, 17));
        assert_ne!(capture_progress(epoch, through + 1, 17), capture_progress(epoch, now, 17));
        let end = epoch + 26;
        let cool_through = cooldown_valid_through(2, end, now);
        assert!(cool_through >= now);
        assert_eq!(cooldown_remaining(2, end, cool_through), cooldown_remaining(2, end, now));
        assert_ne!(cooldown_remaining(2, end, cool_through + 1), cooldown_remaining(2, end, now));
    }
    assert_eq!(native_progress(25_000_000, 100), 250_000);
    assert_eq!(native_progress(-25_000_000, 100), -250_000);
    assert_eq!(native_progress(1, 0), 0xffff_ffff);
    assert_eq!(native_progress(i64::MAX, 1), 0xffff_ffff);
}

mod espionage {
    use super::super::espionage::{Ending, OperationState, current_phase, description, elapsed_days, endings, operatives, preparation_ratio};
    use dashboard_query::log::Cell;
    use game_model::calendar::HOUR_EPOCH;
    use replay_data::rows::{OperationAssignmentRow, OperationRow, OperativeMembershipRow, Recorded};

    fn operation(duration_days: i32, start_hour: i32, phase_count: u32, terminal: u8) -> OperationRow {
        OperationRow { id: 7, duration_days, start_hour, phase_count, terminal, assigned_slots: 2, ..OperationRow::default() }
    }

    #[test]
    fn an_operation_ends_at_the_first_read_that_no_longer_holds_it_and_one_carried_to_the_stretch_end_has_not_ended() {
        let reads: Vec<(i32, i32)> = (0..=8).map(|day| (day * 24, day * 24)).collect();
        let lives = [(1u128, 0i64, 47i64), (2, 24, 200), (3, 48, 71), (4, 120, 191)];
        let ended = endings(&lives, (0, 200), &reads, 0, 200);
        assert_eq!(ended, vec![Ending { key: 1, first: 0, last: 47, by: 48 }, Ending { key: 3, first: 48, last: 71, by: 72 }, Ending { key: 4, first: 120, last: 191, by: 192 }]);
        assert_eq!(endings(&lives, (0, 200), &reads, 0, 60).iter().map(|ending| ending.key).collect::<Vec<_>>(), vec![1]);
        assert_eq!(endings(&lives, (0, 200), &reads, 49, 200).iter().map(|ending| ending.key).collect::<Vec<_>>(), vec![3, 4]);
    }

    #[test]
    fn preparation_phase_and_elapsed_days_follow_the_games_view() {
        let mut row = operation(0, 0, 3, 0);
        row.preparation_current = 50_000;
        row.preparation_required = 200_000;
        assert_eq!(preparation_ratio(&row), Some(25_000));
        row.preparation_current = i64::MAX;
        assert_eq!(preparation_ratio(&row), Some(100_000), "a sentinel is a full meter");
        row.preparation_current = -5;
        assert_eq!(preparation_ratio(&row), Some(0), "clamped at zero");
        row.preparation_required = 0;
        assert_eq!(preparation_ratio(&row), Some(0), "nothing required is an empty meter");
        let state = OperationState::of(&row, HOUR_EPOCH + 240);
        assert_eq!((state.start_hour, state.elapsed_days, state.current_phase), (None, None, None));
        let start = HOUR_EPOCH + 240;
        let row = operation(30, start, 3, 0);
        assert_eq!(elapsed_days(&row, start + 15 * 24 + 5), Some(15));
        assert_eq!(current_phase(&row, start + 15 * 24), Some(1));
        assert_eq!(elapsed_days(&row, start + 30 * 24), None);
        assert_eq!(elapsed_days(&row, start - 24), None);
        assert_eq!(elapsed_days(&operation(30, start, 3, 1), start + 24), None);
    }

    #[test]
    fn operatives_are_named_in_slot_order_and_unknown_where_a_slot_or_a_name_was_not_read() {
        let row = operation(30, 0, 3, 0);
        let slot = |ordinal: u32, operative: u64| OperationAssignmentRow { operation: 7, ordinal, operative, selection: 0 };
        let member = |id: u64, roster_country: u32, name: &[u8]| OperativeMembershipRow { id, roster_country, name: Recorded::Value(name.to_vec()), ..OperativeMembershipRow::default() };
        let members = [member(11, 1, b"Ada\0"), member(12, 1, b"Bo"), member(12, 2, b"Other")];
        let named = operatives(&row, Some(&[slot(1, 0), slot(0, 11)]), Some(&members)).unwrap();
        assert_eq!(named.iter().map(|name| name.name.as_deref()).collect::<Vec<_>>(), vec![Some("Ada")]);
        let named = operatives(&row, Some(&[slot(0, 12), slot(1, 11)]), Some(&members)).unwrap();
        assert_eq!(named[0].name, None);
        assert!(named[0].unavailable.is_some());
        assert_eq!(operatives(&row, Some(&[slot(0, 11)]), Some(&members)), None);
        assert_eq!(operatives(&row, None, Some(&members)), None);
        assert!(operatives(&row, Some(&[slot(0, 11), slot(1, 0)]), None).unwrap()[0].unavailable.is_some());
    }

    #[test]
    fn a_completions_description_is_its_stored_text_or_the_named_refusal() {
        let cells = |text: &str, error: i64| {
            let mut cells = vec![Cell::Number(0); 10];
            cells[7] = Cell::Text(text.to_owned());
            cells[8] = Cell::Number(error);
            cells
        };
        assert_eq!(description(&cells("Stole a blueprint.", 0)), Ok("Stole a blueprint.".to_owned()));
        assert!(description(&cells("bad \u{FFFD} bytes", 0)).unwrap_err().contains("non-UTF-8"));
        assert!(description(&cells("", 1)).unwrap_err().contains("byte budget"));
        assert!(description(&cells("", 2)).unwrap_err().contains("byte ring"));
    }
}

mod agencies {
    use super::super::Country;
    use super::super::agencies::{Agency, AgencyRank, AgencyTop, ranked, upgrades};
    use replay_data::rows::AgencyUpgradeLevelRow;
    use std::collections::HashMap;

    fn level(ordinal: u32, level: i32, identity_status: u8) -> AgencyUpgradeLevelRow {
        AgencyUpgradeLevelRow { country: 9, ordinal, upgrade_token: Some(1), branch_token: Some(2), level, identity_status, ..AgencyUpgradeLevelRow::default() }
    }

    #[test]
    fn acquired_upgrades_sum_levels_at_or_above_zero_only_where_every_entry_is_authenticated_and_ordered() {
        let tokens = HashMap::from([(1, "upgrade_a".to_owned()), (2, "branch_a".to_owned())]);
        let rows = [level(1, -2, 0), level(0, 3, 0), level(2, 1, 0)];
        let read = upgrades(&rows.iter().collect::<Vec<_>>(), &tokens, None);
        assert_eq!(read.items.iter().map(|item| item.ordinal).collect::<Vec<_>>(), vec![0, 1, 2], "in stored order");
        assert_eq!(read.nonnegative_level_sum.as_deref(), Some("4"), "a negative level adds nothing");
        assert!(read.items.iter().all(|item| item.name.is_none() && item.missing.as_deref() == Some("Acquired upgrade content unavailable")));
        assert_eq!(upgrades(&[], &tokens, None).nonnegative_level_sum.as_deref(), Some("0"));
        let refused = [level(0, 3, 0), level(1, 1, 4)];
        let read = upgrades(&refused.iter().collect::<Vec<_>>(), &tokens, None);
        assert_eq!(read.nonnegative_level_sum, None);
        assert!(read.unavailable.is_some());
        assert!(read.items[1].missing.as_deref().is_some_and(|why| why.starts_with("Acquired upgrade identity refused")));
        let gap = [level(0, 3, 0), level(2, 1, 0)];
        assert_eq!(upgrades(&gap.iter().collect::<Vec<_>>(), &tokens, None).nonnegative_level_sum, None);
        let read = upgrades(&[&level(0, 1, 0)], &HashMap::new(), None);
        assert_eq!(read.items[0].branch, None);
    }

    fn agency(id: u32, defense: i64, active: i32) -> Agency {
        Agency {
            country: Country { id, tag: Some(format!("T{id}")) },
            created: true,
            name: None,
            name_missing: None,
            defense_raw: defense.to_string(),
            defense_text: None,
            defense_value: None,
            active_count: active,
            maximum: active,
            upgrades: None,
        }
    }

    #[test]
    fn the_top_agencies_are_ranked_by_the_asked_measure_ties_by_country_and_none_asked_keeps_every_agency() {
        let all = vec![agency(3, 500_000, 2), agency(1, 900_000, 1), agency(2, 500_000, 7), agency(4, 100_000, 7)];
        let ids = |list: Vec<Agency>| list.iter().map(|agency| agency.country.id).collect::<Vec<_>>();
        assert_eq!(ids(ranked(all.clone(), None)), vec![3, 1, 2, 4], "no top: as read");
        assert_eq!(ids(ranked(all.clone(), Some(AgencyTop { count: 3, by: AgencyRank::Defense }))), vec![1, 2, 3], "equal defence: lower country id first");
        assert_eq!(ids(ranked(all.clone(), Some(AgencyTop { count: 2, by: AgencyRank::Active }))), vec![2, 4]);
        assert_eq!(ids(ranked(all, Some(AgencyTop { count: 9, by: AgencyRank::Maximum }))).len(), 4, "a count above the population keeps everyone");
    }
}

mod exact_playback {
    use super::super::{
        decryption::Decryption,
        operative_content::{CAPTURED, COOLDOWN, MISSION, OPERATION, OPERATIVE_CATEGORY},
        operative_matrix::OperativeMatrix,
        roster::OperativesRoster,
    };
    use dashboard_base::{
        Source, SourceKey,
        products::{
            LoadContext,
            at_hour::{exact_panel, frame_panel},
        },
        test_fixtures::{
            content_identity, materialized_frame, played, records, source, source_with_content,
            streamed, visual_mods::Machine, with_facts,
        },
    };
    use replay_capture_spool::spool::Record;
    use replay_data::rows::{
        CharacterOperativeRow, CharacterPortraitRow, CountryAppearanceRow,
        CountryLinkRow, CountryRow, CryptologyRow, FactionMemberRow, FactionRow,
        IntelligenceAgencyRow, LexicalTokenRow, OperationRow, OperativeMembershipRow,
        PoliticsRow, RaidIntelInputRow, Recorded, ReservedCountryNamingRow,
        SimulationFeatureRow, TableId, TableRows,
    };
    use replay_frame::{
        branches::BranchReason,
        frame::{DecodedFrame, TableView},
    };
    use serde_json::json;
    use std::sync::Arc;

    fn two_branch_source(
        branch1: Vec<DecodedFrame>,
        branch2: Vec<DecodedFrame>,
        root: &std::path::Path,
    ) -> (Arc<Source>, LoadContext) {
        std::fs::create_dir_all(root.join("user")).unwrap();
        let launcher = json!({"enabled_mods": [], "disabled_dlcs": []}).to_string();
        for (path, text) in [
            ("hoi4.exe", "test"),
            ("user/settings.txt", "language=l_english force_pow2_textures=no"),
            ("user/dlc_load.json", &launcher),
        ] {
            std::fs::write(root.join(path), text).unwrap();
        }
        let (options, identity) = content_identity(root);
        let mut input = records(1, branch1);
        let Record::SessionStart(session) = &mut input[0] else {
            unreachable!()
        };
        session.identity = identity.clone();
        let mut second = records(2, branch2);
        second.remove(0);
        if let Some(Record::BranchStart(start)) = second.first_mut() {
            start.session = 1;
            start.reason = BranchReason::LoadSignal;
            start.loaded_hour = Some(start.first_hour);
        }
        input.extend(second);
        let replay = with_facts(&options, &identity, input)
            .into_timelines(&Default::default())
            .unwrap();
        let built = Source::from_replay(
            SourceKey { source_id: "intel-exact".into(), revision: "1".into() },
            "memory.battleplan".into(),
            Arc::new(replay),
            &false.into(),
        )
        .unwrap();
        (Arc::new(built), LoadContext { content: Some(options) })
    }

    fn base_frame(branch: u64, hour: i32, include_ger: bool) -> DecodedFrame {
        let mut frame = source(branch, hour, Some(i64::from(hour) * 13));
        let mut countries = Vec::new();
        if include_ger {
            countries.push(CountryRow {
                id: 9001,
                canonical_slot: 7,
                tag: b"GER".to_vec(),
                political_power_raw: Some(i64::from(hour) * 17),
                ..CountryRow::default()
            });
        }
        countries.push(CountryRow {
            id: 9002,
            canonical_slot: 8,
            tag: b"FRA".to_vec(),
            political_power_raw: Some(i64::from(hour) * 23),
            ..CountryRow::default()
        });
        countries.push(CountryRow {
            id: 9003,
            canonical_slot: 9,
            tag: b"ENG".to_vec(),
            political_power_raw: Some(i64::from(hour) * 29),
            ..CountryRow::default()
        });
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
        let mut links = vec![
            CountryLinkRow { id: 9002, canonical_slot: 8 },
            CountryLinkRow { id: 9003, canonical_slot: 9 },
        ];
        if include_ger {
            links.push(CountryLinkRow { id: 11, canonical_slot: 7 });
            links.push(CountryLinkRow { id: 9001, canonical_slot: 7 });
        }
        links.sort_by_key(|row| row.id);
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links));
        *frame.tables[TableId::SimulationFeatures.index()] = TableView::Complete(TableRows::SimulationFeatures(vec![
            SimulationFeatureRow { dlc_mask: 1 << 5, ..SimulationFeatureRow::default() },
        ]));
        frame
    }

    #[test]
    fn operatives_roster_exact_playback_holds_through_unrelated_edits_and_rebuilds_on_declared_changes() {
        dashboard_core::test_fixtures::game::enter();
        let machine = Machine::new(&format!("intel_roster_exact_{}", std::process::id()));
        machine.write(
            "common/defines/probe.lua",
            b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=7},NOperatives={OPERATIVE_CAPTURE_DURATION_IN_DAYS=29}}",
        );
        machine.write(
            "localisation/english/probe_l_english.yml",
            "\u{feff}l_english:\n DURATION_DAYS:0 \"$NUM$ ticks\"\n OPERATIVE_CAPTURED_BY:0 \"Held by $CAPTURER$\"\n OPERATIVE_MISSION_NO_MISSION_TITLE:0 \"Idle\"\n OPERATIVE_MISSION_BUILD_INTEL_NETWORK_TITLE:0 \"Build\"\n FRA:0 \"Republic\"\n FRA_ALT:0 \"Empire\"\n".as_bytes(),
        );
        let epoch = game_model::calendar::HOUR_EPOCH;
        let first = epoch + 100;
        let last = epoch + 123;
        let make_frame = |branch: u64, hour: i32| {
            let offset = hour - first;
            let frozen = if branch == 1 && (15..=17).contains(&offset) { 15 } else { offset };
            let include_ger = !(branch == 1 && (20..=21).contains(&offset));
            let mut frame = base_frame(branch, first + frozen, include_ger);
            frame.hour = hour;
            let ger_exp = if branch == 2 { 500_000 } else if offset >= 5 { 200_000 } else { 100_000 };
            let ger_assignment = if branch == 2 {
                MISSION
            } else if (8..=11).contains(&offset) {
                CAPTURED
            } else if (5..=7).contains(&offset) {
                OPERATION
            } else {
                MISSION
            };
            let ger_op = OperativeMembershipRow {
                id: 501,
                owner_country: 9001,
                roster_country: 9001,
                roster_kind: 0,
                assignment: ger_assignment,
                experience_raw: ger_exp,
                skill: Some(1),
                capturer_country: if ger_assignment == CAPTURED { 9002 } else { 0 },
                captured_hour: if ger_assignment == CAPTURED { first + 8 } else { engine_native_memory::cgame_date::NULL_RAW_HOUR },
                action_variant: (ger_assignment == MISSION).then_some(1),
                ordinal: Recorded::Value(0),
                target_country: Recorded::Value(Some(9002)),
                operation: Recorded::Value((ger_assignment == OPERATION).then_some(701)),
                name: Recorded::Value(if branch == 2 { b"Branch Two Agent".to_vec() } else { b"Alpha Agent".to_vec() }),
                cooldown_kind: Recorded::Value(None),
                cooldown_start_hour: Recorded::Value(None),
                cooldown_end_hour: Recorded::Value(None),
                ..OperativeMembershipRow::default()
            };
            let foreign_op = OperativeMembershipRow {
                id: 502,
                owner_country: 9002,
                roster_country: 9002,
                roster_kind: 0,
                assignment: MISSION,
                experience_raw: i64::from(frozen) * 1_000,
                skill: Some(1),
                action_variant: Some(1),
                ordinal: Recorded::Value(0),
                target_country: Recorded::Value(Some(9003)),
                operation: Recorded::Value(None),
                name: Recorded::Value(b"Foreign Agent".to_vec()),
                cooldown_kind: Recorded::Value(None),
                cooldown_start_hour: Recorded::Value(None),
                cooldown_end_hour: Recorded::Value(None),
                ..OperativeMembershipRow::default()
            };
            if branch == 1 && (15..=17).contains(&offset) {
                *frame.tables[TableId::OperativeMemberships.index()] = TableView::Unavailable { reason: "damaged roster".into() };
            } else {
                *frame.tables[TableId::OperativeMemberships.index()] = TableView::Complete(TableRows::OperativeMemberships(vec![ger_op, foreign_op]));
            }
            let ger_traits = if offset >= 12 { vec![1] } else { Vec::new() };
            let chars = vec![
                CharacterOperativeRow {
                    character: 101,
                    reference: 501,
                    resolved: true,
                    category: OPERATIVE_CATEGORY,
                    owner: 9001,
                    skill_level: Some(frozen),
                    trait_definitions: ger_traits,
                    ..CharacterOperativeRow::default()
                },
                CharacterOperativeRow {
                    character: 102,
                    reference: 502,
                    resolved: true,
                    category: OPERATIVE_CATEGORY,
                    owner: 9002,
                    trait_definitions: vec![u32::try_from(frozen).unwrap()],
                    ..CharacterOperativeRow::default()
                },
                CharacterOperativeRow {
                    character: 103,
                    reference: 999,
                    resolved: true,
                    category: 1,
                    owner: 9001,
                    trait_definitions: vec![u32::try_from(frozen).unwrap()],
                    ..CharacterOperativeRow::default()
                },
            ];
            if branch == 1 && offset == 18 {
                *frame.tables[TableId::CharacterOperatives.index()] = TableView::Truncated { rows: TableRows::CharacterOperatives(chars), missing: 1 };
            } else {
                *frame.tables[TableId::CharacterOperatives.index()] = TableView::Complete(TableRows::CharacterOperatives(chars));
            }
            *frame.tables[TableId::CharacterPortraits.index()] = TableView::Complete(TableRows::CharacterPortraits(vec![
                CharacterPortraitRow { character: 101, portraits: Vec::new() },
            ]));
            *frame.tables[TableId::Operations.index()] = TableView::Complete(TableRows::Operations(vec![
                OperationRow { id: 701, owner: 9001, target: 9002, definition_token: 42, preparation_current: i64::from(frozen) * 100, ..OperationRow::default() },
            ]));
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow { token: 42, literal: b"probe_op".to_vec() },
            ]));
            *frame.tables[TableId::CountryAppearance.index()] = TableView::Complete(TableRows::CountryAppearance(vec![
                CountryAppearanceRow {
                    country: 9002,
                    cosmetic_tag: if offset >= 10 { b"FRA_ALT".to_vec() } else { Vec::new() },
                    ..CountryAppearanceRow::default()
                },
            ]));
            *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![
                PoliticsRow { country: 9002, last_election: frozen, ..PoliticsRow::default() },
            ]));
            *frame.tables[TableId::PoliticalParties.index()] = TableView::Complete(TableRows::PoliticalParties(Vec::new()));
            *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(Vec::new()));
            *frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow { tag: b"ZZZ".to_vec(), ..ReservedCountryNamingRow::default() },
            ]));
            frame
        };

        let branch1: Vec<DecodedFrame> = (first..=last).map(|hour| make_frame(1, hour)).collect();
        let (single, load) = source_with_content(branch1.clone(), &machine.root);
        let single = Arc::new(single);
        let params = json!({"branchId": "1", "countries": ["GER"]});
        let exact_p = exact_panel::<OperativesRoster>(&single, &load, &params).unwrap();
        let reader = exact_p.exact().unwrap();
        let mut whole = frame_panel::<OperativesRoster>(&single, &load, &params).unwrap();
        let steps = played(single.replay_ref(), reader, first, last);
        assert_eq!(steps.len(), usize::try_from(last - first + 1).unwrap());
        for step in &steps {
            let offset = step.hour - first;
            let expected = materialized_frame(&mut *whole, single.replay_ref(), 1, step.hour);
            if step.pending {
                assert!((8..=11).contains(&offset), "pending only while NAMING warms at offset {offset}");
                continue;
            }
            assert_eq!(streamed(&step.value, step.fresh), streamed(&expected, step.fresh), "roster mismatch at offset {offset}");
            if matches!(offset, 1..=4 | 6..=7 | 13..=14 | 21 | 23) {
                assert_eq!(step.product_calls, 0, "unrelated edits must not call roster at offset {offset}");
            }
            if matches!(offset, 5 | 12 | 15 | 16 | 17 | 18 | 19 | 20 | 22) {
                assert_eq!(step.product_calls, 1, "declared change must rebuild roster at offset {offset}");
            }
        }
        for offset in 15..=17 {
            let reason = steps[offset as usize].value.as_ref().as_ref().unwrap()["missing"].as_str().unwrap();
            assert!(reason.contains("damaged roster") && reason.contains(&format!("hour {} ", first + offset)), "missing must track hour at offset {offset}: {reason}");
        }
        assert_eq!(steps[20].value.as_ref().as_ref().unwrap()["unheld"], json!(["GER"]));

        let captured_panel = exact_panel::<OperativesRoster>(&single, &load, &params).unwrap();
        let captured_reader = captured_panel.exact().unwrap();
        let captured_steps = played(single.replay_ref(), captured_reader, first + 8, first + 11);
        for step in &captured_steps {
            assert!(!step.pending, "seeded with captured operative must never pend at {}", step.hour);
            let expected = materialized_frame(&mut *whole, single.replay_ref(), 1, step.hour);
            assert_eq!(streamed(&step.value, step.fresh), streamed(&expected, step.fresh), "captured roster mismatch at offset {}", step.hour - first);
        }
        assert_eq!(captured_steps[1].value.as_ref().as_ref().unwrap()["operatives"][0]["assignmentName"], json!("Held by Republic"));
        assert_eq!(captured_steps[2].value.as_ref().as_ref().unwrap()["operatives"][0]["assignmentName"], json!("Held by Empire"));
        assert_eq!(captured_steps[2].product_calls, 1);

        for seek_hour in [first + 10, first + 5, first] {
            let seed = (reader.advance)(seek_hour, true, &false.into(), None);
            assert!(seed.fresh);
            let expected = materialized_frame(&mut *whole, single.replay_ref(), 1, seek_hour);
            assert_eq!(*seed.value, expected, "backward seek mismatch at {seek_hour}");
        }

        let branch2: Vec<DecodedFrame> = (first..=first + 4).map(|hour| make_frame(2, hour)).collect();
        let (branched, b_load) = two_branch_source(branch1, branch2, &machine.root);
        let b1_panel = exact_panel::<OperativesRoster>(&branched, &b_load, &params).unwrap();
        let b1_read = (b1_panel.exact().unwrap().advance)(first, true, &false.into(), None);
        assert_eq!(b1_read.value.as_ref().as_ref().unwrap()["operatives"][0]["name"], json!("Alpha Agent"));
        let b2_params = json!({"branchId": "2", "countries": ["GER"]});
        let b2_panel = exact_panel::<OperativesRoster>(&branched, &b_load, &b2_params).unwrap();
        let b2_reader = b2_panel.exact().unwrap();
        let mut b2_whole = frame_panel::<OperativesRoster>(&branched, &b_load, &b2_params).unwrap();
        for hour in first..=first + 4 {
            let read = (b2_reader.advance)(hour, hour == first, &false.into(), None);
            let expected = materialized_frame(&mut *b2_whole, branched.replay_ref(), 2, hour);
            assert_eq!(streamed(&read.value, read.fresh), streamed(&expected, read.fresh), "branch 2 mismatch at {hour}");
            assert_eq!(read.value.as_ref().as_ref().unwrap()["operatives"][0]["name"], json!("Branch Two Agent"));
        }

        let boundary_frames: Vec<DecodedFrame> = (epoch..=epoch + 25).map(|hour| {
            let mut frame = base_frame(1, epoch, true);
            frame.hour = hour;
            *frame.tables[TableId::OperativeMemberships.index()] = TableView::Complete(TableRows::OperativeMemberships(vec![
                OperativeMembershipRow {
                    id: 601,
                    owner_country: 9001,
                    roster_country: 9001,
                    roster_kind: 0,
                    assignment: COOLDOWN,
                    skill: Some(1),
                    ordinal: Recorded::Value(0),
                    target_country: Recorded::Value(None),
                    operation: Recorded::Value(None),
                    name: Recorded::Value(b"Cool".to_vec()),
                    cooldown_kind: Recorded::Value(Some(2)),
                    cooldown_start_hour: Recorded::Value(Some(epoch)),
                    cooldown_end_hour: Recorded::Value(Some(epoch + 48)),
                    ..OperativeMembershipRow::default()
                },
                OperativeMembershipRow {
                    id: 602,
                    owner_country: 9001,
                    roster_country: 9001,
                    roster_kind: 0,
                    assignment: CAPTURED,
                    skill: Some(1),
                    capturer_country: 9002,
                    captured_hour: epoch,
                    ordinal: Recorded::Value(1),
                    target_country: Recorded::Value(None),
                    operation: Recorded::Value(None),
                    name: Recorded::Value(b"Held".to_vec()),
                    cooldown_kind: Recorded::Value(None),
                    cooldown_start_hour: Recorded::Value(None),
                    cooldown_end_hour: Recorded::Value(None),
                    ..OperativeMembershipRow::default()
                },
            ]));
            *frame.tables[TableId::CharacterOperatives.index()] = TableView::Complete(TableRows::CharacterOperatives(Vec::new()));
            *frame.tables[TableId::CharacterPortraits.index()] = TableView::Complete(TableRows::CharacterPortraits(Vec::new()));
            *frame.tables[TableId::Operations.index()] = TableView::Complete(TableRows::Operations(Vec::new()));
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(Vec::new()));
            *frame.tables[TableId::CountryAppearance.index()] = TableView::Complete(TableRows::CountryAppearance(vec![
                CountryAppearanceRow { country: 9002, ..CountryAppearanceRow::default() },
            ]));
            *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(Vec::new()));
            *frame.tables[TableId::PoliticalParties.index()] = TableView::Complete(TableRows::PoliticalParties(Vec::new()));
            *frame.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(Vec::new()));
            *frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow { tag: b"ZZZ".to_vec(), ..ReservedCountryNamingRow::default() },
            ]));
            frame
        }).collect();
        let (cool_src, cool_load) = source_with_content(boundary_frames, &machine.root);
        let cool_src = Arc::new(cool_src);
        let cool_panel = exact_panel::<OperativesRoster>(&cool_src, &cool_load, &params).unwrap();
        let cool_reader = cool_panel.exact().unwrap();
        let mut cool_whole = frame_panel::<OperativesRoster>(&cool_src, &cool_load, &params).unwrap();
        let cool_steps = played(cool_src.replay_ref(), cool_reader, epoch, epoch + 25);
        for step in &cool_steps {
            let expected = materialized_frame(&mut *cool_whole, cool_src.replay_ref(), 1, step.hour);
            assert_eq!(streamed(&step.value, step.fresh), streamed(&expected, step.fresh), "day boundary mismatch at {}", step.hour);
            let delta = step.hour - epoch;
            if delta == 24 {
                assert_eq!(step.product_calls, 1, "cooldown and captivity must rebuild at day boundary");
            } else if delta > 0 {
                assert_eq!(step.product_calls, 0, "cooldown and captivity must stay quiet between day boundaries at +{delta}");
            }
        }

        let no_content_load = LoadContext { content: None };
        let nc_panel = exact_panel::<OperativesRoster>(&cool_src, &no_content_load, &params).unwrap();
        let nc_reader = nc_panel.exact().unwrap();
        let mut nc_whole = frame_panel::<OperativesRoster>(&cool_src, &no_content_load, &params).unwrap();
        let nc_steps = played(cool_src.replay_ref(), nc_reader, epoch, epoch + 25);
        for step in &nc_steps {
            let expected = materialized_frame(&mut *nc_whole, cool_src.replay_ref(), 1, step.hour);
            assert_eq!(streamed(&step.value, step.fresh), streamed(&expected, step.fresh), "unconfigured boundary mismatch at {}", step.hour);
            if step.hour > epoch {
                assert_eq!(step.product_calls, 0, "without content, day boundaries must not rebuild at {}", step.hour);
            }
        }
    }

    #[test]
    fn operative_matrix_exact_playback_follows_faction_membership_and_holds_on_unrelated_edits() {
        dashboard_core::test_fixtures::game::enter();
        let machine = Machine::new(&format!("intel_matrix_exact_{}", std::process::id()));
        machine.write(
            "common/defines/probe.lua",
            b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=7}}",
        );
        let first = 100;
        let last = 125;
        let make_frame = |branch: u64, hour: i32| {
            let offset = hour - first;
            let frozen = if branch == 1 && (16..=18).contains(&offset) {
                16
            } else if branch == 1 && (20..=21).contains(&offset) {
                20
            } else if branch == 1 && (24..=25).contains(&offset) {
                24
            } else {
                offset
            };
            let mut frame = base_frame(branch, first + frozen, true);
            frame.hour = hour;
            if branch == 1 && (24..=25).contains(&offset) {
                *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "damaged links".into() };
            }
            let faction_name = if offset >= 10 { b"Pact Beta".to_vec() } else { b"Pact Alpha".to_vec() };
            let mut factions = Vec::new();
            let mut members = Vec::new();
            if branch == 1 && offset >= 6 {
                factions.push(FactionRow { id: 1, name: faction_name, ..FactionRow::default() });
                members.push(FactionMemberRow { country: 9001, faction: 1, ordinal: 0 });
                if offset >= 8 {
                    members.push(FactionMemberRow { country: 9003, faction: 1, ordinal: 1 });
                }
            }
            *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(factions));
            *frame.tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(members));

            let mut ops = vec![
                OperativeMembershipRow {
                    id: 1,
                    owner_country: 9001,
                    roster_country: 9001,
                    roster_kind: 0,
                    assignment: MISSION,
                    action_variant: Some(1),
                    target_country: Recorded::Value(Some(if branch == 2 { 9003 } else { 9002 })),
                    operation: Recorded::Value(None),
                    name: Recorded::Value(b"Spy One".to_vec()),
                    experience_raw: i64::from(frozen) * 10,
                    ..OperativeMembershipRow::default()
                },
                OperativeMembershipRow {
                    id: 2,
                    owner_country: 9002,
                    roster_country: 9002,
                    roster_kind: 1,
                    assignment: MISSION,
                    action_variant: Some(1),
                    target_country: Recorded::Value(Some(9001)),
                    operation: Recorded::Value(None),
                    name: Recorded::Value(format!("Inactive {frozen}").into_bytes()),
                    ..OperativeMembershipRow::default()
                },
            ];
            if branch == 1 && offset >= 12 {
                ops.push(OperativeMembershipRow {
                    id: 3,
                    owner_country: 9003,
                    roster_country: 9003,
                    roster_kind: 0,
                    assignment: MISSION,
                    action_variant: Some(1),
                    target_country: Recorded::Value(Some(9001)),
                    operation: Recorded::Value(None),
                    name: Recorded::Value(b"Spy Three".to_vec()),
                    ..OperativeMembershipRow::default()
                });
            }
            if branch == 1 && (16..=18).contains(&offset) {
                *frame.tables[TableId::OperativeMemberships.index()] = TableView::Unavailable { reason: "damaged memberships".into() };
            } else {
                *frame.tables[TableId::OperativeMemberships.index()] = TableView::Complete(TableRows::OperativeMemberships(ops));
            }
            let agencies = vec![
                IntelligenceAgencyRow {
                    country: 9001,
                    upgrade_progress_raw: i64::from(frozen) * 50,
                    defense_raw: i64::from(frozen) * 7,
                    active_count: if offset >= 14 { 3 } else { 1 },
                    unknown_f0: 4,
                    created: 1,
                    ..IntelligenceAgencyRow::default()
                },
                IntelligenceAgencyRow {
                    country: 9002,
                    active_count: frozen,
                    unknown_f0: 2,
                    created: 0,
                    ..IntelligenceAgencyRow::default()
                },
            ];
            if branch == 1 && (20..=21).contains(&offset) {
                *frame.tables[TableId::IntelligenceAgencies.index()] = TableView::Unavailable { reason: "damaged agencies".into() };
            } else {
                *frame.tables[TableId::IntelligenceAgencies.index()] = TableView::Complete(TableRows::IntelligenceAgencies(agencies));
            }
            frame
        };

        let branch1: Vec<DecodedFrame> = (first..=last).map(|hour| make_frame(1, hour)).collect();
        let branch2: Vec<DecodedFrame> = (first..=first + 3).map(|hour| make_frame(2, hour)).collect();
        let (single, load) = source_with_content(branch1.clone(), &machine.root);
        let single = Arc::new(single);
        let (branched, b_load) = two_branch_source(branch1, branch2, &machine.root);

        for (form, quiet_offsets, change_offsets) in [
            ("matrix", [1, 2, 3, 4, 5, 7, 9, 11, 13, 15, 17, 18, 21, 23].as_slice(), [6, 8, 10, 12, 14, 16, 19, 20, 22, 24, 25].as_slice()),
            ("pairs", [1, 2, 3, 4, 5, 7, 9, 11, 13, 14, 15, 17, 18, 20, 21, 22, 23].as_slice(), [6, 8, 10, 12, 16, 19, 24, 25].as_slice()),
        ] {
            let params = json!({"branchId": "1", "countries": [], "form": form});
            let exact_p = exact_panel::<OperativeMatrix>(&single, &load, &params).unwrap();
            let reader = exact_p.exact().unwrap();
            let mut whole = frame_panel::<OperativeMatrix>(&single, &load, &params).unwrap();
            let steps = played(single.replay_ref(), reader, first, last);
            for step in &steps {
                let expected = materialized_frame(&mut *whole, single.replay_ref(), 1, step.hour);
                assert_eq!(streamed(&step.value, step.fresh), streamed(&expected, step.fresh), "{form} mismatch at offset {}", step.hour - first);
                let offset = step.hour - first;
                if quiet_offsets.contains(&offset) {
                    assert_eq!(step.product_calls, 0, "{form} must stay quiet at offset {offset}");
                }
                if change_offsets.contains(&offset) {
                    assert_eq!(step.product_calls, 1, "{form} must rebuild at offset {offset}");
                }
            }
            for offset in [24, 25] {
                let reason = steps[offset as usize].value.as_ref().as_ref().unwrap()["missing"].as_str().unwrap();
                assert!(reason.contains(&format!("hour {} ", first + offset)), "{form} missing must track hour at offset {offset}: {reason}");
            }
            let at_6 = steps[6].value.as_ref().as_ref().unwrap()["countries"].as_array().unwrap();
            assert!(at_6.iter().any(|c| c["tag"] == "GER" && c["factionName"] == "Pact Alpha"));
            let at_10 = steps[10].value.as_ref().as_ref().unwrap()["countries"].as_array().unwrap();
            assert!(at_10.iter().any(|c| c["tag"] == "GER" && c["factionName"] == "Pact Beta"));
            for seek_hour in [first + 12, first + 6, first] {
                let seed = (reader.advance)(seek_hour, true, &false.into(), None);
                assert!(seed.fresh);
                assert_eq!(*seed.value, materialized_frame(&mut *whole, single.replay_ref(), 1, seek_hour));
            }
            let b2_params = json!({"branchId": "2", "countries": [], "form": form});
            let b2_panel = exact_panel::<OperativeMatrix>(&branched, &b_load, &b2_params).unwrap();
            let b2_reader = b2_panel.exact().unwrap();
            let mut b2_whole = frame_panel::<OperativeMatrix>(&branched, &b_load, &b2_params).unwrap();
            for hour in first..=first + 3 {
                let read = (b2_reader.advance)(hour, hour == first, &false.into(), None);
                let expected = materialized_frame(&mut *b2_whole, branched.replay_ref(), 2, hour);
                assert_eq!(streamed(&read.value, read.fresh), streamed(&expected, read.fresh), "{form} branch 2 mismatch at {hour}");
            }
        }

        let chosen_params = json!({"branchId": "1", "countries": ["GER"], "form": "matrix"});
        let chosen_panel = exact_panel::<OperativeMatrix>(&single, &load, &chosen_params).unwrap();
        let chosen_reader = chosen_panel.exact().unwrap();
        let mut chosen_whole = frame_panel::<OperativeMatrix>(&single, &load, &chosen_params).unwrap();
        let chosen_steps = played(single.replay_ref(), chosen_reader, first, last);
        for step in &chosen_steps {
            let expected = materialized_frame(&mut *chosen_whole, single.replay_ref(), 1, step.hour);
            assert_eq!(streamed(&step.value, step.fresh), streamed(&expected, step.fresh), "chosen matrix mismatch at offset {}", step.hour - first);
            let offset = step.hour - first;
            if matches!(offset, 1..=5 | 7 | 9 | 11 | 13 | 15 | 23) {
                assert_eq!(step.product_calls, 0, "chosen matrix must stay quiet at offset {offset}");
            }
        }
    }

    #[test]
    fn decryption_exact_playback_follows_attempts_seven_day_window_and_live_cells() {
        dashboard_core::test_fixtures::game::enter();
        let machine = Machine::new(&format!("intel_decryption_exact_{}", std::process::id()));
        machine.write(
            "common/defines/probe.lua",
            b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=7},NOperatives={CRYPTO_BASE_CRYPTO_LEVEL=2,CRYPTO_CRYPTO_LEVEL_PER_CRYPTO_UPGRADE=1}}",
        );
        let first = 100;
        let last = 310;
        let make_frame = |branch: u64, hour: i32| {
            let frozen = if branch == 1 && (305..=307).contains(&hour) { 305 } else { hour };
            let mut frame = base_frame(branch, frozen, true);
            frame.hour = hour;
            let crypto_rows = if branch == 2 {
                vec![CryptologyRow {
                    owner_country: 9002,
                    target_country: 9001,
                    amount_raw: 150_000,
                    bonus_days: hour - first,
                    bonus_active: 0,
                    decrypting: 1,
                    ordinal: Recorded::Value(0),
                }]
            } else if hour < 120 {
                vec![CryptologyRow {
                    owner_country: 9001,
                    target_country: 9002,
                    amount_raw: 0,
                    bonus_days: 0,
                    bonus_active: 0,
                    decrypting: 0,
                    ordinal: Recorded::Value(0),
                }]
            } else {
                let amount_raw = if hour >= 300 { 600_000 } else { 100_000 };
                vec![CryptologyRow {
                    owner_country: 9001,
                    target_country: 9002,
                    amount_raw,
                    bonus_days: hour - 100,
                    bonus_active: u8::from(hour >= 300),
                    decrypting: u8::from(hour != 125),
                    ordinal: Recorded::Value(hour as u32),
                }]
            };
            if branch == 1 && (305..=307).contains(&hour) {
                *frame.tables[TableId::Cryptology.index()] = TableView::Unavailable { reason: "damaged ciphers".into() };
            } else {
                *frame.tables[TableId::Cryptology.index()] = TableView::Complete(TableRows::Cryptology(crypto_rows));
            }
            *frame.tables[TableId::RaidIntelInputs.index()] = TableView::Complete(TableRows::RaidIntelInputs(vec![
                RaidIntelInputRow {
                    country: 9002,
                    country_defense: i64::from(frozen) * 5,
                    crypto_strength: Some(if hour >= 295 { 300_000 } else { 200_000 }),
                    ..RaidIntelInputRow::default()
                },
                RaidIntelInputRow {
                    country: 9003,
                    country_defense: i64::from(frozen) * 9,
                    crypto_strength: Some(i64::from(frozen) * 100_000),
                    ..RaidIntelInputRow::default()
                },
            ]));
            frame
        };

        let branch1: Vec<DecodedFrame> = (first..=last).map(|hour| make_frame(1, hour)).collect();
        let branch2: Vec<DecodedFrame> = (first..=first + 4).map(|hour| make_frame(2, hour)).collect();
        let (single, load) = source_with_content(branch1.clone(), &machine.root);
        let single = Arc::new(single);
        let params = json!({"branchId": "1"});
        let exact_p = exact_panel::<Decryption>(&single, &load, &params).unwrap();
        let reader = exact_p.exact().unwrap();
        let mut whole = frame_panel::<Decryption>(&single, &load, &params).unwrap();
        let steps = played(single.replay_ref(), reader, first, last);
        assert_eq!(steps.len(), usize::try_from(last - first + 1).unwrap());
        let expected_calls = [120, 125, 126, 268, 288, 295, 300, 305, 306, 307, 308];
        let mut total_calls = 0;
        for step in &steps {
            let expected = materialized_frame(&mut *whole, single.replay_ref(), 1, step.hour);
            assert_eq!(streamed(&step.value, step.fresh), streamed(&expected, step.fresh), "decryption mismatch at hour {}", step.hour);
            total_calls += step.product_calls;
            if step.hour > first {
                if expected_calls.contains(&step.hour) {
                    assert_eq!(step.product_calls, 1, "decryption must rebuild at hour {}", step.hour);
                } else {
                    assert_eq!(step.product_calls, 0, "decryption must stay quiet at hour {}", step.hour);
                }
            }
        }
        assert_eq!(total_calls, expected_calls.len() as u64);
        assert!(steps[(267 - first) as usize].value.as_ref().as_ref().unwrap()["pairs"][0]["weekChangeTenths"].is_null());
        assert_eq!(steps[(268 - first) as usize].value.as_ref().as_ref().unwrap()["pairs"][0]["weekChangeTenths"], json!(250));
        assert_eq!(steps[(288 - first) as usize].value.as_ref().as_ref().unwrap()["pairs"][0]["weekChangeTenths"], json!(0));
        assert_eq!(steps[(300 - first) as usize].value.as_ref().as_ref().unwrap()["pairs"][0]["broken"], json!(true));
        for hour in 305..=307 {
            let reason = steps[(hour - first) as usize].value.as_ref().as_ref().unwrap()["missing"].as_str().unwrap();
            assert!(reason.contains("damaged ciphers") && reason.contains(&format!("hour {hour} ")), "missing must track hour {hour}: {reason}");
        }

        for seek_hour in [302, 288, 268, 120, 100] {
            let seed = (reader.advance)(seek_hour, true, &false.into(), None);
            assert!(seed.fresh);
            let expected = materialized_frame(&mut *whole, single.replay_ref(), 1, seek_hour);
            assert_eq!(*seed.value, expected, "backward seek live values must match at {seek_hour}");
        }

        let (branched, b_load) = two_branch_source(branch1, branch2, &machine.root);
        let b2_params = json!({"branchId": "2"});
        let b2_panel = exact_panel::<Decryption>(&branched, &b_load, &b2_params).unwrap();
        let b2_reader = b2_panel.exact().unwrap();
        let mut b2_whole = frame_panel::<Decryption>(&branched, &b_load, &b2_params).unwrap();
        for hour in first..=first + 4 {
            let read = (b2_reader.advance)(hour, hour == first, &false.into(), None);
            let expected = materialized_frame(&mut *b2_whole, branched.replay_ref(), 2, hour);
            assert_eq!(streamed(&read.value, read.fresh), streamed(&expected, read.fresh), "decryption branch 2 mismatch at {hour}");
            assert_eq!(read.value.as_ref().as_ref().unwrap()["pairs"][0]["owner"]["tag"], json!("FRA"));
        }
    }
}
