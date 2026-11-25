use super::*;
use crate::identity::{Country, HolderRun};

const RUNNING: u8 = 0;
const DONE: u8 = 1;

fn holders() -> Holders {
    let held = |slot: u32, first: i32, last: i32| HolderRun { first, last, holder: Holder::Found { slot, id: slot }, starts_segment: true };
    Holders::new(
        [("GER".to_owned(), vec![held(7, 0, 1_000)]), ("POL".to_owned(), vec![held(8, 0, 1_000)]), ("ITA".to_owned(), vec![held(9, 0, 1_000)])].into_iter().collect(),
        Vec::new(),
    )
}

fn occurrence(id: u128, date: i32, observed: i32) -> Occurrence<()> {
    Occurrence { id, date, observed, kind: 0, roles: Box::new([]), subjects: Box::new([]), states: Box::new([]), sort_keys: Box::new([]), text: "".into(), payload: () }
}

fn ask<'a>(hour: i32) -> Ask<'a> {
    Ask { hour, from: None, to: None, selection: None, subject: None, kinds: None, states: None, search: None, order: Order::Date(Direction::Descending), live_first: false, offset: 0, limit: 64, chips: &[] }
}

fn ids(index: &Index<()>, answer: &Answer) -> Vec<u128> {
    answer.rows.iter().map(|shown| index.get(shown.at).id).collect()
}

#[test]
fn an_occurrence_exists_from_its_observation_and_is_ranged_ordered_and_dated_by_its_own_hour() {
    let index = Index::new(vec![occurrence(1, 120, 150), occurrence(2, 145, 146), occurrence(3, 100, 100)], Vec::new(), &[]).unwrap();
    let at_147 = index.ask(&holders(), &ask(147)).unwrap();
    assert_eq!(ids(&index, &at_147), [2, 3], "the one dated 120 is not observed until 150");
    assert_eq!(at_147.total, 2);
    assert_eq!(at_147.next_change, Some(150), "its observation is the next change");
    let ranged = index.ask(&holders(), &Ask { from: Some(140), to: Some(150), ..ask(150) }).unwrap();
    assert_eq!(ids(&index, &ranged), [2], "a range of 140 to 150 holds the event of 145, never one dated 120 read at 150");
    let all = index.ask(&holders(), &ask(150)).unwrap();
    assert_eq!(ids(&index, &all), [2, 1, 3], "newest first by occurrence date, not by read");
    assert_eq!(all.next_change, None);
    let oldest = index.ask(&holders(), &Ask { order: Order::Date(Direction::Ascending), ..ask(150) }).unwrap();
    assert_eq!(ids(&index, &oldest), [3, 1, 2]);
}

#[test]
fn predicates_apply_before_the_count_and_the_slice_and_chips_count_the_matching_population() {
    let mut entries: Vec<Occurrence<()>> = (0..100).map(|at| Occurrence { states: Box::new([(at, DONE)]), ..occurrence(at as u128, at, at) }).collect();
    for at in 0..20 {
        entries[at].states = Box::new([(at as i32, RUNNING)]);
        entries[at].kind = 1;
    }
    let index = Index::new(entries, Vec::new(), &[RUNNING]).unwrap();
    let running = [RUNNING];
    let chips = [Chip { kinds: None, states: Some(vec![RUNNING]) }, Chip { kinds: Some(vec![1]), states: None }];
    let filtered = index.ask(&holders(), &Ask { states: Some(&running), limit: 10, chips: &chips, ..ask(200) }).unwrap();
    assert_eq!(filtered.total, 20, "the twenty running rows, all older than the newest eighty");
    assert_eq!(ids(&index, &filtered), (10..20).rev().collect::<Vec<u128>>(), "the window holds matching rows though the newest rows do not match");
    assert_eq!(filtered.chips, [20, 20]);
    let live = index.ask(&holders(), &Ask { live_first: true, offset: 18, limit: 4, chips: &chips, ..ask(200) }).unwrap();
    assert_eq!((live.total, live.live), (100, 20));
    assert_eq!(live.chips, [20, 20], "the running chip counts the population, not the rows shown");
    assert_eq!(ids(&index, &live), [1, 0, 99, 98], "live rows first, newest first, then history newest first");
    assert_eq!(live.rows.iter().map(|shown| (shown.ordinal, shown.live)).collect::<Vec<_>>(), [(18, true), (19, true), (20, false), (21, false)]);
}

