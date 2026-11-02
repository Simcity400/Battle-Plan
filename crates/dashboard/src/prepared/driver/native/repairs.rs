//! Dockyard repairs' native backend: each hour's operator hands its ordered emit the rows of the
//! repair census's tables whose read fields the hour changed, with their exact change facts, and
//! the hour's read evidence; the emit patches the branch's frame of them and observes it with
//! the repair census as the hourly walk does, its unchanged-hour step where nothing it reads
//! moved. It publishes the pages the walk publishes: the rate series and its catalogue.

use super::backend::{Native, Projected};
use crate::{
    prepared::{driver::publication::Stage, frame_changes::FrameChanges},
    products::dockyard_repairs::{Prepared as Census, SELECTION_FIELDS},
};
use dashboard_base::prepared::{
    roots::Published,
    series::{compress, decompress},
};
use dashboard_keyed::table_changes::{Change, apply_frame};
use dashboard_world::{Needs, World};
use replay::{
    columnar_frames::Segment,
    columns,
    container::Replay,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        DeltaRow, ShipRow, TABLE_COUNT, TableId, TableRows, TaskForceRow, VariantChangeRow,
        full_mask,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, Frame, Metrics, TableUpdate, TableView};
use std::{
    collections::BTreeMap,
    io,
    mem::size_of,
    sync::{Arc, atomic::AtomicBool},
};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

fn cell<T: Copy>(values: &[T], slot: u32) -> io::Result<T> {
    values
        .get(slot as usize)
        .copied()
        .ok_or_else(|| invalid("A census slot holds no cell"))
}

/// The census's tables with the fields it reads of each, by name: every field for those named
/// with none.
const READS: &[(TableId, &[&str])] = &[
    (TableId::Countries, &["canonical_slot", "tag"]),
    (TableId::NavalRepairCountries, &["num_used_dockyards"]),
    (
        TableId::CountryProductionModifiers,
        &["power_raw", "factory_speed", "trees"],
    ),
    (TableId::CountryAppearance, &[]),
    (TableId::LexicalTokens, &[]),
    (TableId::VariantDefinitions, &[]),
    (TableId::VariantModules, &[]),
    (TableId::VariantUpgrades, &[]),
    (TableId::VariantChanges, &[]),
];

/// Every table the census reads: [`READS`]' and the repair selector's.
pub(crate) const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::NavalRepairCountries,
    TableId::CountryProductionModifiers,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::NavalBases,
    TableId::Ships,
    TableId::ShipModels,
    TableId::TaskForces,
    TableId::Provinces,
    TableId::RelationLinks,
    TableId::FactionMembers,
    TableId::Subjects,
    TableId::WarRelations,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::VariantChanges,
];

/// The daily tables whose read dates the census's hours: an hour between reads of both repeats
/// the last read.
const DATED: [TableId; 2] = [TableId::CountryProductionModifiers, TableId::VariantDefinitions];

/// The tables whose written rows the census visits by key, read fields or not. Of every other
/// table it asks only whether a row was written in a field it reads.
const KEYED: [TableId; 2] = [
    TableId::NavalRepairCountries,
    TableId::CountryProductionModifiers,
];

/// The tables the census reads only as holding rows or none, and by when their view changes:
/// each is handed on as its first live row's key alone, replaced wherever the hour wrote it.
const WITNESSED: [TableId; 1] = [TableId::VariantChanges];

/// Each of [`TABLES`] with the change-mask bits of the fields the census reads.
fn reads() -> io::Result<Vec<(TableId, u64)>> {
    let mask = |table: TableId, names: &[&str]| -> io::Result<u64> {
        if names.is_empty() {
            return Ok(u64::MAX);
        }
        let mut mask = 0_u64;
        for name in names {
            mask |= 1_u64 << columns::field(table, name)?;
        }
        Ok(mask)
    };
    TABLES
        .iter()
        .map(|&table| {
            let names = READS
                .iter()
                .chain(SELECTION_FIELDS)
                .find(|&&(named, _)| named == table)
                .map_or(&[][..], |&(_, names)| names);
            Ok((table, mask(table, names)?))
        })
        .collect()
}

