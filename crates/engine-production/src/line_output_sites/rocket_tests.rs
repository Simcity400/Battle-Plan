use super::*;
use crate::memory::SliceMemory;

const BASE: usize = 0x140000000;
const ROCKET: usize = 0x100;
const STATUS: usize = 0x400;
const COUNTRY: usize = 0x1000;
const VARIANT: usize = 0x3000;
const DESIGN: u64 = 0x99_0000_0046;

struct World {
    bytes: Vec<u8>,
    instant: Option<u8>,
    blocked: Option<usize>,
}

impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == BASE + crate::construction_ic_sites::INSTANT_CONSTRUCTION {
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

impl World {
    fn new() -> Self {
        let mut world = Self {
            bytes: vec![0; 0x4000],
            instant: Some(0),
            blocked: None,
        };
        for (address, value) in [
            (ROCKET, (BASE + ROCKET_VTABLE) as u64),
            (ROCKET + ROCKET_STATUS, STATUS as u64),
            (ROCKET + ROCKET_VARIANT, VARIANT as u64),
            (ROCKET + ROCKET_SPEED, 175_003),
            (STATUS, (BASE + cproduction_status::VTABLE_RVA) as u64),
            (STATUS + cproduction_status::COUNTRY, COUNTRY as u64),
            (COUNTRY, (BASE + ccountry::VTABLE_RVA) as u64),
            (COUNTRY + ccountry::ID, 5001),
            (COUNTRY + ccountry::PRODUCTION_STATUS, STATUS as u64),
            (VARIANT, (BASE + cequipment_variant::VTABLE_RVA) as u64),
            (VARIANT + VARIANT_HANDLE, DESIGN),
        ] {
            world.word(address, value);
        }
        world
    }

    fn word(&mut self, address: usize, value: u64) {
        self.bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn tick(&self) -> RocketTick {
        read_rocket_spending(self, BASE, ROCKET).unwrap()
    }
}

#[test]
fn rocket_signed_input_needs_no_handle_census_or_cost_read() {
    let mut world = World::new();
    // The native rocket constructor's handle and registration never gate spending.
    world.blocked = Some(ROCKET + LINE_HANDLE);
    assert_eq!(world.tick().failure, RocketFailure::None);
    world.blocked = Some(VARIANT + cequipment_variant::BUILD_COST_CACHE_CANDIDATE);
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
                VARIANT + cequipment_variant::BUILD_COST_CACHE_CANDIDATE,
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
    assert_eq!(read_rocket_spending(&world, BASE, ROCKET), None);
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
    world.blocked = Some(VARIANT + cequipment_variant::BUILD_COST_CACHE_CANDIDATE);
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
        let tick = read_rocket_spending(&world, BASE, pointer).unwrap();
        assert_eq!(
            (tick.country, tick.variant, tick.failure),
            (0, 0, RocketFailure::Country)
        );
    }
}
