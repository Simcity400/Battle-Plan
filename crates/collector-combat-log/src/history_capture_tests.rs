use super::*;
use engine_land_inputs::{
    ncombat_log_cequipment_loss as loss, ncombat_log_cmanager as manager,
    ncombat_log_corders_group_logs as group,
};
use engine_native::{
    cequipment_variant as variant, cequipment_variant_pool as pool, equipment_entry,
};
use std::cell::Cell;

fn isolated() -> bool {
    const CHILD: &str = "BATTLEPLAN_COMBAT_LOG_ISOLATED_TEST";
    let thread = std::thread::current();
    let name = thread.name().unwrap();
    if std::env::var_os(CHILD).as_deref() == Some(std::ffi::OsStr::new(name)) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "isolated regression failed: {}\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    false
}
struct TerminalHooks;
impl TerminalHooks {
    fn install() -> Self {
        fn failed() {
            let _ = crate::allocation::record_failure(
                crate::allocation::phase(),
                crate::allocation::Code::Reserve,
            );
        }
        crate::allocation::reset();
        engine_allocation::set_hooks(None, Some(failed));
        engine_allocation::set_stop_hook(Some(crate::allocation::failed));
        assert!(engine_allocation::capture_active());
        Self
    }
    fn assert_live(&self) {
        assert!(!engine_allocation::capture_stopped());
        assert!(crate::allocation::failure().is_none());
    }
    fn prove_terminal(&self) {
        assert!(engine_allocation::vec::<u8>(usize::MAX).is_err());
        assert!(engine_allocation::capture_stopped());
        assert!(crate::allocation::failure().is_some());
    }
}
impl Drop for TerminalHooks {
    fn drop(&mut self) {
        engine_allocation::set_hooks(None, None);
        engine_allocation::set_stop_hook(None);
        crate::allocation::reset();
    }
}
fn capture_public(state: &mut State, memory: &impl Memory) -> Captured {
    state.begin(SIGNAL, false);
    state.acquire(memory, 0, GAME_STATE);
    let mut batch = Captured::new();
    state.snapshot(&mut batch);
    batch
}

const GAME_STATE: usize = 0x1000;
const SIGNAL: i64 = 123;
const DATE: i32 = 60_759_374;

struct Fixture {
    blocks: BTreeMap<usize, Vec<u8>>,
    next: usize,
    unreadable: Option<usize>,
}
impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self
            .unreadable
            .is_some_and(|p| (address..address.saturating_add(destination.len())).contains(&p))
        {
            return false;
        }
        let Some((&start, bytes)) = self.blocks.range(..=address).next_back() else {
            return false;
        };
        let offset = address - start;
        let Some(source) = bytes.get(offset..offset + destination.len()) else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}
