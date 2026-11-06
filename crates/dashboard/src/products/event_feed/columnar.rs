//! The feed prepared on a columnar recording's segments ([`replay::segments`]). Each segment's
//! worker reads the changed slots of the feed's tables, the rest through the walk's views
//! ([`dashboard_world`]), names occurrences at its own hours through an identity history of
//! its own, presents their items in the hour's views and encodes their facts; the ordered emit
//! holds every state the feed carries, its saved continuation, and advances it once per hour.

use super::{
    Builder, EventFeedItem, EventFeedKind, EventFeedRow, HourTexts, Notice, Shown,
    country_groups, diplomacy, native, nuclear,
    prepared::{ChargeKey, Draft, Observer, Present, charges_of, presented, strike_facts, strike_types},
    script_effects,
};
use crate::catalogue::Catalogue;
use crate::{
    metadata::{LinkTags, branch_bounds},
    prepared::frame_changes::FrameChanges,
    products::country_history::HistoryScan,
};
use dashboard_keyed::table_changes::{self, Change, apply_frame, empty_frame};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::metrics,
    columns,
    container::Replay,
    segments::{self, Operator, Sink, Start},
};
use replay_data::{
    event_feed_rows::{
        EventFeedCoverage, EventFeedHistoryRow, EventFeedSource, state::PackHeader,
    },
    format::FormatVersion,
    rows::{
        CountryCounterInputRow, CountryExperienceGainRow, CountryRow, NuclearStrikeRow,
        RelationTupleRow, SliceReadRow, TABLE_COUNT, TableId, TableRows, TableSchema,
        tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, HourFrame, Metrics, TableView};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// The tables the feed and its identity history read.
const OWN: &[TableId] = &[
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

/// The tables an occurrence's text is expanded in at the hour that read it: country names and
/// every table scripted text reads.
pub fn presentation() -> &'static [TableId] {
    static TABLES: std::sync::OnceLock<Vec<TableId>> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let mut tables: BTreeSet<TableId> =
            installed_content_country_names::country_names::TABLES.into_iter().collect();
        tables.extend(crate::scripted_text::TABLES.iter().copied());
        tables.into_iter().collect()
    })
}

/// The tables the feed reads: its own, and those its occurrences are presented in.
pub fn tables() -> &'static [TableId] {
    static TABLES: std::sync::OnceLock<Vec<TableId>> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let mut tables: BTreeSet<TableId> = OWN.iter().copied().collect();
        tables.extend(presentation().iter().copied());
        tables.into_iter().collect()
    })
}

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

/// What a worker reads of its walk's world: the views of the strikes, handed on whole, and of
/// the tables it presents occurrences in, which scripted text reads whole. The identity tables
/// it reads by their changed slots ([`Projector::read`]).
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: presentation()
            .iter()
            .chain(&[TableId::NuclearStrikes])
            .map(|&table| (table, Hold::Whole))
            .collect(),
        ..Needs::default()
    })
}

/// Both sides observe an hour whole; nothing cancels inside one.
static WHOLE_HOUR: AtomicBool = AtomicBool::new(false);

/// What a worker made of a pack.
#[derive(replay_spool::Spooled)]
enum Facts {
    /// The occurrences' notices and facts, in order.
    Drafted(Vec<Notice>, Vec<Draft>),
    /// An occurrence dated before the walk, which the emit names: the pack whole, the hour's
    /// proven charges, and the items the worker presented in the hour's world, by row.
    Deferred(
        Arc<EventFeedHistoryRow>,
        BTreeMap<ChargeKey, Vec<super::ScriptCost>>,
        Presented,
    ),
}

#[derive(replay_spool::Spooled)]
enum Native {
    /// Why the hour has no pack to observe.
    Missing(String),
    Packet(Arc<PackHeader>, Facts),
}

/// The hour's strikes: the read's notices, then their facts, or the strikes themselves when
/// one is dated before the walk.
#[derive(replay_spool::Spooled)]
enum Nuclear {
    Drafted(Vec<Notice>, Vec<Draft>),
    Deferred(Vec<Notice>, Vec<NuclearStrikeRow>),
}

/// Items a worker presented at the hour that read them, by their rows' identities, with what
/// each showed.
type Presented = HashMap<String, (EventFeedItem, Shown)>;

/// One hour of a segment, as its worker prepared it.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// The walk reads every table whole at this hour.
    opens: bool,
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

