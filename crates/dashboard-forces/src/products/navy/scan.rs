use super::*;
use dashboard_base::prepared::frame_changes::field_mask;
use dashboard_base::products::{
    country_history::observe_identity,
    force_census::{Countries, changed_ids},
};
use dashboard_text::metadata::validate_campaign;
use replay::{
    container::Replay,
    frame::{DecodedFrame, Frame, FrameDecoder, ViewIdentity},
    rows::{ShipRow, TableId, tables},
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Ship {
    class: u32,
    owners: [Option<u32>; 2],
    hp: i64,
    maximum: Option<i64>,
}

#[derive(Clone)]
struct Summary {
    count: i64,
    classes: Arc<BTreeMap<u32, i64>>,
    hp: Option<i64>,
    hp_classes: Arc<BTreeMap<u32, i64>>,
    maximum: Option<i64>,
    refused: bool,
    maximum_overflow: bool,
}

#[derive(Default)]
struct Totals {
    // Wider sums permit exact subtraction after an overflowing native read.
    classes: BTreeMap<u32, (i64, i128)>,
    count: i64,
    hp: i128,
    negative: i64,
    maximum: i128,
    maximums: BTreeMap<u64, i64>,
    missing: BTreeSet<u64>,
    count_snapshot: Option<Arc<BTreeMap<u32, i64>>>,
    snapshot: Option<Summary>,
}

impl Totals {
    fn count(&mut self, ship: Ship, sign: i64) {
        self.count += sign;
        self.classes.entry(ship.class).or_default().0 += sign;
        self.count_snapshot = None;
    }

    fn strength(&mut self, ship: Ship, sign: i64) {
        self.hp += i128::from(sign) * i128::from(ship.hp);
        self.negative += sign * i64::from(ship.hp < 0);
        self.classes.entry(ship.class).or_default().1 += i128::from(sign) * i128::from(ship.hp);
        self.snapshot = None;
    }

    fn maximum(&mut self, id: u64, maximum: Option<i64>, sign: i64) {
        if let Some(maximum) = maximum {
            self.maximum += i128::from(sign) * i128::from(maximum);
            if sign > 0 {
                self.maximums.insert(id, maximum);
            } else {
                self.maximums.remove(&id);
            }
        } else if sign > 0 {
            self.missing.insert(id);
        } else {
            self.missing.remove(&id);
        }
        self.snapshot = None;
    }

    fn summary(&mut self, empty: &Arc<BTreeMap<u32, i64>>) -> Summary {
        if let Some(summary) = &self.snapshot {
            return summary.clone();
        }
        self.classes.retain(|_, (count, _)| *count != 0);
        let classes = Arc::clone(self.count_snapshot.get_or_insert_with(|| {
            Arc::new(
                self.classes
                    .iter()
                    .map(|(&class, &(count, _))| (class, count))
                    .collect(),
            )
        }));
        // Task-force aggregation is signed i64; negative HP is malformed
        // because stored strength is clamped. Refuse locally.
        let hp = (self.negative == 0)
            .then(|| i64::try_from(self.hp).ok())
            .flatten();
        let hp_classes = match hp {
            Some(_) => Arc::new(
                self.classes
                    .iter()
                    .map(|(&class, &(_, hp))| {
                        (
                            class,
                            i64::try_from(hp).expect("nonnegative class HP fits total"),
                        )
                    })
                    .collect(),
            ),
            None => Arc::clone(empty),
        };
        let maximum = self
            .missing
            .is_empty()
            .then(|| i64::try_from(self.maximum).ok())
            .flatten();
        // Preserve the old ordered fold's warning: after the first missing
        // counter it cannot overflow. Only a pathological large sum needs this walk.
        let maximum_overflow = self.maximum > i128::from(i64::MAX)
            && self.missing.first().is_none_or(|first| {
                self.maximums
                    .range(..*first)
                    .map(|(_, &hp)| i128::from(hp))
                    .sum::<i128>()
                    > i128::from(i64::MAX)
            });
        let summary = Summary {
            count: self.count,
            classes,
            hp,
            hp_classes,
            maximum,
            refused: hp.is_none(),
            maximum_overflow,
        };
        self.snapshot = Some(summary.clone());
        summary
    }
}

#[derive(Default)]
struct Census {
    ships: HashMap<u64, Ship>,
    dirty: BTreeSet<u32>,
    totals: [HashMap<u32, Totals>; 2],
    unattributed: [usize; 2],
    forces: HashMap<u64, [Option<u32>; 2]>,
    force_view: Option<ViewIdentity>,
    lexical_view: Option<ViewIdentity>,
    lexical_complete: Option<bool>,
    models_complete: Option<bool>,
    class_keys: HashMap<u32, u32>,
    class_ids: HashMap<ClassKey, u32>,
    ready: bool,
    #[cfg(test)]
    updated: usize,
}

impl Census {
    fn change(&mut self, id: u64, before: Option<Ship>, after: Option<Ship>) {
        for owner in 0..2 {
            let same_owner = before
                .zip(after)
                .is_some_and(|(old, new)| old.owners[owner] == new.owners[owner]);
            let same_class = before
                .zip(after)
                .is_some_and(|(old, new)| old.class == new.class);
            let same_maximum = before
                .zip(after)
                .is_some_and(|(old, new)| old.maximum == new.maximum);
            for (ship, sign) in [(before, -1), (after, 1)] {
                let Some(ship) = ship else {
                    continue;
                };
                if let Some(slot) = ship.owners[owner] {
                    self.dirty.insert(slot);
                    let totals = self.totals[owner].entry(slot).or_default();
                    if !(same_owner && same_class) {
                        totals.count(ship, sign);
                    }
                    totals.strength(ship, sign);
                    if !(same_owner && same_maximum) {
                        totals.maximum(id, ship.maximum, sign);
                    }
                } else if sign > 0 {
                    self.unattributed[owner] += 1;
                } else {
                    self.unattributed[owner] -= 1;
                }
            }
            for slot in before
                .into_iter()
                .chain(after)
                .filter_map(|ship| ship.owners[owner])
            {
                if self.totals[owner]
                    .get(&slot)
                    .is_some_and(|total| total.count == 0)
                {
                    self.totals[owner].remove(&slot);
                }
            }
        }
    }

    fn ship(
        &mut self,
        row: &ShipRow,
        maximum: Option<i64>,
        decoded: &DecodedFrame,
        result: &mut ScannedNavy,
    ) -> io::Result<Ship> {
        let token = row.class_id as u32;
        let class = match self.class_keys.entry(token) {
            std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                // The serialized authored-name token supplies the dated spelling.
                let spelling = decoded
                    .complete::<tables::LexicalTokens>()
                    .ok()
                    .and_then(|rows| {
                        rows.binary_search_by_key(&token, |row| row.token)
                            .ok()
                            .and_then(|index| {
                                std::str::from_utf8(&rows[index].literal)
                                    .ok()
                                    .map(str::to_owned)
                            })
                    });
                let id = match self.class_ids.entry((token, spelling)) {
                    std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let id = u32::try_from(result.classes.len()).map_err(|_| {
                            io::Error::other("Navy class identity index exceeds u32")
                        })?;
                        Arc::make_mut(&mut result.classes).push(entry.key().clone());
                        entry.insert(id);
                        id
                    }
                };
                entry.insert(id);
                id
            }
        };
        Ok(Ship {
            class,
            owners: self
                .forces
                .get(&row.task_force)
                .copied()
                .unwrap_or([None; 2]),
            hp: row.strength,
            maximum,
        })
    }

    fn update(
        &mut self,
        decoded: &DecodedFrame,
        frame: &Frame,
        countries: &Countries,
        countries_changed: bool,
        result: &mut ScannedNavy,
        cancelled: &AtomicBool,
    ) -> io::Result<[bool; 2]> {
        self.dirty.clear();
        let ships = decoded.complete::<tables::Ships>();
        let forces = decoded.complete::<tables::TaskForces>();
        let Ok(ships) = ships else {
            self.ready = false;
            result.unread += 1;
            return Ok([false; 2]);
        };
        if !ships.is_empty() && forces.is_err() {
            self.ready = false;
            result.unread += 1;
            return Ok([false; 2]);
        }
        let force_view = &decoded.tables[TableId::TaskForces.index()];
        let mut owners_changed = countries_changed;
        if countries_changed
            || self
                .force_view
                .as_ref()
                .is_none_or(|old| !old.is(force_view))
        {
            // Roster owner, logical owner and expeditionary sender are separate
            // recorded links; the sender is never an inferred host.
            let current: HashMap<_, _> = forces
                .unwrap_or_default()
                .iter()
                .map(|force| {
                    (
                        force.id,
                        [force.roster_owner, force.logical_owner].map(|link| countries.owner(link)),
                    )
                })
                .collect();
            owners_changed |= current != self.forces;
            self.forces = current;
            self.force_view = Some(force_view.identity());
        }
        let lexical_view = &decoded.tables[TableId::LexicalTokens.index()];
        let lexical_complete = decoded.complete::<tables::LexicalTokens>().is_ok();
        let names_changed = self.lexical_complete != Some(lexical_complete)
            || (lexical_complete
                && self
                    .lexical_view
                    .as_ref()
                    .is_none_or(|old| !old.is(lexical_view)));
        self.lexical_complete = Some(lexical_complete);
        if names_changed {
            self.class_keys.clear();
        }
        self.lexical_view = Some(lexical_view.identity());
        let models = decoded.complete::<tables::ShipModels>().ok();
        let evidence_changed = self.models_complete != Some(models.is_some());
        self.models_complete = Some(models.is_some());
        let mut ids = Vec::new();
        static FIELDS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        let fields = *FIELDS
            .get_or_init(|| field_mask(TableId::Ships, &["task_force", "strength", "class_id"]));
        let sparse = self.ready
            && !owners_changed
            && !names_changed
            && !evidence_changed
            && changed_ids(&frame.tables[TableId::Ships.index()], 0, &mut ids, |row| {
                row.mask & fields != 0 || !self.ships.contains_key(&(row.key as u64))
            })
            && (models.is_none()
                || changed_ids(
                    &frame.tables[TableId::ShipModels.index()],
                    0,
                    &mut ids,
                    |_| true,
                ));
        let models = models.unwrap_or_default();
        // Maximum strength clamps the model word to 100 (fixed5).
        if sparse {
            let dense = ids.len() >= ships.len();
            let mut ship_index = 0;
            let mut model_index = 0;
            for (index, id) in ids.into_iter().enumerate() {
                if index.is_multiple_of(4096) {
                    check_cancel(cancelled)?;
                }
                let row = if dense {
                    while ship_index < ships.len() && ships[ship_index].id < id {
                        ship_index += 1;
                    }
                    ships.get(ship_index).filter(|ship| ship.id == id)
                } else {
                    ships
                        .binary_search_by_key(&id, |ship| ship.id)
                        .ok()
                        .map(|index| &ships[index])
                };
                let after = if let Some(row) = row {
                    let model = if dense {
                        while model_index < models.len() && models[model_index].ship < id {
                            model_index += 1;
                        }
                        models.get(model_index).filter(|model| model.ship == id)
                    } else {
                        models
                            .binary_search_by_key(&id, |model| model.ship)
                            .ok()
                            .map(|index| &models[index])
                    };
                    let maximum = model
                        .and_then(|model| model.counter.as_ref())
                        .map(|counter| counter.maximum_strength.max(100));
                    Some(self.ship(row, maximum, decoded, result)?)
                } else {
                    None
                };
                let before = self.ships.remove(&id);
                if before != after {
                    self.change(id, before, after);
                }
                if let Some(ship) = after {
                    self.ships.insert(id, ship);
                    #[cfg(test)]
                    {
                        self.updated += 1;
                    }
                }
            }
        } else {
            self.ships.clear();
            self.dirty
                .extend(self.totals.iter().flat_map(|totals| totals.keys()).copied());
            self.totals = Default::default();
            self.unattributed = [0; 2];
            // Full acquisitions use one sorted merge join, never Ships x Models.
            let mut models = models.iter().peekable();
            for (index, row) in ships.iter().enumerate() {
                if index.is_multiple_of(4096) {
                    check_cancel(cancelled)?;
                }
                while models.peek().is_some_and(|model| model.ship < row.id) {
                    models.next();
                }
                let maximum = models
                    .peek()
                    .filter(|model| model.ship == row.id)
                    .and_then(|model| model.counter.as_ref())
                    .map(|counter| counter.maximum_strength.max(100));
                let ship = self.ship(row, maximum, decoded, result)?;
                self.change(row.id, None, Some(ship));
                self.ships.insert(row.id, ship);
                #[cfg(test)]
                {
                    self.updated += 1;
                }
            }
        }
        self.ready = true;
        for (counter, count) in result.unattributed.iter_mut().zip(self.unattributed) {
            *counter += count;
        }
        Ok(self.unattributed.map(|count| count == 0))
    }
}

