//! Stability's parts emit: each country's parts of stability and war support as runs on the
//! family's series publication, a key per part and measure (the part `""` dating the read
//! that supplies them), each with its holder evidence, presented where it changes; and each
//! country's names: its measures' dating hours and its warnings as an event of its names key,
//! and each part's names with the hour it was first held as an event of a key of its own,
//! split by part under the country's names actor, at the cut after they change.
//!
//! A country is offered where its parts or naming may have changed. A read's parts are worked
//! out once for its holder: named by the projection at the read's own hour, else here, each
//! character as the read's hour recorded it. A run goes on while its holder keeps its tag and
//! the hour follows the last; a tag leaving the countries with a holder and inputs, or a read
//! whose lines were not recorded with it, ends its runs; so does an unread hour.

use super::{
    Character, NAMED_HOURS, Part, assemble,
    facts::{self, PartNames},
};
use crate::products::country_stability::BreakdownLine;
use dashboard_base::prepared::series::{compress_serialized, decompress};
use dashboard_series::{
    Cell, Event, RunMode, Writer,
    carry::{Carry, Read},
    codec::Decode,
    family::{Actors, Directory, Place},
};
use dashboard_text::{catalogue::Refusals, identity::NamingCountry};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    io,
    mem::take,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 2;
/// Each part's measures, by key measure.
pub const MEASURES: [&str; 2] = ["stability", "warSupport"];
/// Fixed5, as every displayed measure; the dating part's read hour shares the scale.
pub const DECIMALS: u8 = dashboard_base::products::country_measure::MEASURE_DECIMALS;
/// The event kind of a country's dating hours and warnings ([`Summary`]).
pub const SUMMARY: u64 = 1;
/// The event kind of a part's names ([`Entry`]).
pub const NAMES: u64 = 2;
/// The measure of the names keys of measure `index`'s parts: `PART_NAMES + index`, under the
/// country's names actor, split by part.
pub const PART_NAMES: u64 = 1;

/// The actor of `tag`'s part `id`: the key a reply names it by.
pub fn part_actor(tag: &str, id: &str) -> io::Result<String> {
    Ok(serde_json::to_string(&(tag, id))?)
}

/// The actor of `tag`'s names, apart from every part's.
pub fn names_actor(tag: &str) -> io::Result<String> {
    Ok(serde_json::to_string(&[tag])?)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// A country's parts as replies name them: each measure's parts by id, as the latest read
/// holding each named it, with the hour it was first held; the hour each measure's dating
/// part was first held; and the country's warnings.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Catalogue {
    pub(super) parts: [BTreeMap<String, (Part, i32)>; 2],
    pub(super) dated: [Option<i32>; 2],
    pub(super) warnings: Vec<String>,
}

/// What a country's summary event holds.
#[derive(Serialize, Deserialize)]
pub(super) struct Summary {
    pub(super) dated: [Option<i32>; 2],
    pub(super) warnings: Vec<String>,
}

/// What a part's names event holds: its id, its names and the hour it was first held.
#[derive(Serialize, Deserialize)]
pub(super) struct Entry {
    pub(super) id: String,
    pub(super) part: Part,
    pub(super) first: i32,
}

/// [`Entry`] as written from what the emit holds.
#[derive(Serialize)]
struct EntryOf<'a> {
    id: &'a str,
    part: PartOf<'a>,
    first: i32,
}

/// [`Part`] as written: a part's runs are its keys', never its names'.
#[derive(Serialize)]
struct PartOf<'a> {
    term: &'a Option<String>,
    line: &'a Option<BreakdownLine>,
    character: &'a Option<Character>,
    country: &'a NamingCountry,
    runs: [(); 0],
}

/// A key's holder: slot, country id, read hour, and its segment's first hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Holder {
    slot: u32,
    country: u32,
    read: i32,
    first: i32,
}

/// One part of one measure of a country: its id, its key and holder, and its names as last
/// handed on and as last published. Slot 0 of each measure is its dating part.
struct Slot {
    id: Arc<str>,
    handle: Option<u64>,
    holder: Option<Holder>,
    open: bool,
    /// The projection that last held it.
    mark: u32,
    /// Its names hold a line, whose country names it.
    lined: bool,
    names: Option<Arc<PartNames>>,
    country: Option<Arc<NamingCountry>>,
    /// The hour it was first held.
    first: Option<i32>,
    /// Its names may have changed since the last cut.
    dirty: bool,
    names_key: Option<u64>,
    published: Option<Box<str>>,
}

impl Slot {
    fn new(id: Arc<str>) -> Self {
        Self {
            id,
            handle: None,
            holder: None,
            open: false,
            mark: 0,
            lined: false,
            names: None,
            country: None,
            first: None,
            dirty: false,
            names_key: None,
            published: None,
        }
    }
}

#[derive(Default)]
struct Measure {
    slots: Vec<Slot>,
    /// Each part's slot by the emit's id.
    by_id: HashMap<u32, u32>,
    /// The open slots, its dating part among them.
    open: Vec<u32>,
    epoch: u32,
    /// The hour its dating part was first held.
    dated: Option<i32>,
}