/// A segment's walk: its identity tables' changes, and the identity history and tags it names
/// countries with from its first hour on.
pub struct Projector<'p> {
    format: FormatVersion,
    mode: CaptureMode,
    bounds: BTreeMap<u64, (i32, i32)>,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    masks: [u64; TABLE_COUNT],
    /// The identity tables in the fields read, kept from the walk's first hour by the changes
    /// it hands the emit, as the emit keeps its own ([`apply_frame`]).
    identity: Option<DecodedFrame>,
    /// The recorded content items are presented with; none without the game's content.
    catalogue: Option<Arc<Catalogue>>,
    /// The recording an occurrence named before the hour that read it seeks its world in.
    source: Option<&'p crate::Source>,
    /// The pack last read, with its supplying hour.
    packet: Option<(i32, Result<Pack, String>)>,
    identities: HistoryScan,
    tags: LinkTags,
    /// The walk's first hour and its branch's bounds.
    first: i32,
    within: (i32, i32),
    /// The guarantees the walk handed on.
    sent: BTreeSet<(u32, u32, i32)>,
    /// The keys of the relations of guarantee type the walk's relation table holds; none while
    /// it is incomplete.
    guaranteed: Option<BTreeSet<u128>>,
    types: BTreeMap<u32, u32>,
}

impl<'p> Projector<'p> {
    pub fn new(replay: &Replay) -> io::Result<Self> {
        let bounds = replay
            .branches
            .iter()
            .map(|start| (start.id, branch_bounds(replay, start)))
            .collect();
        Self::within(replay, bounds, None, None)
    }

    /// [`Self::new`] of a recording whose branches' first and last hours are `bounds`,
    /// presenting items with `catalogue` in `source`'s worlds.
    pub fn within(
        replay: &Replay,
        bounds: BTreeMap<u64, (i32, i32)>,
        catalogue: Option<Arc<Catalogue>>,
        source: Option<&'p crate::Source>,
    ) -> io::Result<Self> {
        let masks = read_fields()?;
        Ok(Self {
            format: replay.format,
            mode: replay.capture_mode,
            bounds,
            shapes: [None; TABLE_COUNT],
            masks,
            identity: None,
            catalogue,
            source,
            packet: None,
            identities: HistoryScan::new(false, &[]),
            tags: LinkTags::default(),
            first: 0,
            within: (0, 0),
            sent: BTreeSet::new(),
            guaranteed: None,
            types: strike_types(),
        })
    }

    fn read_packet(
        &mut self,
        hour: &segments::Hour<'_, World>,
    ) -> io::Result<Result<Pack, String>> {
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
                // The pack read last, which no item holds, keeps its occurrences' allocation.
                if let Some((_, Ok((row, _)))) = self.packet.take()
                    && let Ok(row) = Arc::try_unwrap(row)
                {
                    replay_data::event_feed_rows::reuse(row.events);
                }
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
                        // Only its supplying hour observes a pack.
                        let packet = if read_hour == hour.hour {
                            row.validate()
                                .and_then(|()| PackHeader::of(&row))
                                .map(|header| (Arc::new(row), Arc::new(header)))
                                .map_err(|error| error.to_string())
                        } else {
                            replay_data::event_feed_rows::reuse(row.events);
                            Err("The pack was read at another hour.".into())
                        };
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

    /// The relations of guarantee type at `hour`, kept by their keys from the table's edits and
    /// read whole where the walk reads its tables whole, before its shapes are noted.
    fn guaranteed(&mut self, hour: &segments::Hour<'_, World>) -> io::Result<()> {
        let table = TableId::RelationTuples;
        if !columns::complete(hour.state, table) {
            self.guaranteed = None;
            return Ok(());
        }
        let held = columns::table(hour.state, table)?;
        let guarantee = |key: u128| (key >> 16) as u32 == engine_native::cguarantee_relation::TYPE;
        let whole = hour.starts
            || hour.opens
            || self.shapes[table.index()] != Some(columns::Status::Complete);
        let key = |slot: u32| {
            held.key(slot)
                .ok_or_else(|| crate::invalid("A relation slot holds no row"))
        };
        match &mut self.guaranteed {
            Some(guaranteed) if !whole => {
                let edits = columns::changes(hour.state, table)?;
                for deleted in &edits.deleted_keys {
                    guaranteed.remove(deleted);
                }
                for &slot in &edits.inserted {
                    let key = key(slot)?;
                    if guarantee(key) {
                        guaranteed.insert(key);
                    }
                }
            }
            _ => {
                let mut guaranteed = BTreeSet::new();
                for slot in held.live().ones() {
                    let key = key(slot)?;
                    if guarantee(key) {
                        guaranteed.insert(key);
                    }
                }
                self.guaranteed = Some(guaranteed);
            }
        }
        Ok(())
    }

    fn read<T: TableSchema>(
        &self,
        hour: &segments::Hour<'_, World>,
        reduce: impl Fn(T::Row) -> T::Row,
    ) -> io::Result<Option<Change>> {
        let index = T::ID.index();
        table_changes::read(
            hour,
            T::ID,
            self.masks[index],
            self.shapes[index],
            (self.format, self.mode),
            &|rows| {
                let rows = T::take(rows)
                    .map_err(|_| crate::invalid("Event feed rows hold another table"))?;
                Ok(T::wrap(rows.into_iter().map(&reduce).collect()))
            },
        )
    }
}

/// A pack whole, which its worker drafts, and what its emit observes of it.
type Pack = (Arc<EventFeedHistoryRow>, Arc<PackHeader>);

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

/// What seeks the world of an occurrence named before the hour that read it, in `source`.
fn seeking(
    source: Option<&crate::Source>,
    branch: u64,
) -> Option<impl Fn(i32) -> io::Result<replay::container::SeekResult> + '_> {
    source.map(|source| {
        move |hour| {
            dashboard_base::prepared::query::frames::seek(
                source,
                branch,
                hour,
                presentation(),
                &WHOLE_HOUR,
            )
        }
    })
}

