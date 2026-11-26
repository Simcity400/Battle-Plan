use crate::army_stat_changes::Changes;
use collector_observers::unit_history_changes;

#[derive(Default)]
pub struct Taken {
    epoch: Option<u64>,
    touched: Vec<usize>,
    filter: [u64; 8],
}

#[inline]
fn bloom_bits(receiver: usize) -> (usize, u64, usize, u64) {
    let hash = (receiver >> 3).wrapping_mul(0x9e37_79b9_7f4a_7c15_u64 as usize);
    let b1 = (hash >> 23) & 511;
    let b2 = (hash >> 14) & 511;
    (b1 >> 6, 1_u64 << (b1 & 63), b2 >> 6, 1_u64 << (b2 & 63))
}

impl Taken {
    pub(crate) fn take(&mut self, take: impl FnOnce(&mut dyn FnMut(usize)) -> Option<u64>) {
        self.touched.clear();
        self.filter = [0; 8];
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
        for &receiver in &self.touched {
            let (w1, m1, w2, m2) = bloom_bits(receiver);
            self.filter[w1] |= m1;
            self.filter[w2] |= m2;
        }
        self.epoch = epoch.filter(|_| kept);
    }

    fn clear(&mut self) {
        self.epoch = None;
        self.touched.clear();
        self.filter = [0; 8];
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
        if self.touched.is_empty() {
            return false;
        }
        let (w1, m1, w2, m2) = bloom_bits(receiver);
        (self.filter[w1] & m1) != 0
            && (self.filter[w2] & m2) != 0
            && self.touched.binary_search(&receiver).is_ok()
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
