use super::{
    LeaderKinds, LeaderSelection, Tally, career::{RunValues, career_gains, career_runs, join_battles, led_spans},
    fold_date, held_traits, KIND_ARMY, KIND_NAVY,
};
use replay_data::rows::{LeaderBattleRow as Battle, LeaderTraitGainRow as Gain, UnitLeaderTraitRow};
use std::collections::HashMap;

#[test]
fn a_leaders_battles_are_its_participations_and_its_results_won_or_lost() {
    let facet = |id: u32, entry: u32, outcome: u32| (u128::from(id) << 96) | (u128::from(entry) << 64) | (u128::from(outcome) << 32);
    let mut tally = Tally::default();
    tally.counts = HashMap::from([(facet(7, 0, u32::MAX), 5), (facet(7, 1, 1), 2), (facet(7, 1, 2), 1), (facet(7, 1, u32::MAX), 1), (facet(8, 0, u32::MAX), 3)]);
    let seven = tally.battles(7);
    assert_eq!((seven.fought, seven.won, seven.lost), (5, 2, 1), "a result whose outcome is not recorded is neither won nor lost");
    assert_eq!(tally.battles(8).fought, 3);
    assert_eq!(tally.battles(9).fought, 0);
}

#[test]
fn a_trait_is_dated_by_its_gain_and_a_later_first_observation_does_not_move_it() {
    let mut dates = HashMap::new();
    let key = |token: &str| (5u64, token.to_owned());
    fold_date(&mut dates, key("organizer"), 100, Gain::HELD_BEFORE);
    fold_date(&mut dates, key("organizer"), 400, Gain::FIRST_OBSERVED);
    fold_date(&mut dates, key("panzer_leader"), 200, Gain::GAINED);
    fold_date(&mut dates, key("trickster"), 300, Gain::FIRST_OBSERVED);
    fold_date(&mut dates, key("panzer_leader"), 350, Gain::HELD_BEFORE);
    fold_date(&mut dates, key("trickster"), 500, Gain::GAINED);
    assert_eq!(dates[&key("organizer")], (100, Gain::HELD_BEFORE));
    assert_eq!(dates[&key("panzer_leader")], (200, Gain::GAINED));
    assert_eq!(dates[&key("trickster")], (500, Gain::GAINED));
}

#[test]
fn a_careers_trait_lane_lists_each_gain_and_first_dates_only() {
    let rows = vec![(10, "a".to_owned(), Gain::HELD_BEFORE), (20, "a".to_owned(), Gain::FIRST_OBSERVED), (30, "b".to_owned(), Gain::FIRST_OBSERVED), (40, "a".to_owned(), Gain::GAINED)];
    assert_eq!(career_gains(&rows), vec![(10, "a".to_owned(), Gain::HELD_BEFORE), (30, "b".to_owned(), Gain::FIRST_OBSERVED), (40, "a".to_owned(), Gain::GAINED)]);
}

#[test]
fn a_careers_runs_join_equal_hours_and_break_where_the_row_is_absent() {
    let values = |skill: Option<i32>| RunValues { owner: 3, category: 1, kind: KIND_ARMY, skill, components: [1, 2, 3, 4] };
    let steps = vec![(100, Some(values(Some(2)))), (110, Some(values(Some(2)))), (120, Some(values(Some(3)))), (130, None), (140, Some(values(None)))];
    assert_eq!(career_runs(&steps, 150), vec![(100, 119, values(Some(2))), (120, 129, values(Some(3))), (140, 150, values(None))]);
}

#[test]
fn a_formation_is_commanded_while_its_leader_column_names_the_leader() {
    let steps = vec![(10, Some(9)), (15, Some(9)), (20, Some(4)), (30, None), (35, Some(9))];
    assert_eq!(led_spans(&steps, 9, 50), vec![(10, 19), (35, 50)]);
    assert!(led_spans(&steps, 1, 50).is_empty());
}

#[test]
fn a_participation_takes_the_first_result_of_its_combat_read_after_it() {
    let participation = |read: i32, combat: u64, unknown: bool| (read, Battle::PARTICIPATION, Battle::LAND, combat, read - 2, unknown, None, None);
    let result = |read: i32, combat: u64, outcome: Option<u8>| (read, Battle::RESULT, Battle::LAND, combat, read, false, outcome, Some(read - 1));
    let rows = vec![participation(10, 1, true), participation(12, 2, false), result(15, 1, Some(Battle::WON)), result(16, 2, None), participation(20, 1, false), result(22, 1, Some(Battle::LOST))];
    let battles = join_battles(&rows);
    assert_eq!(battles.len(), 3);
    assert_eq!((battles[0].start, battles[0].unknown, battles[0].won, battles[0].result_hour, battles[0].result_read), (8, true, Some(true), Some(14), Some(15)));
    assert_eq!((battles[1].won, battles[1].result_hour), (None, None), "a result with no outcome leaves the battle undecided");
    assert_eq!((battles[2].read, battles[2].won), (20, Some(false)), "a reused combat handle takes the result read after it");
}

