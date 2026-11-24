use super::super::*;
use engine_decisions::{
    border_effect_demand, cdecision, ctargeted_decision as targeted, event_namespace,
};
use engine_native::{cgame_state, cland_combat, cland_combatant};
use engine_native_parts::{
    cland_border_war_combat as combat,
    cland_border_war_combatant as side,
    creference_registry as registry,
    reference_bucket as bucket,
};
use engine_native_memory::{cpdx_array, memory::{CountedMemory, ReadCounts}};
use std::{cell::Cell, collections::BTreeMap, time::Instant};

const IMAGE: usize = 0x1400_0000;
const GS: usize = 0x1000;
const COUNTRIES: usize = 0x2000;
const LINKS: usize = 0xc000;
const NEEDED: usize = 0x12000;
const OTHER: usize = 0x14000;
const COMBAT: usize = 0x16000;
const EVENT: usize = 0x18000;
const COUNTRY_BASE: usize = 0x20000;
const COUNTRY_STRIDE: usize = 0x2000;

struct Fixture {
    bytes: Vec<u8>,
    globals: BTreeMap<usize, u8>,
    seeds: Vec<usize>,
    countries: usize,
}

impl Fixture {
    fn put(&mut self, address: usize, value: &[u8]) {
        if address < self.bytes.len() {
            self.bytes[address..address + value.len()].copy_from_slice(value);
        } else {
            self.globals.extend(
                value
                    .iter()
                    .enumerate()
                    .map(|(index, byte)| (address + index, *byte)),
            );
        }
    }
    fn word(&mut self, address: usize, value: usize) {
        self.put(address, &(value as u64).to_le_bytes());
    }
    fn integer(&mut self, address: usize, value: i32) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: usize) {
        self.word(address, data);
        self.integer(address + cpdx_array::CAPACITY, count as i32);
        self.integer(address + cpdx_array::COUNT, count as i32);
    }
    fn text(&mut self, address: usize, value: &[u8]) {
        assert!(value.len() <= 15);
        self.put(address, &[0; 32]);
        self.put(address, value);
        self.word(address + 16, value.len());
        self.word(address + 24, 15);
    }

    fn new(countries: usize, active: bool) -> Self {
        let mut f = Self {
            bytes: vec![0; COUNTRY_BASE + countries * COUNTRY_STRIDE],
            globals: BTreeMap::new(),
            seeds: Vec::with_capacity(countries),
            countries,
        };
        f.array(GS + cgame_state::COUNTRIES, COUNTRIES, countries + 1);
        f.array(GS + cgame_state::COUNTRY_LINKS, LINKS, countries + 1);
        for (definition, key) in [(NEEDED, b"needed".as_slice()), (OTHER, b"other".as_slice())] {
            f.word(definition, IMAGE + cdecision::VTABLE_RVA.0);
            f.text(definition + cdecision::TOKEN, key);
        }
        for index in 0..countries {
            let country = COUNTRY_BASE + index * COUNTRY_STRIDE;
            let status = country + 0x1400;
            f.word(COUNTRIES + (index + 1) * 8, country);
            f.integer(LINKS + (index + 1) * 4, index as i32 + 1);
            f.word(country, IMAGE + ccountry::VTABLE_RVA.0);
            f.integer(country + ccountry::ID, index as i32 + 1);
            f.word(country + ccountry::DECISION_STATUS, status);
            f.word(status, IMAGE + cdecision_status::VTABLE_RVA.0);
            f.word(status + cdecision_status::COUNTRY, country);
            f.array(status + cdecision_status::TARGETED, status + 0x240, 2);
            f.array(status + cdecision_status::RANDOM_ITEMS, status + 0x400, 2);
            for (ordinal, definition) in [NEEDED, OTHER].into_iter().enumerate() {
                let instance = status + 0x300 + ordinal * 0x40;
                f.word(status + 0x240 + ordinal * 8, instance);
                f.word(instance, IMAGE + targeted::VTABLE_RVA.0);
                f.word(instance + targeted::DEFINITION, definition);
                f.integer(instance + targeted::NUMERIC_TARGET, 7);
                let random = status + 0x400 + ordinal * cdecision_status::RANDOM_ITEM_STRIDE;
                f.word(random, IMAGE + cdecision_status::RANDOM_ITEM_VTABLE_RVA.0);
                f.word(
                    random + cdecision_status::RANDOM_ITEM_DEFINITION,
                    definition,
                );
                f.integer(random + cdecision_status::RANDOM_ITEM_TARGET, 7);
                f.integer(random + cdecision_status::RANDOM_ITEM_SEED, -17);
                if ordinal == 0 {
                    f.seeds.push(random + cdecision_status::RANDOM_ITEM_SEED);
                }
            }
        }
        f.array(GS + cgame_state::COMBATS + 8, 0x15800, usize::from(active));
        if active {
            f.border_event();
        }
        f
    }

    fn border_event(&mut self) {
        self.word(0x15800, COMBAT);
        self.word(COMBAT, IMAGE + combat::VTABLE_RVA.0);
        self.word(COMBAT + cland_combat::BODY, IMAGE + combat::BODY_VTABLE_RVA.0);
        self.word(
            COMBAT + cland_combat::HANDLE,
            ((42_u64 << 32) | 4714) as usize,
        );
        for (index, pointer) in [0x17000, 0x17800].into_iter().enumerate() {
            self.word(COMBAT + cland_combat::ATTACKER + index * 8, pointer);
            self.word(pointer, IMAGE + side::VTABLE_RVA.0);
            self.word(pointer + cland_combatant::COMBAT, COMBAT);
            self.put(pointer + cland_combatant::IS_ATTACKER, &[(1 - index) as u8]);
            self.text(pointer + side::WIN_EVENT, b"");
            self.text(pointer + side::LOSE_EVENT, b"");
        }
        self.text(0x17000 + side::WIN_EVENT, b"test.7");
        self.array(IMAGE + event_namespace::TABLE_RVA.0, 0x15500, 1);
        self.text(0x15500, b"test");
        let hash = b"test".iter().fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
        });
        self.integer(0x15500 + event_namespace::NAME_HASH, hash as i32);
        let handle = (1_000_007_u64 << 32) | 0x32;
        self.word(IMAGE + registry::LOW_ROOTS_RVA.0 + 0x32 * 8, 0x19000);
        self.word(0x19000 + registry::ENTRIES, 0x19100);
        self.integer(0x19000 + registry::COUNT, 1);
        self.integer(0x19000 + registry::MASK, 7);
        self.put(0x19000 + registry::OVERFLOW_DISTANCE, &[5]);
        self.put(0x19100 + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let cell = 0x19100 + (((hash >> 32) as u32 ^ hash as u32) & 7) as usize * bucket::STRIDE;
        self.put(cell + bucket::DISTANCE, &[1]);
        self.word(cell + bucket::TYPE, handle as usize);
        self.word(cell + bucket::OBJECT, EVENT);
        self.word(EVENT, IMAGE + 0x02a1_6008);
        self.word(EVENT + 8, handle as usize);
        self.word(EVENT + 0x118, IMAGE + 0x02a1_5f68);
        self.word(EVENT + 0x238, IMAGE + 0x0285_26e0);
        self.array(EVENT + 0x240, 0x18900, 1);
        self.word(0x18900, 0x18a00);
        self.word(0x18a00, IMAGE + 0x027f_1ff0);
        self.text(0x18a00 + 0x58, b"needed");
    }

    fn prepared(&self) -> Captured {
        let mut captured = Captured::vacant();
        captured.counts = CaptureRows::new(self.countries);
        captured.decisions = CaptureRows::new(self.countries * 2);
        captured.decision_countries = Arena::new(self.countries * 2);
        captured.decision_text = Arena::new(self.countries * (b"needed".len() + b"other".len()));
        captured.effect_tokens = Arena::<border_effect_demand::Token>::new(1);
        captured.effect_frames = Arena::<border_effect_demand::Frame>::new(2);
        captured.effect_names = Arena::<border_effect_demand::Name>::new(1);
        captured.effect_bytes = Arena::new(6);
        captured.effect_events = Arena::new(1);
        captured
    }
}

impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if let Some(source) = self
            .bytes
            .get(address..address.saturating_add(destination.len()))
        {
            destination.copy_from_slice(source);
            return true;
        }
        for (index, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.globals.get(&(address + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

struct Fault<'a> {
    fixture: &'a Fixture,
    address: Option<usize>,
    hits: Cell<usize>,
}

impl Memory for Fault<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self
            .address
            .is_some_and(|fault| (address..address + destination.len()).contains(&fault))
        {
            self.hits.set(self.hits.get() + 1);
            return false;
        }
        self.fixture.copy(address, destination)
    }
}

fn run(memory: &impl Memory, captured: &mut Captured) -> Observation {
    let mut enabled = [false; N];
    enabled[8] = true;
    enabled[9] = true;
    acquire(
        memory,
        IMAGE,
        GS,
        0,
        captured.buffers(),
        &enabled,
        Slices {
            technologies: &mut SliceFingerprints::new(),
            focus: &mut SliceFingerprints::new(),
            changes: &crate::country_input_changes::Observer,
        },
        None,
    )
}

fn portable(captured: &mut Captured, observation: &Observation) -> Vec<rows::DecisionRow> {
    captured.counts.outcome = observation.outcomes[8];
    captured.decisions.outcome = observation.outcomes[9];
    captured.decision_text.len = observation.lens[4];
    captured.decision_countries.len = observation.lens[11];
    let TableView::Complete(TableRows::Decisions(rows)) =
        captured.tables().into_iter().nth(9).unwrap()
    else {
        panic!(
            "decision projection unavailable: {:?}",
            observation.outcomes[9]
        );
    };
    rows
}

