//! Unit leaders prepared on their own, block by block. A pre-pass worker
//! reads a block alone, only at the hours a field leaders read changes
//! ([`FAMILY`]'s fields): every field the scan reads of its own tables (all
//! of a leader but its retired experience; the combat details' sides, whose
//! commanders it reads; a formation's leader, name and kind, a mission's
//! leader and a task force's fleet; the countries' slots and tags), and of
//! the shared trigger tables the fields their declared reads read
//! (`triggers::inputs::Key::dependence`), the row keys of the others.
//!
//! A run binds what its blocks are read with ([`bound`]): the scan's own
//! tables, the trigger world's core and, where content binds, the shared
//! trigger tables the declared reads read; not the rest of the trigger kit,
//! which only an evaluation the hour's check refuses reads. Every table the
//! read leaves out is taken as changed anywhere at every hour, and as
//! differing from the hour's whole frame.
//!
//! At each hour it reads, the worker takes what changed since the hour
//! before from the block reader (`BlockChange::Changed`: each watched table
//! whose watched fields, row keys or availability changed), or finds it so
//! from the hour before's rows where its read watches more than its fields,
//! and keeps it as the hour's change facts (`BlockTables`); a combat whose
//! commanders did not change is not a change of what leaders read, although
//! its other operands move every hour. An hour at which nothing leaders read
//! changed, and whose reads are the hour before's, joins the quiet hours,
//! whose change facts are the reader's (`QuietTables`). It keeps the block's
//! first hour (and one after a gap) as read, every other hour it keeps
//! reduced: the hourly-moving tables only by the fields leaders and the
//! declared reads read (a table's reduced rows made again only where its
//! watched fields changed since they were made), and the tables no declared
//! read reads not at all. A quiet hour whose reads differ from the hour
//! before's is kept as the reads alone (its metrics and sample hours), on the
//! last kept hour's rows.
//!
//! The family folds each block in order on the carried leaders: each hour
//! kept as read as hourly preparation observes it, or, where the read left
//! tables out, through the same step as a kept hour; each kept hour through
//! the same step on its kept frame ([`Prepared::observe_kept`]), with the
//! hour's change facts, so a visibility verdict is reused while its inputs'
//! declared dependence did not change, read again on the kept frame only
//! where its inputs read there what the whole frame holds, and evaluated
//! there only where everything the evaluation read is the whole frame's;
//! each quiet span as the carried leaders' unchanged hours
//! ([`Prepared::advance_quiet`]). Where an hour cannot give its verdicts so,
//! or a quiet span would not carry, the rest of the block is observed hour
//! by hour, on its whole frames read with the whole trigger kit, as hourly
//! preparation observes it.

use super::{
    Commanders, Content, battle_tables, experience_field, land_commanders, naval_commanders,
    prepared::Prepared, read_mask,
};
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::prepared::kept::{note_frames, note_observed};
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    kept::{Observed, consecutive, kept_changes, observe_hours},
    prepass::{Bound, Extract, Family, Folded},
};
use dashboard_text::triggers::inputs::{BlockTables, QuietTables, TableChange, TableChanges};
use replay::{
    BranchId,
    container::{BlockIndex, QuietHours, Replay, TableFields},
    format::FormatVersion,
    frame::{DecodedFrame, Metrics, SharedView, TableView},
    rows::{
        CountryCounterInputRow, CountryRow, LandCombatDetailsRow, LandCombatSideDetails,
        NavalCombatDetailsRow, NavalCombatSideDetails, NavalMissionTaskForceRow, ProvinceRow,
        RowCodec, StateRow, TableId, TableRows, TableSchema, TaskForceRow, UnitLeaderRow,
        WarRelationRow, full_mask, tables,
    },
};
use std::{
    any::Any, cell::Cell, collections::BTreeMap, io, marker::PhantomData, ops::Range,
    sync::LazyLock,
};

/// What the family watches of a table.
#[derive(Clone, Copy)]
enum Watch {
    /// Every field.
    All,
    /// Its row keys and availability.
    Keys,
    /// These fields.
    Named(&'static [&'static str]),
    /// Every field but the leaders' retired experience.
    Leaders,
}

/// Which runs read a table ([`bound`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Every run: the scan's own tables and the trigger world's core.
    Every,
    /// A run whose content binds, whose leaders' visibility evaluations
    /// read it.
    Content,
}

/// How a kept frame keeps a table.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keep {
    /// As read.
    Whole,
    /// Its rows with only the fields leaders and the declared reads read.
    Reduced,
    /// Not at all: no declared read reads it, so nothing on a kept frame may.
    Dropped,
}

