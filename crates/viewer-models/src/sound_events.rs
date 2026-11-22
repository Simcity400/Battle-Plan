
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
    pub age: f32,
    pub combat: bool,
}

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

pub fn collect(
    events: &[Event],
    names: &mut Names,
    out: &mut Vec<SoundEvent>,
    combat: impl Fn(&Event) -> bool,
) {
    for event in events {
        if event.hidden {
            continue;
        }
        let Some(effect) = event.definition.sound.as_deref() else {
            continue;
        };
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
