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
    Ask { hour, from: None, to: None, gap_end: None, selection: None, subject: None, kinds: None, states: None, search: None, order: Order::Date(Direction::Descending), live_first: false, offset: 0, limit: 64, chips: &[] }
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
    let past = index.ask(&holders(), &Ask { offset: u64::MAX, ..ask(150) }).unwrap();
    assert!(past.rows.is_empty());
    assert_eq!(past.offset, past.total);
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
    assert_eq!(filtered.live, 20, "the live count is every matching live row, whether or not they are moved to the front");
    let live = index.ask(&holders(), &Ask { live_first: true, offset: 18, limit: 4, chips: &chips, ..ask(200) }).unwrap();
    assert_eq!((live.total, live.live), (100, 20));
    assert_eq!(live.chips, [20, 20], "the running chip counts the population, not the rows shown");
    assert_eq!(ids(&index, &live), [1, 0, 99, 98], "live rows first, newest first, then history newest first");
    assert_eq!(live.rows.iter().map(|shown| (shown.ordinal, shown.live)).collect::<Vec<_>>(), [(18, true), (19, true), (20, false), (21, false)]);
    let counts = Ask { limit: 1, states: Some(&[RUNNING]), ..ask(200) };
    assert_eq!(index.ask(&holders(), &counts).unwrap().total, 20);
    STATE_READS.set(0);
    let deep = index.ask(&holders(), &Ask { live_first: true, offset: 95, limit: 3, chips: &chips, ..ask(200) }).unwrap();
    assert_eq!(ids(&index, &deep), [24, 23, 22]);
    assert!(STATE_READS.get() <= 3, "a deep page checked {} row states for three rows", STATE_READS.get());
    STATE_READS.set(0);
    assert_eq!(index.ask(&holders(), &counts).unwrap().total, 20);
    assert_eq!(STATE_READS.get(), 1, "a second view of the same index keeps its counted population");
    let custom = Index::new((1..=3).map(|hour| Occurrence {
        id: hour as u128, date: hour, observed: hour, kind: 0, roles: Box::new([]), subjects: Box::new([]), states: Box::new([]), sort_keys: Box::new([]), text: "".into(), payload: hour,
    }).collect(), Vec::new(), &[]).unwrap();
    let visible = custom.ask_visible(&holders(), &ask(1), |&id| id != 2).unwrap();
    assert_eq!((visible.total, visible.next_change), (1, Some(3)));
    assert_eq!(custom.ask_visible(&holders(), &ask(2), |&id| id == 2).unwrap().total, 1);
    assert_eq!(custom.ask(&holders(), &ask(2)).unwrap().total, 2);
}

#[test]
fn a_state_belongs_to_the_hour_asked_and_its_change_is_the_next_change() {
    let entry = Occurrence { states: Box::new([(10, RUNNING), (30, DONE)]), ..occurrence(1, 10, 10) };
    let index = Index::new(vec![entry], Vec::new(), &[RUNNING]).unwrap();
    let running = [RUNNING];
    let at_20 = index.ask(&holders(), &Ask { states: Some(&running), ..ask(20) }).unwrap();
    assert_eq!((at_20.total, at_20.live, at_20.next_change), (1, 1, Some(30)), "one running row is one live row in any order");
    let newer = Occurrence { states: Box::new([(15, DONE)]), ..occurrence(2, 15, 15) };
    let both = Index::new(vec![Occurrence { states: Box::new([(10, RUNNING), (30, DONE)]), ..occurrence(1, 10, 10) }, newer], Vec::new(), &[RUNNING]).unwrap();
    let page = both.ask(&holders(), &Ask { limit: 1, ..ask(20) }).unwrap();
    assert_eq!(ids(&both, &page), [2], "the running row is off the page");
    assert_eq!((page.live, page.next_change), (1, Some(30)), "its end changes the live count, so the reply changes then");
    let at_30 = index.ask(&holders(), &Ask { states: Some(&running), ..ask(30) }).unwrap();
    assert_eq!(at_30.total, 0, "it ended at 30");
    assert!(Index::new(vec![Occurrence { states: Box::new([(11, RUNNING)]), ..occurrence(1, 10, 10) }], Vec::new(), &[]).is_err(), "a first state is the observation");
    assert!(Index::new(vec![occurrence(1, 11, 10)], Vec::new(), &[]).is_err(), "nothing is dated after it was seen");
    assert!(Index::new(vec![occurrence(1, 10, 10), occurrence(2, 11, 11), occurrence(1, 12, 12)], Vec::new(), &[]).is_err(), "ids are unique across dates");
    let excluded = Occurrence { states: Box::new([(12, DONE), (24, DONE), (60, RUNNING)]), ..occurrence(3, 12, 12) };
    let index = Index::new(vec![excluded], Vec::new(), &[RUNNING]).unwrap();
    let answer = index.ask(&holders(), &Ask { states: Some(&running), ..ask(10) }).unwrap();
    assert_eq!(answer.next_change, Some(60), "the first admission is when the row enters the selected state");
}

