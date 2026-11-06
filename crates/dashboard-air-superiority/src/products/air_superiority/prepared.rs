//! Joined country/region values use shared series; queries reuse the air renderers.
use super::*;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};

mod facts;
mod first_view;
mod reader;
mod values;

/// Air superiority's own read of the country groups, kept under its own name.
pub fn groups_section(branch: u64) -> String {
    format!("air-superiority-groups-{branch}")
}

/// The section and catalogue name of its groups' change publisher, kept beside its read of them.
pub const GROUP_CHANGES: &str = "air-superiority-group-changes";

#[derive(Serialize, Deserialize)]
enum Field {
    Friendly(String, u32),
    Enemy(String, u32),
    Wings(u32),
    Active(u32),
}

/// Query operands from the continuation. The fold's accumulators, open joins
/// and score dictionaries are not read by a page query.
#[derive(Deserialize)]
struct QueryState {
    previous_hour: Option<i32>,
    countries: BTreeSet<String>,
    regions: BTreeSet<u32>,
    theatres: BTreeMap<String, Vec<u8>>,
    available: bool,
    facts: QueryFacts,
}
#[derive(Deserialize)]
struct QueryFacts {
    checkpoints: Option<facts::Checkpoints>,
}

/// The roster last assigned: its groups (None where no sides were
/// assigned) and their faction names.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(super) struct Assigned {
    groups: Option<Vec<daily::Group>>,
    names: Vec<String>,
}

impl Assigned {
    /// The sides `groups` assign the present countries at `hour`, with their faction names;
    /// none without country identities.
    pub(super) fn read(
        groups: &dashboard_base::products::country_groups::Scan,
        hour: i32,
        identity_known: bool,
        present: &BTreeSet<String>,
    ) -> Self {
        // Groups runs only open at and seal to observed hours: this is the final groups' roster.
        let sides = identity_known
            .then(|| assignment::native_roster(groups, hour, present))
            .flatten();
        Self {
            names: sides
                .iter()
                .flatten()
                .flat_map(|side| side.factions.iter().map(|(_, name)| name.clone()))
                .collect(),
            groups: sides.as_deref().map(daily::roster),
        }
    }

    /// The bytes it holds on the heap.
    pub(super) fn heap(&self) -> usize {
        let text = |text: &String| size_of::<String>() + text.capacity();
        let group = |group: &daily::Group| {
            size_of::<daily::Group>()
                + group.factions.capacity() * size_of::<u64>()
                + group.leader.as_ref().map_or(0, String::capacity)
                + group.members.iter().map(text).sum::<usize>()
        };
        self.names.iter().map(text).sum::<usize>()
            + self.groups.iter().flatten().map(group).sum::<usize>()
    }
}

/// The tables whose change assigns the day's sides again: any change of
/// their rows (a new decoding of them) marks the day's roster for a new read.
#[cfg(any(test, feature = "test-support"))]
pub(super) const ROSTER_TABLES: [TableId; 6] = [
    TableId::Factions,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::LexicalTokens,
    TableId::Capitulation,
    TableId::HumanControllers,
];

/// The countries' score identity an hour's scores are projected through:
/// the score operands and each raw owner's tag.
#[cfg(any(test, feature = "test-support"))]
pub(super) type ScoreIdentity = (scan::ScoreOperands, HashMap<u32, String>);

/// An hour's score projection made on a pre-pass worker, through the score
/// identity that worker read ([`Prepared::observe_scored`]).
#[cfg(any(test, feature = "test-support"))]
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
#[cfg(any(test, feature = "test-support"))]
pub(super) struct ProjectionMark;

/// A worker projection's pairs against the worker's projection before it
/// (`after`): the pairs new or with other scores, in pair order, and the
/// pairs gone, in pair order.
#[cfg(any(test, feature = "test-support"))]
pub(super) struct PairDiff {
    pub after: Arc<ProjectionMark>,
    pub changed: Vec<((String, u32), (i64, i64))>,
    pub removed: Vec<(String, u32)>,
}

