use super::groups::{CountryGroupReply, MembershipRun};
use dashboard_base::{Source, products::LoadContext, span::HourSpan};
use dashboard_query::{
    identity::{Country, Holders},
    series::ranked::{GroupKind, GroupMembership, GroupSelection, Members},
};
use std::{
    collections::HashSet,
    io,
    sync::{Arc, atomic::AtomicBool},
};

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

#[derive(Clone, Default)]
pub struct GroupsAt {
    reply: Option<Arc<CountryGroupReply>>,
    first: i32,
}

impl GroupsAt {
    pub fn load(source: &Source, load: &LoadContext, branch_id: &str, group: Option<&GroupSelection>) -> io::Result<Self> {
        let (Some(_), Ok(branch)) = (group, branch_id.parse::<u64>()) else { return Ok(Self::default()) };
        let query = dashboard_base::queries::query(source, branch)?;
        let reply = super::groups::shared(source, &query, load, &AtomicBool::new(false))?;
        Ok(Self { reply: Some(reply), first: query.timeline.first })
    }

    pub fn members(&self, group: &GroupSelection, hour: i32) -> io::Result<Members> {
        let reply = self.reply.as_deref().ok_or_else(|| invalid("The group's memberships were not loaded"))?;
        members_at(reply, group, hour, self.first)
    }

    pub fn chosen_slots(&self, group: Option<&GroupSelection>, countries: &[Country], holders: &Holders, hour: i32) -> io::Result<Result<Option<HashSet<u32>>, String>> {
        let slot = |country: &Country| holders.slot_at(country, hour);
        let Some(group) = group else { return Ok(Ok((!countries.is_empty()).then(|| countries.iter().filter_map(slot).collect()))) };
        let members = self.members(group, hour)?;
        if let Some(refused) = members.refused {
            return Ok(Err(refused));
        }
        Ok(Ok(Some(members.tags.into_iter().filter_map(|tag| slot(&Country::Tag(tag))).collect())))
    }

    pub fn valid_through(&self, group: Option<&GroupSelection>, hour: i32) -> i32 {
        group.map_or(i32::MAX, |group| self.members(group, hour).map_or(hour, |members| members.until.max(hour)))
    }
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

    #[test]
    fn flows_choose_the_group_members_at_the_hour_by_slot_and_hold_only_until_the_next_join() {
        use dashboard_query::identity::{Holder, HolderRun};
        let run = |slot: u32| vec![HolderRun { first: 0, last: 1000, holder: Holder::Found { slot, id: slot * 10 }, starts_segment: true }];
        let holders = Holders::new([("GER".to_owned(), run(7)), ("ITA".to_owned(), run(8))].into_iter().collect(), Vec::new());
        let groups = GroupsAt { reply: Some(Arc::new(reply())), first: 0 };
        let group = faction(GroupMembership::Hour);
        let chosen = |hour: i32| groups.chosen_slots(Some(&group), &[], &holders, hour).unwrap();
        assert_eq!(chosen(500), Ok(Some([7].into_iter().collect())), "ITA joins at 600: its flows are not the faction's before");
        assert_eq!(chosen(700), Ok(Some([7, 8].into_iter().collect())));
        assert_eq!(chosen(310), Err(UNKNOWN.to_owned()));
        assert_eq!(groups.valid_through(Some(&group), 500), 599);
        assert_eq!(groups.valid_through(None, 500), i32::MAX);
        assert_eq!(groups.chosen_slots(None, &[Country::Tag("ITA".into())], &holders, 500).unwrap(), Ok(Some([8].into_iter().collect())));
    }
}