#[test]
fn zero_active_border_events_do_not_read_decision_random_inputs() {
    let f = Fixture::new(2, false);
    let memory = CountedMemory::new(Fault {
        fixture: &f,
        address: Some(COUNTRY_BASE + 0x1400 + cdecision_status::RANDOM_ITEMS),
        hits: Cell::new(0),
    });
    let mut captured = f.prepared();
    let observation = run(&memory, &mut captured);
    assert_eq!(observation.outcomes[8], Outcome::Rows { len: 2 });
    assert_eq!(observation.outcomes[9], Outcome::Rows { len: 4 });
    assert_eq!(memory.0.hits.get(), 0);
    assert!(portable(&mut captured, &observation).iter().all(|row| {
        row.description_seed
            .require("description_seed")
            .unwrap()
            .is_none()
    }));
}

#[test]
fn reachable_border_event_selects_only_matching_targeted_decisions() {
    let f = Fixture::new(2, true);
    let memory = CountedMemory::new(Fault {
        fixture: &f,
        address: Some(OTHER + cdecision::FIXED_RANDOM_SEED),
        hits: Cell::new(0),
    });
    let mut captured = f.prepared();
    let observation = run(&memory, &mut captured);
    assert_eq!(observation.outcomes[8], Outcome::Rows { len: 2 });
    assert_eq!(observation.outcomes[9], Outcome::Rows { len: 4 });
    assert_eq!(memory.0.hits.get(), 0);
    let rows = portable(&mut captured, &observation);
    assert_eq!(
        rows.iter()
            .map(|row| *row.description_seed.require("description_seed").unwrap())
            .collect::<Vec<_>>(),
        [Some(Some(-17)), None, Some(Some(-17)), None]
    );
}

#[test]
fn demand_and_matching_seed_faults_withhold_only_decisions() {
    let f = Fixture::new(2, true);
    for address in [GS + cgame_state::COMBATS + 8, f.seeds[0]] {
        let memory = CountedMemory::new(Fault {
            fixture: &f,
            address: Some(address),
            hits: Cell::new(0),
        });
        let mut captured = f.prepared();
        let observation = run(&memory, &mut captured);
        assert_eq!(observation.outcomes[8], Outcome::Rows { len: 2 });
        assert!(matches!(
            observation.outcomes[9],
            Outcome::Unavailable { .. }
        ));
        assert!(memory.0.hits.get() > 0);
        assert!(
            observation
                .outcomes
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != 8 && *index != 9)
                .all(|(_, outcome)| *outcome
                    == Outcome::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON
                    })
        );
    }
}

