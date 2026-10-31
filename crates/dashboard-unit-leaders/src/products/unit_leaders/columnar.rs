use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_text::triggers::inputs::{TableChange, TableChanges};
use dashboard_world::{Needs, World};
use replay::{
    columns,
    container::Replay,
    segments::{self, Item, Operator, Sink, Start, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        DeltaRow, FleetRow, OrdersGroupPlanRow, OrdersGroupRow, RowChanges, RowCodec, TABLE_COUNT,
        TableId, TableRows, full_mask,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, SharedView, TableView, not_selected};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    mem::size_of,
    sync::Arc,
};

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// How the projection holds a table leaders may read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// Every field of every row.
    Whole,
    /// Every row, current in these fields; its others as the row was first read.
    Watched(u64),
    /// Every row's key alone, as a kept frame keeps the trigger world's core.
    Keys,
    /// The rows the roster reads alone: each leader's character and portrait, and the tokens
    /// spelling their names, their template tags and the leaders' training.
    Roster,
}

/// How a run holds each table leaders may read: by [`block::bound`]'s read, with its
/// watched fields; None for a table only a fallback reads.
fn holds(catalogue: bool) -> Vec<Option<Hold>> {
    let bound = block::bound(catalogue);
    let mut holds = vec![None; TABLE_COUNT];
    for &table in &bound.tables {
        let all = full_mask(table.schema().len());
        let watched = bound
            .fields
            .as_ref()
            .and_then(|fields| fields.iter().find(|watch| watch.table == table))
            .map_or(all, |watch| watch.fields);
        holds[table.index()] = Some(match table {
            TableId::Characters | TableId::CharacterPortraits | TableId::LexicalTokens => {
                Hold::Roster
            }
            TableId::Provinces | TableId::States if watched == 0 => Hold::Keys,
            _ if watched & all == all => Hold::Whole,
            _ => Hold::Watched(watched),
        });
    }
    holds
}

/// What a frame held as `holds` says may differ from the hour's whole frame by.
struct Reduced(Vec<TableChange<'static>>);

impl Reduced {
    fn new(holds: &[Option<Hold>]) -> Self {
        let mut reduced = vec![TableChange::Same; TABLE_COUNT];
        for &table in block::tables() {
            let all = full_mask(table.schema().len());
            reduced[table.index()] = match holds[table.index()] {
                Some(Hold::Whole) => TableChange::Same,
                Some(Hold::Watched(fields)) => TableChange::Fields(all & !fields),
                Some(Hold::Keys) => TableChange::Fields(all),
                Some(Hold::Roster) | None => TableChange::Any,
            };
        }
        Self(reduced)
    }
}

impl TableChanges for Reduced {
    fn table(&self, table: TableId) -> TableChange<'_> {
        self.0[table.index()]
    }
}

/// The tokens the roster reads: its characters' names and template tags, and its training's.
fn spelled(frame: &DecodedFrame) -> Vec<u128> {
    let mut tokens = Vec::new();
    for character in frame.complete::<tables::Characters>().unwrap_or_default() {
        if let Ok(token) = u32::try_from(character.name_token_id) {
            tokens.push(u128::from(token));
        }
        if character.template_present != 0 {
            tokens.push(u128::from(character.template_tag));
        }
    }
    for training in frame
        .complete::<tables::UnitLeaderTraitExperience>()
        .unwrap_or_default()
    {
        tokens.push(u128::from(training.token_id as u32));
    }
    tokens.sort_unstable();
    tokens.dedup();
    tokens
}

/// What an hour wrote of a complete table: each row it changed or inserted, ascending by key,
/// with its slot, the fields written and whether it was inserted, and the keys it deleted and
/// did not insert again.
struct Written {
    rows: Vec<(u128, u32, u64, bool)>,
    deleted: Vec<u128>,
    /// The fields the recording's format records.
    recorded: usize,
}

impl Written {
    fn read(state: &State, table: TableId, format: FormatVersion) -> io::Result<Self> {
        let held = columns::table(state, table)?;
        let changes = columns::changes(state, table)?;
        if changes.changed.len() != changes.masks.len()
            || changes.fill_slots.len() != changes.fill_masks.len()
        {
            return Err(invalid("A leaders input's masks miss its changed slots"));
        }
        let recorded = table.last_change().fields(table, format.version());
        let mut rows = Vec::with_capacity(
            changes.changed.len() + changes.fill_slots.len() + changes.inserted.len(),
        );
        for (&slot, &mask) in changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            .filter(|(_, mask)| **mask != 0)
        {
            let key = held
                .key(slot)
                .ok_or_else(|| invalid("A changed leaders input slot has no row"))?;
            rows.push((key, slot, mask, false));
        }
        for &slot in &changes.inserted {
            let key = held
                .key(slot)
                .ok_or_else(|| invalid("An inserted leaders input slot has no row"))?;
            rows.push((key, slot, full_mask(recorded), true));
        }
        // A row inserted at a key replaces the key's other writes.
        rows.sort_unstable_by_key(|&(key, _, _, inserted)| (key, inserted));
        rows.dedup_by(|later, kept| {
            if later.0 != kept.0 {
                return false;
            }
            if later.3 {
                *kept = *later;
            } else {
                kept.2 |= later.2;
            }
            true
        });
        let mut deleted = changes.deleted_keys.clone();
        deleted.sort_unstable();
        deleted.dedup();
        // A key deleted and inserted again this hour is replaced, never removed.
        deleted.retain(|key| rows.binary_search_by_key(key, |row| row.0).is_err());
        Ok(Self {
            rows,
            deleted,
            recorded,
        })
    }

    fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.deleted.is_empty()
    }

    /// The hour's change of the table, as its decoded delta tells it.
    fn change(&self, format: FormatVersion) -> Change {
        Change::Rows(
            self.rows
                .iter()
                .map(|&(key, _, mask, _)| DeltaRow {
                    format: format.version(),
                    key,
                    mask,
                    recorded_fields: self.recorded,
                    values: Vec::new(),
                })
                .collect(),
            self.deleted.clone(),
        )
    }
}

/// The tables the roster reads by its own keys alone.
const ROSTER: [TableId; 3] = [
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
];

/// How the walk's frame is asked for the tables held as `holds` say, but those the roster
/// reads by its own keys: a table held by its keys alone is current in none of its fields.
fn asks(holds: &[Option<Hold>]) -> Vec<(TableId, dashboard_world::Hold)> {
    block::tables()
        .iter()
        .filter_map(|&table| {
            let hold = match holds[table.index()]? {
                Hold::Whole => dashboard_world::Hold::Whole,
                Hold::Watched(fields) => dashboard_world::Hold::Fields(fields),
                Hold::Keys => dashboard_world::Hold::Fields(0),
                Hold::Roster => return None,
            };
            Some((table, hold))
        })
        .collect()
}

/// What a run's leaders read of its walk's world: every table held but the roster's own.
pub fn needs(catalogue: bool) -> Needs {
    Needs {
        frame: asks(&holds(catalogue)),
        ..Needs::default()
    }
}

/// The walker's tables as the projection holds them, hour by hour: the walk's shared frame
/// ([`dashboard_world`]), with the rows the roster reads by its own keys kept here.
struct Held {
    format: FormatVersion,
    mode: CaptureMode,
    holds: Vec<Option<Hold>>,
    asks: Vec<(TableId, dashboard_world::Hold)>,
    reduced: Reduced,
    /// Whether the roster tables are held by the keys the roster reads.
    roster: bool,
    /// Whether every table leaders may read is held whole.
    whole: bool,
    statuses: Vec<Option<columns::Status>>,
    samples: Vec<Option<i32>>,
    /// The keys each roster table holds, ascending.
    keys: Vec<Vec<u128>>,
    /// The characters and the tokens the roster reads, ascending.
    characters: Vec<u128>,
    tokens: Vec<u128>,
    /// Rows read from the walker's columns, by table.
    decoded: Vec<u64>,
    /// The roster tables' own views between hours, as [`ROSTER`] orders them.
    own: [Option<SharedView>; ROSTER.len()],
    /// The hour they were kept at.
    last: Option<(u64, i32)>,
    /// The frame each hour is read into, kept between hours holding none of the walk's views.
    frame: Option<DecodedFrame>,
}

impl Held {
    fn new(format: FormatVersion, mode: CaptureMode, holds: Vec<Option<Hold>>) -> Self {
        Self {
            format,
            mode,
            asks: asks(&holds),
            reduced: Reduced::new(&holds),
            roster: holds.contains(&Some(Hold::Roster)),
            whole: block::tables()
                .iter()
                .all(|table| holds[table.index()] == Some(Hold::Whole)),
            holds,
            statuses: vec![None; TABLE_COUNT],
            samples: vec![None; TABLE_COUNT],
            keys: vec![Vec::new(); TABLE_COUNT],
            characters: Vec::new(),
            tokens: Vec::new(),
            decoded: vec![0; TABLE_COUNT],
            own: Default::default(),
            last: None,
            frame: None,
        }
    }

