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
    assert!(LeaderKinds::Land.includes(KIND_ARMY) && !LeaderKinds::Land.includes(KIND_NAVY));
    assert!(LeaderKinds::Navy.includes(KIND_NAVY) && !LeaderKinds::Navy.includes(KIND_ARMY));
    let countries = LeaderSelection::Countries { countries: vec!["GER".into()] };
    assert!(countries.keeps(Some(7), Some("GER"), None, 100));
    assert!(!countries.keeps(None, None, None, 100), "a leader whose owner is not named is in no country");
    let slot = LeaderSelection::Countries { countries: vec![dashboard_base::products::Country::Slot(7)] };
    assert!(slot.keeps(Some(7), Some("DEU"), None, 100) && !slot.keeps(Some(9), Some("GER"), None, 100));
    assert!(LeaderSelection::All.keeps(None, None, None, 100));
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
    let params = UnitLeadersParams { branch_id: "1".into(), selection: LeaderSelection::All, leaders: LeaderKinds::Land };
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
        skill_valid: 1,
        skill_level: 2,
        ..Default::default()
    };
    let second_leader = UnitLeaderRow {
        id: 2,
        reference_type: 1,
        kind: KIND_ARMY,
        owner: 9001,
        character_id: 20,
        skill_valid: 1,
        skill_level: 4,
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
        reference: Some(handle(&second_leader).to_string()),
        selection: None,
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
    let top = LeaderHourParams { branch_id: "1".into(), reference: None, selection: Some(LeaderSelection::Countries { countries: vec!["GER".into()] }) };
    let reply = LeaderHour::reply(&frame, &top, &at).unwrap();
    assert_eq!(reply.reference, Some(handle(&second_leader).to_string()), "the top general is the selection's most skilled army leader: {:?}", reply.missing);
    assert_eq!(reply.name.as_deref(), Some("Isoroku Yamamoto"));
    let nobody = LeaderHourParams { branch_id: "1".into(), reference: None, selection: Some(LeaderSelection::Countries { countries: vec!["ITA".into()] }) };
    let reply = LeaderHour::reply(&frame, &nobody, &at).unwrap();
    assert_eq!((reply.reference, reply.missing), (None, None), "a selection with no generals has no top general, and nothing is missing");
}

#[test]
fn skill_icons_are_the_installed_leader_windows_buttons_and_a_missing_one_is_named() {
    dashboard_core::test_fixtures::game::enter();
    use content_reader_gui::gui::native;
    let gui = native::read(
        br#"guiTypes = {
            containerWindowType = {
                name = "unitleader_trait_tree_window"
                iconType = { name = "skill_bg" spriteType = "GFX_mod_skill" }
            }
            containerWindowType = {
                name = "armyleaderentry"
                containerWindowType = {
                    name = "skill_container"
                    iconType = { name = "leader_stat_attack_bg" spriteType = "GFX_mod_attack" }
                    iconType = { name = "leader_stat_defense_bg" spriteType = "GFX_mod_defense" }
                    iconType = { name = "leader_stat_logistics_bg" spriteType = "GFX_mod_logistics" }
                }
            }
        }"#,
    );
    let guis = native::Guis::new(vec![("interface/unitleaderwindow.gui".into(), gui)]);
    let mut warnings = Vec::new();
    let icons = super::window_icons(Ok(&guis), super::LAND_SKILL_ICONS, &mut warnings);
    assert_eq!(
        icons.iter().map(|icon| icon.as_ref().map(|icon| (icon.key.as_str(), icon.frame, icon.text))).collect::<Vec<_>>(),
        [Some(("GFX_mod_skill", 0, None)), Some(("GFX_mod_attack", 0, None)), Some(("GFX_mod_defense", 0, None)), None, Some(("GFX_mod_logistics", 0, None))],
        "a mod's own box-free icons, in skill, attack, defense, planning, logistics order"
    );
    assert!(warnings.iter().any(|warning| warning.contains("leader_stat_planning_bg")), "the missing icon is named: {warnings:?}");
    let refused = super::window_icons(Err("Game content is not configured".into()), super::NAVY_SKILL_ICONS, &mut warnings);
    assert!(refused.iter().all(Option::is_none));
}

