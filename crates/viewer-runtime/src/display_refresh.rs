
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const FALLBACK: f64 = 60.0;

const REREAD_MS: u64 = 1_000;

static GIVEN: AtomicU64 = AtomicU64::new(0);
static WINDOWS: Mutex<Vec<(usize, u64)>> = Mutex::new(Vec::new());

thread_local! {
    static FOLLOWED: Cell<(usize, u64, u64)> = const { Cell::new((0, 0, 0)) };
}

fn started() -> Instant {
    static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *STARTED.get_or_init(Instant::now)
}

fn windows() -> std::sync::MutexGuard<'static, Vec<(usize, u64)>> {
    WINDOWS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn pace_at(hz: f64) {
    if hz.is_finite() && hz > 0.0 {
        GIVEN.store(hz.to_bits(), Ordering::Release);
    }
}

#[must_use = "the window is followed while this is held"]
pub struct Followed(usize);

impl Drop for Followed {
    fn drop(&mut self) {
        let mut held = windows();
        if let Some(at) = held.iter().position(|&(window, _)| window == self.0) {
            held.swap_remove(at);
        }
        FOLLOWED.set((0, 0, 0));
    }
}

pub fn follow_window(hwnd: usize) -> Followed {
    FOLLOWED.set((hwnd, 0, 0));
    windows().push((hwnd, 0));
    Followed(hwnd)
}

pub fn hz() -> Option<f64> {
    limited(shown())
}

fn limited(shown: Option<f64>) -> Option<f64> {
    match (shown, super::frame_limit::limit()) {
        (Some(shown), Some(limit)) => Some(shown.min(limit)),
        (shown, limit) => shown.or(limit),
    }
}

fn shown() -> Option<f64> {
    let given = GIVEN.load(Ordering::Acquire);
    if given != 0 {
        return Some(f64::from_bits(given));
    }
    let now = started().elapsed().as_millis() as u64 + 1;
    let (window, read, at) = FOLLOWED.get();
    let read = if at != 0 && now.saturating_sub(at) < REREAD_MS {
        read
    } else {
        let read = u64::from(content_reader_settings::settings::display_refresh_hz((window != 0).then_some(window)).unwrap_or(0));
        FOLLOWED.set((window, read, now));
        if window != 0
            && let Some(held) = windows().iter_mut().find(|(held, _)| *held == window)
        {
            held.1 = read;
        }
        read
    };
    (read != 0).then_some(read as f64)
}

pub fn rate() -> f64 {
    hz().unwrap_or(FALLBACK)
}

pub fn fastest() -> f64 {
    let given = GIVEN.load(Ordering::Acquire);
    let shown = match given {
        0 => fastest_read(&windows()),
        _ => Some(f64::from_bits(given)),
    };
    match shown {
        Some(shown) => limited(Some(shown)).unwrap_or(shown),
        None => rate(),
    }
}

fn fastest_read(windows: &[(usize, u64)]) -> Option<f64> {
    windows.iter().map(|&(_, read)| read).max().filter(|&read| read != 0).map(|read| read as f64)
}

#[cfg(test)]
mod tests {
    use super::{fastest_read, follow_window, windows};

    #[test]
    fn the_fastest_open_window_caps_the_simulation_and_a_closed_one_leaves_it() {
        let (main, popout) = (0x5eed_0001, 0x5eed_0002);
        let set = |window: usize, hz: u64| {
            if let Some(held) = windows().iter_mut().find(|(held, _)| *held == window) {
                held.1 = hz;
            }
        };
        let ours = || windows().iter().copied().filter(|&(window, _)| window == main || window == popout).collect::<Vec<_>>();
        let (followed_main, followed_popout) = (follow_window(main), follow_window(popout));
        set(main, 144);
        set(popout, 60);
        assert_eq!(fastest_read(&ours()), Some(144.0), "the main map's display, not the pop-out's");
        set(popout, 240);
        assert_eq!(fastest_read(&ours()), Some(240.0), "a faster pop-out raises it");
        drop(followed_main);
        assert_eq!(ours().len(), 1, "a closed window is followed no more");
        set(popout, 60);
        assert_eq!(fastest_read(&ours()), Some(60.0), "and the window left caps it");
        drop(followed_popout);
        assert!(ours().is_empty());
    }
}