    /// How the hour's frame may differ from its whole frame; None once it holds every table
    /// whole.
    fn kept(&self) -> Option<Kept<'_>> {
        (!self.whole).then(|| Kept::projected(&self.reduced, self.roster))
    }

    fn need(&self, table: TableId) -> &[u128] {
        if table == TableId::LexicalTokens {
            &self.tokens
        } else {
            &self.characters
        }
    }

    /// The place of a roster table's own view among [`ROSTER`].
    fn own(table: TableId) -> usize {
        ROSTER
            .iter()
            .position(|&held| held == table)
            .expect("a roster table")
    }

    /// The hour's frame, its change facts and what changed since the hour before.
    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
    ) -> io::Result<(DecodedFrame, FrameChanges, Changes)> {
        let continuous = self.last.is_some_and(|(branch, at)| {
            branch == hour.branch && at.checked_add(1) == Some(hour.hour)
        });
        if !continuous && !hour.opens {
            return Err(invalid(
                "A leaders projection is missing its segment's first hour",
            ));
        }
        let fresh = hour.opens || !continuous;
        let mut frame = self.frame.take().unwrap_or_else(|| DecodedFrame {
            branch: hour.branch,
            hour: hour.hour,
            tables: std::array::from_fn(|_| not_selected()),
            metrics: Default::default(),
        });
        hour.world
            .frame_held_into(hour.state, &self.asks, &mut frame)?;
        let mut present = [false; TABLE_COUNT];
        for section in &hour.manifest.sections {
            let held = present
                .get_mut(section.table)
                .ok_or_else(|| invalid("A leaders input section is outside the schema"))?;
            *held = true;
        }
        let mut changes = Changes((0..TABLE_COUNT).map(|_| None).collect());
        let mut anew = [false; TABLE_COUNT];
        for &table in block::tables() {
            let index = table.index();
            let status = columns::status(hour.state, table);
            anew[index] = fresh || self.statuses[index].replace(status) != Some(status);
            self.samples[index] = matches!(status, columns::Status::Complete)
                .then(|| hour.state.sampled_at(index))
                .flatten();
            changes.0[index] = match self.holds[index] {
                // A table the frame does not hold is only told changed.
                None => (anew[index] || present[index]).then_some(Change::Any),
                Some(Hold::Roster) => continue,
                Some(_) if anew[index] => Some(Change::Any),
                Some(hold) => self.change(hour.state, table, hold, present[index])?,
            };
        }
        if self.roster {
            let mut own = std::mem::take(&mut self.own);
            let mut regrouped = fresh;
            if (fresh || changes.0[TableId::UnitLeaders.index()].is_some())
                && let Ok(rows) = frame.complete::<tables::UnitLeaders>()
            {
                let mut characters: Vec<u128> = rows
                    .iter()
                    .map(|row| u128::from(row.character_id))
                    .collect();
                characters.sort_unstable();
                characters.dedup();
                regrouped |= characters != self.characters;
                self.characters = characters;
            }
            for table in [TableId::Characters, TableId::CharacterPortraits] {
                if self.holds[table.index()] == Some(Hold::Roster) {
                    let view = &mut own[Self::own(table)];
                    changes.0[table.index()] =
                        self.roster_table(hour.state, view, table, anew[table.index()], regrouped)?;
                    frame.tables[table.index()] = view.take().unwrap_or_else(not_selected);
                }
            }
            let lexical = TableId::LexicalTokens;
            if self.holds[lexical.index()] == Some(Hold::Roster) {
                let mut retokened = fresh;
                if regrouped
                    || [TableId::Characters, TableId::UnitLeaderTraitExperience]
                        .into_iter()
                        .any(|table| changes.0[table.index()].is_some())
                {
                    let tokens = spelled(&frame);
                    retokened |= tokens != self.tokens;
                    self.tokens = tokens;
                }
                let view = &mut own[Self::own(lexical)];
                changes.0[lexical.index()] =
                    self.roster_table(hour.state, view, lexical, anew[lexical.index()], retokened)?;
                frame.tables[lexical.index()] = view.take().unwrap_or_else(not_selected);
            }
        }
        for &table in block::tables() {
            let index = table.index();
            if self.holds[index].is_some()
                && frame.tables[index].sampled_at() != self.samples[index]
            {
                let view = std::mem::replace(&mut frame.tables[index], not_selected());
                frame.tables[index] = view.with_sampled_at(self.samples[index]);
            }
        }
        let facts = FrameChanges::rebuilt(&frame, self.format, !fresh, |table| {
            changes.0[table.index()].is_some()
        });
        Ok((frame, facts, changes))
    }

    /// Keeps the roster's own views of `frame` for the next hour, and `frame` to read it into;
    /// the walk's views go, so its next build changes them in place.
    fn keep(&mut self, mut frame: DecodedFrame) {
        self.last = Some((frame.branch, frame.hour));
        for table in ROSTER {
            if self.holds[table.index()] == Some(Hold::Roster) {
                self.own[Self::own(table)] = Some(std::mem::replace(
                    &mut frame.tables[table.index()],
                    not_selected(),
                ));
            }
        }
        dashboard_world::release(&mut frame, &self.asks);
        self.frame = Some(frame);
    }

    /// A roster table's view as the walker holds it this hour, its rows the roster's keys'.
    fn snapshot(&mut self, state: &State, table: TableId) -> io::Result<TableView> {
        let index = table.index();
        let version = self.format.version();
        Ok(match columns::status(state, table) {
            columns::Status::Complete => {
                let walked = columns::table(state, table)?;
                let keys: Vec<u128> = self
                    .need(table)
                    .iter()
                    .copied()
                    .filter(|&key| walked.slot(key).is_some())
                    .collect();
                let slots: Vec<u32> = keys.iter().filter_map(|&key| walked.slot(key)).collect();
                self.keys[index] = keys;
                self.decoded[index] += slots.len() as u64;
                TableView::Complete(columns::rows_at(state, table, &slots, version)?)
            }
            columns::Status::Truncated { missing } => {
                let slots = columns::truncated(state, table)
                    .ok_or_else(|| invalid("A truncated leaders input has no read slots"))?;
                self.decoded[index] += slots.len() as u64;
                self.keys[index].clear();
                TableView::Truncated {
                    rows: columns::rows_at(state, table, slots, version)?,
                    missing,
                }
            }
            columns::Status::Unavailable { reason } => {
                let bytes = state
                    .dictionary()
                    .get(reason)
                    .ok_or_else(|| invalid("A leaders input's reason is absent"))?;
                TableView::Unavailable {
                    reason: std::str::from_utf8(bytes)
                        .map_err(|_| invalid("A leaders input's reason is not UTF-8"))?
                        .to_owned(),
                }
            }
            columns::Status::NotRecorded => TableView::NotRecorded { mode: self.mode },
        })
    }

    /// A roster table's own `view` read anew.
    fn renew(
        &mut self,
        state: &State,
        view: &mut Option<SharedView>,
        table: TableId,
    ) -> io::Result<()> {
        *view = Some(self.snapshot(state, table)?.into());
        Ok(())
    }

    /// What the hour changed of `table`, which the walk's frame holds as `hold` says.
    fn change(
        &self,
        state: &State,
        table: TableId,
        hold: Hold,
        present: bool,
    ) -> io::Result<Option<Change>> {
        match columns::status(state, table) {
            columns::Status::Complete if present => {}
            columns::Status::Truncated { .. } => return Ok(Some(Change::Any)),
            _ => return Ok(None),
        }
        let watched = match hold {
            Hold::Watched(fields) => fields,
            Hold::Keys => 0,
            Hold::Whole | Hold::Roster => full_mask(table.schema().len()),
        };
        let changes = columns::changes(state, table)?;
        if changes.inserted.is_empty() && changes.deleted.is_empty() {
            let fields = changes
                .masks
                .iter()
                .chain(&changes.fill_masks)
                .fold(0_u64, |fields, mask| fields | mask);
            if fields & watched == 0 {
                // Only fields the frame does not hold changed: its rows stand.
                return Ok((fields != 0).then_some(Change::Fields(fields)));
            }
        }
        let written = Written::read(state, table, self.format)?;
        Ok((!written.is_empty()).then(|| written.change(self.format)))
    }

    /// A roster table's rows this hour, its own `view`: those the roster reads, held as the
    /// walker holds them, read again where the hour wrote them; and what changed since the hour
    /// before.
    fn roster_table(
        &mut self,
        state: &State,
        view: &mut Option<SharedView>,
        table: TableId,
        anew: bool,
        regrouped: bool,
    ) -> io::Result<Option<Change>> {
        if anew {
            self.renew(state, view, table)?;
            return Ok(Some(Change::Any));
        }
        match columns::status(state, table) {
            columns::Status::Complete => {}
            columns::Status::Truncated { .. } => {
                self.renew(state, view, table)?;
                return Ok(Some(Change::Any));
            }
            _ => return Ok(None),
        }
        let index = table.index();
        let written = Written::read(state, table, self.format)?;
        let walked = columns::table(state, table)?;
        let mut patches = Vec::new();
        let mut read = Vec::new();
        let mut dropped = Vec::new();
        let mut held = None;
        if regrouped || !written.deleted.is_empty() || written.rows.iter().any(|row| row.3) {
            let keys: Vec<u128> = self
                .need(table)
                .iter()
                .copied()
                .filter(|&key| walked.slot(key).is_some())
                .collect();
            let mut at = 0;
            for &key in &self.keys[index] {
                while keys.get(at).is_some_and(|&next| next < key) {
                    at += 1;
                }
                if keys.get(at) != Some(&key) {
                    dropped.push(key);
                }
            }
            for &key in &keys {
                let kept = self.keys[index].binary_search(&key).is_ok();
                let wrote = written
                    .rows
                    .binary_search_by_key(&key, |row| row.0)
                    .ok()
                    .map(|at| written.rows[at]);
                match wrote {
                    Some((_, slot, mask, false)) if kept => patches.push((key, slot, mask)),
                    None if kept => {}
                    _ => read.push(
                        walked
                            .slot(key)
                            .ok_or_else(|| invalid("A held leaders input key has no row"))?,
                    ),
                }
            }
            held = Some(keys);
        } else {
            for &(key, slot, mask, _) in &written.rows {
                if self.keys[index].binary_search(&key).is_ok() {
                    patches.push((key, slot, mask));
                }
            }
        }
        if !patches.is_empty() || !read.is_empty() || !dropped.is_empty() {
            let added = if read.is_empty() {
                None
            } else {
                Some(columns::rows_at(
                    state,
                    table,
                    &read,
                    self.format.version(),
                )?)
            };
            self.decoded[index] += (patches.len() + read.len()) as u64;
            self.write(state, view, table, &patches, added, dropped)?;
        }
        if let Some(keys) = held {
            self.keys[index] = keys;
        }
        Ok((!written.is_empty()).then(|| written.change(self.format)))
    }

    /// `table`'s own complete `view`, its rows at `patches` (key, slot and fields, by key) set
    /// from the walker, `added` set at their keys and `deleted` removed.
    fn write(
        &self,
        state: &State,
        view: &mut Option<SharedView>,
        table: TableId,
        patches: &[(u128, u32, u64)],
        added: Option<TableRows>,
        deleted: Vec<u128>,
    ) -> io::Result<()> {
        let TableView::Complete(mut rows) = view
            .take()
            .ok_or_else(|| invalid("A leaders input delta has no baseline"))?
            .try_unwrap()
            .map_err(|_| invalid("A leaders input's previous rows are still shared"))?
        else {
            return Err(invalid("A leaders input delta has no complete baseline"));
        };
        if !patches.is_empty() {
            columns::patch(state, table, &mut rows, patches, self.format.version())?;
        }
        if added.is_some() || !deleted.is_empty() {
            let added = match added {
                Some(added) => added,
                None => TableRows::empty(table),
            };
            rows = rows.apply_changes(RowChanges::new(added, deleted))?;
        }
        *view = Some(TableView::Complete(rows).into());
        Ok(())
    }

    /// Holds more after a held frame could not give an hour's verdicts: each table a blocked
    /// evaluation read beyond what the frame held as the whole frame holds it, current in the
    /// fields it read, or every table leaders may read whole where a verdict needs them all.
    /// False when it holds nothing more.
    fn widen(
        &mut self,
        hour: &segments::Hour<'_, World>,
        frame: &mut DecodedFrame,
        wanted: &Wanted,
    ) -> io::Result<bool> {
        let tables = block::tables();
        let mut holds = self.holds.clone();
        let whole = wanted.whole
            || wanted
                .reads
                .iter()
                .any(|read| tables.binary_search(&read.table).is_err());
        if whole {
            for &table in tables {
                holds[table.index()] = Some(Hold::Whole);
            }
        } else {
            for read in &wanted.reads {
                let index = read.table.index();
                let all = full_mask(read.table.schema().len());
                let fields = match holds[index] {
                    Some(Hold::Whole) => continue,
                    Some(Hold::Watched(fields)) => fields | read.fields,
                    Some(Hold::Roster) => all,
                    Some(Hold::Keys) | None => read.fields,
                };
                holds[index] = Some(if fields & all == all {
                    Hold::Whole
                } else {
                    Hold::Watched(fields)
                });
            }
        }
        let mut widened = Vec::new();
        for &table in tables {
            let index = table.index();
            let Some(hold) = holds[index].filter(|hold| self.holds[index] != Some(*hold)) else {
                continue;
            };
            self.holds[index] = Some(hold);
            widened.push(table);
        }
        if !widened.is_empty() {
            self.asks = asks(&self.holds);
            let fuller = hour.world.frame_held(hour.state, &self.asks)?;
            for &table in &widened {
                let index = table.index();
                frame.tables[index] = fuller.tables[index]
                    .clone()
                    .with_sampled_at(self.samples[index]);
            }
        }
        self.roster = self.holds.contains(&Some(Hold::Roster));
        self.whole = tables
            .iter()
            .all(|table| self.holds[table.index()] == Some(Hold::Whole));
        self.reduced = Reduced::new(&self.holds);
        Ok(!widened.is_empty())
    }
}

enum Change {
    Any,
    Fields(u64),
    Rows(Vec<DeltaRow>, Vec<u128>),
}

struct Changes(Vec<Option<Change>>);

impl TableChanges for Changes {
    fn table(&self, table: TableId) -> TableChange<'_> {
        match &self.0[table.index()] {
            None => TableChange::Same,
            Some(Change::Any) => TableChange::Any,
            Some(Change::Fields(fields)) => TableChange::Fields(*fields),
            Some(Change::Rows(changed, deleted)) => TableChange::Rows(changed, deleted),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Join {
    Combat(u8, u64),
    Naval(u64),
}

struct Side {
    leader: u64,
    join: Join,
    side: u8,
    hour: i32,
    active: bool,
}

type CombatSets = [Option<Arc<Vec<u64>>>; 2];
type SharedCensus = [Option<(i32, Arc<Vec<u64>>)>; 2];

fn changed_rows<'a, T: replay_data::rows::TableSchema>(
    frame: &'a DecodedFrame,
    changes: &dyn TableChanges,
) -> Vec<&'a T::Row> {
    let rows = frame.complete::<T>().unwrap_or_default();
    match changes.table(T::ID) {
        // A held table is told changed by fields only where none it holds changed.
        TableChange::Same | TableChange::Fields(_) => Vec::new(),
        TableChange::Rows(changed, _) => changed
            .iter()
            .filter_map(|row| {
                rows.binary_search_by_key(&row.key, RowCodec::key)
                    .ok()
                    .map(|at| &rows[at])
            })
            .collect(),
        _ => rows.iter().collect(),
    }
}

enum Match {
    Land {
        combat: u64,
        end: i32,
        read: i32,
        winners: u8,
    },
    Source {
        result: u64,
        source: NavalSource,
    },
    Naval {
        result: u64,
        winner: u8,
        read: i32,
    },
    Sides {
        read: i32,
        before: Option<SharedCensus>,
        sides: Vec<Side>,
    },
}

struct CommandRead {
    leader: u64,
    formation: u64,
    kind: LeaderFormationKind,
    name: Option<String>,
    first: i32,
    last: i32,
    /// Its place among the hour's command reads: its formation's table and row key.
    order: (u8, u128),
}

/// Extends the command read of `leader`'s `formation` that the hour before read with the same
/// kind and name, or begins one placed at `order` among the hour's reads; its index.
#[allow(clippy::too_many_arguments)]
fn read_command(
    reads: &mut Vec<CommandRead>,
    last: &mut HashMap<(u64, u64), usize>,
    leader: u64,
    formation: u64,
    kind: LeaderFormationKind,
    name: Option<&str>,
    hour: i32,
    order: (u8, u128),
) -> usize {
    let same = last.get(&(leader, formation)).copied().filter(|&at| {
        let read = &reads[at];
        read.last.checked_add(1) == Some(hour) && read.kind == kind && read.name.as_deref() == name
    });
    let at = same.unwrap_or_else(|| {
        reads.push(CommandRead {
            leader,
            formation,
            kind,
            name: name.map(str::to_owned),
            first: hour,
            last: hour,
            order,
        });
        reads.len() - 1
    });
    reads[at].last = hour;
    last.insert((leader, formation), at);
    at
}