impl Fixture {
    fn new() -> Self {
        let mut f = Self {
            blocks: BTreeMap::from([(GAME_STATE, vec![0; 0x1000])]),
            next: 0x10000,
            unreadable: None,
        };
        f.managers(&[]);
        f
    }
    fn alloc(&mut self, size: usize) -> usize {
        let pointer = self.next;
        self.next += size + 16;
        self.blocks.insert(pointer, vec![0; size]);
        pointer
    }
    fn put(&mut self, address: usize, value: &[u8]) {
        let (&start, bytes) = self.blocks.range_mut(..=address).next_back().unwrap();
        bytes[address - start..address - start + value.len()].copy_from_slice(value);
    }
    fn word(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }
    fn int(&mut self, address: usize, value: i32) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, pointer: usize, count: usize) {
        self.word(address, pointer as u64);
        self.int(address + 8, count as i32);
        self.int(address + 12, count as i32);
    }
    fn pointers(&mut self, address: usize, pointers: &[usize]) {
        let list = self.alloc(pointers.len() * 8);
        for (i, &pointer) in pointers.iter().enumerate() {
            self.word(list + i * 8, pointer as u64);
        }
        self.array(address, list, pointers.len());
    }
    fn managers(&mut self, countries: &[(u32, Vec<usize>)]) {
        let mut pointers = vec![0];
        for (country, groups) in countries {
            let p = self.alloc(manager::SIZE);
            self.word(p, manager::VTABLE_RVA.0 as u64);
            self.int(p + manager::COUNTRY, *country as i32);
            self.pointers(p + manager::GROUPS, groups);
            pointers.push(p);
        }
        self.pointers(
            GAME_STATE + engine_native::cgame_state::COMBAT_LOGS,
            &pointers,
        );
    }
    fn group(&mut self, id: u32) -> usize {
        let p = self.alloc(group::SIZE);
        self.word(p, group::VTABLE_RVA.0 as u64);
        self.word(
            p + group::GROUP,
            (u64::from(id) << 32) | u64::from(engine_native::corders_group::REFERENCE_TYPE),
        );
        p
    }
    fn manpower(&mut self, group: usize, values: &[[i32; 3]]) -> Vec<usize> {
        let pointers: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(i, values)| {
                let p = self.alloc(native::MANPOWER_SIZE);
                self.word(p, native::MANPOWER_VTABLE_RVA.0 as u64);
                self.int(p + loss::DATE, DATE + i as i32);
                for (index, &value) in values.iter().enumerate() {
                    self.int(p + native::MANPOWER_LOSSES + index * 4, value);
                }
                p
            })
            .collect();
        self.pointers(group + native::MANPOWER, &pointers);
        pointers
    }
    fn equipment(&mut self, owner: usize, amounts: &[i64]) {
        let v = self.alloc(16);
        self.word(v, variant::VTABLE_RVA.0 as u64);
        self.word(
            v + 8,
            u64::from(variant::REFERENCE_TYPE) | (u64::from(u32::MAX) << 32),
        );
        let items = self.alloc(amounts.len() * equipment_entry::STRIDE);
        for (i, &amount) in amounts.iter().enumerate() {
            self.word(items + i * equipment_entry::STRIDE, v as u64);
            self.word(
                items + i * equipment_entry::STRIDE + equipment_entry::AMOUNT,
                amount as u64,
            );
        }
        self.array(owner + pool::ITEMS, items, amounts.len());
        let allocation = self.alloc(24);
        self.word(allocation + 16, 1);
        self.array(owner + pool::ALLOCATIONS, allocation, 1);
    }
    fn month(&mut self, group: usize, kind: usize, amounts: &[i64]) {
        let p = self.alloc(loss::SIZE);
        self.word(p, loss::VTABLE_RVA.0 as u64);
        self.int(p + loss::DATE, DATE);
        self.int(
            p + loss::REASON,
            if kind < group::REASON_COUNT {
                kind as i32
            } else {
                0
            },
        );
        self.word(p + loss::POOL, pool::VTABLE_RVA.0 as u64);
        self.put(p + loss::POOL + pool::ALLOW_ZERO, &[1]);
        self.equipment(p + loss::POOL, amounts);
        self.pointers(group + event::ARRAYS[kind], &[p]);
    }
    fn references(&mut self, group: usize, sides: &[u8]) {
        let entries = self.alloc(sides.len() * 8);
        for (i, &side) in sides.iter().enumerate() {
            self.int(entries + i * 8, 0);
            self.put(entries + i * 8 + 4, &[side]);
        }
        self.array(group + native::REFERENCES, entries, sides.len());
    }
    fn battle(&mut self, count: i16) -> usize {
        self.blocks.insert(
            native::BATTLE_POOL_RVA.0,
            vec![0; 3 * engine_native::cpdx_array::STRIDE],
        );
        let p = self.alloc(native::BATTLE_SIZE);
        self.word(p, native::BATTLE_VTABLE_RVA.0 as u64);
        self.int(p + native::BATTLE_DATE, DATE);
        self.int(p + native::BATTLE_PROVINCE, 53);
        self.put(p + native::BATTLE_FLAGS, &[2, 0, 1, 255]);
        for (i, offset) in [native::BATTLE_ATTACKER, native::BATTLE_DEFENDER]
            .into_iter()
            .enumerate()
        {
            let side = p + offset;
            self.word(side, native::SIDE_VTABLE_RVA.0 as u64);
            self.int(side + native::SIDE_MANPOWER, i32::MIN + i as i32);
            self.word(side + native::SIDE_AIR_FACTOR, i64::MIN as u64);
            self.word(side + native::SIDE_LEADER, 67 | (9 << 32));
            for offset in [
                native::SIDE_EQUIPMENT,
                native::SIDE_CAPTURED_BY_ENEMY,
                native::SIDE_RECOVERED,
            ] {
                self.word(side + offset, pool::VTABLE_RVA.0 as u64);
            }
            self.equipment(side + native::SIDE_EQUIPMENT, &[i64::MIN, 0, i64::MAX]);
            let modifiers = self.alloc(16);
            self.word(modifiers, native::MODIFIER_HOURS_VTABLE_RVA.0 as u64);
            self.int(modifiers + 8, -1);
            self.int(modifiers + 12, i32::MAX);
            self.array(side + native::SIDE_MODIFIERS, modifiers, 1);
            let countries = self.alloc(8);
            self.int(countries, 7);
            self.int(countries + 4, 7);
            self.array(side + native::SIDE_COUNTRIES, countries, 2);
        }
        self.array(native::BATTLE_POOL_RVA.0, p, 1);
        let counts = self.alloc(2);
        self.put(counts, &count.to_le_bytes());
        self.array(native::BATTLE_COUNTS_RVA.0, counts, 1);
        let used = self.alloc(1);
        self.put(used, &[u8::from(count > 0)]);
        self.array(native::BATTLE_USED_RVA.0, used, 1);
        p
    }
    fn noncombat_sources(&mut self, countries: usize, variants: usize) {
        use engine_native::{carmy, ccountry, cgame_state};
        let mut pointers = vec![0];
        let links = self.alloc((countries + 1) * 4);
        for slot in 1..=countries {
            self.int(links + slot * 4, slot as i32);
            let country = self.alloc((ccountry::DIVISIONS + 16).max(ccountry::RESOURCES + 8));
            self.word(country, ccountry::VTABLE_RVA.0 as u64);
            self.int(country + ccountry::ID, slot as i32);
            let resources = self.alloc(engine_resources::clend_lease_exchange::OUTGOING + 16);
            self.word(
                resources,
                engine_resources::ccountry_resources::VTABLE_RVA.0 as u64,
            );
            self.int(
                resources + engine_resources::ccountry_resources::OWNER,
                slot as i32,
            );
            self.word(country + ccountry::RESOURCES, resources as u64);
            let army = self.alloc(carmy::EQUIPMENT_POOL + pool::ALLOW_ZERO + 1);
            self.word(army, carmy::VTABLE_RVA.0 as u64);
            self.word(army + carmy::EQUIPMENT_POOL, pool::VTABLE_RVA.0 as u64);
            let entries = self.alloc(variants * equipment_entry::STRIDE);
            self.array(
                army + carmy::EQUIPMENT_POOL + pool::ITEMS,
                entries,
                variants,
            );
            self.pointers(country + ccountry::DIVISIONS, &[army]);
            pointers.push(country);
        }
        self.pointers(GAME_STATE + cgame_state::COUNTRIES, &pointers);
        self.array(
            GAME_STATE + cgame_state::COUNTRY_LINKS,
            links,
            countries + 1,
        );
    }
    fn complete(&mut self) -> (usize, usize) {
        let g = self.group(9);
        let manpower = self.manpower(g, &[[i32::MIN, 0, i32::MAX]])[0];
        self.month(g, 0, &[]);
        self.month(g, 5, &[i64::MIN, 0, i64::MAX]);
        let t = self.alloc(native::TEMPLATE_SIZE);
        self.word(t, native::TEMPLATE_VTABLE_RVA.0 as u64);
        self.word(t + native::TEMPLATE_REFERENCE, 52 | (7 << 32));
        self.int(t + native::TEMPLATE_DATE, DATE);
        self.int(t + native::TEMPLATE_WINS, -1);
        self.int(t + native::TEMPLATE_TOTAL, 5);
        self.pointers(g + native::TEMPLATES, &[t]);
        self.references(g, &[1, 0]);
        self.battle(2);
        self.managers(&[(7, vec![g])]);
        (g, manpower)
    }
}
fn witness(revision: u64, lost: u64) -> Option<history_events::Witness> {
    Some(history_events::Witness { revision, lost })
}
fn capture(
    state: &mut State,
    f: &impl Memory,
    signal: i64,
    w: Option<history_events::Witness>,
) -> Captured {
    state.begin_observed(signal, false, w);
    if let Err(reason) = state.acquire_observed(f, 0, GAME_STATE, || w) {
        state.refuse(reason);
    }
    let mut captured = Captured::new();
    state.snapshot(&mut captured);
    captured
}
fn views(c: &mut Conversion, batch: &Captured) -> Views {
    assert_eq!(batch.failure, None);
    c.apply(batch).unwrap();
    c.held.snapshot()
}

