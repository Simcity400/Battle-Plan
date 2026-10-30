//! The feed prepared on a columnar recording's segments ([`replay::segments`]). Each segment's
//! worker reads the changed slots of the feed's tables, the rest from the walk's shared frame
//! ([`dashboard_world`]), names occurrences at its own hours through an identity history of
//! its own, plans each pack's reduction and encodes their facts;
//! the ordered emit holds every state the feed carries, its saved continuation, and advances it
//! once per hour. Facts whose bytes need the emit's state of an hour but no more (roster
//! rebuilds, retained observations) are written by threads of their own, in batches, and
//! appended in order before any flush.

use super::{
    Builder, EventFeedKind, Notice, country_groups,
    decision_roster::Captured,
    diplomacy, native, nuclear,
    prepared::{ChargeKey, Draft, Observer, charges_of, strike_facts, strike_types},
};
use crate::{
    metadata::{LinkTags, branch_bounds},
    prepared::frame_changes::FrameChanges,
    products::country_history::HistoryScan,
};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::{metrics, view_heap_bytes},
    columns,
    container::Replay,
    segments::{self, Item, Operator, Sink, Start},
};
use replay_data::{
    event_feed_rows::{
        EventFeedCountry, EventFeedCoverage, EventFeedSource,
        state::{PreparedHistory, Written},
    },
    format::FormatVersion,
    rows::{
        CountryCounterInputRow, CountryExperienceGainRow, CountryRow, NuclearStrikeRow, RowChanges,
        RowCodec, SliceReadRow, TABLE_COUNT, TableId, TableRows, TableSchema, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, Metrics, TableView};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    mem::size_of,
    sync::{Arc, atomic::AtomicBool},
    thread::JoinHandle,
};

/// The tables the feed and its identity history read.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryCounterInputs,
    TableId::CountryExperienceGains,
    TableId::RelationTuples,
    TableId::SliceReads,
    TableId::EventFeedHistory,
    TableId::NuclearStrikes,
    TableId::CountryPowerChanges,
    TableId::PoliticsCaptureCoverage,
];

/// The tables the worker reads as rows from the walk's frame.
const FRAME: [TableId; 7] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryCounterInputs,
    TableId::CountryExperienceGains,
    TableId::RelationTuples,
    TableId::NuclearStrikes,
];

/// The fields of the identity tables the worker and its emit read, by table; every field of
/// the others.
fn read_fields() -> io::Result<[u64; TABLE_COUNT]> {
    let mut masks = [u64::MAX; TABLE_COUNT];
    for (table, names) in [
        (TableId::Countries, ["canonical_slot", "tag"].as_slice()),
        (
            TableId::CountryCounterInputs,
            ["owned_state_count"].as_slice(),
        ),
        (TableId::CountryExperienceGains, [].as_slice()),
    ] {
        masks[table.index()] = names.iter().try_fold(0_u64, |mask, name| {
            Ok::<_, io::Error>(mask | 1_u64 << columns::field(table, name)?)
        })?;
    }
    Ok(masks)
}

/// What a worker reads of its walk's world: [`FRAME`], each in the fields it reads.
pub fn needs() -> io::Result<Needs> {
    let masks = read_fields()?;
    Ok(Needs {
        frame: FRAME
            .iter()
            .map(|&table| {
                let mask = masks[table.index()];
                (
                    table,
                    if mask == u64::MAX {
                        Hold::Whole
                    } else {
                        Hold::Fields(mask)
                    },
                )
            })
            .collect(),
        ..Needs::default()
    })
}

/// Both sides observe an hour whole; nothing cancels inside one.
static WHOLE_HOUR: AtomicBool = AtomicBool::new(false);

/// A table's change at an hour: its whole view, or its changed rows and deleted keys.
pub(super) enum Change {
    Replace(replay_frame::frame::SharedView),
    Patch {
        rows: TableRows,
        deleted: Vec<u128>,
        keys_changed: bool,
    },
}

/// What a worker made of a pack, as a reduction that succeeds leaves it.
enum Facts {
    /// The occurrences' notices and facts, in order, and the pack's own retained-state
    /// observation notes.
    Drafted(Vec<Notice>, Vec<Draft>, native::retained::Observations),
    /// An occurrence dated before the walk, which the emit names: the hour's proven charges.
    Deferred(BTreeMap<ChargeKey, Vec<super::ScriptCost>>),
}

enum Native {
    /// Why the hour has no pack to observe.
    Missing(String),
    Packet(Arc<PreparedHistory>, Facts),
}

/// The hour's strikes: the read's notices, then their facts, or the strikes themselves when
/// one is dated before the walk.
enum Nuclear {
    Drafted(Vec<Notice>, Vec<Draft>),
    Deferred(Vec<Notice>, Vec<NuclearStrikeRow>),
}

