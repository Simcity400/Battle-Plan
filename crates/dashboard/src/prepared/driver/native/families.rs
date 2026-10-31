//! Every family with a native backend ([`REGISTERED`]): its operators project its hours from the
//! recording's columns, and its branch states keep them and publish at any segment's end, its
//! next publication going on from the last. A family on the series engine writes each hour into
//! the series its stage opens for the batch ([`Stage::series`]). Each family that can saves what
//! its branch state goes on with after a batch, its own continuation codec's, for a preparation of
//! the recording grown to take up ([`Native::save`]). A family missing here is refused by name on
//! every branch of a native recording until it gains one.

use super::{
    backend::{Bound, Member, Native, Projected, Taken},
    content::{Content, Politics},
    identities::Identities,
};
use crate::prepared::driver::publication::Stage;
use crate::{
    prepared::driver::registry::FamilyId,
    products::{event_feed::columnar as event_feed, style_runs},
};
use dashboard_army::products::division_equipment::history as division_equipment;
use dashboard_base::prepared::{
    roots::{self, Published},
    series::{compress, decompress},
};
use dashboard_country_fuel::products::country_fuel;
use dashboard_diplomacy::products::diplomacy_state;
use dashboard_equipment_losses::products::equipment_losses;
use dashboard_equipment_stockpile::products::equipment_stockpile;
use dashboard_forces::products::{army_size, supply_health};
use dashboard_politics_ledger::products::politics_ledger;
use dashboard_stability::products::country_stability as stability;
use dashboard_text::{
    catalogue::Catalogue,
    metadata::{BranchExtent, Extents},
};
use dashboard_unit_leaders::products::unit_leaders;
use dashboard_world::Needs;
use replay::{columnar_frames::Segment, container::Replay, segments};
use replay_data::{format::FormatVersion, rows::TableId, scope::CaptureMode};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// A member of a run, made of a family's backend bound to its stage.
type Made<'p> = io::Result<Box<dyn Member + 'p>>;

/// A family with a native backend.
pub(crate) struct Registered {
    pub name: &'static str,
    /// Loads the content its backend reads, on a loader thread before the run.
    pub load: fn(&Content<'_>) -> io::Result<()>,
    /// Its backend, taking up what `load` loaded, bound to `stage`, going on as `taken` says.
    pub bind: for<'p> fn(FamilyId, &'p Content<'p>, Stage<'p, 'p>, Taken<'p>) -> Made<'p>,
}

/// Every family with a native backend, in the order their lanes run.
pub(crate) static REGISTERED: &[Registered] = &[
    Registered {
        name: "identities",
        load: free,
        bind: identities,
    },
    Registered {
        name: "army",
        load: free,
        bind: army,
    },
    Registered {
        name: "fuel",
        load: free,
        bind: fuel,
    },
    Registered {
        name: "diplomacy_state",
        load: free,
        bind: diplomacy,
    },
    Registered {
        name: "equipment_losses",
        load: free,
        bind: losses,
    },
    Registered {
        name: "leaders",
        load: |content| content.leaders().map(drop),
        bind: leaders,
    },
    Registered {
        name: "event_feed",
        load: free,
        bind: feed,
    },
    Registered {
        name: "supply",
        load: |content| content.supply().map(drop),
        bind: supply,
    },
    Registered {
        name: "division-equipment",
        load: |content| content.equipment().map(drop),
        bind: equipment,
    },
    Registered {
        name: "stability",
        load: free,
        bind: stability,
    },
    Registered {
        name: "politics_ledger",
        load: |content| content.politics().map(drop),
        bind: ledger,
    },
    Registered {
        name: "stockpile",
        load: |content| content.stockpile().map(drop),
        bind: stockpile,
    },
    Registered {
        name: style_runs::NAME,
        load: |content| content.styles().map(drop),
        bind: styles,
    },
];

/// `branch`'s extent in `extents`; a branch the recording holds no hour of is none of a
/// family's to prepare.
fn extent(extents: &Extents, branch: u64) -> io::Result<&BranchExtent> {
    extents.branch(branch).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "The recording holds no hour of the branch",
        )
    })
}

/// The content a contentless backend loads.
fn free(_: &Content<'_>) -> io::Result<()> {
    Ok(())
}

/// `state`, saved as a row preparation saves a branch's state.
fn saved(state: &impl Serialize) -> io::Result<Option<Vec<u8>>> {
    compress(&serde_json::to_vec(state)?, 3).map(Some)
}

