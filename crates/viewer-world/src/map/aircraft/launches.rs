//! Launch effects: the entity a base-equipment wing starts at its base
//! instead of a missile bombing scenario.
//!
//! The regional builder runs a pass over every region every
//! [`PASS_SECONDS`]. A bit-17 mission (while the other side holds aircraft in
//! the region) and a launch-flagged pool definition each start one launch
//! record at the wing's base, unless a live record already holds the base's
//! province; the records span every region, visited in ascending ID. The
//! record's entity is the wing's primary equipment type, by its token's
//! spelling, with `_entity` appended; a name no entity declares starts no
//! record. The record waits `NAirGfx.RANDOM_SCENARIO_ANIMATION_MAX_DELAY`
//! times a unit draw on the aircraft clock, then spawns its entity in its
//! default state where the base's position finder puts it, turned about +Y
//! by a whole-turn draw. Once the entity's root reads its clip's length the
//! record is removed; a launch-flagged definition's record then plays the
//! missile explosion entity in a random province of its mission's strategic
//! region, as listed by the game. The next pass that finds the base free
//! starts the next record.
//!
//! A base's position is its air-base slot for an ordinary base, its host's
//! for a base on a ship, and for a rocket site or gun emplacement the first
//! admitted authored slot of its state, in `map/buildings.txt` order, whose
//! bucket holds a type with `rocket_launch_capacity` or `gun_emplacement`.
//! Missile bombing scenarios take off from the same place.
//!
//! The game draws the delay, heading and explosion province from wall-clock
//! random state, which is not recorded. The viewer draws them from fixed
//! seeds: the delay once per launch, so its records repeat with a fixed
//! period on the presentation clock's passes, and the heading and province
//! once per record.

use std::{collections::BTreeMap, io, sync::Arc};

use installed_content::{
    ResolvedLayers,
    buildings::{Buildings, Slot, SpawnPoint},
    defines::Defines,
    map::MapData,
    mesh,
    positions::BuildingPosition,
};
use viewer_foundation::map::building_facts::{SlotAdmission, admit_slot, is_floating_harbor_slot};
use viewer_foundation::map::province_map::ProvinceMap;

use super::Random;
use crate::map::models::ModelInstance;

/// The regional builder's pass interval, in presentation seconds.
pub(super) const PASS_SECONDS: f32 = 2.0;
/// Launch draws are this XOR the launch's seed.
const SEED: u32 = 0x4c41_554e;
/// Launch entities' animation seeds are this XOR the launch's index, apart
/// from the flights' small indices.
const INSTANCE_SEED: u32 = 0x5355_4e43;
/// The game's fixed-point scale, of the delay define and the heading draw.
const FIXED5: f32 = 100_000.0;
/// A draw's unit fraction, 2^-31.
const UNIT: f32 = 1.0 / 2_147_483_648.0;
/// The base kind of a rocket site.
pub(super) const ROCKET_SITE: i32 = 1;
/// The base kind of a gun emplacement.
pub(super) const GUN_EMPLACEMENT: i32 = 2;

/// Each state's rocket-site and gun-emplacement position by base kind, or
/// why it cannot be decided ([`site_table`]).
pub(super) type Sites = BTreeMap<(u32, i32), Result<[f32; 3], &'static str>>;

/// A mission region's provinces as the game lists them, each with its
/// standstill anchor and admission box when it has them.
pub(super) type ExplosionPlaces = Arc<Vec<(u32, Option<([f32; 3], [f32; 4])>)>>;

/// What launch effects and site take-offs read from the content.
#[derive(Debug)]
pub(super) struct Content {
    /// Each state's rocket-site and gun-emplacement position by base kind,
    /// or why it cannot be decided; or why the building sites do not read.
    sites: Result<Sites, String>,
    /// `NAirGfx.RANDOM_SCENARIO_ANIMATION_MAX_DELAY`, fixed-point as the
    /// game reads it and no less than zero, in aircraft-clock seconds.
    max_delay: Result<f32, &'static str>,
}

impl Content {
    pub(super) fn load(
        layers: &ResolvedLayers,
        map: &MapData,
        defines: &Defines,
        roster: &mesh::Roster,
        warnings: &mut Vec<installed_content::Diagnostic>,
    ) -> Self {
        let sites = sites(layers, map, defines, roster).map_err(|error| error.to_string());
        if let Err(reason) = &sites {
            warnings.push((
                installed_content::DiagnosticKind::Refused,
                format!(
                    "Rocket-site and gun-emplacement bases have no position, so their launch effects and missile take-offs are not drawn: the building sites do not read: {reason}"
                ),
            ));
        }
        Self {
            sites,
            max_delay: max_delay(defines),
        }
    }

