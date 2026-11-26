use super::*;
use engine_native_parts::{creference_registry as registry, reference_bucket as bucket};
use std::collections::BTreeMap;

const BASE: usize = 0x0001_4000_0000;
const GAME: usize = 0x10000;
const MANAGERS: usize = 0x11000;
const VARIANTS: usize = 0x80_0000;
const OCTOBER: i32 = 60_827_868;
const NOVEMBER: i32 = 60_827_979;

const HIGH: usize = 1 << 32;
const REGISTRY: usize = 0x20000;
const BUCKETS: usize = 0x21000;

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
    fn alloc(&mut self, len: usize) -> usize {
        let address = 0x100_0000 + self.next;
        self.next += (len + 0xff) & !0xff;
        self.put(address, &vec![0; len]);
        address
    }
    fn registry(&mut self, groups: &[u32]) {
        let capacity = (groups.len() * 2).next_power_of_two().max(8) as u32;
        let exponent = capacity.trailing_zeros();
        let overflow = exponent + 2;
        let terminal = capacity + overflow;
        let root = BASE + registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8;
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
        self.pointer(variant, BASE + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0);
        self.u32(variant + 8, engine_native::cequipment_variant::REFERENCE_TYPE);
        self.u32(variant + 12, id);
        variant
    }

    fn new(groups: &[(u32, [&[Entry]; 5])]) -> (Self, usize) {
        let mut f = Self::default();
        f.array(GAME + cgame_state::COMBAT_LOGS, MANAGERS, 4);
        f.pointer(MANAGERS, 0);
        f.pointer(MANAGERS + 16, 0);
        let empty = f.alloc(SIZE);
        f.pointer(MANAGERS + 24, empty);
        f.pointer(empty, BASE + VTABLE_RVA.0);
        f.array(empty + GROUPS, 0, 0);
        f.u32(empty + COUNTRY, 3);
        let manager = f.manager(7, groups);
        f.pointer(MANAGERS + 8, manager);
        (f, manager)
    }

    fn manager(&mut self, country: u32, groups: &[(u32, [&[Entry]; 5])]) -> usize {
        let manager = self.alloc(SIZE);
        self.pointer(manager, BASE + VTABLE_RVA.0);
        self.u32(manager + COUNTRY, country);
        let list = self.alloc(groups.len() * 8);
        self.array(manager + GROUPS, list, groups.len() as u32);
        for (index, (id, kinds)) in groups.iter().enumerate() {
            let group = self.alloc(logs::SIZE);
            self.pointer(list + index * 8, group);
            self.pointer(group, BASE + logs::VTABLE_RVA.0);
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
                    self.pointer(object, BASE + loss::VTABLE_RVA.0);
                    self.put(object + loss::REASON, &entry.reason.to_le_bytes());
                    self.put(object + loss::DATE, &entry.date.to_le_bytes());
                    self.pointer(object + loss::POOL, BASE + pool::VTABLE_RVA.0);
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
fn reads_every_group_and_month_in_native_order() {
    let (f, _) = standard();
    let mut rows = vec![Row::default(); 16];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::default()
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
fn each_pool_reads_in_variant_order_and_a_repeat_anywhere_in_it_is_refused() {
    let pool = |pool| {
        Fixture::new(&[(
            12793,
            [
                &[Entry {
                    reason: 0,
                    date: OCTOBER,
                    pool,
                }],
                NONE,
                NONE,
                NONE,
                NONE,
            ],
        )])
        .0
    };
    let f = pool(&[(30, 3), (10, 1), (40, 4), (20, 2)]);
    {
        let mut memo = VariantMemo::default();
        let mut rows = vec![Row::default(); 4];
        assert_eq!(
            walk(&f, BASE, GAME, |_| Some(Months::All), &mut rows, &mut memo),
            Outcome::Rows { len: 4 }
        );
        assert_eq!(
            rows.iter().map(|row| (row.variant, row.amount_raw)).collect::<Vec<_>>(),
            [(10, 1), (20, 2), (30, 3), (40, 4)]
        );
    }
    let f = pool(&[(1, 1), (2, 2), (3, 3), (1, 4)]);
    assert_eq!(unavailable(&f), "combat log pool names a variant twice");
}

#[test]
fn empty_logs_are_a_complete_empty_table() {
    let (f, _) = Fixture::new(&[]);
    let mut rows = vec![Row::default(); 4];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::default()
        ),
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn a_country_whose_rows_start_short_grows_whole() {
    let (mut f, _) = standard();
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
    let mut rows = vec![Row::default(); 7];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::default()
        ),
        Outcome::Rows { len: 7 }
    );
    let mut rows = vec![Row::default(); 3];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::default()
        ),
        Outcome::Rows { len: 7 }
    );
    let mut rows = vec![Row::default(); 1];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |country| (country == 8).then_some(Months::All),
            &mut rows,
            &mut VariantMemo::default()
        ),
        Outcome::Rows { len: 1 }
    );
    let (f, _) = standard();
    let mut rows = vec![Row::default(); 1];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::default()
        ),
        Outcome::Rows { len: 6 }
    );
}

