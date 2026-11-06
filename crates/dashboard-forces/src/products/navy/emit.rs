//! Navy size's emit: the row census's runs rebuilt from the projected slot counts, as runs on the
//! family's series publication. Each tag holds a run list under each channel (ownership by
//! measure, [`channel`]): a run starts where the channel's total, composition or maximum hull HP
//! changes, or where the tag's country or the hours' adjacency breaks it. A run starts a segment
//! where it breaks, or where its total or the run before's is unknown. Of each run it writes the
//! channel's total, its composition version with the composition as an event, the strength
//! channels' maximum hull HP and each grouping's parts as content names them; each total ranks its
//! known countries descending, ties by tag.
//!
//! The emit holds each class token's spelling over the branch: a class is its token with the
//! spelling LexicalTokens gave it at the hour, as the row census names it.

use super::{
    ClassKey, MAX_EXACT, NavyGrouping,
    facts::{self, Counts},
    part_key,
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use installed_content_core::unit_leader::SubUnits;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A tag's totals, by channel ([`channel`]): measures 0..4.
pub const VALUES: u64 = 0;
/// A tag's composition versions, by channel: measures 4..8. Version 0 is a run that knows
/// nothing; any other is the run's own, its composition a [`MIX_KIND`] event.
pub const MIX: u64 = 4;
/// A tag's maximum hull HP, by ownership: measures 8..10.
pub const MAXIMUM: u64 = 8;
/// A tag's parts of each total, by part ([`PART_KEYS`]): measures 10..18, by [`parts`].
pub const PARTS: u64 = 10;
/// The tag at each rank, by total: measures 18..22 of [`FAMILY`], by position.
pub const POSITIONS: u64 = 18;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
pub const MIX_KIND: u64 = 1;
/// Family scalars: the census's counts (unread hours, then by ownership the unattributed ship
/// observations, the country-hours with refused hull HP and those with an overflowing maximum),
/// and each tag that is not UTF-8 with its hours.
pub const COUNTS: u64 = 0;
pub const UNDISPLAYABLE: u64 = 1;
/// Each class a composition names (its token and spelling, as JSON), by id.
pub const CLASSES: u64 = 1 << 20;
/// Each part key a total's parts name, by id.
pub const PART_KEYS: u64 = 1 << 26;
/// Every navy key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
/// A tag's keys: its totals, composition versions and maximum hull HP.
const KEYS: u64 = 10;

/// The groupings parts are named under, by index.
pub const GROUPINGS: [NavyGrouping; 2] = [NavyGrouping::Class, NavyGrouping::Type];

/// The total of `channel`: ownership (roster 0, logical 1) * 2 + measure (ships 0, strength 1).
pub fn channel(ownership: usize, strength: bool) -> usize {
    ownership * 2 + usize::from(strength)
}

/// The parts measure of `channel`'s total under `grouping` ([`GROUPINGS`]).
pub fn parts(channel: usize, grouping: usize) -> u64 {
    PARTS + (channel * 2 + grouping) as u64
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved navy emit is not valid")
}

/// One channel's run: its total, maximum hull HP (strength only) and by class its ships or hull
/// HP, ascending by class; empty where the total is unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Run {
    total: Option<i64>,
    reference: Option<i64>,
    classes: Vec<(u32, i64)>,
}

impl Run {
    const UNKNOWN: Self = Self {
        total: None,
        reference: None,
        classes: Vec::new(),
    };

    fn knows_nothing(&self) -> bool {
        self.total.is_none() && self.reference.is_none() && self.classes.is_empty()
    }

    /// Whether replies can show it: the row census projects no parts of a run they cannot.
    fn displayable(&self) -> bool {
        self.total
            .into_iter()
            .chain(self.reference)
            .all(|value| value <= MAX_EXACT)
    }
}

/// A tag's parts of one total under one grouping: each part's key, those last written nonzero,
/// and whether the total was known then.
#[derive(Clone, Default)]
struct Parts {
    keys: BTreeMap<u32, KeyHandle>,
    present: BTreeSet<u32>,
    open: bool,
}

/// What the row census notes of a live tag's slot at a read hour, by ownership.
const REFUSED: u8 = 1;
const OVERFLOW: u8 = 2;

