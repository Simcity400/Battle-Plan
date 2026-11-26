use super::groups::{self, CountryGroupReply, MembershipRun};
use dashboard_base::{Source, products::LoadContext, span::HourSpan};
use dashboard_query::{
    Query,
    identity::{Country, Holder, Holders, SLOT_MARK, TAG_UNLINKED},
    log::{CountrySpan, UnobservedSpan, occurrence::Selection},
    series::{MissingSpan, ranked::{GroupKind, GroupMembership, GroupSelection, Members}},
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

const UNKNOWN: &str = "Membership of this group is not recorded at this hour";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub struct Runs<'r> {
    pub members: Cow<'r, [MembershipRun]>,
    pub unknown: Vec<HourSpan>,
}

fn slot_of(key: &str) -> Option<u32> {
    key.strip_prefix(SLOT_MARK)?.parse().ok()
}

fn enemies_of_slot(reply: &CountryGroupReply, slot: u32) -> Vec<MembershipRun> {
    let held: HashMap<&str, Vec<(i32, i32)>> = reply
        .country_identities
        .iter()
        .map(|history| (history.tag.as_str(), history.identities.iter().filter(|run| run.slot == slot).map(|run| (run.first_hour, run.last_hour)).collect::<Vec<_>>()))
        .filter(|(_, spans)| !spans.is_empty())
        .collect();
    let mut members = Vec::new();
    for pair in &reply.wars {
        for (mine, enemy) in [(&pair.first, &pair.second), (&pair.second, &pair.first)] {
            for &(first, last) in held.get(mine.as_str()).into_iter().flatten() {
                let (first_hour, last_hour) = (first.max(pair.first_hour), last.min(pair.last_hour));
                if first_hour <= last_hour {
                    members.push(MembershipRun { tag: enemy.clone(), first_hour, last_hour });
                }
            }
        }
    }
    members
}

pub fn runs<'r>(reply: &'r CountryGroupReply, group: &GroupSelection) -> io::Result<Result<Runs<'r>, String>> {
    match group.kind {
        GroupKind::Humans => Ok(Ok(Runs { members: Cow::Borrowed(&reply.humans), unknown: reply.unobserved.humans.clone() })),
        GroupKind::Faction => {
            let id = group.id.as_deref().ok_or_else(|| invalid("A faction group names its id"))?;
            let Some(faction) = reply.factions.iter().find(|faction| faction.id == id) else {
                return Ok(Err(format!("No faction {id} in this history")));
            };
            Ok(Ok(Runs { members: Cow::Borrowed(&faction.members), unknown: reply.unobserved.factions.iter().chain(&faction.incomplete).cloned().collect() }))
        }
        GroupKind::War => {
            let tag = group.tag.as_deref().ok_or_else(|| invalid("A war group names its country"))?;
            let members = match slot_of(tag) {
                Some(slot) => enemies_of_slot(reply, slot),
                None => reply
                    .wars
                    .iter()
                    .filter_map(|pair| {
                        let enemy = if pair.first == tag { &pair.second } else if pair.second == tag { &pair.first } else { return None };
                        Some(MembershipRun { tag: enemy.clone(), first_hour: pair.first_hour, last_hour: pair.last_hour })
                    })
                    .collect(),
            };
            Ok(Ok(Runs { members: Cow::Owned(members), unknown: reply.unobserved.wars.clone() }))
        }
    }
}

