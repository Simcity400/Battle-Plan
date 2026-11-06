//! The production streams. A flush appends one immutable chunk to `production-index`: what
//! every history recorded since the flush before, each part encoded once, behind a
//! directory of fixed-width entries sorted by (history, part, key), so a reader slices
//! only the parts it answers from. Each chunk's range and hours go into a fixed-width
//! table, paged into `production-chunks` as pages fill. A publication appends one commit
//! to `production-commits` (the chunk count, the table's partial page, and where the
//! identities a query resolves lie) and one ledger record to `production-ledger` (the
//! compact state a resume restores; the keyed trees and put-away dormant histories lie
//! there too). The absence's unread spans are paged into `production-absence`.
//!
//! An interval (a run, a step, an output read, a modifier read) is written once, in the
//! chunk where it starts. One still open when its chunk is written is carried into every
//! later chunk its history writes until one closes it; while none has, it ends at its
//! history's horizon.
//!
//! A native recording's chunks follow its segments: one through each segment's first census,
//! then one or more after it. After that census a country's lines are folded per segment, in
//! parallel, into a lens: parts of their own, keyed past [`LENS`], shared by every history
//! that reads that country there. Such a history's chunk holds only its [`Binding`]: the lens
//! it reads and how the lens's ids, appearances, counts, versions and anchors read as its own.
//! A history's ids are numbered from its chunk's number up, so later chunks' come after.

use super::*;
use crate::prepared::pages::Pages;
use crate::products::country_groups::HourSpan;
use replay_data::rows::{CountryProductionModifiersRow, ProductionLineModifiersRow, TableRows};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

pub(super) const INDEX: &str = "production-index";
pub(super) const COMMITS: &str = "production-commits";
pub(super) const LEDGER: &str = "production-ledger";
pub(super) const CHUNKS: &str = "production-chunks";
pub(super) const ABSENCE: &str = "production-absence";

pub(super) fn index_section(branch: u64) -> String {
    format!("{INDEX}-v4-{branch}")
}
pub(super) fn commits_section(branch: u64) -> String {
    format!("{COMMITS}-v4-{branch}")
}
pub(super) fn ledger_section(branch: u64) -> String {
    format!("{LEDGER}-v3-{branch}")
}
pub(super) fn chunks_section(branch: u64) -> String {
    format!("{CHUNKS}-v2-{branch}")
}
pub(super) fn absence_section(branch: u64) -> String {
    format!("{ABSENCE}-v2-{branch}")
}

const MAGIC: &[u8; 4] = b"PLX1";
const HEADER: u64 = 12;
const ENTRY: u64 = 40;
/// Parts this small stay uncompressed: a frame costs more than it saves.
const RAW_BELOW: usize = 512;
const LEVEL: i32 = 1;

/// The history a chunk's branch-wide parts belong to; histories count from 1.
pub(super) const GLOBAL: u64 = 0;
/// A lens's parts lie under its id with this bit set; no history is numbered this far.
pub(super) const LENS: u64 = 1 << 63;
/// The bits of an id below its chunk's number.
pub(super) const ID_BITS: u32 = 32;

/// The least id a history allocates in chunk `chunk`.
pub(super) fn id_floor(chunk: u64) -> u64 {
    chunk << ID_BITS
}

/// Chunks in one page of `production-chunks`.
pub(super) const CHUNK_PAGE: usize = 1024;
const CHUNK_ENTRY: usize = 16;
/// Unread spans in one page of `production-absence`.
pub(super) const SPAN_PAGE: usize = 512;
const SPAN_ENTRY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Part {
    Variants = 0,
    IconEvents = 1,
    Aliases = 2,
    Overview = 3,
    Detail = 4,
    Closes = 5,
    Versions = 6,
    Countries = 7,
    Modifiers = 8,
    /// A history's [`Binding`] to the lens it reads in a chunk.
    Binding = 9,
    /// A lens's conditional versions and names ([`Marks`]).
    Marks = 10,
    /// Each variant's icon as the census inputs held it, where it changed ([`Timeline`]).
    Timeline = 11,
}

/// A lifetime as the read that began it named it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Life {
    pub id: u64,
    pub handle: u64,
    pub appearance: u32,
    pub kind: Option<LineKind>,
    pub before_recording: bool,
    pub between_reads: Option<HourSpan>,
    pub variants: Vec<u64>,
}

/// A lifetime's runs joined while the variant and factories hold ([`line_steps`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Step {
    pub id: u64,
    pub lifetime: u64,
    pub first_hour: i32,
    pub last_hour: i32,
    pub closed: bool,
    pub starts_segment: bool,
    pub variant: u64,
    pub active_factories: i32,
    pub requested_factories: i32,
}

/// A history's counts as they stand after its chunk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Counters {
    pub unknown_kinds: usize,
    pub unlisted_output: usize,
    pub unread_output_reads: usize,
    pub unreadable_stretches: usize,
    pub unlinked_rows: usize,
    pub refusals: BTreeMap<String, usize>,
    /// Some read recorded modifier operands for one of its lines.
    pub modified: bool,
}

/// Where a country woke: the absence's unread spans from the `from`th to before the `to`th
/// that end after `hour` fold in, each from `hour + 1`, before its own `at`th unread span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Wake {
    pub at: u64,
    pub from: u64,
    pub to: u64,
    pub hour: i32,
}