#[test]
fn a_state_belongs_to_the_hour_asked_and_its_change_is_the_next_change() {
    let entry = Occurrence { states: Box::new([(10, RUNNING), (30, DONE)]), ..occurrence(1, 10, 10) };
    let index = Index::new(vec![entry], Vec::new(), &[RUNNING]).unwrap();
    let running = [RUNNING];
    let at_20 = index.ask(&holders(), &Ask { states: Some(&running), ..ask(20) }).unwrap();
    assert_eq!((at_20.total, at_20.live, at_20.next_change), (1, 0, Some(30)));
    let at_30 = index.ask(&holders(), &Ask { states: Some(&running), ..ask(30) }).unwrap();
    assert_eq!(at_30.total, 0, "it ended at 30");
    assert!(Index::new(vec![Occurrence { states: Box::new([(11, RUNNING)]), ..occurrence(1, 10, 10) }], Vec::new(), &[]).is_err(), "a first state is the observation");
    assert!(Index::new(vec![occurrence(1, 11, 10)], Vec::new(), &[]).is_err(), "nothing is dated after it was seen");
    assert!(Index::new(vec![occurrence(1, 10, 10), occurrence(1, 11, 11)], Vec::new(), &[]).is_err(), "ids are unique");
}

#[test]
fn gaps_are_cut_at_the_hour_asked_and_placed_by_ordinal_in_the_replys_order() {
    let gap = |first_hour: i32, last_hour: i32| MissingSpan { first_hour, last_hour, reason: format!("unread {first_hour}") };
    let index = Index::new(vec![occurrence(1, 10, 10), occurrence(2, 50, 50), occurrence(3, 90, 90)], vec![gap(20, 30), gap(60, 80)], &[]).unwrap();
    let at_70 = index.ask(&holders(), &ask(70)).unwrap();
    assert_eq!(at_70.gaps.iter().map(|gap| (gap.first_hour, gap.last_hour, gap.before.as_str())).collect::<Vec<_>>(), [(60, 70, "0"), (20, 30, "1")], "nothing after 70 in a gap; each before the first row older than it");
    assert_eq!(at_70.next_change, Some(71), "inside a gap the gap grows every hour");
    let at_40 = index.ask(&holders(), &ask(40)).unwrap();
    assert_eq!(at_40.next_change, Some(50));
    let at_130 = index.ask(&holders(), &ask(130)).unwrap();
    assert_eq!(at_130.gaps.iter().map(|gap| gap.before.as_str()).collect::<Vec<_>>(), ["1", "2"], "the gap of 60 to 80 sits below the row of 90 and above 50");
    assert_eq!(index.ask(&holders(), &Ask { from: Some(40), ..ask(130) }).unwrap().gaps.len(), 1, "a gap before the range is not the range's");
}

#[test]
fn a_selection_matches_every_role_at_its_own_date_through_dated_membership() {
    let role = |slot: Option<u32>, at: i32| Role { slot, at };
    let entries = vec![
        Occurrence { roles: Box::new([role(Some(9), 100)]), ..occurrence(1, 100, 100) },
        Occurrence { roles: Box::new([role(Some(9), 250)]), ..occurrence(2, 250, 250) },
        Occurrence { roles: Box::new([role(Some(7), 260), role(None, 260), role(Some(8), 260)]), ..occurrence(3, 260, 260) },
        Occurrence { roles: Box::new([role(Some(9), 320)]), ..occurrence(4, 320, 320) },
        Occurrence { roles: Box::new([role(None, 330)]), ..occurrence(5, 330, 330) },
    ];
    let index = Index::new(entries, Vec::new(), &[]).unwrap();
    let span = |tag: &str, first_hour: i32, last_hour: i32| CountrySpan { country: Country::Tag(tag.to_owned()), first_hour, last_hour };
    let selection = Selection { membership: vec![span("ITA", 200, 299), span("POL", 0, 1_000)], unobserved: vec![UnobservedSpan { first_hour: 300, last_hour: 325 }], unknown: Vec::new() };
    let answer = index.ask(&holders(), &Ask { selection: Some(&selection), ..ask(400) }).unwrap();
    assert_eq!(ids(&index, &answer), [3, 2], "Italy joins at 200: its row of 100 is not the group's; the war join is kept by its ally alone");
    assert_eq!(answer.gaps.iter().map(|gap| (gap.first_hour, gap.last_hour, gap.reason.as_str(), gap.before.as_str())).collect::<Vec<_>>(), [(300, 325, MEMBERSHIP_UNKNOWN, "0")], "Italy's row in the unknown stretch is the gap's, not a member's");
    assert_eq!(answer.unattributed, 1, "a row naming no identified country cannot be placed by a selection, and is counted");
    let everyone = index.ask(&holders(), &ask(400)).unwrap();
    assert_eq!(everyone.total, 5, "with no selection every row is kept, unidentified roles too");
}

