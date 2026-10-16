//! The hourly boundary's shared state: the frame pool's size and whether the
//! calling thread runs the native tick.

// Reuse a bounded producer/consumer pool; a burst waits rather than dropping an hour.
pub const BUFFERS: usize = 4;
std::thread_local! {
    /// This thread is running the hourly original, the native tick.
    pub static TICKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Whether the calling thread is inside the hourly original: the native tick
/// the hourly detour runs after its capture.
pub fn ticking() -> bool {
    TICKING.get()
}
/// Tests: run `body` as inside the hourly original.
#[cfg(any(test, feature = "test-support"))]
pub fn in_tick<T>(body: impl FnOnce() -> T) -> T {
    let ticking = TICKING.replace(true);
    let result = body();
    TICKING.set(ticking);
    result
}