/// Where each roster leader's own reads are found: its handle, its character and the tokens
/// naming it, its template and its training.
#[derive(Default)]
struct Roster {
    /// The roster's leaders by handle, each at its state.
    members: HashMap<u64, usize>,
    characters: HashMap<u32, BTreeSet<usize>>,
    tokens: HashMap<u32, BTreeSet<usize>>,
    /// Each leader's character and tokens as indexed.
    reads: HashMap<usize, (u32, Vec<u32>)>,
}

impl Roster {
    fn clear(&mut self) {
        self.members.clear();
        self.characters.clear();
        self.tokens.clear();
        self.reads.clear();
    }

    fn forget(&mut self, at: usize) {
        let Some((character, tokens)) = self.reads.remove(&at) else {
            return;
        };
        if let Some(users) = self.characters.get_mut(&character) {
            users.remove(&at);
            if users.is_empty() {
                self.characters.remove(&character);
            }
        }
        for token in tokens {
            if let Some(users) = self.tokens.get_mut(&token) {
                users.remove(&at);
                if users.is_empty() {
                    self.tokens.remove(&token);
                }
            }
        }
    }

    fn index(
        &mut self,
        at: usize,
        row: &UnitLeaderRow,
        characters: &[CharacterRow],
        training: &[UnitLeaderTraitExperienceRow],
    ) {
        self.forget(at);
        let reference = handle(row);
        self.members.insert(reference, at);
        let mut tokens = Vec::new();
        if let Ok(found) =
            characters.binary_search_by_key(&row.character_id, |character| character.id)
        {
            let character = &characters[found];
            if let Ok(token) = u32::try_from(character.name_token_id) {
                tokens.push(token);
            }
            if character.template_present != 0 {
                tokens.push(character.template_tag);
            }
        }
        tokens.extend(
            training_rows(training, reference)
                .iter()
                .map(|entry| entry.token_id as u32),
        );
        tokens.sort_unstable();
        tokens.dedup();
        self.characters
            .entry(row.character_id)
            .or_default()
            .insert(at);
        for &token in &tokens {
            self.tokens.entry(token).or_default().insert(at);
        }
        self.reads.insert(at, (row.character_id, tokens));
    }

    /// Adds to `dirty` the roster leaders whose own reads include `table`'s row `key`.
    fn readers(&self, table: TableId, key: u128, dirty: &mut BTreeSet<usize>) {
        let users = match table {
            TableId::Characters | TableId::CharacterPortraits => self.characters.get(&(key as u32)),
            TableId::LexicalTokens => self.tokens.get(&(key as u32)),
            _ => {
                // Assigned traits and training lead their key with their leader's handle.
                let shift = if table == TableId::UnitLeaderTraits {
                    32
                } else {
                    33
                };
                dirty.extend(self.members.get(&((key >> shift) as u64)));
                None
            }
        };
        if let Some(users) = users {
            dirty.extend(users);
        }
    }
}

/// Which leaders' visibility evaluations read what: each distinct declared read with the
/// leaders whose evaluation reads it, so a change checks again only those evaluations.
#[derive(Default)]
struct Registry {
    readers: HashMap<TableRead, BTreeSet<usize>>,
    /// The distinct reads of each table.
    tables: HashMap<TableId, Vec<TableRead>>,
    /// The distinct reads comparing the hour with row end hours, which change with the hour.
    ending: Vec<TableRead>,
    /// Each registered leader's reads.
    of: HashMap<usize, Vec<TableRead>>,
    /// Leaders whose evaluation read an input that declares no dependence.
    undeclared: BTreeSet<usize>,
}

impl Registry {
    fn clear(&mut self) {
        self.readers.clear();
        self.tables.clear();
        self.ending.clear();
        self.of.clear();
        self.undeclared.clear();
    }

    fn remove(&mut self, at: usize) {
        self.undeclared.remove(&at);
        let Some(reads) = self.of.remove(&at) else {
            return;
        };
        for read in reads {
            let Some(users) = self.readers.get_mut(&read) else {
                continue;
            };
            users.remove(&at);
            if users.is_empty() {
                self.readers.remove(&read);
                if let Some(held) = self.tables.get_mut(&read.table) {
                    held.retain(|other| *other != read);
                }
                if read.ends {
                    self.ending.retain(|other| *other != read);
                }
            }
        }
    }

    fn insert(&mut self, at: usize, inputs: &Inputs) {
        self.remove(at);
        let mut reads: Vec<TableRead> = Vec::new();
        for key in inputs.keys() {
            match key.dependence() {
                Some(declared) => {
                    for read in declared {
                        if !reads.contains(&read) {
                            reads.push(read);
                        }
                    }
                }
                None => {
                    self.undeclared.insert(at);
                }
            }
        }
        for &read in &reads {
            let users = self.readers.entry(read).or_default();
            if users.is_empty() {
                self.tables.entry(read.table).or_default().push(read);
                if read.ends {
                    self.ending.push(read);
                }
            }
            users.insert(at);
        }
        self.of.insert(at, reads);
    }

    /// The registered leaders whose evaluation may read differently than at the hour before:
    /// a declared read changed, an ongoing relation it compares ended, or an input declares no
    /// dependence.
    fn suspects(&self, changes: &dyn TableChanges, since: Option<&Since<'_>>) -> BTreeSet<usize> {
        let Some(since) = since else {
            return self.of.keys().copied().collect();
        };
        let mut suspects = self.undeclared.clone();
        for (&table, reads) in &self.tables {
            if matches!(changes.table(table), TableChange::Same) {
                continue;
            }
            for read in reads {
                if since.changed(read) {
                    suspects.extend(&self.readers[read]);
                }
            }
        }
        for read in &self.ending {
            if since.changed(read) {
                suspects.extend(&self.readers[read]);
            }
        }
        suspects
    }
}

/// The formation tables commands are bound from, in the order an hour binds them.
const GROUPS: u8 = 0;
const FLEETS: u8 = 1;
const MISSIONS: u8 = 2;

/// A formation bound to a visible leader, with the command read its binding extends.
struct Bound {
    leader: u64,
    formation: u64,
    kind: LeaderFormationKind,
    name: Option<String>,
    read: usize,
}

impl Bound {
    fn same(&self, other: &Self) -> bool {
        (self.leader, self.formation, &self.kind, &self.name)
            == (other.leader, other.formation, &other.kind, &other.name)
    }
}

/// The formations bound to leaders as the last hour read them, by table and row key.
#[derive(Default)]
struct Commands {
    bound: BTreeMap<(u8, u128), Bound>,
    /// The formations naming each leader, by handle, and the leader each formation names.
    named: HashMap<u64, BTreeSet<(u8, u128)>>,
    names: HashMap<(u8, u128), u64>,
    /// How many bound formations each leader and formation id has, and how many of those
    /// pairs are bound twice: an hour then binds every formation again, as each repeats.
    pairs: HashMap<(u64, u64), u32>,
    repeats: usize,
    /// Fleets whose commander capture was refused.
    refused: BTreeSet<u128>,
    /// Each mission fleet's commander, None where its missions conflict, and whether every
    /// mission joined one fleet.
    missions: Option<(BTreeMap<u64, Option<u64>>, bool)>,
    /// What the last read read commands from: orders groups, fleets with their leaders,
    /// missions.
    mode: Option<(bool, bool, bool)>,
}

impl Commands {
    fn name(&mut self, key: (u8, u128), leader: Option<u64>) {
        if let Some(old) = self.names.remove(&key)
            && let Some(keys) = self.named.get_mut(&old)
        {
            keys.remove(&key);
            if keys.is_empty() {
                self.named.remove(&old);
            }
        }
        if let Some(leader) = leader {
            self.names.insert(key, leader);
            self.named.entry(leader).or_default().insert(key);
        }
    }

    fn open(
        &mut self,
        key: (u8, u128),
        mut bound: Bound,
        reads: &mut Vec<CommandRead>,
        last: &mut HashMap<(u64, u64), usize>,
        hour: i32,
    ) {
        bound.read = read_command(
            reads,
            last,
            bound.leader,
            bound.formation,
            bound.kind.clone(),
            bound.name.as_deref(),
            hour,
            key,
        );
        let count = self
            .pairs
            .entry((bound.leader, bound.formation))
            .or_default();
        *count += 1;
        if *count == 2 {
            self.repeats += 1;
        }
        self.bound.insert(key, bound);
    }

    /// Ends formation `key`'s binding, its read last read at `through`.
    fn close(&mut self, key: (u8, u128), reads: &mut [CommandRead], through: i32) {
        let Some(bound) = self.bound.remove(&key) else {
            return;
        };
        reads[bound.read].last = through;
        let pair = (bound.leader, bound.formation);
        if let Some(count) = self.pairs.get_mut(&pair) {
            if *count == 2 {
                self.repeats -= 1;
            }
            *count -= 1;
            if *count == 0 {
                self.pairs.remove(&pair);
            }
        }
    }

    /// Ends every binding, each read last read at `through`.
    fn close_all(&mut self, reads: &mut [CommandRead], through: Option<i32>) {
        if let Some(through) = through {
            for bound in self.bound.values() {
                reads[bound.read].last = through;
            }
        }
        self.bound.clear();
        self.pairs.clear();
        self.repeats = 0;
    }

    /// Whether binding `planned` would bind a leader's formation id twice.
    fn repeating(&self, planned: &[((u8, u128), Option<Bound>)]) -> bool {
        let mut counts: HashMap<(u64, u64), i64> = HashMap::new();
        for (key, bound) in planned {
            if let Some(old) = self.bound.get(key) {
                *counts.entry((old.leader, old.formation)).or_default() -= 1;
            }
            if let Some(new) = bound {
                *counts.entry((new.leader, new.formation)).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .any(|(pair, delta)| i64::from(self.pairs.get(&pair).copied().unwrap_or(0)) + delta > 1)
    }
}

/// What binding a formation reads: the hour's formations and the leaders visible at it.
struct Binder<'a> {
    groups: &'a [OrdersGroupRow],
    plans: &'a [OrdersGroupPlanRow],
    fleets: &'a [FleetRow],
    live: &'a BTreeSet<usize>,
    leaders: &'a [LeaderState],
    indices: &'a HashMap<u64, usize>,
    content: Option<&'a Content>,
}

type FormationNames = HashMap<u64, (Option<Vec<u8>>, Option<String>)>;

impl Binder<'_> {
    fn visible(&self, reference: u64) -> Option<&LeaderState> {
        let &index = self.indices.get(&reference)?;
        self.live.contains(&index).then(|| &self.leaders[index])
    }

    /// An orders group's binding, as the roster's command read reads it.
    fn group(
        &self,
        group: &OrdersGroupRow,
        names: &mut FormationNames,
        refusals: &mut catalogue::Refusals,
    ) -> Option<Bound> {
        let state = self.visible(group.leader)?;
        let raw = match &group.name {
            Recorded::Value(Some(bytes)) => Some(bytes.as_slice()),
            Recorded::NotRecorded => self
                .plans
                .binary_search_by_key(&group.handle, |plan| plan.group)
                .ok()
                .map(|at| self.plans[at].name.as_slice()),
            Recorded::Value(None) => {
                refusals.note(
                    "Stored army command name capture refused",
                    group.handle.to_string(),
                );
                None
            }
        };
        let name = formation_name(names, group.handle, raw, self.content, refusals);
        Some(Bound {
            leader: state.reference,
            formation: group.handle,
            kind: if group.is_army_group != 0 {
                LeaderFormationKind::ArmyGroup
            } else {
                LeaderFormationKind::Army
            },
            name: name.map(str::to_owned),
            read: 0,
        })
    }

    /// A fleet's commander, None where its capture was refused, and its binding.
    fn fleet(
        &self,
        fleet: &FleetRow,
        names: &mut FormationNames,
        refusals: &mut catalogue::Refusals,
    ) -> (Option<u64>, Option<Bound>) {
        let Recorded::Value(Some(commander)) = fleet.leader else {
            refusals.note(
                "Fleet commander reference capture refused",
                fleet.id.to_string(),
            );
            return (None, None);
        };
        let Some(state) = self.visible(commander).filter(|state| {
            state
                .last_read
                .as_ref()
                .is_some_and(|last| last.row.kind == engine_native::cunit_leader::KIND_NAVY)
        }) else {
            return (Some(commander), None);
        };
        let raw = match &fleet.name {
            Recorded::Value(Some(bytes)) => Some(bytes.as_slice()),
            _ => {
                refusals.note("Stored fleet name unavailable", fleet.id.to_string());
                None
            }
        };
        let name = formation_name(names, fleet.id, raw, self.content, refusals);
        (
            Some(commander),
            Some(Bound {
                leader: state.reference,
                formation: fleet.id,
                kind: LeaderFormationKind::Fleet,
                name: name.map(str::to_owned),
                read: 0,
            }),
        )
    }