/// What the parts of a tag last held.
#[derive(Default)]
struct Track {
    /// Its holder and the last hour it was offered at.
    last: Option<((u32, u32), i32)>,
    /// The holder and read whose parts it holds, and each measure's parts by slot.
    current: Option<((u32, u32), i32, [Option<Vec<(u32, i64)>>; 2])>,
    /// Read hours that recorded no lines with their inputs, and those that did.
    unlined: BTreeSet<i32>,
    lined: BTreeSet<i32>,
    /// How many retired read hours recorded no lines ([`Emit::retire`]).
    unlined_before: usize,
    available: bool,
    naming: Option<Arc<NamingCountry>>,
    refused: Refusals,
}

impl Track {
    fn warnings(&self, tag: &str) -> Vec<String> {
        let missing = self.unlined_before + self.unlined.difference(&self.lined).count();
        let mut warnings = Vec::new();
        if missing != 0 {
            warnings.push(format!(
                "{tag}: {missing} reads recorded no modifier lines with their inputs; their parts are not known."
            ));
        }
        warnings.extend(self.refused.lines("stability tree labels are"));
        warnings
    }
}

struct Country {
    tag: Arc<str>,
    track: Track,
    measures: [Measure; 2],
    /// It holds the countries' place for its tag with inputs and a read.
    active: bool,
    /// From the first hour its parts were projected.
    catalogued: bool,
    /// Its dating hours or warnings may have changed since the last cut.
    summary_dirty: bool,
    /// The summary last published.
    summary: Option<Box<str>>,
    names_actor: Option<u64>,
    summary_key: Option<u64>,
    /// Among the countries to publish at the next cut.
    pending: bool,
    /// Its saved ids and published names, while none changed.
    heavy: Option<Vec<u8>>,
}

impl Country {
    fn new(tag: Arc<str>) -> Self {
        let measure = || Measure {
            slots: vec![Slot::new(Arc::from(""))],
            ..Measure::default()
        };
        Self {
            tag,
            track: Track::default(),
            measures: [measure(), measure()],
            active: false,
            catalogued: false,
            summary_dirty: false,
            summary: None,
            names_actor: None,
            summary_key: None,
            pending: false,
            heavy: None,
        }
    }
}

/// Each character's naming as the hours recorded it, and whether they recorded any: what a
/// read met after its hour names its character lines by.
#[derive(Default, Serialize, Deserialize)]
struct Recorded {
    /// By character id, from the hour each naming was recorded, oldest first; None where gone.
    characters: HashMap<u32, Vec<(i32, Option<Character>)>>,
    /// From which hours Characters was complete, oldest first.
    known: Vec<(i32, bool)>,
    /// It changed since it was last saved.
    #[serde(skip)]
    changed: bool,
}

impl Recorded {
    fn known_from(&mut self, hour: i32, known: bool) {
        if self.known.last().map(|&(_, held)| held) != Some(known) {
            self.known.push((hour, known));
            self.changed = true;
        }
    }

    fn set(&mut self, id: u32, hour: i32, naming: Option<Character>) {
        let history = self.characters.entry(id).or_default();
        match history.last_mut() {
            Some((_, held)) if *held == naming => return,
            Some((from, held)) if *from == hour => *held = naming,
            _ => history.push((hour, naming)),
        }
        self.changed = true;
    }

    /// The hour's characters.
    fn hour(&mut self, hour: i32, characters: facts::Characters) {
        match characters {
            facts::Characters::Unknown => self.known_from(hour, false),
            facts::Characters::Every(every) => {
                self.known_from(hour, true);
                let mut seen = HashSet::with_capacity(every.len());
                for (id, naming) in every {
                    seen.insert(id);
                    self.set(id, hour, Some(naming));
                }
                let gone: Vec<u32> = self
                    .characters
                    .iter()
                    .filter(|(id, history)| {
                        !seen.contains(id)
                            && history.last().is_some_and(|(_, naming)| naming.is_some())
                    })
                    .map(|(&id, _)| id)
                    .collect();
                for id in gone {
                    self.set(id, hour, None);
                }
            }
            facts::Characters::Changed(changed) => {
                self.known_from(hour, true);
                for (id, naming) in changed {
                    self.set(id, hour, naming);
                }
            }
        }
    }

    /// Character `id` as hour `hour` recorded it; `hour` must be one the walk read.
    fn at(&self, id: u32, hour: i32) -> Option<Character> {
        let at = self.known.partition_point(|&(from, _)| from <= hour);
        if !self.known[..at].last().is_some_and(|&(_, known)| known) {
            return None;
        }
        let history = self.characters.get(&id)?;
        let at = history.partition_point(|&(from, _)| from <= hour);
        history[..at].last()?.1.clone()
    }