#[derive(Clone, Default)]
struct Tag {
    /// Totals 0..4, composition versions 4..8, maximum hull HP 8..10.
    keys: Vec<KeyHandle>,
    runs: [Option<Run>; 4],
    versions: [u64; 4],
    parts: [Parts; 8],
    notes: [u8; 2],
}

/// One total's ranking: its known tags in rank order, each one's amount, and the positions'
/// keys.
#[derive(Default)]
struct Ranking {
    order: Vec<u64>,
    amounts: HashMap<u64, i128>,
    positions: Vec<KeyHandle>,
}

/// The family's emit on one branch.
pub struct Emit {
    units: Option<Arc<SubUnits>>,
    actors: Actors,
    carry: Carry,
    /// Each projection index's actor, for the segment under way.
    named: Vec<Option<u64>>,
    labels: Vec<Arc<[u8]>>,
    tags: Vec<Tag>,
    live: HashMap<u64, (u32, u32)>,
    by_slot: HashMap<u32, Vec<u64>>,
    counts: [HashMap<u32, Arc<Counts>>; 2],
    empty: Arc<Counts>,
    complete: [bool; 2],
    /// Whether the last hour's census read the ships, and whether it was read at all.
    read: bool,
    census: bool,
    /// The fingerprint of the row census's country join, while the countries are complete.
    join: Option<u64>,
    /// Each class token's spelling, while the census holds it.
    spellings: HashMap<u32, Option<Arc<str>>>,
    class_ids: HashMap<(u32, Option<Arc<str>>), u32>,
    classes: Vec<ClassKey>,
    /// Each class's part key under each grouping.
    class_parts: Vec<[Option<u32>; 2]>,
    part_keys: HashMap<Arc<str>, u32>,
    part_names: Vec<Arc<str>>,
    rankings: [Ranking; 4],
    counters: [u64; 7],
    /// The live tags noted refused (0) and overflowing (1), by ownership.
    notes: [[u64; 2]; 2],
    undisplayable: BTreeMap<Vec<u8>, u64>,
    undisplayable_now: Vec<Vec<u8>>,
    /// The scalars changed since the last cut.
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

impl Emit {
    pub fn new(units: Option<Arc<SubUnits>>) -> Self {
        Self {
            units,
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            named: Vec::new(),
            labels: Vec::new(),
            tags: Vec::new(),
            live: HashMap::new(),
            by_slot: HashMap::new(),
            counts: Default::default(),
            empty: Arc::new(Counts::empty()),
            complete: [false; 2],
            read: false,
            census: false,
            join: None,
            spellings: HashMap::new(),
            class_ids: HashMap::new(),
            classes: Vec::new(),
            class_parts: Vec::new(),
            part_keys: HashMap::new(),
            part_names: Vec::new(),
            rankings: Default::default(),
            counters: [0; 7],
            notes: [[0; 2]; 2],
            undisplayable: BTreeMap::new(),
            undisplayable_now: Vec::new(),
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(
        &mut self,
        writer: &mut Writer,
        actor: u64,
        measure: u64,
        dimension: Option<u64>,
    ) -> io::Result<KeyHandle> {
        self.actors.key(
            writer,
            Place {
                actor,
                measure,
                dimension,
            },
            (RunMode::Exact, 0),
            Some(DOMAIN),
        )
    }

    /// The tag actor of `tag`, with its keys, first seen at `hour`.
    fn tag(&mut self, writer: &mut Writer, tag: &str, hour: i32) -> io::Result<u64> {
        let actor = match self.actors.find(tag.as_bytes()) {
            Some(actor) => actor,
            None => self.actors.actor(writer, tag.as_bytes(), hour)?,
        };
        let at = actor as usize;
        if at >= self.tags.len() {
            self.tags.resize(at + 1, Tag::default());
            self.labels.resize(at + 1, Arc::from(&[][..]));
        }
        self.labels[at] = Arc::from(tag.as_bytes());
        if self.tags[at].keys.is_empty() {
            let mut keys = Vec::with_capacity(KEYS as usize);
            for measure in 0..KEYS {
                keys.push(self.key(writer, actor, measure, None)?);
            }
            self.tags[at].keys = keys;
        }
        Ok(actor)
    }

    /// The class `token` names now: the token with its spelling.
    fn class(&mut self, writer: &mut Writer, token: u32) -> io::Result<u32> {
        let spelling = self.spellings.get(&token).cloned().flatten();
        let held = (token, spelling);
        if let Some(&id) = self.class_ids.get(&held) {
            return Ok(id);
        }
        let id = u32::try_from(self.classes.len())
            .map_err(|_| io::Error::other("Navy class identity index exceeds u32"))?;
        let key: ClassKey = (token, held.1.as_deref().map(str::to_owned));
        let text = family::cell(writer, &Scalar::Text(serde_json::to_vec(&key)?.into()))?;
        writer.catalogue(CLASSES + u64::from(id), text)?;
        self.class_ids.insert(held, id);
        self.classes.push(key);
        Ok(id)
    }

    /// The part key id of `key`.
    fn part(&mut self, writer: &mut Writer, key: String) -> io::Result<u32> {
        if let Some(&id) = self.part_keys.get(key.as_str()) {
            return Ok(id);
        }
        let id = self.part_names.len() as u32;
        let text = family::cell(writer, &Scalar::Text(key.as_bytes().into()))?;
        writer.catalogue(PART_KEYS + u64::from(id), text)?;
        let key: Arc<str> = Arc::from(key);
        self.part_keys.insert(Arc::clone(&key), id);
        self.part_names.push(key);
        Ok(id)
    }

    /// The part key `class` counts under in `grouping`.
    fn class_part(
        &mut self,
        writer: &mut Writer,
        class: u32,
        grouping: usize,
    ) -> io::Result<u32> {
        let at = class as usize;
        if let Some(Some(id)) = self.class_parts.get(at).map(|parts| parts[grouping]) {
            return Ok(id);
        }
        let key = part_key(
            &self.classes[at],
            self.units.as_deref(),
            GROUPINGS[grouping],
        );
        let id = self.part(writer, key)?;
        if self.class_parts.len() <= at {
            self.class_parts.resize(at + 1, [None; 2]);
        }
        self.class_parts[at][grouping] = Some(id);
        Ok(id)
    }

    /// Takes the hour's spellings: the tokens whose spelling changed.
    fn respell(&mut self, whole: bool, spellings: Vec<(u32, Option<Arc<str>>)>) -> HashSet<u32> {
        let mut changed = HashSet::new();
        if whole {
            let mut before = std::mem::take(&mut self.spellings);
            for (token, spelling) in spellings {
                if before.remove(&token).flatten() != spelling {
                    changed.insert(token);
                }
                self.spellings.insert(token, spelling);
            }
            changed.extend(
                before
                    .into_iter()
                    .filter(|(_, spelling)| spelling.is_some())
                    .map(|(token, _)| token),
            );
            return changed;
        }
        for (token, spelling) in spellings {
            if self.spellings.insert(token, spelling.clone()).flatten() != spelling {
                changed.insert(token);
            }
        }
        changed
    }

    /// Whether `actor`'s country's counts name a token of `tokens`.
    fn holds(&self, slot: u32, tokens: &HashSet<u32>) -> bool {
        self.counts.iter().any(|counts| {
            counts.get(&slot).is_some_and(|counts| {
                counts
                    .classes
                    .iter()
                    .any(|(token, _, _)| tokens.contains(token))
            })
        })
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An unrecorded hour ends every run.
            self.carry.close(writer, last)?;
            for tag in &mut self.tags {
                tag.runs = Default::default();
                tag.notes = [0; 2];
                for parts in &mut tag.parts {
                    parts.present.clear();
                    parts.open = false;
                }
            }
            for ranking in &mut self.rankings {
                ranking.order.clear();
                ranking.amounts.clear();
            }
            self.notes = [[0; 2]; 2];
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        for (index, tag) in &hour.named {
            let actor = self.tag(writer, tag, at)?;
            let index = *index as usize;
            if index >= self.named.len() {
                self.named.resize(index + 1, None);
            }
            self.named[index] = Some(actor);
        }
        let renamed = self.respell(hour.spelled_whole, hour.spellings);
        // Classes are numbered as the row census first reads them, which orders their warnings.
        for &token in &hour.seen {
            self.class(writer, token)?;
        }
        if hour.whole {
            self.counts = Default::default();
        }
        let mut touched: HashSet<u64> = HashSet::new();
        for (ownership, slot, counts) in hour.counts {
            if let Some(actors) = self.by_slot.get(&slot) {
                touched.extend(actors.iter().copied());
            }
            self.counts[ownership as usize].insert(slot, counts);
        }
        let mut all =
            !adjacent || hour.complete != self.complete || hour.read != self.read || hour.whole;
        let previous = match hour.live {
            Some(live) => {
                all = true;
                let mut held = HashMap::with_capacity(live.len());
                for (index, identity) in live {
                    let actor = self
                        .named
                        .get(index as usize)
                        .copied()
                        .flatten()
                        .ok_or_else(|| dashboard_core::invalid("A navy country was never named"))?;
                    held.insert(actor, identity);
                }
                self.by_slot.clear();
                for (&actor, &(slot, _)) in &held {
                    self.by_slot.entry(slot).or_default().push(actor);
                }
                Some(std::mem::replace(&mut self.live, held))
            }
            None => None,
        };
        // The row census keeps its notes through an unread hour after one whose ships every
        // ownership left unattributed, where the join held, and counts them again.
        let stale = !hour.read
            && hour.census
            && self.census
            && adjacent
            && self.complete == [false; 2]
            && hour.join.is_none_or(|join| self.join == Some(join));
        self.join = if hour.census { hour.join.or(self.join) } else { None };
        self.complete = hour.complete;
        self.read = hour.read;
        self.census = hour.census;
        let mut visit: Vec<u64> = if all {
            let mut actors: Vec<u64> = self.live.keys().copied().collect();
            if let Some(previous) = &previous {
                actors.extend(
                    previous
                        .keys()
                        .filter(|actor| !self.live.contains_key(actor)),
                );
            }
            actors
        } else {
            touched.into_iter().collect()
        };
        if !renamed.is_empty() && !all {
            for (&actor, &(slot, _)) in &self.live {
                if self.holds(slot, &renamed) {
                    visit.push(actor);
                }
            }
        }
        visit.sort_unstable();
        visit.dedup();
        let mut ranked = Vec::with_capacity(visit.len());
        for actor in visit {
            let Some(identity) = self.live.get(&actor).copied() else {
                if self.tags[actor as usize].runs.iter().any(Option::is_some) {
                    self.end(writer, at, actor)?;
                    ranked.push(actor);
                }
                if !stale {
                    self.note(actor, [0; 2]);
                }
                continue;
            };
            let was = previous
                .as_ref()
                .map_or_else(|| self.live.get(&actor), |previous| previous.get(&actor))
                .copied();
            let continuous = adjacent && was == Some(identity);
            let mut moved = false;
            for channel in 0..4 {
                let run = self.run_of(writer, channel, identity.0)?;
                let before = self.tags[actor as usize].runs[channel].as_ref();
                if continuous && before == Some(&run) {
                    continue;
                }
                let starts = !(continuous
                    && run.total.is_some()
                    && before.is_some_and(|before| before.total.is_some()));
                self.offer(writer, at, actor, channel, run, starts)?;
                moved = true;
            }
            if moved {
                ranked.push(actor);
            }
            let notes = std::array::from_fn(|ownership| {
                let mut notes = 0;
                if self.read
                    && let Some(counts) = self.counts[ownership].get(&identity.0)
                {
                    if counts.hp.is_none() {
                        notes |= REFUSED;
                    }
                    if counts.overflow {
                        notes |= OVERFLOW;
                    }
                }
                notes
            });
            if !stale {
                self.note(actor, notes);
            }
        }
        self.rank(writer, at, &ranked)?;
        let mut count = |at: usize, by: u64| {
            if by > 0 {
                self.counters[at] += by;
                self.stale = true;
            }
        };
        if hour.census {
            for ownership in 0..2 {
                count(1 + ownership, hour.unattributed[ownership]);
                count(3 + ownership, self.notes[0][ownership]);
                count(5 + ownership, self.notes[1][ownership]);
            }
        }
        if !hour.read {
            count(0, 1);
        }
        if let Some(tags) = hour.undisplayable {
            self.undisplayable_now = tags;
        }
        for tag in &self.undisplayable_now {
            *self.undisplayable.entry(tag.clone()).or_default() += 1;
            self.stale = true;
        }
        Ok(())
    }

    /// Notes `actor`'s refused and overflowing ownerships as `notes`.
    fn note(&mut self, actor: u64, notes: [u8; 2]) {
        let tag = &mut self.tags[actor as usize];
        for ownership in 0..2 {
            for (kind, bit) in [REFUSED, OVERFLOW].into_iter().enumerate() {
                let held = &mut self.notes[kind][ownership];
                *held -= u64::from(tag.notes[ownership] & bit != 0);
                *held += u64::from(notes[ownership] & bit != 0);
            }
        }
        tag.notes = notes;
    }

    /// The run of `channel` for the country at `slot` this hour.
    fn run_of(&mut self, writer: &mut Writer, channel: usize, slot: u32) -> io::Result<Run> {
        let ownership = channel / 2;
        if !self.complete[ownership] {
            return Ok(Run::UNKNOWN);
        }
        let counts = self.counts[ownership]
            .get(&slot)
            .cloned()
            .unwrap_or_else(|| Arc::clone(&self.empty));
        let strength = channel % 2 == 1;
        let total = if strength {
            counts.hp
        } else {
            Some(counts.ships)
        };
        let mut classes = Vec::new();
        if total.is_some() {
            classes.reserve(counts.classes.len());
            for &(token, ships, hp) in &counts.classes {
                let class = self.class(writer, token)?;
                classes.push((class, if strength { hp } else { ships }));
            }
            classes.sort_unstable_by_key(|&(class, _)| class);
        }
        Ok(Run {
            total,
            reference: if strength { counts.maximum } else { None },
            classes,
        })
    }

    /// The value of `key` from `hour` on, starting a segment where `starts` or where the key held
    /// none the hour before, as the row census's series start them.
    fn put(
        &mut self,
        writer: &mut Writer,
        key: KeyHandle,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        let starts = starts || self.carry.held(key).is_none();
        self.carry.value(writer, key, hour, value, starts)?;
        Ok(())
    }

    /// Ends `actor`'s runs at `hour`.
    fn end(&mut self, writer: &mut Writer, hour: i32, actor: u64) -> io::Result<()> {
        let tag = &mut self.tags[actor as usize];
        for &key in &tag.keys {
            self.carry.value(writer, key, hour, None, false)?;
        }
        for parts in &mut tag.parts {
            for &key in parts.keys.values() {
                self.carry.value(writer, key, hour, None, false)?;
            }
            parts.present.clear();
            parts.open = false;
        }
        tag.runs = Default::default();
        Ok(())
    }

    /// Starts `run` of `channel` for `actor` at `hour`, a segment where `starts`.
    fn offer(
        &mut self,
        writer: &mut Writer,
        hour: i32,
        actor: u64,
        channel: usize,
        run: Run,
        starts: bool,
    ) -> io::Result<()> {
        let at = actor as usize;
        let (value, mix, maximum) = {
            let keys = &self.tags[at].keys;
            (
                keys[channel],
                keys[MIX as usize + channel],
                keys[MAXIMUM as usize + channel / 2],
            )
        };
        self.put(writer, value, hour, run.total.map(i128::from), starts)?;
        if channel % 2 == 1 {
            self.put(writer, maximum, hour, run.reference.map(i128::from), starts)?;
        }
        if run.knows_nothing() {
            self.carry.value(writer, mix, hour, Some(0), true)?;
        } else {
            self.tags[at].versions[channel] += 1;
            let version = self.tags[at].versions[channel];
            self.put(writer, mix, hour, Some(i128::from(version)), starts)?;
            let known = |value: Option<i64>| value.map_or(Cell::Null, |value| Cell::Integer(value.into()));
            let mut classes = Vec::with_capacity(run.classes.len() * 2);
            for &(class, amount) in &run.classes {
                classes.push(Cell::Unsigned(u64::from(class)));
                classes.push(Cell::Integer(i128::from(amount)));
            }
            let cells = vec![
                known(run.total),
                known(run.reference),
                Cell::List(writer.cells(classes)?),
            ];
            let fields = writer.cells(cells)?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key: mix,
                native_hour: hour,
                read_hour: None,
                sequence,
                kind: MIX_KIND,
                fields,
            })?;
        }
        for grouping in 0..2 {
            self.parts(writer, hour, actor, channel, grouping, &run, starts)?;
        }
        self.tags[at].runs[channel] = Some(run);
        Ok(())
    }

