//! Research folded per segment on the walk's workers ([`Lens`]). From a segment's first hour
//! every country folds from its last read before the segment, read again as a first read, then
//! through every hour as the ordered emit would fold it. From each country's first read a day
//! into the segment on, the lens hands the emit what the country publishes, as ops ([`Op`]), and
//! once, its state there ([`Bind`]): the emit, which folded the country itself so far, binds the
//! two where they fold alike from there ([`super::emit`]), maps what the lens could not know
//! (the hours of what began before the segment, which the lens holds as its first read's hour),
//! and steps itself each slot state the two hold apart, as only it knows that state's countdown.
//! At the segment's end, and where a country's slot comes to hold another tag, the lens hands
//! the country back whole ([`Taken`]).

use super::{
    Country, CountryRows, FreshRead, Indexes, Scan,
    emit::{self, Closed, KeptReads},
    facts::{Fresh, Index},
};
use crate::{ResearchEntry, ResearchProgress, ResearchSlotState};
use replay_data::cadence::STAGGER_PERIOD;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

/// A slot read's step of the slot's state, as a lens's country took it: whether the read
/// continues the state in all but its countdown (`gate`), and whether it continued it (`kept`);
/// the read's remaining days, whether it names a technology, and the assignment it names, for a
/// state the emit starts there itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(super) struct Chain {
    pub(super) slot: u32,
    pub(super) gate: bool,
    pub(super) kept: bool,
    pub(super) remaining: Option<u32>,
    pub(super) held: bool,
    pub(super) entry: Option<i32>,
}

/// A key of a country's series.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, replay_spool::Spooled)]
pub(super) enum Key {
    Coverage,
    History,
    Available(u32),
    State(u32),
    Progress(u32),
}

/// A slot's items a later push closes the item before in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, replay_spool::Spooled)]
pub(super) enum Items {
    Availability,
    Entries,
    Progress,
}

const ITEMS: [Items; 3] = [Items::Availability, Items::Entries, Items::Progress];

/// What an event's text holds of the lens's first read's hour in place of what began before
/// the segment, by where it stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(super) enum Prior {
    /// A closed coverage span's first hour.
    Coverage,
    /// Slot `.0`'s closed availability span's first hour.
    Available(u32),
    /// Slot `.0`'s progress: its first hour and its assignment.
    Progress(u32),
    /// Slot `.0`'s state's assignment.
    Assignment(u32),
    /// A closed assignment's continuity.
    Continuity,
}

/// An event a bound country published, as its text is written: the emit writes it.
#[derive(Clone, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(super) enum Event {
    State(Box<ResearchSlotState>),
    Read(Option<ResearchProgress>),
    Closed(Box<Closed>),
}

/// What a bound country published.
#[derive(Clone, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(super) enum Op {
    /// A slot read's step of its state; before the hour's other ops.
    Chain(Chain),
    /// Slot `.0` first published since the country bound.
    Slot(u32),
    /// Slot `.0`'s items of the kind first held since the country bound.
    Filled(u32, Items),
    /// The country's read was observed at the hour.
    Read,
    Value {
        key: Key,
        value: Option<i128>,
        starts: bool,
    },
    /// An event of `key`.
    Event {
        key: Key,
        event: Event,
        prior: Option<Prior>,
    },
    /// A closed assignment that began before the segment.
    Entry {
        slot: u32,
        entry: Box<ResearchEntry>,
    },
}

/// A bound country's ops of an hour, in the order it published them.
#[derive(Debug, replay_spool::Spooled)]
pub(super) struct Ops {
    pub(super) country: u32,
    pub(super) ops: Vec<Op>,
}

/// A country the lens binds at the end of an hour: what the emit checks its own against.
#[derive(replay_spool::Spooled)]
pub(super) struct Bind {
    /// What its ops and its return name it by.
    pub(super) number: u32,
    pub(super) tag: String,
    pub(super) canonical: u32,
    /// The lens's first hour, and the hour of the read before it it began the country from.
    pub(super) first: i32,
    pub(super) virgin: Option<i32>,
    pub(super) country: Box<Country>,
    /// Whether its slot is observed again at the next hour.
    pub(super) recheck: bool,
    /// What each of its keys holds, as the emit's carry would: a key absent holds nothing.
    pub(super) held: Vec<(Key, i128)>,
}

/// A bound country the lens hands back at the end of an hour.
#[derive(replay_spool::Spooled)]
pub(super) struct Taken {
    pub(super) number: u32,
    pub(super) country: Box<Country>,
    pub(super) recheck: bool,
}