#[test]
fn leaders_order_by_skill_then_battles_won_and_fought() {
    let row = |reference: &str, name: &str, skill: Option<i32>, won: u64, fought: u64, components: [i32; 4]| super::LeaderRow {
        reference: reference.into(),
        name: Some(name.into()),
        role: super::LeaderRole { owner: Some("JAP".into()), kind: 1, category: 0, rank: Some("General".into()), attribute_names: [None, None, None, None], maximum_skill: Some(9) },
        skill,
        components,
        traits: Some(Vec::new()),
        trait_dates: Vec::new(),
        first_seen_hour: Some(0),
        fought,
        won,
        lost: fought.saturating_sub(won),
    };
    let mut rows = vec![
        row("2", "Hisaichi Terauchi", Some(7), 831, 1643, [10, 6, 8, 2]),
        row("3", "Toshizo Nishio", Some(6), 0, 0, [7, 4, 9, 5]),
        row("1", "Tomoyuki Yamashita", Some(7), 1071, 1876, [9, 5, 7, 6]),
    ];
    super::sort_leaders(&mut rows);
    assert_eq!(rows.iter().map(|r| r.reference.as_str()).collect::<Vec<_>>(), ["1", "2", "3"]);
}

#[test]
fn leader_career_cuts_at_hour_computes_next_change_and_resolves_selection() {
    use super::career::{LeaderCareer, LeaderCareerParams};
    use dashboard_base::{
        Source,
        products::{LoadContext, Product},
        test_fixtures::{CAMPAIGN, snapshot_records, source},
    };
    use replay::campaign::Replay;
    use replay_data::rows::{CharacterRow, CountryLinkRow, CountryRow, LexicalTokenRow, Recorded, TableId, TableRows, UnitLeaderRow};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    let frames: Vec<_> = (100..=120)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1_000));
            frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 9001,
                canonical_slot: 7,
                tag: b"JAP".to_vec(),
                border_land_doctrine_level: Recorded::Value(None),
                ..Default::default()
            }]))
            .into();
            frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow { id: 9001, canonical_slot: 7 }])).into();
            let leader = UnitLeaderRow {
                id: 42,
                reference_type: 1,
                kind: KIND_ARMY,
                owner: 9001,
                character_id: if hour < 118 { 77 } else { 79 },
                skill_valid: 1,
                skill_level: if hour < 110 { 3 } else { 5 },
                components: [2, 2, 2, 2],
                ..Default::default()
            };
            let rival = UnitLeaderRow { id: 43, character_id: 78, skill_level: 5, owner: 9001, components: if hour < 117 { [1; 4] } else { [3; 4] }, ..leader.clone() };
            frame.tables[TableId::UnitLeaders.index()] = TableView::Complete(TableRows::UnitLeaders(vec![leader, rival])).into();
            frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(vec![
                CharacterRow { id: 77, token: b"JAP_yamashita".to_vec(), ..Default::default() },
                CharacterRow { id: 78, token: b"JAP_homma".to_vec(), ..Default::default() },
                CharacterRow { id: 79, token: b"JAP_yamashita".to_vec(), ..Default::default() },
            ]))
            .into();
            frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 0, literal: if hour < 119 { b"yamashita".to_vec() } else { b"yamashita_tomoyuki".to_vec() } }])).into();
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[snapshot_records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(
        dashboard_base::SourceKey { source_id: "career-at".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let load = LoadContext { content: None };
    let cancelled = AtomicBool::new(false);
    let handle_ref = ((42u64 << 32) | 1u64).to_string();

    let early = LeaderCareer::load(
        &src,
        &LeaderCareerParams { branch_id: "1".into(), reference: Some(handle_ref.clone()), selection: None, hour: 105 },
        &load,
        &cancelled,
    )
    .unwrap();
    assert_eq!((early.hour, early.next_change), (105, Some(110)));
    assert_eq!(early.runs.len(), 1);
    assert_eq!((early.runs[0].first_hour, early.runs[0].last_hour, early.runs[0].skill), (100, 105, Some(3)));
    assert_eq!(early.portrait_hour, Some(105));
    assert_eq!(early.observed, vec![(100, 105)]);
    let ask = |hour| LeaderCareer::load(&src, &LeaderCareerParams { branch_id: "1".into(), reference: Some(handle_ref.clone()), selection: None, hour }, &load, &cancelled).unwrap();
    let reads = || super::career::FULL_READS.with(std::cell::Cell::get);
    let before = reads();
    ask(111);
    ask(112);
    assert_eq!(reads() - before, 0, "a later hour cuts the career already read for this leader and branch");

    let late = LeaderCareer::load(
        &src,
        &LeaderCareerParams { branch_id: "1".into(), reference: None, selection: Some(LeaderSelection::All), hour: 115 },
        &load,
        &cancelled,
    )
    .unwrap();
    assert_eq!(late.reference.as_deref(), Some(handle_ref.as_str()));
    assert_eq!((late.hour, late.next_change), (115, Some(117)), "a change in what ranks the generals ends the top general's career");
    assert_eq!(late.runs.len(), 2);
    assert_eq!(late.runs[1].skill, Some(5));
    let overtaken = LeaderCareer::load(&src, &LeaderCareerParams { branch_id: "1".into(), reference: None, selection: Some(LeaderSelection::All), hour: 117 }, &load, &cancelled).unwrap();
    assert_eq!(overtaken.reference, Some(((43u64 << 32) | 1).to_string()), "the tie of equal skill turns to the rival once its components are higher");
    assert_eq!(overtaken.next_change, Some(118), "a general's character link is an input of its name, so of the order");
    let relinked = LeaderCareer::load(&src, &LeaderCareerParams { branch_id: "1".into(), reference: None, selection: Some(LeaderSelection::All), hour: 118 }, &load, &cancelled).unwrap();
    assert_eq!(relinked.next_change, Some(119), "so is the spelling of its name token");
}

#[test]
fn top_general_follows_recorded_faction_membership() {
    use super::career::{LeaderCareer, LeaderCareerParams};
    use dashboard_base::{Source, SourceKey, products::{LoadContext, Product}, test_fixtures::{CAMPAIGN, ownership_frame, snapshot_records}};
    use replay::campaign::Replay;
    use replay_data::rows::{FactionMemberRow, FactionRow, LandCombatDetailsRow, LandCombatSideDetails, TableId, TableRows, UnitLeaderRow, UnitLeaderTraitExperienceRow};
    use replay_frame::frame::TableView;
    use serde_json::json;
    use std::sync::{Arc, atomic::AtomicBool};

    let leader = |id, owner, skill_level| UnitLeaderRow { id, owner, skill_level, skill_valid: 1, kind: KIND_ARMY, ..Default::default() };
    let home = leader(1, 9001, 2);
    let visitor = leader(2, 44, 4);
    let frames: Vec<_> = (150..=255).map(|hour| {
        let mut frame = ownership_frame(1, hour, 9001);
        let mut members = vec![FactionMemberRow { faction: 9, country: 9001, ordinal: 0 }];
        if (200..251).contains(&hour) {
            members.insert(0, FactionMemberRow { faction: 9, country: 44, ordinal: 1 });
        }
        *frame.tables[TableId::FactionMembers.index()] = if hour == 253 {
            TableView::Unavailable { reason: "roster unavailable".into() }
        } else {
            TableView::Complete(TableRows::FactionMembers(members))
        };
        *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(vec![FactionRow { id: 9, ..Default::default() }]));
        for table in [TableId::NavalCombatDetails, TableId::UnitLeaderTraits, TableId::Characters, TableId::LexicalTokens] {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
        let combat = LandCombatDetailsRow { combat: 7, sides: [LandCombatSideDetails { commander: super::handle(&visitor), ..Default::default() }, Default::default()], ..Default::default() };
        *frame.tables[TableId::LandCombatDetails.index()] = TableView::Complete(TableRows::LandCombatDetails(if hour >= 175 { vec![combat] } else { vec![] }));
        *frame.tables[TableId::UnitLeaderTraitExperience.index()] = TableView::Complete(TableRows::UnitLeaderTraitExperience(vec![
            UnitLeaderTraitExperienceRow { leader_reference: super::handle(&visitor), experience: i64::from(hour), ..Default::default() },
        ]));
        let skill_level = if hour == 225 { 1 } else { visitor.skill_level };
        *frame.tables[TableId::UnitLeaders.index()] = TableView::Complete(TableRows::UnitLeaders(vec![
            UnitLeaderRow { experience: i64::from(hour), ..home.clone() },
            UnitLeaderRow { skill_level, captured: u8::from(hour == 232), ..visitor.clone() },
        ]));
        frame
    }).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[snapshot_records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(SourceKey { source_id: "faction-career".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &cancelled).unwrap();
    let load = LoadContext { content: None };
    let home = super::handle(&home).to_string();
    let visitor = super::handle(&visitor).to_string();
    let unknown = Some(dashboard_world::products::group_members::UNKNOWN);
    for membership in ["hour", "ever"] {
        let selection = json!({"kind":"group","group":{"type":"faction","id":"9","membership":membership}});
        let params = json!({"branchId":"1","selection":selection});
        let panel = crate::registrations::EXACT_LEADERHOUR(&source, &load, &params).unwrap();
        let selection: LeaderSelection = serde_json::from_value(selection).unwrap();
        let ever = membership == "ever";
        let exact = panel.exact().unwrap();
        let advance = exact.advance.clone();
        let follow = dashboard_follow::panel::AtHour {
            cold: exact.cold.clone(), read: exact.read.clone(),
            advance: Arc::new(move |hour, values, cancelled, step| {
                let mut reply = advance(hour, values, cancelled, step);
                reply.lanes.sort_by_key(|lane| lane.table);
                reply.lanes.dedup_by(|second, first| {
                    if first.table != second.table { return false; }
                    first.columns.extend(&second.columns);
                    first.columns.sort_unstable();
                    first.columns.dedup();
                    true
                });
                reply
            }),
        };
        let played = dashboard_base::test_fixtures::played(&source.replay(), &follow, 150, 255);
        let ranks: Vec<_> = played.iter().filter(|read| read.hour > 150 && read.visited > 0).map(|read| read.hour).collect();
        let changes = if ever { vec![200, 225, 226, 232, 233] } else { vec![200, 225, 226, 232, 233, 251, 254] };
        assert_eq!(ranks, changes, "{membership} follow ranks only at changes with a known roster");
        for (hour, winner, missing, next) in [
            (150, Some(&home), None, Some(200)), (200, Some(&visitor), None, Some(225)), (225, Some(&home), None, Some(226)),
            (226, Some(&visitor), None, Some(232)), (232, Some(&home), None, Some(233)),
            (250, Some(&visitor), None, Some(if ever { 253 } else { 251 })), (251, Some(if ever { &visitor } else { &home }), None, Some(253)),
            (253, None, unknown, if ever { None } else { Some(254) }), (254, if ever { None } else { Some(&home) }, if ever { unknown } else { None }, None),
        ] {
            let head = (exact.cold)(hour, &cancelled).unwrap();
            if hour == 200 { assert_eq!(head["fought"], 1, "the visitor fought before joining"); }
            let followed = played.iter().find(|read| read.hour == hour).unwrap().value.as_ref().as_ref().unwrap();
            assert_eq!((&followed["reference"], &followed["missing"]), (&head["reference"], &head["missing"]));
            let params = LeaderCareerParams { branch_id: "1".into(), reference: None, selection: Some(selection.clone()), hour };
            let career = LeaderCareer::load(&source, &params, &load, &cancelled).unwrap();
            assert_eq!((head["reference"].as_str(), head["missing"].as_str()), (winner.map(String::as_str), missing), "{membership} at {hour}");
            assert_eq!((career.reference.as_ref(), career.missing.as_deref(), career.next_change), (winner, missing, next), "{membership} at {hour}");
        }
    }
}