    /// Forgets what no hour from `floor` on reads.
    fn retire(&mut self, floor: i32) {
        let keep = |held: usize, from: &dyn Fn(usize) -> i32| {
            (0..held).take_while(|&at| at + 1 < held && from(at + 1) <= floor).count()
        };
        let drop = keep(self.known.len(), &|at| self.known[at].0);
        let mut changed = drop != 0;
        self.known.drain(..drop);
        self.characters.retain(|_, history| {
            let drop = keep(history.len(), &|at| history[at].0);
            changed |= drop != 0;
            history.drain(..drop);
            let gone = history.len() == 1 && history[0].0 <= floor && history[0].1.is_none();
            changed |= gone;
            !gone
        });
        self.changed |= changed;
    }
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    countries: Vec<Country>,
    by_tag: HashMap<Arc<str>, u32>,
    /// Each projection index's country, for the segment under way.
    names: Vec<Option<u32>>,
    /// Every part id, numbered as first met, and each projection part index's number.
    ids: HashMap<Arc<str>, u32>,
    id_text: Vec<Arc<str>>,
    part_ids: Vec<Option<u32>>,
    last: Option<i32>,
    recorded: Recorded,
    /// Recorded as last saved, while it did not change.
    recorded_saved: Option<Vec<u8>>,
    /// The last hours read, ascending: a read is named only while its hour is among them, as
    /// the walk names it from the frames it keeps.
    observed: VecDeque<i32>,
    /// Every country's inputs read hour, by id, as of the last hour whose inputs were read
    /// whole, and how many countries each hour dates: no later hour dates a read before the
    /// earliest.
    dated: HashMap<u32, i32>,
    admitted: BTreeMap<i32, usize>,
    /// Countries with names to publish at the next cut.
    pending: Vec<u32>,
    /// Countries whose runs end at the hour.
    removed: Vec<u32>,
    blank: NamingCountry,
    json: Vec<u8>,
}

/// One key's value at `hour`: a holder continuing the key's segment keeps its first hour;
/// any other starts one, with a break.
fn record(
    carry: &mut Carry,
    writer: &mut Writer<'_>,
    (handle, holder): (u64, &mut Option<Holder>),
    hour: i32,
    value: i128,
    joined: bool,
    (slot, country, read): (u32, u32, i32),
) -> io::Result<()> {
    let held = *holder;
    let same = joined && held.is_some_and(|held| (held.slot, held.country) == (slot, country));
    carry.value(writer, handle, hour, Some(value), !same)?;
    let next = Holder {
        slot,
        country,
        read,
        first: match held.filter(|_| same) {
            Some(held) => held.first,
            None => hour,
        },
    };
    if held != Some(next) {
        carry.evidence(
            writer,
            handle,
            hour,
            Some(Read {
                holder: Some(u64::from(slot) << 32 | u64::from(country)),
                read: Some(read),
                identity_first: Some(next.first),
                presented: true,
            }),
        )?;
        *holder = Some(next);
    }
    Ok(())
}

/// The key of `tag`'s part in `slot` of measure `index`, interned at `hour` when new.
fn key(
    actors: &mut Actors,
    writer: &mut Writer<'_>,
    slot: &mut Slot,
    (tag, index): (&str, usize),
    hour: i32,
) -> io::Result<u64> {
    if let Some(handle) = slot.handle {
        return Ok(handle);
    }
    let actor = actors.actor(writer, part_actor(tag, &slot.id)?.as_bytes(), hour)?;
    let handle = actors.key(
        writer,
        Place {
            actor,
            measure: index as u64,
            dimension: None,
        },
        (RunMode::Exact, DECIMALS),
        None,
    )?;
    slot.handle = Some(handle);
    Ok(handle)
}

/// Ends the run and evidence of `tag`'s part in `slot` of measure `index` before `hour`.
fn close(
    actors: &Actors,
    carry: &mut Carry,
    writer: &mut Writer<'_>,
    slot: &mut Slot,
    (tag, index): (&str, usize),
    hour: i32,
) -> io::Result<()> {
    slot.open = false;
    let handle = match slot.handle {
        Some(handle) => handle,
        None => {
            let found = actors
                .find(part_actor(tag, &slot.id)?.as_bytes())
                .and_then(|actor| {
                    actors.held(Place {
                        actor,
                        measure: index as u64,
                        dimension: None,
                    })
                });
            let Some(handle) = found else {
                return Ok(());
            };
            slot.handle = Some(handle);
            handle
        }
    };
    carry.value(writer, handle, hour, None, false)?;
    carry.evidence(writer, handle, hour, None)?;
    slot.holder = None;
    Ok(())
}

/// Ends every open run of `country` before `hour`.
fn close_all(
    actors: &Actors,
    carry: &mut Carry,
    writer: &mut Writer<'_>,
    country: &mut Country,
    hour: i32,
) -> io::Result<()> {
    for index in 0..2 {
        let measure = &mut country.measures[index];
        for slot in take(&mut measure.open) {
            close(
                actors,
                carry,
                writer,
                &mut measure.slots[slot as usize],
                (&*country.tag, index),
                hour,
            )?;
        }
    }
    Ok(())
}

/// `measure`'s slot of the emit's part id `id`, made when new.
fn slot_of(
    measure: &mut Measure,
    heavy: &mut Option<Vec<u8>>,
    id_text: &[Arc<str>],
    id: u32,
) -> io::Result<u32> {
    if let Some(&slot) = measure.by_id.get(&id) {
        return Ok(slot);
    }
    let text = id_text
        .get(id as usize)
        .ok_or_else(|| invalid("A stability part was never numbered"))?;
    let slot = measure.slots.len() as u32;
    measure.slots.push(Slot::new(Arc::clone(text)));
    measure.by_id.insert(id, slot);
    *heavy = None;
    Ok(slot)
}

