//! What the operators of one walk of a columnar recording share ([`segments::World`]): the
//! countries with what each hour changed of them, each state's owner, the spellings and
//! cosmetic tags names are read with, and views of the tables its lanes read ([`World::columns`]):
//! each table's live rows in key order, read where the walker holds them, nothing copied. A part
//! is kept only when a lane the walk carries reads it ([`Needs`]), and follows the walker's
//! changed rows alone; the walk's first hour builds every part whole.

use replay::{
    columnar_frames::metrics,
    columns::Status,
    segments::{self, Manifest, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId},
    scope::CaptureMode,
};
use replay_frame::frame::{
    ColumnFrame, ColumnView, DecodedFrame, Metrics, Orders, SharedView, not_selected,
};
use std::{
    io,
    sync::{Mutex, PoisonError},
};

mod changes;
mod copies;
mod countries;
mod divisions;
mod epochs;
mod flags;
mod frame;
mod lexicon;
mod naming;
mod owners;
#[cfg(test)]
mod tests;

pub use changes::Changes;
pub use copies::Copies;
pub use countries::{Country, Roster, Tag, Tagged};
pub use divisions::{Division, Divisions, Held, Kept};
pub use epochs::{Epochs, Mark, Viewed};
pub use flags::{Appearance, FLAG_TABLES, FlagInputs, Flags};
pub use frame::Hold;
pub use lexicon::{Found, Lexicon};
pub use naming::Naming;
pub use owners::Owners;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// The tables country identity is read from.
pub const IDENTITY: [TableId; 2] = [TableId::Countries, TableId::CountryLinks];

/// How a frame holds [`IDENTITY`] for identity alone: Countries' tags and slots, every link.
pub fn identity() -> io::Result<Vec<(TableId, Hold)>> {
    let field = |name| replay::columns::field(TableId::Countries, name);
    Ok(vec![
        (
            TableId::Countries,
            Hold::Fields(1 << field("tag")? | 1 << field("canonical_slot")?),
        ),
        (TableId::CountryLinks, Hold::Whole),
    ])
}

/// Gives back the walk's views that [`World::frame_held_into`] gave `frame` for `asks`, and
/// Campaign's: each table not selected again.
pub fn release(frame: &mut DecodedFrame, asks: &[(TableId, Hold)]) {
    for &(table, _) in asks.iter().chain(&[(TableId::Campaign, Hold::Whole)]) {
        frame.tables[table.index()] = not_selected();
    }
}

/// What a lane reads of its walk's world.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Needs {
    /// The countries and their links ([`World::countries`]).
    pub countries: bool,
    /// Each state's owner ([`World::owners`]).
    pub owners: bool,
    /// The divisions, their maximum hit points and manpower ([`World::divisions`]).
    pub divisions: bool,
    /// Token spellings and cosmetic tags ([`World::naming`]).
    pub naming: bool,
    /// The lexer tokens by their literal under the fold ([`World::lexicon`]).
    pub lexicon: bool,
    /// Each country's flag inputs ([`World::flags`]), read from [`FLAG_TABLES`].
    pub flags: bool,
    /// The tables the lane reads through the world's views ([`World::columns`]), and the fields
    /// of each it reads.
    pub frame: Vec<(TableId, Hold)>,
}

impl Needs {
    /// What `lanes` read together: each table whole where a lane reads it whole, and by the
    /// fields every lane reading it by fields reads, held apart.
    pub fn union<'n>(lanes: impl IntoIterator<Item = &'n Needs>) -> Self {
        let mut union = Self::default();
        // Each table's whole hold, and the fields read by fields, if any lane reads it so.
        let mut holds = vec![(false, None::<u64>); TABLE_COUNT];
        for needs in lanes {
            union.countries |= needs.countries;
            union.owners |= needs.owners;
            union.divisions |= needs.divisions;
            union.naming |= needs.naming;
            union.lexicon |= needs.lexicon;
            union.flags |= needs.flags;
            for &(table, hold) in &needs.frame {
                let held = &mut holds[table.index()];
                match hold.of(table) {
                    Hold::Whole => held.0 = true,
                    Hold::Fields(fields) => held.1 = Some(held.1.unwrap_or(0) | fields),
                }
            }
        }
        for table in TableId::ALL {
            let (whole, fields) = holds[table.index()];
            if whole {
                union.frame.push((table, Hold::Whole));
            }
            if let Some(fields) = fields {
                let read = Hold::Fields(fields).of(table);
                if !(whole && read == Hold::Whole) {
                    union.frame.push((table, read));
                }
            }
        }
        union
    }
}

