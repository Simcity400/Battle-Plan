//! Built-in modifier indices consumed by fixed-key readers and playback.
//!
//! `modifier_definition::Register@14054A990`; each entry cites a callsite.
//! Name/id pairs come from the generated token registrar `140082CB0`;
//! `CPdxLexer::Init@1424A87E0` seeds its dictionary through the static ordinal
//! getter `142057ED0`. These facts are independent of content order. Dynamic
//! keys must still come from live data.

mod builtins;
pub use builtins::NAMES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub index: u32,
    pub name: &'static str,
}

macro_rules! keys {
    ($($(#[$doc:meta])* $constant:ident = $index:literal;)+) => {
        $($(#[$doc])* pub const $constant: u32 = $index;)+
        pub const ALL: &[Key] = &[$(Key { index: $constant, name: NAMES[$index] }),+];
    };
}

keys! {
    /// Register@14058ba2a (token global 0x1427bff98).
    AIR_MAXIMUM_SPEED_FACTOR = 0x2;
    /// Register@14058cabe (token global 0x1427bff9c).
    AIR_RANGE_FACTOR = 0x5;
    /// Register@14058AB21 (token global 0x1427bff80).
    AIR_STRATEGIC_BOMBER_BOMBING_FACTOR = 0xa;
    /// Register@1405d3603 (token global 0x1427c00d0).
    NAVAL_COORDINATION = 0x19;
    /// Register@1405cad3e (token global 0x1427c00e0).
    INVASION_PREPARATION = 0x1a;
    /// Register@1405cc61c (token global 0x1427c058c).
    NAVAL_COMMANDO_RAID_DISTANCE = 0x21;
    /// Register@1405cd28b (token global 0x1427bfee8).
    CARRIER_TRAFFIC = 0x22;
    /// Register@1405908e9 (token global 0x1427bfcec).
    CARRIER_NIGHT_TRAFFIC = 0x23;
    /// Register@1405cd6b0 (token global 0x1427c0018).
    SORTIE_EFFICIENCY = 0x24;
    /// Register@1405ce744 (token global 0x1427bfed8).
    NAVAL_SPEED_FACTOR = 0x29;
    /// Register@1405ceb69 (token global 0x1427bfedc).
    NAVAL_DETECTION = 0x2a;
    /// Register@1405ac460 (token global 0x1427bff64).
    ARMY_SPEED_FACTOR = 0x2b;
    /// Register@14059e8b7 (token global 0x1427bff68).
    ARMY_ARMOR_SPEED_FACTOR = 0x2c;
    /// Register@1405c350d (token global 0x1427bff04).
    SUBMARINE_ATTACK = 0x38;
    /// Register@1405d3a28 (token global 0x1427bff34).
    NAVY_ORG = 0x46;
    /// Register@1405d3e4d (token global 0x1427bff38).
    NAVY_ORG_FACTOR = 0x47;
    /// Register@140569722 (token global 0x1427bfef8).
    LOCAL_INTEL_TO_ENEMIES = 0x4b;
    /// Register@1405A7DEB; fresh own-army attrition 140C65DC0.
    ATTRITION = 0x4f;
    /// Register@1405A96C9; fresh weather branch 140C65DC0.
    WINTER_ATTRITION_FACTOR = 0x50;
    /// Register@1405A9AEE; fresh weather branch 140C65DC0.
    WINTER_ATTRITION = 0x51;
    /// Register@1405AA338; fresh weather branch 140C65DC0.
    HEAT_ATTRITION = 0x52;
    /// Register@1405A9F13; fresh weather branch 140C65DC0.
    HEAT_ATTRITION_FACTOR = 0x53;
    /// Register@1405b40b6 (token global 0x1427c0228).
    UNIT_UPKEEP_ATTRITION_FACTOR = 0x54;
    /// Register@1405D0447; selected by escape chance 14195C7D0.
    NAVAL_RETREAT_SPEED_AFTER_INITIAL_COMBAT = 0x59;
    /// Register@1405d086c (token global 0x1427bfce8).
    NAVAL_ENEMY_POSITIONING_IN_INITIAL_ATTACK = 0x5a;
    /// Register@1405D0022; selected by escape chance 14195C7D0.
    NAVAL_RETREAT_SPEED = 0x5b;
    /// Register@1405D10B6; selected by escape chance 14195C7D0.
    SUB_RETREAT_SPEED = 0x5c;
    /// Register@1405D0C91; selected by escape chance 14195C7D0.
    CONVOY_RETREAT_SPEED = 0x5d;
    /// Register@1405aa75d (token global 0x1427bff08).
    OUT_OF_SUPPLY_FACTOR = 0x5e;
    /// Register@14058B36B (token global 0x1427bffa8).
    AIR_DEFENCE_FACTOR = 0x70;
    /// Register@14058BBB5 (token global 0x1427bffac).
    AIR_AGILITY_FACTOR = 0x71;
    /// Register@14058d308 (token global 0x1427c0498).
    AIR_ACE_BONUSES_FACTOR = 0x72;
    /// Register@14058D8B8 (token global 0x1427c04a8).
    AIR_HOME_DEFENCE_FACTOR = 0x74;
    /// Register@14058DCDD (token global 0x1427c04ac).
    AIR_POWER_PROJECTION_FACTOR = 0x75;
    /// Register@14058ED71 (token global 0x1427bff7c).
    AIR_STRATEGIC_BOMBER_DEFENCE_FACTOR = 0x76;
    /// Register@1405c903b (token global 0x1427bffb0).
    NAVY_ANTI_AIR_ATTACK = 0x88;
    /// Register@1405c9460 (token global 0x1427bffb4).
    NAVY_ANTI_AIR_ATTACK_FACTOR = 0x89;
    /// Register@1405C9885; built-in navy_max_range.
    NAVAL_RANGE = 0x8a;
    /// Register@1405C9CAA; built-in navy_max_range_factor.
    NAVAL_RANGE_FACTOR = 0x8b;
    /// Register@140562322 (token global 0x1427c0524).
    POWER_BALANCE_DAILY = 0x98;
    /// Register@140562747 (token global 0x1427c0528).
    POWER_BALANCE_WEEKLY = 0x99;
    /// Register@1405c3932 (token global 0x1427c001c).
    NAVY_SUBMARINE_ATTACK_FACTOR = 0xc2;
    /// Register@1405c3d57 (token global 0x1427c0020).
    NAVY_SUBMARINE_DEFENCE_FACTOR = 0xc3;
    /// Register@1405c417c (token global 0x1427c0024).
    NAVY_CAPITAL_SHIP_ATTACK_FACTOR = 0xc4;
    /// Register@1405c45a1 (token global 0x1427c0028).
    NAVY_CAPITAL_SHIP_DEFENCE_FACTOR = 0xc5;
    /// Register@1405c49c6 (token global 0x1427c002c).
    NAVY_SCREEN_ATTACK_FACTOR = 0xc6;
    /// Register@1405c4deb (token global 0x1427c0030).
    NAVY_SCREEN_DEFENCE_FACTOR = 0xc7;
    /// Register@1405c7fa7 (token global 0x1427c0044).
    CONVOY_RAIDING_EFFICIENCY_FACTOR = 0xcc;
    /// Register@140553467 (token global 0x1427c0050).
    RESEARCH_SPEED_FACTOR = 0xcd;
    /// Register@140553c9f (token global 0x1427c0168).
    RESEARCH_SHARING_PER_COUNTRY_BONUS_FACTOR = 0xcf;
    /// Register@14056c4b9 (token global 0x1427c0678).
    LAND_REINFORCE_RATE = 0xfb;
    /// Register@14058F196 (token global 0x1427c00e8).
    AIR_STRATEGIC_BOMBER_NIGHT_PENALTY = 0x104;
    /// Register@14058FE05 (token global 0x1427c00ec).
    AIR_NIGHT_PENALTY = 0x105;
    /// Register@140590A74 (token global 0x1427c00f0).
    AIR_WEATHER_PENALTY = 0x106;
    /// Register@14058f855 (token global 0x1427bfcd8).
    NAVAL_NIGHT_ATTACK = 0x107;
    /// Register@14059022A (token global 0x1427c049c).
    AIR_CARRIER_NIGHT_PENALTY_REDUCTION_FACTOR = 0x10d;
    /// Register@1405c8c16 (token global 0x1427c0690).
    CONVOY_ESCORT_EFFICIENCY = 0x10e;
    /// Register@1405a589e (token global 0x1427c00d8).
    COMBAT_WIDTH_FACTOR = 0x10f;
    /// Register@140594b39 (token global 0x1427bfb64).
    MAX_COMMAND_POWER = 0x14b;
    /// Register@140595383 (token global 0x1427bfb6c).
    MAX_COMMAND_POWER_MULT = 0x14d;
    /// Register@140592a11 (token global 0x1427bfbdc).
    RIVER_CROSSING_FACTOR = 0x16c;
    /// Register@140592e36 (token global 0x1427c064c).
    RIVER_CROSSING_FACTOR_AGAINST = 0x16d;
    /// Register@1405aab82 (token global 0x1427c0218).
    SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR = 0x17f;
    /// Register@1405c0fe4 (token global 0x1427bfc1c).
    NAVAL_INVASION_PREP_DAYS = 0x184;
    /// Register@140598d89 (token global 0x1427c02a8).
    NAVAL_MINES_EFFECT_REDUCTION = 0x1a7;
    /// Register@1405d4272 (token global 0x1427bfc78).
    SCREENING_EFFICIENCY = 0x1b1;
    /// Register@1405d4697 (token global 0x1427bfea4).
    POSITIONING = 0x1b2;
    /// Register@1405d4abc (token global 0x1427bfc88).
    SCREENING_WITHOUT_SCREENS = 0x1b3;
    /// Register@1405cdefa (token global 0x1427bfc74).
    FIGHTER_SORTIE_EFFICIENCY = 0x1b6;
    /// Register@1405d7853 (token global 0x1427c02d8).
    NAVAL_DAMAGE_FACTOR = 0x1b7;
    /// Register@1405d7c78 (token global 0x1427c02dc).
    NAVAL_DEFENSE_FACTOR = 0x1b8;
    /// Register@1405d88e7 (token global 0x1427c02e8).
    NAVAL_CRITICAL_EFFECT_FACTOR = 0x1bb;
    /// Register@1405c5a5a (token global 0x1427c05c4).
    NAVY_CAPITAL_SHIP_ATTACK_FACTOR_AGAINST = 0x1c8;
    /// Register@1405c5e7f (token global 0x1427c05c8).
    NAVY_CAPITAL_SHIP_DEFENCE_FACTOR_AGAINST = 0x1c9;
    /// Register@1405c62a4 (token global 0x1427c05cc).
    NAVY_SCREEN_ATTACK_FACTOR_AGAINST = 0x1ca;
    /// Register@1405c66c9 (token global 0x1427c05d0).
    NAVY_SCREEN_DEFENCE_FACTOR_AGAINST = 0x1cb;
    /// Register@1405c6aee (token global 0x1427c05d4).
    NAVY_SUBMARINE_ATTACK_FACTOR_AGAINST = 0x1cc;
    /// Register@1405c6f13 (token global 0x1427c05d8).
    NAVY_SUBMARINE_DEFENCE_FACTOR_AGAINST = 0x1cd;
    /// Register@1405de83a (token global 0x1427c05c0).
    INVASION_PREPARATION_AGAINST = 0x1ce;
    /// Register@1405dae34 (token global 0x1427c030c).
    NAVAL_ENEMY_FLEET_SIZE_RATIO_PENALTY_FACTOR = 0x1d0;
    /// Register@1405db259 (token global 0x1427c0310).
    NAVAL_HAS_POTF_IN_COMBAT_ATTACK = 0x1d1;
    /// Register@1405db67e (token global 0x1427c0314).
    NAVAL_HAS_POTF_IN_COMBAT_DEFENSE = 0x1d2;
    /// Register@1405accaa (token global 0x1427c03e0).
    ARMY_SPEED_FACTOR_FOR_CONTROLLER = 0x1e7;
    /// Register@1405ad0cf (token global 0x1427c0530).
    ENEMY_ARMY_SPEED_FACTOR = 0x1e8;
    /// Register@1405ac885 (token global 0x1427bfcd4).
    ARMY_ATTACK_SPEED_FACTOR = 0x1e9;
    /// Register@14057306f (token global 0x1427c03d0).
    DISABLE_STRATEGIC_REDEPLOYMENT = 0x1ea;
    /// Register@140573494 (token global 0x1427c03d4).
    DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER = 0x1eb;
    /// Register@1405A8210; fresh state branch 140C65DC0.
    ATTRITION_FOR_CONTROLLER = 0x1ee;
    /// Register@1405A8635; fresh state branch 140C65DC0.
    ENEMY_ATTRITION = 0x1ef;
    /// Register@1405EC993 (token global 0x1427C0348); `1411DED90/140C07140`.
    INTELLIGENCE_AGENCY_DEFENSE = 0x1ff;
    /// Register@1405F1C6B (token global 0x1427C0388); `1413E3350`.
    CRYPTO_STRENGTH = 0x210;
    /// Register@14057b50f (token global 0x1427c0440).
    STATE_RESOURCES_FACTOR = 0x223;
    /// Register@1405dc712 (token global 0x1427bfc94).
    NAVY_WEATHER_PENALTY = 0x248;
    /// Register@1405edcb5 (token global 0x1427c04e0).
    INTEL_FROM_COMBAT_FACTOR = 0x24b;
    /// Register@1405f8ee0 (token global 0x1427c04f0).
    INITIATIVE_FACTOR = 0x250;
    /// Register@1405cca41 (token global 0x1427c0518).
    ACE_EFFECTIVENESS_FACTOR = 0x25a;
    /// Register@1405fbc77 (token global 0x1427c0660).
    SCIENTIST_RESEARCH_BONUS_FACTOR = 0x27a;
    /// Register@1405DCB37; built-in underway_replenishment_range.
    UNDERWAY_REPLENISHMENT_RANGE_FACTOR = 0x27c;
    /// Register@1405d1d25 (token global 0x1427bfb10).
    MAX_ORGANISATION = 0x27e;
    /// Register@1405fd97a (token global 0x1427c05dc).
    ARMY_RETREAT_SPEED_FACTOR = 0x286;
    /// Register@140586321 (token global 0x1427c060c).
    LOCAL_RESOURCE_GAIN_EFFICIENCY_PER_INFRASTRUCTURE = 0x292;
    /// Register@1405FE1C4; fresh own-counter dig-in getter 1418BC515.
    COMBAT_ENTRENCHMENT = 0x293;
    /// Register@1405ff258 (token global 0x1427bfb70).
    CANNOT_RETREAT_WHILE_DEFENDING = 0x297;
    /// Register@1405ff67d (token global 0x1427bfb74).
    CANNOT_RETREAT_WHILE_ATTACKING = 0x298;
}