impl Track {
    fn advance(&mut self, hour: i32) {
        let Some((_, before)) = &mut self.last else {
            return;
        };
        if let Some(segment) = self.segments.last_mut() {
            segment.last_hour = hour;
        }
        self.last_identity.advance(hour);
        for runs in self.runs.iter_mut().chain(&mut self.strength_runs) {
            if let Some(run) = runs.last_mut().filter(|run| run.last_hour == *before) {
                run.last_hour = hour;
            }
        }
        *before = hour;
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Scan {
    pub(super) result: ScannedNavy,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    pub(super) tracks: HashMap<String, Track>,
    last_hour: Option<i32>,
    #[serde(skip)]
    pub(super) countries: Countries,
    #[serde(skip)]
    census: Census,
    live: BTreeSet<String>,
    #[serde(skip)]
    pub(super) observed: Vec<String>,
    #[serde(skip)]
    pub(super) removed: Vec<String>,
    #[serde(skip)]
    pub(super) applied: Option<i32>,
    #[serde(skip)]
    joined: bool,
    #[serde(skip)]
    slots: HashMap<u32, Vec<(String, Identity)>>,
    #[serde(skip)]
    previous_exact: Option<[bool; 2]>,
    #[serde(skip)]
    notes: HashMap<String, ([usize; 2], [usize; 2])>,
    #[serde(skip)]
    note_totals: ([usize; 2], [usize; 2]),
    #[serde(skip)]
    rates: [usize; 7],
    #[serde(skip)]
    undisplayable: Vec<Vec<u8>>,
    #[cfg(test)]
    #[serde(skip)]
    pub(super) work: dashboard_base::prepared::observer_tests::Work,
}
impl Scan {
    pub(super) fn rejoin(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if self.last_hour != Some(decoded.hour) || self.census.ready {
            return Ok(());
        }
        let tracks = std::mem::take(&mut self.tracks);
        let live = std::mem::take(&mut self.live);
        let result = self.result.clone();
        self.step_changes(frame, decoded, cancelled, None)?;
        self.tracks = tracks;
        self.live = live;
        self.result = result;
        Ok(())
    }
    pub(super) fn seal(&mut self) {
        if let Some(hour) = self.last_hour {
            for tag in &self.live {
                self.tracks
                    .get_mut(tag)
                    .expect("live navy track")
                    .advance(hour);
            }
        }
    }
    pub(super) fn step_changes(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        cancelled: &AtomicBool,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        self.observed.clear();
        self.removed.clear();
        self.joined = !matches!(self.countries.update_changed(decoded, changes), Ok(false));
        let unchanged = self
            .last_hour
            .is_some_and(|at| at.checked_add(1) == Some(decoded.hour))
            && !self.joined
            && changes.is_some_and(|changes| {
                ![
                    TableId::Ships,
                    TableId::ShipModels,
                    TableId::TaskForces,
                    TableId::LexicalTokens,
                ]
                .iter()
                .any(|id| changes.rows_changed[id.index()])
            });
        if unchanged {
            self.last_hour = Some(decoded.hour);
            self.result.unread += self.rates[0];
            for owner in 0..2 {
                self.result.unattributed[owner] += self.rates[1 + owner];
                self.result.refused_strength[owner] += self.rates[3 + owner];
                self.result.maximum_overflows[owner] += self.rates[5 + owner];
            }
            for tag in &self.undisplayable {
                *self
                    .result
                    .undisplayable_tags
                    .entry(tag.clone())
                    .or_default() += 1;
            }
            return Ok(());
        }
        let before = [
            self.result.unread,
            self.result.unattributed[0],
            self.result.unattributed[1],
            self.result.refused_strength[0],
            self.result.refused_strength[1],
            self.result.maximum_overflows[0],
            self.result.maximum_overflows[1],
        ];
        #[cfg(test)]
        {
            if let Some(changes) = changes {
                self.work.row_visits += <super::prepared::Navy as crate::products::parts_race_prepared::Family>::SAMPLE_TABLES.iter()
                    .filter(|id| changes.rows_changed[id.index()])
                    .map(|id| match &*decoded.tables[id.index()] { replay::frame::TableView::Complete(rows)
                        | replay::frame::TableView::Truncated { rows, .. } => rows.len(), _ => 0 }).sum::<usize>();
            }
        }
        self.step(frame, decoded, cancelled)?;
        self.applied = Some(decoded.hour);
        self.rates = [
            self.result.unread,
            self.result.unattributed[0],
            self.result.unattributed[1],
            self.result.refused_strength[0],
            self.result.refused_strength[1],
            self.result.maximum_overflows[0],
            self.result.maximum_overflows[1],
        ];
        for (rate, before) in self.rates.iter_mut().zip(before) {
            *rate -= before;
        }
        Ok(())
    }
    pub(super) fn step(
        &mut self,
        frame: &Frame,
        decoded: &DecodedFrame,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if self.census.class_ids.is_empty() && !self.result.classes.is_empty() {
            self.census.class_ids = self
                .result
                .classes
                .iter()
                .cloned()
                .enumerate()
                .map(|(id, key)| (key, id as u32))
                .collect();
        }
        let continuous = self
            .last_hour
            .is_some_and(|at| at.checked_add(1) == Some(decoded.hour));
        if !continuous {
            self.seal();
        }
        self.last_hour = Some(decoded.hour);
        let Self {
            result,
            tracks,
            countries,
            census,
            live,
            observed,
            removed,
            joined,
            slots,
            previous_exact,
            notes,
            note_totals,
            undisplayable,
            ..
        } = self;
        let empty = Arc::new(BTreeMap::new());
        let Ok(changed) = countries.update(decoded) else {
            result.unread += 1;
            if continuous {
                for tag in live.iter() {
                    tracks
                        .get_mut(tag)
                        .expect("live navy track")
                        .advance(decoded.hour - 1);
                }
            }
            census.ready = false;
            removed.extend(std::mem::take(live));
            slots.clear();
            *previous_exact = None;
            return Ok(());
        };
        let changed = changed || *joined || !continuous || slots.is_empty();
        let previous = continuous.then(|| decoded.hour - 1);
        if changed {
            // Materialise the previous live set before changing native membership.
            if let Some(hour) = previous {
                for tag in live.iter() {
                    tracks.get_mut(tag).expect("live navy track").advance(hour);
                }
            }
            slots.clear();
            undisplayable.clear();
            notes.clear();
            *note_totals = Default::default();
            let mut current = BTreeSet::new();
            for (tag, identity) in &countries.tags {
                if let Ok(tag) = std::str::from_utf8(tag) {
                    tracks.entry(tag.to_owned()).or_default();
                } else {
                    undisplayable.push(tag.clone());
                }
                if let (Ok(tag), Some(identity)) = (std::str::from_utf8(tag), *identity) {
                    current.insert(tag.to_owned());
                    slots
                        .entry(identity.0)
                        .or_default()
                        .push((tag.to_owned(), identity));
                }
            }
            removed.extend(live.difference(&current).cloned());
            *live = current;
        }
        let exact = census.update(decoded, frame, countries, changed, result, cancelled)?;
        let full = changed || *previous_exact != Some(exact);
        *previous_exact = Some(exact);
        let affected: Vec<_> = if full {
            slots.values().flatten().cloned().collect()
        } else {
            census
                .dirty
                .iter()
                .filter_map(|slot| slots.get(slot))
                .flatten()
                .cloned()
                .collect()
        };
        for tag in undisplayable.iter() {
            *result.undisplayable_tags.entry(tag.clone()).or_default() += 1;
        }
        for (tag, identity) in affected {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let tag = tag.as_str();
            let track = tracks.entry(tag.to_owned()).or_default();
            // An unchanged join extends a live track through the hour before.
            // Where the join changed, the tags live then were extended above;
            // a tag entering now was absent then, so its track (from before
            // it left) is not extended across the hours it was away.
            if let Some(hour) = previous.filter(|_| !changed) {
                track.advance(hour);
            }
            let mut after_notes = ([0; 2], [0; 2]);
            observe_identity(
                &mut track.segments,
                &mut track.last_identity,
                identity,
                frame.hour,
            );
            let identity_continuous = track.last.is_some_and(|(seen, at)| {
                seen == identity && at.checked_add(1) == Some(frame.hour)
            });
            for (owner, exact) in exact.into_iter().enumerate() {
                let summary = census.totals[owner].get_mut(&identity.0).map_or_else(
                    || Totals::default().summary(&empty),
                    |totals| totals.summary(&empty),
                );
                if census.ready {
                    after_notes.0[owner] = usize::from(summary.refused);
                    after_notes.1[owner] = usize::from(summary.maximum_overflow);
                }
                for (runs, total, classes, reference_total) in [
                    (
                        &mut track.runs[owner],
                        exact.then_some(summary.count),
                        summary.classes,
                        None,
                    ),
                    (
                        &mut track.strength_runs[owner],
                        exact.then_some(summary.hp).flatten(),
                        summary.hp_classes,
                        exact.then_some(summary.maximum).flatten(),
                    ),
                ] {
                    let starts_segment = !(identity_continuous
                        && total.is_some()
                        && runs.last().is_some_and(|run| run.total.is_some()));
                    let classes = if total.is_some() {
                        classes
                    } else {
                        Arc::clone(&empty)
                    };
                    match runs.last_mut() {
                        Some(run)
                            if identity_continuous
                                && run.last_hour.checked_add(1) == Some(frame.hour)
                                && run.total == total
                                && run.classes == classes
                                && run.reference_total == reference_total =>
                        {
                            run.last_hour = frame.hour;
                        }
                        _ => runs.push(RawNavyRun {
                            first_hour: frame.hour,
                            last_hour: frame.hour,
                            starts_segment,
                            total,
                            classes,
                            reference_total,
                        }),
                    }
                }
            }
            if track
                .runs
                .iter()
                .chain(&track.strength_runs)
                .any(|runs| runs.last().is_some_and(|run| run.first_hour == frame.hour))
            {
                observed.push(tag.to_owned());
            }
            let before_notes = notes
                .insert(tag.to_owned(), after_notes)
                .unwrap_or_default();
            for owner in 0..2 {
                note_totals.0[owner] =
                    note_totals.0[owner] + after_notes.0[owner] - before_notes.0[owner];
                note_totals.1[owner] =
                    note_totals.1[owner] + after_notes.1[owner] - before_notes.1[owner];
            }
            track.last = Some((identity, frame.hour));
        }
        for owner in 0..2 {
            result.refused_strength[owner] += note_totals.0[owner];
            result.maximum_overflows[owner] += note_totals.1[owner];
        }
        Ok(())
    }
    fn finish(mut self) -> ScannedNavy {
        self.seal();
        for (tag, track) in self.tracks {
            self.result.segments.insert(tag.clone(), track.segments);
            for (owner, runs) in track.runs.into_iter().enumerate() {
                self.result.series[owner].insert(tag.clone(), runs);
            }
            for (owner, runs) in track.strength_runs.into_iter().enumerate() {
                self.result.strength_series[owner].insert(tag.clone(), runs);
            }
        }
        #[cfg(test)]
        {
            self.result.updated_ships = self.census.updated;
        }
        self.result
    }
}

pub(super) fn scan(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<ScannedNavy> {
    scan_with_decode(replay, branch, cancelled, |_| {})
}

// The ignored benches time these boundaries; production's empty hook compiles out.
pub(super) fn scan_with_decode(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
    mut decoding: impl FnMut(bool),
) -> io::Result<ScannedNavy> {
    if !replay.branches.iter().any(|entry| entry.id == branch) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unknown navy size branch",
        ));
    }
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::TaskForces,
        TableId::Ships,
        TableId::ShipModels,
        TableId::LexicalTokens,
    ];
    let mut observer = Scan::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            // Waiting for the next block is its decoding.
            loop {
                check_cancel(cancelled)?;
                decoding(true);
                let next = blocks.next();
                decoding(false);
                let Some((_, frames)) = next else {
                    break;
                };
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    decoding(true);
                    let decoded = decoder.advance(&frame);
                    decoding(false);
                    let Ok(decoded) = decoded else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    observer.step(&frame, decoded, cancelled)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(observer.finish())
}