    /// A mission fleet's binding to the commander its missions name.
    fn mission(&self, fleet: u64, commander: Option<u64>) -> Option<Bound> {
        let state = self.visible(commander?)?;
        state
            .last_read
            .as_ref()
            .is_some_and(|last| last.row.kind == engine_native::cunit_leader::KIND_NAVY)
            .then(|| Bound {
                leader: state.reference,
                formation: fleet,
                kind: LeaderFormationKind::Fleet,
                name: None,
                read: 0,
            })
    }
}

/// Each fleet the hour's missions join, with the commander they name (None where two
/// conflict), and whether every mission joined a fleet.
fn mission_fleets(
    frame: &DecodedFrame,
    refusals: &mut catalogue::Refusals,
) -> (BTreeMap<u64, Option<u64>>, bool) {
    let mut fleets = BTreeMap::<u64, Option<u64>>::new();
    let mut joined = true;
    let (Ok(missions), Ok(tasks)) = (
        frame.complete::<tables::NavalMissionTaskForces>(),
        frame.complete::<tables::TaskForces>(),
    ) else {
        return (fleets, joined);
    };
    for mission in missions {
        let Some(task) = tasks
            .binary_search_by_key(&mission.task_force, |task| task.id)
            .ok()
            .map(|index| &tasks[index])
        else {
            joined = false;
            continue;
        };
        if task.fleet == 0 {
            joined = false;
            continue;
        }
        let commander = fleets.entry(task.fleet).or_insert(Some(mission.leader));
        if *commander != Some(mission.leader) {
            *commander = None;
            joined = false;
            refusals.note(
                "Conflicting recorded fleet commander references",
                task.fleet.to_string(),
            );
        }
    }
    (fleets, joined)
}

/// The hour's command inputs ([`Work::bind_commands`]).
pub(super) struct CommandHour<'a> {
    pub(super) frame: &'a DecodedFrame,
    pub(super) changes: &'a dyn TableChanges,
    pub(super) hour: i32,
    /// Whether the hour reads every leader.
    pub(super) full: bool,
    /// Whether the orders groups are read.
    pub(super) land: bool,
    /// The fleets, read with every leader recorded.
    pub(super) fleets: Option<&'a [FleetRow]>,
    /// Whether the missions are read for fleets that do not record their leader.
    pub(super) missions: bool,
}

#[derive(Default)]
pub(super) struct Work {
    pub(super) role_keys: Vec<String>,
    pub(super) person_warnings: HashMap<u32, Vec<String>>,
    pub(super) first_visible: HashMap<u64, (i32, [Option<i32>; 2], u8)>,
    pub(super) owner_read: bool,
    pub(super) resolved: usize,
    /// Leader walks over every leader, and over the changed leaders alone.
    walks: [u64; 2],
    /// Hours carried with nothing read, and hours reading only combats and results.
    quiet: u64,
    battle_hours: u64,
    /// Evaluations whose declared reads changed, and of those, the ones confirmed unread.
    suspects: u64,
    confirmed: u64,
    /// Runs begun, and hours every command was bound anew.
    pub(super) runs: u64,
    rebuilds: u64,
    /// Hours the leader reads were discarded at: an earlier segment's role is reused only by
    /// runs before them.
    invalidations: Vec<i32>,
    valid: Option<bool>,
    matches: Vec<Match>,
    land: BTreeSet<(u64, i32, u8)>,
    sources: BTreeMap<u64, NavalSource>,
    naval: BTreeMap<u64, u8>,
    sides: HashMap<(u64, Join, Option<(u64, i32)>), u8>,
    battles_read: bool,
    pub(super) live: BTreeSet<usize>,
    /// Whether the `live` leaders' careers run on through the frontier, unwritten until sealed.
    pub(super) lazy: bool,
    /// The roster leaders whose owner or visibility is unknown: coverage holds while none is.
    pub(super) failing: BTreeSet<usize>,
    /// The leaders read this hour, and those whose formations may bind differently since.
    pub(super) read: Vec<usize>,
    pub(super) relive: BTreeSet<u64>,
    /// Each kind's combat availability and whether assigned traits were read, as last read.
    reads: Option<([bool; 2], bool)>,
    scoped: BTreeSet<u128>,
    roster: Roster,
    registry: Registry,
    commands: Commands,
    before: Option<i32>,
    command_reads: Vec<CommandRead>,
    command_last: HashMap<(u64, u64), usize>,
    /// Leaders visible this hour and not the hour before: their ongoing combats and results
    /// are joined again.
    pub(super) gained: BTreeSet<u64>,
    active: [BTreeMap<u64, [u64; 2]>; 2],
    active_by_leader: HashMap<u64, BTreeSet<(usize, u64)>>,
    sets: CombatSets,
    previous: SharedCensus,
    results_recorded: bool,
    new_sources: BTreeSet<u64>,
    /// The hour `new_sources` were found at.
    results_at: Option<i32>,
    result_reads: [bool; 2],
    /// Whether the active combats are unknown since a roster that could not be read.
    battle_stale: bool,
    witnesses: HashMap<(TableId, u128), [u64; 2]>,
    witnesses_by_leader: HashMap<u64, BTreeSet<(TableId, u128)>>,
}

impl Work {
    fn witness(&mut self, table: TableId, key: u128, leaders: Option<[u64; 2]>) {
        if let Some(old) = self.witnesses.remove(&(table, key)) {
            for leader in old {
                if let Some(held) = self.witnesses_by_leader.get_mut(&leader) {
                    held.remove(&(table, key));
                }
            }
        }
        if let Some(leaders) = leaders {
            for leader in leaders {
                self.witnesses_by_leader
                    .entry(leader)
                    .or_default()
                    .insert((table, key));
            }
            self.witnesses.insert((table, key), leaders);
        }
    }

    fn index_witnesses(&mut self, frame: &DecodedFrame, changes: &dyn TableChanges) {
        for table in [
            TableId::LandCombatResults,
            TableId::NavalCombatResultDetails,
        ] {
            let deleted = match changes.table(table) {
                TableChange::Same | TableChange::Fields(_) => continue,
                TableChange::Rows(_, deleted) => deleted.to_vec(),
                _ => self
                    .witnesses
                    .keys()
                    .filter(|(held, _)| *held == table)
                    .map(|(_, key)| *key)
                    .collect(),
            };
            for key in deleted {
                self.witness(table, key, None);
            }
            if table == TableId::LandCombatResults {
                for row in changed_rows::<tables::LandCombatResults>(frame, changes) {
                    self.witness(
                        table,
                        row.key(),
                        Some(row.sides.each_ref().map(|side| side.commander)),
                    );
                }
            } else {
                for row in changed_rows::<tables::NavalCombatResultDetails>(frame, changes) {
                    self.witness(
                        table,
                        row.key(),
                        Some(row.sides.each_ref().map(|side| side.leader)),
                    );
                }
            }
        }
    }
    pub(super) fn advance(&mut self, hour: i32) {
        for (at, _) in self.previous.iter_mut().flatten() {
            *at = hour;
        }
    }

    fn finish_commands(&mut self, through: i32) {
        for bound in self.commands.bound.values() {
            self.command_reads[bound.read].last = through;
        }
    }

    /// An hour whose roster could not be read binds no command: each binding ends at the
    /// hour before, and the next read binds every formation again.
    pub(super) fn drop_commands(&mut self) {
        self.commands
            .close_all(&mut self.command_reads, self.before);
        self.commands.mode = None;
        self.relive.clear();
    }

    /// The hour's command reads: each formation bound to a visible leader extends its read,
    /// as binding every formation in table order does. Only the formations whose rows changed
    /// or whose leader's visibility or kind changed are bound again, unless the hour reads
    /// every leader, what commands are read from changed, or a leader's formation id is bound
    /// twice. Whether the land and naval commands were covered.
    pub(super) fn bind_commands(
        &mut self,
        input: CommandHour<'_>,
        leaders: &[LeaderState],
        indices: &HashMap<u64, usize>,
        names: &mut FormationNames,
        content: Option<&Content>,
        refusals: &mut catalogue::Refusals,
    ) -> [bool; 2] {
        let Self {
            live,
            relive,
            commands,
            command_reads,
            command_last,
            before,
            rebuilds,
            ..
        } = self;
        let relive = std::mem::take(relive);
        let changes = input.changes;
        let hour = input.hour;
        let any = |table: TableId| matches!(changes.table(table), TableChange::Any);
        let mode = (input.land, input.fleets.is_some(), input.missions);
        let binder = Binder {
            groups: if mode.0 {
                input
                    .frame
                    .complete::<tables::OrdersGroups>()
                    .unwrap_or_default()
            } else {
                &[]
            },
            plans: input
                .frame
                .complete::<tables::OrdersGroupPlans>()
                .unwrap_or_default(),
            fleets: input.fleets.unwrap_or_default(),
            live,
            leaders,
            indices,
            content,
        };
        let mut rebuild = input.full
            || commands.mode != Some(mode)
            || commands.repeats > 0
            || mode.0 && (any(TableId::OrdersGroups) || any(TableId::OrdersGroupPlans))
            || mode.1 && any(TableId::Fleets);
        commands.mode = Some(mode);
        if !rebuild {
            let mut touched = BTreeSet::new();
            let keys = |table: TableId, kind: u8, touched: &mut BTreeSet<(u8, u128)>| {
                if let TableChange::Rows(changed, deleted) = changes.table(table) {
                    touched.extend(changed.iter().map(|row| (kind, row.key)));
                    touched.extend(deleted.iter().map(|&key| (kind, key)));
                }
            };
            if mode.0 {
                // A plan's key is its group's handle, the group's key.
                keys(TableId::OrdersGroups, GROUPS, &mut touched);
                keys(TableId::OrdersGroupPlans, GROUPS, &mut touched);
            }
            if mode.1 {
                keys(TableId::Fleets, FLEETS, &mut touched);
            }
            if mode.2
                && [TableId::NavalMissionTaskForces, TableId::TaskForces]
                    .into_iter()
                    .any(|table| {
                        !matches!(
                            changes.table(table),
                            TableChange::Same | TableChange::Fields(_)
                        )
                    })
            {
                let (fleets, joined) = mission_fleets(input.frame, refusals);
                let old = commands
                    .missions
                    .take()
                    .map(|(fleets, _)| fleets)
                    .unwrap_or_default();
                for (&fleet, commander) in &fleets {
                    if old.get(&fleet) != Some(commander) {
                        touched.insert((MISSIONS, u128::from(fleet)));
                    }
                }
                for &fleet in old.keys() {
                    if !fleets.contains_key(&fleet) {
                        touched.insert((MISSIONS, u128::from(fleet)));
                    }
                }
                commands.missions = Some((fleets, joined));
            }
            for leader in &relive {
                if let Some(named) = commands.named.get(leader) {
                    touched.extend(named);
                }
            }
            let mut planned = Vec::new();
            for key in touched {
                let bound = match key.0 {
                    GROUPS => {
                        let group = binder
                            .groups
                            .binary_search_by_key(&key.1, RowCodec::key)
                            .ok()
                            .map(|at| &binder.groups[at]);
                        commands.name(key, group.map(|group| group.leader));
                        group.and_then(|group| binder.group(group, names, refusals))
                    }
                    FLEETS => {
                        let fleet = binder
                            .fleets
                            .binary_search_by_key(&key.1, RowCodec::key)
                            .ok()
                            .map(|at| &binder.fleets[at]);
                        let (commander, bound) = match fleet {
                            Some(fleet) => binder.fleet(fleet, names, refusals),
                            None => (None, None),
                        };
                        if fleet.is_some() && commander.is_none() {
                            commands.refused.insert(key.1);
                        } else {
                            commands.refused.remove(&key.1);
                        }
                        commands.name(key, commander);
                        bound
                    }
                    _ => {
                        let fleet = key.1 as u64;
                        let commander = commands
                            .missions
                            .as_ref()
                            .and_then(|(fleets, _)| fleets.get(&fleet).copied())
                            .flatten();
                        commands.name(key, commander);
                        binder.mission(fleet, commander)
                    }
                };
                let unchanged = match (commands.bound.get(&key), &bound) {
                    (Some(held), Some(bound)) => held.same(bound),
                    (None, None) => true,
                    _ => false,
                };
                if !unchanged {
                    planned.push((key, bound));
                }
            }
            if commands.repeating(&planned) {
                rebuild = true;
            } else {
                for (key, bound) in planned {
                    commands.close(key, command_reads, hour - 1);
                    if let Some(bound) = bound {
                        commands.open(key, bound, command_reads, command_last, hour);
                    }
                }
            }
        }
        if rebuild {
            *rebuilds += 1;
            commands.close_all(command_reads, *before);
            commands.names.clear();
            commands.named.clear();
            commands.refused.clear();
            commands.missions = None;
            let mut planned = Vec::new();
            for group in binder.groups {
                let key = (GROUPS, group.key());
                commands.name(key, Some(group.leader));
                if let Some(bound) = binder.group(group, names, refusals) {
                    planned.push((key, bound));
                }
            }
            for fleet in binder.fleets {
                let key = (FLEETS, fleet.key());
                let (commander, bound) = binder.fleet(fleet, names, refusals);
                if commander.is_none() {
                    commands.refused.insert(key.1);
                }
                commands.name(key, commander);
                planned.extend(bound.map(|bound| (key, bound)));
            }
            if mode.2 {
                let (fleets, joined) = mission_fleets(input.frame, refusals);
                for (&fleet, &commander) in &fleets {
                    let key = (MISSIONS, u128::from(fleet));
                    commands.name(key, commander);
                    planned.extend(binder.mission(fleet, commander).map(|bound| (key, bound)));
                }
                commands.missions = Some((fleets, joined));
            }
            for (key, bound) in planned {
                commands.open(key, bound, command_reads, command_last, hour);
            }
        }
        let naval = if mode.1 {
            commands.refused.is_empty()
        } else {
            mode.2
                && commands
                    .missions
                    .as_ref()
                    .is_some_and(|(_, joined)| *joined)
        };
        [input.land, naval]
    }

