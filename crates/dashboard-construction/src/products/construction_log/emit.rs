//! Construction's emit: the claim algorithm ([`Scan`]) over a branch's projected hours
//! ([`observed::Hour`]), its projects and spending published on the family's series root.
//!
//! - A project's key holds its facts as events (its start, each run as it starts, its
//!   counts, ending, first repair advance and output reads) and its run's first hour as a run,
//!   held while the line is carried.
//! - A window's key lists, as events, every project with a row date in it or an output read
//!   or run carried through it.
//! - The output reads, each with why the table was unavailable, are events of one key.
//! - A spending leaf's key (a tag's actor) holds its total as a run, and its metadata: each
//!   final project's entries once, as it is forgotten, and at each cut where they changed the
//!   entries of the projects a later hour can still change.
//! - What every reply shares (the last reads, the types and states named, the unattributed
//!   output, the counts its warnings read) is an event of one key, at each cut where it changed.

use super::{
    ConstructionOutputRead, ConstructionProject, Counts, LevelsBuilt, ProjectEnd, ProjectKind,
    ProjectRun, ProjectSource, Scan, SpendingGap, observed,
};
use crate::products::construction_spending::{
    self as spending, LeafProject, SpendingLeaf, SpendingUnknown,
};
use dashboard_base::products::country_groups::HourSpan;
use dashboard_series::{
    Blob, Cell, Event, Evidence, KeyHandle, KeySpec, Knownness, RunMode, Span, Writer,
    family::{Actors, Directory, Place},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// The family's name.
pub const NAME: &str = "construction";
/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 2;
/// Project keys are spelled under it, so the family's directory never holds them.
pub const PROJECTS: u64 = 2;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// The family's measures: each project's facts (by project number), each window's members
/// (by window), the output reads, what the replies share, and a tag's spending leaves (by
/// leaf number).
pub const PROJECT: u64 = 0;
pub const WINDOW: u64 = 1;
pub const READS: u64 = 2;
pub const SHARED: u64 = 3;
pub const LEAF: u64 = 4;
/// Every run's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
/// A leaf's total is fixed5.
const DECIMALS: u8 = 5;

/// Event kinds.
pub mod kind {
    /// `[Text project]` (JSON, its lists empty), on a project's key at its first read.
    pub const START: u64 = 0;
    /// `[Integer first, last, factories, remaining, repair_levels]`, where a run starts.
    pub const PROGRESS: u64 = 1;
    /// `[Text counts]` (JSON: built, unaccounted, possible, added), where a read adds some.
    pub const COUNTS: u64 = 2;
    /// `[Text end]` (JSON), where its ending is known or changes.
    pub const END: u64 = 3;
    /// No fields: its first repair advance, at that read.
    pub const REPAIR: u64 = 4;
    /// `[since | Null, construction | Null, repair | Null, reason | Null, observed since
    /// creation, repair ticks]`, at each output read of it.
    pub const OUTPUT: u64 = 5;
    /// `[Unsigned project key]`, on a window's key.
    pub const MEMBER: u64 = 6;
    /// `[Unsigned reason | Null]`, on the reads key.
    pub const READ: u64 = 7;
    /// `[Text shared]` (JSON [`super::Shared`]), on the shared key.
    pub const SHARED: u64 = 8;
    /// `[Text leaf]` (JSON [`super::LeafRecord`]): the leaf and its open projects' entries.
    pub const LIVE: u64 = 9;
    /// `[Text entries]` (JSON [`super::LeafMetadata`]): a final project's entries.
    pub const FINAL: u64 = 10;
    pub const CENSUS: u64 = 11;
}

/// Every reason output can be unknown for, by code.
const GAPS: [SpendingGap; 8] = [
    SpendingGap::NotRecorded,
    SpendingGap::ReadFailed,
    SpendingGap::LinkedOutput,
    SpendingGap::InexactOutput,
    SpendingGap::ObservationRestart,
    SpendingGap::FinalOutputMissing,
    SpendingGap::OwnerUnknown,
    SpendingGap::Preparing,
];

pub(crate) fn gap_code(gap: SpendingGap) -> u64 {
    GAPS.iter().position(|&held| held == gap).unwrap_or(0) as u64
}

pub(crate) fn gap_of(code: u64) -> Option<SpendingGap> {
    usize::try_from(code)
        .ok()
        .and_then(|code| GAPS.get(code))
        .copied()
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// The window an hour falls in: the last starting at or before it (an hour before the first
/// falls in the first).
pub(crate) fn window_of(windows: &[HourSpan], hour: i32) -> u64 {
    windows
        .partition_point(|window| window.first_hour <= hour)
        .saturating_sub(1) as u64
}

/// What every reply shares, as the last cut left it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Shared {
    /// The last hour observed.
    pub last: Option<i32>,
    /// The building and railway queues' last two reads.
    pub reads: Vec<i32>,
    pub railway_reads: Vec<i32>,
    /// Every building type a project names, with its spelling, in naming order.
    pub types: Vec<(u32, Option<String>)>,
    /// Every state a project names.
    pub states: BTreeSet<u32>,
    pub unattributed: Vec<SpendingUnknown>,
    /// Each tag's projects whose work before observation began was not captured.
    pub before_first: BTreeMap<String, u32>,
    pub counts: Counts,
}

/// A leaf's projects and unknown stretches, by project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct LeafMetadata {
    #[serde(with = "dashboard_core::map_pairs")]
    projects: BTreeMap<(String, Option<i32>, i32), LeafProject>,
    #[serde(with = "dashboard_core::map_pairs")]
    unknown: BTreeMap<(String, i32), SpendingUnknown>,
}

