use super::*;
use crate::{Memory, carmy, corder_instance as order, corders_group as group, cunit};

const ARMY: usize = 0x1000;
const HQ: usize = 0x2000;
const GROUP: usize = 0x3000;
const LEADER: usize = 0x4000;
const ROOT: usize = 0x6000;
const ROOTS: usize = 0x7000;
const MEMBERS: usize = 0x8000;
const TERRAIN: usize = 0x9000;
const PARENT: usize = 0xa000;

struct Observed {
    fixture: std::cell::RefCell<Fixture>,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    fault: std::cell::Cell<Option<usize>>,
}
impl Observed {
    fn new(fixture: Fixture) -> Self {
        Self {
            fixture: std::cell::RefCell::new(fixture),
            reads: Default::default(),
            fault: Default::default(),
        }
    }
    fn count(&self, address: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == address)
            .count()
    }
}
impl Memory for Observed {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        self.fault.get() != Some(address) && self.fixture.borrow().copy(address, destination)
    }
}

fn primary_fixture(count: u32) -> Fixture {
    let mut fixture = Fixture::new();
    fixture.root(GROUP, group::PRIMARY_ORDER_DATA, ARMY);
    fixture.array(ROOT + order::ASSIGNED_MEMBER_DATA, MEMBERS, count);
    fixture.bytes[ARMY + crate::corders_group_member::HOST_OFFSET + PRIMARY_ROOT_DIRTY] = 1;
    fixture.bytes[HQ + crate::corders_group_member::HOST_OFFSET + PRIMARY_ROOT_DIRTY] = 1;
    fixture
}

#[test]
fn primary_prefix_preserves_lazy_chunks_and_reuses_members_without_source_reads() {
    let mut fixture = primary_fixture(129);
    let member = ARMY + crate::corders_group_member::HOST_OFFSET;
    let hq = HQ + crate::corders_group_member::HOST_OFFSET;
    fixture.word(MEMBERS + 8, hq as u64);
    let source = Observed::new(fixture);
    source.fault.set(Some(MEMBERS + 512));
    let mut cache = PrimaryRootCache::new(8, 4);
    let allocated = cache.allocated_bytes();
    assert_eq!(cache.resolve(&source, 0, member), Ok(ROOT));
    assert_eq!(cache.resolve(&source, 0, hq), Ok(ROOT));
    assert_eq!(cache.resolve(&source, 0, member), Ok(ROOT));
    assert_eq!(source.count(MEMBERS), 1);
    assert_eq!(source.count(MEMBERS + 512), 0);
    // Each query validates its current member state and prefix header.
    assert_eq!(source.count(member + PRIMARY_ROOT_DIRTY), 2);
    assert_eq!(source.count(ROOT + order::ASSIGNED_MEMBER_DATA), 3);
    cache.clear();
    assert_eq!(source.count(MEMBERS), 1);
    assert_eq!(cache.allocated_bytes(), allocated);
    assert!(cache.chunks.is_empty());
    assert_eq!(cache.resolve(&source, 0, member), Ok(ROOT));
    assert_eq!(source.count(MEMBERS), 2);
}

