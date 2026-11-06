//! Faction states prepared from a columnar walk. Each segment's projection starts cold, as a
//! walk's first hour reads every input anew, and then follows only what each hour changed: the
//! walker's change facts name the rows; an hour none of whose changes the projection reads is
//! projected without a frame, a held dynamic text checked against those facts alone; otherwise
//! it reads the walk's views, which hold every table the family and its dynamic texts read,
//! and tells a table unchanged since an earlier hour by its epoch ([`Epochs`]). Programme
//! operands come from the walk's own supplying read: at its hour, its views; after it, the
//! tables kept from them there. A read the walk never held, one before its restart, is read
//! from the native capture by a walk of its own ([`Earlier`]). The ordered emit
//! ([`super::emit`]) numbers and publishes.
use super::prepared::{ACCOUNT_FIELDS, CHANNELS, Edits, Frames, Projected, Runtime, Supply};
use dashboard_base::{
    catalogue,
    metadata::{check_cancel, slot_read, slot_reads},
    prepared::frame_changes::{FrameChanges, field_mask},
    products::faction_details::selectors,
    triggers::inputs::{self as reuse, TableChange, TableChanges},
};
use dashboard_world::{Epochs, Hold, Needs, Viewed, World};
use replay::{
    columnar_frames::{Recording, Views, Walker, metrics, schema},
    columns,
    segments::{self, Operator, Sink, Start, State},
};
use replay_data::{
    cadence::STAGGER_PERIOD,
    format::FormatVersion,
    rows::{DeltaRow, TABLE_COUNT, TableId, full_mask},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, HourFrame};
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

/// The tables a held reading tells unchanged by their epochs: the links country naming is
/// rebuilt by, and the dictionary goal labels are held for.
const MARKED: [(TableId, Hold); 2] = [
    (TableId::CountryLinks, Hold::Whole),
    (TableId::LexicalTokens, Hold::Whole),
];

/// What a projection reads of its walk's world: views of every table it and its dynamic texts
/// may read ([`tables`]), which cost no rows.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: tables().iter().map(|&table| (table, Hold::Whole)).collect(),
        lexicon: true,
        ..Needs::default()
    })
}

/// One projected hour, for the ordered emit.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub(super) projected: Projected,
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

/// A programmes' supplying read kept past its hour: the programme tables, and the
/// scripted-text tables when it names a scripted project ([`selectors::read_names_scripted`]).
#[derive(Clone)]
pub struct Kept {
    pub(super) hour: i32,
    pub(super) wide: bool,
    pub(super) frame: Arc<DecodedFrame>,
}

