use super::{Building, GroupCount, CountedGroup, Index, ROSTER_UNREAD, SUBJECTS_UNREAD};
use dashboard_base::span::HourSpan;
use std::collections::BTreeMap;

fn span(first_hour: i32, last_hour: i32) -> HourSpan {
    HourSpan { first_hour, last_hour }
}

fn index(groups: &[(&str, &[(i32, i32)], &[(i32, i32)])], unread: &[(i32, i32)]) -> Index {
    let mut built: BTreeMap<String, Building> = BTreeMap::new();
    for (key, held, gaps) in groups {
        let building = built.entry((*key).to_owned()).or_default();
        for &(first, last) in *held {
            building.held(first, last);
        }
        building.unread.extend(gaps.iter().map(|&(first, last)| span(first, last)));
    }
    Index::of(built, unread.iter().map(|&(first, last)| span(first, last)).collect())
}

fn shown(groups: &[GroupCount]) -> Vec<(&str, Option<u32>)> {
    groups.iter().map(|group| (group.key.as_str(), group.count)).collect()
}

#[test]
fn groups_are_ranked_by_their_count_at_the_hour_and_one_whose_count_is_unknown_is_listed_after_with_its_reason() {
    let index = index(
        &[
            ("1", &[(100, 300), (110, 300)], &[]),
            ("2", &[(105, 300), (120, 300), (130, 300)], &[(140, 150)]),
            ("3", &[(90, 200)], &[]),
        ],
        &[(250, 260)],
    );
    let (missing, groups) = index.answer(CountedGroup::Factions, 125);
    assert_eq!(missing, None);
    assert_eq!(shown(&groups), [("1", Some(2)), ("2", Some(2)), ("3", Some(1))], "ties go to the earliest");
    let (_, groups) = index.answer(CountedGroup::Factions, 145);
    assert_eq!(shown(&groups), [("1", Some(2)), ("3", Some(1)), ("2", None)], "a roster not read whole is not ranked as zero");
    assert_eq!(groups[2].missing.as_deref(), Some(ROSTER_UNREAD));
    let (_, groups) = index.answer(CountedGroup::Factions, 210);
    assert_eq!(shown(&groups), [("2", Some(3)), ("1", Some(2)), ("3", Some(0))], "a dissolved faction is witnessed empty");
    let (_, groups) = index.answer(CountedGroup::Factions, 95);
    assert_eq!(shown(&groups), [("3", Some(1))], "a group begun after the hour is not there");
    let (missing, groups) = index.answer(CountedGroup::Factions, 255);
    assert!(missing.is_some() && groups.iter().all(|group| group.count.is_none()), "an unread hour ranks nothing");
    assert_eq!((index.next_change(125), index.next_change(201), index.next_change(301)), (Some(130), Some(250), None));
}

#[test]
fn overlords_are_those_holding_a_subject_at_the_hour_and_an_unread_hour_names_none() {
    let index = index(&[("AAA", &[(100, 200)], &[]), ("BBB", &[(100, 150), (100, 300)], &[])], &[(160, 170)]);
    assert_eq!(shown(&index.answer(CountedGroup::Overlords, 120).1), [("BBB", Some(2)), ("AAA", Some(1))]);
    assert_eq!(shown(&index.answer(CountedGroup::Overlords, 250).1), [("BBB", Some(1))], "an overlord with no subject left is not ranked");
    assert_eq!(index.answer(CountedGroup::Overlords, 165), (Some(SUBJECTS_UNREAD.to_owned()), Vec::new()));
}