/// Whether `held` names a country as `naming` does.
fn same_naming(held: Option<&Arc<NamingCountry>>, naming: &Arc<NamingCountry>) -> bool {
    held.is_some_and(|held| Arc::ptr_eq(held, naming) || **held == **naming)
}

/// Keeps `country`'s held parts at `hour`, its holder `identity` with read `read`.
fn project(
    actors: &mut Actors,
    carry: &mut Carry,
    writer: &mut Writer<'_>,
    country: &mut Country,
    hour: i32,
    (identity, read): ((u32, u32), i32),
    joined: bool,
) -> io::Result<bool> {
    let Country {
        tag,
        track,
        measures,
        catalogued,
        summary_dirty,
        ..
    } = country;
    let tag: &str = tag;
    let (Some((_, _, values)), Some(naming)) = (&track.current, &track.naming) else {
        return Ok(false);
    };
    let mut changed = false;
    if !*catalogued {
        *catalogued = true;
        *summary_dirty = true;
        changed = true;
    }
    let holder = (identity.0, identity.1, read);
    for (index, measure) in measures.iter_mut().enumerate() {
        let parts = values[index].as_deref();
        measure.epoch = measure.epoch.wrapping_add(1);
        let epoch = measure.epoch;
        if let Some(parts) = parts {
            measure.slots[0].mark = epoch;
            for &(slot, _) in parts {
                measure.slots[slot as usize].mark = epoch;
            }
        }
        let mut open = take(&mut measure.open);
        for &slot in &open {
            let held = &mut measure.slots[slot as usize];
            if held.mark != epoch {
                close(actors, carry, writer, held, (tag, index), hour)?;
            }
        }
        open.clear();
        let Some(parts) = parts else {
            measure.open = open;
            continue;
        };
        open.push(0);
        open.extend(parts.iter().map(|&(slot, _)| slot));
        for &slot in &open {
            measure.slots[slot as usize].open = true;
        }
        measure.open = open;
        if measure.dated.is_none() {
            measure.dated = Some(hour);
            *summary_dirty = true;
            changed = true;
        }
        for &(slot, _) in parts {
            let held = &mut measure.slots[slot as usize];
            if held.first.is_none() {
                held.first = Some(hour);
                held.dirty = true;
                changed = true;
            }
            if held.lined && !same_naming(held.country.as_ref(), naming) {
                held.country = Some(Arc::clone(naming));
                held.dirty = true;
                changed = true;
            }
        }
        // The empty part dates the read supplying them.
        for (slot, raw) in std::iter::once((0, i64::from(read))).chain(parts.iter().copied()) {
            let held = &mut measure.slots[slot as usize];
            let handle = key(actors, writer, held, (tag, index), hour)?;
            record(
                carry,
                writer,
                (handle, &mut held.holder),
                hour,
                i128::from(raw),
                joined,
                holder,
            )?;
        }
    }
    Ok(changed)
}

