//! Built-in modifier indices consumed by fixed-key readers and playback.
//!
//! Fixed names come from verified executable metadata, independently of content
//! order. Dynamic keys come from live data.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub index: u32,
    pub name: &'static str,
}

macro_rules! keys {
    ($($(#[$doc:meta])* $constant:ident = $index:literal;)+) => {
        $($(#[$doc])* pub const $constant: u32 = $index;)+
        pub const ALL_INDICES: &[u32] = &[$($constant),+];
    };
}

pub fn names() -> &'static [&'static str] {
    &crate::executable_tables::get().keywords.modifier_names
}

pub fn name(index: u32) -> Option<&'static str> {
    names().get(index as usize).copied()
}

pub fn all() -> impl Iterator<Item = Key> {
    ALL_INDICES.iter().map(|&index| Key {
        index,
        name: name(index).expect("fixed modifier index is present in verified metadata"),
    })
}

keys! {

    AIR_MAXIMUM_SPEED_FACTOR = 0x2;

    AIR_RANGE_FACTOR = 0x5;

    AIR_STRATEGIC_BOMBER_BOMBING_FACTOR = 0xa;
    /// Current air-superiority mission branch.
    AIR_SUPERIORITY_EFFICIENCY = 0xb;
    /// Current interception mission branch.
    AIR_INTERCEPT_EFFICIENCY = 0xd;
    /// Preparation for project type four.
    AIR_INVASION_PREPARATION = 0xe;
    /// Current close-air-support mission branch.
    AIR_CAS_EFFICIENCY = 0xf;
    /// Current naval mission branch.
    AIR_NAV_EFFICIENCY = 0x10;

    NAVAL_COORDINATION = 0x19;

    INVASION_PREPARATION = 0x1a;

    NAVAL_COMMANDO_RAID_DISTANCE = 0x21;

    CARRIER_TRAFFIC = 0x22;

    CARRIER_NIGHT_TRAFFIC = 0x23;

    SORTIE_EFFICIENCY = 0x24;

    NAVAL_SPEED_FACTOR = 0x29;

    NAVAL_DETECTION = 0x2a;

    ARMY_SPEED_FACTOR = 0x2b;

    ARMY_ARMOR_SPEED_FACTOR = 0x2c;

    SUBMARINE_ATTACK = 0x38;

    NAVY_ORG = 0x46;

    NAVY_ORG_FACTOR = 0x47;

    LOCAL_INTEL_TO_ENEMIES = 0x4b;
    /// Fresh own-army attrition.
    ATTRITION = 0x4f;
    /// Fresh weather attrition.
    WINTER_ATTRITION_FACTOR = 0x50;
    /// Fresh weather attrition.
    WINTER_ATTRITION = 0x51;
    /// Fresh weather attrition.
    HEAT_ATTRITION = 0x52;
    /// Fresh weather attrition.
    HEAT_ATTRITION_FACTOR = 0x53;

    UNIT_UPKEEP_ATTRITION_FACTOR = 0x54;

    SPOTTING_CHANCE = 0x55;
    /// Selected for naval escape chance.
    NAVAL_RETREAT_SPEED_AFTER_INITIAL_COMBAT = 0x59;

    NAVAL_ENEMY_POSITIONING_IN_INITIAL_ATTACK = 0x5a;
    /// Selected for naval escape chance.
    NAVAL_RETREAT_SPEED = 0x5b;
    /// Selected for naval escape chance.
    SUB_RETREAT_SPEED = 0x5c;
    /// Selected for naval escape chance.
    CONVOY_RETREAT_SPEED = 0x5d;

    OUT_OF_SUPPLY_FACTOR = 0x5e;
    /// Direct attack and positive-type scans.
    AIR_ATTACK_FACTOR = 0x6f;

    AIR_DEFENCE_FACTOR = 0x70;

    AIR_AGILITY_FACTOR = 0x71;

    AIR_ACE_BONUSES_FACTOR = 0x72;

    AIR_HOME_DEFENCE_FACTOR = 0x74;

    AIR_POWER_PROJECTION_FACTOR = 0x75;

    AIR_STRATEGIC_BOMBER_DEFENCE_FACTOR = 0x76;

    NAVY_ANTI_AIR_ATTACK = 0x88;

    NAVY_ANTI_AIR_ATTACK_FACTOR = 0x89;

    NAVAL_RANGE = 0x8a;

    NAVAL_RANGE_FACTOR = 0x8b;

    POWER_BALANCE_DAILY = 0x98;

    POWER_BALANCE_WEEKLY = 0x99;
    /// Selected-attack weather-region modifier.
    ARMY_ATTACK_FACTOR = 0xad;
    /// Production-switch efficiency retention.
    LINE_CHANGE_PRODUCTION_EFFICIENCY_FACTOR = 0xa5;
    /// Powered factory fraction.
    OUT_OF_POWER_IMPACT_FACTOR = 0xa8;
    /// Production efficiency cap.
    PRODUCTION_FACTORY_MAX_EFFICIENCY_FACTOR = 0xa9;
    /// Factory efficiency growth.
    PRODUCTION_FACTORY_EFFICIENCY_GAIN_FACTOR = 0xaa;
    /// Starting production efficiency.
    PRODUCTION_FACTORY_START_EFFICIENCY_FACTOR = 0xab;

    NAVY_SUBMARINE_ATTACK_FACTOR = 0xc2;

    NAVY_SUBMARINE_DEFENCE_FACTOR = 0xc3;

    NAVY_CAPITAL_SHIP_ATTACK_FACTOR = 0xc4;

    NAVY_CAPITAL_SHIP_DEFENCE_FACTOR = 0xc5;

    NAVY_SCREEN_ATTACK_FACTOR = 0xc6;

    NAVY_SCREEN_DEFENCE_FACTOR = 0xc7;

    CONVOY_RAIDING_EFFICIENCY_FACTOR = 0xcc;

    RESEARCH_SPEED_FACTOR = 0xcd;

    RESEARCH_SHARING_PER_COUNTRY_BONUS_FACTOR = 0xcf;
    /// Military factory output.
    INDUSTRIAL_CAPACITY_FACTORY = 0xd4;
    /// Powered military factory output.
    INDUSTRIAL_CAPACITY_FACTORY_POWERED = 0xd5;
    /// Naval factory output.
    INDUSTRIAL_CAPACITY_DOCKYARD = 0xd6;
    /// Powered naval factory output.
    INDUSTRIAL_CAPACITY_DOCKYARD_POWERED = 0xd7;
    /// Equipment-conversion speed.
    EQUIPMENT_CONVERSION_SPEED = 0xdc;

    LAND_REINFORCE_RATE = 0xfb;
    /// Selected-attack own-country modifier.
    LAND_NIGHT_ATTACK = 0x103;

    AIR_STRATEGIC_BOMBER_NIGHT_PENALTY = 0x104;

    AIR_NIGHT_PENALTY = 0x105;

    AIR_WEATHER_PENALTY = 0x106;

    NAVAL_NIGHT_ATTACK = 0x107;

    NIGHT_SPOTTING_CHANCE = 0x108;

    AIR_CARRIER_NIGHT_PENALTY_REDUCTION_FACTOR = 0x10d;

    CONVOY_ESCORT_EFFICIENCY = 0x10e;

    COMBAT_WIDTH_FACTOR = 0x10f;

    MAX_COMMAND_POWER = 0x14b;

    MAX_COMMAND_POWER_MULT = 0x14d;

    RIVER_CROSSING_FACTOR = 0x16c;

    RIVER_CROSSING_FACTOR_AGAINST = 0x16d;
    /// Transport weight.
    PARATROOPER_WEIGHT_FACTOR = 0x171;

    SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR = 0x17f;

    NAVAL_INVASION_PREP_DAYS = 0x184;
    /// Preparation for project type four.
    AIR_INVASION_PREP_DAYS = 0x185;

    NAVY_VISIBILITY = 0x19f;

    NAVAL_MINES_EFFECT_REDUCTION = 0x1a7;

    SCREENING_EFFICIENCY = 0x1b1;

    POSITIONING = 0x1b2;

    SCREENING_WITHOUT_SCREENS = 0x1b3;

    FIGHTER_SORTIE_EFFICIENCY = 0x1b6;

    NAVAL_DAMAGE_FACTOR = 0x1b7;

    NAVAL_DEFENSE_FACTOR = 0x1b8;

    NAVAL_CRITICAL_EFFECT_FACTOR = 0x1bb;
    /// The torpedo gun-rate term of port-strike readiness, recorded in the
    /// naval source trees.
    NAVAL_TORPEDO_COOLDOWN_FACTOR = 0x1be;
    /// Targeted spotting reads the current relation accumulator.
    SPOTTING_CHANCE_AGAINST = 0x1c5;

    NAVY_CAPITAL_SHIP_ATTACK_FACTOR_AGAINST = 0x1c8;

    NAVY_CAPITAL_SHIP_DEFENCE_FACTOR_AGAINST = 0x1c9;

    NAVY_SCREEN_ATTACK_FACTOR_AGAINST = 0x1ca;

    NAVY_SCREEN_DEFENCE_FACTOR_AGAINST = 0x1cb;

    NAVY_SUBMARINE_ATTACK_FACTOR_AGAINST = 0x1cc;

    NAVY_SUBMARINE_DEFENCE_FACTOR_AGAINST = 0x1cd;

    INVASION_PREPARATION_AGAINST = 0x1ce;

    NAVAL_ENEMY_FLEET_SIZE_RATIO_PENALTY_FACTOR = 0x1d0;

    NAVAL_HAS_POTF_IN_COMBAT_ATTACK = 0x1d1;

    NAVAL_HAS_POTF_IN_COMBAT_DEFENSE = 0x1d2;
    /// Selected-attack weather-region modifier.
    BREAKTHROUGH_FACTOR = 0x1dd;
    /// Refit output.
    REFIT_SPEED = 0x1d5;

    ARMY_SPEED_FACTOR_FOR_CONTROLLER = 0x1e7;

    ENEMY_ARMY_SPEED_FACTOR = 0x1e8;

    ARMY_ATTACK_SPEED_FACTOR = 0x1e9;

    DISABLE_STRATEGIC_REDEPLOYMENT = 0x1ea;

    DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER = 0x1eb;
    /// Fresh state attrition.
    ATTRITION_FOR_CONTROLLER = 0x1ee;
    /// Fresh state attrition.
    ENEMY_ATTRITION = 0x1ef;

    INTELLIGENCE_AGENCY_DEFENSE = 0x1ff;

    CRYPTO_STRENGTH = 0x210;

    STATE_RESOURCES_FACTOR = 0x223;

    NAVY_WEATHER_PENALTY = 0x248;

    INTEL_FROM_COMBAT_FACTOR = 0x24b;

    INITIATIVE_FACTOR = 0x250;

    ACE_EFFECTIVENESS_FACTOR = 0x25a;
    /// Selected-attack country/state modifier.
    LAND_BUNKER_EFFECTIVENESS_FACTOR = 0x272;

    SCIENTIST_RESEARCH_BONUS_FACTOR = 0x27a;

    UNDERWAY_REPLENISHMENT_RANGE_FACTOR = 0x27c;

    MAX_ORGANISATION = 0x27e;

    ARMY_RETREAT_SPEED_FACTOR = 0x286;

    LOCAL_RESOURCE_GAIN_EFFICIENCY_PER_INFRASTRUCTURE = 0x292;
    /// Fresh own-counter dig-in.
    COMBAT_ENTRENCHMENT = 0x293;

    CANNOT_RETREAT_WHILE_DEFENDING = 0x297;

    CANNOT_RETREAT_WHILE_ATTACKING = 0x298;
}
