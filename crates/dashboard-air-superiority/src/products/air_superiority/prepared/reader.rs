//! Timelines and side choices composed from the daily facts: a request reads
//! only its rows' days and ranks regions from running totals, with every
//! roster's keys resolved once a publication.
use super::*;
use crate::products::air_superiority::daily::{
    self, Compose, Day, DayOperands, Group, Layout, Pair, Tags, Theatres, View,
};
use dashboard_base::prepared::facts::{Facts, ReadPages, ReadPhase, Resident, Status};
use serde::de::DeserializeOwned;
use std::{borrow::Cow, sync::Mutex};

#[cfg(test)]
thread_local! {
    /// Cells composed by timeline requests, and days read live.
    pub(super) static COMPOSED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Where a request reads: the publication and, outside tests, the source
/// whose observation cache keeps what it reads once a publication.
pub(super) struct Reads<'a> {
    pub source: Option<&'a Source>,
    pub store: &'a store::Store,
    pub key: &'a store::SourceKey,
    pub branch: u64,
    pub cancelled: &'a AtomicBool,
}

impl Reads<'_> {
    fn cached<T: Send + Sync + 'static>(
        &self,
        daily: &Daily,
        name: &'static str,
        params: &impl Serialize,
        extract: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<Arc<T>> {
        match self.source {
            Some(source) => {
                dashboard_base::prepared::query::observations::scope(source, name, Some(self.key))
                    .get_entry(
                        self.branch,
                        params,
                        &daily.publication,
                        self.cancelled,
                        extract,
                    )
            }
            None => extract().map(Arc::new),
        }
    }

    fn reader(&self, section: &str) -> io::Result<Reader<'_>> {
        Reader::new(
            self.store,
            self.key,
            format!("air-superiority-{}-{section}", self.branch),
            self.cancelled,
        )
    }

    fn session(&self) -> io::Result<store::Session> {
        self.store
            .session(self.key)?
            .ok_or_else(|| invalid("Prepared air-superiority publication is missing"))
    }
}

/// A side's read without its hour: name, leader, tag, co-belligerents,
/// factions and colour faction.
type Values = (
    String,
    Option<String>,
    Option<String>,
    usize,
    Vec<(String, String)>,
    Option<String>,
);

/// A roster with its keys resolved against the branch's anchors.
pub(super) struct Resolved {
    layout: Layout,
    /// Each group's key, faction and read, by group index.
    groups: Vec<(String, Option<String>, Values)>,
    /// The lone countries whose key is not their own, by place.
    remnants: HashMap<u32, String>,
}

impl Resolved {
    fn key(&self, position: u32) -> Cow<'_, str> {
        if let Some(index) = self.layout.group_index(position) {
            return Cow::Borrowed(&self.groups[index].0);
        }
        if let Some(key) = self.remnants.get(&position) {
            return Cow::Borrowed(key);
        }
        match self.layout.side(position) {
            daily::Side::Lone(tag) => Cow::Owned(format!("c:{tag}")),
            daily::Side::Group(_) => unreachable!("groups are keyed above"),
        }
    }

    /// The side at `position`: its key, faction and read at `hour`.
    fn read(&self, position: u32, hour: i32) -> (Cow<'_, str>, Option<String>, AirSideRead) {
        let key = self.key(position);
        if let Some(index) = self.layout.group_index(position) {
            let (_, faction, values) = &self.groups[index];
            let (name, leader, tag, count, factions, colour) = values.clone();
            return (
                key,
                faction.clone(),
                (hour, name, leader, tag, count, factions, colour),
            );
        }
        let daily::Side::Lone(tag) = self.layout.side(position) else {
            unreachable!("a place outside every group is a lone country");
        };
        (
            key,
            None,
            (
                hour,
                tag.clone(),
                Some(tag.clone()),
                Some(tag.clone()),
                0,
                vec![],
                None,
            ),
        )
    }

    /// The first place, in roster order, of a side keyed `key`.
    fn position(&self, key: &str) -> Option<u32> {
        let groups = self
            .groups
            .iter()
            .zip(self.layout.groups())
            .filter(|((held, _, _), _)| held == key)
            .map(|(_, group)| group.position);
        let remnants = self
            .remnants
            .iter()
            .filter(|(_, held)| *held == key)
            .map(|(position, _)| *position);
        let lone = key
            .strip_prefix("c:")
            .and_then(|tag| self.layout.lone_position(tag))
            .filter(|position| !self.remnants.contains_key(position));
        groups.chain(remnants).chain(lone).min()
    }
}

/// The present countries' identities at an hour, as side keys name them,
/// with their historical anchors counted and the sole remnants among them.
struct Identities {
    by_tag: HashMap<String, String>,
    counts: assignment::PresentAnchors,
    remnants: Vec<String>,
}

impl Identities {
    fn read(
        groups: &dashboard_base::products::country_groups::Scan,
        anchors: &assignment::SideAnchors,
        present: &[String],
        hour: i32,
    ) -> Option<Self> {
        let mut by_tag = HashMap::with_capacity(present.len());
        for tag in present {
            by_tag.insert(tag.clone(), assignment::identity_at(groups, tag, hour)?);
        }
        let countries: BTreeSet<String> = by_tag.values().cloned().collect();
        let counts = assignment::PresentAnchors::new(&countries, anchors);
        let remnants = present
            .iter()
            .filter(|tag| counts.sole_remnant(&by_tag[*tag], anchors))
            .cloned()
            .collect();
        Some(Self {
            by_tag,
            counts,
            remnants,
        })
    }
}