    /// Writes `channel`'s parts of `run` under `grouping`: the parts it holds nonzero and those
    /// written so before, every part the tag ever held where the total reopens, zero where absent;
    /// every part ends where the total is unknown or replies cannot show the run.
    #[allow(clippy::too_many_arguments)]
    fn parts(
        &mut self,
        writer: &mut Writer,
        hour: i32,
        actor: u64,
        channel: usize,
        grouping: usize,
        run: &Run,
        starts: bool,
    ) -> io::Result<()> {
        let index = channel * 2 + grouping;
        if run.total.is_none() || !run.displayable() {
            let parts = &mut self.tags[actor as usize].parts[index];
            for &key in parts.keys.values() {
                self.carry.value(writer, key, hour, None, starts)?;
            }
            parts.present.clear();
            parts.open = false;
            return Ok(());
        }
        let mut amounts: BTreeMap<u32, i128> = BTreeMap::new();
        for &(class, amount) in &run.classes {
            let part = self.class_part(writer, class, grouping)?;
            *amounts.entry(part).or_default() += i128::from(amount);
        }
        let current: BTreeSet<u32> = amounts
            .iter()
            .filter(|(_, amount)| **amount != 0)
            .map(|(&part, _)| part)
            .collect();
        let parts = &self.tags[actor as usize].parts[index];
        let mut targets: BTreeSet<u32> = parts.present.union(&current).copied().collect();
        if starts || !parts.open {
            targets.extend(parts.keys.keys().copied());
        }
        for part in targets {
            let amount = amounts.get(&part).copied().unwrap_or(0);
            let key = match self.tags[actor as usize].parts[index].keys.get(&part) {
                Some(&key) => key,
                None => {
                    let key = self.key(
                        writer,
                        actor,
                        parts_measure(index),
                        Some(u64::from(part)),
                    )?;
                    self.tags[actor as usize].parts[index]
                        .keys
                        .insert(part, key);
                    key
                }
            };
            self.put(writer, key, hour, Some(amount), starts)?;
        }
        let parts = &mut self.tags[actor as usize].parts[index];
        parts.present = current;
        parts.open = true;
        Ok(())
    }

