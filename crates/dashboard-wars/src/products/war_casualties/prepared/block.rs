//! War casualties prepared block by block, on their own. A pre-pass worker
//! reads a block alone with the casualty scan's tables, only at the hours a
//! field the scan reads changes ([`FAMILY`]'s fields), and runs the casualty
//! observer on it from the block's first hour with no state of earlier
//! blocks. Casualty counters move every hour of a war, so wartime hours are
//! observed there, on the workers; each span of quiet hours advances in
//! constant time. After the first hour the extract records, for each hour it
//! observes, what the carried observer needs to follow it: whether the
//! relations were read again, the lifetimes first held, the naming snapshot
//! when it named again, and each changed tag (an endpoint of the relations
//! read) with its country's counters and unknown losses as the extract's scan
//! counts them and as they move for a country seen before. A block whose
//! census, read or continuity changes after its first hour, or whose scan
//! has a gap to recover there, is observed hour by hour.
//!
//! The family's own thread folds the blocks in order. It observes each
//! block's first hour on the carried observer, as hourly preparation does.
//! After that hour both scans hold the same census, relations and naming
//! snapshot; they differ in what earlier blocks left: country indices, the
//! counters of ended lifetimes in each country's totals, unknown-loss totals,
//! gap-recovery flags, the read sequence and generations, tracks, keys and
//! history. Each of those either moves the same way in both (a country's
//! totals keep their difference, sequences their offset, recovery flags their
//! value) or is the carried observer's own, which the fold continues: for
//! each hour it moves the carried read frontiers, holds the new lifetimes and
//! samples each changed tag with the extract's values carried over (an unknown
//! total counted from the carried total as for a country seen before, or as
//! the extract counts it for a country the carried scan never saw), then
//! publishes the tags' values and observations and labels new names with the
//! code hourly observation runs. After the block it takes the relations'
//! counters, read sequences and generations, the countries' totals and the
//! pair history's runs, numbered and marked incomplete as the carried scan
//! has them. No hour but the first is observed on the carried observer. When
//! the carried scan does not continue the first hour as an hourly supplying
//! read with nothing pending, holds a lifetime the extract first held later,
//! or its pair history disagrees there, the rest of the block is observed
//! hour by hour.
//!
//! Each block's pair window is rendered at its end and written with the
//! family's next commit, before its flush, where hourly preparation writes it
//! at the block's end into the same batch.

use super::super::{Country, PairKey, Unknown};
use super::{Prepared, pairs_section, prepared_content};
use dashboard_base::{
    Source,
    prepared::{
        frame_changes::{FrameChangeTracker, FrameChanges},
        prepass::{Extract, Family, Folded, Held, HeldBytes},
    },
    products::LoadContext,
};
use dashboard_text::metadata::slot_read;
use replay::{
    container::BlockIndex, container::QuietHours, container::Replay, container::TableFields,
};
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::{
    any::Any,
    io,
    ops::Range,
    sync::{Arc, atomic::AtomicBool},
};

/// The tables the casualty scan reads.
const TABLES: &[TableId] = &super::super::SCAN_TABLES;

pub static FAMILY: Family = Family {
    name: "casualties",
    extract: "casualties.extract",
    sequential: "casualties.sequential",
    fallback: "casualties.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields the casualty scan reads, so that a block is read only at the
/// hours they change.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // The campaign every hour validates, compared whole.
        TableFields::all(TableId::Campaign),
        // The census: country ids (the row key), slots and tags. Balances
        // change every hour and nothing here reads them.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // The census compares the links whole.
        TableFields::all(TableId::CountryLinks),
        // Relation keys, starts and the four counters the scan sums, and the
        // rest of the row: a relation table decoded again is a supplying
        // read the scan processes.
        TableFields::all(TableId::WarRelations),
        // The naming operands `same_name_rows` compares.
        TableFields::named(
            TableId::CountryAppearance,
            &["cosmetic_tag", "name_tag", "leader_ideology", "overlord"],
        )?,
        TableFields::named(TableId::Politics, &["ruling_party"])?,
        TableFields::named(TableId::PoliticalParties, &["token"])?,
        TableFields::named(TableId::Autonomy, &["state"])?,
    ])
}

/// The tables read by some of their fields only. A truncated view of one is
/// compared whole, and its unwatched rows can change at hours a read by
/// changes does not keep, so a block holding one is observed hour by hour.
const PARTIAL: [TableId; 5] = [
    TableId::Countries,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
];

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        prepared: None,
        first: None,
        hours: Vec::new(),
        tags: None,
        #[cfg(any(test, feature = "test-support"))]
        frames: 0,
    })
}

