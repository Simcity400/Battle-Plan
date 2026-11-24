use super::{groups_at, subjects_at};
use crate::products::groups::{CountryGroupReply, FactionGroup, FactionNameRun, MembershipRun, UnobservedHours};
use crate::products::subjects::{LevelStep, SubjectReply, SubjectRun};
use dashboard_base::span::HourSpan;

fn run(tag: &str, first_hour: i32, last_hour: i32) -> MembershipRun {
    MembershipRun { tag: tag.into(), first_hour, last_hour }
}

fn faction(id: &str, first_hour: i32, last_hour: i32, members: Vec<MembershipRun>) -> FactionGroup {
    FactionGroup {
        id: id.into(),
        name: Some(format!("Later name {id}")),
        name_reason: None,
        names: vec![
            FactionNameRun { first_hour, last_hour: first_hour + 9, name: Some(format!("First name {id}")), reason: None },
            FactionNameRun { first_hour: first_hour + 10, last_hour, name: Some(format!("Later name {id}")), reason: None },
        ],
        recorded_name: String::new(),
        first_hour,
        last_hour,
        leaders: members.first().cloned().into_iter().collect(),
        members,
        incomplete: Vec::new(),
    }
}

fn reply() -> CountryGroupReply {
    CountryGroupReply {
        branch_id: "1".into(),
        humans: vec![run("AAA", 100, 300)],
        factions: vec![faction("1", 100, 300, vec![run("AAA", 100, 300), run("BBB", 121, 300)]), faction("2", 121, 300, vec![run("CCC", 121, 300)])],
        wars: Vec::new(),
        war_starts: Vec::new(),
        capitulations: Vec::new(),
        identity_changes: Vec::new(),
        country_identities: Vec::new(),
        unobserved: UnobservedHours { humans: Vec::new(), factions: vec![HourSpan { first_hour: 200, last_hour: 209 }], wars: Vec::new() },
        warnings: Vec::new(),
    }
}

#[test]
fn a_faction_or_member_first_seen_after_the_hour_is_not_in_the_reply_and_names_are_those_seen_by_it() {
    let (at, next) = groups_at(&reply(), 120);
    assert_eq!(at.factions.len(), 1, "a faction founded at 121 is not in the reply asked at 120");
    let first = &at.factions[0];
    assert_eq!(first.members.iter().map(|member| (member.tag.as_str(), member.last_hour)).collect::<Vec<_>>(), [("AAA", 120)]);
    assert_eq!((first.last_hour, first.name.as_deref()), (120, Some("Later name 1")), "named by the run that covers the hour, never a later one");
    assert_eq!(groups_at(&reply(), 105).0.factions[0].name.as_deref(), Some("First name 1"));
    assert_eq!(next, Some(121));
    let (gap, _) = groups_at(&reply(), 205);
    assert_eq!(gap.factions[0].members[0].last_hour, 199, "inside an unread stretch a run ends at the last hour read");
    assert_eq!(gap.unobserved.factions, vec![HourSpan { first_hour: 200, last_hour: 205 }]);
}

#[test]
fn a_subject_relation_first_seen_after_the_hour_is_not_in_the_reply_and_its_levels_are_cut() {
    let step = |first_hour, last_hour, level: &str| LevelStep { first_hour, last_hour, level: Some(level.into()) };
    let whole = SubjectReply {
        branch_id: "1".into(),
        levels: Vec::new(),
        runs: vec![
            SubjectRun { overlord: "AAA".into(), subject: "BBB".into(), start_hour: 90, first_hour: 100, last_hour: 300, steps: vec![step(100, 149, "low"), step(150, 300, "high")] },
            SubjectRun { overlord: "AAA".into(), subject: "CCC".into(), start_hour: 131, first_hour: 131, last_hour: 300, steps: vec![step(131, 300, "low")] },
        ],
        unobserved: Vec::new(),
        warnings: Vec::new(),
    };
    let (at, next) = subjects_at(&whole, 130);
    assert_eq!(at.runs.len(), 1, "a relation first seen at 131 is not in the reply asked at 130");
    assert_eq!((at.runs[0].last_hour, at.runs[0].steps.len()), (130, 1), "a level that starts after the hour is not there");
    assert_eq!(next, Some(131));
}
