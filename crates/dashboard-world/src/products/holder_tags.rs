use dashboard_base::products::Country;
use dashboard_query::{Query, identity::{Holder, Holders}};
use std::collections::{BTreeSet, HashMap};

pub type Identity = (u32, u32);

pub fn matches(country: &Country, identity: Identity, tag: &str) -> bool {
    match country {
        Country::Tag(chosen) => chosen == tag,
        Country::Slot(slot) => identity.0 == *slot,
    }
}

#[derive(Default)]
pub struct HolderTags {
    by: HashMap<Identity, Vec<(String, i32, i32)>>,
}

impl HolderTags {
    pub fn new(query: &Query<'_>) -> Self {
        Self::from_holders(&query.holders)
    }

    pub fn from_holders(holders: &Holders) -> Self {
        let mut by: HashMap<Identity, Vec<(String, i32, i32)>> = HashMap::new();
        for (tag, runs) in &holders.tags {
            for run in runs {
                if let Holder::Found { slot, id } = run.holder {
                    by.entry((slot, id)).or_default().push((tag.clone(), run.first, run.last));
                }
            }
        }
        Self::from_runs(by)
    }

    pub fn from_runs(mut by: HashMap<Identity, Vec<(String, i32, i32)>>) -> Self {
        for runs in by.values_mut() {
            runs.sort_unstable_by_key(|run| run.1);
        }
        Self { by }
    }

    pub fn tag_at(&self, identity: Identity, hour: i32) -> Option<&str> {
        let runs = self.by.get(&identity)?;
        let at = runs.partition_point(|run| run.2 < hour);
        runs.get(at).filter(|run| run.1 <= hour).map(|run| run.0.as_str())
    }

    pub fn latest_at(&self, identity: Identity, hour: i32) -> Option<&str> {
        let runs = self.by.get(&identity)?;
        let at = runs.partition_point(|run| run.1 <= hour);
        at.checked_sub(1).and_then(|at| runs.get(at)).map(|run| run.0.as_str())
    }

    pub fn names_in(&self, identity: Identity, first: i32, last: i32) -> BTreeSet<&str> {
        self.by.get(&identity).into_iter().flatten()
            .filter(|(_, from, to)| *from <= last && *to >= first)
            .map(|(tag, _, _)| tag.as_str())
            .collect()
    }

    pub fn edges(&self, identity: Identity, first: i32, last: i32, out: &mut BTreeSet<i32>) {
        for (_, a, b) in self.by.get(&identity).map(Vec::as_slice).unwrap_or_default() {
            for edge in [*a, b.saturating_add(1)] {
                if first < edge && edge <= last {
                    out.insert(edge);
                }
            }
        }
    }

    pub fn holding(&self, country: &Country, first: i32, last: i32) -> Vec<(Identity, i32, i32)> {
        let mut held: Vec<(Identity, i32, i32)> = self
            .by
            .iter()
            .flat_map(|(identity, runs)| runs.iter().filter(move |run| matches(country, *identity, &run.0)).map(move |run| (*identity, run.1.max(first), run.2.min(last))))
            .filter(|(_, a, b)| a <= b)
            .collect();
        held.sort_unstable();
        let mut out: Vec<(Identity, i32, i32)> = Vec::with_capacity(held.len());
        for run in held {
            match out.last_mut() {
                Some(last) if last.0 == run.0 && i64::from(last.2) + 1 >= i64::from(run.1) => last.2 = last.2.max(run.2),
                _ => out.push(run),
            }
        }
        out.sort_unstable_by_key(|run| run.1);
        out
    }
}
