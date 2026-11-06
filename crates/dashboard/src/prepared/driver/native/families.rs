//! Every family with a native backend ([`REGISTERED`]): its operators project its hours from the
//! recording's columns, and its branch states keep them and publish at any segment's end, its
//! next publication going on from the last. A family on the series engine writes each hour into
//! the series its stage opens for the batch ([`Stage::series`]). Each family that can saves what
//! its branch state goes on with after a batch, its own continuation codec's, for a preparation of
//! the recording grown to take up ([`Native::save`]). A recording-wide job beside the families is
//! registered as one of them. A family or job missing here is refused by name on every branch of
//! a native recording until it gains one.

use super::{
    backend::{self, Bound, Member, Native, Projected, Taken},
    content::{Content, Politics},
    convoys::Convoys,
    groups::Groups,
    identities::Identities,
    repairs::Dockyards,
};
use crate::prepared::driver::publication::Stage;
use crate::{
    prepared::driver::registry::FamilyId,
    prepared_focus as national_focus,
    prepared_production::{ProductionAccumulator, columnar as production},
    products::{
        air_superiority, event_feed::columnar as event_feed, intelligence_prepared,
        land_battles, land_owners::native as land_owners, naval_battles, raids, ships_sunk,
        style_runs,
    },
};
use dashboard_air_aces::products::air_aces;
use dashboard_army::products::division_equipment::history as division_equipment;
use dashboard_construction::products::construction_log;
use dashboard_base::prepared::{
    roots::{self, Published},
    series::{compress, decompress},
};
use dashboard_base::products::territory;
use dashboard_base::products::research;
use dashboard_base::products::{
    equipment_icon::IconContent, production_lines::columnar as production_lines,
};
use dashboard_base::products::country_factories;
use dashboard_country_fuel::products::country_fuel;
use dashboard_country_nukes::products::country_nukes;
use dashboard_diplomacy::products::diplomacy_state;
use dashboard_economy::products::lend_lease_balance;
use dashboard_economy::products::fuel_branches;
use dashboard_economy::products::market;
use dashboard_economy::products::production_catalogue;
use dashboard_equipment_losses::products::equipment_losses;
use dashboard_equipment_stockpile::products::equipment_stockpile;
use dashboard_factions::products::faction_state;
use dashboard_game_score::products::game_score;
use dashboard_ideas::products::ideas;
use dashboard_doctrine_mastery::products::doctrine_mastery;
use dashboard_intelligence::products::intel_coverage;
use dashboard_keyed::{
    native::{self as scanned, Native as Scanned},
    segment::{self as keyed, Segmented},
};
use dashboard_naval_regions::products::{minefields, naval_supremacy};
use dashboard_occupation::products::{occupation_reserve, occupied_states};
use dashboard_party_history::products::party_history;
use dashboard_weather::products::regional_weather;
use dashboard_naval_yards::products::yards::{
    self,
    segment::{Refits, Repairs, View},
};
use dashboard_manpower::products::country_people;
use dashboard_place_names::products::place_names;
use dashboard_special_projects::products::special_projects;
use dashboard_politics::products::politics_state::history as politics_state;
use dashboard_measure::products::country_balances;
use dashboard_peace_deals::products::peace_deals;
use dashboard_air_operations::products::{air_missions, air_statistics};
use dashboard_forces::products::{air_force, army_size, navy, supply_health};
use dashboard_military_air_ledger::products::military_air_ledger;
use dashboard_politics_ledger::products::politics_ledger;
use dashboard_resource_balance::products::trade_flows;
use dashboard_resource_balance::products::resource_balance;
use dashboard_stability::products::{country_stability as stability, experience, stability_parts};
use dashboard_subjects::products::subjects;
use dashboard_subjects::products::autonomy as subject_autonomy;
use dashboard_text::{
    catalogue::Catalogue,
    metadata::{BranchExtent, Extents},
};
use dashboard_unit_leaders::products::unit_leaders;
use dashboard_world_tension::products::world_tension;
use dashboard_world::Needs;
use replay::{columnar_frames::Segment, container::Replay};
use replay_data::{format::FormatVersion, rows::TableId};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    io,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
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

/// Every family and recording-wide job with a native backend, in the order their lanes run.
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
    Registered {
        name: "naval-repairs",
        load: free,
        bind: yard::<Repairs>,
    },
    Registered {
        name: "naval-refits",
        load: free,
        bind: yard::<Refits>,
    },
    Registered {
        name: place_names::NAME,
        load: free,
        bind: names,
    },
    Registered {
        name: country_people::NAME,
        load: free,
        bind: manpower,
    },
    Registered {
        name: "nukes",
        load: free,
        bind: nukes,
    },
    Registered {
        name: "naval",
        load: |content| content.naval().map(drop),
        bind: naval,
    },
    Registered {
        name: politics_state::FAMILY,
        load: free,
        bind: politics,
    },
    Registered {
        name: "wars",
        load: free,
        bind: wars,
    },
    Registered {
        name: country_balances::NAME,
        load: free,
        bind: balances,
    },
    Registered {
        name: "territory",
        load: |content| content.territory().map(drop),
        bind: territory,
    },
    Registered {
        name: "game_score",
        load: |content| content.score().map(drop),
        bind: score,
    },
    Registered {
        name: "groups",
        load: free,
        bind: groups,
    },
    Registered {
        name: "tension",
        load: free,
        bind: tension,
    },
    Registered {
        name: "production",
        load: free,
        bind: production,
    },
    Registered {
        name: "subjects",
        load: free,
        bind: relations,
    },
    Registered {
        name: party_history::NAME,
        load: free,
        bind: parties,
    },
    Registered {
        name: construction_log::emit::NAME,
        load: free,
        bind: construction,
    },
    Registered {
        name: "research",
        load: free,
        bind: research,
    },
    Registered {
        name: "peace",
        load: free,
        bind: peace,
    },
    Registered {
        name: lend_lease_balance::FAMILY,
        load: free,
        bind: lend_lease,
    },
    Registered {
        name: fuel_branches::FAMILY,
        load: free,
        bind: consumers,
    },
    Registered {
        name: "air",
        load: |content| content.air().map(drop),
        bind: air,
    },
    Registered {
        name: "land",
        load: |content| content.land().map(drop),
        bind: land,
    },
    Registered {
        name: market::FAMILY,
        load: free,
        bind: markets,
    },
    Registered {
        name: "experience",
        load: free,
        bind: experience,
    },
    Registered {
        name: "faction_state",
        load: |content| content.factions().map(drop),
        bind: faction_states,
    },
    Registered {
        name: <doctrine_mastery::DoctrineMasteryObserver as dashboard_keyed::Observer>::NAME,
        load: |content| {
            content
                .keyed::<doctrine_mastery::DoctrineMasteryObserver>()
                .map(drop)
        },
        bind: observed::<doctrine_mastery::DoctrineMasteryObserver>,
    },
    Registered {
        name: <occupation_reserve::OccupationReserveObserver as dashboard_keyed::Observer>::NAME,
        load: |content| {
            content
                .keyed::<occupation_reserve::OccupationReserveObserver>()
                .map(drop)
        },
        bind: observed::<occupation_reserve::OccupationReserveObserver>,
    },
    Registered {
        name: <intel_coverage::IntelCoverageObserver as dashboard_keyed::Observer>::NAME,
        load: |content| {
            content
                .keyed::<intel_coverage::IntelCoverageObserver>()
                .map(drop)
        },
        bind: observed::<intel_coverage::IntelCoverageObserver>,
    },
    Registered {
        name: "intelligence",
        load: |content| content.intelligence().map(drop),
        bind: intel,
    },
    Registered {
        name: "autonomy",
        load: free,
        bind: autonomy,
    },
    Registered {
        name: production_catalogue::FAMILY,
        load: free,
        bind: catalogue,
    },
    Registered {
        name: "production-lines",
        load: |content| content.production_icons().map(drop),
        bind: lines,
    },
    Registered {
        name: resource_balance::NAME,
        load: free,
        bind: balance,
    },
    Registered {
        name: "trade",
        load: free,
        bind: trade,
    },
    Registered {
        name: "factories",
        load: free,
        bind: factories,
    },
    Registered {
        name: "focus",
        load: |content| content.factions().map(drop),
        bind: focus,
    },
    Registered {
        name: ideas::NAME,
        load: free,
        bind: holdings,
    },
    Registered {
        name: "projects",
        load: |content| content.projects().map(drop),
        bind: projects,
    },
    Registered {
        name: <regional_weather::block::Observer as dashboard_keyed::Observer>::NAME,
        load: |content| {
            content
                .keyed::<regional_weather::block::Observer>()
                .map(drop)
        },
        bind: observed::<regional_weather::block::Observer>,
    },
    Registered {
        name: "occupation",
        load: |content| content.occupation().map(drop),
        bind: occupied,
    },
    Registered {
        name: "parts",
        load: free,
        bind: parts,
    },
    Registered {
        name: "air_ledger",
        load: free,
        bind: ledger_air,
    },
    Registered {
        name: <air_missions::AirMissionsObserver as dashboard_keyed::Observer>::NAME,
        load: |content| {
            content
                .keyed::<air_missions::AirMissionsObserver>()
                .map(drop)
        },
        bind: observed::<air_missions::AirMissionsObserver>,
    },
    Registered {
        name: air_statistics::NAME,
        load: free,
        bind: air_statistics_family,
    },
    Registered {
        name: "raids",
        load: |content| content.raids().map(drop),
        bind: raided,
    },
    Registered {
        name: "aces",
        load: free,
        bind: aces,
    },
    Registered {
        name: "air_superiority",
        load: free,
        bind: superiority,
    },
    Registered {
        name: "casualties",
        load: |content| content.casualties().map(drop),
        bind: casualties,
    },
    Registered {
        name: "divisions",
        load: |content| content.divisions().map(drop),
        bind: divisions,
    },
    Registered {
        name: "navy",
        load: |content| content.navy().map(drop),
        bind: fleets,
    },
    Registered {
        name: "repairs",
        load: free,
        bind: dockyards,
    },
    Registered {
        name: "convoys",
        load: free,
        bind: convoys,
    },
    Registered {
        name: <minefields::MinefieldsObserver as dashboard_keyed::Observer>::NAME,
        load: |content| content.keyed::<minefields::MinefieldsObserver>().map(drop),
        bind: observed::<minefields::MinefieldsObserver>,
    },
    Registered {
        name: <naval_supremacy::NavalSupremacyObserver as dashboard_keyed::Observer>::NAME,
        load: |content| {
            content
                .keyed::<naval_supremacy::NavalSupremacyObserver>()
                .map(drop)
        },
        bind: observed::<naval_supremacy::NavalSupremacyObserver>,
    },
    Registered {
        name: land_owners::NAME,
        load: |content| content.names().map(drop),
        bind: catalogue_owners,
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

fn convoys<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Convoys {
        replay: content.source.replay_ref(),
    };
    bound(id, native, stage, taken)
}