    pub(super) fn combats_changed(
        &self,
        frame: &DecodedFrame,
        format: FormatVersion,
        changes: &dyn TableChanges,
    ) -> bool {
        let (land, navy) = battle_tables(format, frame);
        for (kind, table, available) in [
            (0, TableId::LandCombatDetails, land.is_some()),
            (1, TableId::NavalCombatDetails, navy.is_some()),
        ] {
            if available != self.sets[kind].is_some() {
                return true;
            }
            match changes.table(table) {
                TableChange::Same => {}
                TableChange::Fields(_) if available => {}
                TableChange::Rows(_, deleted) if available => {
                    if !deleted.is_empty() {
                        return true;
                    }
                    let changed = if kind == 0 {
                        changed_rows::<tables::LandCombatDetails>(frame, changes)
                            .into_iter()
                            .any(|row| {
                                self.active[kind].get(&row.combat)
                                    != Some(&row.sides.each_ref().map(|side| side.commander))
                            })
                    } else {
                        changed_rows::<tables::NavalCombatDetails>(frame, changes)
                            .into_iter()
                            .any(|row| {
                                self.active[kind].get(&row.combat)
                                    != Some(&row.sides.each_ref().map(|side| side.commander))
                            })
                    };
                    if changed {
                        return true;
                    }
                }
                _ => return true,
            }
        }
        false
    }
    fn handoff(&mut self) -> Self {
        Self {
            role_keys: std::mem::take(&mut self.role_keys),
            person_warnings: std::mem::take(&mut self.person_warnings),
            first_visible: std::mem::take(&mut self.first_visible),
            owner_read: self.owner_read,
            resolved: self.resolved,
            invalidations: std::mem::take(&mut self.invalidations),
            matches: std::mem::take(&mut self.matches),
            battles_read: self.battles_read,
            previous: self.previous.clone(),
            results_recorded: self.results_recorded,
            command_reads: std::mem::take(&mut self.command_reads),
            ..Self::default()
        }
    }

    pub(super) fn scoped(&mut self, frame: &DecodedFrame, changes: &dyn TableChanges) -> bool {
        let rows = frame.complete::<tables::UnitLeaders>().unwrap_or_default();
        match changes.table(TableId::UnitLeaders) {
            TableChange::Same => {}
            TableChange::Rows(changed, deleted) => {
                for key in deleted {
                    self.scoped.remove(key);
                }
                for row in changed {
                    if let Ok(at) = rows.binary_search_by_key(&row.key, RowCodec::key) {
                        let row = &rows[at];
                        if row.visibility_choice != 0 || row.template_visibility != 0 {
                            self.scoped.insert(row.key());
                        } else {
                            self.scoped.remove(&row.key());
                        }
                    }
                }
            }
            _ => {
                self.scoped.clear();
                self.scoped.extend(
                    rows.iter()
                        .filter(|row| row.visibility_choice != 0 || row.template_visibility != 0)
                        .map(RowCodec::key),
                );
            }
        }
        !self.scoped.is_empty()
    }

    pub(super) fn validity(&mut self, hour: i32, valid: bool, changed: bool) {
        if (changed || !valid && self.valid != Some(false))
            && self.invalidations.last() != Some(&hour)
        {
            self.invalidations.push(hour);
        }
        self.valid = Some(valid);
        if !valid {
            self.battle_stale = true;
        }
    }

    pub(super) fn results(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        changes: &dyn TableChanges,
    ) {
        self.index_witnesses(frame, changes);
        let at = frame.hour;
        let land = slot_read(format, &frame.metrics, TableId::LandCombatResults, at)
            && frame.complete::<tables::LandCombatResults>().is_ok();
        if land {
            self.results_recorded = true;
            let rows = if self.result_reads[0] {
                changed_rows::<tables::LandCombatResults>(frame, changes)
            } else {
                frame
                    .complete::<tables::LandCombatResults>()
                    .unwrap_or_default()
                    .iter()
                    .collect()
            };
            for row in rows {
                if self.land.insert((row.combat, row.end_hour, row.winners)) {
                    self.matches.push(Match::Land {
                        combat: row.combat,
                        end: row.end_hour,
                        read: at,
                        winners: row.winners,
                    });
                }
            }
        }
        // An hour observed again after a kept frame stopped it keeps the sources it found.
        if self.results_at.replace(at) != Some(at) {
            self.new_sources.clear();
        }
        let sources = slot_read(format, &frame.metrics, TableId::NavalResultSources, at)
            && frame.complete::<tables::NavalResultSources>().is_ok();
        if sources {
            self.results_recorded = true;
            let rows = if self.result_reads[1] {
                changed_rows::<tables::NavalResultSources>(frame, changes)
            } else {
                frame
                    .complete::<tables::NavalResultSources>()
                    .unwrap_or_default()
                    .iter()
                    .collect()
            };
            for row in rows {
                if row.transient || row.merged {
                    continue;
                }
                let source = NavalSource {
                    combat: row.combat,
                    game_hour: row.game_hour,
                };
                if self.sources.insert(row.result, source) != Some(source) {
                    self.naval.remove(&row.result);
                    self.new_sources.insert(row.result);
                    self.matches.push(Match::Source {
                        result: row.result,
                        source,
                    });
                }
            }
        }
        self.result_reads = [land, sources];
        let mut naval: BTreeSet<_> = changed_rows::<tables::NavalResults>(frame, changes)
            .into_iter()
            .map(|row| row.id)
            .collect();
        naval.extend(&self.new_sources);
        let rows = frame.complete::<tables::NavalResults>().unwrap_or_default();
        for result in naval {
            if let Ok(at) = rows.binary_search_by_key(&result, |row| row.id) {
                let row = &rows[at];
                if self.naval.insert(row.id, row.winning_side) != Some(row.winning_side) {
                    self.matches.push(Match::Naval {
                        result: row.id,
                        winner: row.winning_side,
                        read: frame.hour,
                    });
                }
            }
        }
    }

    pub(super) fn battles(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        changes: &dyn TableChanges,
        indices: &HashMap<u64, usize>,
    ) {
        if self.battle_stale {
            self.active.iter_mut().for_each(BTreeMap::clear);
            self.active_by_leader.clear();
            self.sets = [None, None];
            self.battle_stale = false;
        }
        let mut touched = BTreeSet::new();
        let (land, navy) = battle_tables(format, frame);
        for (kind, table, available) in [
            (0, TableId::LandCombatDetails, land.is_some()),
            (1, TableId::NavalCombatDetails, navy.is_some()),
        ] {
            let change = changes.table(table);
            let mut edits = Vec::new();
            let mut deleted = Vec::new();
            if !available
                || !matches!(
                    change,
                    TableChange::Same | TableChange::Fields(_) | TableChange::Rows(..)
                )
            {
                deleted.extend(self.active[kind].keys().copied());
            } else if let TableChange::Rows(_, gone) = change {
                deleted.extend(gone.iter().map(|&key| key as u64));
            }
            if available {
                if kind == 0 {
                    edits.extend(
                        (if self.sets[kind].is_none() {
                            land.unwrap_or_default().iter().collect()
                        } else {
                            changed_rows::<tables::LandCombatDetails>(frame, changes)
                        })
                        .into_iter()
                        .map(|row| (row.combat, row.sides.each_ref().map(|side| side.commander))),
                    );
                } else {
                    edits.extend(
                        (if self.sets[kind].is_none() {
                            navy.unwrap_or_default().iter().collect()
                        } else {
                            changed_rows::<tables::NavalCombatDetails>(frame, changes)
                        })
                        .into_iter()
                        .map(|row| (row.combat, row.sides.each_ref().map(|side| side.commander))),
                    );
                }
            }
            let mut membership = !deleted.is_empty();
            for combat in deleted {
                if let Some(old) = self.active[kind].remove(&combat) {
                    for leader in old {
                        if let Some(held) = self.active_by_leader.get_mut(&leader) {
                            held.remove(&(kind, combat));
                        }
                    }
                }
            }
            for (combat, commanders) in edits {
                if self.active[kind].get(&combat) == Some(&commanders) {
                    continue;
                }
                if let Some(old) = self.active[kind].insert(combat, commanders) {
                    for leader in old {
                        if let Some(held) = self.active_by_leader.get_mut(&leader) {
                            held.remove(&(kind, combat));
                        }
                    }
                } else {
                    membership = true;
                }
                for leader in commanders {
                    self.active_by_leader
                        .entry(leader)
                        .or_default()
                        .insert((kind, combat));
                }
                touched.insert((kind, combat));
            }
            if !available {
                self.sets[kind] = None;
            } else if membership || self.sets[kind].is_none() {
                self.sets[kind] = Some(Arc::new(self.active[kind].keys().copied().collect()));
            }
        }
        for leader in &self.gained {
            if let Some(combats) = self.active_by_leader.get(leader) {
                touched.extend(combats);
            }
        }
        let mut candidates = Vec::new();
        for (kind, combat) in touched {
            if let Some(commanders) = self.active[kind].get(&combat) {
                for (at, &leader) in commanders.iter().enumerate() {
                    candidates.push(Side {
                        leader,
                        join: Join::Combat(kind as u8 + 1, combat),
                        side: 1 << at,
                        hour: frame.hour,
                        active: true,
                    });
                }
            }
        }
        let mut land_keys: BTreeSet<_> = changed_rows::<tables::LandCombatResults>(frame, changes)
            .into_iter()
            .map(RowCodec::key)
            .collect();
        let mut naval_keys: BTreeSet<_> =
            changed_rows::<tables::NavalCombatResultDetails>(frame, changes)
                .into_iter()
                .map(RowCodec::key)
                .collect();
        naval_keys.extend(self.new_sources.iter().map(|&result| u128::from(result)));
        for leader in &self.gained {
            for &(table, key) in self.witnesses_by_leader.get(leader).into_iter().flatten() {
                if table == TableId::LandCombatResults {
                    land_keys.insert(key);
                } else {
                    naval_keys.insert(key);
                }
            }
        }
        if slot_read(
            format,
            &frame.metrics,
            TableId::LandCombatResults,
            frame.hour,
        ) {
            let rows = frame
                .complete::<tables::LandCombatResults>()
                .unwrap_or_default();
            for key in land_keys {
                if let Ok(at) = rows.binary_search_by_key(&key, RowCodec::key) {
                    let row = &rows[at];
                    for (at, side) in row.sides.iter().enumerate() {
                        candidates.push(Side {
                            leader: side.commander,
                            join: Join::Combat(1, row.combat),
                            side: 1 << at,
                            hour: row.end_hour,
                            active: false,
                        });
                    }
                }
            }
        }
        let rows = frame
            .complete::<tables::NavalCombatResultDetails>()
            .unwrap_or_default();
        for key in naval_keys {
            if let Ok(at) = rows.binary_search_by_key(&key, RowCodec::key) {
                let row = &rows[at];
                for (at, side) in row.sides.iter().enumerate() {
                    candidates.push(Side {
                        leader: side.leader,
                        join: Join::Naval(row.result),
                        side: 1 << at,
                        hour: frame.hour,
                        active: false,
                    });
                }
            }
        }
        let mut sides = Vec::new();
        for mut side in candidates {
            let Some(&index) = indices.get(&side.leader) else {
                continue;
            };
            if !self.live.contains(&index) {
                continue;
            }
            let source = match side.join {
                Join::Naval(result) => self
                    .sources
                    .get(&result)
                    .map(|source| (source.combat, source.game_hour)),
                Join::Combat(..) => None,
            };
            let held = self
                .sides
                .entry((side.leader, side.join, source))
                .or_default();
            let added = side.side & !*held;
            *held |= side.side;
            if added != 0 {
                side.side = added;
                sides.push(side);
            }
        }
        if !sides.is_empty() {
            self.matches.push(Match::Sides {
                read: frame.hour,
                before: self.battles_read.then(|| self.previous.clone()),
                sides,
            });
        }
        self.gained.clear();
        self.battles_read = true;
        self.previous = self
            .sets
            .each_ref()
            .map(|set| set.as_ref().map(|set| (frame.hour, Arc::clone(set))));
    }
}

