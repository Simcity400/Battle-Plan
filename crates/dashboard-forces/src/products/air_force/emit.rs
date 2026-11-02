//! Air force size's emit: the row census's runs rebuilt from the projected slot counts, as
//! runs on the family's series publication. Each tag's run holds its counts under both
//! ownerships; a new run starts where they change, or where the tag's country, the census's
//! completeness or the hour's adjacency breaks its run (then it starts a segment). Of each run
//! it writes the four totals ([`super::AirForceOwnership`] by [`super::AirForceMeasure`]), its
//! composition version with the composition as an event, and each total's parts as content
//! names them; each total ranks its known countries descending, ties by tag.
//!
//! The emit holds the variant catalogue over the branch: a variant is spelled by the token its
//! last complete read gave, while that read is known.

use super::{
    MAX_EXACT, UNIDENTIFIED,
    facts::{self, Counts, Names, Part},
    reply::part_key,
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use installed_content_core::equipment::Database;
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
/// A tag's composition version: 0 for a run whose census is unknown under both ownerships,
/// else each run's own, its composition a [`MIX_KIND`] event.
pub const MIX: u64 = 4;
/// A tag's parts of each total, by part ([`PART_KEYS`]): measures 5..9, by channel.
pub const PARTS: u64 = 5;
/// The tag at each rank, by total: measures 10..14 of [`FAMILY`], by position.
pub const POSITIONS: u64 = 10;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
pub const MIX_KIND: u64 = 1;
/// Family scalars: the census's counted hours (unreadable roster and effective ownership,
/// missing, refused and remainder banks), and each tag that is not UTF-8 with its hours.
pub const COUNTS: u64 = 0;
pub const UNDISPLAYABLE: u64 = 1;
/// Each spelled part a composition names ("variant/token", or the unidentified part), by id.
pub const SPELLED: u64 = 1 << 20;
/// Each part key a total's parts name, by id.
pub const PART_KEYS: u64 = 1 << 26;
/// Every air key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// The total of `channel`: ownership (roster 0, effective 1) * 2 + measure (planes 0,
/// wings 1).
pub fn channel(ownership: usize, wings: bool) -> usize {
    ownership * 2 + usize::from(wings)
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved air emit is not valid")
}

fn inexact() -> io::Error {
    io::Error::other("Air force count exceeds exact integer range")
}

/// A spelled part: a variant with the token the catalogue names it by, or the unidentified one.
type Spelled = Option<(u64, Arc<str>)>;

fn spelled_text(spelled: &Spelled) -> String {
    match spelled {
        Some((variant, token)) => format!("{variant}/{token}"),
        None => UNIDENTIFIED.into(),
    }
}

/// A tag's open run: its country and its counts under each ownership, None where unknown.
#[derive(Clone)]
struct Run {
    identity: (u32, u32),
    counts: [Option<Arc<Counts>>; 2],
}

impl Run {
    fn total(&self, channel: usize) -> Option<i128> {
        self.counts[channel / 2].as_ref().map(|counts| {
            i128::from(if channel % 2 == 0 {
                counts.planes
            } else {
                counts.wings
            })
        })
    }

    fn names(&self, variant: u64) -> bool {
        self.counts.iter().flatten().any(|counts| {
            counts
                .parts
                .iter()
                .any(|(part, _)| *part == Part::Variant(variant))
                || counts
                    .signatures
                    .iter()
                    .any(|(parts, _)| parts.contains(&Part::Variant(variant)))
        })
    }
}

/// A tag's parts of one total: each part's key, those last written nonzero or unknown, and
/// whether the total was known then.
#[derive(Clone, Default)]
struct Parts {
    keys: BTreeMap<u32, KeyHandle>,
    present: BTreeSet<u32>,
    open: bool,
}

#[derive(Clone, Default)]
struct Tag {
    /// Totals 0..4, then the composition version.
    keys: Vec<KeyHandle>,
    run: Option<Run>,
    version: u64,
    parts: [Parts; 4],
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
    units: Option<Arc<Database>>,
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
    /// Whether the last hour's census was known under either ownership.
    known: bool,
    /// Each variant's token by its last complete read, while known.
    names: HashMap<u64, Arc<str>>,
    spelled: HashMap<Spelled, u32>,
    spellings: Vec<Spelled>,
    /// Each spelled part's part key, and each wing signature's, by spelled parts.
    plane_parts: Vec<Option<u32>>,
    wing_parts: HashMap<Vec<u32>, u32>,
    part_keys: HashMap<Arc<str>, u32>,
    part_names: Vec<Arc<str>>,
    rankings: [Ranking; 4],
    counters: [u64; 5],
    undisplayable: BTreeMap<Vec<u8>, u64>,
    undisplayable_now: Vec<Vec<u8>>,
    /// The scalars changed since the last cut.
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

impl Emit {
    pub fn new(units: Option<Arc<Database>>) -> Self {
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
            empty: Arc::new(Counts::default()),
            complete: [false; 2],
            known: false,
            names: HashMap::new(),
            spelled: HashMap::new(),
            spellings: Vec::new(),
            plane_parts: Vec::new(),
            wing_parts: HashMap::new(),
            part_keys: HashMap::new(),
            part_names: Vec::new(),
            rankings: Default::default(),
            counters: [0; 5],
            undisplayable: BTreeMap::new(),
            undisplayable_now: Vec::new(),
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(
        &mut self,
        writer: &mut Writer<'_>,
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
    fn tag(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
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
            let mut keys = Vec::with_capacity(5);
            for measure in VALUES..=MIX {
                keys.push(self.key(writer, actor, measure, None)?);
            }
            self.tags[at].keys = keys;
        }
        Ok(actor)
    }

    /// The spelled part `part` names now.
    fn spell(&mut self, writer: &mut Writer<'_>, part: Part) -> io::Result<u32> {
        let spelled: Spelled = match part {
            Part::Variant(variant) => Some((
                variant,
                self.names
                    .get(&variant)
                    .cloned()
                    .unwrap_or_else(|| Arc::from("")),
            )),
            Part::Unidentified => None,
        };
        if let Some(&id) = self.spelled.get(&spelled) {
            return Ok(id);
        }
        let id = self.spellings.len() as u32;
        let text = family::cell(
            writer,
            &Scalar::Text(spelled_text(&spelled).into_bytes().into()),
        )?;
        writer.catalogue(SPELLED + u64::from(id), text)?;
        self.spelled.insert(spelled.clone(), id);
        self.spellings.push(spelled);
        Ok(id)
    }

    /// The part key id of `key`.
    fn part(&mut self, writer: &mut Writer<'_>, key: String) -> io::Result<u32> {
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

    /// The part key a spelled part's planes count under.
    fn plane_part(&mut self, writer: &mut Writer<'_>, spelled: u32) -> io::Result<u32> {
        if let Some(Some(id)) = self.plane_parts.get(spelled as usize) {
            return Ok(*id);
        }
        let key = part_key(
            &spelled_text(&self.spellings[spelled as usize]),
            self.units.as_deref(),
        );
        let id = self.part(writer, key)?;
        if self.plane_parts.len() <= spelled as usize {
            self.plane_parts.resize(spelled as usize + 1, None);
        }
        self.plane_parts[spelled as usize] = Some(id);
        Ok(id)
    }

    /// The part key a wing holding `spelled` counts under: its one type, or its mix of them.
    fn wing_part(&mut self, writer: &mut Writer<'_>, mut spelled: Vec<u32>) -> io::Result<u32> {
        spelled.sort_unstable();
        if let Some(&id) = self.wing_parts.get(&spelled) {
            return Ok(id);
        }
        let mut keys = BTreeSet::new();
        for &part in &spelled {
            let id = self.plane_part(writer, part)?;
            keys.insert(Arc::clone(&self.part_names[id as usize]));
        }
        let key = if keys.len() == 1 {
            keys.into_iter().next().expect("one wing type").to_string()
        } else {
            format!(
                "mixed:{}",
                serde_json::to_string(&keys.iter().map(|key| &**key).collect::<Vec<_>>())?
            )
        };
        let id = self.part(writer, key)?;
        self.wing_parts.insert(spelled, id);
        Ok(id)
    }

    /// Takes the hour's catalogue reads: the variants whose spelling changed.
    fn read_names(&mut self, names: Names) -> HashSet<u64> {
        let mut changed = HashSet::new();
        match names {
            Names::Carried => {}
            Names::Forget => {
                for (variant, token) in self.names.drain() {
                    if !token.is_empty() {
                        changed.insert(variant);
                    }
                }
            }
            Names::Read { tokens, .. } => {
                for (variant, token) in tokens {
                    let before = self.names.insert(variant, Arc::clone(&token));
                    if before.as_deref().unwrap_or("") != &*token {
                        changed.insert(variant);
                    }
                }
            }
        }
        changed
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An unrecorded hour ends every run, and what the catalogue knew.
            self.carry.close(writer, last)?;
            for tag in &mut self.tags {
                tag.run = None;
                for parts in &mut tag.parts {
                    parts.present.clear();
                    parts.open = false;
                }
            }
            for ranking in &mut self.rankings {
                ranking.order.clear();
                ranking.amounts.clear();
            }
            self.names.clear();
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
        let renamed = self.read_names(hour.names);
        if hour.whole {
            self.counts = Default::default();
        }
        let mut touched: HashSet<u64> = HashSet::new();
        for (channel, slot, counts) in hour.counts {
            if let Some(actors) = self.by_slot.get(&slot) {
                touched.extend(actors.iter().copied());
            }
            self.counts[channel as usize].insert(slot, counts);
        }
        let known = hour.complete[0] || hour.complete[1];
        let mut all = !adjacent || hour.complete != self.complete || !known || hour.whole;
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
                        .ok_or_else(|| dashboard_core::invalid("An air country was never named"))?;
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
        let known_before = self.known && adjacent;
        self.complete = hour.complete;
        self.known = known;
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
            let held: HashSet<u64> = visit.iter().copied().collect();
            for &actor in self.live.keys() {
                if !held.contains(&actor)
                    && self.tags[actor as usize]
                        .run
                        .as_ref()
                        .is_some_and(|run| renamed.iter().any(|&variant| run.names(variant)))
                {
                    visit.push(actor);
                }
            }
        }
        visit.sort_unstable();
        visit.dedup();
        let mut ranked = Vec::with_capacity(visit.len());
        for actor in visit {
            let identity = self.live.get(&actor).copied();
            let Some(identity) = identity else {
                if self.tags[actor as usize].run.is_some() {
                    self.end(writer, at, actor)?;
                    ranked.push(actor);
                }
                continue;
            };
            let was = previous
                .as_ref()
                .map_or_else(|| self.live.get(&actor), |previous| previous.get(&actor));
            let continues = known && known_before && was == Some(&identity);
            let counts: [Option<Arc<Counts>>; 2] = std::array::from_fn(|ownership| {
                hour.complete[ownership].then(|| {
                    self.counts[ownership]
                        .get(&identity.0)
                        .cloned()
                        .unwrap_or_else(|| Arc::clone(&self.empty))
                })
            });
            let tag = &self.tags[actor as usize];
            let same = continues
                && tag.run.as_ref().is_some_and(|run| {
                    run.counts == counts && !renamed.iter().any(|&variant| run.names(variant))
                });
            if same {
                continue;
            }
            self.offer(writer, at, actor, Run { identity, counts }, !continues)?;
            ranked.push(actor);
        }
        self.rank(writer, at, &ranked)?;
        for (at, flag) in [
            facts::UNREADABLE_ROSTER,
            facts::UNREADABLE_EFFECTIVE,
            facts::MISSING,
            facts::REFUSED,
            facts::REMAINDERS,
        ]
        .into_iter()
        .enumerate()
        {
            if hour.flags & flag != 0 {
                self.counters[at] += 1;
                self.stale = true;
            }
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

    /// The value of `key` from `hour` on, starting a segment where `starts` or where the key held
    /// none the hour before, as the row census's series start them.
    fn put(
        &mut self,
        writer: &mut Writer<'_>,
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
    fn end(&mut self, writer: &mut Writer<'_>, hour: i32, actor: u64) -> io::Result<()> {
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
        tag.run = None;
        Ok(())
    }

    /// Starts `run` for `actor` at `hour`, a segment where `starts`.
    fn offer(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        actor: u64,
        run: Run,
        starts: bool,
    ) -> io::Result<()> {
        let keys = self.tags[actor as usize].keys.clone();
        for channel in 0..4 {
            self.put(writer, keys[channel], hour, run.total(channel), starts)?;
        }
        let mix = keys[MIX as usize];
        if run.counts.iter().all(Option::is_none) {
            self.carry.value(writer, mix, hour, Some(0), true)?;
        } else {
            let tag = &mut self.tags[actor as usize];
            tag.version += 1;
            let version = tag.version;
            self.put(writer, mix, hour, Some(i128::from(version)), starts)?;
            let mut sides = Vec::with_capacity(2);
            for counts in &run.counts {
                sides.push(match counts {
                    Some(counts) => self.composition(writer, counts)?,
                    None => Cell::Null,
                });
            }
            let fields = writer.cells(sides)?;
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
        for channel in 0..4 {
            self.parts(writer, hour, actor, channel, &run, starts)?;
        }
        self.tags[actor as usize].run = Some(run);
        Ok(())
    }

    /// A composition as event cells: planes, wings, (spelled part, planes or null) pairs and
    /// (spelled parts, wings) pairs.
    fn composition(&mut self, writer: &mut Writer<'_>, counts: &Counts) -> io::Result<Cell> {
        let mut parts = Vec::with_capacity(counts.parts.len() * 2);
        for &(part, amount) in &counts.parts {
            parts.push(Cell::Unsigned(u64::from(self.spell(writer, part)?)));
            parts.push(amount.map_or(Cell::Null, |amount| Cell::Integer(i128::from(amount))));
        }
        let mut signatures = Vec::with_capacity(counts.signatures.len() * 2);
        for (held, count) in &counts.signatures {
            let mut spelled = Vec::with_capacity(held.len());
            for &part in held.iter() {
                spelled.push(Cell::Unsigned(u64::from(self.spell(writer, part)?)));
            }
            signatures.push(Cell::List(writer.cells(spelled)?));
            signatures.push(Cell::Integer(i128::from(*count)));
        }
        let cells = vec![
            Cell::Integer(i128::from(counts.planes)),
            Cell::Integer(i128::from(counts.wings)),
            Cell::List(writer.cells(parts)?),
            Cell::List(writer.cells(signatures)?),
        ];
        Ok(Cell::List(writer.cells(cells)?))
    }

    /// Writes `channel`'s parts of `run`: the parts it holds nonzero or unknown and those
    /// written so before, every part the tag ever held where the total reopens, zero where
    /// absent; every part ends where the total is unknown.
    fn parts(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        actor: u64,
        channel: usize,
        run: &Run,
        starts: bool,
    ) -> io::Result<()> {
        let Some(counts) = &run.counts[channel / 2] else {
            let parts = &mut self.tags[actor as usize].parts[channel];
            for &key in parts.keys.values() {
                self.carry.value(writer, key, hour, None, starts)?;
            }
            parts.present.clear();
            parts.open = false;
            return Ok(());
        };
        let mut amounts: BTreeMap<u32, Option<i128>> = BTreeMap::new();
        let mut add = |part: u32, amount: Option<i128>| -> io::Result<()> {
            let held = amounts.entry(part).or_insert(Some(0));
            *held = match (*held, amount) {
                (Some(held), Some(amount)) => Some(
                    held.checked_add(amount)
                        .filter(|sum| *sum <= i128::from(MAX_EXACT))
                        .ok_or_else(inexact)?,
                ),
                _ => None,
            };
            Ok(())
        };
        if channel % 2 == 0 {
            for &(part, amount) in &counts.parts {
                let spelled = self.spell(writer, part)?;
                let part = self.plane_part(writer, spelled)?;
                add(part, amount.map(i128::from))?;
            }
        } else {
            for (held, count) in &counts.signatures {
                let mut spelled = Vec::with_capacity(held.len());
                for &part in held.iter() {
                    spelled.push(self.spell(writer, part)?);
                }
                let part = self.wing_part(writer, spelled)?;
                add(part, Some(i128::from(*count)))?;
            }
        }
        let current: BTreeSet<u32> = amounts
            .iter()
            .filter(|(_, amount)| **amount != Some(0))
            .map(|(&part, _)| part)
            .collect();
        let parts = &self.tags[actor as usize].parts[channel];
        let mut targets: BTreeSet<u32> = parts.present.union(&current).copied().collect();
        if starts || !parts.open {
            targets.extend(parts.keys.keys().copied());
        }
        for part in targets {
            let amount = amounts.get(&part).copied().unwrap_or(Some(0));
            let key = match self.tags[actor as usize].parts[channel].keys.get(&part) {
                Some(&key) => key,
                None => {
                    let key =
                        self.key(writer, actor, PARTS + channel as u64, Some(u64::from(part)))?;
                    self.tags[actor as usize].parts[channel]
                        .keys
                        .insert(part, key);
                    key
                }
            };
            self.put(writer, key, hour, amount, starts)?;
        }
        let parts = &mut self.tags[actor as usize].parts[channel];
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

    /// Moves the `touched` tags in each ranking to their totals' places; the positions that
    /// changed go on from `hour`.
    fn rank(&mut self, writer: &mut Writer<'_>, hour: i32, touched: &[u64]) -> io::Result<()> {
        if touched.is_empty() {
            return Ok(());
        }
        for channel in 0..4 {
            let mut moved = Vec::new();
            let mut changed = HashSet::new();
            for &actor in touched {
                let amount = self.tags[actor as usize]
                    .run
                    .as_ref()
                    .and_then(|run| run.total(channel));
                if self.rankings[channel].amounts.get(&actor).copied() != amount {
                    changed.insert(actor);
                    if let Some(amount) = amount {
                        moved.push((actor, amount));
                    }
                }
            }
            if changed.is_empty() {
                continue;
            }
            let labels = &self.labels;
            moved.sort_unstable_by(|&left, &right| Self::before(labels, left, right));
            let ranking = &mut self.rankings[channel];
            for actor in &changed {
                ranking.amounts.remove(actor);
            }
            for &(actor, amount) in &moved {
                ranking.amounts.insert(actor, amount);
            }
            let old = std::mem::take(&mut ranking.order);
            let mut order = Vec::with_capacity(ranking.amounts.len());
            let mut kept = old
                .iter()
                .copied()
                .filter(|actor| !changed.contains(actor))
                .peekable();
            let mut moved = moved.into_iter().peekable();
            loop {
                let next = match (kept.peek(), moved.peek()) {
                    (Some(&left), Some(&(right, amount))) => {
                        let held = (left, ranking.amounts[&left]);
                        if Self::before(labels, held, (right, amount)) == Ordering::Greater {
                            moved.next().map(|(actor, _)| actor)
                        } else {
                            kept.next()
                        }
                    }
                    (Some(_), None) => kept.next(),
                    (None, Some(_)) => moved.next().map(|(actor, _)| actor),
                    (None, None) => break,
                };
                order.extend(next);
            }
            for position in 0..old.len().max(order.len()) {
                let (was, now) = (old.get(position).copied(), order.get(position).copied());
                if was == now {
                    continue;
                }
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
            self.rankings[channel].order = order;
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
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
            known: self.known,
            complete: self.complete,
            live: self
                .live
                .iter()
                .map(|(&actor, &identity)| (actor, identity))
                .collect(),
            counts: self.counts.each_ref().map(|counts| {
                counts
                    .iter()
                    .map(|(&slot, counts)| (slot, SavedCounts::of(counts)))
                    .collect::<Vec<_>>()
            }),
            names: self
                .names
                .iter()
                .map(|(&variant, token)| (variant, token.to_string()))
                .collect(),
            counters: self.counters,
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
                .filter(|(_, tag)| tag.run.is_some() || tag.version != 0)
                .map(|(actor, tag)| SavedTag {
                    actor: actor as u64,
                    run: tag.run.as_ref().map(|run| {
                        (
                            run.identity,
                            run.counts
                                .each_ref()
                                .map(|counts| counts.as_deref().map(SavedCounts::of)),
                        )
                    }),
                    version: tag.version,
                    parts: tag.parts.each_ref().map(|parts| {
                        (
                            parts.present.iter().copied().collect::<Vec<_>>(),
                            parts.open,
                        )
                    }),
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
        units: Option<Arc<Database>>,
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
            let mut keys = Vec::with_capacity(5);
            for measure in VALUES..=MIX {
                keys.push(directory.key(actor, measure, None).ok_or_else(invalid)?);
            }
            tag.keys = keys;
            for (channel, parts) in tag.parts.iter_mut().enumerate() {
                parts.keys = directory
                    .dimensions(actor, PARTS + channel as u64)
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
        while let Some(spelled) = text(SPELLED + id)? {
            let spelled: Spelled = if spelled == UNIDENTIFIED {
                None
            } else {
                let (variant, token) = spelled.split_once('/').ok_or_else(invalid)?;
                Some((variant.parse().map_err(|_| invalid())?, Arc::from(token)))
            };
            emit.spelled.insert(spelled.clone(), id as u32);
            emit.spellings.push(spelled);
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
        emit.known = saved.known;
        emit.complete = saved.complete;
        let mut signatures: HashMap<Vec<Part>, Arc<[Part]>> = HashMap::new();
        let mut restore = |counts: SavedCounts| counts.counts(&mut signatures);
        for (channel, counts) in saved.counts.into_iter().enumerate() {
            for (slot, counts) in counts {
                emit.counts[channel].insert(slot, Arc::new(restore(counts)));
            }
        }
        emit.live = saved.live.into_iter().collect();
        for (&actor, &(slot, _)) in &emit.live {
            emit.by_slot.entry(slot).or_default().push(actor);
        }
        emit.names = saved
            .names
            .into_iter()
            .map(|(variant, token)| (variant, Arc::from(token)))
            .collect();
        emit.counters = saved.counters;
        emit.undisplayable = saved.undisplayable.into_iter().collect();
        emit.undisplayable_now = saved.undisplayable_now;
        for tag in saved.tags {
            let held = emit.tags.get_mut(tag.actor as usize).ok_or_else(invalid)?;
            held.version = tag.version;
            held.run = tag.run.map(|(identity, counts)| Run {
                identity,
                counts: counts.map(|counts| counts.map(|counts| Arc::new(restore(counts)))),
            });
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
                .filter_map(|(actor, tag)| Some((actor as u64, tag.run.as_ref()?.total(channel)?)))
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

/// A part as saved: a variant, or none for the unidentified part.
fn saved_part(part: Part) -> Option<u64> {
    match part {
        Part::Variant(variant) => Some(variant),
        Part::Unidentified => None,
    }
}

fn restored_part(part: Option<u64>) -> Part {
    part.map_or(Part::Unidentified, Part::Variant)
}

#[derive(Serialize, Deserialize)]
struct SavedCounts {
    planes: i64,
    wings: i64,
    parts: Vec<(Option<u64>, Option<i64>)>,
    signatures: Vec<(Vec<Option<u64>>, i64)>,
}

impl SavedCounts {
    fn of(counts: &Counts) -> Self {
        Self {
            planes: counts.planes,
            wings: counts.wings,
            parts: counts
                .parts
                .iter()
                .map(|&(part, amount)| (saved_part(part), amount))
                .collect(),
            signatures: counts
                .signatures
                .iter()
                .map(|(parts, count)| (parts.iter().copied().map(saved_part).collect(), *count))
                .collect(),
        }
    }

    fn counts(self, signatures: &mut HashMap<Vec<Part>, Arc<[Part]>>) -> Counts {
        Counts {
            planes: self.planes,
            wings: self.wings,
            parts: self
                .parts
                .into_iter()
                .map(|(part, amount)| (restored_part(part), amount))
                .collect(),
            signatures: self
                .signatures
                .into_iter()
                .map(|(parts, count)| {
                    let parts: Vec<Part> = parts.into_iter().map(restored_part).collect();
                    let held = signatures
                        .entry(parts.clone())
                        .or_insert_with(|| Arc::from(parts))
                        .clone();
                    (held, count)
                })
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SavedTag {
    actor: u64,
    run: Option<((u32, u32), [Option<SavedCounts>; 2])>,
    version: u64,
    parts: [(Vec<u32>, bool); 4],
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    known: bool,
    complete: [bool; 2],
    live: Vec<(u64, (u32, u32))>,
    counts: [Vec<(u32, SavedCounts)>; 2],
    names: Vec<(u64, String)>,
    counters: [u64; 5],
    undisplayable: Vec<(Vec<u8>, u64)>,
    undisplayable_now: Vec<Vec<u8>>,
    tags: Vec<SavedTag>,
}