/// A live track's last run in one run list, as the next hour reads it.
#[derive(PartialEq)]
struct RunTail {
    total: Option<i64>,
    classes: Arc<BTreeMap<u32, i64>>,
    reference_total: Option<i64>,
    last_hour: i32,
}

/// A live track's state after an hour, as the next hour reads it: each run
/// list's last run, the track's last identity and hour, its identity track
/// and its last identity segment's hour.
#[derive(PartialEq)]
struct TrackTail {
    runs: [Option<RunTail>; 4],
    last: Option<(Identity, i32)>,
    identity: String,
    segment: Option<i32>,
}

impl TrackTail {
    /// A live track's tail as advancing it to `hour` leaves it. A live track
    /// the hour did not observe stays where it was last observed: whatever
    /// reads it next (its next observation, a change of the country join, a
    /// gap or a flush) first advances it through the hours since, so its
    /// state then is the advanced one. A track the hour observed is there.
    fn advanced(track: &Track, hour: i32) -> Self {
        let before = track.last.map(|(_, at)| at);
        let mut last_identity = track.last_identity.clone();
        if before.is_some() {
            last_identity.advance(hour);
        }
        let run = |runs: &[RawNavyRun]| {
            runs.last().map(|run| RunTail {
                total: run.total,
                classes: Arc::clone(&run.classes),
                reference_total: run.reference_total,
                last_hour: if before == Some(run.last_hour) {
                    hour
                } else {
                    run.last_hour
                },
            })
        };
        Self {
            runs: [
                run(track.runs[0].as_slice()),
                run(track.strength_runs[0].as_slice()),
                run(track.runs[1].as_slice()),
                run(track.strength_runs[1].as_slice()),
            ],
            last: track.last.map(|(identity, _)| (identity, hour)),
            identity: serde_json::to_string(&last_identity).unwrap_or_default(),
            segment: track.segments.last().map(|segment| {
                if before.is_some() {
                    hour
                } else {
                    segment.last_hour
                }
            }),
        }
    }

