//! Production read a segment at a time from a columnar walk ([`replay::segments`]): each hour
//! hands its branch the rows of the line and Rocket output that changed, each country the
//! ProductionLines rows name, the countries where they changed, and the variant operands a
//! complete read built anew ([`Projector`]). A branch keeps the rows and feeds its
//! accumulator one hour at a time as a row frame would ([`Branch`]), so its facts and spent
//! series are those the row walk prepares.

use super::{
    ProductionAccumulator,
    input::{Changed, Complete, Hour as Input, Lines, Pending, Reads, Variants},
    slot_read,
};
use crate::products::equipment::{
    RawVariant, VariantHour,
    columnar::{Naming, slots},
    production::{CountryHolders, Rows},
};
use dashboard_world::{Needs, Roster, World};
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryRow, ProductionLineOutputRow, RocketProductionOutputRow, RowCodec, TABLE_COUNT,
        TableId,
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    mem::size_of,
    sync::{Arc, atomic::AtomicBool},
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = super::TABLES;

/// What a projection reads of its walk's world: the countries, and the cosmetic tags variant
/// names are read with.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        naming: true,
        ..Needs::default()
    }
}

/// The tables whose changes the row walk observes production at, beside the countries' keys
/// and every dated read.
const WATCHED: [TableId; 9] = [
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::ProductionLines,
    TableId::ProductionLineOutput,
    TableId::RocketProductionOutput,
];

/// The countries and links of an hour as production compares them, and what they resolve.
struct Countries {
    /// Each country's id, canonical slot and tag, by id.
    rows: Vec<CountryRow>,
    /// Each link's raw id and canonical slot; none while CountryLinks is not complete.
    links: Option<Vec<(u32, u32)>>,
    holders: Arc<CountryHolders>,
}

impl Countries {
    fn read(roster: &Roster, links: bool) -> io::Result<Self> {
        let rows = roster.country_rows()?;
        let (links, slots) = if links {
            let held = roster.links();
            (
                Some(
                    held.iter()
                        .map(|link| (link.id, link.canonical_slot))
                        .collect(),
                ),
                crate::metadata::link_slots(held),
            )
        } else {
            (None, HashMap::new())
        };
        let holders = Arc::new(CountryHolders::of(&rows, Arc::new(slots)));
        Ok(Self {
            rows,
            links,
            holders,
        })
    }

    /// Whether `other` holds the same countries and links, as the row walk compares them.
    fn same(&self, other: &Self) -> bool {
        self.links == other.links
            && self.rows.len() == other.rows.len()
            && self.rows.iter().zip(&other.rows).all(|(left, right)| {
                (left.id, left.canonical_slot, &left.tag)
                    == (right.id, right.canonical_slot, &right.tag)
            })
    }

    fn heap(&self) -> usize {
        size_of::<Self>()
            + self.rows.capacity() * size_of::<CountryRow>()
            + self
                .rows
                .iter()
                .map(|row| row.tag.capacity())
                .sum::<usize>()
            + self.links.as_ref().map_or(0, |links| links.capacity() * 8)
            + size_of::<CountryHolders>()
            + self.holders.holders.len() * (size_of::<(String, Option<(u32, u32)>)>() + 8)
            + self.holders.slots.capacity() * 9
    }
}

/// A keyed table's rows the hour changed: every live row where `whole`, else those it
/// inserted or wrote and the keys it deleted.
struct Delta<R> {
    whole: bool,
    rows: Vec<(u128, R)>,
    deleted: Vec<u128>,
}

impl<R: RowCodec> Delta<R> {
    fn apply(self, held: &mut BTreeMap<u128, R>) {
        if self.whole {
            held.clear();
        }
        for key in &self.deleted {
            held.remove(key);
        }
        held.extend(self.rows);
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<(u128, R)>()
            + self
                .rows
                .iter()
                .map(|(_, row)| row.heap_bytes())
                .sum::<usize>()
            + self.deleted.capacity() * size_of::<u128>()
    }
}