/// The state [`saved`] wrote.
fn taken_up<T: DeserializeOwned>(saved: &[u8]) -> io::Result<T> {
    Ok(serde_json::from_slice(&decompress(saved)?)?)
}

/// The series directory of `published`, the publication of `stage`'s family on `branch` it goes
/// on from, its keys spelled under `schema`.
fn directory(
    stage: &mut Stage<'_, '_>,
    branch: u64,
    published: &Published,
    schema: u64,
) -> io::Result<dashboard_series::family::Directory> {
    let family = stage.family();
    roots::directory(stage.pages()?, family, branch, published, schema)
}

fn identities<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    bound(id, Identities { format }, stage, taken)
}

fn bound<'p, N: Native + 'p>(
    id: FamilyId,
    native: N,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    Ok(Box::new(Bound::new(id, native, stage, taken)))
}

macro_rules! projected {
    ($($hour:ty),* $(,)?) => {
        $(impl Projected for $hour {
            fn hour(&self) -> i32 {
                self.hour
            }
        })*
    };
}

projected!(
    army_size::facts::Hour,
    country_fuel::facts::Hour,
    diplomacy_state::columnar::Hour,
    equipment_losses::columnar::Hour,
    unit_leaders::columnar::Hour,
    event_feed::Hour,
    supply_health::facts::Hour,
    stability::facts::Hour,
    division_equipment::facts::Hour,
    politics_ledger::facts::Hour,
    equipment_stockpile::columnar::Hour,
    style_runs::facts::Hour,
);

/// A family's emit on the series engine.
trait Emits: Send {
    type Hour;

    fn hour(&mut self, series: &mut roots::Series, hour: Self::Hour) -> io::Result<()>;

    /// Appends what is open through `last`, for its batch's root to cut.
    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()>;

    /// What it goes on with after a cut.
    fn save(&self, out: &mut Vec<u8>) -> io::Result<()>;
}

macro_rules! emits {
    ($($emit:ty => $hour:ty),* $(,)?) => {
        $(impl Emits for $emit {
            type Hour = $hour;

            fn hour(&mut self, series: &mut roots::Series, hour: $hour) -> io::Result<()> {
                <$emit>::hour(self, series, hour)
            }

            fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()> {
                <$emit>::cut(self, series, last)
            }

            fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
                <$emit>::save(self, out)
            }
        })*
    };
}

emits!(
    stability::emit::Emit => stability::facts::Hour,
    supply_health::emit::Emit => supply_health::facts::Hour,
    army_size::emit::Emit => army_size::facts::Hour,
    division_equipment::emit::Emit => division_equipment::facts::Hour,
    style_runs::emit::Emit => style_runs::facts::Hour,
    politics_ledger::emit::Emit => politics_ledger::facts::Hour,
    equipment_stockpile::emit::Emit => equipment_stockpile::columnar::Hour,
    country_fuel::emit::Emit => country_fuel::facts::Hour,
);

impl<E: Emits> Emits for Box<E> {
    type Hour = E::Hour;

    fn hour(&mut self, series: &mut roots::Series, hour: E::Hour) -> io::Result<()> {
        (**self).hour(series, hour)
    }

    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()> {
        (**self).cut(series, last)
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        (**self).save(out)
    }
}

/// A branch of a family on the series engine: its emit, the branch's first hour, from which its
/// series starts, and the last hour the emit kept since its batch's root last cut.
struct Series<E> {
    emit: E,
    first: i32,
    last: Option<i32>,
}

impl<E: Emits> Series<E> {
    fn new(emit: E, first: i32) -> Self {
        Self {
            emit,
            first,
            last: None,
        }
    }

    fn observe(
        &mut self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        at: i32,
        hour: E::Hour,
    ) -> io::Result<()> {
        let series = stage.series(branch, self.first).map_err(segments::fatal)?;
        self.emit.hour(series, hour)?;
        self.last = Some(at);
        Ok(())
    }

    fn flush(&mut self, stage: &mut Stage<'_, '_>, branch: u64) -> io::Result<()> {
        match self.last.take() {
            Some(last) => self.emit.cut(stage.series(branch, self.first)?, last),
            None => Ok(()),
        }
    }

    /// What its emit goes on with after its last cut.
    fn save(&self) -> io::Result<Option<Vec<u8>>> {
        let mut out = Vec::new();
        self.emit.save(&mut out)?;
        Ok(Some(out))
    }
}

