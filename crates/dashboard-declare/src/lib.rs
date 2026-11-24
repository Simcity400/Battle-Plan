
use replay_columnar::summary::{self, Cells, Computed, Facet, Shape, Source, aggregate};
use replay_data::rows::{FuelDayRow, NukeRow, TableId, column};
use replay_derive::factories;
use std::io;

pub mod budget;

pub use replay_columnar::summary::aggregate::{CHANGES, FALLS, SUM};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Country,
    Slot,
    Holder,
    Singleton,
    Faction,
}

impl Key {
    pub fn of(self, slot: u32, id: u32) -> u128 {
        match self {
            Self::Country => u128::from(id),
            Self::Slot => u128::from(slot),
            Self::Holder => u128::from(slot) << 32 | u128::from(id),
            Self::Singleton | Self::Faction => 0,
        }
    }

    pub fn fixed(self, name: &str) -> Option<u128> {
        match self {
            Self::Singleton => Some(0),
            Self::Faction => name.parse::<u64>().ok().map(u128::from),
            Self::Country | Self::Slot | Self::Holder => None,
        }
    }

    pub fn unowned(self) -> bool {
        matches!(self, Self::Singleton | Self::Faction)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parts {
    pub bits: u32,
    pub id: Option<(u32, u32)>,
}

impl Parts {
    pub fn mask(&self) -> u128 {
        (1u128 << self.bits) - 1
    }

    pub fn key(&self, owner: u128, part: u128) -> u128 {
        owner << self.bits | (part & self.mask())
    }

    pub fn owner(&self, key: u128) -> u128 {
        key >> self.bits
    }

    pub fn part(&self, key: u128) -> u128 {
        key & self.mask()
    }

    pub fn range(&self, owner: u128) -> (u128, u128) {
        (owner << self.bits, owner << self.bits | self.mask())
    }

    pub fn id(&self, part: u128) -> Option<u64> {
        let (shift, width) = self.id?;
        Some(((part >> shift) & ((1u128 << width) - 1)) as u64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carry {
    None,
    Sum,
    Rebase,
    Max,
}

#[derive(Clone, Copy)]
pub enum Column {
    Cell(&'static str),
    Computed {
        id: u32,
        inputs: &'static [&'static str],
        value: fn(&dyn Cells, u32) -> Computed,
    },
}

#[derive(Clone, Copy)]
pub struct Measure {
    pub name: &'static str,
    pub column: Column,
    pub decimals: u8,
    pub aggregates: u8,
    pub carry: Carry,
    pub rank: u8,
}

impl Measure {
    const fn ranked(self, rank: u8) -> Self {
        Self { rank, ..self }
    }

    const fn carried(self, carry: Carry) -> Self {
        Self { carry, ..self }
    }
}

pub struct Series {
    pub kind: &'static str,
    pub table: TableId,
    pub key: Key,
    pub parts: Option<Parts>,
    pub measures: &'static [Measure],
    pub drops: bool,
    pub absent_none: bool,
}

impl Series {
    pub fn measure(&self, name: &str) -> Option<&'static Measure> {
        self.measures.iter().find(|measure| measure.name == name)
    }

    pub fn row_key(&self, slot: u32, id: u32, part: u128) -> u128 {
        let owner = self.key.of(slot, id);
        match self.parts {
            Some(parts) => parts.key(owner, part),
            None => owner,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FacetFrom {
    Column(&'static str),
    Key(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holds {
    Value,
    Owner(Key),
}

#[derive(Clone, Copy, Debug)]
pub struct LogFacet {
    pub name: &'static str,
    pub from: FacetFrom,
    pub holds: Holds,
}

pub struct Log {
    pub name: &'static str,
    pub table: TableId,
    pub facets: &'static [LogFacet],
}

const fn fixed5(name: &'static str, column: &'static str, aggregates: u8) -> Measure {
    Measure {
        name,
        column: Column::Cell(column),
        decimals: 5,
        aggregates,
        carry: Carry::None,
        rank: 0,
    }
}

const fn count(name: &'static str, column: &'static str) -> Measure {
    Measure {
        name,
        column: Column::Cell(column),
        decimals: 0,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }
}

const fn raw(name: &'static str, column: &'static str) -> Measure {
    Measure {
        name,
        column: Column::Cell(column),
        decimals: 0,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }
}

pub static SERIES: &[Series] = &[
    Series {
        kind: "country-measure",
        table: TableId::Countries,
        key: Key::Country,
        parts: None,
        measures: &[
            fixed5("politicalPower", "political_power_raw", aggregate::FALLS | aggregate::CHANGES),
            fixed5("commandPower", "command_power_raw", aggregate::FALLS),
        ],
        drops: true,
        absent_none: false,
    },
    Series {
        kind: "country-stability",
        table: TableId::CountryStability,
        key: Key::Country,
        parts: None,
        measures: &[
            fixed5("stability", "stability", 0),
            fixed5("warSupport", "war_support", 0),
            fixed5("maxCommandPower", "max_command_power", 0),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "stability-parts",
        table: TableId::StabilityParts,
        key: Key::Country,
        parts: Some(Parts { bits: 40, id: None }),
        measures: &[fixed5("value", "value", aggregate::ABS)],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "army-totals",
        table: TableId::ArmyTotals,
        key: Key::Slot,
        parts: None,
        measures: &[
            count("ownerDivisions", "owner_divisions").ranked(32),
            fixed5("ownerStrength", "owner_strength_raw", 0).ranked(32),
            count("hostDivisions", "host_divisions").ranked(32),
            fixed5("hostStrength", "host_strength_raw", 0).ranked(32),
            count("ownerBattalions", "owner_battalions").ranked(32),
            count("hostBattalions", "host_battalions").ranked(32),
            count("ownerManpower", "owner_manpower").ranked(32),
            count("hostManpower", "host_manpower").ranked(32),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "country-resources",
        table: TableId::CountryResources,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: Some((0, 32)) }),
        measures: &[
            fixed5("produced", "produced", 0),
            fixed5("adjustment", "adjustment", 0),
            fixed5("imported", "imported", 0),
            fixed5("toExport", "to_export", 0),
            fixed5("exported", "exported", 0),
            fixed5("used", "used", 0),
            fixed5("specialTotal", "special_total", 0),
            fixed5("productionDemand", "production_demand", 0),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "country-deployed-manpower",
        table: TableId::CountryDeployedManpower,
        key: Key::Slot,
        parts: None,
        measures: &[
            count("army", "army"),
            count("navy", "navy"),
            count("navyRequired", "navy_required"),
            count("air", "air"),
            count("exileAir", "exile_air"),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "supply-health",
        table: TableId::SupplyHealth,
        key: Key::Slot,
        parts: None,
        measures: &[
            count("noSupply", "no_supply"),
            count("strained", "strained"),
            count("inSupply", "in_supply"),
            share("noSupplyShare", 0, no_supply_share),
            share("strainedShare", 1, strained_share),
            share("inSupplyShare", 2, in_supply_share),
            Measure {
                name: "divisions",
                column: Column::Computed { id: 3, inputs: SUPPLY_COUNT_NAMES, value: supply_divisions },
                decimals: 0,
                aggregates: 0,
                carry: Carry::None,
                rank: 0,
            },
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "air-force-totals",
        table: TableId::AirForceTotals,
        key: Key::Slot,
        parts: None,
        measures: &[
            count("rosterPlanes", "roster_planes").ranked(32),
            count("rosterWings", "roster_wings").ranked(32),
            count("effectivePlanes", "effective_planes").ranked(32),
            count("effectiveWings", "effective_wings").ranked(32),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "navy-totals",
        table: TableId::NavyTotals,
        key: Key::Slot,
        parts: None,
        measures: &[
            count("rosterShips", "roster_ships").ranked(32),
            fixed5("rosterStrength", "roster_strength_raw", 0).ranked(32),
            fixed5("rosterMaxStrength", "roster_max_strength_raw", 0),
            count("logicalShips", "logical_ships").ranked(32),
            fixed5("logicalStrength", "logical_strength_raw", 0).ranked(32),
            fixed5("logicalMaxStrength", "logical_max_strength_raw", 0),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "country-experience-daily",
        table: TableId::CountryExperienceDaily,
        key: Key::Country,
        parts: None,
        measures: &[
            raw("armyChange", "army_change"),
            raw("navyChange", "navy_change"),
            raw("airChange", "air_change"),
            raw("armyGain", "army_gain"),
            raw("navyGain", "navy_gain"),
            raw("airGain", "air_gain"),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "country-game-scores",
        table: TableId::CountryGameScores,
        key: Key::Country,
        parts: None,
        measures: &[
            fixed5("army", "army", 0).ranked(32),
            fixed5("navy", "navy", 0).ranked(32),
            fixed5("air", "air", 0).ranked(32),
            fixed5("industry", "industry", 0).ranked(32),
            fixed5("land", "land", 0).ranked(32),
            fixed5("total", "total", 0).ranked(32),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "fuel-days",
        table: TableId::FuelDays,
        key: Key::Country,
        parts: None,
        measures: &[
            raw("stripGain", "gained"),
            raw("stripBurn", "burned"),
            fuel_day("gainBase", 0, fuel_measure::<2>),
            fuel_day("gainStates", 1, fuel_measure::<3>),
            fuel_day("gainOil", 2, fuel_measure::<4>),
            fuel_day("gainNegativeCost", 3, fuel_measure::<5>),
            fuel_day("unidentified", 4, fuel_measure::<6>),
            fuel_day("incoming", 5, fuel_measure::<7>),
            fuel_day("outgoing", 6, fuel_measure::<8>),
            fuel_day("flat", 7, fuel_measure::<9>),
            fuel_day("other", 8, fuel_measure::<10>),
            fuel_day("conversion", 9, fuel_measure::<11>),
            fuel_day("consumerArmy", 10, fuel_measure::<12>),
            fuel_day("consumerAir", 11, fuel_measure::<13>),
            fuel_day("consumerNavy", 12, fuel_measure::<14>),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "fuel-demand-totals",
        table: TableId::FuelDemandTotals,
        key: Key::Country,
        parts: None,
        measures: &[
            fixed5("nativeArmy", "native_army", 0),
            fixed5("nativeAir", "native_air", 0),
            fixed5("nativeNavy", "native_navy", 0),
            fixed5("knownArmy", "known_army", 0),
            fixed5("knownAir", "known_air", 0),
            fixed5("knownNavy", "known_navy", 0),
            fixed5("positiveArmy", "positive_army", 0),
            fixed5("positiveAir", "positive_air", 0),
            fixed5("positiveNavy", "positive_navy", 0),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "production-country-totals",
        table: TableId::ProductionCountryTotals,
        key: Key::Country,
        parts: None,
        measures: &[
            Measure { carry: Carry::Sum, ..count("units", "units") }.ranked(32),
            Measure { carry: Carry::Sum, ..fixed5("spent", "spent", 0) }.ranked(32),
            Measure { carry: Carry::Sum, ..count("unitsWithoutRefits", "units_without_refits") }.ranked(32),
            Measure { carry: Carry::Sum, ..fixed5("spentWithoutRefits", "spent_without_refits", 0) }.ranked(32),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "production-type-totals",
        table: TableId::ProductionTypeTotals,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: Some((0, 32)) }),
        measures: &[
            Measure { carry: Carry::Sum, ..count("units", "units") },
            Measure { carry: Carry::Sum, ..fixed5("spent", "spent", 0) },
            Measure { carry: Carry::Sum, ..count("unitsWithoutRefits", "units_without_refits") },
            Measure { carry: Carry::Sum, ..fixed5("spentWithoutRefits", "spent_without_refits", 0) },
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "casualty-totals",
        table: TableId::CasualtyTotals,
        key: Key::Holder,
        parts: None,
        measures: &[
            count("dealt", "dealt").carried(Carry::Rebase).ranked(32),
            count("taken", "taken").carried(Carry::Rebase).ranked(32),
            count("unknown", "unknown").carried(Carry::Rebase),
            count("partial", "partial").carried(Carry::Max),
            Measure {
                name: "dealtAndTaken",
                column: Column::Computed { id: 0, inputs: &["dealt", "taken"], value: casualties_ranked },
                decimals: 0,
                aggregates: 0,
                carry: Carry::Rebase,
                rank: 32,
            },
        ],
        drops: false,
        absent_none: true,
    },
    Series {
        kind: "occupier-totals",
        table: TableId::OccupierTotals,
        key: Key::Holder,
        parts: None,
        measures: &[
            fixed5("resistanceSum", "resistance_sum", 0),
            fixed5("complianceSum", "compliance_sum", 0),
            count("states", "states"),
            count("heldStates", "held_states"),
            count("heldComplete", "held_complete"),
            Measure {
                name: "resistance",
                column: Column::Computed { id: 0, inputs: &["resistance_sum", "states"], value: occupation_resistance },
                decimals: 5,
                aggregates: 0,
                carry: Carry::None,
                rank: 32,
            },
            Measure {
                name: "compliance",
                column: Column::Computed { id: 1, inputs: &["compliance_sum", "states"], value: occupation_compliance },
                decimals: 5,
                aggregates: 0,
                carry: Carry::None,
                rank: 32,
            },
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "territory",
        table: TableId::TerritoryTotals,
        key: Key::Slot,
        parts: None,
        measures: &[
            count("ownedStates", "owned_states").ranked(32),
            count("controlledStates", "controlled_states").ranked(32),
            count("occupiedStates", "occupied_states").ranked(32),
            count("ownedProvinces", "owned_provinces").ranked(32),
            count("controlledProvinces", "controlled_provinces").ranked(32),
            count("occupiedProvinces", "occupied_provinces").ranked(32),
            count("ownedVictoryPoints", "owned_victory_points").ranked(32),
            count("controlledVictoryPoints", "controlled_victory_points").ranked(32),
            count("occupiedVictoryPoints", "occupied_victory_points").ranked(32),
            count("ownedArea", "owned_area").ranked(32),
            count("controlledArea", "controlled_area").ranked(32),
            count("occupiedArea", "occupied_area").ranked(32),
            Measure {
                name: "holding",
                column: Column::Computed { id: 0, inputs: &["owned_states", "controlled_states"], value: territory_holding },
                decimals: 0,
                aggregates: 0,
                carry: Carry::None,
                rank: 0,
            },
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "country-factories",
        table: TableId::CountryFactories,
        key: Key::Country,
        parts: None,
        measures: &FACTORY_CHANNELS,
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "country-nukes",
        table: TableId::Nukes,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: None }),
        measures: &[
            Measure {
                name: "amount",
                column: Column::Computed { id: 0, inputs: &["amount_raw"], value: nuke_amount },
                decimals: 0,
                aggregates: 0,
                carry: Carry::None,
                rank: 0,
            },
            count("ready", "ready"),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "world-tension",
        table: TableId::WorldTension,
        key: Key::Singleton,
        parts: None,
        measures: &[Measure { name: "tension", column: Column::Cell("total_raw"), decimals: 3, aggregates: 0, carry: Carry::None, rank: 0 }],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "threat-contributions",
        table: TableId::ThreatCountryAggregates,
        key: Key::Slot,
        parts: None,
        measures: &[fixed5("contribution", "total_raw", 0).ranked(32)],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "air-force-parts",
        table: TableId::AirForceParts,
        key: Key::Slot,
        parts: Some(Parts { bits: 73, id: Some((0, 64)) }),
        measures: &[count("planes", "planes")],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "air-force-wing-kinds",
        table: TableId::AirForceWingKinds,
        key: Key::Slot,
        parts: Some(Parts { bits: 40, id: Some((0, 32)) }),
        measures: &[count("wings", "wings")],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "navy-classes",
        table: TableId::NavyClasses,
        key: Key::Slot,
        parts: Some(Parts { bits: 40, id: Some((0, 32)) }),
        measures: &[count("ships", "ships"), fixed5("strength", "strength_raw", 0)],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "military-aid",
        table: TableId::MilitaryAid,
        key: Key::Slot,
        parts: Some(Parts { bits: 40, id: None }),
        measures: &[count("divisions", "divisions")],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "country-experience",
        table: TableId::CountryExperience,
        key: Key::Country,
        parts: None,
        measures: &[raw("army", "army_raw"), raw("navy", "navy_raw"), raw("air", "air_raw")],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "autonomy",
        table: TableId::Autonomy,
        key: Key::Country,
        parts: None,
        measures: &[
            Measure {
                name: "progress",
                column: Column::Computed { id: 0, inputs: &["score", "lower_score", "upper_score"], value: autonomy_progress },
                decimals: 5,
                aggregates: 0,
                carry: Carry::None,
                rank: 0,
            },
            fixed5("score", "score", 0),
            fixed5("lower", "lower_score", 0),
            fixed5("upper", "upper_score", 0),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "relation-opinions",
        table: TableId::RelationOpinions,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: None }),
        measures: &[raw("opinion", "opinion")],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "country-nuke-totals",
        table: TableId::CountryNukeTotals,
        key: Key::Country,
        parts: None,
        measures: &[count("total", "total").ranked(32), count("ready", "ready")],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "fuel-consumers",
        table: TableId::CountryFuelConsumers,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: None }),
        measures: &[
            fixed5("requested", "requested_raw", 0),
            fixed5("received", "received_raw", 0),
            count("priority", "priority"),
            Measure {
                name: "receivedShare",
                column: Column::Computed { id: 0, inputs: &["requested_raw", "received_raw"], value: fuel_received_share },
                decimals: 4,
                aggregates: 0,
                carry: Carry::None,
                rank: 0,
            },
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "resource-balance",
        table: TableId::CountryResources,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: Some((0, 32)) }),
        measures: &[
            resource_value("supply", 1, resource_supply),
            resource_value("demand", 2, resource_demand),
            fixed5("produced", "produced", 0),
            resource_value("fromSubjects", 3, resource_from_subjects),
            fixed5("imported", "imported", 0),
            resource_value("projects", 4, resource_projects),
            resource_value("production", 5, resource_production),
            resource_value("toOverlord", 6, resource_to_overlord),
            resource_value("exportReserve", 7, resource_export_reserve),
            resource_value("other", 8, resource_other),
            resource_value("barLength", 9, resource_bar_length),
            resource_value("net", 10, resource_net),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "naval-losses",
        table: TableId::NavalLossTotals,
        key: Key::Holder,
        parts: Some(Parts { bits: 16, id: None }),
        measures: &[count("total", "total").carried(Carry::Sum)],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "country-fuel",
        table: TableId::CountryFuel,
        key: Key::Country,
        parts: None,
        measures: &[raw("fuel", "current_raw"), raw("capacity", "maximum_raw")],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "equipment-losses",
        table: TableId::EquipmentLossTotals,
        key: Key::Holder,
        parts: Some(Parts { bits: 41, id: Some((0, 32)) }),
        measures: &[fixed5("total", "total", 0).carried(Carry::Sum)],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "party-popularity",
        table: TableId::PoliticalParties,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: None }),
        measures: &[fixed5("popularity", "popularity", 0)],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "threat-sum",
        table: TableId::ThreatTotals,
        key: Key::Singleton,
        parts: None,
        measures: &[fixed5("sum", "sum", 0)],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "faction-totals",
        table: TableId::FactionTotals,
        key: Key::Faction,
        parts: None,
        measures: &FACTION_MEASURES,
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "intel-networks",
        table: TableId::IntelNetworkSummaries,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: None }),
        measures: &[
            count("coveredControlled", "covered_controlled_states"),
            count("coveredCore", "covered_core_states"),
            fixed5("controlledStrength", "controlled_strength_sum", 0),
            fixed5("coreStrength", "core_strength_sum", 0),
            fixed5("maxStrength", "max_state_strength", 0),
        ],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "air-scores",
        table: TableId::RaidAirScores,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: None }),
        measures: &[raw("friendly", "friendly"), raw("enemy", "enemy")],
        drops: false,
        absent_none: false,
    },
    Series {
        kind: "produced-values",
        table: TableId::ProducedValues,
        key: Key::Country,
        parts: Some(Parts { bits: 32, id: Some((0, 32)) }),
        measures: &[
            Measure { carry: Carry::Sum, ..count("units", "units") },
            Measure { carry: Carry::Sum, ..count("unitsWithoutRefits", "units_without_refits") },
            Measure { carry: Carry::Sum, ..fixed5("ic", "ic", 0) },
            Measure { carry: Carry::Sum, ..fixed5("icWithoutRefits", "ic_without_refits", 0) },
        ],
        drops: false,
        absent_none: false,
    },
];

pub struct RunningTotal {
    pub kind: &'static str,
    pub clock: u128,
    pub since: &'static str,
    pub as_of: &'static str,
    pub reasons: &'static [(&'static str, &'static str)],
    pub rest: Option<u128>,
}

impl RunningTotal {
    pub fn reason(&self, column: &str) -> Option<&'static str> {
        self.reasons.iter().find(|(value, _)| *value == column).map(|(_, reason)| *reason)
    }
}

pub static RUNNING_TOTALS: &[RunningTotal] = &[RunningTotal {
    kind: "produced-values",
    clock: replay_data::rows::ProducedValueRow::clock(),
    since: "since",
    as_of: "as_of",
    reasons: &[("units", "units_reason"), ("units_without_refits", "units_without_refits_reason"), ("ic", "ic_reason"), ("ic_without_refits", "ic_without_refits_reason")],
    rest: Some(replay_data::rows::ProducedValueRow::REST as u128),
}];

pub fn running_total(kind: &str) -> Option<&'static RunningTotal> {
    RUNNING_TOTALS.iter().find(|total| total.kind == kind)
}

pub const FACTION_SOURCES: [(&str, &str, &str); 33] = [
    ("territory", "ownedStates", "owned_states"),
    ("territory", "controlledStates", "controlled_states"),
    ("territory", "occupiedStates", "occupied_states"),
    ("territory", "ownedProvinces", "owned_provinces"),
    ("territory", "controlledProvinces", "controlled_provinces"),
    ("territory", "occupiedProvinces", "occupied_provinces"),
    ("territory", "ownedVictoryPoints", "owned_victory_points"),
    ("territory", "controlledVictoryPoints", "controlled_victory_points"),
    ("territory", "occupiedVictoryPoints", "occupied_victory_points"),
    ("territory", "ownedArea", "owned_area"),
    ("territory", "controlledArea", "controlled_area"),
    ("territory", "occupiedArea", "occupied_area"),
    ("country-factories", "free1", "free1"),
    ("country-factories", "free2", "free2"),
    ("country-factories", "free3", "free3"),
    ("country-factories", "free4", "free4"),
    ("country-factories", "free5", "free5"),
    ("country-factories", "free6", "free6"),
    ("country-factories", "free7", "free7"),
    ("country-factories", "built1", "built1"),
    ("country-factories", "built2", "built2"),
    ("country-factories", "built3", "built3"),
    ("country-factories", "built4", "built4"),
    ("country-factories", "built5", "built5"),
    ("country-factories", "built6", "built6"),
    ("country-factories", "built7", "built7"),
    ("country-factories", "topBar1", "top_bar1"),
    ("country-factories", "topBar2", "top_bar2"),
    ("country-factories", "topBar3", "top_bar3"),
    ("country-factories", "topBar4", "top_bar4"),
    ("country-factories", "topBar5", "top_bar5"),
    ("country-factories", "topBar6", "top_bar6"),
    ("country-factories", "topBar7", "top_bar7"),
];

macro_rules! faction_measures {
    ($($index:literal),+ $(,)?) => {
        [$(count(FACTION_SOURCES[$index].1, FACTION_SOURCES[$index].2)),+]
    };
}

static FACTION_MEASURES: [Measure; 33] =
    faction_measures![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32];

pub fn faction_measure(kind: &str, measure: &str) -> Option<&'static str> {
    FACTION_SOURCES.iter().find(|(held, name, _)| *held == kind && *name == measure).map(|(_, name, _)| *name)
}

pub fn measure_value(series: &Series, measure: &Measure, cells: &dyn Cells, slot: u32) -> io::Result<Option<i64>> {
    Ok(match measure.column {
        Column::Cell(name) => cells.number(field(series.table, name)?, slot),
        Column::Computed { value, .. } => value(cells, slot).value(),
    })
}

const OWNED_STATES: usize = 0;
const CONTROLLED_STATES: usize = 1;

fn territory_holding(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(match (cells.number(OWNED_STATES, slot), cells.number(CONTROLLED_STATES, slot)) {
        (Some(owned), _) if owned > 0 => Some(1),
        (_, Some(controlled)) if controlled > 0 => Some(1),
        (Some(_), Some(_)) => Some(0),
        _ => None,
    })
}

const DEALT: usize = 0;
const TAKEN: usize = 1;

fn casualties_ranked(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(cells.number(DEALT, slot).zip(cells.number(TAKEN, slot)).and_then(|(dealt, taken)| dealt.checked_add(taken)))
}

const RESISTANCE_SUM: usize = 0;
const COMPLIANCE_SUM: usize = 1;
const OCCUPIED_STATES: usize = 2;

fn occupation_mean(sum: i64, count: i64) -> Option<i64> {
    let finite = (sum as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd;
    if count <= 0 || !finite {
        return None;
    }
    sum.wrapping_mul(100_000).checked_div(count.checked_mul(100_000)?)
}

fn occupation_resistance(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(cells.number(RESISTANCE_SUM, slot).zip(cells.number(OCCUPIED_STATES, slot)).and_then(|(sum, count)| occupation_mean(sum, count)))
}

fn occupation_compliance(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(cells.number(COMPLIANCE_SUM, slot).zip(cells.number(OCCUPIED_STATES, slot)).and_then(|(sum, count)| occupation_mean(sum, count)))
}

pub const FACTORY_MODES: [&str; 4] = factories::MODES;

macro_rules! factory_channels {
    ($($index:literal $name:literal $rank:literal),+ $(,)?) => {
        [$(Measure {
            name: $name,
            column: Column::Computed { id: $index, inputs: &FACTORY_INPUTS, value: factory_channel::<$index> },
            decimals: 0,
            aggregates: 0,
            carry: Carry::None,
            rank: $rank,
        }),+]
    };
}

static FACTORY_CHANNELS: [Measure; 28] = factory_channels![
    0 "free1" 32, 1 "free2" 32, 2 "free3" 32, 3 "free4" 32, 4 "free5" 32, 5 "free6" 32, 6 "free7" 32,
    7 "built1" 32, 8 "built2" 32, 9 "built3" 32, 10 "built4" 32, 11 "built5" 32, 12 "built6" 32, 13 "built7" 32,
    14 "topBar1" 32, 15 "topBar2" 32, 16 "topBar3" 32, 17 "topBar4" 32, 18 "topBar5" 32, 19 "topBar6" 32, 20 "topBar7" 32,
    21 "damaged1" 0, 22 "damaged2" 0, 23 "damaged3" 0, 24 "damaged4" 0, 25 "damaged5" 0, 26 "damaged6" 0, 27 "damaged7" 0,
];

const FACTORY_INPUTS: [&str; 17] = [
    "military_total_raw",
    "military_damaged_raw",
    "military_to_overlord",
    "military_to_others",
    "naval_total_raw",
    "naval_damaged_raw",
    "naval_to_overlord",
    "naval_to_others",
    "civilian_total_raw",
    "civilian_damaged_raw",
    "civilian_to_overlord",
    "civilian_to_others",
    "consumer_goods",
    "contract_payments",
    "special_projects",
    "license_imports",
    "trade_imports",
];

pub fn factory_count(cells: &dyn Cells, slot: u32, group: usize, mode: usize) -> Option<i64> {
    factories::count(&|column| cells.number(column, slot), group, mode)
}

fn factory_channel<const C: usize>(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(factories::channel(&|column| cells.number(column, slot), C / 7, C % 7 + 1))
}

pub fn factory_measure(mode: &str, mask: usize) -> Option<&'static str> {
    let mode = FACTORY_MODES.iter().position(|held| *held == mode)?;
    (1..=7).contains(&mask).then(|| FACTORY_CHANNELS[mode * 7 + mask - 1].name)
}

fn nuke_amount(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(cells.number(0, slot).map(|amount| i64::from(NukeRow::shown(amount))))
}

const AUTONOMY_SCORE: usize = 0;
const AUTONOMY_LOWER: usize = 1;
const AUTONOMY_UPPER: usize = 2;

pub const NO_AUTONOMY_RANGE: &str = "No autonomy range";

pub fn autonomy_progress_milli(score: i64, lower: i64, upper: i64) -> Result<Option<i64>, &'static str> {
    let room = i128::from(upper) - i128::from(lower);
    if room <= 0 {
        return Err(NO_AUTONOMY_RANGE);
    }
    Ok(i64::try_from((i128::from(score) - i128::from(lower)) * 100_000 / room).ok())
}

fn autonomy_progress(cells: &dyn Cells, slot: u32) -> Computed {
    let (Some(score), Some(lower), Some(upper)) = (cells.number(AUTONOMY_SCORE, slot), cells.number(AUTONOMY_LOWER, slot), cells.number(AUTONOMY_UPPER, slot)) else {
        return Computed::Unrecorded;
    };
    match autonomy_progress_milli(score, lower, upper) {
        Ok(value) => Computed::of(value),
        Err(reason) => Computed::NotApplicable(reason),
    }
}

const FUEL_REQUESTED: usize = 1;
const FUEL_RECEIVED: usize = 2;

fn fuel_received_share(cells: &dyn Cells, slot: u32) -> Computed {
    let (Some(requested), Some(received)) = (cells.number(FUEL_REQUESTED, slot), cells.number(FUEL_RECEIVED, slot)) else {
        return Computed::Unrecorded;
    };
    let requested = i128::from(requested);
    if requested <= 0 {
        return Computed::NotApplicable("No fuel requested");
    }
    Computed::of(i64::try_from(i128::from(received) * 1_000_000 / requested).ok())
}

const SPECIAL_TOTAL: usize = 4;
const PRODUCTION_DEMAND: usize = 5;

const SUPPLY_COUNTS: [usize; 3] = [0, 1, 2];
const SUPPLY_COUNT_NAMES: &[&str] = &["no_supply", "strained", "in_supply"];
const SHARE_DECIMALS: u8 = 12;
const SHARE_SCALE: i128 = 1_000_000_000_000;

const fn share(name: &'static str, id: u32, value: fn(&dyn Cells, u32) -> Computed) -> Measure {
    Measure {
        name,
        column: Column::Computed { id, inputs: SUPPLY_COUNT_NAMES, value },
        decimals: SHARE_DECIMALS,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }
}

fn supply_share(cells: &dyn Cells, slot: u32, at: usize) -> Computed {
    let mut counts = [0; 3];
    for (count, column) in counts.iter_mut().zip(SUPPLY_COUNTS) {
        let Some(value) = cells.number(column, slot) else {
            return Computed::Unrecorded;
        };
        *count = value;
    }
    let total: i128 = counts.iter().map(|&count| i128::from(count)).sum();
    if total <= 0 {
        return Computed::NotApplicable("No divisions");
    }
    Computed::of(i64::try_from(i128::from(counts[at]) * SHARE_SCALE / total).ok())
}

fn no_supply_share(cells: &dyn Cells, slot: u32) -> Computed {
    supply_share(cells, slot, 0)
}

fn strained_share(cells: &dyn Cells, slot: u32) -> Computed {
    supply_share(cells, slot, 1)
}

fn in_supply_share(cells: &dyn Cells, slot: u32) -> Computed {
    supply_share(cells, slot, 2)
}

fn supply_divisions(cells: &dyn Cells, slot: u32) -> Computed {
    let mut total: i128 = 0;
    for column in SUPPLY_COUNTS {
        let Some(value) = cells.number(column, slot) else {
            return Computed::Unrecorded;
        };
        total += i128::from(value);
    }
    Computed::of(i64::try_from(total).ok())
}

const FUEL_DAY: [usize; 14] = {
    let mut out = [0; 14];
    let mut at = 0;
    while at < out.len() {
        out[at] = column::<FuelDayRow>(FuelDayRow::RAW[at]);
        at += 1;
    }
    out
};

fn fuel_measure<const AT: usize>(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(FuelDayRow::measures(FUEL_DAY.map(|at| cells.number(at, slot)))[AT])
}

const fn fuel_day(name: &'static str, id: u32, value: fn(&dyn Cells, u32) -> Computed) -> Measure {
    Measure {
        name,
        column: Column::Computed { id, inputs: &FuelDayRow::RAW, value },
        decimals: 0,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }
}

const PRODUCED: usize = 0;
const ADJUSTMENT: usize = 1;
const IMPORTED: usize = 2;
const USED: usize = 3;
const TO_EXPORT: usize = 6;

const RESOURCE_POOLS: &[&str] = &["produced", "adjustment", "imported", "used", "special_total", "production_demand", "to_export"];

const fn resource_value(name: &'static str, id: u32, value: fn(&dyn Cells, u32) -> Computed) -> Measure {
    Measure {
        name,
        column: Column::Computed { id, inputs: RESOURCE_POOLS, value },
        decimals: 5,
        aggregates: 0,
        carry: Carry::None,
        rank: 0,
    }
}

struct Balance {
    supply: i128,
    demand: i128,
    net: i128,
    produced: i128,
    imported: i128,
    from_subjects: i128,
    to_overlord: i128,
    projects: i128,
    production: i128,
    reserve: i128,
}

fn balance(cells: &dyn Cells, slot: u32) -> Option<Balance> {
    let mut pools = [0i128; 7];
    for (at, column) in [PRODUCED, ADJUSTMENT, IMPORTED, USED, SPECIAL_TOTAL, PRODUCTION_DEMAND, TO_EXPORT].into_iter().enumerate() {
        pools[at] = i128::from(cells.number(column, slot)?);
    }
    let [produced, adjustment, imported, used, after_projects, production_demand, reserve] = pools;
    let projects = used - after_projects;
    let production = -production_demand;
    let from_subjects = adjustment.max(0);
    let to_overlord = (-adjustment).max(0);
    Some(Balance {
        supply: produced + from_subjects + imported,
        demand: projects + production + to_overlord + reserve,
        net: after_projects - production,
        produced,
        imported,
        from_subjects,
        to_overlord,
        projects,
        production,
        reserve,
    })
}

fn narrow(value: i128) -> Option<i64> {
    i64::try_from(value).ok()
}

fn resource_supply(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| narrow(held.supply)))
}

fn resource_demand(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| narrow(held.demand)))
}

fn resource_net(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| narrow(held.supply - held.demand)))
}

fn resource_from_subjects(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| narrow(held.from_subjects)))
}