/// Where a projection reads a supplying read its walk does not hold: one before the walk's
/// restart, or one it passed without keeping.
pub trait Earlier: Send + Sync {
    /// The latest supplying read of the programmes in `hours` of `branch`, their table
    /// complete at every hour from it through the last of `hours`, scripted names judged by
    /// `cat`; None when no hour is one.
    fn read(
        &self,
        branch: u64,
        hours: (i32, i32),
        cat: Option<&catalogue::Catalogue>,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Kept>>;
}

fn invalid(what: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// The native capture a preparation walks, walked again for supplying reads: from the restart
/// before them, decoding the programme tables alone, and the scripted-text tables only to a read
/// that names a scripted project.
pub struct Capture {
    narrow: Recording,
    wide: Recording,
    format: FormatVersion,
    mode: CaptureMode,
}

impl Capture {
    pub fn new(recording: &Recording, format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let mut narrow = selectors::PROGRAMME_TABLES.to_vec();
        let mut wide = selectors::programme_text_tables();
        narrow.push(TableId::Campaign);
        wide.push(TableId::Campaign);
        Ok(Self {
            narrow: recording.selecting(&narrow)?,
            wide: recording.selecting(&wide)?,
            format,
            mode,
        })
    }

    /// [`Earlier::read`] in `recording`, the read's frame holding `tables`.
    fn walk(
        &self,
        recording: &Recording,
        tables: &[TableId],
        branch: u64,
        (lo, hi): (i32, i32),
        cancelled: &AtomicBool,
    ) -> io::Result<Option<(i32, DecodedFrame)>> {
        let held = (0..recording.run_count()).find_map(|index| {
            let run = recording.run(index).ok()?;
            let last = i64::from(run.first_hour) + i64::from(run.ordinals) - 1;
            (run.branch == branch && run.first_hour <= hi && i64::from(hi) <= last)
                .then_some((index, run.first_hour))
        });
        let Some((run, first_hour)) = held else {
            return Err(invalid(format!(
                "The native capture holds no hour {hi} of branch {branch}"
            )));
        };
        let lo = lo.max(first_hour);
        if lo > hi {
            return Ok(None);
        }
        let ordinal = |hour: i32| u32::try_from(i64::from(hour) - i64::from(first_hour));
        let (Ok(from), Ok(to)) = (ordinal(lo), ordinal(hi)) else {
            return Err(invalid(format!("Hour {hi} is off branch {branch}'s run")));
        };
        let mut walker = Walker::new(recording)?;
        let mut state = State::new(schema()?)?;
        let mut read = None;
        let mut begun = false;
        // The segments after the first go on from the state it left.
        for segment in recording
            .segments()
            .iter()
            .filter(|segment| segment.run == run && segment.last >= from && segment.first <= to)
        {
            if !begun {
                walker.begin(segment, &mut state)?;
                begun = true;
            }
            let going = walker.walk(segment, &mut state, |state, manifest, _| {
                check_cancel(cancelled)?;
                let hour = manifest.hour;
                if hour < lo {
                    return Ok(true);
                }
                if !columns::complete(state, TableId::ProjectPrograms) {
                    read = None;
                } else if slot_read(
                    self.format,
                    &metrics(manifest),
                    TableId::ProjectPrograms,
                    hour,
                ) {
                    let mut views = Views::new(tables, self.format, self.mode)?;
                    read = Some((hour, views.restored(state, branch, hour)?));
                }
                Ok(hour < hi)
            })?;
            if !going {
                break;
            }
        }
        Ok(read)
    }
}

impl Earlier for Capture {
    fn read(
        &self,
        branch: u64,
        hours: (i32, i32),
        cat: Option<&catalogue::Catalogue>,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Kept>> {
        let _cpu = ::replay_frame::cpu::scope("faction supplying read", None);
        let narrow = selectors::PROGRAMME_TABLES;
        let Some((hour, frame)) = self.walk(&self.narrow, narrow, branch, hours, cancelled)?
        else {
            return Ok(None);
        };
        if !cat.is_some_and(|cat| selectors::read_names_scripted(HourFrame::from(&frame), cat)) {
            return Ok(Some(Kept {
                hour,
                wide: false,
                frame: Arc::new(frame),
            }));
        }
        let wide = selectors::programme_text_tables();
        match self.walk(&self.wide, &wide, branch, (hour, hour), cancelled)? {
            Some((at, frame)) if at == hour => Ok(Some(Kept {
                hour,
                wide: true,
                frame: Arc::new(frame),
            })),
            _ => Err(invalid(format!(
                "The native capture's supplying read at hour {hour} of branch {branch} reads otherwise when walked again"
            ))),
        }
    }
}

/// The supplying read at `hour` in the walk's views, kept for the hours after it.
fn keep(
    (world, state): (&World, &State),
    hour: i32,
    cat: Option<&catalogue::Catalogue>,
) -> io::Result<Kept> {
    let wide = cat.is_some_and(|cat| {
        selectors::read_names_scripted(HourFrame::from(world.columns(state)), cat)
    });
    let tables = if wide {
        selectors::programme_text_tables()
    } else {
        selectors::PROGRAMME_TABLES.to_vec()
    };
    Ok(Kept {
        hour,
        wide,
        frame: Arc::new(world.frame(state, &tables)?),
    })
}

/// The programmes' supplying read as the walk finds it: the latest hour that read them, with
/// their table complete at every hour since.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Read {
    /// The walk restarted after a read it has not seen: the capture finds it ([`Earlier`]).
    Unseen,
    At(i32),
    Lost,
}

/// The supplying reads a walk holds past their hour.
#[derive(Default)]
struct Reads {
    /// The read at its last read hour, kept from its views or read from the capture.
    kept: Option<Kept>,
    /// The read before its restart, once the capture was asked for it.
    before: Option<Option<Kept>>,
}

/// The hour's frames, all the walk's views: they hold every table the family reads, so the own
/// frame, a dynamic text's and the programmes' supplying read at this hour are one.
struct Hourly<'p, 'h> {
    hour: &'p segments::Hour<'h, World>,
    epochs: &'p Epochs,
    read: Read,
    /// The walk's first hour after its restart.
    first: Option<i32>,
    reads: &'p RefCell<Reads>,
    earlier: &'p dyn Earlier,
    cat: Option<&'p catalogue::Catalogue>,
    /// Whether a dynamic text asked for every table, or for some, this hour.
    whole: OnceCell<()>,
    reading: OnceCell<()>,
    #[cfg(test)]
    built: &'p std::cell::Cell<usize>,
}

impl Hourly<'_, '_> {
    fn views(&self) -> HourFrame<'_> {
        HourFrame::from(self.hour.world.columns(self.hour.state))
    }

    /// Notes a dynamic text's first ask of `cell` this hour.
    fn asked(&self, cell: &OnceCell<()>) {
        cell.get_or_init(|| {
            #[cfg(test)]
            self.built.set(self.built.get() + 1);
        });
    }

    /// The hour of the read before the walk's restart, read from the capture at the first ask.
    fn before(&self, cancelled: &AtomicBool) -> io::Result<Option<i32>> {
        let mut reads = self.reads.borrow_mut();
        if reads.before.is_none() {
            let first = self
                .first
                .ok_or_else(|| io::Error::other("A restart's walk holds no first hour"))?;
            let hours = (
                first.saturating_sub(STAGGER_PERIOD - 1),
                first.saturating_sub(1),
            );
            let read = self
                .earlier
                .read(self.hour.branch, hours, self.cat, cancelled)?;
            reads.before = Some(read);
        }
        Ok(reads.before.iter().flatten().map(|read| read.hour).next())
    }

    /// The read at `hour`, an earlier hour the walk found one at: kept, or read from the
    /// capture.
    fn held(&self, hour: i32, cancelled: &AtomicBool) -> io::Result<Kept> {
        let mut reads = self.reads.borrow_mut();
        let Reads { kept, before } = &mut *reads;
        if let Some(read) = kept
            .iter()
            .chain(before.iter().flatten())
            .find(|read| read.hour == hour)
        {
            return Ok(read.clone());
        }
        let read = self
            .earlier
            .read(self.hour.branch, (hour, hour), self.cat, cancelled)?
            .filter(|read| read.hour == hour)
            .ok_or_else(|| {
                invalid(format!(
                    "The native capture holds no supplying read at hour {hour} of branch {}",
                    self.hour.branch
                ))
            })?;
        *kept = Some(read.clone());
        Ok(read)
    }
}

impl Frames for Hourly<'_, '_> {
    fn own(&self) -> Viewed<'_> {
        Viewed::walked(self.hour.world, self.hour.state, self.epochs)
    }
    fn whole(&self) -> io::Result<HourFrame<'_>> {
        self.asked(&self.whole);
        Ok(self.views())
    }
    fn reading(&self, _: &[TableId]) -> io::Result<HourFrame<'_>> {
        // The whole frame holds every table a reading asks for.
        if self.whole.get().is_none() {
            self.asked(&self.reading);
        }
        Ok(self.views())
    }
    fn read_hour(&self, cancelled: &AtomicBool) -> io::Result<Option<i32>> {
        let at = match self.read {
            Read::At(at) => at,
            Read::Lost => return Ok(None),
            Read::Unseen => match self.before(cancelled)? {
                Some(at) => at,
                None => return Ok(None),
            },
        };
        Ok((self.hour.hour - at < STAGGER_PERIOD).then_some(at))
    }
    fn supplying(
        &self,
        hour: i32,
        wide: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Supply>> {
        // The walk holds the read's own hour; an earlier one is kept, or the capture's.
        if self.read == Read::At(hour) && hour == self.hour.hour {
            return Ok(Some(Supply::Own));
        }
        let read = self.held(hour, cancelled)?;
        if wide && !read.wide {
            return Err(io::Error::other(format!(
                "The supplying read at hour {hour} was kept without the scripted-text tables its names read"
            )));
        }
        Ok(Some(Supply::Read(read.frame)))
    }
}