fn dockyards<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Dockyards {
        replay: content.source.replay_ref(),
    };
    bound(id, native, stage, taken)
}

fn groups<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Groups {
        source: content.source,
        context: content.context,
        extents: &content.extents,
    };
    bound(id, native, stage, taken)
}

fn bound<'p, N: Native + 'p>(
    id: FamilyId,
    native: N,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    Ok(Box::new(Bound::new(id, native, stage, taken)))
}

impl Projected for unit_leaders::columnar::Pushed {
    fn hour(&self) -> i32 {
        unit_leaders::columnar::Pushed::hour(self)
    }
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
    diplomacy_state::facts::Hour,
    equipment_losses::columnar::Hour,
    event_feed::Hour,
    supply_health::facts::Hour,
    stability::facts::Hour,
    division_equipment::facts::Hour,
    politics_ledger::facts::Hour,
    equipment_stockpile::columnar::Hour,
    style_runs::facts::Hour,
    place_names::facts::Hour,
    country_people::facts::Hour,
    country_nukes::facts::Hour,
    naval_battles::columnar::Hour,
    politics_state::facts::Hour,
    crate::products::wars::facts::Hour,
    country_balances::facts::Hour,
    territory::facts::Hour,
    game_score::facts::Hour,
    world_tension::facts::Hour,
    production::Hour,
    subjects::facts::Hour,
    party_history::facts::Hour,
    construction_log::observed::Hour,
    research::facts::Hour,
    peace_deals::facts::Hour,
    lend_lease_balance::facts::Hour,
    fuel_branches::facts::Hour,
    air_force::facts::Hour,
    land_battles::facts::Hour,
    market::facts::Hour,
    experience::facts::Hour,
    faction_state::columnar::Hour,
    intelligence_prepared::facts::Hour,
    subject_autonomy::facts::Hour,
    production_catalogue::facts::Hour,
    production_lines::Hour,
    resource_balance::facts::Hour,
    trade_flows::facts::Hour,
    country_factories::facts::Hour,
    national_focus::facts::Hour,
    ideas::facts::Hour,
    special_projects::facts::Hour,
    occupied_states::columnar::Hour,
    stability_parts::facts::Hour,
    military_air_ledger::facts::Hour,
    crate::products::war_casualties::facts::Hour,
    crate::products::division_log::columnar::Hour,
    navy::facts::Hour,
    land_owners::Hour,
    air_aces::facts::Hour,
    air_superiority::columnar::Hour,
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
    diplomacy_state::emit::Emit => diplomacy_state::facts::Hour,
    place_names::emit::Emit => place_names::facts::Hour,
    country_people::emit::Emit => country_people::facts::Hour,
    country_nukes::emit::Emit => country_nukes::facts::Hour,
    politics_state::emit::Emit => politics_state::facts::Hour,
    crate::products::wars::emit::Emit => crate::products::wars::facts::Hour,
    country_balances::emit::Emit => country_balances::facts::Hour,
    territory::emit::Emit => territory::facts::Hour,
    game_score::emit::Emit => game_score::facts::Hour,
    world_tension::emit::Emit => world_tension::facts::Hour,
    subjects::emit::Emit => subjects::facts::Hour,
    party_history::emit::Emit => party_history::facts::Hour,
    construction_log::emit::Emit => construction_log::observed::Hour,
    research::emit::Emit => research::facts::Hour,
    peace_deals::emit::Emit => peace_deals::facts::Hour,
    lend_lease_balance::emit::Emit => lend_lease_balance::facts::Hour,
    fuel_branches::emit::Emit => fuel_branches::facts::Hour,
    air_force::emit::Emit => air_force::facts::Hour,
    market::emit::Emit => market::facts::Hour,
    experience::emit::Emit => experience::facts::Hour,
    faction_state::emit::Emit => faction_state::columnar::Hour,
    subject_autonomy::emit::Emit => subject_autonomy::facts::Hour,
    production_catalogue::emit::Emit => production_catalogue::facts::Hour,
    resource_balance::emit::Emit => resource_balance::facts::Hour,
    trade_flows::emit::Emit => trade_flows::facts::Hour,
    country_factories::emit::Emit => country_factories::facts::Hour,
    ideas::emit::Emit => ideas::facts::Hour,
    special_projects::emit::Emit => special_projects::facts::Hour,
    occupied_states::columnar::Emit => occupied_states::columnar::Hour,
    stability_parts::emit::Emit => stability_parts::facts::Hour,
    military_air_ledger::emit::Emit => military_air_ledger::facts::Hour,
    navy::emit::Emit => navy::facts::Hour,
    air_statistics::emit::Emit => keyed::Hour<air_statistics::segment::Segments>,
);

// Its hours are kept with the recording their completions are presented from ([`Focus`]).
impl Emits for national_focus::emit::Emit {
    type Hour = std::convert::Infallible;

    fn hour(&mut self, _: &mut roots::Series, hour: Self::Hour) -> io::Result<()> {
        match hour {}
    }

    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()> {
        national_focus::emit::Emit::cut(self, series, last)
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        national_focus::emit::Emit::save(self, out)
    }
}

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
        let series = stage.series(branch, self.first).map_err(backend::alone)?;
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
}

