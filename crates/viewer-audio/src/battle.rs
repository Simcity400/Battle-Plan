//! Battle ambience, as the game sounds the land battles on its map.
//!
//! The battles are the game's own list, recorded each hour (the recording's
//! `BattleSounds`): each battle's centre, radius and member provinces, in the
//! order the game's per-frame pass visits them. The viewer adds the divisions
//! of the member provinces' land combats and their sub-units.
//!
//! Each frame, a battle within hearing of the camera (its radius plus the
//! distant effect's falloff maximum, or `BATTLE_SOUND_FALLOFF_DEFAULT`
//! without a falloff) whose combats hold at least
//! `BATTLE_SOUND_MIN_UNIT_COUNT` divisions plays `BATTLE_SOUND_NAME` at its
//! centre, [`BATTLE_HEIGHT`] above the ground, and each `combat_sounds`
//! definition whose sub-unit count lies in its range ([`emitters`]). A sound
//! that ends starts again while its battle stays audible.

use crate::{Content, Emitter, Group};
use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

/// The height above the map the game plays battle ambience at.
pub const BATTLE_HEIGHT: f32 = 9.5;

/// One recorded battle and what its member provinces' combats hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Battle {
    /// The identity its sounds keep from hour to hour: a new key is a new
    /// battle, whose sounds start afresh.
    pub key: u64,
    /// Member province ids in the recorded member order.
    pub provinces: Vec<u32>,
    /// World `(x, z)` of the recorded centre.
    pub center: [f32; 2],
    pub radius: f32,
    /// Divisions in the member provinces' land combats, attackers and
    /// defenders, summed over each member.
    pub divisions: usize,
    /// Those divisions' sub-units by type name: one per template slot.
    pub sub_units: BTreeMap<String, usize>,
}

/// The hour's battles, in recorded order, and what the recording could not
/// answer about them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Battles {
    pub battles: Vec<Battle>,
    pub refusals: Vec<String>,
}

/// Why the content plays no battle ambience at all: its battle defines are
/// unreadable, or the distant effect they name is not declared, in which
/// case the game skips its whole battle-sound update.
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

/// The battle ambience emitters for a listener at `listener`, appended to
/// `out`. Nothing plays when the content lacks the battle defines or the
/// distant effect, as the game skips the whole update then.
pub fn emitters(battles: &Battles, content: &Content, listener: [f32; 3], out: &mut Vec<Emitter>) {
    let Some(defines) = &content.battle else {
        return;
    };
    let Some(effect) = content.catalog.effect(&defines.effect) else {
        return;
    };
    let hearing = content.catalog.effects[effect]
        .falloff
        .map_or(defines.falloff_default, |falloff| {
            content.catalog.falloffs[falloff].max_distance
        });
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