    /// This tail with its runs' classes renumbered.
    fn renumbered(&self, numbering: &Numbering) -> Self {
        Self {
            runs: std::array::from_fn(|index| {
                self.runs[index].as_ref().map(|run| RunTail {
                    total: run.total,
                    classes: numbering.classes(&run.classes),
                    reference_total: run.reference_total,
                    last_hour: run.last_hour,
                })
            }),
            last: self.last,
            identity: self.identity.clone(),
            segment: self.segment,
        }
    }
}

/// What decides a scan's next hours besides its live tracks: its last hour
/// and the hour its tables were last applied, the country join and the live
/// tags, slots and undisplayable tags it gives, the census's readiness and
/// read evidence, whether the last counts were exact, each country's notes
/// and their totals, and the counter rates an unchanged hour adds. The census
/// itself holds the hour's ships either way, read from the same frame.
/// Whether the hour's join changed is left out: every hour sets it before
/// reading it, and a census starting afresh joins anew where the carried one
/// finds its join held.
#[derive(PartialEq)]
struct Decision {
    last_hour: Option<i32>,
    applied: Option<i32>,
    countries: Vec<(Vec<u8>, Option<Identity>)>,
    live: BTreeSet<String>,
    slots: HashMap<u32, Vec<(String, Identity)>>,
    undisplayable: Vec<Vec<u8>>,
    census: (bool, Option<bool>, Option<bool>),
    previous_exact: Option<[bool; 2]>,
    notes: HashMap<String, ([usize; 2], [usize; 2])>,
    note_totals: ([usize; 2], [usize; 2]),
    rates: [usize; 7],
}