pub fn dated(source: &Source, query: &Query<'_>, context: &LoadContext, countries: Option<&[Country]>, group: Option<&GroupSelection>, cancelled: &AtomicBool) -> io::Result<Option<Selection>> {
    let (first, last) = (query.timeline.first, query.timeline.last);
    match (countries, group) {
        (Some(_), Some(_)) => Err(invalid("Ask a group or countries, never both")),
        (None, None) => Ok(None),
        (Some(countries), None) => Ok(Some(Selection {
            membership: countries.iter().map(|country| CountrySpan { country: country.clone(), first_hour: first, last_hour: last }).collect(),
            unobserved: Vec::new(),
            unknown: Vec::new(),
        })),
        (None, Some(group)) => {
            let reply = groups::shared(source, query, context, cancelled)?;
            let found = runs(&reply, group)?.map_err(dashboard_core::refused)?;
            let unobserved = found.unknown.iter().map(|span| UnobservedSpan { first_hour: span.first_hour, last_hour: span.last_hour }).collect();
            match group.membership {
                GroupMembership::Hour => Ok(Some(Selection {
                    membership: found.members.iter().map(|run| CountrySpan { country: Country::Tag(run.tag.clone()), first_hour: run.first_hour, last_hour: run.last_hour }).collect(),
                    unobserved,
                    unknown: Vec::new(),
                })),
                GroupMembership::Ever => {
                    let mut slots: BTreeMap<u32, i32> = BTreeMap::new();
                    let mut unknown: Vec<MissingSpan> = Vec::new();
                    for run in found.members.iter() {
                        for held in query.holders.within(&run.tag, run.first_hour, run.last_hour) {
                            let start = held.first.max(run.first_hour);
                            let end = held.last.min(run.last_hour);
                            if start > end {
                                continue;
                            }
                            match held.holder {
                                Holder::Found { slot, .. } => {
                                    slots.entry(slot).and_modify(|prev| *prev = (*prev).min(start)).or_insert(start);
                                }
                                Holder::Unresolved => {
                                    unknown.push(MissingSpan {
                                        first_hour: start,
                                        last_hour: last,
                                        reason: format!("Who holds {} is not known: {TAG_UNLINKED}", run.tag),
                                    });
                                }
                            }
                        }
                        for (u_first, u_last, reason) in &query.holders.unknown {
                            let start = (*u_first).max(run.first_hour);
                            let end = (*u_last).min(run.last_hour);
                            if start <= end {
                                unknown.push(MissingSpan {
                                    first_hour: start,
                                    last_hour: last,
                                    reason: format!("Who holds {} is not known: {reason}", run.tag),
                                });
                            }
                        }
                    }
                    let membership = slots
                        .into_iter()
                        .map(|(slot, start)| CountrySpan { country: Country::Slot(slot), first_hour: start, last_hour: last })
                        .collect();
                    Ok(Some(Selection { membership, unobserved, unknown }))
                }
            }
        }
    }
}

