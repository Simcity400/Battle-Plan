use super::*;

fn of(stock: Stock) -> Visibility {
    super::of(&MapMode::Stock(stock))
}

fn modes_where(test: impl Fn(&Visibility) -> bool) -> Vec<Stock> {
    Stock::ALL
        .into_iter()
        .filter(|stock| test(&of(*stock)))
        .collect()
}

#[test]
fn each_mode_loads_the_games_priority_table() {
    use PriorityTable as T;
    let expected = [
        (Stock::Default, T::Default),
        (Stock::StrategicAir, T::StrategicAir),
        (Stock::StrategicNavy, T::StrategicNavy),
        (Stock::Operatives, T::Operatives),
        (Stock::States, T::States),
        (Stock::Supply, T::Default),
        (Stock::Resistance, T::Resistance),
        (Stock::Compliance, T::Resistance),
        (Stock::Resources, T::Resources),
        (Stock::Diplomacy, T::Diplomacy),
        (Stock::Factions, T::Factions),
        (Stock::Players, T::Players),
        (Stock::Infrastructure, T::Infrastructure),
        (Stock::Manpower, T::Manpower),
        (Stock::Ideology, T::Ideology),
        (Stock::Terrain, T::Default),
        (Stock::Raids, T::States),
    ];
    assert_eq!(expected.len(), Stock::ALL.len());
    for (stock, table) in expected {
        assert_eq!(of(stock).priorities, table, "{stock:?}");
    }
    assert_eq!(
        super::of(&MapMode::Scripted("any".into())).priorities,
        T::States
    );
    assert_eq!(T::StrategicNavy.prefix(), "STRATEGIC_NAVY");
}

#[test]
fn the_game_hides_unit_counters_in_the_resource_and_raid_modes() {
    assert_eq!(
        modes_where(|v| !v.unit_counters),
        [Stock::Resources, Stock::Raids]
    );
    assert_eq!(
        modes_where(|v| v.unit_kinds == UnitKinds::CarrierFleets),
        [Stock::StrategicAir]
    );
    assert_eq!(
        modes_where(|v| v.unit_kinds == UnitKinds::Nothing),
        [Stock::Operatives]
    );
    assert_eq!(
        modes_where(|v| v.army_opacity != 1.0),
        [Stock::StrategicNavy]
    );
    assert_eq!(of(Stock::StrategicNavy).army_opacity, 0.5);
    assert_eq!(
        modes_where(|v| v.strategic_grouping),
        [Stock::StrategicAir, Stock::StrategicNavy]
    );
}

#[test]
fn the_raid_mode_clears_province_icons_and_reaches_further_with_raid_icons() {
    assert_eq!(modes_where(|v| !v.province_icons), [Stock::Raids]);
    assert_eq!(modes_where(|v| !v.land_models), [Stock::Raids]);
    assert_eq!(
        modes_where(|v| !v.unit_arrows),
        [Stock::Operatives, Stock::Raids]
    );
    assert_eq!(
        modes_where(|v| v.raid_icons.is_some()),
        [
            Stock::Default,
            Stock::StrategicAir,
            Stock::StrategicNavy,
            Stock::Operatives,
            Stock::Raids
        ]
    );
    assert_eq!(of(Stock::Raids).raid_icons, Some(RaidReach::RaidMode));
    assert_eq!(of(Stock::Default).raid_icons, Some(RaidReach::Normal));
    assert_eq!(
        modes_where(|v| !v.naval_missions),
        [Stock::StrategicAir, Stock::Raids]
    );
    assert_eq!(
        modes_where(|v| v.faction_pins),
        [Stock::Default, Stock::StrategicAir, Stock::StrategicNavy]
    );
}

#[test]
fn fog_borders_labels_routes_and_straits_follow_the_games_mode_lists() {
    assert_eq!(
        modes_where(|v| v.fog_of_war),
        [
            Stock::Default,
            Stock::StrategicNavy,
            Stock::Resistance,
            Stock::Compliance,
            Stock::Players
        ]
    );
    assert_eq!(
        modes_where(|v| !v.province_borders),
        [Stock::Resources, Stock::Infrastructure, Stock::Manpower]
    );
    assert_eq!(modes_where(|v| !v.label_text), [Stock::Resources]);
    let routes = [Stock::StrategicNavy, Stock::Supply, Stock::Resources];
    assert_eq!(modes_where(|v| v.trade_routes), routes);
    assert_eq!(modes_where(|v| v.adjacency_rules), routes);
    assert_eq!(modes_where(|v| v.resource_icons), [Stock::Resources]);
    assert!(!super::of(&MapMode::Scripted("any".into())).resource_icons);
}

#[test]
fn scripted_modes_take_the_rules_of_a_mode_outside_the_political_group() {
    let scripted = super::of(&MapMode::Scripted("any".into()));
    assert_eq!(scripted, of(Stock::States));
    assert!(scripted.unit_counters && scripted.province_icons && scripted.label_text);
    assert!(scripted.raid_icons.is_none() && !scripted.fog_of_war && !scripted.faction_pins);
}

#[test]
fn political_port_policy_is_independent_of_the_icon_priority_table() {
    assert!(of(Stock::Default).political_mode);
    for stock in Stock::ALL {
        assert_eq!(
            of(stock).political_mode,
            stock == Stock::Default,
            "{stock:?}"
        );
    }
    assert!(!super::of(&MapMode::Scripted("synthetic".into())).political_mode);
}