/// A changed tag's country at an hour the extract followed: its index in the
/// extract's scan, its counters and its unknown losses as that scan counts
/// them (`unknown`) and as they moved since the first hour for a country
/// seen before (`relative`).
struct Values {
    country: usize,
    dealt: i128,
    taken: i128,
    unknown: Option<i64>,
    relative: Option<i64>,
}

/// One hour of a block's facts after its first, in hour order.
#[allow(
    clippy::large_enum_variant,
    reason = "Block facts retain observed frames inline without a per-hour allocation."
)]
enum Hour {
    /// An hour the extract observed.
    Observed {
        hour: i32,
        /// Whether its relations were read again.
        process: bool,
        /// Its naming snapshot, when it named again.
        renamed: Option<Box<DecodedFrame>>,
        /// The lifetimes first held there, in the order met, by the extract's
        /// country indices.
        inserted: Vec<PairKey>,
        /// The changed tags, in tag order, and their countries.
        tags: Arc<[String]>,
        values: Vec<Values>,
    },
    /// Hours `first..=last`, which the extract advanced in constant time.
    Quiet { first: i32, last: i32 },
}

impl Hour {
    fn last(&self) -> i32 {
        match *self {
            Self::Observed { hour, .. } => hour,
            Self::Quiet { last, .. } => last,
        }
    }
}

/// The block's first hour, which the fold observes on the carried observer,
/// and what the extract's scan held right after it.
struct First {
    frame: DecodedFrame,
    changes: FrameChanges,
    /// Each country's counters, by the extract's index.
    countries: Vec<(i128, i128)>,
    /// The lifetimes it held, its read sequence and its generation.
    pairs: usize,
    read: u64,
    generation: u64,
}

struct BlockFacts {
    first: First,
    hours: Vec<Hour>,
    /// The extract's observer at the block's end.
    block: Box<Prepared>,
    /// Frames the extract was given or built.
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

struct BlockExtract {
    prepared: Option<Box<Prepared>>,
    first: Option<First>,
    hours: Vec<Hour>,
    /// The changed tags last recorded, shared by later hours with equal ones.
    tags: Option<Arc<[String]>>,
    #[cfg(any(test, feature = "test-support"))]
    frames: usize,
}

macro_rules! inline {
    ($($type:ty),* $(,)?) => {
        $(impl Held for $type {
            fn held(&self, _: &mut HeldBytes) {}
        })*
    };
}

inline!(
    Country,
    super::super::HeldPair,
    super::super::Checkpoint,
    super::Observation,
);

impl Held for super::super::Track {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.packed).heap(&self.checkpoints);
    }
}

impl Held for super::super::Scan {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.identities)
            .heap(&self.indices)
            .heap(&self.countries)
            .heap(&self.pairs)
            .heap(&self.live)
            .heap(&self.holders)
            .heap(&self.tracks)
            .heap(&self.bounds)
            .heap(&self.active)
            .heap(&self.observed)
            .heap(&self.touched)
            .heap(&self.partial_changed)
            .heap(&self.inserted)
            .heap(&self.relative);
        // Each tag's naming operands where they change: the list, each tag,
        // each operand read (its tag, cosmetic tag and suffixes) and each
        // reason an operand could not be read.
        fn element<T>(_: &[T]) -> usize {
            size_of::<T>()
        }
        bytes.add(self.names.capacity() * element(&self.names));
        for (tag, inputs) in &self.names {
            bytes.heap(tag).add(inputs.capacity() * element(inputs));
            for (_, read) in inputs {
                match read {
                    Some(Ok(operands)) => {
                        bytes.add(operands.heap_bytes());
                    }
                    Some(Err(reason)) => {
                        bytes.heap(reason);
                    }
                    None => {}
                }
            }
        }
    }
}

impl Held for super::super::ScanFrames {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.scan)
            .heap(&self.census)
            .heap(&self.live)
            .heap(&self.by_slot)
            .heap(&self.seen)
            .heap(&self.endpoints)
            .heap(&self.relation_keys)
            .heap(&self.current_tags)
            .heap(&self.supplying_pairs)
            .heap(&self.changed_pairs)
            .heap(&self.changed_endpoints)
            .heap(&self.changed_tags)
            .heap(&self.name_snapshot)
            .heap(&self.name_entries)
            .heap(&self.name_views);
    }
}

