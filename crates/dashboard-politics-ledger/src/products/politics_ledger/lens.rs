//! Politics accounting folded per segment on the walk's workers ([`Lens`]). From the segment's
//! first hour, which hands every table whole, the lens runs the ledger's accounting for every
//! country ([`super::ledger`]), taking the rows that hour dates earlier as recorded before. Once
//! a country's daily rows and costs are of a later day, it binds each country no other country's
//! joins reach ([`Bind`]): from then on it hands the emit what that country writes, as ops
//! ([`Op`]), with the rollups and warnings it
//! adds apart from those the emit holds. The emit, which folded the country itself so far, binds
//! where its state is the lens's and replays the ops ([`super::emit`]). The lens hands a country
//! back whole ([`Taken`]) before an hour whose census or links change what it holds, before a
//! refused hour, and at the segment's end.

use super::emit::{COVERED, PARTS, PART_FIELDS};
use super::facts::{self, CensusRow, Hour};
use super::ledger::{Country, FastSet, Field, Ledger, Out, Raw, Tags};
use super::packed;
use super::tally;
use super::*;
use dashboard_base::products::politics_content::PoliticsContent;
use dashboard_series::hash::FastMap;

/// Hours after its first a lens binds the countries it has not yet, whatever day their rows are
/// of: by then every capture read twice.
const BIND_BY: i32 = 72;

/// A field of an op's event: a record packed at `.0..(.0 + .1)` of the part's payload
/// ([`packed`]), or an item as the lens numbers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(super) enum Piece {
    Record(u32, u32),
    Item(u32),
}

/// What a bound country wrote, in the order it wrote it ([`Out`]).
#[derive(Debug, replay_spool::Spooled)]
pub(super) enum Op {
    Event {
        measure: u64,
        hours: (i32, Option<i32>),
        kind: u64,
        fields: Vec<Piece>,
    },
    Value {
        measure: u64,
        dimension: Option<u64>,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    },
    /// Item `ordinal` of `key` is named, first since the country bound; `shown` is its packed
    /// record where this payment is its first the lens read.
    Item {
        ordinal: u32,
        key: [u8; 32],
        shown: Option<(u32, u32)>,
        hours: (i32, Option<i32>),
        kind: u64,
    },
    /// A payment in category dimension `.0`.
    Category(u64),
    First {
        dimension: u64,
        hour: i32,
    },
    /// The categories of currency `currency` the country paid nothing in today.
    Unpaid {
        currency: u8,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    },
    /// The periods the settled frontier `settled` closed of what the lens rolled up since the
    /// country bound, and the days through its binding day it took out apart.
    Settled {
        settled: i32,
        closed: Vec<(usize, i32, tally::Totals)>,
        split: tally::Split,
    },
}

/// A country the lens binds at the end of an hour: what the emit checks its own against.
#[derive(replay_spool::Spooled)]
pub(super) struct Bind {
    /// What its ops and its return name it by.
    pub(super) number: u32,
    pub(super) tag: Box<str>,
    /// The settled frontier, and where it was last rolled.
    pub(super) settled: (Option<i32>, Option<i32>),
    /// Its accounting, its rollups and warnings begun again from the binding.
    pub(super) country: Box<Country>,
    /// Its raw countries' rows, and their capture identities.
    pub(super) raws: Vec<(u32, Raw)>,
    /// Of them, those whose costs are matched to their records again at the next cost read.
    pub(super) stale: Vec<u32>,
    pub(super) identities: Vec<(u32, u64)>,
    pub(super) witnesses: Vec<(u32, Option<PoliticsCaptureCoverageRow>)>,
    /// What each of its keys holds, as the emit's carry would: its coverage, its fixed parts and
    /// the categories the lens knows it by, and what a category it paid nothing in today holds.
    pub(super) held: Vec<((u64, Option<u64>), Option<i128>)>,
    pub(super) unpaid: [Option<i128>; 2],
    pub(super) volatile: bool,
    pub(super) tomorrow: Vec<i32>,
    pub(super) command: Vec<(u32, Option<i64>)>,
}