    /// Where a state base of `kind` (a rocket site or gun emplacement) stands.
    pub(super) fn site(&self, state: u32, kind: i32) -> Result<[f32; 3], &'static str> {
        let sites = self
            .sites
            .as_ref()
            .map_err(|_| "the content's building sites do not read")?;
        match sites.get(&(state, kind)) {
            Some(site) => *site,
            None if kind == ROCKET_SITE => Err("its state has no admitted rocket-site slot"),
            None => Err("its state has no admitted gun-emplacement slot"),
        }
    }

    pub(super) fn max_delay(&self) -> Result<f32, &'static str> {
        self.max_delay
    }

    #[cfg(test)]
    pub(super) fn for_tests(sites: Sites, max_delay: f32) -> Self {
        Self {
            sites: Ok(sites),
            max_delay: Ok(max_delay),
        }
    }
}

/// `NAirGfx.RANDOM_SCENARIO_ANIMATION_MAX_DELAY` as the game reads it:
/// fixed-point, and no less than zero.
pub(super) fn max_delay(defines: &Defines) -> Result<f32, &'static str> {
    defines
        .fixed5("NAirGfx", "RANDOM_SCENARIO_ANIMATION_MAX_DELAY")
        .map(|raw| raw.max(0) as f32 / FIXED5)
        .map_err(|_| "NDefines.NAirGfx.RANDOM_SCENARIO_ANIMATION_MAX_DELAY is unreadable")
}

/// Reads the authored building slots, types, states and map the site table
/// needs ([`site_table`]); a slot is admitted as the building layer admits it.
fn sites(
    layers: &ResolvedLayers,
    map: &MapData,
    defines: &Defines,
    roster: &mesh::Roster,
) -> io::Result<Sites> {
    let types = installed_content::buildings::read(layers)?;
    let (states, _) = installed_content::states::read(layers, defines, &map.definitions)?;
    let provinces = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    Ok(site_table(
        installed_content::positions::read_buildings(layers)?,
        &types,
        |spawn, position| {
            let province = provinces.at(position.x, position.z);
            admit_slot(spawn, position, province, map, states.definitions(), roster)
                == SlotAdmission::Admitted
        },
    ))
}

/// The first admitted authored slot of each state whose bucket holds a
/// rocket-site (`rocket_launch_capacity`) or gun-emplacement type, in file
/// order, by base kind: the slot the game's position finder reaches through
/// the state's site provinces. A slot whose bucket is declared more than
/// once differently decides nothing, so a site it may be is undecided.
pub(super) fn site_table(
    positions: Vec<BuildingPosition>,
    types: &Buildings,
    admitted: impl Fn(&SpawnPoint, &BuildingPosition) -> bool,
) -> Sites {
    let mut out = BTreeMap::new();
    for position in positions {
        if is_floating_harbor_slot(&position.kind) {
            continue;
        }
        let (spawn, ambiguous) = match types.slot(&position.kind) {
            None => continue,
            Some(Slot::Bucket(spawn)) => (spawn, false),
            Some(Slot::Ambiguous(spawn)) => (spawn, true),
        };
        let mut kinds = [false; 2];
        for candidate in types.slot_candidates(&spawn.name) {
            kinds[0] |= candidate.rocket_launch_capacity > 0;
            kinds[1] |= candidate.gun_emplacement;
        }
        for (kind, holds) in [ROCKET_SITE, GUN_EMPLACEMENT].into_iter().zip(kinds) {
            if !holds || out.contains_key(&(position.state, kind)) {
                continue;
            }
            if ambiguous {
                out.insert(
                    (position.state, kind),
                    Err("its state's first site slot names a building declared more than once differently"),
                );
            } else if admitted(spawn, &position) {
                out.insert(
                    (position.state, kind),
                    Ok([position.x, position.y, position.z]),
                );
            }
        }
    }
    out
}