struct Army<'p> {
    replay: &'p Replay,
}

fn army<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, Army { replay }, stage, taken)
}

impl Native for Army<'_> {
    type Operator = army_size::facts::Projector;
    type Branch = Series<Box<army_size::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        army_size::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(army_size::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        Ok(army_size::facts::Projector::new(self.replay))
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(army_size::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: army_size::facts::Hour,
    ) -> io::Result<()> {
        state.observe(stage, branch, hour.hour, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage, branch)
    }
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save()
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let directory = directory(stage, branch, published, army_size::emit::SCHEMA)?;
        let emit = army_size::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Fuel {
    format: FormatVersion,
}

fn fuel<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    bound(id, Fuel { format }, stage, taken)
}

impl Native for Fuel {
    type Operator = country_fuel::facts::Projector;
    type Branch = Series<Box<country_fuel::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        country_fuel::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(country_fuel::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        Ok(country_fuel::facts::Projector::new(self.format))
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = country_fuel::emit::Emit::new(self.format);
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: country_fuel::facts::Hour,
    ) -> io::Result<()> {
        state.observe(stage, branch, hour.hour, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage, branch)
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save()
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let directory = directory(stage, branch, published, country_fuel::emit::SCHEMA)?;
        let emit = country_fuel::emit::Emit::load(self.format, &directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Diplomacy {
    format: FormatVersion,
    mode: CaptureMode,
}

fn diplomacy<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    let native = Diplomacy {
        format: replay.format,
        mode: replay.capture_mode,
    };
    bound(id, native, stage, taken)
}

impl Native for Diplomacy {
    type Operator = diplomacy_state::columnar::Projector;
    type Branch = Box<diplomacy_state::prepared::Prepared>;

    fn tables(&self) -> &'static [TableId] {
        diplomacy_state::block::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        diplomacy_state::columnar::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        diplomacy_state::columnar::Projector::new(self.format, self.mode)
    }

    fn branch(&self, _: u64, _: i32) -> io::Result<Self::Branch> {
        Ok(Box::default())
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        _: u64,
        hour: diplomacy_state::columnar::Hour,
    ) -> io::Result<()> {
        state.observe_projected(hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let pages = stage.pages()?;
        state.flush(pages, branch, cancelled)?;
        state.save(pages, branch)
    }

    // It goes on from the state its flush saved.
    fn save(&self, _: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        Ok(Some(Vec::new()))
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        _: &[u8],
    ) -> io::Result<Self::Branch> {
        let section = diplomacy_state::prepared::state_section(branch);
        let range = stage.pages()?.read_at(&section, 0)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Its saved state is missing")
        })?;
        diplomacy_state::prepared::Prepared::restore(range).map(Box::new)
    }
}

struct Losses<'p> {
    replay: &'p Replay,
    extents: &'p Extents,
}

fn losses<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Losses {
        replay: content.source.replay_ref(),
        extents: &content.extents,
    };
    bound(id, native, stage, taken)
}

impl Native for Losses<'_> {
    type Operator = equipment_losses::columnar::Projector;
    type Branch = Box<equipment_losses::prepared::Prepared>;

    fn tables(&self) -> &'static [TableId] {
        equipment_losses::columnar::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(equipment_losses::columnar::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        equipment_losses::columnar::Projector::new(self.replay)
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        let first = extent(self.extents, branch)?.bounds.0;
        Ok(Box::new(equipment_losses::prepared::Prepared::starting(
            self.replay.format,
            first,
        )))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        _: u64,
        hour: equipment_losses::columnar::Hour,
    ) -> io::Result<()> {
        state.observe_projected(hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        // The branch goes on through the recording's gaps after its last recorded hour.
        if through.is_some() {
            state.finish_block(extent(self.extents, branch)?.bounds.1);
        }
        state.flush(stage.pages()?, branch, cancelled)
    }
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        saved(&**state)
    }

    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        _: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        taken_up(saved).map(Box::new)
    }
}

struct Leaders<'p> {
    replay: &'p Replay,
    binding: &'p unit_leaders::Binding,
    /// Each branch's first recorded hour.
    starts: BTreeMap<u64, i32>,
}

fn leaders<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let starts = content.extents.iter();
    let native = Leaders {
        replay: content.source.replay_ref(),
        binding: content.leaders()?,
        starts: starts
            .map(|(branch, extent)| (branch, extent.recorded.0))
            .collect(),
    };
    bound(id, native, stage, taken)
}

