use super::*;

#[test]
fn content_bound_read_ahead_keeps_all_three_first_log_reads_warm() {
    use dashboard_base::{products::Product, source::Work, test_fixtures::{source, visual_mods::{Machine, RED, png, select}}};
    let machine = Machine::new("feed_index_warm");
    machine.write_mod("art", "gfx/warm.png", &png(RED));
    let frames = (100..=140).map(|hour| source(1, hour, Some(1))).collect();
    let (source, mut options) = machine.open_frames(frames);
    let unbound = LoadContext { content: None };
    warm(&source, 1, &unbound).unwrap();
    assert!(kept(&source).is_empty(), "unavailable content never starts a competing unbound scan");
    for visual_mods in [false, true] {
        select(&source, &mut options, if visual_mods { &["art"] } else { &[] });
        warm(&source, 1, &LoadContext { content: source.admit_resolved(Work::Ahead) }).unwrap();
        assert_eq!(kept(&source), [1], "the source-open warm keeps the branch index");
        let (scanned, decisions, choices, feed) = {
            let branch = branch_of(&source, 1);
            let held = branch.lock().unwrap();
            (held.scanned.clone().unwrap(), held.decisions.clone().unwrap(), held.choices.clone().unwrap(), held.feed.clone().unwrap())
        };
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let context = LoadContext { content: source.admit_resolved(Work::Content(Arc::new(AtomicBool::new(false)))) };
        assert_eq!(!dashboard_base::catalogue::reads_recorded(context.content.as_ref().unwrap()), visual_mods);
        let catalogue = super::super::resolve_content(&source, &context).unwrap();
        let cancelled = AtomicBool::new(false);
        assert!(scanned.titles.lock().unwrap().is_empty());
        let title_index = || {
            let presenter = super::super::text::Presenter::new(&query, Some(&catalogue), super::super::text::kept_naming(&source));
            scanned.title_index(&query, &feed.index, None, TitleKeys::Item, presenter, &cancelled).unwrap()
        };
        assert!(Arc::ptr_eq(&title_index(), &title_index()), "pages share the title index for their base");
        super::super::EventFeedKinds::load(&source, &dashboard_base::products::BranchParams { branch_id: "1".into() }, &context, &cancelled).unwrap();
        let reused = [
            Arc::ptr_eq(&decisions, &decisions_for(&source, &query, 1, Some(&catalogue), &cancelled).unwrap()),
            Arc::ptr_eq(&choices, &choices_for(&source, &query, 1, Some(&catalogue), &cancelled).unwrap()),
            Arc::ptr_eq(&feed, &feed_for(&source, &query, 1, &context, Some(&catalogue), &cancelled).unwrap()),
        ];
        assert_eq!(reused.into_iter().filter(|reused| !reused).count(), 0, "index builds on first log reads after open, visual mods: {visual_mods}");
        warm(&source, 1, &unbound).unwrap();
        assert!(Arc::ptr_eq(&scanned, &scanned_for(&source, &query, 1, Some(&catalogue), &cancelled).unwrap()));
    }
    dashboard_base::queries::forget(&source);
    crate::forget(&source);
    assert!(kept(&source).is_empty(), "forgotten with the recording's index");
}

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
        costs: [None, None],
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
    let index = decisions(&scanned, &Names { slots: HashMap::new() }, &HashMap::new()).unwrap();
    let gaps: Vec<_> = index.gaps().iter().map(|gap| (gap.first_hour, gap.last_hour, gap.reason.as_str())).collect();
    assert_eq!(gaps, [(100, 200, "The event history could not be read: damaged")]);
    assert_eq!(scanned.decision_breaks, [(100, 200)]);
}

