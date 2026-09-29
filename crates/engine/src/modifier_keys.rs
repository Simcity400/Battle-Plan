//! Built-in modifier indices consumed by fixed-key readers and playback.
//!
//! `modifier_definition::Register@14054A990`; each entry cites a callsite.
//! The token global at that site resolves through the pinned lexer registrar
//! (`142057ED0`, then `CPdxLexer::Init@1424A87E0`). These are executable facts,
//! independent of content order. Dynamic keys must still come from live data.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub index: u32,
    pub name: &'static str,
}

macro_rules! keys {
    ($($(#[$doc:meta])* $constant:ident = $index:literal => $name:literal;)+) => {
        $($(#[$doc])* pub const $constant: u32 = $index;)+
        pub const ALL: &[Key] = &[$(Key { index: $constant, name: $name }),+];
    };
}

keys! {
    /// Register@14058ba2a (token global 0x1427bff98).
    AIR_MAXIMUM_SPEED_FACTOR = 0x2 => "air_maximum_speed_factor";
    /// Register@14058cabe (token global 0x1427bff9c).
    AIR_RANGE_FACTOR = 0x5 => "air_range_factor";
    /// Register@1405cad3e (token global 0x1427c00e0).
    INVASION_PREPARATION = 0x1a => "invasion_preparation";
    /// Register@1405cc61c (token global 0x1427c058c).
    NAVAL_COMMANDO_RAID_DISTANCE = 0x21 => "naval_commando_raid_distance";
    /// Register@1405cd28b (token global 0x1427bfee8).
    CARRIER_TRAFFIC = 0x22 => "carrier_traffic";
    /// Register@1405908e9 (token global 0x1427bfcec).
    CARRIER_NIGHT_TRAFFIC = 0x23 => "carrier_night_traffic";
    /// Register@1405ce744 (token global 0x1427bfed8).
    NAVAL_SPEED_FACTOR = 0x29 => "naval_speed_factor";
    /// Register@1405ac460 (token global 0x1427bff64).
    ARMY_SPEED_FACTOR = 0x2b => "army_speed_factor";
    /// Register@14059e8b7 (token global 0x1427bff68).
    ARMY_ARMOR_SPEED_FACTOR = 0x2c => "army_armor_speed_factor";
    /// Register@1405c350d (token global 0x1427bff04).
    SUBMARINE_ATTACK = 0x38 => "submarine_attack";
    /// Register@1405d3a28 (token global 0x1427bff34).
    NAVY_ORG = 0x46 => "navy_org";
    /// Register@1405d3e4d (token global 0x1427bff38).
    NAVY_ORG_FACTOR = 0x47 => "navy_org_factor";
    /// Register@140569722 (token global 0x1427bfef8).
    LOCAL_INTEL_TO_ENEMIES = 0x4b => "local_intel_to_enemies";
    /// Register@1405b40b6 (token global 0x1427c0228).
    UNIT_UPKEEP_ATTRITION_FACTOR = 0x54 => "unit_upkeep_attrition_factor";
    /// Register@1405d086c (token global 0x1427bfce8).
    NAVAL_ENEMY_POSITIONING_IN_INITIAL_ATTACK = 0x5a => "naval_enemy_positioning_in_initial_attack";
    /// Register@1405aa75d (token global 0x1427bff08).
    OUT_OF_SUPPLY_FACTOR = 0x5e => "out_of_supply_factor";
    /// Register@1405aafa7 (token global 0x1427bff14).
    OFFENCE = 0x5f => "offence";
    /// Register@1405ab3cc (token global 0x1427bff18).
    DEFENCE = 0x60 => "defence";
    /// Register@14058d308 (token global 0x1427c0498).
    AIR_ACE_BONUSES_FACTOR = 0x72 => "air_ace_bonuses_factor";
    /// Register@140562322 (token global 0x1427c0524).
    POWER_BALANCE_DAILY = 0x98 => "power_balance_daily";
    /// Register@140562747 (token global 0x1427c0528).
    POWER_BALANCE_WEEKLY = 0x99 => "power_balance_weekly";
    /// Register@1405c3932 (token global 0x1427c001c).
    NAVY_SUBMARINE_ATTACK_FACTOR = 0xc2 => "navy_submarine_attack_factor";
    /// Register@1405c3d57 (token global 0x1427c0020).
    NAVY_SUBMARINE_DEFENCE_FACTOR = 0xc3 => "navy_submarine_defence_factor";
    /// Register@1405c417c (token global 0x1427c0024).
    NAVY_CAPITAL_SHIP_ATTACK_FACTOR = 0xc4 => "navy_capital_ship_attack_factor";
    /// Register@1405c45a1 (token global 0x1427c0028).
    NAVY_CAPITAL_SHIP_DEFENCE_FACTOR = 0xc5 => "navy_capital_ship_defence_factor";
    /// Register@1405c49c6 (token global 0x1427c002c).
    NAVY_SCREEN_ATTACK_FACTOR = 0xc6 => "navy_screen_attack_factor";
    /// Register@1405c4deb (token global 0x1427c0030).
    NAVY_SCREEN_DEFENCE_FACTOR = 0xc7 => "navy_screen_defence_factor";
    /// Register@140553467 (token global 0x1427c0050).
    RESEARCH_SPEED_FACTOR = 0xcd => "research_speed_factor";
    /// Register@140553c9f (token global 0x1427c0168).
    RESEARCH_SHARING_PER_COUNTRY_BONUS_FACTOR = 0xcf => "research_sharing_per_country_bonus_factor";
    /// Register@14056c4b9 (token global 0x1427c0678).
    LAND_REINFORCE_RATE = 0xfb => "land_reinforce_rate";
    /// Register@14058f855 (token global 0x1427bfcd8).
    NAVAL_NIGHT_ATTACK = 0x107 => "naval_night_attack";
    /// Register@1405a589e (token global 0x1427c00d8).
    COMBAT_WIDTH_FACTOR = 0x10f => "combat_width_factor";
    /// Register@140594b39 (token global 0x1427bfb64).
    MAX_COMMAND_POWER = 0x14b => "max_command_power";
    /// Register@140595383 (token global 0x1427bfb6c).
    MAX_COMMAND_POWER_MULT = 0x14d => "max_command_power_mult";
    /// Register@140592a11 (token global 0x1427bfbdc).
    RIVER_CROSSING_FACTOR = 0x16c => "river_crossing_factor";
    /// Register@140592e36 (token global 0x1427c064c).
    RIVER_CROSSING_FACTOR_AGAINST = 0x16d => "river_crossing_factor_against";
    /// Register@1405aab82 (token global 0x1427c0218).
    SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR = 0x17f => "special_forces_out_of_supply_factor";
    /// Register@1405c0fe4 (token global 0x1427bfc1c).
    NAVAL_INVASION_PREP_DAYS = 0x184 => "naval_invasion_prep_days";
    /// Register@140598d89 (token global 0x1427c02a8).
    NAVAL_MINES_EFFECT_REDUCTION = 0x1a7 => "naval_mines_effect_reduction";
    /// Register@1405d4272 (token global 0x1427bfc78).
    SCREENING_EFFICIENCY = 0x1b1 => "screening_efficiency";
    /// Register@1405d4697 (token global 0x1427bfea4).
    POSITIONING = 0x1b2 => "positioning";
    /// Register@1405d4abc (token global 0x1427bfc88).
    SCREENING_WITHOUT_SCREENS = 0x1b3 => "screening_without_screens";
    /// Register@1405d7853 (token global 0x1427c02d8).
    NAVAL_DAMAGE_FACTOR = 0x1b7 => "naval_damage_factor";
    /// Register@1405d7c78 (token global 0x1427c02dc).
    NAVAL_DEFENSE_FACTOR = 0x1b8 => "naval_defense_factor";
    /// Register@1405d88e7 (token global 0x1427c02e8).
    NAVAL_CRITICAL_EFFECT_FACTOR = 0x1bb => "naval_critical_effect_factor";
    /// Register@1405c5a5a (token global 0x1427c05c4).
    NAVY_CAPITAL_SHIP_ATTACK_FACTOR_AGAINST = 0x1c8 => "navy_capital_ship_attack_factor_against";
    /// Register@1405c5e7f (token global 0x1427c05c8).
    NAVY_CAPITAL_SHIP_DEFENCE_FACTOR_AGAINST = 0x1c9 => "navy_capital_ship_defence_factor_against";
    /// Register@1405c62a4 (token global 0x1427c05cc).
    NAVY_SCREEN_ATTACK_FACTOR_AGAINST = 0x1ca => "navy_screen_attack_factor_against";
    /// Register@1405c66c9 (token global 0x1427c05d0).
    NAVY_SCREEN_DEFENCE_FACTOR_AGAINST = 0x1cb => "navy_screen_defence_factor_against";
    /// Register@1405c6aee (token global 0x1427c05d4).
    NAVY_SUBMARINE_ATTACK_FACTOR_AGAINST = 0x1cc => "navy_submarine_attack_factor_against";
    /// Register@1405c6f13 (token global 0x1427c05d8).
    NAVY_SUBMARINE_DEFENCE_FACTOR_AGAINST = 0x1cd => "navy_submarine_defence_factor_against";
    /// Register@1405de83a (token global 0x1427c05c0).
    INVASION_PREPARATION_AGAINST = 0x1ce => "invasion_preparation_against";
    /// Register@1405dae34 (token global 0x1427c030c).
    NAVAL_ENEMY_FLEET_SIZE_RATIO_PENALTY_FACTOR = 0x1d0 => "naval_enemy_fleet_size_ratio_penalty_factor";
    /// Register@1405db259 (token global 0x1427c0310).
    NAVAL_HAS_POTF_IN_COMBAT_ATTACK = 0x1d1 => "naval_has_potf_in_combat_attack";
    /// Register@1405db67e (token global 0x1427c0314).
    NAVAL_HAS_POTF_IN_COMBAT_DEFENSE = 0x1d2 => "naval_has_potf_in_combat_defense";
    /// Register@1405accaa (token global 0x1427c03e0).
    ARMY_SPEED_FACTOR_FOR_CONTROLLER = 0x1e7 => "army_speed_factor_for_controller";
    /// Register@1405ad0cf (token global 0x1427c0530).
    ENEMY_ARMY_SPEED_FACTOR = 0x1e8 => "enemy_army_speed_factor";
    /// Register@1405ac885 (token global 0x1427bfcd4).
    ARMY_ATTACK_SPEED_FACTOR = 0x1e9 => "army_attack_speed_factor";
    /// Register@14057306f (token global 0x1427c03d0).
    DISABLE_STRATEGIC_REDEPLOYMENT = 0x1ea => "disable_strategic_redeployment";
    /// Register@140573494 (token global 0x1427c03d4).
    DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER = 0x1eb => "disable_strategic_redeployment_for_controller";
    /// Register@14057b50f (token global 0x1427c0440).
    STATE_RESOURCES_FACTOR = 0x223 => "state_resources_factor";
    /// Register@1405dc712 (token global 0x1427bfc94).
    NAVY_WEATHER_PENALTY = 0x248 => "navy_weather_penalty";
    /// Register@1405edcb5 (token global 0x1427c04e0).
    INTEL_FROM_COMBAT_FACTOR = 0x24b => "intel_from_combat_factor";
    /// Register@1405f8ee0 (token global 0x1427c04f0).
    INITIATIVE_FACTOR = 0x250 => "initiative_factor";
    /// Register@1405cca41 (token global 0x1427c0518).
    ACE_EFFECTIVENESS_FACTOR = 0x25a => "ace_effectiveness_factor";
    /// Register@1405fbc77 (token global 0x1427c0660).
    SCIENTIST_RESEARCH_BONUS_FACTOR = 0x27a => "scientist_research_bonus_factor";
    /// Register@1405d1d25 (token global 0x1427bfb10).
    MAX_ORGANISATION = 0x27e => "max_organisation";
    /// Register@1405fd97a (token global 0x1427c05dc).
    ARMY_RETREAT_SPEED_FACTOR = 0x286 => "army_retreat_speed_factor";
    /// Register@140586321 (token global 0x1427c060c).
    LOCAL_RESOURCE_GAIN_EFFICIENCY_PER_INFRASTRUCTURE = 0x292 => "local_resource_gain_efficiency_per_infrastructure";
    /// Register@1405ff258 (token global 0x1427bfb70).
    CANNOT_RETREAT_WHILE_DEFENDING = 0x297 => "cannot_retreat_while_defending";
    /// Register@1405ff67d (token global 0x1427bfb74).
    CANNOT_RETREAT_WHILE_ATTACKING = 0x298 => "cannot_retreat_while_attacking";
}
