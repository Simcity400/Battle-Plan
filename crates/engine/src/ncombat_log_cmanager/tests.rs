use super::*;
use crate::{creference_registry as registry, reference_bucket as bucket};
use std::collections::BTreeMap;

const BASE: usize = 0x0001_4000_0000;
const GAME: usize = 0x10000;
const MANAGERS: usize = 0x11000;
const VARIANTS: usize = 0x80_0000;
/// 1943.10.27.13 and 1943.11.1.4 (raw game hours).
const OCTOBER: i32 = 60_827_868;
const NOVEMBER: i32 = 60_827_979;

/// Addresses at or above this (the module's) live in the sparse overlay.
const HIGH: usize = 1 << 32;
/// The orders-group reference table and its buckets.
const REGISTRY: usize = 0x20000;
const BUCKETS: usize = 0x21000;

/// Flat synthetic memory from address zero, and a sparse overlay for module
/// globals; only written bytes are readable.
#[derive(Default)]
struct Fixture {
    bytes: Vec<u8>,
    present: Vec<bool>,
    high: BTreeMap<usize, u8>,
    next: usize,
}

impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let Some(end) = address.checked_add(destination.len()) else {
            return false;
        };
        if address >= HIGH {
            for (offset, byte) in destination.iter_mut().enumerate() {
                let Some(value) = self.high.get(&(address + offset)) else {
                    return false;
                };
                *byte = *value;
            }
            return true;
        }
        if end > self.bytes.len() || !self.present[address..end].iter().all(|&p| p) {
            return false;
        }
        destination.copy_from_slice(&self.bytes[address..end]);
        true
    }
}

/// One month entry: reason, date and (variant id, amount) pairs.
struct Entry<'a> {
    reason: i32,
    date: i32,
    pool: &'a [(u32, i64)],
}