/// What `production-lines` reads of a history's chunk.
#[derive(Clone, Debug, Default)]
pub(super) struct Overview {
    pub lives: Vec<Life>,
    pub ends: Vec<(u64, i32)>,
    pub finals: Vec<(u64, RawOutput)>,
    pub steps: Vec<Step>,
    pub step_closes: Vec<(u64, i32)>,
    pub reads: Vec<i32>,
    pub line_hours: Vec<HourSpan>,
    pub output_hours: Vec<HourSpan>,
    pub organisation_hours: Vec<HourSpan>,
    pub unread_output: Vec<HourSpan>,
    pub wakes: Vec<Wake>,
    /// Variants the chunk's output reads name.
    pub output_variants: BTreeSet<u64>,
    /// Organisation versions whose naming operands differ from the version before.
    pub names: Vec<(u64, RawOrganisation)>,
    /// Each variant's runs of consecutive censuses producing it, first and last census hour:
    /// a census's icon of it is the [`Timeline`]'s at that hour.
    pub produced: Vec<(u64, i32, i32)>,
    pub counters: Counters,
}

/// A run's values without its hours, column by column.
#[derive(Clone, Debug, Default)]
pub(super) struct Values {
    pub starts_segment: Vec<bool>,
    pub variant: Vec<u64>,
    pub mio: Vec<u64>,
    pub active_factories: Vec<i32>,
    pub requested_factories: Vec<i32>,
    pub max_factories: Vec<i32>,
    pub cost_raw: Vec<i64>,
    pub speed_raw: Vec<i64>,
    pub efficiency_raw: Vec<i64>,
    pub priority: Vec<i32>,
    pub amount: Vec<i32>,
}

impl Values {
    pub fn push(&mut self, run: &RawRun) {
        self.starts_segment.push(run.starts_segment);
        self.variant.push(run.variant);
        self.mio.push(run.mio);
        self.active_factories.push(run.active_factories);
        self.requested_factories.push(run.requested_factories);
        self.max_factories.push(run.max_factories);
        self.cost_raw.push(run.cost_raw);
        self.speed_raw.push(run.speed_raw);
        self.efficiency_raw.push(run.efficiency_raw);
        self.priority.push(run.priority);
        self.amount.push(run.amount);
    }

    pub fn len(&self) -> usize {
        self.variant.len()
    }

    fn checked(&self) -> io::Result<()> {
        let len = self.len();
        if [
            self.starts_segment.len(),
            self.mio.len(),
            self.active_factories.len(),
            self.requested_factories.len(),
            self.max_factories.len(),
            self.cost_raw.len(),
            self.speed_raw.len(),
            self.efficiency_raw.len(),
            self.priority.len(),
            self.amount.len(),
        ]
        .into_iter()
        .any(|other| other != len)
        {
            return Err(crate::invalid("Production run columns differ in length"));
        }
        Ok(())
    }

    pub fn run(&self, at: usize, first_hour: i32, last_hour: i32) -> RawRun {
        RawRun {
            first_hour,
            last_hour,
            starts_segment: self.starts_segment[at],
            variant: self.variant[at],
            mio: self.mio[at],
            active_factories: self.active_factories[at],
            requested_factories: self.requested_factories[at],
            max_factories: self.max_factories[at],
            cost_raw: self.cost_raw[at],
            speed_raw: self.speed_raw[at],
            efficiency_raw: self.efficiency_raw[at],
            priority: self.priority[at],
            amount: self.amount[at],
        }
    }
}

/// Runs with their identities and hours, column by column. `last` is final where
/// `closed`; an open run's end is resolved later.
#[derive(Clone, Debug, Default)]
pub(super) struct Runs {
    pub id: Vec<u64>,
    pub lifetime: Vec<u64>,
    pub first: Vec<i32>,
    pub last: Vec<i32>,
    pub closed: Vec<bool>,
    pub values: Values,
}

impl Runs {
    pub fn push(&mut self, id: u64, lifetime: u64, run: &RawRun) -> usize {
        self.id.push(id);
        self.lifetime.push(lifetime);
        self.first.push(run.first_hour);
        self.last.push(run.last_hour);
        self.closed.push(false);
        self.values.push(run);
        self.id.len() - 1
    }

    pub fn close(&mut self, at: usize, last: i32) {
        self.last[at] = last;
        self.closed[at] = true;
    }

    pub fn len(&self) -> usize {
        self.id.len()
    }

    pub fn checked(&self) -> io::Result<()> {
        let len = self.len();
        if [
            self.lifetime.len(),
            self.first.len(),
            self.last.len(),
            self.closed.len(),
            self.values.len(),
        ]
        .into_iter()
        .any(|other| other != len)
        {
            return Err(crate::invalid("Production run columns differ in length"));
        }
        self.values.checked()
    }

    pub fn run(&self, at: usize) -> RawRun {
        self.values.run(at, self.first[at], self.last[at])
    }
}

/// An output read held until the next, as [`Runs`] holds a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OutputRecord {
    pub id: u64,
    pub lifetime: u64,
    pub output: RawOutput,
    pub closed: bool,
}

/// What `production-line-detail` reads of a history's chunk: the runs and output reads
/// open when it began (`carried_*`) and those it began.
#[derive(Clone, Debug, Default)]
pub(super) struct Detail {
    /// Every lifetime the chunk's runs and output reads name: its handle and appearance.
    pub lives: BTreeMap<u64, (u64, u32)>,
    pub carried_runs: Runs,
    pub runs: Runs,
    pub carried_outputs: Vec<OutputRecord>,
    pub outputs: Vec<OutputRecord>,
}

/// How the intervals open when a chunk began ended within it, without their values.
#[derive(Clone, Debug, Default)]
pub(super) struct Closes {
    pub runs: Vec<(u64, i32, bool)>,
    pub outputs: Vec<(u64, i32, bool)>,
}

