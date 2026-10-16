//! Unit leaders prepared on their own, block by block. A pre-pass worker
//! reads a block alone, only at the hours a field leaders read changes
//! ([`FAMILY`]'s fields): every field the scan reads of its own tables (all
//! of a leader but its retired experience; the combat details' sides, whose
//! commanders it reads; a formation's leader, name and kind, a mission's
//! leader and a task force's fleet; the countries' slots and tags), and of
//! the shared trigger tables the fields their declared reads read
//! (`triggers::inputs::Key::dependence`), the row keys of the others.
//!
//! At each hour it reads, the worker finds what changed since the hour
//! before as the block reader does (`BlockChange::Changed`: each watched
//! table whose watched fields, row keys or availability changed, made again
//! from the hour before's rows) and keeps it as the hour's change facts
//! (`BlockTables`); a combat whose commanders did not change is not a change
//! of what leaders read, although its other operands move every hour. An
//! hour at which nothing leaders read changed, and whose reads are the hour
//! before's, joins the quiet hours, whose change facts are the reader's
//! (`QuietTables`). It keeps the block's first hour (and one after a gap)
//! whole, every other hour it keeps reduced: the hourly-moving tables only by
//! the fields leaders and the declared reads read, and the tables no declared
//! read reads not at all. A quiet hour whose reads differ from the hour
//! before's is kept as the reads alone (its metrics and sample hours), on the
//! last kept hour's rows.
//!
//! The family folds each block in order on the carried leaders: the whole
//! hours as hourly preparation observes them; each kept hour through the same
//! step on its kept frame ([`Prepared::observe_kept`]), with the hour's change
//! facts, so a visibility verdict is reused while its inputs' declared
//! dependence did not change, read again on the kept frame only where its
//! inputs read there what the whole frame holds, and evaluated there only
//! where everything the evaluation read is the whole frame's; each quiet span
//! as the carried leaders' unchanged hours ([`Prepared::advance_quiet`]).
//! Where a kept hour cannot give its verdicts so, or a quiet span would not
//! carry, the rest of the block is observed hour by hour, on its whole
//! frames, as hourly preparation observes it.

