use crate::acquisition;

#[derive(Clone, Copy, Debug, Default)]
pub struct JoinSample {
    pub role: Option<usize>,
    pub wall_ns: u64,
    pub readiness: bool,
    pub profile: acquisition::Profile,
}

thread_local! {
    static JOIN: std::cell::Cell<Option<JoinSample>> = const { std::cell::Cell::new(None) };
}

pub fn reset_join() {
    JOIN.set(None);
}

pub fn record_join(role: usize, wall_ns: u64, profile: crate::acquisition::Profile) {
    keep(JoinSample {
        role: Some(role),
        wall_ns,
        readiness: false,
        profile,
    });
}

pub(crate) fn record_readiness(ticks: i64) {
    let mut frequency = 0;
    unsafe { windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency) };
    keep(JoinSample {
        wall_ns: (ticks.max(0) as u128 * 1_000_000_000 / frequency.max(1) as u128) as u64,
        readiness: true,
        ..Default::default()
    });
}

fn keep(sample: JoinSample) {
    if JOIN.get().is_none_or(|join| sample.wall_ns > join.wall_ns) {
        JOIN.set(Some(sample));
    }
}

pub fn take_join() -> JoinSample {
    JOIN.take().unwrap_or_default()
}

pub fn thread_cycles() -> Option<u64> {
    unsafe extern "system" {
        fn QueryThreadCycleTime(
            thread: windows_sys::Win32::Foundation::HANDLE,
            cycles: *mut u64,
        ) -> i32;
    }
    let mut cycles = 0;
    let ok = unsafe {
        QueryThreadCycleTime(
            windows_sys::Win32::System::Threading::GetCurrentThread(),
            &mut cycles,
        )
    };
    (ok != 0).then_some(cycles)
}

pub(crate) const JOIN_SCALARS: usize = 15;

impl JoinSample {
    pub fn fields(self) -> [u64; JOIN_SCALARS] {
        [
            u64::from(self.role.is_some()),
            self.role.unwrap_or(0) as u64,
            self.wall_ns,
            self.profile.queued_qpc as u64,
            self.profile.claimed_qpc as u64,
            self.profile.runner_begin_qpc as u64,
            self.profile.runner_end_qpc as u64,
            self.profile.published_qpc as u64,
            self.profile.collected_qpc as u64,
            self.profile.submit_lock_wait_ticks,
            self.profile.finish_lock_wait_ticks,
            u64::from(self.profile.runner_thread_cycles.is_some()),
            self.profile.runner_thread_cycles.unwrap_or(0),
            u64::from(self.profile.runner_thread_id),
            u64::from(self.readiness),
        ]
    }

    pub(crate) fn from_fields(fields: [u64; JOIN_SCALARS]) -> Self {
        Self {
            role: (fields[0] != 0).then_some(fields[1] as usize),
            wall_ns: fields[2],
            readiness: fields[14] != 0,
            profile: crate::acquisition::Profile {
                queued_qpc: fields[3] as i64,
                claimed_qpc: fields[4] as i64,
                runner_begin_qpc: fields[5] as i64,
                runner_end_qpc: fields[6] as i64,
                published_qpc: fields[7] as i64,
                collected_qpc: fields[8] as i64,
                submit_lock_wait_ticks: fields[9],
                finish_lock_wait_ticks: fields[10],
                runner_thread_cycles: (fields[11] != 0).then_some(fields[12]),
                runner_thread_id: fields[13] as u32,
            },
        }
    }

    pub fn json(self) -> impl serde::Serialize {
        let label = self
            .role
            .and_then(|index| crate::roles::ROLES.get(index))
            .map(collector_status::status::DebugValue);
        collector_status::status::status!({
            "role": self.role,
            "role_label": label,
            "kind": if self.readiness { "readiness" } else { "join" },
            "wall_ns": self.wall_ns,
            "queued_qpc": (!self.readiness).then_some(self.profile.queued_qpc),
            "claimed_qpc": (!self.readiness).then_some(self.profile.claimed_qpc),
            "runner_begin_qpc": (!self.readiness).then_some(self.profile.runner_begin_qpc),
            "runner_end_qpc": (!self.readiness).then_some(self.profile.runner_end_qpc),
            "published_qpc": (!self.readiness).then_some(self.profile.published_qpc),
            "collected_qpc": (!self.readiness).then_some(self.profile.collected_qpc),
            "submit_lock_wait_ticks": (!self.readiness).then_some(self.profile.submit_lock_wait_ticks),
            "finish_lock_wait_ticks": (!self.readiness).then_some(self.profile.finish_lock_wait_ticks),
            "runner_thread_cycles": (!self.readiness).then_some(self.profile.runner_thread_cycles).flatten(),
            "runner_thread_id": (!self.readiness).then_some(self.profile.runner_thread_id),
        })
    }
}
