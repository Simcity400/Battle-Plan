//! What a map mode shows of the map's other layers.
//!
//! The game keeps these rules in its renderers rather than in content: the unit-icon test, the
//! per-province icon pass, the strategic-region icon pass, the label texts, the fog-of-war and
//! province-border fades, the trade-route pass and the map icon priority table it loads on every
//! mode switch. [`of`] gathers them for one mode; content only supplies the
//! priority values ([`PriorityTable`]).
//!
//! | mode | differences from the political mode |
//! |---|---|
//! | political | none |
//! | air | air priorities; no army or railway-gun counters, and at sea only fleets whose first carrier has an air base; strategic grouping distance; no naval mission icons; no fog of war |
//! | navy | navy priorities; army counters at half opacity; strategic grouping distance; adjacency-rule icons; trade routes |
//! | operatives | operative priorities; no unit counters but the port counters; no unit arrows; no faction pins; no fog of war |
//! | states | state priorities; no raid icons; no faction pins; no fog of war |
//! | supply | adjacency-rule icons; trade routes; no raid icons; no faction pins; no fog of war |
//! | resistance, compliance | resistance priorities; no raid icons; no faction pins |
//! | resources | resource priorities; no unit counters; label texts empty; adjacency-rule icons; trade routes; no province borders; no raid icons; no faction pins; no fog of war |
//! | diplomacy, factions, ideology | their own priorities; no raid icons; no faction pins; no fog of war |
//! | players | player priorities; no raid icons; no faction pins |
//! | infrastructure, manpower | their own priorities; no province borders; no raid icons; no faction pins; no fog of war |
//! | terrain | no raid icons; no faction pins; no fog of war |
//! | raids | state priorities; no unit counters; no victory points, bases, combats, radars, rocket sites or strategic locations; no land unit models or unit arrows; no naval mission icons; raid icons out to the raid mode's distance; no faction pins; no fog of war |
//! | scripted | state priorities; no raid icons; no faction pins; no fog of war |
//!
//! Faction pins show in the political, air and navy modes only. Raid icons show in the
//! political, air, navy, operatives and raids modes.

use viewer_foundation::map::mode_visibility::{PriorityTable, RaidReach, UnitKinds, Visibility};

use super::{MapMode, Stock};

/// What `mode` shows of the map's other layers.
pub fn of(mode: &MapMode) -> Visibility {
    let stock = match mode {
        MapMode::Stock(stock) => *stock,
        MapMode::Scripted(_) => return other(PriorityTable::States),
    };
    let base = Visibility::POLITICAL;
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

/// A mode outside the political, air, navy and operatives group: no raid icons, faction
/// pins or fog of war.
fn other(priorities: PriorityTable) -> Visibility {
    Visibility {
        priorities,
        raid_icons: None,
        faction_pins: false,
        fog_of_war: false,
        ..Visibility::POLITICAL
    }
}

#[cfg(test)]
mod tests;