impl Held for super::Catalogue {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.actors)
            .heap(&self.holders)
            .heap(&self.holder_coverage)
            .heap(&self.unknown_label)
            .heap(&self.warnings);
    }
}

/// The block's own observer: its series, its scan and pair history (every
/// country's packed samples, every pair's runs), and its keys and catalogue.
impl Held for Prepared {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.builder)
            .heap(&self.pairs)
            .heap(&self.tags)
            .heap(&self.values)
            .heap(&self.observations)
            .heap(&self.coverage)
            .heap(&self.ranks)
            .heap(&self.latest)
            .heap(&self.actors)
            .heap(&self.catalogue)
            .heap(&self.presentation_error);
    }
}

impl Extract for BlockExtract {
    /// What the block's facts hold: its first frame, each hour's renamed
    /// frame, pairs, tags (once however many hours share them) and values,
    /// and the block's own observer.
    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        if let Some(first) = &self.first {
            held.frame(&first.frame).changes(&first.changes);
            held.add(size_of_val(first.countries.as_slice()));
        }
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            if let Hour::Observed {
                renamed,
                inserted,
                tags,
                values,
                ..
            } = hour
            {
                if let Some(frame) = renamed {
                    held.frame(frame);
                }
                held.add(size_of_val(inserted.as_slice()) + size_of_val(values.as_slice()));
                held.shared(
                    Arc::as_ptr(tags),
                    tags.iter().map(|tag| size_of::<String>() + tag.len()).sum(),
                );
            }
        }
        held.heap(&self.prepared).total()
    }

    fn observe(
        &mut self,
        replay: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.frames += 1;
        }
        if PARTIAL
            .iter()
            .any(|table| matches!(&*frame.tables[table.index()], TableView::Truncated { .. }))
        {
            return Err(io::Error::other(
                "A casualty table read by some of its fields is truncated",
            ));
        }
        let running = AtomicBool::new(false);
        let prepared = self
            .prepared
            .get_or_insert_with(|| Box::new(Prepared::new(replay, frame.branch)));
        prepared.observe_changes(frame, replay, &running, changes)?;
        let builder = &mut prepared.builder;
        if self.first.is_none() {
            let scan = &mut builder.scan;
            self.first = Some(First {
                frame: frame.clone(),
                changes: FrameChanges {
                    rows_changed: changes.rows_changed,
                    table_read: changes.table_read,
                    slot_read: changes.slot_read,
                    updates: None,
                },
                countries: scan
                    .countries
                    .iter()
                    .map(|country| (country.dealt, country.taken))
                    .collect(),
                pairs: scan.pairs.len(),
                read: scan.read,
                generation: scan.generation,
            });
            scan.inserted = Some(Vec::new());
            scan.relative = Some(vec![Some(0); scan.countries.len()]);
            builder.irregular = false;
            return Ok(());
        }
        if builder.irregular {
            return Err(io::Error::other(
                "A casualty block's census, read or continuity changes after its first hour",
            ));
        }
        let same = self
            .tags
            .as_ref()
            .filter(|tags| {
                tags.len() == builder.changed_tags.len()
                    && tags.iter().zip(&builder.changed_tags).all(|(a, b)| a == b)
            })
            .cloned();
        let tags = match same {
            Some(tags) => tags,
            None => {
                let tags: Arc<[String]> = builder.changed_tags.iter().cloned().collect();
                self.tags = Some(Arc::clone(&tags));
                tags
            }
        };
        let renamed = if builder.renamed {
            builder.name_snapshot.clone().map(Box::new)
        } else {
            None
        };
        let process = builder.processed;
        let scan = &mut builder.scan;
        let inserted = scan
            .inserted
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        let mut values = Vec::with_capacity(tags.len());
        for tag in tags.iter() {
            let &country = scan
                .live
                .get(tag)
                .ok_or_else(|| io::Error::other("A changed casualty tag is not live"))?;
            let row = &scan.countries[country];
            values.push(Values {
                country,
                dealt: row.dealt,
                taken: row.taken,
                unknown: row.unknown.total,
                relative: scan
                    .relative
                    .as_ref()
                    .and_then(|relative| relative.get(country).copied())
                    .unwrap_or(Some(0)),
            });
        }
        self.hours.push(Hour::Observed {
            hour: frame.hour,
            process,
            renamed,
            inserted,
            tags,
            values,
        });
        Ok(())
    }

    fn quiet(&mut self, replay: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let all = hours.hours();
        let read = span.clone().all(|position| {
            slot_read(
                replay.format,
                &hours.metrics()[position],
                TableId::WarRelations,
                all[position],
            )
        });
        let span = &all[span];
        let (Some(&first), Some(&last)) = (span.first(), span.last()) else {
            return Ok(());
        };
        let prepared = self.prepared.as_mut().ok_or_else(|| {
            io::Error::other("Casualty hours were quiet before the block's first")
        })?;
        if !prepared.quiet_continues(first, read) {
            return Err(io::Error::other(
                "Casualty quiet hours do not continue the hour observed last",
            ));
        }
        prepared.advance_quiet(first, last)?;
        if let Some(Hour::Quiet { last: end, .. }) = self.hours.last_mut()
            && end.checked_add(1) == Some(first)
        {
            *end = last;
        } else {
            self.hours.push(Hour::Quiet { first, last });
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let BlockExtract {
            prepared,
            first,
            hours,
            #[cfg(any(test, feature = "test-support"))]
            frames,
            ..
        } = *self;
        let (Some(block), Some(first)) = (prepared, first) else {
            return Err(io::Error::other("Casualty block has no hours"));
        };
        Ok(Box::new(BlockFacts {
            first,
            hours,
            block,
            #[cfg(any(test, feature = "test-support"))]
            frames,
        }))
    }
}