fn encoded(views: &Views) -> Vec<Vec<u8>> {
    views
        .iter()
        .map(|view| {
            let mut bytes = Vec::new();
            view.as_ref().unwrap().encode_rows(&mut bytes).unwrap();
            bytes
        })
        .collect()
}
fn captured_entries(
    state: &mut State,
    f: &impl Memory,
    w: Option<history_events::Witness>,
    entries: &[history_events::Logged],
) -> Captured {
    state.begin_observed(SIGNAL, false, w);
    if !state.full
        && let Err(reason) = state.incremental(SIGNAL, entries)
    {
        state.recover(reason);
    }
    if let Err(reason) = state.acquire_observed(f, 0, GAME_STATE, || w) {
        state.refuse(reason);
    }
    let mut batch = Captured::new();
    state.snapshot(&mut batch);
    batch
}
fn logged(facts: &[event::Fact]) -> Vec<history_events::Logged> {
    facts
        .iter()
        .enumerate()
        .map(|(order, &fact)| history_events::Logged {
            order: order as u64 + 1,
            signal: SIGNAL,
            fact,
        })
        .collect()
}
struct NoFullWalk<'a>(&'a Fixture);
impl Memory for NoFullWalk<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        assert_ne!(
            address,
            GAME_STATE + engine_native::cgame_state::COMBAT_LOGS,
            "incremental hour walked retained history"
        );
        self.0.copy(address, destination)
    }
}

#[test]
fn an_empty_attach_seed_cannot_limit_three_noncombat_writes_at_the_third_hour() {
    if !isolated() {
        return;
    }
    let producer = history_events::test_support::Producer::new();
    let mut f = Fixture::new();
    let mut state = State::new();
    let mut c = Conversion::new();
    f.noncombat_sources(1, 11_133);
    let mut capacities = crate::combat_log::Capacities::default();
    // SAFETY: test runs in an isolated process with no concurrent combat-log sizing.
    unsafe { capacities.presize(&f, 0, GAME_STATE) }.unwrap();
    assert!(producer.capacity() >= 11_136);
    for _hour in 60_759_371..60_759_374 {
        let batch = capture_public(&mut state, &f);
        assert!(
            views(&mut c, &batch)
                .iter()
                .all(|v| v.as_ref().unwrap().is_empty())
        );
    }
    assert!(
        producer.required() >= 11_137,
        "seed demand adds to noncombat writers"
    );
    let g = f.group(9);
    f.month(g, 1, &[3, 5]);
    f.managers(&[(7, vec![g])]);
    collector_test_support::with_allocations_denied(|| {
        producer.write(SIGNAL, |emit| {
            event::seed_group(&f, 0, g, 7, 0, &f.blocks[&g], emit).unwrap();
            for _ in 2..11_136 {
                event::last_month(&f, 0, g, 1, emit).unwrap();
            }
        });
    });
    assert_eq!(producer.rejected(), 0);
    assert_eq!(producer.losses(), 0);
    let batch = capture_public(&mut state, &NoFullWalk(&f));
    assert!(!batch.full);
    let rows = views(&mut c, &batch);
    assert_eq!(rows[0].as_ref().unwrap().len(), 1);
    assert_eq!(rows[1].as_ref().unwrap().len(), 1);
    let expected = capture(&mut State::new(), &f, SIGNAL, history_events::quiet());
    assert_eq!(
        encoded(&rows),
        encoded(&views(&mut Conversion::new(), &expected))
    );
    assert!(state.available);
}

#[test]
fn pending_occurrences_survive_growth_deferred_until_public_begin_drains_them() {
    if !isolated() {
        return;
    }
    let producer = history_events::test_support::Producer::new();
    let mut f = Fixture::new();
    let mut state = State::new();
    let mut c = Conversion::new();
    assert!(
        views(&mut c, &capture_public(&mut state, &f))
            .iter()
            .all(Result::is_ok)
    );
    assert_eq!(producer.capacity(), 1);
    f.noncombat_sources(1, 11_133);
    let g = f.group(9);
    f.managers(&[(7, vec![g])]);
    let mut capacities = crate::combat_log::Capacities::default();
    producer.write(SIGNAL, |emit| {
        // SAFETY: test runs in an isolated process and verifies active-writer exclusion.
        unsafe { capacities.presize(&f, 0, GAME_STATE) }.unwrap();
        assert_eq!(producer.capacity(), 1, "an active writer excludes growth");
        event::seed_group(&f, 0, g, 7, 0, &f.blocks[&g], emit).unwrap();
    });
    assert!(producer.required() >= 11_137);
    // SAFETY: test runs in an isolated process as the sole history-events sizer.
    unsafe { history_events::prepare(0) };
    assert_eq!(producer.capacity(), 1, "pending occurrences exclude growth");
    let batch = capture_public(&mut state, &NoFullWalk(&f));
    assert!(!batch.full);
    assert_eq!(batch.batch.facts.len(), 1);
    assert_eq!(batch.batch.facts[0].root, g as u64);
    assert!(producer.capacity() >= 11_137);
    assert_eq!(producer.losses(), 0);
    let actual = views(&mut c, &batch);
    let expected = capture(&mut State::new(), &f, SIGNAL, history_events::quiet());
    assert_eq!(
        encoded(&actual),
        encoded(&views(&mut Conversion::new(), &expected))
    );
}