/// The names `names` give a part, apart from its country.
fn names_of(value: super::Value) -> (Arc<str>, i64, PartNames) {
    let super::Value {
        id,
        term,
        line,
        character,
        raw,
    } = value;
    (
        Arc::from(id),
        raw,
        PartNames {
            term,
            line,
            character,
        },
    )
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            countries: Vec::new(),
            by_tag: HashMap::new(),
            names: Vec::new(),
            ids: HashMap::new(),
            id_text: Vec::new(),
            part_ids: Vec::new(),
            last: None,
            recorded: Recorded::default(),
            recorded_saved: None,
            observed: VecDeque::new(),
            dated: HashMap::new(),
            admitted: BTreeMap::new(),
            pending: Vec::new(),
            removed: Vec::new(),
            blank: NamingCountry::default(),
            json: Vec::new(),
        }
    }

    fn country_of(&mut self, tag: Arc<str>) -> u32 {
        if let Some(&country) = self.by_tag.get(&tag) {
            return country;
        }
        let country = self.countries.len() as u32;
        self.countries.push(Country::new(Arc::clone(&tag)));
        self.by_tag.insert(tag, country);
        country
    }

    /// Country `id` dated at `read`, or no more.
    fn date(&mut self, id: u32, read: Option<i32>) {
        let held = match read {
            Some(read) => self.dated.insert(id, read),
            None => self.dated.remove(&id),
        };
        if held == read {
            return;
        }
        if let Some(old) = held
            && let Some(count) = self.admitted.get_mut(&old)
        {
            *count -= 1;
            if *count == 0 {
                self.admitted.remove(&old);
            }
        }
        if let Some(read) = read {
            *self.admitted.entry(read).or_default() += 1;
        }
    }

    fn dates(&mut self, reads: facts::Reads) {
        match reads {
            facts::Reads::Every(every) => {
                self.dated.clear();
                self.admitted.clear();
                for (id, read) in every {
                    self.date(id, Some(read));
                }
            }
            facts::Reads::Changed(changed) => {
                for (id, read) in changed {
                    self.date(id, read);
                }
            }
        }
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("parts.emit", None);
        let at = hour.hour;
        let previous = self.last;
        let adjacent = previous.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = previous.filter(|_| !adjacent) {
            // An unread hour ends every run; the next offers start their segments.
            self.carry.close(writer, last)?;
            for country in &mut self.countries {
                for measure in &mut country.measures {
                    for slot in &mut measure.slots {
                        slot.holder = None;
                    }
                }
            }
        }
        self.last = Some(at);
        self.observed.push_back(at);
        while self.observed.len() > NAMED_HOURS {
            self.observed.pop_front();
        }
        for (index, tag) in hour.named {
            let index = index as usize;
            if index >= self.names.len() {
                self.names.resize(index + 1, None);
            }
            self.names[index] = Some(self.country_of(tag));
        }
        for (index, id) in hour.ids {
            let index = index as usize;
            if index >= self.part_ids.len() {
                self.part_ids.resize(index + 1, None);
            }
            self.part_ids[index] = Some(number_of(&mut self.ids, &mut self.id_text, id));
        }
        let name = |names: &[Option<u32>], index: u32| {
            names
                .get(index as usize)
                .copied()
                .flatten()
                .ok_or_else(|| invalid("A stability parts country was never named"))
        };
        self.recorded.hour(at, hour.characters);
        if let Some(reads) = hour.reads {
            self.dates(reads);
        }
        self.removed.clear();
        if let Some(valid) = hour.valid {
            let mut now = vec![false; self.countries.len()];
            for index in valid {
                now[name(&self.names, index)? as usize] = true;
            }
            for (position, country) in self.countries.iter_mut().enumerate() {
                if country.active && !now[position] {
                    self.removed.push(position as u32);
                    country.track.last = None;
                }
                country.active = now[position];
            }
        }
        let previous = previous.filter(|_| adjacent);
        for offered in hour.countries {
            let country = name(&self.names, offered.index)?;
            self.offer(writer, at, previous, country, offered)?;
        }
        let Self {
            actors,
            carry,
            countries,
            removed,
            ..
        } = self;
        for &country in removed.iter() {
            close_all(actors, carry, writer, &mut countries[country as usize], at)?;
        }
        Ok(())
    }

    /// One offered country at `hour`; `previous` the hour before it, where it was read.
    fn offer(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        previous: Option<i32>,
        index: u32,
        offered: facts::Country,
    ) -> io::Result<()> {
        let Self {
            actors,
            carry,
            countries,
            ids,
            id_text,
            part_ids,
            recorded,
            observed,
            pending,
            removed,
            ..
        } = self;
        let identity = offered.holder;
        let read = offered.read;
        let country = &mut countries[index as usize];
        let mut changed = false;
        let active = country.active;
        let track = &mut country.track;
        // A tag held on unread by its holder continues to the hour before.
        if let Some(previous) = previous
            && active
            && let Some((_, last)) = &mut track.last
        {
            *last = previous;
        }
        let joined = track
            .last
            .is_some_and(|(seen, last)| seen == identity && last.checked_add(1) == Some(hour));
        track.last = Some((identity, hour));
        let fresh = track
            .current
            .as_ref()
            .is_none_or(|(seen, at, _)| (*seen, *at) != (identity, read));
        if !offered.lined {
            if track.unlined.insert(read) {
                country.summary_dirty = true;
                changed = true;
            }
            track.available = false;
            track.naming = None;
            removed.push(index);
        } else if fresh {
            let parts = offered.parts.as_ref().ok_or_else(|| {
                invalid("A stability parts read new to the emit came without its parts")
            })?;
            let number = |part_ids: &[Option<u32>], id: u32| {
                part_ids
                    .get(id as usize)
                    .copied()
                    .flatten()
                    .ok_or_else(|| invalid("A stability part was never numbered"))
            };
            match parts {
                facts::Parts::Same => {
                    let current = track.current.as_mut().ok_or_else(|| {
                        invalid("A stability parts read handed on as the same holds none")
                    })?;
                    current.0 = identity;
                    current.1 = read;
                }
                facts::Parts::Named(named) => {
                    let mut values: [Option<Vec<(u32, i64)>>; 2] = [None, None];
                    for (at, parts) in named.measures.iter().enumerate() {
                        let Some(parts) = parts else {
                            continue;
                        };
                        let measure = &mut country.measures[at];
                        let mut slots = Vec::with_capacity(parts.len());
                        for &(id, raw) in parts {
                            let id = number(&part_ids[..], id)?;
                            slots.push((slot_of(measure, &mut country.heavy, id_text, id)?, raw));
                        }
                        values[at] = Some(slots);
                    }
                    for (at, id, names) in &named.names {
                        let measure = country
                            .measures
                            .get_mut(usize::from(*at))
                            .ok_or_else(|| invalid("A stability part names no measure"))?;
                        let id = number(&part_ids[..], *id)?;
                        let slot = slot_of(measure, &mut country.heavy, id_text, id)?;
                        let held = &mut measure.slots[slot as usize];
                        held.lined = names.line.is_some();
                        held.names = Some(Arc::clone(names));
                        held.dirty = true;
                    }
                    if !named.refused.is_empty() {
                        track.refused.merge(&named.refused);
                        country.summary_dirty = true;
                    }
                    track.current = Some((identity, read, values));
                }
                facts::Parts::Unnamed(unnamed) => {
                    // Named as the read's own hour recorded its characters, while the walk
                    // keeps that hour.
                    let kept = observed.binary_search(&read).is_ok();
                    let character = |line: &BreakdownLine| {
                        kept.then(|| recorded.at(line.identity_id, read)).flatten()
                    };
                    let mut values: [Option<Vec<(u32, i64)>>; 2] = [None, None];
                    for (at, addends) in unnamed.measures.iter().enumerate() {
                        let Some(addends) = addends else {
                            continue;
                        };
                        let measure = &mut country.measures[at];
                        let mut slots = Vec::new();
                        for value in assemble(addends, &character) {
                            let (id, raw, names) = names_of(value);
                            let id = number_of(ids, id_text, id);
                            let slot = slot_of(measure, &mut country.heavy, id_text, id)?;
                            let held = &mut measure.slots[slot as usize];
                            if held.names.as_deref() != Some(&names) {
                                held.lined = names.line.is_some();
                                held.names = Some(Arc::new(names));
                                held.dirty = true;
                            }
                            slots.push((slot, raw));
                        }
                        values[at] = Some(slots);
                    }
                    if !unnamed.refused.is_empty() {
                        track.refused.merge(&unnamed.refused);
                        country.summary_dirty = true;
                    }
                    track.current = Some((identity, read, values));
                }
            }
            track.lined.insert(read);
            country.summary_dirty = true;
            changed = true;
        }
        if offered.lined {
            let naming = offered
                .naming
                .ok_or_else(|| invalid("A lined stability parts read came without its naming"))?;
            let track = &mut country.track;
            let same = joined
                && !fresh
                && track.available
                && same_naming(track.naming.as_ref(), &naming);
            if !same {
                track.available = true;
                track.naming = Some(naming);
                changed |= project(actors, carry, writer, country, hour, (identity, read), joined)?;
            }
        }
        if changed && !country.pending {
            country.pending = true;
            pending.push(index);
        }
        Ok(())
    }

    /// Retires each tag's read hours below the earliest read the last hour whose inputs were
    /// read whole dated, keeping how many recorded no lines: a complete read's hour only moves
    /// on, and a truncated read's is its own hour, so no later hour reads one again; and the
    /// characters' namings before every hour still kept.
    fn retire(&mut self) {
        if let Some(&oldest) = self.observed.front() {
            self.recorded.retire(oldest);
        }
        let Some((&floor, _)) = self.admitted.first_key_value() else {
            return;
        };
        for country in &mut self.countries {
            let track = &mut country.track;
            let unlined = track.unlined.split_off(&floor);
            let lined = track.lined.split_off(&floor);
            track.unlined_before += track.unlined.difference(&track.lined).count();
            track.unlined = unlined;
            track.lined = lined;
        }
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, and each country's names that changed since the last cut.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("parts.emit.cut", None);
        self.retire();
        let Self {
            actors,
            carry,
            countries,
            pending,
            recorded,
            recorded_saved,
            blank,
            json,
            ..
        } = self;
        for country in take(pending) {
            let country = &mut countries[country as usize];
            country.pending = false;
            if country.catalogued {
                publish(actors, writer, country, last, blank, json)?;
            } else {
                country.summary_dirty = false;
            }
        }
        for country in countries.iter_mut().filter(|country| country.heavy.is_none()) {
            country.heavy = Some(heavy_of(country)?);
        }
        if recorded.changed || recorded_saved.is_none() {
            *recorded_saved = Some(compress_serialized(&*recorded, 1)?);
            recorded.changed = false;
        }
        carry.cut(writer, last)
    }

    /// What the emit goes on with after a cut: every tag's track, parts and open keys, the
    /// names it published, the characters' recorded namings and its open edges.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("parts.emit.save", None);
        let saved = Saved {
            last: self.last,
            observed: self.observed.iter().copied().collect(),
            dated: {
                let mut dated: Vec<(u32, i32)> =
                    self.dated.iter().map(|(&id, &read)| (id, read)).collect();
                dated.sort_unstable();
                dated
            },
            admitted: self
                .admitted
                .iter()
                .map(|(&hour, &count)| (hour, count))
                .collect(),
            countries: self.countries.iter().map(SavedCountry::of).collect(),
        };
        put(out, &compress_serialized(&saved, 1)?);
        out.extend_from_slice(&(self.countries.len() as u64).to_le_bytes());
        for country in &self.countries {
            match &country.heavy {
                Some(heavy) => put(out, heavy),
                None => put(out, &heavy_of(country)?),
            }
        }
        match &self.recorded_saved {
            Some(recorded) if !self.recorded.changed => put(out, recorded),
            _ => put(out, &compress_serialized(&self.recorded, 1)?),
        }
        self.carry.save(out)
    }

    /// The emit going on after a cut of the publication `directory` reads.
    pub fn resume(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let malformed = || invalid("Saved stability parts emit is not valid");
        let next = |d: &mut Decode<'_>| -> io::Result<Vec<u8>> {
            let length = usize::try_from(d.u64()?).map_err(|_| malformed())?;
            Ok(d.bytes(length)?.to_vec())
        };
        let mut d = Decode::new(bytes);
        let saved: Saved = serde_json::from_slice(&decompress(&next(&mut d)?)?)?;
        let count = usize::try_from(d.u64()?).map_err(|_| malformed())?;
        if count != saved.countries.len() {
            return Err(malformed());
        }
        let mut heavies = Vec::with_capacity(count);
        for _ in 0..count {
            let raw = next(&mut d)?;
            let heavy: Heavy = serde_json::from_slice(&decompress(&raw)?)?;
            heavies.push((heavy, raw));
        }
        let raw = next(&mut d)?;
        let recorded: Recorded = serde_json::from_slice(&decompress(&raw)?)?;
        let at = d.position();
        let mut emit = Self {
            actors: Actors::resume(SCHEMA, directory)?,
            carry: Carry::load(&bytes[at..])?,
            recorded,
            recorded_saved: Some(raw),
            last: saved.last,
            observed: saved.observed.into_iter().collect(),
            admitted: saved.admitted.into_iter().collect(),
            dated: saved.dated.into_iter().collect(),
            ..Self::new()
        };
        for (saved, (heavy, raw)) in saved.countries.into_iter().zip(heavies) {
            let index = emit.countries.len() as u32;
            let tag: Arc<str> = Arc::from(saved.tag);
            let mut country = Country::new(Arc::clone(&tag));
            let mut dirty = saved.summary_dirty;
            for (((measure, held), ids), published) in country
                .measures
                .iter_mut()
                .zip(saved.measures)
                .zip(heavy.ids)
                .zip(heavy.published)
            {
                if held.slots.len() != ids.len() || ids.len() != published.len() || ids.is_empty()
                {
                    return Err(malformed());
                }
                measure.slots.clear();
                for (at, ((slot, id), published)) in
                    held.slots.into_iter().zip(ids).zip(published).enumerate()
                {
                    let id: Arc<str> = Arc::from(id);
                    if at != 0 {
                        let number = number_of(&mut emit.ids, &mut emit.id_text, Arc::clone(&id));
                        measure.by_id.insert(number, at as u32);
                    }
                    dirty |= slot.dirty;
                    measure.slots.push(Slot {
                        holder: slot.holder,
                        first: slot.first,
                        lined: slot.lined,
                        dirty: slot.dirty,
                        published: published.map(String::into_boxed_str),
                        ..Slot::new(id)
                    });
                }
                for &slot in &held.open {
                    measure
                        .slots
                        .get_mut(slot as usize)
                        .ok_or_else(malformed)?
                        .open = true;
                }
                measure.open = held.open;
                measure.dated = held.dated;
            }
            country.track = Track {
                last: saved.last,
                current: saved.current,
                unlined: saved.unlined.into_iter().collect(),
                lined: saved.lined.into_iter().collect(),
                unlined_before: saved.unlined_before,
                available: saved.available,
                naming: saved.naming.map(Arc::new),
                refused: saved.refused,
            };
            country.active = saved.active;
            country.catalogued = saved.catalogued;
            country.summary_dirty = saved.summary_dirty;
            country.summary = saved.summary.map(String::into_boxed_str);
            country.heavy = Some(raw);
            if dirty {
                country.pending = true;
                emit.pending.push(index);
            }
            emit.by_tag.insert(tag, index);
            emit.countries.push(country);
        }
        Ok(emit)
    }
}

