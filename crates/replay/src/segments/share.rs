//! Work a family's emit shares with the run's workers waiting idle ([`share`]).

use super::crew::{Board, Joined};
use engine_allocation::labels;
use std::{
    any::Any,
    cell::{Cell, OnceCell},
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    ptr::NonNull,
    sync::{Mutex, PoisonError},
};

/// An emit's board, and its crew once it first shares.
pub(super) struct Sharing<'b> {
    board: &'b Board,
    at: usize,
    crew: OnceCell<Option<Joined<'b>>>,
}

thread_local! {
    /// The emit this worker runs, while it runs it outside any share.
    static SHARING: Cell<Option<NonNull<Sharing<'static>>>> = const { Cell::new(None) };
}

/// Puts back what the thread shares to, however the call it outlives ends.
struct Restore(Option<NonNull<Sharing<'static>>>);

impl Drop for Restore {
    fn drop(&mut self) {
        SHARING.with(|cell| cell.set(self.0));
    }
}

impl<'b> Sharing<'b> {
    /// Segment `at`'s emit's, on `board`.
    pub(super) fn new(board: &'b Board, at: usize) -> Self {
        Self {
            board,
            at,
            crew: OnceCell::new(),
        }
    }

    /// Runs `emit` on this thread, its shares going to this board.
    pub(super) fn during<T>(&self, emit: impl FnOnce() -> T) -> T {
        let pointer = NonNull::from(self).cast::<Sharing<'static>>();
        let _restored = Restore(SHARING.with(|cell| cell.replace(Some(pointer))));
        emit()
    }

    /// Its crew; none where the board could not take one, and its shares run here.
    fn crew(&self) -> Option<&Joined<'b>> {
        self.crew
            .get_or_init(|| self.board.join(self.at, true).ok())
            .as_ref()
    }
}

/// How many ways work shared now may go: this worker and the run's workers waiting idle for
/// any work; one outside a family's emit, or inside a share.
pub fn ways() -> usize {
    SHARING.with(|cell| match cell.get() {
        // SAFETY: `Sharing::during` sets it, on this thread, while what it points at lives.
        Some(sharing) => 1 + unsafe { sharing.as_ref() }.board.idle(),
        None => 1,
    })
}

/// Runs `job` once for each index below `count`, in no set order: here, and on the run's
/// workers waiting idle as it goes, each taking what is left. Outside a family's emit, and
/// inside a share, all here in index order. A panic in any goes on here once all ran.
pub fn share(count: usize, job: &(dyn Fn(usize) + Sync)) {
    let taken = SHARING.with(Cell::take);
    let _restored = Restore(taken);
    // SAFETY: as in `ways`; taken meanwhile, so a share inside one runs where it is.
    let crew = taken
        .filter(|_| count > 1)
        .and_then(|sharing| unsafe { sharing.as_ref() }.crew());
    let Some(crew) = crew else {
        for index in 0..count {
            job(index);
        }
        return;
    };
    let label = labels::current();
    let cpu = replay_frame::cpu::Context::capture();
    let panicked: Mutex<Option<Box<dyn Any + Send>>> = Mutex::new(None);
    crew.hour(count, count, &|index| {
        let _memory = labels::resume(label);
        let _cpu = cpu.clone().enter("shared");
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| job(index))) {
            panicked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert(payload);
        }
    });
    if let Some(payload) = panicked
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
    {
        resume_unwind(payload);
    }
}
