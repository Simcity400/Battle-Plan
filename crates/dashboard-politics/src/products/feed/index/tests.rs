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

#[test]
fn card_edges_date_membership_at_effective_hour_and_gate_admission_and_search_by_observation() {
    use dashboard_query::{
        identity::{Country, Holder, HolderRun, Holders},
        log::{CountrySpan, occurrence::{Ask, Direction, Order, Selection}},
    };
    let held = |slot: u32| vec![HolderRun { first: 0, last: 1_000, holder: Holder::Found { slot, id: slot }, starts_segment: true }];
    let holders = Holders::new([("GER".to_owned(), held(7)), ("POL".to_owned(), held(8))].into_iter().collect(), Vec::new());
    let opener = brief(1, EventFeedKind::DecisionSelected, 100, None, Some(30));
    let later = Brief {
        at: Locator { run: 1, hour: 110, key: (1u128 << 64) | 2 },
        kind: EventFeedKind::DecisionDurationChanged,
        generation: 1,
        sequence: 2,
        hour: 105,
        opener: Some(1),
        slots: [Some(7), Some(8), None],
        option: None,
        days: Some(40),
        token: Some("mission_b".into()),
    };
    let scanned = Scanned {
        briefs: vec![opener, later],
        native: Vec::new(),
        incomplete: Vec::new(),
        windows_unrecorded: Vec::new(),
        states: HashMap::new(),
        resolved: Mutex::default(),
        presented: Mutex::default(),
        breaks: Vec::new(),
        decision_breaks: Vec::new(),
        partial_reads: Vec::new(),
        gaps: Vec::new(),
    };
    let index = decisions(&scanned, &Names::new(&holders)).unwrap();
    let ask_at = |hour: i32| Ask { hour, from: None, to: None, gap_end: None, selection: None, subject: None, kinds: None, states: None, search: None, order: Order::Date(Direction::Descending), live_first: false, offset: 0, limit: 10, chips: &[] };
    let poland = |first_hour: i32, last_hour: i32| Selection {
        membership: vec![CountrySpan { country: Country::Tag("POL".into()), first_hour, last_hour }],
        unobserved: Vec::new(),
        unknown: Vec::new(),
    };
    let member_at_105 = poland(100, 108);
    let at_105 = index.ask(&holders, &Ask { selection: Some(&member_at_105), ..ask_at(105) }).unwrap();
    assert_eq!((at_105.total, at_105.next_change), (0, Some(110)), "later edge dated 105 and read at 110 does not admit at 105");
    let at_110 = index.ask(&holders, &Ask { selection: Some(&member_at_105), ..ask_at(110) }).unwrap();
    assert_eq!(at_110.total, 1, "admitted at 110 using effective-hour 105 membership");
    let joined_at_108 = poland(108, 200);
    assert_eq!(index.ask(&holders, &Ask { selection: Some(&joined_at_108), ..ask_at(110) }).unwrap().total, 0, "later membership at 110 does not admit an edge dated 105");
    let search_before = index.ask(&holders, &Ask { search: Some("POL"), ..ask_at(109) }).unwrap();
    assert_eq!((search_before.total, search_before.next_change), (0, Some(110)), "target tag from edge read at 110 is not searchable before 110");
    assert_eq!(index.ask(&holders, &Ask { search: Some("POL"), ..ask_at(110) }).unwrap().total, 1);
    assert_eq!(index.ask(&holders, &Ask { search: Some("mission_b"), ..ask_at(109) }).unwrap().total, 0);
    assert_eq!(index.ask(&holders, &Ask { search: Some("mission_b"), ..ask_at(110) }).unwrap().total, 1);
    assert_eq!(index.ask(&holders, &Ask { search: Some("mission_a"), ..ask_at(100) }).unwrap().total, 1);

    let mut swapped = brief(3, EventFeedKind::DecisionSelected, 100, None, None);
    swapped.slots = [Some(8), Some(7), None];
    let swapped_scanned = Scanned { briefs: vec![swapped], ..scanned };
    let swapped_index = decisions(&swapped_scanned, &Names::new(&holders)).unwrap();
    assert_eq!(swapped_index.ask(&holders, &Ask { search: Some("GER POL"), ..ask_at(100) }).unwrap().total, 1, "tags observed together keep sorted order in search text");
}