/// What a worker makes of a pack: its occurrences' notices and facts, their items presented
/// by `present`, and its own retained-state observation notes, named from `first` on, with the
/// `charges` of its hour. A pack the emit names keeps the items presented in its hour's world.
fn draft(
    pack: &Arc<EventFeedHistoryRow>,
    identities: &HistoryScan,
    first: i32,
    (start, end): (i32, i32),
    charges: impl FnOnce() -> io::Result<BTreeMap<ChargeKey, Vec<super::ScriptCost>>>,
    present: &mut Present<'_>,
) -> io::Result<Facts> {
    let mut builder = Builder::new(start, end);
    builder.captured = Some(Vec::new());
    native::Projection::occurrences(
        &mut builder,
        pack,
        |hour, country| identities.native_tag(hour, country),
        &WHOLE_HOUR,
    )?;
    if pack.events.iter().any(|event| event.hour < first) {
        let decided = pack.events.iter().any(|event| {
            matches!(
                event.source,
                EventFeedSource::DecisionSelected | EventFeedSource::DecisionTimeoutEffect
            )
        });
        let proven = if decided { charges()? } else { BTreeMap::new() };
        // Those named at the read are presented here; the emit presents the rest.
        let mut given = Presented::new();
        for row in &builder.feed.rows {
            let Some(index) = row.item else { continue };
            if row.name_hour != pack.read_hour || !presented(row.kind) {
                continue;
            }
            let mut item = builder
                .feed
                .items
                .get(index as usize)
                .cloned()
                .ok_or_else(|| crate::invalid("Event feed fact item is not the hour's"))?;
            let script = builder.script_inputs.get(&row.id).cloned().map(|mut script| {
                script.resolve(|country| identities.native_scope_slot(row.hour, country));
                script
            });
            let shown = present(row, &mut item, script.as_ref())?;
            given.insert(row.id.clone(), (item, shown));
        }
        return Ok(Facts::Deferred(Arc::clone(pack), proven, given));
    }
    let mut proven = if builder.feed.rows.iter().any(|row| charged(row.kind)) {
        charges()?
    } else {
        BTreeMap::new()
    };
    let drafts = builder.drafts(
        &mut proven,
        |hour, country| identities.native_scope_slot(hour, country),
        present,
    )?;
    Ok(Facts::Drafted(
        builder.captured.take().unwrap_or_default(),
        drafts,
    ))
}

