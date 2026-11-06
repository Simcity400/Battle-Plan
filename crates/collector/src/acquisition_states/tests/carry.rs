//! The States job's carried rows ([`StateStore`]): a quiet hour reads no
//! state's tags, template, building status or names, still reads every
//! state's identity, manpower and flags, and publishes exactly a fresh
//! read's rows.
use super::*;
use crate::territory_changes::{Domain, Target};
use engine_mutations::{territory_mutations::MUTATIONS, territory_mutations::Receiver};

fn domain() -> &'static Domain {
    Box::leak(Box::new(Domain::enabled(64, 64)))
}

fn store(domain: &'static Domain) -> Option<Box<StateStore>> {
    Some(Box::new(StateStore::new(domain)))
}

/// The fixture with a custom name on the first state and an owner name on
/// the last, so a carried hour must lay out the name arena as a fresh read
/// does.
fn named() -> Vec<u8> {
    use engine_native::msvc_string::LENGTH;
    let mut bytes = fixture();
    for (at, text) in [
        (FIRST + cstate::PRIMARY_DISPLAY_NAME, b"Alpha".as_slice()),
        (
            LAST + cstate::SECONDARY_DISPLAY_NAME,
            b"Bravo Land".as_slice(),
        ),
    ] {
        bytes[at..at + text.len()].copy_from_slice(text);
        put(&mut bytes, at + LENGTH, (text.len() as u64).to_le_bytes());
    }
    bytes
}

fn carried_request(store: &mut Option<Box<StateStore>>) -> Request {
    let mut request = request(2);
    request.inputs = Inputs::with_capacity(2).unwrap();
    request.store = store.take();
    request
}

/// One hour of the States job with the store.
fn hour(store: &mut Option<Box<StateStore>>, memory: &impl Memory) -> Request {
    let mut request = carried_request(store);
    let report = request.run(memory);
    assert!(matches!(report.census.result, Outcome::Rows { len: 2 }));
    *store = request.store.take();
    request
}

/// The published rows, names, fractions and flags equal a store-less read.
fn assert_fresh(carried: &Request, memory: &impl Memory) {
    let mut fresh = carried_request(&mut None);
    fresh.run(memory);
    assert_eq!(carried.census.payload, fresh.census.payload);
    assert_eq!(
        (carried.names.len, carried.names.required),
        (fresh.names.len, fresh.names.required)
    );
    assert_eq!(
        carried.names.bytes[..carried.names.len],
        fresh.names.bytes[..fresh.names.len]
    );
    assert_eq!(carried.inputs.outcome, fresh.inputs.outcome);
    for id in [1, 3] {
        assert_eq!(carried.inputs.fraction(id), fresh.inputs.fraction(id));
    }
    assert_eq!(carried.visitor.outcomes(), fresh.visitor.outcomes());
    assert_eq!(carried.visitor.rows(), fresh.visitor.rows());
}

fn reads(memory: &Traced, address: usize) -> usize {
    memory
        .calls
        .borrow()
        .iter()
        .filter(|(at, _)| *at == address)
        .count()
}

/// Which states an hour read afresh, by their owner tag reads.
fn reread(memory: &Traced) -> [bool; 2] {
    [FIRST, LAST].map(|state| reads(memory, state + cstate::OWNER) != 0)
}