/// A segment's projection. It starts cold at every start, as a walk reads its first
/// hour anew; what earlier segments published is the emit's.
pub struct Projector<'a> {
    format: FormatVersion,
    cat: Option<Arc<catalogue::Catalogue>>,
    earlier: Arc<dyn Earlier + 'a>,
    reads: RefCell<Reads>,
    /// The walk's first hour after its restart.
    first: Option<i32>,
    /// The tables whose changes decide whether an hour is evaluated.
    own_tables: Vec<TableId>,
    /// The tables a held reading tells unchanged by, moved on at each hour that may change them.
    epochs: Epochs,
    /// The fields of each table the held dynamic texts read.
    read_fields: Box<[u64; TABLE_COUNT]>,
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
    /// Frames asked for dynamic texts.
    #[cfg(test)]
    built: std::cell::Cell<usize>,
    /// Hours projected from a frame rather than as idle.
    #[cfg(test)]
    pub(super) evaluated: usize,
}

impl<'a> Projector<'a> {
    /// A projection of a recording at `format`, naming by `cat`, the recorded stack's
    /// catalogue, and reading the supplying reads its walk does not hold from `earlier`.
    pub fn new(
        format: FormatVersion,
        cat: Option<Arc<catalogue::Catalogue>>,
        earlier: Arc<dyn Earlier + 'a>,
    ) -> io::Result<Self> {
        Ok(Self {
            format,
            cat,
            earlier,
            reads: RefCell::new(Reads::default()),
            first: None,
            named: false,
            own_tables: own_tables(),
            epochs: Epochs::new(&MARKED)?,
            read_fields: Box::new([0; TABLE_COUNT]),
            country_mask: field_mask(TableId::Countries, &["canonical_slot", "tag"]),
            runtime: Runtime::exact(),
            read: Read::Unseen,
            moved: None,
            last: None,
            cancelled: AtomicBool::new(false),
            #[cfg(test)]
            built: std::cell::Cell::new(0),
            #[cfg(test)]
            evaluated: 0,
        })
    }
}