impl Fixture {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        if address >= HIGH {
            self.high
                .extend(bytes.iter().enumerate().map(|(i, &b)| (address + i, b)));
            return;
        }
        let end = address + bytes.len();
        if end > self.bytes.len() {
            self.bytes.resize(end, 0);
            self.present.resize(end, false);
        }
        self.bytes[address..end].copy_from_slice(bytes);
        self.present[address..end].fill(true);
    }
    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    fn pointer(&mut self, address: usize, value: usize) {
        self.put(address, &(value as u64).to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.pointer(address, data);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }
    fn forget(&mut self, address: usize, len: usize) {
        self.present[address..address + len].fill(false);
    }
    /// A fresh zeroed region for one object: like a real allocation, every
    /// byte of it is readable, including fields the fixture does not set.
    fn alloc(&mut self, len: usize) -> usize {
        let address = 0x100_0000 + self.next;
        self.next += (len + 0xff) & !0xff;
        self.put(address, &vec![0; len]);
        address
    }
    /// The orders-group reference table (type 53) resolving exactly `groups`,
    /// with Robin Hood placement as the engine's lookup expects.
    fn registry(&mut self, groups: &[u32]) {
        let capacity = (groups.len() * 2).next_power_of_two().max(8) as u32;
        let exponent = capacity.trailing_zeros();
        let overflow = exponent + 2;
        let terminal = capacity + overflow;
        let root = BASE + registry::LOW_ROOTS_RVA + corders_group::REFERENCE_TYPE as usize * 8;
        self.pointer(root, REGISTRY);
        self.pointer(REGISTRY + registry::ENTRIES, BUCKETS);
        self.u32(REGISTRY + registry::COUNT, groups.len() as u32);
        self.u32(REGISTRY + registry::MASK, capacity - 1);
        self.put(REGISTRY + registry::OVERFLOW_DISTANCE, &[overflow as u8]);
        let mut cells: Vec<Option<(u8, u64)>> = vec![None; terminal as usize];
        for &id in groups {
            let handle = u64::from(id) << 32 | u64::from(corders_group::REFERENCE_TYPE);
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let mut index = (((hash >> 32) as u32 ^ hash as u32) & (capacity - 1)) as usize;
            let mut entry = (1_u8, handle);
            loop {
                assert!(u32::from(entry.0) <= overflow + 1 && index < terminal as usize);
                match cells[index] {
                    None => {
                        cells[index] = Some(entry);
                        break;
                    }
                    Some(held) if held.0 < entry.0 => {
                        cells[index] = Some(entry);
                        entry = held;
                    }
                    Some(_) => {}
                }
                entry.0 += 1;
                index += 1;
            }
        }
        self.put(BUCKETS, &vec![0; (terminal as usize + 1) * bucket::STRIDE]);
        for (index, cell) in cells.iter().enumerate() {
            if let Some((distance, handle)) = cell {
                let at = BUCKETS + index * bucket::STRIDE;
                self.put(at + bucket::DISTANCE, &[*distance]);
                self.put(at + bucket::TYPE, &handle.to_le_bytes());
                self.pointer(at + bucket::OBJECT, 0x0dd0_0000 + index);
            }
        }
        self.put(
            BUCKETS + terminal as usize * bucket::STRIDE + bucket::DISTANCE,
            &[bucket::TERMINAL_DISTANCE],
        );
    }
    fn variant(&mut self, id: u32) -> usize {
        let variant = VARIANTS + id as usize * 0x100;
        self.pointer(variant, BASE + cequipment_variant::VTABLE_RVA);
        self.u32(variant + 8, cequipment_variant::REFERENCE_TYPE);
        self.u32(variant + 12, id);
        variant
    }

    /// Slots: 0 null (sentinel), 1 a manager with `groups`, 2 null, 3 an
    /// empty manager.
    fn new(groups: &[(u32, [&[Entry]; 5])]) -> (Self, usize) {
        let mut f = Self::default();
        f.array(GAME + cgame_state::COMBAT_LOGS, MANAGERS, 4);
        f.pointer(MANAGERS, 0);
        f.pointer(MANAGERS + 16, 0);
        let empty = f.alloc(SIZE);
        f.pointer(MANAGERS + 24, empty);
        f.pointer(empty, BASE + VTABLE_RVA);
        f.array(empty + GROUPS, 0, 0);
        f.u32(empty + COUNTRY, 3);
        let manager = f.manager(7, groups);
        f.pointer(MANAGERS + 8, manager);
        (f, manager)
    }

    /// A manager of country `country` holding `groups`.
    fn manager(&mut self, country: u32, groups: &[(u32, [&[Entry]; 5])]) -> usize {
        let manager = self.alloc(SIZE);
        self.pointer(manager, BASE + VTABLE_RVA);
        self.u32(manager + COUNTRY, country);
        let list = self.alloc(groups.len() * 8);
        self.array(manager + GROUPS, list, groups.len() as u32);
        for (index, (id, kinds)) in groups.iter().enumerate() {
            let group = self.alloc(logs::SIZE);
            self.pointer(list + index * 8, group);
            self.pointer(group, BASE + logs::VTABLE_RVA);
            self.u32(group + logs::GROUP, corders_group::REFERENCE_TYPE);
            self.u32(group + logs::GROUP + 4, *id);
            self.array(group + logs::ENEMY_LOSSES, 0xdead_0000, 0);
            for (kind, entries) in kinds.iter().enumerate() {
                let header = if kind < logs::REASON_COUNT {
                    group + logs::REASONS + kind * logs::REASON_STRIDE
                } else {
                    group + logs::RECOVERED
                };
                let data = self.alloc(entries.len() * 8);
                self.array(header, data, entries.len() as u32);
                for (position, entry) in entries.iter().enumerate() {
                    let object = self.alloc(loss::SIZE);
                    self.pointer(data + position * 8, object);
                    self.pointer(object, BASE + loss::VTABLE_RVA);
                    self.put(object + loss::REASON, &entry.reason.to_le_bytes());
                    self.put(object + loss::DATE, &entry.date.to_le_bytes());
                    self.pointer(object + loss::POOL, BASE + pool::VTABLE_RVA);
                    let items = self.alloc(entry.pool.len() * 16);
                    self.array(
                        object + loss::POOL + pool::ITEMS,
                        items,
                        entry.pool.len() as u32,
                    );
                    for (slot, &(variant, amount)) in entry.pool.iter().enumerate() {
                        let pointer = self.variant(variant);
                        self.pointer(items + slot * 16, pointer);
                        self.put(items + slot * 16 + 8, &amount.to_le_bytes());
                    }
                }
            }
        }
        manager
    }

    /// `slots` country slots (slot 0 the null sentinel); every `every`-th
    /// slot's manager holds `groups` group logs, each with `months` month
    /// entries of `variants` variants in reasons 0, 1 and 3 and the
    /// recovered array; the rest are empty managers.
    fn scale(slots: usize, every: usize, groups: u32, months: i32, variants: u32) -> Self {
        let mut f = Self::default();
        let array = f.alloc(slots * 8);
        f.array(GAME + cgame_state::COMBAT_LOGS, array, slots as u32);
        f.pointer(array, 0);
        let pool: Vec<(u32, i64)> = (0..variants)
            .map(|v| (v + 1, i64::from(v) * 100_000))
            .collect();
        let entries = |reason| -> Vec<Entry> {
            (0..months)
                .map(|month| Entry {
                    reason,
                    date: OCTOBER + month * 24 * 31,
                    pool: &pool,
                })
                .collect()
        };
        let (combat, other, training, recovered) = (entries(0), entries(1), entries(3), entries(0));
        let logs: Vec<(u32, [&[Entry]; 5])> = (0..groups)
            .map(|id| {
                (
                    id + 1,
                    [&combat[..], &other[..], NONE, &training[..], &recovered[..]],
                )
            })
            .collect();
        for slot in 1..slots {
            let manager = if slot % every == 0 {
                f.manager(slot as u32, &logs)
            } else {
                f.manager(slot as u32, &[])
            };
            f.pointer(array + slot * 8, manager);
        }
        f
    }
}