#[test]
fn a_quiet_hour_carries_every_state_and_reads_only_identity_manpower_and_flags() {
    let bytes = named();
    let domain = domain();
    let mut store = store(domain);
    let first = Traced::new(&bytes);
    hour(&mut store, &first);
    assert_eq!(reread(&first), [true, true]);
    let memory = Traced::new(&bytes);
    let carried = hour(&mut store, &memory);
    // The old behaviour read both tags, the movement identity and both
    // name headers of every state.
    for state in [FIRST, LAST] {
        assert_eq!(reads(&memory, state + cstate::OWNER), 0);
        assert_eq!(reads(&memory, state + cstate::PRIMARY_DISPLAY_NAME), 0);
        assert_eq!(reads(&memory, state + cstate::MANPOWER_AVAILABLE), 1);
        assert_eq!(reads(&memory, state + cstate::DEMILITARIZED), 1);
        // The identity batch and the flags' authentication, not movement.
        assert_eq!(reads(&memory, state), 2);
    }
    assert_fresh(&carried, &Traced::new(&bytes));
    let counts = store.as_ref().unwrap().counts();
    assert_eq!((counts.reused, counts.read, counts.quiet), (2, 0, true));
    // Manpower is read every hour.
    let mut changed = bytes.clone();
    put(
        &mut changed,
        FIRST + cstate::MANPOWER_AVAILABLE,
        77_i32.to_le_bytes(),
    );
    let carried = hour(&mut store, &Traced::new(&changed));
    assert_eq!(carried.census.payload[0].manpower_available, 77);
    assert_fresh(&carried, &Traced::new(&changed));
}

/// A writer through any state site that reaches the States row marks only
/// its state; the next hour re-reads exactly that state and publishes its
/// write. Province-only and province sites re-read nothing; the name effect
/// names no state and re-reads every one.
#[test]
fn each_state_site_rereads_exactly_its_state() {
    let bytes = named();
    let mut written = bytes.clone();
    put(
        &mut written,
        LAST + cstate::OWNER,
        0x8000_0007_u32.to_le_bytes(),
    );
    for (site, (mutation, receiver)) in MUTATIONS.iter().enumerate() {
        let domain = domain();
        let mut store = store(domain);
        hour(&mut store, &Traced::new(&bytes));
        let (target, expected) = match *receiver {
            Receiver::State(rows) => (
                Target::State {
                    id: Some(3),
                    state: rows.state,
                    provinces: rows.provinces,
                },
                [false, rows.state],
            ),
            Receiver::Province => (Target::Province(Some(3)), [false, false]),
            Receiver::EveryProvince => (Target::EveryProvince, [false, false]),
            Receiver::EveryState => (Target::EveryState, [true, true]),
        };
        drop(domain.notify(site, target));
        let memory = Traced::new(&written);
        let carried = hour(&mut store, &memory);
        assert_eq!(reread(&memory), expected, "{}", mutation.name);
        if expected[1] {
            assert_fresh(&carried, &Traced::new(&written));
            assert_eq!(carried.census.payload[1].owner, 0x8000_0007);
        }
    }
}

/// A shared infrastructure operand changed (only loaders write them): every
/// state is read afresh.
#[test]
fn a_changed_infrastructure_operand_rereads_every_state() {
    struct PerLevel<'a>(&'a Traced<'a>, i64);
    impl Memory for PerLevel<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == 0x033e_5080 && destination.len() == 8 {
                destination.copy_from_slice(&self.1.to_le_bytes());
                return true;
            }
            self.0.copy(address, destination)
        }
    }
    let bytes = named();
    let mut store = store(domain());
    hour(&mut store, &Traced::new(&bytes));
    let traced = Traced::new(&bytes);
    let memory = PerLevel(&traced, -15_000);
    let carried = hour(&mut store, &memory);
    assert_eq!(reread(&traced), [true, true]);
    assert_eq!(
        carried.census.payload[0].movement_infrastructure,
        i64::from(MAXIMUM) * -15_000
    );
    assert_fresh(&carried, &memory);
    assert!(store.as_ref().unwrap().counts().full);
}

/// A writer running across the take refuses it: that hour reads every state
/// and holds nothing, so the next one reads every state too; after that the
/// writer's own state is read and the rest carried.
#[test]
fn a_refused_take_reads_everything_and_holds_nothing() {
    let bytes = named();
    let domain = domain();
    let mut store = store(domain);
    hour(&mut store, &Traced::new(&bytes));
    let running = domain.notify(
        7,
        Target::State {
            id: Some(1),
            state: true,
            provinces: true,
        },
    );
    let memory = Traced::new(&bytes);
    hour(&mut store, &memory);
    assert_eq!(reread(&memory), [true, true]);
    drop(running);
    let memory = Traced::new(&bytes);
    hour(&mut store, &memory);
    assert_eq!(reread(&memory), [true, true]);
    let memory = Traced::new(&bytes);
    let carried = hour(&mut store, &memory);
    assert_eq!(reread(&memory), [false, false]);
    assert_fresh(&carried, &memory);
}