/// One hour of a segment, as its worker prepared it.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// The walk reads every table whole at this hour.
    opens: bool,
    /// The walk's account of what its packs wrote ([`Written`]) begins at this hour.
    fresh: bool,
    metrics: Metrics,
    /// The identity tables' changes, for the emit's own frame.
    changes: Vec<(TableId, Arc<Change>)>,
    native: Native,
    /// Whether an owner's relations were read, and the guarantees `(first, second, start)`
    /// the walk read first there with their start reached, in the order they were read.
    relations: bool,
    guarantees: Vec<(u32, u32, i32)>,
    nuclear: Nuclear,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        let changes: usize = self
            .changes
            .iter()
            .map(|(_, change)| {
                size_of::<Change>()
                    + match &**change {
                        // Its own once the walk's next build copies it.
                        Change::Replace(view) => view_heap_bytes(view),
                        Change::Patch { rows, deleted, .. } => {
                            rows.heap_bytes() + deleted.capacity() * size_of::<u128>()
                        }
                    }
            })
            .sum();
        // A pack goes to its supplying hour alone, which keeps it once the projector moves on.
        let native = match &self.native {
            Native::Missing(reason) => reason.capacity(),
            Native::Packet(pack, Facts::Drafted(notices, drafts, (notes, tags))) => {
                size_of::<PreparedHistory>()
                    + pack.heap_bytes()
                    + notices_heap(notices)
                    + drafts_heap(drafts)
                    + notes.capacity() * size_of::<String>()
                    + tags.len() * size_of::<String>()
                    + notes
                        .iter()
                        .chain(tags)
                        .map(String::capacity)
                        .sum::<usize>()
            }
            Native::Packet(pack, Facts::Deferred(proven)) => {
                size_of::<PreparedHistory>()
                    + pack.heap_bytes()
                    + proven
                        .iter()
                        .map(|((_, _, token, _), costs)| {
                            size_of::<(ChargeKey, Vec<super::ScriptCost>)>()
                                + token.capacity()
                                + costs.capacity() * size_of::<super::ScriptCost>()
                                + costs
                                    .iter()
                                    .map(|cost| cost.amount.capacity())
                                    .sum::<usize>()
                        })
                        .sum::<usize>()
            }
        };
        let nuclear = match &self.nuclear {
            Nuclear::Drafted(notices, drafts) => notices_heap(notices) + drafts_heap(drafts),
            Nuclear::Deferred(notices, strikes) => {
                notices_heap(notices)
                    + strikes.capacity() * size_of::<NuclearStrikeRow>()
                    + strikes.iter().map(RowCodec::heap_bytes).sum::<usize>()
            }
        };
        self.changes.capacity() * size_of::<(TableId, Arc<Change>)>()
            + changes
            + native
            + self.guarantees.capacity() * size_of::<(u32, u32, i32)>()
            + nuclear
    }
}

/// The bytes `notices` own on the heap.
fn notices_heap(notices: &Vec<Notice>) -> usize {
    notices.capacity() * size_of::<Notice>()
        + notices
            .iter()
            .map(|notice| {
                notice.samples.capacity() * size_of::<String>()
                    + [&notice.key.0, &notice.key.1, &notice.reason]
                        .into_iter()
                        .chain(&notice.key.2)
                        .chain(&notice.samples)
                        .map(String::capacity)
                        .sum::<usize>()
            })
            .sum::<usize>()
}

/// The bytes `drafts` own on the heap.
fn drafts_heap(drafts: &Vec<Draft>) -> usize {
    drafts.capacity() * size_of::<Draft>() + drafts.iter().map(Draft::heap).sum::<usize>()
}

/// A segment's walk: its identity tables' changes, and the identity history and tags it names
/// countries with from its first hour on.
pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    bounds: BTreeMap<u64, (i32, i32)>,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    masks: [u64; TABLE_COUNT],
    /// [`FRAME`] as the worker asks the walk's frame for it.
    holds: Vec<(TableId, Hold)>,
    packet: Option<(i32, Result<Arc<PreparedHistory>, String>)>,
    identities: HistoryScan,
    tags: LinkTags,
    written: Written,
    fresh: bool,
    /// The walk's first hour and its branch's bounds.
    first: i32,
    within: (i32, i32),
    /// The guarantees the walk handed on.
    sent: BTreeSet<(u32, u32, i32)>,
    types: BTreeMap<u32, u32>,
}

impl Projector {
    pub fn new(replay: &Replay) -> io::Result<Self> {
        let bounds = replay
            .branches
            .iter()
            .map(|start| (start.id, branch_bounds(replay, start)))
            .collect();
        Self::within(replay, bounds)
    }

    /// [`Self::new`] of a recording whose branches' first and last hours are `bounds`.
    pub fn within(replay: &Replay, bounds: BTreeMap<u64, (i32, i32)>) -> io::Result<Self> {
        let masks = read_fields()?;
        Ok(Self {
            format: replay.format,
            mode: replay.capture_mode,
            bounds,
            shapes: [None; TABLE_COUNT],
            masks,
            holds: needs()?.frame,
            packet: None,
            identities: HistoryScan::new(false, &[]),
            tags: LinkTags::default(),
            written: Written::default(),
            fresh: false,
            first: 0,
            within: (0, 0),
            sent: BTreeSet::new(),
            types: strike_types(),
        })
    }

    fn read_packet(
        &mut self,
        hour: &segments::Hour<'_, World>,
    ) -> io::Result<Result<Arc<PreparedHistory>, String>> {
        let table = TableId::EventFeedHistory;
        let held = columns::table(hour.state, table)?;
        let edits = columns::changes(hour.state, table)?;
        let status = columns::status(hour.state, table);
        if status == columns::Status::Complete {
            if hour.starts
                || hour.opens
                || self.shapes[table.index()] != Some(status)
                || self.packet.is_none()
                || !edits.changed.is_empty()
                || !edits.fill_slots.is_empty()
                || !edits.inserted.is_empty()
                || !edits.deleted_keys.is_empty()
            {
                let mut slots = held.live().ones();
                self.packet = match (slots.next(), slots.next()) {
                    (Some(slot), None) => {
                        let row = columns::row::<replay_data::event_feed_rows::EventFeedHistoryRow>(
                            hour.state,
                            table,
                            slot,
                            self.format.version(),
                        )?;
                        let read_hour = row.read_hour;
                        let packet = PreparedHistory::following(row, &mut self.written)
                            .map(Arc::new)
                            .map_err(|error| error.to_string());
                        if read_hour != hour.hour {
                            // This hour reduces no pack, so no later plan may count on its writes.
                            self.written = Written::default();
                        }
                        Some((read_hour, packet))
                    }
                    _ => None,
                };
            }
            return Ok(match &self.packet {
                Some((read_hour, packet)) if *read_hour == hour.hour => packet.clone(),
                _ => Err("Event history has no unique pack for its supplying hour.".into()),
            });
        }
        self.packet = None;
        let view = match status {
            columns::Status::Truncated { missing } => TableView::Truncated {
                rows: TableRows::EventFeedHistory(Vec::new()),
                missing,
            },
            columns::Status::Unavailable { reason } => {
                let reason =
                    hour.state.dictionary().get(reason).ok_or_else(|| {
                        crate::invalid("Feed packet refusal has no dictionary entry")
                    })?;
                TableView::Unavailable {
                    reason: std::str::from_utf8(reason)
                        .map_err(|_| crate::invalid("Feed packet refusal is not text"))?
                        .to_owned(),
                }
            }
            columns::Status::NotRecorded => TableView::NotRecorded { mode: self.mode },
            columns::Status::Complete => unreachable!(),
        };
        let mut frame = empty_frame(hour.branch, hour.hour, metrics(hour.manifest));
        frame.tables[table.index()] = view.into();
        Ok(Err(frame
            .complete::<tables::EventFeedHistory>()
            .unwrap_err()
            .to_string()))
    }