/// What a complete variant read built ([`Naming::read`]).
struct Built {
    whole: bool,
    built: Vec<(u64, RawVariant)>,
    dropped: Vec<u64>,
}

/// What an hour hands its branch beside its flags.
struct Facts {
    /// The row walk observes the hour: something production watches changed, or a read is
    /// dated there.
    observed: bool,
    /// The countries, where they or their completeness may have changed: none while
    /// Countries is not complete.
    countries: Option<Option<Arc<Countries>>>,
    changed: Changed,
    reads: Reads,
    output: Option<Delta<ProductionLineOutputRow>>,
    /// Why the line output is unavailable, at a read where it is.
    output_reason: Option<String>,
    /// Each country the ProductionLines rows name, ascending, where it changed.
    lines: Option<Arc<[u32]>>,
    rocket: Option<Delta<RocketProductionOutputRow>>,
    variants: Option<Built>,
    /// The bytes the facts hold.
    bytes: usize,
}

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    complete: Complete,
    /// VariantChanges holds an accepted design edit.
    design_changed: bool,
    /// None when nothing production reads changed and no read is dated.
    facts: Option<Box<Facts>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.facts
            .as_ref()
            .map_or(0, |facts| size_of::<Facts>() + facts.bytes)
    }
}

/// Each country the ProductionLines rows name, kept from the hour's changes.
#[derive(Default)]
struct LineCountries {
    /// Each slot's country.
    slots: Vec<Option<u32>>,
    /// Each country's rows.
    counts: HashMap<u32, u32>,
    /// A country gained its first row or lost its last.
    moved: bool,
}

impl LineCountries {
    fn clear(&mut self) {
        self.slots.clear();
        self.counts.clear();
        self.moved = true;
    }

    fn remove(&mut self, slot: u32) {
        let Some(country) = self.slots.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        match self.counts.get_mut(&country) {
            Some(count) if *count > 1 => *count -= 1,
            _ => {
                self.counts.remove(&country);
                self.moved = true;
            }
        }
    }

    fn insert(&mut self, slot: u32, country: u32) {
        self.remove(slot);
        let index = slot as usize;
        if self.slots.len() <= index {
            self.slots.resize(index + 1, None);
        }
        self.slots[index] = Some(country);
        let count = self.counts.entry(country).or_default();
        self.moved |= *count == 0;
        *count += 1;
    }

    fn countries(&self) -> Arc<[u32]> {
        let mut countries: Vec<u32> = self.counts.keys().copied().collect();
        countries.sort_unstable();
        countries.into()
    }

    fn heap(&self) -> usize {
        self.slots.capacity() * size_of::<Option<u32>>() + self.counts.capacity() * 9
    }
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"production projector: cold";

/// One segment's projection: every segment starts cold, its first hour read whole.
pub struct Projector {
    format: FormatVersion,
    line_country: usize,
    cold: bool,
    last: Option<i32>,
    /// Each table's status the hour before.
    shapes: [Option<columns::Status>; TABLE_COUNT],
    lines: LineCountries,
    naming: Naming,
}

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            line_country: columns::field(TableId::ProductionLines, "country")?,
            cold: true,
            last: None,
            shapes: [None; TABLE_COUNT],
            lines: LineCountries::default(),
            naming: Naming::new()?,
        })
    }

    /// The country of the ProductionLines row at `slot`.
    fn line_country(&self, state: &State, slot: u32) -> io::Result<u32> {
        let table = TableId::ProductionLines;
        let held = columns::table(state, table)?;
        if held.recorded(self.line_country, slot) {
            return Ok(columns::plain::<u32>(held, self.line_country)?[slot as usize]);
        }
        Ok(columns::row::<replay_data::rows::ProductionLineRow>(
            state,
            table,
            slot,
            self.format.version(),
        )?
        .country)
    }

    /// Each country the ProductionLines rows name, where it changed: read whole, or by the
    /// hour's changes.
    fn lines(&mut self, state: &State, whole: bool) -> io::Result<Option<Arc<[u32]>>> {
        let table = TableId::ProductionLines;
        if !columns::complete(state, table) {
            self.lines.clear();
            return Ok(None);
        }
        if whole {
            self.lines.clear();
            for slot in columns::table(state, table)?
                .live()
                .ones()
                .collect::<Vec<_>>()
            {
                let country = self.line_country(state, slot)?;
                self.lines.insert(slot, country);
            }
        } else {
            let changes = columns::changes(state, table)?;
            for &slot in &changes.deleted {
                self.lines.remove(slot);
            }
            for slot in slots::edits(state, table, 1_u64 << self.line_country)? {
                let country = self.line_country(state, slot)?;
                self.lines.insert(slot, country);
            }
        }
        Ok(std::mem::take(&mut self.lines.moved).then(|| self.lines.countries()))
    }

    fn heap(&self) -> usize {
        self.lines.heap()
    }
}

