pub mod gate;

#[cfg(windows)]
mod hooks_windows;

pub fn hooks_installed() -> bool {
    #[cfg(windows)]
    {
        hooks_windows::installed()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn gate_census() -> [u64; 3] {
    gate::gate_census()
}

#[cfg(windows)]
pub use hooks_windows::{
    abort_partial_install_and_uninstall, install, installed, installed_hook_receipts,
    release_pending, take_released_hook_receipts, uninstall, HOOK_RECEIPT_COUNT,
    PRODUCTION_HOOK_SPECS,
};