fn resource_projects(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| narrow(-held.projects)))
}

fn resource_production(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| narrow(-held.production)))
}

fn resource_to_overlord(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| narrow(-held.to_overlord)))
}

fn resource_export_reserve(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| narrow(-held.reserve)))
}

fn resource_other(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| narrow(held.net - (held.supply - held.demand))))
}

fn resource_bar_length(cells: &dyn Cells, slot: u32) -> Computed {
    Computed::of(balance(cells, slot).and_then(|held| {
        let supply = held.produced.abs() + held.from_subjects.abs() + held.imported.abs();
        let demand = (held.projects + held.production).abs() + held.to_overlord.abs() + held.reserve.abs();
        narrow(supply.max(demand))
    }))
}

pub static LOGS: &[Log] = &[
    Log {
        name: "market-events",
        table: TableId::MarketEvents,
        facets: &[
            LogFacet { name: "kind", from: FacetFrom::Column("kind"), holds: Holds::Value },
            LogFacet { name: "country", from: FacetFrom::Column("country"), holds: Holds::Owner(Key::Country) },
            LogFacet { name: "receiver", from: FacetFrom::Column("receiver"), holds: Holds::Owner(Key::Country) },
        ],
    },
    Log {
        name: "balance-drops",
        table: TableId::BalanceDrops,
        facets: &[
            LogFacet { name: "measure", from: FacetFrom::Key(64), holds: Holds::Value },
            LogFacet { name: "country", from: FacetFrom::Key(32), holds: Holds::Owner(Key::Country) },
        ],
    },
    Log {
        name: "threat-events",
        table: TableId::ThreatEvents,
        facets: &[
            LogFacet { name: "kind", from: FacetFrom::Key(96), holds: Holds::Value },
            LogFacet { name: "slot", from: FacetFrom::Key(64), holds: Holds::Owner(Key::Slot) },
        ],
    },
    Log {
        name: "land-battle-ends",
        table: TableId::LandBattleEnds,
        facets: &[LogFacet { name: "kind", from: FacetFrom::Column("kind"), holds: Holds::Value }],
    },
    Log {
        name: "division-events",
        table: TableId::DivisionEvents,
        facets: &[
            LogFacet { name: "kind", from: FacetFrom::Key(96), holds: Holds::Value },
            LogFacet { name: "slot", from: FacetFrom::Column("slot"), holds: Holds::Owner(Key::Slot) },
            LogFacet { name: "template", from: FacetFrom::Column("template_id"), holds: Holds::Value },
        ],
    },
    Log {
        name: "leader-battles",
        table: TableId::LeaderBattles,
        facets: &[
            LogFacet { name: "leader", from: FacetFrom::Key(96), holds: Holds::Value },
            LogFacet { name: "entry", from: FacetFrom::Column("entry"), holds: Holds::Value },
            LogFacet { name: "outcome", from: FacetFrom::Column("outcome"), holds: Holds::Value },
        ],
    },
    Log {
        name: "leader-trait-gains",
        table: TableId::LeaderTraitGains,
        facets: &[
            LogFacet { name: "leader", from: FacetFrom::Key(96), holds: Holds::Value },
            LogFacet { name: "class", from: FacetFrom::Column("class"), holds: Holds::Value },
        ],
    },
    Log {
        name: "ship-losses",
        table: TableId::ShipLosses,
        facets: &[LogFacet { name: "slot", from: FacetFrom::Key(104), holds: Holds::Owner(Key::Slot) }],
    },
    Log {
        name: "peace-outcomes",
        table: TableId::PeaceOutcomes,
        facets: &[
            LogFacet { name: "observation", from: FacetFrom::Column("observation"), holds: Holds::Value },
            LogFacet { name: "change", from: FacetFrom::Column("change"), holds: Holds::Value },
            LogFacet { name: "from", from: FacetFrom::Column("from_slot"), holds: Holds::Owner(Key::Slot) },
            LogFacet { name: "to", from: FacetFrom::Column("to_slot"), holds: Holds::Owner(Key::Slot) },
        ],
    },
    Log {
        name: "peace-conference-spans",
        table: TableId::PeaceConferenceSpans,
        facets: &[],
    },
    Log {
        name: "feed-events",
        table: TableId::FeedEvents,
        facets: &[
            LogFacet { name: "kind", from: FacetFrom::Column("kind"), holds: Holds::Value },
            LogFacet { name: "actor", from: FacetFrom::Column("actor_slot"), holds: Holds::Owner(Key::Slot) },
            LogFacet { name: "opener", from: FacetFrom::Column("opener_hour"), holds: Holds::Value },
            LogFacet { name: "incomplete", from: FacetFrom::Column("incomplete"), holds: Holds::Value },
        ],
    },
    Log {
        name: "air-losses",
        table: TableId::AirLossEvents,
        facets: &[
            LogFacet { name: "cause", from: FacetFrom::Column("native_cause"), holds: Holds::Value },
            LogFacet { name: "region", from: FacetFrom::Column("region"), holds: Holds::Value },
            LogFacet { name: "country", from: FacetFrom::Column("country"), holds: Holds::Owner(Key::Country) },
            LogFacet { name: "opponent", from: FacetFrom::Column("opponent"), holds: Holds::Owner(Key::Country) },
        ],
    },
    Log {
        name: "bombing-damage",
        table: TableId::BuildingBombingDamage,
        facets: &[
            LogFacet { name: "cause", from: FacetFrom::Column("cause"), holds: Holds::Value },
            LogFacet { name: "building", from: FacetFrom::Column("building_type"), holds: Holds::Value },
            LogFacet { name: "attacker", from: FacetFrom::Column("attacker"), holds: Holds::Owner(Key::Country) },
            LogFacet { name: "victim", from: FacetFrom::Column("victim"), holds: Holds::Owner(Key::Country) },
        ],
    },
    Log {
        name: "operation-completions",
        table: TableId::OperationCompletions,
        facets: &[
            LogFacet { name: "owner", from: FacetFrom::Key(64), holds: Holds::Owner(Key::Country) },
            LogFacet { name: "target", from: FacetFrom::Column("target"), holds: Holds::Owner(Key::Country) },
            LogFacet { name: "operation", from: FacetFrom::Column("definition_token"), holds: Holds::Value },
        ],
    },
    Log {
        name: "raid-ends",
        table: TableId::RaidEnds,
        facets: &[
            LogFacet { name: "raider", from: FacetFrom::Column("raider"), holds: Holds::Owner(Key::Slot) },
            LogFacet { name: "victim", from: FacetFrom::Column("victim_slot"), holds: Holds::Owner(Key::Slot) },
            LogFacet { name: "outcome", from: FacetFrom::Column("outcome"), holds: Holds::Value },
        ],
    },
    Log { name: "peace-settlements", table: TableId::PeaceSettlements, facets: &[] },
    Log {
        name: "combat-log-battle-modifiers",
        table: TableId::CombatLogBattleModifiers,
        facets: &[LogFacet { name: "battle", from: FacetFrom::Key(40), holds: Holds::Value }],
    },
    Log { name: "land-combat-results", table: TableId::LandCombatResults, facets: &[] },
    Log { name: "nuclear-strikes", table: TableId::NuclearStrikes, facets: &[] },
    Log {
        name: "leader-commands",
        table: TableId::LeaderCommands,
        facets: &[LogFacet { name: "leader", from: FacetFrom::Key(96), holds: Holds::Value }],
    },
];