/// Design reads and organisation states pushed in a chunk, in order, and the chunk where
/// the version holding when it began was pushed (none before it), for each variant and
/// organisation it pushed a version of, began a run with, or carried a run of.
#[derive(Clone, Debug, Default)]
pub(super) struct Versions {
    pub designs: Vec<(u64, DesignRead)>,
    pub organisations: Vec<(u64, RawOrganisation)>,
    pub design_anchors: BTreeMap<u64, Option<u64>>,
    pub organisation_anchors: BTreeMap<u64, Option<u64>>,
}

impl Versions {
    pub fn is_empty(&self) -> bool {
        self.designs.is_empty()
            && self.organisations.is_empty()
            && self.design_anchors.is_empty()
            && self.organisation_anchors.is_empty()
    }
}

/// One modifier read of a line, held until it stops being carried.
#[derive(Clone, Debug)]
pub(super) struct ModifierRecord {
    pub first_hour: i32,
    pub last_hour: i32,
    pub closed: bool,
    /// Its country row's place in the chunk's [`Part::Countries`].
    pub country: u32,
    /// None where it equals the lifetime's read before.
    pub line: Option<ProductionLineModifiersRow>,
    pub run: RawRun,
    pub organisation: Option<String>,
}

/// One lifetime's modifier reads in a chunk, and how the one open when it began ended.
#[derive(Clone, Debug, Default)]
pub(super) struct Modifiers {
    pub carried_last: Option<i32>,
    pub records: Vec<ModifierRecord>,
}

/// One lifetime's modifier reads as a chunk records them, until stored: each read's hours,
/// close and country row, and its line operands, run values and organisation only where they
/// differ from the read before, each by the read's place.
#[derive(Clone, Debug, Default)]
pub(super) struct ModifierLog {
    pub carried_last: Option<i32>,
    first: Vec<i32>,
    last: Vec<i32>,
    closed: Vec<bool>,
    country: Vec<u32>,
    lines: Vec<(u32, ProductionLineModifiersRow)>,
    /// Kept without their hours, which are each read's own.
    values: Vec<(u32, RawRun)>,
    organisations: Vec<(u32, Option<String>)>,
}

impl ModifierLog {
    /// Adds a read at `hour`, its line operands none where they equal the read before's: its
    /// place.
    pub fn push(
        &mut self,
        hour: i32,
        country: u32,
        line: Option<&ProductionLineModifiersRow>,
        run: &RawRun,
        organisation: Option<String>,
    ) -> usize {
        let at = self.first.len();
        let place = at as u32;
        self.first.push(hour);
        self.last.push(hour);
        self.closed.push(false);
        self.country.push(country);
        if let Some(line) = line {
            self.lines.push((place, line.clone()));
        }
        let values = RawRun {
            first_hour: 0,
            last_hour: 0,
            ..*run
        };
        if self.values.last().is_none_or(|(_, held)| *held != values) {
            self.values.push((place, values));
        }
        if self
            .organisations
            .last()
            .is_none_or(|(_, held)| *held != organisation)
        {
            self.organisations.push((place, organisation));
        }
        at
    }

    /// Ends the read at `at` at hour `last`.
    pub fn close(&mut self, at: usize, last: i32) {
        self.last[at] = last;
        self.closed[at] = true;
    }
}

/// What one history recorded since the flush before.
#[derive(Clone, Debug, Default)]
pub(super) struct Events {
    pub overview: Overview,
    pub detail: Detail,
    pub versions: Versions,
    pub countries: Vec<Arc<CountryProductionModifiersRow>>,
    pub modifiers: BTreeMap<u64, ModifierLog>,
    /// What the buffer holds, as the batch budget counts it.
    pub bytes: usize,
    /// Anything was recorded, the counts included.
    pub touched: bool,
}

impl Events {
    pub fn is_empty(&self) -> bool {
        !self.touched
    }
}

/// A chunk's changes to the icons the census inputs held, in hour order: each variant's from
/// an hour on, and the hours an input began anew, holding none until it chose one again (an
/// hour's choices come after its beginning).
#[derive(Clone, Debug, Default)]
pub(super) struct Timeline {
    pub changes: Vec<(i32, u64, IconChoice)>,
    pub resets: Vec<i32>,
}

impl Timeline {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.resets.is_empty()
    }
}

/// What a lens holds of its own at the census it began from, as one history held it then:
/// its lines' lifetimes (with the history's appearance), runs (with where the history's run
/// began), steps and output reads, and the history's counts then.
#[derive(Clone, Debug, Default)]
pub(super) struct Start {
    pub lifetimes: Vec<(u64, u64, u32)>,
    pub runs: Vec<(u64, u64, i32, bool)>,
    pub steps: Vec<(u64, u64)>,
    pub outputs: Vec<(u64, u64)>,
    pub counters: Counters,
    /// How many more appearances of each handle the census held than the lens.
    pub appearances: Vec<(u64, i64)>,
}

/// How one history reads a lens's parts in a chunk as its own. A lens id it does not name is
/// the history's `(map << ID_BITS) | id`; a lens's appearance of a handle reads `delta` more.
#[derive(Clone, Debug, Default)]
pub(super) struct Binding {
    /// The lens's id in the chunk.
    pub lens: u64,
    /// The chunk whose binding holds [`Start`]: the first the history read the lens in.
    pub map: u64,
    pub start: Option<Start>,
    /// The conditional versions ([`Marks`]) the history pushed, and those it named.
    pub included: Vec<u32>,
    pub named: Vec<u32>,
    /// Where the version holding when the chunk began was pushed, for each the lens names.
    pub design_anchors: BTreeMap<u64, Option<u64>>,
    pub organisation_anchors: BTreeMap<u64, Option<u64>>,
    pub appearances: Vec<(u64, i64)>,
}