/// A bound country the lens hands back: its accounting through the hour before.
#[derive(replay_spool::Spooled)]
pub(super) struct Taken {
    pub(super) number: u32,
    /// Its accounting: its rollups and warnings those added since it bound.
    pub(super) country: Box<Country>,
    pub(super) raws: Vec<(u32, Raw)>,
    pub(super) stale: Vec<u32>,
    pub(super) volatile: bool,
    pub(super) tomorrow: Vec<i32>,
}

/// What a lens hands the emit with an hour: the countries it stopped folding before the hour,
/// the categories and record shapes it named, its bound countries' ops, then the countries it
/// binds.
#[derive(Default, replay_spool::Spooled)]
pub(super) struct Part {
    /// The lens began at the hour: it names its countries, categories and shapes anew.
    pub(super) begins: bool,
    pub(super) returned: Vec<Taken>,
    /// Category dimensions as the lens numbers them, with their currency and name.
    pub(super) named: Vec<(u64, u8, Box<str>)>,
    /// The record shapes it named, numbered on from those named before ([`packed`]).
    pub(super) shapes: Vec<Vec<u8>>,
    pub(super) ops: Vec<(u32, Vec<Op>)>,
    pub(super) payload: Vec<u8>,
    pub(super) bound: Vec<Bind>,
}

impl Part {
    fn is_empty(&self) -> bool {
        !self.begins
            && self.returned.is_empty()
            && self.named.is_empty()
            && self.shapes.is_empty()
            && self.ops.is_empty()
            && self.bound.is_empty()
    }
}

fn narrow(value: usize) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| io::Error::other("A politics lens part is too large"))
}

/// The accounting's writes, as the lens takes them: every country's values, as the emit's
/// carry would hold them, and a bound country's writes as ops.
#[derive(Default)]
struct Recorder {
    /// The countries bound, with the items each named since, and the same countries by bit.
    bound: FastMap<u32, FastSet<u32>>,
    kept: Tags,
    /// Countries may still bind: every country's values are held, not only a bound one's.
    binding: bool,
    held: Vec<FastMap<(u64, Option<u64>), i128>>,
    unpaid: Vec<[Option<i128>; 2]>,
    sequence: u64,
    ops: BTreeMap<u32, Vec<Op>>,
    payload: Vec<u8>,
    named: Vec<(u64, u8, Box<str>)>,
    shapes: packed::Shapes,
    /// The last record, and where it lies in the payload: a row recorded twice is handed once.
    last: Option<(u32, u32)>,
    record: Vec<u8>,
}

/// Whether `value` from `hour` on changes a key holding `held`, as the carry takes it.
fn changes(held: Option<i128>, value: Option<i128>, starts: bool) -> bool {
    !(held == value && (!starts || value.is_none()))
}

impl Recorder {
    fn record(&mut self, bytes: &[u8]) -> io::Result<(u32, u32)> {
        if let Some(last) = self.last
            && self.record == bytes
        {
            return Ok(last);
        }
        let start = self.payload.len();
        self.shapes.pack(bytes, &mut self.payload)?;
        let at = narrow(start)?;
        let len = narrow(self.payload.len() - start)?;
        self.record.clear();
        self.record.extend_from_slice(bytes);
        self.last = Some((at, len));
        Ok((at, len))
    }

    fn bind(&mut self, tag: u32) {
        self.bound.insert(tag, FastSet::default());
        self.kept.insert(tag);
    }

    fn unbind(&mut self, tag: u32) -> bool {
        self.kept.remove(tag);
        self.bound.remove(&tag).is_some()
    }

    fn push(&mut self, tag: u32, op: Op) {
        self.ops.entry(tag).or_default().push(op);
    }
}

impl Out for Recorder {
    fn keeps(&self, tag: u32) -> bool {
        self.kept.contains(tag)
    }

    fn tagged(&mut self, tag: u32, _: &str, _: i32) -> io::Result<()> {
        let at = tag as usize;
        if self.held.len() <= at {
            self.held.resize_with(at + 1, FastMap::default);
            self.unpaid.resize(at + 1, [None; 2]);
        }
        Ok(())
    }