const NONE: &[Entry] = &[];

fn standard() -> (Fixture, usize) {
    Fixture::new(&[
        (
            12793,
            [
                &[
                    Entry {
                        reason: 0,
                        date: OCTOBER,
                        pool: &[(1326, 300_000), (1327, 100_000)],
                    },
                    Entry {
                        reason: 0,
                        date: NOVEMBER,
                        pool: &[(1326, 2_600_000)],
                    },
                ],
                &[Entry {
                    reason: 1,
                    date: NOVEMBER,
                    pool: &[(1329, 900_000)],
                }],
                NONE,
                &[Entry {
                    reason: 3,
                    date: OCTOBER,
                    pool: &[(5215, 200_000)],
                }],
                &[Entry {
                    reason: 0,
                    date: OCTOBER,
                    pool: &[(1846, 100_000)],
                }],
            ],
        ),
        (
            12794,
            [
                NONE,
                &[Entry {
                    reason: 1,
                    date: OCTOBER,
                    pool: &[],
                }],
                NONE,
                NONE,
                NONE,
            ],
        ),
    ])
}

fn row(group: u32, kind: u8, date: i32, variant: u32, amount_raw: i64) -> Row {
    Row {
        country: 7,
        group,
        kind,
        month: loss::month_bucket(date) as u16,
        variant,
        date,
        amount_raw,
    }
}

#[test]
fn reads_every_group_month_and_variant_in_native_order() {
    let (f, _) = standard();
    let mut rows = [Row::default(); 16];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::new()
        ),
        Outcome::Rows { len: 6 }
    );
    assert_eq!(
        rows[..6],
        [
            row(12793, 0, OCTOBER, 1326, 300_000),
            row(12793, 0, OCTOBER, 1327, 100_000),
            row(12793, 0, NOVEMBER, 1326, 2_600_000),
            row(12793, 1, NOVEMBER, 1329, 900_000),
            row(12793, 3, OCTOBER, 5215, 200_000),
            row(12793, RECOVERED_KIND, OCTOBER, 1846, 100_000),
        ]
    );
    assert_eq!(rows[0].month, 1943 * 12 + 9);
    assert_eq!(rows[2].month, 1943 * 12 + 10);
}

