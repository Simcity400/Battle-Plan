//! Viewer-local application frames following the a729 CApplication counter law.
//!
//! This is a fresh viewer runtime, not the original game's historical frame
//! prefix or SDL provider. Native custody: 142219f70 entry/exit, 1422164e0
//! application construction, and image-global 143468304.
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationFrameError {
    Invalidated,
    WrongThread,
    ExpiredInvocation,
    ForeignApplication,
    OutOfOrderExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationFrameScope {
    ViewerLocalGraphicsRuntime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginalGameFrameQualification {
    UnresolvedHistoricalPrefixAndSdlProvider,
}

impl std::fmt::Display for ApplicationFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "viewer application frame: {self:?}")
    }
}
impl std::error::Error for ApplicationFrameError {}

struct RuntimeState {
    stamp: u32,
    active: Vec<ActiveInvocation>,
    invalidated: bool,
}

struct ActiveInvocation {
    token: Arc<()>,
    counter: Arc<Mutex<u64>>,
}

/// Image-global stamp custody. Creating another application does not reset it.
pub struct ViewerGraphicsRuntime {
    state: Arc<Mutex<RuntimeState>>,
    thread: ThreadId,
}

impl ViewerGraphicsRuntime {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RuntimeState {
                stamp: 0,
                active: Vec::new(),
                invalidated: false,
            })),
            thread: thread::current().id(),
        }
    }

    pub fn new_application(&self) -> ViewerApplication {
        ViewerApplication {
            state: Arc::clone(&self.state),
            thread: self.thread,
            counter: Arc::new(Mutex::new(1)),
        }
    }
}

impl Default for ViewerGraphicsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Application+330 custody, independent of adapters, roots and presentation.
pub struct ViewerApplication {
    state: Arc<Mutex<RuntimeState>>,
    counter: Arc<Mutex<u64>>,
    thread: ThreadId,
}

#[derive(Clone)]
pub struct ApplicationFrameWitness {
    state: Arc<Mutex<RuntimeState>>,
    counter: Arc<Mutex<u64>>,
    invocation: Arc<()>,
    thread: ThreadId,
}

/// Retained objects belong to the application, rather than the invocation
/// that last prepared them. This custody survives a nested frame's return.
#[derive(Clone)]
pub(crate) struct ApplicationFrameOwner {
    state: Arc<Mutex<RuntimeState>>,
    counter: Arc<Mutex<u64>>,
    thread: ThreadId,
}

impl ApplicationFrameOwner {
    pub(crate) fn published_stamp(&self) -> Result<u32, ApplicationFrameError> {
        if thread::current().id() != self.thread {
            return Err(ApplicationFrameError::WrongThread);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| ApplicationFrameError::Invalidated)?;
        if state.invalidated {
            return Err(ApplicationFrameError::Invalidated);
        }
        if !state
            .active
            .iter()
            .any(|active| Arc::ptr_eq(&active.counter, &self.counter))
        {
            return Err(ApplicationFrameError::ExpiredInvocation);
        }
        Ok(state.stamp)
    }

    pub(crate) fn check(
        &self,
        frame: &ApplicationFrameWitness,
    ) -> Result<u32, ApplicationFrameError> {
        let stamp = frame.published_stamp()?;
        if !Arc::ptr_eq(&self.state, &frame.state) || !Arc::ptr_eq(&self.counter, &frame.counter) {
            return Err(ApplicationFrameError::ForeignApplication);
        }
        Ok(stamp)
    }
}

impl ApplicationFrameWitness {
    pub fn scope(&self) -> ApplicationFrameScope {
        ApplicationFrameScope::ViewerLocalGraphicsRuntime
    }

    pub fn original_game_qualification(&self) -> OriginalGameFrameQualification {
        OriginalGameFrameQualification::UnresolvedHistoricalPrefixAndSdlProvider
    }

    /// Read the live shared S, including a publication made by a nested frame.
    pub fn published_stamp(&self) -> Result<u32, ApplicationFrameError> {
        if thread::current().id() != self.thread {
            return Err(ApplicationFrameError::WrongThread);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| ApplicationFrameError::Invalidated)?;
        if state.invalidated {
            return Err(ApplicationFrameError::Invalidated);
        }
        if !state
            .active
            .iter()
            .any(|active| Arc::ptr_eq(&active.token, &self.invocation))
        {
            return Err(ApplicationFrameError::ExpiredInvocation);
        }
        Ok(state.stamp)
    }