#[test]
fn kinds_and_countries_choose_leaders_by_kind_and_owner_tag() {
    assert!(LeaderKinds::All.includes(KIND_ARMY) && LeaderKinds::All.includes(KIND_NAVY));
    assert!(!LeaderKinds::Land.includes(KIND_NAVY) && LeaderKinds::Navy.includes(KIND_NAVY));
    let countries = LeaderSelection::Countries { countries: vec!["GER".into()] };
    assert!(countries.keeps(Some((7, "GER"))));
    assert!(!countries.keeps(None), "a leader whose owner is not named is in no country");
    let slot = LeaderSelection::Countries { countries: vec![dashboard_base::products::Country::Slot(7)] };
    assert!(slot.keeps(Some((7, "DEU"))) && !slot.keeps(Some((9, "GER"))));
    assert!(LeaderSelection::All.keeps(None));
}

#[test]
fn a_leaders_traits_are_listed_in_their_recorded_order() {
    let row = |leader: u64, ordinal: u32, token: &str| UnitLeaderTraitRow { leader_reference: leader, ordinal, token: token.as_bytes().to_vec(), ..Default::default() };
    let held = held_traits(&[row(2, 1, "b"), row(2, 0, "a"), row(3, 0, "c")]);
    assert_eq!(held[&2], vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(held[&3], vec!["c".to_owned()]);
}

#[test]
fn a_leaders_battles_from_its_totals_sum_its_handles_and_runs() {
    let run = |battles: Vec<(u64, [u64; 3])>| super::RunTally { battles, dates: HashMap::new() };
    let mut tally = Tally::default();
    tally.fold(run(vec![(7 << 32 | 1, [3, 2, 1]), (7 << 32 | 2, [1, 0, 0]), (8 << 32 | 1, [2, 0, 2])]), Some(&run(vec![(7 << 32 | 1, [1, 1, 0])])), 50);
    tally.fold(run(vec![(7 << 32 | 1, [2, 1, 1])]), None, 100);
    let seven = tally.battles(7);
    assert_eq!((seven.fought, seven.won, seven.lost), (5, 2, 2));
    let eight = tally.battles(8);
    assert_eq!((eight.fought, eight.won, eight.lost), (2, 0, 2));
}

#[test]
fn trait_dates_from_their_table_are_the_fold_of_the_gain_rows_over_runs() {
    use recorder_derive_framework::{framework::Writers, testing};
    use recorder_derive_land::leader_traits::LeaderTraits;
    use replay_columnar::{manifest::Status, store::Store, unit::Discard};
    use replay_data::rows::{TableId, UnitLeaderRow};
    const LEADER: u64 = 11 << 32 | 5;
    let texts: &[&[u8]] = &[b"", b"t1", b"t2"];
    let hour = |store: &mut Store, writers: &mut Writers, at: i32, first: bool, held: &[i32], logged: &mut Vec<((u64, String), i32, u8)>| {
        store.begin_hour(at, if first { texts } else { &[] }).unwrap();
        for table in [TableId::UnitLeaderTraits, TableId::UnitLeaders] {
            store.status(table.index(), Status::Complete, true, Some(at), true).unwrap();
        }
        testing::put(store, TableId::UnitLeaders, &UnitLeaderRow { id: (LEADER >> 32) as u32, reference_type: LEADER as u32, ..Default::default() }, texts).unwrap();
        for (key, _) in testing::rows(store, TableId::UnitLeaderTraits) {
            if key as u32 as usize >= held.len() {
                testing::remove(store, TableId::UnitLeaderTraits, key).unwrap();
            }
        }
        for (ordinal, &token_id) in held.iter().enumerate() {
            let row = UnitLeaderTraitRow { leader_reference: LEADER, ordinal: ordinal as u32, token_id, definition_ordinal: token_id, token: format!("t{token_id}").into_bytes() };
            testing::put(store, TableId::UnitLeaderTraits, &row, texts).unwrap();
        }
        store.settle();
        writers.hour(1, at, first, store).unwrap();
        let view = store.view(TableId::LeaderTraitGains.index());
        for (key, _) in testing::rows(store, TableId::LeaderTraitGains) {
            let slot = view.slot(key).unwrap();
            let spelling = String::from_utf8(view.text(1, slot).and_then(|id| view.string(id)).unwrap().to_vec()).unwrap();
            logged.push((((key >> 64) as u64, spelling), at, view.number(0, slot).unwrap() as u8));
        }
        store.end_hour(&mut Discard).unwrap();
    };
    let mut logged = Vec::new();
    let mut tally = Tally::default();
    for (first, hours) in [(0, vec![vec![1], vec![1, 2], vec![2], vec![2, 1]]), (10, vec![vec![1], vec![1, 2]])] {
        let mut store = testing::store();
        let mut writers = Writers::new(vec![Box::new(LeaderTraits)]);
        writers.start_run();
        for (at, held) in hours.iter().enumerate() {
            hour(&mut store, &mut writers, first + at as i32, at == 0, held, &mut logged);
        }
        let through = super::RunTally::of(&store.view(TableId::LeaderBattleTotals.index()), &store.view(TableId::LeaderTraitDates.index()), super::Asked::All);
        tally.fold(through, None, first);
    }
    let mut folded = HashMap::new();
    for (key, at, class) in logged {
        fold_date(&mut folded, key, at, class);
    }
    assert_eq!(tally.dates, folded);
    assert_eq!(tally.date(LEADER, "t1"), Some((3, Gain::GAINED)), "regained in the first run; the second run's held-before moves nothing");
    assert_eq!(tally.date(LEADER, "t2"), Some((11, Gain::GAINED)));
}

#[test]
fn a_leader_whose_role_may_have_conditions_reads_its_character_first_and_the_trigger_tables_only_for_conditions_to_evaluate() {
    use super::{LeadersContext, UnitLeaders, UnitLeadersParams};
    use dashboard_base::products::{LoadContext, at_hour::{AtHour, ReadsAt}};
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay_data::rows::{TableId, TableRows, UnitLeaderRow};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (100..=101).map(|hour| source(1, hour, Some(1_000))).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    let recording = dashboard_base::Source::from_replay(dashboard_base::SourceKey { source_id: "leader-reads".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let run = dashboard_base::queries::query(&recording, 1).unwrap().timeline.stretches[0].run;
    let context = LeadersContext::load(&recording, &LoadContext { content: None }).unwrap();
    let mut held = source(1, 100, Some(1_000));
    let leader = UnitLeaderRow { id: 1, kind: KIND_ARMY, owner: 11, character_id: 5, visibility_choice: 1, ..Default::default() };
    held.tables[TableId::UnitLeaders.index()] = TableView::Complete(TableRows::UnitLeaders(vec![leader])).into();
    let params = UnitLeadersParams { branch_id: "1".into(), selection: LeaderSelection::All, leaders: LeaderKinds::All };
    let holders = dashboard_query::identity::Holders::new(Default::default(), Vec::new());
    let at = ReadsAt { holders: &holders, context: &context, run, hour: 100, since: None };
    let first: Vec<TableId> = UnitLeaders::reads_in(&params, None, 1, &held, &at).iter().map(|read| read.table).collect();
    assert_eq!(first, [TableId::Characters, TableId::LexicalTokens], "the leader's character and the tokens, not every trigger table");
    assert!(UnitLeaders::reads_in(&params, None, 2, &held, &at).is_empty(), "without the content no role is known to have conditions, and no table read would decide one");
}

#[test]
fn leader_hour_reads_and_names_the_requested_leader_not_the_first_row_in_a_shared_frame() {
    use super::{
        LeadersContext,
        career::{LeaderHour, LeaderHourParams},
        handle,
    };
    use dashboard_base::products::{
        LoadContext,
        at_hour::{At, AtHour, Keys},
    };
    use dashboard_base::test_fixtures::{records, source};
    use replay_data::rows::{CharacterRow, TableId, TableRows, UnitLeaderRow};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};
    let first_leader = UnitLeaderRow {
        id: 1,
        reference_type: 1,
        kind: KIND_ARMY,
        owner: 9001,
        character_id: 10,
        visibility_choice: 1,
        ..Default::default()
    };
    let second_leader = UnitLeaderRow {
        id: 2,
        reference_type: 1,
        kind: KIND_ARMY,
        owner: 9001,
        character_id: 20,
        visibility_choice: 1,
        ..Default::default()
    };
    let mut frame = source(1, 100, Some(1_000));
    frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![first_leader, second_leader.clone()])).into();
    frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(vec![
        CharacterRow { id: 10, token: b"ABK_zaitsev".to_vec(), ..Default::default() },
        CharacterRow { id: 20, token: b"JAP_yamamoto".to_vec(), ..Default::default() },
    ]))
    .into();
    let params = LeaderHourParams {
        branch_id: "1".into(),
        reference: handle(&second_leader).to_string(),
    };
    let step1 = LeaderHour::reads(&params, None, 1, &frame);
    assert_eq!(step1.len(), 1);
    assert_eq!(step1[0].table, TableId::Characters);
    match &step1[0].keys {
        Keys::These(keys) => assert_eq!(keys, &[20u128], "must request the asked leader's character_id, not row 0"),
        other => panic!("expected Keys::These([20]), got {other:?}"),
    }

    let (_, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
        ("localisation/languages.yml", "\u{feff}l_english:\n l_english: \"English\"\n"),
        (
            "localisation/english/characters_l_english.yml",
            "\u{feff}l_english:\n ABK_zaitsev:0 \"Anatoly Zaitsev\"\n JAP_yamamoto:0 \"Isoroku Yamamoto\"\n",
        ),
        (
            "common/characters/test.txt",
            "characters = { ABK_zaitsev = { name = ABK_zaitsev } JAP_yamamoto = { name = JAP_yamamoto } }",
        ),
        ("common/defines/test.lua", "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { } } }"),
    ]);
    let (content, identity) = dashboard_base::test_fixtures::content_identity(&folder.0);
    let mut input = records(1, vec![frame.clone()]);
    if let replay_capture_spool::spool::Record::SessionStart(session) = &mut input[0] {
        session.identity = identity.clone();
    }
    let replay = dashboard_base::test_fixtures::with_facts(&content, &identity, input)
        .into_timelines(&Default::default())
        .unwrap();
    let recording = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey { source_id: "leader-hour-shared".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LeadersContext::load(&recording, &LoadContext { content: Some(content) }).unwrap();
    let holders = dashboard_query::identity::Holders::new(Default::default(), Vec::new());
    let live = dashboard_follow::live::Cells::default();
    let cancelled = AtomicBool::new(false);
    let at = At {
        owner: None,
        holders: &holders,
        context: &context,
        hour: 100,
        cancelled: &cancelled,
        live: &live,
    };
    let reply = LeaderHour::reply(&frame, &params, &at).unwrap();
    assert_eq!(reply.name.as_deref(), Some("Isoroku Yamamoto"), "{:?}", reply.warnings);
}