#[test]
fn empty_logs_are_a_complete_empty_table() {
    let (f, _) = Fixture::new(&[]);
    let mut rows = [Row::default(); 4];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::new()
        ),
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn a_country_that_does_not_fit_is_counted_unread_whole() {
    let (mut f, _) = standard();
    // A second country (slot 2) whose one row still fits after country 7's six.
    let other = f.manager(
        8,
        &[(
            1,
            [
                &[Entry {
                    reason: 0,
                    date: OCTOBER,
                    pool: &[(1326, 1)],
                }][..],
                NONE,
                NONE,
                NONE,
                NONE,
            ],
        )],
    );
    f.pointer(MANAGERS + 16, other);
    let mut rows = [Row::default(); 7];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::new()
        ),
        Outcome::Rows { len: 7 }
    );
    // Country 7's first two pools fit (2 + 1 rows) but the rest do not: all
    // six of its rows are counted, and so is every later country's.
    let mut rows = [Row::default(); 3];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::new()
        ),
        Outcome::Truncated {
            len: 0,
            unread_rows: 7
        }
    );
    // Selecting only the second country reads it whole.
    let mut rows = [Row::default(); 1];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |country| (country == 8).then_some(Months::All),
            &mut rows,
            &mut VariantMemo::new()
        ),
        Outcome::Rows { len: 1 }
    );
    let (f, _) = standard();
    let mut rows = [Row::default(); 1];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::new()
        ),
        Outcome::Truncated {
            len: 0,
            unread_rows: 6
        }
    );
}

fn unavailable(f: &Fixture) -> &'static str {
    let mut rows = [Row::default(); 16];
    match walk(
        f,
        BASE,
        GAME,
        |_| Some(Months::All),
        &mut rows,
        &mut VariantMemo::new(),
    ) {
        Outcome::Unavailable { reason } => reason,
        other => panic!("expected unavailable, got {other:?}"),
    }
}

/// The address of the first group's log and of its first reason-0 entry.
fn first_group(f: &Fixture, manager: usize) -> (usize, usize) {
    let list = read::<u64>(f, manager + GROUPS).unwrap() as usize;
    let group = read::<u64>(f, list).unwrap() as usize;
    let entries = read::<u64>(f, group + logs::REASONS).unwrap() as usize;
    (group, read::<u64>(f, entries).unwrap() as usize)
}

#[test]
fn refuses_every_unexpected_form() {
    let (mut f, manager) = standard();
    f.pointer(manager, BASE + 8);
    assert_eq!(unavailable(&f), "combat log manager vtable mismatch");

    let (mut f, manager) = standard();
    let (group, _) = first_group(&f, manager);
    f.pointer(group, BASE + 8);
    assert_eq!(unavailable(&f), "combat log group vtable mismatch");

    let (mut f, manager) = standard();
    let (group, _) = first_group(&f, manager);
    f.u32(group + logs::GROUP, 52);
    assert_eq!(
        unavailable(&f),
        "combat log group reference is not an orders group"
    );

    let (mut f, manager) = standard();
    let (_, entry) = first_group(&f, manager);
    f.put(entry + loss::REASON, &2_i32.to_le_bytes());
    assert_eq!(
        unavailable(&f),
        "combat log month reason differs from its array"
    );

    let (mut f, manager) = standard();
    let (_, entry) = first_group(&f, manager);
    f.put(entry + loss::DATE, &NOVEMBER.to_le_bytes());
    assert_eq!(
        unavailable(&f),
        "combat log months repeat or are out of order"
    );

    let (mut f, manager) = standard();
    let (_, entry) = first_group(&f, manager);
    f.put(entry + loss::DATE, &(-1_i32).to_le_bytes());
    assert_eq!(
        unavailable(&f),
        "combat log month outside the recorded range"
    );

    let (mut f, manager) = standard();
    let (_, entry) = first_group(&f, manager);
    f.pointer(entry + loss::POOL, BASE + 8);
    assert_eq!(unavailable(&f), "combat log month pool vtable mismatch");

    let (mut f, manager) = standard();
    let (_, entry) = first_group(&f, manager);
    let items = read::<u64>(&f, entry + loss::POOL + pool::ITEMS).unwrap() as usize;
    let other = read::<u64>(&f, items).unwrap();
    f.pointer(items + 16, other as usize);
    assert_eq!(unavailable(&f), "combat log pool names a variant twice");

    let (mut f, _) = standard();
    f.u32(VARIANTS + 1327 * 0x100 + 8, 71);
    assert_eq!(unavailable(&f), "variant vtable or reference type mismatch");

    let (mut f, manager) = standard();
    let (_, entry) = first_group(&f, manager);
    let items = read::<u64>(&f, entry + loss::POOL + pool::ITEMS).unwrap() as usize;
    f.forget(items + 24, 8);
    assert_eq!(unavailable(&f), "combat log pool entries unreadable");

    let (mut f, manager) = standard();
    let list = read::<u64>(&f, manager + GROUPS).unwrap() as usize;
    f.pointer(list + 8, 0);
    assert_eq!(unavailable(&f), "combat log group pointer null");

    let (mut f, manager) = standard();
    let (group, _) = first_group(&f, manager);
    f.forget(group + logs::GROUP, 8);
    assert_eq!(unavailable(&f), "combat log group unreadable");

    let (mut f, _) = standard();
    f.array(GAME + cgame_state::COMBAT_LOGS, MANAGERS, 0);
    assert_eq!(
        unavailable(&f),
        "combat log manager array has no sentinel slot"
    );

    let (mut f, _) = standard();
    f.forget(MANAGERS + 8, 8);
    assert_eq!(unavailable(&f), "combat log manager pointers unreadable");
}