#[test]
fn growth_and_both_drain_arenas_retry_with_production_terminal_hooks_installed() {
    if !isolated() {
        return;
    }
    let hooks = TerminalHooks::install();
    let producer = history_events::test_support::Producer::new();
    collector_test_support::with_allocations_denied(|| history_events::prepare_seed(0));
    assert_eq!(producer.capacity(), 0);
    assert_eq!(producer.losses(), 1);
    hooks.assert_live();
    // SAFETY: test runs in an isolated process as the sole history-events sizer.
    unsafe { history_events::prepare(0) };
    assert_eq!(producer.capacity(), 1);
    {
        let _denied = collector_test_support::NullGuard::new(2);
        // SAFETY: test runs in an isolated process as the sole history-events sizer.
        unsafe { history_events::prepare(8) };
    }
    assert_eq!(
        producer.capacity(),
        1,
        "failed lane boxing keeps the old allocation"
    );
    assert_eq!(producer.losses(), 2);
    hooks.assert_live();
    // SAFETY: test runs in an isolated process as the sole history-events sizer.
    unsafe { history_events::prepare(0) };
    assert!(producer.capacity() >= 8);
    let mut f = Fixture::new();
    let mut state = State::new();
    let mut c = Conversion::new();
    assert!(
        views(&mut c, &capture_public(&mut state, &f))
            .iter()
            .all(Result::is_ok)
    );
    let g = f.group(9);
    f.managers(&[(7, vec![g])]);
    producer.write(SIGNAL, |emit| {
        event::seed_group(&f, 0, g, 7, 0, &f.blocks[&g], emit).unwrap();
    });
    collector_test_support::with_allocations_denied(|| state.begin(SIGNAL, false));
    assert!(state.full);
    assert_eq!(
        state.recovery_reason,
        "shared observer lane drain allocation failed"
    );
    hooks.assert_live();
    state.acquire(&f, 0, GAME_STATE);
    let mut batch = Captured::new();
    state.snapshot(&mut batch);
    assert!(batch.full);
    let actual = views(&mut c, &batch);
    let expected = capture(&mut State::new(), &f, SIGNAL, history_events::quiet());
    assert_eq!(
        encoded(&actual),
        encoded(&views(&mut Conversion::new(), &expected))
    );
    producer.write(SIGNAL, |emit| {
        event::seed_group(&f, 0, g, 7, 0, &f.blocks[&g], emit).unwrap();
    });
    let mut output = Vec::new();
    let failed =
        collector_test_support::with_allocations_denied(|| history_events::drain(&mut output));
    assert_eq!(failed, Err("combat log event drain allocation failed"));
    assert!(output.is_empty());
    hooks.assert_live();
    history_events::drain(&mut output).unwrap();
    assert_eq!(
        output.len(),
        1,
        "failed drain preserves its pending occurrence"
    );
    assert_eq!(output[0].fact.root, g as u64);
    assert_eq!(producer.losses(), 2);
    let next = capture_public(&mut state, &NoFullWalk(&f));
    assert!(!next.full);
    assert_eq!(encoded(&views(&mut c, &next)), encoded(&actual));
    hooks.assert_live();
    hooks.prove_terminal();
}

#[test]
fn incremental_wartime_hours_read_only_changed_slots_and_never_walk_retained_groups() {
    let mut f = Fixture::new();
    let (g, manpower) = f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    assert!(
        views(&mut c, &capture(&mut state, &f, SIGNAL, witness(0, 0)))
            .iter()
            .all(Result::is_ok)
    );
    for hour in 1..=24 {
        f.int(manpower + native::MANPOWER_LOSSES, hour);
        let mut facts = Vec::new();
        event::last_manpower(&f, 0, g, &mut |fact| facts.push(fact)).unwrap();
        facts.push(event::Fact {
            kind: event::COUNT,
            root: 0,
            ..Default::default()
        });
        let batch = captured_entries(
            &mut state,
            &NoFullWalk(&f),
            witness(hour as u64, 0),
            &logged(&facts),
        );
        assert!(!batch.full);
        assert_eq!(batch.batch.counts, [(0, 2)]);
        let actual = views(&mut c, &batch);
        let expected = capture(&mut State::new(), &f, SIGNAL, witness(hour as u64, 0));
        assert_eq!(
            encoded(&actual),
            encoded(&views(&mut Conversion::new(), &expected))
        );
    }
}

#[test]
fn overflow_stays_incremental_and_a_refused_spill_recovers_by_the_first_readable_hour() {
    if !isolated() {
        return;
    }
    let hooks = TerminalHooks::install();
    {
        let producer = history_events::test_support::Producer::new();
        let mut f = Fixture::new();
        let mut state = State::new();
        let mut c = Conversion::new();
        assert!(
            views(&mut c, &capture_public(&mut state, &f))
                .iter()
                .all(Result::is_ok)
        );
        assert_eq!(producer.capacity(), 1);
        let g = f.group(9);
        f.manpower(g, &[[i32::MIN, 0, i32::MAX], [1, 2, 3], [4, 5, 6]]);
        f.month(g, 0, &[]);
        f.month(g, 5, &[i64::MIN, 0, i64::MAX]);
        f.managers(&[(7, vec![g])]);
        let mut facts = Vec::new();
        event::seed_group(&f, 0, g, 7, 0, &f.blocks[&g], &mut |fact| facts.push(fact)).unwrap();
        assert!(facts.len() > 1);
        producer.write(SIGNAL, |emit| {
            event::seed_group(&f, 0, g, 7, 0, &f.blocks[&g], emit).unwrap();
        });
        assert_eq!(producer.rejected(), 0);
        assert_eq!(producer.losses(), 0);
        assert!(producer.required() >= facts.len());
        let batch = capture_public(&mut state, &NoFullWalk(&f));
        assert!(!batch.full, "a burst past the lane stays incremental");
        assert!(producer.capacity() >= facts.len());
        let expected = capture(&mut State::new(), &f, SIGNAL, history_events::quiet());
        assert_eq!(
            encoded(&views(&mut c, &batch)),
            encoded(&views(&mut Conversion::new(), &expected))
        );
        hooks.assert_live();
    }
    for unreadable_this_hour in [false, true] {
        let producer = history_events::test_support::Producer::new();
        let mut f = Fixture::new();
        let mut state = State::new();
        let mut c = Conversion::new();
        assert!(
            views(&mut c, &capture_public(&mut state, &f))
                .iter()
                .all(Result::is_ok)
        );
        assert_eq!(producer.capacity(), 1);
        let (g, p) = f.complete();
        f.int(p + native::MANPOWER_LOSSES, 77);
        let mut facts = Vec::new();
        event::seed_group(&f, 0, g, 7, 0, &f.blocks[&g], &mut |fact| facts.push(fact)).unwrap();
        collector_test_support::with_allocations_denied(|| {
            producer.write(SIGNAL, |emit| {
                event::seed_group(&f, 0, g, 7, 0, &f.blocks[&g], emit).unwrap();
            });
        });
        assert_eq!(producer.rejected(), facts.len() as u64 - 1);
        assert_eq!(producer.losses(), producer.rejected());
        let status = serde_json::to_value(crate::history_events::refusals()).unwrap();
        assert_eq!(status["first_loss"], "combat log event allocation failed");
        assert_eq!(status["lost"], producer.losses());
        // SAFETY: test runs in an isolated process as the sole history-events sizer.
        unsafe { history_events::prepare(0) };
        assert_eq!(
            producer.capacity(),
            1,
            "a refused spill cannot replace a pending entry"
        );
        state.begin(SIGNAL, false);
        assert!(
            state.full,
            "emit's loss notification revokes incremental capture"
        );
        assert_eq!(state.recovery_reason, "combat log writer occurrence loss");
        assert_eq!(state.drained.len(), 1);
        assert_eq!(state.drained[0].fact, facts[0]);
        assert!(producer.capacity() >= facts.len());
        if unreadable_this_hour {
            f.unreadable = Some(p);
        }
        state.acquire(&f, 0, GAME_STATE);
        let mut batch = Captured::new();
        state.snapshot(&mut batch);
        assert!(batch.full);
        if unreadable_this_hour {
            assert!(
                batch
                    .outcomes()
                    .iter()
                    .all(|o| matches!(o, Outcome::Unavailable { .. }))
            );
            f.unreadable = None;
            batch = capture_public(&mut state, &f);
            assert!(batch.full);
        }
        let actual = views(&mut c, &batch);
        let expected = capture(&mut State::new(), &f, SIGNAL, history_events::quiet());
        assert_eq!(
            encoded(&actual),
            encoded(&views(&mut Conversion::new(), &expected))
        );
        let lost = producer.losses();
        f.int(p + native::MANPOWER_LOSSES, 88);
        producer.write(SIGNAL, |emit| {
            event::last_manpower(&f, 0, g, emit).unwrap();
        });
        let next = capture_public(&mut state, &NoFullWalk(&f));
        assert!(!next.full);
        assert_eq!(producer.losses(), lost);
        let expected = capture(&mut State::new(), &f, SIGNAL, history_events::quiet());
        assert_eq!(
            encoded(&views(&mut c, &next)),
            encoded(&views(&mut Conversion::new(), &expected))
        );
        hooks.assert_live();
    }
    hooks.prove_terminal();
}