#[test]
fn skill_icons_are_the_installed_leader_windows_buttons_and_a_missing_one_is_named() {
    engine_executable::executable_tables::initialize_for_tests().expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use content_reader_gui::gui::native;
    let gui = native::read(
        br#"guiTypes = { containerWindowType = {
        name = "armyleaderwindow"
        buttonType = { name = "sort_skills" spriteType = "GFX_mod_skill" }
        buttonType = { name = "sort_attack_skill" spriteType = "GFX_mod_attack" }
        buttonType = { name = "sort_defense_skill" spriteType = "GFX_mod_defense" }
        buttonType = { name = "sort_supply_skill" spriteType = "GFX_mod_supply" }
    } }"#,
    );
    let guis = native::Guis::new(vec![("interface/unitleaderwindow.gui".into(), gui)]);
    let mut warnings = Vec::new();
    let icons = super::window_icons(Ok(&guis), "armyleaderwindow", super::LAND_SKILL_BUTTONS, &mut warnings);
    assert_eq!(
        icons.iter().map(|icon| icon.as_ref().map(|icon| (icon.key.as_str(), icon.frame, icon.text))).collect::<Vec<_>>(),
        [Some(("GFX_mod_skill", 0, None)), Some(("GFX_mod_attack", 0, None)), Some(("GFX_mod_defense", 0, None)), None, Some(("GFX_mod_supply", 0, None))],
        "a mod's own sprites, in skill, attack, defense, planning, logistics order"
    );
    assert!(warnings.iter().any(|warning| warning.contains("sort_planning_skill")), "the missing button is named: {warnings:?}");
    let refused = super::window_icons(Err("Game content is not configured".into()), "navyleaderwindow", super::NAVY_SKILL_BUTTONS, &mut warnings);
    assert!(refused.iter().all(Option::is_none));
}