#[test]
fn the_recovered_array_keeps_whatever_reason_it_was_built_with() {
    let (mut f, manager) = standard();
    let (group, _) = first_group(&f, manager);
    let entries = read::<u64>(&f, group + logs::RECOVERED).unwrap() as usize;
    let entry = read::<u64>(&f, entries).unwrap() as usize;
    f.put(entry + loss::REASON, &4_i32.to_le_bytes());
    let mut rows = [Row::default(); 16];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::new()
        ),
        Outcome::Rows { len: 6 }
    );
}

#[test]
fn many_groups_and_months_cross_batch_boundaries() {
    let pools: Vec<Vec<(u32, i64)>> = (0..70)
        .map(|month| {
            (0..70)
                .map(|v| (v + 1, i64::from(month * 100 + v)))
                .collect()
        })
        .collect();
    let entries: Vec<Entry> = pools
        .iter()
        .enumerate()
        .map(|(month, pool)| Entry {
            reason: 0,
            date: OCTOBER + month as i32 * 24 * 31,
            pool,
        })
        .collect();
    let groups: Vec<(u32, [&[Entry]; 5])> = (0..70)
        .map(|id| (id + 1, [&entries[..], NONE, NONE, NONE, NONE]))
        .collect();
    let (f, _) = Fixture::new(&groups);
    let mut rows = vec![Row::default(); 70 * 70 * 70];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::new()
        ),
        Outcome::Rows { len: 70 * 70 * 70 }
    );
    let last = rows[70 * 70 * 70 - 1];
    assert_eq!((last.group, last.variant, last.amount_raw), (70, 70, 6969));
    let months: Vec<u16> = rows.iter().step_by(70).take(70).map(|r| r.month).collect();
    assert!(months.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn viewing_country_is_the_raw_signed_reference() {
    let mut f = Fixture::default();
    f.put(
        GAME + cgame_state::LOCAL_PLAYER_COUNTRY,
        &(-1_i32).to_le_bytes(),
    );
    assert_eq!(viewing_country(&f, GAME), Some(-1));
    f.put(
        GAME + cgame_state::LOCAL_PLAYER_COUNTRY,
        &4_i32.to_le_bytes(),
    );
    assert_eq!(viewing_country(&f, GAME), Some(4));
    assert_eq!(viewing_country(&Fixture::default(), GAME), None);
}

/// Release timing at 5,000 country slots, over a flat `SliceMemory` copy of
/// the fixture (guarded-read work, not gameplay slowdown). `human` (default)
/// logs one country with 60 orders groups; `observer` logs every third slot
/// (1,666 countries, the case in which the game logs every country) with 5
/// groups. Every group holds 12 months of 20 (human) or 10 (observer)
/// variants in reasons 0, 1 and 3 and the recovered array, and resolves in
/// the registry. It times a full read (every month of every country, a full
/// hour), one hour's slice with window reads (the ordinary hour), the window
/// read of every country (what one daily read would cost) and the hour-23
/// deleted-group scan. Run
/// `cargo test -p engine --release --lib combat_log_scale -- --ignored --nocapture`.
#[test]
#[ignore]
fn combat_log_scale() {
    let shape = std::env::var("BATTLE_PLAN_COMBAT_LOG_SCALE").or_else(|_| std::env::var("HOI4_COMBAT_LOG_SCALE")).unwrap_or_else(|_| "human".into());
    let (every, groups, variants) = match shape.as_str() {
        "observer" => (3, 5, 10),
        _ => (4999, 60, 20),
    };
    let mut f = Fixture::scale(5000, every, groups, 12, variants);
    f.registry(&(1..=groups).collect::<Vec<_>>());
    let countries = (1..5000).filter(|slot| slot % every == 0).count();
    let arrays = countries * groups as usize * 4;
    let rows = arrays * 12 * variants as usize;
    let mut output = vec![Row::default(); rows];
    let mut memo = VariantMemo::new();
    let memory = crate::memory::SliceMemory {
        base: 0,
        bytes: &f.bytes,
    };
    // The registry overlay is read from the fixture itself.
    let time = |label: &str, run: &mut dyn FnMut() -> usize, expected: usize| {
        let mut best = std::time::Duration::MAX;
        for _ in 0..20 {
            let started = std::time::Instant::now();
            let len = run();
            best = best.min(started.elapsed());
            assert_eq!(len, expected, "{shape} {label}");
        }
        println!(
            "combat log {shape} {label}: {countries} logged of 5000 slots, {expected} rows or countries, fastest {:.3} ms",
            best.as_secs_f64() * 1e3
        );
    };
    let full = |output: &mut [Row], memo: &mut VariantMemo| {
        walk(&memory, BASE, GAME, |_| Some(Months::All), output, memo).len()
    };
    time("full hour", &mut || full(&mut output, &mut memo), rows);
    let slice = (1..5000_u32)
        .filter(|slot| slot % every as u32 == 0 && replay_slice(*slot) == 0)
        .count();
    time(
        "one slice, window",
        &mut || {
            walk(
                &memory,
                BASE,
                GAME,
                |country| (replay_slice(country) == 0).then_some(Months::Window),
                &mut output,
                &mut memo,
            )
            .len()
        },
        slice * groups as usize * 4 * 2 * variants as usize,
    );
    time(
        "every country, window",
        &mut || {
            walk(
                &memory,
                BASE,
                GAME,
                |_| Some(Months::Window),
                &mut output,
                &mut memo,
            )
            .len()
        },
        arrays * 2 * variants as usize,
    );
    time(
        "deleted-group scan",
        &mut || {
            let mut found = 0;
            countries_with_deleted_groups(&f, BASE, GAME, |_| found += 1).unwrap();
            found
        },
        0,
    );
}

/// The recorder's slice of an owner (`replay::cadence::slice_of`: owner % 24).
fn replay_slice(owner: u32) -> u32 {
    owner % 24
}

#[test]
fn a_window_read_keeps_each_arrays_last_two_months() {
    let three: Vec<Entry> = [OCTOBER - 24 * 31, OCTOBER, NOVEMBER]
        .into_iter()
        .map(|date| Entry {
            reason: 0,
            date,
            pool: &[(1326, 1)],
        })
        .collect();
    let (f, _) = Fixture::new(&[(9, [&three[..], NONE, NONE, NONE, &three[..1]])]);
    let mut rows = [Row::default(); 8];
    let outcome = walk(
        &f,
        BASE,
        GAME,
        |_| Some(Months::Window),
        &mut rows,
        &mut VariantMemo::new(),
    );
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        rows[..3]
            .iter()
            .map(|row| (row.kind, row.date))
            .collect::<Vec<_>>(),
        [
            (0, OCTOBER),
            (0, NOVEMBER),
            (RECOVERED_KIND, OCTOBER - 24 * 31)
        ]
    );
    let outcome = walk(
        &f,
        BASE,
        GAME,
        |_| Some(Months::All),
        &mut rows,
        &mut VariantMemo::new(),
    );
    assert_eq!(outcome, Outcome::Rows { len: 4 });
}

