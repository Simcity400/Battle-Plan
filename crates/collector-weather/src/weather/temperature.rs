//! The hourly temperature pass's recorded inputs ([`replay_data::rows::TemperaturePass`]).
//!
//! The decode worker sees the hours in order. Each complete read gives every province record's
//! words ([`State`]) and the manager's cursor, window and seed. An hour extends the current anchor
//! when the read proves what the hour did since the hour before: with both suspension gates clear,
//! hourly weather update ran one ordinary pass, so the cursor moved exactly as weather window pass
//! walks it and no record outside the window it visited changed, and the hour's seed is recorded;
//! with a gate set, nothing moved. Anything else (the first read, a missed hour, a load, a console
//! reload or the `randomize_weather` effect, which reinitialize every record through weather
//! reinitialize) starts a new anchor holding the hour's words, as does every [`ANCHOR_HOURS`] hours
//! so that a frame needs at most that many hours of passes.

use engine_allocation::{self as allocation, SharedBox};
use replay_data::{
    rows::{TemperaturePass, TemperatureWitness},
    temperature::{State, window_span, witness},
};
use replay_frame::KEYFRAME_INTERVAL;

/// Hours an anchor lasts: two keyframe intervals. A block's keyframe holds
/// its frame's anchor words, and the keyframe one interval after an anchor
/// repeats them, which the anchored keyframe encoding stores at a few bytes,
/// so the province words (about seven bytes a province compressed) are
/// stored once per two blocks; a frame derives from at most this many hours
/// of passes and its pass lists at most this many seeds.
pub const ANCHOR_HOURS: i32 = 2 * KEYFRAME_INTERVAL as i32;

/// The manager words one hour's read gives besides the records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
    pub cursor: i32,
    pub window: i32,
    pub seed: u32,
    /// The suspension gates, `None` when unread.
    pub gates: Option<[u8; 2]>,
}

#[derive(Debug, Default)]
struct Read {
    hour: i32,
    words: Option<Words>,
    states: Vec<State>,
}

/// The words an hour's rows hold and the pass its slot 0 carries.
pub type Anchored<'a> = (SharedBox<Vec<State>>, &'a TemperaturePass);

#[derive(Debug, Default)]
pub struct Tracker {
    previous: Option<Read>,
    /// The anchor's words, shared with every frame that holds them, and the pass, which holds
    /// its hour, cursor and the hours since.
    anchored: Option<(SharedBox<Vec<State>>, TemperaturePass)>,
}

impl Tracker {
    /// The hour's anchored words and pass, from the hour's complete read (`None` when the
    /// records or the manager words were not read whole). A refused hour forgets the hours
    /// before it. The read's words are kept as the next hour's comparison: the caller gets the
    /// buffer they replace back to fill next hour, so an ordinary hour copies no words.
    pub fn observe(
        &mut self,
        hour: i32,
        read: Option<(Words, &mut Vec<State>)>,
    ) -> std::io::Result<Option<Anchored<'_>>> {
        let Some((words, states)) = read else {
            self.forget();
            return Ok(None);
        };
        let proven = self
            .previous
            .as_ref()
            .and_then(|previous| classify(previous, hour, words, states));
        let extend = proven.zip(self.anchored.as_ref()).is_some_and(|(_, (_, pass))| {
            i64::from(hour) - i64::from(pass.anchor) < i64::from(ANCHOR_HOURS)
        });
        if let (true, Some(entry), Some((_, pass))) = (extend, proven, self.anchored.as_mut()) {
            if let Err(error) = allocation::reserve(&mut pass.hours, 1) {
                self.forget();
                return Err(error);
            }
            pass.hours.push(entry);
        } else {
            let anchor = allocation::copy_vec(states)
                .and_then(allocation::shared)
                .inspect_err(|_| self.forget())?;
            let pass = TemperaturePass {
                anchor: hour,
                cursor: words.cursor,
                // The anchor hour's own ordinary pass, when the hour before proves it.
                anchor_seed: proven.flatten(),
                hours: Vec::new(),
                witness: TemperatureWitness::default(),
            };
            self.anchored = Some((anchor, pass));
        }
        let (anchor, pass) = self.anchored.as_mut().expect("extended or anchored");
        pass.witness =
            TemperatureWitness::new(witness(states.iter().map(|state| state.temperature)));
        let previous = self.previous.get_or_insert_with(Read::default);
        previous.hour = hour;
        previous.words = Some(words);
        std::mem::swap(&mut previous.states, states);
        Ok(Some((anchor.clone(), pass)))
    }

    fn forget(&mut self) {
        self.previous = None;
        self.anchored = None;
    }
}

/// `Some(Some(seed))` for an ordinary pass since the hour before, `Some(None)` for a
/// suspended hour, `None` when the reads do not prove either.
fn classify(previous: &Read, hour: i32, words: Words, states: &[State]) -> Option<Option<u32>> {
    let before = previous.words?;
    if previous.hour.checked_add(1) != Some(hour)
        || previous.states.len() != states.len()
        || before.window != words.window
    {
        return None;
    }
    let changed = |slot: usize| previous.states[slot] != states[slot];
    match words.gates? {
        [0, 0] => {
            let count = i32::try_from(states.len()).ok()?;
            let (span, next) = window_span(before.cursor, words.window, count).ok()?;
            let visited = |slot: usize| (slot as i32 - before.cursor).rem_euclid(count) < span;
            (next == words.cursor && (0..states.len()).all(|slot| visited(slot) || !changed(slot)))
                .then_some(Some(words.seed))
        }
        _ => (words.cursor == before.cursor
            && words.seed == before.seed
            && !(0..states.len()).any(changed))
        .then_some(None),
    }
}

#[cfg(test)]
mod tests;
