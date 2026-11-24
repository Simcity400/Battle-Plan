use crate::army_stat_changes::Changes;
use collector_observers::unit_history_changes;

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

    #[inline]
    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }

    #[inline]
    pub fn touched_receivers(&self) -> &[usize] {
        &self.touched
    }

    #[inline]
    pub fn touched_len(&self) -> usize {
        self.touched.len()
    }

    #[inline]
    pub fn touched(&self, receiver: usize) -> bool {
        !self.touched.is_empty() && self.touched.binary_search(&receiver).is_ok()
    }

    #[inline]
    pub fn quiet(&self, receiver: usize, epoch: u64) -> bool {
        self.epoch == Some(epoch) && !self.touched(receiver)
    }

    pub fn allocated_bytes(&self) -> usize {
        self.touched.capacity() * std::mem::size_of::<usize>()
    }
}

#[derive(Default)]
pub struct HourObservations {
    taken: bool,
    pub armies: Taken,
    pub experience: Taken,
    pub progression: Taken,
    pub history: Taken,
    pub edits: Taken,
}

impl HourObservations {
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
    }

    pub fn is_taken(&self) -> bool {
        self.taken
    }

    pub fn finish(&mut self) {
        self.taken = false;
    }

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