/// What an hour changed of a table: the fields it wrote, and whether it inserted or deleted
/// rows or read the table in part.
#[derive(Clone, Copy, Default)]
struct Touch {
    written: u64,
    rows: bool,
}

/// The hour's change facts, as every part reads them.
pub(crate) struct Facts<'w> {
    first: bool,
    restated: &'w [bool],
    touches: &'w [Touch],
}

impl Facts<'_> {
    /// See [`World::touched`].
    pub(crate) fn touched(&self, table: TableId, fields: u64) -> bool {
        let index = table.index();
        self.first || self.restated[index] || {
            let touch = self.touches[index];
            touch.rows || touch.written & fields != 0
        }
    }

    pub(crate) fn first(&self) -> bool {
        self.first
    }
}

/// A walk's world: built for the walk, advanced once an hour before its operators.
pub struct World {
    format: FormatVersion,
    mode: CaptureMode,
    branch: u64,
    hour: i32,
    /// Before the walk's first hour.
    cold: bool,
    /// The hour is the walk's first.
    first: bool,
    metrics: Metrics,
    statuses: Vec<Option<Status>>,
    /// Each table's status differs from the hour before's.
    restated: Vec<bool>,
    touches: Vec<Touch>,
    /// The tables whose touches the hour set, to clear before the next.
    sectioned: Vec<usize>,
    countries: Option<Roster>,
    owners: Option<Owners>,
    divisions: Option<Divisions>,
    naming: Option<Naming>,
    lexicon: Option<Lexicon>,
    /// Each table the lanes read by view, its live slots in key order.
    orders: Orders,
    /// Rows built for a lane asking a frame of them ([`Self::frame_held`]) in place of views,
    /// only of the tables it asks; asked by the walk's operators on whichever workers run them.
    frame: Mutex<frame::Frame>,
    /// How [`Self::identity`] asks for its tables.
    identity: Vec<(TableId, Hold)>,
}

fn undeclared(part: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("No lane of the walk declared it reads the world's {part}"),
    )
}

impl World {
    /// A world keeping what `needs` asks for, of a recording at `format` captured in `mode`.
    pub fn new(needs: &Needs, format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        // Each table once, as every declaration of it holds it.
        let needs = &Needs::union([needs]);
        let roster = needs.countries || needs.owners || needs.naming || needs.flags;
        let viewed = needs
            .frame
            .iter()
            .map(|&(table, _)| table)
            .chain(IDENTITY)
            .chain([TableId::Campaign]);
        Ok(Self {
            format,
            mode,
            branch: 0,
            hour: 0,
            cold: true,
            first: false,
            metrics: Metrics::default(),
            statuses: engine_allocation::filled(TABLE_COUNT, None)?,
            restated: engine_allocation::filled(TABLE_COUNT, false)?,
            touches: engine_allocation::filled(TABLE_COUNT, Touch::default())?,
            sectioned: Vec::new(),
            countries: roster.then(|| Roster::new(needs.flags)).transpose()?,
            owners: needs.owners.then(Owners::new).transpose()?,
            divisions: needs
                .divisions
                .then(|| Divisions::new(format))
                .transpose()?,
            naming: needs.naming.then(|| Naming::new(format)).transpose()?,
            lexicon: needs
                .lexicon
                .then(|| Lexicon::new(format.version()))
                .transpose()?,
            orders: Orders::new(viewed)?,
            frame: Mutex::new(frame::Frame::new(&[], format, mode)?),
            identity: identity()?,
        })
    }