impl LeafMetadata {
    fn is_empty(&self) -> bool {
        self.projects.is_empty() && self.unknown.is_empty()
    }

    /// A later exact read resolves `project`'s open unknown stretch at `hour`.
    fn resolve(&mut self, project: &str, hour: i32) {
        if let Some((_, gap)) = self
            .unknown
            .range_mut((project.to_owned(), i32::MIN)..=(project.to_owned(), i32::MAX))
            .next_back()
            && gap.until.is_none()
        {
            gap.until = Some(hour);
        }
    }

    /// `project`'s entries, taken out.
    fn take(&mut self, project: &str) -> Self {
        let projects = self
            .projects
            .range(
                (project.to_owned(), None, i32::MIN)
                    ..=(project.to_owned(), Some(i32::MAX), i32::MAX),
            )
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let unknown = self
            .unknown
            .range((project.to_owned(), i32::MIN)..=(project.to_owned(), i32::MAX))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        Self {
            projects: projects
                .into_iter()
                .filter_map(|key| self.projects.remove_entry(&key))
                .collect(),
            unknown: unknown
                .into_iter()
                .filter_map(|key| self.unknown.remove_entry(&key))
                .collect(),
        }
    }

    /// Takes in `other`'s entries.
    pub(crate) fn extend(&mut self, other: Self) {
        self.projects.extend(other.projects);
        self.unknown.extend(other.unknown);
    }

    /// The leaf `record` names, holding these entries, its spent steps empty.
    pub(crate) fn materialize(&self, record: &LeafRecord) -> SpendingLeaf {
        let mut projects: Vec<_> = self.projects.values().cloned().collect();
        let mut unknown: Vec<_> = self.unknown.values().cloned().collect();
        projects.sort_by(|a, b| {
            (a.first_read, a.until, &a.project).cmp(&(b.first_read, b.until, &b.project))
        });
        unknown.sort_by(|a, b| (a.known_at, &a.project).cmp(&(b.known_at, &b.project)));
        SpendingLeaf {
            repair: record.repair,
            kind: if record.railway {
                ProjectKind::Railway
            } else {
                ProjectKind::Building
            },
            building: record.building,
            state: record.state,
            to_state: record.to_state,
            spent: Vec::new(),
            unknown,
            projects,
        }
    }
}

/// Merges a leaf's projects and unknown stretches as a project's later reads give them.
fn merge_leaf(previous: &mut LeafMetadata, incoming: SpendingLeaf) {
    for span in incoming.projects {
        let lower = (span.project.clone(), span.known_at, i32::MIN);
        let upper = (span.project.clone(), span.known_at, span.first_read);
        let before = previous
            .projects
            .range_mut(lower..=upper)
            .next_back()
            .map(|(_, before)| before);
        if let Some(before) = before.filter(|before| {
            before.first_read == span.first_read
                || before.until.is_none()
                || before.until.is_some_and(|until| span.first_read < until)
        }) {
            let first = before.first_read;
            *before = span;
            before.first_read = first;
        } else {
            previous
                .projects
                .insert((span.project.clone(), span.known_at, span.first_read), span);
        }
    }
    for gap in incoming.unknown {
        if let Some((_, before)) = previous
            .unknown
            .range_mut((gap.project.clone(), i32::MIN)..=(gap.project.clone(), i32::MAX))
            .next_back()
            && before.until.is_none()
        {
            if before.reason == gap.reason {
                before.until = gap.until;
                continue;
            }
            before.until = Some(gap.known_at);
        }
        previous
            .unknown
            .insert((gap.project.clone(), gap.known_at), gap);
    }
}