/// One projected hour: its read evidence, the census tables it changed in the fields read, as
/// patches or whole views, and the change facts of every table it wrote rows of: the rows
/// written in fields read, every row written of [`KEYED`].
pub(crate) struct Hour {
    hour: i32,
    /// The hour is its segment's first, or follows an hour the operator did not walk: every
    /// table is handed on whole.
    opens: bool,
    metrics: Box<Metrics>,
    changes: Vec<(TableId, Arc<Change>)>,
    deltas: Vec<(TableId, Vec<DeltaRow>, Vec<u128>)>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        size_of::<Metrics>()
            + self.changes.capacity() * size_of::<(TableId, Arc<Change>)>()
            + self
                .changes
                .iter()
                .map(|(_, change)| change.heap())
                .sum::<usize>()
            + self
                .deltas
                .iter()
                .map(|(_, rows, deleted)| {
                    size_of::<(TableId, Vec<DeltaRow>, Vec<u128>)>()
                        + rows.capacity() * size_of::<DeltaRow>()
                        + deleted.capacity() * size_of::<u128>()
                })
                .sum::<usize>()
    }
}

impl Projected for Hour {
    fn hour(&self) -> i32 {
        self.hour
    }
}

/// The fields of the selector's hottest tables a reduced row holds: everything else of those
/// rows stays at its default, which the census never reads.
struct Hot {
    task_force: usize,
    strength: usize,
    roster_owner: usize,
    province: usize,
}

/// One segment's operator.
pub(crate) struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    reads: Vec<(TableId, u64)>,
    hot: Hot,
    /// Each table's status the hour before.
    shapes: [Option<columns::Status>; TABLE_COUNT],
    last: Option<i32>,
}