/// The observation of one hour's outcome: `Ok(true)` while the family goes
/// on, `Ok(false)` once it refused the branch.
pub type Observed<'a> = dyn FnMut(i32, io::Result<()>) -> io::Result<bool> + 'a;

/// The carried relation key of the extract's lifetime `key` (endpoints in
/// carried index order), and whether its endpoints swap places.
fn carried_key(countries: &[usize], (first, second, start): PairKey) -> (PairKey, bool) {
    let (first, second) = (countries[first], countries[second]);
    if first < second {
        ((first, second, start), false)
    } else {
        ((second, first, start), true)
    }
}

/// How the carried observer continues the extract's after the block's
/// first hour, which both observed.
struct Follow {
    /// The first hour.
    hour: i32,
    /// The carried index of each of the extract's countries.
    countries: Vec<usize>,
    /// Each extract country's carried counters less its own, which stay
    /// apart through the block: they differ by ended lifetimes alone.
    offsets: Vec<(i128, i128)>,
    /// Each extract country's unknown losses as the carried scan counts
    /// them: from its carried total as for a country seen before (`Some`), or
    /// as the extract counts them for a country the carried scan never saw.
    seen: Vec<Option<Option<i64>>>,
    /// Each extract country's carried gap-recovery flag.
    at_least: Vec<bool>,
    /// The extract's first supplying read, whose agreement is already carried.
    first_read: u64,
    /// The carried read sequence and generation less the extract's.
    read: u64,
    generation: u64,
}

impl Follow {
    /// None when the carried observer does not continue the first hour as
    /// the extract's did: an hourly supplying read with a valid census,
    /// naming snapshot and pair history and nothing pending, holding every
    /// extract country at its census place, none of the lifetimes the
    /// extract first held later, and the pair history's runs at that hour.
    fn new(prepared: &Prepared, first: &First, block: &Prepared) -> Option<Self> {
        let hour = first.frame.hour;
        let builder = &prepared.builder;
        let scan = &builder.scan;
        let own = &block.builder.scan;
        let continues = scan.hour == Some(hour)
            && scan.last_read == Some(hour)
            && scan.gap_read.is_none()
            && !scan.active_dirty
            && builder.census.is_some()
            && builder.name_snapshot.is_some()
            && !builder.pending_relations
            && builder.supplying_pairs.is_some()
            && prepared.covered
            && prepared.named == scan.names.len();
        if !continues
            || own.identities.len() != first.countries.len()
            || own.live.len() != scan.live.len()
        {
            return None;
        }
        let mut countries = Vec::with_capacity(own.identities.len());
        for identity in &own.identities {
            countries.push(*scan.indices.get(identity)?);
        }
        if own
            .live
            .iter()
            .any(|(tag, &index)| scan.live.get(tag) != Some(&countries[index]))
        {
            return None;
        }
        for (&key, pair) in &own.pairs {
            let held = scan.pairs.contains_key(&carried_key(&countries, key).0);
            if held == (pair.ordinal >= first.pairs) {
                return None;
            }
        }
        if !prepared.pairs.follows(&block.pairs, hour) {
            return None;
        }
        let offsets = first
            .countries
            .iter()
            .zip(&countries)
            .map(|(&(dealt, taken), &index)| {
                let carried = &scan.countries[index];
                (carried.dealt - dealt, carried.taken - taken)
            })
            .collect();
        let seen = countries
            .iter()
            .map(|&index| {
                let unknown = &scan.countries[index].unknown;
                unknown.seen.then_some(unknown.total)
            })
            .collect();
        let at_least = countries
            .iter()
            .map(|&index| scan.countries[index].at_least)
            .collect();
        Some(Self {
            hour,
            countries,
            offsets,
            seen,
            at_least,
            first_read: first.read,
            read: scan.read.wrapping_sub(first.read),
            generation: scan.generation.wrapping_sub(first.generation),
        })
    }