/// The hour's read owners' guarantees the walk had not handed on, with their start reached,
/// and whether any owner's relations were read: of `reads`, the hour's reads of the relations'
/// owners in owner order ([`Projector::relation_reads`]); of `guaranteed`, the keys of the
/// relations of guarantee type ([`Projector::guaranteed`]).
fn guarantees(
    (identity, hour, format): (&DecodedFrame, &segments::Hour<'_, World>, FormatVersion),
    reads: Option<&[SliceReadRow]>,
    guaranteed: Option<&BTreeSet<u128>>,
    tags: &mut LinkTags,
    sent: &mut BTreeSet<(u32, u32, i32)>,
) -> io::Result<(bool, Vec<(u32, u32, i32)>)> {
    let mut relations = false;
    let mut seen = Vec::new();
    if tags.update(identity).is_err() {
        return Ok((relations, seen));
    }
    let (Some(guaranteed), Some(reads)) = (guaranteed, reads) else {
        return Ok((relations, seen));
    };
    let table = TableId::RelationTuples;
    let held = columns::table(hour.state, table)?;
    for read in reads.iter().filter(|read| tags.get(read.owner).is_some()) {
        relations = true;
        let owner = u128::from(read.owner) << 80;
        for &key in guaranteed.range(owner..owner + (1 << 80)) {
            let slot = held
                .slot(key)
                .ok_or_else(|| crate::invalid("A guarantee's key holds no relation slot"))?;
            let row = columns::row::<RelationTupleRow>(hour.state, table, slot, format.version())?;
            if !diplomacy::reads_relation(&row) {
                continue;
            }
            // A relation counts at its first read with its start reached; later reads see it held.
            let relation = (row.first_country, row.second_country, row.start_hour);
            if row.start_hour <= hour.hour && sent.insert(relation) {
                seen.push(relation);
            }
        }
    }
    Ok((relations, seen))
}

/// The hour's strikes as a worker names them from `first` on.
fn strikes(
    frame: HourFrame<'_>,
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
        frame.hour(),
        &strikes,
        |hour, country| identities.native_tag(hour, country),
        &mut builder,
        types,
    );
    let drafts = facts
        .into_iter()
        .map(Draft::new)
        .collect::<io::Result<_>>()?;
    Ok(Nuclear::Drafted(
        builder.captured.take().unwrap_or_default(),
        drafts,
    ))
}

impl Operator<World> for Projector<'_> {
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
        self.sent.clear();
        self.guaranteed = None;
        self.identity = None;
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
        self.guaranteed(hour)?;
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:pack", None);
        let packet = self.read_packet(hour)?;
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:frame", None);
        for &table in OWN {
            self.shapes[table.index()] = Some(columns::status(hour.state, table));
        }
        let opens = hour.opens || hour.starts;
        let measured = metrics(hour.manifest);
        let changed = apply_frame(
            &mut self.identity,
            (hour.branch, hour.hour),
            opens,
            &measured,
            &changes,
            self.format,
        )?;
        let identity = self
            .identity
            .as_ref()
            .ok_or_else(|| crate::invalid("The worker holds no identity frame"))?;
        let views = HourFrame::from(hour.world.columns(hour.state));
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:identities", None);
        advance_identities(
            &mut self.identities,
            (hour.hour, opens),
            &changes,
            identity,
            &changed,
        );
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:draft", None);
        let format = self.format;
        let native = match packet {
            Ok((row, packet)) => {
                let seek = seeking(self.source, hour.branch);
                let texts =
                    HourTexts::new(hour.hour, Some(views), seek.as_ref().map(|seek| seek as _));
                let catalogue = self.catalogue.as_deref();
                let mut present = |row: &EventFeedRow,
                                   item: &mut EventFeedItem,
                                   script: Option<&script_effects::Inputs>| {
                    let _phase = replay_frame::cpu::scope_phase("ef-w:present");
                    catalogue.map_or_else(
                        || Ok(Shown::default()),
                        |catalogue| {
                            super::present(
                                catalogue,
                                row.kind,
                                item,
                                script,
                                Some((&texts, row.name_hour)),
                            )
                        },
                    )
                };
                let drafted = draft(
                    &row,
                    &self.identities,
                    self.first,
                    self.within,
                    || charges_at(hour, format),
                    &mut present,
                )?;
                Native::Packet(packet, drafted)
            }
            Err(reason) => Native::Missing(reason),
        };
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:guarantees", None);
        let (relations, guarantees) = guarantees(
            (identity, hour, self.format),
            reads.as_deref(),
            self.guaranteed.as_ref(),
            &mut self.tags,
            &mut self.sent,
        )?;
        drop(probe);
        let probe = replay_frame::cpu::scope("ef-w:strikes", None);
        let nuclear = strikes(
            views,
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
    /// The content items of packs the emit names are presented with.
    catalogue: Option<Arc<Catalogue>>,
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

    /// [`Self::new`] of a branch whose first and last hours are `bounds`, presenting items
    /// with `catalogue`.
    pub fn within(
        replay: &Replay,
        branch: u64,
        bounds: (i32, i32),
        catalogue: Option<Arc<Catalogue>>,
    ) -> Self {
        let (identities, feed) = (HistoryScan::new(false, &[]), Observer::default());
        let mut prepared = Self::bounded(replay, branch, Some(bounds), identities, feed);
        prepared.catalogue = catalogue;
        prepared
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
            catalogue: None,
        }
    }

    /// One hour, as the row path observes it: the identity history, then the feed; occurrences
    /// named before a walk's first hour are presented in `source`'s worlds.
    pub fn observe_projected(&mut self, hour: Hour, source: Option<&crate::Source>) -> io::Result<()> {
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
            catalogue,
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
        advance_identities(
            identities,
            (hour.hour, hour.opens),
            &hour.changes,
            frame,
            &facts,
        );
        counters.note(&hour.changes)?;
        feed.events.through(hour.hour);
        let mut complete = false;
        let read = match hour.native {
            Native::Missing(reason) => Err(crate::invalid(reason)),
            Native::Packet(pack, facts) => {
                complete =
                    pack.coverage == EventFeedCoverage::Complete && pack.script_refusals == 0;
                let presenting = (catalogue.as_deref(), seeking(source, hour.branch));
                natives(feed, identities, presenting, &pack, facts)
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
        feed.publish(drafts, hour.hour, complete)?;
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
                .map(Draft::new)
                .collect::<io::Result<_>>()?
            }
        };
        for strike in strikes {
            feed.publish_one(strike)?;
        }
        feed.end_hour();
        Ok(())
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        write_identities(writer, branch, &mut self.identities)?;
        self.feed.flush(writer, branch, cancelled)
    }

    pub fn save_state(&mut self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        self.counters.settle(&mut self.feed.diplomacy);
        save_feed(writer, branch, &self.feed)
    }
}

