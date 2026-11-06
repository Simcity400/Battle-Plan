//! The refits view: each owner's refit lines, the ships refitting on them, and its refit totals.
//! A link names one ship's line; a line is projected for each owner whose ships it holds.
use super::*;
use dashboard_base::products::finite_fixed64;

const LINE: usize = line::CELLS - 2;
const TOTALS: usize = refit_totals::CELLS - 2;

fn at(cell: usize) -> usize {
    cell - 2
}

/// A ship's link to its line, as the hour resolves it.
struct Link {
    line: u64,
    /// Its ship's task force, where its ship is recorded.
    task: Option<u64>,
    owner: Option<u32>,
}

/// One owner's line.
pub(super) struct Line {
    cells: Box<[Spelled]>,
    refusals: Vec<(u32, u64)>,
    /// The ships it holds as live, ascending.
    ships: Vec<u64>,
    /// Whether its progress is recorded: it is live and counts in the owner's totals.
    live: bool,
    /// Its daily cost and allocation, for the totals: none where not recorded.
    dockyards: Option<i128>,
    /// Its cost less its progress, for the totals: none where either is unusable.
    remaining: Option<i128>,
}

#[derive(Default)]
pub(super) struct Refits {
    links: HashMap<u64, Link>,
    /// The linked ships of each line handle, and of each task force.
    by_line: HashMap<u64, Vec<u64>>,
    by_task: HashMap<u64, Vec<u64>>,
    lines: HashMap<(u32, u64), Line>,
    /// Each owner's line handles, and each line handle's owners.
    owned: HashMap<u32, BTreeSet<u64>>,
    owners: HashMap<u64, BTreeSet<u32>>,
    /// The line handles each target variant names.
    targets: HashMap<u64, Vec<u64>>,
    /// Each line handle's target variant, where recorded.
    target: HashMap<u64, u64>,
    /// The links counted live or unknown whose owner does not resolve, by line handle.
    unowned: HashMap<u64, usize>,
    /// Whether the census was complete as last read.
    complete: bool,
    refused: HashMap<u32, Vec<(u32, u64)>>,
}

impl Refits {
    pub(super) fn build<F: Tables + ?Sized>(
        cx: &mut Cx<'_, '_, F>,
        ships: &mut Ships,
        dirty: &mut Dirty,
    ) -> Self {
        let mut refits = Self::default();
        let frame = cx.frame;
        let links: Vec<(u64, u64)> = frame
            .complete::<tables::NavalRefitShips>()
            .map(|rows| frame.each(rows.iter()))
            .unwrap_or_default()
            .iter()
            .filter(|link| link.line != 0)
            .map(|link| (link.ship, link.line))
            .collect();
        let mut lines = BTreeSet::new();
        for (ship, line) in links {
            refits.link(cx, ship, Some(line));
            lines.insert(line);
        }
        for line in lines {
            refits.line(cx, ships, dirty, line);
        }
        refits.complete = refits.census(cx);
        refits
    }

    /// The link of `ship`, now to `line`, resolved again; the lines it left and joined.
    fn link<F: Tables + ?Sized>(
        &mut self,
        cx: &mut Cx<'_, '_, F>,
        ship: u64,
        line: Option<u64>,
    ) -> [Option<u64>; 2] {
        let old = self.links.remove(&ship);
        if let Some(old) = &old {
            remove(&mut self.by_line, old.line, ship);
            if let Some(task) = old.task {
                remove(&mut self.by_task, task, ship);
            }
        }
        let frame = cx.frame;
        let task = frame.get::<tables::Ships>(ship).map(|row| row.task_force);
        let now = line.map(|line| {
            let owner = task
                .and_then(|task| frame.get::<tables::TaskForces>(task))
                .and_then(|task| cx.owner(task.roster_owner));
            Link { line, task, owner }
        });
        if let Some(now) = &now {
            self.by_line.entry(now.line).or_default().push(ship);
            if let Some(task) = now.task {
                self.by_task.entry(task).or_default().push(ship);
            }
        }
        let lines = [old.map(|link| link.line), now.as_ref().map(|link| link.line)];
        if let Some(now) = now {
            self.links.insert(ship, now);
        }
        lines
    }