/// A lens's versions and names a history pushes only as its own last versions say: each by
/// its place in the lens's lists, with its condition's number.
#[derive(Clone, Debug, Default)]
pub(super) struct Marks {
    pub designs: Vec<(u32, u32)>,
    pub organisations: Vec<(u32, u32)>,
    pub names: Vec<(u32, u32)>,
}

pub(super) fn encode_row(rows: TableRows) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    rows.encode_rows(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn decode_row(table: TableId, bytes: &[u8]) -> io::Result<TableRows> {
    let mut input = replay_data::codec::Decoder::new(bytes);
    let rows = table.decode_rows(1, &mut input)?;
    input.finish()?;
    Ok(rows)
}

pub(super) fn decode_country(bytes: &[u8]) -> io::Result<CountryProductionModifiersRow> {
    match decode_row(TableId::CountryProductionModifiers, bytes)? {
        TableRows::CountryProductionModifiers(mut rows) if rows.len() == 1 => Ok(rows.remove(0)),
        _ => Err(crate::invalid(
            "A production country modifier row does not decode",
        )),
    }
}

pub(super) fn decode_line(bytes: &[u8]) -> io::Result<ProductionLineModifiersRow> {
    match decode_row(TableId::ProductionLineModifiers, bytes)? {
        TableRows::ProductionLineModifiers(mut rows) if rows.len() == 1 => Ok(rows.remove(0)),
        _ => Err(crate::invalid(
            "A production line modifier row does not decode",
        )),
    }
}

/// A modifier part as it is stored: rows in the recording's own row encoding.
pub(super) struct StoredModifiers {
    pub carried_last: Option<i32>,
    pub first: Vec<i32>,
    pub last: Vec<i32>,
    pub closed: Vec<bool>,
    pub country: Vec<u32>,
    pub line: Vec<Option<Vec<u8>>>,
    pub values: Values,
    pub organisation: Vec<Option<String>>,
}

impl StoredModifiers {
    fn of(log: &ModifierLog) -> io::Result<Self> {
        let len = log.first.len();
        let mut line = Vec::with_capacity(len);
        let mut lines = log.lines.iter().peekable();
        let mut values = Values::default();
        let mut runs = log.values.iter().peekable();
        let mut run = None;
        let mut organisation = Vec::with_capacity(len);
        let mut names = log.organisations.iter().peekable();
        let mut name = None;
        for at in 0..len as u32 {
            line.push(
                lines
                    .next_if(|(place, _)| *place == at)
                    .map(|(_, row)| encode_row(TableRows::ProductionLineModifiers(vec![row.clone()])))
                    .transpose()?,
            );
            if let Some((_, held)) = runs.next_if(|(place, _)| *place == at) {
                run = Some(held);
            }
            values.push(run.ok_or_else(|| crate::invalid("A production modifier read has no run"))?);
            if let Some((_, held)) = names.next_if(|(place, _)| *place == at) {
                name = Some(held);
            }
            organisation.push(
                name.ok_or_else(|| crate::invalid("A production modifier read has no organisation"))?
                    .clone(),
            );
        }
        Ok(Self {
            carried_last: log.carried_last,
            first: log.first.clone(),
            last: log.last.clone(),
            closed: log.closed.clone(),
            country: log.country.clone(),
            line,
            values,
            organisation,
        })
    }

    pub fn records(self) -> io::Result<Modifiers> {
        let len = self.first.len();
        if [
            self.last.len(),
            self.closed.len(),
            self.country.len(),
            self.line.len(),
            self.values.len(),
            self.organisation.len(),
        ]
        .into_iter()
        .any(|other| other != len)
        {
            return Err(crate::invalid(
                "Production modifier columns differ in length",
            ));
        }
        self.values.checked()?;
        let mut records = Vec::with_capacity(len);
        for (at, ((line, organisation), first)) in self
            .line
            .into_iter()
            .zip(self.organisation)
            .zip(self.first.iter().copied())
            .enumerate()
        {
            records.push(ModifierRecord {
                first_hour: first,
                last_hour: self.last[at],
                closed: self.closed[at],
                country: self.country[at],
                line: line.as_deref().map(decode_line).transpose()?,
                run: self.values.run(at, first, first),
                organisation,
            });
        }
        Ok(Modifiers {
            carried_last: self.carried_last,
            records,
        })
    }
}

/// One part of a chunk, encoded and framed.
pub(super) struct Encoded {
    pub history: u64,
    pub part: Part,
    pub key: u64,
    pub bytes: Vec<u8>,
}

pub(super) fn encode_part<T: product_codec::Product>(
    history: u64,
    part: Part,
    key: u64,
    value: &T,
) -> io::Result<Encoded> {
    let mut raw = vec![0_u8];
    value.encode(&mut raw)?;
    let bytes = if raw.len() < RAW_BELOW {
        raw
    } else {
        let mut framed = vec![1_u8];
        framed.extend(crate::prepared::series::compress(&raw[1..], LEVEL)?);
        framed
    };
    Ok(Encoded {
        history,
        part,
        key,
        bytes,
    })
}

/// Every part of one history's events, each encoded once.
pub(super) fn encode_history(history: u64, events: &Events) -> io::Result<Vec<Encoded>> {
    let mut parts = vec![encode_part(history, Part::Overview, 0, &events.overview)?];
    let detail = &events.detail;
    if detail.runs.len() != 0
        || detail.carried_runs.len() != 0
        || !detail.outputs.is_empty()
        || !detail.carried_outputs.is_empty()
    {
        parts.push(encode_part(history, Part::Detail, 0, detail)?);
        let closes = Closes {
            runs: (0..detail.carried_runs.len())
                .map(|at| {
                    (
                        detail.carried_runs.id[at],
                        detail.carried_runs.last[at],
                        detail.carried_runs.closed[at],
                    )
                })
                .collect(),
            outputs: detail
                .carried_outputs
                .iter()
                .map(|output| (output.id, output.output.last_hour, output.closed))
                .collect(),
        };
        parts.push(encode_part(history, Part::Closes, 0, &closes)?);
    }
    if !events.versions.is_empty() {
        parts.push(encode_part(history, Part::Versions, 0, &events.versions)?);
    }
    if !events.countries.is_empty() {
        let rows = events
            .countries
            .iter()
            .map(|row| encode_row(TableRows::CountryProductionModifiers(vec![(**row).clone()])))
            .collect::<io::Result<Vec<_>>>()?;
        parts.push(encode_part(history, Part::Countries, 0, &rows)?);
    }
    for (&lifetime, modifiers) in &events.modifiers {
        parts.push(encode_part(
            history,
            Part::Modifiers,
            lifetime,
            &StoredModifiers::of(modifiers)?,
        )?);
    }
    Ok(parts)
}

/// Lays `parts` out as one chunk: the directory, sorted, then each payload.
pub(super) fn chunk(mut parts: Vec<Encoded>) -> io::Result<Vec<u8>> {
    parts.sort_by_key(|part| (part.history, part.part, part.key));
    let count = parts.len() as u64;
    let mut offset = HEADER
        .checked_add(
            count
                .checked_mul(ENTRY)
                .ok_or_else(|| crate::invalid("Production chunk directory overflow"))?,
        )
        .ok_or_else(|| crate::invalid("Production chunk directory overflow"))?;
    let payload: usize = parts.iter().map(|part| part.bytes.len()).sum();
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(offset as usize + payload)
        .map_err(io::Error::other)?;
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&count.to_le_bytes());
    for part in &parts {
        bytes.extend_from_slice(&part.history.to_le_bytes());
        bytes.extend_from_slice(&(part.part as u32).to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&part.key.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&(part.bytes.len() as u64).to_le_bytes());
        offset += part.bytes.len() as u64;
    }
    for part in parts {
        bytes.extend(part.bytes);
    }
    Ok(bytes)
}

