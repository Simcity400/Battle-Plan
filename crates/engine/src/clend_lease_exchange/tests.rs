use super::*;
use crate::ccountry_resources::test_support::Fixture;

/// The giver (raw 7, resources 0x6000) holds two exchanges; the other country
/// (raw 2, resources 0x7000) holds none.
const EXCHANGES: usize = 0x40000;
const LIST: usize = 0x3f000;
const POOLS: usize = 0x50000;
const VARIANTS: usize = 0x60000;

fn variant(f: &mut Fixture, id: u32) -> usize {
    let at = VARIANTS + id as usize * 0x20;
    f.u64(at, cequipment_variant::VTABLE_RVA as u64);
    f.u32(at + 8, cequipment_variant::REFERENCE_TYPE);
    f.u32(at + 12, id);
    at
}

fn pool_at(f: &mut Fixture, pool_address: usize, data: usize, entries: &[(u32, i64)]) {
    f.u64(pool_address, pool::VTABLE_RVA as u64);
    f.array(pool_address + pool::ITEMS, data, entries.len() as u32);
    for (index, &(id, amount)) in entries.iter().enumerate() {
        let pointer = variant(f, id);
        f.u64(data + index * 16, pointer as u64);
        f.u64(data + index * 16 + 8, amount as u64);
    }
}

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    f.array(0x6000 + OUTGOING, LIST, 2);
    f.array(0x7000 + OUTGOING, 0, 0);
    for (index, (id, receiver, delivered, sunk)) in [
        (
            97730_u32,
            2_u32,
            &[(1723, 1_800_000_i64), (1722, 6_600_000)][..],
            &[][..],
        ),
        (97731, 9, &[(5157, 4_600_000)][..], &[(5157, 300_000)][..]),
    ]
    .into_iter()
    .enumerate()
    {
        let exchange = EXCHANGES + index * 0x400;
        f.put(exchange, &[0; SIZE]);
        f.u64(exchange, VTABLE_RVA as u64);
        f.u64(exchange + HANDLE, u64::from(id) << 32 | 4713);
        f.u32(exchange + RECEIVER, receiver);
        let data = POOLS + index * 0x200;
        pool_at(&mut f, exchange + DELIVERED, data, delivered);
        pool_at(&mut f, exchange + SUNK, data + 0x100, sunk);
        f.u64(LIST + index * 8, exchange as u64);
    }
    f
}

fn row(exchange: u32, kind: u8, variant: u32, receiver: u32, amount_raw: i64) -> Row {
    Row {
        giver: 7,
        exchange,
        kind,
        variant,
        receiver,
        amount_raw,
    }
}

#[test]
fn reads_every_outgoing_exchange_delivered_then_sunk() {
    let f = fixture();
    let mut rows = [Row::default(); 8];
    assert_eq!(walk(&f, 0, 0x1000, &mut rows), Outcome::Rows { len: 4 });
    assert_eq!(
        rows[..4],
        [
            row(97730, DELIVERED_KIND, 1723, 2, 1_800_000),
            row(97730, DELIVERED_KIND, 1722, 2, 6_600_000),
            row(97731, DELIVERED_KIND, 5157, 9, 4_600_000),
            row(97731, SUNK_KIND, 5157, 9, 300_000),
        ]
    );
}

#[test]
fn truncation_keeps_a_prefix_and_counts_the_rest() {
    let f = fixture();
    let mut rows = [Row::default(); 2];
    assert_eq!(
        walk(&f, 0, 0x1000, &mut rows),
        Outcome::Truncated {
            len: 2,
            unread_rows: 2
        }
    );
}