    /// Every owner's line of handle `handle` projected again, and what that changes.
    fn line<F: Tables + ?Sized>(
        &mut self,
        cx: &mut Cx<'_, '_, F>,
        ships: &mut Ships,
        dirty: &mut Dirty,
        handle: u64,
    ) {
        let frame = cx.frame;
        let progress = frame.complete::<tables::NavalRefitProgress>();
        let amount = progress.and_then(|rows| frame.found(rows.get(u128::from(handle))));
        let live = progress.map(|_| amount.is_some());
        let metadata = frame.get::<tables::ProductionLines>(handle);
        let (amount, metadata) = (amount.as_deref(), metadata.as_deref());
        // A stale carried link is not an active or queued ship row.
        let mut owners: BTreeMap<u32, Vec<u64>> = BTreeMap::new();
        let mut unowned = 0;
        if live != Some(false) {
            for ship in self.by_line.get(&handle).into_iter().flatten() {
                match self.links[ship].owner {
                    Some(owner) => owners.entry(owner).or_default().push(*ship),
                    None => unowned += 1,
                }
            }
        }
        if unowned == 0 {
            self.unowned.remove(&handle);
        } else {
            self.unowned.insert(handle, unowned);
        }
        if let Some(variant) = self.target.remove(&handle) {
            remove(&mut self.targets, variant, handle);
        }
        if let Some(line) = metadata.filter(|line| line.variant != 0) {
            self.target.insert(handle, line.variant);
            self.targets.entry(line.variant).or_default().push(handle);
        }
        let mut every = self.owners.remove(&handle).unwrap_or_default();
        every.extend(owners.keys().copied());
        for owner in every {
            let old = self.lines.remove(&(owner, handle));
            let now = owners.remove(&owner).map(|mut linked| {
                linked.sort_unstable();
                project(cx, (amount, metadata), linked)
            });
            cx.refusals.shift(dirty, owner, refused(&old), refused(&now));
            dirty.place(owner, layout::HOLDERS, handle);
            dirty.totals(owner);
            let ships_of = |line: &Option<Line>| line.as_ref().map(|line| (owner, line.ships.clone()));
            ships.rehold(cx, dirty, handle, ships_of(&old), ships_of(&now));
            match now {
                Some(now) => {
                    self.owned.entry(owner).or_default().insert(handle);
                    self.owners.entry(handle).or_default().insert(owner);
                    self.lines.insert((owner, handle), now);
                }
                None => {
                    if let Some(owned) = self.owned.get_mut(&owner) {
                        owned.remove(&handle);
                        if owned.is_empty() {
                            self.owned.remove(&owner);
                        }
                    }
                }
            }
        }
    }

    /// Whether the census of live refits is complete.
    fn census<F: Tables + ?Sized>(&self, cx: &Cx<'_, '_, F>) -> bool {
        let frame = cx.frame;
        let progress = frame.complete::<tables::NavalRefitProgress>();
        let recorded = progress.is_some_and(|rows| rows.is_empty())
            || (progress.is_some()
                && frame.complete::<tables::NavalRefitShips>().is_some()
                && frame.complete::<tables::Ships>().is_some()
                && frame.complete::<tables::TaskForces>().is_some());
        recorded && self.unowned.is_empty()
    }

    /// The hour's changes to the lines, through `touched`; `variants` those whose names changed.
    pub(super) fn hour<F: Tables + ?Sized>(
        &mut self,
        cx: &mut Cx<'_, '_, F>,
        ships: &mut Ships,
        dirty: &mut Dirty,
        touched: &Touched,
        variants: &[u64],
    ) {
        let mut lines = BTreeSet::new();
        let frame = cx.frame;
        let relinked: BTreeSet<u64> = keys(touched.of(TableId::NavalRefitShips))
            .map(|key| key as u64)
            .collect();
        // A ship's owner reads its task force, and the task force's roster.
        let mut resolved: BTreeSet<u64> = touched
            .of(TableId::Ships)
            .written
            .iter()
            .filter(|&&(_, mask)| mask != cx.strength)
            .map(|&(key, _)| key as u64)
            .chain(touched.of(TableId::Ships).deleted.iter().map(|&key| key as u64))
            .filter(|ship| self.links.contains_key(ship))
            .collect();
        for task in keys(touched.of(TableId::TaskForces)) {
            resolved.extend(self.by_task.get(&(task as u64)).into_iter().flatten());
        }
        for ship in relinked.iter().chain(resolved.difference(&relinked)) {
            let line = if relinked.contains(ship) {
                frame
                    .get::<tables::NavalRefitShips>(*ship)
                    .map(|link| link.line)
                    .filter(|&line| line != 0)
            } else {
                self.links.get(ship).map(|link| link.line)
            };
            lines.extend(self.link(cx, *ship, line).into_iter().flatten());
        }
        for table in [TableId::NavalRefitProgress, TableId::ProductionLines] {
            lines.extend(keys(touched.of(table)).map(|key| key as u64));
        }
        for variant in variants {
            lines.extend(self.targets.get(variant).into_iter().flatten());
        }
        for handle in lines {
            self.line(cx, ships, dirty, handle);
        }
        let complete = self.census(cx);
        if complete != self.complete {
            self.complete = complete;
            dirty.totals_all = true;
        }
    }