/// Part id `id`'s number, given when new.
fn number_of(ids: &mut HashMap<Arc<str>, u32>, id_text: &mut Vec<Arc<str>>, id: Arc<str>) -> u32 {
    if let Some(&number) = ids.get(&id) {
        return number;
    }
    let number = id_text.len() as u32;
    id_text.push(Arc::clone(&id));
    ids.insert(id, number);
    number
}

/// Writes one event of `kind` on `key`, of `json`, at the cut ending at `last`.
fn event(writer: &mut Writer<'_>, key: u64, last: i32, kind: u64, json: &[u8]) -> io::Result<()> {
    let text = writer.blob(&mut &json[..], json.len() as u64)?;
    let fields = writer.cells([Cell::Text(text)])?;
    let sequence = writer.sequence()?;
    writer.event(Event {
        key,
        native_hour: last,
        read_hour: None,
        sequence,
        kind,
        fields,
    })
}

/// [`Summary`] as written.
#[derive(Serialize)]
struct SummaryOf<'a> {
    dated: [Option<i32>; 2],
    warnings: &'a [String],
}

/// Publishes what changed of `country`'s names at the cut ending at `last`: its summary, and
/// each part's names, where they differ from what it published last.
fn publish(
    actors: &mut Actors,
    writer: &mut Writer<'_>,
    country: &mut Country,
    last: i32,
    blank: &NamingCountry,
    json: &mut Vec<u8>,
) -> io::Result<()> {
    let actor = match country.names_actor {
        Some(actor) => actor,
        None => {
            let actor = actors.actor(writer, names_actor(&country.tag)?.as_bytes(), last)?;
            country.names_actor = Some(actor);
            actor
        }
    };
    if take(&mut country.summary_dirty) {
        json.clear();
        let warnings = country.track.warnings(&country.tag);
        serde_json::to_writer(
            &mut *json,
            &SummaryOf {
                dated: [country.measures[0].dated, country.measures[1].dated],
                warnings: &warnings,
            },
        )?;
        if country.summary.as_deref().map(str::as_bytes) != Some(&json[..]) {
            let key = match country.summary_key {
                Some(key) => key,
                None => {
                    let key = actors.key(
                        writer,
                        Place {
                            actor,
                            measure: 0,
                            dimension: None,
                        },
                        (RunMode::Exact, 0),
                        None,
                    )?;
                    country.summary_key = Some(key);
                    key
                }
            };
            event(writer, key, last, SUMMARY, json)?;
            country.summary = Some(text_of(json)?);
        }
    }
    let Country {
        measures, heavy, ..
    } = country;
    for (index, measure) in measures.iter_mut().enumerate() {
        for (at, slot) in measure.slots.iter_mut().enumerate().skip(1) {
            if !take(&mut slot.dirty) {
                continue;
            }
            let first = slot
                .first
                .ok_or_else(|| invalid("A stability part's names came before it was held"))?;
            json.clear();
            match &slot.names {
                Some(names) => {
                    let country = match &slot.country {
                        Some(country) if slot.lined => &**country,
                        _ => blank,
                    };
                    serde_json::to_writer(
                        &mut *json,
                        &EntryOf {
                            id: &slot.id,
                            part: PartOf {
                                term: &names.term,
                                line: &names.line,
                                character: &names.character,
                                country,
                                runs: [],
                            },
                            first,
                        },
                    )?;
                }
                None => {
                    let published = slot
                        .published
                        .as_deref()
                        .ok_or_else(|| invalid("A stability part's names were never handed on"))?;
                    let mut entry: Entry = serde_json::from_str(published)?;
                    if slot.lined
                        && let Some(country) = &slot.country
                    {
                        entry.part.country = (**country).clone();
                    }
                    entry.first = first;
                    serde_json::to_writer(&mut *json, &entry)?;
                }
            }
            if slot.published.as_deref().map(str::as_bytes) == Some(&json[..]) {
                continue;
            }
            let key = match slot.names_key {
                Some(key) => key,
                None => {
                    let key = actors.key(
                        writer,
                        Place {
                            actor,
                            measure: PART_NAMES + index as u64,
                            dimension: Some(at as u64),
                        },
                        (RunMode::Exact, 0),
                        None,
                    )?;
                    slot.names_key = Some(key);
                    key
                }
            };
            event(writer, key, last, NAMES, json)?;
            slot.published = Some(text_of(json)?);
            *heavy = None;
        }
    }
    Ok(())
}

