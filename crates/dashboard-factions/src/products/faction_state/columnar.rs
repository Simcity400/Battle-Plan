//! Faction states prepared from a columnar walk. Each segment's projection starts cold, as a
//! walk's first hour reads every input anew, and then follows only what each hour changed: the
//! walker's change facts name the rows; an hour none of whose changes the projection reads is
//! projected without a frame, a held dynamic text checked against those facts alone; otherwise
//! its tables come from the walk's shared frame ([`dashboard_world`]), Countries by its tags and
//! slots alone. A dynamic text read again or evaluated reads the trigger tables, which the
//! projection keeps itself, each built at its first ask and patched at later ones by the rows
//! the hours since changed. Programme operands come from the walk's own supplying reads, or the
//! recording's for a read before the segment. The ordered emit ([`super::emit`]) numbers and
//! publishes.
use super::prepared::{ACCOUNT_FIELDS, CHANNELS, Edits, Frames, Projected, Runtime, Supplying};
use dashboard_base::{
    Source, catalogue,
    metadata::{slot_read, slot_reads},
    prepared::frame_changes::{FrameChanges, field_mask},
    products::faction_details::selectors,
    triggers::inputs::{self as reuse, TableChange, TableChanges},
};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::{Views, metrics},
    columns,
    segments::{self, Item, Manifest, Operator, Sink, Start, State},
};
use replay_data::{
    cadence::STAGGER_PERIOD,
    format::FormatVersion,
    rows::{DeltaRow, TABLE_COUNT, TableId, full_mask},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, NOT_SELECTED, SharedView, TableView, not_selected};
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};

/// The family's own tables.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ReservedCountryNaming,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::FactionAccounts,
    TableId::FactionProjection,
    TableId::FactionUnlocks,
    TableId::FactionGoalStatus,
    TableId::FactionGoalEntries,
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionManpower,
    TableId::LexicalTokens,
    TableId::ProjectPrograms,
    TableId::Projects,
    // A dynamic rule or doctrine name is expanded for the viewing country.
    TableId::LocalPlayer,
];

/// Every table a walk decodes for the family: its own and the trigger kit's, which a dynamic
/// name can read beyond them.
pub fn tables() -> &'static [TableId] {
    static WHOLE: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = TABLES.to_vec();
        tables.extend_from_slice(dashboard_base::triggers::RECORDED_TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &WHOLE
}

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
/// naming. The trigger tables a dynamic text reads are the projection's own.
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
    let mut own: Vec<TableId> = TABLES
        .iter()
        .copied()
        .filter(|table| !NAMING.contains(table) && !PROGRAMMES.contains(table))
        .collect();
    for table in [TableId::WarRelations, TableId::Subjects] {
        if tables().contains(&table) && !own.contains(&table) {
            own.push(table);
        }
    }
    own
}

/// The tables whose changes the hour's evaluation reads as rows: the own frame's, the naming's
/// and the programmes'.
fn watched() -> &'static [TableId] {
    static WATCHED: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut watched = own_tables();
        watched.extend(NAMING);
        watched.extend(PROGRAMMES);
        watched.sort_unstable();
        watched.dedup();
        watched
    });
    &WATCHED
}

/// The tables a held text's reuse dates by: the links whose countries own its rows, and the
/// relations whose spans end.
const SPANS: [(TableId, Hold); 3] = [
    (TableId::CountryLinks, Hold::Whole),
    (TableId::WarRelations, Hold::Whole),
    (TableId::Subjects, Hold::Whole),
];

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
        // Read only by a held dynamic text, which each hour checks.
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

impl Edit {
    fn change(&self) -> TableChange<'_> {
        match self {
            Edit::Same => TableChange::Same,
            Edit::Any => TableChange::Any,
            Edit::Rows(rows, deleted) => TableChange::Rows(rows, deleted),
        }
    }
}

/// What the hour changed in each table the walk reads, each found when first asked.
struct Moved<'h> {
    state: &'h State,
    world: &'h World,
    format: FormatVersion,
    /// A walk's start: every table may have changed anywhere.
    restart: bool,
    /// The fields of each table a held text reads: a change to none of them is told by the
    /// fields the hour wrote, without its rows.
    read: &'h [u64; TABLE_COUNT],
    edits: [OnceCell<Edit>; TABLE_COUNT],
}

impl<'h> Moved<'h> {
    fn new(
        (state, world): (&'h State, &'h World),
        format: FormatVersion,
        restart: bool,
        read: &'h [u64; TABLE_COUNT],
    ) -> Self {
        Self {
            state,
            world,
            format,
            restart,
            read,
            edits: std::array::from_fn(|_| OnceCell::new()),
        }
    }