fn unavailable(f: &Fixture) -> &'static str {
    let mut rows = vec![Row::default(); 16];
    match walk(
        f,
        BASE,
        GAME,
        |_| Some(Months::All),
        &mut rows,
        &mut VariantMemo::default(),
    ) {
        Outcome::Unavailable { reason } => reason,
        other => panic!("expected unavailable, got {other:?}"),
    }
}

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
    let mut rows = vec![Row::default(); 16];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |_| Some(Months::All),
            &mut rows,
            &mut VariantMemo::default()
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
            &mut VariantMemo::default()
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

fn replay_slice(owner: u32) -> u32 {
    owner % 24
}

#[test]
fn a_window_reuses_sealed_loss_rows_and_reads_a_short_recovery_array() {
    let three: Vec<Entry> = [OCTOBER - 24 * 31, OCTOBER, NOVEMBER]
        .into_iter()
        .map(|date| Entry {
            reason: 0,
            date,
            pool: &[(1326, 1)],
        })
        .collect();
    let (mut f, manager) = Fixture::new(&[(9, [&three[..], NONE, NONE, NONE, &three[..1]])]);
    let mut rows = Vec::new();
    let mut sealed = SealedMonths::default();
    let mut memo = VariantMemo::default();
    let mut sample = |memory: &Fixture, whole: bool, rows: &mut Vec<Row>| {
        walk_prepared(
            memory,
            BASE,
            GAME,
            &mut sealed,
            |sealed, _| Ok(Some(Months::Retaining { whole, sealed })),
            rows,
            &mut memo,
        )
    };
    assert_eq!(sample(&f, true, &mut rows), Outcome::Rows { len: 4 });
    let (group, oldest) = first_group(&f, manager);
    let pool_items =
        |f: &Fixture, entry| read::<u64>(f, entry + loss::POOL + pool::ITEMS).unwrap() as usize;
    f.forget(pool_items(&f, oldest), equipment_entry::STRIDE);
    let recoveries = read::<u64>(&f, group + logs::RECOVERED).unwrap() as usize;
    let recovery = read::<u64>(&f, recoveries).unwrap() as usize;
    f.put(
        pool_items(&f, recovery) + equipment_entry::AMOUNT,
        &7_i64.to_le_bytes(),
    );
    assert_eq!(sample(&f, false, &mut rows), Outcome::Rows { len: 4 });
    assert_eq!(
        rows[..4]
            .iter()
            .map(|row| (row.kind, row.date, row.amount_raw))
            .collect::<Vec<_>>(),
        [
            (0, three[0].date, 1),
            (0, OCTOBER, 1),
            (0, NOVEMBER, 1),
            (RECOVERED_KIND, three[0].date, 7)
        ]
    );
}