#[test]
fn primary_zero_capacity_matches_direct_trace_and_keeps_first_root_and_clean_staleness() {
    let mut fixture = primary_fixture(1);
    // A later primary root and all parent roots are unreachable after the
    // first match, including when a warmed membership lookup supplies it.
    fixture.array(GROUP + group::PRIMARY_ORDER_DATA, ROOTS, 2);
    fixture.word(ROOTS + 8, 1);
    fixture.word(GROUP + group::PARENT, 1);
    let source = Observed::new(fixture);
    let member = ARMY + crate::corders_group_member::HOST_OFFSET;
    assert_eq!(movement_primary_root(&source, 0, member), Ok(ROOT));
    let direct = source.reads.borrow().clone();
    source.reads.borrow_mut().clear();
    let mut empty = PrimaryRootCache::new(0, 0);
    assert_eq!(empty.resolve(&source, 0, member), Ok(ROOT));
    assert_eq!(*source.reads.borrow(), direct);
    let mut cache = PrimaryRootCache::new(8, 4);
    assert_eq!(cache.resolve(&source, 0, member), Ok(ROOT));
    source.fixture.borrow_mut().bytes[member + PRIMARY_ROOT_DIRTY] = 0;
    assert_eq!(
        cache.resolve(&source, 0, member),
        Err("movement primary root clean cache is stale")
    );
    source
        .fixture
        .borrow_mut()
        .word(member + PRIMARY_ROOT_CACHE, ROOT as u64);
    assert_eq!(cache.resolve(&source, 0, member), Ok(ROOT));
    assert_eq!(source.count(ROOTS + 8), 0);
    assert_eq!(source.count(1), 0);
    cache.clear();
}

#[test]
fn primary_prefix_capacity_resumes_unread_suffix_and_failed_chunks_are_not_retained() {
    let mut fixture = primary_fixture(129);
    let member = ARMY + crate::corders_group_member::HOST_OFFSET;
    let hq = HQ + crate::corders_group_member::HOST_OFFSET;
    fixture.word(MEMBERS + 128 * 8, hq as u64);
    let source = Observed::new(fixture);
    let mut cache = PrimaryRootCache::new(1, 1);
    assert_eq!(cache.resolve(&source, 0, member), Ok(ROOT));
    source.fault.set(Some(MEMBERS + 512));
    assert_eq!(
        cache.resolve(&source, 0, hq),
        Err("movement assigned members unreadable")
    );
    assert_eq!(cache.chunks.len(), 1);
    assert_eq!(source.count(MEMBERS), 1);
    source.fault.set(None);
    assert_eq!(cache.resolve(&source, 0, hq), Ok(ROOT));
    assert_eq!(source.count(MEMBERS), 1);
    assert_eq!(source.count(MEMBERS + 512), 2);
    assert_eq!(source.count(MEMBERS + 1024), 1);
    cache.clear();

    let mut cache = PrimaryRootCache::new(8, 4);
    source.fault.set(Some(MEMBERS));
    assert_eq!(
        cache.resolve(&source, 0, member),
        Err("movement assigned members unreadable")
    );
    assert!(cache.chunks.is_empty());
    source.fault.set(None);
    assert_eq!(cache.resolve(&source, 0, member), Ok(ROOT));
    assert_eq!(cache.chunks.len(), 1);
    cache.clear();
}

#[test]
fn primary_prefix_collision_preserves_native_fallback_and_clear_reuses_storage() {
    let mut fixture = primary_fixture(65);
    let member = ARMY + crate::corders_group_member::HOST_OFFSET;
    let hq = HQ + crate::corders_group_member::HOST_OFFSET;
    fixture.word(MEMBERS + 512, hq as u64);
    let source = Observed::new(fixture);
    let mut cache = PrimaryRootCache::new(1, 4);
    assert_eq!(cache.resolve(&source, 0, hq), Ok(ROOT));
    let header = crate::read::<[u8; 16]>(&source, ROOT + order::ASSIGNED_MEMBER_DATA).unwrap();
    let members = crate::cpdx_array::Array::from_header(header, 8).unwrap();
    let before = source.count(MEMBERS);
    assert!(
        cache
            .contains(
                &source,
                ROOT + 8,
                ROOT + order::ASSIGNED_MEMBER_DATA,
                header,
                members,
                member
            )
            .unwrap()
    );
    assert_eq!(source.count(MEMBERS), before + 1);
    assert_eq!(cache.prefixes[0].unwrap().root, ROOT);
    assert_eq!(cache.chunks.len(), 2);
    let reads = source.reads.borrow().len();
    cache.clear();
    assert_eq!(source.reads.borrow().len(), reads);
    assert!(cache.chunks.is_empty());
    assert!(cache.prefixes.iter().all(Option::is_none));
    assert!(cache.index.iter().all(|code| *code == 0));
}