    fn event(
        &mut self,
        tag: u32,
        measure: u64,
        hours: (i32, Option<i32>),
        kind: u64,
        fields: &[Field<'_>],
    ) -> io::Result<u64> {
        self.sequence += 1;
        if self.keeps(tag) {
            let mut pieces = Vec::with_capacity(fields.len());
            for field in fields {
                pieces.push(match *field {
                    Field::Record(bytes) => {
                        let (at, len) = self.record(bytes)?;
                        Piece::Record(at, len)
                    }
                    Field::Item(ordinal) => Piece::Item(ordinal),
                    Field::Plain(_) => {
                        return Err(io::Error::other("A politics lens hands no plain field"));
                    }
                });
            }
            self.push(
                tag,
                Op::Event {
                    measure,
                    hours,
                    kind,
                    fields: pieces,
                },
            );
        }
        Ok(self.sequence)
    }

    fn value(
        &mut self,
        tag: u32,
        measure: u64,
        dimension: Option<u64>,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        let kept = self.keeps(tag);
        // No later bind or op reads what a country neither bound nor binding holds.
        if !kept && !self.binding {
            return Ok(());
        }
        let held = &mut self.held[tag as usize];
        let key = (measure, dimension);
        let old = match value {
            Some(value) => held.insert(key, value),
            None => held.remove(&key),
        };
        if changes(old, value, starts) && kept {
            self.push(
                tag,
                Op::Value {
                    measure,
                    dimension,
                    hour,
                    value,
                    starts,
                },
            );
        }
        Ok(())
    }

    fn named(&mut self, dimension: u64, currency: usize, category: &str) -> io::Result<()> {
        self.named.push((dimension, currency as u8, category.into()));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn item(
        &mut self,
        tag: u32,
        ordinal: u32,
        key: &[u8; 32],
        new: bool,
        shown: Option<&[u8]>,
        hours: (i32, Option<i32>),
        kind: u64,
    ) -> io::Result<()> {
        let Some(named) = self.bound.get_mut(&tag) else {
            return Ok(());
        };
        if !named.insert(ordinal) && !new {
            return Ok(());
        }
        let shown = match shown.filter(|_| new) {
            Some(bytes) => Some(self.record(bytes)?),
            None => None,
        };
        self.push(
            tag,
            Op::Item {
                ordinal,
                key: *key,
                shown,
                hours,
                kind,
            },
        );
        Ok(())
    }

    fn category(&mut self, tag: u32, dimension: u64) -> io::Result<()> {
        if self.keeps(tag) {
            self.push(tag, Op::Category(dimension));
        }
        Ok(())
    }

    fn first(&mut self, tag: u32, dimension: u64, hour: i32) -> io::Result<()> {
        if self.keeps(tag) {
            self.push(tag, Op::First { dimension, hour });
        }
        Ok(())
    }

    fn unpaid(
        &mut self,
        tag: u32,
        currency: usize,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        let kept = self.keeps(tag);
        if !kept && !self.binding {
            return Ok(());
        }
        let held = &mut self.unpaid[tag as usize][currency];
        let changed = changes(*held, value, starts);
        *held = value;
        if changed && kept {
            self.push(
                tag,
                Op::Unpaid {
                    currency: currency as u8,
                    hour,
                    value,
                    starts,
                },
            );
        }
        Ok(())
    }

    fn settled(
        &mut self,
        tag: u32,
        _: i32,
        settled: i32,
        closed: Vec<(usize, i32, tally::Totals)>,
        split: tally::Split,
    ) -> io::Result<()> {
        if self.keeps(tag) && !(closed.is_empty() && split.is_empty()) {
            self.push(
                tag,
                Op::Settled {
                    settled,
                    closed,
                    split,
                },
            );
        }
        Ok(())
    }
}

/// One segment's fold of every country, from its first hour.
pub(super) struct Lens {
    ledger: Ledger,
    recorder: Recorder,
    first: i32,
    /// The countries it bound or found it could not, each once.
    attempted: FastSet<u32>,
    begins: bool,
    returned: Vec<Taken>,
    bound: Vec<Bind>,
}

impl Lens {
    /// A lens from hour `first`, which hands every table whole.
    pub(super) fn start(first: i32, content: Option<Arc<PoliticsContent>>) -> Self {
        let mut ledger = Ledger::new(content);
        ledger.begun = Some(first);
        Self {
            ledger,
            recorder: Recorder::default(),
            first,
            attempted: FastSet::default(),
            begins: true,
            returned: Vec::new(),
            bound: Vec::new(),
        }
    }

    /// Folds `hour`: first hands back the bound countries whose holdings it changes, then
    /// accounts it as the emit would, and binds the countries whose rows it now holds alike.
    pub(super) fn hour(&mut self, hour: &Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("politics_ledger.lens", None);
        let at = hour.hour;
        if !self.recorder.bound.is_empty() {
            self.give_changed(hour)?;
        }
        self.recorder.binding = at <= self.first.saturating_add(BIND_BY);
        let mut joined = self.ledger.open(hour);
        if joined.previous.is_some() && !joined.adjacent {
            return Err(io::Error::other("A politics lens folds an hour after a gap"));
        }
        self.ledger.join(&mut self.recorder, hour, &mut joined)?;
        self.ledger.rows(&mut self.recorder, hour, &mut joined)?;
        for tag in std::mem::take(&mut joined.dirty) {
            self.ledger.account(&mut self.recorder, tag, at, &joined)?;
        }
        self.ledger.settle(&mut self.recorder, at)?;
        if self.attempted.len() < self.ledger.countries.len()
            && at <= self.first.saturating_add(BIND_BY)
        {
            self.bind(at, at == self.first.saturating_add(BIND_BY));
        }
        Ok(())
    }

    /// What it hands the emit with the hour it folded last.
    pub(super) fn part(&mut self) -> Option<Box<Part>> {
        let part = Part {
            begins: std::mem::take(&mut self.begins),
            returned: std::mem::take(&mut self.returned),
            named: std::mem::take(&mut self.recorder.named),
            shapes: self.recorder.shapes.take(),
            ops: std::mem::take(&mut self.recorder.ops).into_iter().collect(),
            payload: std::mem::take(&mut self.recorder.payload),
            bound: std::mem::take(&mut self.bound),
        };
        self.recorder.last = None;
        (!part.is_empty()).then(|| Box::new(part))
    }

    /// Hands every bound country back, as the hours it folded through `last` leave them.
    pub(super) fn hand_over(&mut self, last: i32) -> Option<Box<Part>> {
        let mut bound: Vec<u32> = self.recorder.bound.keys().copied().collect();
        bound.sort_unstable();
        for tag in bound {
            self.give(tag, last);
        }
        self.part()
    }

    /// Hands back, through the hour before `hour`, each bound country whose slots, refused
    /// slots or raw countries `hour`'s census or links change.
    fn give_changed(&mut self, hour: &Hour) -> io::Result<()> {
        let ledger = &self.ledger;
        let census_complete = hour.complete & facts::bit(TableId::Countries) != 0;
        let census_changed = hour.census.is_some() || census_complete != ledger.census_complete;
        if !census_changed && hour.links.is_none() {
            return Ok(());
        }
        let mut slots: FastMap<u32, Vec<u32>> = FastMap::default();
        let mut refused = Vec::new();
        if census_changed && census_complete {
            let mut by_slot: BTreeMap<u32, &CensusRow> = BTreeMap::new();
            match hour.census.as_deref() {
                Some(census) => by_slot.extend(census.iter().map(|row| (row.slot, row))),
                None => by_slot.extend(ledger.census.values().map(|row| (row.slot, row))),
            }
            for row in by_slot.values() {
                match rows::valid_tag(row.id, row.slot, &row.tag) {
                    Ok(spelling) => {
                        if let Some(&tag) = ledger.numbers.get(spelling) {
                            slots.entry(tag).or_default().push(row.slot);
                        }
                    }
                    Err(_) => refused.push(row.slot),
                }
            }
        }
        // Each slot's raw countries, as the links' last word on each raw country has them.
        let links: Option<FastMap<u32, Vec<u32>>> = hour.links.as_deref().map(|links| {
            let raws: FastMap<u32, u32> = links.iter().copied().collect();
            let mut by_slot: FastMap<u32, Vec<u32>> = FastMap::default();
            for (raw, slot) in raws {
                by_slot.entry(slot).or_default().push(raw);
            }
            by_slot
        });
        let mut affected = Vec::new();
        for &tag in self.recorder.bound.keys() {
            let country = &ledger.countries[tag as usize];
            let held = if census_changed {
                slots.remove(&tag).unwrap_or_default()
            } else {
                country.slots.clone()
            };
            let mut same = held == country.slots
                && !refused
                    .iter()
                    .any(|slot| ledger.known.get(slot) == Some(&tag));
            if same && let Some(links) = &links {
                let mut raws: Vec<u32> = held
                    .iter()
                    .filter_map(|slot| links.get(slot))
                    .flatten()
                    .copied()
                    .collect();
                raws.sort_unstable();
                raws.dedup();
                same = raws == ledger.raws_of(tag);
            }
            if !same {
                affected.push(tag);
            }
        }
        affected.sort_unstable();
        let through = hour.hour.saturating_sub(1);
        for tag in affected {
            self.give(tag, through);
        }
        Ok(())
    }

    /// Hands bound country `tag` back as the hours through `through` leave it; it folds it on
    /// apart.
    fn give(&mut self, tag: u32, through: i32) {
        if !self.recorder.unbind(tag) {
            return;
        }
        let ledger = &mut self.ledger;
        let country = &mut ledger.countries[tag as usize];
        country.split = None;
        country.materialize(through);
        let held = ledger.raws_of(tag);
        let stale = stale(ledger, &held);
        let raws = held
            .into_iter()
            .map(|raw| (raw, ledger.raws.get(&raw).cloned().unwrap_or_default()))
            .collect();
        let mut country = Box::new(ledger.countries[tag as usize].clone());
        country.item_index = FastMap::default();
        self.returned.push(Taken {
            number: tag,
            country,
            raws,
            stale,
            volatile: ledger.volatile.contains(tag),
            tomorrow: tomorrow(ledger, tag),
        });
    }

    /// Binds, at the end of hour `at`, every active country not yet tried whose rows are all of a
    /// day after the lens's first hour's (any, when `last`): one whose raw countries and capture
    /// identities no other country's joins reach, and whose rows wait for no tag.
    fn bind(&mut self, at: i32, last: bool) {
        let ledger = &self.ledger;
        let first_day = self.first.div_euclid(24) * 24;
        // Rows the lens took as recorded before it are of its first day or earlier: a country's
        // next day's rows replace them.
        let renewed = |raws: &[u32]| {
            raws.iter().all(|raw| {
                ledger.raws.get(raw).is_some_and(|held| {
                    held.daily.as_ref().is_some_and(|row| row.day_hour > first_day)
                        && held.costs.values().all(|cost| cost.day_hour > first_day)
                })
            })
        };
        // The raw countries each tag's capture identities name.
        let mut named: FastMap<u32, Vec<u32>> = FastMap::default();
        for (&(raw, _), tags) in &ledger.identities {
            for &(_, tag) in tags {
                named.entry(tag).or_default().push(raw);
            }
        }
        let refused: FastSet<u32> = ledger
            .refused
            .keys()
            .filter_map(|slot| ledger.known.get(slot))
            .copied()
            .collect();
        let mut binding = Vec::new();
        let mut tried = Vec::new();
        for tag in 0..ledger.countries.len() as u32 {
            if self.attempted.contains(&tag) {
                continue;
            }
            let country = &ledger.countries[tag as usize];
            let raws = ledger.raws_of(tag);
            if !last && !renewed(&raws) {
                continue;
            }
            tried.push(tag);
            if !country.active || country.slots.is_empty() || refused.contains(&tag) {
                continue;
            }
            if raws.is_empty()
                || named
                    .get(&tag)
                    .is_some_and(|named| named.iter().any(|raw| raws.binary_search(raw).is_err()))
            {
                continue;
            }
            let mut days = 0;
            let clean = raws.iter().all(|&raw| {
                let identities = ledger
                    .identities
                    .range((raw, 0)..=(raw, u64::MAX))
                    .all(|(_, tags)| tags.iter().all(|&(_, held)| held == tag));
                let held = ledger.raws.get(&raw);
                days += held.map_or(0, |held| held.days.len());
                identities
                    && held.is_none_or(|held| {
                        held.waiting.is_empty() && held.days.values().all(|day| day.tag == tag)
                    })
            });
            if clean && ledger.day_counts.get(tag as usize).copied() == Some(days as u32) {
                binding.push((tag, raws));
            }
        }
        self.attempted.extend(tried);
        for (tag, raws) in binding {
            self.bind_tag(tag, raws, at);
        }
    }

    /// Binds `tag`, holding `raws`, at the end of hour `at`: what it rolls up and warns of from
    /// here is handed apart from what came before.
    fn bind_tag(&mut self, tag: u32, raws: Vec<u32>, at: i32) {
        let ledger = &mut self.ledger;
        {
            let country = &mut ledger.countries[tag as usize];
            country.materialize(at);
            country.warnings.clear();
            country.warnings_changed = false;
            country.rollups = tally::Rollups::default();
            country.split = Some(at.div_euclid(24) * 24);
        }
        ledger.reschedule(tag);
        let ledger = &self.ledger;
        let country = &ledger.countries[tag as usize];
        let held = &self.recorder.held[tag as usize];
        let value = |key: (u64, Option<u64>)| (key, held.get(&key).copied());
        let mut values: Vec<((u64, Option<u64>), Option<i128>)> =
            COVERED.iter().map(|&measure| value((measure, None))).collect();
        values.extend((0..PART_FIELDS.len() as u64).map(|dimension| value((PARTS, Some(dimension)))));
        values.extend(
            country
                .categories
                .keys()
                .map(|&dimension| value((PARTS, Some(dimension)))),
        );
        let mut snapshot = Box::new(country.clone());
        snapshot.item_index = FastMap::default();
        let bind = Bind {
            number: tag,
            tag: ledger.tags[tag as usize].clone(),
            settled: (ledger.settled, ledger.rolled),
            country: snapshot,
            raws: raws
                .iter()
                .map(|&raw| (raw, ledger.raws.get(&raw).cloned().unwrap_or_default()))
                .collect(),
            stale: stale(ledger, &raws),
            identities: raws
                .iter()
                .flat_map(|&raw| {
                    ledger
                        .identities
                        .range((raw, 0)..=(raw, u64::MAX))
                        .map(|(&key, _)| key)
                })
                .collect(),
            witnesses: raws
                .iter()
                .map(|&raw| (raw, ledger.witnesses.get(&raw).cloned()))
                .collect(),
            held: values,
            unpaid: self.recorder.unpaid[tag as usize],
            volatile: ledger.volatile.contains(tag),
            tomorrow: tomorrow(ledger, tag),
            command: country
                .slots
                .iter()
                .map(|&slot| (slot, ledger.command.get(&slot).copied()))
                .collect(),
        };
        self.recorder.bind(tag);
        self.bound.push(bind);
    }
}

/// Those of `raws` whose costs are matched to their records again at the next cost read.
pub(super) fn stale(ledger: &Ledger, raws: &[u32]) -> Vec<u32> {
    raws.iter()
        .copied()
        .filter(|raw| ledger.stale_costs.contains(raw))
        .collect()
}

/// The midnights at which `tag`'s parts are accounted again.
pub(super) fn tomorrow(ledger: &Ledger, tag: u32) -> Vec<i32> {
    ledger
        .tomorrow
        .iter()
        .filter(|(_, tags)| tags.contains(&tag))
        .map(|(&at, _)| at)
        .collect()
}