/// A leaf as its live event names it: its kind of work, building type and states, and the
/// entries of its projects a later hour can still change.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct LeafRecord {
    pub repair: bool,
    pub railway: bool,
    pub building: u32,
    pub state: Option<u32>,
    pub to_state: Option<u32>,
    pub metadata: LeafMetadata,
}

/// A spending leaf: a tag's projects of one repair or new kind, building type and states.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct LeafId {
    tag: String,
    repair: bool,
    railway: bool,
    building: u32,
    state: Option<u32>,
    to_state: Option<u32>,
}

impl LeafId {
    /// `project`'s leaf of `repair` work.
    fn of(project: &ConstructionProject, repair: bool) -> Self {
        let railway = project.kind == ProjectKind::Railway;
        Self {
            tag: project.tag.clone(),
            repair,
            railway,
            building: project.building,
            state: project.state,
            to_state: project.to_state.filter(|_| railway),
        }
    }

    /// `leaf` of `tag`.
    fn named(tag: &str, leaf: &SpendingLeaf) -> Self {
        Self {
            tag: tag.to_owned(),
            repair: leaf.repair,
            railway: leaf.kind == ProjectKind::Railway,
            building: leaf.building,
            state: leaf.state,
            to_state: leaf.to_state,
        }
    }
}

/// A leaf as the emit holds it: its key, its total, whether a run of it was written, the
/// entries of its projects a later hour can still change, and each final project's open
/// unknown stretch, which a later line of its handle can still close.
#[derive(Serialize, Deserialize)]
struct Leaf {
    key: KeyHandle,
    total: i128,
    observed: bool,
    live: LeafMetadata,
    #[serde(with = "dashboard_core::map_pairs")]
    finals: BTreeMap<(u64, i32), SpendingGap>,
    /// Its entries changed since the last cut.
    dirty: bool,
}

impl Leaf {
    /// A final project's open stretches go on in the live entries once its handle is read
    /// again here, as merges and resolutions of one metadata made them.
    fn revive(&mut self, project: &str) {
        let Ok(handle) = project.parse::<u64>() else {
            return;
        };
        let held = self
            .live
            .unknown
            .range((project.to_owned(), i32::MIN)..=(project.to_owned(), i32::MAX))
            .next()
            .is_some();
        if held {
            return;
        }
        let revived: Vec<_> = self
            .finals
            .range((handle, i32::MIN)..=(handle, i32::MAX))
            .map(|(&key, &reason)| (key, reason))
            .collect();
        for ((_, known_at), reason) in revived {
            self.finals.remove(&(handle, known_at));
            self.live.unknown.insert(
                (project.to_owned(), known_at),
                SpendingUnknown {
                    known_at,
                    reason,
                    project: project.to_owned(),
                    until: None,
                },
            );
        }
    }
}

/// A project's open run: the run's first hour as its value from `first`, and the last hour a
/// cut appended of it.
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Open {
    first: i32,
    value: i128,
    break_before: bool,
    through: Option<i32>,
}

/// The open runs, by key: a project's while it is carried, a leaf's total while walked. Held
/// sparse, as only the live ones are: key handles grow with every project.
#[derive(Default, Serialize, Deserialize)]
struct Runs {
    #[serde(with = "dashboard_core::map_pairs")]
    open: BTreeMap<KeyHandle, Open>,
}

impl Runs {
    /// Appends the piece of `run` no cut appended, through `last`.
    fn append(writer: &mut Writer<'_>, key: KeyHandle, run: Open, last: i32) -> io::Result<()> {
        let first = match run.through {
            Some(through) => through.saturating_add(1),
            None => run.first,
        };
        if first <= last {
            writer.run(
                key,
                Span::new(first, last)?,
                run.value,
                run.break_before && run.through.is_none(),
                false,
            )?;
        }
        Ok(())
    }

    /// The value of `key` from `hour` on: None ends its open run before `hour`. An equal value
    /// going on without a break changes nothing.
    fn value(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        hour: i32,
        value: Option<i128>,
        break_before: bool,
    ) -> io::Result<()> {
        if let (Some(run), Some(value)) = (self.open.get(&key), value)
            && run.value == value
            && !break_before
        {
            return Ok(());
        }
        if let Some(run) = self.open.remove(&key)
            && let Some(last) = hour.checked_sub(1)
        {
            Self::append(writer, key, run, last)?;
        }
        if let Some(value) = value {
            self.open.insert(
                key,
                Open {
                    first: hour,
                    value,
                    break_before,
                    through: None,
                },
            );
        }
        Ok(())
    }