/// `groups` of `present` at `hour`, keyed against `anchors` as the
/// assignment keys them.
fn resolve(
    scan: &dashboard_base::products::country_groups::Scan,
    anchors: &assignment::SideAnchors,
    groups: Vec<Group>,
    present: Tags,
    identities: &Identities,
    hour: i32,
) -> io::Result<Resolved> {
    let layout = Layout::new(groups, present)?;
    let identity = |tag: &String| identities.by_tag[tag].clone();
    let mut sides: Vec<assignment::SideKey> = layout
        .groups()
        .iter()
        .map(|group| {
            assignment::SideKey::native(
                &group.factions,
                &group.members,
                group.members.iter().map(identity).collect(),
            )
        })
        .collect();
    // Only a sole remnant of the lone countries can take a key other than its own.
    let remnants: Vec<(u32, &String)> = identities
        .remnants
        .iter()
        .filter_map(|tag| layout.lone_position(tag).map(|position| (position, tag)))
        .collect();
    sides.extend(remnants.iter().map(|(_, tag)| {
        assignment::SideKey::native(&[], std::slice::from_ref(*tag), vec![identity(tag)])
    }));
    let mut keys = assignment::resolve_present(&sides, &identities.counts, anchors).into_iter();
    let resolved = layout
        .groups()
        .iter()
        .map(|group| {
            let key = keys.next().expect("a key per side");
            let factions: Vec<(String, String)> = group
                .factions
                .iter()
                .map(|id| {
                    (
                        id.to_string(),
                        scan.faction_name_at(*id, hour).unwrap_or_default(),
                    )
                })
                .collect();
            let faction = (!group.factions.is_empty())
                .then(|| key.strip_prefix("f:").map(str::to_owned))
                .flatten();
            let name = if factions.is_empty() {
                group.members[0].clone()
            } else {
                factions
                    .iter()
                    .map(|(_, name)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            };
            let tag = faction.is_none().then(|| group.members[0].clone());
            let values = (
                name,
                group.leader.clone(),
                tag,
                group.co_belligerents as usize,
                factions,
                group.colour.map(|id| id.to_string()),
            );
            (key, faction, values)
        })
        .collect();
    let mut lone = HashMap::new();
    for ((position, tag), key) in remnants.into_iter().zip(keys) {
        if key.strip_prefix("c:") != Some(tag.as_str()) {
            lone.insert(position, key);
        }
    }
    Ok(Resolved {
        layout,
        groups: resolved,
        remnants: lone,
    })
}

/// What a day's cells are composed from.
pub(super) struct DayData {
    offset: i32,
    roster: Option<Arc<Resolved>>,
    operands: Arc<DayOperands>,
    theatres: Arc<Theatres>,
    theatre_age: Option<i32>,
    excluded: usize,
    used: Option<Vec<u32>>,
}

/// Consecutive days composed from the same data.
struct Run {
    first: i32,
    last: i32,
    data: Arc<DayData>,
}

/// A region's representatives' pairs over the days it was active.
type RegionPairs = Vec<(i32, i32, Arc<Vec<Pair>>)>;

/// A region's running sides activity: each interval's total through its
/// first day, unchanged through its last.
type Activity = Vec<(i32, i32, u128)>;

/// Activity that days add at a rate: each span's total before it, and what
/// each of its days adds.
type Spans = Vec<(i32, i32, u128, u128)>;

fn activity_through(totals: &[(i32, i32, u128)], day: i32) -> u128 {
    let at = totals.partition_point(|(first, _, _)| *first <= day);
    at.checked_sub(1).map_or(0, |at| totals[at].2)
}

fn spans_through(spans: &[(i32, i32, u128, u128)], day: i32) -> u128 {
    let at = spans.partition_point(|(first, _, _, _)| *first <= day);
    at.checked_sub(1).map_or(0, |at| {
        let (first, last, before, rate) = spans[at];
        before + rate * (i64::from(day.min(last)) - i64::from(first) + 1) as u128
    })
}

/// Add `rate` for each of the days from `first` through `last` to `spans`.
fn add_span(spans: &mut Spans, first: i32, last: i32, rate: u128) {
    if rate == 0 {
        return;
    }
    let before = spans.last().map_or(0, |&(first, last, before, rate)| {
        before + rate * (i64::from(last) - i64::from(first) + 1) as u128
    });
    match spans.last_mut() {
        Some((_, held, _, held_rate))
            if *held_rate == rate && held.checked_add(1) == Some(first) =>
        {
            *held = last
        }
        _ => spans.push((first, last, before, rate)),
    }
}

/// The catalogue's keys a live read takes.
#[derive(Default)]
struct Catalogue {
    active: Vec<(u32, u64)>,
    pairs: HashMap<String, Vec<(u32, Option<u64>, Option<u64>)>>,
}

impl Catalogue {
    fn read(fields: &[(Field, u64)]) -> Self {
        let mut active = Vec::new();
        let mut pairs: HashMap<String, BTreeMap<u32, (Option<u64>, Option<u64>)>> = HashMap::new();
        for (field, key) in fields {
            match field {
                Field::Active(region) => active.push((*region, *key)),
                Field::Friendly(tag, region) => {
                    pairs
                        .entry(tag.clone())
                        .or_default()
                        .entry(*region)
                        .or_default()
                        .0 = Some(*key)
                }
                Field::Enemy(tag, region) => {
                    pairs
                        .entry(tag.clone())
                        .or_default()
                        .entry(*region)
                        .or_default()
                        .1 = Some(*key)
                }
                Field::Wings(_) => {}
            }
        }
        active.sort_unstable();
        Self {
            active,
            pairs: pairs
                .into_iter()
                .map(|(tag, regions)| {
                    let regions = regions
                        .into_iter()
                        .map(|(region, (friendly, enemy))| (region, friendly, enemy))
                        .collect();
                    (tag, regions)
                })
                .collect(),
        }
    }
}

/// A day composed from the hourly series at one hour: the branch's open
/// day, or a requested day cut short.
pub(super) struct Live {
    hour: i32,
    data: Arc<DayData>,
    /// Each active region's representatives' pairs.
    pairs: BTreeMap<u32, Vec<Pair>>,
    /// The requested country's scores, in each active region it has them.
    country: BTreeMap<u32, (i64, i64)>,
}

/// What every request of a publication reads alike.
pub(super) struct Daily {
    pub state: Prepared,
    pub groups: dashboard_base::products::country_groups::Scan,
    pub anchors: Arc<assignment::SideAnchors>,
    /// The groups' runs by hour, for rosters a day cut short reads live.
    active: assignment::GroupIndex,
    pub fields: std::sync::OnceLock<Vec<(Field, u64)>>,
    /// The pages this state was read from, which every read it keeps names.
    publication: Vec<u8>,
    columns: Option<facts::Columns>,
    /// Consecutive runs with the same theatres, for theatre filters.
    theatres: Vec<(i32, i32, Arc<Theatres>)>,
    epochs: Vec<i32>,
    identities: Mutex<HashMap<(Tags, usize), Option<Arc<Identities>>>>,
    catalogue: Catalogue,
    runs: Vec<Run>,
    /// The open day's representatives' pairs, by active region.
    tail: Option<(i32, BTreeMap<u32, Vec<Pair>>)>,
    /// The sides view's dictionary over every day, in the order sides are first shown.
    pub dictionary: Arc<Vec<AirSide>>,
    index: HashMap<String, u32>,
    activity: HashMap<u32, Activity>,
}

fn intervals<'a, K, V, R>(
    phase: &ReadPhase<'a, R>,
    facts: &'a Facts<K, V>,
    key: &K,
    cancelled: &AtomicBool,
) -> io::Result<Vec<(i32, i32, V)>>
where
    K: Ord + Clone + Serialize + DeserializeOwned + Resident + 'static,
    V: Clone + PartialEq + Serialize + DeserializeOwned + Resident + 'static,
    R: ReadPages,
{
    let mut reader = phase.reader(facts)?;
    let mut cursor = reader.cursor(key, i32::MIN, i32::MAX)?;
    let mut values = Vec::new();
    while let Some(row) = reader.next(&mut cursor)? {
        check_cancel(cancelled)?;
        if let Status::Value(value) = &row.status {
            values.push((row.first, row.last, value.clone()));
        }
    }
    Ok(values)
}

/// A dictionary's entries, by id.
fn entries<'a, V, R>(
    phase: &ReadPhase<'a, R>,
    facts: &'a Facts<(), V>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<V>>
where
    V: Clone + PartialEq + Serialize + DeserializeOwned + Resident + 'static,
    R: ReadPages,
{
    let mut entries = Vec::new();
    // Equal entries with consecutive ids merge into one interval.
    for (first, last, value) in intervals(phase, facts, &(), cancelled)? {
        if i64::from(first) != entries.len() as i64 || last < first {
            return Err(invalid("Prepared air fact dictionary is not dense"));
        }
        for _ in first..last {
            entries.push(value.clone());
        }
        entries.push(value);
    }
    Ok(entries)
}

/// The keys of a keyed column.
fn keys<'a, V, R>(phase: &ReadPhase<'a, R>, facts: &'a Facts<u32, V>) -> io::Result<Vec<u32>>
where
    V: PartialEq + Serialize + DeserializeOwned + Resident + 'static,
    R: ReadPages,
{
    let reader = phase.reader(facts)?;
    let mut cursor = reader.keys(None, None)?;
    let mut keys = Vec::new();
    while let Some(key) = reader.next_key(&mut cursor)? {
        keys.push(*key);
    }
    Ok(keys)
}

/// Each start and end of a country identity, in order: an hour's identities
/// are those of every other hour between the same two.
fn epochs(groups: &dashboard_base::products::country_groups::Scan) -> Vec<i32> {
    let mut epochs: Vec<i32> = groups
        .identities
        .values()
        .flat_map(|runs| runs.iter())
        .flat_map(|run| [Some(run.first_hour), run.last_hour.checked_add(1)])
        .flatten()
        .collect();
    epochs.sort_unstable();
    epochs.dedup();
    epochs
}

