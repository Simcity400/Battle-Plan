use super::*;
use engine_native_memory::memory::SliceMemory;

const BASE: usize = 0x140000000;
const ROCKET: usize = 0x100;
const STATUS: usize = 0x400;
const COUNTRY: usize = 0x1000;
const VARIANT: usize = 0x3000;
const DEFINITION: usize = 0x3800;
const DESIGN: u64 = 0x99_0000_0046;
const EQUIP_TOKEN: u32 = 41;

struct World {
    bytes: Vec<u8>,
    instant: Option<u8>,
    blocked: Option<usize>,
}

impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == BASE + crate::construction_ic_sites::INSTANT_CONSTRUCTION.0 {
            assert_eq!(output.len(), 1);
            return self.instant.is_some_and(|value| {
                output[0] = value;
                true
            });
        }
        if self.blocked.is_some_and(|blocked| {
            (address..address.saturating_add(output.len())).contains(&blocked)
        }) {
            return false;
        }
        SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output)
    }
}

struct QuietTestWitness;
impl cequipment_variant::StatWitness for QuietTestWitness {
    fn quiet(&self) -> Result<cequipment_variant::QuietToken, &'static str> {
        Ok(cequipment_variant::QuietToken::new(1, 7))
    }
}

impl World {
    fn new() -> Self {
        let mut world = Self {
            bytes: vec![0; 0x4000],
            instant: Some(0),
            blocked: None,
        };
        for (address, value) in [
            (ROCKET, (BASE + ROCKET_VTABLE.0) as u64),
            (ROCKET + ROCKET_STATUS, STATUS as u64),
            (ROCKET + ROCKET_VARIANT, VARIANT as u64),
            (ROCKET + ROCKET_SPEED, 175_003),
            (STATUS, (BASE + cproduction_status::VTABLE_RVA.0) as u64),
            (STATUS + cproduction_status::COUNTRY, COUNTRY as u64),
            (COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64),
            (COUNTRY + ccountry::ID, 5001),
            (COUNTRY + ccountry::PRODUCTION_STATUS, STATUS as u64),
            (VARIANT, (BASE + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0) as u64),
            (VARIANT + VARIANT_HANDLE, DESIGN),
            (VARIANT + cequipment_variant::DEFINITION, DEFINITION as u64),
            (
                DEFINITION,
                (BASE + engine_native_parts::cequipment_type::VTABLE_RVA.0) as u64,
            ),
        ] {
            world.word(address, value);
        }
        world.bytes[VARIANT + cequipment_variant::EQUIPMENT_TOKEN
            ..VARIANT + cequipment_variant::EQUIPMENT_TOKEN + 4]
            .copy_from_slice(&EQUIP_TOKEN.to_le_bytes());
        world.bytes[DEFINITION + engine_native_parts::cequipment_type::TOKEN
            ..DEFINITION + engine_native_parts::cequipment_type::TOKEN + 4]
            .copy_from_slice(&EQUIP_TOKEN.to_le_bytes());
        world
    }