impl Native for Leaders<'_> {
    type Operator = unit_leaders::columnar::Projector;
    type Branch = Box<unit_leaders::prepared::Prepared>;

    fn tables(&self) -> &'static [TableId] {
        unit_leaders::block::tables()
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(unit_leaders::columnar::needs(self.binding.catalogue))
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        Ok(unit_leaders::columnar::Projector::starting(
            (self.replay.format, self.replay.capture_mode),
            self.starts.clone(),
            self.binding.content.clone(),
            self.binding.catalogue,
        ))
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        let start = *self.starts.get(&branch).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The recording holds no hour of the branch",
            )
        })?;
        Ok(Box::new(unit_leaders::prepared::Prepared::starting(
            self.replay.format,
            branch,
            start,
        )))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        _: u64,
        hour: unit_leaders::columnar::Hour,
    ) -> io::Result<()> {
        state.observe_projected(hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(
            stage.pages()?,
            branch,
            self.binding.content.as_deref(),
            &self.binding.errors,
            cancelled,
        )
    }
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.saved_catalogue().map(Some)
    }

    // The continuation its flush writes, with the catalogue it saved.
    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let section = format!("unit-leaders-{branch}");
        let range = stage.pages()?.read_at(&section, 0)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Its continuation is missing")
        })?;
        unit_leaders::prepared::Prepared::resumed(range, saved).map(Box::new)
    }
}

struct Feed<'p> {
    replay: &'p Replay,
    /// Each branch's first and last hours, gaps included.
    bounds: BTreeMap<u64, (i32, i32)>,
}

fn feed<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let bounds = content.extents.iter();
    let native = Feed {
        replay: content.source.replay_ref(),
        bounds: bounds
            .map(|(branch, extent)| (branch, extent.bounds))
            .collect(),
    };
    bound(id, native, stage, taken)
}

impl Native for Feed<'_> {
    type Operator = event_feed::Projector;
    type Branch = Box<event_feed::Prepared>;

    fn tables(&self) -> &'static [TableId] {
        event_feed::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        event_feed::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        event_feed::Projector::within(self.replay, self.bounds.clone())
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        let bounds = *self.bounds.get(&branch).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The recording holds no hour of the branch",
            )
        })?;
        Ok(Box::new(event_feed::Prepared::within(
            self.replay,
            branch,
            bounds,
        )))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        _: u64,
        hour: event_feed::Hour,
    ) -> io::Result<()> {
        state.observe_projected(hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let pages = stage.pages()?;
        state.flush(pages, branch, cancelled)?;
        state.save_state(pages, branch)
    }
}

struct Supply<'p> {
    /// The content's defines, or why they were refused, and its warnings.
    configured: &'p supply_health::Configured,
    format: FormatVersion,
}

fn supply<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Supply {
        configured: content.supply()?,
        format: content.source.replay_ref().format,
    };
    bound(id, native, stage, taken)
}

impl Native for Supply<'_> {
    type Operator = supply_health::facts::Projector;
    type Branch = Series<supply_health::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        supply_health::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(supply_health::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        self.configured.projector(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(self.configured.emit(), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: supply_health::facts::Hour,
    ) -> io::Result<()> {
        state.observe(stage, branch, hour.hour, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage, branch)
    }
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save()
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let directory = directory(stage, branch, published, supply_health::emit::SCHEMA)?;
        let emit = supply_health::emit::Emit::load(self.configured, &directory, saved)?;
        Ok(Series::new(emit, published.frontier.first_hour))
    }
}

struct Stability {
    format: u16,
}

fn stability<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format.version();
    bound(id, Stability { format }, stage, taken)
}

impl Native for Stability {
    type Operator = stability::facts::Projector;
    type Branch = Series<stability::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        stability::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(stability::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        stability::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(stability::emit::Emit::new(), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: stability::facts::Hour,
    ) -> io::Result<()> {
        state.observe(stage, branch, hour.hour, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage, branch)
    }
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save()
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let directory = directory(stage, branch, published, stability::emit::SCHEMA)?;
        let emit = stability::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(emit, published.frontier.first_hour))
    }
}

struct Styles<'p> {
    /// The recorded flag folders, or why flags cannot be chosen.
    configured: &'p style_runs::Configured,
    /// Every segment's full rebuilds, by cause.
    rebuilds: Arc<style_runs::facts::Rebuilds>,
}