#[test]
fn gaps_are_cut_at_the_hour_asked_and_placed_by_ordinal_in_the_replys_order() {
    let gap = |first_hour: i32, last_hour: i32| MissingSpan { first_hour, last_hour, reason: format!("unread {first_hour}") };
    let index = Index::new(vec![occurrence(1, 10, 10), occurrence(2, 50, 50), occurrence(3, 90, 90)], vec![gap(20, 30), gap(60, 80)], &[]).unwrap();
    let at_70 = index.ask(&holders(), &ask(70)).unwrap();
    assert_eq!(at_70.gaps.iter().map(|gap| (gap.first_hour, gap.last_hour, gap.before.as_str())).collect::<Vec<_>>(), [(60, 80, "0"), (20, 30, "1")], "an open gap carries its evidence end for the view to clip");
    assert_eq!(at_70.next_change, Some(90), "the next observed row changes this page");
    let at_40 = index.ask(&holders(), &ask(40)).unwrap();
    assert_eq!(at_40.next_change, Some(50));
    let at_130 = index.ask(&holders(), &ask(130)).unwrap();
    assert_eq!(at_130.gaps.iter().map(|gap| gap.before.as_str()).collect::<Vec<_>>(), ["1", "2"], "the gap of 60 to 80 sits below the row of 90 and above 50");
    assert_eq!(index.ask(&holders(), &Ask { from: Some(40), ..ask(130) }).unwrap().gaps.len(), 1, "a gap before the range is not the range's");
    let range = index.ask(&holders(), &Ask { from: Some(25), to: Some(65), ..ask(130) }).unwrap();
    assert_eq!(range.gaps.iter().map(|gap| (gap.first_hour, gap.last_hour)).collect::<Vec<_>>(), [(60, 65), (25, 30)]);
    let complete = index.ask(&holders(), &Ask { from: Some(35), to: Some(55), ..ask(50) }).unwrap();
    assert!(complete.gaps.is_empty());
    assert_eq!(complete.next_change, None);
}

#[test]
fn a_selection_matches_every_role_at_its_own_date_through_dated_membership() {
    let role = |slot: Option<u32>, at: i32| Role::at(slot, at);
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
    let running = [RUNNING];
    let states = Index::new(
        vec![Occurrence { roles: Box::new([role(None, 100)]), states: Box::new([(100, RUNNING), (150, DONE)]), ..occurrence(1, 100, 100) }],
        Vec::new(),
        &[RUNNING],
    )
    .unwrap();
    let asked = |hour: i32| states.ask(&holders(), &Ask { selection: Some(&selection), states: Some(&running), ..ask(hour) }).unwrap();
    assert_eq!((asked(120).unattributed, asked(120).next_change), (1, Some(150)), "a running unattributed row passes a running filter until it ends");
    assert_eq!(asked(160).unattributed, 0, "after it ends the running filter leaves it out, as it would a placed row");
    let everyone = index.ask(&holders(), &ask(400)).unwrap();
    assert_eq!(everyone.total, 5, "with no selection every row is kept, unidentified roles too");
}