/// The pages a publication's air superiority reads: its continuation, its
/// catalogue and its read of the groups.
pub(super) fn publication(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Vec<u8>> {
    let page = |section: String| -> io::Result<Option<[u64; 3]>> {
        Ok(store
            .read(key, &section, 0)?
            .map(|file| file.id().cache_key()))
    };
    let pages = (
        page(format!("air-superiority-{branch}"))?,
        series::catalogue_pages(store, key, "air-superiority", branch)?,
        page(super::groups_section(branch))?,
        page(format!("country-groups-{branch}"))?,
    );
    Ok(serde_json::to_vec(&pages)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::prepared::series::{Run as ValueRun, tests::Fixture};

    #[test]
    fn country_scores_apply_backdated_runs_as_point_reads_do() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut amounts = Accumulator::default();
        for (key, runs) in [
            vec![(0, 2, 1), (5, 5, 2), (0, 10, 3)],
            vec![(0, 10, 8), (2, 3, 4)],
        ]
        .into_iter()
        .enumerate()
        {
            for (first, last, value) in runs {
                amounts.push(
                    key as u64,
                    ValueRun::new(first * DAY_HOURS, last * DAY_HOURS, value, true),
                );
            }
        }
        let mut writer = fixture.writer();
        amounts
            .flush(&mut writer, "air-superiority-1-values", 0, &flag)
            .unwrap();
        writer.commit().unwrap();
        let reads = Reads {
            source: None,
            store: &fixture.store,
            key: &fixture.key,
            branch: 1,
            cancelled: &flag,
        };
        let mut daily = Daily::empty(Default::default());
        daily.catalogue = Catalogue::read(&[
            (Field::Friendly("AAA".into(), 1), 0),
            (Field::Enemy("AAA".into(), 1), 1),
            (Field::Friendly("AAA".into(), 2), 2),
        ]);
        daily.runs.push(Run {
            first: 0,
            last: 11,
            data: Arc::new(DayData {
                offset: 0,
                roster: None,
                operands: Arc::new(DayOperands {
                    identity_known: true,
                    present: Arc::default(),
                    witnessed: Arc::default(),
                    invalid: Arc::default(),
                    complete: true,
                    unresolved: false,
                }),
                theatres: Arc::new(Theatres::read(false, &BTreeMap::new())),
                theatre_age: None,
                excluded: 0,
                used: None,
            }),
        });
        let scores = daily.country_scores(&reads, "AAA").unwrap();
        let mut values = reads.reader("values").unwrap();
        for (day, hour) in daily.sample_hours() {
            let raw = values.values_at(&[0, 1], hour).unwrap();
            let expected = match (raw[0], raw[1]) {
                (None, None) => None,
                (friendly, enemy) => Some((
                    i64::try_from(friendly.unwrap_or(0)).unwrap(),
                    i64::try_from(enemy.unwrap_or(0)).unwrap(),
                )),
            };
            assert_eq!(scores.at(1, day), expected, "day {day}");
            assert_eq!(scores.at(2, day), None, "day {day}");
        }
        assert_eq!(scores.at(1, -1), None);
        assert_eq!(scores.at(1, 12), None);
    }
}

impl Daily {
    fn epoch(&self, hour: i32) -> usize {
        self.epochs.partition_point(|boundary| *boundary <= hour)
    }

    /// The identities of `present` at `hour`, kept for later reads only while
    /// the state is built (`keep`): its charge is measured once built.
    fn identities(&self, present: &Tags, hour: i32, keep: bool) -> Option<Arc<Identities>> {
        let key = (Arc::clone(present), self.epoch(hour));
        let mut held = self
            .identities
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(identities) = held.get(&key) {
            return identities.clone();
        }
        let identities = Identities::read(&self.groups, &self.anchors, present, hour).map(Arc::new);
        if keep {
            held.insert(key, identities.clone());
        }
        identities
    }

    /// No prepared day: before the first publication.
    pub fn empty(groups: dashboard_base::products::country_groups::Scan) -> Self {
        Self {
            state: Prepared::default(),
            anchors: Arc::new(assignment::historical_anchors(&groups)),
            active: assignment::GroupIndex::new(&groups),
            groups,
            fields: std::sync::OnceLock::new(),
            publication: Vec::new(),
            theatres: Vec::new(),
            columns: None,
            epochs: Vec::new(),
            identities: Mutex::default(),
            catalogue: Catalogue::default(),
            runs: Vec::new(),
            tail: None,
            dictionary: Arc::default(),
            index: HashMap::new(),
            activity: HashMap::new(),
        }
    }

    pub fn read(reads: &Reads<'_>) -> io::Result<Self> {
        let _cpu = replay_frame::cpu::scope_phase("air_superiority:query_daily");
        let (store, key, branch, cancelled) =
            (reads.store, reads.key, reads.branch, reads.cancelled);
        let state = Prepared::read_query(store, key, branch)?;
        let groups = read_groups(store, key, branch)?;
        let mut daily = Self::empty(groups);
        daily.publication = publication(store, key, branch)?;
        daily.epochs = epochs(&daily.groups);
        let fields = state.fields()?;
        daily.catalogue = Catalogue::read(&fields);
        let _ = daily.fields.set(fields);
        daily.state = state;
        if let Some(saved) = daily.state.facts.checkpoints().cloned() {
            let mut session = reads.session()?;
            let columns = facts::Columns::restore(&mut session, &saved)?;
            let phase = ReadPhase::new(&mut session);
            let sets: Vec<Tags> = entries(&phase, &columns.sets, cancelled)?
                .into_iter()
                .map(|set: Vec<String>| set.into())
                .collect();
            let set = |id: u32| {
                sets.get(id as usize)
                    .cloned()
                    .ok_or_else(|| invalid("Prepared air fact names no tag set"))
            };
            let rosters: Vec<daily::Roster> = entries(&phase, &columns.rosters, cancelled)?;
            let operands = entries(&phase, &columns.operands, cancelled)?
                .into_iter()
                .map(|operands: daily::Operands| {
                    Ok(Arc::new(DayOperands {
                        identity_known: operands.identity_known,
                        present: set(operands.present)?,
                        witnessed: set(operands.witnessed)?,
                        invalid: set(operands.invalid)?,
                        complete: operands.complete,
                        unresolved: operands.unresolved,
                    }))
                })
                .collect::<io::Result<Vec<_>>>()?;
            let theatres: Vec<Arc<Theatres>> = entries(&phase, &columns.theatres, cancelled)?
                .into_iter()
                .map(Arc::new)
                .collect();
            let days: Vec<(i32, i32, Day)> = intervals(&phase, &columns.days, &(), cancelled)?;
            for region in keys(&phase, &columns.activity)? {
                let totals = intervals(&phase, &columns.activity, &region, cancelled)?;
                daily.activity.insert(region, totals);
            }
            drop(phase);
            daily.columns = Some(columns);
            daily.runs = daily.runs_of(days, &rosters, &set, &operands, &theatres, cancelled)?;
        }
        if let Some(hour) = daily.state.previous_hour {
            let day = hour.div_euclid(DAY_HOURS);
            if daily.runs.last().is_none_or(|run| run.last < day)
                && let Some(live) = daily.live(reads, hour, None, true)?
            {
                let compose = Compose::new(
                    live.data.roster.as_ref().map(|roster| &roster.layout),
                    &live.data.operands,
                    View::Sides,
                );
                for (region, pairs) in &live.pairs {
                    let totals = daily.activity.entry(*region).or_default();
                    let before = activity_through(totals, day);
                    totals.push((day, day, before + compose.activity(pairs)));
                }
                drop(compose);
                daily.runs.push(Run {
                    first: day,
                    last: day,
                    data: Arc::clone(&live.data),
                });
                daily.tail = Some((day, live.pairs));
            }
        }
        let mut sides = Vec::new();
        let mut index = HashMap::new();
        // A run's later days show the sides its first does, with the same reads.
        for run in &daily.runs {
            note_uses(
                &mut sides,
                &mut index,
                &run.data,
                run.first * DAY_HOURS + run.data.offset,
            );
        }
        daily.dictionary = Arc::new(sides);
        daily.index = index;
        for run in &daily.runs {
            match daily.theatres.last_mut() {
                Some((_, last, theatres))
                    if Arc::ptr_eq(theatres, &run.data.theatres)
                        && last.checked_add(1) == Some(run.first) =>
                {
                    *last = run.last
                }
                _ => daily
                    .theatres
                    .push((run.first, run.last, Arc::clone(&run.data.theatres))),
            }
        }
        Ok(daily)
    }

    fn runs_of(
        &self,
        days: Vec<(i32, i32, Day)>,
        rosters: &[daily::Roster],
        set: &dyn Fn(u32) -> io::Result<Tags>,
        operands: &[Arc<DayOperands>],
        theatres: &[Arc<Theatres>],
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<Run>> {
        let mut resolved: HashMap<(u32, usize), Option<Arc<Resolved>>> = HashMap::new();
        let mut runs = Vec::new();
        for (first, last, day) in days {
            check_cancel(cancelled)?;
            let operands = operands
                .get(day.operands as usize)
                .cloned()
                .ok_or_else(|| invalid("Prepared air day names no operands"))?;
            let theatres = theatres
                .get(day.theatres as usize)
                .cloned()
                .ok_or_else(|| invalid("Prepared air day names no theatres"))?;
            let mut from = first;
            while from <= last {
                let hour = from * DAY_HOURS + day.offset;
                let epoch = self.epoch(hour);
                // The last day sampled before the next identity boundary.
                let to = self.epochs.get(epoch).map_or(last, |boundary| {
                    last.min((boundary - 1 - day.offset).div_euclid(DAY_HOURS))
                });
                let roster = match day.roster {
                    None => None,
                    Some(id) => {
                        if !resolved.contains_key(&(id, epoch)) {
                            let roster = rosters
                                .get(id as usize)
                                .ok_or_else(|| invalid("Prepared air day names no roster"))?;
                            let present = set(roster.present)?;
                            let value = match self.identities(&present, hour, true) {
                                Some(identities) => Some(Arc::new(resolve(
                                    &self.groups,
                                    &self.anchors,
                                    roster.groups.clone(),
                                    present,
                                    &identities,
                                    hour,
                                )?)),
                                None => None,
                            };
                            resolved.insert((id, epoch), value);
                        }
                        resolved[&(id, epoch)].clone()
                    }
                };
                // A country without identity in the groups leaves no sides recorded.
                let unrecorded = day.roster.is_some() && roster.is_none();
                runs.push(Run {
                    first: from,
                    last: to,
                    data: Arc::new(DayData {
                        offset: day.offset,
                        roster,
                        operands: Arc::clone(&operands),
                        theatres: Arc::clone(&theatres),
                        theatre_age: day.theatre_age,
                        excluded: if unrecorded { 0 } else { day.excluded as usize },
                        used: if unrecorded {
                            Some(vec![])
                        } else {
                            day.used.clone()
                        },
                    }),
                });
                from = to + 1;
            }
        }
        Ok(runs)
    }

    /// The theatre periods overlapping the days from `first` through `last`.
    fn theatre_periods(&self, first: i32, last: i32) -> &[(i32, i32, Arc<Theatres>)] {
        let from = self.theatres.partition_point(|(_, end, _)| *end < first);
        let to = self
            .theatres
            .partition_point(|(start, _, _)| *start <= last);
        &self.theatres[from..to.max(from)]
    }

    /// The side viewpoint `key`'s dictionary: its reads on every day it is
    /// present, as its view classifies them, built once a publication.
    fn viewpoint_sides(
        &self,
        reads: &Reads<'_>,
        key: &str,
    ) -> io::Result<Arc<(Vec<AirSide>, HashMap<String, u32>)>> {
        reads.cached(
            self,
            "air-superiority:viewpoint",
            &key,
            || {
                let mut sides = Vec::new();
                let mut index = HashMap::new();
                for run in &self.runs {
                    if let Some(roster) = &run.data.roster
                        && let Some(position) = roster.position(key)
                    {
                        let (key, faction, read) =
                            roster.read(position, run.first * DAY_HOURS + run.data.offset);
                        scan::dictionary_read(&mut sides, &mut index, &key, faction, read);
                    }
                }
                Ok((sides, index))
            },
        )
    }

    /// The run holding `day`.
    fn run(&self, day: i32) -> Option<&Run> {
        let at = self.runs.partition_point(|run| run.last < day);
        self.runs.get(at).filter(|run| run.first <= day)
    }

    /// The day composed at `hour` from the hourly series, with `country`'s
    /// scores for a country view; None where no operands precede it.
    fn live(
        &self,
        reads: &Reads<'_>,
        hour: i32,
        country: Option<&str>,
        keep: bool,
    ) -> io::Result<Option<Live>> {
        #[cfg(test)]
        COMPOSED.with(|count| count.set(count.get() + 1));
        let mut values = reads.reader("values")?;
        let mut events = reads.reader("operands")?;
        let fields: Vec<(Field, u64)> = self
            .catalogue
            .active
            .iter()
            .map(|(region, key)| (Field::Active(*region), *key))
            .collect();
        let mut read = None;
        values::each_hour(
            &[hour],
            &fields,
            &mut values,
            &mut events,
            reads.cancelled,
            |_, projection, auxiliary| {
                read = Some((
                    projection.operands.clone(),
                    projection.active.clone(),
                    auxiliary.theatres_known,
                    Arc::clone(&auxiliary.theatres),
                    auxiliary.theatre_read,
                ));
                Ok(())
            },
        )?;
        let Some((operands, active, theatres_known, theatres, theatre_read)) = read else {
            return Ok(None);
        };
        let present: Tags = operands.present.iter().cloned().collect();
        let roster = match operands
            .identity_known
            .then(|| {
                assignment::indexed_roster(&self.groups, &self.active, hour, &operands.present)
            })
            .flatten()
        {
            Some(sides) => match self.identities(&present, hour, keep) {
                Some(identities) => Some(Arc::new(resolve(
                    &self.groups,
                    &self.anchors,
                    daily::roster(&sides),
                    Arc::clone(&present),
                    &identities,
                    hour,
                )?)),
                None => None,
            },
            None => None,
        };
        let day_operands = Arc::new(DayOperands {
            identity_known: operands.identity_known,
            present,
            witnessed: operands.witnessed.iter().cloned().collect(),
            invalid: operands.invalid.iter().cloned().collect(),
            complete: operands.complete,
            unresolved: operands.unresolved,
        });
        let mut wanted: Vec<&str> = Vec::new();
        if let Some(roster) = &roster {
            let layout = &roster.layout;
            for position in 0..layout.len() {
                match layout.side(position) {
                    daily::Side::Lone(tag) => wanted.push(tag),
                    daily::Side::Group(group) => match &group.leader {
                        Some(leader) => wanted.push(leader),
                        None if group.factions.is_empty() => {
                            wanted.extend(group.members.iter().map(String::as_str))
                        }
                        None => {}
                    },
                }
            }
        }
        let representatives = wanted.len();
        wanted.extend(country);
        let mut keys = Vec::new();
        let mut slots = Vec::new();
        for (at, tag) in wanted.iter().enumerate() {
            for (region, friendly, enemy) in self.catalogue.pairs.get(*tag).into_iter().flatten() {
                if active.contains(region) {
                    let mut slot = |key: &Option<u64>| {
                        key.map(|key| {
                            keys.push(key);
                            keys.len() - 1
                        })
                    };
                    let friendly = slot(friendly);
                    let enemy = slot(enemy);
                    slots.push((at, *region, friendly, enemy));
                }
            }
        }
        let amounts = values.values_at(&keys, hour)?;
        let mut pairs: BTreeMap<u32, Vec<Pair>> =
            active.iter().map(|region| (*region, Vec::new())).collect();
        let mut scores = BTreeMap::new();
        for (at, region, friendly, enemy) in slots {
            let friendly = friendly.and_then(|at| amounts[at]);
            let enemy = enemy.and_then(|at| amounts[at]);
            // A pair holds what either score holds, the other side as zero.
            if friendly.is_none() && enemy.is_none() {
                continue;
            }
            let score = |value: Option<i128>, refusal: &'static str| {
                i64::try_from(value.unwrap_or(0)).map_err(|_| invalid(refusal))
            };
            let value = (
                score(friendly, "Prepared friendly score is invalid")?,
                score(enemy, "Prepared enemy score is invalid")?,
            );
            if at < representatives {
                pairs
                    .get_mut(&region)
                    .expect("an active region")
                    .push((wanted[at].to_owned(), value));
            } else {
                scores.insert(region, value);
            }
        }
        for region in pairs.values_mut() {
            region.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        }
        let compose = Compose::new(
            roster.as_ref().map(|roster| &roster.layout),
            &day_operands,
            View::Sides,
        );
        let used = compose.uses(pairs.values().map(Vec::as_slice));
        let excluded = compose.excluded();
        let shows_all = compose.shows_all();
        drop(compose);
        let data = Arc::new(DayData {
            offset: hour - hour.div_euclid(DAY_HOURS) * DAY_HOURS,
            roster,
            operands: day_operands,
            theatres: Arc::new(Theatres::read(theatres_known, &theatres)),
            theatre_age: theatre_read.map(|read| hour - read),
            excluded,
            used: (!shows_all).then_some(used),
        });
        Ok(Some(Live {
            hour,
            data,
            pairs,
            country: scores,
        }))
    }
}

/// Add each side `data` first shows in the sides view to the dictionary,
/// read at `hour`, as classifying the day does.
fn note_uses(
    sides: &mut Vec<AirSide>,
    index: &mut HashMap<String, u32>,
    data: &DayData,
    hour: i32,
) {
    let Some(roster) = &data.roster else {
        return;
    };
    let mut note = |position: u32| {
        let (key, faction, read) = roster.read(position, hour);
        scan::dictionary_read(sides, index, &key, faction, read);
    };
    match &data.used {
        Some(used) => used.iter().copied().for_each(&mut note),
        None => (0..roster.layout.len()).for_each(note),
    }
}

/// What a request composes: its view and side filter.
#[derive(Clone, Copy)]
enum RequestView<'a> {
    Sides,
    Side(&'a str),
    Country(&'a str),
}

fn request_view(params: &AirParams) -> RequestView<'_> {
    match sampled_view(params) {
        (None, _) => RequestView::Sides,
        (Some(tag), true) => RequestView::Country(tag),
        (Some(key), false) => RequestView::Side(key),
    }
}

fn compose<'d>(view: RequestView<'d>, data: &'d DayData) -> Compose<'d> {
    let layout = data.roster.as_ref().map(|roster| &roster.layout);
    let view = match view {
        RequestView::Sides => View::Sides,
        RequestView::Country(tag) => View::Country(tag),
        RequestView::Side(key) => {
            View::Side(data.roster.as_ref().and_then(|roster| roster.position(key)))
        }
    };
    Compose::new(layout, &data.operands, view)
}

/// A column of the reply: its sample hour and data, and the live read when
/// the day is cut short.
#[derive(Clone, Copy)]
struct Column<'d> {
    hour: i32,
    data: &'d DayData,
    live: Option<&'d Live>,
}

