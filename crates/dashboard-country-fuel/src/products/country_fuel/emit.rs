//! Fuel's emit: each tag's supplying reads, fuel level and capacity, current consumption and
//! days of updates, and each native country's consumption segments, on the family's series
//! publication.
//!
//! A read is one event at its hour on the tag's read key, holding everything the replies show
//! of it but its last hour, which the key's run gives. Its day is decided as it is read (its
//! ownership proof, and the overlap rule against the kept days before it), except a day read
//! outside the fuel table's slot: a slot read up to 23 hours later can still overlap it, which
//! a drop event at that read's hour records. The day measures are runs dated 23 hours late
//! ([`LAG`]): the value for hour `h` is at `h + 23`, written once every read that can decide it
//! was read, so a cut never holds a day still undecided. Replies read the last 23 hours of a
//! publication from its reads instead.

use super::{
    AwardSums, DAY_HOURS, DayLeftOut, GAIN_UNITS, Identity, QUANTITY_UNITS, RawDay, RawRead,
    demand::RawConsumption,
    facts::{self, Handed, Sample, Supplied},
    pack::{self, Encoded},
};
use dashboard_series::{
    Blob, Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use replay_data::{
    cadence::{self, Cadence},
    format::FormatVersion,
    rows::TableId,
};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeSet, VecDeque},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 2;

/// A tag's keys: its supplying read (its runs' value; its events the reads), fuel level and
/// capacity, positive current consumption by family, the day measures ([`DAY_MEASURES`], dated
/// [`LAG`] hours late), the drops of overlapped days and its reads' demand packs, as chains of
/// a keyframe and at most [`CHAIN`] - 1 deltas, an event at each read that changes the pack.
pub const READ: u64 = 0;
pub const FUEL: u64 = 1;
pub const CAPACITY: u64 = 2;
pub const CONSUMPTION: u64 = 3;
pub const DAY: u64 = 6;
pub const DROPS: u64 = DAY + DAY_MEASURES.len() as u64;
pub const DEMAND: u64 = DROPS + 1;

/// Events in a demand chain, its keyframe's included.
pub const CHAIN: u8 = 16;
/// A native country's key: the first hour of its consumption segment.
pub const SEGMENT: u64 = 0;

/// The day measures, in key order; the first [`STRIPS`] hold a day at its read's hour alone.
pub const DAY_MEASURES: [&str; 15] = [
    "stripGain",
    "stripBurn",
    "gain-0",
    "gain-1",
    "gain-2",
    "gain-3",
    "unidentified",
    "incoming",
    "outgoing",
    "flat",
    "other",
    "conversion",
    "consumer-0",
    "consumer-1",
    "consumer-2",
];
pub const STRIPS: usize = 2;

/// Hours a day measure is dated late: the overlap rule decides a day within them.
pub const LAG: i32 = DAY_HOURS - 1;

/// Evidence domains: the hours walked, known for every key but the days; the same hours dated
/// [`LAG`] late, known for the days.
pub const WALKED: u64 = 0;
pub const LATE: u64 = 1;

/// Event kinds.
pub mod kind {
    /// On a tag's read key: `[Unsigned country, instance, continuity, Boolean starts,
    /// Integer fuel, capacity, lend-lease, left-out, reason, day, Boolean consumption starts,
    /// Integer segment start, native ×3, known ×3, positive ×3, recipients]`; left-out an
    /// Unsigned ([`super::left_out_code`]), a reason an Unsigned reason id, each Null where
    /// none; the day as recorded, left out or not, a List of the ring's 6 or the awards' 14
    /// Integers; a native rate an Integer, or the Unsigned id of why it is unknown; the
    /// recipients a Text of the applied day's ([`super::pack::recipients`]), Null where the day
    /// is refused.
    pub const READ: u64 = 0;
    /// On a tag's drops key, at a slot read's hour: `[Integer hour]`, the read whose kept day
    /// it overlaps.
    pub const DROP: u64 = 1;
    /// On a tag's demand key: `[Text pack]`, a whole pack ([`super::pack::keyframe`]).
    pub const KEYFRAME: u64 = 2;
    /// On a tag's demand key: `[Integer keyframe hour, Text delta]`, the pack as the change
    /// from the event before ([`super::pack::delta`]).
    pub const DELTA: u64 = 3;
}

/// Family cells: the countries whose tags are not UTF-8, the first and last hours walked, each
/// reason's text by id, each tag's meta (Boolean read, Unsigned left-out counts ×3, by
/// [`super::left_out_code`]) and each native country's (Unsigned country, Boolean conflict,
/// List of the Unsigned tag actors that held it).
pub const UNNAMED: u64 = 0;
pub const BOUNDS: u64 = 1;
pub const REASONS: u64 = 1 << 24;
pub const TAG_META: u64 = 2 << 24;
pub const ACTOR_META: u64 = 3 << 24;

/// The first byte of a native country's actor name; no tag starts with it.
const NATIVE: u8 = 0xFF;

pub fn native_name(country: u32) -> [u8; 5] {
    let mut name = [NATIVE; 5];
    name[1..].copy_from_slice(&country.to_le_bytes());
    name
}

pub fn native_of(name: &[u8]) -> Option<u32> {
    match name {
        [NATIVE, rest @ ..] if rest.len() == 4 => Some(u32::from_le_bytes(rest.try_into().ok()?)),
        _ => None,
    }
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved fuel emit is not valid")
}

