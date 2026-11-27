
use super::{
    WarCapitulation, WarName, WarObservation, WarParticipantRun, WarRelationKey, WarRelationMembership, WarSide, WarTimeline, WarsReply,
    inputs::Countries,
    native::{Blob, Pair, World},
    naming,
};
use dashboard_world::products::groups::MembershipRun;
use dashboard_base::span::HourSpan;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};

const CAPITULATIONS_UNOBSERVED: &str = "Capitulations whose country identity at the event hour is unobserved are omitted.";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RelationRun {
    first: String,
    second: String,
    start: i32,
    identity: WarRelationKey,
    oriented: bool,
}

#[derive(Clone)]
pub(crate) struct Observed {
    data: WarObservation,
    relations: Vec<RelationRun>,
    representative: Option<WarRelationKey>,
}

#[derive(Clone, Debug)]
struct OpenWar {
    id: usize,
    groups: usize,
    relations: Vec<RelationRun>,
}

#[derive(Clone)]
pub(crate) struct Viewed {
    pub countries: Arc<Countries>,
    pub view: u32,
}

pub(crate) struct Seen<'a> {
    pub hour: i32,
    pub read: Result<(Viewed, Vec<Observed>), String>,
    pub capitulations: Result<&'a [(u32, i32)], ()>,
}

pub(crate) fn observed(
    world: &World,
    (tags, identities): (&HashMap<u32, String>, &HashMap<u32, (u32, u32)>),
    view: u32,
    (blob, pairs): (&Blob, &[Pair]),
    named: Result<(Pair, WarName), String>,
    hour: i32,
) -> Observed {
    let first: HashSet<_> = blob.first.iter().copied().collect();
    let relations = pairs
        .iter()
        .map(|&p| {
            let first_side = first.contains(&world.relations[&p].first_country);
            let r = &world.relations[&p];
            let (a, b) = (&tags[&p.0], &tags[&p.1]);
            let (low, high) = if a < b { (a, b) } else { (b, a) };
            let identity = WarRelationKey::new(identities[&p.0], identities[&p.1], r.start_hour);
            let canonical = identities[&r.first_country] == identity.first;
            RelationRun { first: low.clone(), second: high.clone(), start: r.start_hour, oriented: first_side == canonical, identity }
        })
        .collect();
    let representative = named.as_ref().ok().map(|(p, _)| WarRelationKey::new(identities[&p.0], identities[&p.1], world.relations[p].start_hour));
    let (orientation, represented, name) = match named {
        Ok((p, name)) => {
            let r = &world.relations[&p];
            let instigator = if r.first_was_instigator != 0 { r.first_country } else { r.second_country };
            (Some(first.contains(&instigator)), Some([tags[&r.first_country].clone(), tags[&r.second_country].clone()]), name)
        }
        Err(reason) => (None, None, naming::unknown(reason)),
    };
    Observed {
        relations,
        representative,
        data: WarObservation {
            first_hour: hour,
            last_hour: hour,
            viewpoint_tag: tags[&view].clone(),
            viewpoint_side: blob.first.iter().map(|s| tags[s].clone()).collect(),
            opposing_side: blob.second.iter().map(|s| tags[s].clone()).collect(),
            attackers_are_viewpoint: orientation,
            representative: represented,
            name,
        },
    }
}

fn opening(observed: &[Observed], hour: i32) -> Vec<Observed> {
    observed
        .iter()
        .map(|o| Observed { data: WarObservation { first_hour: hour, last_hour: hour, ..o.data.clone() }, ..o.clone() })
        .collect()
}

fn same_groups(before: &[Observed], held: &[Observed]) -> bool {
    before.len() == held.len()
        && before.iter().zip(held).all(|(a, b)| {
            let WarObservation { first_hour: _, last_hour: _, viewpoint_tag, viewpoint_side, opposing_side, attackers_are_viewpoint, representative, name } = &a.data;
            a.relations == b.relations
                && a.representative == b.representative
                && *viewpoint_tag == b.data.viewpoint_tag
                && *viewpoint_side == b.data.viewpoint_side
                && *opposing_side == b.data.opposing_side
                && *attackers_are_viewpoint == b.data.attackers_are_viewpoint
                && *representative == b.data.representative
                && *name == b.data.name
        })
}