fn diplomacy<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    bound(id, Diplomacy { format }, stage, taken)
}

impl Native for Diplomacy {
    type Operator = diplomacy_state::facts::Projector;
    type Branch = Series<Box<diplomacy_state::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        diplomacy_state::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        diplomacy_state::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        diplomacy_state::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = diplomacy_state::emit::Emit::new();
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: diplomacy_state::facts::Hour,
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
        let directory = directory(stage, branch, published, diplomacy_state::emit::SCHEMA)?;
        let emit = diplomacy_state::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Losses<'p> {
    replay: &'p Replay,
    extents: &'p Extents,
    /// The most any branch's queued steps kept at once.
    queued: AtomicU64,
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
        queued: AtomicU64::new(0),
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
        state.observe_projected(hour)?;
        let peak = u64::try_from(state.queued_peak()).unwrap_or(u64::MAX);
        self.queued.fetch_max(peak, Ordering::Relaxed);
        Ok(())
    }

    fn counts(&self) -> Vec<(&'static str, u64)> {
        vec![(
            "bytes queued steps kept at most",
            self.queued.load(Ordering::Relaxed),
        )]
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

    fn resumes(&self) -> bool {
        true
    }

    // Its leaves are its own sums of the losses it read: each restarts after the lost hours.
    fn gap(&self, state: &mut Self::Branch, _: i32, last: i32) -> io::Result<()> {
        state.lost(last)
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
    type Branch = unit_leaders::prepared::Leading;

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
        Ok(unit_leaders::prepared::Leading::starting(
            self.replay.format,
            branch,
            start,
        ))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        _: u64,
        hour: unit_leaders::columnar::Pushed,
    ) -> io::Result<()> {
        state.observe(hour)
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
        state.saved_catalogue()
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
        unit_leaders::prepared::Leading::resumed(range, saved)
    }
}

struct Feed<'p> {
    replay: &'p Replay,
    source: &'p crate::Source,
    /// Each branch's first and last hours, gaps included.
    bounds: BTreeMap<u64, (i32, i32)>,
    /// The recorded content items are presented with; none without the game's content.
    catalogue: Option<Arc<Catalogue>>,
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
        source: content.source,
        bounds: bounds
            .map(|(branch, extent)| (branch, extent.bounds))
            .collect(),
        catalogue: content.factions()?.clone(),
    };
    bound(id, native, stage, taken)
}

impl<'p> Native for Feed<'p> {
    type Operator = event_feed::Projector<'p>;
    type Branch = Box<event_feed::Prepared>;

    fn tables(&self) -> &'static [TableId] {
        event_feed::tables()
    }

    fn needs(&self) -> io::Result<Needs> {
        event_feed::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        event_feed::Projector::within(
            self.replay,
            self.bounds.clone(),
            self.catalogue.clone(),
            Some(self.source),
        )
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
            self.catalogue.clone(),
        )))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        _: u64,
        hour: event_feed::Hour,
    ) -> io::Result<()> {
        state.observe_projected(hour, Some(self.source))
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

impl<S: Segmented> Projected for keyed::Hour<S> {
    fn hour(&self) -> i32 {
        self.hour
    }
}

impl<V: View> Emits for yards::emit::Emit<V> {
    type Hour = keyed::Hour<V>;

    fn hour(&mut self, series: &mut roots::Series, hour: keyed::Hour<V>) -> io::Result<()> {
        yards::emit::Emit::hour(self, series, hour)
    }

    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()> {
        yards::emit::Emit::cut(self, series, last)
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        yards::emit::Emit::save(self, out)
    }
}

/// A yard view: its operators read its hours a segment at a time ([`keyed`]), and its emit
/// writes them into the series its stage opens for the batch.
struct Yard<V> {
    format: FormatVersion,
    view: PhantomData<fn() -> V>,
}

fn yard<'p, V: View>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    let native = Yard::<V> {
        format: replay.format,
        view: PhantomData,
    };
    bound(id, native, stage, taken)
}

impl<V: View> Native for Yard<V> {
    type Operator = keyed::Operator<V>;
    type Branch = Series<Box<yards::emit::Emit<V>>>;

    fn tables(&self) -> &'static [TableId] {
        V::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        keyed::needs::<V>()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        keyed::Operator::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(
            Box::new(yards::emit::Emit::new(self.format)),
            first,
        ))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: keyed::Hour<V>,
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
        let directory = directory(stage, branch, published, yards::layout::SCHEMA)?;
        let emit = yards::emit::Emit::load(self.format, &directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

/// Air statistics: their operators observe their hours a segment at a time
/// ([`air_statistics::segment`]), handing on what each observation moved, and each branch's emit
/// orders those and writes them into the series its stage opens for the batch.
struct AirStatistics {
    format: FormatVersion,
}

fn air_statistics_family<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    bound(id, AirStatistics { format }, stage, taken)
}

impl Native for AirStatistics {
    type Operator = keyed::Operator<air_statistics::segment::Segments>;
    type Branch = Series<Box<air_statistics::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        air_statistics::segment::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        keyed::needs::<air_statistics::segment::Segments>()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        keyed::Operator::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(air_statistics::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: keyed::Hour<air_statistics::segment::Segments>,
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

    fn resumes(&self) -> bool {
        true
    }

    // What it shows are the game's own values, read anew after the lost hours: nothing to
    // restart, and its emit closes what it held at the first hour after them.
    fn gap(&self, _: &mut Self::Branch, _: i32, _: i32) -> io::Result<()> {
        Ok(())
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
        let directory = directory(stage, branch, published, air_statistics::emit::SCHEMA)?;
        let emit = air_statistics::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

/// Raids: their operators read their hours a segment at a time ([`keyed`]), and each branch's
/// emit gives them to its one scan, writing them into the series its stage opens for the batch.
struct Raided {
    format: FormatVersion,
    content: Option<Arc<raids::Content>>,
}

fn raided<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Raided {
        format: content.source.replay_ref().format,
        content: content.raids()?.clone(),
    };
    bound(id, native, stage, taken)
}

impl Native for Raided {
    type Operator = keyed::Operator<raids::segment::Segments>;
    type Branch = Series<Box<raids::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        raids::segment::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        keyed::needs::<raids::segment::Segments>()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        keyed::Operator::new(self.format)
    }

    fn branch(&self, branch: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = raids::emit::Emit::new(self.format, branch, self.content.clone());
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: keyed::Hour<raids::segment::Segments>,
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
        let directory = directory(stage, branch, published, raids::emit::SCHEMA)?;
        let emit = raids::emit::Emit::load(
            self.format,
            branch,
            self.content.clone(),
            &directory,
            saved,
        )?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

impl Emits for raids::emit::Emit {
    type Hour = keyed::Hour<raids::segment::Segments>;

    fn hour(&mut self, series: &mut roots::Series, hour: Self::Hour) -> io::Result<()> {
        raids::emit::Emit::hour(self, series, hour)
    }

    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()> {
        raids::emit::Emit::cut(self, series, last)
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        raids::emit::Emit::save(self, out)
    }
}

struct Names;

fn names<'p>(id: FamilyId, _: &'p Content<'p>, stage: Stage<'p, 'p>, taken: Taken<'p>) -> Made<'p> {
    bound(id, Names, stage, taken)
}

impl Native for Names {
    type Operator = place_names::facts::Projector;
    type Branch = Series<place_names::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        place_names::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        place_names::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        place_names::facts::Projector::new()
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(place_names::emit::Emit::new(), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: place_names::facts::Hour,
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
        let directory = directory(stage, branch, published, place_names::emit::SCHEMA)?;
        let emit = place_names::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(emit, published.frontier.first_hour))
    }
}

struct Manpower {
    format: u16,
}

fn manpower<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format.version();
    bound(id, Manpower { format }, stage, taken)
}

impl Native for Manpower {
    type Operator = country_people::facts::Projector;
    type Branch = Series<country_people::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        country_people::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(country_people::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        country_people::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(country_people::emit::Emit::new(), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: country_people::facts::Hour,
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
        let directory = directory(stage, branch, published, country_people::emit::SCHEMA)?;
        let emit = country_people::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(emit, published.frontier.first_hour))
    }
}