/// What a segment's projection read and did, for measuring it.
#[derive(Clone, Debug, Default)]
pub struct Counts {
    pub hours: u64,
    /// Rows read from the walker's columns, whole or by the fields held, by table.
    pub decoded: Vec<u64>,
    /// Hours observed again after a held frame could not give their verdicts.
    pub fallbacks: u64,
    /// Leader walks over every leader, and over the changed leaders alone.
    pub walks: [u64; 2],
    pub resolved: u64,
    pub invalidations: u64,
    /// Leaders whose visibility reads an input that declares no dependence, at its end.
    pub undeclared: u64,
    /// Hours carried with nothing read, and hours reading only combats and results.
    pub quiet: u64,
    pub battles: u64,
    /// Evaluations whose declared reads changed, and of those, the ones confirmed unread.
    pub suspects: u64,
    pub confirmed: u64,
    /// Runs the segment's leaders began.
    pub runs: u64,
    /// Hours every command was bound anew.
    pub rebuilds: u64,
}

impl Counts {
    pub fn add(&mut self, other: &Self) {
        self.hours += other.hours;
        if self.decoded.len() < other.decoded.len() {
            self.decoded.resize(other.decoded.len(), 0);
        }
        for (held, added) in self.decoded.iter_mut().zip(&other.decoded) {
            *held += added;
        }
        self.fallbacks += other.fallbacks;
        for (held, added) in self.walks.iter_mut().zip(other.walks) {
            *held += added;
        }
        self.resolved += other.resolved;
        self.invalidations += other.invalidations;
        self.undeclared += other.undeclared;
        self.quiet += other.quiet;
        self.battles += other.battles;
        self.suspects += other.suspects;
        self.confirmed += other.confirmed;
        self.runs += other.runs;
        self.rebuilds += other.rebuilds;
    }

    pub fn report(&self) -> serde_json::Value {
        let decoded: serde_json::Map<String, serde_json::Value> = TableId::ALL
            .iter()
            .zip(&self.decoded)
            .filter(|(_, rows)| **rows != 0)
            .map(|(table, rows)| (format!("{table:?}"), (*rows).into()))
            .collect();
        serde_json::json!({
            "hours": self.hours,
            "decoded_rows": decoded,
            "fallbacks": self.fallbacks,
            "full_walks": self.walks[0],
            "restricted_walks": self.walks[1],
            "quiet_hours": self.quiet,
            "battle_hours": self.battles,
            "leaders_resolved": self.resolved,
            "visibility_suspects": self.suspects,
            "visibility_confirmed": self.confirmed,
            "runs": self.runs,
            "command_rebuilds": self.rebuilds,
            "invalidations": self.invalidations,
            "undeclared_visibility": self.undeclared,
        })
    }
}

pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(super) first: i32,
    pub counts: Counts,
    scan: Scan,
    work: Work,
    pub(super) refusals: catalogue::OwnRefusals,
    pub(super) admissions: Vec<(u64, LeaderRole, Option<(u32, u32)>)>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        // Not counted: refusals, the combats' census (matches share it) and the name caches.
        let (scan, work) = (&self.scan, &self.work);
        let leaders: usize = scan
            .leaders
            .iter()
            .map(|state| {
                state.career.reference.capacity()
                    + state.career.runs.capacity() * size_of::<UnitLeaderRun>()
            })
            .sum();
        let runs: usize = scan
            .stored_runs
            .iter()
            .filter_map(|stored| stored.run.value.progress.as_ref())
            .map(progress_heap)
            .sum();
        let people: usize = scan
            .reply
            .people
            .iter()
            .map(|person| {
                [&person.portrait_reference]
                    .into_iter()
                    .chain(&person.token)
                    .chain(&person.name)
                    .map(String::capacity)
                    .sum::<usize>()
            })
            .sum();
        let names: usize = scan
            .people
            .keys()
            .chain(scan.trait_tokens.keys())
            .chain(&scan.token_list)
            .chain(scan.roles.keys())
            .chain(&work.role_keys)
            .map(String::capacity)
            .sum();
        let battles: usize = scan
            .battle_hours
            .values()
            .map(|hours| hours.capacity() * size_of::<(i32, i32, bool, u8, u64, u8)>())
            .sum();
        let sides: usize = work
            .matches
            .iter()
            .map(|matched| match matched {
                Match::Sides { sides, .. } => sides.capacity() * size_of::<Side>(),
                _ => 0,
            })
            .sum();
        let warnings: usize = work
            .person_warnings
            .values()
            .map(|warnings| {
                warnings.capacity() * size_of::<String>()
                    + warnings.iter().map(String::capacity).sum::<usize>()
            })
            .sum();
        scan.leaders.capacity() * size_of::<LeaderState>()
            + leaders
            + scan.stored_runs.capacity() * size_of::<StoredRun>()
            + runs
            + scan.run_progress.capacity() * size_of::<(usize, Arc<[u8]>)>()
            + scan
                .run_progress
                .values()
                .map(|progress| progress.len())
                .sum::<usize>()
            + scan.reply.people.capacity() * size_of::<LeaderPerson>()
            + people
            + scan.reply.roles.capacity() * size_of::<LeaderRole>()
            + scan.reply.roles.iter().map(role_heap).sum::<usize>()
            + scan.people.capacity() * size_of::<(String, u32)>()
            + scan.trait_tokens.capacity() * size_of::<(String, u32)>()
            + scan.token_list.capacity() * size_of::<String>()
            + scan.roles.capacity() * size_of::<(String, u32)>()
            + work.role_keys.capacity() * size_of::<String>()
            + names
            + scan.battle_hours.capacity() * size_of::<(u64, Vec<(i32, i32, bool, u8, u64, u8)>)>()
            + battles
            + scan.battle_indices.capacity() * size_of::<((u64, u8, u64), usize)>()
            + scan.outcomes.len() * size_of::<((u8, u64), Option<(i32, i32, u8)>)>()
            + scan.naval_sources.len() * size_of::<(u64, NavalSource)>()
            + work.matches.capacity() * size_of::<Match>()
            + sides
            + work.command_reads.capacity() * size_of::<CommandRead>()
            + work
                .command_reads
                .iter()
                .filter_map(|read| read.name.as_ref())
                .map(String::capacity)
                .sum::<usize>()
            + work.invalidations.capacity() * size_of::<i32>()
            + work.first_visible.capacity() * size_of::<(u64, (i32, [Option<i32>; 2], u8))>()
            + work.person_warnings.capacity() * size_of::<(u32, Vec<String>)>()
            + warnings
            + self.counts.decoded.capacity() * size_of::<u64>()
            + self.admissions.capacity() * size_of::<(u64, LeaderRole, Option<(u32, u32)>)>()
            + self
                .admissions
                .iter()
                .map(|(_, role, _)| role_heap(role))
                .sum::<usize>()
    }
}

/// The bytes a role's names own on the heap.
fn role_heap(role: &LeaderRole) -> usize {
    role.owner
        .iter()
        .chain(&role.rank)
        .chain(role.attribute_names.iter().flatten())
        .map(String::capacity)
        .sum()
}

/// The bytes a run's progress owns on the heap.
fn progress_heap(progress: &LeaderProgress) -> usize {
    let training = progress.training.as_ref().map_or(0, |training| {
        training.capacity() * size_of::<LeaderTraining>()
            + training
                .iter()
                .map(|step| {
                    [&step.experience_raw]
                        .into_iter()
                        .chain(&step.token)
                        .chain(&step.name)
                        .chain(&step.required_raw)
                        .map(String::capacity)
                        .sum::<usize>()
                })
                .sum::<usize>()
    });
    let factors = progress.factors.as_ref().map_or(0, |factors| {
        factors.capacity() * size_of::<LeaderTrainingFactor>()
            + factors
                .iter()
                .map(|factor| {
                    [&factor.factor_raw]
                        .into_iter()
                        .chain(&factor.token)
                        .chain(&factor.name)
                        .map(String::capacity)
                        .sum::<usize>()
                })
                .sum::<usize>()
    });
    progress.experience_raw.capacity()
        + progress
            .next_required_raw
            .as_ref()
            .map_or(0, String::capacity)
        + training
        + factors
}

pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    starts: BTreeMap<u64, i32>,
    content: Option<Arc<Content>>,
    holds: Vec<Option<Hold>>,
    held: Held,
    scan: Option<Scan>,
    first: Option<i32>,
    hours: u64,
    fallbacks: u64,
    refusals: catalogue::OwnRefusals,
    admissions: Vec<(u64, LeaderRole, Option<(u32, u32)>)>,
}

impl Projector {
    /// A segment's leaders, read as a run whose content binds as `catalogue` says reads its
    /// blocks ([`block::bound`]).
    pub fn new(replay: &Replay, content: Option<Arc<Content>>, catalogue: bool) -> Self {
        let mut starts = BTreeMap::<u64, i32>::new();
        for block in &replay.index {
            starts
                .entry(block.branch)
                .and_modify(|at| *at = (*at).min(block.first_hour))
                .or_insert(block.first_hour);
        }
        Self::starting(
            (replay.format, replay.capture_mode),
            starts,
            content,
            catalogue,
        )
    }

    /// [`Self::new`] of a recording whose branches' first recorded hours are `starts`.
    pub fn starting(
        (format, mode): (FormatVersion, CaptureMode),
        starts: BTreeMap<u64, i32>,
        content: Option<Arc<Content>>,
        catalogue: bool,
    ) -> Self {
        let holds = holds(catalogue);
        Self {
            format,
            mode,
            starts,
            content,
            held: Held::new(format, mode, holds.clone()),
            holds,
            scan: None,
            first: None,
            hours: 0,
            fallbacks: 0,
            refusals: Default::default(),
            admissions: Vec::new(),
        }
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        self.held = Held::new(self.format, self.mode, self.holds.clone());
        self.first = Some(first);
        let start = *self
            .starts
            .get(&branch)
            .ok_or_else(|| invalid("A leaders segment has no branch start"))?;
        let mut scan = Scan::at(self.format, branch, start);
        scan.reuse_evaluations = true;
        scan.projection = Some(Work::default());
        self.scan = Some(scan);
        Ok(())
    }

    fn hour(&mut self, hour: &segments::Hour<'_, World>, _: &mut dyn Sink<Hour>) -> io::Result<()> {
        if self.scan.is_none() {
            self.start(Start::Run { continues: false }, hour.branch, hour.hour)?;
        }
        self.hours += 1;
        let (mut frame, facts, changes) = {
            let _cpu = ::replay_frame::cpu::scope_phase("leaders:inputs");
            self.held.hour(hour)?
        };
        let _cpu = ::replay_frame::cpu::scope_phase("leaders:observe");
        let format = self.format;
        let content = self.content.as_deref();
        // Check mode reads every reused input again, which needs every table whole.
        if content.is_some() && check_reuse() && !self.held.whole {
            let whole = Wanted {
                reads: Vec::new(),
                whole: true,
            };
            self.held.widen(hour, &mut frame, &whole)?;
        }
        let scan = self.scan.as_mut().expect("a segment scan");
        let step = {
            let _cpu = ::replay_frame::cpu::scope_phase("leaders:carry_proof");
            let kept = self.held.kept();
            catalogue::own_refusals(&mut self.refusals, || {
                plan(
                    scan,
                    &frame,
                    format,
                    content,
                    &facts,
                    &changes,
                    kept.as_ref(),
                )
            })?
        };
        match step {
            Step::Quiet => scan.advance(frame.hour),
            Step::Battles => {
                let (land, navy) = battle_tables(format, &frame);
                scan.observe_battles(&frame, format, land, navy, &changes);
            }
            Step::Read {
                rows,
                leaders,
                gone,
            } => {
                let params = prepared::full_params(&scan.reply.branch_id);
                let admissions = &mut self.admissions;
                // A held frame that cannot give the hour's verdicts holds what they read, and
                // the hour is read again from where it began.
                loop {
                    let mark = scan.mark();
                    prepare(scan, rows.is_none(), &leaders, &gone);
                    scan.frontier = Some(frame.hour);
                    scan.materialized = Some(frame.hour);
                    let wanted = {
                        let kept = self.held.kept();
                        let since = Since::hour_before(&frame, &changes);
                        let observed = catalogue::own_refusals(&mut self.refusals, || {
                            scan.observe_rows(
                                &frame,
                                format,
                                &params,
                                content,
                                Some(&changes),
                                since.as_ref(),
                                kept.as_ref(),
                                rows.as_deref(),
                                &mut |reference, role, holder| {
                                    admissions.push((reference, role.clone(), holder))
                                },
                            )
                        })?;
                        (!observed).then(|| kept.as_ref().map(Kept::wanted).unwrap_or_default())
                    };
                    let Some(wanted) = wanted else {
                        break;
                    };
                    let _cpu = ::replay_frame::cpu::scope_phase("leaders:fallback");
                    self.fallbacks += 1;
                    if !self.held.widen(hour, &mut frame, &wanted)? {
                        return Err(invalid(
                            "A leaders hour cannot give its visibility verdicts on every table",
                        ));
                    }
                    scan.resume(mark);
                }
                index(scan, &frame, rows.is_none());
            }
        }
        self.held.keep(frame);
        Ok(())
    }

