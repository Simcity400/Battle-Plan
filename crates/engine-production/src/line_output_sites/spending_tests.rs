use super::*;
use engine_native_memory::memory::SliceMemory;

const BASE: usize = 0x140000000;
const STATE: usize = 0x1000;
const POINTERS: usize = 0x1500;
const LINKS: usize = 0x1600;

struct World {
    bytes: Vec<u8>,
    instant: Option<u8>,
    training: Option<u8>,
    singleton: Option<usize>,
    blocked: Option<usize>,
}

impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        for (rva, flag) in [
            (
                crate::construction_ic_sites::INSTANT_CONSTRUCTION,
                self.instant,
            ),
            (INSTANT_ARMY_TRAINING, self.training),
        ] {
            if address == BASE + rva.0 {
                assert_eq!(output.len(), 1);
                return flag.is_some_and(|value| {
                    output[0] = value;
                    true
                });
            }
        }
        if address == BASE + cgame_state::SINGLETON_RVA.0 {
            assert_eq!(output.len(), 8);
            return self.singleton.is_some_and(|value| {
                output.copy_from_slice(&(value as u64).to_le_bytes());
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
    fn word(&mut self, address: usize, value: u64) {
        self.bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn dword(&mut self, address: usize, value: u32) {
        self.bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn header(&mut self, address: usize, data: usize, count: u32) {
        self.word(address, data as u64);
        self.dword(address + 8, count);
        self.dword(address + 12, count);
    }
    fn new() -> Self {
        let mut world = Self {
            bytes: vec![0; 0x6000],
            instant: Some(0),
            training: Some(0),
            singleton: Some(STATE),
            blocked: None,
        };
        world.header(STATE + cgame_state::COUNTRIES, POINTERS, 3);
        world.header(STATE + cgame_state::COUNTRY_LINKS, LINKS, 8);
        world.word(POINTERS + 8, 0x2000);
        world.word(POINTERS + 16, 0x4000);
        for (raw, slot) in [(1, 1), (2, 2), (5, 1), (6, 1), (7, 2)] {
            world.dword(LINKS + raw * 4, slot);
        }
        for (country, raw) in [(0x2000, 5), (0x4000, 7)] {
            world.word(country, (BASE + ccountry::VTABLE_RVA.0) as u64);
            world.dword(country + ccountry::ID, raw);
        }
        world.dword(STATE + cgame_state::LOCAL_PLAYER_COUNTRY, 5);
        world
    }
}

#[test]
fn applied_input_keeps_signed_zero_and_extreme_amounts_for_all_four_producers() {
    let mut world = World::new();
    for producer in 0..PRODUCERS.len() {
        for amount in [i64::MIN, -1, 0, 125_001, i64::MAX] {
            assert_eq!(
                applied_spending(&world, BASE, 5, producer, amount),
                Ok((amount, 0))
            );
        }
    }
    world.instant = Some(255);
    world.training = None;
    world.singleton = None;
    for producer in 0..PRODUCERS.len() {
        assert_eq!(
            applied_spending(&world, BASE, 5, producer, -123),
            Ok((0, -123))
        );
    }
}

#[test]
fn training_uses_only_naval_and_railway_viewing_country_including_aliases() {
    let mut world = World::new();
    world.training = Some(1);
    for producer in [1, 2] {
        for country in [5, 6] {
            assert_eq!(
                applied_spending(&world, BASE, country, producer, 900_001),
                Ok((0, 900_001))
            );
        }
        assert_eq!(
            applied_spending(&world, BASE, 7, producer, 900_001),
            Ok((900_001, 0))
        );
    }
    for producer in [0, 3] {
        assert_eq!(
            applied_spending(&world, BASE, 5, producer, 900_001),
            Ok((900_001, 0))
        );
    }
    world.dword(STATE + cgame_state::LOCAL_PLAYER_COUNTRY, u32::MAX);
    world.dword(STATE + cgame_state::OBSERVED_COUNTRY, 7);
    assert_eq!(applied_spending(&world, BASE, 7, 1, 88), Ok((0, 88)));
    world.dword(STATE + cgame_state::OBSERVED_COUNTRY, 0);
    assert_eq!(applied_spending(&world, BASE, 7, 1, 88), Ok((88, 0)));
}

#[test]
fn training_compares_readable_raw_links_without_country_resolution() {
    let mut world = World::new();
    world.training = Some(1);
    world.blocked = Some(STATE + cgame_state::COUNTRIES);
    for producer in [1, 2] {
        assert_eq!(
            applied_spending(&world, BASE, 6, producer, -88),
            Ok((0, -88))
        );
        assert_eq!(
            applied_spending(&world, BASE, 7, producer, -88),
            Ok((-88, 0))
        );
        world.dword(STATE + cgame_state::LOCAL_PLAYER_COUNTRY, 3);
        assert_eq!(
            applied_spending(&world, BASE, 5, producer, -88),
            Ok((-88, 0))
        );
        world.dword(LINKS + 5 * 4, 0);
        assert_eq!(
            applied_spending(&world, BASE, 5, producer, -88),
            Ok((0, -88))
        );
        world.dword(STATE + cgame_state::LOCAL_PLAYER_COUNTRY, 0);
        assert_eq!(
            applied_spending(&world, BASE, 5, producer, -88),
            Ok((0, -88))
        );
        world.dword(LINKS + 5 * 4, 1);
        world.dword(STATE + cgame_state::LOCAL_PLAYER_COUNTRY, 5);
    }
}

#[test]
fn independent_native_read_failures_refuse_spending_by_name() {
    let mut world = World::new();
    assert_eq!(
        applied_spending(&world, BASE, 5, 4, 1),
        Err("production output producer unsupported")
    );
    world.instant = None;
    assert_eq!(
        applied_spending(&world, BASE, 5, 0, 1),
        Err("production output instant flag unreadable")
    );
    world.instant = Some(0);
    world.training = None;
    assert_eq!(
        applied_spending(&world, BASE, 5, 1, 1),
        Err("production output instant army training flag unreadable")
    );
    world.training = Some(1);
    world.singleton = None;
    assert_eq!(
        applied_spending(&world, BASE, 5, 2, 1),
        Err("production output instant viewing state unreadable or null")
    );
    world.singleton = Some(STATE);
    world.blocked = Some(STATE + cgame_state::OBSERVED_COUNTRY);
    assert_eq!(applied_spending(&world, BASE, 5, 1, 1), Ok((0, 1)));
    world.dword(STATE + cgame_state::LOCAL_PLAYER_COUNTRY, u32::MAX);
    assert_eq!(
        applied_spending(&world, BASE, 5, 1, 1),
        Err("production output instant viewing country unreadable")
    );
    world.dword(STATE + cgame_state::LOCAL_PLAYER_COUNTRY, 5);
    world.blocked = Some(LINKS + 6 * 4);
    assert_eq!(
        applied_spending(&world, BASE, 6, 1, 1),
        Err("production output instant country link unreadable")
    );
}