    /// This hour's reads of the relations' owners, as [`guarantees`] reads them, or none while
    /// the slice reads are incomplete. A read of this hour was written this hour, so only the
    /// slots written are read, but where the walk reads its tables whole.
    fn relation_reads(
        &self,
        hour: &segments::Hour<'_, World>,
    ) -> io::Result<Option<Vec<SliceReadRow>>> {
        let table = TableId::SliceReads;
        if !columns::complete(hour.state, table) {
            return Ok(None);
        }
        let held = columns::table(hour.state, table)?;
        let mut slots: Vec<u32> = if hour.starts
            || hour.opens
            || self.shapes[table.index()] != Some(columns::Status::Complete)
        {
            held.live().ones().collect()
        } else {
            let edits = columns::changes(hour.state, table)?;
            edits
                .inserted
                .iter()
                .chain(&edits.changed)
                .chain(&edits.fill_slots)
                .copied()
                .collect()
        };
        slots.sort_unstable();
        slots.dedup();
        let mut reads = Vec::new();
        for slot in slots {
            let read =
                columns::row::<SliceReadRow>(hour.state, table, slot, self.format.version())?;
            if diplomacy::reads_slice(&read, hour.hour) {
                reads.push(read);
            }
        }
        reads.sort_unstable_by_key(|read| read.owner);
        Ok(Some(reads))
    }

    fn read<T: TableSchema>(
        &self,
        hour: &segments::Hour<'_, World>,
        reduce: impl Fn(T::Row) -> T::Row,
    ) -> io::Result<Option<Change>> {
        let table = T::ID;
        let index = table.index();
        let status = columns::status(hour.state, table);
        let held = columns::table(hour.state, table)?;
        let edits = columns::changes(hour.state, table)?;
        let whole =
            hour.starts || hour.opens || self.shapes[index] != Some(columns::Status::Complete);
        let rows = |slots: Vec<u32>| -> io::Result<TableRows> {
            let mut rows = slots
                .into_iter()
                .map(|slot| {
                    columns::row::<T::Row>(hour.state, table, slot, self.format.version())
                        .map(&reduce)
                })
                .collect::<io::Result<Vec<_>>>()?;
            rows.sort_unstable_by_key(|row| row.key());
            Ok(T::wrap(rows))
        };
        let replace = match status {
            columns::Status::Complete if whole => {
                Some(TableView::Complete(rows(held.live().ones().collect())?))
            }
            columns::Status::Complete => {
                let mask = self.masks[index];
                let mut slots: BTreeSet<u32> = edits.inserted.iter().copied().collect();
                for (changed, masks) in [
                    (&edits.changed, &edits.masks),
                    (&edits.fill_slots, &edits.fill_masks),
                ] {
                    if changed.len() != masks.len() {
                        return Err(crate::invalid("Event feed slot masks miss their rows"));
                    }
                    slots.extend(
                        changed
                            .iter()
                            .zip(masks)
                            .filter(|(_, fields)| **fields & mask != 0)
                            .map(|(&slot, _)| slot),
                    );
                }
                if slots.is_empty() && edits.deleted_keys.is_empty() {
                    return Ok(None);
                }
                let rows = rows(slots.into_iter().collect())?;
                let mut deleted = edits.deleted_keys.clone();
                deleted.sort_unstable();
                deleted.dedup();
                let fresh = T::borrow(&rows).expect("projected table rows");
                deleted.retain(|key| fresh.binary_search_by_key(key, |row| row.key()).is_err());
                return Ok(Some(Change::Patch {
                    rows,
                    deleted,
                    keys_changed: !edits.inserted.is_empty() || !edits.deleted_keys.is_empty(),
                }));
            }
            columns::Status::Truncated { missing } => Some(TableView::Truncated {
                // Incomplete rows are never read; diagnostics use only their status.
                rows: T::wrap(Vec::new()),
                missing,
            }),
            columns::Status::Unavailable { reason } => {
                let bytes = hour.state.dictionary().get(reason).ok_or_else(|| {
                    crate::invalid("Event feed table reason is missing from its dictionary")
                })?;
                Some(TableView::Unavailable {
                    reason: std::str::from_utf8(bytes)
                        .map_err(|_| crate::invalid("Event feed table reason is not text"))?
                        .to_owned(),
                })
            }
            columns::Status::NotRecorded => Some(TableView::NotRecorded { mode: self.mode }),
        };
        if !hour.starts && !hour.opens && self.shapes[index] == Some(status) {
            return Ok(None);
        }
        Ok(replace.map(|view| Change::Replace(view.into())))
    }
}

fn empty_frame(branch: u64, hour: i32, metrics: Metrics) -> DecodedFrame {
    DecodedFrame {
        branch,
        hour,
        metrics,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: String::new(),
            }
            .into()
        }),
    }
}

fn patch<T: TableSchema>(
    view: &mut TableView,
    fresh: TableRows,
    deleted: Vec<u128>,
) -> io::Result<()> {
    let TableView::Complete(held) = view else {
        return Err(crate::invalid("Event feed patch has no complete baseline"));
    };
    let held = T::borrow_mut(held)
        .ok_or_else(|| crate::invalid("Event feed patch baseline holds another table"))?;
    let previous = T::wrap(std::mem::take(held));
    let next = previous.apply_changes(RowChanges::new(fresh, deleted))?;
    *held = T::take(next).map_err(|_| crate::invalid("Event feed patch holds another table"))?;
    Ok(())
}