    /// The world `held` keeps for a walk driven hour by hour, advanced to `state`'s hour: built
    /// anew of `needs` at the walk's first hour (`starts`), or while none is held.
    pub fn at<'w>(
        held: &'w mut Option<World>,
        starts: bool,
        needs: &Needs,
        (format, mode): (FormatVersion, CaptureMode),
        state: &State,
        manifest: &Manifest<'_>,
    ) -> io::Result<&'w World> {
        if starts || held.is_none() {
            *held = Some(Self::new(needs, format, mode)?);
        }
        let world = held
            .as_mut()
            .ok_or_else(|| invalid("A walk holds no world"))?;
        segments::World::hour(world, state, manifest)?;
        Ok(world)
    }

    /// The hour's branch and hour.
    pub fn branch_hour(&self) -> (u64, i32) {
        (self.branch, self.hour)
    }

    /// Whether the hour is the walk's first, at which every part was built whole.
    pub fn first(&self) -> bool {
        self.first
    }

    /// The hour's metrics, as its manifest keeps them.
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// Whether the hour may have changed `fields` of `table`: it wrote one of them, inserted
    /// or deleted rows, read the table in part or gave it another status, or is the walk's
    /// first.
    pub fn touched(&self, table: TableId, fields: u64) -> bool {
        self.facts().touched(table, fields)
    }

    /// Whether `table`'s status differs from the hour before's, or the hour is the walk's first.
    pub fn restated(&self, table: TableId) -> bool {
        self.first || self.restated[table.index()]
    }

    pub fn countries(&self) -> io::Result<&Roster> {
        self.countries
            .as_ref()
            .ok_or_else(|| undeclared("countries"))
    }

    /// Each country's flag inputs: what the game picks its flag by beside its tag.
    pub fn flags(&self) -> io::Result<&Flags> {
        self.countries
            .as_ref()
            .and_then(Roster::flags)
            .ok_or_else(|| undeclared("flags"))
    }

    pub fn owners(&self) -> io::Result<&Owners> {
        self.owners.as_ref().ok_or_else(|| undeclared("owners"))
    }

    pub fn divisions(&self) -> io::Result<&Divisions> {
        self.divisions
            .as_ref()
            .ok_or_else(|| undeclared("divisions"))
    }

    pub fn naming(&self) -> io::Result<&Naming> {
        self.naming.as_ref().ok_or_else(|| undeclared("naming"))
    }

    /// The walk's one lexer fold index, which every lane's trigger worlds on its frames share;
    /// none unless a lane declared it.
    pub fn lexicon(&self) -> Option<&Lexicon> {
        self.lexicon.as_ref()
    }

    /// The hour read through views of the tables the walk's lanes declared ([`Needs::frame`]),
    /// Campaign and [`IDENTITY`]: each table's rows in key order where the walker holds them,
    /// read by slot. Any other table is not selected.
    pub fn columns<'s>(&'s self, state: &'s State) -> ColumnFrame<'s> {
        ColumnFrame::new(
            (self.branch, self.hour),
            &self.metrics,
            state,
            &self.orders,
            (self.mode, self.format.version()),
        )
    }

    /// `table` this hour, as [`Self::columns`] reads it.
    pub fn view<'s>(&'s self, state: &'s State, table: TableId) -> ColumnView<'s> {
        self.columns(state).view(table)
    }

    /// A decoded copy of `table` this hour, its rows holding the fields `hold` names: for a lane
    /// handing the table on whole, built from the walker's columns. A complete table is sampled
    /// at its last complete read, as a decoder's view is.
    pub fn copy(&self, state: &State, table: TableId, hold: Hold) -> io::Result<SharedView> {
        let view = self.view(state, table);
        let sampled = view
            .is_complete()
            .then(|| state.sampled_at(table.index()))
            .flatten();
        let mask = match hold.of(table) {
            Hold::Whole => table.full_mask(),
            Hold::Fields(fields) => fields,
        };
        Ok(SharedView::from(view.to_table_view(mask)?).with_sampled_at(sampled))
    }

    /// Copies of `asks` and Campaign into `frame`, as [`Self::copy`] makes them, its other
    /// tables left as they are, dated this hour: for a lane handing those tables on whole.
    pub fn copy_into(
        &self,
        state: &State,
        asks: &[(TableId, Hold)],
        frame: &mut DecodedFrame,
    ) -> io::Result<()> {
        frame.branch = self.branch;
        frame.hour = self.hour;
        frame.metrics.clone_from(&self.metrics);
        for &(table, hold) in asks.iter().chain(&[(TableId::Campaign, Hold::Whole)]) {
            frame.tables[table.index()] = self.copy(state, table, hold)?;
        }
        Ok(())
    }

    /// The bytes the views' key orders hold.
    pub fn views_heap_bytes(&self) -> usize {
        self.orders.heap_bytes()
    }

    /// [`Self::frame_held`] of every field of `tables`.
    pub fn frame(&self, state: &State, tables: &[TableId]) -> io::Result<DecodedFrame> {
        let asks =
            engine_allocation::collect_vec(tables.iter().map(|&table| (table, Hold::Whole)))?;
        self.frame_held(state, &asks)
    }

    /// A frame of rows of each table of `asks`, and Campaign, at this hour, for a lane that
    /// reads rows in place of [`Self::columns`]: the walk's one row copy of each, held from the
    /// first ask of it on and rebuilt at the first ask of an hour from the rows changed since.
    /// Drop the frame before the hour ends: a view still shared when the next hour builds it is
    /// copied, not changed.
    pub fn frame_held(&self, state: &State, asks: &[(TableId, Hold)]) -> io::Result<DecodedFrame> {
        let mut frame = DecodedFrame {
            branch: self.branch,
            hour: self.hour,
            tables: std::array::from_fn(|_| not_selected()),
            metrics: Metrics::default(),
        };
        self.frame_held_into(state, asks, &mut frame)?;
        Ok(frame)
    }

    /// [`Self::frame_held`] into `frame`, a frame kept from hour to hour, its other tables left
    /// as they are: no frame is made anew. It must hold none of the walk's views when asked:
    /// [`release`] them before the hour ends, so the next hour's first ask, any lane's,
    /// changes them in place.
    pub fn frame_held_into(
        &self,
        state: &State,
        asks: &[(TableId, Hold)],
        frame: &mut DecodedFrame,
    ) -> io::Result<()> {
        frame.branch = self.branch;
        frame.hour = self.hour;
        frame.metrics.clone_from(&self.metrics);
        // The views are the walk's, built by whichever lane asks first.
        let _memory = engine_allocation::labels::enter(
            engine_allocation::labels::Owner::RUN,
            engine_allocation::labels::Kind::World,
        );
        self.frame
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .frame(state, asks, self.hour, frame)
    }

    /// The frame of the countries' identity tables ([`IDENTITY`]), as [`identity`] holds them.
    pub fn identity(&self, state: &State) -> io::Result<DecodedFrame> {
        self.frame_held(state, &self.identity)
    }

    fn facts(&self) -> Facts<'_> {
        Facts {
            first: self.first,
            restated: &self.restated,
            touches: &self.touches,
        }
    }
}

