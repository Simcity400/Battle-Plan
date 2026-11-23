use std::sync::{Mutex, MutexGuard};

pub(crate) static GPU_TESTS: Mutex<()> = Mutex::new(());

pub fn serial() -> MutexGuard<'static, ()> {
    GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner())
}

pub mod gpu;
pub mod province_geometry;
pub mod ui_scale;