#[test]
fn each_country_is_read_with_the_months_its_selector_chooses() {
    let three: Vec<Entry> = [OCTOBER - 24 * 31, OCTOBER, NOVEMBER]
        .into_iter()
        .map(|date| Entry {
            reason: 0,
            date,
            pool: &[(1326, 1)],
        })
        .collect();
    let (mut f, _) = Fixture::new(&[(9, [&three[..], NONE, NONE, NONE, NONE])]);
    let other = f.manager(8, &[(10, [&three[..], NONE, NONE, NONE, NONE])]);
    f.pointer(MANAGERS + 16, other);
    let mut rows = [Row::default(); 8];
    let mut asked = Vec::new();
    let outcome = walk(
        &f,
        BASE,
        GAME,
        |country| {
            asked.push(country);
            Some(if country == 7 {
                Months::All
            } else {
                Months::Window
            })
        },
        &mut rows,
        &mut VariantMemo::new(),
    );
    assert_eq!(outcome, Outcome::Rows { len: 5 });
    // Country 3's manager holds no group log and is not asked.
    assert_eq!(
        asked,
        [7, 8],
        "once per manager holding a log, in slot order"
    );
    assert_eq!(
        rows[..5]
            .iter()
            .map(|row| (row.country, row.date))
            .collect::<Vec<_>>(),
        [
            (7, OCTOBER - 24 * 31),
            (7, OCTOBER),
            (7, NOVEMBER),
            (8, OCTOBER),
            (8, NOVEMBER)
        ]
    );
}

