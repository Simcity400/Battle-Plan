use super::{
    decryption::{CipherState, Observation, attempt},
    placements::{CAPTURED, MISSION, OPERATION, target},
};
use replay_data::rows::{OperativeMembershipRow, Recorded};

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
fn an_operative_leaves_at_its_first_disbanded_or_killed_read_or_the_hour_after_its_last_and_one_still_serving_stays() {
    use super::operative_content::{DISBANDED, KILLED};
    use super::roster::{Leaving, departure, leavings};
    use std::collections::HashMap;
    let lives = [(1, 100, 400), (2, 100, 250), (3, 100, 400), (4, 120, 300)];
    let steps: HashMap<u128, Vec<(i32, Option<u32>)>> = [
        (1, vec![(100, Some(MISSION)), (300, Some(KILLED))]),
        (2, vec![(100, Some(MISSION)), (251, None)]),
        (3, vec![(100, Some(OPERATION))]),
        (4, vec![(120, Some(CAPTURED)), (200, Some(DISBANDED))]),
    ]
    .into_iter()
    .collect();
    let left = leavings(&lives, &steps, 400);
    assert_eq!(left, vec![
        Leaving { key: 1, first: 100, last: 299, departed: 300, assignment: Some(KILLED) },
        Leaving { key: 2, first: 100, last: 250, departed: 251, assignment: Some(MISSION) },
        Leaving { key: 4, first: 120, last: 199, departed: 200, assignment: Some(DISBANDED) },
    ]);
    assert_eq!(left.iter().map(|leaving| departure(leaving.assignment)).collect::<Vec<_>>(), ["Killed", "Left (reason not recorded)", "Disbanded"]);
    assert!(leavings(&lives, &steps, 299).iter().all(|leaving| leaving.key != 1));
}

#[test]
fn captivity_and_cooldown_days_round_as_the_game_does_and_progress_is_never_clamped() {
    use super::operative_content::{capture_progress, cooldown_progress, native_progress};
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
