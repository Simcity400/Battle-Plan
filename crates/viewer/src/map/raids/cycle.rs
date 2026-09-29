//! The viewing country's raid offers, evaluated a batch of targets at a
//! time as the game's player cache evaluates them.
//!
//! `CPlayerStatusCache::Update@141998AB0` keeps one row per target status,
//! index for index with `CRaidSystem+8` (`EvaluateTarget@14199A190` reads
//! both at one index), each row holding the target's shown and launchable
//! bits. A rebuild frame, after every executed command and so at least every
//! hour, makes every row pending. Each frame then evaluates
//! `n = min(pending, NRaids.MAX_TARGETS_TO_UPDATE_PER_FRAME)` rows from a
//! rolling cursor, `(cursor + i) % rows`, and advances the cursor to
//! `(cursor + n) % rows`; the other rows keep the bits they were last
//! evaluated with. The target array only grows (`SyncTargets@141997E60`
//! adds rows, `AppendTarget@141948CF0` is its only writer), so a row is one
//! target for the whole session.
//!
//! A [`Cycle`] is that cache for one viewing country since a reset: each
//! hour begins as a rebuild frame ([`Cycle::begin`]) and each step evaluates
//! one batch at that hour ([`Cycle::step`]). A row not evaluated since the
//! reset shows nothing. The cursor's native position is one client's frame
//! history, which no recording holds; a cycle starts at row 0. Each kept row
//! holds its target's scope, and an hour whose row holds another target, or
//! whose table lost rows, cannot continue the cycle: it must start again.
//! A step's rows may be evaluated on several threads, as the game's
//! `parallel_for` splits them; the result is the in-order one
//! ([`Sweep::evaluate`]).
//!
//! Each province's offer is kept with its rows and made again only when a
//! step evaluates one of them (`refresh_counters@141999D30` recounts every
//! province; the counts of the others are unchanged).

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use replay::{BranchId, Hour, rows::RaidScope};

use super::{
    RaidAssets,
    offers::{Accumulator, Evaluation, Province, Sweep},
    script::Eval,
};

/// Rows per shared chunk: a step changes a few chunks, and a copy of the
/// cycle shares the rest.
const CHUNK: usize = 64;

/// One player cache row.
#[derive(Clone, Debug)]
struct Row {
    scope: RaidScope,
    /// The province its offer counts in (`province_resolver@140FD1760`).
    province: Option<u32>,
    /// Its last evaluation and the hour it was made at, since the reset.
    kept: Option<Arc<(Hour, Evaluation)>>,
}

/// One province's rows and what their kept evaluations show.
#[derive(Clone, Debug, Default)]
pub struct Shown {
    /// Its rows, ascending.
    rows: Vec<usize>,
    /// Why its undecided rows are undecided, in row order.
    pub reasons: Vec<String>,
    /// Some row is undecided: the province is withheld.
    pub withheld: bool,
    /// A byte counter wrapped past 255.
    pub wrapped: bool,
    pub offer: Option<Province>,
}

/// The player cache for one viewing country since a reset.
#[derive(Clone, Debug)]
pub struct Cycle {
    branch: BranchId,
    ours: u32,
    /// `CPlayerStatusCache+0x134`: the next row to evaluate.
    cursor: usize,
    /// `+0x138`: rows still to evaluate at this hour.
    pending: usize,
    /// `+0x70`, by row, in chunks.
    rows: Vec<Arc<Vec<Row>>>,
    len: usize,
    /// Each target province's rows and offer.
    provinces: BTreeMap<u32, Arc<Shown>>,
    /// Rows whose scope resolves to no province.
    unresolved: usize,
    /// Why this hour's offers are withheld.
    withheld: Vec<String>,
    /// Why the cycle started again, until every row is evaluated since.
    restarted: Option<String>,
}

impl Cycle {
    /// A reset cache: no row evaluated.
    pub fn new(branch: BranchId, ours: u32) -> Self {
        Self {
            branch,
            ours,
            cursor: 0,
            pending: 0,
            rows: Vec::new(),
            len: 0,
            provinces: BTreeMap::new(),
            unresolved: 0,
            withheld: Vec::new(),
            restarted: None,
        }
    }

    /// A reset cache that replaces one whose rows no longer hold their
    /// targets, for `reason`.
    pub fn restarted(branch: BranchId, ours: u32, reason: String) -> Self {
        Self {
            restarted: Some(reason),
            ..Self::new(branch, ours)
        }
    }

    /// Why the cycle started again, until every row is evaluated since.
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

    /// The rebuild frame of `sweep`'s hour: new target statuses get rows and
    /// every row is pending. `Err` when a kept row holds another target or
    /// rows went away since: the cycle must start again at this hour.
    pub fn begin(&mut self, assets: &RaidAssets, sweep: &Sweep<'_, '_>) -> Result<(), String> {
        self.withheld.clone_from(&sweep.withheld);
        if !self.withheld.is_empty() {
            // Nothing can be evaluated; the rows wait for a complete hour.
            self.pending = 0;
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
        for target in &targets[self.len..] {
            if self.len.is_multiple_of(CHUNK) {
                self.rows.push(Arc::new(Vec::with_capacity(CHUNK)));
            }
            let province = assets.province(&target.scope);
            match province {
                // A row not yet evaluated leaves the province's offer as it is.
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
            });
            self.len += 1;
        }
        self.pending = self.len;
        Ok(())
    }

    /// One frame of `Update@141998AB0` at `sweep`'s hour: the next
    /// `min(pending, sweep.batch)` rows from the cursor, evaluated.
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
        for (at, evaluation) in rows.iter().zip(sweep.evaluate(&rows)) {
            let row = self.row_mut(*at);
            row.kept = Some(Arc::new((hour, evaluation)));
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

    /// Whether every row has been evaluated at this hour.
    pub fn complete(&self) -> bool {
        self.pending == 0
    }

    /// Rows evaluated at this hour, of all rows.
    pub fn progress(&self) -> (usize, usize) {
        (self.len - self.pending, self.len)
    }

    /// A cycle with a row still pending, standing for any incomplete one.
    #[cfg(test)]
    pub fn pending(branch: BranchId, ours: u32) -> Self {
        Self {
            pending: 1,
            ..Self::new(branch, ours)
        }
    }

    /// Row `at`'s kept evaluation and its hour.
    #[cfg(test)]
    pub fn kept(&self, at: usize) -> Option<&Arc<(Hour, Evaluation)>> {
        self.row(at).kept.as_ref()
    }

    /// Each target province's rows and offer.
    pub fn provinces(&self) -> &BTreeMap<u32, Arc<Shown>> {
        &self.provinces
    }

    /// Rows whose scope resolves to no province.
    pub fn unresolved(&self) -> usize {
        self.unresolved
    }

    /// Why this hour's offers are withheld; empty when they are not.
    pub fn withheld(&self) -> &[String] {
        &self.withheld
    }

    /// What `rows`, one province's in ascending order, show: its shown and
    /// launchable counts, anchor and glyph pairs.
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

    /// The offers the kept rows show, as a full evaluation reports them.
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