/// Time a measured fold spent in each part of the observation, with the
/// times it took that part ([`Prepared::measure`]).
#[cfg(any(test, feature = "test-support"))]
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
    /// Every score owner's tag a pair named, by the id pairs are held by, in the order named.
    owners: Vec<String>,
    #[serde(skip)]
    owner_ids: HashMap<String, u32>,
    /// The ids of the owners the walk's segment names, by its index of them.
    #[serde(skip)]
    segment_owners: Vec<u32>,
    /// Each current pair's keys, by owner id and region.
    #[serde(with = "dashboard_core::map_pairs")]
    current_keys: BTreeMap<(u32, u32), (u64, u64)>,
    active_regions: BTreeSet<u32>,
    wing_regions: BTreeSet<u32>,
    roster_dirty: bool,
    /// The roster last assigned, None before the first.
    roster: Option<Assigned>,
    /// Hours at which recorded wars end with no row changing, latest first.
    war_ends: Vec<i32>,
    facts: facts::Ledger,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    country_keys: Option<dashboard_text::metadata::CountryKeys>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    score_identity: Option<ScoreIdentity>,
    /// A worker's score identity found equal to [`Self::score_identity`],
    /// whose projections are this observer's own until the identity changes.
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    verified: Option<Arc<ScoreIdentity>>,
    /// Each region's wings and active keys once named, so a region's fields
    /// are named (serialized and looked up) once, not every hour.
    #[serde(skip)]
    region_keys: HashMap<u32, [Option<u64>; 2]>,
    /// The worker projection this observer applied last, while its pairs
    /// are [`Self::current_keys`]: the projection a worker's next pair diff
    /// must follow to be taken.
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    applied: Option<Arc<ProjectionMark>>,
    #[cfg(any(test, feature = "test-support"))]
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
    /// A columnar walk's hour was observed since this state was built or loaded.
    #[serde(skip)]
    walked: bool,
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn measure(&mut self) {
        self.parts.get_or_insert_with(Box::default);
    }
    /// The time each part took since last asked, by part name, with the
    /// times it took it; none unless measured.
    #[cfg(any(test, feature = "test-support"))]
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
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn clock(&self) -> Option<std::time::Instant> {
        self.parts.as_ref().map(|_| std::time::Instant::now())
    }
    /// Count the time since `started` to `part`.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn lap(&mut self, started: Option<std::time::Instant>, part: &'static str) {
        if let (Some(parts), Some(started)) = (self.parts.as_mut(), started) {
            let entry = parts.parts.entry(part).or_default();
            entry.0 += started.elapsed();
            entry.1 += 1;
        }
    }
    /// The owner ids by tag, as [`Self::owners`] names them.
    fn index_owners(&mut self) {
        if self.owner_ids.len() != self.owners.len() {
            self.owner_ids = self
                .owners
                .iter()
                .enumerate()
                .map(|(id, tag)| (tag.clone(), id as u32))
                .collect();
        }
    }
    /// The id pairs hold `tag`'s scores by, named now where it was not before.
    fn owner(&mut self, tag: &str) -> io::Result<u32> {
        self.index_owners();
        if let Some(&id) = self.owner_ids.get(tag) {
            return Ok(id);
        }
        let id =
            u32::try_from(self.owners.len()).map_err(|_| invalid("Air score owner count overflow"))?;
        self.owners.push(tag.to_owned());
        self.owner_ids.insert(tag.to_owned(), id);
        Ok(id)
    }
    /// The owner id of the walk's segment's owner `index`.
    fn segment_owner(&self, index: u32) -> io::Result<u32> {
        self.segment_owners
            .get(index as usize)
            .copied()
            .ok_or_else(|| invalid("Air score pair names an owner its segment did not"))
    }
    /// One projected pair's scores at `hour` as the pair loop observes them,
    /// on its held keys while it is current and on its named fields when it
    /// is new; its keys.
    fn observe_pair(
        &mut self,
        pair: (u32, u32),
        (friendly, enemy): (i64, i64),
        hour: i32,
        continuous: bool,
    ) -> io::Result<(u64, u64)> {
        let held = self.current_keys.get(&pair).copied();
        let previous = held
            .and_then(|(key, _)| self.values.state(key))
            .and_then(|state| state.last);
        self.roster_dirty |= previous.is_none_or(|run| (run.value > 0) != (friendly > 0));
        let fresh = !continuous || held.is_none();
        #[cfg(test)]
        let held = held.filter(|_| !self.plain);
        // A pair's friendly field is named before its enemy field, as each
        // is named before it is observed: the same numbering.
        let (friendly_key, enemy_key) = match held {
            Some(keys) => keys,
            None => {
                let tag = &self.owners[pair.0 as usize];
                let (friendly, enemy) = (
                    Field::Friendly(tag.clone(), pair.1),
                    Field::Enemy(tag.clone(), pair.1),
                );
                (self.key(&friendly)?, self.key(&enemy)?)
            }
        };
        self.observe_key(friendly_key, hour, Some(i128::from(friendly)), fresh);
        self.observe_key(enemy_key, hour, Some(i128::from(enemy)), fresh);
        Ok((friendly_key, enemy_key))
    }
    /// A pair gone from the projection: both its values end.
    fn end_pair(&mut self, (friendly, enemy): (u64, u64), hour: i32) {
        self.roster_dirty = true;
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
    #[cfg(any(test, feature = "test-support"))]
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
    #[cfg(any(test, feature = "test-support"))]
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
    #[cfg(any(test, feature = "test-support"))]
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
            if self.roster_dirty || self.roster.is_none() {
                groups.finish(replay, frame.branch);
            }
            self.publish_day(groups, false)?;
            self.lap(started, "air_superiority.fold.day");
        }
        let continuous = self
            .previous_hour
            .is_some_and(|hour| hour.checked_add(1) == Some(frame.hour));
        let restoring = self.score_identity.is_none();
        if restoring || changes.rows_changed[TableId::WarRelations.index()] {
            self.war_ends = frame
                .complete::<tables::WarRelations>()
                .map(|rows| {
                    rows.iter()
                        .filter(|row| row.end_hour > row.start_hour && row.end_hour > frame.hour)
                        .map(|row| row.end_hour)
                        .collect()
                })
                .unwrap_or_default();
            self.war_ends
                .sort_unstable_by(|left, right| right.cmp(left));
            self.war_ends.dedup();
        }
        self.end_wars(frame.hour);
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
                for ((tag, region), scores) in diff.changed {
                    let pair = (self.owner(&tag)?, region);
                    let keys = self.observe_pair(pair, scores, frame.hour, continuous)?;
                    self.current_keys.insert(pair, keys);
                }
                for (tag, region) in diff.removed {
                    #[cfg(test)]
                    {
                        self.work.history_visits += 1;
                    }
                    let pair = (self.owner(&tag)?, region);
                    let keys = self
                        .current_keys
                        .remove(&pair)
                        .ok_or_else(|| invalid("Air pair diff names a pair not held"))?;
                    self.end_pair(keys, frame.hour);
                }
                self.lap(started, "air_superiority.fold.diffed");
            } else {
                let mut current = BTreeMap::new();
                for ((tag, region), scores) in projection.pairs {
                    let pair = (self.owner(&tag)?, region);
                    let keys = self.observe_pair(pair, scores, frame.hour, continuous)?;
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
            self.roster_dirty |= !changed.is_empty();
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
            self.theatres.extend(
                auxiliary
                    .theatre_names
                    .iter()
                    .map(|(id, name)| (id.clone(), name.clone())),
            );
        }
        if auxiliary_changed {
            let operands = scan::Auxiliary {
                wings: BTreeMap::new(),
                wings_known: auxiliary.wings_known,
                theatres: Arc::clone(&auxiliary.theatres),
                theatre_names: Arc::clone(&auxiliary.theatre_names),
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
        self.roster_dirty |= score_operands_changed
            || identity_changed
            || !continuous
            || ROSTER_TABLES
                .iter()
                .any(|table| changes.rows_changed[table.index()]);
        self.previous_hour = Some(frame.hour);
        Ok(())
    }
    /// Whether `hour` can take [`Self::quiet_hour`]: it follows the last hour
    /// observed within the same day, after the scores' identity was read.
    #[cfg(any(test, feature = "test-support"))]
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
    /// read), and the day's roster is assigned again when a table it reads
    /// was decoded again (`roster_changed`) or a war ended, as that
    /// observation would find there. Rows decoded again with equal values change nothing else:
    /// every value it would observe is the one it holds, and every key it
    /// would name it named before.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn quiet_hour(
        &mut self,
        hour: i32,
        theatres_read: bool,
        roster_changed: bool,
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
        self.roster_dirty |= roster_changed;
        self.end_wars(hour);
        self.previous_hour = Some(hour);
        Ok(())
    }
    /// [`Self::observe_scored`] of `hour` as a columnar walk's operator projected it
    /// ([`columnar::Air`]): each part it hands on is the part that hour's tables would give where
    /// they changed, and none where they did not, and the day's roster where the hour may end its
    /// day. The first hour after this state was built or loaded is one its operator gave whole.
    pub(super) fn observe_columnar(&mut self, hour: i32, air: columnar::Air) -> io::Result<()> {
        let restoring = !self.walked;
        if restoring && !air.whole {
            return Err(invalid(
                "Air superiority went on at an hour its operator did not give whole",
            ));
        }
        self.walked = true;
        if self
            .previous_hour
            .is_some_and(|previous| previous.div_euclid(DAY_HOURS) != hour.div_euclid(DAY_HOURS))
        {
            self.finish_day()?;
        }
        let continuous = self
            .previous_hour
            .is_some_and(|previous| previous.checked_add(1) == Some(hour));
        if air.first {
            self.segment_owners.clear();
        }
        for tag in &air.owners {
            let id = self.owner(tag)?;
            self.segment_owners.push(id);
        }
        self.countries.extend(air.countries);
        self.values.advance(hour)?;
        self.events.through(hour);
        self.hours.advance(hour)?;
        if !continuous {
            self.hours.observe(0, hour, Some(1), false);
            self.theatre_read = None;
        }
        let mut new_regions = BTreeSet::new();
        if let Some(scores) = air.scores {
            let columnar::Scores {
                operands,
                pairs,
                active,
            } = *scores;
            // None where the operator handed on the same operands before.
            if let Some(operands) = operands {
                self.available |= operands.available;
                if self.operands.as_ref() != Some(&operands) {
                    self.events.event(0, hour, &operands)?;
                    self.operands = Some(operands);
                }
            }
            match pairs {
                // Only the pairs the projection before this one held otherwise observe anything:
                // an unchanged pair would observe its held score unforced.
                columnar::Pairs::Changed { changed, removed } => {
                    if !continuous {
                        return Err(invalid("Air score changes reached an hour after a gap"));
                    }
                    for (owner, region, friendly, enemy) in changed {
                        let pair = (self.segment_owner(owner)?, region);
                        let keys = self.observe_pair(pair, (friendly, enemy), hour, continuous)?;
                        self.current_keys.insert(pair, keys);
                    }
                    for (owner, region) in removed {
                        let pair = (self.segment_owner(owner)?, region);
                        let keys = self
                            .current_keys
                            .remove(&pair)
                            .ok_or_else(|| invalid("Air pair changes name a pair not held"))?;
                        self.end_pair(keys, hour);
                    }
                }
                columnar::Pairs::Whole(pairs) => {
                    let mut current = BTreeMap::new();
                    for (owner, region, friendly, enemy) in pairs {
                        let pair = (self.segment_owner(owner)?, region);
                        let keys = self.observe_pair(pair, (friendly, enemy), hour, continuous)?;
                        current.insert(pair, keys);
                    }
                    let held = std::mem::replace(&mut self.current_keys, current);
                    for (pair, keys) in held {
                        if !self.current_keys.contains_key(&pair) {
                            self.end_pair(keys, hour);
                        }
                    }
                }
            }
            if let Some(active) = active {
                let changed: BTreeSet<_> = self
                    .active_regions
                    .symmetric_difference(&active)
                    .copied()
                    .collect();
                for region in &active {
                    if self.regions.insert(*region) {
                        new_regions.insert(*region);
                    }
                }
                for region in changed {
                    self.observe_value(
                        &Field::Active(region),
                        hour,
                        Some(i128::from(active.contains(&region))),
                        false,
                    )?;
                }
                self.active_regions = active;
            }
        }
        let previous_theatre_read = self.theatre_read;
        // A part read anew reads every table the walk's auxiliary read takes: wings and theatres.
        let auxiliary_changed = restoring || air.auxiliary.is_some();
        let auxiliary = self.auxiliary.apply(
            air.auxiliary.as_deref(),
            air.theatres_read,
            hour,
            &mut self.theatre_read,
        );
        let known_changed = self
            .last_auxiliary
            .as_ref()
            .is_none_or(|old| old.wings_known != auxiliary.wings_known);
        if auxiliary_changed {
            self.theatres.extend(
                auxiliary
                    .theatre_names
                    .iter()
                    .map(|(id, name)| (id.clone(), name.clone())),
            );
            let operands = scan::Auxiliary {
                wings: BTreeMap::new(),
                wings_known: auxiliary.wings_known,
                theatres: Arc::clone(&auxiliary.theatres),
                theatre_names: Arc::clone(&auxiliary.theatre_names),
                theatres_known: auxiliary.theatres_known,
                theatre_read: None,
            };
            if self.last_auxiliary.as_ref() != Some(&operands) {
                self.events.event(1, hour, &operands)?;
                self.last_auxiliary = Some(operands);
            }
        }
        if !continuous || previous_theatre_read != self.theatre_read {
            self.events.event(2, hour, &self.theatre_read)?;
        }
        if auxiliary_changed {
            for region in auxiliary.wings.keys() {
                if self.regions.insert(*region) {
                    new_regions.insert(*region);
                }
            }
        }
        let reopen = !continuous || restoring;
        let wing_regions: BTreeSet<_> = if known_changed || reopen {
            self.regions.clone()
        } else if auxiliary_changed {
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
                hour,
                auxiliary
                    .wings_known
                    .then(|| i128::from(auxiliary.wings.get(&region).copied().unwrap_or(0))),
                known_changed || reopen,
            )?;
        }
        if auxiliary_changed {
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
                hour,
                Some(i128::from(self.active_regions.contains(&region))),
                reopen,
            )?;
        }
        // The roster as the hour leaves it, for the day it may end.
        if let Some(roster) = air.roster {
            self.roster = Some(*roster);
        }
        self.previous_hour = Some(hour);
        Ok(())
    }
    /// Its series keys, by field name, which its continuation does not save.
    pub(super) fn keys(&self) -> &BTreeMap<String, u64> {
        &self.keys
    }
    pub(super) fn set_keys(&mut self, keys: BTreeMap<String, u64>) {
        self.keys = keys;
    }
    /// The wars ending by `hour` change the groups with no row changing: the
    /// roster is assigned again.
    #[cfg(any(test, feature = "test-support"))]
    fn end_wars(&mut self, hour: i32) {
        while self.war_ends.last().is_some_and(|end| *end <= hour) {
            self.war_ends.pop();
            self.roster_dirty = true;
        }
    }
    /// The day of the last hour observed: its roster assigned again where it
    /// may have changed and, once the day is over (not `terminal`), its facts
    /// recorded.
    #[cfg(any(test, feature = "test-support"))]
    pub fn publish_day(
        &mut self,
        groups: &dashboard_base::products::country_groups::Scan,
        terminal: bool,
    ) -> io::Result<()> {
        let Some(hour) = self.previous_hour else {
            return Ok(());
        };
        let operands = self
            .operands
            .clone()
            .ok_or_else(|| invalid("Air score continuation is missing"))?;
        if self.roster_dirty || self.roster.is_none() {
            self.roster = Some(Assigned::read(
                groups,
                hour,
                operands.identity_known,
                &operands.present,
            ));
            self.roster_dirty = false;
        }
        if terminal {
            return Ok(());
        }
        self.finish_day()
    }
    /// Records the finished day of the last hour observed, with the roster held for it.
    fn finish_day(&mut self) -> io::Result<()> {
        let Some(hour) = self.previous_hour else {
            return Ok(());
        };
        let operands = self
            .operands
            .clone()
            .ok_or_else(|| invalid("Air score continuation is missing"))?;
        self.index_owners();
        let assigned = self
            .roster
            .as_ref()
            .ok_or_else(|| invalid("Air superiority's day roster is missing"))?;
        let (current_keys, values, owner_ids) =
            (&self.current_keys, &self.values, &self.owner_ids);
        let pairs = |tag: &str, each: &mut dyn FnMut(u32, (i64, i64))| -> io::Result<()> {
            let Some(&owner) = owner_ids.get(tag) else {
                return Ok(());
            };
            for (&(_, region), (friendly, enemy)) in
                current_keys.range((owner, 0)..=(owner, u32::MAX))
            {
                let value = |key: &u64| {
                    values
                        .state(*key)
                        .and_then(|state| state.last)
                        .map(|run| run.value)
                        .ok_or_else(|| invalid("Prepared air score is missing"))
                };
                each(
                    region,
                    (
                        i64::try_from(value(friendly)?)
                            .map_err(|_| invalid("Prepared friendly score is invalid"))?,
                        i64::try_from(value(enemy)?)
                            .map_err(|_| invalid("Prepared enemy score is invalid"))?,
                    ),
                );
            }
            Ok(())
        };
        self.facts.finish(facts::Sample {
            hour,
            operands: &operands,
            auxiliary: self.last_auxiliary.as_ref(),
            theatre_read: self.theatre_read,
            roster: assigned.groups.as_deref(),
            names: &assigned.names,
            pairs: &pairs,
            active: &self.active_regions,
        })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn bytes(&self) -> usize {
        self.values.bytes() + self.events.bytes() + self.hours.bytes() + self.facts.bytes()
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
        self.facts.flush(writer, branch)?;
        series::write_catalogue(writer, "air-superiority", branch, &self.keys, &())?;
        let bytes =
            dashboard_base::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 3)?;
        let result = writer.write_at(&format!("air-superiority-{branch}"), 0, bytes.as_slice());
        #[cfg(any(test, feature = "test-support"))]
        if result.is_ok() {
            dashboard_base::prepared::pages::framing::note_compressed(&bytes);
        }
        result
    }
    #[cfg(any(test, feature = "test-support"))]
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
        state.keys = catalogue
            .ok_or_else(|| invalid("Prepared air-superiority catalogue is missing"))?
            .0;
        Ok(state)
    }

    fn read_query(store: &store::Store, key: &store::SourceKey, branch: u64) -> io::Result<Self> {
        let _cpu = replay_frame::cpu::scope_phase("air_superiority:query_continuation");
        let state: QueryState =
            series::read_reply(store, key, &format!("air-superiority-{branch}"), 0)?
                .ok_or_else(|| invalid("Prepared air-superiority continuation is missing"))?;
        let keys = series::read_catalogue::<()>(store, key, "air-superiority", branch)?
            .ok_or_else(|| invalid("Prepared air-superiority catalogue is missing"))?
            .0;
        Ok(Self {
            previous_hour: state.previous_hour,
            countries: state.countries,
            regions: state.regions,
            theatres: state.theatres,
            available: state.available,
            facts: facts::Ledger::read(state.facts.checkpoints),
            keys,
            ..Default::default()
        })
    }
    /// The catalogue's fields with their keys.
    fn fields(&self) -> io::Result<Vec<(Field, u64)>> {
        self.keys
            .iter()
            .map(|(field, &id)| serde_json::from_str::<Field>(field).map(|field| (field, id)))
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }
    /// The hours `params` samples, and the series reading their inputs.
    fn sampled<'a>(
        &self,
        store: &'a store::Store,
        key: &'a store::SourceKey,
        branch: u64,
        params: &AirParams,
        fields: &'a std::sync::OnceLock<Vec<(Field, u64)>>,
        cancelled: &'a AtomicBool,
    ) -> io::Result<Sampled<'a>> {
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
        let values = Reader::new(
            store,
            key,
            format!("air-superiority-{branch}-values"),
            cancelled,
        )?;
        let events = Reader::new(
            store,
            key,
            format!("air-superiority-{branch}-operands"),
            cancelled,
        )?;
        let fields = match fields.get() {
            Some(fields) => fields,
            None => {
                let parsed = self.fields()?;
                fields.get_or_init(|| parsed)
            }
        };
        Ok(Sampled {
            wanted,
            values,
            events,
            fields,
        })
    }
    /// Each hour `params` samples, classified in turn against `sides`, which it extends.
    #[allow(clippy::too_many_arguments)]
    fn samples(
        &self,
        store: &store::Store,
        key: &store::SourceKey,
        branch: u64,
        params: &AirParams,
        fields: &std::sync::OnceLock<Vec<(Field, u64)>>,
        groups: &dashboard_base::products::country_groups::Scan,
        anchors: &assignment::SideAnchors,
        sides: &mut Vec<AirSide>,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<Snapshot>> {
        let mut sampled = self.sampled(store, key, branch, params, fields, cancelled)?;
        let _cpu = replay_frame::cpu::scope_phase("air_superiority:query_days");
        let mut indices = catalogue_indices(sides);
        let mut samples = Vec::new();
        let mut rosters = scan::RosterRead::default();
        let view = sampled_view(params);
        values::each_hour(
            &sampled.wanted,
            sampled.fields,
            &mut sampled.values,
            &mut sampled.events,
            cancelled,
            |hour, projection, auxiliary| {
                let _cpu =
                    replay_frame::cpu::scope_phase("air_superiority:query_sample_classification");
                let mut sample = scan::prepared_snapshot(
                    hour,
                    projection,
                    groups,
                    anchors,
                    view,
                    (&mut *sides, &mut indices),
                    &mut rosters,
                )?;
                scan::apply_auxiliary(&mut sample, auxiliary);
                samples.push(sample);
                Ok(())
            },
        )?;
        Ok(samples)
    }
}

