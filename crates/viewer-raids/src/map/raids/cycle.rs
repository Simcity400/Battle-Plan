
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use replay_data::{BranchId, Hour, rows::RaidScope};

use super::{
    RaidAssets,
    offers::{Accumulator, Evaluation, Province, Sweep},
    paths::Held,
    script::Eval,
};

const CHUNK: usize = 64;

#[derive(Clone, Debug)]
struct Row {
    scope: RaidScope,
    province: Option<u32>,
    kept: Option<Arc<(Hour, Evaluation)>>,
    held: Box<[Held]>,
}

#[derive(Clone, Debug, Default)]
pub struct Shown {
    rows: Vec<usize>,
    pub reasons: Vec<String>,
    pub withheld: bool,
    pub wrapped: bool,
    pub offer: Option<Province>,
}

#[derive(Clone, Debug)]
pub struct Cycle {
    branch: BranchId,
    ours: u32,
    cursor: usize,
    pending: usize,
    begun: Option<Hour>,
    rows: Vec<Arc<Vec<Row>>>,
    len: usize,
    provinces: BTreeMap<u32, Arc<Shown>>,
    unresolved: usize,
    withheld: Vec<String>,
    restarted: Option<String>,
}

impl Cycle {
    pub fn new(branch: BranchId, ours: u32) -> Self {
        Self {
            branch,
            ours,
            cursor: 0,
            pending: 0,
            begun: None,
            rows: Vec::new(),
            len: 0,
            provinces: BTreeMap::new(),
            unresolved: 0,
            withheld: Vec::new(),
            restarted: None,
        }
    }

    pub fn restarted(branch: BranchId, ours: u32, reason: String) -> Self {
        Self {
            restarted: Some(reason),
            ..Self::new(branch, ours)
        }
    }

    pub fn restart(&self) -> Option<&str> {
        self.restarted.as_deref()
    }

    pub fn branch(&self) -> BranchId {
        self.branch
    }

    pub fn ours(&self) -> u32 {
        self.ours
    }

    fn row(&self, at: usize) -> &Row {
        &self.rows[at / CHUNK][at % CHUNK]
    }

    fn row_mut(&mut self, at: usize) -> &mut Row {
        &mut Arc::make_mut(&mut self.rows[at / CHUNK])[at % CHUNK]
    }

    pub fn begin(&mut self, assets: &RaidAssets, sweep: &Sweep<'_, '_>) -> Result<(), String> {
        self.withheld.clone_from(&sweep.withheld);
        if !self.withheld.is_empty() {
            self.pending = 0;
            self.begun = None;
            return Ok(());
        }
        let targets = sweep.targets;
        if targets.len() < self.len {
            return Err(format!(
                "the raid target statuses went from {} rows to {}",
                self.len,
                targets.len()
            ));
        }
        for (at, target) in targets.iter().enumerate().take(self.len) {
            if self.row(at).scope != target.scope {
                return Err(format!(
                    "raid target status {at} names another target than before"
                ));
            }
        }
        let grew = targets.len() > self.len;
        for target in &targets[self.len..] {
            if self.len.is_multiple_of(CHUNK) {
                self.rows.push(Arc::new(Vec::with_capacity(CHUNK)));
            }
            let province = assets.province(&target.scope);
            match province {
                Some(province) => Arc::make_mut(self.provinces.entry(province).or_default())
                    .rows
                    .push(self.len),
                None => self.unresolved += 1,
            }
            let chunk = Arc::make_mut(self.rows.last_mut().expect("pushed above"));
            chunk.push(Row {
                scope: target.scope.clone(),
                province,
                kept: None,
                held: Box::default(),
            });
            self.len += 1;
        }
        let hour = sweep.hour();
        if grew || self.begun != Some(hour) {
            self.pending = self.len;
        }
        self.begun = Some(hour);
        Ok(())
    }

