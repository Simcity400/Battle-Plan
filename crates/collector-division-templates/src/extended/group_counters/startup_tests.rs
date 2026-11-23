use super::*;
use engine_native::cequipment_variant::pool as cequipment_variant_pool;
use engine_native_parts::{creference_registry as registry, reference_bucket as bucket};
use engine_native_land::template_hq_context as hq;
use replay_capture_spool::spool::{Record, encode_record};
use replay_frame::{
    frame::FrameDecoder, frame::FrameEncoder, frame::FrameInput, frame::Metrics, frame::TableInput,
};

const TABLE: usize = 0x10000;
const BUCKETS: usize = TABLE + 0x100;
const GROUPS: usize = 0x100000;
const GROUP_STRIDE: usize = 0x1000;
const HOLDER_IN_GROUP: usize = 0x400;
const HOLDER_PERSISTENT: usize = 0x18;
const HOLDER_OWNER: usize = 0x20;
const HOLDER_EQUIPMENT: usize = 0x28;
const HOLDER_REQUIREMENTS: usize = 0x68;
const HOLDER_ORDER: usize = 0x90;
const REQUIRED_POOL_VTABLE: u64 = 0x0280_7538;
const TEMPLATE: u64 = (1 << 32) | 52;

struct World(Vec<u8>);
impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8
            && output.len() == 8
        {
            output.copy_from_slice(&(TABLE as u64).to_le_bytes());
            return true;
        }
        engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &self.0,
        }
        .copy(address, output)
    }
}
impl World {
    fn word(&mut self, at: usize, value: u64) {
        self.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn new(count: usize) -> Self {
        let capacity = count
            .next_power_of_two()
            .max(1 << registry::MINIMUM_EXPONENT);
        let distance = capacity.trailing_zeros() as u8 + 2;
        let mut world = Self(vec![0; GROUPS + count * GROUP_STRIDE]);
        world.word(TABLE + registry::ENTRIES, BUCKETS as u64);
        world.word(
            TABLE + registry::COUNT,
            count as u64 | (((capacity - 1) as u64) << 32),
        );
        world.0[TABLE + registry::OVERFLOW_DISTANCE] = distance;
        world.0[BUCKETS + (capacity + distance as usize) * bucket::STRIDE + bucket::DISTANCE] =
            bucket::TERMINAL_DISTANCE;
        for ordinal in 0..count {
            let group = GROUPS + ordinal * GROUP_STRIDE;
            let holder = group + HOLDER_IN_GROUP;
            let handle = ((ordinal as u64 + 1) << 32) | u64::from(corders_group::REFERENCE_TYPE);
            let entry = BUCKETS + ordinal * bucket::STRIDE;
            world.0[entry + bucket::DISTANCE] = 1;
            for (at, value) in [
                (entry + bucket::TYPE, handle),
                (entry + bucket::OBJECT, group as u64),
                (group, corders_group::VTABLE_RVA.0 as u64),
                (group + corders_group::HANDLE, handle),
                (group + corders_group::COUNTRY, 1),
                (group + hq::HOLDER, holder as u64),
                (group + hq::TARGET, TEMPLATE),
                (holder, hq::HOLDER_VTABLE_RVA.0 as u64),
                (
                    holder + HOLDER_PERSISTENT,
                    hq::HOLDER_PERSISTENT_VTABLE_RVA.0 as u64,
                ),
                (holder + HOLDER_OWNER, 1),
                (
                    holder + HOLDER_EQUIPMENT,
                    cequipment_variant_pool::VTABLE_RVA.0 as u64,
                ),
                (holder + HOLDER_REQUIREMENTS, REQUIRED_POOL_VTABLE),
                (holder + HOLDER_ORDER, ordinal as u64 + 1),
            ] {
                world.word(at, value);
            }
        }
        world
    }
}

#[test]
fn enabled_unsized_startup_sizes_all_hq_receivers_and_spools_known_continuous_contexts() {
    use crate::template_changes::Changes;
    const TEST: &str = "extended::group_counters::startup_tests::enabled_unsized_startup_sizes_all_hq_receivers_and_spools_known_continuous_contexts";
    const CHILD: &str = "BATTLEPLAN_HQ_UNSIZED_STARTUP_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let count = 1500;
    let world = World::new(count);
    let observer = crate::template_changes::unsized_observer_for_tests();
    assert!(observer.quiet().is_some());
    assert!(observer.sequence(GROUPS).is_none());
    crate::template_changes::prepare(1, 1);
    let mut capacity = Capacities::default();
    // SAFETY: test runs in a dedicated child process with no live template hooks.
    unsafe { capacity.presize(&world, 0, 0) }.unwrap();
    assert_eq!(capacity.rows, count);
    crate::template_changes::boundary(true);
    for receiver in [0x5000, 1, 0x8400, 0x8000] {
        assert!(observer.sequence(receiver).is_some());
    }
    let mut capture = Captured::with_capacities(&capacity);
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..3 {
        if hour != 0 {
            crate::template_changes::prepare(0, 1);
            // SAFETY: test runs in a dedicated child process with no live template hooks.
            unsafe { crate::template_changes::presize() };
            crate::template_changes::boundary(false);
        }
        capture.prepare_hour(hour == 0);
        // SAFETY: Unit test runs against an isolated fixture.
        unsafe { capture.capture(&world, 0, 0) };
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: count }]);
        let mut input = FrameInput {
            branch: 1,
            hour,
            tables: std::array::from_fn(|_| {
                TableInput::Sampled(collector_projection::table_conversion::unavailable(
                    "unused test table",
                ))
            }),
            metrics: Metrics::default(),
        };
        let [table] = capture.tables();
        input.tables[TableId::OrdersGroupCounterInputs.index()] = TableInput::Sampled(table);
        let mut prepared = encoder.prepare_bytes(input, hour == 0).unwrap();
        let frame = prepared.take_frame().decode().unwrap();
        assert_eq!(frame.keyframe, hour == 0);
        assert!(
            !encode_record(&Record::Frame(frame.clone()))
                .unwrap()
                .is_empty()
        );
        encoder.commit(prepared).unwrap();
        let decoded = decoder.apply(&frame).unwrap();
        let rows = decoded
            .complete::<tables::OrdersGroupCounterInputs>()
            .unwrap();
        assert_eq!(rows.len(), count);
        for (ordinal, row) in rows.iter().enumerate() {
            let context = row
                .template_context
                .require("HQ template context")
                .unwrap()
                .as_ref()
                .unwrap_or_else(|| {
                    panic!("readable HQ context must be known at hour {hour}, group {ordinal}")
                });
            assert_eq!(context.target, TEMPLATE);
            let deployment = context
                .deployment
                .as_ref()
                .expect("readable holder must be known");
            assert_eq!(deployment.country, 1);
            assert_eq!(deployment.order, ordinal as i32 + 1);
        }
    }
}

#[test]
fn group_sizing_prepares_retreat_ban_watches_before_the_first_capture() {
    use collector_observers::retreat_ban_changes::{Changes, Observer};
    const COUNT: usize = 5_000;
    let world = World::new(COUNT);
    collector_observers::retreat_ban_changes::forwarder(true);
    assert!(Observer.sequence(GROUPS).is_none());
    let mut capacity = Capacities::default();
    // SAFETY: synthetic memory fixture with no live group-counter observer hooks.
    unsafe { capacity.presize(&world, 0, 0) }.unwrap();
    collector_observers::retreat_ban_changes::boundary(true);
    for ordinal in 0..COUNT {
        assert!(Observer.sequence(GROUPS + ordinal * GROUP_STRIDE).is_some());
    }
}
