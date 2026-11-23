use super::*;
use replay_data::scope::CaptureMode;
use std::sync::atomic::AtomicUsize;

pub const ID: &[u8] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";

#[derive(Clone, Copy, Debug)]
pub enum Fixture {
    Divisions,
    Wars,
}

#[derive(Clone, Copy, Debug)]
pub struct Read {
    pub role: Option<collector_workers::roles::Role>,
    pub address: usize,
    pub len: usize,
    pub accepted: bool,
}

struct World {
    bytes: Vec<u8>,
    hour: AtomicUsize,
    fault: Option<usize>,
    changing: (usize, i64),
    reads: Mutex<Vec<Read>>,
    roles: Mutex<Vec<collector_workers::roles::Role>>,
}

struct Traced<'a> {
    world: &'a World,
    role: Option<collector_workers::roles::Role>,
}
impl Memory for Traced<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let refused = matches!(self.world.hour.load(Ordering::Acquire), 2 | 13)
            && self
                .world
                .fault
                .is_some_and(|fault| address <= fault && fault - address < output.len());
        let accepted = !refused
            && engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &self.world.bytes,
            }
            .copy(address, output);
        if accepted {
            let (field, initial) = self.world.changing;
            let value = (initial + self.world.hour.load(Ordering::Acquire) as i64).to_le_bytes();
            let begin = address.max(field);
            let end = address
                .saturating_add(output.len())
                .min(field + value.len());
            if begin < end {
                output[begin - address..end - address]
                    .copy_from_slice(&value[begin - field..end - field]);
            }
        }
        self.world.reads.lock().unwrap().push(Read {
            role: self.role,
            address,
            len: output.len(),
            accepted,
        });
        accepted
    }
}

/// # Safety
/// Run only in an isolated fixture that owns the process-wide capture, observer and decode state.
pub unsafe fn capture(
    mode: CaptureMode,
    fixture: Fixture,
    refuse: bool,
    mut visit: impl FnMut(&Captured, &[Read], &[collector_workers::roles::Role]),
) {
    let (bytes, fault) = match fixture {
        Fixture::Divisions => (
            divisions_test_kit::fixture(),
            divisions_test_kit::C0 + ccountry::DIVISIONS,
        ),
        Fixture::Wars => (
            collector_states::wars_test_kit::fixture(),
            collector_states::wars_test_kit::C0 + ccountry::DIPLOMACY_STATUS,
        ),
    };
    let field = match fixture {
        Fixture::Divisions => {
            let header = divisions_test_kit::A1
                + engine_native::carmy::EQUIPMENT_POOL
                + engine_native::cequipment_variant::pool::ITEMS;
            u64::from_le_bytes(bytes[header..header + 8].try_into().unwrap()) as usize + 8
        }
        Fixture::Wars => {
            collector_states::wars_test_kit::WAR2 + engine_native::cwar_relation::FIRST_CASUALTIES
        }
    };
    let initial = i64::from_le_bytes(bytes[field..field + 8].try_into().unwrap());
    let world = Arc::new(World {
        changing: (field, initial),
        bytes,
        hour: AtomicUsize::new(0),
        fault: refuse.then_some(fault),
        reads: Mutex::new(Vec::new()),
        roles: Mutex::new(Vec::new()),
    });
    let _payloads =
    // SAFETY: Attach owns payload storage through worker join. Fixture processes own their test
    // pools.
    unsafe { decode::prepare_payload_pools(true) }.unwrap();
    let mut sizing = crate::country_scale::Sizing::for_slots(5);
    sizing.core[1] = 8;
    sizing.core[3] = 8;
    sizing.core[4] = 64;
    sizing.core[5] = 64;
    let mut shared =
    // SAFETY: This is unpublished attach or isolated fixture state, with no competing observer
    // boundary or detach.
    unsafe { Shared::with_fixture_sizing(0, 0, 10_000_000, &sizing, 0) }.unwrap();
    shared.capture_mode = mode;
    // SAFETY: This is the unpublished attachment's own roster.
    unsafe {
        shared.prepare_roster(&Traced {
            world: &world,
            role: None,
        })
    };
    shared.prepare_clock_len(ID.len());
    let worker_world = Arc::clone(&world);
    shared
        .acquisition
        .set(acquisition::Pool::with_test_runner(2, move |job| {
            let role = job.role();
            worker_world.roles.lock().unwrap().push(role);
            acquisition::run_for_test(
                job,
                &CountedMemory::new(Traced {
                    world: &worker_world,
                    role: Some(role),
                }),
            )
        }))
        .ok()
        .unwrap();
    let mut memo = collector_air::extended::air::AirBasesMemo::default();
    for hour in 0..=26 {
        world.hour.store(hour as usize, Ordering::Release);
        world.reads.lock().unwrap().clear();
        world.roles.lock().unwrap().clear();
        if hour == 13 {
            shared.refresh_next.store(true, Ordering::Release);
        }
        let index =
        // SAFETY: This is the admitted hour or isolated fixture taking exclusive ownership of an
        // installed slot.
        unsafe { shared.take_frame(&mut 0) }.unwrap();
        let reads = collector_memory::read_accounting::Capture::new();
        unsafe {
            acquire_with_memory(
                &shared,
                Dispatch {
                    world_generation: collector_lifetime::world_lifetime::generation(),
                    campaign_epoch: 1,
                    index: Some(index),
                    entry: Some(Clock {
                        game_state: divisions_test_kit::GAME_STATE as u64,
                        hour,
                        play_time: 1,
                        play_session_started_at: 1234,
                        seed: 17,
                        campaign_id: ID,
                        speed: 5,
                        tick_busy: 0,
                    }),
                    entry_ticks: 0,
                    dispatched: qpc(),
                    phases: HourPhases::default(),
                    full: false,
                },
                CountedMemory::new(Traced {
                    world: &world,
                    role: None,
                }),
            );
        }
        let _ = reads.finish();
        decode::drain_for_test(&shared, &mut memo);
        assert_eq!(
            // SAFETY: The sole spool consumer takes responsibility for the decoded slot until recycle.
            unsafe { shared.frames.pop_ready() },
            Some(index)
        );
        let frame = unsafe { &*shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(frame.clock.as_ref().unwrap().hour, hour);
        assert_eq!(frame.force_full, matches!(hour, 0 | 13));
        let spans = std::mem::take(&mut *world.reads.lock().unwrap());
        let roles = std::mem::take(&mut *world.roles.lock().unwrap());
        assert!(spans.iter().any(|span| span.accepted && span.len != 0));
        if refuse && matches!(hour, 2 | 13) {
            assert!(
                spans
                    .iter()
                    .any(|span| span.address == fault && !span.accepted)
            );
        }
        visit(frame, &spans, &roles);
        // SAFETY: This owner has finished with the installed slot and its helpers have joined.
        unsafe { shared.recycle_frame_prepared(index) };
    }
    let pool = shared.acquisition.get().unwrap();
    // SAFETY: This is the pool's owner. No submitted work is still borrowing its storage at
    // shutdown.
    unsafe { pool.shutdown() }.unwrap();
    assert_eq!(pool.calibration().executor.panicked, 0);
    assert_eq!(shared.acquisition_panics.load(Ordering::Acquire), 0);
}