/// The hours a query samples and the series reading their inputs.
struct Sampled<'a> {
    wanted: Vec<i32>,
    values: Reader<'a>,
    events: Reader<'a>,
    fields: &'a [(Field, u64)],
}

/// The viewpoint a query classifies its samples for, and whether it is a country.
fn sampled_view(params: &AirParams) -> (Option<&str>, bool) {
    (
        (params.mode == Some(AirMode::Viewpoint))
            .then_some(params.viewpoint.as_deref())
            .flatten(),
        params.viewpoint_country == Some(true),
    )
}

/// Each side's index in the catalogue `sides`, by key.
fn catalogue_indices(sides: &[AirSide]) -> HashMap<String, u32> {
    sides
        .iter()
        .enumerate()
        .map(|(index, side)| (side.key.clone(), index as u32))
        .collect()
}

/// The country groups the published values were chosen against: the row reference's own read
/// of them where it kept one, the groups' published pages otherwise, whose coverage a native
/// preparation's replies wait for beside their own. Each read makes the groups' calls on the
/// groups' frames, so both hold the same history.
fn read_groups(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<dashboard_base::products::country_groups::Scan> {
    let _cpu = replay_frame::cpu::scope_phase("air_superiority:query_groups");
    let file = match store.read(key, &groups_section(branch), 0)? {
        Some(file) => file,
        None => store
            .read(key, &format!("country-groups-{branch}"), 0)?
            .ok_or_else(|| invalid("Prepared country groups are missing"))?,
    };
    dashboard_base::products::country_groups::Scan::read(io::BufReader::new(file))
}

/// The prepared theatre membership of the hours from `first` to `last`, each
/// read's operands until the next.
fn theatre_membership(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    first: i32,
    last: i32,
    cancelled: &AtomicBool,
) -> io::Result<scan::DaySegment> {
    let mut hours = Reader::new(
        store,
        key,
        format!("air-superiority-{branch}-hours"),
        cancelled,
    )?;
    let mut events = Reader::new(
        store,
        key,
        format!("air-superiority-{branch}-operands"),
        cancelled,
    )?;
    let mut segment = scan::DaySegment::default();
    for run in hours.runs(0, first, last)? {
        let (_, value) = events
            .latest_event(1, run.first_hour)?
            .ok_or_else(|| invalid("Prepared theatre operands are missing"))?;
        let mut auxiliary: scan::Auxiliary = serde_json::from_slice(&value)?;
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
                auxiliary = serde_json::from_slice(&row.row)?;
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
    Ok(segment)
}

/// The daily state of `branch`'s publication, kept for the source while the
/// pages it reads are unchanged.
fn daily_state(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<reader::Daily>> {
    let reads = reader::Reads {
        source: Some(source),
        store,
        key,
        branch,
        cancelled,
    };
    dashboard_base::prepared::query::observations::scope(source, "air-superiority:daily", Some(key))
        .get_entry(
            branch,
            &(),
            &reader::publication(store, key, branch)?,
            cancelled,
            reader::Daily::bytes,
            || reader::Daily::read(&reads),
        )
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
    let coverage = dashboard_base::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        "air-superiority",
        cancelled,
    )?;
    if coverage.as_ref().is_none_or(|coverage| !coverage.complete) {
        if params.view == AirView::Moment {
            return first_view::moment(source, &params, context, coverage, cancelled);
        }
        return first_view::query(source, &params, context, coverage, cancelled);
    }
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let daily = daily_state(source, &store, &key, branch, cancelled)?;
    let naming_hour = params
        .last_hour
        .unwrap_or_else(|| dashboard_text::metadata::branch_bounds(replay, start).1);
    let reply = match params.view {
        AirView::Choices => {
            params.mode = None;
            params.viewpoint = None;
            params.viewpoint_country = None;
            params.sides = None;
            let mut cache = daily.choices();
            if params
                .theatres
                .as_ref()
                .is_some_and(|theatres| !theatres.is_empty())
            {
                cache.segments.push(Arc::new(theatre_membership(
                    &store,
                    &key,
                    branch,
                    params.first_hour.unwrap_or(i32::MIN),
                    params.last_hour.unwrap_or(i32::MAX),
                    cancelled,
                )?));
            }
            cache_reply(
                source,
                replay,
                &daily.groups,
                &cache,
                &params,
                context,
                naming_hour,
                cancelled,
            )?
        }
        AirView::Moment => {
            let mut sides = Vec::new();
            let samples = daily.state.samples(
                &store,
                &key,
                branch,
                &params,
                &daily.fields,
                &daily.groups,
                &daily.anchors,
                &mut sides,
                cancelled,
            )?;
            let mut reply = AirReply::Moment(moment_reply(
                samples,
                sides,
                &daily.groups,
                &params,
                &daily.anchors,
            )?);
            resolve_names(
                source,
                &daily.groups,
                context,
                naming_hour,
                None,
                &mut reply,
            );
            check_reply_size(&reply)?;
            reply
        }
        AirView::Timeline => {
            let reads = reader::Reads {
                source: Some(source),
                store: &store,
                key: &key,
                branch,
                cancelled,
            };
            present_timeline(
                source,
                &daily.groups,
                &daily.anchors,
                context,
                naming_hour,
                reader::timeline(&daily, &reads, &params, i32::MAX)?,
                cancelled,
            )?
        }
    };
    check_cancel(cancelled)?;
    serde_json::to_value(reply).map_err(Into::into)
}

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod tests;