pub static READ_BY_KEY: &[TableId] = &[TableId::RelationOpinions, TableId::IntelNetworkSummaries];

pub fn read_by_key(table: TableId) -> bool {
    READ_BY_KEY.contains(&table)
}

pub static ABSENT_IS_ZERO: &[TableId] = &[TableId::RelationOpinions];

pub fn absent_is_zero(table: TableId) -> bool {
    ABSENT_IS_ZERO.contains(&table)
}

pub static LIFETIMES: &[TableId] = &[TableId::TagHolders, TableId::GroupSpans, TableId::WarEvents, TableId::LookSpans, TableId::ArmyMix, TableId::ResearchCoverage, TableId::OperativeMemberships, TableId::Raids, TableId::CasualtyLifetimes, TableId::ConstructionProjects, TableId::UnitLeaders, TableId::AirStatistics, TableId::ProductionTotals, TableId::DockyardRepairRates, TableId::Operations, TableId::NavalBattles, TableId::NavalResults];

pub fn series(kind: &str) -> Option<&'static Series> {
    SERIES.iter().find(|series| series.kind == kind)
}

pub fn log(name: &str) -> Option<&'static Log> {
    LOGS.iter().find(|log| log.name == name)
}

pub fn field(table: TableId, name: &str) -> io::Result<usize> {
    (0..table.field_count())
        .find(|&field| table.field_name(field) == Some(name))
        .ok_or_else(|| invalid(format!("{table:?} has no field {name}")))
}

