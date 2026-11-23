use super::reference::ReferenceReads;
use super::*;
use crate::army_stat_changes::{MEMBERSHIP_TEST, serial};
use engine_native::{
    carmy,
    ccountry,
    cequipment_variant as variant,
    cequipment_variant_pool as pool,
    cstate,
    modifier_accumulator::Value,
    modifier_definition as definition,
};
use engine_native_land::division_panel_operands;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
};

#[derive(Clone, Copy)]
enum Fault {
    Sticky(usize),
    Once(usize, usize),
}
impl Fault {
    fn address(self) -> usize {
        match self {
            Self::Sticky(at) | Self::Once(at, _) => at,
        }
    }
}

type Hook = (u32, Result<Value, &'static str>);

#[derive(Default)]
struct Fixture {
    bytes: BTreeMap<usize, u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<Fault>,
    touches: Cell<usize>,
    hook: Option<Hook>,
    hooked: Cell<usize>,
}
impl Fixture {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.bytes
            .extend(bytes.iter().enumerate().map(|(i, &byte)| (at + i, byte)));
    }
    fn array(&mut self, at: usize, data: usize, len: u32) {
        self.put(at, &(data as u64).to_le_bytes());
        self.put(at + 8, &len.to_le_bytes());
        self.put(at + 12, &len.to_le_bytes());
    }
    fn run(&self, fault: Option<Fault>, hook: Option<Hook>) -> Self {
        Self {
            bytes: self.bytes.clone(),
            reads: RefCell::new(Vec::with_capacity(256)),
            fault,
            hook,
            ..Default::default()
        }
    }
    fn touches(&self, at: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|&&(start, len)| (start..start + len).contains(&at))
            .count()
    }
    fn demanded(&self) -> BTreeSet<usize> {
        self.reads
            .borrow()
            .iter()
            .flat_map(|&(at, len)| at..at + len)
            .collect()
    }
    fn fixture() -> Self {
        let mut memory = Self::default();
        for (army, entries) in [(0x1000, 0x6000), (0x2000, 0x6100)] {
            memory.put(
                army + carmy::EQUIPMENT_POOL,
                &(pool::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            memory.array(army + carmy::EQUIPMENT_POOL + pool::ITEMS, entries, 3);
            for (ordinal, amount) in [0_i64, -1, i64::MAX].into_iter().enumerate() {
                memory.put(entries + ordinal * 16, &0x5000_u64.to_le_bytes());
                memory.put(entries + ordinal * 16 + 8, &amount.to_le_bytes());
            }
        }
        memory.put(0x5000, &(variant::VTABLE_RVA.0 as u64).to_le_bytes());
        memory.put(
            0x5008,
            &((1_u64 << 32) | u64::from(variant::REFERENCE_TYPE)).to_le_bytes(),
        );
        memory.array(
            0x5000 + division_panel_operands::VARIANT_TERRAIN_ATTRITION,
            0x7000,
            3,
        );
        for (index, raw) in [i64::MIN, -1, i64::MAX].into_iter().enumerate() {
            memory.put(0x7000 + index * 8, &raw.to_le_bytes());
        }
        memory.put(0x3000, &(cstate::VTABLE_RVA.0 as u64).to_le_bytes());
        memory.put(0x3000 + cstate::COUNTRY_MODIFIERS, &[0; 17]);
        memory.put(
            0x3000 + cstate::COUNTRY_MODIFIERS,
            &0x8000_u64.to_le_bytes(),
        );
        memory.put(0x8004, &[0]);
        let mut keys = [
            engine_executable::modifier_keys::ATTRITION,
            engine_executable::modifier_keys::ATTRITION_FOR_CONTROLLER,
            engine_executable::modifier_keys::ENEMY_ATTRITION,
        ];
        keys.sort_unstable();
        memory.array(0x3000 + cstate::BASE_MODIFIERS, 0x8100, 3);
        memory.put(definition::TABLE_RVA.0, &0x9000_u64.to_le_bytes());
        for (index, key) in keys.into_iter().enumerate() {
            memory.put(0x8100 + index * 16, &u64::from(key).to_le_bytes());
            memory.put(
                0x8108 + index * 16,
                &[i64::MIN, -1, i64::MAX][index].to_le_bytes(),
            );
            let row = 0x9000 + key as usize * definition::STRIDE;
            memory.put(row + definition::FEATURE_SELECTOR, &0_u32.to_le_bytes());
            memory.put(
                row + definition::FLAGS,
                &definition::CLAMP_MAXIMUM.to_le_bytes(),
            );
        }
        memory
    }
}
impl Memory for Fixture {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, out.len()));
        if let Some(fault) = self.fault
            && (address..address + out.len()).contains(&fault.address())
        {
            let touch = self.touches.get() + 1;
            self.touches.set(touch);
            if match fault {
                Fault::Sticky(_) => true,
                Fault::Once(_, nth) => touch == nth,
            } {
                return false;
            }
        }
        for (index, byte) in out.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
    fn modifier_accumulator_value(
        &self,
        _address: usize,
        key: u32,
    ) -> Option<Result<Value, &'static str>> {
        let (hooked, value) = self.hook?;
        (hooked == key).then(|| {
            self.hooked.set(self.hooked.get() + 1);
            value
        })
    }
}