#[test]
fn fifteen_hundred_countries_publish_incremental_occurrences_without_full_walks() {
    let mut f = Fixture::new();
    let mut state = State::new();
    let mut c = Conversion::new();
    assert!(
        views(&mut c, &capture(&mut state, &f, SIGNAL, witness(0, 0)))
            .iter()
            .all(Result::is_ok)
    );
    let mut countries = Vec::new();
    let mut facts = Vec::new();
    for country in 1..=1_500 {
        let g = f.group(country);
        f.manpower(g, &[[i32::MIN, 0, i32::MAX]]);
        event::seed_group(&f, 0, g, country, 0, &f.blocks[&g], &mut |fact| {
            facts.push(fact)
        })
        .unwrap();
        countries.push((country, vec![g]));
    }
    f.managers(&countries);
    let batch = captured_entries(
        &mut state,
        &NoFullWalk(&f),
        witness(3_000, 0),
        &logged(&facts),
    );
    assert!(!batch.full);
    let actual = views(&mut c, &batch);
    assert_eq!(actual[0].as_ref().unwrap().len(), 1_500);
    assert_eq!(actual[2].as_ref().unwrap().len(), 1_500);
    let expected = capture(&mut State::new(), &f, SIGNAL, witness(3_000, 0));
    assert_eq!(
        encoded(&actual),
        encoded(&views(&mut Conversion::new(), &expected))
    );
}

#[test]
fn native_demand_grows_on_the_worker_across_fifteen_hundred_countries() {
    let mut f = Fixture::new();
    let mut countries = Vec::new();
    let mut state = State::new();
    assert_eq!(capture(&mut state, &f, SIGNAL, witness(0, 0)).failure, None);
    for country in 1..=1_500 {
        let g = f.group(country);
        let count = 11_136 / 1_500 + usize::from(country <= 11_136 % 1_500);
        f.manpower(g, &vec![[i32::MIN, 0, i32::MAX]; count]);
        countries.push((country, vec![g]));
    }
    f.managers(&countries);
    let batch = capture(&mut state, &f, SIGNAL, witness(11_136, 1));
    assert_eq!(batch.failure, None);
    assert_eq!(batch.batch.facts.len(), 1_500 + 11_136);
    let mut c = Conversion::new();
    let rows = views(&mut c, &batch);
    assert_eq!(rows[0].as_ref().unwrap().len(), 1_500);
    assert_eq!(rows[2].as_ref().unwrap().len(), 11_136);
}

#[test]
fn loss_rebuilds_every_retained_table_at_the_same_hour_without_changing_native_values() {
    let mut f = Fixture::new();
    f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    let before = views(&mut c, &capture(&mut state, &f, SIGNAL, witness(1, 0)));
    assert!(before.iter().all(|v| !v.as_ref().unwrap().is_empty()));
    let batch = capture(&mut state, &f, SIGNAL, witness(1, 2));
    assert!(batch.full);
    let after = views(&mut c, &batch);
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(old.as_ref().unwrap(), new.as_ref().unwrap());
    }
    let TableRows::CombatLogManpower(manpower) = after[2].as_ref().unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(
        (
            manpower[0].losses_0,
            manpower[0].losses_1,
            manpower[0].losses_2
        ),
        (i32::MIN, 0, i32::MAX)
    );
    let TableRows::CombatLogTemplates(templates) = after[3].as_ref().unwrap().as_ref() else {
        panic!()
    };
    assert_eq!((templates[0].wins, templates[0].total), (-1, 5));
    let TableRows::CombatLogReferences(references) = after[4].as_ref().unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(references.len(), 2);
    assert_eq!(references[0].battle, references[1].battle);
    assert_ne!(references[0].attacker, references[1].attacker);
    let TableRows::CombatLogBattles(battles) = after[5].as_ref().unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(battles[0].flags, [2, 0, 1, 255]);
    let TableRows::CombatLogBattleEquipment(equipment) = after[8].as_ref().unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(
        equipment.iter().map(|r| r.amount_raw).collect::<Vec<_>>(),
        [i64::MIN, 0, i64::MAX, i64::MIN, 0, i64::MAX]
    );
}