    /// The order of `left` before `right` by a total: larger first, ties by tag.
    fn before(labels: &[Arc<[u8]>], left: (u64, i128), right: (u64, i128)) -> Ordering {
        right
            .1
            .cmp(&left.1)
            .then_with(|| labels[left.0 as usize].cmp(&labels[right.0 as usize]))
    }

    /// Where `target` stands, or would, in `order` (each actor's amount in `amounts`).
    fn locate(
        labels: &[Arc<[u8]>],
        order: &[u64],
        amounts: &HashMap<u64, i128>,
        target: (u64, i128),
    ) -> Result<usize, usize> {
        order.binary_search_by(|&probe| Self::before(labels, (probe, amounts[&probe]), target))
    }

    /// Moves the `touched` tags in each ranking to their totals' places; the positions between
    /// the places a move left and took go on from `hour`, where they changed.
    fn rank(&mut self, writer: &mut Writer, hour: i32, touched: &[u64]) -> io::Result<()> {
        if touched.is_empty() {
            return Ok(());
        }
        for channel in 0..4 {
            let (mut low, mut high) = (usize::MAX, 0_usize);
            let before = self.rankings[channel].order.len();
            for &actor in touched {
                let amount = self.tags[actor as usize].runs[channel]
                    .as_ref()
                    .and_then(|run| run.total)
                    .map(i128::from);
                let labels = &self.labels;
                let ranking = &mut self.rankings[channel];
                let held = ranking.amounts.get(&actor).copied();
                if held == amount {
                    continue;
                }
                if let Some(held) = held {
                    let at = Self::locate(labels, &ranking.order, &ranking.amounts, (actor, held))
                        .map_err(|_| io::Error::other("A ranked navy country is not in its rank"))?;
                    ranking.order.remove(at);
                    ranking.amounts.remove(&actor);
                    (low, high) = (low.min(at), high.max(at));
                }
                if let Some(amount) = amount {
                    let at = match Self::locate(labels, &ranking.order, &ranking.amounts, (actor, amount)) {
                        Ok(at) | Err(at) => at,
                    };
                    ranking.order.insert(at, actor);
                    ranking.amounts.insert(actor, amount);
                    (low, high) = (low.min(at), high.max(at));
                }
            }
            if low == usize::MAX {
                continue;
            }
            let after = self.rankings[channel].order.len();
            if after != before {
                // Every place after a country joined or left moved by one.
                high = before.max(after) - 1;
            }
            for position in low..=high {
                let now = self.rankings[channel].order.get(position).copied();
                let key = match self.rankings[channel].positions.get(position) {
                    Some(&key) => key,
                    None => {
                        let key = self.key(
                            writer,
                            FAMILY,
                            POSITIONS + channel as u64,
                            Some(position as u64),
                        )?;
                        self.rankings[channel].positions.push(key);
                        key
                    }
                };
                self.carry
                    .value(writer, key, hour, now.map(i128::from), false)?;
            }
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        if self.stale {
            let counts = Scalar::List(
                self.counters
                    .iter()
                    .map(|&count| Scalar::Unsigned(count))
                    .collect(),
            );
            let tags = Scalar::List(
                self.undisplayable
                    .iter()
                    .map(|(tag, &count)| {
                        Scalar::List(vec![
                            Scalar::Text(tag.as_slice().into()),
                            Scalar::Unsigned(count),
                        ])
                    })
                    .collect(),
            );
            for (id, scalar) in [(COUNTS, counts), (UNDISPLAYABLE, tags)] {
                let cell = family::cell(writer, &scalar)?;
                writer.catalogue(id, cell)?;
            }
            self.stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            read: self.read,
            census: self.census,
            join: self.join,
            complete: self.complete,
            live: self
                .live
                .iter()
                .map(|(&actor, &identity)| (actor, identity))
                .collect(),
            counts: self.counts.each_ref().map(|counts| {
                counts
                    .iter()
                    .map(|(&slot, counts)| (slot, (**counts).clone()))
                    .collect::<Vec<_>>()
            }),
            spellings: self
                .spellings
                .iter()
                .map(|(&token, spelling)| (token, spelling.as_deref().map(str::to_owned)))
                .collect(),
            counters: self.counters,
            notes: self.notes,
            undisplayable: self
                .undisplayable
                .iter()
                .map(|(tag, &count)| (tag.clone(), count))
                .collect(),
            undisplayable_now: self.undisplayable_now.clone(),
            tags: self
                .tags
                .iter()
                .enumerate()
                .filter(|(_, tag)| {
                    tag.runs.iter().any(Option::is_some)
                        || tag.versions.iter().any(|&version| version != 0)
                        || tag.notes != [0; 2]
                })
                .map(|(actor, tag)| SavedTag {
                    actor: actor as u64,
                    runs: tag.runs.clone(),
                    versions: tag.versions,
                    parts: tag.parts.each_ref().map(|parts| {
                        (
                            parts.present.iter().copied().collect::<Vec<_>>(),
                            parts.open,
                        )
                    }),
                    notes: tag.notes,
                })
                .collect(),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(
        units: Option<Arc<SubUnits>>,
        directory: &Directory,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let mut emit = Self::new(units);
        emit.stale = false;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.labels = directory
            .actors
            .iter()
            .map(|actor| Arc::from(&actor.name[..]))
            .collect();
        emit.tags = vec![Tag::default(); directory.actors.len()];
        for (actor, tag) in emit.tags.iter_mut().enumerate() {
            let actor = actor as u64;
            let mut keys = Vec::with_capacity(KEYS as usize);
            for measure in 0..KEYS {
                keys.push(directory.key(actor, measure, None).ok_or_else(invalid)?);
            }
            tag.keys = keys;
            for (index, parts) in tag.parts.iter_mut().enumerate() {
                parts.keys = directory
                    .dimensions(actor, parts_measure(index))
                    .iter()
                    .map(|&(part, key)| Ok((u32::try_from(part).map_err(|_| invalid())?, key)))
                    .collect::<io::Result<_>>()?;
            }
        }
        for (channel, ranking) in emit.rankings.iter_mut().enumerate() {
            ranking.positions = directory
                .dimensions(FAMILY, POSITIONS + channel as u64)
                .iter()
                .enumerate()
                .map(|(at, &(position, key))| {
                    (position == at as u64).then_some(key).ok_or_else(invalid)
                })
                .collect::<io::Result<_>>()?;
        }
        let text = |id: u64| -> io::Result<Option<String>> {
            directory
                .scalar(id)
                .map(|scalar| {
                    scalar
                        .text()
                        .and_then(|text| String::from_utf8(text.to_vec()).ok())
                        .ok_or_else(invalid)
                })
                .transpose()
        };
        let mut id = 0;
        while let Some(class) = text(CLASSES + id)? {
            let class: ClassKey = serde_json::from_str(&class).map_err(|_| invalid())?;
            emit.class_ids
                .insert((class.0, class.1.as_deref().map(Arc::from)), id as u32);
            emit.classes.push(class);
            id += 1;
        }
        let mut id = 0;
        while let Some(key) = text(PART_KEYS + id)? {
            let key: Arc<str> = Arc::from(key);
            emit.part_keys.insert(Arc::clone(&key), id as u32);
            emit.part_names.push(key);
            id += 1;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.read = saved.read;
        emit.census = saved.census;
        emit.join = saved.join;
        emit.complete = saved.complete;
        for (ownership, counts) in saved.counts.into_iter().enumerate() {
            for (slot, counts) in counts {
                emit.counts[ownership].insert(slot, Arc::new(counts));
            }
        }
        emit.live = saved.live.into_iter().collect();
        for (&actor, &(slot, _)) in &emit.live {
            emit.by_slot.entry(slot).or_default().push(actor);
        }
        emit.spellings = saved
            .spellings
            .into_iter()
            .map(|(token, spelling)| (token, spelling.map(Arc::from)))
            .collect();
        emit.counters = saved.counters;
        emit.notes = saved.notes;
        emit.undisplayable = saved.undisplayable.into_iter().collect();
        emit.undisplayable_now = saved.undisplayable_now;
        for tag in saved.tags {
            let held = emit.tags.get_mut(tag.actor as usize).ok_or_else(invalid)?;
            held.runs = tag.runs;
            held.versions = tag.versions;
            held.notes = tag.notes;
            for (parts, (present, open)) in held.parts.iter_mut().zip(tag.parts) {
                parts.present = present.into_iter().collect();
                parts.open = open;
            }
        }
        // Each ranking holds every tag with a known total, in rank order.
        for channel in 0..4 {
            let mut known: Vec<(u64, i128)> = emit
                .tags
                .iter()
                .enumerate()
                .filter_map(|(actor, tag)| {
                    Some((actor as u64, i128::from(tag.runs[channel].as_ref()?.total?)))
                })
                .collect();
            let labels = &emit.labels;
            known.sort_unstable_by(|&left, &right| Self::before(labels, left, right));
            let ranking = &mut emit.rankings[channel];
            ranking.order = known.iter().map(|&(actor, _)| actor).collect();
            ranking.amounts = known.into_iter().collect();
        }
        Ok(emit)
    }
}

/// The parts measure of part index `index` (channel * 2 + grouping).
fn parts_measure(index: usize) -> u64 {
    PARTS + index as u64
}

#[derive(Serialize, Deserialize)]
struct SavedTag {
    actor: u64,
    runs: [Option<Run>; 4],
    versions: [u64; 4],
    parts: [(Vec<u32>, bool); 8],
    notes: [u8; 2],
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    read: bool,
    census: bool,
    join: Option<u64>,
    complete: [bool; 2],
    live: Vec<(u64, (u32, u32))>,
    counts: [Vec<(u32, Counts)>; 2],
    spellings: Vec<(u32, Option<String>)>,
    counters: [u64; 7],
    notes: [[u64; 2]; 2],
    undisplayable: Vec<(Vec<u8>, u64)>,
    undisplayable_now: Vec<Vec<u8>>,
    tags: Vec<SavedTag>,
}