fn append_viewpoint(reply: &mut WarsReply, tag: Option<&String>, hour: i32) {
    let Some(tag) = tag else { return };
    if let Some(run) = reply.viewpoints.last_mut()
        && &run.tag == tag
        && run.last_hour.checked_add(1) == Some(hour)
    {
        run.last_hour = hour;
    } else {
        reply.viewpoints.push(MembershipRun { tag: tag.clone(), first_hour: hour, last_hour: hour });
    }
}

fn span(spans: &mut Vec<HourSpan>, first: i32, last: i32) {
    if first > last {
        return;
    }
    if let Some(previous) = spans.last_mut()
        && previous.last_hour.checked_add(1) == Some(first)
    {
        previous.last_hour = last;
    } else {
        spans.push(HourSpan { first_hour: first, last_hour: last });
    }
}

fn sides(data: &WarObservation) -> impl Iterator<Item = (&String, WarSide)> {
    let (a, b) = match data.attackers_are_viewpoint {
        Some(true) => (WarSide::Attacker, WarSide::Defender),
        Some(false) => (WarSide::Defender, WarSide::Attacker),
        None => (WarSide::Viewpoint, WarSide::Opponent),
    };
    data.viewpoint_side.iter().map(move |tag| (tag, a)).chain(data.opposing_side.iter().map(move |tag| (tag, b)))
}

fn add_observation(war: &mut WarTimeline, mut data: WarObservation, previous: Option<i32>) {
    let hour = data.first_hour;
    if let Some(old) = war.observations.last_mut() {
        data.first_hour = old.first_hour;
        data.last_hour = old.last_hour;
        if old == &data && Some(old.last_hour) == previous {
            old.last_hour = hour;
            war.last_hour = hour;
            for &index in war.open_participants.values() {
                war.participants[index].last_hour = hour;
            }
            return;
        }
        data.first_hour = hour;
        data.last_hour = hour;
    }
    let existing = std::mem::take(&mut war.open_participants);
    let mut current = HashSet::new();
    let ordered: Vec<_> = sides(&data).map(|(tag, side)| (tag.clone(), side)).filter(|key| current.insert(key.clone())).collect();
    for (key, index) in &existing {
        if !current.contains(key) {
            war.participants[*index].left_by = false;
        }
    }
    for (tag, side) in ordered {
        if let Some(&index) = existing.get(&(tag.clone(), side)) {
            war.participants[index].last_hour = hour;
            war.open_participants.insert((tag, side), index);
        } else {
            war.open_participants.insert((tag.clone(), side), war.participants.len());
            war.participants.push(WarParticipantRun { tag, side, first_hour: hour, last_hour: hour, joined_by: previous.is_none(), left_by: true });
        }
    }
    war.last_hour = hour;
    war.observations.push(data);
}

fn append_relations(war: &mut WarTimeline, hour: i32, relations: &[RelationRun]) {
    for relation in relations {
        let open = (relation.identity.clone(), relation.oriented);
        if let Some(&index) = war.open_relations.get(&open)
            && war.relations[index].last_hour.checked_add(1) == Some(hour)
        {
            war.relations[index].last_hour = hour;
        } else {
            war.open_relations.insert(open, war.relations.len());
            war.relations.push(WarRelationMembership { key: relation.identity.clone(), oriented: relation.oriented, first_hour: hour, last_hour: hour });
        }
    }
}

fn merge_groups(mut groups: Vec<Observed>) -> Observed {
    let mut merged = groups.remove(0);
    if groups.is_empty() {
        return merged;
    }
    let mut viewpoint: HashSet<String> = merged.data.viewpoint_side.iter().cloned().collect();
    let mut opposing: HashSet<String> = merged.data.opposing_side.iter().cloned().collect();
    let mut relations: HashSet<WarRelationKey> = merged.relations.iter().map(|r| r.identity.clone()).collect();
    for group in groups {
        merged.data.viewpoint_side.extend(group.data.viewpoint_side.into_iter().filter(|tag| viewpoint.insert(tag.clone())));
        merged.data.opposing_side.extend(group.data.opposing_side.into_iter().filter(|tag| opposing.insert(tag.clone())));
        merged.relations.extend(group.relations.into_iter().filter(|r| relations.insert(r.identity.clone())));
    }
    merged
}