#[test]
fn empty_selection_admits_no_rows_and_does_not_collide_with_no_selection_in_cache() {
    let entries = vec![
        Occurrence { roles: Box::new([Role::at(Some(7), 100)]), ..occurrence(1, 100, 100) },
        Occurrence { roles: Box::new([Role::at(Some(9), 110)]), ..occurrence(2, 110, 110) },
        Occurrence { roles: Box::new([Role::at(None, 120)]), ..occurrence(3, 120, 120) },
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
    let entries = vec![Occurrence { roles: Box::new([Role::at(Some(7), 100), Role::at(Some(8), 120)]), ..occurrence(1, 100, 100) }];
    let index = Index::new(entries, Vec::new(), &[]).unwrap();
    let span = |tag: &str| CountrySpan { country: Country::Tag(tag.to_owned()), first_hour: 0, last_hour: 1_000 };
    let poland = Selection { membership: vec![span("POL")], unobserved: Vec::new(), unknown: Vec::new() };
    let at_110 = index.ask(&holders(), &Ask { selection: Some(&poland), ..ask(110) }).unwrap();
    assert_eq!((at_110.total, at_110.next_change), (0, Some(120)), "Poland's role is first known at 120: the row is not Poland's before it");
    assert_eq!(index.ask(&holders(), &Ask { selection: Some(&poland), ..ask(120) }).unwrap().total, 1);
    let germany = Selection { membership: vec![span("GER")], unobserved: Vec::new(), unknown: Vec::new() };
    assert_eq!(index.ask(&holders(), &Ask { selection: Some(&germany), ..ask(110) }).unwrap().total, 1, "a role known at the observation admits from it");
    let untagged = Index::new(vec![Occurrence { roles: Box::new([Role::at(Some(40), 100)]), ..occurrence(2, 100, 100) }], Vec::new(), &[]).unwrap();
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
        Occurrence { roles: Box::new([Role::at(Some(7), 50)]), ..occurrence(1, 50, 50) },
        Occurrence { roles: Box::new([Role::at(Some(7), 110)]), ..occurrence(2, 110, 110) },
        Occurrence { roles: Box::new([Role::at(Some(7), 160)]), ..occurrence(3, 160, 160) },
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
    assert_eq!((at_110.gaps[0].first_hour, at_110.gaps[0].last_hour), (101, 120));
    assert!(at_110.gaps[0].reason.contains(crate::identity::TAG_UNLINKED), "reason must name unlinked tag: {}", at_110.gaps[0].reason);
    assert_eq!(at_110.next_change, Some(150), "the next coverage gap changes the view");

    let at_160 = index.ask(&holders, &Ask { selection: Some(&selection), ..ask(160) }).unwrap();
    assert_eq!(at_160.gaps.len(), 2, "both unresolved run and holders.unknown span must surface");
    assert!(at_160.gaps.iter().any(|g| (g.first_hour, g.last_hour) == (101, 120) && g.reason.contains(crate::identity::TAG_UNLINKED)));
    assert!(at_160.gaps.iter().any(|g| (g.first_hour, g.last_hour) == (150, 170) && g.reason.contains("Countries table unread")));
    assert_eq!(at_160.next_change, None);
}

#[test]
fn cutoff_aware_sort_keys_order_ongoing_rows_at_the_asked_hour_and_support_text_and_indexed_subjects() {
    let entries = vec![
        Occurrence {
            subjects: Box::new([101, 202]),
            sort_keys: Box::new([
                SortKey::dated_numbers([(100, Some(50)), (130, Some(500))]).shared(),
                SortKey::dated_text([(100, Some("gdynia".into())), (140, Some("Alpha".into()))]).shared(),
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
    let second = index.ask(&holders(), &Ask { offset: 1, limit: 1, order: Order::Key(0, Direction::Descending), ..ask(130) }).unwrap();
    assert_eq!((ids(&index, &second), second.total), (vec![2], 3));

    let smallest = index.ask(&holders(), &Ask { order: Order::Key(0, Direction::Ascending), limit: 1, ..ask(130) }).unwrap();
    assert_eq!(ids(&index, &smallest), [2]);
    assert_eq!(smallest.next_change, None, "an off-page figure becoming known below the visible page does not invalidate it");
    let num_135_asc = index.ask(&holders(), &Ask { order: Order::Key(0, Direction::Ascending), ..ask(135) }).unwrap();
    assert_eq!(ids(&index, &num_135_asc), [2, 1, 3], "at 135 asc: 200 < 500 < 900");

    let crossing = |id, figures| Occurrence { sort_keys: Box::new([SortKey::dated_numbers(figures)]), ..occurrence(id, 0, 0) };
    let middle = Index::new(vec![
        crossing(1, vec![(0, Some(0)), (6, Some(20)), (8, Some(9))]),
        crossing(2, vec![(0, Some(10))]),
        crossing(3, vec![(0, Some(20)), (6, Some(0))]),
    ], Vec::new(), &[]).unwrap();
    let page = middle.ask(&holders(), &Ask { order: Order::Key(0, Direction::Ascending), offset: 1, limit: 1, ..ask(5) }).unwrap();
    assert_eq!(ids(&middle, &page), [2]);
    assert_eq!(page.next_change, Some(8), "two rows swapping across a page leave its ordinal unchanged until another crosses");

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

#[test]
fn dated_roles_and_dated_search_text_gate_admission_by_observation_and_membership_by_effective_hour() {
    assert!(Index::new(vec![Occurrence { roles: Box::new([Role::dated(Some(7), 110, 100)]), ..occurrence(1, 100, 100) }], Vec::new(), &[]).is_err());
    assert!(Index::new(vec![Occurrence { text: SearchText::Dated(Box::new([(110, "a".into()), (100, "b".into())])), ..occurrence(1, 100, 100) }], Vec::new(), &[]).is_err());
    let entries = vec![
        Occurrence {
            roles: Box::new([Role::at(Some(7), 100), Role::dated(Some(8), 105, 110)]),
            text: SearchText::dated([(100, "GER".into()), (110, "POL".into())]),
            ..occurrence(1, 100, 100)
        },
        Occurrence {
            roles: Box::new([Role::at(None, 100)]),
            text: SearchText::dated([(100, "base".into()), (120, "later".into())]),
            ..occurrence(2, 100, 100)
        },
    ];
    let index = Index::new(entries, Vec::new(), &[]).unwrap();
    let member = |first_hour: i32, last_hour: i32| Selection {
        membership: vec![CountrySpan { country: Country::Tag("POL".into()), first_hour, last_hour }],
        unobserved: Vec::new(),
        unknown: Vec::new(),
    };
    let poland_early = member(100, 108);
    let at_105 = index.ask(&holders(), &Ask { selection: Some(&poland_early), ..ask(105) }).unwrap();
    assert_eq!((at_105.total, at_105.next_change), (0, Some(110)), "role dated 105 and observed at 110 is not admitted at 105");
    let at_109 = index.ask(&holders(), &Ask { selection: Some(&poland_early), ..ask(109) }).unwrap();
    assert_eq!((at_109.total, at_109.next_change), (0, Some(110)));
    let at_110 = index.ask(&holders(), &Ask { selection: Some(&poland_early), ..ask(110) }).unwrap();
    assert_eq!((ids(&index, &at_110), at_110.total, at_110.next_change), (vec![1], 1, None), "membership holds at effective hour 105 even after Poland left at 108");
    let poland_late = member(108, 200);
    let not_member_at_105 = index.ask(&holders(), &Ask { selection: Some(&poland_late), ..ask(110) }).unwrap();
    assert_eq!((not_member_at_105.total, not_member_at_105.next_change), (0, None), "membership at 110 does not admit a role effective at 105");

    let search_before = index.ask(&holders(), &Ask { search: Some("pol"), ..ask(109) }).unwrap();
    assert_eq!((search_before.total, search_before.next_change), (0, Some(110)), "later search text does not match before observation");
    let search_at = index.ask(&holders(), &Ask { search: Some("pol"), ..ask(110) }).unwrap();
    assert_eq!((ids(&index, &search_at), search_at.total, search_at.next_change), (vec![1], 1, None));
    assert_eq!(index.ask(&holders(), &Ask { search: Some("ger pol"), ..ask(109) }).unwrap().total, 0);
    assert_eq!(index.ask(&holders(), &Ask { search: Some("ger pol"), ..ask(110) }).unwrap().total, 1);
    assert_eq!(index.ask(&holders(), &Ask { search: Some("ger"), ..ask(100) }).unwrap().total, 1);
    let unattr_before = index.ask(&holders(), &Ask { selection: Some(&poland_early), search: Some("later"), ..ask(110) }).unwrap();
    assert_eq!((unattr_before.unattributed, unattr_before.next_change), (0, Some(120)));
    let unattr_at = index.ask(&holders(), &Ask { selection: Some(&poland_early), search: Some("later"), ..ask(120) }).unwrap();
    assert_eq!((unattr_at.unattributed, unattr_at.next_change), (1, None));

    let delayed_name = Index::new(
        vec![Occurrence { roles: Box::new([Role::at(None, 100), Role::dated(Some(8), 105, 110)]), ..occurrence(3, 100, 100) }],
        Vec::new(),
        &[],
    )
    .unwrap();
    let sel_before = delayed_name.ask(&holders(), &Ask { selection: Some(&poland_early), ..ask(105) }).unwrap();
    assert_eq!((sel_before.total, sel_before.unattributed, sel_before.next_change), (0, 1, Some(110)), "initially unidentified row stays unattributed until its first role is observed");
    let sel_at = delayed_name.ask(&holders(), &Ask { selection: Some(&poland_early), ..ask(110) }).unwrap();
    assert_eq!((sel_at.total, sel_at.unattributed, sel_at.next_change), (1, 0, None));
    let germany = Selection { membership: vec![CountrySpan { country: Country::Tag("GER".into()), first_hour: 0, last_hour: 1_000 }], unobserved: Vec::new(), unknown: Vec::new() };
    let other_before = delayed_name.ask(&holders(), &Ask { selection: Some(&germany), ..ask(105) }).unwrap();
    assert_eq!((other_before.total, other_before.unattributed, other_before.next_change), (0, 1, Some(110)), "even an unselected later role ends unattributed notice at its observation");
    let other_at = delayed_name.ask(&holders(), &Ask { selection: Some(&germany), ..ask(110) }).unwrap();
    assert_eq!((other_at.total, other_at.unattributed, other_at.next_change), (0, 0, None));
}

#[test]
fn bins_before_count_avoids_scanning_historical_rows_and_matches_mixed_observation_hours() {
    let entries: Vec<Occurrence<()>> = (0..2_000)
        .map(|hour| {
            let observed = if (100..200).contains(&hour) { 2_500 } else if hour == 500 { 1_995 } else { hour };
            occurrence(hour as u128, hour, observed)
        })
        .collect();
    let index = Index::new(entries, Vec::new(), &[]).unwrap();
    BIN_SCANS.set(0);
    let (before, bins) = index.bins(&holders(), &ask(1_999), 1_990, 1_999, 5).unwrap();
    assert_eq!((before, bins), (1_890, vec![(0, 5), (1, 5)]));
    assert!(BIN_SCANS.get() <= 260, "tail bins scanned {} rows instead of pruning historical blocks", BIN_SCANS.get());
    assert_eq!(index.bins(&holders(), &ask(1_994), 1_990, 1_999, 5).unwrap(), (1_889, vec![(0, 5)]), "row 500 observed at 1995 is excluded at 1994");
    assert_eq!(index.bins(&holders(), &ask(1_994), 1_995, 2_000, 5).unwrap(), (1_894, Vec::new()), "when ask.hour < first, all observed rows count in before");
}

#[test]
fn retired_occurrence_is_visible_until_retirement_wakes_next_change_and_keeps_bins_bounded() {
    let entries = vec![occurrence(1, 101, 101), occurrence(2, 101, 103)];
    let index = Index::new(entries, Vec::new(), &[]).unwrap().with_retirements(HashMap::from([(1, 103)]));
    let at_100 = index.ask(&holders(), &ask(100)).unwrap();
    assert_eq!((ids(&index, &at_100), at_100.next_change), (Vec::new(), Some(101)));
    let at_101 = index.ask(&holders(), &ask(101)).unwrap();
    assert_eq!((ids(&index, &at_101), at_101.next_change), (vec![1], Some(103)));
    let at_102 = index.ask(&holders(), &ask(102)).unwrap();
    assert_eq!((ids(&index, &at_102), at_102.next_change), (vec![1], Some(103)));
    let at_103 = index.ask(&holders(), &ask(103)).unwrap();
    assert_eq!((ids(&index, &at_103), at_103.next_change), (vec![2], None));
    assert_eq!(index.bins(&holders(), &ask(102), 100, 105, 10).unwrap(), (0, vec![(0, 1)]));
    assert_eq!(index.bins(&holders(), &ask(103), 100, 105, 10).unwrap(), (0, vec![(0, 1)]));
    assert_eq!(index.bins(&holders(), &ask(103), 102, 105, 10).unwrap(), (1, Vec::new()));
}

#[test]
fn dated_cursor_matches_rebuilt_order_counts_gaps_and_proof_with_bounded_visits() {
    let mut entries: Vec<_> = (0..2000).map(|hour| occurrence(hour as u128, hour, hour)).collect();
    let mut late = occurrence(2100, 1950, 2002);
    late.states = vec![(2002, RUNNING), (2004, DONE)].into_boxed_slice();
    entries.push(late);
    entries.push(occurrence(2101, 2003, 2003));
    let gaps = vec![
        MissingSpan { first_hour: 1990, last_hour: 1991, reason: "before the late row".into() },
        MissingSpan { first_hour: 2001, last_hour: 2002, reason: "gap".into() },
    ];
    let index = Arc::new(Index::new(entries, gaps, &[RUNNING]).unwrap().with_retirements(HashMap::from([(2100, 2005)])));
    let late = index.find(2100).unwrap();
    let holders = holders();
    let chips = [Chip { kinds: None, states: Some(vec![DONE]) }];
    let mut cursor = index.dated_cursor(&holders, &Ask { chips: &chips, ..ask(1999) }).unwrap();
    for hour in [1999, 2000, 2001, 2002, 2003, 2004, 2005, 2006, 2001, 2005] {
        let asked = Ask { chips: &chips, ..ask(hour) };
        STATE_READS.set(0);
        index.with_dated(&holders, &asked, |full| {
            let rebuilt = STATE_READS.replace(0);
            let step = cursor.advance(hour);
            assert_eq!(step.newest_first().collect::<Vec<_>>(), full.rows, "rows at hour {hour}");
            assert_eq!(step.live_newest_first().len() as u64, step.live);
            assert_eq!((step.live, step.unattributed, step.until), (full.live, full.unattributed, full.until), "counts and proof at hour {hour}");
            let mut gaps: Vec<_> = full.gaps.iter().map(|gap| (gap.first_hour, gap.last_hour, &gap.reason)).collect();
            gaps.sort_unstable();
            let coverage: Vec<_> = step.gaps.iter().map(|gap| (gap.first_hour, gap.last_hour, &gap.reason)).collect();
            assert_eq!(gaps, coverage, "gaps at hour {hour}");
            if hour == 2004 {
                assert!(step.changed.iter().any(|edit| edit.at == late && !edit.visibility_changed), "live and chip counts change without a visibility edit");
            }
            if hour == 2003 {
                assert!(rebuilt >= 2000);
                assert_eq!(step.visited, 1);
                assert_eq!(STATE_READS.get(), 0);
            }
            assert_eq!(step.chips, index.ask(&holders, &asked).unwrap().chips, "filter counts at hour {hour}");
            let live_first = Ask { live_first: true, ..asked.clone() };
            index.with_dated(&holders, &live_first, |full| {
                let rows: Vec<_> = step.live_newest_first().chain(step.newest_first().filter(|&at| !step.is_live(at))).collect();
                assert_eq!(rows, full.rows, "live-first rows at hour {hour}");
            }).unwrap();
        }).unwrap();
    }
}

#[test]
fn dated_windows_read_only_page_states_and_match_the_reference() {
    let mut entries: Vec<_> = (0..2000).map(|hour| occurrence(hour as u128, hour, hour)).collect();
    entries[0].states = vec![(0, 2), (2007, 3)].into_boxed_slice();
    let mut late = occurrence(2100, 1950, 2002);
    late.states = vec![(2002, RUNNING), (2004, DONE)].into_boxed_slice();
    entries.push(late);
    entries.push(occurrence(2101, 2003, 2003));
    let gaps = vec![
        MissingSpan { first_hour: 5, last_hour: 9, reason: "old gap".into() },
        MissingSpan { first_hour: 6, last_hour: 9, reason: "overlapping gap".into() },
        MissingSpan { first_hour: 2001, last_hour: 2002, reason: "new gap".into() },
    ];
    let index = Arc::new(Index::new(entries, gaps, &[RUNNING]).unwrap().with_retirements(HashMap::from([(2100, 2005)])));
    let holders = holders();
    let chips = [Chip { kinds: None, states: Some(vec![DONE]) }];
    for direction in [Direction::Descending, Direction::Ascending] {
        for live_first in [true, false] {
            let shape = Ask { chips: &chips, live_first, order: Order::Date(direction), ..ask(1999) };
            let mut page = index.dated_window(&holders, &shape).unwrap();
            for hour in [1999, 2001, 2002, 2003, 2004, 2005, 2006, 2007, 2002, 2006] {
                for offset in [0, 1, 63, 1970, u64::MAX] {
                    let asked = Ask { hour, offset, limit: 7, ..shape.clone() };
                    STATE_READS.set(0);
                    let exact = index.ask(&holders, &asked).unwrap();
                    if hour == 2003 && offset == 0 {
                        assert!(STATE_READS.get() >= 2000);
                    }
                    STATE_READS.set(0);
                    assert_eq!(page.advance(hour, offset, asked.limit).unwrap(), exact, "{direction:?}, live first {live_first}, hour {hour}, offset {offset}");
                    assert_eq!(STATE_READS.get(), exact.rows.len());
                }
            }
        }
    }
}