/// `changes` applied to `held`, the frame of the hour before, and the hour's change facts.
fn apply_frame(
    held: &mut Option<DecodedFrame>,
    (branch, hour): (u64, i32),
    opens: bool,
    metrics: &Metrics,
    changes: &[(TableId, Arc<Change>)],
    format: FormatVersion,
) -> io::Result<FrameChanges> {
    let frame = held.get_or_insert_with(|| empty_frame(branch, hour, Metrics::default()));
    if frame.branch != branch {
        return Err(crate::invalid("Feed stitch crossed branches"));
    }
    frame.hour = hour;
    frame.metrics = metrics.clone();
    let mut changed = [opens; TABLE_COUNT];
    for (table, change) in changes {
        changed[table.index()] = true;
        match &**change {
            Change::Replace(view) => frame.tables[table.index()] = view.clone(),
            Change::Patch { rows, deleted, .. } => {
                macro_rules! apply {
                    ($($table:ident),+ $(,)?) => {
                        match table {
                            $(TableId::$table => patch::<tables::$table>(
                                &mut frame.tables[table.index()], rows.clone(), deleted.clone())?,)+
                            _ => return Err(crate::invalid("Feed stitch received another table")),
                        }
                    };
                }
                apply!(
                    Campaign,
                    Countries,
                    CountryLinks,
                    CountryCounterInputs,
                    CountryExperienceGains,
                    RelationTuples,
                    NuclearStrikes
                );
            }
        }
    }
    let mut changes = FrameChanges::alone(frame, format);
    changes.rows_changed = changed;
    Ok(changes)
}

fn identity_table(table: TableId) -> bool {
    matches!(
        table,
        TableId::Campaign
            | TableId::Countries
            | TableId::CountryLinks
            | TableId::CountryCounterInputs
            | TableId::CountryExperienceGains
    )
}

/// The identity history at `hour`: a quiet step when no country key, link or gain moved and
/// the counters moved only in value, or else the hour's whole observation. Whether it was a
/// quiet step, which names the hour as the hour before, and whether its witnesses were read.
fn advance_identities(
    scan: &mut HistoryScan,
    (hour, opens): (i32, bool),
    changes: &[(TableId, Arc<Change>)],
    frame: &DecodedFrame,
    facts: &FrameChanges,
) -> (bool, bool) {
    let same = !opens
        && changes.iter().all(|(table, change)| match table {
            TableId::Countries | TableId::CountryLinks | TableId::CountryExperienceGains => false,
            TableId::CountryCounterInputs => matches!(
                &**change,
                Change::Patch {
                    keys_changed: false,
                    ..
                }
            ),
            _ => true,
        });
    let witnessed = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryExperienceGains,
    ]
    .iter()
    .all(|table| frame.metrics.table_ns[table.index()] != 0);
    if same && scan.advance_unchanged(hour, witnessed) {
        return (true, witnessed);
    }
    scan.observe_owned_states(frame, Some(facts), |_, _, _| {});
    (false, witnessed)
}

/// The charges `hour`'s power changes prove ([`charges_of`]), read whole at an hour whose
/// facts may hold one.
fn charges_at(
    hour: &segments::Hour<'_, World>,
    format: FormatVersion,
) -> io::Result<BTreeMap<ChargeKey, Vec<super::ScriptCost>>> {
    if !columns::complete(hour.state, TableId::CountryPowerChanges)
        || !columns::complete(hour.state, TableId::PoliticsCaptureCoverage)
    {
        return Ok(BTreeMap::new());
    }
    let changes = whole::<tables::CountryPowerChanges>(hour, format)?;
    let coverage = whole::<tables::PoliticsCaptureCoverage>(hour, format)?;
    Ok(charges_of(&changes, &coverage))
}

/// Every live row of `T` at `hour`.
fn whole<T: TableSchema>(
    hour: &segments::Hour<'_, World>,
    format: FormatVersion,
) -> io::Result<Vec<T::Row>> {
    columns::table(hour.state, T::ID)?
        .live()
        .ones()
        .map(|slot| columns::row::<T::Row>(hour.state, T::ID, slot, format.version()))
        .collect()
}

/// Whether a fact of `kind` can hold a decision charge.
fn charged(kind: EventFeedKind) -> bool {
    matches!(
        kind,
        EventFeedKind::DecisionSelected | EventFeedKind::DecisionTimeoutEffect
    )
}

/// What a worker makes of a pack: its occurrences' notices and facts and its own retained-state
/// observation notes, named from `first` on, with the `charges` of its hour.
fn draft(
    packet: &PreparedHistory,
    identities: &HistoryScan,
    first: i32,
    (start, end): (i32, i32),
    charges: impl FnOnce() -> io::Result<BTreeMap<ChargeKey, Vec<super::ScriptCost>>>,
) -> io::Result<Facts> {
    let pack = packet.row();
    if pack.events.iter().any(|event| event.hour < first) {
        let decided = pack.events.iter().any(|event| {
            matches!(
                event.source,
                EventFeedSource::DecisionSelected | EventFeedSource::DecisionTimeoutEffect
            )
        });
        return Ok(Facts::Deferred(if decided {
            charges()?
        } else {
            BTreeMap::new()
        }));
    }
    let mut builder = Builder::new(start, end);
    builder.captured = Some(Vec::new());
    native::Projection::occurrences(
        &mut builder,
        pack,
        |hour, country| identities.native_tag(hour, country),
        &mut native::Rosters::new(),
        &WHOLE_HOUR,
    )?;
    let mut proven = if builder.feed.rows.iter().any(|row| charged(row.kind)) {
        charges()?
    } else {
        BTreeMap::new()
    };
    let drafts = builder.drafts(
        &mut proven,
        |hour, country| identities.native_scope_slot(hour, country),
        false,
    )?;
    let observations = native::retained::observations(pack, &mut |hour, country| {
        identities.native_tag(hour, country)
    });
    Ok(Facts::Drafted(
        builder.captured.take().unwrap_or_default(),
        drafts,
        observations,
    ))
}

