//! The land census kept in order from projected hours ([`facts::Hour`]), published a replay
//! block at a time as the row walk publishes it ([`Census::publish_block`]): the same pages,
//! which the same readers answer from.
//!
//! An hour applies only what changed. A combat the hour did not name stepped its round counter
//! by one, which its run goes on through; every combat it names, and every one whose
//! participants changed, is read as the row census reads each combat. A run that goes on is
//! checked against the row census's reading once, when it starts to, and is extended only when
//! something reads it: the hour its combat changes, a finalizer, a gap, a publication or a save.

use super::{
    BattleBound, BattleEnd, BattleMoment, BattleWindow, Census, LiveSide,
    facts::{self, Combat, Combats, Countries, Loss, Progress, Table},
    held,
    losses::SideLossInputs,
    windows,
};
use dashboard_text::catalogue::Catalogue;
use replay::campaign::Replay;
use replay_data::{format::FormatVersion, rows::LandCombatSideRow};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

fn invalid(why: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

/// A combat as the census last read it: its round counter less the hour, which it steps by one
/// an hour while no hour names it.
#[derive(Clone, Copy)]
struct Row {
    province: u32,
    counts: [u32; 2],
    offset: i64,
}

impl Row {
    fn of(combat: &Combat, hour: i32) -> Self {
        Self {
            province: combat.province,
            counts: combat.counts,
            offset: i64::from(combat.duration) - i64::from(hour),
        }
    }

    fn at(&self, id: u64, hour: i32) -> Combat {
        Combat {
            id,
            province: self.province,
            duration: (self.offset + i64::from(hour)) as i32,
            counts: self.counts,
        }
    }
}

/// The open battles whose runs go on while their combats step their counters.
#[derive(Default)]
struct Stepping {
    /// Battles whose last run holds through `through`, though it records an earlier last hour.
    members: foldhash::HashSet<usize>,
    through: i32,
    /// The open battles read in the last hour, by combat: those that may start to step.
    fresh: Vec<(u64, usize)>,
    /// Every open battle may start to step: the last hour read them some other way.
    all: bool,
    /// Combats whose open battle may hold no hourly counters.
    lacking: foldhash::HashSet<u64>,
}

/// The tables the census mirrors beside its own state, from a whole hour on.
#[derive(Default)]
struct Mirror {
    /// Whether `combats` holds LandCombats as the census last read it.
    held: bool,
    combats: foldhash::HashMap<u64, Row>,
    sides: Option<foldhash::HashMap<u64, LandCombatSideRow>>,
    progress: Option<foldhash::HashMap<u64, [SideLossInputs; 2]>>,
    /// Combats whose row holds no open battle: a finalizer ended theirs.
    orphans: foldhash::HashSet<u64>,
    stepping: Stepping,
}

impl Mirror {
    fn forget_combats(&mut self) {
        self.held = false;
        self.combats.clear();
        self.orphans.clear();
    }
}

/// A side's participants where no LandCombatSides row names its combat.
fn unrecorded() -> ([Vec<String>; 2], [bool; 2]) {
    ([Vec::new(), Vec::new()], [true; 2])
}

impl Census {
    /// Ends `ordinal`'s stepping: its last run holds through the hour it stepped through.
    fn settle(&mut self, stepping: &mut Stepping, ordinal: usize) {
        if stepping.members.remove(&ordinal)
            && let Some(run) = self
                .episodes
                .get_mut(&ordinal)
                .and_then(|episode| episode.runs.last_mut())
        {
            run.last_hour = stepping.through;
        }
    }

    /// Ends every battle's stepping; the next hour checks every open battle again.
    fn settle_all(&mut self, stepping: &mut Stepping) {
        let members: Vec<usize> = stepping.members.iter().copied().collect();
        for ordinal in members {
            self.settle(stepping, ordinal);
        }
        stepping.fresh.clear();
        stepping.all = true;
    }

    fn observe_projected(
        &mut self,
        branch: u64,
        hour: facts::Hour,
        mirror: &mut Mirror,
    ) -> io::Result<()> {
        let mut own = std::mem::take(&mut self.refusals);
        let observed = dashboard_text::catalogue::own_refusals(&mut own, || {
            self.observe_own_projected(branch, hour, mirror)
        });
        self.warn_unnamed();
        self.refusals = own;
        observed
    }

    fn observe_own_projected(
        &mut self,
        branch: u64,
        hour: facts::Hour,
        mirror: &mut Mirror,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("land.emit", None);
        let facts::Hour {
            hour: at,
            available,
            countries,
            names,
            variants,
            sides_read,
            sides,
            combats,
            progress_read,
            progress,
            results,
            ..
        } = hour;
        self.available |= available;
        let renamed = self.variants.observe_hour_changed(at, names, |_| variants);
        self.renamed(&renamed);
        let update = match &countries {
            Countries::Incomplete => self.links.update_projected(None, None),
            Countries::Same => Ok(false),
            Countries::Read(read) => self.links.update_projected(Some(&read.0), Some(&read.1)),
        };
        let labels = update.is_ok();
        let links_changed = labels != self.labels_known || matches!(update, Ok(true));
        self.labels_known = labels;

        let mut side_changes = Vec::new();
        let mut rebuild = links_changed || sides_read != self.live_sides_read;
        match sides {
            Table::Incomplete => rebuild |= mirror.sides.take().is_some(),
            Table::Same => {}
            Table::Whole(rows) => {
                mirror.sides = Some(rows.into_iter().map(|row| (row.combat, row)).collect());
                rebuild = true;
            }
            Table::Changed { rows, deleted } => {
                let held = mirror
                    .sides
                    .as_mut()
                    .ok_or_else(|| invalid("A land hour changed participants it never held"))?;
                for combat in deleted {
                    held.remove(&combat);
                    side_changes.push(combat);
                }
                for row in rows {
                    side_changes.push(row.combat);
                    held.insert(row.combat, row);
                }
            }
        }
        if rebuild {
            self.live_sides = match (&mirror.sides, sides_read) {
                (Some(rows), true) => rows
                    .iter()
                    .map(|(&combat, row)| (combat, LiveSide::new(Some(row), &self.links, labels)))
                    .collect(),
                _ => Default::default(),
            };
        } else if sides_read && let Some(rows) = &mirror.sides {
            for &combat in &side_changes {
                match rows.get(&combat) {
                    Some(row) => {
                        let side = LiveSide::new(Some(row), &self.links, labels);
                        self.live_sides.insert(combat, side);
                    }
                    None => {
                        self.live_sides.remove(&combat);
                    }
                }
            }
        }
        self.live_sides_read = sides_read;
        let sides_changed = rebuild || !side_changes.is_empty();

        let progress_changed = progress.changed();
        let mut progress_all = false;
        let mut progress_ids = foldhash::HashSet::default();
        match progress {
            Progress::Incomplete => mirror.progress = None,
            Progress::Same => {}
            Progress::Whole(rows) => {
                mirror.progress = Some(rows.into_iter().collect());
                progress_all = true;
            }
            Progress::Changed { rows, deleted } => {
                let held = mirror
                    .progress
                    .as_mut()
                    .ok_or_else(|| invalid("A land hour changed losses it never held"))?;
                let step = |last: u64, step: u64| {
                    last.checked_add(step)
                        .ok_or_else(|| invalid("A land hour's combats overflow"))
                };
                let mut combat = 0;
                for gone in deleted {
                    combat = step(combat, gone)?;
                    held.remove(&combat);
                    progress_ids.insert(combat);
                }
                combat = 0;
                for (next, loss) in rows {
                    combat = step(combat, next)?;
                    match loss {
                        Loss::Whole(inputs) => {
                            held.insert(combat, inputs);
                        }
                        Loss::Moved(moves) => {
                            let inputs = held.get_mut(&combat).ok_or_else(|| {
                                invalid("A land hour moved losses it never held")
                            })?;
                            for (side, moved) in inputs.iter_mut().zip(moves) {
                                side.apply(moved)?;
                            }
                        }
                    }
                    progress_ids.insert(combat);
                }
            }
        }
        let readable = progress_read && mirror.progress.is_some();

        let previous = self
            .deferred_through
            .or_else(|| self.observed.last().map(|(_, last)| *last));
        let adjacent = previous.and_then(|last| last.checked_add(1)) == Some(at);
        if previous.is_some() && !adjacent {
            self.settle_all(&mut mirror.stepping);
            mirror.stepping.lacking.clear();
            self.gap(previous.and_then(|last| last.checked_add(1)).unwrap_or(at))?;
        }
        match &combats {
            Combats::Unread => {
                mirror.forget_combats();
                self.settle_all(&mut mirror.stepping);
                mirror.stepping.lacking.clear();
                self.gap(at)?;
                self.project_results(branch, labels, results, mirror)?;
                self.reads.through(at);
                return Ok(());
            }
            Combats::Incomplete => {
                mirror.forget_combats();
                self.settle_all(&mut mirror.stepping);
                mirror.stepping.lacking.clear();
                self.materialize()?;
                self.warn(
                    at,
                    "LandCombats unavailable or incomplete; ending not recorded",
                );
                self.gap(at)?;
                self.project_results(branch, labels, results, mirror)?;
                self.reads.through(at);
                return Ok(());
            }
            Combats::Same | Combats::Changed { .. } if !mirror.held => {
                return Err(invalid("A land hour changed combats it never held"));
            }
            Combats::Same => {
                self.settle_all(&mut mirror.stepping);
                for row in mirror.combats.values_mut() {
                    row.offset -= 1;
                }
                if adjacent
                    && !links_changed
                    && !sides_changed
                    && self.open.len() == mirror.combats.len()
                {
                    self.advance(at);
                    if progress_changed {
                        let changes = (progress_all, readable, &progress_ids);
                        self.project_losses(at, changes, &[], mirror);
                    }
                    self.project_results(branch, labels, results, mirror)?;
                    self.reads.through(at);
                    return Ok(());
                }
            }
            Combats::Whole(_) => self.settle_all(&mut mirror.stepping),
            Combats::Changed { .. } => {}
        }
        self.materialize()?;
        self.reads.event(0, at, &at)?;
        if let Some((_, last)) = self
            .observed
            .last_mut()
            .filter(|(_, last)| last.checked_add(1) == Some(at))
        {
            *last = at;
        } else {
            self.observed.push((at, at));
        }

        // The combats the hour names, every one where any may have moved.
        let mut named: Vec<u64> = Vec::new();
        let mut all = !adjacent || links_changed || rebuild;
        let mut regrouped = all;
        let mut absent = Vec::new();
        match combats {
            Combats::Whole(rows) => {
                let ids: foldhash::HashSet<u64> = rows.iter().map(|combat| combat.id).collect();
                absent.extend(self.open.keys().filter(|id| !ids.contains(*id)).copied());
                mirror.combats = rows
                    .iter()
                    .map(|combat| (combat.id, Row::of(combat, at)))
                    .collect();
                mirror.held = true;
                mirror.orphans.clear();
                all = true;
                regrouped = true;
            }
            Combats::Changed { rows, deleted } => {
                // A row deleted and inserted again within the hour is read as it stands.
                let kept: foldhash::HashSet<u64> = rows.iter().map(|combat| combat.id).collect();
                for combat in deleted.into_iter().filter(|combat| !kept.contains(combat)) {
                    mirror.combats.remove(&combat);
                    mirror.orphans.remove(&combat);
                    if self.open.contains_key(&combat) {
                        absent.push(combat);
                    }
                    regrouped = true;
                }
                for combat in rows {
                    regrouped |= mirror
                        .combats
                        .insert(combat.id, Row::of(&combat, at))
                        .is_none();
                    named.push(combat.id);
                }
            }
            Combats::Same => all = true,
            Combats::Unread | Combats::Incomplete => unreachable!("handled before"),
        }
        if all {
            self.settle_all(&mut mirror.stepping);
        }
        for id in absent {
            let at_ordinal = self.open.remove(&id).expect("open battle");
            self.settle(&mut mirror.stepping, at_ordinal);
            mirror.stepping.lacking.remove(&id);
            self.dirty.insert(at_ordinal);
            held(&mut self.episodes, at_ordinal).end = Some((
                at,
                BattleEnd::Moment(BattleMoment {
                    hour: at,
                    bound: BattleBound::Exact,
                }),
            ));
        }
        if regrouped || sides_changed {
            self.live_warnings = self.combat_warnings(mirror, labels);
        }
        for (reason, count) in &self.live_warnings {
            self.warnings.add(at, reason.as_str(), *count);
        }

        let mut read: Vec<u64> = if all {
            mirror.combats.keys().copied().collect()
        } else {
            named.extend(
                side_changes
                    .iter()
                    .filter(|&&combat| mirror.combats.contains_key(&combat)),
            );
            named.extend(mirror.orphans.iter().copied());
            named
        };
        read.sort_unstable();
        read.dedup();
        if !all {
            // A stepping battle the hour reads holds its run through the hour before.
            for combat in &read {
                if let Some(&ordinal) = self.open.get(combat) {
                    self.settle(&mut mirror.stepping, ordinal);
                }
            }
            // A battle read in the hour before starts to step where the row census would extend
            // its run; checked once, it goes on until something reads it.
            let candidates: Vec<(u64, usize)> = if mirror.stepping.all {
                self.open
                    .iter()
                    .map(|(&combat, &at)| (combat, at))
                    .collect()
            } else {
                std::mem::take(&mut mirror.stepping.fresh)
            };
            let none = unrecorded();
            let mut unstepped = Vec::new();
            for (combat, ordinal) in candidates {
                if self.open.get(&combat) != Some(&ordinal)
                    || mirror.stepping.members.contains(&ordinal)
                    || read.binary_search(&combat).is_ok()
                {
                    continue;
                }
                let goes_on = mirror.combats.get(&combat).is_some_and(|row| {
                    let row = row.at(combat, at);
                    let episode = &self.episodes[&ordinal];
                    let (countries, unknown) = match self.live_sides.get(&combat) {
                        Some(side) => (&side.countries, side.unknown),
                        None => (&none.0, none.1),
                    };
                    episode.province == row.province
                        && episode.runs.last().is_some_and(|run| {
                            run.last_hour.checked_add(1) == Some(at)
                                && run.counts == row.counts
                                && run.countries == *countries
                                && run.countries_unknown == unknown
                                && run.duration_at(at) == i64::from(row.duration)
                        })
                });
                if goes_on {
                    mirror.stepping.members.insert(ordinal);
                } else if mirror.combats.contains_key(&combat) {
                    unstepped.push(combat);
                }
            }
            read.extend(unstepped);
            read.sort_unstable();
            read.dedup();
        }
        let mut opened = Vec::new();
        // Read in place: a combat's participants are copied only into a run they start.
        let live_sides = std::mem::take(&mut self.live_sides);
        let none = unrecorded();
        for &combat in &read {
            let row = mirror.combats[&combat].at(combat, at);
            let (countries, unknown) = live_sides
                .get(&combat)
                .map_or((&none.0, none.1), |side| (&side.countries, side.unknown));
            self.read_combat(branch, at, adjacent, row, countries, unknown, &mut opened);
        }
        self.live_sides = live_sides;
        mirror.stepping.fresh = read
            .iter()
            .filter_map(|combat| self.open.get(combat).map(|&ordinal| (*combat, ordinal)))
            .collect();
        mirror.stepping.all = false;
        mirror.stepping.through = at;
        mirror.orphans.clear();
        if progress_changed {
            let changes = (progress_all, readable, &progress_ids);
            self.project_losses(at, changes, &opened, mirror);
        } else if !opened.is_empty() {
            let open: Vec<_> = opened
                .iter()
                .filter_map(|combat| self.open.get(combat).map(|at| (*combat, *at)))
                .collect();
            self.read_losses_of(at, open, readable, mirror);
        }
        self.project_results(branch, labels, results, mirror)?;
        self.reads.through(at);
        Ok(())
    }

    /// Each reason the combats' participants warn of, and how many combats warn of it.
    fn combat_warnings(&self, mirror: &Mirror, labels: bool) -> BTreeMap<String, usize> {
        let unrecorded = LiveSide::new(None, &self.links, labels);
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for combat in mirror.combats.keys() {
            let side = self.live_sides.get(combat).unwrap_or(&unrecorded);
            for (reason, count) in &side.warnings {
                *counts.entry(reason.as_str()).or_default() += count;
            }
        }
        counts
            .into_iter()
            .map(|(reason, count)| (reason.to_owned(), count))
            .collect()
    }

    /// The hourly counters of every open battle whose counters may have changed: those whose
    /// row changed, those holding none, and those `opened` this hour. `changes` is whether the
    /// table was read whole, whether it is readable, and the combats whose rows changed.
    fn project_losses(
        &mut self,
        at: i32,
        (whole, readable, changed): (bool, bool, &foldhash::HashSet<u64>),
        opened: &[u64],
        mirror: &mut Mirror,
    ) {
        let open: Vec<_> = if whole || !readable {
            mirror.stepping.lacking.clear();
            self.open
                .iter()
                .map(|(&combat, &at)| (combat, at))
                .collect()
        } else {
            changed
                .iter()
                .chain(&mirror.stepping.lacking)
                .chain(opened)
                .filter_map(|combat| self.open.get(combat).map(|&at| (*combat, at)))
                .collect()
        };
        self.read_losses_of(at, open, readable, mirror);
    }

    /// The hourly counters of `open`'s battles, in combat order as the row census reads them: a
    /// design's first read numbers it.
    fn read_losses_of(
        &mut self,
        at: i32,
        mut open: Vec<(u64, usize)>,
        readable: bool,
        mirror: &mut Mirror,
    ) {
        open.sort_unstable();
        open.dedup();
        let held = mirror.progress.as_ref().filter(|_| readable);
        self.read_losses(at, &open, |combat| {
            held.and_then(|rows| rows.get(&combat).map(std::borrow::Cow::Borrowed))
        });
        for (combat, ordinal) in open {
            if self.episodes[&ordinal].loss_inputs.is_none() {
                mirror.stepping.lacking.insert(combat);
            } else {
                mirror.stepping.lacking.remove(&combat);
            }
        }
    }

    fn project_results(
        &mut self,
        branch: u64,
        labels: bool,
        results: Option<Box<facts::Results>>,
        mirror: &mut Mirror,
    ) -> io::Result<()> {
        let Some(results) = results else {
            return Ok(());
        };
        // A finalizer reads the newest battle of its combat where its run ends; one it does not
        // end goes on stepping.
        let mut touched = Vec::new();
        for row in &results.rows {
            if let Some(&ordinal) = self.latest.get(&row.combat)
                && mirror.stepping.members.contains(&ordinal)
            {
                if let Some(run) = self
                    .episodes
                    .get_mut(&ordinal)
                    .and_then(|episode| episode.runs.last_mut())
                {
                    run.last_hour = mirror.stepping.through;
                }
                touched.push((row.combat, ordinal));
            }
        }
        let ended = self.read_results(
            branch,
            labels,
            (results.from_hour, results.to_hour),
            &results.rows,
        )?;
        for (combat, ordinal) in touched {
            if self.open.get(&combat) != Some(&ordinal) {
                mirror.stepping.members.remove(&ordinal);
            }
        }
        for combat in ended {
            mirror.stepping.lacking.remove(&combat);
            if mirror.combats.contains_key(&combat) {
                mirror.orphans.insert(combat);
            }
        }
        Ok(())
    }
}

/// What a branch's emit saves: its census, the battles changed since its last publication,
/// and the next block it publishes.
#[derive(Serialize, Deserialize)]
struct Saved<C> {
    census: C,
    dirty: Vec<usize>,
    next: usize,
}

/// `census`, its stepping runs extended, as [`Emit::save`] writes it.
fn save(census: &mut Census, mirror: &mut Mirror, next: usize) -> io::Result<Vec<u8>> {
    census.settle_all(&mut mirror.stepping);
    let saved = Saved {
        census: &*census,
        dirty: census.dirty.iter().copied().collect(),
        next,
    };
    dashboard_base::prepared::series::compress(&serde_json::to_vec(&saved)?, 3)
}

/// The census and next block [`save`] wrote, at `format`, naming designs from `catalogue`.
fn load(
    format: FormatVersion,
    catalogue: Option<Arc<Catalogue>>,
    saved: &[u8],
) -> io::Result<(Census, usize)> {
    let bytes = dashboard_base::prepared::series::decompress(saved)?;
    let Saved {
        mut census,
        dirty,
        next,
    }: Saved<Census> = serde_json::from_slice(&bytes)?;
    census.format = format;
    census.configured = true;
    census.catalogue = catalogue;
    census.dirty = dirty.into_iter().collect();
    Ok((census, next))
}

/// The family's emit on one branch.
pub struct Emit {
    census: Census,
    branch: u64,
    windows: Vec<BattleWindow>,
    /// The next block it publishes.
    next: usize,
    mirror: Mirror,
}

impl Emit {
    /// A branch's emit, naming designs from `catalogue` and warning `warning` where it cannot.
    pub fn new(
        replay: &Replay,
        branch: u64,
        catalogue: Option<Arc<Catalogue>>,
        warning: Option<String>,
    ) -> Self {
        let mut census = Census::new(replay.format);
        census.configured = true;
        census.catalogue = catalogue;
        census.content_warnings = warning.into_iter().collect();
        Self {
            census,
            branch,
            windows: windows(replay, branch),
            next: 0,
            mirror: Mirror::default(),
        }
    }

    /// Keeps one projected hour: every block ending before it is published first ([`Self::due`]).
    pub fn observe(&mut self, hour: facts::Hour) -> io::Result<()> {
        if self.due(hour.hour) {
            return Err(invalid("A land hour follows a block not published yet"));
        }
        self.census
            .observe_projected(self.branch, hour, &mut self.mirror)
    }

    /// Whether a block ending before `before` is not published yet.
    pub fn due(&self, before: i32) -> bool {
        self.windows
            .get(self.next)
            .is_some_and(|window| window.last_hour < before)
    }

    /// Publishes every block ending before `before`; every one left once the branch ends.
    pub fn publish(
        &mut self,
        writer: &mut store::Writer,
        before: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        while self.due(before) {
            self.census.settle_all(&mut self.mirror.stepping);
            // An open battle's run went on through the block, which its pages hold.
            self.census.dirty.extend(self.census.open.values().copied());
            self.census.publish_block_in(
                writer,
                &self.windows,
                self.branch,
                self.next,
                cancelled,
            )?;
            self.next += 1;
        }
        Ok(())
    }

    /// What it goes on with after the blocks it published.
    pub fn save(&mut self) -> io::Result<Vec<u8>> {
        save(&mut self.census, &mut self.mirror, self.next)
    }

    /// The emit [`Self::save`] wrote, naming designs as [`Self::new`] does.
    pub fn load(
        replay: &Replay,
        branch: u64,
        catalogue: Option<Arc<Catalogue>>,
        format: FormatVersion,
        saved: &[u8],
    ) -> io::Result<Self> {
        let (census, next) = load(format, catalogue, saved)?;
        Ok(Self {
            census,
            branch,
            windows: windows(replay, branch),
            next,
            mirror: Mirror::default(),
        })
    }
}

#[cfg(test)]
mod tests;
