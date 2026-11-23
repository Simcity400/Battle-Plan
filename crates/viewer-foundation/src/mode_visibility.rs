
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PriorityTable {
    Default,
    StrategicAir,
    StrategicNavy,
    Operatives,
    States,
    Resistance,
    Resources,
    Diplomacy,
    Factions,
    Players,
    Infrastructure,
    Manpower,
    Ideology,
}

impl PriorityTable {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Default => "DEFAULT",
            Self::StrategicAir => "STRATEGIC_AIR",
            Self::StrategicNavy => "STRATEGIC_NAVY",
            Self::Operatives => "OPERATIVES",
            Self::States => "STATES",
            Self::Resistance => "RESISTANCE",
            Self::Resources => "RESOURCES",
            Self::Diplomacy => "DIPLOMACY",
            Self::Factions => "FACTIONS",
            Self::Players => "PLAYERS",
            Self::Infrastructure => "INFRASTRUCTURE",
            Self::Manpower => "MANPOWER",
            Self::Ideology => "IDEOLOGY",
        }
    }
}

impl PriorityTable {
    pub const ALL: [Self; 13] = [
        Self::Default,
        Self::StrategicAir,
        Self::StrategicNavy,
        Self::Operatives,
        Self::States,
        Self::Resistance,
        Self::Resources,
        Self::Diplomacy,
        Self::Factions,
        Self::Players,
        Self::Infrastructure,
        Self::Manpower,
        Self::Ideology,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PriorityIcon {
    UnitsStack,
    UnitsStackGroup,
    VictoryPoints,
    AirBase,
    NavalBase,
    NavalHeadquarter,
    Facility,
    LandCombat,
    NavalCombat,
    NavalCombatResults,
    NavalMission,
    AirMission,
    Capital,
    RocketSite,
    GunEmplacement,
    StrategicLocation,
    AdjacencyRule,
    Resource,
}

impl PriorityIcon {
    pub const ALL: [Self; 18] = [
        Self::UnitsStack,
        Self::UnitsStackGroup,
        Self::VictoryPoints,
        Self::AirBase,
        Self::NavalBase,
        Self::NavalHeadquarter,
        Self::Facility,
        Self::LandCombat,
        Self::NavalCombat,
        Self::NavalCombatResults,
        Self::NavalMission,
        Self::AirMission,
        Self::Capital,
        Self::RocketSite,
        Self::GunEmplacement,
        Self::StrategicLocation,
        Self::AdjacencyRule,
        Self::Resource,
    ];

    pub fn suffix(self) -> &'static str {
        match self {
            Self::UnitsStack => "UNITS_STACK",
            Self::UnitsStackGroup => "UNITS_STACK_GROUP",
            Self::VictoryPoints => "VICTORY_POINTS",
            Self::AirBase => "AIR_BASE",
            Self::NavalBase => "NAVAL_BASE",
            Self::NavalHeadquarter => "NAVAL_HEADQUARTER",
            Self::Facility => "FACILITY",
            Self::LandCombat => "LAND_COMBAT",
            Self::NavalCombat => "NAVAL_COMBAT",
            Self::NavalCombatResults => "NAVAL_COMBAT_RESULTS",
            Self::NavalMission => "NAVAL_MISSION",
            Self::AirMission => "AIR_MISSION",
            Self::Capital => "CAPITAL",
            Self::RocketSite => "ROCKET_SITE",
            Self::GunEmplacement => "GUN_EMPLACEMENT",
            Self::StrategicLocation => "STRATEGIC_LOCATION",
            Self::AdjacencyRule => "ADJACENCY_RULE",
            Self::Resource => "RESOURCE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Priorities([[i8; PriorityIcon::ALL.len()]; PriorityTable::ALL.len()]);

impl Priorities {
    pub fn load(defines: &content_reader_defines::defines::Defines) -> std::io::Result<Self> {
        let mut tables = [[0; PriorityIcon::ALL.len()]; PriorityTable::ALL.len()];
        for (row, table) in tables.iter_mut().zip(PriorityTable::ALL) {
            for (value, icon) in row.iter_mut().zip(PriorityIcon::ALL) {
                let name = format!("{}_PRIORITY_{}", table.prefix(), icon.suffix());
                *value = priority(defines, &name)?;
            }
        }
        Ok(Self(tables))
    }

    pub const fn uniform(values: [i8; PriorityIcon::ALL.len()]) -> Self {
        Self([values; PriorityTable::ALL.len()])
    }

    pub fn get(&self, table: PriorityTable, icon: PriorityIcon) -> i8 {
        let table = PriorityTable::ALL.iter().position(|t| *t == table);
        let icon = PriorityIcon::ALL.iter().position(|i| *i == icon);
        self.0[table.expect("every table is listed")][icon.expect("every icon is listed")]
    }

    pub fn set(&mut self, table: PriorityTable, icon: PriorityIcon, value: i8) {
        let table = PriorityTable::ALL.iter().position(|t| *t == table);
        let icon = PriorityIcon::ALL.iter().position(|i| *i == icon);
        self.0[table.expect("every table is listed")][icon.expect("every icon is listed")] = value;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitKinds {
    All,
    CarrierFleets,
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RaidReach {
    Normal,
    RaidMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visibility {
    pub priorities: PriorityTable,
    pub political_mode: bool,
    pub unit_counters: bool,
    pub unit_kinds: UnitKinds,
    pub army_opacity: f32,
    pub strategic_grouping: bool,
    pub province_icons: bool,
    pub land_models: bool,
    pub unit_arrows: bool,
    pub label_text: bool,
    pub raid_icons: Option<RaidReach>,
    pub naval_missions: bool,
    pub faction_pins: bool,
    pub fog_of_war: bool,
    pub province_borders: bool,
    pub adjacency_rules: bool,
    pub trade_routes: bool,
    pub resource_icons: bool,
}

impl Visibility {
    pub const POLITICAL: Self = Self {
        priorities: PriorityTable::Default,
        political_mode: true,
        unit_counters: true,
        unit_kinds: UnitKinds::All,
        army_opacity: 1.0,
        strategic_grouping: false,
        province_icons: true,
        land_models: true,
        unit_arrows: true,
        label_text: true,
        raid_icons: Some(RaidReach::Normal),
        naval_missions: true,
        faction_pins: true,
        fog_of_war: true,
        province_borders: true,
        adjacency_rules: false,
        trade_routes: false,
        resource_icons: false,
    };

    pub const NAVY_ARMY_OPACITY: f32 = 0.5;
}

impl Default for Visibility {
    fn default() -> Self {
        Self::POLITICAL
    }
}

pub fn priority(defines: &content_reader_defines::defines::Defines, name: &str) -> std::io::Result<i8> {
    Ok(defines.int32("NMapIcons", name)? as i8)
}