#[test]
fn primary_prefix_rejects_changed_headers_and_preserves_owned_byte_accounting_on_growth() {
    let source = Observed::new(primary_fixture(1));
    let member = ARMY + crate::corders_group_member::HOST_OFFSET;
    let mut cache = PrimaryRootCache::new(8, 4);
    assert_eq!(cache.resolve(&source, 0, member), Ok(ROOT));
    source
        .fixture
        .borrow_mut()
        .array(ROOT + order::ASSIGNED_MEMBER_DATA, MEMBERS, 0);
    assert_eq!(
        cache.resolve(&source, 0, member),
        Err("movement assigned members changed")
    );
    cache.clear();
    cache.grow(16, 8).unwrap();
    let allocated = cache.allocated_bytes();
    assert_eq!(
        allocated,
        cache.metadata.allocated_bytes()
            + cache.prefixes.capacity() * std::mem::size_of::<Option<PrimaryPrefix>>()
            + cache.chunks.capacity() * std::mem::size_of::<PrimaryChunk>()
            + cache.index.capacity() * std::mem::size_of::<u32>()
    );
    cache.grow(1, 1).unwrap();
    assert_eq!(cache.allocated_bytes(), allocated);
    assert_eq!(cache.resolve(&source, 0, member), Ok(0));
    cache.clear();
}

#[test]
fn primary_hook_remains_behind_existing_hq_feature_and_identity_gates() {
    struct Hook {
        source: Observed,
        cache: std::cell::RefCell<PrimaryRootCache>,
        calls: std::cell::Cell<usize>,
    }
    impl Memory for Hook {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.source.copy(address, destination)
        }
        fn movement_primary_root(
            &self,
            base: usize,
            member: usize,
        ) -> Option<Result<usize, &'static str>> {
            self.calls.set(self.calls.get() + 1);
            Some(self.cache.borrow_mut().resolve(&self.source, base, member))
        }
    }
    let hook = Hook {
        source: Observed::new(primary_fixture(1)),
        cache: std::cell::RefCell::new(PrimaryRootCache::new(8, 4)),
        calls: Default::default(),
    };
    hook.source.fixture.borrow_mut().flags = 0;
    assert_eq!(
        movement_accumulator(&hook, 0, ARMY),
        Ok(Some(LEADER + 0x418))
    );
    assert_eq!(hook.calls.get(), 0);
    hook.source.fixture.borrow_mut().flags = 1 << 17;
    hook.source.fault.set(Some(HQ));
    assert!(movement_accumulator(&hook, 0, ARMY).is_err());
    assert_eq!(hook.calls.get(), 0);
    hook.source.fault.set(None);
    assert_eq!(
        movement_accumulator(&hook, 0, ARMY),
        Ok(Some(LEADER + 0x4d8))
    );
    assert_eq!(hook.calls.get(), 2);
    assert_eq!(hook.source.count(MEMBERS), 1);
    hook.cache.borrow_mut().clear();
}

struct Fixture {
    bytes: Vec<u8>,
    flags: u32,
}