/// The hour's read owners' guarantees the walk had not handed on, with their start reached,
/// and whether any owner's relations were read: of `reads`, the hour's reads of the relations'
/// owners in owner order ([`Projector::relation_reads`]).
fn guarantees(
    frame: &DecodedFrame,
    reads: Option<&[SliceReadRow]>,
    tags: &mut LinkTags,
    sent: &mut BTreeSet<(u32, u32, i32)>,
) -> (bool, Vec<(u32, u32, i32)>) {
    let mut relations = false;
    let mut seen = Vec::new();
    if tags.update(frame).is_err() {
        return (relations, seen);
    }
    let (Ok(rows), Some(reads)) = (frame.complete::<tables::RelationTuples>(), reads) else {
        return (relations, seen);
    };
    for read in reads.iter().filter(|read| tags.get(read.owner).is_some()) {
        relations = true;
        let first = rows.partition_point(|row| row.owner_country < read.owner);
        let last = rows.partition_point(|row| row.owner_country <= read.owner);
        for row in rows[first..last]
            .iter()
            .filter(|row| diplomacy::reads_relation(row))
        {
            // A relation counts at its first read with its start reached; later reads see it held.
            let relation = (row.first_country, row.second_country, row.start_hour);
            if row.start_hour <= frame.hour && sent.insert(relation) {
                seen.push(relation);
            }
        }
    }
    (relations, seen)
}

/// The hour's strikes as a worker names them from `first` on.
fn strikes(
    frame: &DecodedFrame,
    identities: &HistoryScan,
    first: i32,
    (start, end): (i32, i32),
    types: &BTreeMap<u32, u32>,
) -> io::Result<Nuclear> {
    let mut builder = Builder::new(start, end);
    builder.captured = Some(Vec::new());
    let strikes = nuclear::read(frame, &mut builder, &WHOLE_HOUR)?;
    if strikes.iter().any(|strike| strike.hour < first) {
        return Ok(Nuclear::Deferred(
            builder.captured.take().unwrap_or_default(),
            strikes,
        ));
    }
    let facts = strike_facts(
        frame.hour,
        &strikes,
        |hour, country| identities.native_tag(hour, country),
        &mut builder,
        types,
    );
    let drafts = facts
        .into_iter()
        .map(|fact| Draft::new(fact, false))
        .collect::<io::Result<_>>()?;
    Ok(Nuclear::Drafted(
        builder.captured.take().unwrap_or_default(),
        drafts,
    ))
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        self.within = *self
            .bounds
            .get(&branch)
            .ok_or_else(|| crate::invalid("Event feed branch is not recorded"))?;
        self.first = first;
        self.shapes = [None; TABLE_COUNT];
        self.packet = None;
        self.identities = HistoryScan::new(false, &[]);
        self.tags = LinkTags::default();
        self.written = Written::default();
        self.fresh = true;
        self.sent.clear();
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("event-feed-render", None);
        let probe = replay_frame::cpu::scope("ef-w:tables", None);
        let mut changes = Vec::new();
        macro_rules! read {
            ($table:ident, $reduce:expr) => {
                if let Some(change) = self.read::<tables::$table>(hour, $reduce)? {
                    changes.push((TableId::$table, Arc::new(change)));
                }
            };
        }
        read!(Campaign, |row| row);
        read!(Countries, |row: CountryRow| CountryRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
            tag: row.tag,
            ..Default::default()
        });
        read!(CountryLinks, |row| row);
        read!(CountryCounterInputs, |row: CountryCounterInputRow| {
            CountryCounterInputRow {
                country: row.country,
                owned_state_count: row.owned_state_count,
                ..Default::default()
            }
        });
        read!(CountryExperienceGains, |row: CountryExperienceGainRow| {
            CountryExperienceGainRow {
                country: row.country,
                instance: row.instance,
                ..Default::default()
            }
        });
        let reads = self.relation_reads(hour)?;
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:pack", None);
        let packet = self.read_packet(hour)?;
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:frame", None);
        for &table in TABLES {
            self.shapes[table.index()] = Some(columns::status(hour.state, table));
        }
        let opens = hour.opens || hour.starts;
        let measured = metrics(hour.manifest);
        let frame = &hour.world.frame_held(hour.state, &self.holds)?;
        let mut flags = [opens; TABLE_COUNT];
        for (table, _) in &changes {
            flags[table.index()] = true;
        }
        for table in [TableId::RelationTuples, TableId::NuclearStrikes] {
            flags[table.index()] |= hour.world.touched(table, u64::MAX);
        }
        let mut changed = FrameChanges::alone(frame, self.format);
        changed.rows_changed = flags;
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:identities", None);
        advance_identities(
            &mut self.identities,
            (hour.hour, opens),
            &changes,
            frame,
            &changed,
        );
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:draft", None);
        let format = self.format;
        let native = match packet {
            Ok(packet) => {
                let drafted = draft(&packet, &self.identities, self.first, self.within, || {
                    charges_at(hour, format)
                })?;
                Native::Packet(packet, drafted)
            }
            Err(reason) => Native::Missing(reason),
        };
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:guarantees", None);
        let (relations, guarantees) =
            guarantees(frame, reads.as_deref(), &mut self.tags, &mut self.sent);
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:strikes", None);
        let nuclear = strikes(
            frame,
            &self.identities,
            self.first,
            self.within,
            &self.types,
        )?;
        drop(probe);
        changes.retain(|(table, _)| identity_table(*table));
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            opens,
            fresh: std::mem::take(&mut self.fresh),
            metrics: measured,
            changes,
            native,
            relations,
            guarantees,
            nuclear,
        })
    }
}

