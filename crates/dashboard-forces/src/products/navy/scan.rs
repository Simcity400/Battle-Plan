//! The row census of navy size: the parity oracle of the series family's publication.
use super::*;
use dashboard_base::prepared::frame_changes::field_mask;
use dashboard_base::products::{
    country_history::{Identity, observe_identity},
    force_census::{Countries, changed_ids},
};
use dashboard_text::metadata::{IdentityTrack, validate_campaign};
use replay::campaign::Replay;
use replay_data::{rows::ShipRow, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::ViewIdentity};

#[derive(Default)]
struct Track {
    runs: [Vec<RawNavyRun>; 2],
    strength_runs: [Vec<RawNavyRun>; 2],
    last: Option<(Identity, i32)>,
    segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
}

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
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
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
            && changed_ids(frame.update(TableId::Ships.index()), 0, &mut ids, |row| {
                row.mask & fields != 0 || !self.ships.contains_key(&(row.key as u64))
            })
            && (models.is_none()
                || changed_ids(
                    frame.update(TableId::ShipModels.index()),
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

#[derive(Default)]
pub struct Scan {
    result: ScannedNavy,
    tracks: HashMap<String, Track>,
    last_hour: Option<i32>,
    countries: Countries,
    census: Census,
    live: BTreeSet<String>,
    slots: HashMap<u32, Vec<(String, Identity)>>,
    previous_exact: Option<[bool; 2]>,
    notes: HashMap<String, ([usize; 2], [usize; 2])>,
    note_totals: ([usize; 2], [usize; 2]),
    undisplayable: Vec<Vec<u8>>,
}
impl Scan {
    fn seal(&mut self) {
        if let Some(hour) = self.last_hour {
            for tag in &self.live {
                self.tracks
                    .get_mut(tag)
                    .expect("live navy track")
                    .advance(hour);
            }
        }
    }
    fn step(
        &mut self,
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
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
            live.clear();
            slots.clear();
            *previous_exact = None;
            return Ok(());
        };
        let changed = changed || !continuous || slots.is_empty();
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
                frame.hour(),
            );
            let identity_continuous = track.last.is_some_and(|(seen, at)| {
                seen == identity && at.checked_add(1) == Some(frame.hour())
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
                                && run.last_hour.checked_add(1) == Some(frame.hour())
                                && run.total == total
                                && run.classes == classes
                                && run.reference_total == reference_total =>
                        {
                            run.last_hour = frame.hour();
                        }
                        _ => runs.push(RawNavyRun {
                            first_hour: frame.hour(),
                            last_hour: frame.hour(),
                            starts_segment,
                            total,
                            classes,
                            reference_total,
                        }),
                    }
                }
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
            track.last = Some((identity, frame.hour()));
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
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    decoding(true);
                    let decoded = Ok::<&DecodedFrame, std::io::Error>(&frame.decoded);
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

