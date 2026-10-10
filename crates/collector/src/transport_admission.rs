//! Numerical admission for transport borrowers within the original callback.
//!
//! The hourly update receives the selected game-state object, advances the
//! clock only after entry and raises its busy flag for the mutation phase.
//! Compare that original callback argument with the original entry clock,
//! then bind each request to the same base/state/hour.
//! The OS main-thread ID is initialized once and remains read-only.
//! Require that original scalar at callback entry, without calling the native
//! ETS main-thread helper (which can allocate). Then require the original
//! authenticated idler's previous graphics task to be absent or completed:
//! The idler can execute commands before its wait, and its render panels
//! reach real convoy/stat writers. Serial command/UI/load writers cannot
//! run while the admitted main thread holds the callback. Joined simulation
//! tasks and other foreign task bodies must remain within the admitted interval.

use engine::{
    Memory,
    cgame_state::{self, Clock},
    cin_game_idler, ctask,
};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

type Fact<T> = Result<T, &'static str>;

const UNAVAILABLE: &str = "transport original callback witness unavailable";
const WORLD: &str = "transport original callback world mismatch";
const BUSY: &str = "transport original callback tick busy";
const REQUEST: &str = "transport request base world or hour mismatch";
const THREAD_UNAVAILABLE: &str = "transport native main thread witness unavailable";
const THREAD_ZERO: &str = "transport native main thread witness zero";
const THREAD_FOREIGN: &str = "transport callback is not the native main thread";
const REASONS: [&str; 20] = [
    UNAVAILABLE,
    WORLD,
    BUSY,
    REQUEST,
    THREAD_UNAVAILABLE,
    THREAD_ZERO,
    THREAD_FOREIGN,
    cin_game_idler::ROOT_UNAVAILABLE,
    cin_game_idler::ROOT_MISSING,
    cin_game_idler::ROOT_MISMATCH,
    cin_game_idler::IDENTITY_UNAVAILABLE,
    cin_game_idler::IDENTITY_UNSUPPORTED,
    cin_game_idler::TASK_UNAVAILABLE,
    ctask::WRAPPER_UNAVAILABLE,
    ctask::WRAPPER_UNSUPPORTED,
    ctask::ITEM_UNAVAILABLE,
    ctask::ITEM_MISSING,
    ctask::STATE_UNAVAILABLE,
    ctask::OUTSTANDING,
    ctask::STATE_UNSUPPORTED,
];
static REFUSALS: [AtomicU64; REASONS.len()] = [const { AtomicU64::new(0) }; REASONS.len()];

#[derive(Debug, Clone, Copy)]
struct Pass {
    base: usize,
    state: usize,
    hour: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Admission(Fact<Pass>);
impl Default for Admission {
    fn default() -> Self {
        Self(Err(UNAVAILABLE))
    }
}
impl Admission {
    pub(crate) fn check(self, base: usize, state: usize, hour: i32) -> Fact<()> {
        let result = self.0.and_then(|pass| {
            if pass.base == base && pass.state == state && pass.hour == hour {
                Ok(())
            } else {
                Err(REQUEST)
            }
        });
        if let Err(reason) = result
            && let Some(index) = REASONS.iter().position(|candidate| *candidate == reason)
        {
            REFUSALS[index].fetch_add(1, Ordering::Relaxed);
        }
        result
    }
}

thread_local! {
    static CALLBACK: Cell<Fact<Pass>> = const { Cell::new(Err(UNAVAILABLE)) };
}

/// The previous callback witness is restored for nested callback scopes.
/// A request copies its numerical verdict before moving to a worker.
pub(crate) struct Scope(Fact<Pass>);
impl Scope {
    pub(crate) fn enter<T>(
        memory: &impl Memory,
        base: usize,
        native_state: usize,
        entry: Option<Clock<T>>,
    ) -> Self {
        let recorded_thread = cgame_state::read_main_thread_id(memory, base);
        let current_thread = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
        let value = thread_witness(recorded_thread, current_thread)
            .and_then(|()| witness(base, native_state, entry))
            .and_then(|pass| {
                cin_game_idler::read_graphics_task_completed(memory, base).map(|()| pass)
            });
        Self(CALLBACK.replace(value))
    }

    #[cfg(test)]
    pub(crate) fn enter_fixture<T>(
        base: usize,
        native_state: usize,
        entry: Option<Clock<T>>,
    ) -> Self {
        Self::enter_full_witness(base, native_state, entry, Some(7), 7, Ok(()))
    }

    #[cfg(test)]
    fn enter_witness<T>(
        base: usize,
        native_state: usize,
        entry: Option<Clock<T>>,
        recorded_thread: Option<u32>,
        current_thread: u32,
    ) -> Self {
        Self::enter_full_witness(
            base,
            native_state,
            entry,
            recorded_thread,
            current_thread,
            Ok(()),
        )
    }

