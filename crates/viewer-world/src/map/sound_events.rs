//! The entity events a presented frame holds that name a sound effect.
//!
//! An entity event's `sound = { soundeffect = ... }` plays when the event
//! fires, positioned at the event's node. The scene lists every present
//! event with a sound after the model passes animate; an audio consumer
//! diffs the list from frame to frame: a new key, or a key whose age falls,
//! is a firing. Keys are hashes of the exact [`EventKey`], so a seek or
//! branch change (a new event namespace) presents new keys.

use super::entity_animation::{Event, EventKey};
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq)]
pub struct SoundEvent {
    pub key: u64,
    pub effect: Arc<str>,
    pub position: [f32; 3],
    /// Seconds since the event fired.
    pub age: f32,
    /// A combat sound: the producer presents the actor in combat, or the
    /// actor is a strike's impact.
    pub combat: bool,
}

/// Effect names, shared across frames.
#[derive(Default)]
pub struct Names(BTreeMap<String, Arc<str>>);

impl Names {
    fn get(&mut self, name: &str) -> Arc<str> {
        if let Some(known) = self.0.get(name) {
            return Arc::clone(known);
        }
        let shared: Arc<str> = name.into();
        self.0.insert(name.to_owned(), Arc::clone(&shared));
        shared
    }
}

pub fn key(key: &EventKey) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

/// A weather entity's sound event, keyed by its anchor, member, event,
/// region and the hour its weather began, `age` seconds after it fired.
pub fn weather(
    identity: (usize, usize, usize, u32, i32),
    names: &mut Names,
    effect: &str,
    position: [f32; 3],
    age: f32,
) -> SoundEvent {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ("weather", identity).hash(&mut hasher);
    SoundEvent {
        key: hasher.finish(),
        effect: names.get(effect),
        position,
        age,
        combat: false,
    }
}

/// Appends the events of `events` that name a sound effect; `combat` says
/// which are combat sounds.
pub fn collect(
    events: &[Event],
    names: &mut Names,
    out: &mut Vec<SoundEvent>,
    combat: impl Fn(&Event) -> bool,
) {
    for event in events {
        // An actor or layer the display hides does not sound.
        if event.hidden {
            continue;
        }
        let Some(effect) = event.definition.sound.as_deref() else {
            continue;
        };
        // Native packets resolved without a node keep a degenerate transform.
        if !event
            .transform
            .translation
            .iter()
            .all(|value| value.is_finite())
        {
            continue;
        }
        out.push(SoundEvent {
            key: key(&event.key),
            effect: names.get(effect),
            position: event.transform.translation,
            age: event.age,
            combat: combat(event),
        });
    }
}
