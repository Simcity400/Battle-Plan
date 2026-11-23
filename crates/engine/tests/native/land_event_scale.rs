use engine_land_inputs::{cstats_observer as stats, land_events as native};
use engine_native::{
    carmy,
    ccountry,
    cequipment_variant::{self as variant, pool as pool},
    cgame_state,
    cland_combat as combat,
    cland_combatant as side,
    cprovince,
    cunit,
};
use std::mem::size_of;

struct Fixture {
    bytes: Vec<u8>,
    base: usize,
    loss_entries: [Vec<stats::LossEntry>; 2],
    loss_variants: Vec<[u64; 2]>,
}
impl Fixture {
    fn q(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn d(&mut self, at: usize, value: u32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn pointer(&mut self, at: usize, target: usize) {
        self.q(at, (self.base + target) as u64);
    }
    fn array(&mut self, at: usize, target: usize, count: u32) {
        self.pointer(at, target);
        self.d(at + 8, count);
        self.d(at + 12, count);
    }
    fn new(countries: usize, variants: usize) -> Self {
        let bytes = vec![0; cgame_state::SINGLETON_RVA.0 + 8];
        let base = bytes.as_ptr() as usize;
        let loss_variants = (0..variants)
            .map(|n| {
                [
                    (base + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0) as u64,
                    (n as u64 + 1) << 32 | 70,
                ]
            })
            .collect::<Vec<_>>();
        let loss_entries = std::array::from_fn(|_| {
            loss_variants
                .iter()
                .enumerate()
                .map(|(n, v)| stats::LossEntry {
                    variant: v.as_ptr() as u64,
                    amount_raw: n as i64 * 100_000 + 1,
                })
                .collect()
        });
        let mut f = Self {
            bytes,
            base,
            loss_entries,
            loss_variants,
        };
        f.pointer(cgame_state::SINGLETON_RVA.0, 0x1000);
        f.d(0x1000 + cgame_state::CURRENT_HOUR, 80);
        f.array(
            0x1000 + cgame_state::COUNTRIES,
            0x10000,
            (countries + 1) as u32,
        );
        f.array(
            0x1000 + cgame_state::COUNTRY_LINKS,
            0x20000,
            (countries + 1) as u32,
        );
        f.array(
            0x1000 + cgame_state::COUNTRY_TAGS,
            0x40000,
            (countries + 1) as u32,
        );
        f.bytes[0x40020..0x40026].copy_from_slice(b"opaque");
        f.q(0x40020 + 16, 6);
        f.q(0x40020 + 24, 15);
        f.pointer(0x10008, 0x30000);
        f.d(0x20004, 1);
        f.pointer(0x30000, ccountry::VTABLE_RVA.0);
        f.d(0x30000 + ccountry::ID, 1);
        f.pointer(0x2000, carmy::VTABLE_RVA.0);
        f.q(0x2000 + cunit::HANDLE, 0x8000_0001_0000_1269);
        f.d(0x2000 + cunit::ROSTER_OWNER, 1);
        f.d(0x2000 + cunit::LOGICAL_OWNER, 1);
        f.q(0x2000 + carmy::ORDERS_GROUP, 0);
        f.array(0x2000 + cunit::COMBAT_REFERENCE_DATA, 0x6000, 1);
        f.pointer(0x6000, 0x7000);
        for (p, id) in [(0x4000, 17), (0x5000, 18)] {
            f.pointer(p, cprovince::VTABLE_RVA.0);
            f.d(p + cprovince::ID, id);
            f.d(p + cprovince::CONTROLLER, 1);
        }
        f.pointer(0x7000, combat::VTABLE_RVA.0);
        f.q(0x7000 + combat::HANDLE, 0x8000_0002_0000_003e);
        f.pointer(0x7000 + combat::PROVINCE, 0x4000);
        f.d(0x7000 + combat::DAY, 9);
        f.d(0x7000 + combat::DURATION, 24);
        f.pointer(0x7000 + combat::ATTACKER, 0x8000);
        f.pointer(0x7000 + combat::DEFENDER, 0x9000);
        for (n, (p, node, list)) in [(0x8000, 0xa000, 0xc000), (0x9000, 0xb000, 0xd000)]
            .into_iter()
            .enumerate()
        {
            f.pointer(p, side::VTABLE_RVA.0);
            f.pointer(p + side::COMBAT, 0x7000);
            f.array(p + side::UNITS, list, 1);
            f.pointer(list, 0x2000);
            f.pointer(p + side::COUNTRIES, node);
            f.pointer(p + side::COUNTRIES + 8, node);
            f.d(p + side::COUNTRIES + 16, 1);
            f.d(node, 1);
            f.q(node + 0x10, 0);
            f.q(p + side::PROGRESS, 17);
            f.bytes[p + side::OUTCOME_FLAGS] = 2;
            let observer = p + side::STATS_OBSERVER;
            f.pointer(observer, stats::VTABLE_RVA.0);
            f.d(observer + stats::MANPOWER_LOSS, 73);
            let losses = observer + stats::EQUIPMENT_LOSSES;
            f.pointer(losses, pool::VTABLE_RVA.0);
            f.q(losses + pool::ITEMS, f.loss_entries[n].as_ptr() as u64);
            f.d(losses + pool::CAPACITY, variants as u32);
            f.d(losses + pool::COUNT, variants as u32);
        }
        f
    }
}

#[test]
#[ignore = "print-only guarded event snapshot bench on owned allocations; lead runs through build slot"]
fn land_event_snapshot_scale() {
    let variants: usize = std::env::var("BATTLEPLAN_LAND_LOSS_VARIANTS").or_else(|_| std::env::var("HOI4_LAND_LOSS_VARIANTS"))
        .map(|v| v.parse().unwrap())
        .unwrap_or(12);
    for countries in [1500, 5000] {
        for dispatched in [false, true] {
            let f = Fixture::new(countries, variants);
            assert_eq!(f.loss_variants.len(), variants);
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            const SAMPLES: usize = 201;
            let mut times = Vec::with_capacity(SAMPLES);
            let mut words = 0usize;
            for _ in 0..SAMPLES {
                let start = std::time::Instant::now();
                for _ in 0..countries {
                    let mut dispatch_words = 0;
                    let result = if dispatched {
                        let (header, _) = native::dispatch_inputs(
                            &memory,
                            f.base,
                            f.base + 0x7000,
                            f.base + 0x9000,
                            f.base + 0x8000,
                        )
                        .unwrap();
                        let participants =
                            native::ResultPlan::read_header(&memory, f.base, header, None)
                                .unwrap()
                                .participants();
                        participants
                            .copy(&memory, f.base, |word| {
                                std::hint::black_box(word);
                            })
                            .unwrap();
                        dispatch_words = participants.words + 6;
                        native::ResultPlan::read(&memory, f.base, f.base + 0x7000, None)
                            .unwrap()
                            .without_candidates()
                    } else {
                        native::ResultPlan::read(&memory, f.base, f.base + 0x7000, None).unwrap()
                    };
                    result
                        .copy(&memory, f.base, |word| {
                            std::hint::black_box(word);
                        })
                        .unwrap();
                    let losses =
                        native::LossesPlan::read(&memory, f.base, f.base + 0x7000).unwrap();
                    losses
                        .copy(&memory, f.base, |word| {
                            std::hint::black_box(word);
                        })
                        .unwrap();
                    let history =
                        native::HistoryPlan::read(&memory, f.base, f.base + 0x7000).unwrap();
                    history
                        .copy(&memory, |word| {
                            std::hint::black_box(word);
                        })
                        .unwrap();
                    let dead = native::DestructionPlan::read(
                        &memory,
                        f.base,
                        f.base + 0x4000,
                        f.base + 0x5000,
                        f.base + 0x2000,
                        0,
                    )
                    .unwrap();
                    words =
                        dispatch_words + result.words + losses.words + history.words + dead.words;
                    dead.copy(&memory, f.base, |word| {
                        std::hint::black_box(word);
                    })
                    .unwrap();
                }
                times.push(start.elapsed().as_nanos());
            }
            times.sort_unstable();
            println!(
                "LAND_SNAPSHOT countries={countries} variants/side={variants} samples={SAMPLES} dispatched={dispatched} bundles={countries} words_per_bundle={words} p50_us={} p95_us={} max_us={} per_bundle_p95_ns={} fixture_bytes={} profile={} scope=synthetic_guarded_result_losses_history_destruction_no_native_original_or_log_append",
                times[SAMPLES / 2] / 1000,
                times[(SAMPLES - 1) * 95 / 100] / 1000,
                times[SAMPLES - 1] / 1000,
                times[(SAMPLES - 1) * 95 / 100] / countries as u128,
                f.bytes.len()
                    + f.loss_entries
                        .iter()
                        .map(|v| v.capacity() * size_of::<stats::LossEntry>())
                        .sum::<usize>()
                    + f.loss_variants.capacity() * size_of::<[u64; 2]>(),
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "optimized"
                }
            );
        }
    }
}