fn text_of(json: &[u8]) -> io::Result<Box<str>> {
    Ok(std::str::from_utf8(json)
        .map_err(|_| invalid("Stability part names are not UTF-8"))?
        .into())
}

fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// A country's part ids and the names it published of each, as saved.
#[derive(Serialize)]
struct HeavyOf<'a> {
    ids: [Vec<&'a str>; 2],
    published: [Vec<Option<&'a str>>; 2],
}

#[derive(Deserialize)]
struct Heavy {
    ids: [Vec<String>; 2],
    published: [Vec<Option<String>>; 2],
}

fn heavy_of(country: &Country) -> io::Result<Vec<u8>> {
    compress_serialized(
        &HeavyOf {
            ids: country
                .measures
                .each_ref()
                .map(|measure| measure.slots.iter().map(|slot| &*slot.id).collect()),
            published: country.measures.each_ref().map(|measure| {
                measure
                    .slots
                    .iter()
                    .map(|slot| slot.published.as_deref())
                    .collect()
            }),
        },
        1,
    )
}

/// What the emit saves apart from each country's ids and published names.
#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    observed: Vec<i32>,
    dated: Vec<(u32, i32)>,
    admitted: Vec<(i32, usize)>,
    countries: Vec<SavedCountry>,
}

#[derive(Serialize, Deserialize)]
struct SavedSlot {
    holder: Option<Holder>,
    first: Option<i32>,
    lined: bool,
    dirty: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedMeasure {
    open: Vec<u32>,
    dated: Option<i32>,
    slots: Vec<SavedSlot>,
}

#[derive(Serialize, Deserialize)]
struct SavedCountry {
    tag: String,
    active: bool,
    catalogued: bool,
    summary_dirty: bool,
    summary: Option<String>,
    last: Option<((u32, u32), i32)>,
    current: Option<((u32, u32), i32, [Option<Vec<(u32, i64)>>; 2])>,
    unlined: Vec<i32>,
    lined: Vec<i32>,
    unlined_before: usize,
    available: bool,
    naming: Option<NamingCountry>,
    refused: Refusals,
    measures: [SavedMeasure; 2],
}

impl SavedCountry {
    fn of(country: &Country) -> Self {
        let track = &country.track;
        Self {
            tag: country.tag.to_string(),
            active: country.active,
            catalogued: country.catalogued,
            summary_dirty: country.summary_dirty,
            summary: country.summary.as_deref().map(str::to_owned),
            last: track.last,
            current: track.current.clone(),
            unlined: track.unlined.iter().copied().collect(),
            lined: track.lined.iter().copied().collect(),
            unlined_before: track.unlined_before,
            available: track.available,
            naming: track.naming.as_deref().cloned(),
            refused: track.refused.clone(),
            measures: country.measures.each_ref().map(|measure| SavedMeasure {
                open: measure.open.clone(),
                dated: measure.dated,
                slots: measure
                    .slots
                    .iter()
                    .map(|slot| SavedSlot {
                        holder: slot.holder,
                        first: slot.first,
                        lined: slot.lined,
                        dirty: slot.dirty,
                    })
                    .collect(),
            }),
        }
    }
}
