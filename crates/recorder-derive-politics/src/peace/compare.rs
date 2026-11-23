
use super::{Government, Snapshot, Tag};
use replay_data::rows::PeaceOutcomeRow;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    sync::Arc,
};

pub(super) fn pair(a: Tag, b: Tag) -> (Tag, Tag) {
    if a < b { (a, b) } else { (b, a) }
}

fn ongoing(start: i32, end: i32, hour: i32) -> bool {
    end <= start || hour < end
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Transfer {
    pub state: u32,
    pub from: Tag,
    pub to: Tag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Item {
    pub change: u8,
    pub state: u32,
    pub from: (u32, u32),
    pub to: (u32, u32),
    pub vp: Option<i64>,
}

const NOBODY: (u32, u32) = (PeaceOutcomeRow::NONE, PeaceOutcomeRow::NONE);

pub(super) fn transfers(before: &Snapshot, after: &Snapshot, recorded: bool, states: Option<&[u32]>) -> Vec<Transfer> {
    let mut unresolved_pairs = HashSet::new();
    let mut unresolved_countries = HashSet::new();
    let mut unidentified = false;
    for (first, second) in &after.wars.unresolved {
        match (after.countries.tags.get(first), after.countries.tags.get(second)) {
            (Some(&first), Some(&second)) => {
                unresolved_pairs.insert(pair(first, second));
            }
            (Some(&country), None) | (None, Some(&country)) => {
                unresolved_countries.insert(country);
            }
            (None, None) => unidentified = true,
        }
    }
    let changed_tags: HashSet<Tag> = if Arc::ptr_eq(&before.countries, &after.countries) {
        HashSet::new()
    } else {
        before
            .countries
            .tags
            .iter()
            .filter_map(|(raw, &tag)| after.countries.tags.get(raw).filter(|&&now| now != tag).map(|&now| [tag, now]))
            .flatten()
            .collect()
    };
    let every: Vec<u32>;
    let states = match states {
        Some(states) => states,
        None => {
            every = before.owners.keys().copied().collect();
            &every
        }
    };
    let mut found = Vec::new();
    for &state in states {
        let (Some(from), Some(to)) = (before.owners.get(&state), after.owners.get(&state)) else { continue };
        let (Some(&from), Some(&to)) = (before.countries.tags.get(from), after.countries.tags.get(to)) else { continue };
        if from == to || changed_tags.contains(&from) || changed_tags.contains(&to) {
            continue;
        }
        if [from, to].into_iter().any(|tag| before.countries.identities.get(&tag).is_none() || before.countries.identities.get(&tag) != after.countries.identities.get(&tag)) {
            continue;
        }
        let key = pair(from, to);
        let Some(&(start, end)) = before.wars.pairs.get(&key) else { continue };
        if start > before.hour || !ongoing(start, end, before.hour) {
            continue;
        }
        match after.wars.pairs.get(&key) {
            Some(&(new_start, new_end)) => {
                if new_start != start || new_end <= start || new_end <= before.hour || new_end > after.hour || (!recorded && new_end != after.hour) {
                    continue;
                }
            }
            None => {
                if unidentified || unresolved_pairs.contains(&key) || unresolved_countries.contains(&from) || unresolved_countries.contains(&to) {
                    continue;
                }
            }
        }
        found.push(Transfer { state, from, to });
    }
    found.sort_unstable_by_key(|transfer| transfer.state);
    found.dedup();
    found
}

pub(super) fn components(transfers: &[Transfer], after: &Snapshot) -> Vec<Vec<Transfer>> {
    let mut adjacency: HashMap<Tag, Vec<Tag>> = HashMap::new();
    for transfer in transfers {
        adjacency.entry(transfer.from).or_default().push(transfer.to);
        adjacency.entry(transfer.to).or_default().push(transfer.from);
    }
    let holder = |tag: Tag| after.countries.identities.get(&tag).copied().unwrap_or(NOBODY);
    let mut roots: Vec<Tag> = adjacency.keys().copied().collect();
    roots.sort_unstable_by_key(|&tag| (holder(tag), tag));
    let mut group: HashMap<Tag, usize> = HashMap::new();
    let mut count = 0;
    for root in roots {
        if group.contains_key(&root) {
            continue;
        }
        let mut queue = VecDeque::from([root]);
        group.insert(root, count);
        while let Some(country) = queue.pop_front() {
            for &other in &adjacency[&country] {
                if let std::collections::hash_map::Entry::Vacant(entry) = group.entry(other) {
                    entry.insert(count);
                    queue.push_back(other);
                }
            }
        }
        count += 1;
    }
    let mut groups = vec![Vec::new(); count];
    for transfer in transfers {
        groups[group[&transfer.from]].push(*transfer);
    }
    groups
}

fn existence(snapshot: &Snapshot) -> Option<HashMap<Tag, bool>> {
    let exists = snapshot.exists.as_ref()?;
    let mut raws: Vec<(&u32, &bool)> = exists.iter().collect();
    raws.sort_unstable();
    raws.into_iter().map(|(raw, &exists)| Some((*snapshot.countries.tags.get(raw)?, exists))).collect()
}

pub(super) fn populate(before: &Snapshot, after: &Snapshot, transfers: &[Transfer], first_existing: &HashMap<Tag, i32>) -> Vec<Item> {
    let identity = |snapshot: &Snapshot, tag: Tag| snapshot.countries.identities.get(&tag).copied().unwrap_or(NOBODY);
    let vp = |state: u32| after.vps.as_ref().and_then(|vps| vps.get(&state)).copied();
    let mut winners = BTreeSet::new();
    let mut losers = BTreeSet::new();
    let mut items = Vec::new();
    for transfer in transfers {
        winners.insert(transfer.to);
        losers.insert(transfer.from);
        items.push(Item { change: PeaceOutcomeRow::TRANSFER, state: transfer.state, from: identity(before, transfer.from), to: identity(after, transfer.to), vp: vp(transfer.state) });
    }
    let mut releases = HashSet::new();
    if let (Some(old), Some(new)) = (existence(before), existence(after)) {
        let mut owned: BTreeMap<Tag, Vec<u32>> = BTreeMap::new();
        for (&state, raw) in before.owners.iter() {
            if let Some(&tag) = before.countries.tags.get(raw)
                && losers.contains(&tag)
            {
                owned.entry(tag).or_default().push(state);
            }
        }
        let mut released: BTreeMap<(u32, u32), Vec<(u32, Tag)>> = BTreeMap::new();
        for (&from, states) in &owned {
            for &state in states {
                let Some(&country) = after.owners.get(&state).and_then(|raw| after.countries.tags.get(raw)) else { continue };
                if country == from
                    || old.get(&country) != Some(&false)
                    || new.get(&country) != Some(&true)
                    || before.countries.identities.get(&country).is_none()
                    || before.countries.identities.get(&country) != after.countries.identities.get(&country)
                    || first_existing.get(&country).is_some_and(|&first| first <= before.hour)
                {
                    continue;
                }
                releases.insert(country);
                released.entry(identity(after, country)).or_default().push((state, from));
            }
        }
        for (country, mut states) in released {
            states.sort_unstable();
            for (state, from) in states {
                items.push(Item { change: PeaceOutcomeRow::RELEASED, state, from: identity(before, from), to: country, vp: vp(state) });
            }
        }
    }
    if let (Some(old), Some(new)) = (&before.subjects, &after.subjects) {
        for (&(overlord, subject), &start) in new.iter() {
            if !winners.contains(&overlord) || old.contains_key(&(overlord, subject)) || !(losers.contains(&subject) || releases.contains(&subject)) || start <= before.hour || start > after.hour {
                continue;
            }
            items.push(Item { change: PeaceOutcomeRow::NEW_SUBJECT, state: PeaceOutcomeRow::NONE, from: identity(after, subject), to: identity(after, overlord), vp: None });
        }
    }
    if let (Some(old), Some(new)) = (&before.governments, &after.governments) {
        for &country in &losers {
            let (Some(old), Some(new)) = (old.get(&country), new.get(&country)) else { continue };
            if changed(old, new) {
                items.push(Item { change: PeaceOutcomeRow::GOVERNMENT, state: PeaceOutcomeRow::NONE, from: identity(before, country), to: identity(after, country), vp: None });
            }
        }
    }
    items.sort_unstable_by_key(|item| {
        let holder = match item.change {
            PeaceOutcomeRow::TRANSFER | PeaceOutcomeRow::RELEASED => item.to,
            _ => item.from,
        };
        (item.change, holder, item.state)
    });
    items
}

fn changed(old: &Government, new: &Government) -> bool {
    let party = matches!((old.ruling, new.ruling), (Some(a), Some(b)) if a != b);
    let ideology = matches!((&old.ideology, &new.ideology), (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() && a != b);
    let token = matches!((&old.party, &new.party), (Some(a), Some(b)) if a != b);
    party || ideology || token
}
