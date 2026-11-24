
use viewer_foundation::mode_visibility::{PriorityTable, RaidReach, UnitKinds, Visibility};

use super::{MapMode, Stock};

pub fn of(mode: &MapMode) -> Visibility {
    let stock = match mode {
        MapMode::Stock(stock) => *stock,
        MapMode::Scripted(_) => return other(PriorityTable::States),
    };
    let base = Visibility {
        political_mode: stock == Stock::Default,
        ..Visibility::POLITICAL
    };
    match stock {
        Stock::Default => base,
        Stock::StrategicAir => Visibility {
            priorities: PriorityTable::StrategicAir,
            unit_kinds: UnitKinds::CarrierFleets,
            strategic_grouping: true,
            naval_missions: false,
            fog_of_war: false,
            ..base
        },
        Stock::StrategicNavy => Visibility {
            priorities: PriorityTable::StrategicNavy,
            army_opacity: Visibility::NAVY_ARMY_OPACITY,
            strategic_grouping: true,
            adjacency_rules: true,
            trade_routes: true,
            ..base
        },
        Stock::Operatives => Visibility {
            priorities: PriorityTable::Operatives,
            unit_kinds: UnitKinds::Nothing,
            unit_arrows: false,
            faction_pins: false,
            fog_of_war: false,
            ..base
        },
        Stock::States => other(PriorityTable::States),
        Stock::Supply => Visibility {
            adjacency_rules: true,
            trade_routes: true,
            ..other(PriorityTable::Default)
        },
        Stock::Resistance | Stock::Compliance => Visibility {
            fog_of_war: true,
            ..other(PriorityTable::Resistance)
        },
        Stock::Resources => Visibility {
            unit_counters: false,
            label_text: false,
            province_borders: false,
            adjacency_rules: true,
            trade_routes: true,
            resource_icons: true,
            ..other(PriorityTable::Resources)
        },
        Stock::Diplomacy => other(PriorityTable::Diplomacy),
        Stock::Factions => other(PriorityTable::Factions),
        Stock::Players => Visibility {
            fog_of_war: true,
            ..other(PriorityTable::Players)
        },
        Stock::Infrastructure => Visibility {
            province_borders: false,
            ..other(PriorityTable::Infrastructure)
        },
        Stock::Manpower => Visibility {
            province_borders: false,
            ..other(PriorityTable::Manpower)
        },
        Stock::Ideology => other(PriorityTable::Ideology),
        Stock::Terrain => other(PriorityTable::Default),
        Stock::Raids => Visibility {
            unit_counters: false,
            province_icons: false,
            land_models: false,
            unit_arrows: false,
            raid_icons: Some(RaidReach::RaidMode),
            naval_missions: false,
            ..other(PriorityTable::States)
        },
    }
}

fn other(priorities: PriorityTable) -> Visibility {
    Visibility {
        priorities,
        political_mode: false,
        raid_icons: None,
        faction_pins: false,
        fog_of_war: false,
        ..Visibility::POLITICAL
    }
}

