//! Faction states prepared from a columnar walk. Each segment's projection starts cold, as a
//! row block's first hour reads every input anew, and then follows only what each hour
//! changed: the walker's change facts name the rows; an hour none of whose changes the
//! projection reads is projected without a frame; otherwise its tables come from the walk's
//! shared frame ([`dashboard_world`]), Countries by its tags and slots alone, and the trigger
//! tables no lane declares are asked for only at an hour a dynamic text or scripted programme
//! name reads anew. Programme operands come from the walk's own supplying reads, or the
//! recording's for a read before the segment. The ordered emit, the block fold's own, numbers
//! and publishes.
use super::{
    block,
    prepared::{ACCOUNT_FIELDS, CHANNELS, Edits, Frames, Projected, Runtime, Supplying},
};
use dashboard_base::{
    Source, catalogue,
    metadata::{slot_read, slot_reads},
    prepared::frame_changes::{FrameChanges, field_mask},
    products::{LoadContext, faction_details::selectors},
    triggers::inputs::{TableChange, TableChanges},
};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink, Start, State},
};
use replay_data::{
    cadence::STAGGER_PERIOD,
    format::FormatVersion,
    rows::{DeltaRow, TABLE_COUNT, TableId, full_mask},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, not_selected};
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// The programme tables with the naming and the dictionary they name by.
const PROGRAMME_FRAME: [TableId; 7] = [
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ReservedCountryNaming,
    TableId::LexicalTokens,
];

/// `tables` as a projection asks the walk's frame for them: Countries by its tags and slots
/// alone, as country naming reads it.
fn held(tables: impl IntoIterator<Item = TableId>) -> io::Result<Vec<(TableId, Hold)>> {
    let identity = dashboard_world::identity()?;
    Ok(tables
        .into_iter()
        .map(|table| {
            identity
                .iter()
                .copied()
                .find(|&(named, _)| named == table)
                .unwrap_or((table, Hold::Whole))
        })
        .collect())
}

/// What a projection reads of its walk's world: its own tables, the programmes and the
/// naming. The other trigger tables are asked for at the hours a text reads them.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: held(own_tables().into_iter().chain(PROGRAMMES).chain(NAMING))?,
        ..Needs::default()
    })
}

/// One projected hour, for the ordered emit.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(super) projected: Projected,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.projected.heap()
    }
}

/// The tables country naming is read from: rebuilt whole, only when tags or links change.
const NAMING: [TableId; 2] = [TableId::Countries, TableId::CountryLinks];

/// The programme tables a supplying read holds beside the naming and the dictionary.
const PROGRAMMES: [TableId; 2] = [TableId::ProjectPrograms, TableId::Projects];

/// The tables an hour's own frame holds: the family's but naming and programmes, and the
/// relations a held dynamic text's reuse dates its spans by.
fn own_tables() -> Vec<TableId> {
    let mut own: Vec<TableId> = block::TABLES
        .iter()
        .copied()
        .filter(|table| !NAMING.contains(table) && !PROGRAMMES.contains(table))
        .collect();
    for table in [TableId::WarRelations, TableId::Subjects] {
        if block::whole_tables().contains(&table) && !own.contains(&table) {
            own.push(table);
        }
    }
    own
}

/// Whether the hour's changes of `table`, one of the own frame's, may change what the
/// projection reads: `whole` when its status changed or the walk started.
fn reads_change(state: &State, table: TableId, whole: bool) -> io::Result<bool> {
    if whole {
        return Ok(true);
    }
    let changes = columns::changes(state, table)?;
    let rows = !changes.inserted.is_empty() || !changes.deleted.is_empty() || changes.truncated;
    let written = || {
        changes
            .masks
            .iter()
            .chain(&changes.fill_masks)
            .fold(0_u64, |fields, mask| fields | mask)
    };
    Ok(match table {
        TableId::Campaign => false,
        // Only their rows name owners; their values are read nowhere.
        TableId::FactionProjection | TableId::FactionUnlocks => rows,
        TableId::FactionAccounts => rows || written() & *ACCOUNT_FIELDS != 0,
        // Read only by a held dynamic text, which projects every hour.
        TableId::WarRelations | TableId::Subjects => false,
        _ => rows || written() != 0,
    })
}