#[test]
fn an_orphan_choice_is_ordered_and_ranged_at_its_recorded_observation() {
    use dashboard_query::{identity::Holders, log::occurrence::{Ask, Direction, Order}};
    let mut orphan = brief(1, EventFeedKind::EventOptionSelected, 137, Some(91), None);
    orphan.hour = 119;
    orphan.option = Some(0);
    let mut scanned = Scanned::refused(100, 200, "unused");
    scanned.gaps.clear();
    scanned.breaks.clear();
    scanned.briefs = vec![orphan, brief(2, EventFeedKind::ScriptEventFired, 128, None, None)];
    let index = choices(&scanned, &Names { slots: HashMap::new() }).unwrap();
    let holders = Holders::new(BTreeMap::new(), Vec::new());
    let ask = Ask { hour: 137, from: Some(130), to: None, gap_end: None, selection: None, subject: None, kinds: None, states: None, search: None,
        order: Order::Date(Direction::Descending), live_first: false, offset: 0, limit: 1, chips: &[] };
    let answer = index.ask(&holders, &ask).unwrap();
    assert_eq!(answer.total, 1);
    let occurrence = index.get(answer.rows[0].at);
    assert_eq!((occurrence.date, occurrence.observed, occurrence.kind), (137, 137, kind_code(EventFeedKind::EventOptionSelected)));
    let first = index.ask(&holders, &Ask { from: None, ..ask }).unwrap();
    assert_eq!(index.get(first.rows[0].at).date, 137);
    let next = index.ask(&holders, &Ask { from: None, offset: 1, ..ask }).unwrap();
    assert_eq!(index.get(next.rows[0].at).date, 128);
    assert_eq!(index.ask(&holders, &Ask { hour: 136, ..ask }).unwrap().total, 0);
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
        costs: [None, None],
    };
    let labels = Labels::from([
        (opener.at.key, (Some("Opening name".into()), None)),
        (later.at.key, (Some("Later name".into()), None)),
    ]);
    let scanned = Scanned {
        briefs: vec![opener, later],
        places: Vec::new(),
        unique_record_keys: true,
        native: Vec::new(),
        war_ends: Vec::new(),
        incomplete: Vec::new(),
        windows_unrecorded: Vec::new(),
        lifecycles: Vec::new(),
        resolved: Mutex::default(),
        presented: Mutex::default(),
        titles: Mutex::default(),
        breaks: Vec::new(),
        choice_breaks: Vec::new(),
        decision_breaks: Vec::new(),
        partial_reads: Vec::new(),
        gaps: Vec::new(),
    };
    let names = Names::new(&holders);
    let index = decisions(&scanned, &names, &HashMap::new()).unwrap();
    let index = scanned.labelled(&index, true, TitleKeys::Search, &names, &labels).unwrap();
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
    assert_eq!(index.ask(&holders, &Ask { search: Some("Later name"), ..ask_at(109) }).unwrap().total, 0);
    assert_eq!(index.ask(&holders, &Ask { search: Some("Later name"), ..ask_at(110) }).unwrap().total, 1);
    assert_eq!(index.ask(&holders, &Ask { search: Some("mission_a"), ..ask_at(100) }).unwrap().total, 1);

    let mut swapped = brief(3, EventFeedKind::DecisionSelected, 100, None, None);
    swapped.slots = [Some(8), Some(7), None];
    let swapped_scanned = Scanned { briefs: vec![swapped], ..scanned };
    let swapped_index = decisions(&swapped_scanned, &names, &HashMap::new()).unwrap();
    let swapped_index = swapped_scanned.labelled(&swapped_index, true, TitleKeys::Search, &names, &Labels::new()).unwrap();
    assert_eq!(swapped_index.ask(&holders, &Ask { search: Some("GER POL"), ..ask_at(100) }).unwrap().total, 1, "tags observed together keep sorted order in search text");
}

#[test]
fn decision_cost_sort_keeps_unrecorded_after_recorded_in_both_directions() {
    use dashboard_query::{identity::Holders, log::occurrence::{Ask, Direction, Order}};
    let mut briefs: Vec<_> = (0..4).map(|i| brief(i + 1, EventFeedKind::DecisionSelected, 100 + i as i32, None, None)).collect();
    briefs[0].costs = [Some(Some(-20)), Some(None)];
    briefs[1].costs = [Some(Some(-10)), Some(None)];
    briefs[2].costs = [Some(Some(0)), Some(Some(-15))];
    let scanned = Scanned { briefs, gaps: Vec::new(), ..Scanned::refused(0, 0, "fixture") };
    let index = decisions(&scanned, &Names { slots: HashMap::new() }, &HashMap::new()).unwrap();
    for (direction, expected) in [(Direction::Ascending, [101, 102, 100, 103]), (Direction::Descending, [100, 102, 101, 103])] {
        let answer = index.ask(&Holders::new(BTreeMap::new(), Vec::new()), &Ask { hour: 110, from: None, to: None, gap_end: None, selection: None, subject: None, kinds: None, states: None, search: None, order: Order::Key(2, direction), live_first: false, offset: 0, limit: 4, chips: &[] }).unwrap();
        assert_eq!(answer.rows.iter().map(|shown| index.get(shown.at).date).collect::<Vec<_>>(), expected);
        assert_eq!(index.get(answer.rows[3].at).sort_keys[2].at(110), None);
    }
}

