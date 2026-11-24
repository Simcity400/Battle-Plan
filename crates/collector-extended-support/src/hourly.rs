
pub const BUFFERS: usize = 2;
std::thread_local! {
    pub static TICKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub fn ticking() -> bool {
    TICKING.get()
}
#[cfg(any(test, feature = "test-support"))]
pub fn in_tick<T>(body: impl FnOnce() -> T) -> T {
    let ticking = TICKING.replace(true);
    let result = body();
    TICKING.set(ticking);
    result
}