    fn word(&mut self, address: usize, value: u64) {
        self.bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn tick(&self) -> RocketTick {
        read_rocket_spending(self, BASE, ROCKET, &QuietTestWitness).unwrap()
    }
}

#[test]
fn rocket_signed_input_needs_no_handle_census_or_cost_read() {
    let mut world = World::new();
    world.blocked = Some(ROCKET + LINE_HANDLE);
    assert_eq!(world.tick().failure, RocketFailure::None);
    world.blocked = Some(VARIANT + cequipment_variant::BUILD_COST_IC_STAT);
    for speed in [i64::MIN, -175_003, 0, 175_003, i64::MAX] {
        world.word(ROCKET + ROCKET_SPEED, speed as u64);
        assert_eq!(
            world.tick(),
            RocketTick {
                country: 5001,
                variant: DESIGN,
                spent_raw: speed,
                instant_raw: 0,
                failure: RocketFailure::None,
            }
        );
    }
}

#[test]
fn rocket_instant_input_is_the_native_signed_maximum() {
    let mut world = World::new();
    world.instant = Some(1);
    for speed in [i64::MIN, -175_003, 0, 175_003, i64::MAX] {
        world.word(ROCKET + ROCKET_SPEED, speed as u64);
        for cost in [i64::MIN, -950_009, 0, 950_009, i64::MAX] {
            world.word(
                DEFINITION + engine_native_parts::cequipment_type::BUILD_COST_IC,
                cost as u64,
            );
            world.word(VARIANT + cequipment_variant::BUILD_COST, cost as u64);
            world.word(
                VARIANT + cequipment_variant::BUILD_COST_IC_STAT,
                cost as u64,
            );
            assert_eq!(
                world.tick(),
                RocketTick {
                    country: 5001,
                    variant: DESIGN,
                    spent_raw: 0,
                    instant_raw: speed.max(cost),
                    failure: RocketFailure::None,
                }
            );
        }
    }
}

#[test]
fn null_variant_skips_spending_but_a_zero_design_id_is_bound() {
    let mut world = World::new();
    let design = u64::from(cequipment_variant::REFERENCE_TYPE);
    world.word(VARIANT + VARIANT_HANDLE, design);
    assert_eq!(
        (world.tick().variant, world.tick().failure),
        (design, RocketFailure::None)
    );
    world.word(ROCKET + ROCKET_VARIANT, 0);
    world.blocked = Some(ROCKET + ROCKET_STATUS);
    world.instant = None;
    assert_eq!(read_rocket_spending(&world, BASE, ROCKET, &QuietTestWitness), None);
}

#[test]
fn rocket_short_reads_keep_independent_scope_and_never_zero_the_input() {
    for (blocked, country, variant, failure) in [
        (ROCKET + ROCKET_STATUS, 0, DESIGN, RocketFailure::Country),
        (COUNTRY + ccountry::ID, 0, DESIGN, RocketFailure::Country),
        (ROCKET + ROCKET_VARIANT, 5001, 0, RocketFailure::Design),
        (VARIANT, 5001, 0, RocketFailure::Design),
        (ROCKET, 5001, DESIGN, RocketFailure::Line),
        (ROCKET + ROCKET_SPEED, 5001, DESIGN, RocketFailure::Amount),
    ] {
        let mut world = World::new();
        world.blocked = Some(blocked);
        assert_eq!(
            world.tick(),
            RocketTick {
                country,
                variant,
                failure,
                ..RocketTick::default()
            }
        );
    }
    let mut world = World::new();
    world.instant = None;
    assert_eq!(world.tick().failure, RocketFailure::Instant);
    world.instant = Some(1);
    world.word(
        DEFINITION + engine_native_parts::cequipment_type::BUILD_COST_IC,
        500_000,
    );
    world.word(VARIANT + cequipment_variant::BUILD_COST, 500_000);
    world.word(VARIANT + cequipment_variant::BUILD_COST_IC_STAT, 500_000);
    assert_eq!(
        read_rocket_spending(&world, BASE, ROCKET, &()).unwrap().failure,
        RocketFailure::Instant
    );
    world.blocked = Some(VARIANT + cequipment_variant::BUILD_COST_IC_STAT);
    assert_eq!(world.tick().failure, RocketFailure::Instant);
    world.blocked = None;
    world.word(VARIANT + cequipment_variant::BUILD_COST_IC_STAT, 999_999);
    assert_eq!(world.tick().failure, RocketFailure::Instant);
    assert_eq!(
        world.tick(),
        RocketTick {
            country: 5001,
            variant: DESIGN,
            failure: RocketFailure::Instant,
            ..RocketTick::default()
        }
    );
}

#[test]
fn rocket_authenticates_owner_backlink_and_both_receiver_identities() {
    for (address, country, variant, failure) in [
        (ROCKET, 5001, DESIGN, RocketFailure::Line),
        (STATUS, 0, DESIGN, RocketFailure::Country),
        (
            STATUS + cproduction_status::COUNTRY,
            0,
            DESIGN,
            RocketFailure::Country,
        ),
        (COUNTRY, 0, DESIGN, RocketFailure::Country),
        (COUNTRY + ccountry::ID, 0, DESIGN, RocketFailure::Country),
        (
            COUNTRY + ccountry::PRODUCTION_STATUS,
            0,
            DESIGN,
            RocketFailure::Country,
        ),
        (VARIANT, 5001, 0, RocketFailure::Design),
        (VARIANT + VARIANT_HANDLE, 5001, 0, RocketFailure::Design),
    ] {
        let mut world = World::new();
        world.word(address, 0);
        assert_eq!(
            world.tick(),
            RocketTick {
                country,
                variant,
                failure,
                ..RocketTick::default()
            }
        );
    }
    let world = World::new();
    for pointer in [0, usize::MAX - ROCKET_VARIANT] {
        let tick = read_rocket_spending(&world, BASE, pointer, &QuietTestWitness).unwrap();
        assert_eq!(
            (tick.country, tick.variant, tick.failure),
            (0, 0, RocketFailure::Country)
        );
    }
}

#[test]
fn rocket_instant_refuses_variant_replaced_between_identity_and_cost_read() {
    use std::cell::Cell;

    struct ReplacingWorld {
        world: World,
        identity_reads: Cell<usize>,
    }

    impl Memory for ReplacingWorld {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if !self.world.copy(address, output) {
                return false;
            }
            if address == VARIANT && output.len() == 16 {
                let reads = self.identity_reads.get();
                self.identity_reads.set(reads + 1);
                if reads >= 1 {
                    let replacement: u64 = 0x9a_0000_0046;
                    output[8..16].copy_from_slice(&replacement.to_le_bytes());
                }
            }
            true
        }
    }

    let mut world = World::new();
    world.instant = Some(1);
    world.word(
        DEFINITION + engine_native_parts::cequipment_type::BUILD_COST_IC,
        777_000,
    );
    world.word(VARIANT + cequipment_variant::BUILD_COST, 777_000);
    world.word(VARIANT + cequipment_variant::BUILD_COST_IC_STAT, 777_000);
    let replacing = ReplacingWorld {
        world,
        identity_reads: Cell::new(0),
    };
    let tick = read_rocket_spending(&replacing, BASE, ROCKET, &QuietTestWitness).unwrap();
    assert_eq!(tick.failure, RocketFailure::Instant);
    assert_eq!(tick.instant_raw, 0);
}