struct Nukes<'p> {
    replay: &'p Replay,
}

fn nukes<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, Nukes { replay }, stage, taken)
}

impl Native for Nukes<'_> {
    type Operator = country_nukes::facts::Projector;
    type Branch = Series<Box<country_nukes::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        country_nukes::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(country_nukes::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        country_nukes::facts::Projector::new(self.replay.format, self.replay.capture_mode)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = country_nukes::emit::Emit::new(self.replay.format);
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: country_nukes::facts::Hour,
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
        let directory = directory(stage, branch, published, country_nukes::emit::SCHEMA)?;
        let emit = country_nukes::emit::Emit::load(self.replay.format, &directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Naval<'p> {
    source: &'p crate::Source,
    context: &'p crate::config::LoadContext,
    content: &'p ships_sunk::PreparedContent,
}

fn naval<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Naval {
        source: content.source,
        context: content.context,
        content: content.naval()?,
    };
    bound(id, native, stage, taken)
}

impl<'p> Native for Naval<'p> {
    type Operator = naval_battles::columnar::Projector<'p>;
    type Branch = naval_battles::columnar::Branch;

    fn tables(&self) -> &'static [TableId] {
        naval_battles::columnar::tables()
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(naval_battles::columnar::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        let replay = self.source.replay_ref();
        naval_battles::columnar::Projector::new(
            replay.format,
            replay.capture_mode,
            self.source,
            self.context,
            self.content,
        )
    }

    fn branch(&self, branch: u64, first: i32) -> io::Result<Self::Branch> {
        naval_battles::columnar::Branch::new(self.source, branch, first, self.content)
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: naval_battles::columnar::Hour,
    ) -> io::Result<()> {
        // An hour's observation runs whole; the run's cancellation is seen between hours.
        let running = AtomicBool::new(false);
        state.observe(stage.pages()?, hour, self.source, &running)?;
        let first = state.first();
        state.append(stage.series(branch, first)?)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if state.observed() {
            let first = state.first();
            state.cut(stage.series(branch, first)?, cancelled)?;
        }
        match through {
            Some(_) => state.finish(self.source.replay_ref(), stage.pages()?, cancelled),
            None => Ok(()),
        }
    }

    fn resumes(&self) -> bool {
        true
    }

    // Its naval losses are its own tallies of the sinkings it read: pending ones are dropped,
    // and each tally counts only what is read after the lost hours.
    fn gap(&self, state: &mut Self::Branch, _: i32, _: i32) -> io::Result<()> {
        state.lost()
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        let mut out = Vec::new();
        state.save(&mut out)?;
        Ok(Some(out))
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let schema = crate::products::naval_losses::prepared::emit::SCHEMA;
        let directory = directory(stage, branch, published, schema)?;
        naval_battles::columnar::Branch::resume(
            self.source,
            branch,
            published.frontier.first_hour,
            self.content,
            &directory,
            saved,
        )
    }
}

struct PoliticsState;

fn politics<'p>(
    id: FamilyId,
    _: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    bound(id, PoliticsState, stage, taken)
}

impl Native for PoliticsState {
    type Operator = politics_state::facts::Projector;
    type Branch = Series<Box<politics_state::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        politics_state::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(politics_state::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        politics_state::facts::Projector::new()
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(politics_state::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: politics_state::facts::Hour,
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
        let directory = directory(stage, branch, published, politics_state::emit::SCHEMA)?;
        let emit = politics_state::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Wars {
    format: FormatVersion,
    /// The recorded content wars and countries are named with; none without the game's content.
    catalogue: Option<Arc<Catalogue>>,
}

fn wars<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    let catalogue = content.factions()?.clone();
    bound(id, Wars { format, catalogue }, stage, taken)
}

impl Native for Wars {
    type Operator = crate::products::wars::facts::Projector;
    type Branch = Series<Box<crate::products::wars::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        crate::products::wars::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        crate::products::wars::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        crate::products::wars::facts::Projector::new(self.format, self.catalogue.clone())
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = crate::products::wars::emit::Emit::new();
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: crate::products::wars::facts::Hour,
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
        let schema = crate::products::wars::emit::SCHEMA;
        let directory = directory(stage, branch, published, schema)?;
        let emit = crate::products::wars::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Balances {
    format: u16,
}

fn balances<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format.version();
    bound(id, Balances { format }, stage, taken)
}

impl Native for Balances {
    type Operator = country_balances::facts::Projector;
    type Branch = Series<country_balances::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        country_balances::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(country_balances::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        country_balances::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(country_balances::emit::Emit::new(), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: country_balances::facts::Hour,
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
        let directory = directory(stage, branch, published, country_balances::emit::SCHEMA)?;
        let emit = country_balances::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(emit, published.frontier.first_hour))
    }
}

struct Territory<'p> {
    format: FormatVersion,
    configured: &'p territory::Configured,
}

fn territory<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Territory {
        format: content.source.replay_ref().format,
        configured: content.territory()?,
    };
    bound(id, native, stage, taken)
}

impl Native for Territory<'_> {
    type Operator = territory::facts::Projector;
    type Branch = Series<Box<territory::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        territory::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(territory::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        territory::facts::Projector::new(
            self.format,
            self.configured.areas.clone(),
            self.configured.regions.clone(),
        )
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = territory::emit::Emit::new(Arc::clone(&self.configured.content));
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: territory::facts::Hour,
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
        let directory = directory(stage, branch, published, territory::emit::SCHEMA)?;
        let content = Arc::clone(&self.configured.content);
        let emit = territory::emit::Emit::load(content, &directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Score<'p> {
    format: FormatVersion,
    content: &'p game_score::ScoreContent,
}

fn score<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    let content = content.score()?;
    bound(id, Score { format, content }, stage, taken)
}

impl<'p> Native for Score<'p> {
    type Operator = game_score::facts::Projector<'p>;
    type Branch = Series<Box<game_score::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        game_score::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        game_score::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        game_score::facts::Projector::new(self.content, self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = game_score::emit::Emit::new(self.content);
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: game_score::facts::Hour,
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
        let directory = directory(stage, branch, published, game_score::emit::SCHEMA)?;
        let emit = game_score::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Tension<'p> {
    replay: &'p Replay,
}

fn tension<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, Tension { replay }, stage, taken)
}

impl Native for Tension<'_> {
    type Operator = world_tension::facts::Projector;
    type Branch = Series<Box<world_tension::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        world_tension::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(world_tension::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        world_tension::facts::Projector::new(self.replay.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(world_tension::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: world_tension::facts::Hour,
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
        let directory = directory(stage, branch, published, world_tension::emit::SCHEMA)?;
        let emit = world_tension::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Production {
    format: FormatVersion,
}

fn production<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    bound(id, Production { format }, stage, taken)
}

// Its branch state is the row walk's accumulator, which writes the sections production's
// readers read.
impl Native for Production {
    type Operator = production::Projector;
    type Branch = Box<production::Branch>;

    fn tables(&self) -> &'static [TableId] {
        production::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(production::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        production::Projector::new(self.format)
    }

    fn branch(&self, branch: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Box::new(production::Branch::new(self.format, branch, first)))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        _: u64,
        hour: production::Hour,
    ) -> io::Result<()> {
        state.observe(hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage.pages()?, cancelled)
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        saved(state.accumulator())
    }

    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        _: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let accumulator: ProductionAccumulator = taken_up(saved)?;
        Ok(Box::new(production::Branch::resumed(Box::new(accumulator))))
    }
}

struct Relations {
    format: FormatVersion,
}

fn relations<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    bound(id, Relations { format }, stage, taken)
}

impl Native for Relations {
    type Operator = subjects::facts::Projector;
    type Branch = Series<Box<subjects::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        subjects::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(subjects::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        subjects::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(subjects::emit::Emit::new(first)), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: subjects::facts::Hour,
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
        let directory = directory(stage, branch, published, subjects::emit::SCHEMA)?;
        let emit = subjects::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Parties<'p> {
    replay: &'p Replay,
}

fn parties<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, Parties { replay }, stage, taken)
}

