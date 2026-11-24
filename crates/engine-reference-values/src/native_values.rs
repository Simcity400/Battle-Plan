
pub mod equipment_variant {
    pub const REFERENCE_TYPE: u32 = 70;
    pub const STAT_COUNT: usize = 78;
}

pub mod orders_group {
    pub const REFERENCE_TYPE: u32 = 53;
}

pub mod equipment_type {
    pub const PLANE_MASK: u64 = 0x0000_001f_0037_fc00;
    pub const TANK_MASK: u64 = 0x0000_0040_8000_003c;
    pub const SHIP_MASK: u64 = 0x0000_0080_0040_03c1;
}

pub mod dlc_feature {
    pub const MAN_THE_GUNS: u32 = 0x0000_0010;
    pub const NO_STEP_BACK: u32 = 0x0000_0080;
    pub const BY_BLOOD_ALONE: u32 = 0x0000_0100;
}

pub mod subject_relation {
    pub const TYPE_VALUE: u32 = 0x30d1;
}

pub mod fuel_status {
    pub const QUANTITY_SCALE: i64 = 32_768;
    pub const MODIFIER_SCALE: i64 = 100_000;
}

pub mod production_line {
    pub const KIND_MILITARY: u8 = 0;
    pub const KIND_NAVAL: u8 = 1;
    pub const KIND_SHIP_REFIT: u8 = 2;
    pub const KIND_RAILWAY_GUN: u8 = 3;
}

pub mod air_loss {
    pub const FAMILY_BASES: [u32; 13] = [0, 3, 10, 17, 24, 30, 40, 44, 51, 58, 65, 71, 81];
}

pub mod air_history {
    pub const DAILY_WINDOW_CAPACITY: u32 = 30;
    pub const FIELD_COUNT: u32 = 89;
}

pub mod production_modifiers {
    use crate::modifier_keys as keys;
    pub const KEYS: [u32; 11] = [
        keys::LINE_CHANGE_PRODUCTION_EFFICIENCY_FACTOR,
        keys::OUT_OF_POWER_IMPACT_FACTOR,
        keys::PRODUCTION_FACTORY_MAX_EFFICIENCY_FACTOR,
        keys::PRODUCTION_FACTORY_EFFICIENCY_GAIN_FACTOR,
        keys::PRODUCTION_FACTORY_START_EFFICIENCY_FACTOR,
        keys::INDUSTRIAL_CAPACITY_FACTORY,
        keys::INDUSTRIAL_CAPACITY_FACTORY_POWERED,
        keys::INDUSTRIAL_CAPACITY_DOCKYARD,
        keys::INDUSTRIAL_CAPACITY_DOCKYARD_POWERED,
        keys::EQUIPMENT_CONVERSION_SPEED,
        keys::REFIT_SPEED,
    ];
}

pub mod naval_modifiers {
    use crate::modifier_keys;
    pub const KEYS: [u32; 10] = [
        modifier_keys::SORTIE_EFFICIENCY,
        modifier_keys::NAVAL_RETREAT_SPEED_AFTER_INITIAL_COMBAT,
        modifier_keys::NAVAL_ENEMY_POSITIONING_IN_INITIAL_ATTACK,
        modifier_keys::NAVAL_RETREAT_SPEED,
        modifier_keys::SUB_RETREAT_SPEED,
        modifier_keys::CONVOY_RETREAT_SPEED,
        modifier_keys::POSITIONING,
        modifier_keys::SCREENING_WITHOUT_SCREENS,
        modifier_keys::FIGHTER_SORTIE_EFFICIENCY,
        modifier_keys::NAVAL_TORPEDO_COOLDOWN_FACTOR,
    ];
}