#[test]
fn only_selected_countries_are_followed_past_their_header() {
    let (mut f, manager) = standard();
    // An unselected country's broken log is never read.
    let (group, _) = first_group(&f, manager);
    f.pointer(group, BASE + 8);
    let mut rows = [Row::default(); 8];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |country| (country != 7).then_some(Months::All),
            &mut rows,
            &mut VariantMemo::new(),
        ),
        Outcome::Rows { len: 0 }
    );
    // Every manager's header is still authenticated.
    f.pointer(manager, BASE + 8);
    assert!(matches!(
        walk(&f, BASE, GAME, |_| None, &mut rows, &mut VariantMemo::new(),),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn deleted_groups_are_the_logs_whose_orders_group_no_longer_resolves() {
    let scan = |f: &Fixture| {
        let mut countries = Vec::new();
        countries_with_deleted_groups(f, BASE, GAME, |country| countries.push(country))
            .map(|()| countries)
    };
    // A null reference table for the type: nothing resolves.
    let (mut f, _) = standard();
    let root = BASE + registry::LOW_ROOTS_RVA + corders_group::REFERENCE_TYPE as usize * 8;
    f.pointer(root, 0);
    assert_eq!(scan(&f), Ok(vec![7]));
    f.registry(&[12793, 12794]);
    assert_eq!(scan(&f), Ok(vec![]));
    f.registry(&[12793]);
    assert_eq!(scan(&f), Ok(vec![7]));
    // An unreadable registry is an error, not a deleted group.
    for byte in 0..8 {
        f.high.remove(&(root + byte));
    }
    assert!(scan(&f).is_err());
}

#[test]
fn the_variant_memo_changes_no_row_and_no_failure() {
    let (f, _) = standard();
    let mut remembered = [Row::default(); 16];
    let mut unremembered = [Row::default(); 16];
    let mut memo = VariantMemo::new();
    // Twice with the same memo: it is cleared at the start of each walk.
    for _ in 0..2 {
        assert_eq!(
            walk(
                &f,
                BASE,
                GAME,
                |_| Some(Months::All),
                &mut remembered,
                &mut memo
            ),
            Outcome::Rows { len: 6 }
        );
    }
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut unremembered,
            &mut VariantMemo::vacant()
        ),
        Outcome::Rows { len: 6 }
    );
    assert_eq!(remembered, unremembered);
    // A variant broken after an earlier walk remembered it is still refused.
    let (mut f, _) = standard();
    walk(
        &f,
        BASE,
        GAME,
        |_| Some(Months::All),
        &mut remembered,
        &mut memo,
    );
    f.u32(VARIANTS + 1326 * 0x100 + 8, 71);
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut remembered,
            &mut memo
        ),
        Outcome::Unavailable {
            reason: "variant vtable or reference type mismatch"
        }
    );
    // A memo that stops remembering still answers only what it holds.
    let mut full = VariantMemo::new();
    for pointer in 1..=MEMO_LIMIT {
        full.insert(pointer << 4, pointer as u32);
    }
    assert_eq!(full.len, MEMO_LIMIT);
    full.insert(0xdead_0000, 1);
    assert_eq!(full.get(0xdead_0000), None);
    assert_eq!(full.get(7 << 4), Some(7));
}