    fn end(&mut self, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        let Some(scan) = self.scan.take() else {
            return Ok(());
        };
        let counts = Counts {
            hours: std::mem::take(&mut self.hours),
            decoded: std::mem::take(&mut self.held.decoded),
            fallbacks: std::mem::take(&mut self.fallbacks),
            ..Counts::default()
        };
        out.push(hand(
            scan,
            self.first.take().expect("a segment start"),
            counts,
            std::mem::take(&mut self.refusals),
            std::mem::take(&mut self.admissions),
        )?)
    }
}

/// A segment's scan handed to the ordered emit: its careers and commands written through its
/// last hour, its runtime reads dropped.
fn hand(
    mut scan: Scan,
    first: i32,
    mut counts: Counts,
    refusals: catalogue::OwnRefusals,
    admissions: Vec<(u64, LeaderRole, Option<(u32, u32)>)>,
) -> io::Result<Hour> {
    scan.seal();
    let mut work = scan.projection.take().expect("a projected segment");
    if let Some(at) = scan.frontier {
        work.finish_commands(at);
    }
    scan.previous_combats = work
        .previous
        .each_ref()
        .map(|held| held.as_ref().map(|(at, set)| (*at, set.as_ref().clone())));
    scan.results_recorded = work.results_recorded;
    counts.walks = work.walks;
    counts.resolved = work.resolved as u64;
    counts.invalidations = work.invalidations.len() as u64;
    counts.undeclared = work.registry.undeclared.len() as u64;
    counts.quiet = work.quiet;
    counts.battles = work.battle_hours;
    counts.suspects = work.suspects;
    counts.confirmed = work.confirmed;
    counts.runs = work.runs;
    counts.rebuilds = work.rebuilds;
    let work = work.handoff();
    scan.last_people.clear();
    scan.formation_names.clear();
    scan.row_indices.clear();
    scan.active_commands.clear();
    scan.evaluated.clear();
    for state in &mut scan.leaders {
        state.visibility = None;
        state.evaluation = None;
        state.career.commands.clear();
        state.command_indices.clear();
        state.career.trait_gains.clear();
    }
    let hour = scan
        .frontier
        .ok_or_else(|| invalid("A leaders segment has no observed hours"))?;
    Ok(Hour {
        branch: scan
            .reply
            .branch_id
            .parse()
            .map_err(|_| invalid("A leaders branch is not numeric"))?,
        hour,
        first,
        counts,
        scan,
        work,
        refusals,
        admissions,
    })
}

/// What an hour does ([`plan`]).
enum Step {
    /// Nothing leaders read changed: every career and command goes on through the hour.
    Quiet,
    /// Only combats and results changed.
    Battles,
    /// The hour reads these roster rows, every row when None. `leaders` are the read leaders
    /// already on the roster, `gone` those that left it.
    Read {
        rows: Option<Vec<usize>>,
        leaders: Vec<usize>,
        gone: Vec<usize>,
    },
}

/// The leader tables whose changed rows name the roster leaders that read them.
const OWN: [TableId; 6] = [
    TableId::UnitLeaders,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
    TableId::UnitLeaderTraits,
    TableId::UnitLeaderTraitExperience,
];

/// What the hour reads, from what changed since the hour before: every leader where what
/// all of them read changed (validity, holders, owners, links, availability, a leader table
/// as a whole), else the leaders whose own rows changed, whose visibility evaluation does not
/// read the same, or that joined or left the roster; the commands where theirs changed; the
/// battles alone, or nothing.
fn plan(
    scan: &mut Scan,
    frame: &DecodedFrame,
    format: FormatVersion,
    content: Option<&Content>,
    facts: &FrameChanges,
    changes: &dyn TableChanges,
    kept: Option<&Kept<'_>>,
) -> io::Result<Step> {
    let mut work = scan.projection.take().expect("a projected scan");
    let step = plan_with(
        scan, &mut work, frame, format, content, facts, changes, kept,
    );
    scan.projection = Some(work);
    step
}

#[allow(clippy::too_many_arguments)]
fn plan_with(
    scan: &mut Scan,
    work: &mut Work,
    frame: &DecodedFrame,
    format: FormatVersion,
    content: Option<&Content>,
    facts: &FrameChanges,
    changes: &dyn TableChanges,
    kept: Option<&Kept<'_>>,
) -> io::Result<Step> {
    let hour = frame.hour;
    let since = Since::hour_before(frame, changes);
    let moved = |read: &TableRead| since.as_ref().is_none_or(|since| since.changed(read));
    let holders = scan.refresh_holders(frame, &moved);
    let reads = reads();
    if moved(&reads.unit_leaders) {
        scan.scoped_visibility = work.scoped(frame, changes);
    }
    let valid = facts.slot_read[TableId::UnitLeaders.index()]
        && frame.complete::<tables::UnitLeaders>().is_ok()
        && frame.complete::<tables::Countries>().is_ok()
        && frame.complete::<tables::CountryLinks>().is_ok();
    let validity = (
        valid,
        (scan.scoped_visibility && content.is_some())
            .then(|| core_gap(frame))
            .flatten(),
    );
    let continuous = scan
        .frontier
        .is_some_and(|at| at.checked_add(1) == Some(hour));
    let invalidated = scan.validity.as_ref().is_some_and(|old| *old != validity)
        || scan.frontier.is_some() && !continuous;
    if invalidated {
        invalidate_leader_reads(&mut scan.leaders, &mut scan.last_people);
        scan.evaluated.clear();
        scan.read_gap = None;
        work.registry.clear();
    }
    work.before = scan.frontier;
    work.validity(hour, valid, invalidated);
    scan.validity = Some(validity);
    let owners = valid && scan.owner_tags.update(frame)?;
    if owners {
        scan.owner_epoch += 1;
    }
    let (land, navy) = battle_tables(format, frame);
    let traits = frame.complete::<tables::UnitLeaderTraits>().is_ok()
        && slot_read(format, &frame.metrics, TableId::UnitLeaderTraits, hour);
    let read = ([land.is_some(), navy.is_some()], traits);
    let steady = work.reads.replace(read) == Some(read);
    let every = !continuous
        || invalidated
        || !valid
        || holders
        || owners
        || !steady
        || !matches!(changes.table(TableId::CountryLinks), TableChange::Same)
        || OWN.iter().any(|&table| {
            !matches!(
                changes.table(table),
                TableChange::Same | TableChange::Rows(..)
            )
        });
    let rows = frame.complete::<tables::UnitLeaders>().unwrap_or_default();
    let mut dirty = BTreeSet::new();
    let mut added = Vec::new();
    let mut gone = Vec::new();
    if !every {
        if let TableChange::Rows(written, deleted) = changes.table(TableId::UnitLeaders) {
            for row in written {
                match work.roster.members.get(&(row.key as u64)) {
                    Some(&at) => {
                        dirty.insert(at);
                    }
                    None => added.push(row.key),
                }
            }
            for &key in deleted {
                if let Some(at) = work.roster.members.remove(&(key as u64)) {
                    gone.push(at);
                }
            }
        }
        for &at in &gone {
            work.roster.forget(at);
            work.registry.remove(at);
            work.failing.remove(&at);
        }
        for &table in &OWN[1..] {
            if let TableChange::Rows(written, deleted) = changes.table(table) {
                for key in written
                    .iter()
                    .map(|row| row.key)
                    .chain(deleted.iter().copied())
                {
                    work.roster.readers(table, key, &mut dirty);
                }
            }
        }
    }
    let positions = (!every)
        .then(|| {
            dirty
                .iter()
                .map(|&at| u128::from(scan.leaders[at].reference))
                .chain(added.iter().copied())
                .map(|key| rows.binary_search_by_key(&key, RowCodec::key).ok())
                .collect::<Option<Vec<_>>>()
        })
        .flatten();
    let Some(mut positions) = positions else {
        // Every leader, or a changed leader the roster index does not hold where it is.
        if continuous && !invalidated {
            let registered: Vec<usize> = work.registry.of.keys().copied().collect();
            carry_all(scan, frame, &registered);
        }
        work.walks[0] += 1;
        return Ok(Step::Read {
            rows: None,
            leaders: Vec::new(),
            gone: Vec::new(),
        });
    };
    let suspects = work.registry.suspects(changes, since.as_ref());
    work.suspects += suspects.len() as u64;
    let registered: Vec<usize> = dirty
        .iter()
        .chain(&suspects)
        .copied()
        .filter(|at| work.registry.of.contains_key(at))
        .collect();
    carry_all(scan, frame, &registered);
    let confirmed = confirm(
        scan,
        frame,
        content,
        since.as_ref(),
        kept,
        &suspects,
        &dirty,
    );
    work.confirmed += confirmed.len() as u64;
    let unconfirmed: Vec<usize> = suspects
        .iter()
        .copied()
        .filter(|at| !dirty.contains(at) && !confirmed.contains(at))
        .collect();
    for at in unconfirmed {
        let Ok(position) =
            rows.binary_search_by_key(&u128::from(scan.leaders[at].reference), RowCodec::key)
        else {
            work.walks[0] += 1;
            return Ok(Step::Read {
                rows: None,
                leaders: Vec::new(),
                gone: Vec::new(),
            });
        };
        positions.push(position);
        dirty.insert(at);
    }
    let moving = |table: TableId| {
        !matches!(
            changes.table(table),
            TableChange::Same | TableChange::Fields(_)
        )
    };
    let missions = work.commands.mode.is_none_or(|mode| mode.2);
    let commands = work.commands.repeats > 0
        || gone.iter().any(|at| work.live.contains(at))
        || [
            TableId::OrdersGroups,
            TableId::OrdersGroupPlans,
            TableId::Fleets,
        ]
        .into_iter()
        .any(moving)
        || missions
            && [TableId::NavalMissionTaskForces, TableId::TaskForces]
                .into_iter()
                .any(moving);
    if positions.is_empty() && !commands {
        if work.combats_changed(frame, format, changes) || reads.results.iter().any(&moved) {
            work.battle_hours += 1;
            return Ok(Step::Battles);
        }
        work.quiet += 1;
        return Ok(Step::Quiet);
    }
    positions.sort_unstable();
    positions.dedup();
    work.walks[1] += 1;
    Ok(Step::Read {
        rows: Some(positions),
        leaders: dirty.into_iter().collect(),
        gone,
    })
}

/// Before an hour's read: every career carried lazily written through the hour before where
/// the hour reads every leader, else those of the leaders it reads and of the leaders gone
/// from the roster, which then leave its visible leaders.
fn prepare(scan: &mut Scan, every: bool, leaders: &[usize], gone: &[usize]) {
    if every {
        scan.seal();
        return;
    }
    let before = scan.frontier.expect("a consecutive restricted read");
    if let Some(materialized) = scan.materialized {
        if let Some(observed) = scan.reply.observed.last_mut()
            && observed.last_hour == materialized
        {
            observed.last_hour = before;
        }
        for missing in scan.missing_battles.iter_mut().flatten() {
            if *missing == materialized {
                *missing = before;
            }
        }
    }
    let work = scan.projection.as_mut().expect("a projected scan");
    for &at in leaders.iter().chain(gone) {
        if work.live.contains(&at) {
            let state = &mut scan.leaders[at];
            state.visible_hour = Some(before);
            if let Some(run) = state.last_run {
                scan.stored_runs[run].run.last_hour = before;
            }
        }
    }
    for &at in gone {
        if work.live.remove(&at) {
            work.relive.insert(scan.leaders[at].reference);
        }
    }
}

/// After an hour's read: where each leader it read finds its reads, and its evaluation's
/// declared reads while that evaluation gave the hour's verdict.
fn index(scan: &mut Scan, frame: &DecodedFrame, every: bool) {
    let Some(work) = scan.projection.as_mut() else {
        return;
    };
    if every {
        work.roster.clear();
        work.registry.clear();
    }
    let rows = frame.complete::<tables::UnitLeaders>().unwrap_or_default();
    let characters = frame.complete::<tables::Characters>().unwrap_or_default();
    let training = frame
        .complete::<tables::UnitLeaderTraitExperience>()
        .unwrap_or_default();
    for at in std::mem::take(&mut work.read) {
        let state = &scan.leaders[at];
        let Ok(position) = rows.binary_search_by_key(&u128::from(state.reference), RowCodec::key)
        else {
            continue;
        };
        work.roster.index(at, &rows[position], characters, training);
        match &state.evaluation {
            Some(evaluation) if evaluation.hour == frame.hour => {
                work.registry.insert(at, &evaluation.inputs)
            }
            _ => work.registry.remove(at),
        }
    }
}