#[test]
fn a_failed_recovery_stays_unavailable_and_retries_at_the_first_readable_hour() {
    let mut f = Fixture::new();
    let (_, p) = f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    assert!(
        views(&mut c, &capture(&mut state, &f, SIGNAL, witness(1, 0)))
            .iter()
            .all(Result::is_ok)
    );
    f.unreadable = Some(p + native::MANPOWER_LOSSES);
    let failed = capture(&mut state, &f, SIGNAL, witness(2, 2));
    assert!(
        failed
            .outcomes()
            .iter()
            .all(|o| matches!(o, Outcome::Unavailable { .. }))
    );
    assert!(!state.available);
    f.unreadable = None;
    let recovered = capture(&mut state, &f, SIGNAL, witness(2, 2));
    assert!(recovered.full);
    assert!(views(&mut c, &recovered).iter().all(|v| v.is_ok()));
}

#[test]
fn prune_reset_and_load_replace_old_groups_and_reused_battle_slots_exactly() {
    let mut f = Fixture::new();
    let (old, _) = f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    assert!(
        views(&mut c, &capture(&mut state, &f, SIGNAL, witness(1, 0)))
            .iter()
            .all(Result::is_ok)
    );
    f.month(old, 5, &[]);
    f.references(old, &[0]);
    let reused = f.battle(1);
    f.int(reused + native::BATTLE_DATE, DATE + 1);
    f.int(reused + native::BATTLE_PROVINCE, 99);
    let batch = capture(&mut state, &f, SIGNAL, witness(5, 1));
    let rows = views(&mut c, &batch);
    assert_eq!(rows[4].as_ref().unwrap().len(), 1);
    assert!(rows[6].as_ref().unwrap().is_empty());
    let TableRows::CombatLogBattles(battles) = rows[5].as_ref().unwrap().as_ref() else {
        panic!()
    };
    assert_eq!(battles[0].reference_count, 1);
    assert_eq!((battles[0].date, battles[0].province), (DATE + 1, 99));
    f.managers(&[]);
    for at in [
        native::BATTLE_POOL_RVA.0,
        native::BATTLE_COUNTS_RVA.0,
        native::BATTLE_USED_RVA.0,
    ] {
        f.int(at + 12, 0);
    }
    assert!(
        views(&mut c, &capture(&mut state, &f, SIGNAL, witness(6, 2)))
            .iter()
            .all(|v| v.as_ref().unwrap().is_empty())
    );
    let replacement = old;
    f.put(replacement, &vec![0; group::SIZE]);
    f.word(replacement, group::VTABLE_RVA.0 as u64);
    f.word(
        replacement + group::GROUP,
        (19 << 32) | u64::from(engine_native::corders_group::REFERENCE_TYPE),
    );
    f.manpower(replacement, &[[0, -1, 0]]);
    f.managers(&[(1_500, vec![replacement])]);
    let loaded = capture(&mut state, &f, SIGNAL + 1, witness(6, 2));
    assert!(loaded.full);
    let rows = views(&mut c, &loaded);
    assert_eq!(rows[0].as_ref().unwrap().len(), 1);
    assert!(rows[4].as_ref().unwrap().is_empty());
    assert!(rows[5].as_ref().unwrap().is_empty());
    assert!(rows[6].as_ref().unwrap().is_empty());
}

#[test]
fn quiet_hours_reuse_owned_rows_without_native_reads_or_allocations() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("quiet native read")
        }
    }
    let mut f = Fixture::new();
    f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    let before = views(&mut c, &capture(&mut state, &f, SIGNAL, witness(1, 0)));
    state.begin_observed(SIGNAL, false, witness(1, 0));
    assert!(!state.full);
    collector_test_support::with_allocations_denied(|| {
        state
            .acquire_observed(&NoReads, 0, GAME_STATE, || witness(1, 0))
            .unwrap()
    });
    let mut batch = Captured::new();
    state.snapshot(&mut batch);
    assert!(batch.batch.facts.is_empty());
    let after = views(&mut c, &batch);
    for (old, new) in before.iter().zip(&after) {
        assert!(Arc::ptr_eq(old.as_ref().unwrap(), new.as_ref().unwrap()));
    }
}

#[test]
fn missing_or_changed_writer_witness_refuses_instead_of_publishing_a_partial_baseline() {
    let mut f = Fixture::new();
    f.complete();
    let mut state = State::new();
    let batch = capture(&mut state, &f, SIGNAL, None);
    assert!(
        batch
            .outcomes()
            .iter()
            .all(|o| matches!(o, Outcome::Unavailable { .. }))
    );
    state.begin_observed(SIGNAL, false, witness(1, 0));
    let observations = Cell::new(0);
    let result = state.acquire_observed(&f, 0, GAME_STATE, || {
        observations.set(observations.get() + 1);
        witness(observations.get(), 0)
    });
    state.refuse(result.unwrap_err());
    let mut failed = Captured::new();
    state.snapshot(&mut failed);
    assert!(
        failed
            .outcomes()
            .iter()
            .all(|o| matches!(o, Outcome::Unavailable { .. }))
    );
    let recovered = capture(&mut state, &f, SIGNAL, witness(2, 0));
    assert_eq!(recovered.failure, None);
    assert!(recovered.full);
}

#[test]
fn a_writer_that_settles_before_acquisition_rebases_in_the_same_hour() {
    let mut f = Fixture::new();
    let mut state = State::new();
    capture(&mut state, &f, SIGNAL, witness(0, 0));
    state.begin_observed(SIGNAL, false, None);
    f.complete();
    state
        .acquire_observed(&f, 0, GAME_STATE, || witness(3, 0))
        .unwrap();
    assert!(state.full);
    assert!(state.available);
    assert_eq!(state.certificate, witness(3, 0));
}

#[test]
fn a_skipped_conversion_requests_a_fresh_native_baseline_even_without_another_writer() {
    let mut f = Fixture::new();
    f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    let retry = AtomicBool::new(false);
    let first = capture(&mut state, &f, SIGNAL, witness(1, 0));
    let expected = first.convert(&mut c, &retry);
    capture(&mut state, &f, SIGNAL, witness(1, 0));
    let skipped = capture(&mut state, &f, SIGNAL, witness(1, 0));
    assert!(skipped.convert(&mut c, &retry).iter().all(Result::is_err));
    assert!(retry.load(Ordering::Acquire));
    state.begin_observed(SIGNAL, retry.swap(false, Ordering::AcqRel), witness(1, 0));
    state
        .acquire_observed(&f, 0, GAME_STATE, || witness(1, 0))
        .unwrap();
    let mut recovered = Captured::new();
    state.snapshot(&mut recovered);
    assert!(recovered.full);
    let actual = recovered.convert(&mut c, &retry);
    for (old, new) in expected.iter().zip(&actual) {
        assert_eq!(old.as_ref().unwrap(), new.as_ref().unwrap());
    }
    assert!(!retry.load(Ordering::Acquire));
}