    pub(crate) fn application_owner(&self) -> ApplicationFrameOwner {
        ApplicationFrameOwner {
            state: Arc::clone(&self.state),
            counter: Arc::clone(&self.counter),
            thread: self.thread,
        }
    }
}

/// Only an actual event-loop disposition completes an invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationFrameExit {
    Normal,
    Quit,
}

pub struct ApplicationFrame {
    witness: ApplicationFrameWitness,
    finished: bool,
}

impl ViewerApplication {
    #[cfg(test)]
    pub(crate) fn set_counter_for_test(&self, counter: u64) {
        assert_ne!(counter, 0);
        assert!(self.state.lock().unwrap().active.is_empty());
        *self.counter.lock().unwrap() = counter;
    }

    pub fn enter_frame(&self) -> Result<ApplicationFrame, ApplicationFrameError> {
        if thread::current().id() != self.thread {
            return Err(ApplicationFrameError::WrongThread);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| ApplicationFrameError::Invalidated)?;
        if state.invalidated {
            return Err(ApplicationFrameError::Invalidated);
        }
        let counter = self
            .counter
            .lock()
            .map_err(|_| ApplicationFrameError::Invalidated)?;
        // 14221a036 publishes before event dispatch. Do not save an entry C
        // to use at exit: nested invocations can increment the live counter.
        state.stamp = *counter as u32;
        let invocation = Arc::new(());
        state.active.push(ActiveInvocation {
            token: Arc::clone(&invocation),
            counter: Arc::clone(&self.counter),
        });
        Ok(ApplicationFrame {
            witness: ApplicationFrameWitness {
                state: Arc::clone(&self.state),
                counter: Arc::clone(&self.counter),
                invocation,
                thread: self.thread,
            },
            finished: false,
        })
    }
}

impl ApplicationFrame {
    pub fn witness(&self) -> &ApplicationFrameWitness {
        &self.witness
    }

    pub fn finish(mut self, exit: ApplicationFrameExit) -> Result<(), ApplicationFrameError> {
        if thread::current().id() != self.witness.thread {
            return Err(ApplicationFrameError::WrongThread);
        }
        let mut state = self
            .witness
            .state
            .lock()
            .map_err(|_| ApplicationFrameError::Invalidated)?;
        if state.invalidated {
            return Err(ApplicationFrameError::Invalidated);
        }
        if !state
            .active
            .last()
            .is_some_and(|active| Arc::ptr_eq(&active.token, &self.witness.invocation))
        {
            state.invalidated = true;
            state.active.clear();
            return Err(ApplicationFrameError::OutOfOrderExit);
        }
        if exit == ApplicationFrameExit::Normal {
            let mut counter = self
                .witness
                .counter
                .lock()
                .map_err(|_| ApplicationFrameError::Invalidated)?;
            *counter = counter.wrapping_add(1);
            if *counter == 0 {
                *counter = 1;
            }
        }
        state.active.pop();
        // S is deliberately not restored when returning to an outer frame.
        self.finished = true;
        Ok(())
    }

    /// Portable execution errors have no proved native quit correspondence.
    pub fn invalidate(self) {
        drop(self);
    }
}