/// The counters a scan adds to, which a fold carries by the block's
/// increase after its first hour.
struct Counters {
    unread: usize,
    unattributed: [usize; 2],
    refused_strength: [usize; 2],
    maximum_overflows: [usize; 2],
    undisplayable_tags: BTreeMap<Vec<u8>, usize>,
    /// The ships updated and the work done by then, which tests count.
    #[cfg(test)]
    updated: usize,
    #[cfg(test)]
    work: dashboard_base::prepared::observer_tests::Work,
}

/// The scan right after a block's first hour.
pub struct Tail {
    hour: i32,
    decision: Decision,
    /// Each live tag's track, in tag order.
    tracks: Vec<(String, TrackTail)>,
    counters: Counters,
}

/// How a worker's class numbers name the carried scan's ([`Scan::numbering`]).
pub struct Numbering {
    /// The carried number of each of the worker's classes, by its number.
    ids: Vec<u32>,
    /// Whether every class keeps its number.
    same: bool,
    /// For each count of the worker's first classes, how many of them the
    /// carried catalogue lacks.
    added: Vec<usize>,
    /// The carried catalogue as it stood.
    carried: Arc<Vec<ClassKey>>,
    /// The carried catalogue with every class it lacks added, in the order
    /// the worker first saw them.
    full: Arc<Vec<ClassKey>>,
    /// The catalogues between, by the count of classes added: made only for
    /// a count an hour's projection base asks for, once.
    between: std::sync::Mutex<HashMap<usize, Arc<Vec<ClassKey>>>>,
}