/// The sides a reply's cells name: the publication's dictionary, with a live
/// day's reads and sides added.
struct Sides<'d> {
    base: &'d [AirSide],
    index: &'d HashMap<String, u32>,
    /// The reply's first and last read hours: a changed side keeps only the reads between.
    first: i32,
    last: i32,
    changed: HashMap<u32, AirSide>,
    added: Vec<AirSide>,
    added_index: HashMap<String, u32>,
}

impl<'d> Sides<'d> {
    fn new(base: &'d [AirSide], index: &'d HashMap<String, u32>, first: i32, last: i32) -> Self {
        Self {
            base,
            index,
            first,
            last,
            changed: HashMap::new(),
            added: Vec::new(),
            added_index: HashMap::new(),
        }
    }

    fn get(&self, index: u32) -> &AirSide {
        if let Some(side) = self.changed.get(&index) {
            return side;
        }
        let base = self.base.len() as u32;
        if index < base {
            &self.base[index as usize]
        } else {
            &self.added[(index - base) as usize]
        }
    }

    fn index(&self, key: &str) -> Option<u32> {
        self.index
            .get(key)
            .or_else(|| self.added_index.get(key))
            .copied()
    }

    /// Add `read` of the side keyed `key`, as the dictionary adds it.
    fn note(&mut self, key: &str, faction: Option<String>, read: AirSideRead) -> u32 {
        let base = self.base.len();
        match self.index(key) {
            Some(index) if (index as usize) < base => {
                if !self.changed.contains_key(&index) {
                    let held = &self.base[index as usize];
                    let at = held.reads.partition_point(|old| old.0 <= read.0);
                    if at > 0 && {
                        let old = &held.reads[at - 1];
                        (&old.1, &old.2, &old.3, old.4, &old.5, &old.6)
                            == (&read.1, &read.2, &read.3, read.4, &read.5, &read.6)
                    } {
                        return index;
                    }
                    // Only the reads the reply's hours select: the one before its first and those through its last.
                    let from = held
                        .reads
                        .partition_point(|old| old.0 <= self.first)
                        .saturating_sub(1);
                    let to = held.reads.partition_point(|old| old.0 <= self.last);
                    let side = AirSide {
                        key: held.key.clone(),
                        faction: held.faction.clone(),
                        colour_faction: held.colour_faction.clone(),
                        tag: held.tag.clone(),
                        name: held.name.clone(),
                        leader: held.leader.clone(),
                        co_belligerents: held.co_belligerents,
                        factions: held.factions.clone(),
                        reads: held.reads[from..to.max(from)].to_vec(),
                    };
                    self.changed.insert(index, side);
                }
                scan::note_read(self.changed.get_mut(&index).expect("a changed side"), read);
                index
            }
            Some(index) => {
                scan::note_read(&mut self.added[index as usize - base], read);
                index
            }
            None => {
                let index = (base + self.added.len()) as u32;
                let mut indices = HashMap::new();
                scan::dictionary_read(&mut self.added, &mut indices, key, faction, read);
                self.added_index.insert(key.to_owned(), index);
                index
            }
        }
    }
}