/// `table`'s rows the hour changed, read whole at `whole`; none while it is not complete or
/// the hour changed none.
fn delta<R: RowCodec>(
    state: &State,
    table: TableId,
    whole: bool,
    format: u16,
) -> io::Result<Option<Delta<R>>> {
    if !columns::complete(state, table) {
        return Ok(None);
    }
    let held = columns::table(state, table)?;
    let read = |slot: u32| -> io::Result<(u128, R)> {
        let key = held
            .key(slot)
            .ok_or_else(|| io::Error::other(format!("A live {table:?} slot holds no row")))?;
        Ok((key, columns::row(state, table, slot, format)?))
    };
    if whole {
        let rows = held
            .live()
            .ones()
            .map(read)
            .collect::<io::Result<Vec<_>>>()?;
        return Ok(Some(Delta {
            whole: true,
            rows,
            deleted: Vec::new(),
        }));
    }
    let changes = columns::changes(state, table)?;
    let slots = slots::edits(state, table, u64::MAX)?;
    if slots.is_empty() && changes.deleted_keys.is_empty() {
        return Ok(None);
    }
    Ok(Some(Delta {
        whole: false,
        rows: slots
            .into_iter()
            .map(read)
            .collect::<io::Result<Vec<_>>>()?,
        deleted: changes.deleted_keys.clone(),
    }))
}

/// Whether the hour inserted, deleted or wrote a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

/// Why the line output is unavailable, as the row decoder spells it; none otherwise.
fn unavailable(state: &State) -> io::Result<Option<String>> {
    match columns::status(state, TableId::ProductionLineOutput) {
        columns::Status::Unavailable { reason } => {
            let reason = state.dictionary().get(reason).ok_or_else(|| {
                io::Error::other("A ProductionLineOutput reason names no dictionary entry")
            })?;
            Ok(Some(String::from_utf8_lossy(reason).into_owned()))
        }
        _ => Ok(None),
    }
}