pub fn members_at(reply: &CountryGroupReply, group: &GroupSelection, hour: i32, first: i32) -> io::Result<Members> {
    let found = match runs(reply, group)? {
        Ok(found) => found,
        Err(refused) => return Ok(Members { tags: Vec::new(), refused: Some(refused), from: hour, until: hour }),
    };
    let (runs, unknown) = (&*found.members, found.unknown.as_slice());
    let counted = |span: &HourSpan| match group.membership {
        GroupMembership::Hour => span.first_hour <= hour && hour <= span.last_hour,
        GroupMembership::Ever => span.first_hour <= hour && span.last_hour >= first,
    };
    if let Some(span) = unknown.iter().find(|span| counted(span)) {
        let (from, until) = match group.membership {
            GroupMembership::Hour => (span.first_hour, span.last_hour),
            GroupMembership::Ever => (unknown.iter().filter(|span| counted(span)).map(|span| span.first_hour).min().unwrap_or(hour), i32::MAX),
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
    held: Arc<Mutex<Option<(GroupSelection, Members)>>>,
    ever: Arc<Mutex<Option<EverSlots>>>,
}

struct EverSlots {
    group: GroupSelection,
    from: i32,
    until: i32,
    slots: Result<HashSet<u32>, String>,
}

impl GroupsAt {
    pub fn new(reply: Arc<CountryGroupReply>, first: i32) -> Self {
        Self { reply: Some(reply), first, held: Arc::default(), ever: Arc::default() }
    }

    pub fn load(source: &Source, load: &LoadContext, branch_id: &str, group: Option<&GroupSelection>) -> io::Result<Self> {
        let (Some(_), Ok(branch)) = (group, branch_id.parse::<u64>()) else { return Ok(Self::default()) };
        let query = dashboard_base::queries::query(source, branch)?;
        let reply = groups::shared(source, &query, load, &AtomicBool::new(false))?;
        Ok(Self::new(reply, query.timeline.first))
    }

    pub fn load_all(source: &Source, load: &LoadContext, branch_id: &str) -> io::Result<Self> {
        let Ok(branch) = branch_id.parse::<u64>() else { return Ok(Self::default()) };
        let query = dashboard_base::queries::query(source, branch)?;
        let reply = groups::shared(source, &query, load, &AtomicBool::new(false))?;
        Ok(Self::new(reply, query.timeline.first))
    }

    pub fn reply(&self) -> Option<&CountryGroupReply> {
        self.reply.as_deref()
    }

    pub fn members(&self, group: &GroupSelection, hour: i32) -> io::Result<Members> {
        let reply = self.reply.as_deref().ok_or_else(|| invalid("The group's memberships were not loaded"))?;
        let mut held = self.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((asked, members)) = held.as_ref()
            && asked == group
            && members.from <= hour
            && hour <= members.until
        {
            return Ok(members.clone());
        }
        let members = members_at(reply, group, hour, self.first)?;
        *held = Some((group.clone(), members.clone()));
        Ok(members)
    }

    pub fn chosen_slots(&self, group: Option<&GroupSelection>, countries: &[Country], holders: &Holders, hour: i32) -> io::Result<Result<Option<HashSet<u32>>, String>> {
        let slot = |country: &Country| holders.slot_at(country, hour);
        let Some(group) = group else { return Ok(Ok((!countries.is_empty()).then(|| countries.iter().filter_map(slot).collect()))) };
        let members = self.members(group, hour)?;
        if let Some(refused) = members.refused {
            return Ok(Err(refused));
        }
        if group.membership == GroupMembership::Hour {
            return Ok(Ok(Some(members.tags.into_iter().filter_map(|tag| slot(&Country::Tag(tag))).collect())));
        }
        let mut ever = self.ever.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(held) = ever.as_ref()
            && held.group == *group
            && held.from <= hour
            && hour <= held.until
        {
            return Ok(held.slots.clone().map(Some));
        }
        let reply = self.reply.as_deref().ok_or_else(|| invalid("The group's memberships were not loaded"))?;
        let found = runs(reply, group)?.map_err(dashboard_core::refused)?;
        let (slots, until) = ever_slots(&found.members, holders, hour, members.until);
        *ever = Some(EverSlots { group: group.clone(), from: hour, until, slots: slots.clone() });
        Ok(slots.map(Some))
    }

    pub fn valid_through(&self, group: Option<&GroupSelection>, hour: i32) -> i32 {
        group.map_or(i32::MAX, |group| self.members(group, hour).map_or(hour, |members| members.until.max(hour)))
    }
}

fn ever_slots(members: &[MembershipRun], holders: &Holders, hour: i32, mut until: i32) -> (Result<HashSet<u32>, String>, i32) {
    let mut slots = HashSet::new();
    for run in members.iter().filter(|run| run.first_hour <= hour) {
        let last = run.last_hour.min(hour);
        for held in holders.within(&run.tag, run.first_hour, run.last_hour) {
            if held.first > last {
                until = until.min(held.first - 1);
                break;
            }
            match held.holder {
                Holder::Found { slot, .. } => {
                    slots.insert(slot);
                }
                Holder::Unresolved => return (Err(format!("Who held {} while it was a member is not known: {TAG_UNLINKED}", run.tag)), i32::MAX),
            }
        }
        for (first, past, reason) in &holders.unknown {
            if *first <= last && run.first_hour <= *past {
                return (Err(format!("Who held {} while it was a member is not known: {reason}", run.tag)), i32::MAX);
            }
            if *first > last && *first <= run.last_hour {
                until = until.min(*first - 1);
            }
        }
    }
    (Ok(slots), until)
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
        let groups = GroupsAt::new(Arc::new(reply()), 0);
        let group = faction(GroupMembership::Hour);
        let chosen = |hour: i32| groups.chosen_slots(Some(&group), &[], &holders, hour).unwrap();
        assert_eq!(chosen(500), Ok(Some([7].into_iter().collect())), "ITA joins at 600: its flows are not the faction's before");
        assert_eq!(chosen(700), Ok(Some([7, 8].into_iter().collect())));
        assert_eq!(chosen(310), Err(UNKNOWN.to_owned()));
        assert_eq!(groups.valid_through(Some(&group), 500), 599);
        assert_eq!(groups.valid_through(None, 500), i32::MAX);
        assert_eq!(groups.chosen_slots(None, &[Country::Tag("ITA".into())], &holders, 500).unwrap(), Ok(Some([8].into_iter().collect())));
    }

    #[test]
    fn every_member_so_far_is_the_slot_that_held_the_tag_while_a_member_never_a_later_holder_of_it() {
        use dashboard_query::identity::{Holder, HolderRun};
        let held = |slot: u32, first: i32, last: i32| HolderRun { first, last, holder: Holder::Found { slot, id: slot * 10 }, starts_segment: true };
        let holders = Holders::new(
            [("GER".to_owned(), vec![held(7, 0, 400), held(8, 401, 1000)]), ("ITA".to_owned(), vec![held(9, 0, 1000)])].into_iter().collect(),
            Vec::new(),
        );
        let mut history = reply();
        history.factions[0].members = vec![run("GER", 0, 300), run("ITA", 600, 1000)];
        history.factions[0].incomplete.clear();
        let history = Arc::new(history);
        let groups = GroupsAt::new(Arc::clone(&history), 0);
        let ever = faction(GroupMembership::Ever);
        assert_eq!(groups.chosen_slots(Some(&ever), &[], &holders, 500).unwrap(), Ok(Some([7].into_iter().collect())), "slot 8 took GER after GER's membership ended: it never was a member");
        let unread = Holders::new([("GER".to_owned(), vec![held(7, 0, 100)]), ("ITA".to_owned(), vec![held(9, 0, 1000)])].into_iter().collect(), vec![(101, 200, "holders not read".to_owned())]);
        assert_eq!(groups.chosen_slots(Some(&ever), &[], &holders, 550).unwrap(), Ok(Some([7].into_iter().collect())), "reused until ITA joins");
        assert_eq!(groups.chosen_slots(Some(&ever), &[], &holders, 700).unwrap(), Ok(Some([7, 9].into_iter().collect())), "and asked again once ITA has joined");
        let fresh = GroupsAt::new(history, 0);
        assert!(fresh.chosen_slots(Some(&ever), &[], &unread, 500).unwrap().unwrap_err().contains("holders not read"), "an unknown holder during membership is unknown membership, not no member");
    }

    #[test]
    fn a_war_group_chosen_by_recorded_slot_is_that_slots_enemies_under_each_tag_it_held() {
        use crate::products::{groups::CountryIdentityHistory, identities::CountryIdentityRun};
        let mut history = reply();
        history.wars = vec![
            WarPairRun { first: "GER".into(), second: "POL".into(), first_hour: 100, last_hour: 200 },
            WarPairRun { first: "DDR".into(), second: "FRA".into(), first_hour: 300, last_hour: 400 },
            WarPairRun { first: "GER".into(), second: "ENG".into(), first_hour: 500, last_hour: 600 },
        ];
        let identity = |slot: u32, first_hour: i32, last_hour: i32| CountryIdentityRun { first_hour, last_hour, slot, country_id: slot * 10 };
        history.country_identities = vec![
            CountryIdentityHistory { tag: "GER".into(), identities: vec![identity(7, 0, 250), identity(8, 251, 1000)] },
            CountryIdentityHistory { tag: "DDR".into(), identities: vec![identity(7, 251, 1000)] },
        ];
        let war = GroupSelection { kind: GroupKind::War, id: None, tag: Some(format!("{SLOT_MARK}7")), membership: GroupMembership::Hour };
        assert_eq!(members_at(&history, &war, 150, 0).unwrap().tags, ["POL"]);
        assert_eq!(members_at(&history, &war, 350, 0).unwrap().tags, ["FRA"], "slot 7 at war under its later tag");
        assert_eq!(members_at(&history, &war, 550, 0).unwrap().tags, Vec::<String>::new(), "GER's war at 500 is slot 8's, not slot 7's");
    }

    #[test]
    fn ever_dated_selection_resolves_slots_before_extending_past_tag_swap() {
        use dashboard_base::{SourceKey, test_fixtures::{CAMPAIGN, records}};
        use dashboard_query::log::occurrence::{Ask, Direction, Index, Occurrence, Order, Role};
        use replay::campaign::Replay;
        use replay_data::rows::{CampaignRow, CountryLinkRow, CountryRow, FactionMemberRow, FactionRow, Recorded, TableId, TableRows};
        use replay_frame::frame::{DecodedFrame, Metrics, TableView};

        let links = vec![
            CountryLinkRow { id: 9001, canonical_slot: 7 },
            CountryLinkRow { id: 9002, canonical_slot: 8 },
        ];
        let factions = vec![FactionRow { id: 10, name: b"Axis".to_vec(), ..FactionRow::default() }];
        let frames: Vec<DecodedFrame> = (100..=120)
            .map(|hour| {
                let mut tables = std::array::from_fn(|_| TableView::Unavailable { reason: "outside fixture".into() });
                tables[TableId::Campaign.index()] = TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: CAMPAIGN.into() }]));
                let countries = if hour <= 110 {
                    vec![
                        CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9001, canonical_slot: 7, tag: b"AAA".to_vec(), ..CountryRow::default() },
                        CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9002, canonical_slot: 8, tag: b"CCC".to_vec(), ..CountryRow::default() },
                    ]
                } else {
                    vec![
                        CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9001, canonical_slot: 7, tag: b"BBB".to_vec(), ..CountryRow::default() },
                        CountryRow { border_land_doctrine_level: Recorded::Value(None), id: 9002, canonical_slot: 8, tag: b"AAA".to_vec(), ..CountryRow::default() },
                    ]
                };
                tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
                tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links.clone()));
                tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(factions.clone()));
                let members = if hour <= 110 {
                    vec![FactionMemberRow { country: 9001, faction: 10, ordinal: 0 }]
                } else {
                    Vec::new()
                };
                tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(members));
                DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Metrics::default() }
            })
            .collect();
        let built = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let source = Source::from_replay(
            SourceKey { source_id: "ever-tag-swap".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(built),
            &AtomicBool::new(false),
        )
        .unwrap();
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let group = GroupSelection { kind: GroupKind::Faction, id: Some("10".into()), tag: None, membership: GroupMembership::Ever };
        let selection = dated(&source, &query, &LoadContext { content: None }, None, Some(&group), &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        let entries = vec![
            Occurrence { id: 1, date: 105, observed: 105, kind: 0, roles: Box::new([Role { slot: Some(7), at: 105 }]), subjects: Box::new([]), states: Box::new([]), sort_keys: Box::new([]), text: "".into(), payload: () },
            Occurrence { id: 2, date: 115, observed: 115, kind: 0, roles: Box::new([Role { slot: Some(7), at: 115 }]), subjects: Box::new([]), states: Box::new([]), sort_keys: Box::new([]), text: "".into(), payload: () },
            Occurrence { id: 3, date: 116, observed: 116, kind: 0, roles: Box::new([Role { slot: Some(8), at: 116 }]), subjects: Box::new([]), states: Box::new([]), sort_keys: Box::new([]), text: "".into(), payload: () },
        ];
        let index = Index::new(entries, Vec::new(), &[]).unwrap();
        let answer = index
            .ask(
                &query.holders,
                &Ask {
                    hour: 120,
                    from: None,
                    to: None,
                    selection: Some(&selection),
                    subject: None,
                    kinds: None,
                    states: None,
                    search: None,
                    order: Order::Date(Direction::Descending),
                    live_first: false,
                    offset: 0,
                    limit: 64,
                    chips: &[],
                },
            )
            .unwrap();
        let ids: Vec<u128> = answer.rows.iter().map(|shown| index.get(shown.at).id).collect();
        assert_eq!(ids, [2, 1], "member slot 7 keeps its later BBB row and nonmember slot 8 taking AAA is excluded");
    }
}
