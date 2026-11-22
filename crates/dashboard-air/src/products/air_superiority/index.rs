
use super::assignment::{self, AssignedSide, FactionHistory, GroupIndex, Groups, Identity, Run, SideAnchors, WarRun};
use dashboard_base::Source;
use dashboard_world::products::groups::CountryGroupReply;
use dashboard_core::config::LoadContext;
use dashboard_query::{Query, identity::Holder};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};

pub struct Epoch {
    pub first: i32,
    pub last: i32,
    pub present: Arc<BTreeSet<String>>,
    pub sides: Option<Arc<Vec<AssignedSide>>>,
}

pub struct AirIndex {
    pub groups: Groups,
    pub anchors: SideAnchors,
    pub epochs: Vec<Epoch>,
}

impl AirIndex {
    pub fn epoch(&self, hour: i32) -> Option<&Epoch> {
        let at = self.epochs.partition_point(|epoch| epoch.last < hour);
        self.epochs.get(at).filter(|epoch| epoch.first <= hour)
    }

    pub fn anchor(&self, side: &AssignedSide) -> Option<(String, i32)> {
        let (tag, hour) = if let Some(identity) = side.key.strip_prefix("w:") {
            self.anchors.country_reads.get(identity)?
        } else {
            self.anchors.faction_reads.get(&side.faction?)?
        };
        self.groups.identity_run(tag, *hour).map(|_| (tag.clone(), *hour))
    }
}

pub fn groups_of(reply: &CountryGroupReply, query: &Query<'_>) -> Groups {
    let runs = |runs: &[dashboard_world::products::groups::MembershipRun]| runs.iter().map(|run| Run { tag: run.tag.clone(), first_hour: run.first_hour, last_hour: run.last_hour }).collect();
    let spans = |spans: &[dashboard_base::span::HourSpan]| spans.iter().map(|span| (span.first_hour, span.last_hour)).collect();
    let factions = reply
        .factions
        .iter()
        .filter_map(|faction| {
            let id = faction.id.parse::<u64>().ok()?;
            Some((id, FactionHistory { first_hour: faction.first_hour, members: runs(&faction.members), leaders: runs(&faction.leaders), incomplete: spans(&faction.incomplete) }))
        })
        .collect();
    let wars = reply.wars.iter().map(|war| WarRun { first: war.first.clone(), second: war.second.clone(), first_hour: war.first_hour, last_hour: war.last_hour }).collect();
    let identities = query
        .holders
        .tags
        .iter()
        .filter_map(|(tag, held)| {
            let runs: Vec<Identity> = held
                .iter()
                .filter_map(|run| match run.holder {
                    Holder::Found { slot, id } => Some(Identity { first_hour: run.first, last_hour: run.last, slot, country_id: id }),
                    Holder::Unresolved => None,
                })
                .collect();
            (!runs.is_empty()).then(|| (tag.clone(), runs))
        })
        .collect();
    Groups { factions, wars, unobserved_factions: spans(&reply.unobserved.factions), unobserved_wars: spans(&reply.unobserved.wars), identities }
}

fn boundaries(groups: &Groups, first: i32, last: i32) -> Vec<i32> {
    let mut cuts = BTreeSet::from([first]);
    let mut span = |a: i32, b: i32| {
        if a > first && a <= last {
            cuts.insert(a);
        }
        if let Some(after) = b.checked_add(1).filter(|after| *after > first && *after <= last) {
            cuts.insert(after);
        }
    };
    for faction in groups.factions.values() {
        for run in faction.members.iter().chain(&faction.leaders) {
            span(run.first_hour, run.last_hour);
        }
        for &(a, b) in &faction.incomplete {
            span(a, b);
        }
    }
    for war in &groups.wars {
        span(war.first_hour, war.last_hour);
    }
    for runs in groups.identities.values() {
        for run in runs {
            span(run.first_hour, run.last_hour);
        }
    }
    for &(a, b) in groups.unobserved_factions.iter().chain(&groups.unobserved_wars) {
        span(a, b);
    }
    cuts.into_iter().collect()
}

pub fn build(groups: Groups, first: i32, last: i32) -> AirIndex {
    let anchors = assignment::historical_anchors(&groups);
    let index = GroupIndex::new(&groups);
    let cuts = boundaries(&groups, first, last);
    let mut epochs: Vec<Epoch> = Vec::with_capacity(cuts.len());
    for (at, &start) in cuts.iter().enumerate() {
        let end = cuts.get(at + 1).map_or(last, |next| next - 1);
        let present: BTreeSet<String> = groups.identities.iter().filter(|(tag, _)| groups.identity_run(tag, start).is_some()).map(|(tag, _)| tag.clone()).collect();
        let sides = assignment::indexed_roster(&groups, &index, start, &present).and_then(|sides| assignment::keyed(&groups, start, sides, &anchors));
        if let Some(previous) = epochs.last_mut()
            && *previous.present == present
            && previous.sides.as_deref() == sides.as_ref()
        {
            previous.last = end;
            continue;
        }
        let present = match epochs.last() {
            Some(previous) if *previous.present == present => Arc::clone(&previous.present),
            _ => Arc::new(present),
        };
        epochs.push(Epoch { first: start, last: end, present, sides: sides.map(Arc::new) });
    }
    AirIndex { groups, anchors, epochs }
}

#[derive(Default)]
struct Held(HashMap<u64, Arc<AirIndex>>);

pub fn index(source: &Source, branch: u64) -> io::Result<Arc<AirIndex>> {
    let state = source.state::<Held>();
    if let Some(found) = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0.get(&branch) {
        return Ok(Arc::clone(found));
    }
    let query = dashboard_base::queries::query(source, branch)?;
    let reply = dashboard_world::products::groups::groups(source, &query, &branch.to_string(), &LoadContext { content: None }, &AtomicBool::new(false))?;
    let made = Arc::new(build(groups_of(&reply, &query), query.timeline.first, query.timeline.last));
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0.insert(branch, Arc::clone(&made));
    Ok(made)
}