/// The identity history sealed and written whole under the feed's name, also its continuation.
pub(crate) fn write_identities(
    writer: &mut store::Writer,
    branch: u64,
    identities: &mut HistoryScan,
) -> io::Result<()> {
    identities.seal();
    writer.write_at(
        &format!("event-feed-identities-{branch}"),
        0,
        serde_json::to_vec(&*identities)?.as_slice(),
    )
}

/// The feed's continuation, compressed.
pub(crate) fn save_feed(
    writer: &mut store::Writer,
    branch: u64,
    feed: &Observer,
) -> io::Result<()> {
    let encoder = zstd::stream::Encoder::new(Vec::new(), 1)?;
    let mut encoded = io::BufWriter::with_capacity(1 << 20, encoder);
    serde_json::to_writer(&mut encoded, feed)?;
    let bytes = encoded
        .into_inner()
        .map_err(io::IntoInnerError::into_error)?
        .finish()?;
    writer.write_at(
        &crate::prepared::driver::registry::continuation_section("event_feed", branch),
        0,
        bytes.as_slice(),
    )
}

/// A pack observed in order, as the row path observes it: its notices, then its occurrences'
/// facts, a worker's as it drafted them.
fn natives(
    feed: &mut Observer,
    identities: &HistoryScan,
    (catalogue, seek): (Option<&Catalogue>, Option<impl Fn(i32) -> io::Result<replay::container::SeekResult>>),
    pack: &PackHeader,
    facts: Facts,
) -> io::Result<Vec<Draft>> {
    let _cpu = replay_frame::cpu::scope("event-feed-reduce", None);
    let probe = replay_frame::cpu::scope("ef:follows-notices", None);
    feed.native.follows(pack)?;
    feed.native.notices(&mut feed.diagnostics, pack);
    drop(probe);
    let probe = replay_frame::cpu::scope("ef:drafts", None);
    let drafts = match facts {
        Facts::Drafted(notices, drafts) => {
            for notice in notices {
                notice.apply(&mut feed.diagnostics);
            }
            drafts
        }
        Facts::Deferred(row, mut proven, mut given) => {
            let mut tag_at = |hour, country| identities.native_tag(hour, country);
            native::Projection::occurrences(&mut feed.diagnostics, &row, &mut tag_at, &WHOLE_HOUR)?;
            // The worker presented those named at the read in its world; the rest are sought.
            // The emit holds no world of its own: every text is sought.
            let texts = HourTexts::new(i32::MIN, None, seek.as_ref().map(|seek| seek as _));
            let mut present = |row: &EventFeedRow,
                               item: &mut EventFeedItem,
                               script: Option<&script_effects::Inputs>| {
                if let Some((presented, shown)) = given.remove(&row.id) {
                    *item = presented;
                    return Ok(shown);
                }
                catalogue.map_or_else(
                    || Ok(Shown::default()),
                    |catalogue| {
                        super::present(
                            catalogue,
                            row.kind,
                            item,
                            script,
                            Some((&texts, row.name_hour)),
                        )
                    },
                )
            };
            feed.diagnostics.drafts(
                &mut proven,
                |hour, country| identities.native_scope_slot(hour, country),
                &mut present,
            )?
        }
    };
    drop(probe);
    feed.native.settle(pack);
    Ok(drafts)
}

#[cfg(test)]
mod tests;