/// Where one part of a chunk lies.
#[derive(Clone, Copy, Debug)]
pub(super) struct Slice {
    pub key: u64,
    pub offset: u64,
    pub len: u64,
}

/// A chunk opened for reading: its range and directory size.
#[derive(Clone, Copy, Debug)]
pub(super) struct Opened {
    pub chunk: u64,
    count: u64,
}

/// The chunk table's entries `(range, first hour, last hour)` as a page holds them.
pub(super) fn encode_chunks(chunks: &[(u64, i32, i32)]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(chunks.len() * CHUNK_ENTRY);
    for (range, first, last) in chunks {
        bytes.extend_from_slice(&range.to_le_bytes());
        bytes.extend_from_slice(&first.to_le_bytes());
        bytes.extend_from_slice(&last.to_le_bytes());
    }
    bytes
}

/// Unread spans as a page of `production-absence` holds them.
pub(super) fn encode_spans(spans: &[HourSpan]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(spans.len() * SPAN_ENTRY);
    for span in spans {
        bytes.extend_from_slice(&span.first_hour.to_le_bytes());
        bytes.extend_from_slice(&span.last_hour.to_le_bytes());
    }
    bytes
}

/// A full page of the absence's unread spans.
pub(super) fn decode_spans(bytes: &[u8]) -> io::Result<Vec<HourSpan>> {
    if bytes.len() != SPAN_PAGE * SPAN_ENTRY {
        return Err(crate::invalid("Invalid production absence page"));
    }
    Ok(bytes
        .chunks_exact(SPAN_ENTRY)
        .map(|entry| HourSpan {
            first_hour: i32::from_le_bytes(entry[..4].try_into().unwrap()),
            last_hour: i32::from_le_bytes(entry[4..].try_into().unwrap()),
        })
        .collect())
}

/// Whether `count` spans end with a tail of `tail` after whole pages, as the absence keeps them
/// once its full pages are written.
pub(super) fn spans_add_up(count: u64, tail: usize) -> bool {
    tail < SPAN_PAGE
        && count
            .checked_sub(tail as u64)
            .is_some_and(|paged| paged % SPAN_PAGE as u64 == 0)
}

/// A full page of the chunk table.
pub(super) fn decode_chunks(bytes: &[u8]) -> io::Result<Vec<(u64, i32, i32)>> {
    if bytes.len() != CHUNK_PAGE * CHUNK_ENTRY {
        return Err(crate::invalid("Invalid production chunk table page"));
    }
    Ok(bytes
        .chunks_exact(CHUNK_ENTRY)
        .map(|entry| {
            (
                u64::from_le_bytes(entry[..8].try_into().unwrap()),
                i32::from_le_bytes(entry[8..12].try_into().unwrap()),
                i32::from_le_bytes(entry[12..].try_into().unwrap()),
            )
        })
        .collect())
}

fn entry(bytes: &[u8; ENTRY as usize]) -> (u64, u32, u64, u64, u64) {
    let word = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    (
        word(0),
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        word(16),
        word(24),
        word(32),
    )
}

/// Opens a chunk whose directory lies whole in its range.
pub(super) fn open(pages: &mut dyn Pages, section: &str, chunk: u64) -> io::Result<Opened> {
    let mut header = [0_u8; HEADER as usize];
    pages.read_slice(section, chunk, 0, &mut header)?;
    if &header[..4] != MAGIC {
        return Err(crate::invalid("Invalid production index chunk"));
    }
    let count = u64::from_le_bytes(header[4..].try_into().unwrap());
    let end = count
        .checked_mul(ENTRY)
        .and_then(|bytes| bytes.checked_add(HEADER))
        .ok_or_else(|| crate::invalid("Production chunk directory overflows its range"))?;
    // An empty read at the directory's end fails unless the range reaches it.
    pages.read_slice(section, chunk, end, &mut [])?;
    Ok(Opened { chunk, count })
}