    /// `owner`'s totals row at the hour, and the refusals it counts there.
    pub(super) fn totals<F: Tables + ?Sized>(
        &mut self,
        cx: &mut Cx<'_, '_, F>,
        dirty: &mut Dirty,
        owner: u32,
    ) -> Option<Box<[Spelled]>> {
        let mut cells = vec![Spelled::Null; TOTALS];
        let mut refused = Vec::new();
        cells[at(refit_totals::CENSUS_COMPLETE)] = Spelled::Integer(i128::from(self.complete));
        if self.complete {
            let (mut ships, mut lines) = (0, 0);
            let (mut dockyards, mut remaining) = (Some(0_i128), Some(0_i128));
            for handle in self.owned.get(&owner).into_iter().flatten() {
                let line = &self.lines[&(owner, *handle)];
                // Every line contributes once, irrespective of its ship multiplicity.
                if !line.live {
                    continue;
                }
                ships += line.ships.len();
                lines += 1;
                dockyards = dockyards.zip(line.dockyards).map(|(sum, line)| sum + line);
                remaining = remaining.zip(line.remaining).map(|(sum, line)| sum + line);
            }
            cells[at(refit_totals::SHIPS)] = Spelled::Integer(ships as i128);
            cells[at(refit_totals::LINES)] = Spelled::Integer(lines as i128);
            if let Some(dockyards) = dockyards {
                cells[at(refit_totals::DOCKYARDS)] = Spelled::Integer(dockyards);
            }
            if let Some(remaining) = remaining {
                cells[at(refit_totals::REMAINING)] = Spelled::Integer(remaining);
            }
        } else {
            refused.push((
                cx.reasons.number(
                    "Active refit count unavailable: incomplete hourly progress, links, ships or ownership census",
                ),
                1,
            ));
        }
        let old = self.refused.remove(&owner).unwrap_or_default();
        cx.refusals.shift(dirty, owner, &old, &refused);
        if !refused.is_empty() {
            self.refused.insert(owner, refused);
        }
        Some(cells.into_boxed_slice())
    }

    pub(super) fn holder(&self, owner: u32, key: u64) -> Option<&[Spelled]> {
        self.lines
            .get(&(owner, key))
            .filter(|line| line.cells.iter().any(|cell| *cell != Spelled::Null))
            .map(|line| &*line.cells)
    }
}

/// The refusals `line` counts, none where there is none.
fn refused(line: &Option<Line>) -> &[(u32, u64)] {
    line.as_ref().map_or(&[], |line| &line.refusals)
}

/// One owner's line of `progress` and `metadata`, holding its `linked` ships, ascending.
fn project<F: Tables + ?Sized>(
    cx: &mut Cx<'_, '_, F>,
    (progress, metadata): (Option<&rows::NavalRefitProgressRow>, Option<&rows::ProductionLineRow>),
    linked: Vec<u64>,
) -> Line {
    let mut cells = vec![Spelled::Null; LINE];
    let mut refusals = Vec::new();
    if let Some(amount) = progress {
        cells[at(line::LIVE)] = Spelled::Integer(1);
        cells[at(line::PROGRESS)] = Spelled::Integer(i128::from(amount.progress_raw));
    }
    let (mut dockyards, mut remaining) = (None, None);
    // Metadata is retained even when the hourly liveness gate is unknown.
    match metadata {
        Some(production) => {
            cells[at(line::COST)] = Spelled::Integer(i128::from(production.cost_raw));
            cells[at(line::DOCKYARDS)] =
                Spelled::Integer(i128::from(production.active_factories));
            dockyards = Some(i128::from(production.active_factories));
            cells[at(line::TARGET)] = Spelled::Integer(i128::from(production.variant));
            if production.variant != 0 {
                match cx.shared.design(&mut cx.naming, production.variant) {
                    Ok(label) => cells[at(line::TARGET_NAME)] = Spelled::Token(label),
                    Err(why) => refusals.push((cx.reasons.number(&why), 1)),
                }
            }
            if let Some(amount) = progress {
                if production.cost_raw > 0
                    && finite_fixed64(production.cost_raw)
                    && finite_fixed64(amount.progress_raw)
                {
                    let left = i128::from(production.cost_raw) - i128::from(amount.progress_raw);
                    cells[at(line::REMAINING)] = Spelled::Integer(left);
                    remaining = Some(left);
                } else {
                    refusals.push((
                        cx.reasons.number(
                            "Refit ratio and remaining cost unavailable: nonpositive or nonfinite last recorded cost or progress",
                        ),
                        1,
                    ));
                }
            }
        }
        None => refusals.push((
            cx.reasons
                .number("Refit daily target, cost and allocation not recorded"),
            1,
        )),
    }
    Line {
        cells: cells.into_boxed_slice(),
        refusals: ship::merged(refusals),
        ships: if progress.is_some() { linked } else { Vec::new() },
        live: progress.is_some(),
        dockyards,
        remaining,
    }
}
