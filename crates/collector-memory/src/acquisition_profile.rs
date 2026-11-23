use collector_status::status::{DebugValue, map, status};
use engine_native_memory::{acquisition_profile::Costs, memory::ReadCounts};
use replay_data::rows::{TABLE_COUNT, TableId};
use std::{
    cell::Cell,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

const WIDTH: usize = 11;
static TOTALS: [[AtomicU64; WIDTH]; TABLE_COUNT] =
    [const { [const { AtomicU64::new(0) }; WIDTH] }; TABLE_COUNT];
static OVERLAPPED: [AtomicU64; TABLE_COUNT] = [const { AtomicU64::new(0) }; TABLE_COUNT];

pub fn overlapped(table: TableId, ns: u64) {
    OVERLAPPED[table.index()].fetch_add(ns, Ordering::Relaxed);
}

thread_local! {
    static LAST: Cell<Option<(Instant, u64)>> = const { Cell::new(None) };
    static LEASE: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn boundary(table: usize, reads: ReadCounts) {
    let costs = engine_native_memory::acquisition_profile::take();
    let now = Instant::now();
    let growths = engine_allocation::growths();
    let (elapsed, checked_growth_attempts) = LAST
        .with(|last| last.replace(Some((now, growths))))
        .map_or((0, 0), |(last, previous_growths)| {
            (
                now.duration_since(last).as_nanos().min(u64::MAX as u128) as u64,
                growths.saturating_sub(previous_growths),
            )
        });
    let held = LEASE.with(|lease| lease.get() != 0);
    let Costs {
        field_ns,
        bulk_ns,
        pointer_width_reads,
    } = costs;
    let values = [
        reads.bytes,
        reads.spans,
        reads.copy_calls,
        reads.scattered_calls,
        reads.record_calls,
        reads.plan_calls,
        field_ns,
        bulk_ns,
        pointer_width_reads,
        elapsed,
        checked_growth_attempts,
    ];
    if held {
        for (total, value) in TOTALS[table].iter().zip(values) {
            total.fetch_add(value, Ordering::Relaxed);
        }
    }
}

pub(crate) fn enter() {
    LEASE.with(|lease| lease.set(lease.get() + 1));
    LAST.with(|last| last.set(Some((Instant::now(), engine_allocation::growths()))));
}

pub(crate) fn leave() {
    LEASE.with(|lease| lease.set(lease.get() - 1));
    LAST.with(|last| last.set(Some((Instant::now(), engine_allocation::growths()))));
}

pub fn status() -> impl serde::Serialize {
    status!({
        "scope": "feature-only instrumented acquisition inside live-read leases; requested bytes include failed spans; timings include instrumentation overhead; pointer-width reads include nonpointer integers and are not semantic pointer dereferences; checked_growth_attempts counts checked heap growth requests, not every allocation",
        "tables": map(TableId::ALL.iter().map(|table| {
            let [bytes, spans, field_calls, scattered_calls, record_calls, plan_calls,
                field_ns, bulk_ns, pointer_width_reads, lease_ns, checked_growth_attempts] =
                TOTALS[table.index()].each_ref().map(|value| value.load(Ordering::Relaxed));
            (DebugValue(*table), status!({
                "bytes": bytes, "spans": spans, "field_calls": field_calls,
                "scattered_calls": scattered_calls, "record_calls": record_calls,
                "plan_calls": plan_calls, "field_ns": field_ns, "bulk_ns": bulk_ns,
                "pointer_width_reads": pointer_width_reads, "lease_ns": lease_ns,
                "checked_growth_attempts": checked_growth_attempts,
                "overlapped_decode_ns": OVERLAPPED[table.index()].load(Ordering::Relaxed),
            }))
        }))
    })
}