    /// An extract country's unknown losses (its own count `own`, and
    /// `relative` since the first hour as for a country seen before) as the
    /// carried scan counts them.
    fn unknown(&self, country: usize, own: Option<i64>, relative: Option<i64>) -> Option<i64> {
        match self.seen[country] {
            None => own,
            Some(total) => {
                total.and_then(|total| relative.and_then(|relative| total.checked_add(relative)))
            }
        }
    }

    /// A changed tag's country as the carried scan holds it at that hour, for
    /// its track's sample.
    fn country(&self, values: &Values) -> Country {
        let index = values.country;
        let (dealt, taken) = self.offsets[index];
        Country {
            dealt: values.dealt + dealt,
            taken: values.taken + taken,
            unknown: Unknown {
                total: self.unknown(index, values.unknown, values.relative),
                ..Default::default()
            },
            at_least: self.at_least[index],
            ..Default::default()
        }
    }

    /// Continue the carried observer through the extract's hours after the
    /// first, then carry the block's relations, countries and history.
    fn follow(
        &self,
        state: &mut Prepared,
        hours: Vec<Hour>,
        block: Prepared,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let cancelled = AtomicBool::new(false);
        let mut inserted = Vec::new();
        let mut countries = Vec::new();
        for hour in hours {
            check()?;
            match hour {
                Hour::Quiet { first, last } => state.advance_quiet(first, last)?,
                Hour::Observed {
                    hour,
                    process,
                    renamed,
                    inserted: held,
                    tags,
                    values: samples,
                } => {
                    for values in &mut state.values {
                        values.advance(hour)?;
                    }
                    state.observations.advance(hour)?;
                    state.coverage.advance(hour)?;
                    if let Some(snapshot) = renamed {
                        state.builder.rename(*snapshot);
                    }
                    inserted.clear();
                    inserted.extend(
                        held.into_iter()
                            .map(|key| carried_key(&self.countries, key).0),
                    );
                    countries.clear();
                    countries.extend(samples.iter().map(|values| self.country(values)));
                    state
                        .builder
                        .scan
                        .follow_hour(hour, process, &inserted, &tags, &countries);
                    state.first.get_or_insert(hour);
                    state.last = hour;
                    state.observe_tags(tags.iter(), hour, true, true)?;
                    state.name_tags(&cancelled)?;
                }
            }
        }
        self.carry(state, block)
    }

