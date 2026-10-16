//! Joined country/region values use shared series; queries reuse the air renderers.
use super::*;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use replay::frame::DecodedFrame;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
enum Field {
    Friendly(String, u32),
    Enemy(String, u32),
    Wings(u32),
    Active(u32),
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ChoiceOperand {
    keys: scan::KeyRead,
    published: Vec<(usize, AirSide)>,
}
#[derive(Serialize, Deserialize)]
struct ChoiceRead {
    operand: Arc<ChoiceOperand>,
    first_day_ordinal: i32,
    first_hour: i32,
    last_hour: i32,
}
/// The tables whose change chooses the day's sides again: any change of
/// their rows (a new decoding of them) marks the day's choice for a new read.
pub(super) const CHOICE_TABLES: [TableId; 6] = [
    TableId::Factions,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::LexicalTokens,
    TableId::Capitulation,
    TableId::HumanControllers,
];

/// The countries' score identity an hour's scores are projected through:
/// the score operands and each raw owner's tag.
pub(super) type ScoreIdentity = (scan::ScoreOperands, HashMap<u32, String>);

/// An hour's score projection made on a pre-pass worker, through the score
/// identity that worker read ([`Prepared::observe_scored`]).
pub(super) struct Scored {
    pub identity: Arc<ScoreIdentity>,
    pub projection: scan::CountryScores,
    /// This projection, as the next projection's [`PairDiff::after`] names it.
    pub mark: Arc<ProjectionMark>,
    /// How its pairs differ from the worker's projection before it in the
    /// same block, when there was one.
    pub diff: Option<PairDiff>,
}

/// One worker projection, told apart from every other by its allocation.
pub(super) struct ProjectionMark;

/// A worker projection's pairs against the worker's projection before it
/// (`after`): the pairs new or with other scores, in pair order, and the
/// pairs gone, in pair order.
pub(super) struct PairDiff {
    pub after: Arc<ProjectionMark>,
    pub changed: Vec<((String, u32), (i64, i64))>,
    pub removed: Vec<(String, u32)>,
}

/// Time a measured fold spent in each part of the observation, with the
/// times it took that part ([`Prepared::measure`]).
#[derive(Default)]
struct FoldParts {
    parts: BTreeMap<&'static str, (std::time::Duration, u64)>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    values: Accumulator,
    events: Accumulator,
    hours: Accumulator,
    previous_hour: Option<i32>,
    choices: Vec<ChoiceRead>,
    pending_choice: Option<ChoiceRead>,
    countries: BTreeSet<String>,
    regions: BTreeSet<u32>,
    theatres: BTreeMap<String, Vec<u8>>,
    available: bool,
    operands: Option<scan::ScoreOperands>,
    auxiliary: scan::AuxiliaryRead,
    last_auxiliary: Option<scan::Auxiliary>,
    theatre_read: Option<i32>,
    #[serde(skip)]
    keys: BTreeMap<String, u64>,
    #[serde(with = "dashboard_core::map_pairs")]
    current_keys: BTreeMap<(String, u32), (u64, u64)>,
    active_regions: BTreeSet<u32>,
    wing_regions: BTreeSet<u32>,
    choice_dirty: bool,
    #[serde(skip)]
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    #[serde(skip)]
    score_identity: Option<ScoreIdentity>,
    /// A worker's score identity found equal to [`Self::score_identity`],
    /// whose projections are this observer's own until the identity changes.
    #[serde(skip)]
    verified: Option<Arc<ScoreIdentity>>,
    #[serde(skip)]
    choice_indices: HashMap<String, usize>,
    #[serde(skip)]
    choice_input: Option<Arc<ChoiceOperand>>,
    #[serde(skip)]
    choice_key: Option<String>,
    #[serde(skip)]
    choice_ready: bool,
    #[serde(skip)]
    choice_index: Option<usize>,
    /// Each region's wings and active keys once named, so a region's fields
    /// are named (serialized and looked up) once, not every hour.
    #[serde(skip)]
    region_keys: HashMap<u32, [Option<u64>; 2]>,
    /// The worker projection this observer applied last, while its pairs
    /// are [`Self::current_keys`]: the projection a worker's next pair diff
    /// must follow to be taken.
    #[serde(skip)]
    applied: Option<Arc<ProjectionMark>>,
    #[serde(skip)]
    parts: Option<Box<FoldParts>>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    /// Name every field anew and walk every pair, taking no pair diff, as
    /// observation did before held ids and diffs: the parity oracle.
    #[cfg(test)]
    #[serde(skip)]
    plain: bool,
}
impl Prepared {
    fn key(&mut self, field: &Field) -> io::Result<u64> {
        let name = serde_json::to_string(field)?;
        let next =
            u64::try_from(self.keys.len()).map_err(|_| invalid("Air series key count overflow"))?;
        Ok(*self.keys.entry(name).or_insert(next))
    }
    /// [`Self::key`], held per region for its wings and active fields: the
    /// same key, named once.
    fn region_key(&mut self, field: &Field) -> io::Result<u64> {
        let (region, slot) = match field {
            Field::Wings(region) => (*region, 0),
            Field::Active(region) => (*region, 1),
            Field::Friendly(..) | Field::Enemy(..) => return self.key(field),
        };
        #[cfg(test)]
        if self.plain {
            return self.key(field);
        }
        if let Some(key) = self.region_keys.get(&region).and_then(|keys| keys[slot]) {
            return Ok(key);
        }
        let key = self.key(field)?;
        self.region_keys.entry(region).or_default()[slot] = Some(key);
        Ok(key)
    }
    /// Measure the parts of each observation from now on ([`Self::take_parts`]).
    pub fn measure(&mut self) {
        self.parts.get_or_insert_with(Box::default);
    }
    /// The time each part took since last asked, by part name, with the
    /// times it took it; none unless measured.
    pub fn take_parts(&mut self) -> Vec<(&'static str, std::time::Duration, u64)> {
        self.parts
            .as_mut()
            .map(|parts| std::mem::take(&mut parts.parts))
            .unwrap_or_default()
            .into_iter()
            .map(|(name, (took, times))| (name, took, times))
            .collect()
    }
    /// When a measured part starts.
    pub(super) fn clock(&self) -> Option<std::time::Instant> {
        self.parts.as_ref().map(|_| std::time::Instant::now())
    }
    /// Count the time since `started` to `part`.
    pub(super) fn lap(&mut self, started: Option<std::time::Instant>, part: &'static str) {
        if let (Some(parts), Some(started)) = (self.parts.as_mut(), started) {
            let entry = parts.parts.entry(part).or_default();
            entry.0 += started.elapsed();
            entry.1 += 1;
        }
    }
    /// One projected pair's scores at `hour` as the pair loop observes them,
    /// on its held keys while it is current and on its named fields when it
    /// is new; its keys.
    fn observe_pair(
        &mut self,
        pair: &(String, u32),
        (friendly, enemy): (i64, i64),
        hour: i32,
        continuous: bool,
    ) -> io::Result<(u64, u64)> {
        let held = self.current_keys.get(pair).copied();
        let previous = held
            .and_then(|(key, _)| self.values.state(key))
            .and_then(|state| state.last);
        self.choice_dirty |= previous.is_none_or(|run| (run.value > 0) != (friendly > 0));
        let fresh = !continuous || held.is_none();
        #[cfg(test)]
        let held = held.filter(|_| !self.plain);
        // A pair's friendly field is named before its enemy field, as each
        // is named before it is observed: the same numbering.
        let (friendly_key, enemy_key) = match held {
            Some(keys) => keys,
            None => (
                self.key(&Field::Friendly(pair.0.clone(), pair.1))?,
                self.key(&Field::Enemy(pair.0.clone(), pair.1))?,
            ),
        };
        self.observe_key(friendly_key, hour, Some(i128::from(friendly)), fresh);
        self.observe_key(enemy_key, hour, Some(i128::from(enemy)), fresh);
        Ok((friendly_key, enemy_key))
    }
    /// A pair gone from the projection: both its values end.
    fn end_pair(&mut self, (friendly, enemy): (u64, u64), hour: i32) {
        self.choice_dirty = true;
        self.values.observe(friendly, hour, None, false);
        self.values.observe(enemy, hour, None, false);
        #[cfg(test)]
        {
            self.work.emitted += 2;
        }
    }
    fn observe_value(
        &mut self,
        field: &Field,
        hour: i32,
        value: Option<i128>,
        force: bool,
    ) -> io::Result<u64> {
        let key = self.region_key(field)?;
        self.observe_key(key, hour, value, force);
        Ok(key)
    }
    /// Observe `value` on `key` when forced or when it is not the held one.
    fn observe_key(&mut self, key: u64, hour: i32, value: Option<i128>, force: bool) {
        if force
            || self
                .values
                .state(key)
                .and_then(|state| state.last)
                .map(|run| run.value)
                != value
        {
            self.values.observe(key, hour, value, false);
            #[cfg(test)]
            {
                self.work.emitted += 1;
            }
        }
    }
    #[cfg(test)]
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        groups: &mut dashboard_base::products::country_groups::Scan,
    ) -> io::Result<()> {
        let changes = dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
            .observe(frame, replay.format);
        self.observe_changed(frame, replay, groups, &changes)
    }
    pub fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        groups: &mut dashboard_base::products::country_groups::Scan,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.observe_scored(frame, replay, groups, changes, None)
    }
    /// Whether a worker's score identity is this observer's: the same
    /// operands and owners, so its projections are the ones this observer
    /// would make.
    fn accepts(&mut self, identity: &Arc<ScoreIdentity>) -> bool {
        if self
            .verified
            .as_ref()
            .is_some_and(|verified| Arc::ptr_eq(verified, identity))
        {
            return true;
        }
        let held = self.score_identity.as_ref() == Some(&**identity);
        if held {
            self.verified = Some(Arc::clone(identity));
        }
        held
    }
    /// [`Self::observe_changed`], with the hour's score projection made on a
    /// pre-pass worker (`scored`), used when that worker's score identity is
    /// this observer's; otherwise, or without one, the projection is made
    /// here.
    pub(super) fn observe_scored(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        groups: &mut dashboard_base::products::country_groups::Scan,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
        scored: Option<Scored>,
    ) -> io::Result<()> {
        if self
            .previous_hour
            .is_some_and(|hour| hour.div_euclid(DAY_HOURS) != frame.hour.div_euclid(DAY_HOURS))
        {
            let started = self.clock();
            if self.choice_dirty || !self.choice_ready {
                groups.finish(replay, frame.branch);
            }
            self.publish_day(groups, false)?;
            self.lap(started, "air_superiority.fold.day");
        }
        let continuous = self
            .previous_hour
            .is_some_and(|hour| hour.checked_add(1) == Some(frame.hour));
        let restoring = self.score_identity.is_none();
        let country_changed = changes.rows_changed[TableId::Countries.index()]
            && self.country_keys.as_ref().is_none_or(|keys| {
                #[cfg(test)]
                let before = keys.row_visits();
                let changed = !keys.same(frame.complete::<tables::Countries>().unwrap_or(&[]));
                #[cfg(test)]
                {
                    self.work.row_visits += keys.row_visits() - before;
                }
                changed
            });
        let identity_changed =
            restoring || country_changed || changes.rows_changed[TableId::CountryLinks.index()];
        if identity_changed {
            self.score_identity = Some(scan::score_countries(frame)?);
            self.verified = None;
            let countries = frame.complete::<tables::Countries>().unwrap_or(&[]);
            self.country_keys = Some(dashboard_text::metadata::CountryKeys::read(countries));
            #[cfg(test)]
            {
                self.work.row_visits += countries.len()
                    + frame
                        .complete::<tables::CountryLinks>()
                        .map_or(0, |rows| rows.len());
            }
            for country in countries {
                let tag = std::str::from_utf8(&country.tag)
                    .map_err(|_| invalid("Air country tag is not UTF-8"))?;
                dashboard_core::protocol::validate_tag(tag).map_err(invalid)?;
                self.countries.insert(tag.to_owned());
            }
        }
        self.values.advance(frame.hour)?;
        self.events.through(frame.hour);
        self.hours.advance(frame.hour)?;
        if !continuous {
            self.hours.observe(0, frame.hour, Some(1), false);
            self.theatre_read = None;
        }
        let score_changed =
            identity_changed || changes.rows_changed[TableId::RaidAirScores.index()];
        let mut new_regions = BTreeSet::new();
        let mut score_operands_changed = false;
        if score_changed {
            let accepted = scored.filter(|scored| self.accepts(&scored.identity));
            let started = self.clock();
            let (projection, mark, diff) = match accepted {
                Some(scored) => (scored.projection, Some(scored.mark), scored.diff),
                None => {
                    let rows =
                        dashboard_text::metadata::recorded_rows::<tables::RaidAirScores>(frame)?;
                    #[cfg(test)]
                    {
                        self.work.row_visits += rows.len();
                    }
                    let (operands, owners) = self
                        .score_identity
                        .as_ref()
                        .expect("score identity projection");
                    let projection = scan::score_values(
                        scores::ScoreView {
                            rows,
                            complete: frame.complete::<tables::RaidAirScores>().is_ok(),
                            owners: None,
                            active: None,
                        },
                        operands.clone(),
                        owners,
                    );
                    self.lap(started, "air_superiority.fold.projection");
                    (projection, None, None)
                }
            };
            // A worker's pair diff is taken only after the projection it
            // follows, applied the hour before: the held pairs are then that
            // projection's, each holding its score as its last value.
            let diff = diff.filter(|diff| {
                continuous
                    && self
                        .applied
                        .as_ref()
                        .is_some_and(|applied| Arc::ptr_eq(applied, &diff.after))
            });
            #[cfg(test)]
            let diff = diff.filter(|_| !self.plain);
            self.available |= projection.operands.available;
            if self.operands.as_ref() != Some(&projection.operands) {
                score_operands_changed = true;
                self.events.event(0, frame.hour, &projection.operands)?;
                #[cfg(test)]
                {
                    self.work.emitted += 1;
                }
                self.operands = Some(projection.operands);
            }
            let started = self.clock();
            if let Some(diff) = diff {
                // An unchanged pair would observe its held score unforced,
                // naming no field and dirtying no choice: only the changed
                // pairs, then the gone ones, observe anything, in the order
                // the whole walk observes them.
                for (pair, scores) in diff.changed {
                    let keys = self.observe_pair(&pair, scores, frame.hour, continuous)?;
                    self.current_keys.insert(pair, keys);
                }
                for pair in diff.removed {
                    #[cfg(test)]
                    {
                        self.work.history_visits += 1;
                    }
                    let keys = self
                        .current_keys
                        .remove(&pair)
                        .ok_or_else(|| invalid("Air pair diff names a pair not held"))?;
                    self.end_pair(keys, frame.hour);
                }
                self.lap(started, "air_superiority.fold.diffed");
            } else {
                let mut current = BTreeMap::new();
                for (pair, scores) in projection.pairs {
                    let keys = self.observe_pair(&pair, scores, frame.hour, continuous)?;
                    current.insert(pair, keys);
                }
                let held = std::mem::replace(&mut self.current_keys, current);
                for (pair, keys) in held {
                    #[cfg(test)]
                    {
                        self.work.history_visits += 1;
                    }
                    if !self.current_keys.contains_key(&pair) {
                        self.end_pair(keys, frame.hour);
                    }
                }
                self.lap(started, "air_superiority.fold.pairs");
            }
            self.applied = mark;
            let started = self.clock();
            let changed: BTreeSet<_> = self
                .active_regions
                .symmetric_difference(&projection.active)
                .copied()
                .collect();
            for region in &projection.active {
                if self.regions.insert(*region) {
                    new_regions.insert(*region);
                }
            }
            self.choice_dirty |= !changed.is_empty();
            for region in changed {
                self.observe_value(
                    &Field::Active(region),
                    frame.hour,
                    Some(i128::from(projection.active.contains(&region))),
                    false,
                )?;
            }
            self.active_regions = projection.active;
            self.lap(started, "air_superiority.fold.regions");
        }
        let started = self.clock();
        let previous_theatre_read = self.theatre_read;
        let auxiliary = self.auxiliary.read(
            changes.slot_read[TableId::AirTheatres.index()],
            frame,
            &mut self.theatre_read,
        );
        let auxiliary_changed = restoring
            || [
                TableId::AirWings,
                TableId::AirTheatres,
                TableId::AirTheatreGroups,
                TableId::AirTheatreMembers,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
        let wings_changed = restoring || changes.rows_changed[TableId::AirWings.index()];
        let known_changed = self
            .last_auxiliary
            .as_ref()
            .is_none_or(|old| old.wings_known != auxiliary.wings_known);
        if auxiliary_changed {
            #[cfg(test)]
            {
                self.work.row_visits += frame
                    .complete::<tables::AirWings>()
                    .map_or(0, |rows| rows.len())
                    + frame
                        .complete::<tables::AirTheatres>()
                        .map_or(0, |rows| rows.len())
                    + frame
                        .complete::<tables::AirTheatreGroups>()
                        .map_or(0, |rows| rows.len())
                    + frame
                        .complete::<tables::AirTheatreMembers>()
                        .map_or(0, |rows| rows.len());
            }
            self.theatres.extend(auxiliary.theatre_names.clone());
        }
        if auxiliary_changed {
            let operands = scan::Auxiliary {
                wings: BTreeMap::new(),
                wings_known: auxiliary.wings_known,
                theatres: Arc::clone(&auxiliary.theatres),
                theatre_names: auxiliary.theatre_names.clone(),
                theatres_known: auxiliary.theatres_known,
                theatre_read: None,
            };
            if self.last_auxiliary.as_ref() != Some(&operands) {
                self.events.event(1, frame.hour, &operands)?;
                #[cfg(test)]
                {
                    self.work.emitted += 1;
                }
                self.last_auxiliary = Some(operands);
            }
        }
        // A supplying date changes independently of the held theatre operands.
        // Its small dated event must not copy wings or the theatre dictionary.
        if !continuous || previous_theatre_read != self.theatre_read {
            self.events.event(2, frame.hour, &self.theatre_read)?;
            #[cfg(test)]
            {
                self.work.emitted += 1;
            }
        }
        self.lap(started, "air_superiority.fold.auxiliary");
        let started = self.clock();
        if wings_changed {
            for region in auxiliary.wings.keys() {
                if self.regions.insert(*region) {
                    new_regions.insert(*region);
                }
            }
        }
        let reopen = !continuous || restoring;
        let wing_regions: BTreeSet<_> = if known_changed || reopen {
            self.regions.clone()
        } else if wings_changed {
            self.wing_regions
                .iter()
                .chain(auxiliary.wings.keys())
                .chain(&new_regions)
                .copied()
                .collect()
        } else {
            new_regions.clone()
        };
        for region in wing_regions {
            self.observe_value(
                &Field::Wings(region),
                frame.hour,
                auxiliary
                    .wings_known
                    .then(|| i128::from(auxiliary.wings.get(&region).copied().unwrap_or(0))),
                known_changed || reopen,
            )?;
        }
        if wings_changed {
            self.wing_regions = auxiliary.wings.keys().copied().collect();
        }
        let active_regions = if reopen {
            self.regions.clone()
        } else {
            new_regions
        };
        for region in active_regions {
            self.observe_value(
                &Field::Active(region),
                frame.hour,
                Some(i128::from(self.active_regions.contains(&region))),
                reopen,
            )?;
        }
        self.lap(started, "air_superiority.fold.regions");
        // Positive/zero eligibility above can change the published sides; other
        // score magnitudes leave the recorded identity/name operands unchanged.
        self.choice_dirty |= score_operands_changed
            || identity_changed
            || !continuous
            || CHOICE_TABLES
                .iter()
                .any(|table| changes.rows_changed[table.index()]);
        self.previous_hour = Some(frame.hour);
        Ok(())
    }
    /// Whether `hour` can take [`Self::quiet_hour`]: it follows the last hour
    /// observed within the same day, after the scores' identity was read.
    pub(super) fn quiet_follows(&self, hour: i32) -> bool {
        self.score_identity.is_some()
            && self.previous_hour.is_some_and(|previous| {
                previous.checked_add(1) == Some(hour)
                    && previous.div_euclid(DAY_HOURS) == hour.div_euclid(DAY_HOURS)
            })
    }
    /// [`Self::observe_changed`] at an hour that [`Self::quiet_follows`] and
    /// whose scores, country keys, links, wings' regions and theatre rosters
    /// are the hour before's: the series reach the hour, the held theatres
    /// are dated by the hour's read of them (`theatres_read`, their slot
    /// read), and the day's choice is chosen again when a table the choice
    /// reads was decoded again (`choices_changed`), as that observation would
    /// find there. Rows decoded again with equal values change nothing else:
    /// every value it would observe is the one it holds, and every key it
    /// would name it named before.
    pub(super) fn quiet_hour(
        &mut self,
        hour: i32,
        theatres_read: bool,
        choices_changed: bool,
    ) -> io::Result<()> {
        self.values.advance(hour)?;
        self.events.through(hour);
        self.hours.advance(hour)?;
        let previous_theatre_read = self.theatre_read;
        self.auxiliary
            .date(theatres_read, hour, &mut self.theatre_read);
        if previous_theatre_read != self.theatre_read {
            self.events.event(2, hour, &self.theatre_read)?;
            #[cfg(test)]
            {
                self.work.emitted += 1;
            }
        }
        self.choice_dirty |= choices_changed;
        self.previous_hour = Some(hour);
        Ok(())
    }
    fn current_projection(&mut self) -> io::Result<scan::CountryScores> {
        let mut projection = scan::CountryScores {
            operands: self
                .operands
                .clone()
                .ok_or_else(|| invalid("Air score continuation is missing"))?,
            active: self.active_regions.clone(),
            ..Default::default()
        };
        for (pair, &(friendly, enemy)) in &self.current_keys {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let value = |key| {
                self.values
                    .state(key)
                    .and_then(|state| state.last)
                    .map(|run| run.value)
                    .ok_or_else(|| invalid("Prepared air score is missing"))
            };
            projection.pairs.insert(
                pair.clone(),
                (
                    i64::try_from(value(friendly)?)
                        .map_err(|_| invalid("Prepared friendly score is invalid"))?,
                    i64::try_from(value(enemy)?)
                        .map_err(|_| invalid("Prepared enemy score is invalid"))?,
                ),
            );
        }
        Ok(projection)
    }
    pub fn publish_day(
        &mut self,
        groups: &dashboard_base::products::country_groups::Scan,
        terminal: bool,
    ) -> io::Result<()> {
        let Some(hour) = self.previous_hour else {
            return Ok(());
        };
        if self.choice_dirty || !self.choice_ready {
            let projection = self.current_projection()?;
            let mut sides = Vec::new();
            let sample = scan::catalogue_snapshot(hour, &projection, groups, &mut sides)?;
            self.choice_input = sample.key_read.map(|keys| {
                let published_indices: HashMap<_, _> = keys
                    .published
                    .iter()
                    .map(|(index, key)| (key.as_str(), *index))
                    .collect();
                let published = sides
                    .into_iter()
                    .map(|mut side| {
                        let index = published_indices[side.key.as_str()];
                        side.reads.clear();
                        (index, side)
                    })
                    .collect();
                Arc::new(ChoiceOperand {
                    keys: (*keys).clone(),
                    published,
                })
            });
            self.choice_key = self
                .choice_input
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            self.choice_index = self
                .choice_key
                .as_ref()
                .and_then(|key| self.choice_indices.get(key))
                .copied();
            self.choice_dirty = false;
            self.choice_ready = true;
        }
        if terminal {
            self.pending_choice = self.choice_input.clone().map(|operand| ChoiceRead {
                operand,
                first_day_ordinal: hour.div_euclid(DAY_HOURS),
                first_hour: hour,
                last_hour: hour,
            });
        } else {
            self.pending_choice = None;
            if let Some(index) = self.choice_index {
                self.choices[index].last_hour = hour;
            } else if let Some(key) = &self.choice_key {
                let index = self.choices.len();
                self.choices.push(ChoiceRead {
                    operand: self.choice_input.clone().expect("choice operand"),
                    first_day_ordinal: hour.div_euclid(DAY_HOURS),
                    first_hour: hour,
                    last_hour: hour,
                });
                self.choice_indices.insert(key.clone(), index);
                self.choice_index = Some(index);
                #[cfg(test)]
                {
                    self.work.emitted += 1;
                }
            }
        }
        Ok(())
    }
    fn catalogue(&self, groups: &dashboard_base::products::country_groups::Scan) -> DayCache {
        let anchors = Arc::new(assignment::historical_anchors(groups));
        let mut sides = Vec::new();
        let mut indices = HashMap::new();
        let mut reads: Vec<_> = self
            .choices
            .iter()
            .chain(self.pending_choice.iter())
            .collect();
        reads.sort_by_key(|read| read.first_day_ordinal);
        for read in reads {
            let keys = assignment::resolve_keys(
                &read.operand.keys.sides,
                &read.operand.keys.countries,
                &anchors,
            );
            for (index, side) in &read.operand.published {
                let key = keys[*index].clone();
                let assigned = assignment::AssignedSide {
                    faction: key
                        .strip_prefix("f:")
                        .map(str::to_owned)
                        .or_else(|| side.faction.clone()),
                    key,
                    colour_faction: side.colour_faction.clone(),
                    factions: side.factions.clone(),
                    name: side.name.clone(),
                    leader: side.leader.clone(),
                    members: side.tag.iter().cloned().collect(),
                    co_belligerents: side.co_belligerents,
                };
                scan::dictionary(&mut sides, &mut indices, &assigned, read.first_hour);
                if read.last_hour != read.first_hour {
                    scan::dictionary(&mut sides, &mut indices, &assigned, read.last_hour);
                }
            }
        }
        DayCache {
            regions: self.regions.clone(),
            countries: self.countries.clone(),
            theatres: self.theatres.clone(),
            sides: Arc::new(sides),
            #[cfg(test)]
            side_indices: Arc::new(indices),
            anchors,
            warnings: groups.warnings.clone(),
            available: self.available,
            ..Default::default()
        }
    }
    pub fn bytes(&self) -> usize {
        self.values.bytes() + self.events.bytes() + self.hours.bytes()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.values.flush(
            writer,
            &format!("air-superiority-{branch}-values"),
            0,
            cancelled,
        )?;
        self.events.flush(
            writer,
            &format!("air-superiority-{branch}-operands"),
            0,
            cancelled,
        )?;
        self.hours.flush(
            writer,
            &format!("air-superiority-{branch}-hours"),
            0,
            cancelled,
        )?;
        series::write_catalogue(writer, "air-superiority", branch, &self.keys, &())?;
        let bytes =
            dashboard_base::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 3)?;
        writer.write_at(&format!("air-superiority-{branch}"), 0, bytes.as_slice())
    }
    pub fn read(
        store: &store::Store,
        key: &store::SourceKey,
        saved: Option<&store::SourceKey>,
        branch: u64,
    ) -> io::Result<Self> {
        let section = format!("air-superiority-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| invalid("Prepared air-superiority continuation is missing"))?;
        let mut state: Self = serde_json::from_reader(zstd::stream::Decoder::new(file)?)?;
        let catalogue: Option<series::Catalogue<()>> =
            series::read_saved_catalogue(store, key, saved, "air-superiority", branch)?;
        for (index, choice) in state.choices.iter().enumerate() {
            state
                .choice_indices
                .insert(serde_json::to_string(&choice.operand)?, index);
        }
        // Every committed block ends with a terminal publication, which leaves
        // its clean choice pending; resume from that choice as continuing does.
        if state.previous_hour.is_some() && !state.choice_dirty {
            state.choice_input = state
                .pending_choice
                .as_ref()
                .map(|read| Arc::clone(&read.operand));
            state.choice_key = state
                .choice_input
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            state.choice_index = state
                .choice_key
                .as_ref()
                .and_then(|key| state.choice_indices.get(key))
                .copied();
            state.choice_ready = true;
        }
        state.keys = catalogue
            .ok_or_else(|| invalid("Prepared air-superiority catalogue is missing"))?
            .0;
        Ok(state)
    }
    #[allow(clippy::too_many_arguments)]
    fn samples(
        &self,
        store: &store::Store,
        key: &store::SourceKey,
        branch: u64,
        params: &AirParams,
        groups: &dashboard_base::products::country_groups::Scan,
        anchors: &assignment::SideAnchors,
        sides: &mut Vec<AirSide>,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<Snapshot>> {
        let (first, last) = hours(params)?;
        let first = if params.view == AirView::Timeline {
            first
                .div_euclid(DAY_HOURS)
                .checked_mul(DAY_HOURS)
                .ok_or_else(|| invalid("Air timeline start is outside the calendar range"))?
        } else {
            first
        };
        let mut hours = Reader::new(
            store,
            key,
            format!("air-superiority-{branch}-hours"),
            cancelled,
        )?;
        let coverage = hours.runs(0, first, last)?;
        let mut wanted = Vec::new();
        if params.view == AirView::Timeline {
            let mut days = BTreeMap::new();
            for run in coverage {
                let first_day = run.first_hour.div_euclid(DAY_HOURS);
                let last_day = run.last_hour.div_euclid(DAY_HOURS);
                for day in first_day..=last_day {
                    let end = ((i64::from(day) + 1) * i64::from(DAY_HOURS) - 1)
                        .min(i64::from(run.last_hour)) as i32;
                    days.entry(day)
                        .and_modify(|hour: &mut i32| *hour = (*hour).max(end))
                        .or_insert(end);
                }
            }
            wanted.extend(days.into_values());
        } else {
            for run in coverage {
                wanted.extend(run.first_hour..=run.last_hour);
            }
        }
        let mut values = Reader::new(
            store,
            key,
            format!("air-superiority-{branch}-values"),
            cancelled,
        )?;
        let mut events = Reader::new(
            store,
            key,
            format!("air-superiority-{branch}-operands"),
            cancelled,
        )?;
        let fields: Vec<_> = self
            .keys
            .iter()
            .map(|(field, &id)| serde_json::from_str::<Field>(field).map(|field| (field, id)))
            .collect::<Result<_, _>>()?;
        let mut indices = sides
            .iter()
            .enumerate()
            .map(|(index, side)| (side.key.clone(), index as u32))
            .collect();
        let mut samples = Vec::new();
        let mut rosters = scan::RosterRead::default();
        for hour in wanted {
            check_cancel(cancelled)?;
            let (_, operands) = events
                .latest_event(0, hour)?
                .ok_or_else(|| invalid("Prepared air score operands are missing"))?;
            let (_, auxiliary) = events
                .latest_event(1, hour)?
                .ok_or_else(|| invalid("Prepared air auxiliary operands are missing"))?;
            let mut projection = scan::CountryScores {
                operands: serde_json::from_value(operands)?,
                ..Default::default()
            };
            let mut auxiliary: scan::Auxiliary = serde_json::from_value(auxiliary)?;
            let (_, read) = events
                .latest_event(2, hour)?
                .ok_or_else(|| invalid("Prepared air theatre read is missing"))?;
            auxiliary.theatre_read = serde_json::from_value(read)?;
            for (field, id) in &fields {
                let Some(run) = values.value_at(*id, hour)? else {
                    continue;
                };
                match field {
                    Field::Friendly(tag, region) => {
                        projection
                            .pairs
                            .entry((tag.clone(), *region))
                            .or_default()
                            .0 = i64::try_from(run.value)
                            .map_err(|_| invalid("Prepared friendly score is invalid"))?;
                    }
                    Field::Enemy(tag, region) => {
                        projection
                            .pairs
                            .entry((tag.clone(), *region))
                            .or_default()
                            .1 = i64::try_from(run.value)
                            .map_err(|_| invalid("Prepared enemy score is invalid"))?;
                    }
                    Field::Wings(region) if run.value > 0 => {
                        auxiliary.wings.insert(
                            *region,
                            u32::try_from(run.value)
                                .map_err(|_| invalid("Prepared air wing count is invalid"))?,
                        );
                    }
                    Field::Active(region) if run.value != 0 => {
                        projection.active.insert(*region);
                    }
                    _ => {}
                }
            }
            let view = (
                (params.mode == Some(AirMode::Viewpoint))
                    .then_some(params.viewpoint.as_deref())
                    .flatten(),
                params.viewpoint_country == Some(true),
            );
            let mut sample = scan::prepared_snapshot(
                hour,
                &projection,
                groups,
                anchors,
                view,
                (&mut *sides, &mut indices),
                &mut rosters,
            )?;
            scan::apply_auxiliary(&mut sample, &auxiliary);
            samples.push(sample);
        }
        Ok(samples)
    }
}

/// The country groups the published values were chosen against, as complete
/// as air superiority's coverage: its own read of them where it keeps one
/// (prepared on its own, or kept by the hourly walk while the groups are
/// prepared on their own), the published groups otherwise. Its own read makes
/// the groups' calls on the groups' frames, so both hold the same history.
fn read_groups(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<dashboard_base::products::country_groups::Scan> {
    let file = match store.read(key, &super::block::groups_section(branch), 0)? {
        Some(file) => file,
        None => store
            .read(key, &format!("country-groups-{branch}"), 0)?
            .ok_or_else(|| invalid("Prepared country groups are missing"))?,
    };
    dashboard_base::products::country_groups::Scan::read(io::BufReader::new(file))
}

pub fn query(
    source: &Source,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let mut params: AirParams = serde_json::from_value(params)?;
    AirSuperiority::validate(&params).map_err(invalid)?;
    let branch = params.branch_id.parse::<u64>().expect("validated branch");
    let replay = source.replay_ref();
    let start = replay
        .branches
        .iter()
        .find(|row| row.id == branch)
        .ok_or_else(|| invalid("Air branch is not in this recording"))?;
    if params.view == AirView::Choices && params.windows_only == Some(true) {
        let reply = AirReply::Choices(AirChoicesReply {
            view: AirView::Choices,
            branch_id: params.branch_id,
            regions: vec![],
            sides: vec![],
            theatres: vec![],
            countries: vec![],
            windows: moment_windows(replay, branch),
            warnings: vec![],
        });
        check_reply_size(&reply)?;
        return serde_json::to_value(reply).map_err(Into::into);
    }
    // Air superiority's own coverage: the branch's, or its own while it is
    // prepared on its own.
    let Some(coverage) = dashboard_base::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        "air-superiority",
        cancelled,
    )?
    else {
        return Ok(serde_json::Value::Null);
    };
    if !coverage.complete {
        // Stable side keys require the final country-group identity, membership
        // and war history. A single frame cannot supply a regional comparison.
        return Ok(serde_json::Value::Null);
    }
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let state = Prepared::read(&store, &key, None, branch)?;
    let groups = read_groups(&store, &key, branch)?;
    let mut cache = state.catalogue(&groups);
    let naming_hour = params
        .last_hour
        .unwrap_or_else(|| dashboard_text::metadata::branch_bounds(replay, start).1);
    if params.view == AirView::Choices {
        params.mode = None;
        params.viewpoint = None;
        params.viewpoint_country = None;
        params.sides = None;
        if params
            .theatres
            .as_ref()
            .is_some_and(|theatres| !theatres.is_empty())
        {
            let first = params.first_hour.unwrap_or(i32::MIN);
            let last = params.last_hour.unwrap_or(i32::MAX);
            let mut hours = Reader::new(
                &store,
                &key,
                format!("air-superiority-{branch}-hours"),
                cancelled,
            )?;
            let mut events = Reader::new(
                &store,
                &key,
                format!("air-superiority-{branch}-operands"),
                cancelled,
            )?;
            let mut segment = scan::DaySegment::default();
            for run in hours.runs(0, first, last)? {
                let (_, value) = events
                    .latest_event(1, run.first_hour)?
                    .ok_or_else(|| invalid("Prepared theatre operands are missing"))?;
                let mut auxiliary: scan::Auxiliary = serde_json::from_value(value)?;
                let mut at = run.first_hour;
                let mut cursor = 0;
                loop {
                    let page = events.events(&[1], run.first_hour, run.last_hour, cursor, 1024)?;
                    for row in page.rows {
                        if row.hour > at {
                            segment
                                .auxiliary
                                .push((at, row.hour - 1, Arc::new(auxiliary)));
                        }
                        at = row.hour;
                        auxiliary = serde_json::from_value(row.row)?;
                    }
                    let Some(next) = page.cursor else {
                        break;
                    };
                    cursor = next;
                }
                segment
                    .auxiliary
                    .push((at, run.last_hour, Arc::new(auxiliary)));
            }
            cache.segments.push(Arc::new(segment));
        }
        return serde_json::to_value(cache_reply(
            source,
            replay,
            &groups,
            &cache,
            &params,
            context,
            naming_hour,
            cancelled,
        )?)
        .map_err(Into::into);
    }
    let anchors = Arc::clone(&cache.anchors);
    let mut sides = if params.view == AirView::Timeline {
        (*cache.sides).clone()
    } else {
        Vec::new()
    };
    let samples = state.samples(
        &store, &key, branch, &params, &groups, &anchors, &mut sides, cancelled,
    )?;
    let reply = if params.view == AirView::Moment {
        let mut reply = AirReply::Moment(moment_reply(samples, sides, &groups, &params, &anchors)?);
        resolve_names(source, &groups, context, naming_hour, None, &mut reply);
        check_reply_size(&reply)?;
        reply
    } else {
        let mut segment = scan::DaySegment::default();
        for sample in samples {
            segment
                .days
                .insert(sample.hour.div_euclid(DAY_HOURS), Arc::new(sample));
        }
        cache.segments.push(Arc::new(segment));
        cache.sides = Arc::new(sides);
        cache_reply(
            source,
            replay,
            &groups,
            &cache,
            &params,
            context,
            naming_hour,
            cancelled,
        )?
    };
    check_cancel(cancelled)?;
    serde_json::to_value(reply).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{air, groups, params, recording};
    use super::*;
    use dashboard_base::prepared::series::tests::Fixture;
    use dashboard_core::test_fixtures::scale_frame;
    use replay::{
        frame::TableView,
        rows::{RaidAirScoreRow, TableRows, WarRelationRow},
    };

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "5,000-country observer work and reply parity"
    )]
    fn superiority_shared_driver_equal_scores_keep_daily_choices_and_native_work_flat() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let measured = observer_tests::scaled_work(8, |hours| {
            let fixture = Fixture::new();
            let flag = AtomicBool::new(false);
            let mut first = scale_frame(1, 25, 5000);
            for rows in [
                TableRows::Factions(vec![]),
                TableRows::FactionMembers(vec![]),
                TableRows::WarRelations(vec![]),
                TableRows::HumanControllers(vec![]),
                TableRows::Capitulation(vec![]),
                TableRows::AirWings(vec![]),
            ] {
                let table = rows.table();
                first.tables[table.index()] = TableView::Complete(rows).into();
            }
            first.tables[TableId::RaidAirScores.index()] =
                TableView::Complete(TableRows::RaidAirScores(
                    (0..5000)
                        .map(|country| RaidAirScoreRow {
                            country: 100000 + country,
                            region: 7000,
                            friendly: i64::from(country) + 1,
                            enemy: 99,
                            impact: Default::default(),
                        })
                        .collect(),
                ))
                .into();
            let frames: Vec<_> = (0..=25 + hours)
                .map(|hour| {
                    let mut frame = first.clone();
                    frame.hour = hour as i32;
                    frame
                })
                .collect();
            let replay = recording(frames.clone());
            let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
            let mut state = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            for frame in &frames[..=25] {
                groups.observe(frame)?;
                state.observe_changed(
                    frame,
                    &replay,
                    &mut groups,
                    &tracker.observe(frame, replay.format),
                )?;
            }
            let delta = observer_tests::unchanged_frames(
                state.work,
                &first,
                hours as i32,
                replay.format,
                |frame, changes| {
                    groups.observe(frame)?;
                    state.observe_changed(frame, &replay, &mut groups, changes)?;
                    Ok(state.work)
                },
            )?;
            assert_eq!(delta, observer_tests::Work::default());
            groups.finish(&replay, 1);
            state.publish_day(&groups, true)?;
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag)?;
            writer.commit()?;
            parity(&fixture, &frames, None);
            Ok(delta)
        })
        .unwrap();
        observer_tests::assert_flat(measured);
    }

    #[test]
    fn superiority_shared_driver_one_score_change_does_not_walk_historical_regions() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let measured = observer_tests::scaled_work(20, |history| {
            let mut frame = air(0, 100000);
            let replay = recording(vec![frame.clone()]);
            let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
            let mut state = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            for hour in 0..history {
                frame.hour = hour as i32;
                if let TableView::Complete(TableRows::RaidAirScores(rows)) =
                    &mut *frame.tables[TableId::RaidAirScores.index()]
                {
                    rows[0].region = 1000 + hour as u32;
                }
                groups.observe(&frame)?;
                state.observe_changed(
                    &frame,
                    &replay,
                    &mut groups,
                    &tracker.observe(&frame, replay.format),
                )?;
            }
            let end = (frame.hour.div_euclid(DAY_HOURS) + 1) * DAY_HOURS + 1;
            while frame.hour < end {
                frame.hour += 1;
                groups.observe(&frame)?;
                state.observe_changed(
                    &frame,
                    &replay,
                    &mut groups,
                    &tracker.observe(&frame, replay.format),
                )?;
            }
            assert!(state.regions.len() >= history);
            let before = state.work;
            let delta = observer_tests::unchanged_frames(
                before,
                &frame,
                10,
                replay.format,
                |frame, changes| {
                    groups.observe(frame)?;
                    state.observe_changed(frame, &replay, &mut groups, changes)?;
                    Ok(state.work)
                },
            )?;
            assert_eq!(delta, observer_tests::Work::default());
            let mut previous = frame.clone();
            previous.hour += 10;
            tracker.observe(&previous, replay.format);
            frame.hour += 11;
            if let TableView::Complete(TableRows::RaidAirScores(rows)) =
                &mut *frame.tables[TableId::RaidAirScores.index()]
            {
                rows[0].friendly += 1;
            }
            groups.observe(&frame)?;
            state.observe_changed(
                &frame,
                &replay,
                &mut groups,
                &tracker.observe(&frame, replay.format),
            )?;
            let changed = observer_tests::Work {
                row_visits: state.work.row_visits - before.row_visits,
                history_visits: state.work.history_visits - before.history_visits,
                emitted: state.work.emitted - before.emitted,
                ..Default::default()
            };
            assert!(changed.row_visits < 100 && changed.history_visits < 10);
            Ok(changed)
        })
        .unwrap();
        observer_tests::assert_flat(measured);
    }

    #[test]
    fn superiority_choices_refresh_when_held_identity_scores_cross_zero() {
        use dashboard_base::prepared::frame_changes::FrameChangeTracker;
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let first = air(0, 200000);
        let mut zero = first.clone();
        if let TableView::Complete(TableRows::RaidAirScores(rows)) =
            &mut *zero.tables[TableId::RaidAirScores.index()]
        {
            rows.iter_mut()
                .find(|row| row.country == 9001)
                .unwrap()
                .friendly = 0;
        }
        let frames: Vec<_> = (0..=49)
            .map(|hour| {
                let mut frame = if hour < 24 {
                    first.clone()
                } else {
                    zero.clone()
                };
                frame.hour = hour;
                frame
            })
            .collect();
        let replay = recording(frames.clone());
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames {
            groups.observe(frame).unwrap();
            state
                .observe_changed(
                    frame,
                    &replay,
                    &mut groups,
                    &tracker.observe(frame, replay.format),
                )
                .unwrap();
        }
        groups.finish(&replay, 1);
        state.publish_day(&groups, true).unwrap();
        assert_eq!(
            state.choices.len(),
            2,
            "zero scores remove a published side even when its identity is held"
        );
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        parity(&fixture, &frames, None);
    }

    #[test]
    fn superiority_equal_theatre_reads_publish_dates_without_copying_the_dictionary() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        for hours in [8, 80] {
            let fixture = Fixture::new();
            let flag = AtomicBool::new(false);
            let mut first = air(25, 200000);
            first.metrics.table_ns[TableId::AirTheatres.index()] = 1;
            first.tables[TableId::AirTheatres.index()] =
                TableView::Complete(TableRows::AirTheatres(
                    (0..128)
                        .map(|id| replay::rows::AirTheatreRow {
                            id,
                            name: format!("theatre-{id}").into_bytes(),
                            ..Default::default()
                        })
                        .collect(),
                ))
                .into();
            first.tables[TableId::AirTheatreGroups.index()] =
                TableView::Complete(TableRows::AirTheatreGroups(vec![])).into();
            first.tables[TableId::AirTheatreMembers.index()] =
                TableView::Complete(TableRows::AirTheatreMembers(vec![])).into();
            let frames: Vec<_> = (0..=25 + hours)
                .map(|hour| {
                    let mut frame = first.clone();
                    frame.hour = hour;
                    frame
                })
                .collect();
            let replay = recording(frames.clone());
            let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
            let mut state = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            for frame in &frames[..=25] {
                groups.observe(frame).unwrap();
                state
                    .observe_changed(
                        frame,
                        &replay,
                        &mut groups,
                        &tracker.observe(frame, replay.format),
                    )
                    .unwrap();
            }
            let work = observer_tests::unchanged_frames(
                state.work,
                &first,
                hours,
                replay.format,
                |frame, changes| {
                    groups.observe(frame)?;
                    state.observe_changed(frame, &replay, &mut groups, changes)?;
                    Ok(state.work)
                },
            )
            .unwrap();
            assert_eq!(
                work,
                observer_tests::Work {
                    emitted: hours as usize,
                    ..Default::default()
                },
                "the only hourly work is its small supplying-date event"
            );
            groups.finish(&replay, 1);
            state.publish_day(&groups, true).unwrap();
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            let mut reader = Reader::new(
                &fixture.store,
                &fixture.key,
                "air-superiority-1-operands".into(),
                &flag,
            )
            .unwrap();
            assert_eq!(reader.event_count_through(1, i32::MAX).unwrap(), 1);
            let (_, date) = reader.latest_event(2, 25 + hours).unwrap().unwrap();
            assert_eq!(date, serde_json::json!(25 + hours));
            parity(&fixture, &frames, None);
        }
    }

    fn parity(fixture: &Fixture, frames: &[DecodedFrame], theatres: Option<Vec<String>>) {
        let replay = recording(frames.to_vec());
        let groups = groups(&replay);
        let flag = AtomicBool::new(false);
        let state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        let expected = scan::scan_days(&replay, 1, &groups, None, &flag).unwrap();
        let cache = state.catalogue(&groups);
        let choices = |sides: &[AirSide]| {
            sides
                .iter()
                .map(|side| {
                    let mut side = side.clone();
                    if let Some((_, name, leader, tag, count, factions, colour)) = side.reads.last()
                    {
                        side.name = name.clone();
                        side.leader = leader.clone();
                        side.tag = tag.clone();
                        side.co_belligerents = *count;
                        side.factions = factions.clone();
                        side.colour_faction = colour.clone();
                    }
                    side.reads.clear();
                    side
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(choices(&cache.sides), choices(&expected.sides));
        assert_eq!(cache.regions, expected.regions);
        assert_eq!(cache.countries, expected.countries);
        assert_eq!(cache.theatres, expected.theatres);
        let first = frames.first().unwrap().hour;
        let last = frames.last().unwrap().hour;
        for view in [AirView::Moment, AirView::Timeline] {
            for (viewpoint, country) in [(None, false), (Some("f:9"), false), (Some("MOD"), true)] {
                let mut cache = state.catalogue(&groups);
                let mut request = params(view, first, last);
                request.regions = None;
                request.theatres = theatres.clone();
                request.top = Some(5000);
                request.mode = Some(if viewpoint.is_some() {
                    AirMode::Viewpoint
                } else {
                    AirMode::Sides
                });
                request.viewpoint = viewpoint.map(str::to_owned);
                request.viewpoint_country = Some(country);
                let mut sides = if view == AirView::Timeline {
                    (*cache.sides).clone()
                } else {
                    vec![]
                };
                let samples = state
                    .samples(
                        &fixture.store,
                        &fixture.key,
                        1,
                        &request,
                        &groups,
                        &cache.anchors,
                        &mut sides,
                        &flag,
                    )
                    .unwrap();
                if view == AirView::Moment {
                    let actual =
                        moment_reply(samples, sides, &groups, &request, &cache.anchors).unwrap();
                    let expected = scan_moment(&replay, 1, &groups, &request, &flag).unwrap();
                    assert_eq!(
                        serde_json::to_value(actual).unwrap(),
                        serde_json::to_value(expected).unwrap()
                    );
                } else {
                    let mut segment = scan::DaySegment::default();
                    for sample in samples {
                        segment
                            .days
                            .insert(sample.hour.div_euclid(DAY_HOURS), Arc::new(sample));
                    }
                    cache.segments = vec![Arc::new(segment)];
                    cache.sides = Arc::new(sides);
                    let cold = scan::scan_days_kind(&replay, 1, &groups, viewpoint, country, &flag)
                        .unwrap();
                    assert_eq!(
                        serde_json::to_value(timeline(&cache, &request).unwrap()).unwrap(),
                        serde_json::to_value(timeline(&cold, &request).unwrap()).unwrap()
                    );
                }
            }
        }
    }
    #[test]
    fn prepared_air_superiority_two_publications_resume_append_and_final_side_keys_match() {
        let mut fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut frames = vec![
            air(23, 200000),
            air(24, i64::MAX),
            air(47, 400000),
            air(48, 500000),
            air(49, 600000),
        ];
        for frame in &mut frames {
            *frame.tables[TableId::AirWings.index()] =
                TableView::Complete(TableRows::AirWings(vec![replay::rows::AirWingRow {
                    id: 17,
                    owner: 9001,
                    mission_region: Some(if frame.hour >= 48 { 7999 } else { 7000 }),
                    ..Default::default()
                }]));
            *frame.tables[TableId::AirTheatres.index()] =
                TableView::Complete(TableRows::AirTheatres(vec![replay::rows::AirTheatreRow {
                    id: 700,
                    owner: 9001,
                    name: b"Synthetic theatre".to_vec(),
                    ..Default::default()
                }]));
            *frame.tables[TableId::AirTheatreGroups.index()] =
                TableView::Complete(TableRows::AirTheatreGroups(vec![
                    replay::rows::AirTheatreGroupRow {
                        id: 701,
                        theatre: 700,
                        ..Default::default()
                    },
                ]));
            *frame.tables[TableId::AirTheatreMembers.index()] =
                TableView::Complete(TableRows::AirTheatreMembers(vec![
                    replay::rows::AirTheatreMemberRow {
                        wing: 17,
                        group: 701,
                        ..Default::default()
                    },
                ]));
            frame.metrics.table_ns[TableId::AirTheatres.index()] = 1;
        }
        if let TableView::Complete(TableRows::Factions(rows)) =
            &mut *frames[2].tables[TableId::Factions.index()]
        {
            rows[0].name = b"Renamed synthetic pact".to_vec();
        }
        for frame in &mut frames[3..] {
            *frame.tables[TableId::WarRelations.index()] =
                TableView::Complete(TableRows::WarRelations(vec![
                    WarRelationRow {
                        first_country: 44,
                        second_country: 66,
                        ..Default::default()
                    },
                    WarRelationRow {
                        first_country: 9001,
                        second_country: 66,
                        ..Default::default()
                    },
                ]));
        }
        let replay = recording(frames.clone());
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared::default();
        for chunk in [&frames[..1], &frames[1..3]] {
            let mut writer = fixture.writer();
            for frame in chunk {
                groups.observe(frame).unwrap();
                state.observe(frame, &replay, &mut groups).unwrap();
            }
            groups.finish(&replay, 1);
            state.publish_day(&groups, true).unwrap();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            // The restored observer resumes from the same daily choice.
            let cache = |state: &Prepared| {
                (
                    state.choice_ready,
                    state.choice_dirty,
                    state.choice_key.clone(),
                    state.choice_index,
                )
            };
            let continued = cache(&state);
            state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
            assert_eq!(cache(&state), continued);
        }
        parity(&fixture, &frames[..3], None);
        parity(&fixture, &frames[..3], Some(vec!["700".into()]));
        let previous = fixture.key.clone();
        let mut writer = fixture.append_writer();
        state = Prepared::read(&fixture.store, &fixture.key, Some(&previous), 1).unwrap();
        for frame in &frames[3..] {
            groups.observe(frame).unwrap();
            state.observe(frame, &replay, &mut groups).unwrap();
        }
        groups.finish(&replay, 1);
        state.publish_day(&groups, true).unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        parity(&fixture, &frames, None);
        parity(&fixture, &frames, Some(vec!["700".into()]));
        assert_eq!(state.choices.first().unwrap().first_day_ordinal, 0);
        assert_eq!(state.pending_choice.as_ref().unwrap().first_day_ordinal, 2);
        assert_eq!(state.pending_choice.as_ref().unwrap().last_hour, 49);
    }
    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn prepared_air_superiority_5000_countries_preserve_numeric_and_identity_gaps() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut first = scale_frame(1, 23, 5000);
        for rows in [
            TableRows::Factions(vec![]),
            TableRows::FactionMembers(vec![]),
            TableRows::WarRelations(vec![]),
            TableRows::HumanControllers(vec![]),
            TableRows::Capitulation(vec![]),
            TableRows::AirWings(vec![]),
        ] {
            let table = rows.table();
            *first.tables[table.index()] = TableView::Complete(rows);
        }
        *first.tables[TableId::RaidAirScores.index()] =
            TableView::Complete(TableRows::RaidAirScores(
                (0..5000)
                    .map(|country| RaidAirScoreRow {
                        country: 100000 + country,
                        region: 7000,
                        friendly: i64::from(country) + 1,
                        enemy: 99,
                        impact: Default::default(),
                    })
                    .collect(),
            ));
        let mut last = first.clone();
        last.hour = 25;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *last.tables[TableId::Countries.index()]
        {
            rows[0].id = 999999;
            rows[0].tag = b"alias".to_vec();
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *last.tables[TableId::CountryLinks.index()]
        {
            rows[0].id = 999999;
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::RaidAirScores(rows)) =
            &mut *last.tables[TableId::RaidAirScores.index()]
        {
            rows[0].country = 999999;
            rows.sort_by_key(|row| row.country);
        }
        let frames = vec![first, last];
        let replay = recording(frames.clone());
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared::default();
        for frame in &frames {
            let mut writer = fixture.writer();
            groups.observe(frame).unwrap();
            state.observe(frame, &replay, &mut groups).unwrap();
            groups.finish(&replay, 1);
            state.publish_day(&groups, true).unwrap();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        }
        assert_eq!(state.countries.len(), 5001);
        parity(&fixture, &frames, None);
    }
    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn prepared_air_superiority_continuation_grows_with_distinct_operands_and_not_days() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let replay = recording(vec![air(0, 200000)]);
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared::default();
        let mut sizes = vec![];
        for end in [100, 10000] {
            let start = state.previous_hour.map_or(0, |hour| hour + 1);
            let mut writer = fixture.writer();
            for hour in start..end {
                let frame = air(hour, 200000);
                groups.observe(&frame).unwrap();
                state.observe(&frame, &replay, &mut groups).unwrap();
            }
            state.publish_day(&groups, true).unwrap();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            sizes.push(serde_json::to_vec(&state).unwrap().len());
            assert!(state.choices.len() <= 1);
            assert_eq!(state.choices.first().unwrap().first_day_ordinal, 0);
            assert!(state.pending_choice.is_some());
        }
        eprintln!("air-superiority continuation bytes after 100 and 10000 hours: {sizes:?}");
        assert!(sizes[1] < sizes[0] + 4096);
    }
    /// Held key ids and the workers' pair diffs publish, byte for byte, what
    /// naming every field anew and walking every pair publishes: scores
    /// changing every hour of an air war, a score's sign flipping, a pair
    /// appearing, going and coming back, a pair unchanged for hours, a new
    /// region, a wing moving, a gap, two day boundaries and the worker's
    /// projections starting again at block boundaries.
    #[test]
    fn superiority_held_keys_and_pair_diffs_publish_what_plain_observation_does() {
        use super::super::block::ScoreTracker;
        use dashboard_base::prepared::frame_changes::FrameChangeTracker;
        use replay::rows::AirWingRow;
        let score = |country, region, friendly, enemy| RaidAirScoreRow {
            country,
            region,
            friendly,
            enemy,
            impact: Default::default(),
        };
        let frames: Vec<_> = (0..60)
            .filter(|hour| *hour != 33)
            .map(|hour| {
                let mut frame = air(hour, 200_000 + i64::from(hour) * 1_000);
                let germany = if hour % 7 == 3 {
                    -500
                } else {
                    1_000 + i64::from(hour)
                };
                let mut scores = vec![
                    score(44, 7000, 100_000, 200_000 + i64::from(hour) * 1_000),
                    score(9001, 7000, germany, 100_000),
                ];
                if (10..20).contains(&hour) || (30..40).contains(&hour) {
                    scores.push(score(55, 7999, 3_000 + i64::from(hour % 3), 0));
                }
                if hour >= 25 {
                    scores.push(score(66, 7100, 4_000, 1_000));
                }
                *frame.tables[TableId::RaidAirScores.index()] =
                    TableView::Complete(TableRows::RaidAirScores(scores));
                if hour >= 5 {
                    *frame.tables[TableId::AirWings.index()] =
                        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                            id: 1,
                            owner: 44,
                            mission_region: Some(if hour < 50 { 7000 } else { 7200 }),
                            ..Default::default()
                        }]));
                }
                frame
            })
            .collect();
        let replay = recording(frames.clone());
        let flag = AtomicBool::new(false);
        let run = |plain: bool| -> (BTreeMap<String, Vec<Vec<u8>>>, Prepared) {
            let fixture = Fixture::new();
            let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
            let mut state = Prepared {
                plain,
                ..Default::default()
            };
            state.measure();
            let mut changes = FrameChangeTracker::new();
            let mut worker = ScoreTracker::default();
            for frame in &frames {
                // The pre-pass reads each block alone: its worker starts
                // again at each block's first hour.
                if [20, 45].contains(&frame.hour) {
                    worker = ScoreTracker::default();
                }
                let changes = changes.observe(frame, replay.format);
                groups.observe(frame).unwrap();
                if plain {
                    state
                        .observe_changed(frame, &replay, &mut groups, &changes)
                        .unwrap();
                } else {
                    let scored = worker.scored(frame, &changes).unwrap();
                    state
                        .observe_scored(frame, &replay, &mut groups, &changes, scored)
                        .unwrap();
                }
            }
            groups.finish(&replay, 1);
            state.publish_day(&groups, true).unwrap();
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            let mut sections = BTreeMap::new();
            for name in [
                "air-superiority-1",
                "air-superiority-1-values",
                "air-superiority-1-operands",
                "air-superiority-1-hours",
                "air-superiority-1-catalogue",
            ] {
                let count = fixture
                    .store
                    .range_count(&fixture.key, name)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{name} was not written"));
                let ranges = (0..count)
                    .map(|range| {
                        let mut bytes = Vec::new();
                        std::io::Read::read_to_end(
                            &mut fixture
                                .store
                                .read(&fixture.key, name, range)
                                .unwrap()
                                .unwrap(),
                            &mut bytes,
                        )
                        .unwrap();
                        bytes
                    })
                    .collect();
                sections.insert(name.to_owned(), ranges);
            }
            (sections, state)
        };
        let (plain, _) = run(true);
        let (held, mut state) = run(false);
        assert_eq!(
            held.keys().collect::<Vec<_>>(),
            plain.keys().collect::<Vec<_>>()
        );
        for (name, ranges) in &plain {
            assert_eq!(&held[name], ranges, "{name}");
        }
        let parts: BTreeMap<_, _> = state
            .take_parts()
            .into_iter()
            .map(|(name, _, times)| (name, times))
            .collect();
        assert!(
            parts
                .get("air_superiority.fold.diffed")
                .is_some_and(|times| *times > 20),
            "most war hours observe only their changed pairs: {parts:?}"
        );
        assert!(
            parts.contains_key("air_superiority.fold.pairs"),
            "block starts and the gap walk every pair: {parts:?}"
        );
    }

    #[test]
    fn prepared_air_superiority_window_list_reads_no_scan_state() {
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "prepared-air-windows".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            Arc::new(recording(vec![air(0, 100000)])),
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut request = params(AirView::Choices, 0, 0);
        request.windows_only = Some(true);
        let reply = query(
            &source,
            serde_json::to_value(request).unwrap(),
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(!reply["windows"].as_array().unwrap().is_empty());
        assert!(
            source
                .append_state::<AirCache>()
                .lock()
                .unwrap()
                .branches
                .is_empty()
        );
    }
}