/// What a lens hands the emit with an hour: its bound countries' ops, then those it hands back,
/// then those it binds.
#[derive(Default, replay_spool::Spooled)]
pub(super) struct Part {
    pub(super) ops: Vec<Ops>,
    pub(super) returned: Vec<Taken>,
    pub(super) bound: Vec<Bind>,
}

impl Part {
    pub(super) fn is_empty(&self) -> bool {
        self.ops.is_empty() && self.returned.is_empty() && self.bound.is_empty()
    }
}

/// An event's text, as a country's publish hands it.
pub(super) enum Payload<'a> {
    State(&'a ResearchSlotState),
    Read(Option<&'a ResearchProgress>),
    Closed(&'a Closed),
}

/// Where a country's publish goes: the emit's series, or a lens's ops.
pub(super) trait Publish {
    /// Slot `slot` is published.
    fn slot(&mut self, slot: u32) -> io::Result<()>;
    fn value(&mut self, key: Key, at: i32, value: Option<i128>, starts: bool) -> io::Result<()>;
    fn event(&mut self, key: Key, at: i32, payload: Payload<'_>) -> io::Result<()>;
    /// The country's read was observed at the hour.
    fn read(&mut self) -> io::Result<()>;
}

/// What the lens keeps of a country it bound.
struct Bound {
    number: u32,
    canonical: u32,
    virgin: Option<i32>,
    /// Its slots it has published since it bound, and the items of them it held none of.
    slots: BTreeSet<u32>,
    empty: BTreeSet<(u32, Items)>,
}

/// What a country's keys hold, as the emit's carry would: a key absent holds nothing.
#[derive(Default)]
struct Held {
    coverage: Option<i128>,
    history: Option<i128>,
    /// Each slot's AVAILABLE, STATE and PROGRESS, by slot ascending.
    slots: Vec<(u32, [Option<i128>; 3])>,
}

impl Held {
    fn at(&mut self, key: Key) -> &mut Option<i128> {
        let (slot, place) = match key {
            Key::Coverage => return &mut self.coverage,
            Key::History => return &mut self.history,
            Key::Available(slot) => (slot, 0),
            Key::State(slot) => (slot, 1),
            Key::Progress(slot) => (slot, 2),
        };
        let index = match self.slots.binary_search_by_key(&slot, |(held, _)| *held) {
            Ok(index) => index,
            Err(index) => {
                self.slots.insert(index, (slot, [None; 3]));
                index
            }
        };
        &mut self.slots[index].1[place]
    }

    /// Every key holding a value, with it.
    fn held(&self) -> Vec<(Key, i128)> {
        let mut held = Vec::new();
        held.extend(self.coverage.map(|value| (Key::Coverage, value)));
        held.extend(self.history.map(|value| (Key::History, value)));
        for &(slot, values) in &self.slots {
            let keys = [Key::Available(slot), Key::State(slot), Key::Progress(slot)];
            for (key, value) in keys.into_iter().zip(values) {
                held.extend(value.map(|value| (key, value)));
            }
        }
        held
    }
}

/// A country's publish as the lens takes it: what each of its keys holds, as the emit's carry
/// would, and its ops while bound.
struct Recorder<'a> {
    held: &'a mut Held,
    bound: Option<(&'a mut Bound, &'a mut Vec<Op>)>,
}

fn is(virgin: Option<i32>, hour: i32) -> bool {
    virgin == Some(hour)
}

impl Publish for Recorder<'_> {
    fn slot(&mut self, slot: u32) -> io::Result<()> {
        if let Some((bound, ops)) = &mut self.bound
            && bound.slots.insert(slot)
        {
            ops.push(Op::Slot(slot));
            bound.empty.extend(ITEMS.map(|items| (slot, items)));
        }
        Ok(())
    }

    fn value(&mut self, key: Key, _: i32, value: Option<i128>, starts: bool) -> io::Result<()> {
        // As the carry takes it: an equal value going on without a break, or none after none,
        // changes nothing.
        let held = self.held.at(key);
        let same = match (*held, value) {
            (Some(held), Some(value)) => held == value && !starts,
            (None, None) => true,
            _ => false,
        };
        if same {
            return Ok(());
        }
        *held = value;
        if let Some((_, ops)) = &mut self.bound {
            ops.push(Op::Value { key, value, starts });
        }
        Ok(())
    }

    fn event(&mut self, key: Key, _: i32, payload: Payload<'_>) -> io::Result<()> {
        let Some((bound, ops)) = &mut self.bound else {
            return Ok(());
        };
        let virgin = bound.virgin;
        let slot = match key {
            Key::State(slot) | Key::Progress(slot) | Key::Available(slot) => slot,
            Key::Coverage | Key::History => 0,
        };
        let (event, prior) = match payload {
            Payload::State(state) => {
                let prior = state
                    .entry_first_observed_hour
                    .is_some_and(|hour| is(virgin, hour))
                    .then_some(Prior::Assignment(slot));
                (Event::State(Box::new(state.clone())), prior)
            }
            Payload::Read(read) => {
                let prior = read
                    .is_some_and(|progress| {
                        is(virgin, progress.first_hour)
                            || progress
                                .entry_first_observed_hour
                                .is_some_and(|hour| is(virgin, hour))
                    })
                    .then_some(Prior::Progress(slot));
                (Event::Read(read.cloned()), prior)
            }
            Payload::Closed(closed) => {
                let prior = match closed {
                    Closed::Coverage { span } => {
                        is(virgin, span.first_hour).then_some(Prior::Coverage)
                    }
                    Closed::Available { slot, span } => {
                        is(virgin, span.first_hour).then_some(Prior::Available(*slot))
                    }
                    Closed::Progress { slot, progress } => (is(virgin, progress.first_hour)
                        || progress
                            .entry_first_observed_hour
                            .is_some_and(|hour| is(virgin, hour)))
                    .then_some(Prior::Progress(*slot)),
                    Closed::Entry { slot, entry } => {
                        if is(virgin, entry.first_observed_hour) {
                            ops.push(Op::Entry {
                                slot: *slot,
                                entry: Box::new(entry.clone()),
                            });
                            return Ok(());
                        }
                        is(virgin, entry.continuity_start_hour).then_some(Prior::Continuity)
                    }
                };
                (Event::Closed(Box::new(closed.clone())), prior)
            }
        };
        ops.push(Op::Event { key, event, prior });
        Ok(())
    }

    fn read(&mut self) -> io::Result<()> {
        if let Some((_, ops)) = &mut self.bound {
            ops.push(Op::Read);
        }
        Ok(())
    }
}

/// How many tags `canonical`'s holders in `rows` name.
fn holders(rows: &CountryRows, canonical: u32) -> usize {
    rows.get(&canonical).map_or(0, Vec::len)
}

/// A country's read before a lens's first hour, which it begins the country from.
pub(super) struct Virgin {
    pub(super) tag: String,
    pub(super) identity: (u32, u32),
    pub(super) read: i32,
    pub(super) fresh: Fresh,
}

/// One segment's fold of every country, from its first hour.
pub(super) struct Lens {
    first: i32,
    scan: Scan,
    /// What each country's keys hold, as the emit's carry would.
    held: HashMap<String, Held>,
    virgins: HashMap<String, i32>,
    bound: HashMap<String, Bound>,
    numbers: u32,
    part: Part,
}

impl Lens {
    /// A lens from hour `first` on, each of `virgins` begun from its read before.
    pub(super) fn start(first: i32, virgins: Vec<Virgin>) -> Self {
        let mut scan = Scan::default();
        // The hour before is the one its countries' states were begun as of.
        scan.last_frame = first.checked_sub(1);
        let mut begun = HashMap::with_capacity(virgins.len());
        for virgin in virgins {
            let read = FreshRead::kept(&virgin.fresh);
            scan.begin(&virgin.tag, virgin.identity, virgin.read, &read);
            begun.insert(virgin.tag, virgin.read);
        }
        Self {
            first,
            scan,
            held: HashMap::new(),
            virgins: begun,
            bound: HashMap::new(),
            numbers: 0,
            part: Part::default(),
        }
    }

    /// Folds hour `at` from its index step and fresh reads, publishing what every country it
    /// observes publishes; then hands back the bound countries whose slot another tag holds,
    /// and binds those whose read a day into the segment it observed.
    pub(super) fn hour(
        &mut self,
        at: i32,
        index: &Index,
        indexes: Indexes<'_>,
        fresh: &BTreeMap<u32, Fresh>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("research.lens", None);
        let reads = KeptReads {
            dates: indexes.dates,
            fresh,
        };
        // The emit refuses the branch where the country step fails; the lens fails with it.
        self.scan
            .observe_refreshed(at, emit::refreshed(index), indexes, &reads)
            .map_err(|error| io::Error::other(format!("Research lens: {error}")))?;
        let rows = indexes.rows;
        let observed = std::mem::take(&mut self.scan.observed);
        for tag in &observed {
            let Some(country) = self.scan.countries.get_mut(tag) else {
                continue;
            };
            if !self.held.contains_key(tag) {
                self.held.insert(tag.clone(), Held::default());
            }
            let held = self.held.get_mut(tag).expect("held research keys");
            let mut ops = Vec::new();
            let mut recorder = Recorder {
                held,
                bound: None,
            };
            if let Some(bound) = self.bound.get_mut(tag) {
                if let Some(chains) = &mut country.chains {
                    ops.extend(chains.drain(..).map(Op::Chain));
                }
                recorder.bound = Some((bound, &mut ops));
            }
            let read_only = super::emit::publish(country, at, &mut recorder)?;
            if read_only && let Some((_, canonical)) = country.identity {
                self.scan.recheck.insert(canonical);
            }
            if let Some(bound) = self.bound.get_mut(tag) {
                let slots = &country.history.slots;
                bound.empty.retain(|&(index, items)| {
                    let filled = slots.iter().find(|slot| slot.index == index).is_some_and(
                        |slot| match items {
                            Items::Availability => !slot.availability.is_empty(),
                            Items::Entries => !slot.entries.is_empty(),
                            Items::Progress => !slot.progress.is_empty(),
                        },
                    );
                    if filled {
                        ops.push(Op::Filled(index, items));
                    }
                    !filled
                });
                if !ops.is_empty() {
                    self.part.ops.push(Ops {
                        country: bound.number,
                        ops,
                    });
                }
            }
        }
        // A slot two tags hold rechecks for both: the emit folds them together.
        if matches!(index, Index::Changed(changed) if changed.countries_changed) {
            let shared: Vec<String> = self
                .bound
                .iter()
                .filter(|(_, bound)| holders(rows, bound.canonical) > 1)
                .map(|(tag, _)| tag.clone())
                .collect();
            for tag in shared {
                self.give(&tag)?;
            }
        }
        let binding: Vec<(&String, u32)> = observed
            .iter()
            .filter(|_| at >= self.first.saturating_add(STAGGER_PERIOD))
            .filter(|tag| !self.bound.contains_key(*tag))
            .filter_map(|tag| {
                let country = self.scan.countries.get(tag)?;
                let (_, canonical) = country.identity?;
                (country.last_read == Some(at) && holders(rows, canonical) == 1)
                    .then_some((tag, canonical))
            })
            .collect();
        for (tag, canonical) in binding {
            self.bind(tag, canonical);
        }
        self.scan.observed = observed;
        Ok(())
    }

    fn bind(&mut self, tag: &str, canonical: u32) {
        let Some(country) = self.scan.countries.get_mut(tag) else {
            return;
        };
        country.chains = Some(Vec::new());
        let mut bound = Bound {
            number: self.numbers,
            canonical,
            virgin: self.virgins.get(tag).copied(),
            slots: BTreeSet::new(),
            empty: BTreeSet::new(),
        };
        self.numbers += 1;
        for slot in &country.history.slots {
            bound.slots.insert(slot.index);
            for items in ITEMS {
                let empty = match items {
                    Items::Availability => slot.availability.is_empty(),
                    Items::Entries => slot.entries.is_empty(),
                    Items::Progress => slot.progress.is_empty(),
                };
                if empty {
                    bound.empty.insert((slot.index, items));
                }
            }
        }
        let mut state = country.clone();
        state.chains = None;
        let held = self.held.get(tag).map(Held::held).unwrap_or_default();
        self.part.bound.push(Bind {
            number: bound.number,
            tag: tag.to_owned(),
            canonical,
            first: self.first,
            virgin: bound.virgin,
            country: Box::new(state),
            recheck: self.scan.recheck.contains(&canonical),
            held,
        });
        self.bound.insert(tag.to_owned(), bound);
    }

    /// Hands `tag` back to the emit as it holds it now.
    fn give(&mut self, tag: &str) -> io::Result<()> {
        let Some(bound) = self.bound.remove(tag) else {
            return Ok(());
        };
        let country = self
            .scan
            .countries
            .get_mut(tag)
            .ok_or_else(|| io::Error::other("A research lens lost a country it bound"))?;
        country.chains = None;
        self.part.returned.push(Taken {
            number: bound.number,
            country: Box::new(country.clone()),
            recheck: self.scan.recheck.contains(&bound.canonical),
        });
        Ok(())
    }

    /// What it hands the emit with the hour it folded last.
    pub(super) fn part(&mut self) -> Option<Box<Part>> {
        let part = std::mem::take(&mut self.part);
        (!part.is_empty()).then(|| Box::new(part))
    }

    /// Hands every bound country back, as the hours it folded leave them.
    pub(super) fn hand_over(&mut self) -> io::Result<Option<Box<Part>>> {
        let tags: Vec<String> = self.bound.keys().cloned().collect();
        for tag in tags {
            self.give(&tag)?;
        }
        Ok(self.part())
    }
}