impl Native for Parties<'_> {
    type Operator = party_history::facts::Projector;
    type Branch = Series<Box<party_history::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        party_history::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(party_history::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        party_history::facts::Projector::new(self.replay.format, self.replay.capture_mode)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = party_history::emit::Emit::new();
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: party_history::facts::Hour,
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
        let directory = directory(stage, branch, published, party_history::emit::SCHEMA)?;
        let emit = party_history::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct ConstructionQueues<'p> {
    source: &'p dashboard_base::Source,
}

fn construction<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let source = content.source;
    bound(id, ConstructionQueues { source }, stage, taken)
}

impl Native for ConstructionQueues<'_> {
    type Operator = construction_log::facts::Projector;
    type Branch = Series<Box<construction_log::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        construction_log::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(construction_log::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        construction_log::facts::Projector::new(self.source.replay_ref().format)
    }

    fn branch(&self, branch: u64, first: i32) -> io::Result<Self::Branch> {
        let windows = self.source.block_windows(branch).to_vec();
        let emit = construction_log::emit::Emit::new(windows);
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: construction_log::observed::Hour,
    ) -> io::Result<()> {
        state.observe(stage, branch, hour.hour, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        through: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        // The branch's last window ends at its last hour walked.
        if through.is_some()
            && let Some(last) = state.last
        {
            let series = stage.series(branch, state.first)?;
            state.emit.end(series, last)?;
        }
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
        let schema = construction_log::emit::SCHEMA;
        let directory = directory(stage, branch, published, schema)?;
        let windows = self.source.block_windows(branch).to_vec();
        let emit = construction_log::emit::Emit::load(windows, &directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Research<'p> {
    replay: &'p Replay,
}

fn research<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, Research { replay }, stage, taken)
}

impl Native for Research<'_> {
    type Operator = research::facts::Projector;
    type Branch = Series<Box<research::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        research::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(research::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        research::facts::Projector::new(self.replay.format, self.replay.capture_mode)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(research::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: research::facts::Hour,
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
        let directory = directory(stage, branch, published, research::emit::SCHEMA)?;
        let emit = research::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Peace<'p> {
    replay: &'p Replay,
}

fn peace<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, Peace { replay }, stage, taken)
}

impl Native for Peace<'_> {
    type Operator = peace_deals::facts::Projector;
    type Branch = Series<Box<peace_deals::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        peace_deals::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        peace_deals::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        peace_deals::facts::Projector::new(self.replay.format)
    }

    fn branch(&self, branch: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = peace_deals::emit::Emit::new(self.replay, branch)?;
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: peace_deals::facts::Hour,
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
        let directory = directory(stage, branch, published, peace_deals::emit::SCHEMA)?;
        let emit = peace_deals::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct LendLease<'p> {
    replay: &'p Replay,
}

fn lend_lease<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, LendLease { replay }, stage, taken)
}

impl Native for LendLease<'_> {
    type Operator = lend_lease_balance::facts::Projector;
    type Branch = Series<lend_lease_balance::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        lend_lease_balance::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(lend_lease_balance::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        lend_lease_balance::facts::Projector::new(self.replay.format, self.replay.capture_mode)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(lend_lease_balance::emit::Emit::new(), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: lend_lease_balance::facts::Hour,
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
        let directory = directory(stage, branch, published, lend_lease_balance::emit::SCHEMA)?;
        let emit = lend_lease_balance::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(emit, published.frontier.first_hour))
    }
}

struct FuelConsumers<'p> {
    replay: &'p Replay,
}

fn consumers<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, FuelConsumers { replay }, stage, taken)
}

impl Native for FuelConsumers<'_> {
    type Operator = fuel_branches::facts::Projector;
    type Branch = Series<fuel_branches::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        fuel_branches::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(fuel_branches::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        Ok(fuel_branches::facts::Projector::new(
            self.replay.format,
            self.replay.capture_mode,
        ))
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(fuel_branches::emit::Emit::new(), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: fuel_branches::facts::Hour,
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
        let directory = directory(stage, branch, published, fuel_branches::emit::SCHEMA)?;
        let emit = fuel_branches::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(emit, published.frontier.first_hour))
    }
}

struct Air<'p> {
    replay: &'p Replay,
    units: Option<Arc<installed_content_core::equipment::Database>>,
}

fn air<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Air {
        replay: content.source.replay_ref(),
        units: content.air()?.clone(),
    };
    bound(id, native, stage, taken)
}

impl Native for Air<'_> {
    type Operator = air_force::facts::Projector;
    type Branch = Series<Box<air_force::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        air_force::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(air_force::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        air_force::facts::Projector::new(self.replay)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = air_force::emit::Emit::new(self.units.clone());
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: air_force::facts::Hour,
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
        let directory = directory(stage, branch, published, air_force::emit::SCHEMA)?;
        let emit = air_force::emit::Emit::load(self.units.clone(), &directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Land<'p> {
    replay: &'p Replay,
    /// The catalogue designs are named from, and why it is missing.
    content: &'p super::content::Land,
}

fn land<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Land {
        replay: content.source.replay_ref(),
        content: content.land()?,
    };
    bound(id, native, stage, taken)
}

impl Native for Land<'_> {
    type Operator = land_battles::facts::Projector;
    type Branch = Box<land_battles::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        land_battles::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(land_battles::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        land_battles::facts::Projector::new(self.replay.format)
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        let (catalogue, warning) = self.content.clone();
        Ok(Box::new(land_battles::emit::Emit::new(
            self.replay,
            branch,
            catalogue,
            warning,
        )))
    }

    // A block is published as the row walk publishes it: once its hours are kept.
    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        hour: land_battles::facts::Hour,
    ) -> io::Result<()> {
        let kept = AtomicBool::new(false);
        let at = hour.hour;
        if state.due(at) {
            state.publish(stage.pages()?, at, &kept)?;
        }
        state.observe(hour)?;
        let after = at.saturating_add(1);
        if state.due(after) {
            state.publish(stage.pages()?, after, &kept)?;
        }
        Ok(())
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        // Every block left is published once the branch ends.
        if through.is_some() {
            state.publish(stage.pages()?, i32::MAX, cancelled)?;
        }
        Ok(())
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save().map(Some)
    }

    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let emit = land_battles::emit::Emit::load(
            self.replay,
            branch,
            self.content.0.clone(),
            self.replay.format,
            saved,
        )?;
        Ok(Box::new(emit))
    }
}

struct MarketFamily<'p> {
    replay: &'p Replay,
    blocks: Arc<BTreeMap<u64, Arc<[i32]>>>,
}

fn markets<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    let blocks = content
        .extents
        .iter()
        .map(|(branch, _)| {
            let mut starts: Vec<_> = content
                .source
                .block_windows(branch)
                .iter()
                .map(|window| window.first_hour)
                .collect();
            starts.sort_unstable();
            (branch, Arc::from(starts))
        })
        .collect();
    bound(
        id,
        MarketFamily { replay, blocks: Arc::new(blocks) },
        stage,
        taken,
    )
}