fn styles<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Styles {
        configured: content.styles()?,
        rebuilds: Arc::default(),
    };
    bound(id, native, stage, taken)
}

impl Native for Styles<'_> {
    type Operator = style_runs::facts::Projector;
    type Branch = Series<style_runs::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        style_runs::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        style_runs::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        self.configured.projector(&self.rebuilds)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(self.configured.emit(), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: style_runs::facts::Hour,
    ) -> io::Result<()> {
        state.observe(stage, branch, hour.hour, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage, branch)
    }

    fn counts(&self) -> Vec<(&'static str, u64)> {
        self.rebuilds.counts().to_vec()
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save()
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let directory = directory(stage, branch, published, style_runs::emit::SCHEMA)?;
        let emit = style_runs::emit::Emit::load(self.configured, &directory, saved)?;
        Ok(Series::new(emit, published.frontier.first_hour))
    }
}

struct Equipment<'p> {
    format: FormatVersion,
    catalogue: &'p Option<Arc<Catalogue>>,
    /// Each branch's first recorded hour, and each recorded gap.
    firsts: Vec<(u64, i32)>,
    gaps: Vec<(u64, i32, i32)>,
}

fn equipment<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let extents = &content.extents;
    let native = Equipment {
        format: content.source.replay_ref().format,
        catalogue: content.equipment()?,
        firsts: extents
            .iter()
            .map(|(branch, extent)| (branch, extent.recorded.0))
            .collect(),
        gaps: extents
            .iter()
            .flat_map(|(branch, extent)| extent.gaps.iter().map(move |&(f, l)| (branch, f, l)))
            .collect(),
    };
    bound(id, native, stage, taken)
}

impl Native for Equipment<'_> {
    type Operator = division_equipment::facts::Projector;
    type Branch = Series<Box<division_equipment::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        division_equipment::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        division_equipment::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        division_equipment::facts::Projector::within(
            self.format,
            (self.firsts.clone(), self.gaps.clone()),
            self.catalogue.clone(),
        )
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = division_equipment::emit::Emit::new(self.catalogue.clone());
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: division_equipment::facts::Hour,
    ) -> io::Result<()> {
        state.observe(stage, branch, hour.hour, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage, branch)
    }

    // The engine the next segment's first hours are stepped on is saved with the emit.
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save()
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let directory = directory(stage, branch, published, division_equipment::emit::SCHEMA)?;
        let emit = division_equipment::emit::Emit::load(self.catalogue.clone(), &directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Ledger<'p> {
    format: FormatVersion,
    content: &'p Politics,
}

fn ledger<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Ledger {
        format: content.source.replay_ref().format,
        content: content.politics()?,
    };
    bound(id, native, stage, taken)
}

impl Native for Ledger<'_> {
    type Operator = politics_ledger::facts::Projector;
    type Branch = Series<Box<politics_ledger::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        politics_ledger::facts::tables()
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(politics_ledger::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        politics_ledger::facts::Projector::new(self.format, self.content)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(
            Box::new(politics_ledger::emit::Emit::new(self.content)),
            first,
        ))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: politics_ledger::facts::Hour,
    ) -> io::Result<()> {
        state.observe(stage, branch, hour.hour, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage, branch)
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save()
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let directory = directory(stage, branch, published, politics_ledger::emit::SCHEMA)?;
        let emit = politics_ledger::emit::Emit::load(self.content, &directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Stockpile<'p> {
    format: FormatVersion,
    content: &'p equipment_stockpile::emit::Content,
}

fn stockpile<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Stockpile {
        format: content.source.replay_ref().format,
        content: content.stockpile()?,
    };
    bound(id, native, stage, taken)
}

impl Native for Stockpile<'_> {
    type Operator = equipment_stockpile::columnar::Projector;
    type Branch = Series<Box<equipment_stockpile::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        equipment_stockpile::columnar::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(equipment_stockpile::columnar::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        equipment_stockpile::columnar::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = equipment_stockpile::emit::Emit::new(self.format, self.content.clone());
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: equipment_stockpile::columnar::Hour,
    ) -> io::Result<()> {
        state.observe(stage, branch, hour.hour, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage, branch)
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save()
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let directory = directory(stage, branch, published, equipment_stockpile::emit::SCHEMA)?;
        let emit = equipment_stockpile::emit::Emit::load(
            self.format,
            self.content.clone(),
            &directory,
            saved,
        )?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}