#[test]
fn empty_selection_admits_no_rows_and_does_not_collide_with_no_selection_in_cache() {
    let entries = vec![
        Occurrence { roles: Box::new([Role { slot: Some(7), at: 100 }]), ..occurrence(1, 100, 100) },
        Occurrence { roles: Box::new([Role { slot: Some(9), at: 110 }]), ..occurrence(2, 110, 110) },
        Occurrence { roles: Box::new([Role { slot: None, at: 120 }]), ..occurrence(3, 120, 120) },
    ];
    let empty = Selection::default();

    let first_all = Index::new(entries.clone(), Vec::new(), &[]).unwrap();
    let all = first_all.ask(&holders(), &ask(200)).unwrap();
    assert_eq!((all.total, all.unattributed), (3, 0));
    let none = first_all.ask(&holders(), &Ask { selection: Some(&empty), ..ask(200) }).unwrap();
    assert_eq!((none.total, none.rows.len(), none.unattributed), (0, 0, 1), "an empty selection admits no country rows even after all-countries warmed the cache");

    let first_empty = Index::new(entries, Vec::new(), &[]).unwrap();
    let none_cold = first_empty.ask(&holders(), &Ask { selection: Some(&empty), ..ask(200) }).unwrap();
    assert_eq!((none_cold.total, none_cold.rows.len(), none_cold.unattributed), (0, 0, 1));
    let all_after = first_empty.ask(&holders(), &ask(200)).unwrap();
    assert_eq!((all_after.total, all_after.unattributed), (3, 0), "no selection admits every row even after an empty selection warmed the cache");
}

#[test]
fn a_role_known_only_after_the_hour_asked_does_not_admit_its_row() {
    let entries = vec![Occurrence { roles: Box::new([Role { slot: Some(7), at: 100 }, Role { slot: Some(8), at: 120 }]), ..occurrence(1, 100, 100) }];
    let index = Index::new(entries, Vec::new(), &[]).unwrap();
    let span = |tag: &str| CountrySpan { country: Country::Tag(tag.to_owned()), first_hour: 0, last_hour: 1_000 };
    let poland = Selection { membership: vec![span("POL")], unobserved: Vec::new(), unknown: Vec::new() };
    let at_110 = index.ask(&holders(), &Ask { selection: Some(&poland), ..ask(110) }).unwrap();
    assert_eq!((at_110.total, at_110.next_change), (0, Some(120)), "Poland's role is first known at 120: the row is not Poland's before it");
    assert_eq!(index.ask(&holders(), &Ask { selection: Some(&poland), ..ask(120) }).unwrap().total, 1);
    let germany = Selection { membership: vec![span("GER")], unobserved: Vec::new(), unknown: Vec::new() };
    assert_eq!(index.ask(&holders(), &Ask { selection: Some(&germany), ..ask(110) }).unwrap().total, 1, "a role known at the observation admits from it");
    let untagged = Index::new(vec![Occurrence { roles: Box::new([Role { slot: Some(40), at: 100 }]), ..occurrence(2, 100, 100) }], Vec::new(), &[]).unwrap();
    let slot = Selection { membership: vec![CountrySpan { country: Country::Slot(40), first_hour: 0, last_hour: 1_000 }], unobserved: Vec::new(), unknown: Vec::new() };
    assert_eq!(untagged.ask(&holders(), &Ask { selection: Some(&slot), ..ask(110) }).unwrap().total, 1, "a selected slot holding no tag still names its own rows");
}