/// A day's measures in [`DAY_MEASURES`] order; None where its kind of day has none.
pub(super) fn day_values(day: &RawDay) -> [Option<i128>; 15] {
    let (gained, burned, received, other, awards) = match day {
        RawDay::Ring {
            gained,
            burned,
            received,
            other,
        } => (
            i128::from(*gained),
            i128::from(*burned),
            received.map(i128::from),
            i128::from(*other),
            None,
        ),
        RawDay::Awards(awards) => (
            awards.own_gain,
            awards.consumed,
            awards.received,
            awards.transfers[3],
            Some(awards.as_ref()),
        ),
    };
    let quantity = i128::from(QUANTITY_UNITS);
    let gain = i128::from(GAIN_UNITS);
    let mut values = [None; 15];
    values[0] = Some(gained);
    values[1] = Some(burned);
    for (index, value) in received.into_iter().enumerate() {
        values[12 + index] = Some((value * quantity).max(0));
    }
    let received_sum: i128 = received.into_iter().sum();
    match awards {
        Some(awards) => {
            for (index, value) in awards.gains.into_iter().enumerate() {
                values[2 + index] = Some(value.max(0));
            }
            values[7] = Some(awards.transfers[0].max(0));
            values[8] = Some((awards.transfers[1] * gain).max(0));
            values[9] = Some((awards.transfers[2] * gain).max(0));
            values[10] = Some(((other - awards.transfers[1] - awards.transfers[2]) * gain).max(0));
        }
        None => {
            values[6] = Some(gained.max(0));
            values[10] = Some((other * gain).max(0));
        }
    }
    values[11] = Some((burned * gain - other * gain - received_sum * quantity).max(0));
    values
}

/// A read waiting for its day measures, which are written [`LAG`] hours after it.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Unsettled {
    first: i32,
    starts: bool,
    values: Option<[Option<i128>; 15]>,
}

/// A tag's open read.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Open {
    first: i32,
    /// Whether its consumption starts a segment, and that segment's first hour.
    consumption_starts: bool,
    segment: i32,
    /// The native country's actor.
    native: u64,
}

/// What falls due on the day keys at an hour [`LAG`] late.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Due {
    /// The tag's next unsettled read starts its day.
    Start,
    /// The tag's read ended without a read after it.
    End,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Scheduled {
    at: i32,
    tag: u64,
    due: Due,
}

/// A tag's keys, interned at its first read.
#[derive(Clone, Copy, Debug)]
struct Keys {
    current: [KeyHandle; 6],
    days: [KeyHandle; 15],
    drops: Option<KeyHandle>,
    demand: Option<KeyHandle>,
}

/// A tag's demand chain as its last event left it: the pack, the chain's keyframe hour and its
/// events.
#[derive(Clone, Debug)]
struct Stream {
    pack: Arc<Encoded>,
    keyframe: i32,
    events: u8,
}

/// One tag.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Tag {
    #[serde(skip)]
    keys: Option<Keys>,
    identity: Option<Identity>,
    present: bool,
    open: Option<Open>,
    continuity: Option<i32>,
    /// Its samples, as runs of walked hours: the closed ones over the last day, and the open
    /// one while a country holds it.
    holdings: VecDeque<(i32, i32, Sample)>,
    holding: Option<(i32, Sample)>,
    /// The hour from which no country held it; None while one does, or it was never seen.
    absent_since: Option<i32>,
    seen: bool,
    /// The overlap rule: the last kept slot day, the last kept day, and a kept day outside the
    /// slot that a slot read can still overlap.
    kept_slot: Option<i32>,
    kept: Option<i32>,
    pending: Option<i32>,
    unsettled: VecDeque<Unsettled>,
    read: bool,
    left_out: [u64; 3],
    #[serde(skip)]
    stream: Option<Stream>,
    /// The pack the projection last supplied it, which a read supplying none repeats, and the
    /// delta it was handed as from the pack before.
    #[serde(skip)]
    supplied: Option<Arc<Encoded>>,
    #[serde(skip)]
    handed: Option<(Arc<Encoded>, Vec<u8>)>,
    #[serde(skip)]
    stale: bool,
}

/// One native country's consumption segments.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Native {
    #[serde(skip)]
    key: KeyHandle,
    country: u32,
    /// The tags whose open read is of it.
    holders: Vec<u64>,
    /// Its holder and segment's first hour, while it had one holder.
    latest: Option<(u64, i32)>,
    active: bool,
    conflict: bool,
    tags: BTreeSet<u64>,
    #[serde(skip)]
    stale: bool,
}

#[derive(Clone, Debug, Default)]
enum Entity {
    #[default]
    Unknown,
    Tag(Box<Tag>),
    Native(Box<Native>),
}

/// One tag at an hour, as the projection or a removal gives it.
struct Offer {
    identity: Option<Identity>,
    present: bool,
    seen: Sample,
    level: Option<Option<(i64, i64)>>,
    supplied: Option<Box<Supplied>>,
}

/// The family's emit on one branch.
pub struct Emit {
    cadence: Cadence,
    actors: Actors,
    current: Carry,
    days: Carry,
    /// Each projection index's tag actor, for the segment under way.
    named: Vec<Option<u64>>,
    /// By actor id.
    entities: Vec<Entity>,
    natives: FastMap<u32, u64>,
    present: BTreeSet<u64>,
    queue: VecDeque<Scheduled>,
    prior: VecDeque<(i32, i32, Sample)>,
    prior_open: Option<(i32, Sample)>,
    /// The spans of walked hours a cut has still to date late, the last running up to `last`.
    walked: VecDeque<(i32, i32)>,
    /// The last hour cut.
    cut: Option<i32>,
    first: Option<i32>,
    last: Option<i32>,
    reasons: FastMap<Arc<str>, u64>,
    unnamed: BTreeSet<u32>,
    unnamed_stale: bool,
    stale: Vec<u64>,
}

/// A day left out's place in the counts and on the wire.
pub(super) fn left_out_code(reason: DayLeftOut) -> usize {
    match reason {
        DayLeftOut::Overlaps => 0,
        DayLeftOut::NotUpdated => 1,
        DayLeftOut::OwnershipNotRecorded => 2,
    }
}

pub(super) fn left_out_of(code: usize) -> Option<DayLeftOut> {
    [
        DayLeftOut::Overlaps,
        DayLeftOut::NotUpdated,
        DayLeftOut::OwnershipNotRecorded,
    ]
    .get(code)
    .copied()
}

fn tag_of(entities: &mut [Entity], actor: u64) -> &mut Tag {
    match &mut entities[actor as usize] {
        Entity::Tag(tag) => tag,
        _ => unreachable!("a tag actor holds a tag"),
    }
}