use super::{
    Commanders, Content, battle_tables, experience_field, land_commanders, naval_commanders,
    prepared::Prepared, read_mask,
};
#[cfg(any(test, feature = "test-support"))]
use dashboard_base::prepared::kept::{note_frames, note_observed};
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    kept::{Observed, consecutive, kept_changes, observe_hours},
    prepass::{Extract, Family, Folded},
};
use dashboard_text::triggers::inputs::{BlockTables, QuietTables, TableChange, TableChanges};
use replay::{
    container::{BlockIndex, QuietHours, Replay, TableFields},
    format::FormatVersion,
    frame::{DecodedFrame, Metrics, SharedView, TableView},
    rows::{
        CountryRow, LandCombatDetailsRow, LandCombatSideDetails, NavalCombatDetailsRow,
        NavalCombatSideDetails, NavalMissionTaskForceRow, ProvinceRow, RowCodec, StateRow,
        TableId, TableRows, TableSchema, TaskForceRow, UnitLeaderRow, WarRelationRow, full_mask,
        tables,
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
/// trigger world's own reads), with the fields those reads read. A table a
/// declared read reads more of than is watched is only read again on whole
/// frames, so this list decides how often hours are quiet, never what is
/// published.
const POLICY: &[(TableId, Watch, Keep)] = &[
    // The scan's own tables.
    (TableId::UnitLeaders, Watch::Leaders, Keep::Reduced),
    (TableId::Characters, Watch::All, Keep::Whole),
    (TableId::CharacterPortraits, Watch::All, Keep::Whole),
    (TableId::LexicalTokens, Watch::All, Keep::Whole),
    (TableId::UnitLeaderTraits, Watch::All, Keep::Whole),
    (TableId::LandCombatDetails, Watch::Named(&["sides"]), Keep::Reduced),
    (TableId::NavalCombatDetails, Watch::Named(&["sides"]), Keep::Reduced),
    (TableId::LandCombatResults, Watch::All, Keep::Whole),
    (TableId::NavalResultSources, Watch::All, Keep::Whole),
    (TableId::NavalResults, Watch::All, Keep::Whole),
    (TableId::NavalCombatResultDetails, Watch::All, Keep::Whole),
    (
        TableId::OrdersGroups,
        Watch::Named(&["leader", "name", "is_army_group"]),
        Keep::Whole,
    ),
    (TableId::OrdersGroupPlans, Watch::All, Keep::Whole),
    (TableId::Fleets, Watch::All, Keep::Whole),
    (
        TableId::NavalMissionTaskForces,
        Watch::Named(&["leader"]),
        Keep::Reduced,
    ),
    (TableId::TaskForces, Watch::Named(&["fleet"]), Keep::Reduced),
    (
        TableId::Countries,
        Watch::Named(&["canonical_slot", "tag"]),
        Keep::Reduced,
    ),
    (TableId::CountryLinks, Watch::All, Keep::Whole),
    // The shared trigger tables declared reads read.
    (TableId::Clock, Watch::Named(&["seed"]), Keep::Whole),
    (
        TableId::PoliticalParties,
        Watch::Named(&["is_ruling", "token"]),
        Keep::Whole,
    ),
    (
        TableId::Politics,
        Watch::Named(&["ruling_party", "has_elections"]),
        Keep::Whole,
    ),
    (
        TableId::ReservedCountryNaming,
        Watch::Named(&["faction", "original_tag_reference"]),
        Keep::Whole,
    ),
    (TableId::FactionMembers, Watch::Named(&["faction"]), Keep::Whole),
    (
        TableId::WarRelations,
        Watch::Named(&["start_hour", "end_hour"]),
        Keep::Reduced,
    ),
    (
        TableId::Subjects,
        Watch::Named(&["start_hour", "end_hour"]),
        Keep::Whole,
    ),
    (TableId::Focus, Watch::Named(&["tree"]), Keep::Whole),
    (
        TableId::CountryAppearance,
        Watch::Named(&["original_tag_reference"]),
        Keep::Whole,
    ),
    (TableId::CompletedFocuses, Watch::Named(&["token"]), Keep::Whole),
    (
        TableId::Projects,
        Watch::Named(&["definition_id", "completed"]),
        Keep::Whole,
    ),
    (TableId::ScriptFlags, Watch::Named(&["value"]), Keep::Whole),
    (
        TableId::PartyLeaders,
        Watch::Named(&["character", "leader_id", "role"]),
        Keep::Whole,
    ),
    (TableId::CountryCharacters, Watch::Named(&["character"]), Keep::Whole),
    (TableId::CountryCivilWars, Watch::Named(&["target"]), Keep::Whole),
    (TableId::AdvisorTokens, Watch::Named(&["token"]), Keep::Whole),
    (TableId::FactionAdvisors, Watch::Named(&["token"]), Keep::Whole),
    (TableId::HiredAdvisors, Watch::Named(&["idea"]), Keep::Whole),
    (TableId::ActiveIdeas, Watch::Named(&["token"]), Keep::Whole),
    (TableId::CountryWars, Watch::All, Keep::Whole),
    // The trigger world's core tables: their keys and availability.
    (TableId::Provinces, Watch::Keys, Keep::Reduced),
    (TableId::States, Watch::Keys, Keep::Reduced),
];

/// How the family watches and keeps `table`.
fn policy(table: TableId) -> (Watch, Keep) {
    if table == TableId::Campaign {
        return (Watch::Keys, Keep::Whole);
    }
    POLICY
        .iter()
        .find(|(listed, ..)| *listed == table)
        .map_or((Watch::Keys, Keep::Dropped), |&(_, watch, keep)| {
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
    start,
});

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
    /// `changes`' report of each of the family's tables.
    fn read(changes: &dyn TableChanges) -> Self {
        Self(
            tables()
                .iter()
                .filter_map(|&table| match changes.table(table) {
                    TableChange::Same => None,
                    TableChange::Fields(fields) => Some((table, Change::Fields(fields))),
                    TableChange::Rows(..) | TableChange::Any => Some((table, Change::Any)),
                })
                .collect(),
        )
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
            (Keep::Reduced, TableId::Countries) => {
                Some(fields(table, &["canonical_slot", "tag"]))
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
    /// Frames the extract was given or built.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    frames: usize,
}

/// One table of the kept frames reduced: the last view reduced and its
/// reduced view, shared while the reduced rows are equal.
struct Reducer<T> {
    source: Option<SharedView>,
    view: Option<SharedView>,
    table: PhantomData<T>,
}

impl<T> Default for Reducer<T> {
    fn default() -> Self {
        Self {
            source: None,
            view: None,
            table: PhantomData,
        }
    }
}

impl<T: TableSchema> Reducer<T> {
    /// Replace `frame`'s complete view of the table by its rows reduced by
    /// `reduce`. Another view stays as read.
    fn apply(&mut self, frame: &mut DecodedFrame, reduce: impl Fn(&T::Row) -> T::Row) {
        let index = T::ID.index();
        let source = frame.tables[index].clone();
        let sampled = source.sampled_at();
        if let (Some(last), Some(view)) = (&self.source, &self.view)
            && SharedView::ptr_eq(last, &source)
        {
            frame.tables[index] = view.clone().with_sampled_at(sampled);
            return;
        }
        let TableView::Complete(rows) = &*source else {
            return;
        };
        let Some(rows) = rows.rows::<T>() else {
            return;
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
        self.source = Some(source);
        frame.tables[index] = view.with_sampled_at(sampled);
    }
}

#[derive(Default)]
struct Reducers {
    leaders: Reducer<tables::UnitLeaders>,
    countries: Reducer<tables::Countries>,
    provinces: Reducer<tables::Provinces>,
    states: Reducer<tables::States>,
    wars: Reducer<tables::WarRelations>,
    task_forces: Reducer<tables::TaskForces>,
    missions: Reducer<tables::NavalMissionTaskForces>,
    land: Reducer<tables::LandCombatDetails>,
    navy: Reducer<tables::NavalCombatDetails>,
}

struct BlockExtract {
    format: FormatVersion,
    hours: Vec<Hour>,
    /// The last frame observed, as read: the hour before's rows of every
    /// watched field, which the next hour read is compared with.
    last: Option<DecodedFrame>,
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
}

fn start(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        format,
        hours: Vec::new(),
        last: None,
        previous: None,
        mask: None,
        commanders: [None, None],
        stale: Changes::default(),
        deadline: None,
        reducers: Reducers::default(),
        dropped: BTreeMap::new(),
        experience: false,
        frames: 0,
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
        reducers.leaders.apply(&mut kept, |row: &UnitLeaderRow| {
            if row.experience != Default::default() {
                had.set(true);
            }
            UnitLeaderRow {
                experience: Default::default(),
                ..row.clone()
            }
        });
        self.experience |= had.get();
        reducers.countries.apply(&mut kept, |row: &CountryRow| CountryRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
            tag: row.tag.clone(),
            ..Default::default()
        });
        reducers.provinces.apply(&mut kept, |row: &ProvinceRow| ProvinceRow {
            id: row.id,
            ..Default::default()
        });
        reducers.states.apply(&mut kept, |row: &StateRow| StateRow {
            id: row.id,
            ..Default::default()
        });
        reducers.wars.apply(&mut kept, |row: &WarRelationRow| WarRelationRow {
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
            .apply(&mut kept, |row: &LandCombatDetailsRow| LandCombatDetailsRow {
                combat: row.combat,
                sides: row.sides.each_ref().map(|side| LandCombatSideDetails {
                    commander: side.commander,
                    ..Default::default()
                }),
                ..Default::default()
            });
        reducers
            .navy
            .apply(&mut kept, |row: &NavalCombatDetailsRow| NavalCombatDetailsRow {
                combat: row.combat,
                sides: row.sides.each_ref().map(|side| NavalCombatSideDetails {
                    commander: side.commander,
                    ..Default::default()
                }),
                ..Default::default()
            });
        for &table in tables() {
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

    /// Remember `frame` as the last observed.
    fn observed(&mut self, frame: &DecodedFrame, mask: u32, commanders: [Option<Commanders>; 2]) {
        self.deadline = next_end(frame, frame.hour);
        self.previous = Some(frame.hour);
        self.mask = Some(mask);
        self.commanders = commanders;
        self.last = Some(frame.clone());
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

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.frames += 1;
        let hour = frame.hour;
        let mask = read_mask(self.format, &frame.metrics, hour, true);
        let (land, navy) = battle_tables(self.format, frame);
        let commanders = [land.map(land_commanders), navy.map(naval_commanders)];
        let continues = self.previous.and_then(|previous| previous.checked_add(1)) == Some(hour)
            && self.last.as_ref().is_some_and(|last| last.branch == frame.branch);
        if !continues {
            self.hours.push(Hour::Whole {
                frame: frame.clone(),
                changes: kept_changes(changes),
            });
            self.stale = Changes::default();
            self.observed(frame, mask, commanders);
            return Ok(());
        }
        let last = self.last.as_ref().expect("the hour before's frame");
        // The block reader's report of this hour (`BlockChange::Changed`):
        // the tables its update rebuilt, and of those, each whose watched
        // fields, row keys or availability changed. A combat table counts
        // as changed whenever rebuilt; leaders read its commanders.
        let rebuilt: Vec<TableId> = tables()
            .iter()
            .copied()
            .filter(|table| changes.rows_changed[table.index()])
            .collect();
        let changed: Vec<TableId> = rebuilt
            .iter()
            .copied()
            .filter(|&table| {
                is_combat(table)
                    || differs(
                        table,
                        watched_fields(table),
                        &last.tables[table.index()],
                        &frame.tables[table.index()],
                    )
            })
            .collect();
        let since = Changes::read(&BlockTables::new(watch(), &changed, &rebuilt));
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
        self.observed(frame, mask, commanders);
        Ok(())
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
        let watch = watch();
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
                        Changes::read(&QuietTables::new(watch, hours, position)),
                    );
                }
                let since = Changes::read(&QuietTables::new(watch, hours, position + 1));
                let mut stale = self.stale.clone();
                stale.merge(&since);
                self.hours.push(Hour::Built {
                    hour,
                    metrics: hours.metrics()[position].clone(),
                    sampled: tables()
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
                Changes::read(&QuietTables::new(watch, hours, span.end)),
            );
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Leader block has no hours"));
        }
        Ok(Box::new(BlockFacts {
            reduction: reduction(self.experience),
            hours: self.hours,
            frames: self.frames,
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
    note_frames(FAMILY.name, facts.frames);
    let BlockFacts {
        hours, reduction, ..
    } = *facts;
    let mut reached = None;
    // The last kept hour's frame, and whether it is reduced.
    let mut base: Option<(DecodedFrame, bool)> = None;
    for hour in hours {
        check()?;
        match hour {
            Hour::Whole { frame, changes } => {
                reached = Some(frame.hour);
                #[cfg(any(test, feature = "test-support"))]
                note_observed(FAMILY.name);
                let result = prepared.observe(&frame, replay, content, &changes);
                if !observed(frame.hour, result)? {
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
                let (rows, reduced) = base
                    .as_ref()
                    .ok_or_else(|| io::Error::other("A leader hour came before its block's first"))?;
                let frame = built(rows, hour, metrics, &sampled);
                let changes =
                    FrameChanges::rebuilt(&frame, replay.format, true, |table| since.has(table));
                // Its rows are the last kept hour's: what changed since,
                // besides that hour's own reduction.
                let mut differ = stale;
                if *reduced {
                    differ.merge(&reduction);
                }
                let result =
                    prepared.observe_kept(&frame, replay, content, &changes, &since, &differ);
                if matches!(result, Ok(false)) {
                    let reached =
                        observe_block(prepared, replay, index, Some(hour), content, observed, check)?;
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
                let (rows, _) = base
                    .as_ref()
                    .ok_or_else(|| io::Error::other("A leader hour came before its block's first"))?;
                match prepared.advance_quiet(first, last, &since, rows, content) {
                    Ok(true) => {}
                    Ok(false) => {
                        let reached =
                            observe_block(prepared, replay, index, Some(first), content, observed, check)?;
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
/// whole frames, with the change facts of the block alone, as hourly
/// preparation does. Returns the last hour observed.
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