impl Native for MarketFamily<'_> {
    type Operator = market::facts::Projector;
    type Branch = Series<Box<market::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        market::facts::tables()
    }

    fn needs(&self) -> io::Result<Needs> {
        market::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        market::facts::Projector::new(
            self.replay.format,
            self.replay.capture_mode,
            Arc::clone(&self.blocks),
        )
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(market::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: market::facts::Hour,
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
        let directory = directory(stage, branch, published, market::emit::SCHEMA)?;
        let emit = market::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Experience {
    format: FormatVersion,
}

fn experience<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    bound(id, Experience { format }, stage, taken)
}

impl Native for Experience {
    type Operator = experience::facts::Projector;
    type Branch = Series<Box<experience::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        experience::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(experience::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        experience::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(experience::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: experience::facts::Hour,
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
        let directory = directory(stage, branch, published, experience::emit::SCHEMA)?;
        let emit = experience::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct FactionStates<'p> {
    format: FormatVersion,
    catalogue: &'p Option<Arc<Catalogue>>,
    capture: Arc<faction_state::columnar::Capture>,
}

fn faction_states<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    let native = FactionStates {
        format: replay.format,
        catalogue: content.factions()?,
        capture: Arc::new(faction_state::columnar::Capture::new(
            &content.recording,
            replay.format,
            replay.capture_mode,
        )?),
    };
    bound(id, native, stage, taken)
}

impl<'p> Native for FactionStates<'p> {
    type Operator = faction_state::columnar::Projector<'p>;
    type Branch = Series<Box<faction_state::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        faction_state::columnar::tables()
    }

    fn needs(&self) -> io::Result<Needs> {
        faction_state::columnar::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        faction_state::columnar::Projector::new(
            self.format,
            self.catalogue.clone(),
            self.capture.clone(),
        )
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = faction_state::emit::Emit::new();
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: faction_state::columnar::Hour,
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
        let directory = directory(stage, branch, published, faction_state::emit::SCHEMA)?;
        let emit = faction_state::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

impl<O: dashboard_keyed::Observer> Emits for scanned::Emit<O> {
    type Hour = keyed::Hour<Scanned<O>>;

    fn hour(&mut self, series: &mut roots::Series, hour: Self::Hour) -> io::Result<()> {
        scanned::Emit::hour(self, series, hour)
    }

    fn cut(&mut self, series: &mut roots::Series, last: i32) -> io::Result<()> {
        scanned::Emit::cut(self, series, last)
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        scanned::Emit::save(self, out)
    }
}

/// A keyed family published as its scan ([`scanned`]): its operators keep the hours its
/// observer observes, and its emit observes them on the branch's one scan, its observer bound
/// to the recording's content.
struct Observed<O: dashboard_keyed::Observer> {
    content: Arc<O::Content>,
    format: FormatVersion,
    observer: PhantomData<fn() -> O>,
}

fn observed<'p, O: dashboard_keyed::Observer>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Observed::<O> {
        content: content.keyed::<O>()?,
        format: content.source.replay_ref().format,
        observer: PhantomData,
    };
    bound(id, native, stage, taken)
}

impl<O: dashboard_keyed::Observer> Observed<O> {
    /// `emit` with its observer bound to the recording, as the walk binds it.
    fn bound(&self, mut emit: scanned::Emit<O>) -> io::Result<Box<scanned::Emit<O>>> {
        emit.bind(&self.content)?;
        Ok(Box::new(emit))
    }
}

impl<O: dashboard_keyed::Observer> Native for Observed<O> {
    type Operator = keyed::Operator<Scanned<O>>;
    type Branch = Series<Box<scanned::Emit<O>>>;

    fn tables(&self) -> &'static [TableId] {
        O::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        keyed::needs::<Scanned<O>>()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        Scanned::<O>::operator(self.format, &self.content)
    }

    fn branch(&self, branch: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = self.bound(scanned::Emit::new(self.format, branch)?)?;
        Ok(Series::new(emit, first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: keyed::Hour<Scanned<O>>,
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
        let directory = directory(stage, branch, published, scanned::SCHEMA)?;
        let emit = scanned::Emit::load(self.format, branch, &directory, saved)?;
        Ok(Series::new(self.bound(emit)?, published.frontier.first_hour))
    }
}

/// The intelligence observers: each segment's operator reads their tables as they do and hands
/// on what its reads changed, and each branch's emit observes those reads in order, publishing
/// their pages as the walk did.
struct Intel<'p> {
    inputs: intelligence_prepared::emit::Inputs<'p>,
}

fn intel<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let inputs = intelligence_prepared::emit::Inputs {
        source: content.source,
        context: content.context,
        content: content.intelligence()?,
    };
    bound(id, Intel { inputs }, stage, taken)
}

/// The family's stage, as a branch's emit writes its pages into it.
struct Staged<'a, 'p, 's>(&'a mut Stage<'p, 's>);

impl intelligence_prepared::emit::Pages for Staged<'_, '_, '_> {
    fn writer(&mut self) -> io::Result<&mut store::Writer> {
        self.0.pages()
    }
}

impl<'p> Native for Intel<'p> {
    type Operator = intelligence_prepared::facts::Projector<'p>;
    type Branch = Box<intelligence_prepared::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        intelligence_prepared::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        intelligence_prepared::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        intelligence_prepared::facts::Projector::new(self.inputs.content)
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        Ok(Box::new(intelligence_prepared::emit::Emit::new(branch)))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        hour: intelligence_prepared::facts::Hour,
    ) -> io::Result<()> {
        state.observe(self.inputs, hour, &mut Staged(stage))
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(self.inputs, stage.pages()?, through, cancelled)
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        Ok(state.save())
    }

    // The continuation its flush writes, with the next block to finish it saved.
    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let pages = stage.pages()?;
        let mut read = |section: &str| pages.read_at(section, 0);
        intelligence_prepared::emit::Emit::resume(branch, &mut read, saved).map(Box::new)
    }
}

struct SubjectAutonomy<'p> {
    replay: &'p Replay,
}

fn autonomy<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, SubjectAutonomy { replay }, stage, taken)
}

impl Native for SubjectAutonomy<'_> {
    type Operator = subject_autonomy::facts::Projector;
    type Branch = Series<Box<subject_autonomy::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        subject_autonomy::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(subject_autonomy::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        subject_autonomy::facts::Projector::new(self.replay.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(
            Box::new(subject_autonomy::emit::Emit::new()),
            first,
        ))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: subject_autonomy::facts::Hour,
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
        let directory = directory(stage, branch, published, subject_autonomy::emit::SCHEMA)?;
        let emit = subject_autonomy::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct ProductionCatalogue<'p> {
    replay: &'p Replay,
}

fn catalogue<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, ProductionCatalogue { replay }, stage, taken)
}

impl Native for ProductionCatalogue<'_> {
    type Operator = production_catalogue::facts::Projector;
    type Branch = Series<Box<production_catalogue::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        production_catalogue::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(production_catalogue::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        production_catalogue::facts::Projector::new(self.replay.format, self.replay.capture_mode)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = production_catalogue::emit::Emit::new();
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: production_catalogue::facts::Hour,
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
        let schema = production_catalogue::emit::SCHEMA;
        let directory = directory(stage, branch, published, schema)?;
        let emit = production_catalogue::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

/// The production lines job: the fold its row preparation runs, fed what each segment's walk
/// reads of its hours' frames from the native columns.
struct LinesJob<'p> {
    replay: &'p Replay,
    icons: Option<Arc<IconContent>>,
    /// Every country tag the recording names, which the fold prepares a place for.
    tags: Vec<String>,
}

fn lines<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    let icons = content.production_icons()?.clone();
    let tags = content.source.with_metadata(|metadata| {
        metadata
            .countries
            .iter()
            .map(|country| country.tag.clone())
            .collect()
    });
    bound(
        id,
        LinesJob {
            replay,
            icons,
            tags,
        },
        stage,
        taken,
    )
}

impl Native for LinesJob<'_> {
    type Operator = production_lines::Projector;
    type Branch = Box<production_lines::Branch>;

    fn tables(&self) -> &'static [TableId] {
        production_lines::tables(self.icons.is_some())
    }

    fn needs(&self) -> io::Result<Needs> {
        production_lines::needs(self.icons.is_some())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        production_lines::Projector::new(self.replay.format, self.icons.clone())
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        Ok(Box::new(production_lines::Branch::new(
            self.replay,
            branch,
            self.tags.iter().cloned(),
            self.icons.as_deref(),
        )))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        hour: production_lines::Hour,
    ) -> io::Result<()> {
        let pages = stage.pages()?;
        state.observe(pages, hour, self.icons.as_deref())
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.publish(stage.pages()?, self.replay, through, cancelled)
    }

    // Its committed directory and the spellings its icons are chosen by.
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        Ok(Some(state.save()))
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let mut branch = production_lines::Branch::resume(
            stage.pages()?,
            self.replay,
            branch,
            self.icons.as_deref(),
        )?;
        branch.load(saved)?;
        Ok(Box::new(branch))
    }
}

struct Balance {
    format: FormatVersion,
}

fn balance<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    bound(id, Balance { format }, stage, taken)
}