fn read_entry(
    pages: &mut dyn Pages,
    section: &str,
    opened: Opened,
    at: u64,
) -> io::Result<(u64, u32, u64, u64, u64)> {
    if at >= opened.count {
        return Err(crate::invalid(
            "Production chunk entry is past its directory",
        ));
    }
    let offset = at
        .checked_mul(ENTRY)
        .and_then(|offset| offset.checked_add(HEADER))
        .ok_or_else(|| crate::invalid("Production chunk entry overflows its range"))?;
    let mut bytes = [0_u8; ENTRY as usize];
    pages.read_slice(section, opened.chunk, offset, &mut bytes)?;
    Ok(entry(&bytes))
}

/// The first directory position at or after (history, part, key).
fn lower_bound(
    pages: &mut dyn Pages,
    section: &str,
    opened: Opened,
    target: (u64, u32, u64),
) -> io::Result<u64> {
    let (mut low, mut high) = (0, opened.count);
    while low < high {
        let middle = low + (high - low) / 2;
        let (history, part, key, _, _) = read_entry(pages, section, opened, middle)?;
        if (history, part, key) < target {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    Ok(low)
}

/// One history's part at `key`, if the chunk holds it.
pub(super) fn find(
    pages: &mut dyn Pages,
    section: &str,
    opened: Opened,
    history: u64,
    part: Part,
    key: u64,
) -> io::Result<Option<Slice>> {
    let target = (history, part as u32, key);
    let at = lower_bound(pages, section, opened, target)?;
    if at == opened.count {
        return Ok(None);
    }
    let (found, kind, found_key, offset, len) = read_entry(pages, section, opened, at)?;
    Ok(((found, kind, found_key) == target).then_some(Slice {
        key: found_key,
        offset,
        len,
    }))
}

/// Every part of one kind a history holds in the chunk, by key.
pub(super) fn find_all(
    pages: &mut dyn Pages,
    section: &str,
    opened: Opened,
    history: u64,
    part: Part,
) -> io::Result<Vec<Slice>> {
    let mut at = lower_bound(pages, section, opened, (history, part as u32, 0))?;
    let mut found = Vec::new();
    while at < opened.count {
        let (owner, kind, key, offset, len) = read_entry(pages, section, opened, at)?;
        if (owner, kind) != (history, part as u32) {
            break;
        }
        found.push(Slice { key, offset, len });
        at += 1;
    }
    Ok(found)
}

pub(super) fn read<T: product_codec::Product>(
    pages: &mut dyn Pages,
    section: &str,
    opened: Opened,
    slice: Slice,
) -> io::Result<T> {
    let len = usize::try_from(slice.len)
        .map_err(|_| crate::invalid("Production chunk part exceeds the address space"))?;
    if len == 0 {
        return Err(crate::invalid("Empty production chunk part"));
    }
    let end = slice
        .offset
        .checked_add(slice.len)
        .ok_or_else(|| crate::invalid("Production chunk part overflows its range"))?;
    pages.read_slice(section, opened.chunk, end, &mut [])?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).map_err(io::Error::other)?;
    bytes.resize(len, 0);
    pages.read_slice(section, opened.chunk, slice.offset, &mut bytes)?;
    match bytes[0] {
        0 => product_codec::decode(&bytes[1..]),
        1 => product_codec::decode(&crate::prepared::series::decompress(&bytes[1..])?),
        _ => Err(crate::invalid("Unknown production chunk part framing")),
    }
}

/// One history's part at `key` of a chunk, decoded, if the chunk holds it.
pub(super) fn part<T: product_codec::Product>(
    pages: &mut dyn Pages,
    section: &str,
    opened: Opened,
    history: u64,
    part: Part,
    key: u64,
) -> io::Result<Option<T>> {
    find(pages, section, opened, history, part, key)?
        .map(|slice| read(pages, section, opened, slice))
        .transpose()
}

/// A whole record of a stream that is not a chunk: framed as a chunk part is.
pub(super) fn pack(value: &impl product_codec::Product) -> io::Result<Vec<u8>> {
    Ok(encode_part(GLOBAL, Part::Variants, 0, value)?.bytes)
}

pub(super) fn unpack<T: product_codec::Product>(bytes: &[u8]) -> io::Result<T> {
    match bytes.first() {
        Some(0) => product_codec::decode(&bytes[1..]),
        Some(1) => product_codec::decode(&crate::prepared::series::decompress(&bytes[1..])?),
        _ => Err(crate::invalid("Unknown production record framing")),
    }
}

/// A store read as pages: ranges opened once per reader and sliced where they lie.
pub(super) struct StoreRanges<S> {
    store: S,
    key: store::SourceKey,
    opened: HashMap<(String, u64), store::Range>,
    /// The query session its reads share, while the reader lives.
    _scope: Option<store::Scope>,
}

impl<S: std::ops::Deref<Target = store::Store>> StoreRanges<S> {
    pub fn new(store: S, key: store::SourceKey) -> Self {
        Self {
            store,
            key,
            opened: HashMap::new(),
            _scope: None,
        }
    }

    /// [`Self::new`] reading through one store session until it drops.
    pub fn scoped(store: S, key: store::SourceKey) -> io::Result<Self> {
        let scope = store.scope(&key)?;
        Ok(Self {
            _scope: Some(scope),
            ..Self::new(store, key)
        })
    }

    fn range(&mut self, section: &str, range: u64) -> io::Result<Option<&mut store::Range>> {
        let key = (section.to_owned(), range);
        if !self.opened.contains_key(&key) {
            let Some(opened) = self.store.read(&self.key, section, range)? else {
                return Ok(None);
            };
            self.opened.insert(key.clone(), opened);
        }
        Ok(self.opened.get_mut(&key))
    }
}

impl<S: std::ops::Deref<Target = store::Store>> Pages for StoreRanges<S> {
    fn write_at(&mut self, _: &str, _: u64, _: &[u8]) -> io::Result<()> {
        Err(crate::invalid("Production query cannot write pages"))
    }
    fn write(&mut self, _: &str, _: &[u8]) -> io::Result<u64> {
        Err(crate::invalid("Production query cannot write pages"))
    }
    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = self.range(section, range)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }
    fn read_slice(
        &mut self,
        section: &str,
        range: u64,
        offset: u64,
        output: &mut [u8],
    ) -> io::Result<()> {
        let file = self
            .range(section, range)?
            .ok_or_else(|| crate::invalid("Missing production index chunk"))?;
        if offset
            .checked_add(output.len() as u64)
            .is_none_or(|end| end > file.len())
        {
            return Err(crate::invalid("Truncated production index chunk"));
        }
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(output)
    }
    fn range_ids(
        &mut self,
        section: &str,
        count: u64,
    ) -> io::Result<Option<Vec<Option<store::RangeId>>>> {
        (0..count)
            .map(|range| Ok(self.range(section, range)?.map(|file| file.id())))
            .collect::<io::Result<Vec<_>>>()
            .map(Some)
    }
}