impl Fixture {
    fn new() -> Self {
        let mut f = Self {
            bytes: vec![0; 0xc000],
            flags: 1 << 17,
        };
        f.word(ARMY + carmy::ORDERS_GROUP, GROUP as u64);
        f.word(HQ, carmy::VTABLE_RVA as u64);
        f.word(
            HQ + carmy::SECONDARY_BASE,
            carmy::SECONDARY_VTABLE_RVA as u64,
        );
        f.word(HQ + cunit::HANDLE, 1);
        f.word(HQ + carmy::ORDERS_GROUP, GROUP as u64);
        f.word(GROUP, group::VTABLE_RVA as u64);
        f.word(GROUP + group::HANDLE, group::REFERENCE_TYPE as u64);
        f.word(GROUP + group::LEADER, LEADER as u64);
        f.word(
            GROUP + group::HQ_PROVIDER,
            (HQ + crate::corders_group_member::HOST_OFFSET) as u64,
        );
        f.word(LEADER, VTABLE_RVA as u64);
        f.word(
            LEADER + crate::cunit_leader::TYPE,
            crate::cunit_leader::REFERENCE_TYPE as u64,
        );
        f
    }
    fn word(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.word(at, data as u64);
        self.bytes[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
        self.bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
    }
    fn root(&mut self, owner: usize, offset: usize, unit: usize) {
        self.array(owner + offset, ROOTS, 1);
        self.word(ROOTS, ROOT as u64);
        self.word(ROOT, order::VTABLE_RVA as u64);
        self.word(ROOT + order::OWNER, owner as u64);
        self.array(ROOT + order::ASSIGNED_MEMBER_DATA, MEMBERS, 1);
        self.word(
            MEMBERS,
            (unit + crate::corders_group_member::HOST_OFFSET) as u64,
        );
    }
}
impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address == crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA && destination.len() == 4
        {
            destination.copy_from_slice(&self.flags.to_le_bytes());
            return true;
        }
        crate::memory::SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, destination)
    }
}

#[test]
fn accumulator_absence_default_and_feature_short_circuit() {
    let mut f = Fixture::new();
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(Some(LEADER + 0x418)));
    f.word(GROUP + group::HQ_PROVIDER, 1);
    f.flags = 0;
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(Some(LEADER + 0x418)));
    f.flags = 1 << 17;
    f.word(ARMY + cunit::CLASS, 1);
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(Some(LEADER + 0x418)));
    f.word(GROUP + group::LEADER, 0);
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(None));
    f.word(ARMY + carmy::ORDERS_GROUP, 0);
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(None));
}

#[test]
fn a_nonzero_class_never_reads_the_trade_feature_flags() {
    let mut fixture = Fixture::new();
    fixture.flags = 1 << 17;
    fixture.word(ARMY + cunit::CLASS, 1);
    let source = Observed::new(fixture);
    assert_eq!(
        movement_accumulator(&source, 0, ARMY),
        Ok(Some(LEADER + 0x418))
    );
    assert_eq!(
        source.count(crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA),
        0
    );
}

#[test]
fn seeded_accumulator_uses_its_primary_assignment_without_rereading_it() {
    let mut fixture = Fixture::new();
    fixture.flags = 0;
    let source = Observed::new(fixture);
    source.fault.set(Some(ARMY + cunit::CLASS));
    let seed = carmy::DivisionSeed {
        pointer: ARMY,
        orders_group: Some(GROUP),
        class: Some(0),
        ..Default::default()
    };
    assert_eq!(
        movement_accumulator_seeded(&source, 0, &seed),
        Ok(Some(LEADER + 0x418))
    );
    assert_eq!(source.count(ARMY + carmy::ORDERS_GROUP), 0);
    assert_eq!(source.count(ARMY + cunit::CLASS), 0);
    source
        .fixture
        .borrow_mut()
        .word(ARMY + carmy::ORDERS_GROUP, 0);
    assert_eq!(
        movement_accumulator_seeded(&source, 0, &seed),
        Ok(Some(LEADER + 0x418))
    );
}

#[test]
fn seeded_accumulator_keeps_faults_and_class_branch_lazy() {
    let source = Observed::new(Fixture::new());
    let mut seed = carmy::DivisionSeed {
        pointer: ARMY,
        ..Default::default()
    };
    assert_eq!(
        movement_accumulator_seeded(&source, 0, &seed),
        Err("movement leader field unreadable")
    );
    assert!(source.reads.borrow().is_empty());

    seed.orders_group = Some(0);
    source
        .fixture
        .borrow_mut()
        .word(ARMY + carmy::ORDERS_GROUP, 0);
    assert_eq!(movement_accumulator_seeded(&source, 0, &seed), Ok(None));
    seed.orders_group = Some(GROUP);
    source
        .fixture
        .borrow_mut()
        .word(ARMY + carmy::ORDERS_GROUP, GROUP as u64);
    source.fixture.borrow_mut().word(GROUP + group::LEADER, 0);
    assert_eq!(movement_accumulator_seeded(&source, 0, &seed), Ok(None));

    source
        .fixture
        .borrow_mut()
        .word(GROUP + group::LEADER, LEADER as u64);
    assert_eq!(
        movement_accumulator_seeded(&source, 0, &seed),
        Err("movement leader field unreadable")
    );
    assert_eq!(source.count(ARMY + cunit::CLASS), 0);
}