/// Whether the game updated the fuel of `identity` in each of the updates a read at `read`
/// holds (see the module notes of [`super`]): none when it did, else why its day is left out.
/// The walked hours run without a gap from `contiguous`.
fn day_left_out(
    tag: &Tag,
    contiguous: Option<i32>,
    read: i32,
    identity: Identity,
) -> Option<DayLeftOut> {
    let (from, to) = (read - DAY_HOURS, read - 1);
    let mut unknown = contiguous.is_none_or(|first| first > from);
    let open = tag.holding.map(|(first, sample)| (first, to, sample));
    for (first, last, sample) in tag.holdings.iter().copied().chain(open) {
        if last < from || first > to {
            continue;
        }
        match sample {
            Sample::Owns(owner) if owner == identity => {}
            Sample::Unknown => unknown = true,
            _ => return Some(DayLeftOut::NotUpdated),
        }
    }
    unknown.then_some(DayLeftOut::OwnershipNotRecorded)
}

/// Keeps the runs of `holdings` that reach `from`.
fn trim(holdings: &mut VecDeque<(i32, i32, Sample)>, from: i32) {
    while holdings.front().is_some_and(|&(_, last, _)| last < from) {
        holdings.pop_front();
    }
}

/// `open`, extended to `hour` with `sample`: the run it ends, if any.
fn extend(
    open: &mut Option<(i32, Sample)>,
    hour: i32,
    sample: Sample,
) -> Option<(i32, i32, Sample)> {
    match *open {
        Some((_, held)) if held == sample => None,
        Some((first, held)) => {
            *open = Some((hour, sample));
            Some((first, hour - 1, held))
        }
        None => {
            *open = Some((hour, sample));
            None
        }
    }
}

impl Emit {
    pub fn new(format: FormatVersion) -> Self {
        Self {
            cadence: cadence::cadence_in(TableId::CountryFuel, format),
            actors: Actors::new(SCHEMA),
            current: Carry::default(),
            days: Carry::default(),
            named: Vec::new(),
            entities: Vec::new(),
            natives: FastMap::default(),
            present: BTreeSet::new(),
            queue: VecDeque::new(),
            prior: VecDeque::new(),
            prior_open: None,
            walked: VecDeque::new(),
            cut: None,
            first: None,
            last: None,
            reasons: FastMap::default(),
            unnamed: BTreeSet::new(),
            unnamed_stale: false,
            stale: Vec::new(),
        }
    }