/// How a request ranks regions: totals of the days from one day through
/// another.
enum Totals<'d> {
    /// The sides view, unfiltered.
    Activity(&'d HashMap<u32, Activity>),
    /// The sides view's scores of the selected sides, by dictionary index.
    Sides(Arc<HashMap<u32, HashMap<u32, Spans>>>, Vec<u32>),
    /// A viewpoint's scores.
    View(Arc<HashMap<u32, Spans>>),
}

impl Totals<'_> {
    fn between(&self, region: u32, first: i32, last: i32) -> u128 {
        if first > last {
            return 0;
        }
        let before = first.saturating_sub(1);
        match self {
            Self::Activity(totals) => totals.get(&region).map_or(0, |totals| {
                activity_through(totals, last) - activity_through(totals, before)
            }),
            Self::Sides(totals, indices) => totals.get(&region).map_or(0, |sides| {
                indices
                    .iter()
                    .filter_map(|index| sides.get(index))
                    .map(|spans| spans_through(spans, last) - spans_through(spans, before))
                    .sum()
            }),
            Self::View(totals) => totals.get(&region).map_or(0, |spans| {
                spans_through(spans, last) - spans_through(spans, before)
            }),
        }
    }
}

/// A country's scores at each day's sample, by region: runs of days with
/// equal scores, None where it had none.
#[derive(Default)]
struct CountryScores {
    regions: HashMap<u32, Vec<(i32, i32, Option<(i64, i64)>)>>,
}