impl Native for Balance {
    type Operator = resource_balance::facts::Projector;
    type Branch = Series<Box<resource_balance::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        resource_balance::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(resource_balance::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        resource_balance::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = resource_balance::emit::Emit::new();
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: resource_balance::facts::Hour,
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
        let directory = directory(stage, branch, published, resource_balance::emit::SCHEMA)?;
        let emit = resource_balance::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Trade {
    format: u16,
}

fn trade<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format.version();
    bound(id, Trade { format }, stage, taken)
}

impl Native for Trade {
    type Operator = trade_flows::facts::Projector;
    type Branch = Series<Box<trade_flows::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        trade_flows::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(trade_flows::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        trade_flows::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(trade_flows::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: trade_flows::facts::Hour,
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
        let directory = directory(stage, branch, published, trade_flows::emit::SCHEMA)?;
        let emit = trade_flows::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Factories<'p> {
    replay: &'p Replay,
}

fn factories<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let replay = content.source.replay_ref();
    bound(id, Factories { replay }, stage, taken)
}

impl Native for Factories<'_> {
    type Operator = country_factories::facts::Projector;
    type Branch = Series<Box<country_factories::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        country_factories::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(country_factories::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        country_factories::facts::Projector::new(self.replay.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = country_factories::emit::Emit::new();
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: country_factories::facts::Hour,
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
        let directory = directory(stage, branch, published, country_factories::emit::SCHEMA)?;
        let emit = country_factories::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Focus<'p> {
    replay: &'p Replay,
    source: &'p crate::Source,
    /// The recorded content completions are presented with; none without the game's content.
    catalogue: Option<Arc<Catalogue>>,
}

fn focus<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Focus {
        replay: content.source.replay_ref(),
        source: content.source,
        catalogue: content.factions()?.clone(),
    };
    bound(id, native, stage, taken)
}

impl Native for Focus<'_> {
    type Operator = national_focus::facts::Projector;
    type Branch = Series<Box<national_focus::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        if self.catalogue.is_some() {
            national_focus::facts::art_tables()
        } else {
            national_focus::facts::TABLES
        }
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(national_focus::facts::needs(self.catalogue.is_some()))
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        Ok(
            national_focus::facts::Projector::new(self.replay.format, self.replay.capture_mode)?
                .with_art(self.catalogue.clone()),
        )
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(national_focus::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: national_focus::facts::Hour,
    ) -> io::Result<()> {
        let at = hour.hour;
        let recorded = self
            .catalogue
            .as_ref()
            .map(|catalogue| national_focus::emit::Recorded {
                source: self.source,
                catalogue,
            });
        let series = stage.series(branch, state.first).map_err(backend::alone)?;
        national_focus::emit::Emit::hour(&mut state.emit, series, hour, recorded.as_ref())?;
        state.last = Some(at);
        Ok(())
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
        let directory = directory(stage, branch, published, national_focus::emit::SCHEMA)?;
        let emit = national_focus::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Holdings;

fn holdings<'p>(
    id: FamilyId,
    _: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    bound(id, Holdings, stage, taken)
}

impl Native for Holdings {
    type Operator = ideas::facts::Projector;
    type Branch = Series<Box<ideas::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        ideas::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        ideas::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        ideas::facts::Projector::new()
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(ideas::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: ideas::facts::Hour,
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
        let directory = directory(stage, branch, published, ideas::emit::SCHEMA)?;
        let emit = ideas::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Projects<'p> {
    format: FormatVersion,
    binding: &'p special_projects::facts::Binding,
    /// Each branch's first recorded hour, and its gaps.
    starts: Arc<BTreeMap<u64, i32>>,
    gaps: BTreeMap<u64, Vec<(i32, i32)>>,
}

fn projects<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let extents = &content.extents;
    let native = Projects {
        format: content.source.replay_ref().format,
        binding: content.projects()?,
        starts: Arc::new(
            extents
                .iter()
                .map(|(branch, extent)| (branch, extent.recorded.0))
                .collect(),
        ),
        gaps: extents
            .iter()
            .map(|(branch, extent)| (branch, extent.gaps.clone()))
            .collect(),
    };
    bound(id, native, stage, taken)
}

impl Projects<'_> {
    /// `branch`'s first recorded hour and its gaps.
    fn extent(&self, branch: u64) -> (i32, Vec<(i32, i32)>) {
        (
            self.starts.get(&branch).copied().unwrap_or_default(),
            self.gaps.get(&branch).cloned().unwrap_or_default(),
        )
    }
}

impl Native for Projects<'_> {
    type Operator = special_projects::facts::Projector;
    type Branch = Series<Box<special_projects::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        special_projects::facts::tables(self.binding.scoped())
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(special_projects::facts::needs(self.binding.scoped()))
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        special_projects::facts::Projector::new(self.format, self.binding, Arc::clone(&self.starts))
    }

    fn branch(&self, branch: u64, first: i32) -> io::Result<Self::Branch> {
        let (start, gaps) = self.extent(branch);
        let emit = special_projects::emit::Emit::new(self.binding.emitted(), start, gaps);
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: special_projects::facts::Hour,
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
        let directory = directory(stage, branch, published, special_projects::emit::SCHEMA)?;
        let (start, gaps) = self.extent(branch);
        let emit = special_projects::emit::Emit::load(
            self.binding.emitted(),
            start,
            gaps,
            &directory,
            saved,
        )?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct Occupied<'p> {
    replay: &'p Replay,
    binding: &'p occupied_states::prepared::Binding,
}

fn occupied<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Occupied {
        replay: content.source.replay_ref(),
        binding: content.occupation()?,
    };
    bound(id, native, stage, taken)
}

impl Native for Occupied<'_> {
    type Operator = occupied_states::columnar::Projector;
    type Branch = Series<Box<occupied_states::columnar::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        self.binding.tables()
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(self.binding.needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        self.binding
            .projector(self.replay.format, self.replay.capture_mode)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(self.binding.emit()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: occupied_states::columnar::Hour,
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
        let schema = occupied_states::columnar::emit::SCHEMA;
        let directory = directory(stage, branch, published, schema)?;
        let emit = self.binding.resume(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct StabilityParts {
    format: u16,
}

fn parts<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format.version();
    bound(id, StabilityParts { format }, stage, taken)
}

impl Native for StabilityParts {
    type Operator = stability_parts::facts::Projector;
    type Branch = Series<Box<stability_parts::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        stability_parts::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(stability_parts::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        stability_parts::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        Ok(Series::new(Box::new(stability_parts::emit::Emit::new()), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: stability_parts::facts::Hour,
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
        let directory = directory(stage, branch, published, stability_parts::emit::SCHEMA)?;
        let emit = stability_parts::emit::Emit::resume(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

struct AirLedger {
    format: FormatVersion,
}

fn ledger_air<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let format = content.source.replay_ref().format;
    bound(id, AirLedger { format }, stage, taken)
}

impl Native for AirLedger {
    type Operator = military_air_ledger::facts::Projector;
    type Branch = Series<Box<military_air_ledger::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        military_air_ledger::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(military_air_ledger::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        military_air_ledger::facts::Projector::new(self.format)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = military_air_ledger::emit::Emit::new();
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: military_air_ledger::facts::Hour,
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
        let directory = directory(stage, branch, published, military_air_ledger::emit::SCHEMA)?;
        let emit = military_air_ledger::emit::Emit::load(&directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

/// Air aces: their operators project each segment's roster changes and named designs, and each
/// branch's emit keeps them in order, publishing each block's window and, at each flush, the
/// roster's continuation, read series and variant catalogue.
struct Aces<'p> {
    source: &'p crate::Source,
    context: &'p crate::config::LoadContext,
    replay: &'p Replay,
}

fn aces<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Aces {
        source: content.source,
        context: content.context,
        replay: content.source.replay_ref(),
    };
    bound(id, native, stage, taken)
}

impl Native for Aces<'_> {
    type Operator = air_aces::facts::Project;
    type Branch = Box<air_aces::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        air_aces::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        air_aces::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        air_aces::facts::Project::new(self.replay.format)
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        Ok(Box::new(air_aces::emit::Emit::new(self.replay, branch)))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        hour: air_aces::facts::Hour,
    ) -> io::Result<()> {
        let cancelled = AtomicBool::new(false);
        state.observe(
            stage.pages()?,
            self.source,
            self.replay,
            self.context,
            hour,
            &cancelled,
        )
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(
            stage.pages()?,
            self.source,
            self.replay,
            self.context,
            through,
            cancelled,
        )
    }

    fn resumes(&self) -> bool {
        true
    }

    // What it shows are the game's own values, read anew after the lost hours: nothing to
    // restart, and its emit closes what it held at the first hour after them.
    fn gap(&self, _: &mut Self::Branch, _: i32, _: i32) -> io::Result<()> {
        Ok(())
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save().map(Some)
    }

    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        air_aces::emit::Emit::load(self.replay, branch, saved).map(Box::new)
    }
}

/// Air superiority: each segment's operator keeps its own read of the groups and hands on the
/// rosters they assign at the ends of days and what moved of the scores, country identity, wings
/// and theatres; each branch's emit folds them, publishing the fold's pages at each flush.
struct Superiority<'p> {
    replay: &'p Replay,
}

fn superiority<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Superiority {
        replay: content.source.replay_ref(),
    };
    bound(id, native, stage, taken)
}

impl Native for Superiority<'_> {
    type Operator = air_superiority::columnar::Project;
    type Branch = Box<air_superiority::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        air_superiority::columnar::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        air_superiority::columnar::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        air_superiority::columnar::Project::new(self.replay.format)
    }

    fn branch(&self, _: u64, _: i32) -> io::Result<Self::Branch> {
        Ok(Box::new(air_superiority::emit::Emit::new()))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        _: u64,
        hour: air_superiority::columnar::Hour,
    ) -> io::Result<()> {
        state.observe(hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage.pages()?, branch, cancelled)
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save().map(Some)
    }

    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        _: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        air_superiority::emit::Emit::load(saved).map(Box::new)
    }
}