impl Numbering {
    fn class(&self, class: u32) -> u32 {
        self.ids[class as usize]
    }

    /// A class map of the worker's, renumbered.
    pub(super) fn classes(&self, classes: &Arc<BTreeMap<u32, i64>>) -> Arc<BTreeMap<u32, i64>> {
        if self.same {
            return Arc::clone(classes);
        }
        Arc::new(
            classes
                .iter()
                .map(|(&class, &count)| (self.class(class), count))
                .collect(),
        )
    }

    /// The carried catalogue as it stands where the worker's had `len`
    /// classes: with the classes it lacked among those added. The carried
    /// and full catalogues are shared; one between them is made for the
    /// first hour that asks for it and shared by the later ones, so the
    /// catalogues kept grow with the hours that added classes, never with
    /// every class added.
    pub(super) fn catalogue(&self, len: usize) -> Arc<Vec<ClassKey>> {
        let added = self.added[len];
        let length = self.carried.len() + added;
        if length == self.carried.len() {
            return Arc::clone(&self.carried);
        }
        if length == self.full.len() {
            return Arc::clone(&self.full);
        }
        let mut between = self
            .between
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Arc::clone(
            between
                .entry(added)
                .or_insert_with(|| Arc::new(self.full[..length].to_vec())),
        )
    }

    /// The classes every catalogue it keeps holds, counted per catalogue.
    #[cfg(test)]
    fn kept(&self) -> usize {
        let between = self
            .between
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.carried.len()
            + self.full.len()
            + between.values().map(|catalogue| catalogue.len()).sum::<usize>()
    }
}

impl Census {
    /// Renumber the census's classes, and name `classes` by number.
    fn renumber(&mut self, numbering: &Numbering, classes: &[ClassKey]) {
        if !numbering.same {
            for ship in self.ships.values_mut() {
                ship.class = numbering.class(ship.class);
            }
            for class in self.class_keys.values_mut() {
                *class = numbering.class(*class);
            }
            for totals in self.totals.iter_mut().flat_map(HashMap::values_mut) {
                totals.classes = std::mem::take(&mut totals.classes)
                    .into_iter()
                    .map(|(class, sums)| (numbering.class(class), sums))
                    .collect();
                // Summaries are made again from the renumbered sums.
                totals.count_snapshot = None;
                totals.snapshot = None;
            }
        }
        self.class_ids = classes.iter().cloned().zip(0..).collect();
    }
}

impl Scan {
    fn decision(&self) -> Decision {
        Decision {
            last_hour: self.last_hour,
            applied: self.applied,
            countries: self.countries.tags.clone(),
            live: self.live.clone(),
            slots: self.slots.clone(),
            undisplayable: self.undisplayable.clone(),
            census: (
                self.census.ready,
                self.census.lexical_complete,
                self.census.models_complete,
            ),
            previous_exact: self.previous_exact,
            notes: self.notes.clone(),
            note_totals: self.note_totals,
            rates: self.rates,
        }
    }

    /// Each live tag's track tail, advanced to `hour`, in tag order.
    fn track_tails(&self, hour: i32) -> Vec<(String, TrackTail)> {
        self.live
            .iter()
            .map(|tag| (tag.clone(), TrackTail::advanced(&self.tracks[tag], hour)))
            .collect()
    }

    /// The scan right after a block's first hour, `hour`.
    pub(super) fn tail(&self, hour: i32) -> Tail {
        Tail {
            hour,
            decision: self.decision(),
            tracks: self.track_tails(hour),
            counters: Counters {
                unread: self.result.unread,
                unattributed: self.result.unattributed,
                refused_strength: self.result.refused_strength,
                maximum_overflows: self.result.maximum_overflows,
                undisplayable_tags: self.result.undisplayable_tags.clone(),
                #[cfg(test)]
                updated: self.census.updated,
                #[cfg(test)]
                work: self.work,
            },
        }
    }

    /// How `block`'s class numbers name this scan's: a class this scan knows
    /// keeps its number here, and the others take the next numbers in the
    /// order `block` first saw them. After a block's first hour, which both
    /// observed, both step through the same rows, so this scan would have
    /// numbered them in that order.
    pub(super) fn numbering(&self, block: &Scan) -> Numbering {
        let carried = &self.result.classes;
        let known: HashMap<&ClassKey, u32> = carried.iter().zip(0..).collect();
        let mut ids = Vec::with_capacity(block.result.classes.len());
        let mut added = Vec::with_capacity(block.result.classes.len() + 1);
        let mut lacked = Vec::new();
        added.push(0);
        for key in block.result.classes.iter() {
            let id = known.get(key).copied().unwrap_or_else(|| {
                lacked.push(key.clone());
                (carried.len() + lacked.len() - 1) as u32
            });
            ids.push(id);
            added.push(lacked.len());
        }
        let same = ids.iter().zip(0..).all(|(&id, at)| id == at);
        let full = if lacked.is_empty() {
            Arc::clone(carried)
        } else {
            let mut full = Vec::with_capacity(carried.len() + lacked.len());
            full.extend(carried.iter().cloned());
            full.extend(lacked);
            Arc::new(full)
        };
        Numbering {
            ids,
            same,
            added,
            carried: Arc::clone(carried),
            full,
            between: Default::default(),
        }
    }