#[test]
fn failed_worker_growth_never_publishes_empty_rows_and_retries_from_native_state() {
    if !isolated() {
        return;
    }
    let hooks = TerminalHooks::install();
    let mut f = Fixture::new();
    let g = f.group(9);
    f.manpower(g, &[[i32::MIN, 0, i32::MAX]]);
    f.managers(&[(7, vec![g])]);
    let mut state = State::new();
    state.begin_observed(SIGNAL, false, witness(1, 0));
    let failed = collector_test_support::with_allocations_denied(|| {
        state.acquire_observed(&f, 0, GAME_STATE, || witness(1, 0))
    });
    state.refuse(failed.unwrap_err());
    let mut unavailable = Captured::new();
    state.snapshot(&mut unavailable);
    assert!(
        unavailable
            .outcomes()
            .iter()
            .all(|o| matches!(o, Outcome::Unavailable { .. }))
    );
    let recovered = capture(&mut state, &f, SIGNAL, witness(1, 0));
    let mut c = Conversion::new();
    let rows = views(&mut c, &recovered);
    assert_eq!(rows[0].as_ref().unwrap().len(), 1);
    assert_eq!(rows[2].as_ref().unwrap().len(), 1);
    hooks.assert_live();
    hooks.prove_terminal();
}

#[test]
fn retained_history_recovers_independently_of_unreadable_own_equipment_payloads() {
    let mut f = Fixture::new();
    let (g, _) = f.complete();
    f.month(g, 0, &[17]);
    let months = engine_native::cpdx_array::Array::read(&f, g + group::REASONS, 8).unwrap();
    let month = engine_native::read::<u64>(&f, months.data).unwrap() as usize;
    let equipment = engine_native::cpdx_array::Array::read(
        &f,
        month + loss::POOL + pool::ITEMS,
        equipment_entry::STRIDE,
    )
    .unwrap();
    f.unreadable = Some(equipment.data);
    let mut memo = manager::VariantMemo::default();
    let own = manager::walk(
        &f,
        0,
        GAME_STATE,
        |_| Some(manager::Months::All),
        &mut vec![manager::Row::default()],
        &mut memo,
    );
    assert!(matches!(own, Outcome::Unavailable { .. }));
    let mut state = State::new();
    let recovered = capture(&mut state, &f, SIGNAL, witness(3, 2));
    let mut c = Conversion::new();
    assert!(
        views(&mut c, &recovered)
            .iter()
            .all(|v| !v.as_ref().unwrap().is_empty())
    );
}

#[test]
fn reused_slots_and_refused_incremental_allocation_rebuild_in_the_same_hour() {
    if !isolated() {
        return;
    }
    let hooks = TerminalHooks::install();
    let mut f = Fixture::new();
    let (g, p) = f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    assert!(
        views(&mut c, &capture(&mut state, &f, SIGNAL, witness(0, 0)))
            .iter()
            .all(Result::is_ok)
    );
    let reused = f.battle(2);
    f.int(reused + native::BATTLE_PROVINCE, 99);
    let entries = logged(&[event::Fact {
        kind: event::SLOT,
        root: 1234,
        item: 0,
        ..Default::default()
    }]);
    let batch = captured_entries(&mut state, &f, witness(1, 0), &entries);
    assert!(batch.full);
    let actual = views(&mut c, &batch);
    let expected = capture(&mut State::new(), &f, SIGNAL, witness(1, 0));
    assert_eq!(
        encoded(&actual),
        encoded(&views(&mut Conversion::new(), &expected))
    );
    f.int(p + native::MANPOWER_LOSSES, 123);
    let mut facts = Vec::new();
    event::last_manpower(&f, 0, g, &mut |fact| facts.push(fact)).unwrap();
    state.batch.facts = Vec::new();
    state.begin_observed(SIGNAL, false, witness(2, 0));
    let entries = logged(&facts);
    let error =
        collector_test_support::with_allocations_denied(|| state.incremental(SIGNAL, &entries))
            .unwrap_err();
    state.recover(error);
    state
        .acquire_observed(&f, 0, GAME_STATE, || witness(2, 0))
        .unwrap();
    let mut batch = Captured::new();
    state.snapshot(&mut batch);
    assert!(batch.full);
    let actual = views(&mut c, &batch);
    let expected = capture(&mut State::new(), &f, SIGNAL, witness(2, 0));
    assert_eq!(
        encoded(&actual),
        encoded(&views(&mut Conversion::new(), &expected))
    );
    hooks.assert_live();
    hooks.prove_terminal();
}

#[test]
fn noncombat_sizing_counts_all_fifteen_hundred_countries_without_combat_or_history() {
    if !isolated() {
        return;
    }
    let producer = history_events::test_support::Producer::new();
    let mut f = Fixture::new();
    f.noncombat_sources(1_500, 7);
    assert_eq!(
        event::noncombat_writer_demand(&f, 0, GAME_STATE),
        Ok(15_000)
    );
    // SAFETY: test runs in an isolated process with no concurrent combat-log sizing.
    unsafe { crate::combat_log::Capacities::default().presize(&f, 0, GAME_STATE) }.unwrap();
    assert!(producer.capacity() >= 15_000);
    let mut state = State::new();
    let batch = capture_public(&mut state, &f);
    assert!(
        views(&mut Conversion::new(), &batch)
            .iter()
            .all(|view| view.as_ref().unwrap().is_empty())
    );
}