/// The family's tables that are not watched by their keys and dropped from
/// kept frames: the scan's own, and the shared trigger tables a declared
/// read reads (`triggers::world::dependence`, the scope queries' and the
/// trigger world's own reads), with the fields those reads read, and which
/// runs read each. A table a declared read reads more of than is watched is
/// only read again on whole frames, and one a run does not read only on the
/// whole trigger kit's frames, so this list decides how often hours are
/// quiet and blocks fall back, never what is published.
const POLICY: &[(TableId, Watch, Keep, Reach)] = &[
    // The scan's own tables.
    (
        TableId::UnitLeaders,
        Watch::Leaders,
        Keep::Reduced,
        Reach::Every,
    ),
    (TableId::Characters, Watch::All, Keep::Whole, Reach::Every),
    (
        TableId::CharacterPortraits,
        Watch::All,
        Keep::Whole,
        Reach::Every,
    ),
    (
        TableId::LexicalTokens,
        Watch::All,
        Keep::Whole,
        Reach::Every,
    ),
    (
        TableId::UnitLeaderTraits,
        Watch::All,
        Keep::Whole,
        Reach::Every,
    ),
    (
        TableId::LandCombatDetails,
        Watch::Named(&["sides"]),
        Keep::Reduced,
        Reach::Every,
    ),
    (
        TableId::NavalCombatDetails,
        Watch::Named(&["sides"]),
        Keep::Reduced,
        Reach::Every,
    ),
    (
        TableId::LandCombatResults,
        Watch::All,
        Keep::Whole,
        Reach::Every,
    ),
    (
        TableId::NavalResultSources,
        Watch::All,
        Keep::Whole,
        Reach::Every,
    ),
    (TableId::NavalResults, Watch::All, Keep::Whole, Reach::Every),
    (
        TableId::NavalCombatResultDetails,
        Watch::All,
        Keep::Whole,
        Reach::Every,
    ),
    (
        TableId::OrdersGroups,
        Watch::Named(&["leader", "name", "is_army_group"]),
        Keep::Whole,
        Reach::Every,
    ),
    (
        TableId::OrdersGroupPlans,
        Watch::All,
        Keep::Whole,
        Reach::Every,
    ),
    (TableId::Fleets, Watch::All, Keep::Whole, Reach::Every),
    (
        TableId::NavalMissionTaskForces,
        Watch::Named(&["leader"]),
        Keep::Reduced,
        Reach::Every,
    ),
    (
        TableId::TaskForces,
        Watch::Named(&["fleet"]),
        Keep::Reduced,
        Reach::Every,
    ),
    (
        TableId::Countries,
        Watch::Named(&["canonical_slot", "tag"]),
        Keep::Reduced,
        Reach::Every,
    ),
    (TableId::CountryLinks, Watch::All, Keep::Whole, Reach::Every),
    // The shared trigger tables declared reads read.
    (
        TableId::Clock,
        Watch::Named(&["seed"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::PoliticalParties,
        Watch::Named(&["is_ruling", "token"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::Politics,
        Watch::Named(&["ruling_party", "has_elections"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::ReservedCountryNaming,
        Watch::Named(&[
            "faction",
            "original_tag_reference",
            "owned_state_count",
            "original_tag_buckets",
        ]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::CountryCounterInputs,
        Watch::Named(&["owned_state_count"]),
        Keep::Reduced,
        Reach::Content,
    ),
    (
        TableId::FactionMembers,
        Watch::Named(&["faction"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::WarRelations,
        Watch::Named(&["start_hour", "end_hour"]),
        Keep::Reduced,
        Reach::Content,
    ),
    (
        TableId::Subjects,
        Watch::Named(&["start_hour", "end_hour"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::Focus,
        Watch::Named(&["tree"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::CountryAppearance,
        Watch::Named(&["original_tag_reference"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::CompletedFocuses,
        Watch::Named(&["token"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::Projects,
        Watch::Named(&["definition_id", "completed"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::ScriptFlags,
        Watch::Named(&["value"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::ScriptVariables,
        Watch::Named(&["entries", "seed"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::PartyLeaders,
        Watch::Named(&["character", "leader_id", "role"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::CountryCharacters,
        Watch::Named(&["character"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::CountryCivilWars,
        Watch::Named(&["target"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::AdvisorTokens,
        Watch::Named(&["token"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::FactionAdvisors,
        Watch::Named(&["token"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::HiredAdvisors,
        Watch::Named(&["idea"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::ActiveIdeas,
        Watch::Named(&["token"]),
        Keep::Whole,
        Reach::Content,
    ),
    (
        TableId::CountryWars,
        Watch::All,
        Keep::Whole,
        Reach::Content,
    ),
    // The trigger world's core tables: their keys and availability.
    (TableId::Provinces, Watch::Keys, Keep::Reduced, Reach::Every),
    (TableId::States, Watch::Keys, Keep::Reduced, Reach::Every),
];

/// How the family watches and keeps `table`.
fn policy(table: TableId) -> (Watch, Keep) {
    if table == TableId::Campaign {
        return (Watch::Keys, Keep::Whole);
    }
    POLICY
        .iter()
        .find(|(listed, ..)| *listed == table)
        .map_or((Watch::Keys, Keep::Dropped), |&(_, watch, keep, _)| {
            (watch, keep)
        })
}

/// The tables leaders read: the scan's own and the whole shared trigger kit
/// its visibility evaluations read (`triggers::RECORDED_TABLES`), in
/// `TableId` order.
pub fn tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables: Vec<TableId> = POLICY.iter().map(|(table, ..)| *table).collect();
        tables.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
        tables.push(TableId::Campaign);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES
}

pub static FAMILY: LazyLock<Family> = LazyLock::new(|| Family {
    name: "leaders",
    extract: "leaders.extract",
    sequential: "leaders.sequential",
    fallback: "leaders.fallback",
    tables: tables(),
    fields: Some(fields),
    start: |format| start(format, whole()),
});

/// Count a rejected fold proof only while measuring; tests keep their own
/// thread's counts without enabling process-wide CPU accounting.
pub(super) fn rejected(reason: &'static str) {
    #[cfg(any(test, feature = "test-support"))]
    dashboard_base::prepared::prepass::note(reason);
    if replay::cpu::enabled() {
        replay::cpu::note(reason, FAMILY.name, replay::cpu::thread_cycles());
    }
}

/// What a run's block reads hold.
struct Selection {
    /// The tables read, in `TableId` order, the campaign among them.
    tables: Vec<TableId>,
    /// The fields watched of each but the campaign, as the block reader
    /// reads them; None when the schema no longer names one (its blocks are
    /// then read whole).
    fields: Option<Vec<TableFields>>,
    /// Those fields as its change facts read them: every field of a table
    /// whose watched fields the schema no longer names.
    watch: Vec<TableFields>,
    /// Every other table leaders may read ([`tables`]), changed anywhere
    /// at every hour and differing anywhere from the hour's whole frame.
    unread: Changes,
}

impl Selection {
    fn of(read: Vec<TableId>) -> Self {
        let fields: io::Result<Vec<TableFields>> = read
            .iter()
            .filter(|&&table| table != TableId::Campaign)
            .map(|&table| watched(table))
            .collect();
        let watch = match &fields {
            Ok(fields) => fields.clone(),
            Err(_) => read
                .iter()
                .filter(|&&table| table != TableId::Campaign)
                .map(|&table| TableFields::all(table))
                .collect(),
        };
        let unread = Changes(
            tables()
                .iter()
                .filter(|table| !read.contains(table))
                .map(|&table| (table, Change::Any))
                .collect(),
        );
        Self {
            tables: read,
            fields: fields.ok(),
            watch,
            unread,
        }
    }
}

/// The reads [`FAMILY`] declares: every table leaders may read.
fn whole() -> &'static Selection {
    static WHOLE: LazyLock<Selection> = LazyLock::new(|| Selection::of(tables().to_vec()));
    &WHOLE
}

/// A run's reads: the scan's own tables and the trigger world's core, and
/// where `content` binds the shared trigger tables declared reads read.
fn selected(content: bool) -> &'static Selection {
    fn read(content: bool) -> Selection {
        let mut tables: Vec<TableId> = POLICY
            .iter()
            .filter(|(.., reach)| content || *reach == Reach::Every)
            .map(|(table, ..)| *table)
            .collect();
        tables.push(TableId::Campaign);
        tables.sort_unstable();
        tables.dedup();
        Selection::of(tables)
    }
    static OWN: LazyLock<Selection> = LazyLock::new(|| read(false));
    static CONTENT: LazyLock<Selection> = LazyLock::new(|| read(true));
    if content { &CONTENT } else { &OWN }
}

/// A run's block reads, with or without bound `content`: the scan's own
/// tables, the trigger world's core and, with content, the shared trigger
/// tables leader visibility's declared reads read ([`POLICY`]). The rest of
/// the trigger kit is read only where an hour falls back
/// ([`observe_block`]): every hour's evaluations are checked to read only
/// what the read holds.
pub fn bound(content: bool) -> Bound {
    let selection = selected(content);
    Bound {
        tables: selection.tables.clone(),
        fields: selection.fields.clone(),
        start: Box::new(move |format| start(format, selection)),
    }
}

/// What a block is read by: every table but the campaign, which the reader
/// checks on its own.
fn fields() -> io::Result<Vec<TableFields>> {
    tables()
        .iter()
        .filter(|&&table| table != TableId::Campaign)
        .map(|&table| watched(table))
        .collect()
}

fn watched(table: TableId) -> io::Result<TableFields> {
    Ok(match policy(table).0 {
        Watch::All => TableFields::all(table),
        Watch::Keys => TableFields::keys(table),
        Watch::Named(names) => TableFields::named(table, names)?,
        Watch::Leaders => TableFields {
            fields: full_mask(table.schema().len()) & !experience_field(),
            ..TableFields::all(table)
        },
    })
}

/// The fields watched, as the block reader reads them; every field of a
/// table whose watched fields the schema no longer names (the block is then
/// read whole).
fn watch() -> &'static [TableFields] {
    static WATCH: LazyLock<Vec<TableFields>> = LazyLock::new(|| {
        fields().unwrap_or_else(|_| {
            tables()
                .iter()
                .filter(|&&table| table != TableId::Campaign)
                .map(|&table| TableFields::all(table))
                .collect()
        })
    });
    &WATCH
}

fn watched_fields(table: TableId) -> u64 {
    watch()
        .iter()
        .find(|watch| watch.table == table)
        .map_or(full_mask(table.schema().len()), |watch| watch.fields)
}

fn is_combat(table: TableId) -> bool {
    matches!(
        table,
        TableId::LandCombatDetails | TableId::NavalCombatDetails
    )
}

/// How one table may have changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    /// Only among these fields; its row keys and availability are equal.
    Fields(u64),
    /// Anywhere.
    Any,
}

/// What may have changed in each of the family's tables, from a block
/// reader's report ([`BlockTables`], [`QuietTables`]); every other table is
/// the same. Also what a kept frame's rows may differ from the whole frame's
/// by.
#[derive(Clone, Debug, Default)]
pub struct Changes(Vec<(TableId, Change)>);

impl Changes {
    /// `changes`' report of each table `selection` reads, and every table it
    /// does not read changed anywhere.
    fn read(changes: &dyn TableChanges, selection: &Selection) -> Self {
        let mut read = Self(
            selection
                .tables
                .iter()
                .filter_map(|&table| match changes.table(table) {
                    TableChange::Same => None,
                    TableChange::Fields(fields) => Some((table, Change::Fields(fields))),
                    TableChange::Rows(..) | TableChange::Any => Some((table, Change::Any)),
                })
                .collect(),
        );
        read.merge(&selection.unread);
        read
    }

    fn get(&self, table: TableId) -> Option<Change> {
        self.0
            .binary_search_by_key(&table, |(held, _)| *held)
            .ok()
            .map(|at| self.0[at].1)
    }

    fn has(&self, table: TableId) -> bool {
        self.get(table).is_some()
    }

    /// Also what `other` may have changed.
    fn merge(&mut self, other: &Self) {
        for &(table, change) in &other.0 {
            match self.0.binary_search_by_key(&table, |(held, _)| *held) {
                Ok(at) => {
                    self.0[at].1 = match (self.0[at].1, change) {
                        (Change::Fields(held), Change::Fields(fields)) => {
                            Change::Fields(held | fields)
                        }
                        _ => Change::Any,
                    }
                }
                Err(at) => self.0.insert(at, (table, change)),
            }
        }
    }
}

impl TableChanges for Changes {
    fn table(&self, table: TableId) -> TableChange<'_> {
        match self.get(table) {
            None => TableChange::Same,
            Some(Change::Fields(fields)) => TableChange::Fields(fields),
            Some(Change::Any) => TableChange::Any,
        }
    }
}

/// What a kept frame's rows may differ from the hour's whole frame by: the
/// fields a reduced table does not keep, and every dropped table; the
/// leaders' experience only when a kept leader had any.
fn reduction(experience: bool) -> Changes {
    let fields = |table: TableId, kept: &[&str]| {
        let all = full_mask(table.schema().len());
        let kept = kept.iter().fold(0, |mask, name| {
            mask | (0..table.schema().len())
                .find(|&field| table.field_name(field) == Some(*name))
                .map_or(0, |field| 1_u64 << field)
        });
        Change::Fields(all & !kept)
    };
    let mut changes = Vec::new();
    for &table in tables() {
        let change = match (policy(table).1, table) {
            (Keep::Whole, _) => None,
            (Keep::Dropped, _) => Some(Change::Any),
            (Keep::Reduced, TableId::UnitLeaders) => {
                experience.then(|| Change::Fields(experience_field()))
            }
            (Keep::Reduced, TableId::Countries) => Some(fields(table, &["canonical_slot", "tag"])),
            (Keep::Reduced, TableId::CountryCounterInputs) => {
                Some(fields(table, &["owned_state_count"]))
            }
            (Keep::Reduced, TableId::WarRelations) => {
                Some(fields(table, &["start_hour", "end_hour"]))
            }
            (Keep::Reduced, TableId::TaskForces) => Some(fields(table, &["fleet"])),
            (Keep::Reduced, TableId::NavalMissionTaskForces) => Some(fields(table, &["leader"])),
            // Only their keys (the core tables), or their commanders.
            (Keep::Reduced, _) => Some(fields(table, &[])),
        };
        if let Some(change) = change {
            changes.push((table, change));
        }
    }
    Changes(changes)
}

/// One hour of a block's facts, in hour order.
enum Hour {
    /// The block's first hour, or one after a gap, with its frame as read.
    Whole {
        frame: DecodedFrame,
        changes: FrameChanges,
    },
    /// An hour at which something leaders read changed, with its kept frame,
    /// its change facts and what changed since the hour before.
    Kept {
        frame: DecodedFrame,
        changes: FrameChanges,
        since: Changes,
    },
    /// A quiet hour whose reads are not the hour before's: its reads, what
    /// changed since the hour before, and what changed since the last kept
    /// hour, whose rows it is observed on.
    Built {
        hour: i32,
        metrics: Metrics,
        sampled: Vec<(TableId, Option<i32>)>,
        since: Changes,
        stale: Changes,
    },
    /// Consecutive hours `first..=last` at which nothing leaders read
    /// changed, with what may have changed since the hour before `first`.
    Quiet {
        first: i32,
        last: i32,
        since: Changes,
    },
}

/// A block's hours as the extract kept them.
struct BlockFacts {
    hours: Vec<Hour>,
    /// What every kept frame's rows may differ from the whole frame's by.
    reduction: Changes,
    /// What a frame kept as read may differ from the whole frame by: the
    /// tables the block's read left out.
    unread: Changes,
    /// Frames the extract was given or built.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    frames: usize,
    /// Tables the extract compared row by row with the hour before's, and
    /// the countries' kept rows it reduced, counted in tests.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    work: Work,
}

/// The extract's counted work.
#[derive(Default)]
#[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
struct Work {
    compared: usize,
    reduced_countries: usize,
}

/// Each table the extract compares row by row with the hour before's,
/// counted in tests: none where the block reader's changed tables are its
/// own.
pub const COMPARED: &str = "leaders.compared";

/// Each time the extract reduces the countries' rows of a kept frame anew,
/// counted in tests: only where their slots, tags, keys or availability
/// changed since the rows last reduced.
pub const REDUCED_COUNTRIES: &str = "leaders.reduced_countries";

/// Each block folded from an extract that read every table leaders may
/// read, counted in tests.
pub const WHOLE_KIT: &str = "leaders.whole_kit";

/// One table of the kept frames reduced: its last reduced view, shared
/// while the reduced rows are equal, and whether the table changed where
/// leaders watch it since the rows it was made from. Every field a reduced
/// table keeps is a watched field or its row key, so while nothing watched
/// changed the last reduced view holds the reduced rows: the table's rows
/// are neither held nor reduced again.
struct Reducer<T> {
    view: Option<SharedView>,
    stale: bool,
    table: PhantomData<T>,
}

impl<T> Default for Reducer<T> {
    fn default() -> Self {
        Self {
            view: None,
            stale: true,
            table: PhantomData,
        }
    }
}

impl<T: TableSchema> Reducer<T> {
    /// Replace `frame`'s complete view of the table by its rows reduced by
    /// `reduce`: the last reduced view while nothing watched changed since,
    /// else the rows reduced again. Another view stays as read. Whether the
    /// rows were reduced again.
    fn apply(&mut self, frame: &mut DecodedFrame, reduce: impl Fn(&T::Row) -> T::Row) -> bool {
        let index = T::ID.index();
        let source = frame.tables[index].clone();
        let sampled = source.sampled_at();
        let TableView::Complete(rows) = &*source else {
            self.stale = true;
            return false;
        };
        if !self.stale
            && let Some(view) = &self.view
        {
            frame.tables[index] = view.clone().with_sampled_at(sampled);
            return false;
        }
        let Some(rows) = rows.rows::<T>() else {
            self.stale = true;
            return false;
        };
        let reduced: Vec<T::Row> = rows.iter().map(reduce).collect();
        let held = self.view.as_ref().filter(|view| match &***view {
            TableView::Complete(rows) => rows.rows::<T>() == Some(reduced.as_slice()),
            _ => false,
        });
        let view = match held {
            Some(view) => view.clone(),
            None => {
                let view: SharedView = TableView::Complete(T::wrap(reduced)).into();
                self.view = Some(view.clone());
                view
            }
        };
        self.stale = false;
        frame.tables[index] = view.with_sampled_at(sampled);
        true
    }
}

#[derive(Default)]
struct Reducers {
    leaders: Reducer<tables::UnitLeaders>,
    countries: Reducer<tables::Countries>,
    counters: Reducer<tables::CountryCounterInputs>,
    provinces: Reducer<tables::Provinces>,
    states: Reducer<tables::States>,
    wars: Reducer<tables::WarRelations>,
    task_forces: Reducer<tables::TaskForces>,
    missions: Reducer<tables::NavalMissionTaskForces>,
    land: Reducer<tables::LandCombatDetails>,
    navy: Reducer<tables::NavalCombatDetails>,
}

impl Reducers {
    /// `table` changed where leaders watch it.
    fn changed(&mut self, table: TableId) {
        let stale = match table {
            TableId::UnitLeaders => &mut self.leaders.stale,
            TableId::Countries => &mut self.countries.stale,
            TableId::CountryCounterInputs => &mut self.counters.stale,
            TableId::Provinces => &mut self.provinces.stale,
            TableId::States => &mut self.states.stale,
            TableId::WarRelations => &mut self.wars.stale,
            TableId::TaskForces => &mut self.task_forces.stale,
            TableId::NavalMissionTaskForces => &mut self.missions.stale,
            TableId::LandCombatDetails => &mut self.land.stale,
            TableId::NavalCombatDetails => &mut self.navy.stale,
            _ => return,
        };
        *stale = true;
    }
}

struct BlockExtract {
    format: FormatVersion,
    /// What the block is read with.
    selection: &'static Selection,
    hours: Vec<Hour>,
    /// The last frame observed, as read, where the block reader's changed
    /// tables are not given: the hour before's rows of every watched field,
    /// which the next hour read is compared with.
    last: Option<DecodedFrame>,
    /// The branch of the last frame observed.
    branch: Option<BranchId>,
    /// The last recorded hour given, observed or quiet.
    previous: Option<i32>,
    /// The scan's reads at that hour ([`read_mask`]).
    mask: Option<u32>,
    /// The commanders of the last frame observed.
    commanders: [Option<Commanders>; 2],
    /// What changed since the last hour kept.
    stale: Changes,
    /// The next hour a relation the last frame holds ends at, which an
    /// ongoing war or subject read compares with the hour.
    deadline: Option<i32>,
    reducers: Reducers,
    dropped: BTreeMap<TableId, SharedView>,
    /// Whether a kept leader had experience, which kept frames do not keep.
    experience: bool,
    frames: usize,
    work: Work,
}

/// An extract of a block read with `selection`.
fn start(format: FormatVersion, selection: &'static Selection) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        format,
        selection,
        hours: Vec::new(),
        last: None,
        branch: None,
        previous: None,
        mask: None,
        commanders: [None, None],
        stale: Changes::default(),
        deadline: None,
        reducers: Reducers::default(),
        dropped: BTreeMap::new(),
        experience: false,
        frames: 0,
        work: Work::default(),
    })
}

/// Whether a watched table's watched fields, row keys or availability differ
/// between two hours' views: the block reader's test of a changed table,
/// made from the two hours' rows.
fn differs(table: TableId, fields: u64, before: &SharedView, after: &SharedView) -> bool {
    if SharedView::ptr_eq(before, after) {
        return false;
    }
    match (&**before, &**after) {
        (TableView::Complete(before), TableView::Complete(after)) => {
            rows_differ(table, fields, before, after)
        }
        (
            TableView::Truncated {
                rows: before,
                missing: before_missing,
            },
            TableView::Truncated {
                rows: after,
                missing: after_missing,
            },
        ) => before_missing != after_missing || rows_differ(table, fields, before, after),
        (TableView::Unavailable { reason: before }, TableView::Unavailable { reason: after }) => {
            before != after
        }
        _ => true,
    }
}

fn rows_differ(table: TableId, fields: u64, before: &TableRows, after: &TableRows) -> bool {
    if before.len() != after.len() {
        return true;
    }
    let all = full_mask(table.schema().len());
    if fields & all == all {
        return before != after;
    }
    if fields & all == 0 {
        return before
            .iter()
            .zip(after.iter())
            .any(|(before, after)| before.key() != after.key());
    }
    macro_rules! typed {
        ($($table:ident),* $(,)?) => {
            match table {
                // The fields hourly-moving tables are watched by, compared
                // without encoding their rows.
                TableId::Countries => pairs::<tables::Countries>(before, after, |a: &CountryRow, b| {
                    (a.id, a.canonical_slot, &a.tag) != (b.id, b.canonical_slot, &b.tag)
                }),
                TableId::WarRelations => {
                    pairs::<tables::WarRelations>(before, after, |a: &WarRelationRow, b| {
                        (a.first_country, a.second_country, a.start_hour, a.end_hour)
                            != (b.first_country, b.second_country, b.start_hour, b.end_hour)
                    })
                }
                TableId::TaskForces => pairs::<tables::TaskForces>(before, after, |a: &TaskForceRow, b| {
                    (a.id, a.fleet) != (b.id, b.fleet)
                }),
                TableId::NavalMissionTaskForces => {
                    pairs::<tables::NavalMissionTaskForces>(before, after, |a: &NavalMissionTaskForceRow, b| {
                        (a.task_force, a.leader) != (b.task_force, b.leader)
                    })
                }
                $(TableId::$table => masked::<tables::$table>(before, after, fields),)*
                _ => true,
            }
        };
    }
    typed!(
        UnitLeaders,
        OrdersGroups,
        Clock,
        PoliticalParties,
        Politics,
        ReservedCountryNaming,
        FactionMembers,
        Subjects,
        Focus,
        CountryAppearance,
        CompletedFocuses,
        Projects,
        ScriptFlags,
        PartyLeaders,
        CountryCharacters,
        CountryCivilWars,
        AdvisorTokens,
        FactionAdvisors,
        HiredAdvisors,
        ActiveIdeas,
    )
}

/// Whether two equally long row sets of `T` differ by `differ` in any pair.
fn pairs<T: TableSchema>(
    before: &TableRows,
    after: &TableRows,
    differ: impl Fn(&T::Row, &T::Row) -> bool,
) -> bool {
    match (before.rows::<T>(), after.rows::<T>()) {
        (Some(before), Some(after)) => before.iter().zip(after).any(|(a, b)| differ(a, b)),
        _ => true,
    }
}

/// Whether two equally long row sets of `T` differ in a row key or in one of
/// `fields`, by each changed row's delta (a field an older format did not
/// record counts as changed).
fn masked<T: TableSchema>(before: &TableRows, after: &TableRows, fields: u64) -> bool {
    let schema = T::ID.schema().len();
    pairs::<T>(before, after, |a, b| {
        a.key() != b.key()
            || a != b
                && match b.delta(Some(a)) {
                    Ok(Some(delta)) => {
                        delta.mask & fields != 0
                            || delta.recorded_fields < schema
                                && u32::try_from(delta.recorded_fields)
                                    .ok()
                                    .and_then(|recorded| fields.checked_shr(recorded))
                                    .unwrap_or(0)
                                    != 0
                    }
                    Ok(None) => false,
                    Err(_) => true,
                }
    })
}

/// The next hour after `hour` at which a war or subject relation `frame`
/// holds ends (`triggers::ongoing`).
fn next_end(frame: &DecodedFrame, hour: i32) -> Option<i32> {
    let wars = frame
        .complete::<tables::WarRelations>()
        .ok()
        .into_iter()
        .flatten()
        .map(|row| (row.start_hour, row.end_hour));
    let subjects = frame
        .complete::<tables::Subjects>()
        .ok()
        .into_iter()
        .flatten()
        .map(|row| (row.start_hour, row.end_hour));
    wars.chain(subjects)
        .filter(|&(start, end)| end > start && end > hour)
        .map(|(_, end)| end)
        .min()
}

impl BlockExtract {
    /// `frame` as kept: each reduced table by the fields leaders and the
    /// declared reads read, each dropped table not at all.
    fn reduce(&mut self, frame: &DecodedFrame) -> DecodedFrame {
        let mut kept = frame.clone();
        let reducers = &mut self.reducers;
        let had = Cell::new(false);
        let reduced = reducers.leaders.apply(&mut kept, |row: &UnitLeaderRow| {
            if row.experience != Default::default() {
                had.set(true);
            }
            UnitLeaderRow {
                experience: Default::default(),
                ..row.clone()
            }
        });
        // Kept as last reduced: whether a leader has experience now.
        if !reduced
            && !self.experience
            && let Ok(rows) = frame.complete::<tables::UnitLeaders>()
        {
            had.set(rows.iter().any(|row| row.experience != Default::default()));
        }
        self.experience |= had.get();
        if reducers
            .countries
            .apply(&mut kept, |row: &CountryRow| CountryRow {
                id: row.id,
                canonical_slot: row.canonical_slot,
                tag: row.tag.clone(),
                ..Default::default()
            })
        {
            self.work.reduced_countries += 1;
        }
        reducers
            .counters
            .apply(&mut kept, |row: &CountryCounterInputRow| {
                CountryCounterInputRow {
                    country: row.country,
                    owned_state_count: row.owned_state_count,
                    ..Default::default()
                }
            });
        reducers
            .provinces
            .apply(&mut kept, |row: &ProvinceRow| ProvinceRow {
                id: row.id,
                ..Default::default()
            });
        reducers.states.apply(&mut kept, |row: &StateRow| StateRow {
            id: row.id,
            ..Default::default()
        });
        reducers
            .wars
            .apply(&mut kept, |row: &WarRelationRow| WarRelationRow {
                first_country: row.first_country,
                second_country: row.second_country,
                start_hour: row.start_hour,
                end_hour: row.end_hour,
                ..Default::default()
            });
        reducers
            .task_forces
            .apply(&mut kept, |row: &TaskForceRow| TaskForceRow {
                id: row.id,
                fleet: row.fleet,
                ..Default::default()
            });
        reducers
            .missions
            .apply(&mut kept, |row: &NavalMissionTaskForceRow| {
                NavalMissionTaskForceRow {
                    task_force: row.task_force,
                    leader: row.leader,
                    ..Default::default()
                }
            });
        reducers
            .land
            .apply(&mut kept, |row: &LandCombatDetailsRow| {
                LandCombatDetailsRow {
                    combat: row.combat,
                    sides: row.sides.each_ref().map(|side| LandCombatSideDetails {
                        commander: side.commander,
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            });
        reducers
            .navy
            .apply(&mut kept, |row: &NavalCombatDetailsRow| {
                NavalCombatDetailsRow {
                    combat: row.combat,
                    sides: row.sides.each_ref().map(|side| NavalCombatSideDetails {
                        commander: side.commander,
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            });
        for &table in &self.selection.tables {
            if policy(table).1 == Keep::Dropped {
                let index = table.index();
                let view = self
                    .dropped
                    .entry(table)
                    .or_insert_with(|| {
                        TableView::Unavailable {
                            reason: "Not kept for unit leaders".into(),
                        }
                        .into()
                    })
                    .clone();
                kept.tables[index] = view.with_sampled_at(frame.tables[index].sampled_at());
            }
        }
        kept
    }

    /// Quiet hours `first..=last`, after the hour before them, at which
    /// `since` may have changed.
    fn push_quiet(&mut self, first: i32, last: i32, since: Changes) {
        self.stale.merge(&since);
        if let Some(Hour::Quiet {
            last: end,
            since: held,
            ..
        }) = self.hours.last_mut()
            && end.checked_add(1) == Some(first)
        {
            *end = last;
            held.merge(&since);
        } else {
            self.hours.push(Hour::Quiet { first, last, since });
        }
    }

    /// Remember `frame` as the last observed: its rows too where the next
    /// hour's changes are found from them (`compare`).
    fn observed(
        &mut self,
        frame: &DecodedFrame,
        mask: u32,
        commanders: [Option<Commanders>; 2],
        compare: bool,
    ) {
        self.deadline = next_end(frame, frame.hour);
        self.previous = Some(frame.hour);
        self.branch = Some(frame.branch);
        self.mask = Some(mask);
        self.commanders = commanders;
        self.last = compare.then(|| frame.clone());
    }

    /// An hour's frame, with the watched tables the block reader reports
    /// changed since the hour before (`reported`), or none where its read
    /// watches more than leaders' fields: the changed tables are then found
    /// from the hour before's rows.
    fn hour(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        reported: Option<&[TableId]>,
    ) -> io::Result<()> {
        self.frames += 1;
        let hour = frame.hour;
        let compare = reported.is_none();
        let mask = read_mask(self.format, &frame.metrics, hour, true);
        let (land, navy) = battle_tables(self.format, frame);
        let commanders = [land.map(land_commanders), navy.map(naval_commanders)];
        let continues = self.previous.and_then(|previous| previous.checked_add(1)) == Some(hour)
            && self.branch == Some(frame.branch)
            && (!compare || self.last.is_some());
        if !continues {
            self.hours.push(Hour::Whole {
                frame: frame.clone(),
                changes: kept_changes(changes),
            });
            self.stale = Changes::default();
            // Nothing tells what changed since the rows last reduced.
            for table in tables() {
                self.reducers.changed(*table);
            }
            self.observed(frame, mask, commanders, compare);
            return Ok(());
        }
        let selection = self.selection;
        // The block reader's report of this hour (`BlockChange::Changed`):
        // the tables its update rebuilt, and of those, each whose watched
        // fields, row keys or availability changed. A combat table counts
        // as changed whenever rebuilt; leaders read its commanders.
        let rebuilt: Vec<TableId> = selection
            .tables
            .iter()
            .copied()
            .filter(|table| changes.rows_changed[table.index()])
            .collect();
        let changed: Vec<TableId> = match reported {
            Some(reported) => rebuilt
                .iter()
                .copied()
                .filter(|table| is_combat(*table) || reported.contains(table))
                .collect(),
            None => {
                let last = self.last.as_ref().expect("the hour before's frame");
                let mut compared = 0;
                let changed = rebuilt
                    .iter()
                    .copied()
                    .filter(|&table| {
                        is_combat(table) || {
                            compared += 1;
                            differs(
                                table,
                                watched_fields(table),
                                &last.tables[table.index()],
                                &frame.tables[table.index()],
                            )
                        }
                    })
                    .collect();
                self.work.compared += compared;
                changed
            }
        };
        for &table in &changed {
            self.reducers.changed(table);
        }
        let since = Changes::read(
            &BlockTables::new(&selection.watch, &changed, &rebuilt),
            selection,
        );
        let read_changed = changed.iter().any(|&table| !is_combat(table));
        if !read_changed
            && commanders == self.commanders
            && Some(mask) == self.mask
            && self.deadline.is_none_or(|deadline| deadline > hour)
        {
            self.push_quiet(hour, hour, since);
        } else {
            let kept = self.reduce(frame);
            self.hours.push(Hour::Kept {
                frame: kept,
                changes: kept_changes(changes),
                since,
            });
            self.stale = Changes::default();
        }
        self.observed(frame, mask, commanders, compare);
        Ok(())
    }
}

/// The tables reported changed.
impl dashboard_base::prepared::prepass::Held for Changes {
    fn held(&self, bytes: &mut dashboard_base::prepared::prepass::HeldBytes) {
        bytes.add(self.0.capacity() * size_of::<(TableId, Change)>());
    }
}

impl Extract for BlockExtract {
    /// What the block's facts hold: each kept frame's views once, its change
    /// facts, and the hours kept between them, with what changed at each.
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            match hour {
                Hour::Whole { frame, changes } => {
                    held.frame(frame).changes(changes);
                }
                Hour::Kept {
                    frame,
                    changes,
                    since,
                } => {
                    held.frame(frame).changes(changes).heap(since);
                }
                Hour::Built {
                    sampled,
                    since,
                    stale,
                    ..
                } => {
                    held.add(sampled.capacity() * size_of::<(TableId, Option<i32>)>())
                        .heap(since)
                        .heap(stale);
                }
                Hour::Quiet { since, .. } => {
                    held.heap(since);
                }
            }
        }
        held.total()
    }

    /// An hour's frame, whose changes since the hour before are found from
    /// the hour before's rows.
    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.hour(frame, changes, None)
    }

    /// An hour's frame, with the block reader's changed tables, which are
    /// leaders' own.
    fn observe_reported(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        reported: &[TableId],
    ) -> io::Result<()> {
        self.hour(frame, changes, Some(reported))
    }

    fn deadline(&self) -> Option<i32> {
        self.deadline
    }

    /// Hours at which nothing leaders watch changed. One whose reads are not
    /// the hour before's is kept as its reads, observed on the last kept
    /// hour's rows; the others are quiet.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let Some((first, _)) = consecutive(hours, span.clone())? else {
            return Ok(());
        };
        if self.previous.and_then(|previous| previous.checked_add(1)) != Some(first) {
            return Err(io::Error::other(
                "Quiet leader hours do not follow the hour before",
            ));
        }
        let selection = self.selection;
        let watch = &selection.watch;
        let mut run: Option<(i32, i32)> = None;
        for position in span.clone() {
            let hour = hours.hours()[position];
            let mask = read_mask(self.format, &hours.metrics()[position], hour, true);
            if Some(mask) == self.mask {
                run = Some(run.map_or((hour, hour), |(first, _)| (first, hour)));
            } else {
                if let Some((first, last)) = run.take() {
                    self.push_quiet(
                        first,
                        last,
                        Changes::read(&QuietTables::new(watch, hours, position), selection),
                    );
                }
                let since = Changes::read(&QuietTables::new(watch, hours, position + 1), selection);
                let mut stale = self.stale.clone();
                stale.merge(&since);
                self.hours.push(Hour::Built {
                    hour,
                    metrics: hours.metrics()[position].clone(),
                    sampled: selection
                        .tables
                        .iter()
                        .map(|&table| (table, hours.sampled_at(position, table)))
                        .collect(),
                    since,
                    stale: stale.clone(),
                });
                self.stale = stale;
                self.mask = Some(mask);
            }
            self.previous = Some(hour);
        }
        if let Some((first, last)) = run {
            self.push_quiet(
                first,
                last,
                Changes::read(&QuietTables::new(watch, hours, span.end), selection),
            );
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Leader block has no hours"));
        }
        let unread = self.selection.unread.clone();
        let mut reduction = reduction(self.experience);
        reduction.merge(&unread);
        Ok(Box::new(BlockFacts {
            reduction,
            unread,
            hours: self.hours,
            frames: self.frames,
            work: self.work,
        }))
    }
}

/// The frame of a quiet hour whose reads are not the hour before's: the
/// last kept hour's rows with the hour's own reads and sample hours.
fn built(
    rows: &DecodedFrame,
    hour: i32,
    metrics: Metrics,
    sampled: &[(TableId, Option<i32>)],
) -> DecodedFrame {
    let mut frame = rows.clone();
    frame.hour = hour;
    frame.metrics = metrics;
    for &(table, at) in sampled {
        let index = table.index();
        frame.tables[index] = frame.tables[index].clone().with_sampled_at(at);
    }
    frame
}

/// Fold one block's facts into the carried leaders. Returns how it folded
/// and the last hour it reached.
pub fn fold(
    prepared: &mut Prepared,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    content: Option<&Content>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<BlockFacts>()
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
    #[cfg(any(test, feature = "test-support"))]
    {
        note_frames(FAMILY.name, facts.frames);
        for _ in 0..facts.work.compared {
            dashboard_base::prepared::prepass::note(COMPARED);
        }
        for _ in 0..facts.work.reduced_countries {
            dashboard_base::prepared::prepass::note(REDUCED_COUNTRIES);
        }
        if facts.unread.0.is_empty() {
            dashboard_base::prepared::prepass::note(WHOLE_KIT);
        }
    }
    let BlockFacts {
        hours,
        reduction,
        unread,
        ..
    } = *facts;
    let mut reached = None;
    // The last kept hour's frame, and whether it is reduced.
    let mut base: Option<(DecodedFrame, bool)> = None;
    for hour in hours {
        check()?;
        match hour {
            Hour::Whole { frame, changes } => {
                // A frame that lacks tables the read left out gives its
                // verdicts as a kept frame does: only where they read none
                // of them, else the block is observed with every table.
                let result = if unread.0.is_empty() {
                    prepared
                        .observe(&frame, replay, content, &changes)
                        .map(|()| true)
                } else {
                    prepared.observe_kept(&frame, replay, content, &changes, &changes, &unread)
                };
                if matches!(result, Ok(false)) {
                    rejected("leaders.fallback.whole");
                    let reached = observe_block(
                        prepared,
                        replay,
                        index,
                        Some(frame.hour),
                        content,
                        observed,
                        check,
                    )?;
                    return Ok((Folded::Fallback, reached));
                }
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                note_observed(FAMILY.name);
                if !observed(frame.hour, result.map(drop))? {
                    return Ok((Folded::Fast, reached));
                }
                base = Some((frame, false));
            }
            Hour::Kept {
                frame,
                changes,
                since,
            } => {
                let result =
                    prepared.observe_kept(&frame, replay, content, &changes, &since, &reduction);
                if matches!(result, Ok(false)) {
                    rejected("leaders.fallback.kept");
                    let reached = observe_block(
                        prepared,
                        replay,
                        index,
                        Some(frame.hour),
                        content,
                        observed,
                        check,
                    )?;
                    return Ok((Folded::Fallback, reached));
                }
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                note_observed(FAMILY.name);
                if !observed(frame.hour, result.map(drop))? {
                    return Ok((Folded::Fast, reached));
                }
                base = Some((frame, true));
            }
            Hour::Built {
                hour,
                metrics,
                sampled,
                since,
                stale,
            } => {
                let (rows, reduced) = base.as_ref().ok_or_else(|| {
                    io::Error::other("A leader hour came before its block's first")
                })?;
                let frame = built(rows, hour, metrics, &sampled);
                let changes =
                    FrameChanges::rebuilt(&frame, replay.format, true, |table| since.has(table));
                // Its rows are the last kept hour's: what changed since,
                // besides that hour's own reduction, or the tables the read
                // left out.
                let mut differ = stale;
                differ.merge(if *reduced { &reduction } else { &unread });
                let result =
                    prepared.observe_kept(&frame, replay, content, &changes, &since, &differ);
                if matches!(result, Ok(false)) {
                    rejected("leaders.fallback.built");
                    let reached = observe_block(
                        prepared,
                        replay,
                        index,
                        Some(hour),
                        content,
                        observed,
                        check,
                    )?;
                    return Ok((Folded::Fallback, reached));
                }
                reached = Some(hour);
                #[cfg(any(test, feature = "test-support"))]
                note_observed(FAMILY.name);
                if !observed(hour, result.map(drop))? {
                    return Ok((Folded::Fast, reached));
                }
            }
            Hour::Quiet { first, last, since } => {
                let (rows, _) = base.as_ref().ok_or_else(|| {
                    io::Error::other("A leader hour came before its block's first")
                })?;
                match prepared.advance_quiet(first, last, &since, rows, content) {
                    Ok(true) => {}
                    Ok(false) => {
                        rejected("leaders.fallback.quiet");
                        let reached = observe_block(
                            prepared,
                            replay,
                            index,
                            Some(first),
                            content,
                            observed,
                            check,
                        )?;
                        return Ok((Folded::Fallback, reached));
                    }
                    Err(error) => {
                        reached = Some(first);
                        observed(first, Err(error))?;
                        return Ok((Folded::Fast, reached));
                    }
                }
                for hour in first..=last {
                    reached = Some(hour);
                    if !observed(hour, Ok(()))? {
                        return Ok((Folded::Fast, reached));
                    }
                }
            }
        }
    }
    Ok((Folded::Fast, reached))
}

/// Observe the block hour by hour from `from` (every hour without it), on its
/// whole frames read with every table leaders may read, with the change
/// facts of the block alone, as hourly preparation does. Returns the last
/// hour observed.
pub fn observe_block(
    prepared: &mut Prepared,
    replay: &Replay,
    index: &BlockIndex,
    from: Option<i32>,
    content: Option<&Content>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    observe_hours(
        replay,
        index,
        tables(),
        from,
        observed,
        check,
        |frame, facts| {
            #[cfg(any(test, feature = "test-support"))]
            note_observed(FAMILY.name);
            prepared.observe(frame, replay, content, facts)
        },
    )
}

#[cfg(test)]
mod tests;