product_codec::product! { impl Life {
    id: u64,
    handle: u64,
    appearance: u32,
    kind: Option<LineKind>,
    before_recording: bool,
    between_reads: Option<HourSpan>,
    variants: Vec<u64>,
} default {} }
product_codec::product! { impl Step {
    id: u64,
    lifetime: u64,
    first_hour: i32,
    last_hour: i32,
    closed: bool,
    starts_segment: bool,
    variant: u64,
    active_factories: i32,
    requested_factories: i32,
} default {} }
product_codec::product! { impl Counters {
    unknown_kinds: usize,
    unlisted_output: usize,
    unread_output_reads: usize,
    unreadable_stretches: usize,
    unlinked_rows: usize,
    refusals: BTreeMap<String, usize>,
    modified: bool,
} default {} }
product_codec::product! { impl Overview {
    lives: Vec<Life>,
    ends: Vec<(u64, i32)>,
    finals: Vec<(u64, RawOutput)>,
    steps: Vec<Step>,
    step_closes: Vec<(u64, i32)>,
    reads: Vec<i32>,
    line_hours: Vec<HourSpan>,
    output_hours: Vec<HourSpan>,
    organisation_hours: Vec<HourSpan>,
    unread_output: Vec<HourSpan>,
    wakes: Vec<Wake>,
    output_variants: BTreeSet<u64>,
    names: Vec<(u64, RawOrganisation)>,
    produced: Vec<(u64, i32, i32)>,
    counters: Counters,
} default {} }
product_codec::product! { impl Wake {
    at: u64,
    from: u64,
    to: u64,
    hour: i32,
} default {} }
product_codec::product! { impl Values {
    starts_segment: Vec<bool>,
    variant: Vec<u64>,
    mio: Vec<u64>,
    active_factories: Vec<i32>,
    requested_factories: Vec<i32>,
    max_factories: Vec<i32>,
    cost_raw: Vec<i64>,
    speed_raw: Vec<i64>,
    efficiency_raw: Vec<i64>,
    priority: Vec<i32>,
    amount: Vec<i32>,
} default {} }
product_codec::product! { impl Runs {
    id: Vec<u64>,
    lifetime: Vec<u64>,
    first: Vec<i32>,
    last: Vec<i32>,
    closed: Vec<bool>,
    values: Values,
} default {} }
product_codec::product! { impl OutputRecord {
    id: u64,
    lifetime: u64,
    output: RawOutput,
    closed: bool,
} default {} }
product_codec::product! { impl Detail {
    lives: BTreeMap<u64, (u64, u32)>,
    carried_runs: Runs,
    runs: Runs,
    carried_outputs: Vec<OutputRecord>,
    outputs: Vec<OutputRecord>,
} default {} }
product_codec::product! { impl Closes {
    runs: Vec<(u64, i32, bool)>,
    outputs: Vec<(u64, i32, bool)>,
} default {} }
product_codec::product! { impl Versions {
    designs: Vec<(u64, DesignRead)>,
    organisations: Vec<(u64, RawOrganisation)>,
    design_anchors: BTreeMap<u64, Option<u64>>,
    organisation_anchors: BTreeMap<u64, Option<u64>>,
} default {} }
product_codec::product! { impl Timeline {
    changes: Vec<(i32, u64, IconChoice)>,
    resets: Vec<i32>,
} default {} }
product_codec::product! { impl Start {
    lifetimes: Vec<(u64, u64, u32)>,
    runs: Vec<(u64, u64, i32, bool)>,
    steps: Vec<(u64, u64)>,
    outputs: Vec<(u64, u64)>,
    counters: Counters,
    appearances: Vec<(u64, i64)>,
} default {} }
product_codec::product! { impl Binding {
    lens: u64,
    map: u64,
    start: Option<Start>,
    included: Vec<u32>,
    named: Vec<u32>,
    design_anchors: BTreeMap<u64, Option<u64>>,
    organisation_anchors: BTreeMap<u64, Option<u64>>,
    appearances: Vec<(u64, i64)>,
} default {} }
product_codec::product! { impl Marks {
    designs: Vec<(u32, u32)>,
    organisations: Vec<(u32, u32)>,
    names: Vec<(u32, u32)>,
} default {} }
product_codec::product! { impl StoredModifiers {
    carried_last: Option<i32>,
    first: Vec<i32>,
    last: Vec<i32>,
    closed: Vec<bool>,
    country: Vec<u32>,
    line: Vec<Option<Vec<u8>>>,
    values: Values,
    organisation: Vec<Option<String>>,
} default {} }