    /// After the block: the extract's relation counters, read sequences and
    /// generations, its countries' totals, unknown losses and last reads,
    /// its relation joins and its pair history, as the carried scan holds
    /// them.
    fn carry(&self, state: &mut Prepared, block: Prepared) -> io::Result<()> {
        let Prepared { builder, pairs, .. } = block;
        let super::super::ScanFrames {
            scan: own,
            relation_keys,
            endpoints,
            supplying_pairs,
            supplying_keys_current,
            pending_relations,
            name_views,
            ..
        } = builder;
        let scan = &state.builder.scan;
        if scan.read != own.read.wrapping_add(self.read)
            || scan.generation != own.generation.wrapping_add(self.generation)
        {
            return Err(io::Error::other(
                "A casualty block's fold lost its read sequence",
            ));
        }
        // Each of the extract's relation ordinals with the carried one and
        // the carried key.
        let mut ordinals = Vec::with_capacity(own.pairs.len());
        for (&key, pair) in &own.pairs {
            let (key, _) = carried_key(&self.countries, key);
            let held = scan.pairs.get(&key).ok_or_else(|| {
                io::Error::other("A casualty block's relation lifetime was not carried")
            })?;
            ordinals.push((pair.ordinal, held.ordinal, key));
        }
        state
            .pairs
            .carry(pairs, self.hour, &state.builder.scan, &ordinals)?;
        let carried = &mut state.builder;
        carried.relation_keys = relation_keys;
        carried.endpoints = endpoints
            .into_iter()
            .map(|(first, second)| (self.countries[first], self.countries[second]))
            .collect();
        carried.supplying_pairs = supplying_pairs.map(|keys| {
            keys.into_iter()
                .map(|key| carried_key(&self.countries, key).0)
                .collect()
        });
        carried.supplying_keys_current = supplying_keys_current;
        carried.pending_relations = pending_relations;
        carried.name_views = name_views;
        carried.changed_tags.clear();
        carried.changed_pairs.clear();
        carried.changed_endpoints.clear();
        let scan = &mut carried.scan;
        let relative = own.relative;
        for (index, country) in own.countries.into_iter().enumerate() {
            let held = &mut scan.countries[self.countries[index]];
            let (dealt, taken) = self.offsets[index];
            held.dealt = country.dealt + dealt;
            held.taken = country.taken + taken;
            if country.touched != 0 {
                let moved = relative
                    .as_ref()
                    .and_then(|relative| relative.get(index).copied())
                    .unwrap_or(Some(0));
                let total = self.unknown(index, country.unknown.total, moved);
                let seen = held.unknown.seen || country.unknown.seen;
                // The first read has already run against the carried pairs. If
                // no later read touches this country, its agreement stays that
                // read's, rather than the extract's initially unseen pairs'.
                let agreement = if country.touched == self.first_read {
                    std::mem::take(&mut held.unknown)
                } else {
                    country.unknown
                };
                held.unknown = Unknown {
                    total,
                    seen,
                    ..agreement
                };
                held.touched = country.touched.wrapping_add(self.read);
            }
        }
        for (key, pair) in own.pairs {
            let (key, swapped) = carried_key(&self.countries, key);
            let held = scan.pairs.get_mut(&key).ok_or_else(|| {
                io::Error::other("A casualty block's relation lifetime was not carried")
            })?;
            let [first, second, first_unknown, second_unknown] = pair.values;
            held.values = if swapped {
                [second, first, second_unknown, first_unknown]
            } else {
                pair.values
            };
            held.read = pair.read.wrapping_add(self.read);
            held.hour = pair.hour;
            held.generation = if pair.generation == u64::MAX {
                u64::MAX
            } else {
                pair.generation.wrapping_add(self.generation)
            };
        }
        scan.touched = own
            .touched
            .iter()
            .map(|&index| self.countries[index])
            .collect();
        Ok(())
    }
}

/// Fold one block's facts into the carried observer. Returns how it folded
/// and the last hour it reached.
pub fn fold(
    prepared: &mut Prepared,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let facts = {
        let _cpu = ::replay_frame::cpu::scope("fold facts", None);
        facts
            .downcast::<BlockFacts>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    let BlockFacts {
        first,
        hours,
        block,
        #[cfg(any(test, feature = "test-support"))]
        frames,
    } = *facts;
    #[cfg(any(test, feature = "test-support"))]
    FRAMES.with(|count| count.set(count.get() + frames));
    check()?;
    let running = AtomicBool::new(false);
    let hour = first.frame.hour;
    #[cfg(any(test, feature = "test-support"))]
    OBSERVED.with(|count| count.set(count.get() + 1));
    let result = {
        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
        prepared.observe_changes(&first.frame, replay, &running, &first.changes)
    };
    if !observed(hour, result)? {
        return Ok((Folded::Fast, Some(hour)));
    }
    let Some(last) = hours.last().map(Hour::last) else {
        return Ok((Folded::Fast, Some(hour)));
    };
    let Some(follow) = Follow::new(prepared, &first, &block) else {
        let reached = observe_block(prepared, replay, index, Some(hour), observed, check)?;
        return Ok((Folded::Fallback, reached.or(Some(hour))));
    };
    check()?;
    let result = {
        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
        prepared.own(|state| follow.follow(state, hours, *block, check))
    };
    observed(last, result)?;
    Ok((Folded::Fast, Some(last)))
}

/// Observe the block hour by hour after `after` (every hour without it),
/// with the change facts of the block alone, as hourly preparation does.
/// Returns the last hour observed.
pub fn observe_block(
    prepared: &mut Prepared,
    replay: &Replay,
    index: &BlockIndex,
    after: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let running = AtomicBool::new(false);
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        TABLES,
        |raw, frame| {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            let facts = tracker.observe_updates(frame, Arc::new(raw), replay.format);
            if stopped || after.is_some_and(|after| frame.hour <= after) {
                return Ok(());
            }
            reached = Some(frame.hour);
            #[cfg(any(test, feature = "test-support"))]
            OBSERVED.with(|count| count.set(count.get() + 1));
            let result = {
                let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                prepared.observe_changes(frame, replay, &running, &facts)
            };
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Casualty observation stopped"))
                }
            }
        },
        check,
    );
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    Ok(reached)
}