/// A failed job forgets the store: the next hour reads every state.
#[test]
fn a_forgotten_store_reads_everything() {
    let bytes = named();
    let mut store = store(domain());
    hour(&mut store, &Traced::new(&bytes));
    store.as_mut().unwrap().forget();
    let memory = Traced::new(&bytes);
    hour(&mut store, &memory);
    assert_eq!(reread(&memory), [true, true]);
}

/// The hour's category cache is prepared with the store's other scratch, so a
/// carried hour names each shared category definition without allocating; a
/// denied allocation then cannot end the game's process.
#[test]
fn a_carried_hour_names_categories_without_allocating() {
    const CATEGORY: usize = 0x5800;
    const TOKEN: u32 = 0x2fab;
    let mut bytes = named();
    put(
        &mut bytes,
        CATEGORY,
        (cstate::CATEGORY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        CATEGORY + cstate::CATEGORY_TOKEN,
        TOKEN.to_le_bytes(),
    );
    for state in [FIRST, LAST] {
        put(
            &mut bytes,
            state + cstate::CATEGORY,
            (CATEGORY as u64).to_le_bytes(),
        );
    }
    let memory = Movement(SliceMemory {
        base: 0,
        bytes: &bytes,
    });
    let domain = domain();
    let mut store = store(domain);
    hour(&mut store, &memory);
    hour(&mut store, &memory);
    let mut request = carried_request(&mut store);
    let report = crate::test_support::with_allocations_denied(|| request.run(&memory));
    assert!(matches!(report.census.result, Outcome::Rows { len: 2 }));
    assert_eq!(
        request
            .census
            .payload
            .iter()
            .map(|row| row.category)
            .collect::<Vec<_>>(),
        [Some(TOKEN), Some(TOKEN)]
    );
    let counts = request.store.as_ref().unwrap().counts();
    assert_eq!(counts.reused, 2);
}

/// The infrastructure fixture: an ungrouped definition capped at 5 per state,
/// each state's own instance (levels 2 and 4, nothing queued) and one shared
/// category without overrides, in the dense fixture's free space.
mod infrastructure {
    use super::*;
    use engine_native::{cbuilding, cbuilding_status, cstate::infrastructure_map::Infrastructure};

    const DEFINITION: usize = 0x3000;
    const DEFINITIONS: usize = 0x3600;
    const BUILDINGS: [usize; 2] = [0x3800, 0x4a00];
    const CATEGORY: usize = 0x5800;
    const TOKEN: u32 = 0x2fab;
    // The building database's infrastructure definitions array.
    const INFRASTRUCTURE_DEFINITIONS: usize = super::DATABASE + 0x280;
    const NO_GROUP: u32 = 0x165;
    const MAXIMUM_BONUS: usize = 0x1e8;

    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put(bytes, at, (data as u64).to_le_bytes());
        put(bytes, at + 8, count.to_le_bytes());
        put(bytes, at + 12, count.to_le_bytes());
    }

    fn world() -> Vec<u8> {
        let mut bytes = named();
        put(&mut bytes, DEFINITIONS, (DEFINITION as u64).to_le_bytes());
        put(&mut bytes, DEFINITION, (cbuilding_template::VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, DEFINITION + cbuilding_template::TYPE_ID, TOKEN.to_le_bytes());
        bytes[DEFINITION + cbuilding_template::LOOKUP_ENABLED_CANDIDATE] = 1;
        put(&mut bytes, DEFINITION + 0x2b8, 5_i32.to_le_bytes());
        put(&mut bytes, DEFINITION + 0x2c4, NO_GROUP.to_le_bytes());
        put(&mut bytes, CATEGORY, (cstate::CATEGORY_VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, CATEGORY + cstate::CATEGORY_TOKEN, TOKEN.to_le_bytes());
        for ((state, building), level) in [FIRST, LAST].into_iter().zip(BUILDINGS).zip([2_i16, 4]) {
            let status = state + cstate::BUILDING_STATUS;
            put(&mut bytes, state + cstate::CATEGORY, (CATEGORY as u64).to_le_bytes());
            put(&mut bytes, status + cbuilding_status::BUILDING_INDEX_COUNT, 1_i32.to_le_bytes());
            header(&mut bytes, status + cbuilding_status::BUILDING_INDICES, building + 0x200, 1);
            header(&mut bytes, status + cbuilding_status::BUILDINGS, building + 0x210, 1);
            put(&mut bytes, building + 0x210, (building as u64).to_le_bytes());
            put(&mut bytes, building, (cbuilding::VTABLE_RVA.0 as u64).to_le_bytes());
            put(&mut bytes, building + cbuilding::TEMPLATE, (DEFINITION as u64).to_le_bytes());
            put(&mut bytes, building + cbuilding::LEVEL, level.to_le_bytes());
        }
        bytes
    }

    /// The database's definitions header, far above the dense fixture.
    struct Definitions<'a, M>(&'a M);

    impl<M: Memory> Memory for Definitions<'_, M> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == INFRASTRUCTURE_DEFINITIONS && destination.len() == 16 {
                let mut header = [0; 16];
                header[..8].copy_from_slice(&(DEFINITIONS as u64).to_le_bytes());
                header[8..12].copy_from_slice(&1_u32.to_le_bytes());
                header[12..].copy_from_slice(&1_u32.to_le_bytes());
                destination.copy_from_slice(&header);
                return true;
            }
            self.0.copy(address, destination)
        }
    }

    fn values(request: &Request) -> Vec<Option<Infrastructure>> {
        request.census.payload[..2].iter().map(|row| row.infrastructure).collect()
    }

    fn infrastructure(level: i32, maximum: i32) -> Option<Infrastructure> {
        Some(Infrastructure { building_type: TOKEN, level, maximum })
    }

    /// A carried quiet hour publishes each state's infrastructure from one
    /// gather of the instances' queue records: no state, status, instance
    /// level or definition read. A changed level bonus is read and published.
    #[test]
    fn a_quiet_hour_proves_infrastructure_without_reading_a_state() {
        let mut bytes = world();
        let mut store = store(domain());
        let fresh = hour(&mut store, &Definitions(&Traced::new(&bytes)));
        assert_eq!(values(&fresh), [infrastructure(2, 5), infrastructure(4, 5)]);
        let traced = Traced::new(&bytes);
        let memory = Definitions(&traced);
        let carried = hour(&mut store, &memory);
        assert_eq!(reread(&traced), [false, false]);
        assert_eq!(values(&carried), values(&fresh));
        for (state, building) in [FIRST, LAST].into_iter().zip(BUILDINGS) {
            let status = state + cstate::BUILDING_STATUS;
            for address in [
                status + cbuilding_status::BUILDING_INDEX_COUNT,
                status + cbuilding_status::BUILDING_INDICES,
                status + cbuilding_status::BUILDINGS,
                status + cbuilding_status::COUNTRY,
                building,
                building + cbuilding::LEVEL,
            ] {
                assert_eq!(reads(&traced, address), 0, "{address:#x}");
            }
            assert_eq!(reads(&traced, building + MAXIMUM_BONUS), 1);
        }
        assert_eq!(reads(&traced, DEFINITION + 0x2b8), 0);
        assert_fresh(&carried, &memory);
        // The bonus has no hooked writer: the gathered record shows the change.
        put(&mut bytes, BUILDINGS[0] + MAXIMUM_BONUS, (-2_i16).to_le_bytes());
        let traced = Traced::new(&bytes);
        let memory = Definitions(&traced);
        let changed = hour(&mut store, &memory);
        assert_eq!(values(&changed), [infrastructure(2, 3), infrastructure(4, 5)]);
        assert_fresh(&changed, &memory);
    }
}