impl Drop for ApplicationFrame {
    fn drop(&mut self) {
        if !self.finished {
            if let Ok(mut state) = self.witness.state.lock() {
                state.invalidated = true;
                state.active.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_frames_leave_live_stamp_and_increment_live_counter() {
        let runtime = ViewerGraphicsRuntime::new();
        let app = runtime.new_application();
        let outer = app.enter_frame().unwrap();
        let first = app.enter_frame().unwrap();
        first.finish(ApplicationFrameExit::Normal).unwrap();
        let second = app.enter_frame().unwrap();
        assert_eq!(outer.witness().published_stamp(), Ok(2));
        second.finish(ApplicationFrameExit::Normal).unwrap();
        assert_eq!(outer.witness().published_stamp(), Ok(2));
        outer.finish(ApplicationFrameExit::Normal).unwrap();
        assert_eq!(*app.counter.lock().unwrap(), 4);
    }

    #[test]
    fn quit_publishes_without_advancing_and_expired_witness_is_rejected() {
        let app = ViewerGraphicsRuntime::new().new_application();
        let quit = app.enter_frame().unwrap();
        let witness = quit.witness().clone();
        assert_eq!(witness.published_stamp(), Ok(1));
        quit.finish(ApplicationFrameExit::Quit).unwrap();
        assert_eq!(
            witness.published_stamp(),
            Err(ApplicationFrameError::ExpiredInvocation)
        );
        let next = app.enter_frame().unwrap();
        assert_eq!(next.witness().published_stamp(), Ok(1));
        next.finish(ApplicationFrameExit::Normal).unwrap();
    }

    #[test]
    fn low32_wrap_keeps_zero_but_full64_wrap_skips_zero() {
        let app = ViewerGraphicsRuntime::new().new_application();
        *app.counter.lock().unwrap() = u32::MAX as u64;
        app.enter_frame()
            .unwrap()
            .finish(ApplicationFrameExit::Normal)
            .unwrap();
        let zero = app.enter_frame().unwrap();
        assert_eq!(zero.witness().published_stamp(), Ok(0));
        zero.finish(ApplicationFrameExit::Normal).unwrap();
        *app.counter.lock().unwrap() = u64::MAX;
        app.enter_frame()
            .unwrap()
            .finish(ApplicationFrameExit::Normal)
            .unwrap();
        assert_eq!(*app.counter.lock().unwrap(), 1);
    }

    #[test]
    fn application_recreation_preserves_image_stamp_until_next_entry() {
        let runtime = ViewerGraphicsRuntime::new();
        let old = runtime.new_application();
        *old.counter.lock().unwrap() = 9;
        old.enter_frame()
            .unwrap()
            .finish(ApplicationFrameExit::Normal)
            .unwrap();
        drop(old);
        let new = runtime.new_application();
        assert_eq!(runtime.state.lock().unwrap().stamp, 9);
        let frame = new.enter_frame().unwrap();
        assert_eq!(frame.witness().published_stamp(), Ok(1));
        frame.finish(ApplicationFrameExit::Normal).unwrap();
    }

    #[test]
    fn viewer_scope_does_not_admit_original_game_history() {
        let runtime = ViewerGraphicsRuntime::new();
        let app = runtime.new_application();
        let frame = app.enter_frame().unwrap();
        assert_eq!(
            frame.witness().scope(),
            ApplicationFrameScope::ViewerLocalGraphicsRuntime
        );
        assert_eq!(
            frame.witness().original_game_qualification(),
            OriginalGameFrameQualification::UnresolvedHistoricalPrefixAndSdlProvider
        );
        let owner = frame.witness().application_owner();
        assert_eq!(owner.check(frame.witness()), Ok(1));
        frame.finish(ApplicationFrameExit::Normal).unwrap();
        assert_eq!(
            owner.published_stamp(),
            Err(ApplicationFrameError::ExpiredInvocation)
        );
        let new = runtime.new_application();
        let foreign = new.enter_frame().unwrap();
        assert_eq!(
            owner.check(foreign.witness()),
            Err(ApplicationFrameError::ForeignApplication)
        );
        foreign.finish(ApplicationFrameExit::Normal).unwrap();
    }

    #[test]
    fn abort_and_out_of_order_exit_invalidate_every_borrowed_witness() {
        let app = ViewerGraphicsRuntime::new().new_application();
        let outer = app.enter_frame().unwrap();
        let nested = app.enter_frame().unwrap();
        let witness = nested.witness().clone();
        assert_eq!(
            outer.finish(ApplicationFrameExit::Normal),
            Err(ApplicationFrameError::OutOfOrderExit)
        );
        assert_eq!(
            witness.published_stamp(),
            Err(ApplicationFrameError::Invalidated)
        );
        drop(nested);
        assert!(matches!(
            app.enter_frame(),
            Err(ApplicationFrameError::Invalidated)
        ));

        let app = ViewerGraphicsRuntime::new().new_application();
        let frame = app.enter_frame().unwrap();
        let witness = frame.witness().clone();
        frame.invalidate();
        assert_eq!(
            witness.published_stamp(),
            Err(ApplicationFrameError::Invalidated)
        );
    }

    #[test]
    fn witnesses_reject_foreign_thread_without_invalidating_owner_thread() {
        let app = ViewerGraphicsRuntime::new().new_application();
        let frame = app.enter_frame().unwrap();
        let witness = frame.witness().clone();
        assert_eq!(
            std::thread::spawn(move || witness.published_stamp())
                .join()
                .unwrap(),
            Err(ApplicationFrameError::WrongThread)
        );
        assert_eq!(frame.witness().published_stamp(), Ok(1));
        frame.finish(ApplicationFrameExit::Normal).unwrap();
    }
}