/// The memoized selection gives each division the seeded selection or its
/// first failure, reading the group, its leader, the HQ provider with the
/// HQ's primary root and the feature flags once a pass; each division's own
/// member is still its own read, while the root orders and member chunks its
/// own root resolves through are read once a pass.
#[test]
fn memoized_selection_matches_seeded_and_reads_group_inputs_once_a_pass() {
    let member = |army: usize| army + crate::corders_group_member::HOST_OFFSET;
    let mut fixture = Fixture::new();
    fixture.root(GROUP, group::PRIMARY_ORDER_DATA, ARMY);
    fixture.bytes[member(ARMY) + PRIMARY_ROOT_DIRTY] = 1;
    let division = |pointer, class| carmy::DivisionSeed {
        pointer,
        orders_group: Some(GROUP),
        class: Some(class),
        ..Default::default()
    };
    // ARMY's root differs from its HQ's (alternate); the HQ division's own
    // root is its HQ's; a nonzero class never reaches the HQ.
    let seeds = [division(ARMY, 0), division(HQ, 0), division(ARMY, 1)];
    for fault in [
        None,
        Some(GROUP),
        Some(GROUP + group::HQ_PROVIDER),
        Some(MEMBERS),
    ] {
        let seeded = Observed::new(Fixture {
            bytes: fixture.bytes.clone(),
            flags: fixture.flags,
        });
        seeded.fault.set(fault);
        let expected = seeds.map(|seed| movement_accumulator_seeded(&seeded, 0, &seed));
        if fault.is_none() {
            assert_eq!(
                expected,
                [
                    Ok(Some(LEADER + 0x4d8)),
                    Ok(Some(LEADER + 0x418)),
                    Ok(Some(LEADER + 0x418))
                ]
            );
        }
        let memoized = Observed::new(Fixture {
            bytes: fixture.bytes.clone(),
            flags: fixture.flags,
        });
        memoized.fault.set(fault);
        let mut memo = LeaderMemo::try_with_capacity(seeds.len()).unwrap();
        for pass in 0..2 {
            memo.clear();
            memoized.reads.borrow_mut().clear();
            let actual =
                seeds.map(|seed| movement_accumulator_memoized(&memoized, 0, &seed, &mut memo));
            assert_eq!(actual, expected, "{fault:?} pass {pass}");
            // The group's leader word, the HQ provider and the flags are each
            // read at most once a pass, a failed read included.
            for address in [
                GROUP + group::LEADER,
                GROUP + group::HQ_PROVIDER,
                crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA,
            ] {
                assert!(memoized.count(address) <= 1, "{fault:?} {address:#x}");
            }
            assert!(memoized.reads.borrow().len() < seeded.reads.borrow().len());
        }
        match fault {
            None => {
                assert_eq!(memoized.count(GROUP + group::LEADER), 1);
                assert_eq!(seeded.count(GROUP + group::LEADER), seeds.len());
                assert_eq!(memoized.count(GROUP + group::HQ_PROVIDER), 1);
                assert_eq!(seeded.count(GROUP + group::HQ_PROVIDER), 2);
                // Both class-zero divisions still read their own member's
                // root; the HQ's is read once for the group.
                assert_eq!(memoized.count(member(ARMY) + PRIMARY_ROOT_DIRTY), 1);
                assert_eq!(memoized.count(member(HQ) + PRIMARY_ROOT_DIRTY), 2);
                // The root order's members: once for the HQ's root and once
                // for both divisions' own roots, which share the chunk.
                assert_eq!(memoized.count(MEMBERS), 2);
                assert_eq!(seeded.count(MEMBERS), 4);
            }
            Some(GROUP) => {
                assert_eq!(memoized.count(GROUP), 1);
                assert_eq!(seeded.count(GROUP), seeds.len());
            }
            Some(_) => {}
        }
    }
    // A memo with no room selects exactly as the seeded selection.
    let seeded = Observed::new(Fixture {
        bytes: fixture.bytes.clone(),
        flags: fixture.flags,
    });
    let unkept = Observed::new(Fixture {
        bytes: fixture.bytes.clone(),
        flags: fixture.flags,
    });
    let mut vacant = LeaderMemo::vacant();
    for seed in seeds {
        assert_eq!(
            movement_accumulator_memoized(&unkept, 0, &seed, &mut vacant),
            movement_accumulator_seeded(&seeded, 0, &seed)
        );
    }
    assert!(unkept.count(GROUP + group::LEADER) == seeds.len());
}