fn assignment(wars: usize, open: &[OpenWar], current: &[Observed]) -> Vec<usize> {
    let mut inverted: HashMap<&WarRelationKey, Vec<usize>> = HashMap::new();
    for war in open {
        for relation in &war.relations {
            inverted.entry(&relation.identity).or_default().push(war.id);
        }
    }
    let mut used = HashSet::new();
    let mut continued: Vec<Option<usize>> = current
        .iter()
        .map(|observation| {
            let mut counts = HashMap::<usize, usize>::new();
            for relation in &observation.relations {
                for &id in inverted.get(&relation.identity).into_iter().flatten() {
                    if !used.contains(&id) {
                        *counts.entry(id).or_default() += 1;
                    }
                }
            }
            let id = counts.into_iter().max_by(|(a, ac), (b, bc)| ac.cmp(bc).then(b.cmp(a))).map(|(id, _)| id)?;
            used.insert(id);
            Some(id)
        })
        .collect();
    let mut held: HashMap<usize, HashSet<&WarRelationKey>> = HashMap::new();
    for (observation, id) in current.iter().zip(&continued) {
        if let Some(id) = id {
            held.entry(*id).or_default().extend(observation.relations.iter().map(|r| &r.identity));
        }
    }
    let mut new = wars;
    for (index, observation) in current.iter().enumerate() {
        if continued[index].is_some() {
            continue;
        }
        let joined = observation.representative.as_ref().and_then(|named| {
            held.iter()
                .filter(|(_, relations)| relations.contains(named))
                .map(|(&id, relations)| (id, observation.relations.iter().filter(|r| relations.contains(&r.identity)).count()))
                .max_by(|(a, ac), (b, bc)| ac.cmp(bc).then(b.cmp(a)))
                .map(|(id, _)| id)
        });
        let id = joined.unwrap_or_else(|| {
            new += 1;
            new - 1
        });
        continued[index] = Some(id);
        held.entry(id).or_default().extend(observation.relations.iter().map(|r| &r.identity));
    }
    continued.into_iter().map(|id| id.expect("every group continues or starts a war")).collect()
}

fn continue_wars(wars: &mut Vec<WarTimeline>, open: &[OpenWar], current: Vec<Observed>, previous: Option<i32>) -> Vec<OpenWar> {
    let continued = assignment(wars.len(), open, &current);
    let used: HashSet<usize> = continued.iter().copied().collect();
    let mut order: Vec<usize> = Vec::new();
    let mut groups: HashMap<usize, Vec<Observed>> = HashMap::new();
    for (observation, id) in current.into_iter().zip(continued) {
        if id == wars.len() {
            wars.push(WarTimeline {
                id: id.to_string(),
                first_hour: observation.data.first_hour,
                last_hour: observation.data.first_hour,
                started_by: previous.is_none(),
                ended_by: true,
                observations: Vec::new(),
                participants: Vec::new(),
                capitulations: Vec::new(),
                relations: Vec::new(),
                open_relations: HashMap::new(),
                open_participants: HashMap::new(),
            });
        }
        if !groups.contains_key(&id) {
            order.push(id);
        }
        groups.entry(id).or_default().push(observation);
    }
    let mut next = Vec::new();
    for id in order {
        let held = groups.remove(&id).expect("ordered war");
        let count = held.len();
        let observation = merge_groups(held);
        let hour = observation.data.first_hour;
        append_relations(&mut wars[id], hour, &observation.relations);
        add_observation(&mut wars[id], observation.data, previous);
        next.push(OpenWar { id, groups: count, relations: observation.relations });
    }
    if previous.is_some() {
        for OpenWar { id, .. } in open {
            if !used.contains(id) {
                wars[*id].ended_by = false;
                let indices = std::mem::take(&mut wars[*id].open_participants);
                for index in indices.into_values() {
                    wars[*id].participants[index].left_by = false;
                }
            }
        }
    }
    next
}

pub(crate) struct Walk {
    pub reply: WarsReply,
    previous: Option<i32>,
    previous_view: Option<(u32, u32)>,
    open: Vec<OpenWar>,
    seen_caps: HashSet<(String, i32)>,
    dated_caps: HashMap<String, Vec<i32>>,
    identity_since: HashMap<u32, (String, i32)>,
    pub warnings: BTreeSet<String>,
    last_frame: Option<i32>,
    held: Option<(Viewed, Vec<Observed>)>,
    identified: Option<Arc<Countries>>,
    assigned: Vec<usize>,
    settled: bool,
}