fn compare(actual: &NativeReads, expected: &NativeReads) {
    compare_but_attrition(actual, expected);
    let read = |read: &StateRead| (read.pointer, read.certificate, read.values);
    for (&key, old) in &expected.states {
        let current = &actual.states[&key];
        assert_eq!(
            (current.sequence, current.read.as_ref().map(read)),
            (old.sequence, old.read.as_ref().map(read))
        );
    }
}

fn compare_but_attrition(actual: &NativeReads, expected: &NativeReads) {
    assert_eq!(actual.complete, expected.complete);
    assert_eq!(
        actual.membership_observation,
        expected.membership_observation
    );
    assert_eq!(actual.roots.len(), expected.roots.len());
    for (current, old) in actual.roots.iter().zip(&expected.roots) {
        assert_eq!(
            (
                current.receiver,
                current.accumulator,
                current.certificate,
                current.result,
                current.ready,
                current.rows.as_slice()
            ),
            (
                old.receiver,
                old.accumulator,
                old.certificate,
                old.result,
                old.ready,
                old.rows.as_slice()
            )
        );
    }
    assert_eq!(
        (
            actual.roots_used,
            actual.variants_used,
            actual.variant_refresh
        ),
        (
            expected.roots_used,
            expected.variants_used,
            expected.variant_refresh
        )
    );
    assert_eq!(actual.root_indices.len(), expected.root_indices.len());
    for (&key, old) in &expected.root_indices {
        let current = &actual.root_indices[&key];
        assert_eq!((current.sequence, current.read), (old.sequence, old.read));
    }
    assert_eq!(actual.membership_indices, expected.membership_indices);
    assert_eq!(actual.memberships.len(), expected.memberships.len());
    for (current, old) in actual.memberships.iter().zip(&expected.memberships) {
        assert_eq!(
            (
                current.army,
                current.certificate,
                current.result,
                current.ready,
                &current.variants
            ),
            (
                old.army,
                old.certificate,
                old.result,
                old.ready,
                &old.variants
            )
        );
    }
    assert_eq!(actual.states.len(), expected.states.len());
    let read = |read: &StateRead| (read.pointer, read.certificate);
    for (&key, old) in &expected.states {
        let current = &actual.states[&key];
        assert_eq!(
            (current.sequence, current.read.as_ref().map(read)),
            (old.sequence, old.read.as_ref().map(read))
        );
    }
    assert_eq!(actual.variant_indices.len(), expected.variant_indices.len());
    for (&key, old) in &expected.variant_indices {
        let current = &actual.variant_indices[&key];
        assert_eq!((current.sequence, current.read), (old.sequence, old.read));
    }
    assert_eq!(actual.variants.len(), expected.variants.len());
    for (current, old) in actual.variants.iter().zip(&expected.variants) {
        assert_eq!(
            (
                current.pointer,
                current.certificate,
                current.lifetime,
                current.availability,
                current.result,
                current.ready,
                &current.values
            ),
            (
                old.pointer,
                old.certificate,
                old.lifetime,
                old.availability,
                old.result,
                old.ready,
                &old.values
            )
        );
    }
}

