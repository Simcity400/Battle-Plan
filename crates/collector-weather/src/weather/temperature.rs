use engine_allocation::{self as allocation, SharedBox};
use replay_data::{
    rows::{TemperaturePass, TemperatureWitness},
    temperature::{State, window_span, witness},
};
use replay_frame::KEYFRAME_INTERVAL;

pub const ANCHOR_HOURS: i32 = 2 * KEYFRAME_INTERVAL as i32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
    pub cursor: i32,
    pub window: i32,
    pub seed: u32,
    pub gates: Option<[u8; 2]>,
}

#[derive(Debug, Default)]
struct Read {
    hour: i32,
    words: Option<Words>,
    states: Vec<State>,
}

pub type Anchored<'a> = (SharedBox<Vec<State>>, &'a TemperaturePass);

#[derive(Debug, Default)]
pub struct Tracker {
    previous: Option<Read>,
    anchored: Option<(SharedBox<Vec<State>>, TemperaturePass)>,
}

impl Tracker {
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