impl Projector {
    fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        Ok(Self {
            format,
            mode,
            reads: reads()?,
            hot: Hot {
                task_force: columns::field(TableId::Ships, "task_force")?,
                strength: columns::field(TableId::Ships, "strength")?,
                roster_owner: columns::field(TableId::TaskForces, "roster_owner")?,
                province: columns::field(TableId::TaskForces, "province")?,
            },
            shapes: [None; TABLE_COUNT],
            last: None,
        })
    }

    /// The rows of `table`'s live `slots`, in key order: the selector's hottest tables reduced
    /// to the fields it reads, a [`WITNESSED`] table to its first slot's key, every other whole.
    fn rows(&self, state: &State, table: TableId, slots: &[u32]) -> io::Result<TableRows> {
        let version = self.format.version();
        let held = columns::table(state, table)?;
        let key = |slot: u32| {
            held.key(slot)
                .ok_or_else(|| invalid("A census slot holds no row"))
        };
        Ok(match table {
            TableId::Ships => {
                let (task_forces, strengths) = (
                    columns::plain::<u64>(held, self.hot.task_force)?,
                    columns::plain::<i64>(held, self.hot.strength)?,
                );
                let mut rows = Vec::with_capacity(slots.len());
                for &slot in slots {
                    let row = if held.recorded(self.hot.task_force, slot)
                        && held.recorded(self.hot.strength, slot)
                    {
                        ShipRow {
                            id: key(slot)? as u64,
                            task_force: cell(task_forces, slot)?,
                            strength: cell(strengths, slot)?,
                            ..ShipRow::default()
                        }
                    } else {
                        let row: ShipRow = columns::row(state, table, slot, version)?;
                        ShipRow {
                            id: row.id,
                            task_force: row.task_force,
                            strength: row.strength,
                            ..ShipRow::default()
                        }
                    };
                    rows.push(row);
                }
                rows.sort_unstable_by_key(|row| row.id);
                TableRows::Ships(rows)
            }
            TableId::TaskForces => {
                let (owners, provinces) = (
                    columns::plain::<u32>(held, self.hot.roster_owner)?,
                    columns::plain::<u32>(held, self.hot.province)?,
                );
                let mut rows = Vec::with_capacity(slots.len());
                for &slot in slots {
                    let row = if held.recorded(self.hot.roster_owner, slot)
                        && held.recorded(self.hot.province, slot)
                    {
                        TaskForceRow {
                            id: key(slot)? as u64,
                            roster_owner: cell(owners, slot)?,
                            province: cell(provinces, slot)?,
                            ..TaskForceRow::default()
                        }
                    } else {
                        let row: TaskForceRow = columns::row(state, table, slot, version)?;
                        TaskForceRow {
                            id: row.id,
                            roster_owner: row.roster_owner,
                            province: row.province,
                            ..TaskForceRow::default()
                        }
                    };
                    rows.push(row);
                }
                rows.sort_unstable_by_key(|row| row.id);
                TableRows::TaskForces(rows)
            }
            TableId::VariantChanges => TableRows::VariantChanges(
                slots
                    .first()
                    .map(|&slot| {
                        key(slot).map(|key| VariantChangeRow {
                            sequence: key as u64,
                            ..VariantChangeRow::default()
                        })
                    })
                    .transpose()?
                    .into_iter()
                    .collect(),
            ),
            _ => columns::rows_at(state, table, slots, version)?,
        })
    }

    /// What `hour` changed of `table`: its whole view where the hour opens or the table's status
    /// moved; else the rows written in the fields `mask` and the keys deleted, and the change
    /// facts of those rows (of every row written of [`KEYED`]) as the row decoder gives them,
    /// empty where it wrote only fields unread; none where it wrote and deleted nothing.
    #[allow(clippy::type_complexity)]
    fn change(
        &mut self,
        hour: &segments::Hour<'_, World>,
        opens: bool,
        table: TableId,
        mask: u64,
    ) -> io::Result<Option<(Option<Change>, Option<(Vec<DeltaRow>, Vec<u128>)>)>> {
        let state = hour.state;
        let status = columns::status(state, table);
        let shape = std::mem::replace(&mut self.shapes[table.index()], Some(status));
        let whole = opens || shape != Some(status);
        let view = match status {
            columns::Status::Complete if whole => {
                let slots: Vec<u32> = columns::table(state, table)?.live().ones().collect();
                TableView::Complete(self.rows(state, table, &slots)?)
            }
            columns::Status::Complete if WITNESSED.contains(&table) => {
                let held = columns::table(state, table)?;
                let edits = columns::changes(state, table)?;
                let wrote = edits
                    .changed
                    .iter()
                    .chain(&edits.fill_slots)
                    .chain(&edits.inserted)
                    .any(|&slot| held.key(slot).is_some());
                if !wrote && edits.deleted_keys.is_empty() {
                    return Ok(None);
                }
                let first: Vec<u32> = held.live().ones().take(1).collect();
                TableView::Complete(self.rows(state, table, &first)?)
            }
            columns::Status::Complete => {
                let held = columns::table(state, table)?;
                let edits = columns::changes(state, table)?;
                let kept = if KEYED.contains(&table) { u64::MAX } else { mask };
                let mut wrote = false;
                let mut written: BTreeMap<u128, (u32, u64)> = BTreeMap::new();
                for (changed, masks) in [
                    (&edits.changed, &edits.masks),
                    (&edits.fill_slots, &edits.fill_masks),
                ] {
                    if changed.len() != masks.len() {
                        return Err(invalid("A table's slot masks miss their rows"));
                    }
                    for (&slot, &fields) in changed.iter().zip(masks) {
                        if let Some(key) = held.key(slot) {
                            wrote = true;
                            if fields & kept != 0 {
                                written.entry(key).or_insert((slot, 0)).1 |= fields;
                            }
                        }
                    }
                }
                let recorded = table.last_change().fields(table, self.format.version());
                for &slot in &edits.inserted {
                    if let Some(key) = held.key(slot) {
                        wrote = true;
                        written.insert(key, (slot, full_mask(recorded)));
                    }
                }
                let mut deleted = edits.deleted_keys.clone();
                deleted.sort_unstable();
                deleted.dedup();
                // A key deleted and inserted again within the hour is a row written.
                deleted.retain(|key| !written.contains_key(key));
                if !wrote && deleted.is_empty() {
                    return Ok(None);
                }
                let facts = written
                    .iter()
                    .map(|(&key, &(_, fields))| DeltaRow {
                        format: self.format.version(),
                        key,
                        mask: fields,
                        recorded_fields: recorded,
                        values: Vec::new(),
                    })
                    .collect();
                // Only rows written in the fields read are patched: the others' read fields hold.
                let slots: Vec<u32> = written
                    .values()
                    .filter(|&&(_, fields)| fields & mask != 0)
                    .map(|&(slot, _)| slot)
                    .collect();
                let patch = if slots.is_empty() && deleted.is_empty() {
                    None
                } else {
                    Some(Change::Patch {
                        rows: self.rows(state, table, &slots)?,
                        deleted: deleted.clone(),
                        keys_changed: !edits.inserted.is_empty() || !deleted.is_empty(),
                    })
                };
                return Ok(Some((patch, Some((facts, deleted)))));
            }
            _ if !whole => return Ok(None),
            columns::Status::Truncated { missing } => TableView::Truncated {
                rows: TableRows::empty(table),
                missing,
            },
            columns::Status::Unavailable { reason } => {
                let bytes = state
                    .dictionary()
                    .get(reason)
                    .ok_or_else(|| invalid("A table's reason is missing from its dictionary"))?;
                TableView::Unavailable {
                    reason: std::str::from_utf8(bytes)
                        .map_err(|_| invalid("A table's reason is not text"))?
                        .to_owned(),
                }
            }
            columns::Status::NotRecorded => TableView::NotRecorded { mode: self.mode },
        };
        Ok(Some((Some(Change::Replace(view.into())), None)))
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.last = None;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("repairs.columnar", Some("repairs"));
        let at = hour.hour;
        let opens = hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.last = Some(at);
        let mut changes = Vec::new();
        let mut deltas = Vec::new();
        for index in 0..self.reads.len() {
            let (table, mask) = self.reads[index];
            if let Some((change, facts)) = self.change(hour, opens, table, mask)? {
                if let Some(change) = change {
                    changes.push((table, Arc::new(change)));
                }
                if let Some((rows, deleted)) = facts {
                    deltas.push((table, rows, deleted));
                }
            }
        }
        out.push(Hour {
            hour: at,
            opens,
            metrics: Box::new(hour.world.metrics().clone()),
            changes,
            deltas,
        })
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(invalid("Not a saved repairs projector"));
        }
        self.shapes = [None; TABLE_COUNT];
        self.last = None;
        Ok(())
    }
}