#[cfg(test)]
impl Projector<'_> {
    /// Frames asked for dynamic texts since the projection started.
    pub(super) fn trigger_frames(&self) -> usize {
        self.built.get()
    }
}

impl Operator<World> for Projector<'_> {
    type Output = Hour;

    fn start(&mut self, start: Start<'_>, _: u64, first: i32) -> io::Result<()> {
        // Every table moves on at the segment's first hour.
        self.epochs.restart();
        *self.reads.get_mut() = Reads::default();
        // A run's start follows no read the walk could not see.
        (self.read, self.moved, self.first) = match start {
            Start::Restart(state) => (
                Read::Unseen,
                Some((
                    BTreeSet::new(),
                    columns::status(state, TableId::FactionPrograms),
                )),
                Some(first),
            ),
            Start::Run { .. } => (Read::Lost, None, None),
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
            self.epochs.restart();
        }
        self.runtime.gather_reads(&mut self.read_fields);
        // Every hour, so no epoch misses an hour that changed its table.
        self.epochs.hour(world);
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
        let held = columns::complete(state, TableId::FactionPrograms)
            && columns::table(state, TableId::FactionPrograms)?
                .live()
                .count()
                > 0;
        let reads = self.reads.get_mut();
        match self.read {
            Read::At(at) if at == hour.hour => {
                // Let go first, so the world's views change in place.
                reads.kept = None;
                self.runtime.forget_supplying();
                // A read no faction's programmes ask for yet is rarely asked: the capture serves it.
                if held {
                    reads.kept = Some(keep((world, state), at, self.cat.as_deref())?);
                }
            }
            Read::At(at) if hour.hour - at < STAGGER_PERIOD => {}
            _ => reads.kept = None,
        }
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
        let programmes = held
            && (slots[TableId::ProjectPrograms.index()]
                || PROGRAMMES.iter().any(|table| changed[table.index()]));
        let mut evaluated = renamed || programmes || !self.runtime.may_idle(hour.hour);
        for &table in &self.own_tables {
            if evaluated {
                break;
            }
            evaluated = reads_change(state, table, whole(table))?;
        }
        let views = HourFrame::from(world.columns(state));
        if !evaluated && self.runtime.holds_dynamic() {
            // Each held text is checked by the hour's change facts alone.
            let moved = Moved::new((state, world), self.format, restart, &self.read_fields);
            let since = reuse::Since::hour_before_in(views, &moved);
            let fresh = self.runtime.check_held(since.as_ref(), hour.hour);
            drop(since);
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
        let edits = Changes {
            moved: Moved::new((state, world), self.format, restart, &self.read_fields),
            flags: FrameChanges::rebuilt_at(world.metrics(), hour.hour, self.format, !restart, |table| {
                changed[table.index()]
            }),
        };
        self.runtime.kept_programmes = self.moved.as_ref().map(|(moved, _)| moved.clone());
        let frames = Hourly {
            hour,
            epochs: &self.epochs,
            read: self.read,
            first: self.first,
            reads: &self.reads,
            earlier: &*self.earlier,
            cat: self.cat.as_deref(),
            whole: OnceCell::new(),
            reading: OnceCell::new(),
            #[cfg(test)]
            built: &self.built,
        };
        let projected =
            self.runtime
                .project(&frames, &edits, self.cat.as_deref(), &self.cancelled)?;
        drop((frames, edits));
        self.last = Some(hour.hour);
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            projected,
        })
    }
}