/// The section holding a branch's casualty continuation, which hourly
/// preparation keeps in its branch state.
pub fn state_section(branch: u64) -> String {
    format!("war-casualties-state-{branch}")
}

/// One branch's casualties prepared on their own: the observer hourly
/// preparation keeps in its branch state, and the pair windows of the blocks
/// since its last commit.
pub struct OwnBranch {
    prepared: Box<Prepared>,
    /// Whether the static presentation is bound, as preparation binds it
    /// before observing.
    configured: bool,
    /// The block being observed: its first and last hours.
    block: Option<(i32, i32)>,
    /// Each block's pair window and its ordinal, written at the next commit.
    windows: Vec<(u64, Vec<u8>)>,
    /// A window that could not be rendered, reported at the next commit;
    /// hourly preparation publishes no block's window after it.
    failed: Option<io::Error>,
}

impl OwnBranch {
    /// The branch's observer as last committed (`saved`, its continuation
    /// section), or new without a commit to read.
    pub fn load(saved: Option<store::Range>, replay: &Replay, branch: u64) -> io::Result<Self> {
        let prepared: Prepared = match saved {
            Some(file) => serde_json::from_reader(zstd::stream::Decoder::new(file)?)?,
            None => Prepared::new(replay, branch),
        };
        Ok(Self {
            prepared: Box::new(prepared),
            configured: false,
            block: None,
            windows: Vec::new(),
            failed: None,
        })
    }

