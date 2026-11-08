//! The hour's frozen writer observations: one take of every marking registry the division
//! capture certifies its carries under, after the hour's boundary and before any read. Every
//! consumer reads this one batch, so no consumer drains another's marks, and a receiver held
//! under a taken epoch that no take visited is unchanged without a per-division probe.
use crate::army_stat_changes::Changes;
use collector_observers::unit_history_changes;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Observations carried under the hour's take without a certificate query, cumulative: stored
/// XP, previous template, officer XP, career and history.
static QUIET: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Takes that certified nothing, by registry, in [`HourObservations`] order.
static REFUSED: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

pub(crate) fn quiet(kind: usize) {
    QUIET[kind].fetch_add(1, Relaxed);
}


/// One registry's take: the epoch it certified, and the receivers writers touched since the
/// previous take, sorted. A failed take, or one whose visits could not all be kept, certifies
/// nothing.
#[derive(Default)]
pub struct Taken {
    epoch: Option<u64>,
    touched: Vec<usize>,
}

impl Taken {
    pub(crate) fn take(&mut self, take: impl FnOnce(&mut dyn FnMut(usize)) -> Option<u64>) {
        self.touched.clear();
        let touched = &mut self.touched;
        let mut kept = true;
        let epoch = take(&mut |receiver| {
            if engine_allocation::advisory(|| engine_allocation::reserve(touched, 1)).is_ok() {
                touched.push(receiver);
            } else {
                kept = false;
            }
        });
        self.touched.sort_unstable();
        self.touched.dedup();
        self.epoch = epoch.filter(|_| kept);
    }

    fn clear(&mut self) {
        self.epoch = None;
        self.touched.clear();
    }

    /// The epoch the take certified; `None` certifies nothing.
    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }

    /// The receivers writers touched since the previous take, sorted.
    pub fn touched_receivers(&self) -> &[usize] {
        &self.touched
    }

    pub fn touched_len(&self) -> usize {
        self.touched.len()
    }

    /// Whether a writer touched `receiver` since the previous take.
    pub fn touched(&self, receiver: usize) -> bool {
        self.touched.binary_search(&receiver).is_ok()
    }

    /// Whether `receiver`, registered and held under `epoch`, is unchanged since it was held.
    pub fn quiet(&self, receiver: usize, epoch: u64) -> bool {
        self.epoch == Some(epoch) && !self.touched(receiver)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.touched.capacity() * std::mem::size_of::<usize>()
    }
}

/// Every division-capture registry's take for one hour.
#[derive(Default)]
pub struct HourObservations {
    taken: bool,
    /// Armies (attack operands and maxima) and name members.
    pub armies: Taken,
    /// Stored XP, by army.
    pub experience: Taken,
    /// Previous template, by army.
    pub progression: Taken,
    /// Officer receivers, careers (by army), history roots and medal stores.
    pub history: Taken,
    /// History entry edits.
    pub edits: Taken,
}

impl HourObservations {
    /// Takes every registry once for the hour; a second call before [`Self::finish`] keeps the
    /// first take.
    pub fn take(&mut self, changes: &dyn Changes) {
        if std::mem::replace(&mut self.taken, true) {
            return;
        }
        self.armies.take(|visit| changes.take(visit));
        self.experience
            .take(|visit| changes.take_experience(visit));
        self.progression
            .take(|visit| changes.take_progression(visit));
        self.history.take(|visit| unit_history_changes::take(visit));
        self.edits
            .take(|visit| unit_history_changes::take_edits(visit));
        for (refused, taken) in REFUSED.iter().zip([
            &self.armies,
            &self.experience,
            &self.progression,
            &self.history,
            &self.edits,
        ]) {
            if taken.epoch.is_none() {
                refused.fetch_add(1, Relaxed);
            }
        }
    }

    /// Whether this hour's registries were taken.
    pub fn is_taken(&self) -> bool {
        self.taken
    }

    /// The hour ends: the next hour takes again.
    pub fn finish(&mut self) {
        self.taken = false;
    }

    /// Certifies nothing until the next take.
    pub fn forget(&mut self) {
        self.taken = false;
        for taken in [
            &mut self.armies,
            &mut self.experience,
            &mut self.progression,
            &mut self.history,
            &mut self.edits,
        ] {
            taken.clear();
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        [
            &self.armies,
            &self.experience,
            &self.progression,
            &self.history,
            &self.edits,
        ]
        .iter()
        .map(|taken| taken.allocated_bytes())
        .sum()
    }
}