/// War casualties: the walk's casualty observer in each branch's emit, fed the hours each
/// segment's operator projects, publishing the pages the walk published.
struct Casualties<'p> {
    replay: &'p Replay,
    content: &'p crate::products::war_casualties::prepared::PreparedContent,
}

fn casualties<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Casualties {
        replay: content.source.replay_ref(),
        content: content.casualties()?,
    };
    bound(id, native, stage, taken)
}

impl Native for Casualties<'_> {
    type Operator = crate::products::war_casualties::facts::Projector;
    type Branch = Box<crate::products::war_casualties::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        crate::products::war_casualties::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        crate::products::war_casualties::facts::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        crate::products::war_casualties::facts::Projector::new(self.replay)
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        let emit =
            crate::products::war_casualties::emit::Emit::new(self.replay, branch, self.content)?;
        Ok(Box::new(emit))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        hour: crate::products::war_casualties::facts::Hour,
    ) -> io::Result<()> {
        let cancelled = AtomicBool::new(false);
        state.observe(stage.pages()?, self.replay, hour, &cancelled)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage.pages()?, through, cancelled)
    }

    fn resumes(&self) -> bool {
        true
    }

    // What it shows are the game's own values, read anew after the lost hours: nothing to
    // restart, and its emit closes what it held at the first hour after them.
    fn gap(&self, _: &mut Self::Branch, _: i32, _: i32) -> io::Result<()> {
        Ok(())
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save().map(Some)
    }

    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let emit = crate::products::war_casualties::emit::Emit::load(
            self.replay,
            branch,
            self.content,
            saved,
        )?;
        Ok(Box::new(emit))
    }
}

/// The division log: the walk's division scan in each branch's emit, fed the hours each
/// segment's projector projects, publishing each block's pages as the walk published them.
struct Divisions<'p> {
    source: &'p crate::Source,
    names: &'p crate::products::division_log::Names,
    replay: &'p Replay,
}

fn divisions<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Divisions {
        source: content.source,
        names: content.divisions()?,
        replay: content.source.replay_ref(),
    };
    bound(id, native, stage, taken)
}

impl Native for Divisions<'_> {
    type Operator = crate::products::division_log::columnar::Projector;
    type Branch = Box<crate::products::division_log::emit::Emit>;

    fn tables(&self) -> &'static [TableId] {
        crate::products::division_log::columnar::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        crate::products::division_log::columnar::needs()
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        crate::products::division_log::columnar::Projector::new(
            self.replay.format,
            self.replay.capture_mode,
        )
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        let names = self.names.clone();
        let emit = crate::products::division_log::emit::Emit::new(self.replay, branch, names);
        Ok(Box::new(emit))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        hour: crate::products::division_log::columnar::Hour,
    ) -> io::Result<()> {
        let cancelled = AtomicBool::new(false);
        state.observe(self.source, stage.pages()?, hour, &cancelled)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        _: u64,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(self.source, stage.pages()?, through, cancelled)
    }

    fn resumes(&self) -> bool {
        true
    }

    // Its counters are its own tallies: each restarts after the lost hours.
    fn gap(&self, state: &mut Self::Branch, first: i32, _: i32) -> io::Result<()> {
        state.lost(first)
    }

    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.save().map(Some)
    }

    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let emit = crate::products::division_log::emit::Emit::load(
            self.replay,
            branch,
            self.replay.format,
            self.names.clone(),
            saved,
        )?;
        Ok(Box::new(emit))
    }
}

struct Fleets<'p> {
    replay: &'p Replay,
    units: Option<Arc<installed_content_core::unit_leader::SubUnits>>,
}

fn fleets<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = Fleets {
        replay: content.source.replay_ref(),
        units: content.navy()?.clone(),
    };
    bound(id, native, stage, taken)
}

impl Native for Fleets<'_> {
    type Operator = navy::facts::Projector;
    type Branch = Series<Box<navy::emit::Emit>>;

    fn tables(&self) -> &'static [TableId] {
        navy::facts::TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(navy::facts::needs())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        navy::facts::Projector::new(self.replay)
    }

    fn branch(&self, _: u64, first: i32) -> io::Result<Self::Branch> {
        let emit = navy::emit::Emit::new(self.units.clone());
        Ok(Series::new(Box::new(emit), first))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: navy::facts::Hour,
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
        let directory = directory(stage, branch, published, navy::emit::SCHEMA)?;
        let emit = navy::emit::Emit::load(self.units.clone(), &directory, saved)?;
        Ok(Series::new(Box::new(emit), published.frontier.first_hour))
    }
}

/// The land-owner catalogue's backend: a recording-wide job, prepared branch by branch.
struct OwnerCatalogue<'p> {
    format: FormatVersion,
    names: &'p super::content::Names,
}

fn catalogue_owners<'p>(
    id: FamilyId,
    content: &'p Content<'p>,
    stage: Stage<'p, 'p>,
    taken: Taken<'p>,
) -> Made<'p> {
    let native = OwnerCatalogue {
        format: content.source.replay_ref().format,
        names: content.names()?,
    };
    bound(id, native, stage, taken)
}

impl Native for OwnerCatalogue<'_> {
    type Operator = land_owners::Projector;
    type Branch = Box<land_owners::Branch>;

    fn tables(&self) -> &'static [TableId] {
        land_owners::tables(self.names.is_ok())
    }

    fn needs(&self) -> io::Result<Needs> {
        land_owners::needs(self.names.is_ok())
    }

    fn operator(&self, _: &Segment) -> io::Result<Self::Operator> {
        land_owners::Projector::new(self.format, self.names.as_ref().ok().cloned())
    }

    fn branch(&self, _: u64, _: i32) -> io::Result<Self::Branch> {
        Ok(Box::new(land_owners::Branch::new()))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        _: u64,
        hour: land_owners::Hour,
    ) -> io::Result<()> {
        state.observe(hour);
        Ok(())
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        _: &AtomicBool,
    ) -> io::Result<()> {
        state.publish(stage.pages()?, branch)
    }

    // Its published page is all it goes on with.
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
        land_owners::Branch::resume(stage.pages()?, branch).map(Box::new)
    }
}