#[test]
fn different_primary_roots_select_alternate_and_clean_staleness_is_not_repaired() {
    let mut f = Fixture::new();
    f.root(GROUP, group::PRIMARY_ORDER_DATA, ARMY);
    assert!(movement_accumulator(&f, 0, ARMY).is_err());
    f.bytes[ARMY + crate::corders_group_member::HOST_OFFSET + PRIMARY_ROOT_DIRTY] = 1;
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(Some(LEADER + 0x4d8)));
    f.bytes[ARMY + crate::corders_group_member::HOST_OFFSET + PRIMARY_ROOT_DIRTY] = 0;
    f.word(ARMY + carmy::PRIMARY_ORDER, ROOT as u64);
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(Some(LEADER + 0x4d8)));
    f.array(ROOT + order::ASSIGNED_MEMBER_DATA, MEMBERS, 2);
    f.word(
        MEMBERS + 8,
        (HQ + crate::corders_group_member::HOST_OFFSET) as u64,
    );
    f.word(HQ + carmy::PRIMARY_ORDER, ROOT as u64);
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(Some(LEADER + 0x418)));
}

/// The land combat skill hover tests separate-HQ eligibility of a side's units
/// for whichever leader the side selected; it is the movement block's choice.
#[test]
fn separate_hq_order_is_the_movement_block_choice_without_the_leader_block() {
    let mut f = Fixture::new();
    f.root(GROUP, group::PRIMARY_ORDER_DATA, ARMY);
    f.bytes[ARMY + crate::corders_group_member::HOST_OFFSET + PRIMARY_ROOT_DIRTY] = 1;
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(Some(LEADER + 0x4d8)));
    assert_eq!(separate_hq_order(&f, 0, ARMY), Ok(true));
    f.flags = 0;
    assert_eq!(separate_hq_order(&f, 0, ARMY), Ok(false));
    f.flags = 1 << 17;
    f.word(ARMY + cunit::CLASS, 1);
    assert_eq!(separate_hq_order(&f, 0, ARMY), Ok(false));
    f.word(ARMY + cunit::CLASS, 0);
    assert_eq!(separate_hq_order(&f, 0, ARMY), Ok(true));
    // The deployed-leader predicate needs the group's own leader.
    f.word(GROUP + group::LEADER, 0);
    assert_eq!(separate_hq_order(&f, 0, ARMY), Ok(false));
}

