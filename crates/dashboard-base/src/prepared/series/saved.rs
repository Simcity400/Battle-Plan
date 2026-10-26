//! Reuse the compact JSON of unchanged series keys in a continuation. The
//! complete continuation still passes through its ordinary compressor.
use super::{Accumulator, Batch, KeyState, Partial};
use serde::{
    Serialize, Serializer,
    ser::{Error, SerializeMap},
};
use serde_json::value::RawValue;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

#[derive(Default)]
pub(super) struct Cache(Mutex<BTreeMap<u64, Entry>>);
struct Entry {
    state: KeyState,
    json: Box<RawValue>,
}
fn same(a: &KeyState, b: &KeyState) -> bool {
    a.last == b.last
        && a.runs == b.runs
        && a.events == b.events
        && a.event_order == b.event_order
        && a.partial
            .iter()
            .zip(&b.partial)
            .all(|(a, b)| match (*a, *b) {
                (None, None) => true,
                (
                    Some(Partial::Span {
                        aligned: a,
                        first: af,
                        last: al,
                        value: av,
                        first_break: ab,
                    }),
                    Some(Partial::Span {
                        aligned: b,
                        first: bf,
                        last: bl,
                        value: bv,
                        first_break: bb,
                    }),
                ) => (a, af, al, av, ab) == (b, bf, bl, bv, bb),
                (
                    Some(Partial::Bucket {
                        aligned: a,
                        bucket: av,
                    }),
                    Some(Partial::Bucket {
                        aligned: b,
                        bucket: bv,
                    }),
                ) => a == b && super::same_bucket(&av, &bv),
                _ => false,
            })
}
struct States<'a>(&'a BTreeMap<u64, KeyState>, &'a BTreeSet<u64>, &'a Cache);
impl Serialize for States<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let _cpu = ::replay_frame::cpu::scope_phase("series:save_keys");
        #[cfg(test)]
        if WHOLE_SAVED_KEYS.with(std::cell::Cell::get) {
            return self.0.serialize(serializer);
        }
        let mut cached = self.2.0.lock().map_err(S::Error::custom)?;
        let mut active = self.1.iter().peekable();
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (&key, state) in self.0 {
            if active.peek().is_some_and(|&&active| active == key) {
                active.next();
                // Sealing moves every active endpoint at each publication.
                // Stream those changing states without retaining another copy.
                map.serialize_entry(&key, state)?;
                continue;
            }
            let entry = match cached.entry(key) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if !same(state, &entry.get().state) {
                        entry.insert(encode(state).map_err(S::Error::custom)?);
                    }
                    entry.into_mut()
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(encode(state).map_err(S::Error::custom)?)
                }
            };
            map.serialize_entry(&key, &entry.json)?;
        }
        map.end()
    }
}
fn encode(state: &KeyState) -> serde_json::Result<Entry> {
    #[cfg(test)]
    SAVED_KEY_WORK.with(|work| work.set(work.get() + 1));
    Ok(Entry {
        state: state.clone(),
        json: serde_json::value::to_raw_value(state)?,
    })
}
impl Serialize for Accumulator {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Saved<'a> {
            states: States<'a>,
            frontier: Option<i32>,
            previous: Option<i32>,
            active: &'a BTreeSet<u64>,
            // Empty batches keep dual-purpose public catalogues byte-identical.
            #[serde(skip_serializing_if = "Batch::is_empty")]
            batch: &'a Batch,
        }
        Saved {
            states: States(&self.states, &self.active, &self.saved),
            frontier: self.frontier,
            previous: self.previous,
            active: &self.active,
            batch: &self.batch,
        }
        .serialize(serializer)
    }
}
#[cfg(test)]
thread_local! {
    pub(super) static WHOLE_SAVED_KEYS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(super) static SAVED_KEY_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