impl segments::World for World {
    fn hour(&mut self, state: &State, manifest: &Manifest<'_>) -> io::Result<()> {
        self.first = std::mem::take(&mut self.cold);
        self.branch = manifest.branch;
        self.hour = manifest.hour;
        self.metrics = metrics(manifest);
        for &index in &self.sectioned {
            self.touches[index] = Touch::default();
        }
        self.sectioned.clear();
        for section in &manifest.sections {
            let index = section.table;
            if index >= TABLE_COUNT {
                return Err(invalid("A section names a table past the schema"));
            }
            if state.skips(index) {
                continue;
            }
            let changes = state
                .changes(index)
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            self.touches[index] = Touch {
                written: changes
                    .masks
                    .iter()
                    .chain(&changes.fill_masks)
                    .fold(0, |written, mask| written | mask),
                rows: changes.truncated
                    || !changes.inserted.is_empty()
                    || !changes.deleted.is_empty(),
            };
            engine_allocation::reserve(&mut self.sectioned, 1)?;
            self.sectioned.push(index);
        }
        for (index, held) in self.statuses.iter_mut().enumerate() {
            let now = state.status(index);
            self.restated[index] = *held != now;
            *held = now;
        }
        let facts = Facts {
            first: self.first,
            restated: &self.restated,
            touches: &self.touches,
        };
        if let Some(roster) = &mut self.countries {
            roster.hour(state, &facts, self.format)?;
        }
        if let (Some(owners), Some(roster)) = (&mut self.owners, &self.countries) {
            owners.hour(state, &facts, roster, self.format)?;
        }
        if let Some(divisions) = &mut self.divisions {
            divisions.hour(state, &facts)?;
        }
        if let (Some(naming), Some(roster)) = (&mut self.naming, &self.countries) {
            naming.hour(state, &facts, roster)?;
        }
        if let Some(lexicon) = &mut self.lexicon {
            lexicon.hour(state, &facts, (self.branch, self.hour))?;
        }
        self.orders.hour(state, self.first)?;
        self.frame
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .note(state, manifest, &facts)
    }
}
