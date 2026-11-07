//! Payloads of keys without new runs or events, retained for one flush.
use super::{KeyState, LEVELS, Partial};
use std::collections::HashMap;

type Identity = (u8, u8, [u8; 32]);
type Payload = (Vec<u8>, [Option<Partial>; 4]);
#[derive(Default)]
pub(super) struct Cache {
    previous: HashMap<Identity, Payload>,
    current: HashMap<Identity, Payload>,
}
impl Cache {
    pub(super) fn begin(&mut self) {
        self.previous = std::mem::take(&mut self.current);
    }
    pub(super) fn reuse(&mut self, identity: Identity) -> Option<&Payload> {
        let payload = self.previous.remove(&identity)?;
        #[cfg(test)]
        IDLE_REUSES.with(|work| work.set(work.get() + 1));
        Some(self.current.entry(identity).or_insert(payload))
    }
    pub(super) fn insert(
        &mut self,
        identity: Identity,
        bytes: Vec<u8>,
        partial: [Option<Partial>; 4],
    ) {
        self.current.insert(identity, (bytes, partial));
    }
    #[cfg(test)]
    pub(super) fn counts(&self) -> (usize, usize) {
        (self.previous.len(), self.current.len())
    }
    pub(super) fn finish(&mut self) {
        self.previous.clear();
    }
}
/// With no new runs, a level only publishes its incoming prefix. Bounds
/// affect its break flag and whether its partial remains open; the batch's
/// first hour never changes the absent numeric bucket.
pub(super) fn identity(state: &KeyState, last: i32, decimals: u8, input: [u8; 32]) -> Identity {
    let last = i64::from(last);
    let mut flags = 0;
    for (level, size) in LEVELS.into_iter().enumerate() {
        if let Some(partial) = state.partial.level(level) {
            let aligned = partial.aligned();
            let end = aligned + size - 1;
            let held = match partial {
                Partial::Span { last, .. } => last,
                Partial::Bucket { bucket, .. } => bucket.last_hour,
            };
            flags |= u8::from(i64::from(held) < end.min(last)) << (level * 2);
            flags |=
                u8::from(last.div_euclid(size) * size == aligned && last < end) << (level * 2 + 1);
        }
    }
    (decimals, flags, input)
}
#[cfg(test)]
thread_local! {
    pub(super) static IDLE_REUSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