    /// Whether `table`'s status may differ from the hour before's.
    fn whole(&self, table: TableId) -> bool {
        self.restart || self.world.restated(table)
    }

    fn edit(&self, id: TableId) -> Edit {
        let (Ok(changes), Ok(held)) = (
            columns::changes(self.state, id),
            columns::table(self.state, id),
        ) else {
            return Edit::Any;
        };
        // A truncated table is read anew each hour, as the row decoder's view of it is.
        if self.whole(id)
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

    /// `table`'s change told by the fields the hour wrote, when its rows would tell a held
    /// text no more: no row came or went, and no held text reads a field written.
    fn by_fields(&self, table: TableId) -> Option<TableChange<'static>> {
        if self.whole(table) {
            return Some(TableChange::Any);
        }
        let changes = columns::changes(self.state, table).ok()?;
        if changes.truncated
            || !changes.inserted.is_empty()
            || !changes.deleted.is_empty()
            || matches!(
                columns::status(self.state, table),
                columns::Status::Truncated { .. }
            )
        {
            return None;
        }
        let fields = table.last_change().fields(table, self.format.version());
        // A row of an older format reads its unrecorded fields anew.
        let older = fields < table.schema().len();
        let every = full_mask(fields);
        let mut written = 0;
        for &mask in changes.masks.iter().chain(&changes.fill_masks) {
            // A row given every field reads as one inserted.
            if older || mask == every {
                return None;
            }
            written |= mask;
        }
        if written == 0 {
            Some(TableChange::Same)
        } else if written & self.read[table.index()] == 0 {
            Some(TableChange::Fields(written))
        } else {
            None
        }
    }
}

impl TableChanges for Moved<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        let index = table.index();
        // A table the walk does not read is the hour before's: every frame holds it unselected.
        if self.state.skips(index) || self.state.status(index).is_none() {
            return TableChange::Same;
        }
        if let Some(edit) = self.edits[index].get() {
            return edit.change();
        }
        match self.by_fields(table) {
            Some(change) => change,
            None => self.get(table).change(),
        }
    }
}

/// The hour's changes as the projection reads them.
struct Changes<'h> {
    moved: Moved<'h>,
    flags: FrameChanges,
}

impl Edits for Changes<'_> {
    fn changes(&self) -> &FrameChanges {
        &self.flags
    }
    fn rows(&self, table: TableId) -> Option<(&[DeltaRow], &[u128])> {
        match self.moved.get(table) {
            Edit::Same => Some((&[], &[])),
            Edit::Any => None,
            Edit::Rows(rows, deleted) => Some((rows, deleted)),
        }
    }
    fn tables(&self) -> &dyn TableChanges {
        &self.moved
    }
    fn restated(&self, table: TableId) -> bool {
        self.moved.whole(table)
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

/// The tables a dynamic text reads beyond the hour's own frame, and Countries whole: one view
/// of each for the walk, built at its first ask, then noted every hour by slot and patched at
/// each later ask by the rows changed since, so an ask costs what changed, not what is held.
struct Trigger {
    /// Every table of [`tables`], Countries among them.
    views: Views,
    /// Whether each table's view is built, and so noted, since the walk's start.
    held: [bool; TABLE_COUNT],
    /// Some table is held.
    holds: bool,
    /// The tables an ask builds, kept from ask to ask.
    wanted: Vec<TableId>,
    /// One view apart for each table a frame leaves unselected, so a text reading one is not
    /// taken to read them all.
    holes: Vec<SharedView>,
    /// Frames built for dynamic texts, and tables built anew for them.
    #[cfg(test)]
    built: usize,
    #[cfg(test)]
    anew: u32,
}

impl Trigger {
    fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut views = Views::new(tables(), format, mode)?.by_slot();
        #[cfg(test)]
        views.time_builds(true);
        Ok(Self {
            views,
            held: [false; TABLE_COUNT],
            holds: false,
            wanted: Vec::new(),
            holes: Vec::new(),
            #[cfg(test)]
            built: 0,
            #[cfg(test)]
            anew: 0,
        })
    }

    /// Notes the hour's edits of the held tables: a walk's start holds none, building each
    /// anew at its next ask.
    fn hour(&mut self, state: &State, manifest: &Manifest<'_>, restart: bool) -> io::Result<()> {
        if restart {
            self.held = [false; TABLE_COUNT];
            self.holds = false;
        } else if !self.holds {
            return Ok(());
        }
        let held = &self.held;
        self.views
            .note_tables(state, manifest, restart, &|table| held[table.index()])
    }

    /// `own` with each of `tables` the walk reads and Countries whole, every other table
    /// unselected.
    fn frame(
        &mut self,
        state: &State,
        own: &DecodedFrame,
        tables: &[TableId],
    ) -> io::Result<DecodedFrame> {
        #[cfg(test)]
        {
            self.built += 1;
        }
        let mut frame = DecodedFrame {
            branch: own.branch,
            hour: own.hour,
            tables: std::array::from_fn(|index| own.tables[index].clone()),
            metrics: own.metrics.clone(),
        };
        self.wanted.clear();
        for &table in tables.iter().chain(&[TableId::Countries]) {
            let index = table.index();
            let walked = !state.skips(index) && state.status(index).is_some();
            if walked
                && (table == TableId::Countries || frame.tables[index].is_not_selected())
                && self.views.selected().binary_search(&table).is_ok()
            {
                self.wanted.push(table);
            }
        }
        self.wanted.sort_unstable();
        self.wanted.dedup();
        self.views
            .update_tables(state, own.hour, &self.wanted, None)?;
        #[cfg(test)]
        {
            self.anew += self.views.times().anew.0;
        }
        for &table in &self.wanted {
            let index = table.index();
            self.held[index] = true;
            frame.tables[index] = self
                .views
                .view(table)
                .ok_or_else(|| io::Error::other("A trigger table was not built"))?
                .clone()
                .with_sampled_at(state.sampled_at(index));
        }
        self.holds |= !self.wanted.is_empty();
        if self.holes.is_empty() {
            self.holes = (0..TABLE_COUNT)
                .map(|_| {
                    SharedView::from(TableView::Unavailable {
                        reason: NOT_SELECTED.into(),
                    })
                })
                .collect();
        }
        for (view, hole) in frame.tables.iter_mut().zip(&self.holes) {
            if view.is_not_selected() {
                *view = hole.clone();
            }
        }
        Ok(frame)
    }
}