#[test]
fn compact_source_observations_match_the_original_field_faults_and_admission() {
    let _serial = serial(&MEMBERSHIP_TEST);
    operands::demand(2, 1, 1);
    operands::presize();
    operands::boundary(true);
    operands::patched(true);
    operands::state_forwarder(true);
    super::super::tests::observe(true);
    let _enabled = super::super::tests::Enabled;
    struct States;
    impl Drop for States {
        fn drop(&mut self) {
            operands::state_forwarder(false);
        }
    }
    let _states = States;
    let seeds = [0x1000, 0x2000].map(|pointer| DivisionSeed {
        pointer,
        division: pointer as u64,
        roster_owner: Some(1),
        logical_owner: Some(1),
        ..Default::default()
    });
    let selection = |country: bool| Selection {
        country: country.then_some((
            1,
            Ok(ccountry::Country {
                pointer: 0xa000,
                canonical_slot: 0,
                id: 1,
            }),
        )),
        movement: None,
        commander: None,
        province: Ok(0x3000),
        context: Some(Ok(Some((9, 0x3000)))),
        membership: None,
    };
    #[derive(Clone, Copy)]
    struct Case {
        fault: Option<Fault>,
        hook: Option<Hook>,
        warm: bool,
        settled: bool,
        country: bool,
        denied: bool,
    }
    let fixture = Fixture::fixture();
    let run = |case: Case| {
        operands::boundary(true);
        if case.warm {
            for seed in &seeds {
                assert!(operands::sequence(seed.pointer + carmy::EQUIPMENT_POOL).is_some());
            }
            assert!(operands::state_sequence(9).is_some());
            assert!(operands::variant_sequence(0x5000).is_some());
        }
        let old_memory = fixture.run(case.fault, case.hook);
        let memory = fixture.run(case.fault, case.hook);
        let mut store = Store {
            world_epoch: operands::world_epoch(),
            ..Default::default()
        };
        if case.settled {
            store.capture_boundary();
            store.lifetime_epoch = store.boundary.lifetime_epoch;
        }
        let selection = selection(case.country);
        let mut old = ReferenceReads::default();
        let mut current = NativeReads::default();
        old.begin(seeds.len()).unwrap();
        current.begin(seeds.len()).unwrap();
        let capture = || {
            for seed in &seeds {
                old.capture(&old_memory, 0, 0, &store, seed, selection, None);
                current.capture(&memory, 0, 0, &store, seed, selection, None);
            }
        };
        if case.denied {
            collector_test_support::with_allocations_denied(capture);
        } else {
            let mut capture = capture;
            capture();
        }
        (current, old.into_current(), memory, old_memory)
    };
    let exact =
        |current: &NativeReads, old: &NativeReads, memory: &Fixture, old_memory: &Fixture| {
            compare(current, old);
            assert!(memory.demanded().is_subset(&old_memory.demanded()));
        };
    let clean = Case {
        fault: None,
        hook: None,
        warm: true,
        settled: true,
        country: false,
        denied: false,
    };
    let (expected, old, memory, old_memory) = run(clean);
    exact(&expected, &old, &memory, &old_memory);
    assert!(expected.complete);
    assert!(!expected.memberships.is_empty() && !expected.variants.is_empty());
    let attrition_of = |reads: &NativeReads| -> BTreeMap<_, _> {
        (reads.states.iter())
            .filter_map(|(&key, state)| Some((key, state.read.as_ref()?.values)))
            .collect()
    };
    let attrition = attrition_of(&expected);
    assert!(!attrition.is_empty());
    assert!(attrition.values().flatten().all(Result::is_ok));
    let reference = old_memory.demanded();
    for &at in &reference {
        let faults = (1..=old_memory.touches(at)).map(|nth| Fault::Once(at, nth));
        for fault in std::iter::once(Fault::Sticky(at)).chain(faults) {
            let (current, old, faulted, faulted_old) = run(Case {
                fault: Some(fault),
                ..clean
            });
            let Fault::Once(_, nth @ 2..) = fault else {
                exact(&current, &old, &faulted, &faulted_old);
                continue;
            };
            compare_but_attrition(&current, &old);
            for observed in [&current, &old] {
                for (key, values) in attrition_of(observed) {
                    for (value, clean) in values.into_iter().zip(attrition[&key]) {
                        assert!(value.is_err() || value == clean);
                    }
                }
            }
            assert!(faulted.demanded().is_subset(&reference));
            if memory.touches(at) < nth {
                assert_eq!(attrition_of(&current), attrition);
            }
        }
    }
    let mut keys = [
        engine_executable::modifier_keys::ATTRITION,
        engine_executable::modifier_keys::ATTRITION_FOR_CONTROLLER,
        engine_executable::modifier_keys::ENEMY_ATTRITION,
    ];
    keys.sort_unstable();
    let present = Value {
        raw: 41,
        present: true,
    };
    for hook in [
        (keys[0], Ok(present)),
        (keys[1], Ok(Value::default())),
        (keys[2], Err("pass snapshot refused")),
    ] {
        let (current, old, memory, old_memory) = run(Case {
            hook: Some(hook),
            ..clean
        });
        exact(&current, &old, &memory, &old_memory);
        assert!(memory.hooked.get() > 0);
        assert_eq!(memory.hooked.get(), old_memory.hooked.get());
    }
    let (current, old, memory, old_memory) = run(Case {
        country: true,
        ..clean
    });
    exact(&current, &old, &memory, &old_memory);
    assert!(current.root_indices.contains_key(&(0, 1)));
    for case in [
        Case {
            settled: false,
            ..clean
        },
        Case {
            denied: true,
            ..clean
        },
    ] {
        let (current, old, memory, old_memory) = run(case);
        exact(&current, &old, &memory, &old_memory);
    }
    let (current, old, _, _) = run(Case {
        warm: false,
        denied: true,
        ..clean
    });
    assert!(!current.complete && !old.complete);
    assert!(old.membership_observation.is_none());
    assert!(current.membership_observation.is_some());
}
