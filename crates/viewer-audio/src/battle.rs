
use crate::{Content, Emitter, Group};
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

pub const BATTLE_HEIGHT: f32 = 9.5;

pub const DIVISION_CLASS: u32 = 0;

pub use map_display_data::{Battle, Battles};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Front<K = String> {
    pub divisions: usize,
    pub sub_units: Vec<(K, usize)>,
    pub unknown: usize,
}

impl<K: Ord> Front<K> {
    pub fn add(&mut self, units: usize, divisions: impl Iterator<Item = u64>, layers: bool, mut composition: impl FnMut(u64) -> Option<Vec<(K, usize)>>) {
        self.divisions += units;
        if !layers {
            return;
        }
        for division in divisions {
            match composition(division) {
                Some(units) => {
                    for (unit, count) in units {
                        match self.sub_units.binary_search_by(|(held, _)| held.cmp(&unit)) {
                            Ok(at) => self.sub_units[at].1 += count,
                            Err(at) => self.sub_units.insert(at, (unit, count)),
                        }
                    }
                }
                None => self.unknown += 1,
            }
        }
    }
}

pub fn divisions<'a>(attackers: &'a [u64], attacker_classes: &'a [u32], defenders: &'a [u64], defender_classes: &'a [u32]) -> impl Iterator<Item = u64> + 'a {
    attackers.iter().zip(attacker_classes).chain(defenders.iter().zip(defender_classes)).filter(|(_, class)| **class == DIVISION_CLASS).map(|(&division, _)| division)
}

pub fn composition<K>(slots: impl Iterator<Item = (u32, i32)>, mut name: impl FnMut(u32) -> Option<K>) -> Option<Vec<(K, usize)>> {
    slots.filter(|&(token, _)| token != 0).map(|(token, count)| Some((name(token)?, usize::try_from(count).unwrap_or(0)))).collect()
}

pub fn battle<'a, K: Ord + 'a>(key: u64, provinces: Vec<u32>, center: [f32; 2], radius: f32, front: impl Fn(u32) -> Option<&'a Front<K>>, name: impl Fn(&K) -> String, unknown: &mut usize) -> Battle {
    let mut divisions = 0;
    let mut sub_units: BTreeMap<&K, usize> = BTreeMap::new();
    for &province in &provinces {
        let Some(front) = front(province) else { continue };
        divisions += front.divisions;
        *unknown += front.unknown;
        for (unit, count) in &front.sub_units {
            *sub_units.entry(unit).or_default() += count;
        }
    }
    let sub_units = sub_units.into_iter().map(|(unit, count)| (name(unit), count)).collect();
    Battle { key, provinces, center, radius, divisions, sub_units }
}

pub fn unknown_refusal(unknown: usize) -> Option<String> {
    (unknown > 0).then(|| format!("{unknown} combat divisions have no recorded template composition or sub-unit name, so their sub-units do not count toward battle layers"))
}

pub fn silence(content: &Content) -> Option<String> {
    let Some(defines) = &content.battle else {
        return Some("the NSound battle sound defines are unreadable, so no battle plays".into());
    };
    content.catalog.effect(&defines.effect).is_none().then(|| {
        format!(
            "the battle sound effect {} is not declared, so no battle plays (the game skips its whole battle-sound update)",
            defines.effect
        )
    })
}

pub fn hearing(content: &Content) -> Option<f32> {
    let defines = content.battle.as_ref()?;
    let effect = content.catalog.effect(&defines.effect)?;
    Some(content.catalog.effects[effect]
        .falloff
        .map_or(defines.falloff_default, |falloff| {
            content.catalog.falloffs[falloff].max_distance
        }))
}

pub fn emitters(battles: &Battles, content: &Content, listener: [f32; 3], out: &mut Vec<Emitter>) {
    let Some(defines) = &content.battle else {
        return;
    };
    let Some(hearing) = hearing(content) else {
        return;
    };
    let distant: Arc<str> = Arc::from(defines.effect.as_str());
    let layers: Vec<Arc<str>> = content
        .layers
        .sounds
        .iter()
        .map(|sound| Arc::from(sound.effect.as_str()))
        .collect();
    for battle in &battles.battles {
        let [dx, dz] = [
            listener[0] - battle.center[0],
            listener[2] - battle.center[1],
        ];
        let reach = hearing + battle.radius;
        if reach * reach < dx * dx + dz * dz
            || i64::try_from(battle.divisions).unwrap_or(i64::MAX)
                < i64::from(defines.min_unit_count)
        {
            continue;
        }
        let position = [battle.center[0], BATTLE_HEIGHT, battle.center[1]];
        let emitter = |identity: (u64, usize, usize), effect: &Arc<str>| Emitter {
            key: key(identity),
            effect: Arc::clone(effect),
            position,
            age: 0.0,
            group: Group::Combat,
            restart: true,
        };
        out.push(emitter((battle.key, usize::MAX, usize::MAX), &distant));
        for (slot, (unit, sounds)) in content.layers.slots.iter().enumerate() {
            let count = battle.sub_units.get(unit).copied().unwrap_or(0);
            for &sound in sounds {
                if content.layers.sounds[sound].sounds(count) {
                    out.push(emitter((battle.key, slot, sound), &layers[sound]));
                }
            }
        }
    }
}

fn key(identity: (u64, usize, usize)) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ("battle", identity).hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests;