    /// Whether this carried scan, after the block's first hour, decides every
    /// later hour as the worker's did from `tail`: then each later step is
    /// the worker's, as every step reads only the hour's frame, what the
    /// decision holds and the live tracks' tails.
    ///
    /// The worker's tracks live at the first hour each opened every run list
    /// and their identity segment there. A track not live then continues
    /// only through adjacent hours, so its history decides nothing later: a
    /// country entering later in the block was away at its first hour, and
    /// an entering country's track is not extended across the hours it was
    /// away, so it opens new runs and a new segment, as a worker's track
    /// starting afresh does.
    pub(super) fn continues(&self, tail: &Tail, block: &Scan, numbering: &Numbering) -> bool {
        let hour = tail.hour;
        self.decision() == tail.decision
            && self
                .track_tails(hour)
                .iter()
                .zip(&tail.tracks)
                .all(|((tag, carried), (other, extracted))| {
                    tag == other && *carried == extracted.renumbered(numbering)
                })
            && tail.tracks.iter().all(|(tag, _)| {
                block.tracks.get(tag).is_some_and(|track| {
                    track
                        .runs
                        .iter()
                        .chain(&track.strength_runs)
                        .all(|runs| runs.first().is_some_and(|run| run.first_hour == hour))
                        && track
                            .segments
                            .first()
                            .is_some_and(|segment| segment.first_hour == hour)
                })
            })
    }

    /// Carry this scan to the block's end: the runs and segment each track
    /// live at the first hour opened then extend the carried ones it
    /// continued, later ones follow, tags only the worker tracked take its
    /// tracks, the counters add their increase after the first hour, every
    /// class the worker numbered is renumbered as this scan numbers it, and
    /// everything else is the worker's.
    pub(super) fn carry(&mut self, tail: Tail, block: Scan, numbering: Numbering) {
        let Scan {
            result,
            tracks,
            last_hour,
            countries,
            mut census,
            live,
            observed,
            removed,
            applied,
            joined,
            slots,
            previous_exact,
            notes,
            note_totals,
            rates,
            undisplayable,
            #[cfg(test)]
            work,
        } = block;
        let Tail {
            hour,
            tracks: opened,
            counters: before,
            ..
        } = tail;
        let held = &mut self.result;
        held.unread += result.unread - before.unread;
        for owner in 0..2 {
            held.unattributed[owner] += result.unattributed[owner] - before.unattributed[owner];
            held.refused_strength[owner] +=
                result.refused_strength[owner] - before.refused_strength[owner];
            held.maximum_overflows[owner] +=
                result.maximum_overflows[owner] - before.maximum_overflows[owner];
        }
        for (tag, now) in result.undisplayable_tags {
            let then = before.undisplayable_tags.get(&tag).copied().unwrap_or(0);
            *held.undisplayable_tags.entry(tag).or_default() += now - then;
        }
        held.classes = Arc::clone(&numbering.full);
        #[cfg(test)]
        {
            census.updated = self.census.updated + census.updated - before.updated;
            add_work(&mut self.work, work, before.work);
        }
        census.renumber(&numbering, &held.classes);
        let opened: std::collections::HashSet<String> =
            opened.into_iter().map(|(tag, _)| tag).collect();
        for (tag, mut track) in tracks {
            for runs in track.runs.iter_mut().chain(&mut track.strength_runs) {
                for run in runs {
                    run.classes = numbering.classes(&run.classes);
                }
            }
            match self.tracks.get_mut(&tag) {
                Some(carried) => carry_track(carried, track, opened.contains(&tag), hour),
                None => {
                    self.tracks.insert(tag, track);
                }
            }
        }
        self.last_hour = last_hour;
        self.countries = countries;
        self.census = census;
        self.live = live;
        self.observed = observed;
        self.removed = removed;
        self.applied = applied;
        self.joined = joined;
        self.slots = slots;
        self.previous_exact = previous_exact;
        self.notes = notes;
        self.note_totals = note_totals;
        self.rates = rates;
        self.undisplayable = undisplayable;
    }
}

/// Carry one track: with `opened`, the worker's track opened its first run
/// in each list and its first segment at `hour`, where the carried track's
/// continue, so they extend those; every later run and segment follows, and
/// a track the worker observed takes its last identity and hour.
fn carry_track(carried: &mut Track, extracted: Track, opened: bool, hour: i32) {
    let Track {
        runs,
        strength_runs,
        last,
        mut segments,
        last_identity,
    } = extracted;
    let lists = runs
        .into_iter()
        .zip(carried.runs.iter_mut())
        .chain(strength_runs.into_iter().zip(carried.strength_runs.iter_mut()));
    for (mut runs, held) in lists {
        if opened
            && runs.first().is_some_and(|run| run.first_hour == hour)
            && let Some(open) = held.last_mut()
        {
            open.last_hour = runs[0].last_hour;
            runs.remove(0);
        }
        held.extend(runs);
    }
    if opened
        && segments
            .first()
            .is_some_and(|segment| segment.first_hour == hour)
        && let Some(open) = carried.segments.last_mut()
    {
        open.last_hour = segments[0].last_hour;
        segments.remove(0);
    }
    carried.segments.extend(segments);
    if last.is_some() {
        carried.last = last;
        carried.last_identity = last_identity;
    }
}

/// `work` adds what `now` did beyond `then`.
#[cfg(test)]
fn add_work(
    work: &mut dashboard_base::prepared::observer_tests::Work,
    now: dashboard_base::prepared::observer_tests::Work,
    then: dashboard_base::prepared::observer_tests::Work,
) {
    work.content_loads += now.content_loads - then.content_loads;
    work.row_visits += now.row_visits - then.row_visits;
    work.history_visits += now.history_visits - then.history_visits;
    work.emitted += now.emitted - then.emitted;
    work.interpreter_calls += now.interpreter_calls - then.interpreter_calls;
    work.dictionary_interpretations +=
        now.dictionary_interpretations - then.dictionary_interpretations;
}