impl Walk {
    pub fn new(branch: u64, first: i32) -> Self {
        let reply = WarsReply {
            branch_id: branch.to_string(),
            available: false,
            viewpoints: Vec::new(),
            country_names: Vec::new(),
            wars: Vec::new(),
            unobserved: Vec::new(),
            capitulations_unobserved: Vec::new(),
            warnings: vec![
                "War continuity and participant runs are app observations; the game stores no grouped war identity, start/end or join/leave hours.".into(),
                "Attackers and defenders are oriented by the representative relation's instigator.".into(),
                "Native groups can overlap. A group the game lists beside the war it is named for, as it does for a new declaration, joins that war; a war's casualties count each of its bilateral relations once.".into(),
            ],
        };
        Self {
            reply,
            previous: None,
            previous_view: None,
            open: Vec::new(),
            seen_caps: HashSet::new(),
            dated_caps: HashMap::new(),
            identity_since: HashMap::new(),
            warnings: BTreeSet::new(),
            last_frame: first.checked_sub(1),
            held: None,
            identified: None,
            assigned: Vec::new(),
            settled: false,
        }
    }

    fn settle_open(&mut self) {
        for OpenWar { id, .. } in &self.open {
            let war: &mut WarTimeline = &mut self.reply.wars[*id];
            for &index in war.open_participants.values() {
                war.participants[index].last_hour = war.last_hour;
            }
        }
    }

    pub fn observe(&mut self, seen: Seen<'_>) {
        let hour = seen.hour;
        if let Some(before) = self.last_frame
            && before.checked_add(1) != Some(hour)
        {
            span(&mut self.reply.unobserved, before + 1, hour - 1);
            span(&mut self.reply.capitulations_unobserved, before + 1, hour - 1);
            self.settle_open();
            self.previous = None;
            self.open.clear();
            self.identity_since.clear();
        }
        self.last_frame = Some(hour);
        let (viewed, observed) = match seen.read {
            Ok(read) => read,
            Err(reason) => {
                self.warnings.insert(reason);
                span(&mut self.reply.unobserved, hour, hour);
                span(&mut self.reply.capitulations_unobserved, hour, hour);
                self.settle_open();
                self.previous = None;
                self.open.clear();
                self.identity_since.clear();
                self.held = None;
                return;
            }
        };
        let regrouped = self.held.as_ref().is_some_and(|(_, before)| same_groups(before, &observed));
        let identified = self.identified.as_ref().is_some_and(|countries| Arc::ptr_eq(countries, &viewed.countries));
        let viewpoint = viewed.countries.identities.get(&viewed.view).copied();
        append_viewpoint(&mut self.reply, viewed.countries.tags.get(&viewed.view), hour);
        if self.previous_view != viewpoint {
            self.settle_open();
            self.previous = None;
            self.open.clear();
        }
        self.previous_view = viewpoint;
        if regrouped && self.settled && self.previous.is_some() && self.open.iter().map(|war| war.groups).sum::<usize>() == observed.len() {
            for open in &self.open {
                let war = &mut self.reply.wars[open.id];
                append_relations(war, hour, &open.relations);
                war.last_hour = hour;
                war.observations.last_mut().expect("open war observation").last_hour = hour;
            }
        } else {
            if let Some(last) = self.previous {
                for OpenWar { id, .. } in &self.open {
                    let war = &mut self.reply.wars[*id];
                    for &index in war.open_participants.values() {
                        war.participants[index].last_hour = last;
                    }
                }
            }
            let observations = opening(&observed, hour);
            self.assigned = assignment(self.reply.wars.len(), &self.open, &observations);
            self.open = continue_wars(&mut self.reply.wars, &self.open, observations, self.previous);
            self.settled = assignment(self.reply.wars.len(), &self.open, &observed) == self.assigned;
        }
        if self.previous.is_none() || !identified {
            let countries = &viewed.countries;
            self.identity_since.retain(|raw, _| countries.raw_to_slot.get(raw).is_some_and(|slot| countries.tags.contains_key(slot)));
            for (&raw, &slot) in &countries.raw_to_slot {
                let Some(tag) = countries.tags.get(&slot) else { continue };
                let entry = self.identity_since.entry(raw).or_insert_with(|| (tag.clone(), hour));
                if &entry.0 != tag || self.previous.is_none() {
                    *entry = (tag.clone(), hour);
                }
            }
            self.identified = Some(Arc::clone(countries));
        }
        match seen.capitulations {
            Ok(rows) => {
                for &(country, capitulated) in rows {
                    let Some((tag, since)) = self.identity_since.get(&country) else { continue };
                    if capitulated < *since || capitulated > hour {
                        self.warnings.insert(CAPITULATIONS_UNOBSERVED.into());
                        continue;
                    }
                    if !self.seen_caps.insert((tag.clone(), capitulated)) {
                        continue;
                    }
                    self.dated_caps.entry(tag.clone()).or_default().push(capitulated);
                }
            }
            Err(()) => span(&mut self.reply.capitulations_unobserved, hour, hour),
        }
        self.previous = Some(hour);
        self.held = Some((viewed, observed));
    }