impl CountryScores {
    fn at(&self, region: u32, day: i32) -> Option<(i64, i64)> {
        let runs = self.regions.get(&region)?;
        let at = runs.partition_point(|(_, last, _)| *last < day);
        runs.get(at)
            .filter(|(first, _, _)| *first <= day)
            .and_then(|(_, _, value)| *value)
    }
}

impl Daily {
    /// Each day's sample hour, in order.
    fn sample_hours(&self) -> Vec<(i32, i32)> {
        self.runs
            .iter()
            .flat_map(|run| {
                (run.first..=run.last).map(|day| (day, day * DAY_HOURS + run.data.offset))
            })
            .collect()
    }

    /// `tag`'s scores at every day's sample, read from the hourly series.
    fn country_scores(&self, reads: &Reads<'_>, tag: &str) -> io::Result<Arc<CountryScores>> {
        reads.cached(
            self,
            "air-superiority:country",
            &tag,
            || {
                let samples = self.sample_hours();
                let hours: Vec<i32> = samples.iter().map(|(_, hour)| *hour).collect();
                let regions = self.catalogue.pairs.get(tag).map_or(&[][..], Vec::as_slice);
                let mut keys = Vec::new();
                for (_, friendly, enemy) in regions {
                    keys.extend(friendly.iter().chain(enemy.iter()).copied());
                }
                let changes = reads.reader("values")?.changes_at(&keys, &hours)?;
                let mut changes = changes.into_iter();
                let mut scores = CountryScores::default();
                for (region, friendly, enemy) in regions {
                    let mut take = |key: &Option<u64>| {
                        key.map(|_| changes.next().expect("a change list per key"))
                    };
                    let friendly = take(friendly).unwrap_or_default();
                    let enemy = take(enemy).unwrap_or_default();
                    let (mut friendly, mut enemy) = (
                        friendly.into_iter().peekable(),
                        enemy.into_iter().peekable(),
                    );
                    let (mut held_friendly, mut held_enemy) = (None, None);
                    let mut runs: Vec<(i32, i32, Option<(i64, i64)>)> = Vec::new();
                    for (at, (day, _)) in samples.iter().enumerate() {
                        while let Some((_, value)) = friendly.next_if(|(changed, _)| *changed <= at)
                        {
                            held_friendly = value;
                        }
                        while let Some((_, value)) = enemy.next_if(|(changed, _)| *changed <= at) {
                            held_enemy = value;
                        }
                        let value = match (held_friendly, held_enemy) {
                            (None, None) => None,
                            (friendly, enemy) => Some((
                                i64::try_from(friendly.unwrap_or(0))
                                    .map_err(|_| invalid("Prepared friendly score is invalid"))?,
                                i64::try_from(enemy.unwrap_or(0))
                                    .map_err(|_| invalid("Prepared enemy score is invalid"))?,
                            )),
                        };
                        match runs.last_mut() {
                            Some((_, last, held))
                                if *held == value && last.checked_add(1) == Some(*day) =>
                            {
                                *last = *day
                            }
                            _ => runs.push((*day, *day, value)),
                        }
                    }
                    scores.regions.insert(*region, runs);
                }
                Ok(scores)
            },
        )
    }

    /// Visit each region's representatives' pairs over every day it was
    /// active, the open day's last.
    fn each_region_pairs(
        &self,
        reads: &Reads<'_>,
        mut visit: impl FnMut(u32, &[(i32, i32, Arc<Vec<Pair>>)]) -> io::Result<()>,
    ) -> io::Result<()> {
        let read = self.read_pairs(reads)?;
        let mut tails: BTreeMap<u32, RegionPairs> = BTreeMap::new();
        if let Some((day, tail)) = &self.tail {
            for (region, pairs) in tail {
                let mut held = read.get(region).cloned().unwrap_or_default();
                held.push((*day, *day, Arc::new(pairs.clone())));
                tails.insert(*region, held);
            }
        }
        let regions: BTreeSet<u32> = read.keys().chain(tails.keys()).copied().collect();
        for region in regions {
            check_cancel(reads.cancelled)?;
            let pairs = tails
                .get(&region)
                .or_else(|| read.get(&region))
                .map_or(&[][..], Vec::as_slice);
            visit(region, pairs)?;
        }
        Ok(())
    }

    /// Visit each day of `pairs` with the run holding it, a run's days at a time.
    fn each_span<'s>(
        &'s self,
        pairs: &[(i32, i32, Arc<Vec<Pair>>)],
        mut visit: impl FnMut(i32, i32, &'s DayData, &[Pair]),
    ) {
        for (first, last, pairs) in pairs {
            let mut day = *first;
            while day <= *last {
                let at = self.runs.partition_point(|run| run.last < day);
                let Some(run) = self.runs.get(at) else {
                    break;
                };
                if run.first > *last {
                    break;
                }
                let from = day.max(run.first);
                let to = (*last).min(run.last);
                visit(from, to, &run.data, pairs);
                day = to + 1;
            }
        }
    }