/// What the emit knows of the owned-state counter rows between their reads: a read observes
/// the rows changed or gone since the last and those refusing at every read. Every other row
/// is a known, unshared holder whose count did not move, which a read leaves as it was but
/// for its read hour: that of every live slot is `hour`, written into the existence only when
/// the slot is next observed or saved.
#[derive(Default)]
struct Counters {
    /// The memo below is the last read's, under the names this hour still holds.
    known: bool,
    dirty: BTreeSet<u32>,
    /// Each row's slot, the rows refusing at every read, and each slot's holders.
    holds: BTreeMap<u32, u32>,
    refusing: BTreeSet<u32>,
    holders: BTreeMap<u32, u32>,
    shared: usize,
    live: BTreeSet<u32>,
    hour: i32,
}

impl Counters {
    /// The hour's identity changes: a moved name or a replaced counter table needs every row.
    fn note(&mut self, changes: &[(TableId, Arc<Change>)]) -> io::Result<()> {
        for (table, change) in changes {
            match (table, &**change) {
                (TableId::CountryCounterInputs, Change::Patch { rows, deleted, .. }) => {
                    let rows = rows
                        .rows::<tables::CountryCounterInputs>()
                        .ok_or_else(|| crate::invalid("Counter changes hold another table"))?;
                    self.dirty.extend(rows.iter().map(|row| row.country));
                    for &key in deleted {
                        self.dirty.insert(u32::try_from(key).map_err(|_| {
                            crate::invalid("Counter deletion exceeds its country key")
                        })?);
                    }
                }
                (TableId::Countries | TableId::CountryLinks | TableId::CountryCounterInputs, _) => {
                    self.known = false;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `country`'s row now holds `slot`.
    fn hold(&mut self, country: u32, slot: Option<u32>) {
        if let Some(old) = self.holds.remove(&country)
            && let Some(count) = self.holders.get_mut(&old)
        {
            *count -= 1;
            if *count == 1 {
                self.shared -= 1;
            } else if *count == 0 {
                self.holders.remove(&old);
            }
        }
        if let Some(slot) = slot {
            self.holds.insert(country, slot);
            let count = self.holders.entry(slot).or_default();
            *count += 1;
            if *count == 2 {
                self.shared += 1;
            }
        }
    }

    /// `slot`'s read hour written into its existence, which reads no longer refresh unseen.
    fn freeze(&mut self, observer: &mut diplomacy::Observer, slot: u32) {
        if self.live.remove(&slot) {
            observer.read_at(slot, self.hour);
        }
    }

    /// Every live slot's read hour written into its existence, which stays live.
    fn settle(&self, observer: &mut diplomacy::Observer) {
        for &slot in &self.live {
            observer.read_at(slot, self.hour);
        }
    }

    /// The counters read at `hour`, as the row path observes every row.
    fn read(
        &mut self,
        observer: &mut diplomacy::Observer,
        countries: &[CountryRow],
        rows: &[CountryCounterInputRow],
        hour: i32,
        out: &mut country_groups::changes::Scan,
    ) -> io::Result<()> {
        let find = move |country: u32| {
            rows.binary_search_by_key(&country, |row| row.country)
                .ok()
                .map(|at| &rows[at])
        };
        let dirty = std::mem::take(&mut self.dirty);
        if self.known {
            for &country in &dirty {
                if let Some(&slot) = self.holds.get(&country) {
                    self.freeze(observer, slot);
                }
                let slot = find(country)
                    .and_then(|row| observer.holder(countries, row))
                    .map(|(slot, _)| slot);
                self.hold(country, slot);
            }
            // Rows of one slot overwrite each other in country order: only a whole read keeps it.
            self.known = self.shared == 0;
        }
        if !self.known {
            self.settle(observer);
            *self = Self::default();
            for row in rows {
                let slot = observer.exist(countries, row, hour, out)?;
                self.observed(row, slot);
            }
        } else {
            let visit: Vec<u32> = dirty.union(&self.refusing).copied().collect();
            for country in visit {
                let Some(row) = find(country) else {
                    self.refusing.remove(&country);
                    continue;
                };
                if let Some(&slot) = self.holds.get(&country) {
                    self.freeze(observer, slot);
                }
                let slot = observer.exist(countries, row, hour, out)?;
                self.refusing.remove(&country);
                self.observed(row, slot);
            }
        }
        self.known = self.shared == 0;
        self.hour = hour;
        Ok(())
    }

    /// `row` just observed, naming `slot`.
    fn observed(&mut self, row: &CountryCounterInputRow, slot: Option<u32>) {
        if !self
            .holds
            .get(&row.country)
            .is_some_and(|&held| Some(held) == slot)
        {
            self.hold(row.country, slot);
        }
        match slot {
            Some(slot) if row.owned_state_count >= 0 => {
                self.live.insert(slot);
            }
            _ => {
                self.refusing.insert(row.country);
            }
        }
    }
}

/// The hours the emit's identity history names alike: `since` on, through the last observed.
#[derive(Clone, Copy, Default)]
struct Naming {
    since: Option<i32>,
    /// The last hour's witnesses were read.
    read: bool,
}

impl Naming {
    /// After the history's step at `hour`: `quiet` whether it named the hour as the hour before,
    /// `read` whether the hour's witnesses were read. A quiet step after an unread hour names its
    /// hour anew.
    fn advance(&mut self, hour: i32, (quiet, read): (bool, bool)) {
        if !(quiet && self.read) {
            self.since = Some(hour);
        }
        self.read = read;
    }
}

/// Each roster owner's target countries with their slots, as last named while the hours from
/// the first value on are named alike ([`Naming`]).
type Rostered = HashMap<u32, (i32, Vec<([u32; 3], Option<u32>)>)>;

/// `captured`'s target countries named at `hour`: as its owner's last rebuild named the same
/// countries, while the hours since are named alike, or each anew.
fn name_targets(
    rostered: &mut Rostered,
    naming: Naming,
    identities: &HistoryScan,
    hour: i32,
    captured: &mut Captured,
) {
    let since = naming.since.filter(|since| *since <= hour);
    if let Some(since) = since
        && let Some((named, countries)) = rostered.get(&captured.owner)
        && *named == since
        && countries.len() == captured.countries.len()
        && countries
            .iter()
            .zip(&captured.countries)
            .all(|(named, country)| named.0 == country.0)
    {
        captured.countries.clone_from(countries);
        return;
    }
    for (country, slot) in &mut captured.countries {
        let [reference, id, instance] = *country;
        *slot = identities.native_scope_slot(
            hour,
            EventFeedCountry {
                reference,
                id,
                instance,
            },
        );
    }
    if let Some(since) = since {
        rostered.insert(captured.owner, (since, captured.countries.clone()));
    }
}

/// Facts a thread writes at a time: a few days of roster rebuilds, so threads start rarely.
const BATCH: usize = 256;
/// Batches written at once: past them the emit appends the oldest before sending another.
const WRITING: usize = 2;

/// Facts whose bytes another thread writes ([`Draft::write`]) while the emit goes on, appended
/// in their publication's order.
#[derive(Default)]
struct Writing {
    /// Facts held since the last batch went, with their keys.
    open: Vec<(u64, Draft)>,
    /// Batches being written, oldest first.
    sent: VecDeque<JoinHandle<io::Result<Vec<(u64, i32, Vec<u8>)>>>>,
}

impl Writing {
    fn hold(&mut self, facts: Vec<(u64, Draft)>) {
        self.open.extend(facts);
    }

    /// Appends the batches already written, in order, and sends a full batch to be written.
    fn advance(&mut self, feed: &mut Observer) -> io::Result<()> {
        while self.sent.front().is_some_and(JoinHandle::is_finished) {
            self.append(feed)?;
        }
        if self.open.len() >= BATCH {
            while self.sent.len() >= WRITING {
                self.append(feed)?;
            }
            self.send()?;
        }
        Ok(())
    }

    /// Every fact held, written and appended.
    fn finish(&mut self, feed: &mut Observer) -> io::Result<()> {
        if !self.open.is_empty() {
            self.send()?;
        }
        while !self.sent.is_empty() {
            self.append(feed)?;
        }
        Ok(())
    }

    fn send(&mut self) -> io::Result<()> {
        let facts = std::mem::take(&mut self.open);
        let thread = std::thread::Builder::new()
            .name("event feed facts".into())
            .spawn(move || {
                let _cpu = replay_frame::cpu::scope("event-feed-facts", None);
                facts
                    .into_iter()
                    .map(|(key, draft)| {
                        let hour = draft.read_hour();
                        Ok((key, hour, draft.write()?))
                    })
                    .collect()
            })?;
        self.sent.push_back(thread);
        Ok(())
    }

    fn append(&mut self, feed: &mut Observer) -> io::Result<()> {
        let Some(thread) = self.sent.pop_front() else {
            return Ok(());
        };
        let written = thread
            .join()
            .map_err(|_| crate::invalid("An event-feed fact writer panicked"))??;
        for (key, hour, bytes) in written {
            feed.written(key, hour, bytes)?;
        }
        Ok(())
    }
}

/// One branch's ordered preparation. It holds every state the feed carries, which is what its
/// sections save, but for the identity tables last read, the counters' memo, the facts still
/// being written and what names roster targets alike.
pub struct Prepared {
    identities: HistoryScan,
    feed: Observer,
    frame: Option<DecodedFrame>,
    format: FormatVersion,
    bounds: Option<(i32, i32)>,
    counters: Counters,
    types: BTreeMap<u32, u32>,
    writing: Writing,
    /// The walk reduced every pack since its account of what they wrote began.
    sure: bool,
    naming: Naming,
    rostered: Rostered,
}

impl Prepared {
    pub fn new(replay: &Replay, branch: u64) -> Self {
        Self::restore(
            replay,
            branch,
            HistoryScan::new(false, &[]),
            Observer::default(),
        )
    }

    /// [`Self::new`] of a branch whose first and last hours are `bounds`.
    pub fn within(replay: &Replay, branch: u64, bounds: (i32, i32)) -> Self {
        let (identities, feed) = (HistoryScan::new(false, &[]), Observer::default());
        Self::bounded(replay, branch, Some(bounds), identities, feed)
    }

    /// The preparation its saved sections continue: the identity history and the feed.
    pub(crate) fn restore(
        replay: &Replay,
        branch: u64,
        identities: HistoryScan,
        feed: Observer,
    ) -> Self {
        let bounds = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map(|start| branch_bounds(replay, start));
        Self::bounded(replay, branch, bounds, identities, feed)
    }

    /// [`Self::restore`] of a branch whose first and last hours are `bounds`, when recorded.
    fn bounded(
        replay: &Replay,
        branch: u64,
        bounds: Option<(i32, i32)>,
        identities: HistoryScan,
        mut feed: Observer,
    ) -> Self {
        if let Some((first, last)) = bounds {
            feed.diagnostics.feed.first = first;
            feed.diagnostics.feed.last = last;
        }
        feed.native.losses(replay, branch);
        Self {
            identities,
            feed,
            frame: None,
            format: replay.format,
            bounds,
            counters: Counters::default(),
            types: strike_types(),
            writing: Writing::default(),
            sure: false,
            naming: Naming::default(),
            rostered: Rostered::new(),
        }
    }

    /// One hour, as the row path observes it: the identity history, then the feed.
    pub fn observe_projected(&mut self, hour: Hour) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("event-feed-emit", None);
        if self.bounds.is_none() {
            return Err(crate::invalid("Event feed branch is not recorded"));
        }
        let Self {
            identities,
            feed,
            frame,
            format,
            counters,
            types,
            writing,
            sure,
            naming,
            rostered,
            ..
        } = self;
        let facts = apply_frame(
            frame,
            (hour.branch, hour.hour),
            hour.opens,
            &hour.metrics,
            &hour.changes,
            *format,
        )?;
        let frame = frame.as_ref().expect("the emit's identity frame");
        let step = advance_identities(
            identities,
            (hour.hour, hour.opens),
            &hour.changes,
            frame,
            &facts,
        );
        naming.advance(hour.hour, step);
        counters.note(&hour.changes)?;
        feed.events.through(hour.hour);
        if hour.fresh {
            *sure = true;
        }
        let mut complete = false;
        let read = match hour.native {
            Native::Missing(reason) => Err(crate::invalid(reason)),
            Native::Packet(packet, facts) => {
                let pack = packet.row();
                complete =
                    pack.coverage == EventFeedCoverage::Complete && pack.script_refusals == 0;
                let naming = (*naming, &mut *rostered);
                natives(feed, identities, sure, naming, &packet, facts)
            }
        };
        let complete = complete && read.is_ok();
        let drafts = read.unwrap_or_else(|reason| {
            feed.diagnostics.sampled_unavailable(
                "nativeEventCoverage",
                "Native event coverage",
                reason.to_string(),
                1,
            );
            Vec::new()
        });
        let probe = replay_frame::cpu::scope("ef:publish", None);
        writing.hold(feed.publish(drafts, hour.hour, complete)?);
        drop(probe);
        let mut changes = country_groups::changes::Scan {
            moments: Vec::new(),
            observed: BTreeSet::new(),
            unavailable: Vec::new(),
        };
        if let Some(named) = feed.diplomacy.named(frame, &mut changes)? {
            feed.diplomacy.existence(
                frame,
                *format,
                &mut changes,
                |observer, countries, rows, out| {
                    counters.read(observer, countries, rows, hour.hour, out)
                },
            )?;
            if hour.relations {
                changes.observed.insert(TableId::RelationTuples);
            }
            for &relation in &hour.guarantees {
                feed.diplomacy
                    .guarantee(relation, named, hour.hour, &mut changes)?;
            }
        }
        feed.publish_diplomacy(changes)?;
        let strikes = match hour.nuclear {
            Nuclear::Drafted(notices, drafts) => {
                for notice in notices {
                    notice.apply(&mut feed.diagnostics);
                }
                drafts
            }
            Nuclear::Deferred(notices, strikes) => {
                for notice in notices {
                    notice.apply(&mut feed.diagnostics);
                }
                strike_facts(
                    hour.hour,
                    &strikes,
                    |hour, country| identities.native_tag(hour, country),
                    &mut feed.diagnostics,
                    types,
                )
                .into_iter()
                .map(|fact| Draft::new(fact, false))
                .collect::<io::Result<_>>()?
            }
        };
        for strike in strikes {
            feed.publish_one(strike)?;
        }
        feed.end_hour();
        writing.advance(feed)
    }

    /// Every fact published so far written and appended, which a flush and a save need.
    fn settle(&mut self) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("event-feed-settle", None);
        self.writing.finish(&mut self.feed)
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.settle()?;
        crate::prepared::pipeline::write_feed_identities(writer, branch, &mut self.identities)?;
        self.feed.flush(writer, branch, cancelled)
    }

    pub fn save_state(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        self.settle()?;
        self.counters.settle(&mut self.feed.diplomacy);
        crate::prepared::pipeline::save_feed(writer, branch, &self.feed)
    }
}

/// A pack observed in order, as the row path observes it: its notices and reduction, then its
/// occurrences' facts, a worker's held to the reduction that succeeded, then its retained state.
/// `sure` stays only once the reduction is made; roster targets are named through `naming`.
fn natives(
    feed: &mut Observer,
    identities: &HistoryScan,
    sure: &mut bool,
    (naming, rostered): (Naming, &mut Rostered),
    packet: &PreparedHistory,
    facts: Facts,
) -> io::Result<Vec<Draft>> {
    let _cpu = replay_frame::cpu::scope("event-feed-reduce", None);
    let pack = packet.row();
    let walked = std::mem::replace(sure, false);
    let probe = replay_frame::cpu::scope("ef:follows-notices", None);
    feed.native.follows(pack)?;
    feed.native.notices(&mut feed.diagnostics, pack);
    drop(probe);
    let probe = replay_frame::cpu::scope("ef:reduce", None);
    let mut captured = feed.native.reduce(packet, walked)?;
    *sure = walked;
    drop(probe);
    let probe = replay_frame::cpu::scope("ef:drafts", None);
    let mut tag_at = |hour, country| identities.native_tag(hour, country);
    let slot_at = |hour, country| identities.native_scope_slot(hour, country);
    let (mut drafts, observations) = match facts {
        Facts::Drafted(notices, mut drafts, observations) => {
            for notice in notices {
                notice.apply(&mut feed.diagnostics);
            }
            for draft in &mut drafts {
                let Some((sequence, hour)) = draft.held_roster() else {
                    continue;
                };
                let mut rebuilt = captured.remove(&sequence);
                if let Some(rebuilt) = &mut rebuilt {
                    name_targets(rostered, naming, identities, hour, rebuilt);
                }
                draft.hold(rebuilt);
            }
            (drafts, observations)
        }
        Facts::Deferred(mut proven) => {
            let mut rosters = captured
                .into_iter()
                .map(|(sequence, rebuilt)| (sequence, rebuilt.inputs()))
                .collect();
            native::Projection::occurrences(
                &mut feed.diagnostics,
                pack,
                &mut tag_at,
                &mut rosters,
                &WHOLE_HOUR,
            )?;
            let drafts = feed.diagnostics.drafts(&mut proven, slot_at, true)?;
            (drafts, native::retained::observations(pack, &mut tag_at))
        }
    };
    drop(probe);
    let _probe = replay_frame::cpu::scope("ef:retain", None);
    feed.native.settle(pack);
    let within =
        (feed.diagnostics.feed.first..=feed.diagnostics.feed.last).contains(&pack.read_hour);
    if within && let Some(observed) = feed.native.observe(pack, observations, &mut tag_at)? {
        // The retained observation sorts among the facts by its date and kind, which no
        // occurrence shares.
        let observed = Draft::observed(observed);
        let at = drafts.partition_point(|draft| draft.order() < observed.order());
        drafts.insert(at, observed);
    }
    Ok(drafts)
}

#[cfg(test)]
mod tests;