#[test]
fn a_newly_selected_country_reads_every_month_on_a_window_request() {
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
    let mut rows = vec![Row::default(); 8];
    let mut asked = Vec::new();
    let outcome = walk_prepared(
        &f,
        BASE,
        GAME,
        &mut SealedMonths::default(),
        |sealed, country| {
            asked.push(country);
            Ok((country == 8).then_some(Months::Retaining {
                whole: false,
                sealed,
            }))
        },
        &mut rows,
        &mut VariantMemo::default(),
    );
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        asked,
        [7, 8],
        "once per manager holding a log, in slot order"
    );
    assert_eq!(
        rows[..3]
            .iter()
            .map(|row| (row.country, row.date))
            .collect::<Vec<_>>(),
        [(8, three[0].date), (8, OCTOBER), (8, NOVEMBER)]
    );
}

#[test]
fn only_selected_countries_are_followed_past_their_header() {
    let (mut f, manager) = standard();
    let (group, _) = first_group(&f, manager);
    f.pointer(group, BASE + 8);
    let mut rows = vec![Row::default(); 8];
    assert_eq!(
        walk(
            &f,
            BASE,
            GAME,
            |country| (country != 7).then_some(Months::All),
            &mut rows,
            &mut VariantMemo::default(),
        ),
        Outcome::Rows { len: 0 }
    );
    f.pointer(manager, BASE + 8);
    assert!(matches!(
        walk(
            &f,
            BASE,
            GAME,
            |_| None,
            &mut rows,
            &mut VariantMemo::default(),
        ),
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
    let (mut f, _) = standard();
    let root = BASE + registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8;
    f.pointer(root, 0);
    assert_eq!(scan(&f), Ok(vec![7]));
    f.registry(&[12793, 12794]);
    assert_eq!(scan(&f), Ok(vec![]));
    f.registry(&[12793]);
    assert_eq!(scan(&f), Ok(vec![7]));
    for byte in 0..8 {
        f.high.remove(&(root + byte));
    }
    assert!(scan(&f).is_err());
}

#[test]
fn a_deleted_log_count_without_room_is_what_a_read_with_room_returns() {
    let (mut f, manager) = standard();
    let root = BASE + registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8;
    f.pointer(root, 0);
    assert_eq!(
        deleted_logs(&f, BASE, manager, &mut []),
        Ok((
            7,
            Outcome::Truncated {
                len: 0,
                unread_rows: 6
            }
        ))
    );
    let mut rows = [Row::default(); 6];
    assert_eq!(
        deleted_logs(&f, BASE, manager, &mut rows),
        Ok((7, Outcome::Rows { len: 6 }))
    );
    assert_eq!(rows[0], row(12793, 0, OCTOBER, 1326, 300_000));
    let mut short = [Row::default(); 2];
    assert_eq!(
        deleted_logs(&f, BASE, manager, &mut short),
        Ok((
            7,
            Outcome::Truncated {
                len: 2,
                unread_rows: 4
            }
        ))
    );
    assert_eq!(short, rows[..2]);
    f.registry(&[12793, 12794]);
    assert_eq!(
        deleted_logs(&f, BASE, manager, &mut []),
        Ok((7, Outcome::Rows { len: 0 }))
    );
}

#[test]
fn the_variant_memo_changes_no_row_and_no_failure() {
    let (f, _) = standard();
    let mut remembered = vec![Row::default(); 16];
    let mut unremembered = vec![Row::default(); 16];
    let mut memo = VariantMemo::default();
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
        walk_prepared_buffer(
            &f,
            BASE,
            GAME,
            &mut (),
            |_, _| Ok(()),
            |_, _| Ok(Some(Months::All)),
            Buffer::Growing(&mut unremembered),
            None,
            &mut |_, _, _, _| Ok(()),
        ),
        Outcome::Rows { len: 6 }
    );
    assert_eq!(remembered, unremembered);
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
    let mut full = VariantMemo::default();
    for pointer in 1..=10_000 {
        full.reserve(1).unwrap();
        full.insert(pointer << 4, pointer as u32);
    }
    assert_eq!(full.len, 10_000);
    full.reserve(1).unwrap();
    full.insert(0xdead_0000, 1);
    assert_eq!(full.get(0xdead_0000), Some(1));
    assert_eq!(full.get(7 << 4), Some(7));
}