// Every segment starts cold: its first hour reads every table it keeps again; what came
// before is the branch's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("production.columnar", Some("production"));
        let state = hour.state;
        let at = hour.hour;
        let world = hour.world;
        let roster = world.countries()?;
        let whole = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        let mut shaped = [false; TABLE_COUNT];
        let mut changed = [false; TABLE_COUNT];
        for &table in TABLES {
            let status = columns::status(state, table);
            let index = table.index();
            shaped[index] = whole || self.shapes[index] != Some(status);
            changed[index] = shaped[index] || touched(state, table)?;
            self.shapes[index] = Some(status);
        }
        let complete = Complete {
            countries: columns::complete(state, TableId::Countries),
            links: columns::complete(state, TableId::CountryLinks),
            output: columns::complete(state, TableId::ProductionLineOutput),
            lines: columns::complete(state, TableId::ProductionLines),
            rocket: columns::complete(state, TableId::RocketProductionOutput),
            definitions: columns::complete(state, TableId::VariantDefinitions),
            lexical: columns::complete(state, TableId::LexicalTokens),
        };
        let design_changed = columns::complete(state, TableId::VariantChanges)
            && columns::table(state, TableId::VariantChanges)?
                .live()
                .ones()
                .next()
                .is_some();
        let metrics = world.metrics();
        let read = |table| slot_read(self.format, metrics, table, at);
        let reads = Reads {
            output: read(TableId::ProductionLineOutput),
            rocket: read(TableId::RocketProductionOutput),
            variants: read(TableId::VariantDefinitions),
        };
        let identity = whole
            || roster.changed()
            || roster.rebuilt()
            || changed[TableId::CountryLinks.index()]
            || shaped[TableId::Countries.index()]
            || (!complete.countries && changed[TableId::Countries.index()]);
        let others = TABLES
            .iter()
            .filter(|table| {
                !matches!(
                    table,
                    TableId::Campaign | TableId::Countries | TableId::CountryLinks
                )
            })
            .any(|table| changed[table.index()]);
        let observed =
            whole || identity || reads.any() || WATCHED.iter().any(|table| changed[table.index()]);
        let variants = {
            let read = self.naming.read(
                state,
                (roster, world.naming()?),
                self.format,
                at,
                whole,
                &shaped,
                &changed,
                reads.variants,
            )?;
            (read.evidence == VariantHour::Read
                && (read.whole || !read.built.is_empty() || !read.dropped.is_empty()))
            .then_some(Built {
                whole: read.whole,
                built: read.built,
                dropped: read.dropped,
            })
        };
        if !observed && variants.is_none() {
            return out.push(Hour {
                branch: hour.branch,
                hour: at,
                complete,
                design_changed,
                facts: None,
            });
        }
        let countries = identity
            .then(|| {
                complete
                    .countries
                    .then(|| Countries::read(roster, complete.links).map(Arc::new))
                    .transpose()
            })
            .transpose()?;
        let format = self.format.version();
        let (output, rocket) = (
            TableId::ProductionLineOutput,
            TableId::RocketProductionOutput,
        );
        let output = changed[output.index()]
            .then(|| {
                delta::<ProductionLineOutputRow>(state, output, shaped[output.index()], format)
            })
            .transpose()?
            .flatten();
        let rocket = changed[rocket.index()]
            .then(|| {
                delta::<RocketProductionOutputRow>(state, rocket, shaped[rocket.index()], format)
            })
            .transpose()?
            .flatten();
        let lines = if changed[TableId::ProductionLines.index()] {
            self.lines(state, shaped[TableId::ProductionLines.index()])?
        } else {
            None
        };
        let output_reason = (reads.output && !complete.output)
            .then(|| unavailable(state))
            .transpose()?
            .flatten();
        let bytes = countries
            .as_ref()
            .and_then(Option::as_ref)
            .map_or(0, |countries| countries.heap())
            + output.as_ref().map_or(0, Delta::heap)
            + rocket.as_ref().map_or(0, Delta::heap)
            + lines.as_ref().map_or(0, |lines| lines.len() * 4)
            + output_reason.as_ref().map_or(0, String::capacity)
            + variants.as_ref().map_or(0, |variants| {
                variants.built.capacity() * size_of::<(u64, RawVariant)>()
                    + variants
                        .built
                        .iter()
                        .map(|(_, raw)| raw.heap())
                        .sum::<usize>()
                    + variants.dropped.capacity() * 8
            });
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            complete,
            design_changed,
            facts: Some(Box::new(Facts {
                observed,
                countries,
                changed: Changed {
                    others,
                    output: changed[TableId::ProductionLineOutput.index()],
                    rocket: changed[TableId::RocketProductionOutput.index()],
                },
                reads,
                output,
                output_reason,
                lines,
                rocket,
                variants,
                bytes,
            })),
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved production projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}

/// One branch's production: its accumulator, and the rows and countries it reads as the
/// hours left them.
pub struct Branch {
    accumulator: Box<ProductionAccumulator>,
    countries: Option<Arc<Countries>>,
    output: BTreeMap<u128, ProductionLineOutputRow>,
    rocket: BTreeMap<u128, RocketProductionOutputRow>,
    lines: Arc<[u32]>,
    variants: Pending,
    /// The hours since the last ingested that change nothing and date no read, which the
    /// accumulator goes on over at once.
    quiet: Option<(i32, i32)>,
}

