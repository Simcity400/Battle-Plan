use super::*;
use crate::ccountry_resources::test_support::Fixture;

const EXCHANGES: usize = 0x40000;
const LIST: usize = 0x3f000;
const POOLS: usize = 0x50000;
const VARIANTS: usize = 0x60000;

fn variant(f: &mut Fixture, id: u32) -> usize {
    let at = VARIANTS + id as usize * 0x20;
    f.u64(at, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
    f.u32(at + 8, cequipment_variant::REFERENCE_TYPE);
    f.u32(at + 12, id);
    at
}

fn pool_at(f: &mut Fixture, pool_address: usize, data: usize, entries: &[(u32, i64)]) {
    f.u64(pool_address, pool::VTABLE_RVA.0 as u64);
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
        f.u64(exchange, VTABLE_RVA.0 as u64);
        f.u64(exchange + HANDLE, u64::from(id) << 32 | 4713);
        f.u32(exchange + RECEIVER, receiver);
        let data = POOLS + index * 0x200;
        pool_at(&mut f, exchange + DELIVERED, data, delivered);
        pool_at(&mut f, exchange + SUNK, data + 0x100, sunk);
        f.u64(LIST + index * 8, exchange as u64);
    }
    f
}

#[test]
fn lend_lease_terms_keep_all_modes_dates_and_distinct_fuel_scales() {
    let mut f = fixture();
    f.u64(EXCHANGES + GIVER_RESOURCES, 0x6000);
    for (index, offset) in [MONTHLY_NEED, PERCENTAGE_NEED, ONCE_NEED]
        .into_iter()
        .enumerate()
    {
        pool_at(
            &mut f,
            EXCHANGES + offset,
            POOLS + 0x1000 + index * 0x100,
            &[(1723, [17, -123456, 9_000_000_017][index])],
        );
        f.put(
            EXCHANGES + offset + pool::ALLOW_ZERO,
            &[u8::from(index == 1)],
        );
    }
    f.u32(EXCHANGES + ACTIVE_SINCE, 43_800_123);
    f.u64(EXCHANGES + FUEL_DAILY, 32769);
    f.u64(EXCHANGES + FUEL_PERCENTAGE, (-10001_i64) as u64);
    let expected = identity(&f, 0, EXCHANGES).unwrap();
    let mut output = Terms::default();
    terms(&f, 0, EXCHANGES, expected, &mut output).unwrap();
    assert_eq!(output.monthly.equipment[0].amount_raw, 17);
    assert_eq!(output.percentage.equipment[0].amount_raw, -123456);
    assert_eq!(output.once.equipment[0].amount_raw, 9_000_000_017);
    assert!(output.percentage.allow_zero);
    assert_eq!(output.active_since, 43_800_123);
    assert_eq!(
        (output.fuel_daily_raw, output.fuel_percentage_raw),
        (32769, -10001)
    );
    let allocation = output.monthly.equipment.as_ptr();
    terms(&f, 0, EXCHANGES, expected, &mut output).unwrap();
    assert_eq!(allocation, output.monthly.equipment.as_ptr());
    let wrong = Identity {
        receiver: 5000,
        ..expected
    };
    assert_eq!(
        terms(&f, 0, EXCHANGES, wrong, &mut output),
        Err("lend-lease terms identity changed")
    );
}

#[test]
fn lend_lease_identity_preserves_dynamic_reference_types() {
    let mut f = fixture();
    f.u64(EXCHANGES + GIVER_RESOURCES, 0x6000);
    for kind in [0_u32, 53, 4713, u32::MAX] {
        let handle = (97730_u64 << 32) | u64::from(kind);
        f.u64(EXCHANGES + HANDLE, handle);
        assert_eq!(identity(&f, 0, EXCHANGES).unwrap().id, handle);
    }
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
    let mut rows = vec![Row::default(); 8];
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
fn refuses_unexpected_forms() {
    let unavailable = |f: &Fixture| match walk(f, 0, 0x1000, &mut vec![Row::default(); 8]) {
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
        walk(&fixture, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 4 }
    );
    let entries: Vec<_> = (1..=5_000).map(|id| (id, i64::from(id))).collect();
    pool_at(&mut fixture, EXCHANGES + DELIVERED, 0x10_0000, &entries);
    assert_eq!(row_demand(&fixture, 0, 0x1000), Ok(5_002));
    assert_eq!(
        walk(&fixture, 0, 0x1000, &mut rows),
        Outcome::Rows { len: 5_002 }
    );
    assert_eq!(
        (rows[4_999].variant, rows[4_999].amount_raw),
        (5_000, 5_000)
    );
}

#[test]
fn market_amendment_stream_retains_all_modes_before_the_source_disappears() {
    let mut f = fixture();
    let sources = [0x80000, 0x80100, 0x80200];
    for (i, source) in sources.into_iter().enumerate() {
        pool_at(
            &mut f,
            source,
            0x90000 + i * 0x100,
            &[(1723, i as i64 - 1), (1722, 0), (1723, 9_000_000_017)],
        );
        f.put(source + pool::ALLOW_ZERO, &[u8::from(i == 1)]);
    }
    let mut shape = ScheduleHeader::default();
    let mut owned = [ScheduleItem::default(); 9];
    let mut length = 0;
    assert_eq!(
        schedules(
            &f,
            0,
            sources,
            |header| {
                shape = header;
                Ok(())
            },
            |item| {
                owned[length] = item;
                length += 1;
                Ok(())
            }
        ),
        Ok(9)
    );
    f.put(0x90000, &[0; 0x300]);
    assert_eq!(shape.counts, [3; 3]);
    assert_eq!(shape.allow_zero, [false, true, false]);
    assert_eq!(owned[0].amount_raw, -1);
    assert_eq!(owned[3].mode, 1);
    assert_eq!(owned[8].ordinal, 2);
    assert_eq!(owned[8].amount_raw, 9_000_000_017);
    assert_eq!(owned[0].variant, owned[2].variant);
}

#[test]
fn market_amendment_stream_stops_at_a_rejected_header_without_item_reads() {
    let mut f = fixture();
    let sources = [0x80000, 0x80100, 0x80200];
    for source in sources {
        pool_at(&mut f, source, 0x90000, &[(1723, 1)]);
        f.put(source + pool::ALLOW_ZERO, &[0]);
    }
    f.bytes.remove(&0x90000);
    assert_eq!(
        schedules(
            &f,
            0,
            sources,
            |_| Err("prepared occurrence storage exhausted"),
            |_| panic!("item after a rejected replacement")
        ),
        Err("prepared occurrence storage exhausted")
    );
}
