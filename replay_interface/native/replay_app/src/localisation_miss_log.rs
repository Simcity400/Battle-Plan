//! The one runtime sink for localisation misses: every producer names its
//! misses on its own output, and each consumption point in the app reports
//! them here. A miss is logged once per process, so a key drawn as itself on
//! every frame is named once, not once per frame.

use replay_core::LocalisationMiss;
use std::collections::BTreeSet;
use std::sync::Mutex;

static SEEN: Mutex<BTreeSet<LocalisationMiss>> = Mutex::new(BTreeSet::new());

/// Logs each miss not seen before; returns how many were new.
pub(crate) fn report(misses: impl IntoIterator<Item = LocalisationMiss>) -> usize {
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut new = 0;
    for miss in misses {
        if seen.insert(miss.clone()) {
            eprintln!("replay_app: {miss}");
            new += 1;
        }
    }
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_miss_is_logged_once_per_process() {
        let key = format!("LOG_ONCE_{}", std::process::id());
        let miss = LocalisationMiss::effective_vfs(&key, "l_english");
        assert_eq!(report([miss.clone(), miss.clone()]), 1);
        assert_eq!(report([miss.clone()]), 0);
        let other_layer = LocalisationMiss::recorder_packaged(&key, "localisation_join");
        assert_eq!(report([other_layer]), 1);
        assert_eq!(report(std::iter::empty()), 0);
    }
}