    #[cfg(test)]
    fn enter_full_witness<T>(
        base: usize,
        native_state: usize,
        entry: Option<Clock<T>>,
        recorded_thread: Option<u32>,
        current_thread: u32,
        graphics: Fact<()>,
    ) -> Self {
        let value = thread_witness(recorded_thread, current_thread)
            .and_then(|()| witness(base, native_state, entry))
            .and_then(|pass| graphics.map(|()| pass));
        Self(CALLBACK.replace(value))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CALLBACK.set(self.0);
    }
}

fn thread_witness(recorded: Option<u32>, current: u32) -> Fact<()> {
    match recorded {
        None => Err(THREAD_UNAVAILABLE),
        Some(0) => Err(THREAD_ZERO),
        Some(thread) if thread == current => Ok(()),
        Some(_) => Err(THREAD_FOREIGN),
    }
}

fn witness<T>(base: usize, native_state: usize, entry: Option<Clock<T>>) -> Fact<Pass> {
    let clock = entry.ok_or(UNAVAILABLE)?;
    if native_state == 0 || usize::try_from(clock.game_state).ok() != Some(native_state) {
        return Err(WORLD);
    }
    if clock.tick_busy != 0 {
        return Err(BUSY);
    }
    Ok(Pass {
        base,
        state: native_state,
        hour: clock.hour,
    })
}

pub(crate) fn current(base: usize, state: usize, hour: i32) -> Admission {
    Admission(CALLBACK.get().and_then(|pass| {
        if pass.base == base && pass.state == state && pass.hour == hour {
            Ok(pass)
        } else {
            Err(REQUEST)
        }
    }))
}

pub(crate) fn status() -> impl serde::Serialize {
    crate::status::status!({ "refusals": crate::status::map(REASONS.into_iter().zip(
        REFUSALS.iter().map(|count| count.load(Ordering::Relaxed)))) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(state: usize, hour: i32, busy: u8) -> Clock<()> {
        Clock {
            game_state: state as u64,
            hour,
            tick_busy: busy,
            play_time: 0,
            play_session_started_at: 7,
            seed: 0,
            campaign_id: (),
            speed: 0,
        }
    }

    fn observe(base: usize, state: usize, hour: i32) -> Fact<()> {
        current(base, state, hour).check(base, state, hour)
    }

    #[test]
    fn transport_admission_requires_exact_callback_world_hour_and_idle_entry() {
        assert_eq!(observe(1, 2, 3), Err(UNAVAILABLE));
        let _scope = Scope::enter_witness(1, 2, Some(clock(2, 3, 0)), Some(7), 7);
        assert_eq!(observe(1, 2, 3), Ok(()));
        for request in [(9, 2, 3), (1, 9, 3), (1, 2, 9)] {
            assert_eq!(observe(request.0, request.1, request.2), Err(REQUEST));
        }
        for (entry, reason) in [
            (None, UNAVAILABLE),
            (Some(clock(9, 3, 0)), WORLD),
            (Some(clock(2, 3, 1)), BUSY),
            (Some(clock(2, 3, 255)), BUSY),
        ] {
            let scope = Scope::enter_witness(1, 2, entry, Some(7), 7);
            assert_eq!(observe(1, 2, 3), Err(reason));
            drop(scope);
            assert_eq!(observe(1, 2, 3), Ok(()), "nested refusal retires at return");
        }
    }

    #[test]
    fn transport_admission_retires_between_callback_and_same_hour_new_world() {
        let admitted;
        {
            let _scope = Scope::enter_witness(1, 2, Some(clock(2, 3, 0)), Some(7), 7);
            admitted = current(1, 2, 3);
        }
        assert_eq!(
            admitted.check(1, 2, 3),
            Ok(()),
            "worker owns only the copied numerical key"
        );
        assert_eq!(
            admitted.check(1, 9, 3),
            Err(REQUEST),
            "worker validates its own request key"
        );
        assert_eq!(admitted.check(1, 2, 9), Err(REQUEST));
        assert_eq!(observe(1, 2, 3), Err(UNAVAILABLE));
        let _scope = Scope::enter_witness(1, 9, Some(clock(9, 3, 0)), Some(7), 7);
        assert_eq!(observe(1, 2, 3), Err(REQUEST));
        assert_eq!(observe(1, 9, 3), Ok(()));
    }

    #[test]
    fn transport_admission_requires_original_nonzero_native_main_thread() {
        for (recorded, current, expected) in [
            (None, 7, THREAD_UNAVAILABLE),
            (Some(0), 7, THREAD_ZERO),
            (Some(7), 8, THREAD_FOREIGN),
            (Some(7), 0, THREAD_FOREIGN),
        ] {
            let _scope = Scope::enter_witness(1, 2, Some(clock(2, 3, 0)), recorded, current);
            assert_eq!(observe(1, 2, 3), Err(expected));
        }
        let _scope = Scope::enter_witness(1, 2, Some(clock(2, 3, 0)), Some(7), 7);
        assert_eq!(observe(1, 2, 3), Ok(()));
    }

    #[test]
    fn transport_admission_refuses_original_foreign_graphics_writer_overlap() {
        let _scope = Scope::enter_witness(1, 2, Some(clock(2, 3, 0)), Some(7), 7);
        for reason in REASONS[7..].iter().copied() {
            let scope =
                Scope::enter_full_witness(1, 2, Some(clock(2, 3, 0)), Some(7), 7, Err(reason));
            assert_eq!(
                observe(1, 2, 3),
                Err(reason),
                "idle/main-thread entry does not exclude a graphics writer"
            );
            let numerical = current(1, 2, 3);
            drop(scope);
            assert_eq!(observe(1, 2, 3), Ok(()));
            assert_eq!(
                numerical.check(1, 2, 3),
                Err(reason),
                "later task completion cannot heal the original refusal"
            );
        }
    }
}