    pub fn extend(&mut self, last: i32) {
        let Some(first) = self.last_frame else { return };
        if last <= first {
            return;
        }
        if self.previous == Some(first) {
            if let Some((_, observed)) = &self.held {
                let mut at = first;
                while at < last && !self.settled {
                    at += 1;
                    for OpenWar { id, .. } in &self.open {
                        let war = &mut self.reply.wars[*id];
                        for &index in war.open_participants.values() {
                            war.participants[index].last_hour = war.last_hour;
                        }
                    }
                    let observations = opening(observed, at);
                    self.assigned = assignment(self.reply.wars.len(), &self.open, observed);
                    self.open = continue_wars(&mut self.reply.wars, &self.open, observations, self.previous);
                    self.settled = assignment(self.reply.wars.len(), &self.open, observed) == self.assigned;
                    self.previous = Some(at);
                }
            }
            if let Some(viewpoint) = self.reply.viewpoints.last_mut() {
                viewpoint.last_hour = last;
            }
            for OpenWar { id, relations, .. } in &self.open {
                let war = &mut self.reply.wars[*id];
                war.last_hour = last;
                if let Some(observation) = war.observations.last_mut() {
                    observation.last_hour = last;
                }
                for relation in relations {
                    if let Some(&index) = war.open_relations.get(&(relation.identity.clone(), relation.oriented)) {
                        war.relations[index].last_hour = last;
                    }
                }
            }
            if self.reply.capitulations_unobserved.last().is_some_and(|span| span.last_hour == first) {
                span(&mut self.reply.capitulations_unobserved, first + 1, last);
            }
            self.previous = Some(last);
        } else {
            span(&mut self.reply.unobserved, first + 1, last);
            span(&mut self.reply.capitulations_unobserved, first + 1, last);
        }
        self.last_frame = Some(last);
    }

    pub fn finish(mut self, last: i32) -> WarsReply {
        if let Some(before) = self.last_frame {
            span(&mut self.reply.unobserved, before.saturating_add(1), last);
            span(&mut self.reply.capitulations_unobserved, before.saturating_add(1), last);
        }
        self.reply.warnings.extend(self.warnings);
        for hours in self.dated_caps.values_mut() {
            hours.sort_unstable();
        }
        for war in &mut self.reply.wars {
            for &index in war.open_participants.values() {
                war.participants[index].last_hour = war.last_hour;
            }
            let mut attached = HashSet::new();
            for participant in &war.participants {
                let Some(hours) = self.dated_caps.get(&participant.tag) else { continue };
                let first = hours.partition_point(|h| *h < participant.first_hour);
                for &hour in hours[first..].iter().take_while(|h| **h <= participant.last_hour) {
                    if attached.insert((participant.tag.clone(), hour)) {
                        war.capitulations.push(WarCapitulation { tag: participant.tag.clone(), hour });
                    }
                }
            }
            war.open_participants = HashMap::new();
            war.open_relations = HashMap::new();
            war.participants.sort_by(|a, b| a.first_hour.cmp(&b.first_hour).then(a.tag.cmp(&b.tag)));
            war.capitulations.sort_by_key(|c| c.hour);
        }
        self.reply
    }
}