/// The hour's own frame, and the frames a programme or dynamic text asks for, built when first
/// asked.
struct Hourly<'p, 'h, 'a> {
    own: &'p DecodedFrame,
    hour: &'p segments::Hour<'h, World>,
    /// [`PROGRAMME_FRAME`] as the walk's frame is asked for it.
    asks: &'p Asks,
    trigger: &'p RefCell<Trigger>,
    read: Read,
    source: &'a Source,
    /// The recording's supplying reads, for a read before the walk or one it passed.
    lookback: RefCell<Option<Supplying<'a>>>,
    programmes: OnceCell<Result<DecodedFrame, String>>,
    reading: OnceCell<Result<DecodedFrame, String>>,
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

    /// The own frame with `tables` built for a dynamic text, held in `cell`.
    fn evaluation<'c>(
        &self,
        cell: &'c OnceCell<Result<DecodedFrame, String>>,
        tables: &[TableId],
    ) -> io::Result<&'c DecodedFrame> {
        cell.get_or_init(|| {
            let _cpu = ::replay_frame::cpu::scope("faction trigger frame", None);
            self.trigger
                .borrow_mut()
                .frame(self.hour.state, self.own, tables)
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
        self.evaluation(&self.whole, tables())
    }
    fn reading(&self, tables: &[TableId]) -> io::Result<&DecodedFrame> {
        // The whole frame holds every table a reading asks for.
        if let Some(Ok(whole)) = self.whole.get() {
            return Ok(whole);
        }
        self.evaluation(&self.reading, tables)
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
}

/// A segment's projection. It starts cold at every start, as a walk reads its first
/// hour anew; what earlier segments published is the emit's.
pub struct Projector<'a> {
    format: FormatVersion,
    source: &'a Source,
    cat: Option<Arc<catalogue::Catalogue>>,
    /// The tables whose changes decide whether an hour is evaluated.
    own_tables: Vec<TableId>,
    asks: Asks,
    trigger: RefCell<Trigger>,
    /// The fields of each table the held dynamic texts read.
    read_fields: Box<[u64; TABLE_COUNT]>,
    /// A frame kept from hour to hour, holding none of the walk's views between them.
    kept: Option<DecodedFrame>,
    /// The walk named the countries since its start.
    named: bool,
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
    /// A projection of a recording at `format` captured in `mode`, naming by `cat`, the
    /// recorded stack's catalogue, and reading programme operands it did not walk to from
    /// `source`.
    pub fn new(
        (format, mode): (FormatVersion, CaptureMode),
        source: &'a Source,
        cat: Option<Arc<catalogue::Catalogue>>,
    ) -> io::Result<Self> {
        let own = own_tables();
        Ok(Self {
            format,
            source,
            cat,
            named: false,
            asks: Asks {
                own: held(own.iter().copied().chain(NAMING))?,
                programmes: held(PROGRAMME_FRAME)?,
            },
            own_tables: own,
            trigger: RefCell::new(Trigger::new(format, mode)?),
            read_fields: Box::new([0; TABLE_COUNT]),
            kept: None,
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

#[cfg(test)]
impl Projector<'_> {
    /// Frames built for dynamic texts since the projection started.
    pub(super) fn trigger_frames(&self) -> usize {
        self.trigger.borrow().built
    }

    /// Trigger tables built anew since the projection started, rather than patched or kept.
    pub(super) fn trigger_tables_anew(&self) -> u32 {
        self.trigger.borrow().anew
    }
}

/// The kept frame, empty of the walk's views.
fn taken(kept: &mut Option<DecodedFrame>) -> DecodedFrame {
    kept.take().unwrap_or_else(|| DecodedFrame {
        branch: 0,
        hour: 0,
        tables: std::array::from_fn(|_| not_selected()),
        metrics: Default::default(),
    })
}

/// Keeps `frame` for the next hour, giving back the walk's views `asks` gave it.
fn keep(kept: &mut Option<DecodedFrame>, mut frame: DecodedFrame, asks: &[(TableId, Hold)]) {
    dashboard_world::release(&mut frame, asks);
    *kept = Some(frame);
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
        let (state, world) = (hour.state, hour.world);
        // A walk's start reads every input anew.
        let restart =
            hour.opens || self.last.and_then(|last| last.checked_add(1)) != Some(hour.hour);
        if restart {
            self.runtime = Runtime::exact();
            self.named = false;
        }
        self.trigger
            .get_mut()
            .hour(state, hour.manifest, restart)?;
        self.runtime.gather_reads(&mut self.read_fields);
        let whole = |table: TableId| restart || world.restated(table);
        let mut changed = [false; TABLE_COUNT];
        for &table in watched() {
            changed[table.index()] = whole(table)
                || matches!(
                    columns::status(state, table),
                    columns::Status::Truncated { .. }
                )
                || columns::changes(state, table).is_ok_and(|edits| {
                    !edits.changed.is_empty()
                        || !edits.fill_slots.is_empty()
                        || !edits.inserted.is_empty()
                        || !edits.deleted.is_empty()
                        || edits.truncated
                });
        }
        let metrics = metrics(hour.manifest);
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
            || whole(TableId::Countries)
            || changed[TableId::CountryLinks.index()]
            || world.touched(TableId::Countries, self.country_mask);
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
            evaluated = reads_change(state, table, whole(table))?;
        }
        if !evaluated && self.runtime.holds_dynamic() {
            // Each held text is checked by the hour's change facts alone.
            let mut frame = taken(&mut self.kept);
            world.frame_held_into(state, &SPANS, &mut frame)?;
            let moved = Moved::new((state, world), self.format, restart, &self.read_fields);
            let since = reuse::Since::hour_before(&frame, &moved);
            let fresh = self.runtime.check_held(since.as_ref(), hour.hour);
            drop(since);
            keep(&mut self.kept, frame, &SPANS);
            evaluated = !fresh;
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
        let mut frame = taken(&mut self.kept);
        world.frame_held_into(state, &self.asks.own, &mut frame)?;
        let edits = Changes {
            moved: Moved::new((state, world), self.format, restart, &self.read_fields),
            flags: FrameChanges::rebuilt(&frame, self.format, !restart, |table| {
                changed[table.index()]
            }),
        };
        self.runtime.kept_programmes = self.moved.as_ref().map(|(moved, _)| moved.clone());
        let frames = Hourly {
            own: &frame,
            hour,
            asks: &self.asks,
            trigger: &self.trigger,
            read: self.read,
            source: self.source,
            lookback: RefCell::new(None),
            programmes: OnceCell::new(),
            reading: OnceCell::new(),
            whole: OnceCell::new(),
        };
        let projected =
            self.runtime
                .project(&frames, &edits, self.cat.as_deref(), &self.cancelled)?;
        drop((frames, edits));
        keep(&mut self.kept, frame, &self.asks.own);
        self.last = Some(hour.hour);
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            projected,
        })
    }
}