/// `moved` with the factions whose programme rows the hour changed.
fn programme_owners(state: &State, mut moved: BTreeSet<u64>) -> io::Result<BTreeSet<u64>> {
    let table = TableId::FactionPrograms;
    let (changes, held) = (
        columns::changes(state, table)?,
        columns::table(state, table)?,
    );
    for &slot in changes
        .changed
        .iter()
        .chain(&changes.fill_slots)
        .chain(&changes.inserted)
    {
        if let Some(key) = held.key(slot) {
            moved.insert((key >> 32) as u64);
        }
    }
    moved.extend(changes.deleted_keys.iter().map(|key| (key >> 32) as u64));
    Ok(moved)
}

enum Edit {
    Same,
    Any,
    Rows(Vec<DeltaRow>, Vec<u128>),
}

/// What the hour changed in each table the walk reads, each found when first asked.
struct Changes<'h> {
    state: &'h State,
    format: FormatVersion,
    /// The tables the hour may have changed anywhere: a walk's start, or a changed status.
    whole: [bool; TABLE_COUNT],
    edits: [OnceCell<Edit>; TABLE_COUNT],
    flags: FrameChanges,
}

impl Changes<'_> {
    fn edit(&self, id: TableId) -> Edit {
        let (Ok(changes), Ok(held)) = (
            columns::changes(self.state, id),
            columns::table(self.state, id),
        ) else {
            return Edit::Any;
        };
        // A truncated table is read anew each hour, as the row decoder's view of it is.
        if self.whole[id.index()]
            || changes.truncated
            || matches!(
                columns::status(self.state, id),
                columns::Status::Truncated { .. }
            )
        {
            return Edit::Any;
        }
        let fields = id.last_change().fields(id, self.format.version());
        let mut rows = BTreeMap::<u128, u64>::new();
        for (&slot, &mask) in changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        {
            if let Some(key) = held.key(slot) {
                *rows.entry(key).or_default() |= mask;
            }
        }
        for &slot in &changes.inserted {
            if let Some(key) = held.key(slot) {
                rows.insert(key, full_mask(fields));
            }
        }
        let mut deleted = changes.deleted_keys.clone();
        deleted.sort_unstable();
        // A key deleted and inserted again is changed, not deleted.
        deleted.retain(|key| !rows.contains_key(key));
        if rows.is_empty() && deleted.is_empty() {
            return Edit::Same;
        }
        Edit::Rows(
            rows.into_iter()
                .map(|(key, mask)| DeltaRow {
                    format: self.format.version(),
                    key,
                    mask,
                    recorded_fields: fields,
                    values: Vec::new(),
                })
                .collect(),
            deleted,
        )
    }

    fn get(&self, table: TableId) -> &Edit {
        self.edits[table.index()].get_or_init(|| self.edit(table))
    }
}

impl TableChanges for Changes<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        match self.get(table) {
            Edit::Same => TableChange::Same,
            Edit::Any => TableChange::Any,
            Edit::Rows(rows, deleted) => TableChange::Rows(rows, deleted),
        }
    }
}

impl Edits for Changes<'_> {
    fn changes(&self) -> &FrameChanges {
        &self.flags
    }
    fn rows(&self, table: TableId) -> Option<(&[DeltaRow], &[u128])> {
        match self.get(table) {
            Edit::Same => Some((&[], &[])),
            Edit::Any => None,
            Edit::Rows(rows, deleted) => Some((rows, deleted)),
        }
    }
    fn tables(&self) -> &dyn TableChanges {
        self
    }
}

/// The programmes' supplying read as the walk finds it: the latest hour that read them, with
/// their table complete at every hour since, as [`Supplying`] finds it in the recording.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Read {
    /// The walk restarted after a read it has not seen: the recording finds it.
    Unseen,
    At(i32),
    Lost,
}

/// The hour's own frame, and the frames a programme or dynamic text asks for, taken from the
/// walk's frame when first asked.
struct Hourly<'p, 'h, 'a> {
    own: &'p DecodedFrame,
    hour: &'p segments::Hour<'h, World>,
    /// [`PROGRAMME_FRAME`], and every table the projection or a dynamic text reads, as the
    /// walk's frame is asked for them.
    asks: &'p Asks,
    read: Read,
    source: &'a Source,
    /// The recording's supplying reads, for a read before the walk or one it passed.
    lookback: RefCell<Option<Supplying<'a>>>,
    programmes: OnceCell<Result<DecodedFrame, String>>,
    whole: OnceCell<Result<DecodedFrame, String>>,
}