/// What [`Projector::save`] writes: nothing but that it restarts, every table handed on whole.
const SAVED: &[u8] = b"repairs projector: cold";

/// A branch's repair census and its frame of the census tables.
pub(crate) struct Branch {
    census: Box<Census>,
    frame: Option<DecodedFrame>,
}

impl Branch {
    fn observe(
        &mut self,
        replay: &Replay,
        branch: u64,
        hour: Hour,
    ) -> io::Result<()> {
        let format = replay.format;
        let Hour {
            hour,
            opens,
            metrics,
            changes,
            deltas,
        } = hour;
        let mut facts = apply_frame(
            &mut self.frame,
            (branch, hour),
            opens,
            &metrics,
            &changes,
            format,
        )?;
        if changes.is_empty() {
            let read = DATED
                .iter()
                .any(|&table| crate::metadata::slot_read(format, &metrics, table, hour));
            if self.census.advance_quiet(hour, read)? {
                return Ok(());
            }
        }
        if !opens {
            // A table written changed by exactly its rows in the fields read (every row of KEYED),
            // all the census asks of it; a table handed on whole by any.
            let mut exact = [false; TABLE_COUNT];
            let mut rows_changed = facts.rows_changed;
            let mut tables: [TableUpdate; TABLE_COUNT] = std::array::from_fn(|_| {
                TableUpdate::Unavailable {
                    reason: String::new(),
                }
            });
            for (table, changed, deleted) in deltas {
                exact[table.index()] = true;
                rows_changed[table.index()] = true;
                tables[table.index()] = TableUpdate::Delta {
                    baseline: hour - 1,
                    changed,
                    deleted,
                };
            }
            if exact.contains(&true) {
                let recorded = Frame {
                    branch,
                    hour,
                    keyframe: false,
                    metrics: (*metrics).clone(),
                    tables,
                };
                facts = FrameChanges::observed(
                    rows_changed,
                    facts.table_read,
                    facts.slot_read,
                    Some((Arc::new(recorded.into()), exact)),
                );
            }
        }
        let frame = self
            .frame
            .as_ref()
            .ok_or_else(|| invalid("The repair census has no frame"))?;
        self.census.observe(frame, replay, &facts)
    }
}

pub(crate) struct Dockyards<'p> {
    pub(crate) replay: &'p Replay,
}

impl Native for Dockyards<'_> {
    type Operator = Projector;
    type Branch = Branch;

    fn tables(&self) -> &'static [TableId] {
        TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(Needs::default())
    }

    fn operator(&self, _: &Segment) -> io::Result<Projector> {
        Projector::new(self.replay.format, self.replay.capture_mode)
    }

    fn branch(&self, _: u64, _: i32) -> io::Result<Branch> {
        Ok(Branch {
            census: Box::default(),
            frame: None,
        })
    }

    fn observe(
        &self,
        state: &mut Branch,
        _: &mut Stage<'_, '_>,
        branch: u64,
        hour: Hour,
    ) -> io::Result<()> {
        state.observe(self.replay, branch, hour)
    }

    fn flush(
        &self,
        state: &mut Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.census.flush(stage.pages()?, branch, cancelled)
    }

    // The census goes on from what it saved; its frame is read whole again.
    fn save(&self, state: &mut Branch) -> io::Result<Option<Vec<u8>>> {
        compress(&serde_json::to_vec(&*state.census)?, 3).map(Some)
    }

    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        _: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Branch> {
        Ok(Branch {
            census: Box::new(serde_json::from_slice(&decompress(saved)?)?),
            frame: None,
        })
    }
}