#[test]
fn followed_folds_match_fresh_pages_through_late_rows_retirement_and_gap_changes() {
    use dashboard_query::{identity::Holders, log::occurrence::{Ask, DatedRows, Direction, Order}};
    use crate::products::feed::fold::{self, Pages, Window};
    let first = 24 * 2_000;
    let at = |id, date, observed| Occurrence {
        id, date, observed, kind: kind_code(EventFeedKind::ScriptEventFired), roles: Box::new([]),
        subjects: Box::new([]), states: Box::new([]), sort_keys: Box::new([]), text: "".into(),
        payload: Item::Log { head: Locator { run: 1, hour: observed, key: id }, edges: Box::new([]), valid_clock: true },
    };
    let mut entries: Vec<_> = (0..2_000).map(|day| at(day as u128, day * 24, day * 24)).collect();
    for offset in [1, 2, 3, 4, 5, 7, 8, 9, 10, 11] {
        entries.push(at((3_000 + offset) as u128, first + offset, first + offset));
    }
    entries.push(at(3_099, first, first + 14));
    let gap = MissingSpan { first_hour: first + 6, last_hour: first + 6, reason: "unread".into() };
    let index = Arc::new(Index::new(entries, vec![gap], &LIVE).unwrap().with_retirements(HashMap::from([(3011, first + 15)])));
    let fold_meta: Vec<_> = (0..index.len()).map(|at| Some(FoldMeta { day: index.get(at as u32).date.div_euclid(24), country: 0, group: 0 })).collect();
    let feed = Feed { index, notices: Vec::new(), warnings: Vec::new(), unattributed: Vec::new(), unrecorded: Vec::new(), countries: vec!["GER".into()], fold_meta, unsettled: false };
    let holders = Holders::default();
    let expanded = [fold::key_of(FoldMeta { day: first.div_euclid(24), country: 0, group: 0 }, "GER", 3007)];
    for direction in [Direction::Ascending, Direction::Descending] {
        for offset in [0, 5, 1999, 2003] {
            let mut ask = Ask { hour: first + 11, from: None, to: None, gap_end: None, selection: None, subject: None, kinds: None, states: None, search: None,
                order: Order::Date(direction), live_first: false, offset, limit: 8, chips: &[] };
            let window = Window { direction, offset, limit: 8, expanded: Some(&expanded), full: Some(&expanded) };
            let mut cursor = feed.index.dated_cursor(&holders, &ask).unwrap();
            let mut pages = Pages::new(&feed);
            for (hour, has_gap) in [(first + 5, false), (first + 6, true), (first + 11, true), (first + 12, false), (first + 13, true), (first + 14, true), (first + 15, true), (first + 11, true)] {
                ask.hour = hour;
                let mut update = cursor.advance(hour);
                if hour == first + 5 { assert_eq!(update.until, Some(first + 6)); }
                if hour == first + 6 {
                    assert!(update.changed.is_empty());
                    assert_eq!(update.gaps.len(), 1);
                }
                if !has_gap {
                    update.gaps.clear();
                }
                let page = pages.advance(&feed, &update, &window);
                feed.index.with_dated(&holders, &ask, |full| {
                    assert_eq!((update.live, update.unattributed, update.until), (full.live, full.unattributed, full.until));
                    if hour == first + 14 { assert_eq!(full.rows.len(), 2011); }
                    let full = if has_gap { full } else { DatedRows { gaps: &[], ..full } };
                    assert_eq!(page, fold::page(&feed, full, direction, offset, 8, Some(&expanded), Some(&expanded)), "hour {hour}, {direction:?}, offset {offset}");
                }).unwrap();
                if hour == first + 14 {
                    assert!(pages.visited <= 11, "only the affected day is visited, there are {} entries", feed.index.len());
                }
            }
        }
    }
}