impl<'a> Hourly<'_, '_, 'a> {
    /// The programme tables at this hour, with the naming and the dictionary they name by.
    fn programmes(&self) -> io::Result<&DecodedFrame> {
        self.programmes
            .get_or_init(|| {
                self.hour
                    .world
                    .frame_held(self.hour.state, &self.asks.programmes)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|reason| io::Error::other(reason.clone()))
    }

    fn lookback<T>(&self, read: impl FnOnce(&mut Supplying<'a>) -> io::Result<T>) -> io::Result<T> {
        let mut lookback = self.lookback.borrow_mut();
        let (source, branch) = (self.source, self.own.branch);
        read(lookback.get_or_insert_with(|| Supplying::new(source, branch)))
    }
}

impl Frames for Hourly<'_, '_, '_> {
    fn own(&self) -> &DecodedFrame {
        self.own
    }
    fn whole(&self) -> io::Result<&DecodedFrame> {
        self.whole
            .get_or_init(|| {
                let _cpu = ::replay_frame::cpu::scope("faction trigger frame", None);
                self.hour
                    .world
                    .frame_held(self.hour.state, &self.asks.followed)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|reason| io::Error::other(reason.clone()))
    }
    fn read_hour(&self, cancelled: &AtomicBool) -> io::Result<Option<i32>> {
        match self.read {
            Read::At(at) => Ok((self.own.hour - at < STAGGER_PERIOD).then_some(at)),
            Read::Lost => Ok(None),
            Read::Unseen => self.lookback(|walks| walks.read_hour(self.own, cancelled)),
        }
    }
    fn supplying(
        &self,
        hour: i32,
        wide: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Arc<DecodedFrame>>> {
        // The walk holds the read's own hour; an earlier one is the recording's.
        if self.read != Read::At(hour) || hour != self.own.hour {
            return self.lookback(|walks| walks.frame(hour, wide, cancelled));
        }
        let frame = if wide {
            let whole = self.whole()?;
            let mut frame = DecodedFrame {
                branch: whole.branch,
                hour: whole.hour,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: whole.metrics.clone(),
            };
            for table in selectors::programme_text_tables() {
                frame.tables[table.index()] = whole.tables[table.index()].clone();
            }
            frame
        } else {
            self.programmes()?.clone()
        };
        Ok(Some(Arc::new(frame)))
    }
}

/// How a projection asks the walk's frame for its tables.
struct Asks {
    own: Vec<(TableId, Hold)>,
    programmes: Vec<(TableId, Hold)>,
    followed: Vec<(TableId, Hold)>,
}

/// A segment's projection. It starts cold at every start, as each row block reads its first
/// hour anew; what earlier segments published is the emit's.
pub struct Projector<'a> {
    format: FormatVersion,
    source: &'a Source,
    cat: Option<Arc<catalogue::Catalogue>>,
    /// Every table the projection or a dynamic text reads.
    followed: Vec<TableId>,
    /// The tables whose changes decide whether an hour is evaluated.
    own_tables: Vec<TableId>,
    asks: Asks,
    /// The walk named the countries since its start.
    named: bool,
    shapes: [Option<columns::Status>; TABLE_COUNT],
    country_mask: u64,
    runtime: Runtime,
    read: Read,
    /// While the read before a restart supplies the programmes: the factions whose programme
    /// rows moved since the restart, and the programme table's status there; None once any
    /// may have moved.
    moved: Option<(BTreeSet<u64>, columns::Status)>,
    last: Option<i32>,
    cancelled: AtomicBool,
    /// Hours projected from a frame rather than as idle.
    #[cfg(test)]
    pub(super) evaluated: usize,
}

impl<'a> Projector<'a> {
    /// A projection reading the recording's content as the row path does, and programme
    /// operands it did not walk to from `source`.
    pub fn new(
        format: FormatVersion,
        _: CaptureMode,
        source: &'a Source,
        context: &LoadContext,
    ) -> io::Result<Self> {
        let cat = context
            .content
            .as_ref()
            .and_then(|options| catalogue::catalogue(source, options).ok());
        let own = own_tables();
        let followed = block::whole_tables().to_vec();
        Ok(Self {
            format,
            source,
            cat,
            named: false,
            asks: Asks {
                own: held(own.iter().copied().chain(NAMING))?,
                programmes: held(PROGRAMME_FRAME)?,
                followed: held(followed.iter().copied())?,
            },
            own_tables: own,
            followed,
            shapes: [None; TABLE_COUNT],
            country_mask: field_mask(TableId::Countries, &["canonical_slot", "tag"]),
            runtime: Runtime::exact(),
            read: Read::Unseen,
            moved: None,
            last: None,
            cancelled: AtomicBool::new(false),
            #[cfg(test)]
            evaluated: 0,
        })
    }
}

impl Operator<World> for Projector<'_> {
    type Output = Hour;

    fn start(&mut self, start: Start<'_>, _: u64, _: i32) -> io::Result<()> {
        // A run's start follows no read the walk could not see.
        (self.read, self.moved) = match start {
            Start::Restart(state) => (
                Read::Unseen,
                Some((
                    BTreeSet::new(),
                    columns::status(state, TableId::FactionPrograms),
                )),
            ),
            Start::Run { .. } => (Read::Lost, None),
        };
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("faction projection", None);
        let state = hour.state;
        // A walk's start reads every input anew.
        let restart =
            hour.opens || self.last.and_then(|last| last.checked_add(1)) != Some(hour.hour);
        if restart {
            self.runtime = Runtime::exact();
            self.named = false;
        }
        let metrics = metrics(hour.manifest);
        let mut whole = [false; TABLE_COUNT];
        let mut changed = [false; TABLE_COUNT];
        for &table in &self.followed {
            let index = table.index();
            let now = columns::status(state, table);
            whole[index] = restart || self.shapes[index] != Some(now);
            self.shapes[index] = Some(now);
            changed[index] = whole[index]
                || matches!(now, columns::Status::Truncated { .. })
                || columns::changes(state, table).is_ok_and(|edits| {
                    !edits.changed.is_empty()
                        || !edits.fill_slots.is_empty()
                        || !edits.inserted.is_empty()
                        || !edits.deleted.is_empty()
                        || edits.truncated
                });
        }
        self.read =
            if !columns::complete(state, TableId::ProjectPrograms) || (restart && !hour.opens) {
                Read::Lost
            } else if slot_read(self.format, &metrics, TableId::ProjectPrograms, hour.hour) {
                Read::At(hour.hour)
            } else {
                self.read
            };
        self.moved = match self.moved.take() {
            Some((moved, status))
                if self.read == Read::Unseen
                    && columns::status(state, TableId::FactionPrograms) == status =>
            {
                Some((programme_owners(state, moved)?, status))
            }
            _ => None,
        };
        let slots = slot_reads(self.format, &metrics, hour.hour);
        let renamed = !self.named
            || whole[TableId::Countries.index()]
            || changed[TableId::CountryLinks.index()]
            || hour.world.touched(TableId::Countries, self.country_mask);
        // Programme operands move only for factions that hold programmes.
        let programmes = columns::complete(state, TableId::FactionPrograms)
            && columns::table(state, TableId::FactionPrograms)?
                .live()
                .count()
                > 0
            && (slots[TableId::ProjectPrograms.index()]
                || PROGRAMMES.iter().any(|table| changed[table.index()]));
        let mut evaluated = renamed || programmes || !self.runtime.may_idle(hour.hour);
        for &table in &self.own_tables {
            if evaluated {
                break;
            }
            evaluated = reads_change(state, table, whole[table.index()])?;
        }
        if !evaluated {
            let reads = CHANNELS.map(|(_, table)| slots[table.index()]);
            self.last = Some(hour.hour);
            return out.push(Hour {
                branch: hour.branch,
                hour: hour.hour,
                projected: self.runtime.idle(hour.hour, reads),
            });
        }
        #[cfg(test)]
        {
            self.evaluated += 1;
        }
        self.named = true;
        // The projection reads countries by their naming alone.
        changed[TableId::Countries.index()] = renamed;
        changed[TableId::CountryLinks.index()] = renamed;
        let frame = hour.world.frame_held(state, &self.asks.own)?;
        let edits = Changes {
            state,
            format: self.format,
            whole,
            edits: std::array::from_fn(|_| OnceCell::new()),
            flags: FrameChanges::rebuilt(&frame, self.format, !restart, |table| {
                changed[table.index()]
            }),
        };
        self.runtime.kept_programmes = self.moved.as_ref().map(|(moved, _)| moved.clone());
        let frames = Hourly {
            own: &frame,
            hour,
            asks: &self.asks,
            read: self.read,
            source: self.source,
            lookback: RefCell::new(None),
            programmes: OnceCell::new(),
            whole: OnceCell::new(),
        };
        let projected =
            self.runtime
                .project(&frames, &edits, self.cat.as_deref(), &self.cancelled)?;
        drop(frames);
        self.last = Some(hour.hour);
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            projected,
        })
    }
}
