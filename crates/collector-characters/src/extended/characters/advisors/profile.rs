use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
const LIMIT: usize = 512;
const WIDTH: usize = 11;
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Slot {
    ready: AtomicUsize,
    fields: [AtomicU64; WIDTH * 2],
}
static SLOTS: [Slot; LIMIT] = [const {
    Slot {
        ready: AtomicUsize::new(0),
        fields: [const { AtomicU64::new(0) }; WIDTH * 2],
    }
}; LIMIT];
pub(super) struct Stamp {
    wall: i64,
    cycles: u64,
}
fn cycles() -> u64 {
    unsafe extern "system" {
        fn QueryThreadCycleTime(
            thread: windows_sys::Win32::Foundation::HANDLE,
            cycles: *mut u64,
        ) -> i32;
    }
    let mut value = 0;
    unsafe {
        QueryThreadCycleTime(
            windows_sys::Win32::System::Threading::GetCurrentThread(),
            &mut value,
        );
    }
    value
}
impl Stamp {
    pub fn start() -> Self {
        Self {
            wall: collector_primitives::clock::qpc(),
            cycles: cycles(),
        }
    }
    pub fn finish(self) -> [u64; 2] {
        let elapsed_cycles = cycles().saturating_sub(self.cycles);
        [
            collector_primitives::clock::qpc()
                .saturating_sub(self.wall)
                .max(0) as u64,
            elapsed_cycles,
        ]
    }
}
pub(super) fn record(samples: [[u64; WIDTH]; 2]) {
    let index = NEXT.fetch_add(1, Ordering::Relaxed);
    if let Some(slot) = SLOTS.get(index) {
        for (field, value) in slot.fields.iter().zip(samples.into_iter().flatten()) {
            field.store(value, Ordering::Relaxed);
        }
        slot.ready.store(1, Ordering::Release);
    }
}
pub fn status() -> impl serde::Serialize {
    let reserved = NEXT.load(Ordering::Relaxed).min(LIMIT);
    let ready = std::array::from_fn::<_, LIMIT, _>(|index| {
        index < reserved && SLOTS[index].ready.load(Ordering::Acquire) != 0
    });
    let samples =
        collector_status::status::sequence(SLOTS.iter().zip(ready).filter_map(|(slot, ready)| {
            ready.then(|| {
                slot.fields
                    .each_ref()
                    .map(|field| field.load(Ordering::Relaxed))
            })
        }));
    collector_status::status::status!({
        "columns_per_section": ["wall_ticks", "thread_cycles", "read_characters", "reused_characters",
            "planned", "plan_wall_ticks", "plan_thread_cycles", "plan_result", "stop_address", "stop_len", "characters"],
        "section_order": ["historical", "generated"],
        "plan_result": {"0":"not attempted", "1":"complete", "2":"control changed", "3":"read failed", "4":"unavailable"},
        "scope": "First bounded captures in acquisition order, including initial attachment; wall is QPC ticks; cycles exclude time off CPU and are not converted to milliseconds",
        "samples": samples, "observations": NEXT.load(Ordering::Relaxed)
    })
}