#[test]
fn witnessed_release_and_slot_reuse_stay_incremental_and_byte_identical() {
    let mut f = Fixture::new();
    let (g, _) = f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    assert!(
        views(&mut c, &capture(&mut state, &f, SIGNAL, witness(0, 0)))
            .iter()
            .all(Result::is_ok)
    );
    let p = f.battle(2);
    f.int(p + native::BATTLE_PROVINCE, 99);
    let mut facts = vec![
        event::Fact {
            kind: event::PREFIX_REMOVED,
            root: g as u64,
            item: 8,
            values: [2, 0, 0, 0, 0],
        },
        event::Fact {
            kind: event::RELEASE,
            root: 0,
            ..Default::default()
        },
    ];
    event::battle(&f, 0, p, 1234, &mut |fact| facts.push(fact)).unwrap();
    facts.push(event::Fact {
        kind: event::SLOT,
        root: 1234,
        item: 0,
        ..Default::default()
    });
    facts.push(event::Fact {
        kind: event::REFERENCE,
        root: g as u64,
        item: 0,
        values: [0, 1, 0, 0, 0],
    });
    facts.push(event::Fact {
        kind: event::REFERENCE,
        root: g as u64,
        item: 1,
        values: [0, 0, 0, 0, 0],
    });
    facts.push(event::Fact {
        kind: event::COUNT,
        root: 0,
        ..Default::default()
    });
    let batch = captured_entries(&mut state, &NoFullWalk(&f), witness(1, 0), &logged(&facts));
    assert!(!batch.full);
    let actual = views(&mut c, &batch);
    let expected = capture(&mut State::new(), &f, SIGNAL, witness(1, 0));
    assert_eq!(
        encoded(&actual),
        encoded(&views(&mut Conversion::new(), &expected))
    );
}

#[test]
fn a_failed_row_materialization_requests_exact_recovery_before_the_next_incremental_hour() {
    if !isolated() {
        return;
    }
    let hooks = TerminalHooks::install();
    let mut f = Fixture::new();
    f.complete();
    let mut state = State::new();
    let first = capture(&mut state, &f, SIGNAL, witness(0, 0));
    let mut c = Conversion::new();
    c.apply(&first).unwrap();
    first.converted.set([0; N]).unwrap();
    let retry = AtomicBool::new(false);
    let failed = collector_test_support::with_allocations_denied(|| first.convert(&mut c, &retry));
    assert!(failed.iter().any(Result::is_err));
    assert!(retry.load(Ordering::Acquire));
    state.begin_observed(SIGNAL, retry.swap(false, Ordering::AcqRel), witness(0, 0));
    state
        .acquire_observed(&f, 0, GAME_STATE, || witness(0, 0))
        .unwrap();
    let mut batch = Captured::new();
    state.snapshot(&mut batch);
    assert!(batch.full);
    let actual = batch.convert(&mut c, &retry);
    let expected = capture(&mut State::new(), &f, SIGNAL, witness(0, 0));
    assert_eq!(
        encoded(&actual),
        encoded(&views(&mut Conversion::new(), &expected))
    );
    hooks.assert_live();
    hooks.prove_terminal();
}

#[test]
fn an_incremental_lifetime_read_failure_recovers_at_the_same_quiet_boundary() {
    struct OnceUnreadable<'a> {
        fixture: &'a Fixture,
        fail: Cell<bool>,
    }
    impl Memory for OnceUnreadable<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == native::BATTLE_COUNTS_RVA.0 && self.fail.replace(false) {
                return false;
            }
            self.fixture.copy(address, destination)
        }
    }
    let mut f = Fixture::new();
    f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    let initial = capture(&mut state, &f, SIGNAL, witness(0, 0));
    let expected = encoded(&views(&mut c, &initial));
    let memory = OnceUnreadable {
        fixture: &f,
        fail: Cell::new(true),
    };
    let entries = logged(&[event::Fact {
        kind: event::COUNT,
        root: 0,
        ..Default::default()
    }]);
    let batch = captured_entries(&mut state, &memory, witness(1, 0), &entries);
    assert!(batch.full);
    assert_eq!(expected, encoded(&views(&mut c, &batch)));
}

#[test]
fn a_reference_count_update_alone_cannot_authenticate_slot_reuse() {
    let mut f = Fixture::new();
    f.complete();
    let mut state = State::new();
    let mut c = Conversion::new();
    let initial = capture(&mut state, &f, SIGNAL, witness(0, 0));
    let expected = encoded(&views(&mut c, &initial));
    let entries = logged(&[
        event::Fact {
            kind: event::COUNT,
            root: 0,
            ..Default::default()
        },
        event::Fact {
            kind: event::SLOT,
            root: 1234,
            item: 0,
            ..Default::default()
        },
    ]);
    let batch = captured_entries(&mut state, &f, witness(1, 0), &entries);
    assert!(batch.full);
    assert_eq!(expected, encoded(&views(&mut c, &batch)));
}

#[test]
fn an_exact_full_baseline_survives_failure_to_allocate_its_optional_lifetime_cache() {
    if !isolated() {
        return;
    }
    let hooks = TerminalHooks::install();
    let mut f = Fixture::new();
    let g = f.group(9);
    f.references(g, &[1]);
    let battle = f.battle(1);
    for offset in [native::BATTLE_ATTACKER, native::BATTLE_DEFENDER] {
        let side = battle + offset;
        for pool_offset in [
            native::SIDE_EQUIPMENT,
            native::SIDE_CAPTURED_BY_ENEMY,
            native::SIDE_RECOVERED,
        ] {
            f.array(side + pool_offset + pool::ALLOCATIONS, 0, 0);
            f.array(side + pool_offset + pool::ITEMS, 0, 0);
        }
        f.array(side + native::SIDE_MODIFIERS, 0, 0);
        f.array(side + native::SIDE_COUNTRIES, 0, 0);
    }
    f.managers(&[(7, vec![g])]);
    let mut state = State::new();
    let mut c = Conversion::new();
    let mut first = capture(&mut state, &f, SIGNAL, witness(0, 0));
    let expected = encoded(&views(&mut c, &first));
    state.batch = std::mem::take(&mut first.batch);
    state.slots = Vec::new();
    state.begin_observed(SIGNAL, true, witness(1, 0));
    collector_test_support::with_allocations_denied(|| {
        state.acquire_observed(&f, 0, GAME_STATE, || witness(1, 0))
    })
    .unwrap();
    hooks.assert_live();
    assert!(state.available);
    assert!(state.slots.is_empty());
    assert_eq!(state.certificate, None);
    let mut batch = Captured::new();
    state.snapshot(&mut batch);
    assert_eq!(expected, encoded(&views(&mut c, &batch)));
    let next = capture(&mut state, &f, SIGNAL, witness(1, 0));
    assert!(next.full);
    assert_eq!(state.certificate, witness(1, 0));
    assert_eq!(expected, encoded(&views(&mut c, &next)));
    hooks.assert_live();
    hooks.prove_terminal();
}