    /// Ends every open run after `last`.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        for (key, run) in std::mem::take(&mut self.open) {
            Self::append(writer, key, run, last)?;
        }
        Ok(())
    }

    /// Appends every open run through `last`; they stay open.
    fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        for (&key, run) in &mut self.open {
            Self::append(writer, key, *run, last)?;
            run.through = Some(last);
        }
        Ok(())
    }
}

/// What the emit holds of a project the scan holds: its key, its last run's first hour, its
/// last ending, its first repair advance, its last exact output read, and whether a read of
/// its output was seen.
#[derive(Default, Serialize, Deserialize)]
struct Slot {
    key: KeyHandle,
    source: Option<ProjectSource>,
    last_run: Option<i32>,
    last_end: Option<ProjectEnd>,
    first_repair: Option<i32>,
    last_output: Option<ConstructionOutputRead>,
    seen: bool,
}

/// What the emit goes on with besides the scan and its open edges.
#[derive(Default, Serialize, Deserialize)]
struct State {
    /// The number the next project's key takes.
    next_project: u64,
    /// By the scan's project index.
    slots: Vec<Slot>,
    #[serde(with = "dashboard_core::map_pairs")]
    leaves: BTreeMap<LeafId, Leaf>,
    before_first: BTreeMap<String, u32>,
    /// The projects whose run was carried or output read since the last window's end.
    touched: BTreeSet<KeyHandle>,
    /// The window whose end comes next.
    block: usize,
    /// Each output reason's final projects that had it.
    #[serde(with = "dashboard_core::map_pairs")]
    retired: BTreeMap<SpendingGap, usize>,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
    runs: Runs,
}

/// Construction's emit on one branch.
pub struct Emit {
    /// The branch's replay blocks: the log's windows.
    windows: Vec<HourSpan>,
    actors: Actors,
    scan: Scan,
    state: State,
    /// The shared event the last cut wrote, if this emit wrote one.
    shared: Option<Shared>,
}

/// Writes an event of `key` at `hour`.
fn event(
    writer: &mut Writer<'_>,
    key: KeyHandle,
    hour: i32,
    kind: u64,
    fields: Blob,
) -> io::Result<()> {
    let sequence = writer.sequence()?;
    writer.event(Event {
        key,
        native_hour: hour,
        read_hour: None,
        sequence,
        kind,
        fields,
    })
}

/// Writes `value`, as JSON text, as the one field of an event of `key` at `hour`.
fn json_event(
    writer: &mut Writer<'_>,
    key: KeyHandle,
    hour: i32,
    kind: u64,
    value: &impl Serialize,
) -> io::Result<()> {
    let text = writer.text(&serde_json::to_string(value)?)?;
    let fields = writer.cells([Cell::Text(text)])?;
    event(writer, key, hour, kind, fields)
}

/// The key of a family measure: one key, or one by `dimension`.
fn family_key(
    actors: &mut Actors,
    writer: &mut Writer<'_>,
    measure: u64,
    dimension: Option<u64>,
) -> io::Result<KeyHandle> {
    actors.key(
        writer,
        Place {
            actor: FAMILY,
            measure,
            dimension,
        },
        (RunMode::Exact, 0),
        None,
    )
}

/// Lists the project `key` among `window`'s members, at `hour`.
fn member(
    actors: &mut Actors,
    writer: &mut Writer<'_>,
    window: u64,
    hour: i32,
    key: KeyHandle,
) -> io::Result<()> {
    let at = family_key(actors, writer, WINDOW, Some(window))?;
    let fields = writer.cells([Cell::Unsigned(key)])?;
    event(writer, at, hour, kind::MEMBER, fields)
}

/// A fixed5 amount held as a decimal string.
fn amount(raw: &str) -> io::Result<i128> {
    raw.parse()
        .map_err(|_| invalid("Malformed construction IC"))
}