#[test]
fn a_figure_orders_known_values_first_and_search_and_kinds_narrow_before_counting() {
    let figured = |id: u128, figure: Option<i64>, kind: u16, text: &str| Occurrence { sort_keys: Box::new([SortKey::number(figure)]), kind, text: text.into(), ..occurrence(id, id as i32, id as i32) };
    let index = Index::new(vec![figured(1, Some(5), 0, "Battle of Gdynia"), figured(2, None, 0, "Battle of Derna"), figured(3, Some(9), 1, "Battle of Kalamata"), figured(4, Some(1), 0, "battle of gdynia")], Vec::new(), &[]).unwrap();
    let sorted = index.ask(&holders(), &Ask { order: Order::Key(0, Direction::Descending), ..ask(10) }).unwrap();
    assert_eq!(ids(&index, &sorted), [3, 1, 4, 2], "a figure not recorded sorts last, never as zero");
    let searched = index.ask(&holders(), &Ask { search: Some(" GDYNIA "), ..ask(10) }).unwrap();
    assert_eq!((ids(&index, &searched), searched.total), (vec![4, 1], 2));
    let kinds = [1];
    assert_eq!(index.ask(&holders(), &Ask { kinds: Some(&kinds), ..ask(10) }).unwrap().total, 1);
    assert!(index.ask(&holders(), &Ask { order: Order::Key(1, Direction::Descending), ..ask(10) }).is_err());
    let (before, bins) = index.bins(&holders(), &ask(10), 2, 4, 2).unwrap();
    assert_eq!((before, bins), (1, vec![(0, 2), (1, 1)]), "bins count the matching population by date on the absolute grid");
}

#[test]
fn unresolved_explicit_tag_holders_carry_unknown_spans_reasons_and_validity_boundaries() {
    let holders = Holders::new(
        [(
            "GER".to_owned(),
            vec![
                HolderRun { first: 0, last: 100, holder: Holder::Found { slot: 7, id: 7 }, starts_segment: true },
                HolderRun { first: 101, last: 120, holder: Holder::Unresolved, starts_segment: false },
                HolderRun { first: 121, last: 149, holder: Holder::Found { slot: 7, id: 7 }, starts_segment: false },
            ],
        )]
        .into_iter()
        .collect(),
        vec![(150, 170, "Countries table unread".to_owned())],
    );
    let entries = vec![
        Occurrence { roles: Box::new([Role { slot: Some(7), at: 50 }]), ..occurrence(1, 50, 50) },
        Occurrence { roles: Box::new([Role { slot: Some(7), at: 110 }]), ..occurrence(2, 110, 110) },
        Occurrence { roles: Box::new([Role { slot: Some(7), at: 160 }]), ..occurrence(3, 160, 160) },
    ];
    let index = Index::new(entries, Vec::new(), &[]).unwrap();
    let selection = Selection {
        membership: vec![CountrySpan { country: Country::Tag("GER".into()), first_hour: 0, last_hour: 200 }],
        unobserved: Vec::new(),
        unknown: Vec::new(),
    };

    let at_95 = index.ask(&holders, &Ask { selection: Some(&selection), ..ask(95) }).unwrap();
    assert_eq!(ids(&index, &at_95), [1]);
    assert!(at_95.gaps.is_empty());
    assert_eq!(at_95.next_change, Some(101), "unknown holder span start is a validity boundary");

    let at_110 = index.ask(&holders, &Ask { selection: Some(&selection), ..ask(110) }).unwrap();
    assert_eq!(ids(&index, &at_110), [1]);
    assert_eq!(at_110.gaps.len(), 1, "unresolved holder run must surface as a coverage gap");
    assert_eq!((at_110.gaps[0].first_hour, at_110.gaps[0].last_hour), (101, 110));
    assert!(at_110.gaps[0].reason.contains(crate::identity::TAG_UNLINKED), "reason must name unlinked tag: {}", at_110.gaps[0].reason);
    assert_eq!(at_110.next_change, Some(111), "growing unresolved holder gap advances validity");

    let at_160 = index.ask(&holders, &Ask { selection: Some(&selection), ..ask(160) }).unwrap();
    assert_eq!(at_160.gaps.len(), 2, "both unresolved run and holders.unknown span must surface");
    assert!(at_160.gaps.iter().any(|g| (g.first_hour, g.last_hour) == (101, 120) && g.reason.contains(crate::identity::TAG_UNLINKED)));
    assert!(at_160.gaps.iter().any(|g| (g.first_hour, g.last_hour) == (150, 160) && g.reason.contains("Countries table unread")));
    assert_eq!(at_160.next_change, Some(161));
}