    /// The ranking totals of `view` filtered to the sides `keys`.
    fn totals<'d>(
        &'d self,
        reads: &Reads<'_>,
        view: RequestView<'_>,
        keys: Option<&HashSet<&str>>,
    ) -> io::Result<Totals<'d>> {
        match view {
            RequestView::Sides => {
                let Some(keys) = keys else {
                    return Ok(Totals::Activity(&self.activity));
                };
                let totals = reads.cached(
                    self,
                    "air-superiority:side-totals",
                    &(),
                    || {
                        let mut totals: HashMap<u32, HashMap<u32, Spans>> = HashMap::new();
                        let mut composed: HashMap<*const DayData, Compose> = HashMap::new();
                        self.each_region_pairs(reads, |region, pairs| {
                            let sides = totals.entry(region).or_default();
                            self.each_span(pairs, |first, last, data, pairs| {
                                let compose = composed
                                    .entry(data as *const DayData)
                                    .or_insert_with(|| compose(RequestView::Sides, data));
                                let roster = data.roster.as_ref();
                                compose.scores(pairs, |position, score| {
                                    let roster = roster.expect("a scored side has a roster");
                                    let key = roster.key(position);
                                    if let Some(index) = self.index.get(key.as_ref()) {
                                        add_span(
                                            sides.entry(*index).or_default(),
                                            first,
                                            last,
                                            score,
                                        );
                                    }
                                });
                            });
                            Ok(())
                        })?;
                        Ok(totals)
                    },
                )?;
                let indices = keys
                    .iter()
                    .filter_map(|key| self.index.get(*key).copied())
                    .collect();
                Ok(Totals::Sides(totals, indices))
            }
            RequestView::Side(viewpoint) => {
                let totals = reads.cached(
                    self,
                    "air-superiority:view-totals",
                    &viewpoint,
                    || {
                        let mut totals: HashMap<u32, Spans> = HashMap::new();
                        let mut composed: HashMap<*const DayData, Compose> = HashMap::new();
                        self.each_region_pairs(reads, |region, pairs| {
                            let spans = totals.entry(region).or_default();
                            self.each_span(pairs, |first, last, data, pairs| {
                                let compose = composed
                                    .entry(data as *const DayData)
                                    .or_insert_with(|| compose(RequestView::Side(viewpoint), data));
                                add_span(spans, first, last, compose.view_activity(pairs));
                            });
                            Ok(())
                        })?;
                        Ok(totals)
                    },
                )?;
                Ok(Totals::View(totals))
            }
            RequestView::Country(tag) => {
                let scores = self.country_scores(reads, tag)?;
                let totals = reads.cached(
                    self,
                    "air-superiority:country-totals",
                    &tag,
                    || {
                        let mut totals: HashMap<u32, Spans> = HashMap::new();
                        let mut composed: HashMap<*const DayData, Compose> = HashMap::new();
                        for region in scores.regions.keys() {
                            let spans = totals.entry(*region).or_default();
                            for (first, last, _) in self.activity.get(region).into_iter().flatten()
                            {
                                for day in *first..=*last {
                                    let Some(run) = self.run(day) else {
                                        continue;
                                    };
                                    let pair: Vec<Pair> = scores
                                        .at(*region, day)
                                        .map(|value| (tag.to_owned(), value))
                                        .into_iter()
                                        .collect();
                                    let compose =
                                        composed.entry(Arc::as_ptr(&run.data)).or_insert_with(
                                            || compose(RequestView::Country(tag), &run.data),
                                        );
                                    add_span(spans, day, day, compose.view_activity(&pair));
                                }
                            }
                        }
                        Ok(totals)
                    },
                )?;
                Ok(Totals::View(totals))
            }
        }
    }

    /// Every region's representatives' pairs over every day it was active,
    /// read once a publication; the open day's are the tail's.
    fn read_pairs(&self, reads: &Reads<'_>) -> io::Result<Arc<BTreeMap<u32, RegionPairs>>> {
        let Some(columns) = &self.columns else {
            return Ok(Arc::default());
        };
        reads.cached(
            self,
            "air-superiority:pairs",
            &(),
            || {
                let mut session = reads.session()?;
                let phase = ReadPhase::new(&mut session);
                let mut regions = BTreeMap::new();
                for region in keys(&phase, &columns.pairs)? {
                    let pairs = intervals(&phase, &columns.pairs, &region, reads.cancelled)?
                        .into_iter()
                        .map(|(first, last, pairs)| (first, last, Arc::new(pairs)))
                        .collect();
                    regions.insert(region, pairs);
                }
                Ok(regions)
            },
        )
    }

    /// The side choices: every side the sides view shows, as last read.
    pub fn choices(&self) -> DayCache {
        DayCache {
            regions: self.state.regions.clone(),
            countries: self.state.countries.clone(),
            theatres: self.state.theatres.clone(),
            sides: Arc::clone(&self.dictionary),
            anchors: Arc::clone(&self.anchors),
            warnings: self.groups.warnings.clone(),
            available: self.state.available,
            ..Default::default()
        }
    }
}

fn push(cells: &mut Vec<(usize, AirCell)>, cell: AirCell) {
    match cells.last_mut() {
        Some((repeat, old)) if *old == cell => *repeat += 1,
        _ => cells.push((1, cell)),
    }
}

/// The last recorded hour from `first` through `last`.
fn last_recorded(reads: &Reads<'_>, first: i32, last: i32) -> io::Result<Option<i32>> {
    Ok(reads
        .reader("hours")?
        .runs(0, first, last)?
        .last()
        .map(|run| run.last_hour.min(last)))
}

/// Where a cell's region pairs come from: a run of the region's pairs, none
/// (the region was not active), or one day's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pairing {
    Run(usize),
    Inactive,
    Day,
}

