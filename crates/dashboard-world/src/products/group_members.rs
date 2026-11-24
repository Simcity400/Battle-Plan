use super::groups::{CountryGroupReply, MembershipRun};
use dashboard_base::span::HourSpan;
use dashboard_query::series::ranked::{GroupKind, GroupMembership, GroupSelection, Members};
use std::io;

const UNKNOWN: &str = "Membership of this group is not recorded at this hour";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub fn members_at(reply: &CountryGroupReply, group: &GroupSelection, hour: i32, first: i32) -> io::Result<Members> {
    let war_runs: Vec<MembershipRun>;
    let faction_unknown: Vec<HourSpan>;
    let (runs, unknown): (&[MembershipRun], &[HourSpan]) = match group.kind {
        GroupKind::Humans => (&reply.humans, &reply.unobserved.humans),
        GroupKind::Faction => {
            let id = group.id.as_deref().ok_or_else(|| invalid("A faction group names its id"))?;
            let Some(faction) = reply.factions.iter().find(|faction| faction.id == id) else {
                return Ok(Members { tags: Vec::new(), refused: Some(format!("No faction {id} in this history")), from: hour, until: hour });
            };
            faction_unknown = reply.unobserved.factions.iter().chain(&faction.incomplete).cloned().collect();
            (&faction.members, &faction_unknown)
        }
        GroupKind::War => {
            let tag = group.tag.as_deref().ok_or_else(|| invalid("A war group names its country"))?;
            war_runs = reply
                .wars
                .iter()
                .filter_map(|pair| {
                    let enemy = if pair.first == tag { &pair.second } else if pair.second == tag { &pair.first } else { return None };
                    Some(MembershipRun { tag: enemy.clone(), first_hour: pair.first_hour, last_hour: pair.last_hour })
                })
                .collect();
            (&war_runs, &reply.unobserved.wars)
        }
    };
    let counted = |span: &HourSpan| match group.membership {
        GroupMembership::Hour => span.first_hour <= hour && hour <= span.last_hour,
        GroupMembership::Ever => span.first_hour <= hour && span.last_hour >= first,
    };
    if let Some(span) = unknown.iter().find(|span| counted(span)) {
        let (from, until) = match group.membership {
            GroupMembership::Hour => (span.first_hour, span.last_hour),
            GroupMembership::Ever => (hour, hour),
        };
        return Ok(Members { tags: Vec::new(), refused: Some(UNKNOWN.to_owned()), from, until });
    }
    let member = |run: &MembershipRun| match group.membership {
        GroupMembership::Hour => run.first_hour <= hour && hour <= run.last_hour,
        GroupMembership::Ever => run.first_hour <= hour,
    };
    let mut tags: Vec<String> = runs.iter().filter(|run| member(run)).map(|run| run.tag.clone()).collect();
    tags.sort_unstable();
    tags.dedup();
    let (mut from, mut until) = (i32::MIN, i32::MAX);
    let mut edge = |start: i32, past: i32| {
        if start <= hour {
            from = from.max(start);
        } else {
            until = until.min(start - 1);
        }
        if past <= hour {
            from = from.max(past);
        } else {
            until = until.min(past - 1);
        }
    };
    for run in runs {
        match group.membership {
            GroupMembership::Hour => edge(run.first_hour, run.last_hour.saturating_add(1)),
            GroupMembership::Ever => edge(run.first_hour, run.first_hour),
        }
    }
    for span in unknown {
        edge(span.first_hour, span.last_hour.saturating_add(1));
    }
    Ok(Members { tags, refused: None, from, until })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::groups::{FactionGroup, UnobservedHours, WarPairRun};

    fn run(tag: &str, first_hour: i32, last_hour: i32) -> MembershipRun {
        MembershipRun { tag: tag.into(), first_hour, last_hour }
    }

    fn reply() -> CountryGroupReply {
        CountryGroupReply {
            branch_id: "1".into(),
            humans: vec![run("GER", 0, 1000)],
            factions: vec![FactionGroup {
                id: "42".into(),
                name: None,
                name_reason: None,
                names: Vec::new(),
                recorded_name: String::new(),
                first_hour: 0,
                last_hour: 1000,
                members: vec![run("GER", 0, 1000), run("ITA", 600, 1000)],
                leaders: Vec::new(),
                incomplete: vec![HourSpan { first_hour: 300, last_hour: 320 }],
            }],
            wars: vec![WarPairRun { first: "GER".into(), second: "POL".into(), first_hour: 100, last_hour: 200 }],
            war_starts: Vec::new(),
            capitulations: Vec::new(),
            identity_changes: Vec::new(),
            country_identities: Vec::new(),
            unobserved: UnobservedHours::default(),
            warnings: Vec::new(),
        }
    }

    fn faction(membership: GroupMembership) -> GroupSelection {
        GroupSelection { kind: GroupKind::Faction, id: Some("42".into()), tag: None, membership }
    }

    #[test]
    fn a_group_is_its_members_at_the_asked_hour_and_unknown_membership_stays_unknown() {
        let at = |hour: i32| members_at(&reply(), &faction(GroupMembership::Hour), hour, 0).unwrap();
        assert_eq!(at(500), Members { tags: vec!["GER".into()], refused: None, from: 321, until: 599 }, "a country joining tomorrow is not a member today");
        assert_eq!(at(700).tags, ["GER", "ITA"]);
        assert_eq!(at(310), Members { tags: Vec::new(), refused: Some(UNKNOWN.into()), from: 300, until: 320 }, "an unknown roster is never a guess");
        let ever = |hour: i32| members_at(&reply(), &faction(GroupMembership::Ever), hour, 0).unwrap();
        assert_eq!(ever(200).tags, ["GER"], "every member so far, never one from later");
        assert_eq!(ever(700).refused.as_deref(), Some(UNKNOWN), "a past unknown roster leaves who was ever a member unknown");
        let war = GroupSelection { kind: GroupKind::War, id: None, tag: Some("GER".into()), membership: GroupMembership::Hour };
        assert_eq!(members_at(&reply(), &war, 150, 0).unwrap(), Members { tags: vec!["POL".into()], refused: None, from: 100, until: 200 });
        assert_eq!(members_at(&reply(), &war, 250, 0).unwrap().tags, Vec::<String>::new(), "a known empty group is empty, not unknown");
        let missing = GroupSelection { id: Some("7".into()), ..faction(GroupMembership::Hour) };
        assert!(members_at(&reply(), &missing, 150, 0).unwrap().refused.is_some());
    }
}