/// An output read's fields.
fn output_cells(writer: &mut Writer<'_>, read: &ConstructionOutputRead) -> io::Result<Blob> {
    let raw = |raw: &Option<String>| -> io::Result<Cell> {
        Ok(match raw {
            Some(raw) => Cell::Integer(amount(raw)?),
            None => Cell::Null,
        })
    };
    let cells = [
        read.since_hour
            .map_or(Cell::Null, |hour| Cell::Integer(i128::from(hour))),
        raw(&read.construction_raw)?,
        raw(&read.repair_raw)?,
        read.reason
            .map_or(Cell::Null, |reason| Cell::Unsigned(gap_code(reason))),
        Cell::Boolean(read.observed_since_creation),
        Cell::Integer(i128::from(read.repair_ticks)),
    ];
    writer.cells(cells)
}

impl Emit {
    /// The emit of a branch whose replay blocks are `windows`.
    pub fn new(windows: Vec<HourSpan>) -> Self {
        Self {
            windows,
            actors: Actors::new(SCHEMA),
            scan: Scan::default(),
            state: State::default(),
            shared: None,
        }
    }

    /// A new project's key.
    fn project_key(&mut self, writer: &mut Writer<'_>) -> io::Result<KeyHandle> {
        let number = self.state.next_project;
        self.state.next_project += 1;
        let bytes = number.to_le_bytes();
        let dimensions = writer.blob(&mut &bytes[..], 8)?;
        let key = writer.intern(
            KeySpec {
                schema: PROJECTS,
                actor: FAMILY,
                measure: PROJECT,
                dimensions,
            },
            Blob::EMPTY,
            RunMode::Exact,
            0,
        )?;
        writer.bind_domain(key, DOMAIN)?;
        Ok(key)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: observed::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("construction.emit", None);
        let at = hour.hour;
        let adjacent = match self.state.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(invalid(
                    "A construction hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run, and the hours known end before it.
            Some(last) => {
                self.state.runs.close(writer, last)?;
                self.known(writer, last)?;
                false
            }
            None => false,
        };
        self.pass(writer, at)?;
        self.state.last = Some(at);
        self.state.first.get_or_insert(at);
        if !adjacent {
            // Each leaf's spending goes on from its total after a break.
            for leaf in self.state.leaves.values() {
                self.state
                    .runs
                    .value(writer, leaf.key, at, Some(leaf.total), true)?;
            }
        }
        self.scan.observe_changed(&hour);
        if hour.queue.read() || hour.railways.read() {
            self.scan.sync_reads();
        }
        self.project(writer, at)?;
        if self
            .windows
            .get(self.state.block)
            .is_some_and(|window| window.last_hour == at)
        {
            self.end_block(writer, at)?;
            self.state.block += 1;
        }
        Ok(())
    }

    /// Ends each window whose last hour was not walked, before `at`: what it carried is listed
    /// at `at`.
    fn pass(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        while self
            .windows
            .get(self.state.block)
            .is_some_and(|window| window.last_hour < at)
        {
            let window = self.state.block as u64;
            for key in std::mem::take(&mut self.state.touched) {
                member(&mut self.actors, writer, window, at, key)?;
            }
            self.state.block += 1;
        }
        Ok(())
    }

    /// A window's end at its last hour: its carried runs reach it, and every project carried
    /// or read in it is its member.
    fn end_block(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.scan.seal();
        if let Some(hour) = self.scan.last {
            self.project(writer, hour)?;
        }
        let window = window_of(&self.windows, last);
        for key in std::mem::take(&mut self.state.touched) {
            member(&mut self.actors, writer, window, last, key)?;
        }
        Ok(())
    }

    /// The branch ends at `last`, the last hour walked: the window it falls in ends there.
    pub fn end(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        if self.state.last == Some(last) && self.state.block < self.windows.len() {
            self.end_block(writer, last)?;
            self.state.block += 1;
        }
        Ok(())
    }

    /// Publishes what the scan changed of its projects at `hour`.
    fn project(&mut self, writer: &mut Writer<'_>, hour: i32) -> io::Result<()> {
        let dirty = std::mem::take(&mut self.scan.dirty);
        if dirty.is_empty() && self.scan.log.output_reads.is_empty() {
            return Ok(());
        }
        for &index in &dirty {
            if index >= self.state.slots.len() {
                let key = self.project_key(writer)?;
                self.state.slots.push(Slot {
                    key,
                    ..Slot::default()
                });
            }
        }
        // Each project's output at this hour as steps from its last exact read.
        let mut steps: BTreeMap<usize, Vec<ConstructionOutputRead>> = BTreeMap::new();
        for &index in &dirty {
            let mut output: Vec<_> = self.scan.log.outputs[index]
                .iter()
                .filter(|read| read.known_at == hour)
                .cloned()
                .collect();
            for read in &mut output {
                let id = self.scan.log.projects[index]
                    .id
                    .parse::<u64>()
                    .expect("native construction handle");
                let baseline = self
                    .scan
                    .log
                    .output_baselines
                    .get(&id)
                    .is_some_and(|reads| reads.contains(&hour));
                let slot = &mut self.state.slots[index];
                // A read without a row cannot say whether earlier work went unobserved.
                if read.since_hour.is_some()
                    && !std::mem::replace(&mut slot.seen, true)
                    && !read.observed_since_creation
                {
                    let tag = &self.scan.log.projects[index].tag;
                    *self.state.before_first.entry(tag.clone()).or_default() += 1;
                }
                let before = self.state.slots[index].last_output.as_ref();
                for (raw, previous) in [
                    (
                        &mut read.construction_raw,
                        before.and_then(|read| read.construction_raw.as_deref()),
                    ),
                    (
                        &mut read.repair_raw,
                        before.and_then(|read| read.repair_raw.as_deref()),
                    ),
                ] {
                    if let Some(value) = raw {
                        let step = if baseline {
                            0
                        } else {
                            amount(value)? - previous.map_or(Ok(0), amount)?
                        };
                        *value = step.to_string();
                    }
                }
                read.repair_ticks = i32::from(
                    !baseline && read.repair_ticks > before.map_or(0, |read| read.repair_ticks),
                );
            }
            steps.insert(index, output);
        }
        let grouped = spending::all_leaves_for(
            &self.scan.log,
            dirty
                .iter()
                .map(|&index| (&self.scan.log.projects[index], steps[&index].as_slice())),
        );
        for (tag, (leaves, _)) in grouped {
            for mut leaf in leaves {
                let id = LeafId::named(&tag, &leaf);
                if !self.state.leaves.contains_key(&id) {
                    let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
                    let place = Place {
                        actor,
                        measure: LEAF,
                        dimension: Some(self.state.leaves.len() as u64),
                    };
                    let key =
                        self.actors
                            .key(writer, place, (RunMode::Exact, DECIMALS), Some(DOMAIN))?;
                    self.state.leaves.insert(
                        id.clone(),
                        Leaf {
                            key,
                            total: 0,
                            observed: false,
                            live: LeafMetadata::default(),
                            finals: BTreeMap::new(),
                            dirty: true,
                        },
                    );
                }
                let held = self
                    .state
                    .leaves
                    .get_mut(&id)
                    .ok_or_else(|| invalid("A construction leaf is not held"))?;
                let previous = held.total;
                if let Some(step) = leaf.spent.last() {
                    held.total = held
                        .total
                        .checked_add(amount(&step.total_raw)?)
                        .ok_or_else(|| invalid("Construction IC exceeds its exact range"))?;
                }
                if previous != held.total || !held.observed {
                    self.state
                        .runs
                        .value(writer, held.key, hour, Some(held.total), false)?;
                    held.observed = true;
                }
                leaf.spent.clear();
                for gap in &leaf.unknown {
                    held.revive(&gap.project);
                }
                merge_leaf(&mut held.live, leaf);
                held.dirty = true;
            }
        }
        // Only this project's two leaves can have an unresolved output interval.
        for &index in &dirty {
            let project = &self.scan.log.projects[index];
            if steps[&index]
                .iter()
                .any(|read| read.construction_raw.is_some())
            {
                for repair in [false, true] {
                    if let Some(leaf) = self.state.leaves.get_mut(&LeafId::of(project, repair)) {
                        leaf.revive(&project.id);
                        leaf.live.resolve(&project.id, hour);
                        leaf.dirty = true;
                    }
                }
            }
        }
        let windows = &self.windows;
        for index in dirty {
            let project = &mut self.scan.log.projects[index];
            let slot = &mut self.state.slots[index];
            let key = slot.key;
            let mut dates = Vec::new();
            let mut windowed = BTreeSet::new();
            if slot.source != Some(project.source) {
                let mut start = project.clone();
                start.built.clear();
                start.unaccounted.clear();
                start.possible.clear();
                start.added.clear();
                start.runs.clear();
                start.end = None;
                start.first_repair_tick = None;
                json_event(
                    writer,
                    key,
                    hour,
                    if slot.source.is_none() {
                        kind::START
                    } else {
                        kind::CENSUS
                    },
                    &start,
                )?;
                slot.source = Some(project.source);
                dates.extend([
                    project.first_read,
                    project.created_hour.unwrap_or(project.first_read),
                ]);
                // A line created after the read that first lists it shows from that read
                // dated later: every window between holds it.
                if let Some(created) = project
                    .created_hour
                    .filter(|&created| created > project.first_read)
                {
                    windowed.extend(
                        window_of(windows, project.first_read)..=window_of(windows, created),
                    );
                }
            }
            if let Some(run) = project.runs.last() {
                let changed = slot.last_run != Some(run.first_hour);
                if changed {
                    let fields = writer.cells(
                        [
                            run.first_hour,
                            run.last_hour,
                            run.factories,
                            run.remaining,
                            run.repair_levels,
                        ]
                        .map(|value| Cell::Integer(i128::from(value))),
                    )?;
                    event(writer, key, hour, kind::PROGRESS, fields)?;
                    slot.last_run = Some(run.first_hour);
                }
                if changed || run.last_hour != hour {
                    self.state.runs.value(
                        writer,
                        key,
                        hour,
                        (run.last_hour == hour).then_some(i128::from(run.first_hour)),
                        run.first_hour == hour,
                    )?;
                }
                if run.last_hour == hour {
                    self.state.touched.insert(key);
                }
            }
            for values in [&project.built, &project.unaccounted, &project.possible] {
                dates.extend(
                    values
                        .iter()
                        .map(|value| value.hour.unwrap_or(value.known_at)),
                );
            }
            if !project.built.is_empty()
                || !project.unaccounted.is_empty()
                || !project.possible.is_empty()
                || !project.added.is_empty()
            {
                json_event(
                    writer,
                    key,
                    hour,
                    kind::COUNTS,
                    &Counted {
                        built: &project.built,
                        unaccounted: &project.unaccounted,
                        possible: &project.possible,
                        added: &project.added,
                    },
                )?;
            }
            if project.end != slot.last_end {
                if let Some(end) = &project.end {
                    json_event(writer, key, hour, kind::END, end)?;
                    dates.extend([end.known_at, end.hour.unwrap_or(end.known_at)]);
                }
                slot.last_end = project.end.clone();
            }
            if project.first_repair_tick != slot.first_repair {
                if let Some(at) = project.first_repair_tick {
                    event(writer, key, at, kind::REPAIR, Blob::EMPTY)?;
                }
                slot.first_repair = project.first_repair_tick;
            }
            windowed.extend(dates.into_iter().map(|date| window_of(windows, date)));
            for window in windowed {
                member(&mut self.actors, writer, window, hour, key)?;
            }
            let output = &mut self.scan.log.outputs[index];
            for read in output.iter().filter(|read| read.known_at == hour) {
                let fields = output_cells(writer, read)?;
                event(writer, key, hour, kind::OUTPUT, fields)?;
                self.state.touched.insert(key);
                if read.construction_raw.is_some() {
                    slot.last_output = Some(read.clone());
                }
            }
            output.clear();
            project.built.clear();
            project.unaccounted.clear();
            project.possible.clear();
            project.added.clear();
            if project.runs.len() > 2 {
                project.runs.drain(..project.runs.len() - 2);
            }
        }
        if self.scan.log.output_reads.iter().any(|&(at, _)| at == hour) {
            let reads = family_key(&mut self.actors, writer, READS, None)?;
            for &(at, reason) in self
                .scan
                .log
                .output_reads
                .iter()
                .filter(|(at, _)| *at == hour)
            {
                let fields = writer.cells([
                    reason.map_or(Cell::Null, |reason| Cell::Unsigned(gap_code(reason)))
                ])?;
                event(writer, reads, at, kind::READ, fields)?;
            }
        }
        self.scan.log.output_reads.clear();
        for values in [
            &mut self.scan.buildings.reads,
            &mut self.scan.railways.reads,
            &mut self.scan.log.reads,
            &mut self.scan.log.railway_reads,
        ] {
            if values.len() > 2 {
                values.drain(..values.len() - 2);
            }
        }
        self.scan.log.repair_predecessors.clear();
        Ok(())
    }

    /// What every reply shares now.
    fn shared(&self) -> Shared {
        let log = &self.scan.log;
        let mut counts = Counts::of(log);
        let mut reasons = self.state.retired.clone();
        for &(reason, count) in &counts.reasons {
            *reasons.entry(reason).or_default() += count;
        }
        counts.reasons = reasons.into_iter().collect();
        Shared {
            last: self.scan.last,
            reads: log.reads.clone(),
            railway_reads: log.railway_reads.clone(),
            types: log.types.clone(),
            states: log.states.clone(),
            unattributed: log
                .output_unknown
                .iter()
                .map(|gap| SpendingUnknown {
                    known_at: gap.known_at,
                    reason: gap.reason,
                    project: gap.line.map_or_else(String::new, |id| id.to_string()),
                    until: gap.until,
                })
                .collect(),
            before_first: self.state.before_first.clone(),
            counts,
        }
    }

    /// Writes each final project's leaf entries and forgets it.
    fn retire(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        let kept = self.scan.kept();
        for (index, project) in self.scan.log.projects.iter().enumerate() {
            if kept.contains(&index) {
                continue;
            }
            for repair in [false, true] {
                let Some(leaf) = self.state.leaves.get_mut(&LeafId::of(project, repair)) else {
                    continue;
                };
                let entries = leaf.live.take(&project.id);
                if !entries.is_empty() {
                    json_event(writer, leaf.key, last, kind::FINAL, &entries)?;
                    leaf.dirty = true;
                }
                // A later line of its handle can still close what it left open.
                if let Ok(handle) = project.id.parse::<u64>() {
                    for gap in entries.unknown.values().filter(|gap| gap.until.is_none()) {
                        leaf.finals.insert((handle, gap.known_at), gap.reason);
                    }
                }
            }
            // Its reasons are final: only their count is kept.
            for reason in GAPS {
                if self
                    .scan
                    .log
                    .output_reasons
                    .remove(&(reason, project.id.clone()))
                {
                    *self.state.retired.entry(reason).or_default() += 1;
                }
            }
        }
        let remap = self.scan.compact_projects();
        self.state.slots = std::mem::take(&mut self.state.slots)
            .into_iter()
            .enumerate()
            .filter_map(|(index, slot)| remap.contains_key(&index).then_some(slot))
            .collect();
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next: every final project's entries, each changed leaf, what the replies share where it
    /// changed, and the hours since the last cut as known.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.retire(writer, last)?;
        for (id, leaf) in &mut self.state.leaves {
            if !std::mem::take(&mut leaf.dirty) {
                continue;
            }
            let record = LeafRecord {
                repair: id.repair,
                railway: id.railway,
                building: id.building,
                state: id.state,
                to_state: id.to_state,
                metadata: leaf.live.clone(),
            };
            json_event(writer, leaf.key, last, kind::LIVE, &record)?;
        }
        let shared = self.shared();
        if self.shared.as_ref() != Some(&shared) {
            let key = family_key(&mut self.actors, writer, SHARED, None)?;
            json_event(writer, key, last, kind::SHARED, &shared)?;
            self.shared = Some(shared);
        }
        self.state.runs.cut(writer, last)?;
        self.known(writer, last)
    }

    /// The hours walked one after another since the last cut or break, through `last`, as known.
    fn known(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        if let Some(first) = self.state.first.take() {
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
        Ok(())
    }

    /// What the emit goes on with after a cut: the scan, and what it holds of each project and
    /// leaf with their open runs; never what it published.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        serde_json::to_writer(out, &(&self.scan, &self.state))?;
        Ok(())
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads, of a
    /// branch whose replay blocks are `windows`.
    pub fn load(windows: Vec<HourSpan>, directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let (scan, state): (Scan, State) = serde_json::from_slice(bytes)?;
        Ok(Self {
            windows,
            actors: Actors::resume(SCHEMA, directory)?,
            scan,
            state,
            shared: None,
        })
    }
}

/// A read's counts, as a counts event holds them.
#[derive(Serialize)]
struct Counted<'a> {
    built: &'a [LevelsBuilt],
    unaccounted: &'a [LevelsBuilt],
    possible: &'a [LevelsBuilt],
    added: &'a [LevelsBuilt],
}

/// A read's counts, as a counts event is read back.
#[derive(Deserialize)]
pub(crate) struct ReadCounts {
    pub built: Vec<LevelsBuilt>,
    pub unaccounted: Vec<LevelsBuilt>,
    pub possible: Vec<LevelsBuilt>,
    pub added: Vec<LevelsBuilt>,
}

/// A run as its progress event holds it.
pub(crate) fn run_of(values: [i128; 5]) -> io::Result<ProjectRun> {
    let at =
        |value: i128| i32::try_from(value).map_err(|_| invalid("A construction run is not valid"));
    Ok(ProjectRun {
        first_hour: at(values[0])?,
        last_hour: at(values[1])?,
        factories: at(values[2])?,
        remaining: at(values[3])?,
        repair_levels: at(values[4])?,
    })
}

#[cfg(test)]
pub(crate) mod tests;