#[cfg(test)]
mod tests {
    use super::*;

    struct Memory(BTreeMap<(String, u64), Vec<u8>>);
    impl Pages for Memory {
        fn write_at(&mut self, section: &str, range: u64, data: &[u8]) -> io::Result<()> {
            self.0.insert((section.into(), range), data.to_vec());
            Ok(())
        }
        fn write(&mut self, _: &str, _: &[u8]) -> io::Result<u64> {
            unreachable!()
        }
        fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
            Ok(self.0.get(&(section.into(), range)).cloned())
        }
    }

    #[test]
    fn a_chunk_directory_finds_every_part_and_nothing_else() {
        let mut overview = Overview::default();
        overview.reads = (0..400).collect();
        overview
            .counters
            .refusals
            .insert("synthetic refusal".into(), 3);
        let mut runs = Runs::default();
        let at = runs.push(
            9,
            4,
            &RawRun {
                first_hour: -3,
                last_hour: -3,
                variant: u64::MAX,
                cost_raw: i64::MIN,
                amount: -1,
                ..Default::default()
            },
        );
        runs.close(at, 17);
        let detail = Detail {
            lives: BTreeMap::from([(4, (u64::MAX, 2))]),
            runs,
            ..Default::default()
        };
        let parts = vec![
            encode_part(3, Part::Detail, 0, &detail).unwrap(),
            encode_part(3, Part::Overview, 0, &overview).unwrap(),
            encode_part(1, Part::Overview, 0, &Overview::default()).unwrap(),
            encode_part(3, Part::Modifiers, 8, &Overview::default()).unwrap(),
            encode_part(3, Part::Modifiers, 5, &Overview::default()).unwrap(),
        ];
        let mut pages = Memory(BTreeMap::new());
        pages.write_at("chunks", 0, &chunk(parts).unwrap()).unwrap();
        let opened = open(&mut pages, "chunks", 0).unwrap();
        let read: Overview = part(&mut pages, "chunks", opened, 3, Part::Overview, 0)
            .unwrap()
            .unwrap();
        assert_eq!(read.reads, overview.reads);
        assert_eq!(read.counters, overview.counters);
        let read: Detail = part(&mut pages, "chunks", opened, 3, Part::Detail, 0)
            .unwrap()
            .unwrap();
        assert_eq!(read.runs.run(0).last_hour, 17);
        assert!(read.runs.closed[0]);
        assert_eq!(read.runs.run(0).cost_raw, i64::MIN);
        assert_eq!(read.lives[&4], (u64::MAX, 2));
        assert!(
            part::<Overview>(&mut pages, "chunks", opened, 2, Part::Overview, 0)
                .unwrap()
                .is_none()
        );
        assert!(
            part::<Overview>(&mut pages, "chunks", opened, 3, Part::Versions, 0)
                .unwrap()
                .is_none()
        );
        let keys: Vec<_> = find_all(&mut pages, "chunks", opened, 3, Part::Modifiers)
            .unwrap()
            .into_iter()
            .map(|slice| slice.key)
            .collect();
        assert_eq!(keys, [5, 8]);
        assert!(
            find_all(&mut pages, "chunks", opened, 1, Part::Modifiers)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn malformed_directory_counts_and_part_lengths_are_refused_as_invalid_data() {
        let mut pages = Memory(BTreeMap::new());
        for count in [u64::MAX, u64::MAX / ENTRY, (u64::MAX - HEADER) / ENTRY, 2] {
            let mut bytes = MAGIC.to_vec();
            bytes.extend_from_slice(&count.to_le_bytes());
            bytes.extend_from_slice(&[0; ENTRY as usize]);
            pages.write_at("chunks", 0, &bytes).unwrap();
            let error = open(&mut pages, "chunks", 0).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{count}");
        }
        let parts = vec![encode_part(3, Part::Overview, 0, &Overview::default()).unwrap()];
        pages.write_at("chunks", 0, &chunk(parts).unwrap()).unwrap();
        let opened = open(&mut pages, "chunks", 0).unwrap();
        for (offset, len) in [(0, u64::MAX), (u64::MAX, 1), (HEADER + ENTRY, 1 << 40)] {
            let error = read::<Overview>(
                &mut pages,
                "chunks",
                opened,
                Slice {
                    key: 0,
                    offset,
                    len,
                },
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{offset} {len}");
        }
        let error = read_entry(&mut pages, "chunks", opened, opened.count).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            decode_chunks(&[0; CHUNK_ENTRY]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let table: Vec<(u64, i32, i32)> = (0..CHUNK_PAGE as u64)
            .map(|at| (at * 3, at as i32 * 24 - 5, at as i32 * 24 + 18))
            .collect();
        assert_eq!(decode_chunks(&encode_chunks(&table)).unwrap(), table);
    }

    #[test]
    fn small_parts_stay_raw_and_large_ones_are_compressed() {
        let small = encode_part(1, Part::Overview, 0, &Overview::default()).unwrap();
        assert_eq!(small.bytes[0], 0);
        let mut large = Overview::default();
        large.reads = vec![7; 10_000];
        let large = encode_part(1, Part::Overview, 0, &large).unwrap();
        assert_eq!(large.bytes[0], 1);
        assert!(large.bytes.len() < 10_000);
        let decoded: Overview = unpack(&large.bytes).unwrap();
        assert_eq!(decoded.reads, vec![7; 10_000]);
        assert!(unpack::<Overview>(&[2, 0]).is_err());
    }
}