#[test]
fn only_primary_roots_and_immediate_parent_participate() {
    let mut f = Fixture::new();
    f.root(GROUP, group::SECONDARY_ORDER_DATA, ARMY);
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(Some(LEADER + 0x418)));
    f.word(GROUP + group::PARENT, PARENT as u64);
    f.word(PARENT, crate::carmy_group::VTABLE_RVA as u64);
    f.word(PARENT + group::HANDLE, group::REFERENCE_TYPE as u64);
    f.root(PARENT, group::PRIMARY_ORDER_DATA, ARMY);
    f.bytes[ARMY + crate::corders_group_member::HOST_OFFSET + PRIMARY_ROOT_DIRTY] = 1;
    assert_eq!(movement_accumulator(&f, 0, ARMY), Ok(Some(LEADER + 0x4d8)));
    f.word(ROOT + order::OWNER, GROUP as u64);
    assert!(movement_accumulator(&f, 0, ARMY).is_err());
}

#[test]
fn terrain_preserves_signed_width_and_native_index_zero_fallback() {
    let mut f = Fixture::new();
    f.array(LEADER + MOVEMENT_TERRAIN_DATA, TERRAIN, 2);
    f.word(TERRAIN + MOVEMENT_TERRAIN_SPEED, (-7_i64) as u64);
    f.word(
        TERRAIN + MOVEMENT_TERRAIN_STRIDE + MOVEMENT_TERRAIN_SPEED,
        i64::MAX as u64,
    );
    assert_eq!(movement_terrain(&f, 0, LEADER, 1), Ok(i64::MAX));
    for index in [-1, 0, 2, i32::MAX] {
        assert_eq!(movement_terrain(&f, 0, LEADER, index), Ok(-7));
    }
    f.array(LEADER + MOVEMENT_TERRAIN_DATA, TERRAIN, 0);
    assert_eq!(movement_terrain(&f, 0, LEADER, 0), Ok(-7));
    f.word(LEADER + MOVEMENT_TERRAIN_DATA, 0);
    assert!(movement_terrain(&f, 0, LEADER, 0).is_err());
    assert!(movement_terrain(&f, usize::MAX, LEADER, 0).is_err());
}

#[test]
fn counter_dig_in_factor_keeps_native_one_and_wrapping_commander_modifier() {
    struct Definitions<'a>(&'a Fixture);
    impl Memory for Definitions<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == crate::modifier_definition::TABLE_RVA && output.len() == 8 {
                output.copy_from_slice(&0x3000_0000_u64.to_le_bytes());
                return true;
            }
            if (0x3000_0000..0x3004_0000).contains(&address) {
                output.fill(0);
                return true;
            }
            self.0.copy(address, output)
        }
    }
    let mut fixture = Fixture::new();
    fixture.flags = 0;
    let mut seed = carmy::DivisionSeed {
        pointer: ARMY,
        orders_group: Some(0),
        class: Some(0),
        ..Default::default()
    };
    assert_eq!(
        carmy::counter_dig_in_factor(&Definitions(&fixture), 0, &seed),
        Ok(100_000)
    );
    seed.orders_group = Some(GROUP);
    fixture.array(
        LEADER + MOVEMENT_MODIFIER + MODIFIER_ACCUMULATOR,
        MEMBERS,
        1,
    );
    fixture.word(MEMBERS, crate::modifier_keys::COMBAT_ENTRENCHMENT as u64);
    for modifier in [-150_000_i64, 17, i64::MAX] {
        fixture.word(MEMBERS + 8, modifier as u64);
        assert_eq!(
            carmy::counter_dig_in_factor(&Definitions(&fixture), 0, &seed),
            Ok(100_000_i64.wrapping_add(modifier))
        );
    }
}

