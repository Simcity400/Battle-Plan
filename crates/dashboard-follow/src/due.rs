//! Followed answers go out at the clock's hour. The hook answers a step when the playhead builds
//! it, up to its ring ahead of the head; each message waits here until the head reaches the step it
//! was made at (the host's consumer takes that step), in the order made. The messages of steps a
//! seek abandoned never go: the seek's reset sends every answer again.

use crate::{Message, Sink, StepStats, push::Push};
use replay_data::Hour;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

struct Held {
    epoch: u64,
    hour: Hour,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct State {
    held: VecDeque<Held>,
    /// The epoch and last hour of the newest step the head took.
    reached: Option<(u64, Hour)>,
}

/// The messages of one playhead's follow, held until the head reaches them, then pushed.
pub struct Due {
    push: Arc<Push>,
    state: Mutex<State>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Due {
    pub fn new(push: Arc<Push>) -> Arc<Self> {
        Arc::new(Self { push, state: Mutex::default() })
    }

    /// The follow hook's sink: each message held until the head reaches its step.
    pub fn sink(self: &Arc<Self>) -> Sink {
        let due = Arc::clone(self);
        Box::new(move |stats: &StepStats, messages: Vec<Message>| due.hold(stats.epoch, messages))
    }

    fn hold(&self, epoch: u64, messages: Vec<Message>) {
        let mut state = lock(&self.state);
        // A newer epoch is a seek's: the steps of the older were abandoned and are never taken.
        state.held.retain(|held| held.epoch >= epoch);
        state.held.extend(messages.into_iter().map(|message| Held { epoch, hour: message.hour, bytes: message.bytes }));
        self.release(&mut state);
    }

    /// The head took the steps of `epoch` through `hour`: every message made at or before it goes.
    pub fn reached(&self, epoch: u64, hour: Hour) {
        let mut state = lock(&self.state);
        state.reached = Some((epoch, hour));
        self.release(&mut state);
    }

    /// Messages waiting for the head, and their bytes.
    pub fn waiting(&self) -> (usize, usize) {
        let state = lock(&self.state);
        (state.held.len(), state.held.iter().map(|held| held.bytes.len()).sum())
    }

    /// Pushed under the lock: a message the stepper holds and one the head releases never pass each other.
    fn release(&self, state: &mut State) {
        let Some((epoch, hour)) = state.reached else { return };
        let mut kept = VecDeque::with_capacity(state.held.len());
        for held in state.held.drain(..) {
            if held.epoch < epoch {
                continue;
            }
            if held.epoch == epoch && held.hour <= hour {
                self.push.offer(held.bytes);
            } else {
                kept.push_back(held);
            }
        }
        state.held = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::Due;
    use crate::{Message, StepStats, push::Push};
    use replay_data::Hour;
    use std::sync::Arc;

    fn message(hour: Hour) -> Message {
        Message { hour, reset: false, answers: 1, bytes: hour.to_le_bytes().to_vec() }
    }

    /// The hours of the messages pushed since the last call, in order.
    fn pushed(push: &Push) -> Vec<Hour> {
        let counters = push.counters();
        (counters.written..counters.offered).map(|_| push.take().map(|(bytes, ..)| Hour::from_le_bytes(bytes[..4].try_into().unwrap())).unwrap()).collect()
    }

    #[test]
    fn a_message_made_ahead_of_the_head_goes_when_the_head_reaches_its_step_and_one_of_an_abandoned_step_never() {
        let push = Arc::new(Push::default());
        let due = Due::new(Arc::clone(&push));
        let mut sink = due.sink();
        let step = |epoch| StepStats { epoch, ..StepStats::default() };
        sink(&step(1), vec![message(100)]);
        assert_eq!(pushed(&push), Vec::<Hour>::new(), "nothing before the head took a step");
        due.reached(1, 100);
        sink(&step(1), vec![message(124)]);
        sink(&step(1), vec![message(148), message(148)]);
        sink(&step(1), vec![message(172)]);
        assert_eq!(pushed(&push), vec![100], "the steps built past the head wait for it");
        due.reached(1, 148);
        assert_eq!(pushed(&push), vec![124, 148, 148], "in the order made, at the hour the head took");
        // Paused at the head, a view of no hours there (a follow added) goes at once.
        sink(&step(1), vec![message(148)]);
        assert_eq!(pushed(&push), vec![148]);
        // A seek: the step at 172 is abandoned; the reset's message waits for its step to be taken.
        sink(&step(2), vec![message(60)]);
        due.reached(1, 172);
        assert_eq!(pushed(&push), Vec::<Hour>::new(), "never a step of an abandoned epoch");
        due.reached(2, 60);
        assert_eq!(pushed(&push), vec![60]);
        assert_eq!(due.waiting(), (0, 0));
    }
}
