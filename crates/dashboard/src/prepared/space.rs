//! Disk room for preparation ahead of the user: the store's estimate plus a margin.

use std::{io, path::Path};

/// Left to the system and the game whatever preparation writes.
pub const MARGIN: u64 = 1 << 30;

/// Store bytes per recording byte at its peak: 4.02 on a 13-year campaign, rounded up.
const PEAK_PER_RECORDING: (u64, u64) = (17, 4);

/// None when the volume cannot be measured; writes then fail with the real error if it fills.
pub fn free() -> Option<u64> {
    #[cfg(test)]
    return TEST_FREE.with(std::cell::Cell::get);
    #[cfg(not(test))]
    store::local_space().ok().map(|(free, _)| free)
}

#[cfg(test)]
thread_local! {
    /// Unchosen it is unmeasured, so tests never depend on the machine's free space.
    pub(crate) static TEST_FREE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

pub fn needed(recording: u64, stored: u64) -> u64 {
    let (numerator, denominator) = PEAK_PER_RECORDING;
    (recording.saturating_mul(numerator) / denominator).saturating_sub(stored)
}

pub fn needed_for(recording: &Path, folder: &Path) -> io::Result<u64> {
    Ok(needed(
        std::fs::metadata(recording)?.len(),
        on_disk(folder)?,
    ))
}

pub fn admits(free: Option<u64>, needed: u64) -> bool {
    free.is_none_or(|free| free >= needed.saturating_add(MARGIN))
}

pub fn keeps(free: Option<u64>) -> bool {
    free.is_none_or(|free| free >= MARGIN)
}

/// The store with its spool beside it.
pub fn on_disk(folder: &Path) -> io::Result<u64> {
    Ok(tree_bytes(folder)? + tree_bytes(&folder.with_extension("spool"))?)
}

fn tree_bytes(folder: &Path) -> io::Result<u64> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut bytes = 0;
    for entry in entries {
        let entry = entry?;
        bytes += if entry.file_type()?.is_dir() {
            tree_bytes(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_needs_the_estimate_and_the_margin_and_running_work_the_margin() {
        let needed = needed(1 << 30, 0);
        assert_eq!(needed, 17 << 28);
        assert!(admits(Some(needed + MARGIN), needed));
        assert!(!admits(Some(needed + MARGIN - 1), needed));
        assert!(admits(None, u64::MAX));
        assert!(keeps(Some(MARGIN)));
        assert!(!keeps(Some(MARGIN - 1)));
        assert!(keeps(None));
        // A store already holding its estimate needs nothing more, only the margin.
        assert_eq!(super::needed(1 << 30, 5 << 30), 0);
        assert!(admits(Some(MARGIN), 0));
        // No share of the volume: 400 GB free on any volume admits a 1 GB recording.
        assert!(admits(Some(400 << 30), super::needed(1 << 30, 0)));
    }
}