/// The timeline reply of `params`, answered through `through` (the
/// prepared prefix's last hour; every later day is pending).
pub(super) fn timeline(
    daily: &Daily,
    reads: &Reads<'_>,
    params: &AirParams,
    through: i32,
) -> io::Result<AirTimelineReply> {
    let _cpu = replay_frame::cpu::scope_phase("air_superiority:query_timeline");
    let (first, last) = hours(params)?;
    let first_day = first.div_euclid(DAY_HOURS);
    let last_day = last.div_euclid(DAY_HOURS);
    let first_day_hour = first_day
        .checked_mul(DAY_HOURS)
        .ok_or_else(|| invalid("Air timeline start is outside the calendar range"))?;
    let count = (i64::from(last_day) - i64::from(first_day) + 1) as usize;
    // Even a one-byte date needs a separator. Refuse only a proven JSON
    // lower bound before allocating the requested calendar, never a cell cap.
    if count.saturating_mul(2).saturating_sub(1) > MAX_PRODUCT_REPLY_BYTES {
        return Err(reply_budget_error());
    }
    let view = request_view(params);
    let keys = side_filter(params);
    let theatres = params
        .theatres
        .as_deref()
        .filter(|theatres| !theatres.is_empty());
    let country = match view {
        RequestView::Country(tag) => Some(tag),
        _ => None,
    };
    let answered = last.min(through);
    // A day cut short is read from the hourly series at its last recorded hour.
    let mut live = None;
    if answered >= first_day_hour {
        let day = answered.div_euclid(DAY_HOURS);
        if daily
            .run(day)
            .is_some_and(|run| day * DAY_HOURS + run.data.offset > answered)
            && let Some(hour) = last_recorded(reads, day * DAY_HOURS, answered)?
        {
            live = daily.live(reads, hour, country, false)?;
        }
    }
    let mut columns: Vec<Option<Column>> = vec![None; count];
    for (at, column) in columns.iter_mut().enumerate() {
        let day = first_day + at as i32;
        if day * DAY_HOURS > answered {
            break;
        }
        if let Some(live) = live
            .as_ref()
            .filter(|live| live.hour.div_euclid(DAY_HOURS) == day)
        {
            *column = Some(Column {
                hour: live.hour,
                data: &live.data,
                live: Some(live),
            });
        } else if let Some(run) = daily.run(day) {
            let hour = day * DAY_HOURS + run.data.offset;
            if hour <= answered {
                *column = Some(Column {
                    hour,
                    data: &run.data,
                    live: None,
                });
            }
        }
    }
    let samples = || columns.iter().flatten();
    // A column counts once its day is over.
    let counted = if last.rem_euclid(DAY_HOURS) == DAY_HOURS - 1 {
        last_day
    } else {
        last_day - 1
    };
    let periods = daily.theatre_periods(first_day, counted);
    let mut selected: HashMap<*const Theatres, HashSet<u32>> = HashMap::new();
    if let Some(ids) = theatres {
        for theatres in samples()
            .map(|column| &column.data.theatres)
            .chain(periods.iter().map(|(_, _, theatres)| theatres))
        {
            selected
                .entry(Arc::as_ptr(theatres))
                .or_insert_with(|| theatres.selected(ids));
        }
    }
    let mut candidates: BTreeSet<u32> = params
        .regions
        .as_ref()
        .map(|regions| regions.iter().copied().collect())
        .unwrap_or_else(|| daily.state.regions.clone());
    if theatres.is_some() && samples().all(|column| column.data.theatres.known) {
        let allowed: HashSet<u32> = samples()
            .flat_map(|column| &selected[&Arc::as_ptr(&column.data.theatres)])
            .copied()
            .collect();
        candidates.retain(|region| allowed.contains(region));
    }
    let totals = daily.totals(reads, view, keys.as_ref())?;
    // The counted days of each theatre period that knows its theatres, with their regions.
    let spans: Vec<(i32, i32, Option<&HashSet<u32>>)> = match theatres {
        None => vec![(first_day, counted, None)],
        Some(_) => periods
            .iter()
            .filter(|(_, _, theatres)| theatres.known)
            .map(|(first, last, theatres)| {
                (
                    (*first).max(first_day),
                    (*last).min(counted),
                    selected.get(&Arc::as_ptr(theatres)),
                )
            })
            .collect(),
    };
    let mut ranked: Vec<(u32, u128)> = candidates
        .into_iter()
        .map(|region| {
            let total = spans
                .iter()
                .filter(|(_, _, regions)| regions.is_none_or(|regions| regions.contains(&region)))
                .map(|(first, last, _)| totals.between(region, *first, *last))
                .sum::<u128>();
            (region, total)
        })
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if params.regions.is_none() {
        ranked.truncate(params.top.unwrap_or(10));
    }
    let empty = HashMap::new();
    let first_read = samples().map(|column| column.hour).min().unwrap_or(last);
    let viewpoint = match view {
        RequestView::Side(key) => Some(daily.viewpoint_sides(reads, key)?),
        _ => None,
    };
    let mut sides = match (view, &viewpoint) {
        (RequestView::Sides, _) => Sides::new(&daily.dictionary, &daily.index, first_read, last),
        (_, Some(viewpoint)) => Sides::new(&viewpoint.0, &viewpoint.1, first_read, last),
        _ => Sides::new(&[], &empty, first_read, last),
    };
    let note = |sides: &mut Sides, roster: &Resolved, position: u32, hour: i32| {
        let (key, faction, read) = roster.read(position, hour);
        sides.note(&key, faction, read);
    };
    match view {
        RequestView::Sides => {
            if let Some(live) = &live
                && let Some(roster) = &live.data.roster
            {
                match &live.data.used {
                    Some(used) => {
                        for position in used {
                            note(&mut sides, roster, *position, live.hour);
                        }
                    }
                    None => {
                        for position in 0..roster.layout.len() {
                            note(&mut sides, roster, position, live.hour);
                        }
                    }
                }
            }
        }
        RequestView::Side(viewpoint) => {
            if let Some(live) = &live
                && let Some(roster) = &live.data.roster
                && let Some(position) = roster.position(viewpoint)
            {
                note(&mut sides, roster, position, live.hour);
            }
        }
        RequestView::Country(_) => {}
    }
    let country_scores = match country {
        Some(tag) => Some(daily.country_scores(reads, tag)?),
        None => None,
    };
    let read_pairs = match view {
        RequestView::Country(_) => None,
        _ => Some(daily.read_pairs(reads)?),
    };
    let mut composed: HashMap<*const DayData, Compose> = HashMap::new();
    let mut indices: HashMap<(*const DayData, u32), u32> = HashMap::new();
    let mut rows = Vec::new();
    for (region, _) in ranked {
        check_cancel(reads.cancelled)?;
        let region_pairs = read_pairs
            .as_ref()
            .and_then(|read| read.get(&region))
            .map_or(&[][..], Vec::as_slice);
        let active = daily.activity.get(&region).map_or(&[][..], Vec::as_slice);
        let mut cells: Vec<(usize, AirCell)> = Vec::new();
        // The inputs of the cell last composed, which a day with the same repeats.
        let mut previous: Option<(*const DayData, Pairing)> = None;
        for (at, column) in columns.iter().enumerate() {
            let day = first_day + at as i32;
            let Some(column) = column else {
                previous = None;
                push(&mut cells, AirCell(None));
                continue;
            };
            let data = column.data;
            if theatres.is_some() {
                let reason = if !data.theatres.known {
                    Some("theatres-not-recorded")
                } else if !selected[&Arc::as_ptr(&data.theatres)].contains(&region) {
                    Some("outside-theatres")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    previous = None;
                    push(&mut cells, AirCell(Some(AirValue::Reason(reason.into()))));
                    continue;
                }
            }
            let mut single: Vec<Pair> = Vec::new();
            let (pairing, pairs): (Pairing, Option<&[Pair]>) = match (column.live, country) {
                (Some(live), None) => (Pairing::Day, live.pairs.get(&region).map(Vec::as_slice)),
                (Some(live), Some(tag)) => {
                    let held = live.pairs.contains_key(&region);
                    single.extend(
                        live.country
                            .get(&region)
                            .map(|value| (tag.to_owned(), *value)),
                    );
                    (Pairing::Day, held.then_some(single.as_slice()))
                }
                (None, Some(tag)) => {
                    let at = active.partition_point(|(_, last, _)| *last < day);
                    let held = active.get(at).is_some_and(|(first, _, _)| *first <= day);
                    if held {
                        single.extend(
                            country_scores
                                .as_ref()
                                .and_then(|scores| scores.at(region, day))
                                .map(|value| (tag.to_owned(), value)),
                        );
                    }
                    (Pairing::Day, held.then_some(single.as_slice()))
                }
                (None, None) => {
                    let at = region_pairs.partition_point(|(_, last, _)| *last < day);
                    match region_pairs.get(at).filter(|(first, _, _)| *first <= day) {
                        Some((_, _, pairs)) => (Pairing::Run(at), Some(pairs.as_slice())),
                        None => match daily.tail.as_ref().filter(|(tail, _)| *tail == day) {
                            Some((_, tail)) => (Pairing::Day, tail.get(&region).map(Vec::as_slice)),
                            None => (Pairing::Inactive, None),
                        },
                    }
                }
            };
            let inputs = (data as *const DayData, pairing);
            let repeats = pairing != Pairing::Day;
            if repeats && previous == Some(inputs) {
                cells.last_mut().expect("a repeated cell").0 += 1;
                continue;
            }
            previous = repeats.then_some(inputs);
            let compose = composed
                .entry(data as *const DayData)
                .or_insert_with(|| compose(view, data));
            let mut index = |position: u32| -> u32 {
                *indices
                    .entry((data as *const DayData, position))
                    .or_insert_with(|| {
                        let roster = data.roster.as_ref().expect("a shown side has a roster");
                        let (key, faction, read) = roster.read(position, column.hour);
                        match sides.index(&key) {
                            Some(index) => index,
                            None => sides.note(&key, faction, read),
                        }
                    })
            };
            #[cfg(test)]
            COMPOSED.with(|count| count.set(count.get() + 1));
            let cell = compose.cell(pairs, &mut index);
            let cell = selected_by(
                &cell,
                &|index| Some(sides.get(index).key.as_str()),
                keys.as_ref(),
            );
            push(&mut cells, cell);
        }
        rows.push(AirTimelineRow {
            region,
            name: format!("Region {region}"),
            cells,
        });
    }
    let read_hours: Vec<i32> = samples()
        .map(|column| column.hour)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let reply_sides = compact_sides_with(
        &|index| sides.get(index),
        &mut rows
            .iter_mut()
            .flat_map(|row| row.cells.iter_mut().map(|(_, cell)| cell))
            .collect::<Vec<_>>(),
        &read_hours,
        last,
    );
    let excluded = samples()
        .map(|column| {
            let excluded = match view {
                RequestView::Sides => column.data.excluded,
                _ => 0,
            };
            (column.hour, column.hour, excluded)
        })
        .collect();
    let theatre_reads = samples()
        .filter_map(|column| {
            column
                .data
                .theatre_age
                .map(|age| (column.hour, column.hour, column.hour - age))
        })
        .collect();
    Ok(AirTimelineReply {
        view: AirView::Timeline,
        sample_hours: None,
        approximate: Vec::new(),
        rows_approximate: false,
        branch_id: params.branch_id.clone(),
        available: daily.state.available,
        first_hour: first_day_hour,
        last_hour: last,
        days: columns
            .iter()
            .map(|column| column.map(|column| column.hour))
            .collect(),
        sides: reply_sides,
        segments: BTreeMap::new(),
        anchor_reads: BTreeMap::new(),
        rows,
        scale: engine_native::fixed5::SCALE as i32,
        day_hours: DAY_HOURS,
        bad_below: 40_000,
        good_above: 60_000,
        excluded,
        theatre_reads,
        warnings: daily.groups.warnings.clone(),
    })
}