/// The draws a launch's seed gives one record's stream: its heading (0),
/// its explosion province (1) or, once per launch, its delay.
fn draws(seed: u32, record: u32, stream: u32) -> Random {
    Random::new(SEED ^ seed ^ record.wrapping_mul(0x9e37_79b9) ^ stream.wrapping_mul(0x85eb_ca6b))
}

const DELAY_STREAM: u32 = 2;

/// A base's launch effect on the presentation clock.
#[derive(Clone, Debug)]
pub(super) struct Launch {
    pub mesh: usize,
    pub wing: u64,
    pub position: [f32; 3],
    /// Presentation seconds after its pass that a record spawns its entity.
    pub spawn: f32,
    /// Presentation seconds the entity plays until its root reads its
    /// clip's length; infinite when it never does, which holds its base.
    pub length: f32,
    pub seed: u32,
    /// A launch-flagged definition's explosion places: its mission region's
    /// provinces as the game lists them, with each one's standstill anchor
    /// and admission box when it has them.
    pub explosions: Option<ExplosionPlaces>,
}

impl Launch {
    /// A launch whose records wait `delay` aircraft-clock seconds, drawn
    /// once from its seed, on a clock of `clock_speed` per presentation
    /// second.
    pub(super) fn delay(seed: u32, max_delay: f32, clock_speed: f32) -> f32 {
        let unit = draws(seed, 0, DELAY_STREAM).next() as f32 * UNIT;
        unit * max_delay / clock_speed
    }

    /// Presentation seconds between its records: each comes on the first
    /// pass after the previous one's entity finishes.
    pub(super) fn period(&self) -> f32 {
        PASS_SECONDS * (((self.spawn + self.length) / PASS_SECONDS).floor() + 1.0)
    }

    /// The record at presentation time `seconds` and the seconds into it.
    fn at(&self, seconds: f32) -> (u32, f32) {
        let period = self.period();
        let seconds = seconds.max(0.0);
        // An entity that never finishes holds its first record.
        if !period.is_finite() {
            return (0, seconds);
        }
        let record = (seconds / period).floor();
        (record as u32, seconds - record * period)
    }

    fn random(&self, record: u32, stream: u32) -> Random {
        draws(self.seed, record, stream)
    }

    /// The heading a record's entity takes: a whole turn times a draw's
    /// remainder over the fixed-point scale.
    pub(super) fn heading(&self, record: u32) -> f32 {
        let fraction = (self.random(record, 0).next() % 100_000) as f32 / FIXED5;
        (fraction + fraction) * std::f32::consts::PI
    }

    /// The instance fields the `index`th launch's player reads.
    pub(super) fn instance_for(index: usize) -> ModelInstance {
        ModelInstance {
            scale: 1.0,
            animation_seed: INSTANCE_SEED ^ index as u32,
            ..ModelInstance::default()
        }
    }

    /// The entity at presentation time `seconds`, while a record shows it,
    /// with its clock's time since it spawned.
    pub(super) fn instance(&self, index: usize, seconds: f32) -> Option<ModelInstance> {
        let (record, into) = self.at(seconds);
        let age = into - self.spawn;
        (0.0..self.length).contains(&age).then(|| ModelInstance {
            position: self.position,
            rotation: self.heading(record),
            animation_epoch: record,
            animation_phase: age,
            ..Self::instance_for(index)
        })
    }

    /// The last record whose entity finishes after `from` and by `to`.
    pub(super) fn finished(&self, from: f32, to: f32) -> Option<u32> {
        if from.is_nan() || to.is_nan() || to <= from {
            return None;
        }
        let period = self.period();
        let end = self.spawn + self.length;
        let last = ((to - end) / period).floor();
        (last >= 0.0 && last * period + end > from).then_some(last as u32)
    }

    /// The record whose entity finishes at the clock's opening instant,
    /// which no `(from, to]` covers: one that spawns and ends at once.
    pub(super) fn finished_at_open(&self) -> Option<u32> {
        (self.spawn + self.length <= 0.0).then_some(0)
    }

    /// Where a launch-flagged record's explosion plays: a province drawn
    /// from its mission region's list, when it has a place.
    pub(super) fn explosion(&self, record: u32) -> Option<(u32, [f32; 3], [f32; 4])> {
        let places = self
            .explosions
            .as_ref()
            .filter(|places| !places.is_empty())?;
        let (province, place) = places[self.random(record, 1).next() as usize % places.len()];
        place.map(|(position, bounds)| (province, position, bounds))
    }
}