#[test]
fn refuses_unexpected_forms() {
    let unavailable = |f: &Fixture| match walk(f, 0, 0x1000, &mut [Row::default(); 8]) {
        Outcome::Unavailable { reason } => reason,
        other => panic!("{other:?}"),
    };
    let mut f = fixture();
    f.u64(EXCHANGES, 8);
    assert_eq!(unavailable(&f), "lend-lease exchange vtable mismatch");
    let mut f = fixture();
    f.u64(EXCHANGES + SUNK, 8);
    assert_eq!(unavailable(&f), "lend-lease pool vtable mismatch");
    let mut f = fixture();
    f.u64(LIST + 8, 0);
    assert_eq!(unavailable(&f), "lend-lease exchange pointer null");
    let mut f = fixture();
    f.u32(VARIANTS + 1722 * 0x20 + 8, 71);
    assert_eq!(unavailable(&f), "variant vtable or reference type mismatch");
    let mut f = fixture();
    let first = POOLS;
    let pointer = VARIANTS + 1723 * 0x20;
    f.u64(first + 16, pointer as u64);
    assert_eq!(unavailable(&f), "lend-lease pool names a variant twice");
    let mut f = fixture();
    f.bytes.remove(&(EXCHANGES + DELIVERED + 3));
    assert_eq!(unavailable(&f), "lend-lease exchange unreadable");
}

#[test]
fn worker_pool_growth_precedes_first_and_later_publication() {
    let mut fixture = fixture();
    let mut rows = Vec::new();
    assert_eq!(row_demand(&fixture, 0, 0x1000), Ok(4));
    assert_eq!(
        walk_growing(&fixture, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 4 }
    );
    let entries: Vec<_> = (1..=5_000).map(|id| (id, i64::from(id))).collect();
    pool_at(&mut fixture, EXCHANGES + DELIVERED, 0x10_0000, &entries);
    assert_eq!(row_demand(&fixture, 0, 0x1000), Ok(5_002));
    assert_eq!(
        walk_growing(&fixture, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 5_002 }
    );
    assert_eq!(
        (rows[4_999].variant, rows[4_999].amount_raw),
        (5_000, 5_000)
    );
    assert!(matches!(
        walk(&fixture, 0, 0x1000, &mut [Row::default(); 4]),
        Outcome::Truncated { .. }
    ));
}

#[test]
fn growing_lend_lease_covers_5000_country_exchange_pools() {
    let mut world = crate::ccountry::synthetic::World::new(0, 5_000, 0x1600);
    let variant = world.alloc(16);
    world.u64(variant, cequipment_variant::VTABLE_RVA as u64);
    world.u32(variant + 8, cequipment_variant::REFERENCE_TYPE);
    world.u32(variant + 12, 17);
    for country in 1..=5_000 {
        let resources = world.alloc(0x800);
        world.u64(
            world.object(country) + ccountry::RESOURCES,
            resources as u64,
        );
        world.u64(resources, ccountry_resources::VTABLE_RVA as u64);
        world.u32(resources + ccountry_resources::OWNER, country);
        let exchange = world.alloc(SIZE);
        world.u64(exchange, VTABLE_RVA as u64);
        world.u64(exchange + HANDLE, u64::from(country) << 32 | 4713);
        world.u32(exchange + RECEIVER, 1);
        let items = world.alloc(equipment_entry::STRIDE);
        world.u64(items, variant as u64);
        world.u64(items + equipment_entry::AMOUNT, u64::from(country));
        world.u64(exchange + DELIVERED, pool::VTABLE_RVA as u64);
        world.array(exchange + DELIVERED + pool::ITEMS, items, 1);
        world.u64(exchange + SUNK, pool::VTABLE_RVA as u64);
        let list = world.alloc(8);
        world.u64(list, exchange as u64);
        world.array(resources + OUTGOING, list, 1);
    }
    let mut rows = Vec::new();
    assert_eq!(
        row_demand(&world.memory(), 0, world.game_state()),
        Ok(5_000)
    );
    assert_eq!(
        walk_growing(&world.memory(), 0, world.game_state(), &mut rows),
        Outcome::Rows { len: 5_000 }
    );
    assert_eq!((rows[4_999].giver, rows[4_999].amount_raw), (5_000, 5_000));
}