    fn actor(&mut self, writer: &mut Writer, name: &[u8], hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(name) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, name, hour)?;
        if actor >= 1 << 24 {
            return Err(dashboard_core::invalid(
                "Fuel actors exceed their cell range",
            ));
        }
        if let Some(more) = (actor as usize + 1).checked_sub(self.entities.len()) {
            engine_allocation::reserve(&mut self.entities, more)?;
            self.entities
                .resize_with(actor as usize + 1, Entity::default);
        }
        Ok(actor)
    }

    /// `actor`'s key of `measure`, its evidence that of `domain`.
    fn key(
        &mut self,
        writer: &mut Writer,
        actor: u64,
        measure: u64,
        domain: u64,
    ) -> io::Result<KeyHandle> {
        self.actors.key(
            writer,
            Place {
                actor,
                measure,
                dimension: None,
            },
            (RunMode::Exact, 0),
            Some(domain),
        )
    }

    fn tag_mut(&mut self, actor: u64) -> &mut Tag {
        tag_of(&mut self.entities, actor)
    }

    fn native_mut(&mut self, actor: u64) -> &mut Native {
        match &mut self.entities[actor as usize] {
            Entity::Native(native) => native,
            _ => unreachable!("a native actor holds a native country"),
        }
    }

    /// The tag actor named `tag`.
    fn tag(&mut self, writer: &mut Writer, tag: &str, hour: i32) -> io::Result<u64> {
        let actor = self.actor(writer, tag.as_bytes(), hour)?;
        if matches!(self.entities[actor as usize], Entity::Unknown) {
            self.entities[actor as usize] = Entity::Tag(Box::default());
        }
        Ok(actor)
    }

    /// The actor of native country `country`, with its key.
    fn native(&mut self, writer: &mut Writer, country: u32, hour: i32) -> io::Result<u64> {
        if let Some(&actor) = self.natives.get(&country) {
            return Ok(actor);
        }
        let actor = self.actor(writer, &native_name(country), hour)?;
        let key = self.key(writer, actor, SEGMENT, WALKED)?;
        self.entities[actor as usize] = Entity::Native(Box::new(Native {
            key,
            country,
            holders: Vec::new(),
            latest: None,
            active: false,
            conflict: false,
            tags: BTreeSet::new(),
            stale: false,
        }));
        self.natives.insert(country, actor);
        Ok(actor)
    }

    /// The keys of tag actor `actor`, interned at its first read.
    fn keys(&mut self, writer: &mut Writer, actor: u64) -> io::Result<Keys> {
        if let Some(keys) = self.tag_mut(actor).keys {
            return Ok(keys);
        }
        let mut current = [0; 6];
        for (measure, key) in current.iter_mut().enumerate() {
            *key = self.key(writer, actor, measure as u64, WALKED)?;
        }
        let mut days = [0; 15];
        for (measure, key) in days.iter_mut().enumerate() {
            *key = self.key(writer, actor, DAY + measure as u64, LATE)?;
        }
        let keys = Keys {
            current,
            days,
            drops: None,
            demand: None,
        };
        self.tag_mut(actor).keys = Some(keys);
        Ok(keys)
    }

    fn reason(&mut self, writer: &mut Writer, reason: &Arc<str>) -> io::Result<u64> {
        if let Some(&id) = self.reasons.get(reason) {
            return Ok(id);
        }
        let id = self.reasons.len() as u64;
        if id >= 1 << 24 {
            return Err(dashboard_core::invalid(
                "Fuel reasons exceed their cell range",
            ));
        }
        let cell = family::cell(writer, &Scalar::Text(reason.as_bytes().into()))?;
        writer.catalogue(REASONS + id, cell)?;
        self.reasons.insert(Arc::clone(reason), id);
        Ok(id)
    }

    fn mark(&mut self, actor: u64) {
        let stale = match &mut self.entities[actor as usize] {
            Entity::Tag(tag) => &mut tag.stale,
            Entity::Native(native) => &mut native.stale,
            Entity::Unknown => return,
        };
        if !*stale {
            *stale = true;
            self.stale.push(actor);
        }
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, mut hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let previous = self.last;
        let adjacent = previous.and_then(|last| last.checked_add(1)) == Some(at);
        if !adjacent && let Some(last) = previous {
            self.gap(writer, last)?;
        }
        match self.walked.back_mut() {
            Some(span) if adjacent => span.1 = at,
            _ => self.walked.push_back((at, at)),
        }
        self.first.get_or_insert(at);
        for (index, tag) in &hour.named {
            let actor = self.tag(writer, tag, at)?;
            let index = *index as usize;
            if index >= self.named.len() {
                engine_allocation::resize(&mut self.named, index + 1, None)?;
            }
            self.named[index] = Some(actor);
        }
        // The shared samples of tags no country holds.
        if let Some(ended) = extend(&mut self.prior_open, at, hour.prior) {
            self.prior.push_back(ended);
        }
        trim(&mut self.prior, at - DAY_HOURS - 1);
        let named = |named: &[Option<u64>], index: u32| {
            named
                .get(index as usize)
                .copied()
                .flatten()
                .ok_or_else(|| dashboard_core::invalid("A fuel tag was never named"))
        };
        let mut touched = BTreeSet::new();
        let mut handed = Vec::with_capacity(hour.countries.len());
        let mut supplied = Vec::new();
        for country in std::mem::take(&mut hour.countries) {
            let actor = named(&self.named, country.index)?;
            handed.push(actor);
            if country.supplied.is_some() {
                supplied.push(actor);
            }
            let offer = Offer {
                identity: country.identity,
                present: country.present,
                seen: country.seen,
                level: country.level,
                supplied: country.supplied,
            };
            self.offer(writer, at, adjacent, actor, offer, &hour, &mut touched)?;
        }
        if let Some(present) = &hour.present {
            let mut now = BTreeSet::new();
            for &index in present {
                now.insert(named(&self.named, index)?);
            }
            handed.sort_unstable();
            let removed: Vec<u64> = self
                .present
                .iter()
                .copied()
                .filter(|actor| !now.contains(actor) && handed.binary_search(actor).is_err())
                .collect();
            for actor in removed {
                let offer = Offer {
                    identity: None,
                    present: false,
                    seen: hour.prior,
                    level: None,
                    supplied: None,
                };
                self.offer(writer, at, adjacent, actor, offer, &hour, &mut touched)?;
            }
        }
        if hour.read {
            self.retire(&mut supplied);
        }
        self.segments(writer, at, adjacent, &touched)?;
        for id in &hour.unnamed {
            self.unnamed_stale |= self.unnamed.insert(*id);
        }
        self.drain(writer, at)?;
        self.last = Some(at);
        Ok(())
    }

    /// Drops the demand chain of every tag a read hour supplied nothing (`supplied` the tags it
    /// did): its next read starts a chain anew, so the saved emit holds no pack of a tag gone.
    fn retire(&mut self, supplied: &mut [u64]) {
        supplied.sort_unstable();
        for (actor, entity) in self.entities.iter_mut().enumerate() {
            if let Entity::Tag(tag) = entity
                && (tag.stream.is_some() || tag.supplied.is_some())
                && supplied.binary_search(&(actor as u64)).is_err()
            {
                tag.stream = None;
                tag.supplied = None;
                tag.handed = None;
            }
        }
    }

    /// Every open run and read ends at `last`, the hour before a walk's gap.
    fn gap(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.current.close(writer, last)?;
        for actor in 0..self.entities.len() {
            match &mut self.entities[actor] {
                Entity::Tag(tag) => {
                    if let Some((first, sample)) = tag.holding.take() {
                        tag.holdings.push_back((first, last, sample));
                    }
                    if tag.open.take().is_some() {
                        self.queue.push_back(Scheduled {
                            at: last + 1 + LAG,
                            tag: actor as u64,
                            due: Due::End,
                        });
                    }
                }
                Entity::Native(native) => {
                    native.holders.clear();
                    native.active = false;
                }
                Entity::Unknown => {}
            }
        }
        if let Some((first, sample)) = self.prior_open.take() {
            self.prior.push_back((first, last, sample));
        }
        Ok(())
    }

    /// Ends `actor`'s open read before `hour`.
    fn close(
        &mut self,
        writer: &mut Writer,
        actor: u64,
        hour: i32,
        touched: &mut BTreeSet<u64>,
    ) -> io::Result<()> {
        let tag = tag_of(&mut self.entities, actor);
        let Some(open) = tag.open.take() else {
            return Ok(());
        };
        let keys = tag.keys.ok_or_else(invalid)?;
        for key in keys.current {
            self.current.value(writer, key, hour, None, false)?;
        }
        self.queue.push_back(Scheduled {
            at: hour + LAG,
            tag: actor,
            due: Due::End,
        });
        self.native_mut(open.native)
            .holders
            .retain(|&holder| holder != actor);
        touched.insert(open.native);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn offer(
        &mut self,
        writer: &mut Writer,
        at: i32,
        adjacent: bool,
        actor: u64,
        mut offer: Offer,
        hour: &facts::Hour,
        touched: &mut BTreeSet<u64>,
    ) -> io::Result<()> {
        let from = at - DAY_HOURS - 1;
        let tag = tag_of(&mut self.entities, actor);
        match offer
            .supplied
            .as_mut()
            .and_then(|supplied| supplied.pack.take())
        {
            Some(Handed::Whole(pack)) => {
                tag.supplied = Some(pack);
                tag.handed = None;
            }
            Some(Handed::Delta(bytes)) => {
                let before = tag.supplied.take().ok_or_else(invalid)?;
                tag.supplied = Some(Arc::new(pack::apply(&before, &bytes)?));
                tag.handed = Some((before, bytes));
            }
            None => {}
        }
        if offer.present && !tag.present {
            // A tag a country holds again takes up the shared samples since none did.
            let since = tag.absent_since.unwrap_or(i32::MIN);
            let open = self
                .prior_open
                .map(|(first, sample)| (first, at - 1, sample));
            for (first, last, sample) in self.prior.iter().copied().chain(open) {
                let first = first.max(since).max(from);
                let last = last.min(at - 1);
                if first <= last {
                    tag.holdings.push_back((first, last, sample));
                }
            }
            tag.absent_since = None;
        } else if !offer.present && (tag.present || !tag.seen) {
            if let Some((first, sample)) = tag.holding.take() {
                tag.holdings.push_back((first, at - 1, sample));
            }
            tag.absent_since = Some(if tag.seen { at } else { i32::MIN });
        }
        tag.seen = true;
        if offer.present
            && let Some(ended) = extend(&mut tag.holding, at, offer.seen)
        {
            tag.holdings.push_back(ended);
        }
        trim(&mut tag.holdings, from);
        let tracked = tag.identity;
        tag.identity = offer.identity;
        if offer.present != tag.present {
            tag.present = offer.present;
            if offer.present {
                self.present.insert(actor);
            } else {
                self.present.remove(&actor);
            }
        }
        let continuous = adjacent && offer.identity.is_some() && tracked == offer.identity;
        if !continuous {
            self.close(writer, actor, at, touched)?;
        }
        let Some(identity) = offer.identity.filter(|_| hour.available) else {
            return self.close(writer, actor, at, touched);
        };
        if !hour.read {
            if self.tag_mut(actor).open.is_some()
                && let Some(level) = offer.level
            {
                let keys = self.tag_mut(actor).keys.ok_or_else(invalid)?;
                for (key, value) in [
                    (keys.current[FUEL as usize], level.map(|level| level.0)),
                    (keys.current[CAPACITY as usize], level.map(|level| level.1)),
                ] {
                    let broken = self.current.held(key).is_none();
                    self.current
                        .value(writer, key, at, value.map(i128::from), broken)?;
                }
            }
            return Ok(());
        }
        let Some(supplied) = offer.supplied else {
            return self.close(writer, actor, at, touched);
        };
        self.read(writer, at, actor, identity, *supplied, offer.level, touched)
    }

    #[allow(clippy::too_many_arguments)]
    fn read(
        &mut self,
        writer: &mut Writer,
        at: i32,
        actor: u64,
        identity: Identity,
        supplied: Supplied,
        level: Option<Option<(i64, i64)>>,
        touched: &mut BTreeSet<u64>,
    ) -> io::Result<()> {
        let Supplied {
            read,
            consumption,
            recipients,
            ..
        } = supplied;
        let pack = tag_of(&mut self.entities, actor)
            .supplied
            .clone()
            .ok_or_else(invalid)?;
        let (read, consumption) = (&read, &consumption);
        let keys = self.keys(writer, actor)?;
        let native = self.native(writer, read.country, at)?;
        let slot = self.cadence.is_refresh(at);
        let contiguous = self.walked.back().map(|&(first, _)| first);
        let tag = tag_of(&mut self.entities, actor);
        let held = tag.open;
        let consumption_starts = held.is_none();
        let starts = held.is_none() || tag.continuity != read.continuity_start_hour;
        tag.continuity = read.continuity_start_hour;
        let segment = held.map_or(at, |open| open.segment);
        let mut day = read.day.as_ref();
        let mut left_out = None;
        if read.awards_reason.is_some() {
            left_out = day_left_out(tag, contiguous, at, identity);
            if left_out.is_some() {
                day = None;
            }
        }
        let mut dropped = None;
        if day.is_some() {
            let near = |kept: Option<i32>| kept.is_some_and(|kept| at - kept <= LAG);
            if slot {
                if near(tag.kept_slot) {
                    left_out = Some(DayLeftOut::Overlaps);
                    day = None;
                } else {
                    tag.kept_slot = Some(at);
                    tag.kept = Some(at);
                    dropped = tag.pending.take().filter(|&pending| at - pending <= LAG);
                }
            } else if near(tag.kept) {
                left_out = Some(DayLeftOut::Overlaps);
                day = None;
            } else {
                tag.kept = Some(at);
                tag.pending = Some(at);
            }
        }
        tag.read = true;
        if let Some(reason) = left_out {
            tag.left_out[left_out_code(reason)] += 1;
        }
        if let Some(pending) = dropped {
            tag.left_out[left_out_code(DayLeftOut::Overlaps)] += 1;
            if let Some(waiting) = tag
                .unsettled
                .iter_mut()
                .find(|waiting| waiting.first == pending)
            {
                waiting.values = None;
            }
        }
        tag.unsettled.push_back(Unsettled {
            first: at,
            starts,
            values: day.map(day_values),
        });
        self.mark(actor);
        if let Some(pending) = dropped {
            let drops = match keys.drops {
                Some(drops) => drops,
                None => {
                    let drops = self.key(writer, actor, DROPS, WALKED)?;
                    if let Some(keys) = &mut self.tag_mut(actor).keys {
                        keys.drops = Some(drops);
                    }
                    drops
                }
            };
            let fields = writer.cells([Cell::Integer(i128::from(pending))])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key: drops,
                native_hour: at,
                read_hour: None,
                sequence,
                kind: kind::DROP,
                fields,
            })?;
        }
        let fields = self.read_cells(
            writer,
            read,
            consumption,
            (starts, consumption_starts, segment),
            left_out,
            recipients.as_deref(),
        )?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key: keys.current[READ as usize],
            native_hour: at,
            read_hour: None,
            sequence,
            kind: kind::READ,
            fields,
        })?;
        self.demand(writer, actor, at, pack)?;
        let level = level.flatten();
        let mut values = [
            (Some(i128::from(at)), starts),
            (level.map(|level| i128::from(level.0)), starts),
            (level.map(|level| i128::from(level.1)), starts),
            (None, consumption_starts),
            (None, consumption_starts),
            (None, consumption_starts),
        ];
        for family in 0..3 {
            values[CONSUMPTION as usize + family].0 = consumption.native_daily[family]
                .is_ok()
                .then_some(consumption.positive_daily[family]);
        }
        for (key, (value, starts)) in keys.current.into_iter().zip(values) {
            let broken = starts || self.current.held(key).is_none();
            self.current.value(writer, key, at, value, broken)?;
        }
        self.queue.push_back(Scheduled {
            at: at + LAG,
            tag: actor,
            due: Due::Start,
        });
        if let Some(open) = held
            && open.native != native
        {
            self.native_mut(open.native)
                .holders
                .retain(|&holder| holder != actor);
            touched.insert(open.native);
        }
        let holders = &mut self.native_mut(native).holders;
        if !holders.contains(&actor) {
            holders.push(actor);
        }
        touched.insert(native);
        self.tag_mut(actor).open = Some(Open {
            first: at,
            consumption_starts,
            segment,
            native,
        });
        Ok(())
    }

    fn read_cells(
        &mut self,
        writer: &mut Writer,
        read: &RawRead,
        consumption: &RawConsumption,
        (starts, consumption_starts, segment): (bool, bool, i32),
        left_out: Option<DayLeftOut>,
        recipients: Option<&[u8]>,
    ) -> io::Result<Blob> {
        let reason = match &read.awards_reason {
            Some(reason) => Cell::Unsigned(self.reason(writer, reason)?),
            None => Cell::Null,
        };
        let mut native = [Cell::Null; 3];
        for (cell, daily) in native.iter_mut().zip(&consumption.native_daily) {
            *cell = match daily {
                Ok(value) => Cell::Integer(i128::from(*value)),
                Err(reason) => Cell::Unsigned(self.reason(writer, reason)?),
            };
        }
        let day = match &read.day {
            None => Cell::Null,
            Some(RawDay::Ring {
                gained,
                burned,
                received,
                other,
            }) => Cell::List(
                writer.cells(
                    [
                        *gained,
                        *burned,
                        received[0],
                        received[1],
                        received[2],
                        *other,
                    ]
                    .map(|value| Cell::Integer(i128::from(value))),
                )?,
            ),
            Some(RawDay::Awards(sums)) => {
                let AwardSums {
                    gains,
                    own_gain,
                    own_gain_quantity,
                    transfers,
                    consumed,
                    received,
                } = &**sums;
                let values = gains
                    .iter()
                    .chain([own_gain, own_gain_quantity])
                    .chain(transfers)
                    .chain([consumed])
                    .chain(received)
                    .map(|&value| Cell::Integer(value));
                Cell::List(writer.cells(values)?)
            }
        };
        let optional =
            |value: Option<u32>| value.map_or(Cell::Null, |value| Cell::Unsigned(u64::from(value)));
        let hour =
            |value: Option<i32>| value.map_or(Cell::Null, |value| Cell::Integer(i128::from(value)));
        let mut cells = vec![
            Cell::Unsigned(u64::from(read.country)),
            optional(read.country_instance),
            hour(read.continuity_start_hour),
            Cell::Boolean(starts),
            Cell::Integer(i128::from(read.fuel)),
            Cell::Integer(i128::from(read.capacity)),
            Cell::Integer(i128::from(read.lend_lease)),
            left_out.map_or(Cell::Null, |reason| {
                Cell::Unsigned(left_out_code(reason) as u64)
            }),
            reason,
            day,
            Cell::Boolean(consumption_starts),
            Cell::Integer(i128::from(segment)),
        ];
        cells.extend(native);
        cells.extend(consumption.known_daily.map(Cell::Integer));
        cells.extend(consumption.positive_daily.map(Cell::Integer));
        cells.push(match recipients {
            None => Cell::Null,
            Some([]) => Cell::Text(Blob::EMPTY),
            Some(bytes) => Cell::Text(writer.blob(&mut &bytes[..], bytes.len() as u64)?),
        });
        writer.cells(cells)
    }

    /// Writes `actor`'s demand pack at its read at `at`, where it changed: a delta of the pack
    /// before it, or a keyframe where the chain is full or either pack is refused.
    fn demand(
        &mut self,
        writer: &mut Writer,
        actor: u64,
        at: i32,
        pack: Arc<Encoded>,
    ) -> io::Result<()> {
        let tag = tag_of(&mut self.entities, actor);
        let stream = tag.stream.take();
        let handed = tag.handed.take();
        if let Some(stream) = &stream
            && (Arc::ptr_eq(&stream.pack, &pack) || *stream.pack == *pack)
        {
            tag.stream = Some(Stream {
                pack,
                ..stream.clone()
            });
            return Ok(());
        }
        let key = match tag.keys.ok_or_else(invalid)?.demand {
            Some(key) => key,
            None => {
                let key = self.key(writer, actor, DEMAND, WALKED)?;
                if let Some(keys) = &mut self.tag_mut(actor).keys {
                    keys.demand = Some(key);
                }
                key
            }
        };
        let mut bytes = Vec::new();
        let (shape, keyframe, events) = match &stream {
            Some(stream)
                if stream.events < CHAIN
                    && matches!(*stream.pack, Encoded::Pack { .. })
                    && matches!(*pack, Encoded::Pack { .. }) =>
            {
                match handed {
                    // The projection's delta, where it is from the pack published last.
                    Some((before, delta)) if Arc::ptr_eq(&before, &stream.pack) => bytes = delta,
                    _ => pack::delta(&stream.pack, &pack, &mut bytes)?,
                }
                (kind::DELTA, stream.keyframe, stream.events + 1)
            }
            _ => {
                pack::keyframe(&pack, &mut bytes);
                (kind::KEYFRAME, at, 1)
            }
        };
        let blob = writer.blob(&mut bytes.as_slice(), bytes.len() as u64)?;
        let fields = if shape == kind::DELTA {
            writer.cells([Cell::Integer(i128::from(keyframe)), Cell::Text(blob)])?
        } else {
            writer.cells([Cell::Text(blob)])?
        };
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: at,
            read_hour: None,
            sequence,
            kind: shape,
            fields,
        })?;
        self.tag_mut(actor).stream = Some(Stream {
            pack,
            keyframe,
            events,
        });
        Ok(())
    }

    /// The consumption segment of each native country `touched`, whose holders or holders'
    /// reads the hour changed.
    fn segments(
        &mut self,
        writer: &mut Writer,
        at: i32,
        adjacent: bool,
        touched: &BTreeSet<u64>,
    ) -> io::Result<()> {
        for &actor in touched {
            let native = match &self.entities[actor as usize] {
                Entity::Native(native) => native,
                _ => unreachable!("a native actor holds a native country"),
            };
            let key = native.key;
            match native.holders.as_slice() {
                [] => {
                    if native.active {
                        self.current.value(writer, key, at, None, false)?;
                        self.native_mut(actor).active = false;
                    }
                }
                &[holder] => {
                    let open = match &self.entities[holder as usize] {
                        Entity::Tag(tag) => tag.open.ok_or_else(invalid)?,
                        _ => unreachable!("a holder is a tag"),
                    };
                    let native = self.native_mut(actor);
                    let joined = native.active
                        && adjacent
                        && native.latest.is_some_and(|(tag, _)| {
                            tag != holder || !open.consumption_starts || open.first != at
                        });
                    let first = match native.latest {
                        Some((_, first)) if joined => first,
                        _ => at,
                    };
                    native.active = true;
                    native.latest = Some((holder, first));
                    let fresh = native.tags.insert(holder);
                    let broken = !joined || self.current.held(key).is_none();
                    self.current
                        .value(writer, key, at, Some(i128::from(first)), broken)?;
                    if fresh {
                        self.mark(actor);
                    }
                }
                holders => {
                    let holders = holders.to_vec();
                    let native = self.native_mut(actor);
                    let mut changed = !native.conflict;
                    native.conflict = true;
                    native.latest = None;
                    for holder in holders {
                        changed |= native.tags.insert(holder);
                    }
                    let active = std::mem::replace(&mut native.active, false);
                    if active {
                        self.current.value(writer, key, at, None, false)?;
                    }
                    if changed {
                        self.mark(actor);
                    }
                }
            }
        }
        Ok(())
    }

    /// Writes the day measures falling due through `through`.
    fn drain(&mut self, writer: &mut Writer, through: i32) -> io::Result<()> {
        while let Some(due) = self.queue.front().copied().filter(|due| due.at <= through) {
            self.queue.pop_front();
            let tag = tag_of(&mut self.entities, due.tag);
            let keys = tag.keys.ok_or_else(invalid)?;
            let read = match due.due {
                Due::Start => Some(tag.unsettled.pop_front().ok_or_else(invalid)?),
                Due::End => None,
            };
            match read {
                Some(read) => {
                    if read.first + LAG != due.at {
                        return Err(invalid());
                    }
                    for (measure, &key) in keys.days.iter().enumerate() {
                        let value = read.values.and_then(|values| values[measure]);
                        if measure < STRIPS {
                            if let Some(value) = value {
                                writer.run(
                                    key,
                                    Span::new(due.at, due.at)?,
                                    value,
                                    read.starts,
                                    false,
                                )?;
                            }
                        } else {
                            self.days.value(writer, key, due.at, value, read.starts)?;
                        }
                    }
                }
                None => {
                    for &key in &keys.days[STRIPS..] {
                        self.days.value(writer, key, due.at, None, false)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours walked through it as known in both domains, and the cells that changed.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.current.cut(writer, last)?;
        self.days.cut(writer, last)?;
        let from = self.cut.map_or(i32::MIN, |cut| cut + 1);
        for (domain, lag) in [(WALKED, 0), (LATE, LAG)] {
            for &(first, end) in &self.walked {
                let (first, end) = ((first + lag).max(from), (end + lag).min(last));
                if first <= end {
                    writer.domain(
                        domain,
                        Evidence {
                            span: Span::new(first, end)?,
                            known: Knownness::Known,
                            supplying_read: None,
                            holder: None,
                            identity_first: None,
                        },
                    )?;
                }
            }
        }
        self.cut = Some(last);
        while self.walked.len() > 1
            && self
                .walked
                .front()
                .is_some_and(|&(_, end)| end + LAG <= last)
        {
            self.walked.pop_front();
        }
        if std::mem::take(&mut self.unnamed_stale) {
            let ids = Scalar::List(
                self.unnamed
                    .iter()
                    .map(|&id| Scalar::Unsigned(u64::from(id)))
                    .collect(),
            );
            let cell = family::cell(writer, &ids)?;
            writer.catalogue(UNNAMED, cell)?;
        }
        if let Some(first) = self.first {
            let bounds = Scalar::List(vec![
                Scalar::Integer(i128::from(first)),
                Scalar::Integer(i128::from(last)),
            ]);
            let cell = family::cell(writer, &bounds)?;
            writer.catalogue(BOUNDS, cell)?;
        }
        for actor in std::mem::take(&mut self.stale) {
            let (id, scalar) = match &mut self.entities[actor as usize] {
                Entity::Tag(tag) => {
                    tag.stale = false;
                    let mut meta = vec![Scalar::Boolean(tag.read)];
                    meta.extend(tag.left_out.map(Scalar::Unsigned));
                    (TAG_META + actor, Scalar::List(meta))
                }
                Entity::Native(native) => {
                    native.stale = false;
                    (
                        ACTOR_META + actor,
                        Scalar::List(vec![
                            Scalar::Unsigned(u64::from(native.country)),
                            Scalar::Boolean(native.conflict),
                            Scalar::List(
                                native
                                    .tags
                                    .iter()
                                    .map(|&tag| Scalar::Unsigned(tag))
                                    .collect(),
                            ),
                        ]),
                    )
                }
                Entity::Unknown => continue,
            };
            let cell = family::cell(writer, &scalar)?;
            writer.catalogue(id, cell)?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection names its tags anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let mut tags = Vec::new();
        let mut natives = Vec::new();
        for (actor, entity) in self.entities.iter().enumerate() {
            match entity {
                Entity::Tag(tag) => tags.push((actor as u64, Cow::Borrowed(tag.as_ref()))),
                Entity::Native(native) => {
                    natives.push((actor as u64, Cow::Borrowed(native.as_ref())))
                }
                Entity::Unknown => {}
            }
        }
        let saved = Saved {
            tags,
            natives,
            queue: self.queue.iter().copied().collect(),
            prior: self.prior.iter().copied().collect(),
            prior_open: self.prior_open,
            walked: self.walked.iter().copied().collect(),
            cut: self.cut,
            first: self.first,
            last: self.last,
            unnamed: self.unnamed.iter().copied().collect(),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        for carry in [&self.current, &self.days] {
            let mut saved = Vec::new();
            carry.save(&mut saved)?;
            out.extend_from_slice(&(saved.len() as u64).to_le_bytes());
            out.extend_from_slice(&saved);
        }
        // Each tag's last demand pack, as a keyframe: the next read's delta starts from it.
        let mut bytes = Vec::new();
        for (actor, entity) in self.entities.iter().enumerate() {
            if let Entity::Tag(tag) = entity
                && let Some(stream) = &tag.stream
            {
                bytes.clear();
                pack::keyframe(&stream.pack, &mut bytes);
                out.extend_from_slice(&(actor as u64).to_le_bytes());
                out.extend_from_slice(&stream.keyframe.to_le_bytes());
                out.push(stream.events);
                out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
                out.extend_from_slice(&bytes);
            }
        }
        Ok(())
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(format: FormatVersion, directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new(format);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.entities = vec![Entity::Unknown; directory.actors.len()];
        let mut id = 0;
        while let Some(scalar) = directory.scalar(REASONS + id) {
            let reason =
                std::str::from_utf8(scalar.text().ok_or_else(invalid)?).map_err(|_| invalid())?;
            emit.reasons.insert(Arc::from(reason), id);
            id += 1;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved<'static> = serde_json::from_slice(d.bytes(length)?)?;
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        emit.current = Carry::load(d.bytes(length)?)?;
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        emit.days = Carry::load(d.bytes(length)?)?;
        let mut streams = Vec::new();
        while d.position() < bytes.len() {
            let actor = d.u64()?;
            let keyframe = i32::from_le_bytes(d.bytes(4)?.try_into().map_err(|_| invalid())?);
            let events = d.bytes(1)?[0];
            let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
            let pack = pack::encoded_of(d.bytes(length)?)?;
            if events == 0 || events > CHAIN {
                return Err(invalid());
            }
            streams.push((actor, keyframe, events, pack));
        }
        for (actor, tag) in saved.tags {
            let entity = emit
                .entities
                .get_mut(actor as usize)
                .filter(|entity| matches!(entity, Entity::Unknown))
                .ok_or_else(invalid)?;
            let mut tag = tag.into_owned();
            let key = |measure: u64| directory.key(actor, measure, None);
            if let Some(read) = key(READ) {
                let mut current = [read; 6];
                for (measure, key_of) in current.iter_mut().enumerate().skip(1) {
                    *key_of = key(measure as u64).ok_or_else(invalid)?;
                }
                let mut days = [0; 15];
                for (measure, key_of) in days.iter_mut().enumerate() {
                    *key_of = key(DAY + measure as u64).ok_or_else(invalid)?;
                }
                tag.keys = Some(Keys {
                    current,
                    days,
                    drops: key(DROPS),
                    demand: key(DEMAND),
                });
            }
            if tag.present {
                emit.present.insert(actor);
            }
            *entity = Entity::Tag(Box::new(tag));
        }
        for (actor, native) in saved.natives {
            let entity = emit
                .entities
                .get_mut(actor as usize)
                .filter(|entity| matches!(entity, Entity::Unknown))
                .ok_or_else(invalid)?;
            let mut native = native.into_owned();
            native.key = directory.key(actor, SEGMENT, None).ok_or_else(invalid)?;
            if *directory.actors[actor as usize].name != native_name(native.country)[..] {
                return Err(invalid());
            }
            emit.natives.insert(native.country, actor);
            *entity = Entity::Native(Box::new(native));
        }
        for due in &saved.queue {
            let tag = match emit.entities.get(due.tag as usize) {
                Some(Entity::Tag(tag)) => tag,
                _ => return Err(invalid()),
            };
            if tag.keys.is_none() {
                return Err(invalid());
            }
        }
        for entity in &emit.entities {
            if let Entity::Tag(tag) = entity
                && let Some(open) = tag.open
                && !matches!(
                    emit.entities.get(open.native as usize),
                    Some(Entity::Native(_))
                )
            {
                return Err(invalid());
            }
        }
        for (actor, keyframe, events, pack) in streams {
            match emit.entities.get_mut(actor as usize) {
                Some(Entity::Tag(tag))
                    if tag.stream.is_none()
                        && tag.keys.is_some_and(|keys| keys.demand.is_some()) =>
                {
                    tag.stream = Some(Stream {
                        pack: Arc::new(pack),
                        keyframe,
                        events,
                    });
                }
                _ => return Err(invalid()),
            }
        }
        emit.queue = saved.queue.into();
        emit.prior = saved.prior.into();
        emit.prior_open = saved.prior_open;
        emit.walked = saved.walked.into();
        emit.cut = saved.cut;
        emit.first = saved.first;
        emit.last = saved.last;
        emit.unnamed = saved.unnamed.into_iter().collect();
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct Saved<'a> {
    tags: Vec<(u64, Cow<'a, Tag>)>,
    natives: Vec<(u64, Cow<'a, Native>)>,
    queue: Vec<Scheduled>,
    prior: Vec<(i32, i32, Sample)>,
    prior_open: Option<(i32, Sample)>,
    walked: Vec<(i32, i32)>,
    cut: Option<i32>,
    first: Option<i32>,
    last: Option<i32>,
    unnamed: Vec<u32>,
}

#[cfg(test)]
mod tests;
