use super::*;
#[cfg(test)]
use engine_native::{carmy, cequipment_variant::{self as variant, pool as pool}};
#[cfg(test)]
use std::{hint::black_box, time::Instant};
#[cfg(all(test, feature = "tap-foreign-probe"))]
mod foreign_unwind;

static SERIAL: Mutex<()> = Mutex::new(());

pub fn with_active_observer(check: impl FnOnce()) {
    Call::enter(SiteId::Land).run(check);
}

#[cfg(test)]
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub fn fresh_demand() -> std::sync::MutexGuard<'static, ()> {
    let serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    HOURLY_PEAK_EVENTS.store(0, Ordering::Relaxed);
    EVENT_REQUIRED.store(0, Ordering::Relaxed);
    serial
}
#[cfg(test)]
static FORWARDED: Mutex<[usize; 8]> = Mutex::new([0; 8]);
#[cfg(test)]
static OBSERVED_DELETION: Mutex<Option<Deletion>> = Mutex::new(None);

#[cfg(test)]
fn folded() -> std::sync::MutexGuard<'static, Store> {
    let mut store = STORE.lock().unwrap();
    store.fold();
    store
}
#[cfg(test)]
fn poison() {
    folded().poisoned = true;
}
#[cfg(test)]
fn scoped(context: Deletion) -> DeletionScope {
    let id = next_id();
    folded().scopes.insert(id, context);
    DeletionScope::enter(Some(Scope {
        id,
        army: context.army,
        handle: context.handle,
        country: context.country,
        ignored: context.ignored,
        merge_source: context.merge_source,
        observed: false,
    }))
}
#[cfg(test)]
fn deletion_context(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    caller: usize,
) -> Result<Option<Deletion>, &'static str> {
    let Some((scope, expeditionary_owner, classified)) =
        deletion_scope(memory, base, state, army, caller)?
    else {
        return Ok(None);
    };
    let mut store = folded();
    store.delete(scope, expeditionary_owner, classified);
    Ok(store.scopes.get(&scope.id).copied())
}
#[cfg(test)]
fn resolved_scope() -> Option<Deletion> {
    let scope = DELETION.get()?;
    let mut store = STORE
        .try_lock()
        .expect("writer callbacks never hold the drain's store");
    store.fold();
    store.scopes.get(&scope.id).copied()
}
#[cfg(test)]
static GATE: std::sync::LazyLock<std::sync::Barrier> =
    std::sync::LazyLock::new(|| std::sync::Barrier::new(2));
#[cfg(test)]
unsafe extern "system-unwind" fn gated_subtract(pool: usize, design: usize, amount: usize) {
    GATE.wait();
    GATE.wait();
    unsafe { subtract(pool, design, amount) };
}
#[cfg(test)]
static HELD_AFTER_READ: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
pub(super) fn after_pool_read(site: SiteId) {
    if HELD_AFTER_READ
        .compare_exchange(site as usize + 1, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        GATE.wait();
        GATE.wait();
    }
}

#[cfg(test)]
struct World {
    storage: Vec<u64>,
    designs: usize,
}
#[cfg(test)]
impl World {
    fn new(designs: usize) -> Self {
        let mut world = Self {
            storage: vec![0; (0x600 + designs * 0x40) / 8],
            designs,
        };
        world.word(0, carmy::VTABLE_RVA.0 as u64);
        world.word(carmy::SECONDARY_BASE, carmy::SECONDARY_VTABLE_RVA.0 as u64);
        world.word(engine_native::cunit::HANDLE, 0xe00100004b01);
        world.owner(1);
        let p = world.army() + carmy::EQUIPMENT_POOL;
        world.word(carmy::EQUIPMENT_POOL, pool::VTABLE_RVA.0 as u64);
        world.word(
            carmy::EQUIPMENT_POOL + pool::ITEMS,
            (world.army() + 0x600) as u64,
        );
        world.word(
            carmy::EQUIPMENT_POOL + pool::CAPACITY,
            (designs as u64) << 32 | designs as u64,
        );
        for d in 0..designs {
            let v = world.design(d);
            world.word(0x600 + d * 16, v as u64);
            world.word(
                0x600 + designs * 16 + d * 0x30,
                engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64,
            );
            world.word(
                0x600 + designs * 16 + d * 0x30 + 8,
                ((d as u64 + 0xf000) << 32) | variant::REFERENCE_TYPE as u64,
            );
            world.amount(d, 900000);
        }
        assert_eq!(p, world.army() + carmy::EQUIPMENT_POOL);
        world
    }
    fn army(&self) -> usize {
        self.storage.as_ptr() as usize
    }
    fn design(&self, d: usize) -> usize {
        self.army() + 0x600 + self.designs * 16 + d * 0x30
    }
    fn word(&mut self, at: usize, value: u64) {
        assert_eq!(at % 8, 0);
        self.storage[at / 8] = value;
    }
    fn dword(&mut self, at: usize, value: u32) {
        assert_eq!(at % 4, 0);
        let shift = (at % 8) * 8;
        let word = &mut self.storage[at / 8];
        *word = (*word & !(u64::from(u32::MAX) << shift)) | (u64::from(value) << shift);
    }
    fn rebase(&mut self, base: usize) {
        self.word(0, (base + carmy::VTABLE_RVA.0) as u64);
        self.word(
            carmy::SECONDARY_BASE,
            (base + carmy::SECONDARY_VTABLE_RVA.0) as u64,
        );
        self.word(carmy::EQUIPMENT_POOL, (base + pool::VTABLE_RVA.0) as u64);
        for d in 0..self.designs {
            self.word(
                0x600 + self.designs * 16 + d * 0x30,
                (base + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0) as u64,
            );
        }
    }
    fn owner(&mut self, country: u32) {
        self.dword(engine_native::cunit::ROSTER_OWNER, country);
    }
    fn amount(&mut self, d: usize, amount: i64) {
        self.word(0x600 + d * 16 + 8, amount as u64);
    }
    fn restore_pool(&mut self) {
        self.word(carmy::EQUIPMENT_POOL, pool::VTABLE_RVA.0 as u64);
        self.word(
            carmy::EQUIPMENT_POOL + pool::ITEMS,
            (self.army() + 0x600) as u64,
        );
        self.word(
            carmy::EQUIPMENT_POOL + pool::CAPACITY,
            (self.designs as u64) << 32 | self.designs as u64,
        );
        for d in 0..self.designs {
            self.amount(d, 900001);
        }
    }
}

#[cfg(test)]
struct ReserveWorld {
    world: World,
    manager: Vec<u64>,
    _country: Vec<u64>,
}
#[cfg(test)]
impl ReserveWorld {
    #[cfg(test)]
    fn new(designs: usize, owner: u32) -> Self {
        use engine_native::{ccountry, cproduction_status};
        let world = World::new(designs);
        let mut manager = vec![0u64; 0x300 / 8];
        let mut country = vec![0u64; (ccountry::PRODUCTION_STATUS + 8) / 8];
        manager[0] = cproduction_status::VTABLE_RVA.0 as u64;
        manager[cproduction_status::COUNTRY / 8] = country.as_ptr() as u64;
        country[0] = ccountry::VTABLE_RVA.0 as u64;
        country[ccountry::ID / 8] = u64::from(owner);
        country[ccountry::PRODUCTION_STATUS / 8] = manager.as_ptr() as u64;
        let offset = cproduction_status::STOCKPILE_POOL / 8;
        manager[offset..offset + 8].copy_from_slice(
            &world.storage[carmy::EQUIPMENT_POOL / 8..carmy::EQUIPMENT_POOL / 8 + 8],
        );
        Self {
            world,
            manager,
            _country: country,
        }
    }
    #[cfg(test)]
    fn for_source(source: &World, base: usize, owner: u32, initial: i64) -> Self {
        let mut reserves = Self::new(source.designs, owner);
        reserves.world.rebase(base);
        reserves.manager[0] = (base + engine_native::cproduction_status::VTABLE_RVA.0) as u64;
        reserves._country[0] = (base + engine_native::ccountry::VTABLE_RVA.0) as u64;
        reserves.manager[engine_native::cproduction_status::STOCKPILE_POOL / 8] =
            (base + pool::VTABLE_RVA.0) as u64;
        for index in 0..source.designs {
            reserves.world.word(
                0x600 + index * equipment_entry::STRIDE,
                source.design(index) as u64,
            );
            reserves.world.amount(index, initial);
        }
        reserves
    }
    #[cfg(test)]
    fn pool(&self) -> usize {
        self.manager.as_ptr() as usize + engine_native::cproduction_status::STOCKPILE_POOL
    }
}

#[cfg(test)]
struct StateMemory {
    state: Vec<u64>,
    country: Vec<u64>,
    countries: Vec<u64>,
    links: Vec<u32>,
    roster: Vec<u64>,
    armies: Vec<u64>,
}
#[cfg(test)]
impl StateMemory {
    #[cfg(test)]
    fn new(designs: u32) -> Self {
        let mut memory = Self {
            state: vec![0; 0xb00 / 8],
            country: vec![0; 0x300 / 8],
            countries: Vec::new(),
            links: Vec::new(),
            roster: Vec::new(),
            armies: Vec::new(),
        };
        memory.state[engine_native::cgame_state::EQUIPMENT_VARIANTS / 8] =
            memory.state.as_ptr() as u64;
        memory.state[engine_native::cgame_state::EQUIPMENT_VARIANTS_CAPACITY / 8] =
            u64::from(designs) | (u64::from(designs) << 32);
        memory.population(&[0]);
        memory
    }
    #[cfg(test)]
    fn population(&mut self, counts: &[u32]) {
        self.country.resize(counts.len() * 0x1000 / 8, 0);
        self.countries.resize(counts.len() + 1, 0);
        self.links = (0..=counts.len() as u32).collect();
        self.roster
            .resize(counts.iter().map(|&n| n as usize).sum(), 0);
        self.armies.resize(self.roster.len() * 0x500 / 8, 0);
        let designs = self.state[engine_native::cgame_state::EQUIPMENT_VARIANTS_CAPACITY / 8];
        for (index, pointer) in self.roster.iter_mut().enumerate() {
            let army = &mut self.armies[index * 0x500 / 8..][..0x500 / 8];
            *pointer = army.as_ptr() as u64;
            let pool = &mut army[carmy::EQUIPMENT_POOL / 8..];
            pool[0] = pool::VTABLE_RVA.0 as u64;
            pool[pool::ITEMS / 8] = pool.as_ptr() as u64;
            pool[pool::CAPACITY / 8] = designs;
        }
        let mut roster_slot = 0;
        for (index, &count) in counts.iter().enumerate() {
            let country = &mut self.country[index * 0x1000 / 8..][..0x1000 / 8];
            country[0] = engine_native::ccountry::VTABLE_RVA.0 as u64;
            country[engine_native::ccountry::ID / 8] = index as u64 + 1;
            country[engine_native::ccountry::DIVISIONS / 8] =
                (self.roster.as_ptr() as usize + roster_slot * 8) as u64;
            country[engine_native::ccountry::DIVISIONS_CAPACITY / 8] =
                u64::from(count) | (u64::from(count) << 32);
            self.countries[index + 1] = country.as_ptr() as u64;
            roster_slot += count as usize;
        }
        let slots = self.countries.len() as u64;
        self.state[engine_native::cgame_state::COUNTRIES / 8] = self.countries.as_ptr() as u64;
        self.state[engine_native::cgame_state::COUNTRIES_CAPACITY / 8] = slots | (slots << 32);
        self.state[engine_native::cgame_state::COUNTRY_LINKS / 8] = self.links.as_ptr() as u64;
        self.state[engine_native::cgame_state::COUNTRY_LINKS_CAPACITY / 8] = slots | (slots << 32);
    }
    #[cfg(test)]
    fn resetting(&mut self, value: bool) {
        let index = native::RESETTING / 8;
        let shift = (native::RESETTING % 8) * 8;
        self.state[index] = (self.state[index] & !(0xff << shift)) | (u64::from(value) << shift);
    }
    #[cfg(test)]
    fn live_base(&mut self) -> usize {
        let state = self.state.as_ptr() as usize;
        self.state[0] = state as u64;
        state
            .checked_sub(engine_native::cgame_state::SINGLETON_RVA.0)
            .unwrap()
    }
}
#[cfg(test)]
impl Memory for StateMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address == engine_native::cgame_state::SINGLETON_RVA.0 && destination.len() == 8 {
            destination.copy_from_slice(&(self.state.as_ptr() as u64).to_le_bytes());
            true
        } else {
            unsafe { engine_native_memory::memory::LiveMemory::new() }.copy(address, destination)
        }
    }
}

#[cfg(test)]
unsafe extern "system-unwind" fn transfer(manager: usize, source: usize) {
    assert!(STORE.try_lock().is_ok());
    unsafe {
        credit_pool(manager, source);
    }
}
#[cfg(test)]
unsafe extern "system-unwind" fn credit_pool(manager: usize, source: usize) {
    let target = native::reserve_pool(manager).unwrap();
    let source_items = unsafe { *((source + pool::ITEMS) as *const usize) };
    let target_items = unsafe { *((target + pool::ITEMS) as *const usize) };
    let sources = unsafe { *((source + pool::COUNT) as *const i32) };
    let targets = unsafe { *((target + pool::COUNT) as *const i32) };
    for i in 0..sources as usize {
        let source = source_items + i * equipment_entry::STRIDE;
        let variant = unsafe { *(source as *const usize) };
        let quantity = unsafe { *((source + equipment_entry::AMOUNT) as *const i64) };
        let at = (0..targets as usize)
            .find(|&index| unsafe {
                *((target_items + index * equipment_entry::STRIDE) as *const usize) == variant
            })
            .unwrap();
        let amount =
            (target_items + at * equipment_entry::STRIDE + equipment_entry::AMOUNT) as *mut i64;
        unsafe {
            *amount = (i128::from(*amount) + i128::from(quantity))
                .clamp(-0x53e2d620fb40, 0x53e2d620fb40) as i64;
        }
    }
}
#[cfg(test)]
unsafe extern "system-unwind" fn forwarding_delete(
    state: usize,
    army: usize,
    return_equipment: usize,
    behavior: usize,
) {
    *FORWARDED.lock().unwrap() = [state, army, return_equipment, behavior, 0, 0, 0, 0];
    *OBSERVED_DELETION.lock().unwrap() = resolved_scope();
    assert_eq!(ACTIVE[SiteId::Delete as usize].load(Ordering::SeqCst), 1);
    assert!(active() != 0);
}
#[cfg(test)]
unsafe extern "system-unwind" fn forwarding_queue(state: usize, army: usize) {
    *FORWARDED.lock().unwrap() = [state, army, 0, 0, 0, 0, 0, 0];
    assert_eq!(ACTIVE[SiteId::Queue as usize].load(Ordering::SeqCst), 1);
    assert!(active() != 0);
}
#[cfg(test)]
unsafe extern "system-unwind" fn reserve_credit(manager: usize, variant: usize, quantity: usize) {
    assert!(
        STORE.try_lock().is_ok(),
        "native reserve credit must not retain STORE"
    );
    let bank = native::reserve_pool(manager).unwrap();
    let raw = -(quantity as u32 as i64 * equipment_entry::FIXED_POINT_SCALE);
    unsafe { subtract(bank, variant, raw as usize) };
}
#[cfg(test)]
unsafe extern "system-unwind" fn cleanup_delete(
    state: usize,
    army: usize,
    return_equipment: usize,
    _behavior: usize,
) {
    *FORWARDED.lock().unwrap() = [state, army, return_equipment, _behavior, 0, 0, 0, 0];
    *OBSERVED_DELETION.lock().unwrap() = resolved_scope();
    assert!(active() != 0);
    assert_eq!(state, unsafe {
        *((BASE.load(Ordering::Relaxed) + engine_native::cgame_state::SINGLETON_RVA.0)
            as *const usize)
    });
    let bank = army + carmy::EQUIPMENT_POOL;
    let items = unsafe { *((bank + pool::ITEMS) as *const usize) };
    let design = unsafe { *(items as *const usize) };
    if return_equipment != 0 {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let design_count = unsafe { *((bank + pool::COUNT) as *const i32) } as usize;
        let mut reserve = ReserveWorld::new(design_count, 1);
        let base = BASE.load(Ordering::Relaxed);
        reserve.world.rebase(base);
        reserve.manager[0] = (base + engine_native::cproduction_status::VTABLE_RVA.0) as u64;
        reserve._country[0] = (base + engine_native::ccountry::VTABLE_RVA.0) as u64;
        reserve.manager[engine_native::cproduction_status::STOCKPILE_POOL / 8] =
            (base + pool::VTABLE_RVA.0) as u64;
        for index in 0..design_count {
            let source =
                engine_native_memory::read::<u64>(&memory, items + index * equipment_entry::STRIDE)
                    .unwrap();
            reserve
                .world
                .word(0x600 + index * equipment_entry::STRIDE, source);
            reserve.world.amount(index, 0);
        }
        unsafe { returned(reserve.manager.as_ptr() as usize, bank) };
    }
    unsafe {
        *((army + engine_native::cunit::EXPEDITIONARY_OWNER) as *mut u32) = 0;
        land(
            army,
            design,
            if return_equipment == 0 { 100000 } else { 0 },
            0,
            0,
            0,
            0,
            0,
            BASE.load(Ordering::Relaxed) + native::LAND_COMBAT_RETURN.0,
        );
        division_free(army, 1);
    }
}

#[cfg(test)]
unsafe extern "system-unwind" fn queue_metadata_during_delete(
    _state: usize,
    army: usize,
    _return_equipment: usize,
    _behavior: usize,
) {
    record_queue(
        &mut folded(),
        army,
        unsafe { *((army + engine_native::cunit::HANDLE) as *const u64) },
        native::QueuedContext::Transfer(native::Disposition::HqTransferred),
    )
    .unwrap();
}
#[cfg(test)]
unsafe extern "system-unwind" fn erase(army: usize, flags: usize) -> usize {
    *FORWARDED.lock().unwrap() = [army, flags, 0, 0, 0, 0, 0, 0];
    assert_eq!(
        ACTIVE[SiteId::DivisionFree as usize].load(Ordering::SeqCst),
        1
    );
    assert!(active() != 0);
    let pool = army + carmy::EQUIPMENT_POOL;
    unsafe {
        *(pool as *mut u64) = 0;
        *((pool + pool::ITEMS) as *mut u64) = 0;
        *((pool + pool::CAPACITY) as *mut u64) = 0;
    }
    army
}
#[cfg(test)]
unsafe extern "system-unwind" fn disrupt(
    army: usize,
    source: usize,
    equipment: usize,
    manpower: usize,
    organisation: usize,
) {
    *FORWARDED.lock().unwrap() = [army, source, equipment, manpower, organisation, 0, 0, 0];
    let bank = army + carmy::EQUIPMENT_POOL;
    let items = unsafe { *((bank + pool::ITEMS) as *const usize) };
    let count = unsafe { *((bank + pool::COUNT) as *const i32) };
    for i in 0..count as usize {
        let amount = (items + i * equipment_entry::STRIDE + equipment_entry::AMOUNT) as *mut i64;
        let divided = unsafe { *amount } as i128 * equipment_entry::FIXED_POINT_SCALE as i128
            / equipment as i64 as i128;
        let floored = (divided / equipment_entry::FIXED_POINT_SCALE as i128)
            * equipment_entry::FIXED_POINT_SCALE as i128;
        unsafe { *amount = floored as i64 };
    }
}