impl Branch {
    /// `branch`, from its first hour `first`, of a recording at `format`.
    pub fn new(format: FormatVersion, branch: u64, first: i32) -> Self {
        Self::resumed(Box::new(ProductionAccumulator::new(
            format.version(),
            branch,
            first,
        )))
    }

    /// A branch going on from `accumulator`, a saved branch's: the projector's next hour, the
    /// first of a segment, reads every row again.
    pub fn resumed(accumulator: Box<ProductionAccumulator>) -> Self {
        Self {
            accumulator,
            countries: None,
            output: BTreeMap::new(),
            rocket: BTreeMap::new(),
            lines: Arc::from([]),
            variants: Pending::default(),
            quiet: None,
        }
    }

    /// What the branch saves: its accumulator, which every flush leaves with no quiet hour
    /// pending.
    pub fn accumulator(&self) -> &ProductionAccumulator {
        &self.accumulator
    }

    /// Goes on over the quiet hours kept.
    fn settle(&mut self) -> io::Result<()> {
        let Some((first, last)) = self.quiet.take() else {
            return Ok(());
        };
        if self.accumulator.advance_quiet(first, last)? {
            Ok(())
        } else {
            Err(io::Error::other(
                "Production could not go on over its quiet hours",
            ))
        }
    }

    /// Keeps `hour`, and ingests it unless it is quiet.
    pub fn observe(&mut self, hour: Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("production:native");
        let Hour {
            hour: at,
            complete,
            design_changed,
            facts,
            ..
        } = hour;
        let mut countries = None;
        let mut changed = Changed::default();
        let mut reads = Reads::default();
        let mut output_reason = None;
        let mut observed = false;
        if let Some(facts) = facts {
            let Facts {
                observed: watched,
                countries: read,
                changed: moved,
                reads: dated,
                output,
                output_reason: reason,
                lines,
                rocket,
                variants,
                ..
            } = *facts;
            observed = watched;
            changed = moved;
            reads = dated;
            output_reason = reason;
            if let Some(built) = variants {
                self.variants.keep(built.whole, built.built, &built.dropped);
            }
            if let Some(delta) = output {
                delta.apply(&mut self.output);
            }
            if let Some(delta) = rocket {
                delta.apply(&mut self.rocket);
            }
            if let Some(lines) = lines {
                self.lines = lines;
            }
            if let Some(read) = read {
                // As the row walk tells them apart: an incomplete Countries never matches.
                let same = read.as_ref().is_some_and(|read| {
                    self.countries.as_ref().is_some_and(|held| held.same(read))
                });
                if !same {
                    countries = Some(read.as_ref().map(|read| Arc::clone(&read.holders)));
                }
                self.countries = read;
            }
        }
        if observed {
            self.settle()?;
        } else {
            match &mut self.quiet {
                Some((_, last)) if last.checked_add(1) == Some(at) => {
                    *last = at;
                    return Ok(());
                }
                _ => {}
            }
            self.settle()?;
            if self.accumulator.quiet_from(at) {
                self.quiet = Some((at, at));
                return Ok(());
            }
        }
        let input = Input {
            hour: at,
            countries,
            changed,
            dated: reads,
            reads,
            complete,
            design_changed,
            output: Rows::Keyed(&self.output),
            output_reason: output_reason.as_deref(),
            lines: Lines::Countries(&self.lines),
            rocket: Rows::Keyed(&self.rocket),
            variants: Variants::Read(&mut self.variants),
        };
        // Cancellation is checked between hours.
        self.accumulator.ingest_hour(input, &AtomicBool::new(false))
    }

    /// Publishes what the branch kept since it last did.
    pub fn flush(&mut self, writer: &mut store::Writer, cancelled: &AtomicBool) -> io::Result<()> {
        self.settle()?;
        self.accumulator.flush(writer, cancelled)
    }
}