#[test]
fn cutoff_aware_sort_keys_order_ongoing_rows_at_the_asked_hour_and_support_text_and_indexed_subjects() {
    let entries = vec![
        Occurrence {
            subjects: Box::new([101, 202]),
            sort_keys: Box::new([
                SortKey::dated_numbers([(100, Some(50)), (130, Some(500))]),
                SortKey::dated_text([(100, Some("gdynia".into())), (140, Some("Alpha".into()))]),
            ]),
            ..occurrence(1, 100, 100)
        },
        Occurrence {
            subjects: Box::new([202]),
            sort_keys: Box::new([
                SortKey::dated_numbers([(100, Some(120)), (130, Some(200))]),
                SortKey::text(Some("Berlin")),
            ]),
            ..occurrence(2, 102, 102)
        },
        Occurrence {
            subjects: Box::new([303]),
            sort_keys: Box::new([
                SortKey::dated_numbers([(135, Some(900))]),
                SortKey::text(None::<&str>),
            ]),
            ..occurrence(3, 104, 104)
        },
    ];
    let index = Index::new(entries, Vec::new(), &[]).unwrap();

    let by_date = index.ask(&holders(), &ask(110)).unwrap();
    assert_eq!(ids(&index, &by_date), [3, 2, 1]);
    assert_eq!(by_date.next_change, None, "date order ignores sort-key steps so quiet stretches stay quiet");

    let num_110 = index.ask(&holders(), &Ask { order: Order::Key(0, Direction::Descending), ..ask(110) }).unwrap();
    assert_eq!(ids(&index, &num_110), [2, 1, 3], "at 110: 120 > 50 > missing (missing sorts last)");
    assert_eq!(num_110.next_change, Some(130), "numeric sort validity ends at the next sort-key change");

    let num_130 = index.ask(&holders(), &Ask { order: Order::Key(0, Direction::Descending), ..ask(130) }).unwrap();
    assert_eq!(ids(&index, &num_130), [1, 2, 3], "at 130: 500 > 200 > missing");
    assert_eq!(num_130.next_change, Some(135));

    let num_135_asc = index.ask(&holders(), &Ask { order: Order::Key(0, Direction::Ascending), ..ask(135) }).unwrap();
    assert_eq!(ids(&index, &num_135_asc), [2, 1, 3], "at 135 asc: 200 < 500 < 900");

    let text_110 = index.ask(&holders(), &Ask { order: Order::Key(1, Direction::Ascending), ..ask(110) }).unwrap();
    assert_eq!(ids(&index, &text_110), [2, 1, 3], "case-insensitive text order: Berlin < gdynia < missing");
    assert_eq!(text_110.next_change, Some(140));

    let text_140 = index.ask(&holders(), &Ask { order: Order::Key(1, Direction::Ascending), ..ask(140) }).unwrap();
    assert_eq!(ids(&index, &text_140), [1, 2, 3], "renamed text reorders at 140: Alpha < Berlin < missing");

    let subject_two = index.ask(&holders(), &Ask { subject: Some(202), order: Order::Key(0, Direction::Descending), ..ask(130) }).unwrap();
    assert_eq!((ids(&index, &subject_two), subject_two.total), (vec![1, 2], 2));
    let subject_one = index.ask(&holders(), &Ask { subject: Some(101), ..ask(130) }).unwrap();
    assert_eq!((ids(&index, &subject_one), subject_one.total), (vec![1], 1));
    let subject_missing = index.ask(&holders(), &Ask { subject: Some(999), ..ask(130) }).unwrap();
    assert_eq!((subject_missing.rows.len(), subject_missing.total), (0, 0));
}
