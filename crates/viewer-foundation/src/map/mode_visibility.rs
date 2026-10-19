//! What a map mode shows of the map's other layers, as plain values the counter, raid and world
//! passes read. The viewer host decides them from the view's map mode
//! (`viewer_world::map::map_mode::visibility`).

/// The `NMapIcons` priority table a mode's map icons draw with, by its define prefix.
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
    /// The define prefix: `<prefix>_PRIORITY_<ICON>`.
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

/// The map icon kinds whose priority a mode's table sets and the viewer draws, by their define
/// suffix.
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
    /// Strait and canal (adjacency-rule) icons.
    AdjacencyRule,
    /// A state's resource icon.
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

/// Every table's `NMapIcons.<table>_PRIORITY_<icon>` for the icons the viewer draws, each
/// truncated to the signed byte the game stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Priorities([[i8; PriorityIcon::ALL.len()]; PriorityTable::ALL.len()]);

impl Priorities {
    pub fn load(defines: &installed_content_core::defines::Defines) -> std::io::Result<Self> {
        let mut tables = [[0; PriorityIcon::ALL.len()]; PriorityTable::ALL.len()];
        for (row, table) in tables.iter_mut().zip(PriorityTable::ALL) {
            for (value, icon) in row.iter_mut().zip(PriorityIcon::ALL) {
                let name = format!("{}_PRIORITY_{}", table.prefix(), icon.suffix());
                *value = super::sprites::priority(defines, &name)?;
            }
        }
        Ok(Self(tables))
    }

    /// Every table holding `values` for each icon, in [`PriorityIcon::ALL`] order.
    pub const fn uniform(values: [i8; PriorityIcon::ALL.len()]) -> Self {
        Self([values; PriorityTable::ALL.len()])
    }

    pub fn get(&self, table: PriorityTable, icon: PriorityIcon) -> i8 {
        let table = PriorityTable::ALL.iter().position(|t| *t == table);
        let icon = PriorityIcon::ALL.iter().position(|i| *i == icon);
        self.0[table.expect("every table is listed")][icon.expect("every icon is listed")]
    }

    /// Sets one table's priority, for fixtures.
    pub fn set(&mut self, table: PriorityTable, icon: PriorityIcon, value: i8) {
        let table = PriorityTable::ALL.iter().position(|t| *t == table);
        let icon = PriorityIcon::ALL.iter().position(|i| *i == icon);
        self.0[table.expect("every table is listed")][icon.expect("every icon is listed")] = value;
    }
}

/// Which unit counters a mode lets the counter pass build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitKinds {
    /// Armies, railway guns and fleets.
    All,
    /// Only fleets with a ship that carries air wings.
    CarrierFleets,
    /// None.
    Nothing,
}

/// How far raid icons reach in a mode that shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RaidReach {
    /// Out to `RAID_MAP_ICON_DRAW_DISTANCE_MAX`.
    Normal,
    /// Out to `RAID_MAP_ICON_MAX_DRAW_DISTANCE_IN_RAID_MAP_MODE`.
    RaidMode,
}

/// What a map mode shows of the map's layers besides the political fill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visibility {
    pub priorities: PriorityTable,
    /// Native political mode, independently of a shared icon priority table.
    pub political_mode: bool,
    /// Whether unit counters are drawn at all; their camera cutoff and the peace conference
    /// still apply.
    pub unit_counters: bool,
    pub unit_kinds: UnitKinds,
    /// The opacity army counters are drawn at.
    pub army_opacity: f32,
    /// Whether unit counters group by country from `MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE_STRATEGIC`
    /// rather than `MAP_ICONS_COARSE_COUNTRY_GROUPING_DISTANCE`.
    pub strategic_grouping: bool,
    /// Whether a province shows its victory point (dot and name), air base, rocket site, naval
    /// base, radar and strategic-location icons and its land and naval combat and naval
    /// combat-result icons. Special facilities and naval headquarters always show.
    pub province_icons: bool,
    /// Whether 3-D land unit models are drawn; ships always are.
    pub land_models: bool,
    pub unit_arrows: bool,
    /// Whether victory-point and capital labels keep their names (their dots and stars stay).
    pub label_text: bool,
    /// `None` hides raid icons.
    pub raid_icons: Option<RaidReach>,
    pub naval_missions: bool,
    pub faction_pins: bool,
    /// Whether the fog of war is drawn, when the view has it switched on.
    pub fog_of_war: bool,
    pub province_borders: bool,
    /// Strait and canal (adjacency-rule) icons.
    pub adjacency_rules: bool,
    pub trade_routes: bool,
    /// Each state's resource icon (`resource_mapicon`).
    pub resource_icons: bool,
}

impl Visibility {
    /// The political mode's: everything.
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

    /// The navy mode's army counter opacity.
    pub const NAVY_ARMY_OPACITY: f32 = 0.5;
}

impl Default for Visibility {
    fn default() -> Self {
        Self::POLITICAL
    }
}