    pub fn step(&mut self, sweep: &Sweep<'_, '_>) {
        if self.pending == 0 {
            return;
        }
        if self.cursor >= self.len {
            self.cursor = 0;
        }
        let hour = sweep.hour();
        let n = self.pending.min(sweep.batch);
        let rows: Vec<usize> = (0..n).map(|i| (self.cursor + i) % self.len).collect();
        let mut touched = BTreeSet::new();
        for (at, (evaluation, held)) in rows.iter().zip(sweep.evaluate(&rows)) {
            let row = self.row_mut(*at);
            row.kept = Some(Arc::new((hour, evaluation)));
            row.held = held;
            touched.extend(row.province);
        }
        for province in touched {
            let rows = self.provinces[&province].rows.clone();
            let shown = self.shown(rows);
            self.provinces.insert(province, Arc::new(shown));
        }
        self.cursor = (self.cursor + n) % self.len;
        self.pending -= n;
        if self.pending == 0 {
            self.restarted = None;
        }
    }

    pub fn complete(&self) -> bool {
        self.pending == 0
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.len - self.pending, self.len)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pending(branch: BranchId, ours: u32) -> Self {
        Self {
            pending: 1,
            ..Self::new(branch, ours)
        }
    }

    #[cfg(test)]
    pub fn kept(&self, at: usize) -> Option<&Arc<(Hour, Evaluation)>> {
        self.row(at).kept.as_ref()
    }

    pub fn provinces(&self) -> &BTreeMap<u32, Arc<Shown>> {
        &self.provinces
    }

    pub fn unresolved(&self) -> usize {
        self.unresolved
    }

    pub fn withheld(&self) -> &[String] {
        &self.withheld
    }

    fn shown(&self, rows: Vec<usize>) -> Shown {
        let mut entry = Accumulator::default();
        let mut reasons = Vec::new();
        for &at in &rows {
            let row = self.row(at);
            let Some(kept) = &row.kept else {
                continue;
            };
            let (shown, launchable) = &kept.1;
            match shown {
                Ok(shown) if shown.is_empty() => {}
                Err(why) => {
                    entry.unknown = true;
                    reasons.extend(why.iter().cloned());
                }
                Ok(shown) => {
                    entry.shown = entry.shown.wrapping_add(1);
                    entry.shown_wrapped |= entry.shown == 0;
                    entry.anchor.get_or_insert_with(|| row.scope.clone());
                    entry.targets.push(super::offers::TooltipTarget {
                        scope: row.scope.clone(),
                        types: shown.clone(),
                    });
                    entry
                        .pairs
                        .extend(shown.iter().map(|&index| (index, row.scope.building_type)));
                    match launchable {
                        Eval::True => entry.launchable += 1,
                        Eval::False => {}
                        Eval::Unknown(why) => {
                            entry.open += 1;
                            entry.reasons.extend(why.iter().cloned());
                        }
                    }
                }
            }
        }
        let withheld = entry.unknown;
        let wrapped = !withheld && (entry.shown_wrapped || entry.launchable + entry.open > 255);
        let launchable = entry.tint();
        let offer = match (withheld, entry.anchor, entry.shown != 0) {
            (false, Some(anchor), true) => Some(Province {
                anchor,
                pairs: entry.pairs,
                targets: entry.targets,
                launchable,
            }),
            _ => None,
        };
        Shown {
            rows,
            reasons,
            withheld,
            wrapped,
            offer,
        }
    }

    #[cfg(test)]
    pub fn offers(&self) -> super::offers::Offers {
        let mut out = super::offers::Offers {
            reasons: self.withheld.clone(),
            ..Default::default()
        };
        if !self.withheld.is_empty() {
            return out;
        }
        out.unresolved = self.unresolved;
        for (&province, shown) in &self.provinces {
            out.reasons.extend(shown.reasons.iter().cloned());
            out.wrapped += usize::from(shown.wrapped);
            if shown.withheld {
                out.withheld.insert(province);
            }
            if let Some(offer) = &shown.offer {
                out.provinces.insert(province, offer.clone());
            }
        }
        out
    }
}