/// What a block extract keeps of a navy census ([`Held`]): its tracks and
/// runs (each shared class map once), its census of ships, totals, task-force
/// owners and class names, its country join and its per-country notes, and
/// the tail kept after a block's first hour.
mod held {
    use super::*;
    use dashboard_base::prepared::prepass::{Held, HeldBytes};

    impl Held for RawNavyRun {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes.heap(&self.classes);
        }
    }

    impl Held for ScannedNavy {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.classes)
                .heap(&self.series)
                .heap(&self.strength_series)
                .heap(&self.segments)
                .heap(&self.undisplayable_tags);
        }
    }

    impl Held for Track {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.runs)
                .heap(&self.strength_runs)
                .heap(&self.segments);
        }
    }

    impl Held for Ship {
        fn held(&self, _: &mut HeldBytes) {}
    }

    impl Held for Summary {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes.heap(&self.classes).heap(&self.hp_classes);
        }
    }

    impl Held for Totals {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.classes)
                .heap(&self.maximums)
                .heap(&self.missing)
                .heap(&self.count_snapshot)
                .heap(&self.snapshot);
        }
    }

    impl Held for Census {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.ships)
                .heap(&self.dirty)
                .heap(&self.totals)
                .heap(&self.forces)
                .heap(&self.class_keys)
                .heap(&self.class_ids);
        }
    }

    impl Held for RunTail {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes.heap(&self.classes);
        }
    }

    impl Held for TrackTail {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes.heap(&self.runs).heap(&self.identity);
        }
    }

    impl Held for Tail {
        fn held(&self, bytes: &mut HeldBytes) {
            let decision = &self.decision;
            bytes
                .heap(&decision.countries)
                .heap(&decision.live)
                .heap(&decision.slots)
                .heap(&decision.undisplayable)
                .heap(&decision.notes)
                .heap(&self.tracks)
                .heap(&self.counters.undisplayable_tags);
        }
    }

    impl Held for Scan {
        fn held(&self, bytes: &mut HeldBytes) {
            bytes
                .heap(&self.result)
                .heap(&self.tracks)
                .heap(&self.countries.tags)
                .heap(&self.census)
                .heap(&self.live)
                .heap(&self.observed)
                .heap(&self.removed)
                .heap(&self.slots)
                .heap(&self.notes)
                .heap(&self.undisplayable);
        }
    }
}

#[cfg(test)]
mod numbering_tests {
    use super::*;

    fn key(token: u32) -> ClassKey {
        (token, Some(format!("mod_class_{token}")))
    }

    /// A block whose first hour has no ships and whose second names 5,000
    /// classes the carried catalogue lacks keeps the carried and full
    /// catalogues alone, not one per class added; an hour between makes its
    /// own catalogue once, shared by the hours after it.
    #[test]
    fn navy_numbering_keeps_catalogues_linear_in_classes_added_at_once() {
        let mut carried = Scan::default();
        carried.result.classes = Arc::new((0..10).map(key).collect());
        let mut block = Scan::default();
        block.result.classes = Arc::new((10..5_010).map(key).collect());
        let numbering = carried.numbering(&block);
        // The first hour's base (no classes), then every later hour's.
        assert_eq!(*numbering.catalogue(0), *carried.result.classes);
        for _ in 0..100 {
            let full = numbering.catalogue(5_000);
            assert_eq!(full.len(), 5_010);
            assert!(full.iter().zip((0..5_010).map(key)).all(|(a, b)| *a == b));
        }
        assert_eq!(numbering.kept(), 10 + 5_010);
        // An hour between makes one catalogue, which later asks share.
        let between = numbering.catalogue(2_500);
        assert_eq!(*between, (0..2_510).map(key).collect::<Vec<_>>());
        assert!(Arc::ptr_eq(&between, &numbering.catalogue(2_500)));
        assert_eq!(numbering.kept(), 10 + 5_010 + 2_510);
        // The worker's numbers follow the carried ones.
        assert_eq!(numbering.class(0), 10);
        assert_eq!(numbering.class(4_999), 5_009);
    }
}

#[cfg(test)]
mod prepared_continuation_tests {
    use super::*;
    use replay::frame::{TableUpdate, TableView};
    use replay::rows::{LexicalTokenRow, TableRows, TaskForceRow};
    #[test]
    fn prepared_navy_restores_class_identity_when_a_branch_returns_without_a_decoder_rejoin() {
        let mut decoded = dashboard_base::test_fixtures::scale_frame(1, 100, 1);
        *decoded.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                id: 1,
                roster_owner: 100_000,
                logical_owner: 100_000,
                ..Default::default()
            }]));
        *decoded.tables[TableId::Ships.index()] =
            TableView::Complete(TableRows::Ships(vec![ShipRow {
                id: 1,
                task_force: 1,
                class_id: 41,
                ..Default::default()
            }]));
        *decoded.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 41,
                literal: b"modded_ship".to_vec(),
            }]));
        let raw = |decoded: &DecodedFrame| Frame {
            branch: decoded.branch,
            hour: decoded.hour,
            keyframe: true,
            metrics: decoded.metrics.clone(),
            tables: std::array::from_fn(|_| TableUpdate::Unavailable {
                reason: String::new(),
            }),
        };
        let flag = AtomicBool::new(false);
        let mut scan = Scan::default();
        scan.step(&raw(&decoded), &decoded, &flag).unwrap();
        assert_eq!(scan.result.classes.len(), 1);
        let mut resumed: Scan =
            serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
        decoded.hour = 101;
        resumed.step(&raw(&decoded), &decoded, &flag).unwrap();
        assert_eq!(resumed.result.classes, scan.result.classes);
        assert!(
            resumed
                .tracks
                .values()
                .flat_map(|track| &track.runs[0])
                .all(|run| run.classes.keys().all(|&class| class == 0))
        );
    }
}