pub fn source(table: TableId, measure: &Measure) -> io::Result<Source> {
    Ok(match measure.column {
        Column::Cell(name) => Source::Cell(field(table, name)?),
        Column::Computed { id, inputs, value } => {
            let mut bits = 0u64;
            for input in inputs {
                let at = field(table, input)?;
                if at >= 64 {
                    return Err(invalid(format!("{table:?}'s {input} is past the 64 columns a computed column reads")));
                }
                bits |= 1 << at;
            }
            Source::Computed { id, inputs: bits, value }
        }
    })
}

pub fn summary_id(table: TableId, measure: &Measure) -> io::Result<u32> {
    Ok(source(table, measure)?.id())
}

pub fn summaries() -> io::Result<Vec<(usize, Shape)>> {
    let mut out: Vec<(usize, Shape)> = Vec::new();
    for series in SERIES.iter().filter(|series| !read_by_key(series.table)) {
        let index = series.table.index();
        let at = match out.iter().position(|(table, _)| *table == index) {
            Some(at) => at,
            None => {
                out.push((index, Shape::Values(Vec::new())));
                out.len() - 1
            }
        };
        let Shape::Values(columns) = &mut out[at].1 else {
            return Err(invalid(format!("{:?} is declared both a series and another shape", series.table)));
        };
        for measure in series.measures {
            let source = source(series.table, measure)?;
            let same = |held: &summary::Column| match (held.source, source) {
                (Source::Cell(a), Source::Cell(b)) => a == b,
                (Source::Computed { id: a, .. }, Source::Computed { id: b, .. }) => a == b,
                _ => false,
            };
            match columns.iter_mut().find(|held| same(held)) {
                Some(held) => {
                    held.aggregates |= measure.aggregates;
                    held.rank = held.rank.max(measure.rank);
                }
                None => columns.push(summary::Column { source, aggregates: measure.aggregates, rank: measure.rank }),
            }
        }
    }
    for log in LOGS {
        let facets = log
            .facets
            .iter()
            .map(|facet| {
                Ok(match facet.from {
                    FacetFrom::Column(name) => Facet::Column(field(log.table, name)?),
                    FacetFrom::Key(shift) => Facet::Key(shift),
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        if out.iter().any(|(table, _)| *table == log.table.index()) {
            return Err(invalid(format!("{:?} is declared twice", log.table)));
        }
        out.push((log.table.index(), Shape::Log(facets)));
    }
    for &table in LIFETIMES {
        if out.iter().any(|(held, _)| *held == table.index()) {
            return Err(invalid(format!("{table:?} is declared twice")));
        }
        out.push((table.index(), Shape::Lifetimes));
    }
    out.sort_unstable_by_key(|(table, _)| *table);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_table_is_declared_by_more_than_one_of_series_logs_and_lifetimes() {
        let mut series: Vec<TableId> = SERIES.iter().map(|series| series.table).collect();
        series.sort_unstable_by_key(|table| table.index());
        series.dedup();
        let declared: Vec<(TableId, &str)> = (series.into_iter().map(|table| (table, "SERIES")))
            .chain(LOGS.iter().map(|log| (log.table, "LOGS")))
            .chain(LIFETIMES.iter().map(|&table| (table, "LIFETIMES")))
            .collect();
        let twice: Vec<String> = declared
            .iter()
            .enumerate()
            .filter(|(at, (table, _))| declared[..*at].iter().any(|(held, _)| held == table))
            .map(|(_, (table, list))| format!("{table:?} again in {list}"))
            .collect();
        assert!(twice.is_empty(), "each table is declared once: {twice:?}");
    }

    #[test]
    fn every_declaration_names_columns_its_table_holds() {
        let summarized = summaries().unwrap();
        for series in SERIES {
            for measure in series.measures {
                summary_id(series.table, measure).unwrap();
            }
            assert_eq!(summarized.iter().any(|(table, _)| *table == series.table.index()), !read_by_key(series.table), "{}: summarized unless read by key", series.kind);
        }
        let mut kinds: Vec<&str> = SERIES.iter().map(|series| series.kind).collect();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), SERIES.len(), "series kinds are unique");
        for series in SERIES.iter().filter(|series| series.parts.is_some()) {
            let parts = series.parts.unwrap();
            assert!(!series.drops, "{}: drops are an owner's, not a part's", series.kind);
            assert!((1..=96).contains(&parts.bits), "{}: an owner's 32 bits sit above its part", series.kind);
            assert!(parts.id.is_none_or(|(shift, width)| width > 0 && width <= 64 && shift + width <= parts.bits), "{}", series.kind);
        }
        for log in LOGS {
            let mut names: Vec<&str> = log.facets.iter().map(|facet| facet.name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), log.facets.len(), "{}'s facet names are unique", log.name);
            assert!(log.facets.len() <= 4, "{}: a log is counted by four facets at most", log.name);
            assert!(log.facets.iter().all(|facet| facet.holds != Holds::Owner(Key::Holder)), "{}: a facet's 32 bits cannot hold a holder", log.name);
        }
    }

    struct Pools([Option<i64>; 9]);

    impl Cells for Pools {
        fn number(&self, column: usize, _slot: u32) -> Option<i64> {
            self.0.get(column).copied().flatten()
        }
        fn key(&self, _slot: u32) -> Option<u128> {
            Some(0)
        }
        fn slot(&self, _key: u128) -> Option<u32> {
            Some(0)
        }
        fn each_row(&self, each: &mut dyn FnMut(u32)) {
            each(0)
        }
    }

    fn balance_value(row: &Pools, measure: &str) -> Option<i64> {
        match series("resource-balance").unwrap().measure(measure).unwrap().column {
            Column::Computed { value, .. } => value(row, 0).value(),
            Column::Cell(name) => row.number(field(TableId::CountryResources, name).unwrap(), 0),
        }
    }

    #[test]
    fn a_resource_balance_splits_its_pools_as_the_bars_draw_them() {
        for (column, name) in [(PRODUCED, "produced"), (ADJUSTMENT, "adjustment"), (IMPORTED, "imported"), (USED, "used"), (TO_EXPORT, "to_export")] {
            assert_eq!(field(TableId::CountryResources, name).unwrap(), column);
        }
        let overlord = Pools([Some(10), Some(2), Some(3), Some(12), Some(9), Some(-7), Some(1), None, None]);
        let values = |names: &[&str]| names.iter().map(|name| balance_value(&overlord, name)).collect::<Vec<_>>();
        assert_eq!(values(&["supply", "demand"]), [Some(15), Some(11)]);
        assert_eq!(values(&["fromSubjects", "toOverlord", "projects", "production", "exportReserve"]), [Some(2), Some(0), Some(-3), Some(-7), Some(-1)]);
        assert_eq!(values(&["other"]), [Some(2 - (15 - 11))], "the rounding the parts leave against the recorded net");
        assert_eq!(values(&["barLength"]), [Some(15)]);
        let subject = Pools([Some(10), Some(-4), Some(0), Some(6), Some(6), Some(0), Some(0), None, None]);
        assert_eq!([balance_value(&subject, "fromSubjects"), balance_value(&subject, "toOverlord")], [Some(0), Some(-4)]);
        let unknown = Pools([Some(10), None, Some(3), Some(12), Some(9), Some(-7), Some(1), None, None]);
        assert_eq!(balance_value(&unknown, "supply"), None, "an unrecorded pool leaves the balance unrecorded, never zero");
    }

    #[test]
    fn a_fuel_share_reads_its_columns_by_their_schema_numbers() {
        assert_eq!(field(TableId::CountryFuelConsumers, "requested_raw").unwrap(), FUEL_REQUESTED);
        assert_eq!(field(TableId::CountryFuelConsumers, "received_raw").unwrap(), FUEL_RECEIVED);
        assert_eq!(fuel_received_share(&Row([None, Some(100), Some(50)]), 0), Computed::Value(500_000));
        assert_eq!(fuel_received_share(&Row([None, Some(0), Some(0)]), 0), Computed::NotApplicable("No fuel requested"));
        assert_eq!(fuel_received_share(&Row([None, None, Some(50)]), 0), Computed::Unrecorded);
        assert_eq!(fuel_received_share(&Row([None, Some(100), None]), 0), Computed::Unrecorded);
    }

    #[test]
    fn occupier_means_read_their_columns_and_truncate_as_the_game_does() {
        for (column, name) in [(RESISTANCE_SUM, "resistance_sum"), (COMPLIANCE_SUM, "compliance_sum"), (OCCUPIED_STATES, "states")] {
            assert_eq!(field(TableId::OccupierTotals, name).unwrap(), column);
        }
        assert_eq!((field(TableId::CasualtyTotals, "dealt").unwrap(), field(TableId::CasualtyTotals, "taken").unwrap()), (DEALT, TAKEN));
        assert_eq!(occupation_mean(250_000, 3), Some(83_333));
        assert_eq!(occupation_mean(-250_000, 3), Some(-83_333));
        assert_eq!(occupation_mean(1, 0), None);
        assert_eq!(occupation_mean(i64::MAX, 1), None, "a saturated sum has no mean");
        assert_eq!(Key::Holder.of(7, 9), 7u128 << 32 | 9);
    }

    #[test]
    fn autonomy_progress_reads_its_columns_by_their_schema_numbers_and_marks_zero_range_not_applicable() {
        assert_eq!(field(TableId::Autonomy, "score").unwrap(), AUTONOMY_SCORE);
        assert_eq!(field(TableId::Autonomy, "lower_score").unwrap(), AUTONOMY_LOWER);
        assert_eq!(field(TableId::Autonomy, "upper_score").unwrap(), AUTONOMY_UPPER);
        assert_eq!(autonomy_progress(&Row([Some(334_950_000), Some(300_000_000), Some(400_000_000)]), 0), Computed::Value(34_950));
        assert_eq!(autonomy_progress(&Row([Some(197_799_982), Some(100_000_000), Some(200_000_000)]), 0), Computed::Value(97_799));
        assert_eq!(autonomy_progress(&Row([Some(450_000_000), Some(300_000_000), Some(400_000_000)]), 0), Computed::Value(150_000));
        assert_eq!(autonomy_progress(&Row([Some(300_000_000), Some(400_000_000), Some(400_000_000)]), 0), Computed::NotApplicable("No autonomy range"));
        assert_eq!(autonomy_progress(&Row([None, Some(300_000_000), Some(400_000_000)]), 0), Computed::Unrecorded);
        assert!(series("autonomy").unwrap().measure("progress").is_some());
    }

    struct Row([Option<i64>; 3]);

    impl Cells for Row {
        fn number(&self, column: usize, _slot: u32) -> Option<i64> {
            self.0[column]
        }
        fn key(&self, _slot: u32) -> Option<u128> {
            Some(0)
        }
        fn slot(&self, _key: u128) -> Option<u32> {
            Some(0)
        }
        fn each_row(&self, each: &mut dyn FnMut(u32)) {
            each(0);
        }
    }

    #[test]
    fn supply_shares_read_the_counts_by_their_schema_numbers_and_no_divisions_is_no_share() {
        for (at, name) in SUPPLY_COUNT_NAMES.iter().enumerate() {
            assert_eq!(field(TableId::SupplyHealth, name).unwrap(), SUPPLY_COUNTS[at]);
        }
        let row = Row([Some(1), Some(1), Some(1)]);
        assert_eq!([no_supply_share(&row, 0), strained_share(&row, 0), in_supply_share(&row, 0)], [Computed::Value(333_333_333_333); 3]);
        assert_eq!(strained_share(&Row([Some(0), Some(3), Some(1)]), 0), Computed::Value(750_000_000_000));
        assert_eq!(no_supply_share(&Row([Some(i64::MAX), Some(i64::MAX), Some(0)]), 0), Computed::Value(500_000_000_000), "no count is too large");
        assert_eq!(in_supply_share(&Row([Some(0); 3]), 0), Computed::NotApplicable("No divisions"), "no divisions");
        assert_eq!(in_supply_share(&Row([Some(2), None, Some(2)]), 0), Computed::Unrecorded, "a count not recorded");
        assert_eq!(supply_divisions(&Row([Some(2), Some(0), Some(5)]), 0), Computed::Value(7));
        assert_eq!(supply_divisions(&Row([Some(0); 3]), 0), Computed::Value(0), "no divisions is a known zero");
        assert_eq!(supply_divisions(&Row([Some(2), None, Some(2)]), 0), Computed::Unrecorded, "a count not recorded");
    }

    #[test]
    fn territory_is_a_series_whose_rows_lifetimes_are_still_read() {
        let territory = series("territory").unwrap();
        assert_eq!(territory.measures.len(), 13);
        assert!(territory.measures.iter().filter(|measure| measure.name != "holding").all(|measure| measure.rank > 0), "every territory count is a race");
        assert_eq!(field(TableId::TerritoryTotals, "owned_states").unwrap(), OWNED_STATES);
        assert_eq!(field(TableId::TerritoryTotals, "controlled_states").unwrap(), CONTROLLED_STATES);
        let holding = |owned: Option<i64>, controlled: Option<i64>| {
            let row = Named([(OWNED_STATES, owned), (CONTROLLED_STATES, controlled)].into_iter().filter_map(|(at, value)| Some((at, value?))).collect());
            territory_holding(&row, 0).value()
        };
        assert_eq!((holding(Some(0), Some(2)), holding(Some(0), Some(0)), holding(None, Some(1)), holding(None, Some(0))), (Some(1), Some(0), Some(1), None));
        assert!(!LIFETIMES.contains(&TableId::TerritoryTotals), "a series table's pieces hold its lifetimes already");
        let shapes = summaries().unwrap();
        assert!(matches!(shapes.iter().find(|(table, _)| *table == TableId::TerritoryTotals.index()), Some((_, Shape::Values(_)))));
    }

    struct Named(std::collections::HashMap<usize, i64>);

    impl Cells for Named {
        fn number(&self, column: usize, _slot: u32) -> Option<i64> {
            self.0.get(&column).copied()
        }
        fn key(&self, _slot: u32) -> Option<u128> {
            Some(0)
        }
        fn slot(&self, _key: u128) -> Option<u32> {
            Some(0)
        }
        fn each_row(&self, each: &mut dyn FnMut(u32)) {
            each(0);
        }
    }

    fn row(cells: &[(&str, i64)]) -> Named {
        Named(cells.iter().map(|&(name, value)| (field(TableId::CountryFactories, name).unwrap(), value)).collect())
    }

    #[test]
    fn a_factory_count_truncates_as_the_game_shows_and_is_not_recorded_when_a_cell_it_reads_is_not() {
        let full = row(&[
            ("military_total_raw", 1_299_999),
            ("military_damaged_raw", 199_999),
            ("military_to_overlord", 2),
            ("military_to_others", 1),
            ("naval_total_raw", 500_000),
            ("naval_damaged_raw", 0),
            ("naval_to_overlord", 0),
            ("naval_to_others", 0),
            ("civilian_total_raw", 3_000_000),
            ("civilian_damaged_raw", 100_000),
            ("civilian_to_overlord", 0),
            ("civilian_to_others", 1),
            ("consumer_goods", 5),
            ("contract_payments", 1),
            ("special_projects", 2),
            ("license_imports", 1),
            ("trade_imports", 3),
        ]);
        assert_eq!((factory_count(&full, 0, 0, 1), factory_count(&full, 0, 0, 2), factory_count(&full, 0, 0, 0)), (Some(12), Some(8), Some(8)));
        assert_eq!((factory_count(&full, 0, 2, 1), factory_count(&full, 0, 2, 2), factory_count(&full, 0, 2, 0)), (Some(30), Some(28), Some(16)));
        let value = |name: &str| match FACTORY_CHANNELS.iter().find(|measure| measure.name == name).unwrap().column {
            Column::Computed { value, .. } => value(&full, 0).value(),
            Column::Cell(_) => unreachable!(),
        };
        assert_eq!((value("free7"), value("built5"), value("topBar2")), (Some(8 + 5 + 16), Some(12 + 30), Some(5)));
        assert_eq!((value("damaged1"), value("damaged4"), value("damaged7")), (Some(1), Some(1), Some(2)));
        assert!(FACTORY_CHANNELS.iter().filter(|measure| measure.name.starts_with("damaged")).all(|measure| measure.rank == 0));
        assert_eq!(factory_measure("damaged", 5), Some("damaged5"));
        assert_eq!(factory_measure("topBar", 2), Some("topBar2"));
        assert_eq!((factory_measure("free", 0), factory_measure("spare", 1)), (None, None));
        let mut partial = full;
        partial.0.remove(&factories::CONSUMERS[4]);
        assert_eq!(factory_count(&partial, 0, 2, 0), None, "an unrecorded consumer is never read as none consumed");
        assert_eq!((factory_count(&partial, 0, 2, 2), factory_count(&partial, 0, 0, 0)), (Some(28), Some(8)));
        partial.0.insert(factories::TOTAL, i64::from(i32::MAX) * 100_000 + 100_000);
        assert_eq!(factory_count(&partial, 0, 0, 1), None, "a total no whole number the game shows holds is not recorded");
    }

    #[test]
    fn a_nuclear_amount_is_the_top_bars_whole_count_and_its_sentinel() {
        assert_eq!(NukeRow::shown(0), 0);
        assert_eq!(NukeRow::shown(299_999_999), 2);
        assert_eq!(NukeRow::shown(i64::MAX), 0xa7c5);
        assert_eq!(Key::Singleton.of(7, 9), 0);
        assert_eq!((Key::Faction.fixed("401"), Key::Faction.fixed("GER"), Key::Singleton.fixed("World"), Key::Slot.fixed("401")), (Some(401), None, Some(0), None));
        for (kind, measure, column) in FACTION_SOURCES {
            assert!(series(kind).and_then(|held| held.measure(measure)).is_some(), "{kind} declares {measure}");
            assert!(field(TableId::FactionTotals, column).is_ok(), "FactionTotals has {column}");
        }
        assert_eq!(faction_measure("country-factories", "free7"), Some("free7"));
        assert_eq!(faction_measure("country-factories", "damaged7"), None, "damaged factories are no faction total");
    }

    #[test]
    fn computed_ratios_read_every_operand_before_declaring_not_applicable() {
        let map = |pairs: &[(usize, i64)]| Named(pairs.iter().copied().collect());
        assert_eq!(fuel_received_share(&map(&[(FUEL_REQUESTED, 0)]), 0), Computed::Unrecorded);
        assert_eq!(fuel_received_share(&map(&[(FUEL_RECEIVED, 0)]), 0), Computed::Unrecorded);
        assert_eq!(fuel_received_share(&map(&[(FUEL_REQUESTED, 0), (FUEL_RECEIVED, 0)]), 0), Computed::NotApplicable("No fuel requested"));
        assert_eq!(fuel_received_share(&map(&[(FUEL_REQUESTED, 100), (FUEL_RECEIVED, 25)]), 0), Computed::Value(250_000));
        assert_eq!(autonomy_progress(&map(&[(AUTONOMY_LOWER, 100), (AUTONOMY_UPPER, 100)]), 0), Computed::Unrecorded);
        assert_eq!(autonomy_progress(&map(&[(AUTONOMY_SCORE, 100), (AUTONOMY_LOWER, 100), (AUTONOMY_UPPER, 100)]), 0), Computed::NotApplicable("No autonomy range"));
        assert_eq!(no_supply_share(&map(&[(0, 0), (2, 0)]), 0), Computed::Unrecorded);
        assert_eq!(no_supply_share(&map(&[(0, 0), (1, 0), (2, 0)]), 0), Computed::NotApplicable("No divisions"));
    }
}