#[cfg(test)]
unsafe extern "system-unwind" fn subtract(pool: usize, design: usize, amount: usize) {
    let items = unsafe { *((pool + pool::ITEMS) as *const usize) };
    let count = unsafe { *((pool + pool::COUNT) as *const i32) };
    for i in 0..count as usize {
        let row = (items + i * 16) as *mut u64;
        if unsafe { *row } == design as u64 {
            let limit = 0x53e2d620fb40i128;
            unsafe {
                *row.add(1) = ((*row.add(1) as i64 as i128 - amount as i64 as i128)
                    .clamp(-limit, limit) as i64) as u64;
            }
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
unsafe extern "system-unwind" fn forwarding(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
) {
    *FORWARDED.lock().unwrap() = [a, b, c, d, e, f, g, h];
    assert_eq!(ACTIVE[SiteId::Land as usize].load(Ordering::SeqCst), 1);
    assert!(active() != 0);
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
unsafe extern "system-unwind" fn land_removal(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
) {
    unsafe {
        forwarding(a, b, c, d, e, f, g, h);
        subtract(a + carmy::EQUIPMENT_POOL, b, c);
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
unsafe extern "system-unwind" fn land_scale_and_ceiling(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
) {
    unsafe { forwarding(a, b, c, d, e, f, g, h) };
    let bank = a + carmy::EQUIPMENT_POOL;
    unsafe { fraction_scale(bank, (100000i64 - b as i64) as usize) };
    let items = unsafe { *((bank + pool::ITEMS) as *const usize) };
    let count = unsafe { *((bank + pool::COUNT) as *const i32) };
    for index in 0..count as usize {
        let raw = (items + index * equipment_entry::STRIDE + equipment_entry::AMOUNT) as *mut i64;
        let amount = unsafe { *raw };
        let scale = equipment_entry::FIXED_POINT_SCALE;
        unsafe {
            *raw = (amount.div_euclid(scale) + i64::from(amount.rem_euclid(scale) != 0)) * scale;
        }
    }
}

#[cfg(test)]
unsafe extern "system-unwind" fn air_request(wing: usize, request: usize, source: usize) {
    assert!(STORE.try_lock().is_ok());
    *FORWARDED.lock().unwrap() = [wing, request, source, 0, 0, 0, 0, 0];
    let bank = wing + engine_native_air::cair_wing::EQUIPMENT_POOL;
    let items = unsafe { *((bank + pool::ITEMS) as *const usize) };
    let design = unsafe { *(items as *const usize) };
    let amount = i64::from(request as u32 as i32) * equipment_entry::FIXED_POINT_SCALE;
    unsafe { subtract(bank, design, amount as usize) };
}

#[cfg(test)]
fn assert_signed_row(amount_raw: i64, cause: native::Cause, variant: u64) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let mut captured = crate::extended::equipment_destruction::Captured::with_capacities(&8);
    captured.capture(&memory, 0, 0);
    let [
        replay_frame::frame::TableView::Complete(
            replay_data::rows::TableRows::EquipmentDestruction(rows),
        ),
        replay_frame::frame::TableView::Complete(
            replay_data::rows::TableRows::DivisionEquipmentDeletions(deleted),
        ),
    ] = captured.tables()
    else {
        panic!("native correction must publish both complete tables");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].amount_raw, amount_raw);
    assert_eq!(rows[0].country, 4999);
    assert_eq!(rows[0].cause, cause.into());
    assert_eq!(rows[0].variant, variant);
    assert!(deleted.is_empty());
    assert!(!folded().poisoned);
    assert!(folded().refusal.is_none());
}

#[cfg(test)]
fn refusal_diagnostics() -> (u64, [u64; native::SITES.len()], serde_json::Value) {
    drop(folded());
    (
        REFUSALS.load(Ordering::Relaxed),
        std::array::from_fn(|site| SITE_REFUSALS[site].load(Ordering::Relaxed)),
        serde_json::to_value(FIRST_REFUSAL.get()).unwrap(),
    )
}

#[cfg(test)]
fn contested() -> (u64, serde_json::Value) {
    let status = serde_json::to_value(status()).unwrap();
    (
        status["contested_pools"].as_u64().unwrap_or(u64::MAX),
        status["contested_pool"].clone(),
    )
}

#[cfg(test)]
fn record_land_gain(before: i64, damage: i64, expected: i64) {
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(1);
    world.owner(4999);
    world.amount(0, before);
    ORIGINAL[SiteId::Land as usize].store(
        land_scale_and_ceiling as *const () as usize,
        Ordering::Release,
    );
    let diagnostics = refusal_diagnostics();
    unsafe {
        land(
            world.army(),
            damage as usize,
            0,
            0,
            0,
            0,
            0,
            0,
            native::LAND_COMBAT_RETURN.0,
        );
    }
    assert_signed_row(expected, native::Cause::LandCombat, 0xf00000000046);
    assert_eq!(refusal_diagnostics(), diagnostics);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn negative_damage_records_signed_gain_without_poison() {
    let _serial = serial();
    record_land_gain(1000000, -10000, -100000);
}

#[test]
fn fractional_pool_rounding_up_records_signed_gain_without_poison() {
    let _serial = serial();
    record_land_gain(1030000, 1000, -70000);
}

#[test]
fn negative_pool_entries_record_signed_differences_without_poison() {
    let _serial = serial();
    record_land_gain(-1000000, 50000, -500000);
}

#[test]
fn scripted_factor_above_one_records_signed_gain_without_poison() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(1);
    world.owner(4999);
    world.amount(0, 1030000);
    let diagnostics = refusal_diagnostics();
    unsafe { scripted_equipment_fraction(world.army() + carmy::EQUIPMENT_POOL, 200000) };
    assert_signed_row(
        -1030000,
        native::Cause::ScriptedEquipmentFraction,
        0xf00000000046,
    );
    assert_eq!(refusal_diagnostics(), diagnostics);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn negative_air_request_records_signed_gain_without_poison() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(1);
    world.amount(0, 1000000);
    let mut wing = vec![0u64; 0xa00 / 8];
    let pool_offset = engine_native_air::cair_wing::EQUIPMENT_POOL;
    wing[pool_offset / 8] = pool::VTABLE_RVA.0 as u64;
    wing[(pool_offset + pool::ITEMS) / 8] = (world.army() + 0x600) as u64;
    wing[(pool_offset + pool::CAPACITY) / 8] = (1u64 << 32) | 1;
    let address = wing.as_mut_ptr() as usize;
    unsafe { *((address + 0x9c4) as *mut u32) = 4999 };
    ORIGINAL[SiteId::Air as usize].store(air_request as *const () as usize, Ordering::Release);
    let diagnostics = refusal_diagnostics();
    let family = collector_timing::hook_timing::Family::EquipmentDestruction;
    let _sampled = collector_timing::hook_timing::sample_all();
    let timed = collector_timing::hook_timing::counts(family);
    unsafe { air(address, (-2i32) as usize, 1500, 0) };
    let after = collector_timing::hook_timing::counts(family);
    assert!(after.calls > timed.calls && after.sampled > timed.sampled);
    assert!(after.original > timed.original && after.detour > timed.detour);
    assert_signed_row(-200000, native::Cause::Unknown, 0xf00000000046);
    assert_eq!(refusal_diagnostics(), diagnostics);
    assert_eq!(
        *FORWARDED.lock().unwrap(),
        [address, (-2i32) as usize, 1500, 0, 0, 0, 0, 0]
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

core::arch::global_asm!(
    ".text", ".globl destruction_land_test_caller", ".seh_proc destruction_land_test_caller",
    "destruction_land_test_caller:", "sub rsp, 0x48", ".seh_stackalloc 0x48", ".seh_endprologue",
    "mov rax, [rsp + 0x70]", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0x78]", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0x80]", "mov [rsp + 0x30], rax",
    "mov rax, [rsp + 0x88]", "mov [rsp + 0x38], rax",
    "call {entry}", ".globl destruction_land_test_return", "destruction_land_test_return:",
    "add rsp, 0x48", "ret", ".seh_endproc", entry = sym destruction_land_entry);
#[cfg(test)]
unsafe extern "system-unwind" {
    fn destruction_land_test_caller();
    fn destruction_land_test_return();
}

core::arch::global_asm!(
    ".text", ".globl destruction_delete_test_caller", ".seh_proc destruction_delete_test_caller",
    "destruction_delete_test_caller:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
    "call {delete}", ".globl destruction_delete_test_return", "destruction_delete_test_return:",
    "add rsp, 0x28", "ret", ".seh_endproc",
    ".globl destruction_free_test_caller", ".seh_proc destruction_free_test_caller",
    "destruction_free_test_caller:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
    "call {free}", ".globl destruction_free_test_return", "destruction_free_test_return:",
    "add rsp, 0x28", "ret", ".seh_endproc",
    ".globl destruction_queue_test_caller", ".seh_proc destruction_queue_test_caller",
    "destruction_queue_test_caller:", "sub rsp, 0x28", ".seh_stackalloc 0x28", ".seh_endprologue",
    "call {queue}", ".globl destruction_queue_test_return", "destruction_queue_test_return:",
    "add rsp, 0x28", "ret", ".seh_endproc",
    delete = sym delete_entry, free = sym division_free_entry, queue = sym queue_entry,
);
#[cfg(test)]
unsafe extern "system-unwind" {
    fn destruction_delete_test_caller();
    fn destruction_delete_test_return();
    fn destruction_free_test_caller();
    fn destruction_queue_test_caller();
    fn destruction_queue_test_return();
}

#[cfg(test)]
fn record_queue(
    store: &mut Store,
    army: usize,
    handle: u64,
    classified: native::QueuedContext,
) -> Result<(), &'static str> {
    let (cause, disposition) = match classified {
        native::QueuedContext::Loss(cause) => (cause, native::Disposition::Loss),
        native::QueuedContext::Transfer(disposition) => (native::Cause::Unknown, disposition),
        native::QueuedContext::Ignore => panic!("ignored queues have no fixture provenance"),
    };
    if disposition == native::Disposition::HqTransferred && !store.credits.contains(handle) {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        if native::army_identity(&memory, BASE.load(Ordering::Relaxed), army) == Ok(handle)
            && let Ok(rows) = pool_rows(
                &memory,
                BASE.load(Ordering::Relaxed),
                army + carmy::EQUIPMENT_POOL,
            )
        {
            let revision = store.credits.revision();
            store
                .credits
                .begin(handle, army, native::army_owner(&memory, army)?, revision)?;
            for row in &rows {
                store.credits.push(
                    handle,
                    Net {
                        country: 0,
                        variant: row.variant_handle,
                        amount: 0,
                        credited: i128::from(row.amount_raw),
                    },
                )?;
            }
        }
    }
    store.record_queue(army, handle, cause, disposition)
}

#[cfg(test)]
fn setup(capacity: usize, designs: usize) {
    install(Store {
        entries: Vec::with_capacity(capacity),
        queued: FixedIndex::with_capacity(capacity),
        queued_by_army: FixedIndex::with_capacity(capacity),
        queued_limit: capacity,
        credits: Credits::try_new(capacity, capacity * designs).unwrap(),
        ..Store::new()
    });
}

#[cfg(test)]
fn install(store: Store) {
    for active in &ACTIVE {
        active.store(0, Ordering::SeqCst);
    }
    OPEN.store(0, Ordering::SeqCst);
    HELD_AFTER_READ.store(0, Ordering::SeqCst);
    drop(LOG.take());
    let queued = store.queued_limit;
    *STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = store;
    STORE.clear_poison();
    drop(folded());
    *CLOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    CLOCK.clear_poison();
    DELETION.set(None);
    WHOLE_MERGE.set(false);
    OBSERVING.set(false);
    FREEING.set(0);
    BASE.store(0, Ordering::Relaxed);
    QUEUED_CAPACITY.store(queued, Ordering::Release);
    HOURLY_PEAK_EVENTS.store(0, Ordering::Relaxed);
    HOURLY_PEAK_CAPACITY.store(0, Ordering::Relaxed);
    EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
    CALL_ATTEMPTS.store(0, Ordering::Relaxed);
    EVENT_REQUIRED.store(0, Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(true, Ordering::SeqCst);
    UNAVAILABLE.store(false, Ordering::Relaxed);
    SKIPPED_QUEUE.store(false, Ordering::SeqCst);
    ORIGINAL[SiteId::Attrition as usize].store(subtract as *const () as usize, Ordering::Release);
    ORIGINAL[SiteId::Delete as usize]
        .store(forwarding_delete as *const () as usize, Ordering::Release);
    ORIGINAL[SiteId::Return as usize].store(transfer as *const () as usize, Ordering::Release);
    ORIGINAL[SiteId::HqReturn as usize].store(transfer as *const () as usize, Ordering::Release);
    ORIGINAL[SiteId::DivisionFree as usize].store(erase as *const () as usize, Ordering::Release);
    ORIGINAL[SiteId::Queue as usize]
        .store(forwarding_queue as *const () as usize, Ordering::Release);
    ORIGINAL[SiteId::Paradrop as usize].store(disrupt as *const () as usize, Ordering::Release);
    ORIGINAL[SiteId::SetExiled as usize].store(
        exile_scale_and_round as *const () as usize,
        Ordering::Release,
    );
    for site in [
        SiteId::VolunteerReturnLoss,
        SiteId::CapitulationEscapeLoss,
        SiteId::ScriptedEquipmentFraction,
        SiteId::ScriptedReserveEquipmentFraction,
    ] {
        ORIGINAL[site as usize].store(fraction_scale as *const () as usize, Ordering::Release);
    }
    for site in [SiteId::VolunteerTransfer, SiteId::ExileTransfer] {
        ORIGINAL[site as usize].store(bench_roster_remove as *const () as usize, Ordering::Release);
    }
}
#[cfg(test)]
fn clock(hour: i32, session: i64) -> Option<Clock> {
    Some(Clock {
        hour,
        play_session_started_at: session,
        game_state: 0,
        play_time: 0,
        seed: 0,
        campaign_id: Default::default(),
        speed: 0,
        tick_busy: 0,
    })
}

#[test]
fn refusals_identify_each_writer_and_recover_at_clean_boundary() {
    let _serial = serial();
    setup(8, 1);
    for site in 0..native::SITES.len() {
        boundary(clock(1, site as i64 + 1));
        boundary(clock(2, site as i64 + 1));
        let count = SITE_REFUSALS[site].load(Ordering::Relaxed);
        folded()
            .refuse(Some(site), "synthetic destruction refusal");
        let held = folded();
        let refusal = serde_json::to_value(held.refusal).unwrap();
        assert_eq!(refusal["site_index"], site);
        assert_eq!(refusal["reason"], "synthetic destruction refusal");
        let status = serde_json::to_value(status()).unwrap();
        assert!(!status["refusal"].is_null());
        assert_eq!(
            status["refusal"],
            serde_json::to_value(FIRST_REFUSAL.get()).unwrap()
        );
        drop(held);
        assert_eq!(status["hooks"][site]["site_index"], site);
        assert_eq!(status["hooks"][site]["refusals"], count + 1);
        assert!(status["branch_unavailable"].as_bool().unwrap());
        boundary(clock(3, site as i64 + 1));
        assert_eq!(drain(&mut Vec::new()), Err("synthetic destruction refusal"));
        assert!(!folded().poisoned);
        boundary(clock(4, site as i64 + 1));
        assert_eq!(drain(&mut Vec::new()), Ok(0));
        assert_eq!(
            serde_json::to_value(super::status()).unwrap()["refusal"],
            status["refusal"]
        );
    }
    let first = serde_json::to_value(status()).unwrap()["refusal"].clone();
    boundary(clock(1, 10));
    assert_eq!(serde_json::to_value(status()).unwrap()["refusal"], first);
    assert!(!folded().poisoned);
}

#[test]
fn supplemental_snapshot_refusals_identify_writer_and_preserve_first_diagnostic() {
    let _serial = serial();
    let site = SiteId::ScriptedEquipmentFraction;
    for (designs, bad_vtable, quantity, reason) in
        [(1, true, 900000, "variant vtable or reference type mismatch")]
    {
        setup(8, designs);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut world = World::new(1);
        world.amount(0, quantity);
        if bad_vtable {
            world.word(0x600 + world.designs * 16, 0);
        }
        let before = refusal_diagnostics();
        let bank = world.army() + carmy::EQUIPMENT_POOL;
        unsafe { scripted_equipment_fraction(bank, 100000) };
        let after = refusal_diagnostics();
        assert_eq!(after.0, before.0 + 1);
        let mut counts = before.1;
        counts[site as usize] += 1;
        assert_eq!(after.1, counts);
        let diagnostic = serde_json::json!({ "site_index": site as usize, "reason": reason });
        assert_eq!(
            after.2,
            if before.2.is_null() {
                diagnostic.clone()
            } else {
                before.2
            }
        );
        let store = folded();
        assert_eq!(serde_json::to_value(store.refusal).unwrap(), diagnostic);
        assert!(store.entries.is_empty());
        drop(store);
        assert_eq!(*FORWARDED.lock().unwrap(), [bank, 100000, 0, 0, 0, 0, 0, 0]);
        assert!(folded().poisoned);
        assert_eq!(drain(&mut vec![Entry::default(); 8]), Err(reason));
    }
    for quantity in [0x53e2d620fb41, -0x53e2d620fb41] {
        setup(8, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut world = World::new(1);
        world.amount(0, quantity);
        let before = refusal_diagnostics();
        let bank = world.army() + carmy::EQUIPMENT_POOL;
        unsafe { scripted_equipment_fraction(bank, 200000) };
        assert_eq!(refusal_diagnostics(), before);
        assert!(!folded().poisoned);
        let mut entries = vec![Entry::default(); 8];
        assert_eq!(drain(&mut entries).unwrap(), 1);
        assert_eq!(entries[0].amount_raw, -quantity);
        assert_eq!(entries[0].cause, native::Cause::ScriptedEquipmentFraction);
        assert_eq!(*FORWARDED.lock().unwrap(), [bank, 200000, 0, 0, 0, 0, 0, 0]);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn a_held_drain_store_never_refuses_or_delays_a_writer() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(1);
    world.owner(4999);
    let bank = world.army() + carmy::EQUIPMENT_POOL;
    let before = refusal_diagnostics();
    let store = STORE.lock().unwrap();
    for _ in 0..2 {
        unsafe { attrition(bank, world.design(0), 100000) };
    }
    drop(store);
    assert_eq!(world.storage[(0x600 + 8) / 8] as i64, 700000);
    let mut out = vec![Entry::default(); 8];
    assert_eq!(drain(&mut out), Ok(2));
    assert!(out[..2].iter().all(|row| row.amount_raw == 100000
        && row.country == 4999
        && row.cause == native::Cause::Attrition));
    assert_eq!(refusal_diagnostics(), before);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn two_threads_in_the_family_at_once_keep_every_pool_change() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut attrited = World::new(1);
    attrited.owner(4998);
    let mut damaged = World::new(1);
    damaged.owner(4999);
    ORIGINAL[SiteId::Attrition as usize]
        .store(gated_subtract as *const () as usize, Ordering::Release);
    ORIGINAL[SiteId::Land as usize].store(land_removal as *const () as usize, Ordering::Release);
    let (bank, design) = (attrited.army() + carmy::EQUIPMENT_POOL, attrited.design(0));
    let contests = contested();
    let writer = std::thread::spawn(move || unsafe { attrition(bank, design, 100000) });
    GATE.wait();
    unsafe {
        land(
            damaged.army(),
            damaged.design(0),
            200000,
            0,
            0,
            0,
            0,
            0,
            native::LAND_COMBAT_RETURN.0,
        );
    }
    GATE.wait();
    writer.join().unwrap();
    let mut out = vec![Entry::default(); 8];
    let drained = drain(&mut out);
    assert_eq!(drained, Ok(2));
    assert_eq!(
        (out[0].country, out[0].cause, out[0].amount_raw),
        (4999, native::Cause::LandCombat, 200000)
    );
    assert_eq!(
        (out[1].country, out[1].cause, out[1].amount_raw),
        (4998, native::Cause::Attrition, 100000)
    );
    assert_eq!(damaged.storage[(0x600 + 8) / 8] as i64, 700000);
    assert_eq!(attrited.storage[(0x600 + 8) / 8] as i64, 800000);
    assert_eq!(active(), 0);
    assert_eq!(contested(), contests);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn two_threads_writing_one_pool_at_once_refuse_their_interval_by_name_and_name_the_pool() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(1);
    world.owner(4997);
    let (count, _) = contested();
    ORIGINAL[SiteId::Attrition as usize]
        .store(gated_subtract as *const () as usize, Ordering::Release);
    ORIGINAL[SiteId::Land as usize].store(land_removal as *const () as usize, Ordering::Release);
    let (bank, design) = (world.army() + carmy::EQUIPMENT_POOL, world.design(0));
    let writer = std::thread::spawn(move || unsafe { attrition(bank, design, 100000) });
    GATE.wait();
    unsafe {
        land(
            world.army(),
            world.design(0),
            200000,
            0,
            0,
            0,
            0,
            0,
            native::LAND_COMBAT_RETURN.0,
        );
    }
    GATE.wait();
    writer.join().unwrap();
    assert_eq!(drain(&mut vec![Entry::default(); 8]), Err(SHARED_POOL));
    assert_eq!(
        contested(),
        (
            count + 1,
            serde_json::json!({
                "reason": SHARED_POOL,
                "site_index": SiteId::Land as usize,
                "other_site_index": SiteId::Attrition as usize,
                "pool": bank,
                "country": 4997,
            })
        )
    );
    boundary(clock(3, 1));
    assert!(drain(&mut vec![Entry::default(); 8]).is_err());
    boundary(clock(4, 1));
    assert_eq!(drain(&mut vec![Entry::default(); 8]), Ok(0));
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn a_writer_held_after_its_pool_read_overlaps_every_window_another_thread_opens_meanwhile() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(1);
    world.owner(4996);
    ORIGINAL[SiteId::Land as usize].store(land_removal as *const () as usize, Ordering::Release);
    let (bank, design) = (world.army() + carmy::EQUIPMENT_POOL, world.design(0));
    let (count, _) = contested();
    HELD_AFTER_READ.store(SiteId::Attrition as usize + 1, Ordering::SeqCst);
    let writer = std::thread::spawn(move || unsafe { attrition(bank, design, 100000) });
    GATE.wait();
    unsafe {
        land(
            world.army(),
            world.design(0),
            200000,
            0,
            0,
            0,
            0,
            0,
            native::LAND_COMBAT_RETURN.0,
        );
    }
    GATE.wait();
    writer.join().unwrap();
    assert_eq!(world.storage[(0x600 + 8) / 8] as i64, 600000);
    assert_eq!(drain(&mut vec![Entry::default(); 8]), Err(SHARED_POOL));
    assert_eq!(
        contested(),
        (
            count + 1,
            serde_json::json!({
                "reason": SHARED_POOL,
                "site_index": SiteId::Attrition as usize,
                "other_site_index": SiteId::Land as usize,
                "pool": bank,
                "country": 4996,
            })
        )
    );

    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut doomed = World::new(1);
    doomed.owner(4995);
    let ctx = context(&doomed, native::Cause::DivisionDestroyed);
    let (army, bank, design) = (
        doomed.army(),
        doomed.army() + carmy::EQUIPMENT_POOL,
        doomed.design(0),
    );
    HELD_AFTER_READ.store(SiteId::DivisionFree as usize + 1, Ordering::SeqCst);
    let destructor = std::thread::spawn(move || {
        let _scope = scoped(ctx);
        unsafe { division_free(army, 1) }
    });
    GATE.wait();
    unsafe { attrition(bank, design, 100000) };
    GATE.wait();
    assert_eq!(destructor.join().unwrap(), army);
    assert_eq!(drain(&mut vec![Entry::default(); 8]), Err(DESTRUCTOR_OVERLAP));
    assert_eq!(
        contested(),
        (
            count + 2,
            serde_json::json!({
                "reason": DESTRUCTOR_OVERLAP,
                "site_index": SiteId::Attrition as usize,
                "other_site_index": SiteId::DivisionFree as usize,
                "pool": bank,
                "country": 4995,
            })
        )
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn a_store_grown_to_the_hours_need_folds_the_next_such_hour_without_allocating() {
    let _serial = serial();
    install(Store::new());
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let world = World::new(1);
    let bank = world.army() + carmy::EQUIPMENT_POOL;
    let mut out = Vec::new();
    unsafe { attrition(bank, world.design(0), 100000) };
    assert_eq!(drain(&mut out), Ok(1));
    let entries = folded().entries.as_ptr();
    let diagnostics = refusal_diagnostics();
    unsafe { attrition(bank, world.design(0), 100000) };
    let drained = collector_test_support::with_allocations_denied(|| drain(&mut out));
    assert_eq!(drained, Ok(1));
    assert_eq!(out[0].amount_raw, 100000);
    assert_eq!(refusal_diagnostics(), diagnostics);
    assert_eq!(folded().entries.as_ptr(), entries);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn interval_recovery_preserves_skipped_source_refusal_and_admits_other_sources() {
    let _serial = serial();
    for site in [SiteId::Attrition, SiteId::ScriptedEquipmentFraction] {
        setup(8, 2);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(1);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut source = World::new(1);
        source.rebase(base);
        let handle = context_rebased(&source, base, native::Cause::Unknown).handle;
        let reserve = ReserveWorld::for_source(&source, base, 1, 0);
        unsafe {
            hq_returned(
                reserve.manager.as_ptr() as usize,
                source.army() + carmy::EQUIPMENT_POOL,
            );
        }
        folded()
            .credits
            .set_state(handle, source.army(), 1, PoolState::Live)
            .unwrap();
        poison();
        let bank = source.army() + carmy::EQUIPMENT_POOL;
        unsafe {
            match site {
                SiteId::Attrition => attrition(bank, source.design(0), 100000),
                SiteId::ScriptedEquipmentFraction => scripted_equipment_fraction(bank, 50000),
                _ => unreachable!(),
            }
        }
        boundary(clock(3, 1));
        assert!(drain(&mut vec![Entry::default(); 8]).is_err());
        boundary(clock(4, 1));
        assert!(!folded().poisoned);
        assert_eq!(
            folded()
                .credits
                .state(handle, source.army(), 1),
            Err("equipment returned source skipped native mutation")
        );
        let mut other = World::new(1);
        other.rebase(base);
        other.owner(4999);
        other.word(engine_native::cunit::HANDLE, 0xe00200004b01);
        unsafe {
            attrition(
                other.army() + carmy::EQUIPMENT_POOL,
                other.design(0),
                100000,
            )
        };
        assert_signed_row(100000, native::Cause::Attrition, 0xf00000000046);
        unsafe { division_free(source.army(), 1) };
        assert!(folded().poisoned);
        assert!(drain(&mut vec![Entry::default(); 8]).is_err());
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn actual_events_every_country_fail_closed_and_detach() {
    let _serial = serial();
    let mut world = World::new(3);
    setup(50, 3);
    boundary(clock(1, 1));
    assert!(
        drain(&mut vec![Entry::default(); 1])
            .unwrap_err()
            .contains("partial")
    );
    boundary(clock(2, 1));
    let p = world.army() + carmy::EQUIPMENT_POOL;
    let v = world.design(1);
    world.amount(1, 1000000);
    unsafe {
        attrition(p, v, 900000);
    }
    let mut out = vec![Entry::default(); 50];
    assert_eq!(drain(&mut out).unwrap(), 1);
    assert_eq!(out[0].amount_raw, 900000);
    assert_eq!(
        out[0].variant,
        (0xf001u64 << 32) | variant::REFERENCE_TYPE as u64
    );
    unsafe {
        attrition(p, v, 0);
    }
    assert_eq!(drain(&mut out).unwrap(), 0);
    for countries in [15, 50] {
        for country in 0..countries {
            world.owner(country);
            world.amount(1, 900000);
            unsafe {
                attrition(p, v, 100000);
            }
        }
        let len = drain(&mut out).unwrap();
        assert_eq!(len, countries as usize);
        for (country, row) in out[..len].iter().enumerate() {
            assert_eq!(row.country, country as u32);
        }
    }
    assert_eq!(
        serde_json::to_value(status()).unwrap()["hourly_peak_events"].as_u64(),
        Some(50)
    );
    assert_eq!(
        serde_json::to_value(status()).unwrap()["hourly_peak_capacity"].as_u64(),
        Some(50)
    );
    world.amount(0, 450001);
    world.amount(1, 300000);
    world.amount(2, 100000);
    let army = world.army();
    measure_pool(SiteId::Land, army, native::Cause::LandCombat, false, || {
        world.amount(0, 370000);
        world.amount(1, 300000);
        world.amount(2, 0);
    });
    assert_eq!(drain(&mut out).unwrap(), 2);
    assert_eq!((out[0].amount_raw, out[1].amount_raw), (80001, 100000));
    world.amount(1, 900000);
    unsafe {
        attrition(p, v, 100000);
    }
    assert_eq!(drain(&mut Vec::new()), Ok(1));
    folded().entries = Vec::new();
    world.amount(1, 900000);
    unsafe {
        attrition(p, v, 100000);
    }
    assert_eq!(drain(&mut out), Ok(1));
    boundary(clock(1, 3));
    boundary(clock(2, 3));
    world.amount(1, 900000);
    let held = STORE.lock().unwrap();
    unsafe {
        attrition(p, v, 100000);
    }
    drop(held);
    assert_eq!(drain(&mut out), Ok(1));
    assert_eq!(active(), 0);
    boundary(clock(1, 4));
    boundary(clock(2, 4));
    folded().entries = Vec::with_capacity(2);
    world.amount(1, 50);
    unsafe {
        attrition(p, v, 100);
    }
    assert_eq!(drain(&mut out).unwrap(), 1);
    assert_eq!(out[0].amount_raw, 100);
    assert_eq!(
        unsafe { *((world.army() + 0x600 + 16 + 8) as *const i64) },
        -50
    );
    assert!(!folded().poisoned);
    for site in native::SITES.iter().map(|site| site.id) {
        let held = Call::enter(site);
        assert!(active() != 0);
        held.run(|| ());
        drop(held);
        assert_eq!(active(), 0);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    ORIGINAL[SiteId::Land as usize].store(forwarding as *const () as usize, Ordering::Release);
    let shim: ArmyFn = unsafe { std::mem::transmute(destruction_land_entry as *const ()) };
    let slots = std::array::from_fn::<_, 8, _>(|i| usize::MAX - i);
    unsafe {
        shim(
            slots[0], slots[1], slots[2], slots[3], slots[4], slots[5], slots[6], slots[7],
        );
    }
    assert_eq!(*FORWARDED.lock().unwrap(), slots);
    assert_eq!(active(), 0);
}

#[test]
fn enabled_land_shim_passes_caller_pc_and_records_stable_causes() {
    use replay_data::{rows::EquipmentDestructionCause, rows::TableRows};
    use replay_frame::frame::TableView;

    let _serial = serial();
    let caller = destruction_land_test_return as *const () as usize;
    let invoke: ArmyFn = unsafe { std::mem::transmute(destruction_land_test_caller as *const ()) };
    for (return_rva, cause, recorded_cause) in [
        (
            native::LAND_COMBAT_RETURN.0,
            native::Cause::LandCombat,
            EquipmentDestructionCause::LandCombat,
        ),
        (
            native::NAVAL_TRANSPORT_RETURN.0,
            native::Cause::NavalTransport,
            EquipmentDestructionCause::NavalTransport,
        ),
        (
            native::IMMEDIATE_RETURN.0,
            native::Cause::Immediate,
            EquipmentDestructionCause::Immediate,
        ),
        (
            native::LAND_COMBAT_RETURN.0 + 1,
            native::Cause::Unknown,
            EquipmentDestructionCause::Unknown,
        ),
    ] {
        let mut world = World::new(1);
        setup(8, 1);
        let base = caller.checked_sub(return_rva).unwrap();
        BASE.store(base, Ordering::Relaxed);
        world.rebase(base);
        ORIGINAL[SiteId::Land as usize]
            .store(land_removal as *const () as usize, Ordering::Release);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let unknown = UNKNOWN_CALLERS.load(Ordering::Relaxed);
        let slots = [
            world.army(),
            world.design(0),
            100000,
            usize::MAX - 3,
            usize::MAX - 4,
            usize::MAX - 5,
            usize::MAX - 6,
            usize::MAX - 7,
        ];
        assert!(ENABLED.load(Ordering::SeqCst));
        unsafe {
            invoke(
                slots[0], slots[1], slots[2], slots[3], slots[4], slots[5], slots[6], slots[7],
            );
        }
        assert_eq!(*FORWARDED.lock().unwrap(), slots);
        assert_eq!(active(), 0);
        assert!(!folded().poisoned);
        assert_eq!(
            UNKNOWN_CALLERS.load(Ordering::Relaxed),
            unknown + u64::from(cause == native::Cause::Unknown)
        );
        let mut captured = crate::extended::equipment_destruction::Captured::with_capacities(&1);
        captured.capture(
            &engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            0,
        );
        let [
            TableView::Complete(TableRows::EquipmentDestruction(rows)),
            TableView::Complete(TableRows::DivisionEquipmentDeletions(deleted)),
        ] = captured.tables()
        else {
            panic!("enabled shim event must publish a complete destruction table");
        };
        assert_eq!(rows.len(), 1);
        assert!(deleted.is_empty());
        assert_eq!(rows[0].cause, recorded_cause);
        assert_eq!(rows[0].country, 1);
        assert_eq!(rows[0].amount_raw, 100000);
        assert_eq!(
            rows[0].variant,
            (0xf000u64 << 32) | variant::REFERENCE_TYPE as u64
        );
        assert_eq!(
            serde_json::to_value(status()).unwrap()["hourly_peak_events"].as_u64(),
            Some(1)
        );
        assert_eq!(
            serde_json::to_value(status()).unwrap()["hourly_peak_capacity"].as_u64(),
            Some(8)
        );
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    BASE.store(0, Ordering::Relaxed);
}

#[test]
fn real_delete_entry_preserves_four_slots_and_classifies_its_stack_return_pc() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let caller = destruction_delete_test_return as *const () as usize;
    BASE.store(caller - native::READ_CONTEXT_RETURN.0, Ordering::Relaxed);
    let invoke: unsafe extern "system-unwind" fn(usize, usize, usize, usize) =
        unsafe { std::mem::transmute(destruction_delete_test_caller as *const ()) };
    let slots = [usize::MAX, usize::MAX - 1, usize::MAX - 2, usize::MAX - 3];
    unsafe { invoke(slots[0], slots[1], slots[2], slots[3]) };
    assert_eq!(
        *FORWARDED.lock().unwrap(),
        [slots[0], slots[1], slots[2], slots[3], 0, 0, 0, 0]
    );
    assert!(OBSERVED_DELETION.lock().unwrap().unwrap().ignored);
    assert_eq!(drain(&mut vec![Entry::default(); 1]).unwrap(), 0);
    assert!(DELETION.get().is_none());
    assert_eq!(active(), 0);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    unsafe { invoke(slots[0], slots[1], slots[2], slots[3]) };
    assert!(OBSERVED_DELETION.lock().unwrap().is_none());
    assert_eq!(active(), 0);
    BASE.store(0, Ordering::Relaxed);
}

#[test]
fn real_division_free_entry_preserves_flags_result_and_pending_cause_before_free() {
    let _serial = serial();
    for cause in [native::Cause::Unknown, native::Cause::DivisionOverrun] {
        setup(8, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(1);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut world = World::new(1);
        world.rebase(base);
        let handle = context_rebased(&world, base, cause).handle;
        if cause != native::Cause::Unknown {
            record_queue(
                &mut folded(),
                world.army(),
                handle,
                native::QueuedContext::Loss(cause),
            )
            .unwrap();
        }
        let invoke: unsafe extern "system-unwind" fn(usize, usize) -> usize =
            unsafe { std::mem::transmute(destruction_free_test_caller as *const ()) };
        let flags = usize::MAX - 1;
        assert_eq!(unsafe { invoke(world.army(), flags) }, world.army());
        assert_eq!(
            *FORWARDED.lock().unwrap(),
            [world.army(), flags, 0, 0, 0, 0, 0, 0]
        );
        let mut out = vec![Entry::default(); 4];
        assert_eq!(drain(&mut out).unwrap(), 2);
        assert_eq!(out[0].division_handle, handle);
        assert_eq!(out[0].cause, cause);
        assert_eq!(out[0].disposition, native::Disposition::Loss);
        assert_eq!(out[1].amount_raw, 900000);
        assert_eq!(world.storage[carmy::EQUIPMENT_POOL / 8], 0);
        assert!(folded().queued.is_empty());
        assert_eq!(active(), 0);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    BASE.store(0, Ordering::Relaxed);
}

#[test]
fn real_queue_entry_preserves_slots_and_replaces_r8_with_the_stack_return_pc() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    let mut world = World::new(1);
    world.rebase(base);
    let handle = context_rebased(&world, base, native::Cause::Unknown).handle;
    let invoke: unsafe extern "system-unwind" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(destruction_queue_test_caller as *const ()) };
    unsafe {
        invoke(
            state.state.as_ptr() as usize,
            world.army(),
            base + native::HQ_PROVIDER_RETURN.0,
        )
    };
    assert_eq!(
        *FORWARDED.lock().unwrap(),
        [
            state.state.as_ptr() as usize,
            world.army(),
            0,
            0,
            0,
            0,
            0,
            0
        ]
    );
    let pending = folded().queued[&handle];
    assert_eq!(pending.cause, native::Cause::Unknown);
    assert_eq!(pending.disposition, native::Disposition::Loss);
    assert!(!folded().poisoned);
    assert_eq!(active(), 0);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    BASE.store(0, Ordering::Relaxed);
}

#[cfg(test)]
fn context(world: &World, cause: native::Cause) -> Deletion {
    context_rebased(world, 0, cause)
}
#[cfg(test)]
fn context_rebased(world: &World, base: usize, cause: native::Cause) -> Deletion {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    Deletion {
        army: world.army(),
        handle: native::army_identity(&memory, base, world.army()).unwrap(),
        country: native::army_owner(&memory, world.army()).unwrap(),
        cause,
        disposition: native::Disposition::Loss,
        expeditionary_owner: (world.storage[engine_native::cunit::EXPEDITIONARY_OWNER / 8]
            >> ((engine_native::cunit::EXPEDITIONARY_OWNER % 8) * 8))
            as u32,
        returned: false,
        ignored: false,
        merge_source: false,
    }
}

#[test]
fn deletion_paths_snapshot_before_free_and_ignore_actual_returns_and_cleanup() {
    let _serial = serial();
    let mut memory = StateMemory::new(3);
    for cause in [
        native::Cause::DivisionDestroyed,
        native::Cause::DivisionOverrun,
        native::Cause::DivisionDeleted,
        native::Cause::DivisionDisbanded,
        native::Cause::DivisionCapitulation,
        native::Cause::DivisionScripted,
        native::Cause::DivisionTemplateRemoved,
        native::Cause::Unknown,
    ] {
        setup(100, 3);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut world = World::new(3);
        world.owner(4999);
        world.amount(0, 450001);
        world.amount(1, 0);
        world.amount(2, 100000);
        let unknown = UNKNOWN_CALLERS.load(Ordering::Relaxed);
        let ctx = context(&world, cause);
        let _scope = scoped(ctx);
        assert_eq!(unsafe { division_free(world.army(), 1) }, world.army());
        let mut out = vec![Entry::default(); 100];
        assert_eq!(drain(&mut out).unwrap(), 3);
        assert_eq!(out[0].division_handle, ctx.handle);
        assert_eq!(out[0].amount_raw, 0);
        assert_eq!((out[1].amount_raw, out[2].amount_raw), (450001, 100000));
        assert!(
            out[..3]
                .iter()
                .all(|entry| entry.country == 4999 && entry.cause == cause)
        );
        assert_eq!(
            UNKNOWN_CALLERS.load(Ordering::Relaxed),
            unknown + u64::from(cause == native::Cause::Unknown)
        );
        assert!(!folded().poisoned);
    }
    for cause in [
        native::Cause::DivisionDisbanded,
        native::Cause::DivisionCapitulation,
    ] {
        setup(100, 3);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(3);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut world = World::new(3);
        world.rebase(base);
        let reserve = ReserveWorld::for_source(&world, base, 1, 0);
        let _scope = scoped(context_rebased(&world, base, cause));
        unsafe {
            returned(
                reserve.manager.as_ptr() as usize,
                world.army() + carmy::EQUIPMENT_POOL,
            );
            division_free(world.army(), 1);
        }
        let mut out = vec![Entry::default(); 1];
        assert_eq!(drain(&mut out).unwrap(), 1);
        assert_eq!(out[0].disposition, native::Disposition::Returned);
        assert_eq!(out[0].amount_raw, 0);
        assert_eq!(out[0].variant, 0);
    }
    for (caller, reset) in [
        (native::READ_CONTEXT_RETURN.0, false),
        (native::FAILED_CREATE_RETURN.0, false),
        (0, true),
    ] {
        setup(100, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        memory.resetting(reset);
        let mut world = World::new(1);
        world.word(engine_native::cunit::HANDLE, 0);
        world.word(carmy::EQUIPMENT_POOL, 0);
        let ctx = deletion_context(
            &memory,
            0,
            memory.state.as_ptr() as usize,
            world.army(),
            caller,
        )
        .unwrap()
        .unwrap();
        assert!(ctx.ignored);
        let _scope = scoped(ctx);
        unsafe { division_free(world.army(), 1) };
        assert_eq!(drain(&mut vec![Entry::default(); 1]).unwrap(), 0);
        assert!(!folded().poisoned);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn merge_transfers_suppress_source_flush_and_record_signed_uncredited_equipment() {
    let _serial = serial();
    setup(100, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let world = World::new(1);
    let mut ctx = context(&world, native::Cause::Unknown);
    ctx.disposition = native::Disposition::Merged;
    {
        let _scope = scoped(ctx);
        ORIGINAL[SiteId::Land as usize]
            .store(land_removal as *const () as usize, Ordering::Release);
        unsafe {
            land(
                world.army(),
                world.design(0),
                100000,
                0,
                0,
                0,
                0,
                0,
                native::LAND_COMBAT_RETURN.0,
            );
            division_free(world.army(), 1);
        }
    }
    let mut out = vec![Entry::default(); 1];
    assert_eq!(drain(&mut out).unwrap(), 1);
    assert_eq!(out[0].disposition, native::Disposition::Merged);
    assert_eq!(out[0].amount_raw, 0);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn queued_deletion_provenance_is_bounded_exact_and_unknown_on_conflict() {
    let _serial = serial();
    let memory = StateMemory::new(1);
    setup(100, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    for classified in [
        native::QueuedContext::Loss(native::Cause::DivisionOverrun),
        native::QueuedContext::Loss(native::Cause::DivisionDeleted),
        native::QueuedContext::Loss(native::Cause::Unknown),
        native::QueuedContext::Transfer(native::Disposition::HqTransferred),
    ] {
        let world = World::new(1);
        let handle = context(&world, native::Cause::Unknown).handle;
        record_queue(&mut folded(), world.army(), handle, classified).unwrap();
        let ctx = deletion_context(
            &memory,
            0,
            memory.state.as_ptr() as usize,
            world.army(),
            native::QUEUED_DELETE_RETURN.0,
        )
        .unwrap()
        .unwrap();
        assert!(folded().queued.is_empty());
        match classified {
            native::QueuedContext::Loss(cause) => assert_eq!(ctx.cause, cause),
            native::QueuedContext::Transfer(disposition) => {
                assert_eq!(ctx.disposition, disposition);
                assert!(!ctx.ignored);
            }
            _ => unreachable!(),
        }
    }
    let world = World::new(1);
    let handle = context(&world, native::Cause::Unknown).handle;
    {
        let mut store = folded();
        for cause in [
            native::Cause::DivisionDeleted,
            native::Cause::DivisionOverrun,
        ] {
            record_queue(
                &mut store,
                world.army(),
                handle,
                native::QueuedContext::Loss(cause),
            )
            .unwrap();
        }
        assert_eq!(
            store.queued.values().next().unwrap().cause,
            native::Cause::Unknown
        );
        store.queued = FixedIndex::new();
        store.queued_by_army = FixedIndex::new();
        store.queued_count = 0;
        store.queued_limit = 0;
        let refused = collector_test_support::with_allocations_denied(|| {
            record_queue(
                &mut store,
                world.army(),
                handle,
                native::QueuedContext::Loss(native::Cause::DivisionDeleted),
            )
        });
        assert_eq!(refused, Err(QUEUE_STORAGE_FAILED));
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn transferred_provenance_survives_conflicting_requests_and_direct_deletion() {
    let _serial = serial();
    let memory = StateMemory::new(1);
    for requests in [
        [
            native::QueuedContext::Transfer(native::Disposition::HqTransferred),
            native::QueuedContext::Loss(native::Cause::DivisionDeleted),
        ],
        [
            native::QueuedContext::Loss(native::Cause::DivisionOverrun),
            native::QueuedContext::Transfer(native::Disposition::HqTransferred),
        ],
    ] {
        for caller in [
            native::QUEUED_DELETE_RETURN.0,
            native::COMBAT_DELETION_RETURN.0,
        ] {
            setup(100, 1);
            boundary(clock(1, 1));
            boundary(clock(2, 1));
            let world = World::new(1);
            let handle = context(&world, native::Cause::Unknown).handle;
            {
                let mut store = folded();
                for classified in requests {
                    record_queue(&mut store, world.army(), handle, classified).unwrap();
                }
                assert_eq!(store.queued.len(), 1);
                assert_eq!(
                    store.queued.values().next().unwrap().cause,
                    native::Cause::Unknown
                );
                assert_eq!(
                    store.queued.values().next().unwrap().disposition,
                    native::Disposition::HqTransferred
                );
            }
            let ctx = deletion_context(
                &memory,
                0,
                memory.state.as_ptr() as usize,
                world.army(),
                caller,
            )
            .unwrap()
            .unwrap();
            assert!(!ctx.ignored);
            assert_eq!(ctx.disposition, native::Disposition::HqTransferred);
            assert!(folded().queued.is_empty());
            let _scope = scoped(ctx);
            ORIGINAL[SiteId::Land as usize]
                .store(land_removal as *const () as usize, Ordering::Release);
            unsafe {
                land(
                    world.army(),
                    world.design(0),
                    100000,
                    0,
                    0,
                    0,
                    0,
                    0,
                    native::LAND_COMBAT_RETURN.0,
                );
                division_free(world.army(), 1);
            }
            let mut out = vec![Entry::default(); 4];
            assert_eq!(drain(&mut out).unwrap(), 1);
            assert_eq!(out[0].division_handle, handle);
            assert_eq!(out[0].disposition, native::Disposition::HqTransferred);
            assert_eq!(out[0].amount_raw, 0);
        }
    }
    setup(100, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(1);
    let old_handle = context(&world, native::Cause::Unknown).handle;
    record_queue(
        &mut folded(),
        world.army(),
        old_handle,
        native::QueuedContext::Transfer(native::Disposition::HqTransferred),
    )
    .unwrap();
    let new_handle = old_handle + (1 << 32);
    world.word(engine_native::cunit::HANDLE, new_handle);
    record_queue(
        &mut folded(),
        world.army(),
        new_handle,
        native::QueuedContext::Loss(native::Cause::DivisionDeleted),
    )
    .unwrap();
    {
        let store = folded();
        assert_eq!(store.queued.values().next().unwrap().handle, new_handle);
        assert_eq!(
            store.queued.values().next().unwrap().cause,
            native::Cause::DivisionDeleted
        );
        assert_eq!(
            store.queued.values().next().unwrap().disposition,
            native::Disposition::Loss
        );
    }
    let ctx = deletion_context(
        &memory,
        0,
        memory.state.as_ptr() as usize,
        world.army(),
        native::QUEUED_DELETE_RETURN.0,
    )
    .unwrap()
    .unwrap();
    assert!(!ctx.ignored);
    assert_eq!(ctx.cause, native::Cause::DivisionDeleted);
    let _scope = scoped(ctx);
    unsafe { division_free(world.army(), 1) };
    let mut out = vec![Entry::default(); 4];
    assert_eq!(drain(&mut out).unwrap(), 2);
    assert_eq!(out[0].division_handle, new_handle);
    assert_eq!(out[1].amount_raw, 900000);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn pending_transfers_exclude_each_army_writer_and_direct_free_but_not_reused_handles() {
    let _serial = serial();
    for disposal in ["delete", "free"] {
        setup(100, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(1);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut world = World::new(1);
        world.rebase(base);
        let handle = context_rebased(&world, base, native::Cause::Unknown).handle;
        for classified in [
            native::QueuedContext::Transfer(native::Disposition::HqTransferred),
            native::QueuedContext::Loss(native::Cause::DivisionOverrun),
        ] {
            record_queue(&mut folded(), world.army(), handle, classified).unwrap();
        }
        ORIGINAL[SiteId::Land as usize]
            .store(land_removal as *const () as usize, Ordering::Release);
        for writer in ["land", "attrition", "paradrop"] {
            world.restore_pool();
            world.rebase(base);
            unsafe {
                match writer {
                    "land" => land(
                        world.army(),
                        world.design(0),
                        100000,
                        0,
                        0,
                        0,
                        0,
                        0,
                        base + native::LAND_COMBAT_RETURN.0,
                    ),
                    "attrition" => attrition(
                        world.army() + carmy::EQUIPMENT_POOL,
                        world.design(0),
                        100000,
                    ),
                    "paradrop" => paradrop(world.army(), 0, 200000, 0, 0),
                    _ => unreachable!(),
                }
            }
            assert!(world.storage[(0x600 + 8) / 8] < 900001);
            assert_eq!(drain(&mut vec![Entry::default(); 8]).unwrap(), 0);
        }
        if disposal == "delete" {
            ORIGINAL[SiteId::Delete as usize]
                .store(cleanup_delete as *const () as usize, Ordering::Release);
            unsafe {
                delete(
                    state.state.as_ptr() as usize,
                    world.army(),
                    0,
                    0,
                    base + native::COMBAT_DELETION_RETURN.0,
                )
            };
        } else {
            unsafe { division_free(world.army(), 1) };
        }
        let mut out = vec![Entry::default(); 8];
        assert_eq!(drain(&mut out).unwrap(), 1);
        assert_eq!(out[0].division_handle, handle);
        assert_eq!(out[0].disposition, native::Disposition::HqTransferred);
        assert_eq!(out[0].amount_raw, 0);
        assert!(folded().queued.is_empty());
        assert!(DELETION.get().is_none());

        let new_handle = handle + (1 << 32);
        world.word(engine_native::cunit::HANDLE, new_handle);
        record_queue(
            &mut folded(),
            world.army(),
            handle,
            native::QueuedContext::Transfer(native::Disposition::HqTransferred),
        )
        .unwrap();
        for writer in ["land", "attrition", "paradrop"] {
            world.restore_pool();
            world.rebase(base);
            unsafe {
                match writer {
                    "land" => land(
                        world.army(),
                        world.design(0),
                        100000,
                        0,
                        0,
                        0,
                        0,
                        0,
                        base + native::LAND_COMBAT_RETURN.0,
                    ),
                    "attrition" => attrition(
                        world.army() + carmy::EQUIPMENT_POOL,
                        world.design(0),
                        100000,
                    ),
                    "paradrop" => paradrop(world.army(), 0, 200000, 0, 0),
                    _ => unreachable!(),
                }
            }
            let mut out = vec![Entry::default(); 8];
            assert_eq!(drain(&mut out).unwrap(), 1);
            assert!(out[0].amount_raw > 0);
        }
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    BASE.store(0, Ordering::Relaxed);
}

#[test]
fn completed_disposal_retires_owned_metadata_without_reading_a_freed_unit() {
    let _serial = serial();
    setup(100, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    let mut world = World::new(1);
    world.rebase(base);
    let handle = context_rebased(&world, base, native::Cause::Unknown).handle;
    ORIGINAL[SiteId::Delete as usize].store(
        queue_metadata_during_delete as *const () as usize,
        Ordering::Release,
    );
    unsafe { delete(state.state.as_ptr() as usize, world.army(), 0, 0, base + 1) };
    assert!(folded().queued.is_empty());
    for disposal in ["delete", "free"] {
        record_queue(
            &mut folded(),
            world.army(),
            handle,
            native::QueuedContext::Transfer(native::Disposition::HqTransferred),
        )
        .unwrap();
        if disposal == "delete" {
            unsafe {
                delete(
                    state.state.as_ptr() as usize,
                    world.army(),
                    0,
                    0,
                    base + native::READ_CONTEXT_RETURN.0,
                )
            };
        } else {
            state.resetting(true);
            unsafe { division_free(world.army(), 1) };
            state.resetting(false);
        }
        assert!(folded().queued.is_empty());
    }
    let old = 0x1234usize;
    record_queue(
        &mut folded(),
        old,
        handle,
        native::QueuedContext::Transfer(native::Disposition::HqTransferred),
    )
    .unwrap();
    folded().retire(old, Some(handle));
    assert!(folded().queued.is_empty());
    record_queue(
        &mut folded(),
        old,
        handle + (1 << 32),
        native::QueuedContext::Loss(native::Cause::DivisionDeleted),
    )
    .unwrap();
    folded().retire(old, Some(handle));
    assert_eq!(folded().queued.len(), 1);
    folded().retire(old, None);
    assert!(folded().queued.is_empty());
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    BASE.store(0, Ordering::Relaxed);
}

#[test]
fn residual_follows_damage_flush_attrition_and_owner_cleanup_without_double_counting() {
    let _serial = serial();
    for residual in [
        native::Cause::DivisionDestroyed,
        native::Cause::DivisionOverrun,
        native::Cause::DivisionDeleted,
    ] {
        setup(100, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut world = World::new(1);
        world.owner(4999);
        world.dword(engine_native::cunit::EXPEDITIONARY_OWNER, 1500);
        let ctx = context(&world, residual);
        assert_eq!(ctx.country, 1500);
        world.dword(engine_native::cunit::EXPEDITIONARY_OWNER, 0);
        let _scope = scoped(ctx);
        if residual == native::Cause::DivisionDeleted {
            unsafe {
                attrition(
                    world.army() + carmy::EQUIPMENT_POOL,
                    world.design(0),
                    100000,
                )
            };
        } else {
            ORIGINAL[SiteId::Land as usize]
                .store(land_removal as *const () as usize, Ordering::Release);
            unsafe {
                land(
                    world.army(),
                    world.design(0),
                    100000,
                    0,
                    0,
                    0,
                    0,
                    0,
                    native::LAND_COMBAT_RETURN.0,
                )
            };
        }
        unsafe { division_free(world.army(), 1) };
        let mut out = vec![Entry::default(); 4];
        let len = drain(&mut out).unwrap();
        assert_eq!(len, 3);
        assert_eq!(out[0].country, 1500);
        assert_eq!(out[0].amount_raw, 100000);
        assert_eq!(out[1].division_handle, ctx.handle);
        assert_eq!(out[2].country, 1500);
        assert_eq!(out[2].amount_raw, 800000);
        assert_eq!(
            out[..len].iter().map(|entry| entry.amount_raw).sum::<i64>(),
            900000
        );
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn recovered_deletion_scope_retains_owner_and_allows_native_nested_flushes() {
    let _serial = serial();
    for return_equipment in [0, 1] {
        setup(100, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        folded().refuse(
            Some(SiteId::Attrition as usize),
            "synthetic pre-observation overlap",
        );
        boundary(clock(3, 1));
        assert_eq!(
            drain(&mut vec![Entry::default(); 4]),
            Err("synthetic pre-observation overlap")
        );
        boundary(clock(4, 1));
        let mut state = StateMemory::new(1);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut world = World::new(1);
        world.rebase(base);
        world.owner(4999);
        world.dword(engine_native::cunit::EXPEDITIONARY_OWNER, 1500);
        ORIGINAL[SiteId::Land as usize]
            .store(land_removal as *const () as usize, Ordering::Release);
        ORIGINAL[SiteId::Delete as usize]
            .store(cleanup_delete as *const () as usize, Ordering::Release);
        let invoke: unsafe extern "system-unwind" fn(usize, usize, usize, usize) =
            unsafe { std::mem::transmute(destruction_delete_test_caller as *const ()) };
        unsafe {
            invoke(
                state.state.as_ptr() as usize,
                world.army(),
                return_equipment,
                usize::MAX - 3,
            )
        };
        assert_eq!(OBSERVED_DELETION.lock().unwrap().unwrap().country, 1500);
        let mut out = vec![Entry::default(); 4];
        let len = drain(&mut out).unwrap();
        if return_equipment != 0 {
            assert_eq!(len, 1);
            assert_eq!(out[0].disposition, native::Disposition::Returned);
            assert_eq!(out[0].country, 1500);
            assert_eq!(out[0].amount_raw, 0);
        } else {
            assert_eq!(len, 3);
            assert_eq!(out[0].amount_raw, 100000);
            assert_eq!(out[0].country, 1500);
            assert_eq!(out[2].amount_raw, 800000);
            assert_eq!(out[2].country, 1500);
            assert_eq!(out[2].cause, native::Cause::Unknown);
        }
        assert!(DELETION.get().is_none());
        assert_eq!(active(), 0);
        let mut outer = World::new(1);
        outer.rebase(base);
        let mut ctx = context_rebased(&outer, base, native::Cause::Unknown);
        ctx.ignored = true;
        let _scope = scoped(ctx);
        let mut inner = World::new(1);
        inner.rebase(base);
        inner.word(engine_native::cunit::HANDLE, 0xe00200004b01);
        unsafe { division_free(inner.army(), 1) };
        assert_eq!(DELETION.get().unwrap().army, outer.army());
        assert_eq!(drain(&mut out).unwrap(), 2);
        assert_eq!(out[0].division_handle, 0xe00200004b01);
        assert_eq!(out[1].cause, native::Cause::Unknown);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    BASE.store(0, Ordering::Relaxed);
}

#[test]
fn paradrop_records_exact_division_and_flooring_and_preserves_five_abi_slots() {
    let _serial = serial();
    setup(100, 3);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(3);
    world.owner(5000);
    world.amount(0, 450001);
    world.amount(1, 300000);
    world.amount(2, 100000);
    unsafe {
        paradrop(
            world.army(),
            usize::MAX - 1,
            200000,
            usize::MAX - 3,
            usize::MAX - 4,
        )
    };
    assert_eq!(
        *FORWARDED.lock().unwrap(),
        [
            world.army(),
            usize::MAX - 1,
            200000,
            usize::MAX - 3,
            usize::MAX - 4,
            0,
            0,
            0
        ]
    );
    let mut out = vec![Entry::default(); 4];
    assert_eq!(drain(&mut out).unwrap(), 3);
    assert_eq!(
        out[..3]
            .iter()
            .map(|entry| entry.amount_raw)
            .collect::<Vec<_>>(),
        [250001, 200000, 100000]
    );
    assert!(out[..3].iter().all(|entry| entry.country == 5000
        && entry.cause == native::Cause::ParadropDisruption
        && entry.division_handle == 0));
    assert_eq!(active(), 0);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn expeditionary_scope_owns_each_nested_army_writer_but_not_a_reused_handle() {
    let _serial = serial();
    for writer in ["land", "attrition", "paradrop"] {
        setup(8, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut world = World::new(1);
        world.owner(4999);
        world.dword(engine_native::cunit::EXPEDITIONARY_OWNER, 1500);
        let ctx = context(&world, native::Cause::DivisionDestroyed);
        world.dword(engine_native::cunit::EXPEDITIONARY_OWNER, 0);
        let _scope = scoped(ctx);
        ORIGINAL[SiteId::Land as usize]
            .store(land_removal as *const () as usize, Ordering::Release);
        for (handle, expected_country) in [(ctx.handle, 1500), (ctx.handle + (1 << 32), 4999)] {
            world.word(engine_native::cunit::HANDLE, handle);
            world.amount(0, 900000);
            unsafe {
                match writer {
                    "land" => land(
                        world.army(),
                        world.design(0),
                        100000,
                        0,
                        0,
                        0,
                        0,
                        0,
                        native::LAND_COMBAT_RETURN.0,
                    ),
                    "attrition" => attrition(
                        world.army() + carmy::EQUIPMENT_POOL,
                        world.design(0),
                        100000,
                    ),
                    "paradrop" => paradrop(world.army(), 0, 200000, 0, 0),
                    _ => unreachable!(),
                }
            }
            let mut out = vec![Entry::default(); 2];
            assert_eq!(drain(&mut out).unwrap(), 1);
            assert_eq!(
                out[0].country, expected_country,
                "writer={writer} handle={handle:#x}"
            );
        }
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn hq_transfer_provenance_survives_same_session_gap_and_clears_on_load() {
    let _serial = serial();
    let memory = StateMemory::new(1);
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let world = World::new(1);
    let handle = context(&world, native::Cause::Unknown).handle;
    record_queue(
        &mut folded(),
        world.army(),
        handle,
        native::QueuedContext::Transfer(native::Disposition::HqTransferred),
    )
    .unwrap();
    boundary(clock(4, 1));
    assert_eq!(folded().queued.len(), 1);
    assert!(
        drain(&mut vec![Entry::default(); 1])
            .unwrap_err()
            .contains("partial")
    );
    boundary(clock(5, 1));
    let ctx = deletion_context(
        &memory,
        0,
        memory.state.as_ptr() as usize,
        world.army(),
        native::QUEUED_DELETE_RETURN.0,
    )
    .unwrap()
    .unwrap();
    assert_eq!(ctx.disposition, native::Disposition::HqTransferred);
    let _scope = scoped(ctx);
    unsafe { division_free(world.army(), 1) };
    let mut out = vec![Entry::default(); 1];
    assert_eq!(drain(&mut out).unwrap(), 1);
    assert_eq!(out[0].division_handle, handle);
    assert_eq!(out[0].disposition, native::Disposition::HqTransferred);
    assert_eq!(out[0].amount_raw, 0);
    assert_eq!(out[0].variant, 0);
    record_queue(
        &mut folded(),
        world.army(),
        handle,
        native::QueuedContext::Transfer(native::Disposition::HqTransferred),
    )
    .unwrap();
    boundary(clock(5, 2));
    assert!(folded().queued.is_empty());
    assert!(folded().queued_by_army.is_empty());
    assert!(folded().queued.get(&handle).is_none());
    assert!(
        folded()
            .queued_by_army
            .get(&world.army())
            .is_none()
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn an_event_burst_past_the_log_is_kept_whole_and_counted_as_growth() {
    let _serial = serial();
    setup(1, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let world = World::new(1);
    let overflow = OVERFLOW.load(Ordering::Relaxed);
    let site_overflow = SITE_OVERFLOW[SiteId::Attrition as usize].load(Ordering::Relaxed);
    for _ in 0..3 {
        unsafe { attrition(world.army() + carmy::EQUIPMENT_POOL, world.design(0), 1) };
    }
    drop(folded());
    assert!(OVERFLOW.load(Ordering::Relaxed) > overflow);
    assert!(SITE_OVERFLOW[SiteId::Attrition as usize].load(Ordering::Relaxed) > site_overflow);
    assert!(!folded().poisoned);
    let mut out = vec![Entry::default(); 3];
    assert_eq!(drain(&mut out), Ok(3));
    assert!(out[0].amount_raw != 0 && out.iter().all(|row| *row == out[0]));
    let snapshot = serde_json::to_value(status()).unwrap();
    assert_eq!(
        snapshot["hooks"][SiteId::Attrition as usize]["overflow"].as_u64(),
        Some(SITE_OVERFLOW[SiteId::Attrition as usize].load(Ordering::Relaxed))
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn queued_divisions_grow_the_provenance_index_from_empty() {
    let _serial = serial();
    install(Store::new());
    boundary(clock(1, 1));
    drop(folded());
    assert_eq!(QUEUED_CAPACITY.load(Ordering::Acquire), 0);
    let queue_overflow = SITE_OVERFLOW[SiteId::Queue as usize].load(Ordering::Relaxed);
    let mut store = folded();
    for army in 1..=5usize {
        store.record_queue(
            army << 12,
            (army as u64) << 32 | 0x4b01,
            native::Cause::DivisionDeleted,
            native::Disposition::Loss,
        )
        .unwrap();
    }
    assert_eq!(store.queued_count, 5);
    assert!(store.queued_limit >= 5);
    assert!(QUEUED_CAPACITY.load(Ordering::Acquire) >= 5);
    for army in 1..=5usize {
        let queued = store.queued.get(&((army as u64) << 32 | 0x4b01)).unwrap();
        assert_eq!(queued.army, army << 12);
        assert_eq!(store.queued_by_army.get(&(army << 12)), Some(&queued.handle));
    }
    assert!(SITE_OVERFLOW[SiteId::Queue as usize].load(Ordering::Relaxed) > queue_overflow);
    let refused = collector_test_support::with_allocations_denied(|| {
        store.queued_limit = store.queued_count;
        store.record_queue(
            6 << 12,
            6 << 32 | 0x4b01,
            native::Cause::DivisionDeleted,
            native::Disposition::Loss,
        )
    });
    assert_eq!(refused, Err(QUEUE_STORAGE_FAILED));
    assert_eq!(store.queued_count, 5);
    drop(store);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn hundred_division_burst_and_log_growth_preserve_pending_events_and_provenance() {
    let _serial = serial();
    for countries in [150, 500] {
        setup(1, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let pending = Entry {
            country: 4999,
            cause: native::Cause::Attrition,
            division_handle: 0,
            variant: 0xf00100000046,
            amount_raw: 7,
            ..Default::default()
        };
        folded()
            .append(SiteId::Attrition, pending)
            .unwrap();
        let world = World::new(1);
        let ctx = context(&world, native::Cause::DivisionDeleted);
        record_queue(
            &mut folded(),
            world.army(),
            ctx.handle,
            native::QueuedContext::Loss(ctx.cause),
        )
        .unwrap();
        let mut first = vec![Entry::default(); 1];
        assert_eq!(drain(&mut first).unwrap(), 1);
        assert_eq!(first[0], pending);
        assert_eq!(folded().queued.len(), 1);
        for division in 0..100 {
            let mut world = World::new(64);
            world.owner((division * (countries - 1) / 99) as u32);
            world.word(
                engine_native::cunit::HANDLE,
                ((division as u64 + 1) << 32) | 0x4b01,
            );
            let _scope =
                scoped(context(&world, native::Cause::DivisionDestroyed));
            unsafe { division_free(world.army(), 1) };
        }
        let mut out = Vec::new();
        let len = drain(&mut out).unwrap();
        assert_eq!(len, 100 * 65);
        assert_eq!(
            out[..len]
                .iter()
                .filter(|entry| entry.division_handle != 0)
                .count(),
            100
        );
        assert_eq!(out[len - 1].country, (countries - 1) as u32);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn fake_intel_queue_entry_and_direct_free_record_no_witness_or_loss() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let caller = destruction_queue_test_return as *const () as usize;
    BASE.store(
        caller - native::FAKE_INTEL_QUEUE_RETURN.0,
        Ordering::Relaxed,
    );
    let invoke: unsafe extern "system-unwind" fn(usize, usize) =
        unsafe { std::mem::transmute(destruction_queue_test_caller as *const ()) };
    unsafe { invoke(usize::MAX, usize::MAX - 1) };
    assert!(folded().queued.is_empty());
    assert!(!folded().poisoned);
    assert_eq!(drain(&mut vec![Entry::default(); 1]).unwrap(), 0);

    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    let mut world = World::new(1);
    world.rebase(base);
    let mut template = vec![0u64; 0x240 / 8];
    let flag = engine_native::creferenced_division_template::REVEAL_SKIP;
    template[flag / 8] = 1 << ((flag % 8) * 8);
    world.word(carmy::STANDARD_TEMPLATE, template.as_ptr() as u64);
    unsafe { division_free(world.army(), 1) };
    assert_eq!(drain(&mut vec![Entry::default(); 1]).unwrap(), 0);
    assert!(!folded().poisoned);
    assert_eq!(active(), 0);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    BASE.store(0, Ordering::Relaxed);
}

#[test]
fn withheld_site_install_refusal_withholds_both_tables() {
    const CHILD: &str = "BATTLEPLAN_DESTRUCTION_INSTALL_TEST";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "equipment_destruction::tests::withheld_site_install_refusal_withholds_both_tables",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let _serial = serial();
    setup(100, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let memory = Unreadable;
    let mut hooks = unsafe { Hooks::new(&memory, 0) };
    assert!(hooks.original.is_empty());
    assert!(hooks.supplemental.is_empty());
    hooks.set_enabled(true).unwrap();
    assert!(!ENABLED.load(Ordering::SeqCst));
    assert!(!SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst));
    assert!(
        INSTALLED
            .iter()
            .all(|installed| !installed.load(Ordering::SeqCst))
    );
    assert!(
        deletion_unavailable()
            .unwrap()
            .contains("requires original")
    );
    assert_eq!(
        serde_json::to_value(status()).unwrap()["supplemental_enabled"],
        false
    );
    assert!(UNHOOKED.get().unwrap().contains("withheld"));
    assert!(SUPPLEMENTAL_UNHOOKED.get().unwrap().contains("original"));
    let mut captured = crate::extended::equipment_destruction::Captured::with_capacities(&100);
    captured.capture(&memory, 0, 0);
    assert!(
        captured
            .tables()
            .iter()
            .all(|table| matches!(table, replay_frame::frame::TableView::Unavailable { .. }))
    );
}

#[test]
fn worker_drain_grows_empty_and_recycled_frames_for_every_owned_entry() {
    let _serial = serial();
    let mut output = Vec::new();
    for count in [5000, 10003] {
        setup(count, 1);
        boundary(clock(1, 1));
        assert!(drain(&mut output).is_err());
        boundary(clock(2, 1));
        {
            let mut store = folded();
            collector_test_support::with_allocations_denied(|| {
                for index in 0..count {
                    store
                        .append(
                            SiteId::Attrition,
                            Entry {
                                country: (index % 5000) as u32,
                                cause: native::Cause::Attrition,
                                variant: ((index as u64 + 1) << 32) | 0x46,
                                amount_raw: index as i64 + 1,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                }
            });
        }
        std::thread::scope(|threads| {
            threads
                .spawn(|| {
                    assert_eq!(drain_interval(&mut output), Ok(Drain::Rows(count)));
                    boundary(clock(3, 1));
                    assert_eq!(CLOCK.lock().unwrap().unwrap().hour, 3);
                    let lost = LOST.load(Ordering::Relaxed);
                    lost_drain();
                    assert_eq!(LOST.load(Ordering::Relaxed), lost + 1);
                })
                .join()
                .unwrap();
        });
        assert_eq!(output[count - 1].amount_raw, count as i64);
        assert_eq!(output[count - 1].variant, (count as u64) << 32 | 0x46);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn rejected_rows_and_calls_size_the_receiving_frames_to_the_full_burst() {
    let _serial = serial();
    setup(1, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let entry = Entry {
        country: 1,
        cause: native::Cause::Attrition,
        variant: 0xf00300000046,
        amount_raw: 1,
        ..Default::default()
    };
    {
        let mut store = folded();
        collector_test_support::with_allocations_denied(|| {
            for index in 0..10003 {
                assert_eq!(store.append(SiteId::Attrition, entry).is_ok(), index == 0);
            }
        });
        assert_eq!(EVENT_REQUIRED.load(Ordering::Relaxed), 10003);
        store.refuse(Some(SiteId::Attrition as usize), EVENT_STORAGE_FAILED);
    }
    collector_test_support::with_allocations_denied(|| {
        for _ in 0..12007 {
            let call = Call::enter(SiteId::Attrition);
            call.run(|| ());
        }
    });
    assert_eq!(EVENT_REQUIRED.load(Ordering::Relaxed), 12007);
    assert_eq!(demand(), 12007);
    boundary(clock(3, 1));
    assert!(drain(&mut Vec::new()).is_err());
    assert_eq!(EVENT_ATTEMPTS.load(Ordering::Relaxed), 0);
    assert_eq!(CALL_ATTEMPTS.load(Ordering::Relaxed), 0);
    assert_eq!(EVENT_REQUIRED.load(Ordering::Relaxed), 12007);
    boundary(clock(4, 1));
    {
        let mut store = folded();
        for _ in 0..12007 {
            store.append(SiteId::Attrition, entry).unwrap();
        }
    }
    assert_eq!(drain(&mut Vec::new()), Ok(12007));
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn refused_pool_copies_name_their_storage() {
    let _serial = serial();
    setup(1, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    for count in [5000, 10003] {
        let world = World::new(count);
        let live = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let bank = world.army() + carmy::EQUIPMENT_POOL;
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(pool_rows(&live, 0, bank), Err(EVENT_STORAGE_FAILED));
            assert_eq!(
                variant_amount(&live, 0, bank, world.design(0)),
                Err(EVENT_STORAGE_FAILED)
            );
        });
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn a_burst_past_the_starting_store_keeps_every_row_identical_to_a_presized_store() {
    let _serial = serial();
    const DIVISIONS: usize = 300;
    const DESIGNS: usize = 8;
    let hour = |store: Store| {
        install(store);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut worlds: Vec<_> = (0..DIVISIONS)
            .map(|division| {
                let mut world = World::new(DESIGNS);
                world.owner(division as u32 + 1);
                world
            })
            .collect();
        for world in &mut worlds {
            let pool = world.army() + carmy::EQUIPMENT_POOL;
            for design in 0..DESIGNS {
                world.amount(design, 900000);
                unsafe { attrition(pool, world.design(design), 100000) };
            }
        }
        let mut store = folded();
        let mut credits = Vec::new();
        for division in 0..DIVISIONS {
            let handle = ((division as u64 + 1) << 32) | 0x4b01;
            let army = 0x1000 + division * 8;
            let country = division as u32 + 1;
            record_queue(
                &mut store,
                army,
                handle,
                native::QueuedContext::Loss(native::Cause::DivisionDeleted),
            )
            .unwrap();
            let revision = store.credits.revision();
            store.credits.begin(handle, army, country, revision).unwrap();
            for design in 0..DESIGNS {
                let value = Net {
                    country,
                    variant: ((design as u64 + 1) << 32) | 0x46,
                    amount: -(design as i128) - 1,
                    credited: division as i128,
                };
                store.credits.push(handle, value).unwrap();
            }
        }
        for division in 0..DIVISIONS {
            let handle = ((division as u64 + 1) << 32) | 0x4b01;
            let army = 0x1000 + division * 8;
            assert_eq!(store.queued.get(&handle).map(|queued| queued.army), Some(army));
            assert_eq!(store.queued_by_army.get(&army), Some(&handle));
            store
                .credits
                .visit(handle, army, division as u32 + 1, |value| {
                    credits.push((value.country, value.variant, value.amount, value.credited));
                    Ok(())
                })
                .unwrap();
        }
        drop(store);
        let mut out = Vec::new();
        let len = drain(&mut out).unwrap();
        out.truncate(len);
        (out, credits)
    };
    let grown = hour(Store::new());
    let presized = hour(Store {
        entries: Vec::with_capacity(DIVISIONS * DESIGNS),
        queued: FixedIndex::with_capacity(DIVISIONS),
        queued_by_army: FixedIndex::with_capacity(DIVISIONS),
        queued_limit: DIVISIONS,
        credits: Credits::try_new(DIVISIONS, DIVISIONS * DESIGNS).unwrap(),
        ..Store::new()
    });
    assert_eq!(grown.0.len(), DIVISIONS * DESIGNS);
    assert_eq!(grown.1.len(), DIVISIONS * DESIGNS);
    assert!(grown == presized, "rows or credits differ from the presized store");
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn the_drain_store_starts_empty_and_holds_what_the_hours_need_not_the_census() {
    let _serial = serial();
    let (countries, designs) = (40, 16);
    install(Store::new());
    assert_eq!(folded().owned_capacity(), Default::default());
    assert_eq!(demand(), 0);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut worlds: Vec<_> = (0..countries)
        .map(|country| {
            let mut world = World::new(designs);
            world.owner(country as u32);
            world
        })
        .collect();
    let mut out = Vec::new();
    for hour in 3..6 {
        boundary(clock(hour, 1));
        for world in &mut worlds {
            let pool = world.army() + carmy::EQUIPMENT_POOL;
            for design in 0..designs {
                world.amount(design, 900000);
                unsafe { attrition(pool, world.design(design), 100000) };
            }
        }
        assert_eq!(drain(&mut out), Ok(countries * designs));
    }
    let peak = HOURLY_PEAK_EVENTS.load(Ordering::Relaxed);
    assert_eq!(peak, countries * designs);
    assert_eq!(demand(), peak);
    let held = folded().owned_capacity();
    assert!(
        held.payload_vectors as usize <= 2 * peak * std::mem::size_of::<Entry>(),
        "{held:?} for a peak of {peak} rows"
    );
    assert_eq!(held.index_vectors, 0);
    assert_eq!(
        serde_json::to_value(status()).unwrap()["store_bytes"].as_u64(),
        Some(held.payload_vectors)
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
fn queue_churn_bench(countries: usize, designs: usize) {
    install(Store::new());
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let bound = countries;
    let mut handles: Vec<_> = (0..bound)
        .map(|index| ((index as u64 + 1) << 32) | 0x4b01)
        .collect();
    let mut sequence = bound as u64;
    let mut store = folded();
    for (index, &handle) in handles.iter().enumerate() {
        record_queue(
            &mut store,
            0x1000 + index * 8,
            handle,
            native::QueuedContext::Loss(native::Cause::DivisionDeleted),
        )
        .unwrap();
    }
    let bytes = (
        store.queued.allocated_bytes(),
        store.queued_by_army.allocated_bytes(),
    );
    let pairs = bound * 4;
    let mut averages = Vec::new();
    let mut maximum = 0;
    for _ in 0..21 {
        let started = Instant::now();
        for index in 0..pairs {
            let slot = index % bound;
            let army = 0x1000 + slot * 8;
            sequence += 1;
            let handle = (sequence << 32) | 0x4b01;
            let operation = Instant::now();
            assert!(store.take_queued(army, handles[slot]).is_some());
            record_queue(
                &mut store,
                army,
                handle,
                native::QueuedContext::Loss(native::Cause::DivisionDeleted),
            )
            .unwrap();
            maximum = maximum.max(operation.elapsed().as_nanos() as u64);
            handles[slot] = handle;
        }
        averages.push(started.elapsed().as_nanos() as u64 / pairs as u64);
        assert_eq!(store.queued_count, bound);
        assert_eq!(
            (
                store.queued.allocated_bytes(),
                store.queued_by_army.allocated_bytes()
            ),
            bytes
        );
    }
    drop(store);
    averages.sort_unstable();
    println!(
        "queue_churn_bench countries={countries} pool_designs={designs} queue_capacity={bound} full_table_passes_per_repetition=4 retire_record_pairs={pairs} pair_median_ns={} pair_p95_ns={} largest_pair_ns={maximum} repetitions=21 rehashes=0 producer_payload_bytes={} bytes_per_pool_entry={} payload_excludes_allocation_overhead=true individual_timer_overhead_included=true",
        averages[10],
        averages[19],
        producer_payload_bytes(),
        bytes_per_pool_entry(countries, designs)
    );
}

#[cfg(test)]
fn bytes_per_pool_entry(countries: usize, designs: usize) -> usize {
    producer_payload_bytes() / (2 * countries * designs)
}

#[cfg(test)]
fn producer_payload_bytes() -> usize {
    let store = folded();
    store.entries.capacity() * std::mem::size_of::<Entry>()
        + store.queued.allocated_bytes()
        + store.queued_by_army.allocated_bytes()
        + store.credits.allocated_bytes()
}

#[cfg(test)]
unsafe extern "system-unwind" fn fraction_scale(pool: usize, factor: usize) {
    assert!(
        STORE.try_lock().is_ok(),
        "native call must not retain STORE"
    );
    *FORWARDED.lock().unwrap() = [pool, factor, 0, 0, 0, 0, 0, 0];
    let items = unsafe { *((pool + pool::ITEMS) as *const usize) };
    let count = unsafe { *((pool + pool::COUNT) as *const i32) };
    for index in 0..count as usize {
        let raw = (items + index * equipment_entry::STRIDE + equipment_entry::AMOUNT) as *mut i64;
        unsafe {
            *raw = (*raw as i128 * factor as i64 as i128 / 100000) as i64;
        }
    }
}
#[cfg(test)]
unsafe extern "system-unwind" fn exile_scale_and_round(army: usize, enabled: usize) {
    assert!(STORE.try_lock().is_ok());
    *FORWARDED.lock().unwrap() = [army, enabled, 0, 0, 0, 0, 0, 0];
    if enabled == 0 {
        return;
    }
    let bank = army + carmy::EQUIPMENT_POOL;
    let items = unsafe { *((bank + pool::ITEMS) as *const usize) };
    let count = unsafe { *((bank + pool::COUNT) as *const i32) };
    for index in 0..count as usize {
        let raw = (items + index * equipment_entry::STRIDE + equipment_entry::AMOUNT) as *mut i64;
        unsafe {
            *raw = ((*raw / 2 + 50000) / 100000) * 100000;
        }
    }
}
#[cfg(test)]
unsafe extern "system-unwind" fn roster_remove(country: usize, army: usize) -> usize {
    assert!(STORE.try_lock().is_ok());
    *FORWARDED.lock().unwrap() = [country, army, 0, 0, 0, 0, 0, 0];
    if country as u8 != 0 {
        unsafe {
            *(army as *mut u64) = 0;
            *((army + engine_native::cunit::HANDLE) as *mut u64) = 0;
        }
    }
    country
}

#[test]
fn roster_transfers_witness_only_actual_removal_and_keep_owned_pre_call_identity() {
    let _serial = serial();
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    for (site, disposition) in [
        (
            SiteId::VolunteerTransfer,
            native::Disposition::VolunteerTransferred,
        ),
        (SiteId::ExileTransfer, native::Disposition::ExileTransferred),
    ] {
        setup(8, 1);
        BASE.store(base, Ordering::Relaxed);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        ORIGINAL[site as usize].store(roster_remove as *const () as usize, Ordering::Release);
        let mut world = World::new(1);
        world.rebase(base);
        world.owner(4999);
        world.dword(engine_native::cunit::EXPEDITIONARY_OWNER, 1500);
        let callback: unsafe extern "system-unwind" fn(usize, usize) -> usize =
            unsafe { std::mem::transmute(callback(site)) };
        for result in [0x100usize, 0x101] {
            assert_eq!(unsafe { callback(result, world.army()) }, result);
            assert_eq!(
                *FORWARDED.lock().unwrap(),
                [result, world.army(), 0, 0, 0, 0, 0, 0]
            );
            let mut out = vec![Entry::default(); 8];
            let len = drain(&mut out).unwrap();
            assert_eq!(len, usize::from(result as u8 != 0));
            if len != 0 {
                assert_eq!(out[0].disposition, disposition);
                assert_eq!(out[0].division_handle, 0xe00100004b01);
                assert_eq!((out[0].country, out[0].expeditionary_owner), (1500, 1500));
                assert_eq!((out[0].variant, out[0].amount_raw), (0, 0));
            }
        }
    }
    BASE.store(0, Ordering::Relaxed);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn new_live_pool_writers_capture_exact_signed_net_removals() {
    let _serial = serial();
    for (site, cause) in [
        (SiteId::SetExiled, native::Cause::ExileEquipment),
        (
            SiteId::VolunteerReturnLoss,
            native::Cause::VolunteerReturnEquipment,
        ),
        (
            SiteId::CapitulationEscapeLoss,
            native::Cause::CapitulationEscapeEquipment,
        ),
        (
            SiteId::ScriptedEquipmentFraction,
            native::Cause::ScriptedEquipmentFraction,
        ),
    ] {
        setup(16, 2);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut world = World::new(2);
        world.owner(4999);
        world.amount(0, 900001);
        world.amount(1, 300001);
        let (receiver, factor, stub) = if site == SiteId::SetExiled {
            (world.army(), 1, exile_scale_and_round as *const () as usize)
        } else {
            (
                world.army() + carmy::EQUIPMENT_POOL,
                50000,
                fraction_scale as *const () as usize,
            )
        };
        ORIGINAL[site as usize].store(stub, Ordering::Release);
        let callback: unsafe extern "system-unwind" fn(usize, usize) =
            unsafe { std::mem::transmute(callback(site)) };
        unsafe { callback(receiver, factor) };
        let mut out = vec![Entry::default(); 16];
        assert_eq!(drain(&mut out).unwrap(), 2);
        assert!(
            out[..2]
                .iter()
                .all(|entry| entry.cause == cause && entry.country == 4999)
        );
        assert_eq!(
            [out[0].amount_raw, out[1].amount_raw],
            if site == SiteId::SetExiled {
                [400001, 100001]
            } else {
                [450001, 150001]
            }
        );
        if site != SiteId::SetExiled {
            let diagnostics = refusal_diagnostics();
            unsafe { callback(receiver, 200000) };
            assert_eq!(drain(&mut out).unwrap(), 2);
            assert_eq!([out[0].amount_raw, out[1].amount_raw], [-450000, -150000]);
            assert!(out[..2].iter().all(|entry| entry.cause == cause));
            assert!(!folded().poisoned);
            assert_eq!(refusal_diagnostics(), diagnostics);
        }
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn signed_pool_difference_is_shared_by_every_writer_independently_of_cause() {
    let _serial = serial();
    setup(8, 1);
    let mut world = World::new(1);
    let diagnostics = refusal_diagnostics();
    let army = world.army();
    measure_pool(
        SiteId::ScriptedEquipmentFraction,
        army + carmy::EQUIPMENT_POOL,
        native::Cause::Unknown,
        false,
        || world.amount(0, 900002),
    );
    assert_eq!(folded().entries[0].amount_raw, -2);
    measure_pool(
        SiteId::Land,
        army,
        native::Cause::ScriptedEquipmentFraction,
        false,
        || world.amount(0, 900003),
    );
    assert_eq!(folded().entries[1].amount_raw, -1);
    assert!(!folded().poisoned);
    assert_eq!(refusal_diagnostics(), diagnostics);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn scripted_reserve_fraction_captures_authentic_owner_and_all_designs() {
    let _serial = serial();
    setup(16, 2);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut reserve = ReserveWorld::new(2, 1500);
    reserve.world.amount(0, 900001);
    reserve.world.amount(1, 300001);
    ORIGINAL[SiteId::ScriptedReserveEquipmentFraction as usize]
        .store(fraction_scale as *const () as usize, Ordering::Release);
    unsafe { scripted_reserve_equipment_fraction(reserve.pool(), 50000) };
    let mut out = vec![Entry::default(); 16];
    assert_eq!(drain(&mut out).unwrap(), 2);
    assert_eq!([out[0].amount_raw, out[1].amount_raw], [450001, 150001]);
    assert!(
        out[..2].iter().all(|entry| entry.country == 1500
            && entry.cause == native::Cause::ScriptedStockpileFraction)
    );
    let diagnostics = refusal_diagnostics();
    unsafe { scripted_reserve_equipment_fraction(reserve.pool(), 200000) };
    assert_eq!(drain(&mut out).unwrap(), 2);
    assert_eq!([out[0].amount_raw, out[1].amount_raw], [-450000, -150000]);
    assert!(
        out[..2].iter().all(|entry| entry.country == 1500
            && entry.cause == native::Cause::ScriptedStockpileFraction)
    );
    assert!(!folded().poisoned);
    assert_eq!(refusal_diagnostics(), diagnostics);
    reserve._country[engine_native::ccountry::PRODUCTION_STATUS / 8] = 0;
    unsafe { scripted_reserve_equipment_fraction(reserve.pool(), 50000) };
    assert!(
        drain(&mut out)
            .unwrap_err()
            .contains("reserve country identity mismatch")
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn nested_pool_writer_runs_original_and_refuses_observation_without_holding_native_locks() {
    let _serial = serial();
    setup(16, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(1);
    let bank = world.army() + carmy::EQUIPMENT_POOL;
    let before = 900001;
    world.amount(0, before);
    measure_pool(
        SiteId::Land,
        world.army(),
        native::Cause::LandCombat,
        false,
        || {
            assert!(STORE.try_lock().is_ok());
            unsafe { attrition(bank, world.design(0), 100000) };
        },
    );
    assert_eq!(world.storage[(0x600 + 8) / 8] as i64, before - 100000);
    assert!(
        drain(&mut vec![Entry::default(); 16])
            .unwrap_err()
            .contains("observations overlap")
    );
    assert!(!OBSERVING.get());
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn queued_provenance_survives_attach_and_unproven_attach_deletion_is_one_partial_interval() {
    let _serial = serial();
    let memory = StateMemory::new(1);
    for classified in [
        native::QueuedContext::Transfer(native::Disposition::HqTransferred),
        native::QueuedContext::Loss(native::Cause::DivisionOverrun),
        native::QueuedContext::Loss(native::Cause::DivisionDeleted),
    ] {
        setup(8, 1);
        let world = World::new(1);
        let handle = context(&world, native::Cause::Unknown).handle;
        record_queue(&mut folded(), world.army(), handle, classified).unwrap();
        boundary(clock(1, 1));
        assert_eq!(folded().queued_count, 1);
        let deletion = deletion_context(
            &memory,
            0,
            memory.state.as_ptr() as usize,
            world.army(),
            native::QUEUED_DELETE_RETURN.0,
        )
        .unwrap()
        .unwrap();
        assert!(!deletion.ignored);
        assert_eq!(
            deletion.disposition,
            match classified {
                native::QueuedContext::Transfer(disposition) => disposition,
                _ => native::Disposition::Loss,
            }
        );
        assert_eq!(folded().queued_count, 0);
    }
    for timing in ["before_first_boundary", "after_first_boundary"] {
        setup(8, 1);
        let world = World::new(1);
        if timing == "after_first_boundary" {
            boundary(clock(1, 1));
            assert!(
                drain(&mut vec![Entry::default(); 8])
                    .unwrap_err()
                    .contains("first/branch interval")
            );
        }
        let deletion = deletion_context(
            &memory,
            0,
            memory.state.as_ptr() as usize,
            world.army(),
            native::QUEUED_DELETE_RETURN.0,
        )
        .unwrap()
        .unwrap();
        assert!(deletion.ignored);
        assert!(folded().entries.is_empty());
        if timing == "before_first_boundary" {
            boundary(clock(1, 1));
            assert!(
                drain(&mut vec![Entry::default(); 8])
                    .unwrap_err()
                    .contains("first/branch interval")
            );
            let later = deletion_context(
                &memory,
                0,
                memory.state.as_ptr() as usize,
                world.army(),
                native::QUEUED_DELETE_RETURN.0,
            )
            .unwrap()
            .unwrap();
            assert!(later.ignored);
            assert!(folded().attach_interval);
        }
        boundary(clock(2, 1));
        assert!(
            drain(&mut vec![Entry::default(); 8])
                .unwrap_err()
                .contains("queued division provenance")
        );
        assert!(!folded().poisoned);
        boundary(clock(3, 1));
        assert_eq!(drain(&mut vec![Entry::default(); 8]).unwrap(), 0);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn poison_skipped_queue_opens_unproven_window_on_same_session_transition() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    assert_eq!(drain(&mut vec![Entry::default(); 8]).unwrap(), 0);
    assert!(!folded().attach_interval);
    let memory = StateMemory::new(1);
    let world = World::new(1);
    poison();
    unsafe { queue(0, world.army(), native::HQ_PROVIDER_RETURN.0) };
    assert!(folded().queued.is_empty());
    boundary(clock(4, 1));
    assert!(!folded().poisoned);
    assert!(folded().attach_interval);
    assert!(
        drain(&mut vec![Entry::default(); 8])
            .unwrap_err()
            .contains("partial")
    );
    boundary(clock(5, 1));
    let deletion = deletion_context(
        &memory,
        0,
        memory.state.as_ptr() as usize,
        world.army(),
        native::QUEUED_DELETE_RETURN.0,
    )
    .unwrap()
    .unwrap();
    assert!(deletion.ignored);
    assert!(folded().entries.is_empty());
    assert!(
        drain(&mut vec![Entry::default(); 8])
            .unwrap_err()
            .contains("queued division provenance")
    );
    boundary(clock(6, 1));
    assert_eq!(drain(&mut vec![Entry::default(); 8]).unwrap(), 0);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn initial_partial_drain_clears_only_its_unproven_deletion() {
    let _serial = serial();
    let memory = StateMemory::new(1);
    for before_boundary in [false, true] {
        setup(8, 1);
        let world = World::new(1);
        if !before_boundary {
            boundary(clock(1, 1));
        }
        let deletion = deletion_context(
            &memory,
            0,
            memory.state.as_ptr() as usize,
            world.army(),
            native::QUEUED_DELETE_RETURN.0,
        )
        .unwrap()
        .unwrap();
        assert!(deletion.ignored);
        assert!(folded().unproven_queue);
        if before_boundary {
            boundary(clock(1, 1));
        }
        assert!(
            drain(&mut vec![Entry::default(); 8])
                .unwrap_err()
                .contains("first/branch interval")
        );
        assert!(!folded().unproven_queue);
        assert!(folded().attach_interval);
        boundary(clock(2, 1));
        assert_eq!(drain(&mut vec![Entry::default(); 8]).unwrap(), 0);
        assert!(!folded().attach_interval);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn a_writer_across_the_hour_boundary_is_kept_in_the_interval_it_completes() {
    let _serial = serial();
    for load in [false, true] {
        setup(8, 1);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        assert_eq!(drain(&mut vec![Entry::default(); 8]).unwrap(), 0);
        let mut before = World::new(1);
        before.owner(4998);
        unsafe {
            attrition(
                before.army() + carmy::EQUIPMENT_POOL,
                before.design(0),
                100000,
            )
        };
        record_queue(
            &mut folded(),
            0x1000,
            0x123400004b01,
            native::QueuedContext::Transfer(native::Disposition::HqTransferred),
        )
        .unwrap();
        let mut across = World::new(1);
        across.owner(4999);
        ORIGINAL[SiteId::Attrition as usize]
            .store(gated_subtract as *const () as usize, Ordering::Release);
        let (bank, design) = (across.army() + carmy::EQUIPMENT_POOL, across.design(0));
        let writer = std::thread::spawn(move || unsafe { attrition(bank, design, 200000) });
        GATE.wait();
        assert_ne!(active(), 0);
        assert!(STORE.try_lock().is_ok());
        let session = if load { 2 } else { 1 };
        boundary(clock(3, session));
        let mut first = vec![Entry::default(); 8];
        let first_drain = drain(&mut first);
        let provenance = folded().queued_count;
        GATE.wait();
        writer.join().unwrap();
        boundary(clock(4, session));
        let mut second = vec![Entry::default(); 8];
        let second_drain = drain(&mut second);
        if load {
            assert!(first_drain.unwrap_err().contains("first/branch interval"));
            assert_eq!(provenance, 0);
        } else {
            assert_eq!(first_drain, Ok(1));
            assert_eq!((first[0].country, first[0].amount_raw), (4998, 100000));
            assert_eq!(provenance, 1);
        }
        assert_eq!(second_drain, Ok(1));
        assert_eq!(
            (second[0].country, second[0].cause, second[0].amount_raw),
            (4999, native::Cause::Attrition, 200000)
        );
        assert_eq!(active(), 0);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn drain_interval_separates_startup_and_branch_partials_from_refusals() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    assert_eq!(
        drain_interval(&mut vec![Entry::default(); 8]),
        Ok(Drain::Partial(
            replay_data::rows::EQUIPMENT_DESTRUCTION_PARTIAL_REASON
        ))
    );
    boundary(clock(2, 1));
    assert_eq!(
        drain_interval(&mut vec![Entry::default(); 8]),
        Ok(Drain::Rows(0))
    );
    boundary(clock(2, 2));
    assert_eq!(
        drain_interval(&mut vec![Entry::default(); 8]),
        Ok(Drain::Partial(
            replay_data::rows::EQUIPMENT_DESTRUCTION_PARTIAL_REASON
        ))
    );
    boundary(clock(3, 2));
    assert_eq!(
        drain_interval(&mut vec![Entry::default(); 8]),
        Ok(Drain::Rows(0))
    );
    folded().refuse(Some(0), "synthetic destruction refusal");
    boundary(clock(4, 2));
    assert_eq!(
        drain_interval(&mut vec![Entry::default(); 8]),
        Err("synthetic destruction refusal")
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn a_writer_active_across_a_boundary_refuses_nothing() {
    let _serial = serial();
    setup(8, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let writer = Call::enter(SiteId::SetExiled);
    boundary(clock(3, 1));
    writer.run(|| ());
    drop(writer);
    assert!(!folded().poisoned);
    assert_eq!(drain(&mut vec![Entry::default(); 8]), Ok(0));
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn provenance_fixed_indexes_survive_full_churn_and_retirement_while_poisoned() {
    let _serial = serial();
    const BOUND: usize = 64;
    setup(BOUND, 1);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut world = World::new(1);
    let mut store = folded();
    let initial_capacity = (store.queued.capacity(), store.queued_by_army.capacity());
    let mut pending =
        std::array::from_fn::<_, BOUND, _>(|index| ((index as u64 + 1) << 32) | 0x4b01);
    for (index, handle) in pending.iter().copied().enumerate() {
        record_queue(
            &mut store,
            world.army() + index * 8,
            handle,
            native::QueuedContext::Loss(native::Cause::DivisionDeleted),
        )
        .unwrap();
    }
    for sequence in BOUND as u64 + 1..20000u64 {
        let index = sequence as usize % BOUND;
        let army = world.army() + index * 8;
        assert!(store.take_queued(army, pending[index]).is_some());
        pending[index] = (sequence << 32) | 0x4b01;
        record_queue(
            &mut store,
            army,
            pending[index],
            native::QueuedContext::Loss(native::Cause::DivisionDeleted),
        )
        .unwrap();
        assert_eq!(store.queued_count, BOUND);
        assert_eq!(store.queued.capacity(), initial_capacity.0);
        assert_eq!(store.queued_by_army.capacity(), initial_capacity.1);
    }
    for (index, handle) in pending.iter().copied().enumerate() {
        assert!(store.take_queued(world.army() + index * 8, handle).is_some());
    }
    assert_eq!(store.queued_count, 0);
    let handle = context(&world, native::Cause::Unknown).handle;
    record_queue(
        &mut store,
        world.army(),
        handle,
        native::QueuedContext::Loss(native::Cause::DivisionDeleted),
    )
    .unwrap();
    drop(store);
    poison();
    unsafe { division_free(world.army(), 1) };
    assert!(folded().queued.is_empty());
    assert_eq!(folded().queued_count, 0);
    world.restore_pool();
    record_queue(
        &mut folded(),
        world.army(),
        handle,
        native::QueuedContext::Loss(native::Cause::DivisionDeleted),
    )
    .unwrap();
    unsafe { delete(0, world.army(), 0, 0, 0) };
    assert_eq!(folded().queued_count, 0);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
unsafe extern "system-unwind" fn bench_roster_remove(_country: usize, _army: usize) -> usize {
    1
}

#[test]
fn owned_event_and_provenance_capacity_releases_only_when_quiet() {
    let _guard = serial();
    setup(8, 3);
    static TRACKER: collector_primitives::raw_memory::Tracker =
        collector_primitives::raw_memory::Tracker::new();
    let expected = {
        let mut store = folded();
        let capacity = store.owned_capacity();
        store._charge = collector_primitives::raw_memory::Charge::empty_on(&TRACKER);
        store._charge.set(capacity);
        capacity
    };
    assert!(expected.payload_vectors > 0 && expected.index_vectors > 0);
    assert!(release().is_err());
    assert_eq!(TRACKER.snapshot().current, expected);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    ACTIVE[0].store(1, Ordering::SeqCst);
    assert!(release().is_err());
    assert_eq!(TRACKER.snapshot().current, expected);
    ACTIVE[0].store(0, Ordering::SeqCst);
    release().unwrap();
    assert_eq!(TRACKER.snapshot().current, Default::default());
    assert_eq!(CAPACITY.load(Ordering::Acquire), 0);
    setup(2, 1);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    drop(Hooks {
        original: vec![],
        supplemental: vec![],
        publication_attempted: false,
    });
    assert_eq!(CAPACITY.load(Ordering::Acquire), 0);
}

#[test]
fn disabled_queue_unwind_still_blocks_detach() {
    let _serial = serial();
    unsafe extern "system-unwind" fn incomplete_queue(_: usize, _: usize) {
        panic!("synthetic unobserved queue unwind");
    }
    setup(4, 1);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    ORIGINAL[SiteId::Queue as usize]
        .store(incomplete_queue as *const () as usize, Ordering::SeqCst);
    assert!(std::panic::catch_unwind(|| unsafe { queue(0, 0, 0) }).is_err());
    assert_eq!(active(), 1);
    assert!(active() != 0);
    assert!(release().is_err());
    boundary(clock(1, 1));
    boundary(clock(2, 2));
    assert_eq!(active(), 1);
    assert!(release().is_err());
}

#[test]
fn observed_unwind_closes_its_window_but_retains_callback_for_detach() {
    let _serial = serial();
    setup(4, 1);
    let world = World::new(1);
    let refusals = SITE_REFUSALS[SiteId::Land as usize].load(Ordering::Relaxed);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _call = Call::enter(SiteId::Land);
        measure_pool(
            SiteId::Land,
            world.army(),
            native::Cause::LandCombat,
            false,
            || panic!("synthetic native unwind"),
        );
    }));
    assert!(result.is_err());
    assert!(!OBSERVING.get());
    assert_eq!(OPEN.load(Ordering::SeqCst), 0);
    assert_eq!(ACTIVE[SiteId::Land as usize].load(Ordering::SeqCst), 1);
    assert!(active() != 0);
    assert!(folded().poisoned);
    assert!(folded().open.is_empty());
    assert_eq!(
        SITE_REFUSALS[SiteId::Land as usize].load(Ordering::Relaxed),
        refusals + 1
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
    assert!(release().is_err());
    boundary(clock(10, 2));
    assert_eq!(active(), 1);
    setup(4, 1);
    let mut ctx = context(&world, native::Cause::Unknown);
    ctx.disposition = native::Disposition::Merged;
    let _scope = scoped(ctx);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        measure_pool(
            SiteId::Land,
            world.army(),
            native::Cause::LandCombat,
            false,
            || panic!("excluded transfer unwind"),
        );
    }));
    assert!(result.is_err());
    assert!(!folded().poisoned);
}

#[test]
fn returned_callback_unwind_refuses_accepted_credit_but_preserves_ignored_exclusion() {
    let _serial = serial();
    unsafe extern "system-unwind" fn incomplete_credit(_: usize, _: usize) {
        panic!("synthetic credit unwind");
    }
    let original = ORIGINAL[SiteId::Return as usize].load(Ordering::SeqCst);
    for ignored in [false, true] {
        setup(4, 1);
        ORIGINAL[SiteId::Return as usize]
            .store(incomplete_credit as *const () as usize, Ordering::SeqCst);
        let mut state = StateMemory::new(1);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut world = World::new(1);
        world.rebase(base);
        let reserve = ReserveWorld::for_source(&world, base, 1, 0);
        let mut ctx = context_rebased(&world, base, native::Cause::Unknown);
        ctx.ignored = ignored;
        let _scope = scoped(ctx);
        let result = std::panic::catch_unwind(|| unsafe {
            returned(
                reserve.manager.as_ptr() as usize,
                world.army() + carmy::EQUIPMENT_POOL,
            )
        });
        assert!(result.is_err());
        assert_eq!(folded().poisoned, !ignored);
        assert_eq!(ACTIVE[SiteId::Return as usize].load(Ordering::SeqCst), 1);
        assert!(!OBSERVING.get());
        assert!(!folded().scopes[&DELETION.get().unwrap().id].returned);
        if ignored {
            assert!(!folded().credits.contains(ctx.handle));
        } else {
            assert!(
                folded()
                    .credits
                    .state(ctx.handle, ctx.army, ctx.country)
                    .unwrap_err()
                    .contains("lifetime incomplete")
            );
            boundary(clock(20, 1));
            assert!(folded().poisoned);
            boundary(clock(21, 1));
            unsafe {
                division_free(world.army(), 1);
            }
            assert!(!folded().credits.contains(ctx.handle));
        }
    }
    ORIGINAL[SiteId::Return as usize].store(original, Ordering::SeqCst);
}

#[test]
fn returned_and_hq_credit_deficits_use_source_time_and_publish_only_at_deletion() {
    let _serial = serial();
    for site in [SiteId::Return, SiteId::HqReturn] {
        for (source, initial, deficit) in [
            (100001, 0, 0),
            (100001, 0x53e2d620fb40, 100001),
            (-100001, -0x53e2d620fb40, -100001),
        ] {
            setup(8, 2);
            boundary(clock(1, 1));
            boundary(clock(2, 1));
            let mut state = StateMemory::new(1);
            let base = state.live_base();
            BASE.store(base, Ordering::Relaxed);
            let mut world = World::new(1);
            world.rebase(base);
            world.owner(4999);
            world.amount(0, source);
            let reserve = ReserveWorld::for_source(&world, base, 4999, initial);
            ORIGINAL[site as usize].store(transfer as *const () as usize, Ordering::SeqCst);
            let ctx = context_rebased(&world, base, native::Cause::DivisionDisbanded);
            let scope = (site == SiteId::Return).then(|| scoped(ctx));
            unsafe {
                returned_at(
                    site,
                    reserve.manager.as_ptr() as usize,
                    world.army() + carmy::EQUIPMENT_POOL,
                );
            }
            assert!(!folded().poisoned);
            assert!(
                folded().entries.is_empty(),
                "credit does not publish a deletion"
            );
            let mut proof = Vec::new();
            folded()
                .credits
                .visit(ctx.handle, world.army(), ctx.country, |value| {
                    proof.push((value.amount, value.credited));
                    Ok(())
                })
                .unwrap();
            assert_eq!(
                proof,
                vec![(i128::from(deficit), i128::from(source - deficit))]
            );
            drop(scope);
            if site == SiteId::HqReturn {
                folded().record_queue(
                    world.army(),
                    ctx.handle,
                    native::Cause::Unknown,
                    native::Disposition::HqTransferred,
                )
                .unwrap();
                boundary(clock(4, 1));
                boundary(clock(5, 1));
            }
            world.amount(0, 0);
            let mut deletion = ctx;
            if site == SiteId::Return {
                deletion.returned = true;
            } else {
                deletion.disposition = native::Disposition::HqTransferred;
            }
            let _scope = scoped(deletion);
            unsafe {
                division_free(world.army(), 1);
            }
            let mut out = vec![Entry::default(); 8];
            let len = drain(&mut out).unwrap();
            assert_eq!(len, 1 + usize::from(deficit != 0));
            assert_eq!(out[0].division_handle, ctx.handle);
            assert_eq!(
                out[0].disposition,
                if site == SiteId::Return {
                    native::Disposition::Returned
                } else {
                    native::Disposition::HqTransferred
                }
            );
            if deficit != 0 {
                assert_eq!((out[1].country, out[1].amount_raw), (4999, deficit));
            }
            assert!(!folded().credits.contains(ctx.handle));
        }
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn return_credits_require_actual_witness_and_do_not_cross_full_handle_or_load() {
    let _serial = serial();
    setup(8, 2);
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    let mut world = World::new(1);
    world.rebase(base);
    let reserve = ReserveWorld::for_source(&world, base, 1, 0);
    ORIGINAL[SiteId::HqReturn as usize].store(transfer as *const () as usize, Ordering::SeqCst);
    unsafe {
        hq_returned(
            reserve.manager.as_ptr() as usize,
            world.army() + carmy::EQUIPMENT_POOL,
        );
    }
    let old = context_rebased(&world, base, native::Cause::Unknown).handle;
    assert!(folded().credits.contains(old));
    world.word(engine_native::cunit::HANDLE, old + (1 << 32));
    let mut ctx = context_rebased(&world, base, native::Cause::Unknown);
    ctx.disposition = native::Disposition::HqTransferred;
    assert!(
        folded()
            .observe_deleted(ctx, Err("no residual read"), false)
            .unwrap_err()
            .contains("no actual credit witness")
    );
    boundary(clock(1, 1));
    boundary(clock(1, 2));
    assert!(!folded().credits.contains(old));
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn native_country_return_proofs_remain_deferred_and_recycle_without_growth() {
    let _serial = serial();
    let countries = 40;
    setup(countries * 3, 2);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    ORIGINAL[SiteId::HqReturn as usize].store(transfer as *const () as usize, Ordering::SeqCst);
    let mut worlds: Vec<_> = (0..countries).map(|_| World::new(1)).collect();
    let bytes = producer_payload_bytes();
    for (index, world) in worlds.iter_mut().enumerate() {
        world.rebase(base);
        world.owner(index as u32 + 1);
        world.amount(0, 100001);
        world.word(
            engine_native::cunit::HANDLE,
            ((index as u64 + 1) << 32) | 0x4b01,
        );
        let reserve = ReserveWorld::for_source(world, base, index as u32 + 1, 0x53e2d620fb40);
        unsafe {
            hq_returned(
                reserve.manager.as_ptr() as usize,
                world.army() + carmy::EQUIPMENT_POOL,
            );
        }
    }
    assert!(folded().entries.is_empty());
    assert_eq!(producer_payload_bytes(), bytes);
    boundary(clock(3, 1));
    for world in &mut worlds {
        let mut ctx = context_rebased(world, base, native::Cause::Unknown);
        ctx.disposition = native::Disposition::HqTransferred;
        world.amount(0, 0);
        let _scope = scoped(ctx);
        unsafe {
            division_free(world.army(), 1);
        }
    }
    let mut out = vec![Entry::default(); countries * 3];
    let len = drain(&mut out).unwrap();
    assert_eq!(len, countries * 2);
    for (index, pair) in out[..len].chunks_exact(2).enumerate() {
        assert_eq!(
            (pair[0].country, pair[1].country),
            (index as u32 + 1, index as u32 + 1)
        );
        assert_eq!(pair[1].amount_raw, 100001);
    }
    assert_eq!(producer_payload_bytes(), bytes);
    for (index, world) in worlds.iter_mut().enumerate() {
        world.restore_pool();
        world.rebase(base);
        world.amount(0, 100001);
        world.word(
            engine_native::cunit::HANDLE,
            ((index as u64 + countries as u64 + 1) << 32) | 0x4b01,
        );
        let reserve = ReserveWorld::for_source(world, base, index as u32 + 1, 0x53e2d620fb40);
        unsafe {
            hq_returned(
                reserve.manager.as_ptr() as usize,
                world.army() + carmy::EQUIPMENT_POOL,
            );
        }
    }
    assert!(!folded().poisoned);
    assert_eq!(producer_payload_bytes(), bytes);
    let revision = folded().credits.revision();
    poison();
    for (index, world) in worlds.iter().enumerate() {
        let reserve = ReserveWorld::for_source(world, base, index as u32 + 1, 0x53e2d620fb40);
        unsafe {
            hq_returned(
                reserve.manager.as_ptr() as usize,
                world.army() + carmy::EQUIPMENT_POOL,
            );
        }
        let handle = context_rebased(world, base, native::Cause::Unknown).handle;
        assert!(
            folded()
                .credits
                .state(handle, world.army(), index as u32 + 1)
                .is_err()
        );
    }
    assert_eq!(folded().credits.revision(), revision);
    assert_eq!(producer_payload_bytes(), bytes);
    boundary(clock(3, 2));
    assert!(folded().credits.is_empty());
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
struct RequestsWorld {
    root: Vec<u64>,
    reinforcement: Vec<World>,
    upgrades: Vec<World>,
    _reinforcement_pointers: Vec<usize>,
    _upgrade_pointers: Vec<usize>,
}
#[cfg(test)]
impl RequestsWorld {
    #[cfg(test)]
    fn new(army: &mut World, base: usize, reinforcement: &[i64], upgrades: &[i64]) -> Self {
        let held = |amounts: &[i64]| -> Vec<World> {
            amounts
                .iter()
                .map(|&amount| {
                    let mut world = World::new(1);
                    world.rebase(base);
                    world.word(0x600, army.storage[0x600 / 8]);
                    world.amount(0, amount);
                    world
                })
                .collect()
        };
        let reinforcement = held(reinforcement);
        let upgrades = held(upgrades);
        let reinforcement_pointers = reinforcement
            .iter()
            .map(|world| world.army() + carmy::EQUIPMENT_POOL - 0x38)
            .collect::<Vec<_>>();
        let upgrade_pointers = upgrades
            .iter()
            .map(|world| world.army() + carmy::EQUIPMENT_POOL - 0x20)
            .collect::<Vec<_>>();
        let mut root = vec![0; 0xf0 / 8];
        root[0] = (base + 0x2a4bab0) as u64;
        root[1] = army.army() as u64;
        for (offset, vtable, data, pointers) in [
            (0x10, 0x2a4b7f0, 0x28, &reinforcement_pointers),
            (0x70, 0x2a4b9a0, 0x48, &upgrade_pointers),
        ] {
            root[offset / 8] = (base + vtable) as u64;
            root[(offset + 0x20) / 8] = army.army() as u64;
            root[(offset + data) / 8] = pointers.as_ptr() as u64;
            root[(offset + data + 8) / 8] = pointers.len() as u64 | ((pointers.len() as u64) << 32);
        }
        army.word(carmy::REQUESTS, root.as_ptr() as u64);
        Self {
            root,
            reinforcement,
            upgrades,
            _reinforcement_pointers: reinforcement_pointers,
            _upgrade_pointers: upgrade_pointers,
        }
    }
}
#[cfg(test)]
unsafe fn cancel_native_requests(army: usize, manager: usize, include_upgrades: bool) {
    let requests = unsafe { *((army + carmy::REQUESTS) as *const usize) };
    if requests == 0 {
        return;
    }
    for (offset, data, pool_offset) in [(0x10, 0x28, 0x38), (0x70, 0x48, 0x20)] {
        if offset == 0x70 && !include_upgrades {
            continue;
        }
        let header = requests + offset + data;
        let pointers = unsafe { *(header as *const usize) };
        let count = unsafe { *((header + 12) as *const i32) };
        for index in 0..count as usize {
            unsafe {
                transfer(
                    manager,
                    *((pointers + index * 8) as *const usize) + pool_offset,
                );
            }
        }
        unsafe {
            *((header + 12) as *mut i32) = 0;
        }
    }
}
#[cfg(test)]
unsafe extern "system-unwind" fn free_with_canceled_requests(army: usize, flags: usize) -> usize {
    let plan = MERGE_PLAN.lock().unwrap().unwrap();
    unsafe {
        cancel_native_requests(army, plan.source_reserve_manager, true);
        erase(army, flags)
    }
}

#[derive(Clone, Copy)]
#[cfg(test)]
struct MergePlan {
    source: usize,
    reserve_manager: usize,
    source_reserve_manager: usize,
    retained: [i64; 2],
    credit: usize,
}
#[cfg(test)]
static MERGE_PLAN: Mutex<Option<MergePlan>> = Mutex::new(None);
#[cfg(test)]
unsafe extern "system-unwind" fn merge_native_fixture(argument: usize) {
    assert!(STORE.try_lock().is_ok());
    assert!(WHOLE_MERGE.get() && OBSERVING.get());
    let plan = MERGE_PLAN.lock().unwrap().unwrap();
    let survivor = unsafe { *(argument as *const usize) };
    let items = unsafe { *((survivor + carmy::EQUIPMENT_POOL + pool::ITEMS) as *const usize) };
    for (index, value) in plan.retained.into_iter().enumerate() {
        unsafe {
            *((items + index * equipment_entry::STRIDE + equipment_entry::AMOUNT) as *mut i64) =
                value;
        }
    }
    if plan.credit != 0 {
        unsafe {
            reserve_credit(plan.reserve_manager, *(items as *const usize), plan.credit);
        }
    }
    unsafe {
        cancel_native_requests(survivor, plan.reserve_manager, false);
    }
    let base = BASE.load(Ordering::Relaxed);
    let state = unsafe { *((base + engine_native::cgame_state::SINGLETON_RVA.0) as *const usize) };
    unsafe {
        delete(
            state,
            plan.source,
            0,
            0,
            base + native::MERGED_DELETION_RETURN.0,
        );
    }
}
#[cfg(test)]
unsafe extern "system-unwind" fn merged_delete_fixture(_: usize, army: usize, _: usize, _: usize) {
    let base = BASE.load(Ordering::Relaxed);
    let source_pool = army + carmy::EQUIPMENT_POOL;
    let source_items = unsafe { *((source_pool + pool::ITEMS) as *const usize) };
    let variant = unsafe { *(source_items as *const usize) };
    unsafe {
        land(
            army,
            variant,
            100000,
            0,
            0,
            0,
            0,
            0,
            base + native::LAND_COMBAT_RETURN.0,
        );
        attrition(source_pool, variant, 100000);
        division_free(army, 1);
    }
}

#[test]
fn whole_native_merge_captures_retained_saturation_signed_discard_and_rounding_once() {
    let _serial = serial();
    for (survivor_amount, source_amount, retained, credit, expected) in [
        (0x53e2d620fb40, 100000, 0x53e2d620fb40, 0, 100000),
        (0, -100000, 0, 0, -100000),
        (0, 150001, 0, 2, -49999),
    ] {
        setup(16, 8);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(2);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut survivor = World::new(2);
        survivor.rebase(base);
        survivor.word(0x1f0, 0x123456);
        survivor.amount(0, survivor_amount);
        survivor.amount(1, 0);
        let mut source = World::new(2);
        source.rebase(base);
        source.word(0x1f0, 0x123456);
        source.word(engine_native::cunit::HANDLE, 0xe00200004b01);
        for index in 0..2 {
            source.word(
                0x600 + index * equipment_entry::STRIDE,
                survivor.design(index) as u64,
            );
        }
        source.amount(0, source_amount);
        source.amount(1, 0);
        let reserve = ReserveWorld::for_source(&survivor, base, 1, 0);
        state.countries[1] = reserve._country.as_ptr() as u64;
        let inputs = [source.army()];
        let argument = [
            survivor.army() as u64,
            inputs.as_ptr() as u64,
            (1u64 << 32) | 1,
            0,
        ];
        *MERGE_PLAN.lock().unwrap() = Some(MergePlan {
            source: source.army(),
            reserve_manager: reserve.manager.as_ptr() as usize,
            source_reserve_manager: reserve.manager.as_ptr() as usize,
            retained: [retained, 0],
            credit,
        });
        ORIGINAL[SiteId::Merge as usize]
            .store(merge_native_fixture as *const () as usize, Ordering::SeqCst);
        ORIGINAL[SiteId::Delete as usize].store(
            merged_delete_fixture as *const () as usize,
            Ordering::SeqCst,
        );
        ORIGINAL[SiteId::Land as usize].store(land_removal as *const () as usize, Ordering::SeqCst);
        let land_calls = CALLS[SiteId::Land as usize].load(Ordering::Relaxed);
        let attrition_calls = CALLS[SiteId::Attrition as usize].load(Ordering::Relaxed);
        let calls = CALLS[SiteId::Merge as usize].load(Ordering::Relaxed);
        unsafe {
            merge(argument.as_ptr() as usize);
        }
        assert_eq!(
            CALLS[SiteId::Merge as usize].load(Ordering::Relaxed),
            calls + 1
        );
        assert_eq!(
            CALLS[SiteId::Land as usize].load(Ordering::Relaxed),
            land_calls + 1
        );
        assert_eq!(
            CALLS[SiteId::Attrition as usize].load(Ordering::Relaxed),
            attrition_calls + 1
        );
        assert!(!folded().poisoned);
        assert!(!WHOLE_MERGE.get() && !OBSERVING.get());
        let mut out = vec![Entry::default(); 16];
        let len = drain(&mut out).unwrap();
        assert_eq!(len, 2);
        assert_eq!(out[0].disposition, native::Disposition::Merged);
        assert_eq!(
            (out[1].country, out[1].cause, out[1].amount_raw),
            (1, native::Cause::DivisionMerged, expected)
        );
        assert_eq!(active(), 0);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn whole_merge_includes_only_canceled_held_pools_and_all_bulk_designs() {
    let _serial = serial();
    let limit = 0x53e2d620fb40i64;
    for (held, reserve_amount, expected) in [
        (200000, 0, 0),
        (100000, limit, 300000),
        (-100000, 0, 0),
        (0, limit + 547, 547),
    ] {
        setup(32, 16);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(1);
        state.population(&[2]);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut survivor = World::new(2);
        survivor.rebase(base);
        survivor.amount(0, 0);
        survivor.amount(1, 0);
        let mut source = World::new(2);
        source.rebase(base);
        source.amount(0, 0);
        source.amount(1, 0);
        source.word(engine_native::cunit::HANDLE, 0xe00200004b01);
        for index in 0..2 {
            source.word(
                0x600 + index * equipment_entry::STRIDE,
                survivor.design(index) as u64,
            );
        }
        let survivor_requests = RequestsWorld::new(&mut survivor, base, &[held], &[777777]);
        let source_requests = RequestsWorld::new(&mut source, base, &[held], &[held]);
        let source_held = survivor.design(0) as u64;
        for world in source_requests
            .reinforcement
            .iter()
            .chain(&source_requests.upgrades)
        {
            unsafe {
                *((world.army() + 0x600) as *mut u64) = source_held;
            }
        }
        let mut reserve = ReserveWorld::for_source(&survivor, base, 1, reserve_amount);
        state.roster[0] = survivor.army() as u64;
        state.roster[1] = source.army() as u64;
        reserve._country[engine_native::ccountry::DIVISIONS / 8] = state.roster.as_ptr() as u64;
        reserve._country[engine_native::ccountry::DIVISIONS_CAPACITY / 8] = 2 | (2 << 32);
        state.countries[1] = reserve._country.as_ptr() as u64;
        let inputs = [source.army()];
        let argument = [
            survivor.army() as u64,
            inputs.as_ptr() as u64,
            (1u64 << 32) | 1,
            0,
        ];
        *MERGE_PLAN.lock().unwrap() = Some(MergePlan {
            source: source.army(),
            reserve_manager: reserve.manager.as_ptr() as usize,
            source_reserve_manager: reserve.manager.as_ptr() as usize,
            retained: [0, 0],
            credit: 0,
        });
        ORIGINAL[SiteId::Merge as usize]
            .store(merge_native_fixture as *const () as usize, Ordering::SeqCst);
        ORIGINAL[SiteId::Delete as usize].store(
            merged_delete_fixture as *const () as usize,
            Ordering::SeqCst,
        );
        ORIGINAL[SiteId::DivisionFree as usize].store(
            free_with_canceled_requests as *const () as usize,
            Ordering::SeqCst,
        );
        ORIGINAL[SiteId::Land as usize].store(land_removal as *const () as usize, Ordering::SeqCst);
        unsafe {
            merge(argument.as_ptr() as usize);
        }
        assert!(!folded().poisoned);
        let mut out = vec![Entry::default(); 32];
        let len = drain(&mut out).unwrap();
        assert_eq!(
            out[..len]
                .iter()
                .filter(|row| row.division_handle == 0)
                .map(|row| row.amount_raw)
                .sum::<i64>(),
            expected
        );
        assert_eq!(survivor_requests.root[(0x10 + 0x28 + 8) / 8] >> 32, 0);
        assert_eq!(survivor_requests.root[(0x70 + 0x48 + 8) / 8] >> 32, 1);
        assert_eq!(source_requests.root[(0x10 + 0x28 + 8) / 8] >> 32, 0);
        assert_eq!(source_requests.root[(0x70 + 0x48 + 8) / 8] >> 32, 0);
        assert_eq!(
            survivor_requests.upgrades[0].storage[(0x600 + 8) / 8],
            777777
        );
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let mut pools = 0;
        native::canceled_delivery_pools(&memory, base, survivor.army(), true, |_| {
            pools += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(pools, 1);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn canceled_deliveries_conserve_their_own_native_country_destination() {
    let _serial = serial();
    static TRACKER: collector_primitives::raw_memory::Tracker =
        collector_primitives::raw_memory::Tracker::new();
    let limit = 0x53e2d620fb40;
    for countries in [2usize, 50] {
        for initial in [0, limit] {
            setup(0, 0);
            boundary(clock(1, 1));
            boundary(clock(2, 1));
            let mut state = StateMemory::new(2);
            state.population(&vec![1; countries]);
            let base = state.live_base();
            BASE.store(base, Ordering::Relaxed);
            let mut armies: Vec<_> = (0..countries)
                .map(|index| {
                    let mut army = World::new(2);
                    army.rebase(base);
                    army.owner(index as u32 + 1);
                    army.word(
                        engine_native::cunit::HANDLE,
                        ((index as u64 + 1) << 32) | 0x4b01,
                    );
                    army.amount(0, 0);
                    army.amount(1, 0);
                    army
                })
                .collect();
            for pair in armies.chunks_exact_mut(2) {
                for index in 0..2 {
                    let design = pair[0].design(index);
                    pair[1].word(0x600 + index * equipment_entry::STRIDE, design as u64);
                }
            }
            let mut reserves: Vec<_> = (0..countries)
                .map(|index| {
                    ReserveWorld::for_source(
                        &armies[index / 2 * 2],
                        base,
                        index as u32 + 1,
                        initial,
                    )
                })
                .collect();
            for (index, (army, reserve)) in armies.iter().zip(&mut reserves).enumerate() {
                state.roster[index] = army.army() as u64;
                reserve._country[engine_native::ccountry::DIVISIONS / 8] =
                    unsafe { state.roster.as_ptr().add(index) } as u64;
                reserve._country[engine_native::ccountry::DIVISIONS_CAPACITY / 8] = 1 | (1 << 32);
                state.countries[index + 1] = reserve._country.as_ptr() as u64;
            }
            let requests: Vec<_> = armies
                .iter_mut()
                .map(|army| RequestsWorld::new(army, base, &[200000, 200000], &[200000]))
                .collect();
            assert_eq!(requests.len(), countries);
            {
                let mut store = folded();
                let capacity = store.owned_capacity();
                store._charge = collector_primitives::raw_memory::Charge::empty_on(&TRACKER);
                store._charge.set(capacity);
            }
            ORIGINAL[SiteId::Merge as usize]
                .store(merge_native_fixture as *const () as usize, Ordering::SeqCst);
            ORIGINAL[SiteId::Delete as usize].store(
                merged_delete_fixture as *const () as usize,
                Ordering::SeqCst,
            );
            ORIGINAL[SiteId::DivisionFree as usize].store(
                free_with_canceled_requests as *const () as usize,
                Ordering::SeqCst,
            );
            ORIGINAL[SiteId::Land as usize]
                .store(land_removal as *const () as usize, Ordering::SeqCst);
            for index in (0..countries).step_by(2) {
                let survivor = &armies[index];
                let source = &armies[index + 1];
                let inputs = [source.army()];
                let argument = [
                    survivor.army() as u64,
                    inputs.as_ptr() as u64,
                    (1u64 << 32) | 1,
                    0,
                ];
                *MERGE_PLAN.lock().unwrap() = Some(MergePlan {
                    source: source.army(),
                    reserve_manager: reserves[index].manager.as_ptr() as usize,
                    source_reserve_manager: reserves[index + 1].manager.as_ptr() as usize,
                    retained: [0, 0],
                    credit: 0,
                });
                unsafe {
                    merge(argument.as_ptr() as usize);
                }
                assert!(!folded().poisoned);
            }
            let mut out = Vec::new();
            let len = drain(&mut out).unwrap();
            let mut losses = vec![0i64; countries];
            for row in out[..len].iter().filter(|row| row.division_handle == 0) {
                losses[row.country as usize - 1] += row.amount_raw;
            }
            for (index, reserve) in reserves.iter().enumerate() {
                let amount = if index % 2 == 0 { 400000 } else { 600000 };
                let expected = if initial == limit { amount } else { 0 };
                assert_eq!(losses[index], expected, "native owner {}", index + 1);
                assert_eq!(
                    reserve.world.storage[(0x600 + 8) / 8] as i64,
                    initial + amount - expected
                );
                assert_eq!(
                    requests[index].root[(0x70 + 0x48 + 8) / 8] >> 32,
                    u64::from(index % 2 == 0)
                );
            }
            assert_eq!(TRACKER.snapshot().current, folded().owned_capacity());
            assert!(release().is_err());
            ENABLED.store(false, Ordering::SeqCst);
            SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
            release().unwrap();
            assert_eq!(TRACKER.snapshot().current, Default::default());
        }
    }
}

#[test]
fn pending_hq_source_settles_saved_deficit_during_whole_merge_without_scratch_reuse() {
    let _serial = serial();
    setup(32, 16);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    let mut survivor = World::new(2);
    survivor.rebase(base);
    survivor.amount(0, 0);
    survivor.amount(1, 0);
    let mut source = World::new(2);
    source.rebase(base);
    source.word(engine_native::cunit::HANDLE, 0xe00200004b01);
    for index in 0..2 {
        source.word(
            0x600 + index * equipment_entry::STRIDE,
            survivor.design(index) as u64,
        );
    }
    source.amount(0, 100000);
    source.amount(1, 0);
    let reserve = ReserveWorld::for_source(&survivor, base, 1, 0x53e2d620fb40);
    state.countries[1] = reserve._country.as_ptr() as u64;
    unsafe {
        hq_returned(
            reserve.manager.as_ptr() as usize,
            source.army() + carmy::EQUIPMENT_POOL,
        );
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let handle = native::army_identity(&memory, base, source.army()).unwrap();
    folded().record_queue(
        source.army(),
        handle,
        native::Cause::Unknown,
        native::Disposition::HqTransferred,
    )
    .unwrap();
    let inputs = [source.army()];
    let argument = [
        survivor.army() as u64,
        inputs.as_ptr() as u64,
        (1u64 << 32) | 1,
        0,
    ];
    *MERGE_PLAN.lock().unwrap() = Some(MergePlan {
        source: source.army(),
        reserve_manager: reserve.manager.as_ptr() as usize,
        source_reserve_manager: reserve.manager.as_ptr() as usize,
        retained: [0, 0],
        credit: 0,
    });
    ORIGINAL[SiteId::Merge as usize]
        .store(merge_native_fixture as *const () as usize, Ordering::SeqCst);
    ORIGINAL[SiteId::Delete as usize].store(
        merged_delete_fixture as *const () as usize,
        Ordering::SeqCst,
    );
    ORIGINAL[SiteId::Land as usize].store(land_removal as *const () as usize, Ordering::SeqCst);
    unsafe {
        merge(argument.as_ptr() as usize);
    }
    assert!(!folded().poisoned);
    assert!(!folded().credits.contains(handle));
    let mut out = vec![Entry::default(); 32];
    let len = drain(&mut out).unwrap();
    assert_eq!(
        out[..len]
            .iter()
            .filter(|row| row.division_handle == 0)
            .map(|row| row.amount_raw)
            .sum::<i64>(),
        100000
    );
    assert_eq!(
        out[..len]
            .iter()
            .filter(|row| row.division_handle != 0)
            .count(),
        1
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn credited_survivor_reallocation_restores_live_damage_and_return_accounting() {
    let _serial = serial();
    static NESTED_CREDIT: AtomicBool = AtomicBool::new(false);
    unsafe extern "system-unwind" fn nested_credit(manager: usize, source: usize) {
        if !NESTED_CREDIT.swap(true, Ordering::SeqCst) {
            unsafe {
                hq_returned(manager, source);
            }
        }
        unsafe {
            credit_pool(manager, source);
        }
    }
    for action in [0, 1, 2, 3, 4, 6, 7] {
        let second_return = action == 1;
        setup(32, 16);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(1);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut survivor = World::new(2);
        survivor.rebase(base);
        survivor.amount(0, 100000);
        survivor.amount(1, 0);
        let mut source = World::new(2);
        source.rebase(base);
        source.word(engine_native::cunit::HANDLE, 0xe00200004b01);
        for index in 0..2 {
            source.word(
                0x600 + index * equipment_entry::STRIDE,
                survivor.design(index) as u64,
            );
        }
        source.amount(0, 100000);
        source.amount(1, 0);
        let reserve = ReserveWorld::for_source(&survivor, base, 1, 0);
        state.countries[1] = reserve._country.as_ptr() as u64;
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let handle = native::army_identity(&memory, base, survivor.army()).unwrap();
        unsafe {
            hq_returned(
                reserve.manager.as_ptr() as usize,
                survivor.army() + carmy::EQUIPMENT_POOL,
            );
        }
        folded().record_queue(
            survivor.army(),
            handle,
            native::Cause::Unknown,
            native::Disposition::HqTransferred,
        )
        .unwrap();
        let inputs = [source.army()];
        let argument = [
            survivor.army() as u64,
            inputs.as_ptr() as u64,
            (1u64 << 32) | 1,
            0,
        ];
        *MERGE_PLAN.lock().unwrap() = Some(MergePlan {
            source: source.army(),
            reserve_manager: reserve.manager.as_ptr() as usize,
            source_reserve_manager: reserve.manager.as_ptr() as usize,
            retained: [150000, 0],
            credit: 0,
        });
        ORIGINAL[SiteId::Merge as usize]
            .store(merge_native_fixture as *const () as usize, Ordering::SeqCst);
        ORIGINAL[SiteId::Delete as usize].store(
            merged_delete_fixture as *const () as usize,
            Ordering::SeqCst,
        );
        ORIGINAL[SiteId::Land as usize].store(land_removal as *const () as usize, Ordering::SeqCst);
        unsafe {
            merge(argument.as_ptr() as usize);
        }
        assert_eq!(
            folded()
                .credits
                .state(handle, survivor.army(), 1)
                .unwrap(),
            PoolState::Live
        );
        if action >= 2 {
            if action == 2 {
                poison();
            }
            if action == 3 {
                OBSERVING.set(true);
            }
            let old_identity = reserve.manager[0];
            if action == 6 {
                unsafe {
                    *(reserve.manager.as_ptr() as *mut u64) = 0;
                }
            }
            if action == 7 {
                NESTED_CREDIT.store(false, Ordering::SeqCst);
                ORIGINAL[SiteId::HqReturn as usize]
                    .store(nested_credit as *const () as usize, Ordering::SeqCst);
            } else {
                ORIGINAL[SiteId::HqReturn as usize]
                    .store(credit_pool as *const () as usize, Ordering::SeqCst);
            }
            let guard = (action == 4).then(|| folded());
            unsafe {
                hq_returned(
                    reserve.manager.as_ptr() as usize,
                    survivor.army() + carmy::EQUIPMENT_POOL,
                );
            }
            drop(guard);
            unsafe {
                *(reserve.manager.as_ptr() as *mut u64) = old_identity;
            }
            OBSERVING.set(false);
            boundary(clock(20, 1));
            boundary(clock(21, 1));
            assert_eq!(
                folded().credits.state(handle, survivor.army(), 1).is_ok(),
                action == 4
            );
            assert!(!folded().poisoned);
            assert_eq!(
                reserve.world.storage[(0x600 + 8) / 8] as i64,
                if action == 7 { 400000 } else { 250000 }
            );
            unsafe {
                division_free(survivor.army(), 1);
            }
            assert!(!folded().credits.contains(handle));
            assert_eq!(active(), 0);
            continue;
        }
        if second_return {
            unsafe {
                hq_returned(
                    reserve.manager.as_ptr() as usize,
                    survivor.army() + carmy::EQUIPMENT_POOL,
                );
            }
            assert_eq!(
                folded()
                    .credits
                    .state(handle, survivor.army(), 1)
                    .unwrap(),
                PoolState::Bookkeeping
            );
        }
        ORIGINAL[SiteId::Delete as usize]
            .store(cleanup_delete as *const () as usize, Ordering::SeqCst);
        unsafe {
            delete(
                state.state.as_ptr() as usize,
                survivor.army(),
                0,
                0,
                base + native::QUEUED_DELETE_RETURN.0,
            );
        }
        assert!(!folded().poisoned);
        assert!(!folded().credits.contains(handle));
        let mut out = vec![Entry::default(); 32];
        let len = drain(&mut out).unwrap();
        assert_eq!(
            out[..len]
                .iter()
                .filter(|row| row.division_handle == 0)
                .map(|row| row.amount_raw)
                .sum::<i64>(),
            if second_return { -50000 } else { 100000 }
        );
        assert_eq!(
            out[..len]
                .iter()
                .filter(|row| row.division_handle != 0)
                .count(),
            2
        );
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn refused_pool_writers_and_unrelated_destructor_keep_outer_basis_unavailable() {
    let _serial = serial();
    static OTHER_POOL: AtomicUsize = AtomicUsize::new(0);
    static OUTER_POOL: AtomicUsize = AtomicUsize::new(0);
    static MANAGER: AtomicUsize = AtomicUsize::new(0);
    static MODE: AtomicUsize = AtomicUsize::new(0);
    static NESTED: AtomicBool = AtomicBool::new(false);
    unsafe extern "system-unwind" fn held_free(army: usize, flags: usize) -> usize {
        unsafe {
            cancel_native_requests(army, MANAGER.load(Ordering::SeqCst), true);
            erase(army, flags)
        }
    }
    unsafe extern "system-unwind" fn free_before_observer(army: usize, flags: usize) -> usize {
        unsafe {
            hq_returned(
                MANAGER.load(Ordering::SeqCst),
                OUTER_POOL.load(Ordering::SeqCst),
            );
            erase(army, flags)
        }
    }
    unsafe extern "system-unwind" fn credit_during_free(manager: usize, pool: usize) {
        unsafe {
            cancel_native_requests(
                OTHER_POOL.load(Ordering::SeqCst) - carmy::EQUIPMENT_POOL,
                manager,
                true,
            );
            credit_pool(manager, pool);
        }
    }
    unsafe extern "system-unwind" fn shared_credit(manager: usize, pool: usize) {
        if !NESTED.swap(true, Ordering::SeqCst) {
            match MODE.load(Ordering::SeqCst) {
                0 => unsafe {
                    hq_returned(manager, OTHER_POOL.load(Ordering::SeqCst));
                },
                1 | 2 => {
                    if MODE.load(Ordering::SeqCst) == 2 {
                        poison();
                    }
                    unsafe {
                        scripted_reserve_equipment_fraction(
                            native::reserve_pool(manager).unwrap(),
                            50000,
                        );
                    }
                }
                3 => unsafe {
                    division_free(OTHER_POOL.load(Ordering::SeqCst) - carmy::EQUIPMENT_POOL, 1);
                },
                _ => unreachable!(),
            }
        }
        unsafe {
            credit_pool(manager, pool);
        }
    }
    for mode in 0..5 {
        setup(16, 4);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(1);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut outer = World::new(1);
        outer.rebase(base);
        outer.amount(0, 100000);
        let mut other = World::new(1);
        other.rebase(base);
        other.amount(0, 100000);
        other.word(engine_native::cunit::HANDLE, 0xe00200004b01);
        other.word(0x600, outer.design(0) as u64);
        let requests = RequestsWorld::new(&mut other, base, &[200000], &[]);
        let reserve = ReserveWorld::for_source(&outer, base, 1, 0);
        unsafe {
            hq_returned(
                reserve.manager.as_ptr() as usize,
                outer.army() + carmy::EQUIPMENT_POOL,
            );
            hq_returned(
                reserve.manager.as_ptr() as usize,
                other.army() + carmy::EQUIPMENT_POOL,
            );
        }
        let outer_handle = context_rebased(&outer, base, native::Cause::Unknown).handle;
        let other_handle = context_rebased(&other, base, native::Cause::Unknown).handle;
        OTHER_POOL.store(other.army() + carmy::EQUIPMENT_POOL, Ordering::SeqCst);
        OUTER_POOL.store(outer.army() + carmy::EQUIPMENT_POOL, Ordering::SeqCst);
        MANAGER.store(reserve.manager.as_ptr() as usize, Ordering::SeqCst);
        MODE.store(mode, Ordering::SeqCst);
        NESTED.store(false, Ordering::SeqCst);
        ORIGINAL[SiteId::HqReturn as usize]
            .store(shared_credit as *const () as usize, Ordering::SeqCst);
        ORIGINAL[SiteId::ScriptedReserveEquipmentFraction as usize]
            .store(fraction_scale as *const () as usize, Ordering::SeqCst);
        if mode == 3 {
            ORIGINAL[SiteId::DivisionFree as usize]
                .store(held_free as *const () as usize, Ordering::SeqCst);
        }
        if mode == 4 {
            ORIGINAL[SiteId::HqReturn as usize]
                .store(credit_during_free as *const () as usize, Ordering::SeqCst);
            ORIGINAL[SiteId::DivisionFree as usize]
                .store(free_before_observer as *const () as usize, Ordering::SeqCst);
            unsafe {
                division_free(other.army(), 1);
            }
        } else {
            unsafe {
                hq_returned(
                    reserve.manager.as_ptr() as usize,
                    outer.army() + carmy::EQUIPMENT_POOL,
                );
            }
        }
        assert_eq!(
            reserve.world.storage[(0x600 + 8) / 8],
            match mode {
                0 => 400000,
                1 | 2 => 200000,
                3 | 4 => 500000,
                _ => unreachable!(),
            }
        );
        assert!(
            folded()
                .credits
                .state(outer_handle, outer.army(), 1)
                .is_err()
        );
        if mode < 3 {
            unsafe {
                division_free(other.army(), 1);
            }
        } else {
            assert_eq!(requests.root[(0x10 + 0x28 + 8) / 8] >> 32, 0);
        }
        assert!(!folded().credits.contains(other_handle));
        boundary(clock(20, 1));
        boundary(clock(21, 1));
        assert!(
            folded()
                .credits
                .state(outer_handle, outer.army(), 1)
                .is_err()
        );
        assert!(!folded().poisoned);
        assert_eq!(active(), 0);
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn a_refused_writer_inside_a_return_window_keeps_its_credit_unproven() {
    let _serial = serial();
    unsafe extern "system-unwind" fn gated_credit(manager: usize, source: usize) {
        GATE.wait();
        GATE.wait();
        unsafe { credit_pool(manager, source) };
    }
    setup(8, 4);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    let mut world = World::new(1);
    world.rebase(base);
    world.amount(0, 100000);
    let reserve = ReserveWorld::for_source(&world, base, 1, 0);
    ORIGINAL[SiteId::HqReturn as usize].store(gated_credit as *const () as usize, Ordering::SeqCst);
    let (manager, pool) = (
        reserve.manager.as_ptr() as usize,
        world.army() + carmy::EQUIPMENT_POOL,
    );
    let writer = std::thread::spawn(move || unsafe { hq_returned(manager, pool) });
    GATE.wait();
    revoke_active_observation();
    GATE.wait();
    writer.join().unwrap();
    assert_eq!(reserve.world.storage[(0x600 + 8) / 8], 100000);
    let handle = context_rebased(&world, base, native::Cause::Unknown).handle;
    assert!(folded().credits.state(handle, world.army(), 1).is_err());
    assert!(folded().poisoned);
    boundary(clock(20, 1));
    boundary(clock(21, 1));
    assert!(folded().credits.state(handle, world.army(), 1).is_err());
    assert!(!folded().poisoned);
    assert_eq!(active(), 0);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn missed_destructor_invalidation_survives_contention_unwind_and_gap() {
    let _serial = serial();
    unsafe extern "system-unwind" fn incomplete_free(army: usize, flags: usize) -> usize {
        unsafe {
            erase(army, flags);
        }
        panic!("synthetic skipped destructor unwind");
    }
    for unwind in [false, true] {
        setup(8, 4);
        boundary(clock(1, 1));
        boundary(clock(2, 1));
        let mut state = StateMemory::new(1);
        let base = state.live_base();
        BASE.store(base, Ordering::Relaxed);
        let mut world = World::new(1);
        world.rebase(base);
        world.amount(0, 100000);
        let reserve = ReserveWorld::for_source(&world, base, 1, 0);
        unsafe {
            hq_returned(
                reserve.manager.as_ptr() as usize,
                world.army() + carmy::EQUIPMENT_POOL,
            );
        }
        let handle = context_rebased(&world, base, native::Cause::Unknown).handle;
        poison();
        let guard = folded();
        if unwind {
            ORIGINAL[SiteId::DivisionFree as usize]
                .store(incomplete_free as *const () as usize, Ordering::SeqCst);
        }
        let result = std::panic::catch_unwind(|| unsafe {
            division_free(world.army(), 1);
        });
        assert_eq!(result.is_err(), unwind);
        drop(guard);
        assert!(
            folded()
                .credits
                .state(handle, world.army(), 1)
                .is_err()
        );
        boundary(clock(20, 1));
        assert_eq!(folded().poisoned, unwind);
        boundary(clock(21, 1));
        assert_eq!(active(), usize::from(unwind));
        boundary(clock(21, 2));
        assert!(!folded().credits.contains(handle));
    }
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn live_credited_survivor_can_later_merge_as_a_queued_hq_source() {
    let _serial = serial();
    setup(32, 16);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    let mut survivor = World::new(2);
    survivor.rebase(base);
    survivor.amount(0, 100000);
    survivor.amount(1, 0);
    let mut donor = World::new(2);
    donor.rebase(base);
    donor.word(engine_native::cunit::HANDLE, 0xe00200004b01);
    let mut recipient = World::new(2);
    recipient.rebase(base);
    recipient.word(engine_native::cunit::HANDLE, 0xe00300004b01);
    for index in 0..2 {
        donor.word(
            0x600 + index * equipment_entry::STRIDE,
            survivor.design(index) as u64,
        );
        recipient.word(
            0x600 + index * equipment_entry::STRIDE,
            survivor.design(index) as u64,
        );
    }
    donor.amount(0, 100000);
    donor.amount(1, 0);
    recipient.amount(0, 0);
    recipient.amount(1, 0);
    let reserve = ReserveWorld::for_source(&survivor, base, 1, 0);
    state.countries[1] = reserve._country.as_ptr() as u64;
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let handle = native::army_identity(&memory, base, survivor.army()).unwrap();
    unsafe {
        hq_returned(
            reserve.manager.as_ptr() as usize,
            survivor.army() + carmy::EQUIPMENT_POOL,
        );
    }
    folded().record_queue(
        survivor.army(),
        handle,
        native::Cause::Unknown,
        native::Disposition::HqTransferred,
    )
    .unwrap();
    ORIGINAL[SiteId::Merge as usize]
        .store(merge_native_fixture as *const () as usize, Ordering::SeqCst);
    ORIGINAL[SiteId::Delete as usize].store(
        merged_delete_fixture as *const () as usize,
        Ordering::SeqCst,
    );
    ORIGINAL[SiteId::Land as usize].store(land_removal as *const () as usize, Ordering::SeqCst);
    for (receiver, source) in [
        (survivor.army(), donor.army()),
        (recipient.army(), survivor.army()),
    ] {
        let inputs = [source];
        let argument = [receiver as u64, inputs.as_ptr() as u64, (1u64 << 32) | 1, 0];
        *MERGE_PLAN.lock().unwrap() = Some(MergePlan {
            source,
            reserve_manager: reserve.manager.as_ptr() as usize,
            source_reserve_manager: reserve.manager.as_ptr() as usize,
            retained: [150000, 0],
            credit: 0,
        });
        unsafe {
            merge(argument.as_ptr() as usize);
        }
        assert!(!folded().poisoned);
    }
    assert!(!folded().credits.contains(handle));
    ORIGINAL[SiteId::Delete as usize].store(cleanup_delete as *const () as usize, Ordering::SeqCst);
    unsafe {
        delete(
            state.state.as_ptr() as usize,
            recipient.army(),
            0,
            0,
            base + native::COMBAT_DELETION_RETURN.0,
        );
    }
    assert!(!folded().poisoned);
    let mut out = vec![Entry::default(); 32];
    let len = drain(&mut out).unwrap();
    assert_eq!(
        out[..len]
            .iter()
            .filter(|row| row.division_handle == 0)
            .map(|row| row.amount_raw)
            .sum::<i64>(),
        100000
    );
    assert_eq!(
        out[..len]
            .iter()
            .filter(|row| row.division_handle != 0)
            .count(),
        3
    );
    assert_eq!(
        out[..len]
            .iter()
            .find(|row| row.division_handle == handle)
            .unwrap()
            .disposition,
        native::Disposition::HqTransferred
    );
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn conservation_wide_duplicates_cancel_before_serialized_width_check() {
    let mut values = [
        Net {
            country: 0,
            variant: 1,
            amount: i128::from(i64::MAX),
            credited: 0,
        },
        Net {
            country: 0,
            variant: 1,
            amount: i128::from(i64::MAX),
            credited: 0,
        },
        Net {
            country: 0,
            variant: 1,
            amount: -i128::from(i64::MAX),
            credited: 0,
        },
    ];
    assert_eq!(transfers::compact(&mut values).unwrap(), 1);
    assert_eq!(values[0].amount, i128::from(i64::MAX));
}

#[test]
fn native_merge_selection_and_reserve_reads_stay_with_reached_designs() {
    let _serial = serial();
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let mut survivor = World::new(2);
    survivor.word(0x1f0, 17);
    let mut selected = World::new(2);
    selected.word(0x1f0, 17);
    let mut excluded = World::new(2);
    excluded.word(0x1f0, 19);
    excluded.word(0, 0);
    excluded.word(carmy::EQUIPMENT_POOL, 0);
    let inputs = [selected.army(), excluded.army(), selected.army()];
    let argument = [
        survivor.army() as u64,
        inputs.as_ptr() as u64,
        (3u64 << 32) | 3,
        0,
    ];
    assert_eq!(
        native::merge_input_count(&memory, argument.as_ptr() as usize),
        Ok(3)
    );
    let mut armies = [0; 4];
    let (native_survivor, len) =
        native::merge_armies(&memory, 0, argument.as_ptr() as usize, &mut armies).unwrap();
    assert_eq!(native_survivor, survivor.army());
    assert_eq!(len, 2);
    assert!(armies[..len].contains(&selected.army()) && armies[..len].contains(&survivor.army()));
    assert!(
        native::merge_armies(&memory, 0, argument.as_ptr() as usize, &mut [0; 1])
            .unwrap_err()
            .contains("scratch too short")
    );
    let mut reserve = ReserveWorld::for_source(&survivor, 0, 1, 100);
    reserve.world.word(0x600 + equipment_entry::STRIDE, 0xdead);
    let mut rows = [equipment_entry::Row::default(); 2];
    native::snapshot(
        &memory,
        0,
        survivor.army() + carmy::EQUIPMENT_POOL,
        &mut rows,
    )
    .unwrap();
    rows[1].variant_pointer = survivor.design(0);
    rows[1].variant_handle = rows[0].variant_handle;
    rows[0].amount_raw = 0;
    rows[1].amount_raw = -10;
    assert_eq!(
        native::touched_reserves(
            &memory,
            0,
            reserve.pool(),
            &rows,
            true,
            &mut [Default::default(); 2]
        )
        .unwrap(),
        0
    );
    assert_eq!(
        native::touched_reserves(
            &memory,
            0,
            reserve.pool(),
            &rows,
            false,
            &mut [Default::default(); 2]
        )
        .unwrap(),
        1
    );
    rows[1].amount_raw = 10;
    assert_eq!(
        native::touched_reserves(
            &memory,
            0,
            reserve.pool(),
            &rows,
            true,
            &mut [Default::default(); 2]
        )
        .unwrap(),
        1
    );
}

#[test]
fn skipped_whole_merge_revokes_survivor_and_donor_proofs_before_unwind() {
    let _serial = serial();
    unsafe extern "system-unwind" fn incomplete_donor_mutation(argument: usize) {
        let inputs = unsafe { *((argument + 8) as *const usize) };
        let donor = unsafe { *(inputs as *const usize) };
        let items = unsafe { *((donor + carmy::EQUIPMENT_POOL + pool::ITEMS) as *const usize) };
        unsafe {
            *((items + equipment_entry::AMOUNT) as *mut i64) = 0;
        }
        panic!("synthetic skipped merge unwind");
    }
    setup(8, 4);
    boundary(clock(1, 1));
    boundary(clock(2, 1));
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    let mut survivor = World::new(1);
    survivor.rebase(base);
    survivor.amount(0, 100000);
    let mut donor = World::new(1);
    donor.rebase(base);
    donor.amount(0, 100000);
    donor.word(engine_native::cunit::HANDLE, 0xe00200004b01);
    let survivor_reserve = ReserveWorld::for_source(&survivor, base, 1, 0);
    let donor_reserve = ReserveWorld::for_source(&donor, base, 1, 0);
    unsafe {
        hq_returned(
            survivor_reserve.manager.as_ptr() as usize,
            survivor.army() + carmy::EQUIPMENT_POOL,
        );
        hq_returned(
            donor_reserve.manager.as_ptr() as usize,
            donor.army() + carmy::EQUIPMENT_POOL,
        );
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let survivor_handle = native::army_identity(&memory, base, survivor.army()).unwrap();
    let donor_handle = native::army_identity(&memory, base, donor.army()).unwrap();
    poison();
    ORIGINAL[SiteId::Merge as usize].store(
        incomplete_donor_mutation as *const () as usize,
        Ordering::SeqCst,
    );
    let inputs = [donor.army()];
    let argument = [
        survivor.army() as u64,
        inputs.as_ptr() as u64,
        (1u64 << 32) | 1,
        0,
    ];
    assert!(
        std::panic::catch_unwind(|| unsafe {
            merge(argument.as_ptr() as usize);
        })
        .is_err()
    );
    boundary(clock(20, 1));
    boundary(clock(21, 1));
    for (handle, army) in [
        (survivor_handle, survivor.army()),
        (donor_handle, donor.army()),
    ] {
        assert!(
            folded()
                .credits
                .state(handle, army, 1)
                .is_err()
        );
    }
    assert!(!folded().poisoned);
    assert_eq!(active(), 1);
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}

#[test]
fn native_merge_unwind_keeps_original_once_and_poisoned_observation_quiet() {
    let _serial = serial();
    static ORIGINAL_CALLS: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "system-unwind" fn incomplete_merge(argument: usize) {
        let survivor = unsafe { *(argument as *const usize) };
        let items = unsafe { *((survivor + carmy::EQUIPMENT_POOL + pool::ITEMS) as *const usize) };
        unsafe {
            *((items + equipment_entry::AMOUNT) as *mut i64) = 0;
        }
        ORIGINAL_CALLS.fetch_add(1, Ordering::Relaxed);
        panic!("synthetic native merge unwind");
    }
    setup(8, 4);
    let mut state = StateMemory::new(1);
    let base = state.live_base();
    BASE.store(base, Ordering::Relaxed);
    let mut world = World::new(1);
    world.rebase(base);
    world.amount(0, -10);
    let reserve = ReserveWorld::for_source(&world, base, 1, 0);
    unsafe {
        hq_returned(
            reserve.manager.as_ptr() as usize,
            world.army() + carmy::EQUIPMENT_POOL,
        );
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let handle = native::army_identity(&memory, base, world.army()).unwrap();
    let argument = [world.army() as u64, 0, 0, 0];
    ORIGINAL[SiteId::Merge as usize]
        .store(incomplete_merge as *const () as usize, Ordering::SeqCst);
    let calls = ORIGINAL_CALLS.load(Ordering::Relaxed);
    let result = std::panic::catch_unwind(|| unsafe { merge(argument.as_ptr() as usize) });
    assert!(result.is_err());
    assert_eq!(ORIGINAL_CALLS.load(Ordering::Relaxed), calls + 1);
    assert!(!WHOLE_MERGE.get() && !OBSERVING.get());
    assert_eq!(active(), 1);
    assert!(folded().poisoned);
    assert!(
        drain(&mut vec![Entry::default(); 8])
            .unwrap_err()
            .contains("did not settle")
    );
    boundary(clock(20, 1));
    boundary(clock(21, 1));
    assert!(
        folded()
            .credits
            .state(handle, world.army(), 1)
            .unwrap_err()
            .contains("lifetime incomplete")
    );
    assert!(!folded().poisoned);
    boundary(clock(22, 2));
    assert!(!folded().credits.contains(handle));
    ENABLED.store(false, Ordering::SeqCst);
    SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
}
