
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FactionHistory {
    pub first_hour: i32,
    pub members: Vec<Run>,
    pub leaders: Vec<Run>,
    pub incomplete: Vec<(i32, i32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarRun {
    pub first: String,
    pub second: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub first_hour: i32,
    pub last_hour: i32,
    pub slot: u32,
    pub country_id: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Groups {
    pub factions: BTreeMap<u64, FactionHistory>,
    pub wars: Vec<WarRun>,
    pub unobserved_factions: Vec<(i32, i32)>,
    pub unobserved_wars: Vec<(i32, i32)>,
    pub identities: BTreeMap<String, Vec<Identity>>,
}

impl Groups {
    pub fn identity_run(&self, tag: &str, hour: i32) -> Option<&Identity> {
        let runs = self.identities.get(tag)?;
        let at = runs.partition_point(|run| run.last_hour < hour);
        runs.get(at).filter(|run| run.first_hour <= hour)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideAnchors {
    pub countries: BTreeMap<String, String>,
    pub factions: BTreeMap<u64, u64>,
    pub faction_hours: BTreeMap<u64, i32>,
    pub country_reads: BTreeMap<String, (String, i32)>,
    pub faction_reads: BTreeMap<u64, (String, i32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignedSide {
    pub key: String,
    pub faction: Option<u64>,
    pub colour_faction: Option<u64>,
    pub factions: Vec<u64>,
    pub leader: Option<String>,
    pub members: Vec<String>,
    pub co_belligerents: usize,
}

impl AssignedSide {
    pub fn lone(&self) -> bool {
        self.factions.is_empty() && self.members.len() == 1
    }
}

struct Faction {
    id: u64,
    leader: Option<String>,
    members: BTreeSet<String>,
}

#[cfg(test)]
thread_local! {
    pub(super) static VISITED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Stab<T> {
    runs: Vec<(i32, i32, T)>,
    ends: Vec<i32>,
    size: usize,
}

impl<T> Stab<T> {
    fn new(mut runs: Vec<(i32, i32, T)>) -> Self {
        runs.sort_by_key(|(first, _, _)| *first);
        let size = runs.len().next_power_of_two();
        let mut ends = vec![i32::MIN; 2 * size];
        for (at, (_, last, _)) in runs.iter().enumerate() {
            ends[size + at] = *last;
        }
        for node in (1..size).rev() {
            ends[node] = ends[2 * node].max(ends[2 * node + 1]);
        }
        Self { runs, ends, size }
    }

    fn each<'a>(&'a self, hour: i32, visit: &mut impl FnMut(&'a T)) {
        let end = self.runs.partition_point(|(first, _, _)| *first <= hour);
        self.walk(1, 0, self.size, end, hour, visit);
    }

    fn walk<'a>(&'a self, node: usize, low: usize, high: usize, end: usize, hour: i32, visit: &mut impl FnMut(&'a T)) {
        if low >= end || self.ends[node] < hour {
            return;
        }
        if high - low == 1 {
            #[cfg(test)]
            VISITED.with(|count| count.set(count.get() + 1));
            visit(&self.runs[low].2);
            return;
        }
        let middle = (low + high) / 2;
        self.walk(2 * node, low, middle, end, hour, visit);
        self.walk(2 * node + 1, middle, high, end, hour, visit);
    }
}

enum Active {
    Unknown,
    Member(u64, String),
    Leader(u64, usize, String),
    War(String, String),
}

pub struct GroupIndex {
    runs: Stab<Active>,
}

impl GroupIndex {
    pub fn new(groups: &Groups) -> Self {
        let mut runs = Vec::new();
        for &(first, last) in groups.unobserved_factions.iter().chain(&groups.unobserved_wars) {
            runs.push((first, last, Active::Unknown));
        }
        for (&id, faction) in &groups.factions {
            for &(first, last) in &faction.incomplete {
                runs.push((first, last, Active::Unknown));
            }
            for run in &faction.members {
                runs.push((run.first_hour, run.last_hour, Active::Member(id, run.tag.clone())));
            }
            for (order, run) in faction.leaders.iter().enumerate() {
                runs.push((run.first_hour, run.last_hour, Active::Leader(id, order, run.tag.clone())));
            }
        }
        for run in &groups.wars {
            runs.push((run.first_hour, run.last_hour, Active::War(run.first.clone(), run.second.clone())));
        }
        Self { runs: Stab::new(runs) }
    }
}

pub fn indexed_roster(groups: &Groups, index: &GroupIndex, hour: i32, countries: &BTreeSet<String>) -> Option<Vec<AssignedSide>> {
    let mut unknown = false;
    let mut members = BTreeMap::<u64, BTreeSet<String>>::new();
    let mut leaders = BTreeMap::<u64, (usize, &str)>::new();
    let mut wars = Vec::new();
    index.runs.each(hour, &mut |run| match run {
        Active::Unknown => unknown = true,
        Active::Member(id, tag) => {
            if countries.contains(tag) {
                members.entry(*id).or_default().insert(tag.clone());
            }
        }
        Active::Leader(id, order, tag) => {
            let held = leaders.entry(*id).or_insert((*order, tag.as_str()));
            if *order < held.0 {
                *held = (*order, tag.as_str());
            }
        }
        Active::War(first, second) => wars.push((first.clone(), second.clone())),
    });
    if unknown {
        return None;
    }
    let sides = roster(members, &leaders, wars, countries)?;
    sides.iter().flat_map(|side| &side.members).all(|tag| groups.identity_run(tag, hour).is_some()).then_some(sides)
}

pub fn roster(members: BTreeMap<u64, BTreeSet<String>>, leaders: &BTreeMap<u64, (usize, &str)>, wars: Vec<(String, String)>, countries: &BTreeSet<String>) -> Option<Vec<AssignedSide>> {
    let factions = members
        .into_iter()
        .map(|(id, members)| Faction {
            id,
            leader: leaders.get(&id).filter(|(_, tag)| members.contains(*tag)).map(|(_, tag)| (*tag).to_owned()),
            members,
        })
        .collect();
    assign(factions, countries, wars)
}

pub fn keyed(groups: &Groups, hour: i32, mut sides: Vec<AssignedSide>, anchors: &SideAnchors) -> Option<Vec<AssignedSide>> {
    let (present, identities) = native_side_keys(groups, hour, &sides)?;
    for (side, key) in sides.iter_mut().zip(resolve_keys(&identities, &present, anchors)) {
        if let Some(id) = key.strip_prefix("f:").and_then(|id| id.parse().ok()) {
            side.faction = Some(id);
        }
        side.key = key;
    }
    Some(sides)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideKey {
    pub present: String,
    factions: Vec<u64>,
    members: Vec<String>,
}

fn country_key(run: &Identity) -> String {
    format!("{}:{}", run.slot, run.country_id)
}

pub fn native_side_keys(groups: &Groups, hour: i32, sides: &[AssignedSide]) -> Option<(BTreeSet<String>, Vec<SideKey>)> {
    let mut present = BTreeSet::new();
    let keys = sides
        .iter()
        .map(|side| {
            let members = side
                .members
                .iter()
                .map(|tag| {
                    let identity = country_key(groups.identity_run(tag, hour)?);
                    present.insert(identity.clone());
                    Some(identity)
                })
                .collect::<Option<Vec<_>>>()?;
            let present = if let Some(id) = side.factions.iter().min() {
                format!("f:{id}")
            } else if side.members.len() > 1 {
                format!("w:{}", members[0])
            } else {
                format!("c:{}", side.members[0])
            };
            Some(SideKey { present, factions: side.factions.clone(), members })
        })
        .collect::<Option<Vec<_>>>()?;
    Some((present, keys))
}

pub fn resolve_keys(sides: &[SideKey], countries: &BTreeSet<String>, anchors: &SideAnchors) -> Vec<String> {
    let mut historical = BTreeMap::new();
    let mut present = BTreeMap::new();
    for (tag, anchor) in &anchors.countries {
        *historical.entry(anchor.clone()).or_insert(0_usize) += 1;
        if countries.contains(tag) {
            *present.entry(anchor.clone()).or_insert(0_usize) += 1;
        }
    }
    let sole_remnant = |identity: &str| {
        anchors
            .countries
            .get(identity)
            .is_some_and(|anchor| historical.get(anchor).copied().unwrap_or(0) > 1 && present.get(anchor) == Some(&1))
    };
    let mut present_factions = BTreeMap::new();
    for id in sides.iter().flat_map(|side| &side.factions) {
        if let Some(anchor) = anchors.factions.get(id) {
            *present_factions.entry(anchor).or_insert(0_usize) += 1;
        }
    }
    let mut keys: Vec<_> = sides
        .iter()
        .map(|side| {
            if let Some(id) = side.factions.iter().min()
                && let Some(anchor) = anchors.factions.get(id).filter(|anchor| side.factions.len() > 1 || present_factions.get(anchor) == Some(&1))
            {
                return format!("f:{anchor}");
            }
            let remnant = side.factions.is_empty() && side.members.len() == 1 && sole_remnant(&side.members[0]);
            if (side.present.starts_with("w:") || remnant)
                && let Some(anchor) = side.members.iter().filter_map(|tag| anchors.countries.get(tag)).min()
            {
                return format!("w:{anchor}");
            }
            side.present.clone()
        })
        .collect();
    let mut collisions = BTreeMap::<String, Vec<usize>>::new();
    for (index, key) in keys.iter().enumerate() {
        collisions.entry(key.clone()).or_default().push(index);
    }
    for (key, indices) in collisions.into_iter().filter(|(_, indices)| indices.len() > 1) {
        let contains = |side: &SideKey| {
            if let Some(id) = key.strip_prefix("f:").and_then(|id| id.parse::<u64>().ok()) {
                side.factions.contains(&id)
            } else {
                side.members.iter().any(|tag| key.strip_prefix("w:") == Some(tag.as_str()))
            }
        };
        let faction_rank = |side: &SideKey| side.factions.iter().map(|id| (anchors.faction_hours.get(id).copied().unwrap_or(i32::MAX), *id)).min();
        let country_rank = |side: &SideKey| {
            side.members.iter().map(|tag| (anchors.country_reads.get(tag).map_or(i32::MAX, |(_, hour)| *hour), tag.clone())).min()
        };
        let winner = *indices
            .iter()
            .min_by(|&&first, &&second| {
                contains(&sides[second])
                    .cmp(&contains(&sides[first]))
                    .then_with(|| faction_rank(&sides[first]).cmp(&faction_rank(&sides[second])))
                    .then_with(|| country_rank(&sides[first]).cmp(&country_rank(&sides[second])))
                    .then_with(|| sides[first].present.cmp(&sides[second].present))
            })
            .expect("nonempty collision");
        for index in indices {
            if index != winner {
                keys[index].clone_from(&sides[index].present);
            }
        }
    }
    keys
}

fn assign(factions: Vec<Faction>, countries: &BTreeSet<String>, wars: impl IntoIterator<Item = (String, String)>) -> Option<Vec<AssignedSide>> {
    let mut member_of = BTreeMap::new();
    let mut original_membership = BTreeMap::new();
    let mut sides = BTreeMap::new();
    for faction in factions {
        for tag in &faction.members {
            original_membership.insert(tag.clone(), faction.id);
            if member_of.insert(tag.clone(), faction.id).is_some() {
                return None;
            }
        }
        sides.insert(
            faction.id,
            AssignedSide {
                key: format!("f:{}", faction.id),
                faction: Some(faction.id),
                colour_faction: None,
                factions: vec![faction.id],
                leader: faction.leader,
                members: faction.members.into_iter().collect(),
                co_belligerents: 0,
            },
        );
    }
    let mut enemies = BTreeMap::<String, BTreeSet<String>>::new();
    for (first, second) in wars {
        if first == second || !countries.contains(&first) || !countries.contains(&second) {
            continue;
        }
        enemies.entry(first.clone()).or_default().insert(second.clone());
        enemies.entry(second).or_default().insert(first);
    }
    let ids: Vec<_> = sides.keys().copied().collect();
    let tags: Vec<_> = ids.iter().map(ToString::to_string).collect();
    let indexes: BTreeMap<_, _> = tags.iter().enumerate().map(|(index, tag)| (tag.clone(), index)).collect();
    let mut faction_enemies = BTreeMap::<String, BTreeSet<String>>::new();
    let mut enemy_sides = BTreeMap::<String, BTreeSet<u64>>::new();
    for (member, &id) in &member_of {
        for enemy in enemies.get(member).into_iter().flatten() {
            enemy_sides.entry(enemy.clone()).or_default().insert(id);
            if let Some(other) = member_of.get(enemy) {
                faction_enemies.entry(id.to_string()).or_default().insert(other.to_string());
            }
        }
    }
    let mut coalitions = Coalitions::new(&tags, &indexes, &faction_enemies);
    for allies in enemy_sides.values() {
        coalitions.join_all(allies.iter().map(|id| indexes[&id.to_string()]));
    }
    let mut anchors = BTreeMap::new();
    for (index, &id) in ids.iter().enumerate() {
        anchors.entry(coalitions.root(index)).or_insert(id);
    }
    let parents: BTreeMap<_, _> = ids.iter().enumerate().map(|(index, &id)| (id, anchors[&coalitions.root(index)])).collect();
    let mut merged = BTreeMap::<u64, AssignedSide>::new();
    for (id, side) in sides {
        let anchor = parents[&id];
        match merged.get_mut(&anchor) {
            Some(old) => {
                old.members.extend(side.members);
                old.factions.extend(side.factions);
            }
            None => {
                merged.insert(anchor, side);
            }
        }
    }
    let mut sides = merged;
    for id in member_of.values_mut() {
        *id = parents[id];
    }
    let mut enemy_factions = BTreeMap::<String, BTreeSet<u64>>::new();
    for (member, &id) in &member_of {
        for enemy in enemies.get(member).into_iter().flatten() {
            enemy_factions.entry(enemy.clone()).or_default().insert(id);
        }
    }
    let mut enemy_leaders = HashMap::<&str, Vec<u64>>::new();
    for (&id, side) in &sides {
        for enemy in side.leader.as_ref().into_iter().flat_map(|leader| enemies.get(leader).into_iter().flatten()) {
            enemy_leaders.entry(enemy).or_default().push(id);
        }
    }
    let mut unassigned = BTreeSet::new();
    let mut joined = BTreeMap::<&str, u64>::new();
    for tag in countries.iter().filter(|tag| !member_of.contains_key(*tag)) {
        let mut candidates = BTreeMap::<u64, usize>::new();
        let mut blocked = BTreeSet::new();
        let mut direct = HashMap::<u64, usize>::new();
        if let Some(opponents) = enemies.get(tag) {
            for enemy in opponents {
                for &id in enemy_leaders.get(enemy.as_str()).into_iter().flatten() {
                    *direct.entry(id).or_default() += 1;
                }
                if let Some(&faction) = member_of.get(enemy).or_else(|| joined.get(enemy.as_str())) {
                    blocked.insert(faction);
                }
                if let Some(counts) = enemy_factions.get(enemy) {
                    for &faction in counts {
                        *candidates.entry(faction).or_default() += 1;
                    }
                }
            }
        }
        let selected = candidates
            .into_iter()
            .filter(|(id, _)| !blocked.contains(id))
            .max_by(|(first_id, first_count), (second_id, second_count)| {
                first_count
                    .cmp(second_count)
                    .then_with(|| direct.get(first_id).copied().unwrap_or(0).cmp(&direct.get(second_id).copied().unwrap_or(0)))
                    .then_with(|| second_id.cmp(first_id))
            })
            .map(|(id, _)| id);
        if let Some(id) = selected {
            let side = sides.get_mut(&id).expect("recorded faction candidate");
            side.members.push(tag.clone());
            side.co_belligerents += 1;
            joined.insert(tag.as_str(), id);
        } else {
            unassigned.insert(tag.clone());
        }
    }
    let tags: Vec<_> = unassigned.into_iter().collect();
    let indexes: BTreeMap<_, _> = tags.iter().enumerate().map(|(i, tag)| (tag.clone(), i)).collect();
    let mut coalitions = Coalitions::new(&tags, &indexes, &enemies);
    for allies in enemies.values() {
        let allies: Vec<_> = allies.iter().filter_map(|tag| indexes.get(tag).copied()).collect();
        coalitions.join_all(allies);
    }
    let mut independent = BTreeMap::<usize, Vec<String>>::new();
    for (index, tag) in tags.into_iter().enumerate() {
        independent.entry(coalitions.root(index)).or_default().push(tag);
    }
    let mut result: Vec<_> = sides.into_values().collect();
    for members in independent.into_values() {
        let first = &members[0];
        let coalition = members.len() > 1;
        result.push(AssignedSide {
            key: format!("{}:{first}", if coalition { "w" } else { "c" }),
            faction: None,
            colour_faction: None,
            factions: vec![],
            leader: (!coalition).then(|| first.clone()),
            co_belligerents: if coalition { members.len() - 1 } else { 0 },
            members,
        });
    }
    for side in &mut result {
        let mut participating = 0;
        let mut counts = BTreeMap::<u64, usize>::new();
        for tag in &side.members {
            if !enemies.contains_key(tag) {
                continue;
            }
            participating += 1;
            if let Some(&id) = original_membership.get(tag) {
                *counts.entry(id).or_default() += 1;
            }
        }
        side.colour_faction = counts.into_iter().find_map(|(id, count)| (count > participating / 2).then_some(id));
    }
    Some(result)
}

pub fn historical_anchors(groups: &Groups) -> SideAnchors {
    let mut membership = BTreeMap::<&str, Vec<(u64, i32, i32)>>::new();
    for (&id, faction) in &groups.factions {
        for run in &faction.members {
            membership.entry(&run.tag).or_default().push((id, run.first_hour, run.last_hour));
        }
    }
    let membership: BTreeMap<_, _> = membership.into_iter().map(|(tag, runs)| (tag, Memberships::new(runs))).collect();
    let mut exposures = BTreeMap::<&str, Vec<(String, i32, i32)>>::new();
    let mut enemies = BTreeMap::<String, BTreeSet<String>>::new();
    let mut first_seen = BTreeMap::<u64, i32>::new();
    for war in &groups.wars {
        for (tag, enemy) in [(&war.first, &war.second), (&war.second, &war.first)] {
            for &(id, from, to) in membership.get(tag.as_str()).into_iter().flat_map(|runs| runs.overlapping(war.first_hour, war.last_hour)) {
                let first = from.max(war.first_hour);
                let last = to.min(war.last_hour);
                if first > last {
                    continue;
                }
                exposures.entry(enemy).or_default().push((id.to_string(), first, last));
                first_seen.entry(id).and_modify(|old| *old = (*old).min(first)).or_insert(first);
                for &(other, _, _) in membership.get(enemy.as_str()).into_iter().flat_map(|runs| runs.overlapping(first, last)) {
                    enemies.entry(id.to_string()).or_default().insert(other.to_string());
                }
            }
        }
    }
    let ids: Vec<_> = groups.factions.keys().copied().collect();
    let tags: Vec<_> = ids.iter().map(ToString::to_string).collect();
    let indexes: BTreeMap<_, _> = tags.iter().enumerate().map(|(index, tag)| (tag.clone(), index)).collect();
    let mut coalitions = Coalitions::new(&tags, &indexes, &enemies);
    for runs in exposures.values_mut() {
        runs.sort_by_key(|(_, first, _)| *first);
        coalitions.join_runs(runs.iter().map(|(tag, first, last)| (indexes[tag], *first, *last)));
    }
    let mut roots = BTreeMap::<usize, u64>::new();
    let rank = |id: u64| (first_seen.get(&id).copied().unwrap_or(groups.factions[&id].first_hour), id);
    for (index, &id) in ids.iter().enumerate() {
        roots
            .entry(coalitions.root(index))
            .and_modify(|old| {
                if rank(id) < rank(*old) {
                    *old = id;
                }
            })
            .or_insert(id);
    }
    let (countries, country_reads) = historical_country_anchors(groups);
    let faction_reads = groups
        .factions
        .iter()
        .filter_map(|(&id, faction)| {
            let hour = faction.first_hour;
            let leader = faction.leaders.iter().find(|run| run.first_hour <= hour && hour <= run.last_hour)?;
            groups.identity_run(&leader.tag, hour)?;
            Some((id, (leader.tag.clone(), hour)))
        })
        .collect();
    SideAnchors {
        countries,
        country_reads,
        faction_reads,
        faction_hours: first_seen,
        factions: ids.iter().enumerate().map(|(index, id)| (*id, roots[&coalitions.root(index)])).collect(),
    }
}

fn historical_country_anchors(groups: &Groups) -> (BTreeMap<String, String>, BTreeMap<String, (String, i32)>) {
    let mut by_enemy = BTreeMap::<String, Vec<(String, i32, i32)>>::new();
    let mut enemies = BTreeMap::<String, BTreeSet<String>>::new();
    let mut first_seen = BTreeMap::<String, (String, i32)>::new();
    for war in &groups.wars {
        let (Some(first), Some(second)) = (groups.identities.get(&war.first), groups.identities.get(&war.second)) else {
            continue;
        };
        let mut first = first.iter().peekable();
        let mut second = second.iter().peekable();
        while let (Some(a), Some(b)) = (first.peek(), second.peek()) {
            let start = war.first_hour.max(a.first_hour).max(b.first_hour);
            let end = war.last_hour.min(a.last_hour).min(b.last_hour);
            if start <= end {
                let a_key = country_key(a);
                let b_key = country_key(b);
                for (key, enemy, tag) in [(&a_key, &b_key, &war.first), (&b_key, &a_key, &war.second)] {
                    by_enemy.entry(enemy.clone()).or_default().push((key.clone(), start, end));
                    enemies.entry(key.clone()).or_default().insert(enemy.clone());
                    let read = first_seen.entry(key.clone()).or_insert_with(|| (tag.clone(), start));
                    if (start, tag) < (read.1, &read.0) {
                        *read = (tag.clone(), start);
                    }
                }
            }
            let (a_end, b_end) = (a.last_hour, b.last_hour);
            if a_end <= b_end {
                first.next();
            }
            if b_end <= a_end {
                second.next();
            }
        }
    }
    let tags: Vec<_> = first_seen.keys().cloned().collect();
    let indexes: BTreeMap<_, _> = tags.iter().enumerate().map(|(index, tag)| (tag.clone(), index)).collect();
    let mut coalitions = Coalitions::new(&tags, &indexes, &enemies);
    for runs in by_enemy.values_mut() {
        runs.sort_by_key(|(_, first, _)| *first);
        coalitions.join_runs(runs.iter().map(|(tag, first, last)| (indexes[tag], *first, *last)));
    }
    let mut anchors = BTreeMap::<usize, String>::new();
    for (index, tag) in tags.iter().enumerate() {
        let root = coalitions.root(index);
        match anchors.get_mut(&root) {
            Some(old) if (first_seen[tag].1, &first_seen[tag].0, tag) < (first_seen[old].1, &first_seen[old].0, &*old) => *old = tag.clone(),
            None => {
                anchors.insert(root, tag.clone());
            }
            _ => {}
        }
    }
    (tags.iter().enumerate().map(|(index, tag)| (tag.clone(), anchors[&coalitions.root(index)].clone())).collect(), first_seen)
}

struct Memberships {
    runs: Vec<(usize, (u64, i32, i32))>,
    ends: Vec<i32>,
}

impl Memberships {
    fn new(runs: Vec<(u64, i32, i32)>) -> Self {
        let mut runs: Vec<_> = runs.into_iter().enumerate().collect();
        runs.sort_by_key(|(_, (_, first, _))| *first);
        let mut end = i32::MIN;
        let ends = runs
            .iter()
            .map(|(_, (_, _, last))| {
                end = end.max(*last);
                end
            })
            .collect();
        Self { runs, ends }
    }

    fn overlapping(&self, first: i32, last: i32) -> impl Iterator<Item = &(u64, i32, i32)> {
        let from = self.ends.partition_point(|end| *end < first);
        let mut runs: Vec<_> = self.runs[from..].iter().take_while(|(_, (_, start, _))| *start <= last).filter(|(_, (_, _, end))| *end >= first).collect();
        runs.sort_unstable_by_key(|(order, _)| *order);
        runs.into_iter().map(|(_, run)| run)
    }
}

struct Coalitions {
    parents: Vec<usize>,
    sizes: Vec<usize>,
    opponents: Vec<HashSet<usize>>,
}

impl Coalitions {
    fn new(tags: &[String], indexes: &BTreeMap<String, usize>, enemies: &BTreeMap<String, BTreeSet<String>>) -> Self {
        let opponents = tags
            .iter()
            .map(|tag| {
                enemies
                    .get(tag)
                    .into_iter()
                    .flatten()
                    .filter_map(|opponent| indexes.get(opponent).copied())
                    .filter(|&opponent| tags[opponent] != *tag)
                    .collect()
            })
            .collect();
        Self { parents: (0..tags.len()).collect(), sizes: vec![1; tags.len()], opponents }
    }

    fn root(&mut self, index: usize) -> usize {
        let parent = self.parents[index];
        if parent != index {
            self.parents[index] = self.root(parent);
        }
        self.parents[index]
    }

    fn join(&mut self, first: usize, second: usize) -> bool {
        let mut first = self.root(first);
        let mut second = self.root(second);
        if first == second {
            return true;
        }
        if self.sizes[first] < self.sizes[second] {
            std::mem::swap(&mut first, &mut second);
        }
        if self.opponents[first].contains(&second) {
            return false;
        }
        self.parents[second] = first;
        self.sizes[first] += self.sizes[second];
        for opponent in std::mem::take(&mut self.opponents[second]) {
            self.opponents[opponent].remove(&second);
            self.opponents[opponent].insert(first);
            self.opponents[first].insert(opponent);
        }
        true
    }

    fn join_all(&mut self, allies: impl IntoIterator<Item = usize>) {
        self.join_runs(allies.into_iter().map(|index| (index, 0, i32::MAX)));
    }

    fn join_runs(&mut self, runs: impl IntoIterator<Item = (usize, i32, i32)>) {
        let mut representatives = BTreeMap::<usize, (usize, i32)>::new();
        let mut by_root = HashMap::<usize, usize>::new();
        let mut available = BTreeSet::<usize>::new();
        let mut expiry = BTreeSet::<(i32, usize)>::new();
        for (order, (index, first, last)) in runs.into_iter().enumerate() {
            while let Some(&(end, slot)) = expiry.first().filter(|(end, _)| *end < first) {
                expiry.remove(&(end, slot));
                let (root, _) = representatives.remove(&slot).expect("expired representative");
                by_root.remove(&root);
                available.remove(&slot);
            }
            let root = self.root(index);
            let selected = by_root.get(&root).copied().or_else(|| {
                let first = *available.first()?;
                if !self.opponents[root].contains(&representatives[&first].0) {
                    return Some(first);
                }
                let blocked: Vec<_> = self.opponents[root].iter().filter_map(|opponent| by_root.get(opponent).copied()).collect();
                for slot in &blocked {
                    available.remove(slot);
                }
                let selected = available.first().copied();
                available.extend(blocked);
                selected
            });
            if let Some(slot) = selected {
                let (other, end) = representatives[&slot];
                expiry.remove(&(end, slot));
                by_root.remove(&other);
                assert!(self.join(root, other));
                let root = self.root(root);
                let end = end.max(last);
                representatives.insert(slot, (root, end));
                by_root.insert(root, slot);
                expiry.insert((end, slot));
            } else {
                representatives.insert(order, (root, last));
                by_root.insert(root, order);
                available.insert(order);
                expiry.insert((last, order));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn countries(tags: &[&str]) -> BTreeSet<String> {
        tags.iter().map(|tag| (*tag).to_owned()).collect()
    }
    fn faction(id: u64, members: &[&str]) -> Faction {
        Faction { id, leader: members.first().map(|tag| (*tag).to_owned()), members: countries(members) }
    }
    fn wars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }
    fn side<'a>(sides: &'a [AssignedSide], key: &str) -> &'a AssignedSide {
        sides.iter().find(|side| side.key == key).unwrap()
    }

    #[test]
    fn blocked_representatives_keep_hostility_indexed_after_merges() {
        let tags: Vec<_> = (0..16).map(|index| format!("Country {index:04}")).collect();
        let indexes = tags.iter().enumerate().map(|(index, tag)| (tag.clone(), index)).collect();
        let enemies = tags.iter().enumerate().map(|(index, tag)| (tag.clone(), [tags[index ^ 1].clone()].into())).collect();
        let mut coalitions = Coalitions::new(&tags, &indexes, &enemies);
        coalitions.join_all(0..tags.len());
        let first = coalitions.root(0);
        let second = coalitions.root(1);
        for index in 0..tags.len() {
            assert_eq!(coalitions.root(index), if index % 2 == 0 { first } else { second });
        }
        assert!(!coalitions.join(first, second));
        assert_eq!(coalitions.opponents[first], [second].into());
        assert_eq!(coalitions.opponents[second], [first].into());
    }

    #[test]
    fn historical_intersections_keep_overlap_order_and_expire_representatives() {
        let members = Memberships::new(vec![(9, 10, 30), (1, 0, 20), (2, 40, 50)]);
        assert_eq!(members.overlapping(15, 15).copied().collect::<Vec<_>>(), [(9, 10, 30), (1, 0, 20)]);
        assert_eq!(members.overlapping(31, 39).count(), 0);
        let tags: Vec<_> = ["A", "B", "C"].map(String::from).into();
        let indexes = tags.iter().enumerate().map(|(index, tag)| (tag.clone(), index)).collect();
        let mut coalitions = Coalitions::new(&tags, &indexes, &BTreeMap::new());
        coalitions.join_runs([(0, 0, 10), (1, 1, 5), (2, 11, 20)]);
        assert_eq!(coalitions.root(0), coalitions.root(1));
        assert_ne!(coalitions.root(0), coalitions.root(2));
    }

    fn key(present: &str, factions: Vec<u64>, members: &[&str]) -> SideKey {
        SideKey { present: present.into(), factions, members: members.iter().map(|tag| (*tag).to_owned()).collect() }
    }

    #[test]
    fn absent_historical_anchor_uses_earliest_present_participant() {
        let anchors = SideAnchors {
            factions: [(1, 1), (2, 1), (3, 1), (4, 1), (5, 1)].into(),
            faction_hours: [(1, 0), (2, 20), (3, 10), (4, 30), (5, 40)].into(),
            countries: [("A", "A"), ("B", "A"), ("C", "A"), ("D", "A"), ("E", "A")].map(|(tag, anchor)| (tag.into(), anchor.into())).into(),
            faction_reads: BTreeMap::new(),
            country_reads: [("A", 0), ("B", 20), ("C", 10), ("D", 30), ("E", 40)].map(|(tag, hour)| (tag.into(), (tag.into(), hour))).into(),
        };
        let factions = [key("f:2", vec![2, 4], &["B", "D"]), key("f:3", vec![3, 5], &["C", "E"])];
        assert_eq!(resolve_keys(&factions, &countries(&["B", "C", "D", "E"]), &anchors), ["f:2", "f:1"]);
        let independent = [key("w:B", vec![], &["B", "D"]), key("w:C", vec![], &["C", "E"])];
        assert_eq!(resolve_keys(&independent, &countries(&["B", "C", "D", "E"]), &anchors), ["w:B", "w:A"]);
    }

    #[test]
    fn co_belligerents_join_once_and_ties_use_numeric_faction_ids() {
        let tags = countries(&["A", "B", "C", "X", "Y"]);
        let sides = assign(vec![faction(10, &["A"]), faction(9, &["B"])], &tags, wars(&[("A", "X"), ("B", "X"), ("C", "X")])).unwrap();
        assert_eq!(side(&sides, "f:9").members, ["B", "A", "C"]);
        assert_eq!(side(&sides, "f:9").co_belligerents, 1);
        assert_eq!(sides.iter().filter(|side| side.faction.is_some()).count(), 1);
        assert_eq!(sides.iter().flat_map(|s| &s.members).count(), tags.len());
        let sides = assign(
            vec![faction(10, &["A"]), faction(9, &["B"])],
            &tags,
            wars(&[("A", "X"), ("A", "Y"), ("B", "X"), ("C", "X"), ("C", "Y")]),
        )
        .unwrap();
        assert_eq!(side(&sides, "f:9").members, ["B", "A", "C"]);
    }

    #[test]
    fn air_superiority_colour_requires_a_strict_country_majority() {
        let tags = countries(&["A", "B", "C", "D", "X"]);
        for (members, expected) in [(&["A"][..], None), (&["A", "B"][..], None), (&["A", "B", "C"][..], Some(10))] {
            let sides = assign(vec![faction(10, members)], &tags, wars(&[("A", "X"), ("B", "X"), ("C", "X"), ("D", "X")])).unwrap();
            let side = side(&sides, "f:10");
            assert_eq!(side.members.len(), 4);
            assert_eq!(side.colour_faction, expected);
        }
        let sides = assign(vec![faction(10, &["A"]), faction(20, &["B", "C", "D"])], &tags, wars(&[("A", "X"), ("B", "X"), ("C", "X"), ("D", "X")])).unwrap();
        let side = side(&sides, "f:10");
        assert_eq!(side.colour_faction, Some(20));
        assert_eq!(side.leader.as_deref(), Some("A"));
    }

    #[test]
    fn peaceful_faction_members_do_not_vote_on_a_war_side_colour() {
        let tags = countries(&["A", "B", "C", "D", "P", "Q", "R", "X"]);
        let sides = assign(vec![faction(10, &["A", "P", "Q", "R"])], &tags, wars(&[("A", "X"), ("B", "X"), ("C", "X"), ("D", "X")])).unwrap();
        assert_eq!(side(&sides, "f:10").members.len(), 7, "native score roster stays complete");
        assert_eq!(side(&sides, "f:10").colour_faction, None);
        let sides = assign(
            vec![faction(10, &["A", "B", "C"]), faction(20, &["P", "Q", "R"])],
            &tags,
            wars(&[("A", "X"), ("B", "X"), ("C", "X"), ("D", "X")]),
        )
        .unwrap();
        assert_eq!(side(&sides, "f:10").colour_faction, Some(10));
        assert_eq!(side(&sides, "f:20").colour_faction, None);
    }

    #[test]
    fn factionless_allies_form_one_side_and_a_lone_country_keeps_its_viewpoint() {
        let tags = countries(&["opaque:tag", "B", "C", "X"]);
        let sides = assign(vec![], &tags, wars(&[("opaque:tag", "X"), ("B", "X")])).unwrap();
        let coalition = side(&sides, "w:B");
        assert_eq!(coalition.members, ["B", "opaque:tag"]);
        assert_eq!(coalition.leader, None);
        assert_eq!(coalition.co_belligerents, 1);
        assert_eq!(side(&sides, "c:C").leader.as_deref(), Some("C"));
        assert_eq!(side(&sides, "c:X").leader.as_deref(), Some("X"));
    }

    #[test]
    fn a_direct_enemy_cannot_join_a_faction_or_a_factionless_coalition() {
        let tags = countries(&["A", "B", "C", "X"]);
        let relations = wars(&[("A", "X"), ("B", "X"), ("A", "B"), ("C", "X")]);
        let sides = assign(vec![faction(1, &["A"])], &tags, relations.clone()).unwrap();
        assert!(!side(&sides, "f:1").members.contains(&"B".to_owned()));
        let sides = assign(vec![], &tags, relations).unwrap();
        assert!(sides.iter().all(|side| !(side.members.contains(&"A".to_owned()) && side.members.contains(&"B".to_owned()))));
    }

    #[test]
    fn hostile_co_belligerents_never_join_the_same_side() {
        let tags = countries(&["A", "B", "C", "X"]);
        let sides = assign(vec![faction(1, &["A"])], &tags, wars(&[("A", "X"), ("B", "X"), ("C", "X"), ("B", "C")])).unwrap();
        let pact = side(&sides, "f:1");
        assert_eq!(pact.members, ["A", "B"]);
        assert_eq!(pact.co_belligerents, 1);
        assert_eq!(side(&sides, "c:C").leader.as_deref(), Some("C"));
    }

    #[test]
    fn distinct_enemies_beat_member_link_counts_and_leader_participation_breaks_ties() {
        let tags = countries(&["A", "A2", "A3", "B", "B2", "C", "X", "Y"]);
        let relations = wars(&[("A", "X"), ("A2", "X"), ("A3", "X"), ("B", "X"), ("B", "Y"), ("A2", "B2"), ("C", "X"), ("C", "Y")]);
        let sides = assign(vec![faction(10, &["A", "A2", "A3"]), faction(9, &["B", "B2"])], &tags, relations).unwrap();
        assert!(side(&sides, "f:9").members.contains(&"C".into()));
        let sides = assign(vec![faction(1, &["A", "A2"]), faction(9, &["B", "B2"])], &tags, wars(&[("A2", "X"), ("B", "X"), ("A2", "B2"), ("C", "X")])).unwrap();
        assert!(side(&sides, "f:9").members.contains(&"C".into()), "leader participation beats the lower id");
    }

    #[test]
    fn merged_faction_key_does_not_depend_on_members_or_the_regional_score_leader() {
        let tags = countries(&["A", "B", "C", "X"]);
        let first = assign(vec![faction(10, &["A"]), faction(9, &["B"])], &tags, wars(&[("A", "X"), ("B", "X")])).unwrap();
        let next = assign(vec![faction(10, &["C"]), faction(9, &["B"])], &tags, wars(&[("C", "X"), ("B", "X")])).unwrap();
        assert_eq!(side(&first, "f:9").key, side(&next, "f:9").key);
        assert_eq!(side(&first, "f:9").factions, [9, 10]);
    }

    #[test]
    fn merged_side_ties_use_its_supplying_leader_not_every_allied_leader() {
        let tags = countries(&["A", "A2", "B", "C", "D", "D2", "X", "Y", "Z"]);
        let sides = assign(
            vec![faction(1, &["A", "A2"]), faction(2, &["B"]), faction(9, &["D", "D2"])],
            &tags,
            wars(&[("A", "Z"), ("A2", "X"), ("B", "X"), ("B", "Y"), ("D", "X"), ("D2", "Y"), ("A2", "D2"), ("C", "X"), ("C", "Y")]),
        )
        .unwrap();
        assert_eq!(side(&sides, "f:1").factions.len(), 2);
        assert!(side(&sides, "f:9").members.contains(&"C".into()));
    }

    #[test]
    fn duplicate_membership_does_not_create_two_shares_for_one_country() {
        assert!(assign(vec![faction(1, &["A"]), faction(2, &["A"])], &countries(&["A"]), vec![]).is_none());
    }

    fn run(tag: &str, first_hour: i32, last_hour: i32) -> Run {
        Run { tag: tag.into(), first_hour, last_hour }
    }

    fn identity(slot: u32) -> Vec<Identity> {
        vec![Identity { first_hour: 0, last_hour: 1_000, slot, country_id: slot }]
    }

    #[test]
    fn membership_changes_apply_at_their_hour_and_unobserved_groups_assign_no_sides() {
        let groups = Groups {
            factions: [(9, FactionHistory { first_hour: 100, members: vec![run("GER", 100, 100), run("FRA", 101, 103)], leaders: vec![run("GER", 100, 100), run("FRA", 101, 103)], incomplete: vec![] })].into(),
            wars: vec![],
            unobserved_factions: vec![(102, 102)],
            unobserved_wars: vec![],
            identities: [("GER".to_owned(), identity(1)), ("FRA".to_owned(), identity(2))].into(),
        };
        let index = GroupIndex::new(&groups);
        let anchors = historical_anchors(&groups);
        let tags = countries(&["GER", "FRA"]);
        let at = |hour| indexed_roster(&groups, &index, hour, &tags).and_then(|sides| keyed(&groups, hour, sides, &anchors));
        assert_eq!(side(&at(100).unwrap(), "f:9").leader.as_deref(), Some("GER"));
        let next = at(101).unwrap();
        assert_eq!(side(&next, "f:9").leader.as_deref(), Some("FRA"));
        assert_eq!(side(&next, "c:GER").leader.as_deref(), Some("GER"));
        assert!(at(102).is_none());
        assert!(at(104).unwrap().iter().all(|side| side.faction.is_none()), "a faction with no member then is no side");
    }

}
