use replay_data::rows::{WarOverviewCountryRow, WarRelationRow};
use std::{cmp::Ordering, collections::{HashMap, HashSet}};

pub(crate) type Pair = (u32, u32);

pub(super) struct ScoreOrder {
    pub first: u32,
    pub second: u32,
    pub order: Ordering,
}

pub(crate) fn pair(a: u32, b: u32) -> Pair {
    if a < b { (a, b) } else { (b, a) }
}

#[derive(Clone)]
pub(crate) struct World {
    pub owned: HashMap<u32, Vec<u32>>,
    pub relations: HashMap<Pair, WarRelationRow>,
    pub adjacent: HashMap<u32, Vec<u32>>,
    pub factions: HashMap<u32, u64>,
    pub scores: HashMap<u32, WarOverviewCountryRow>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Blob {
    pub first: Vec<u32>,
    pub second: Vec<u32>,
}

pub(crate) struct Grouping {
    pub view: u32,
    pub countries: HashSet<u32>,
    pub blobs: Vec<(Blob, Vec<Pair>)>,
}

impl World {
    pub fn active(&self, a: u32, b: u32) -> bool {
        self.relations.contains_key(&pair(a, b))
    }
    fn same_faction(&self, a: u32, b: u32) -> bool {
        self.factions
            .get(&a)
            .is_some_and(|f| *f != 0 && self.factions.get(&b) == Some(f))
    }
    fn candidate(&self, a: u32, b: u32) -> Vec<u32> {
        let mut side = vec![a];
        for &x in self.owned.get(&b).into_iter().flatten() {
            if x != a && !self.active(a, x) {
                side.push(x);
            }
        }
        let mut positions: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, &x) in side.iter().enumerate().skip(1) {
            positions.entry(x).or_default().push(i);
        }
        let mut remove = HashSet::new();
        for (&x, indices) in &positions {
            for &y in self.adjacent.get(&x).into_iter().flatten() {
                if x >= y {
                    continue;
                }
                let Some(other) = positions.get(&y) else {
                    continue;
                };
                let fx = self.same_faction(a, x);
                let fy = self.same_faction(a, y);
                if !fx || fy {
                    remove.extend(indices.iter().copied());
                }
                if !fy || fx {
                    remove.extend(other.iter().copied());
                }
            }
        }
        swap_remove(&mut side, remove);
        side
    }
    pub fn blobs(&self, view: u32) -> Vec<Blob> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut seed_signatures = HashMap::new();
        let mut seed_counts = HashMap::<u32, usize>::new();
        for &enemy in self.owned.get(&view).into_iter().flatten() {
            *seed_counts.entry(enemy).or_default() += 1;
        }
        let mut by_lengths = HashMap::<(usize, usize), HashSet<(Vec<u32>, Vec<u32>)>>::new();
        let mut repeats = HashMap::<(usize, usize), HashSet<(Vec<u32>, Vec<u32>)>>::new();
        let mut covered = HashSet::new();
        for &enemy in self.owned.get(&view).into_iter().flatten() {
            if covered.contains(&enemy) || !self.active(view, enemy) {
                continue;
            }
            let mut neighbors = self.adjacent.get(&enemy).cloned().unwrap_or_default();
            neighbors.sort_unstable();
            let signature = (
                self.owned.get(&enemy).cloned().unwrap_or_default(),
                neighbors,
                self.factions.get(&enemy).copied().filter(|f| *f != 0),
            );
            if seed_counts[&enemy] == 1 && seed_signatures.get(&signature) == Some(&true) {
                continue;
            }
            let first = self.candidate(view, enemy);
            seed_signatures.insert(
                signature,
                first.iter().skip(1).all(|&x| self.active(enemy, x)),
            );
            let mut second = self.candidate(enemy, view);
            let counts: HashMap<u32, usize> =
                first.iter().skip(1).fold(HashMap::new(), |mut m, &x| {
                    *m.entry(x).or_default() += 1;
                    m
                });
            let mut remove = HashSet::new();
            for (index, &y) in second.iter().enumerate().skip(1) {
                let active = self
                    .adjacent
                    .get(&y)
                    .into_iter()
                    .flatten()
                    .map(|x| counts.get(x).copied().unwrap_or(0))
                    .sum::<usize>();
                if active != first.len() - 1 {
                    remove.insert(index);
                }
            }
            swap_remove(&mut second, remove);
            let key = (
                membership(&first),
                membership(&second),
                first.len(),
                second.len(),
            );
            let unique = key.0.len() == first.len() && key.1.len() == second.len();
            let index = if unique { &repeats } else { &by_lengths };
            let duplicate = seen.contains(&key)
                || index
                    .get(&(first.len(), second.len()))
                    .into_iter()
                    .flatten()
                    .any(|(a, b)| subset(a, &key.0) && subset(b, &key.1));
            if !duplicate {
                let blob = Blob { first, second };
                if self.closed_bipartite(&blob) {
                    covered.extend(blob.second.iter().copied());
                }
                by_lengths
                    .entry((blob.first.len(), blob.second.len()))
                    .or_default()
                    .insert((key.0.clone(), key.1.clone()));
                if !unique {
                    repeats
                        .entry((blob.first.len(), blob.second.len()))
                        .or_default()
                        .insert((key.0.clone(), key.1.clone()));
                }
                seen.insert(key);
                out.push(blob);
            }
        }
        out
    }
    pub fn grouping(&self, view: u32) -> Grouping {
        let enemies = self.owned.get(&view).into_iter().flatten();
        let mut countries = HashSet::from([view]);
        countries.extend(enemies.clone().copied());
        for &enemy in enemies.filter(|&&enemy| self.active(view, enemy)) {
            countries.extend(self.owned.get(&enemy).into_iter().flatten().copied());
        }
        let blobs = self.blobs(view).into_iter().map(|blob| {
            let pairs = self.inside(&blob);
            (blob, pairs)
        }).collect();
        Grouping { view, countries, blobs }
    }
    fn closed_bipartite(&self, blob: &Blob) -> bool {
        let first: HashSet<_> = blob.first.iter().copied().collect();
        let second: HashSet<_> = blob.second.iter().copied().collect();
        if first.len() != blob.first.len() || second.len() != blob.second.len() {
            return false;
        }
        for (side, opposite) in [(&first, &second), (&second, &first)] {
            for country in side {
                let Some(owned) = self.owned.get(country) else {
                    return false;
                };
                let Some(adjacent) = self.adjacent.get(country) else {
                    return false;
                };
                if owned.len() != opposite.len()
                    || adjacent.len() != opposite.len()
                    || owned.iter().copied().collect::<HashSet<_>>() != *opposite
                    || adjacent.iter().any(|x| !opposite.contains(x))
                {
                    return false;
                }
            }
        }
        true
    }
    pub fn inside(&self, blob: &Blob) -> Vec<Pair> {
        let second: HashSet<_> = blob.second.iter().copied().collect();
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for &a in &blob.first {
            for &b in self.adjacent.get(&a).into_iter().flatten() {
                let p = pair(a, b);
                if second.contains(&b) && seen.insert(p) {
                    out.push(p);
                }
            }
        }
        out
    }
    pub fn score(&self, country: u32) -> Result<(u8, i32), String> {
        let s = self.scores.get(&country).ok_or_else(|| {
            format!("Not recorded: war overview score for country slot {country}")
        })?;
        if s.score_available != 1 {
            return Err(format!(
                "Not recorded: war overview score outside captured viewpoint reach for country slot {country}"
            ));
        }
        Ok((s.major, s.divisions.wrapping_add(s.factories)))
    }
    pub(super) fn representative(&self, blob: &Blob, pairs: &[Pair], orders: &mut Vec<ScoreOrder>) -> Result<Pair, String> {
        self.oldest(blob, pairs).and_then(|oldest| self.ranked(blob, oldest, orders))
    }
    fn oldest(&self, blob: &Blob, pairs: &[Pair]) -> Result<Pair, String> {
        let first_rank: HashMap<_, _> = blob
            .first
            .iter()
            .enumerate()
            .rev()
            .map(|(i, &x)| (x, i))
            .collect();
        let second_rank: HashMap<_, _> = blob
            .second
            .iter()
            .enumerate()
            .rev()
            .map(|(i, &x)| (x, i))
            .collect();
        let oldest = pairs
            .iter()
            .copied()
            .min_by_key(|p| {
                let r = &self.relations[p];
                let (a, b) = if first_rank.contains_key(&p.0) {
                    (p.0, p.1)
                } else {
                    (p.1, p.0)
                };
                (r.start_hour, first_rank[&a], second_rank[&b])
            })
            .ok_or_else(|| "Not recorded: blob has no active representative".to_owned())?;
        Ok(oldest)
    }
    fn ranked(&self, blob: &Blob, oldest: Pair, orders: &mut Vec<ScoreOrder>) -> Result<Pair, String> {
        let original = &self.relations[&oldest];
        let major_first = self.score(original.first_country)?.0 != 0;
        let major_second = self.score(original.second_country)?.0 != 0;
        if major_first && major_second {
            return Ok(oldest);
        }
        let (mut first, mut second) = if blob.first.contains(&original.first_country) {
            (blob.first.clone(), blob.second.clone())
        } else {
            (blob.second.clone(), blob.first.clone())
        };
        let scores: HashMap<_, _> = first
            .iter()
            .chain(&second)
            .map(|&x| Ok((x, self.score(x)?)))
            .collect::<Result<_, String>>()?;
        if scores.values().any(|s| s.0 > 1) {
            return Err("Not recorded: unsupported nonboolean native major byte".into());
        }
        let compare = |a: &u32, b: &u32| {
            scores[b]
                .0
                .cmp(&scores[a].0)
                .then(scores[b].1.cmp(&scores[a].1))
        };
        first.sort_by(compare);
        second.sort_by(compare);
        for side in [&first, &second] {
            for pair in side.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if scores[&a].0 == scores[&b].0 {
                    orders.push(ScoreOrder { first: a, second: b, order: scores[&a].1.cmp(&scores[&b].1) });
                }
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < first.len() && j < second.len() {
            let a = if major_first {
                original.first_country
            } else {
                first[i]
            };
            let b = if major_second {
                original.second_country
            } else {
                second[j]
            };
            if self.active(a, b) {
                return Ok(pair(a, b));
            }
            let (first, second) = (first[i], second[j]);
            let order = scores[&first].1.cmp(&scores[&second].1);
            orders.push(ScoreOrder { first, second, order });
            if order.is_gt() {
                i += 1;
            } else {
                j += 1;
            }
        }
        if i == first.len() {
            let a = if major_first {
                original.first_country
            } else {
                first[first.len() - 1]
            };
            while j < second.len() {
                let b = if major_second {
                    original.second_country
                } else {
                    second[j]
                };
                if self.active(a, b) {
                    return Ok(pair(a, b));
                }
                j += 1;
            }
        } else {
            let b = if major_second {
                original.second_country
            } else {
                second[second.len() - 1]
            };
            while i < first.len() {
                let a = if major_first {
                    original.first_country
                } else {
                    first[i]
                };
                if self.active(a, b) {
                    return Ok(pair(a, b));
                }
                i += 1;
            }
        }
        Ok(oldest)
    }
}
fn membership(side: &[u32]) -> Vec<u32> {
    let mut members = side.to_vec();
    members.sort_unstable();
    members.dedup();
    members
}
fn swap_remove(side: &mut Vec<u32>, indices: HashSet<usize>) {
    let mut indices: Vec<_> = indices.into_iter().collect();
    indices.sort_unstable();
    for index in indices.into_iter().rev() {
        side.swap_remove(index);
    }
}

fn subset(old: &[u32], new: &[u32]) -> bool {
    let mut at = 0;
    for member in old {
        while at < new.len() && new[at] < *member {
            at += 1;
        }
        if new.get(at) != Some(member) {
            return false;
        }
    }
    true
}
