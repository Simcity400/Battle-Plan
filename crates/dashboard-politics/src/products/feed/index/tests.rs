use super::*;

fn brief(sequence: u64, kind: EventFeedKind, hour: i32, opener: Option<u64>, days: Option<i32>) -> Brief {
    Brief {
        at: Locator { run: 1, hour, key: (1u128 << 64) | u128::from(sequence) },
        kind,
        generation: 1,
        sequence,
        hour,
        opener,
        slots: [Some(7), None, None],
        option: None,
        days,
        token: Some("mission_a".into()),
    }
}

fn statuses(briefs: &[Brief], states: &HashMap<u128, DecisionLifecycleState>, hours: &[i32]) -> Vec<Vec<ScriptStatus>> {
    let list = super::super::cards::List::Decisions;
    let found = cards(briefs, list.openers(), list.continuing());
    let later = superseded(&found);
    found.iter().zip(later).map(|(card, later)| hours.iter().map(|&hour| decision_status(card, states, &[], later, hour)).collect()).collect()
}

#[test]
fn only_the_latest_open_activation_runs_and_an_unwitnessed_end_is_not_recorded() {
    use EventFeedKind::DecisionActivated;
    let briefs = [brief(1, DecisionActivated, 100, None, None), brief(2, DecisionActivated, 400, None, None)];
    let seen = statuses(&briefs, &HashMap::new(), &[150, 400, 500]);
    assert_eq!(seen[0], [ScriptStatus::Active, ScriptStatus::NotRecorded, ScriptStatus::NotRecorded], "a later activation of the same decision closes the earlier, whose end was not witnessed");
    assert_eq!(seen[1][1..], [ScriptStatus::Active, ScriptStatus::Active]);
    let mut other = brief(3, DecisionActivated, 400, None, None);
    other.token = Some("mission_b".into());
    let apart = statuses(&[brief(1, DecisionActivated, 100, None, None), other], &HashMap::new(), &[500]);
    assert_eq!(apart[0], [ScriptStatus::Active], "another decision closes nothing");
}

#[test]
fn a_timer_that_passes_without_a_recorded_end_is_not_recorded() {
    let briefs = [brief(1, EventFeedKind::DecisionSelected, 100, None, Some(10))];
    let seen = statuses(&briefs, &HashMap::new(), &[100, 339, 340]);
    assert_eq!(seen[0], [ScriptStatus::Active, ScriptStatus::Active, ScriptStatus::NotRecorded]);
    let ended = [brief(1, EventFeedKind::DecisionSelected, 100, None, Some(10)), brief(2, EventFeedKind::DecisionTimedOut, 340, Some(1), None)];
    assert_eq!(statuses(&ended, &HashMap::new(), &[340])[0], [ScriptStatus::TimedOut], "a witnessed end is the end");
}

#[test]
fn an_untargeted_pool_availability_is_not_a_running_decision() {
    let activated = brief(1, EventFeedKind::DecisionActivated, 100, None, None);
    let states = HashMap::from([(activated.at.key, DecisionLifecycleState::Available)]);
    assert_eq!(statuses(&[activated], &states, &[120])[0], [ScriptStatus::Available]);
    let mission = brief(2, EventFeedKind::DecisionActivated, 110, Some(1), None);
    let selected_states = HashMap::from([(mission.at.key, DecisionLifecycleState::Available)]);
    let chosen = [brief(1, EventFeedKind::DecisionSelected, 100, None, None), mission];
    assert_ne!(statuses(&chosen, &selected_states, &[120])[0], [ScriptStatus::Available], "a selected one is a decision taken");
    assert!(!super::LIVE.contains(&ScriptStatus::Available.code()));
}

#[test]
fn a_break_in_the_read_that_brought_an_edge_leaves_its_card_unwitnessed_from_that_hour() {
    let briefs = [brief(1, EventFeedKind::DecisionActivated, 105, None, None)];
    let list = super::super::cards::List::Decisions;
    let found = cards(&briefs, list.openers(), list.continuing());
    let at = |breaks: &[(i32, i32)], hour: i32| decision_status(&found[0], &HashMap::new(), breaks, None, hour);
    assert_eq!(
        [at(&[(105, 105)], 105), at(&[], 105), at(&[(106, 106)], 105), at(&[(106, 106)], 106), at(&[(100, 104)], 105)],
        [ScriptStatus::NotRecorded, ScriptStatus::Active, ScriptStatus::Active, ScriptStatus::NotRecorded, ScriptStatus::Active],
        "the read that delivered the activation may have lost its end; a break before it is none of its"
    );
}

#[test]
fn a_history_that_cannot_be_read_is_one_missing_span_and_no_card_is_open_through_it() {
    let scanned = Scanned::refused(100, 200, "damaged");
    let index = decisions(&scanned, &Names { slots: HashMap::new() }).unwrap();
    let gaps: Vec<_> = index.gaps().iter().map(|gap| (gap.first_hour, gap.last_hour, gap.reason.as_str())).collect();
    assert_eq!(gaps, [(100, 200, "The event history could not be read: damaged")]);
    assert_eq!(scanned.decision_breaks, [(100, 200)]);
}