/// Of `suspects` that no other change reads, those whose evaluation reads the same this
/// hour, each confirmed by value as a reused evaluation is.
fn confirm(
    scan: &mut Scan,
    frame: &DecodedFrame,
    content: Option<&Content>,
    since: Option<&Since<'_>>,
    kept: Option<&Kept<'_>>,
    suspects: &BTreeSet<usize>,
    dirty: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    let mut confirmed = BTreeSet::new();
    let clean: Vec<usize> = suspects
        .iter()
        .copied()
        .filter(|at| !dirty.contains(at))
        .collect();
    // Check mode runs every reused evaluation again, which a read does.
    let Some(scripted) = content
        .filter(|_| !clean.is_empty() && !check_reuse())
        .and_then(|content| content.catalogue.scripted_content().ok())
    else {
        return confirmed;
    };
    let world = HourWorld::new(scripted, frame, kept);
    if world.gap().is_err() {
        return confirmed;
    }
    let mut rereads = Rereads::new();
    let before = frame.hour.saturating_sub(1);
    for at in clean {
        let Some(evaluation) = scan.leaders[at].evaluation.as_mut() else {
            continue;
        };
        if evaluation.hour != before {
            continue;
        }
        let whole = world.reads_whole(&evaluation.inputs, before);
        let same = evaluation.inputs.confirm_since(frame, since, |key| {
            if !whole {
                return None;
            }
            let world = world.get().ok()?;
            reread(&mut rereads, world, frame, key)
        });
        if same {
            evaluation.hour = frame.hour;
            confirmed.insert(at);
        }
    }
    confirmed
}

/// What changed in each table over hours at which the registry found none of an
/// evaluation's declared reads changed: nothing it reads.
struct Unchanged;

impl TableChanges for Unchanged {
    fn table(&self, _: TableId) -> TableChange<'_> {
        TableChange::Same
    }
}

static UNCHANGED: Unchanged = Unchanged;

/// Carries each of `leaders`' evaluations from the hour it last gave its verdict to the hour
/// before `frame`'s, as an evaluation reused at each of them would be: the registry found
/// none of its declared reads changed since. One with an input declaring no dependence is not
/// carried, and is read again.
fn carry_all(scan: &mut Scan, frame: &DecodedFrame, leaders: &[usize]) {
    let before = frame.hour.saturating_sub(1);
    for &at in leaders {
        let Some(evaluation) = scan.leaders[at].evaluation.as_mut() else {
            continue;
        };
        if evaluation.hour >= before {
            continue;
        }
        let since = Since::span(
            frame.branch,
            evaluation.hour,
            before,
            Some(frame),
            &UNCHANGED,
        );
        if evaluation.inputs.advance(&since) {
            evaluation.hour = before;
        }
    }
}

fn role(scan: &mut Scan, previous: Option<u32>, key: &str, value: &LeaderRole) -> u32 {
    let same = |at: u32| {
        let old = &scan.reply.roles[at as usize];
        old.category == value.category
            && old.kind == value.kind
            && old.maximum_skill == value.maximum_skill
            && old.owner == value.owner
    };
    if let Some(at) = previous.filter(|&at| same(at)) {
        return at;
    }
    if let Some(at) = scan.roles.get(key).copied().filter(|&at| same(at)) {
        return at;
    }
    let at = scan.reply.roles.len() as u32;
    scan.reply.roles.push(value.clone());
    scan.roles.insert(key.to_owned(), at);
    at
}

fn command(state: &mut LeaderState, input: CommandRead, bytes: &mut usize) {
    let (at, _, added) = append_command(
        state,
        input.formation,
        input.kind,
        input.name.as_deref(),
        input.first,
    );
    *bytes = bytes.saturating_add(added);
    let held = &mut state.career.commands[at];
    held.last_hour = input.last;
    if input.name.is_none() {
        held.name_unknown
            .last_mut()
            .expect("an unknown name span")
            .last_hour = input.last;
    }
}

impl Hour {
    pub(super) fn stitch(
        mut self,
        scan: &mut Scan,
        published_outcomes: &BattleOutcomes,
        published_sources: &BTreeMap<u64, NavalSource>,
    ) -> io::Result<()> {
        scan.seal();
        let old_missing = scan.missing_battles;
        let mut leaders = Vec::with_capacity(self.scan.leaders.len());
        for local in &self.scan.leaders {
            let index = *scan
                .leader_indices
                .entry(local.reference)
                .or_insert_with(|| {
                    scan.leaders.push(LeaderState::new(local.reference));
                    scan.leaders.len() - 1
                });
            leaders.push(index);
        }
        let mut people = vec![0; self.scan.reply.people.len()];
        let mut person_keys = vec![String::new(); people.len()];
        for (key, &index) in &self.scan.people {
            person_keys[index as usize] = key.clone();
        }
        for (index, person) in self.scan.reply.people.iter().enumerate() {
            let key = &person_keys[index];
            people[index] = match scan.people.get(key) {
                Some(&at) => at,
                None => {
                    let at = scan.reply.people.len() as u32;
                    scan.reply.people.push(person.clone());
                    scan.people.insert(key.clone(), at);
                    if let Some(warnings) = self.work.person_warnings.remove(&(index as u32)) {
                        scan.reply.warnings.extend(warnings);
                    }
                    at
                }
            };
        }
        let mut tokens = Vec::with_capacity(self.scan.token_list.len());
        for token in &self.scan.token_list {
            let at = *scan.trait_tokens.entry(token.clone()).or_insert_with(|| {
                let at = scan.token_list.len() as u32;
                scan.token_list.push(token.clone());
                at
            });
            tokens.push(at);
        }
        let mut owners = vec![0; self.scan.stored_runs.len()];
        let mut prior_roles = HashMap::new();
        for (local, &global) in self.scan.leaders.iter().zip(&leaders) {
            let state = &scan.leaders[global];
            if let Some(read) = &state.last_read {
                if scan
                    .projection_break
                    .is_none_or(|gap| state.visible_hour.is_some_and(|at| at >= gap))
                {
                    prior_roles.insert(global, (read.role, state.visible_hour));
                }
            }
            let mut at = local.last_run;
            while let Some(index) = at {
                owners[index] = global;
                at = self.scan.stored_runs[index].previous;
            }
            if let Some(&(first, missing, kind)) = self.work.first_visible.get(&local.reference) {
                if let Some(last) = state.last_run.map(|at| &scan.stored_runs[at].run) {
                    let kind = usize::from(kind - 1);
                    let missed = old_missing[kind]
                        .into_iter()
                        .chain(missing[kind])
                        .any(|at| at >= last.last_hour);
                    if scan.leaders[global].career.partial_from.is_none()
                        && (last.last_hour.checked_add(1) != Some(first) || missed)
                    {
                        scan.leaders[global].career.partial_from = Some(first);
                    }
                }
            }
        }
        for (index, stored) in self.scan.stored_runs.drain(..).enumerate() {
            let global = owners[index];
            let progress = self.scan.run_progress.remove(&index);
            let mut run = stored.run;
            let local_role = run.value.role as usize;
            let previous = prior_roles
                .get(&global)
                .copied()
                .filter(|(_, at)| {
                    !self
                        .work
                        .invalidations
                        .iter()
                        .any(|&h| at.is_none_or(|at| h > at) && h <= run.first_hour)
                })
                .map(|(role, _)| role);
            run.value.person = people[run.value.person as usize];
            run.value.role = role(
                scan,
                previous,
                &self.work.role_keys[local_role],
                &self.scan.reply.roles[local_role],
            );
            if let Some(held) = &run.value.traits {
                let mut held: Vec<_> = held.iter().map(|&at| tokens[at as usize]).collect();
                held.sort_unstable();
                held.dedup();
                run.value.traits = Some(Arc::from(held));
            }
            prior_roles.insert(global, (run.value.role, Some(run.last_hour)));
            let before = scan.stored_runs.len();
            append_stored_run(
                &mut scan.leaders[global],
                &mut scan.stored_runs,
                run.value,
                run.first_hour,
                scan.reply.first_hour,
                false,
            );
            scan.pending_bytes = scan.pending_bytes.saturating_add(
                (scan.stored_runs.len() - before) * std::mem::size_of::<StoredRun>(),
            );
            if let Some(progress) = progress.filter(|_| scan.stored_runs.len() > before) {
                scan.run_progress
                    .insert(scan.stored_runs.len() - 1, progress);
            }
            let at = scan.leaders[global].last_run.expect("a stitched run");
            scan.stored_runs[at].run.last_hour = run.last_hour;
        }
        for (local, &global) in self.scan.leaders.iter_mut().zip(&leaders) {
            let state = &mut scan.leaders[global];
            state.career.partial_from = state
                .career
                .partial_from
                .into_iter()
                .chain(local.career.partial_from)
                .min();
            if local.visible_hour.is_some() {
                state.visible_hour = local.visible_hour;
            }
            if let Some(mut read) = local.last_read.take() {
                read.person = people[read.person as usize];
                read.role = prior_roles.get(&global).expect("a visible role").0;
                state.last_read = Some(read);
            } else if self
                .work
                .invalidations
                .last()
                .is_some_and(|&h| state.visible_hour.is_none_or(|at| h > at))
            {
                state.last_read = None;
            }
        }
        self.work
            .command_reads
            .sort_by_key(|read| (read.first, read.order));
        for input in self.work.command_reads {
            let index = scan.leader_indices[&input.leader];
            command(&mut scan.leaders[index], input, &mut scan.pending_bytes);
        }
        for observation in self.scan.reply.observed {
            observe(
                &mut scan.reply.observed,
                observation.first_hour,
                observation.leaders,
                observation.land_battles,
                observation.naval_battles,
                observation.land_commands,
                observation.naval_commands,
            );
            scan.reply
                .observed
                .last_mut()
                .expect("an observation")
                .last_hour = observation.last_hour;
        }
        let source = |scan: &mut Scan, result| {
            if !scan.naval_sources.contains_key(&result) {
                if let Some(&source) = published_sources.get(&result) {
                    scan.naval_sources.insert(result, source);
                }
            }
            scan.naval_sources.get(&result).copied()
        };
        for fact in self.work.matches {
            match fact {
                Match::Source { result, source } => {
                    scan.naval_sources.insert(result, source);
                }
                Match::Land {
                    combat,
                    end,
                    read,
                    winners,
                } => {
                    let key = (1, combat);
                    if !scan.outcomes.contains_key(&key) {
                        if let Some(&value) = published_outcomes.get(&key) {
                            scan.outcomes.insert(key, value);
                        }
                    }
                    if record_outcome(&mut scan.outcomes, key, (end, read, winners)) {
                        scan.refusals.note(
                            "Conflicting native land battle outcomes",
                            combat.to_string(),
                        );
                    }
                }
                Match::Naval {
                    result,
                    winner,
                    read,
                } => {
                    if let Some(source) = source(scan, result) {
                        let key = (2, source.combat);
                        if !scan.outcomes.contains_key(&key) {
                            if let Some(&value) = published_outcomes.get(&key) {
                                scan.outcomes.insert(key, value);
                            }
                        }
                        let winners = match winner {
                            1 => 1,
                            2 => 2,
                            _ => 0,
                        };
                        if winner > 2 {
                            scan.refusals.note(
                                "Unsupported native naval winner selector",
                                result.to_string(),
                            );
                        }
                        if record_outcome(
                            &mut scan.outcomes,
                            key,
                            (source.game_hour, read, winners),
                        ) {
                            scan.refusals.note(
                                "Conflicting native naval battle outcomes",
                                source.combat.to_string(),
                            );
                        }
                    }
                }
                Match::Sides {
                    read,
                    before,
                    sides,
                } => {
                    let previous = before.unwrap_or_else(|| {
                        scan.previous_combats
                            .each_ref()
                            .map(|held| held.as_ref().map(|(at, set)| (*at, Arc::new(set.clone()))))
                    });
                    for side in sides {
                        let joined = match side.join {
                            Join::Combat(kind, combat) => Some((kind, combat, side.hour)),
                            Join::Naval(result) => source(scan, result)
                                .map(|source| (2, source.combat, source.game_hour)),
                        };
                        let Some((kind, combat, hour)) = joined else {
                            continue;
                        };
                        let rows = scan.battle_hours.entry(side.leader).or_default();
                        if let Some(&at) = scan.battle_indices.get(&(side.leader, kind, combat)) {
                            rows[at].5 |= side.side;
                        } else {
                            let new = side.active
                                && previous[usize::from(kind - 1)].as_ref().is_some_and(
                                    |(at, set)| {
                                        at.checked_add(1) == Some(read)
                                            && set.binary_search(&combat).is_err()
                                    },
                                );
                            scan.battle_indices
                                .insert((side.leader, kind, combat), rows.len());
                            rows.push((hour, read, !new, kind, combat, side.side));
                            scan.pending_bytes = scan.pending_bytes.saturating_add(
                                std::mem::size_of::<(i32, i32, bool, u8, u64, u8)>(),
                            );
                        }
                    }
                }
            }
        }
        if self.work.battles_read {
            scan.previous_combats = self.scan.previous_combats;
        }
        for (old, new) in scan
            .missing_battles
            .iter_mut()
            .zip(self.scan.missing_battles)
        {
            if new.is_some() {
                *old = new;
            }
        }
        if self.work.owner_read {
            scan.owner_tags = self.scan.owner_tags;
        }
        scan.refusals.merge(&self.scan.refusals);
        scan.results_recorded |= self.scan.results_recorded;
        if let Some(&gap) = self.work.invalidations.last() {
            scan.projection_break = Some(gap);
        }
        scan.frontier = self.scan.frontier;
        scan.materialized = self.scan.materialized;
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/columnar_parity.rs"]
mod tests;