    /// Bind the static presentation once, before the first observation.
    fn configure(&mut self, source: &Source, context: &LoadContext) -> io::Result<()> {
        if !self.configured {
            self.prepared.configure(&prepared_content(source, context)?);
            self.configured = true;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fold_facts(
        &mut self,
        facts: Box<dyn Any + Send>,
        replay: &Replay,
        index: &BlockIndex,
        source: &Source,
        context: &LoadContext,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        self.configure(source, context)?;
        self.block = Some((index.first_hour, index.last_hour));
        fold(&mut self.prepared, facts, replay, index, observed, check)
    }

    pub fn observe_hours(
        &mut self,
        replay: &Replay,
        index: &BlockIndex,
        source: &Source,
        context: &LoadContext,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        self.configure(source, context)?;
        self.block = Some((index.first_hour, index.last_hour));
        observe_block(&mut self.prepared, replay, index, None, observed, check)
    }

    /// After a block the family went on through: its pair window, as hourly
    /// preparation publishes it at the block's end. After a window that
    /// could not be rendered no later one is, as hourly preparation refuses
    /// the branch there.
    pub fn finish_block(&mut self) {
        let Some((first, last)) = self.block.take() else {
            return;
        };
        if self.failed.is_some() {
            return;
        }
        match self.prepared.finish_block_window(first, last) {
            Ok(window) => self.windows.push(window),
            Err(error) => self.failed = Some(error),
        }
    }

    pub fn pending_bytes(&self) -> usize {
        self.prepared.bytes()
            + self
                .windows
                .iter()
                .map(|(_, window)| window.len())
                .sum::<usize>()
    }

    /// The pair windows of the blocks gone through since the last commit,
    /// refused since or not, which hourly preparation wrote at each block's
    /// end; then the failure of a window that could not be rendered.
    pub fn publish_windows(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        for (ordinal, window) in self.windows.drain(..) {
            writer.write_at(&pairs_section(branch), ordinal, window.as_slice())?;
            #[cfg(any(test, feature = "test-support"))]
            dashboard_base::prepared::pages::framing::note_compressed(&window);
        }
        match self.failed.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// The flush hourly preparation makes at a batch's end.
    pub fn flush_batch(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.prepared.flush(writer, branch, cancelled)
    }

    /// The continuation hourly preparation keeps in its branch state, in its
    /// own compressed section.
    pub fn write_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        let json = serde_json::to_vec(&*self.prepared)?;
        let bytes = zstd::stream::encode_all(json.as_slice(), 1)?;
        writer.write_at(&state_section(branch), 0, bytes.as_slice())
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static OBSERVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FRAMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Hours observed on the carried observer on this thread: each folded
/// block's first hour, and every hour of a block observed hour by hour.
#[cfg(any(test, feature = "test-support"))]
pub fn observed_hours() -> usize {
    OBSERVED.with(std::cell::Cell::get)
}

/// Frames the extracts of the blocks folded on this thread were given or
/// built: every hour of a block read whole, only the hours kept of a block
/// read by its changes.
#[cfg(any(test, feature = "test-support"))]
pub fn built_frames() -> usize {
    FRAMES.with(std::cell::Cell::get)
}

#[cfg(test)]
mod bytes_tests {
    use super::*;
    use installed_content_country_names::country_names::Operands;

    /// What a scan holds when `countries` tags each name again at each of
    /// `hours` hours, every operand read carrying a cosmetic tag of
    /// `COSMETIC` bytes.
    fn held(countries: usize, hours: usize) -> u64 {
        let mut scan = super::super::super::Scan::new();
        for hour in 0..hours {
            for country in 0..countries {
                let tag = format!("T{country:03}");
                let mut operands = Operands::for_tag(&tag, Some(&format!("group_{hour}")));
                operands.cosmetic = "c".repeat(COSMETIC);
                scan.names
                    .push((tag, vec![(country as u32, Some(Ok(operands)))]));
            }
        }
        let mut bytes = HeldBytes::default();
        bytes.heap(&scan);
        bytes.total()
    }

    #[test]
    fn carrying_a_quiet_country_keeps_its_first_carried_agreement() {
        use super::super::super::test_fixtures::{casualty_frame, research_replay};
        let replay = research_replay(vec![casualty_frame(100, None, 0)]);
        for touched in [1, 2] {
            let moved = if touched == 1 { 0 } else { 3 };
            let mut state = Prepared::new(&replay, 1);
            state.builder.scan.read = 6;
            state.builder.scan.countries.push(Country {
                unknown: Unknown {
                    total: Some(7),
                    seen: true,
                    present: true,
                    delta: Some(0),
                    valid: true,
                    ..Default::default()
                },
                touched: 5,
                ..Default::default()
            });
            let mut block = Prepared::new(&replay, 1);
            block.builder.scan.read = 2;
            block.builder.scan.relative = Some(vec![Some(moved)]);
            block.builder.scan.countries.push(Country {
                unknown: Unknown {
                    total: None,
                    seen: true,
                    present: true,
                    delta: (touched == 2).then_some(3),
                    valid: true,
                    new_max: if touched == 1 { 11 } else { 0 },
                    ..Default::default()
                },
                touched,
                ..Default::default()
            });
            let follow = Follow {
                hour: 100,
                countries: vec![0],
                offsets: vec![(0, 0)],
                seen: vec![Some(Some(7))],
                at_least: vec![false],
                first_read: 1,
                read: 4,
                generation: 0,
            };
            follow.carry(&mut state, block).unwrap();
            let unknown = &state.builder.scan.countries[0].unknown;
            assert_eq!(unknown.total, Some(7 + moved));
            assert!(unknown.seen && unknown.present && unknown.valid);
            assert_eq!(unknown.new_max, 0);
            assert_eq!(unknown.delta, Some(if touched == 1 { 0 } else { 3 }));
        }
    }

    const COSMETIC: usize = 1024;

    /// Every naming change keeps its operands' own strings, so the naming
    /// history holds at least each cosmetic tag once per country and hour.
    // regression: 36c91d8fee Count the naming operands a casualty block's history keeps
    #[test]
    fn naming_history_bytes_grow_with_countries_times_hours() {
        let per = COSMETIC as u64;
        let (small, wide, long, both) = (held(4, 4), held(32, 4), held(4, 32), held(32, 32));
        assert!(wide - small >= (32 - 4) * 4 * per, "{small} {wide}");
        assert!(long - small >= 4 * (32 - 4) * per, "{small} {long}");
        assert!(both - wide >= 32 * (32 - 4) * per, "{wide} {both}");
        assert!(both - long >= (32 - 4) * 32 * per, "{long} {both}");
    }
}