#[test]
fn prepared_movement_ranges_preserve_serial_faults_and_shared_admission() {
    const DIVISIONS: usize = 310;
    let mut fixture = primary_fixture(129);
    fixture.word(
        MEMBERS + 8,
        (HQ + crate::corders_group_member::HOST_OFFSET) as u64,
    );
    let seeds: Vec<_> = (0..DIVISIONS)
        .map(|index| carmy::DivisionSeed {
            pointer: if index % 3 == 0 { HQ } else { ARMY },
            orders_group: match index % 11 {
                0 => None,
                1 => Some(0),
                _ => Some(GROUP),
            },
            class: match index % 7 {
                0 => None,
                1 => Some(1),
                _ => Some(0),
            },
            ..Default::default()
        })
        .collect();
    for fault in [
        None,
        Some(GROUP),
        Some(GROUP + group::LEADER),
        Some(LEADER),
        Some(crate::modifier_definition::TRADE_FEATURE_FLAGS_RVA),
        Some(GROUP + group::HQ_PROVIDER),
        Some(ROOTS),
        Some(MEMBERS),
        Some(MEMBERS + 512),
        Some(ARMY + crate::corders_group_member::HOST_OFFSET + PRIMARY_ROOT_DIRTY),
    ] {
        let source = Observed::new(Fixture {
            bytes: fixture.bytes.clone(),
            flags: fixture.flags,
        });
        source.fault.set(fault);
        let mut serial = LeaderMemo::try_with_capacity(DIVISIONS).unwrap();
        let expected: Vec<_> = seeds
            .iter()
            .map(|seed| movement_block_memoized(&source, 0, seed, &mut serial))
            .collect();
        for parts in [1, 2, 7, 31] {
            let mut ranges: Vec<_> = (0..parts)
                .map(|part| {
                    LeaderMemo::try_with_capacity(
                        DIVISIONS * (part + 1) / parts - DIVISIONS * part / parts,
                    )
                    .unwrap()
                })
                .collect();
            let mut prepared = vec![MovementPreparation::default(); DIVISIONS];
            let mut ordered = LeaderMemo::try_with_capacity(DIVISIONS).unwrap();
            let mut bytes = None;
            for _ in 0..2 {
                ordered.clear();
                for part in (0..parts).rev() {
                    ranges[part].clear();
                    for index in DIVISIONS * part / parts..DIVISIONS * (part + 1) / parts {
                        prepared[index] = MovementPreparation::capture(
                            &source,
                            0,
                            &seeds[index],
                            &mut ranges[part],
                        );
                    }
                }
                let reads = source.reads.borrow().len();
                let actual: Vec<_> = seeds
                    .iter()
                    .enumerate()
                    .map(|(index, seed)| {
                        movement_block_prepared(
                            &source,
                            0,
                            seed,
                            &mut ordered,
                            Some(prepared[index]),
                        )
                    })
                    .collect();
                assert_eq!(actual, expected, "parts={parts} fault={fault:?}");
                assert_eq!(
                    source.reads.borrow().len(),
                    reads,
                    "settlement reread staged leaves"
                );
                let current = ranges
                    .iter()
                    .map(LeaderMemo::allocated_bytes)
                    .sum::<usize>();
                if let Some(bytes) = bytes {
                    assert_eq!(current, bytes, "range scratch grew on the second hour");
                }
                bytes = Some(current);
                source.reads.borrow_mut().clear();
            }
        }
    }

    // A later range's contradictory observation cannot replace a shared
    // group's admitted result (success or refusal), nor its feature/HQ gate.
    let source = Fixture::new();
    let seed = carmy::DivisionSeed {
        pointer: ARMY,
        orders_group: Some(GROUP),
        class: Some(1),
        ..Default::default()
    };
    let mut memo = LeaderMemo::try_with_capacity(2).unwrap();
    let first = MovementPreparation {
        leader: Some(Ok(LEADER)),
        ..Default::default()
    };
    let later = MovementPreparation {
        leader: Some(Err("later group fault")),
        ..Default::default()
    };
    let admitted = movement_block_prepared(&source, 0, &seed, &mut memo, Some(first));
    assert_eq!(
        movement_block_prepared(&source, 0, &seed, &mut memo, Some(later)),
        admitted
    );
    memo.clear();
    let refused = movement_block_prepared(&source, 0, &seed, &mut memo, Some(later));
    assert_eq!(refused, Err("later group fault"));
    assert_eq!(
        movement_block_prepared(&source, 0, &seed, &mut memo, Some(first)),
        refused
    );
}
