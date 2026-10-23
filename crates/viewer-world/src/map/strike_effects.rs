//! Impact effects: the transient effect entity of a struck province.
//!
//! Each impact in `NuclearStrikes` places one engine-named entity in the impact
//! province's transient effect slot: [`THERMONUCLEAR_ENTITY`] for the
//! thermonuclear type ordinal, [`NUCLEAR_ENTITY`] for every other type. Each
//! critical shore bombardment in `MapEvents` places
//! [`CRITICAL_BOMBARDMENT_ENTITY`] in the bombarded province's slot, as the
//! game does on the critical branch for every viewer. Each missile bombing
//! scenario of the flying aircraft places [`MISSILE_EXPLOSION_ENTITY`] in
//! its scenario province's slot when the scenario finishes
//! ([`super::aircraft::AircraftPlan::missile_explosions`]), as the game's
//! regional scenario update does, and in a drawn province of its mission's
//! region when a launch-flagged launch effect's entity finishes, through
//! the same province effect call as a nuclear impact: the explosion stands on the standstill anchor while
//! the rocket lands at the scenario's anchor (the province's integer
//! centroid), as in the game. The names resolve
//! through the recorded content stack, so a mod's definitions, meshes,
//! events, particles and lights are what play. The entity stands unrotated,
//! at object scale one, on the province's standstill unit-stack anchor, and
//! is asked once for [`IMPACT_STATE`]. A province holds one effect: a later
//! impact destroys the earlier entity first, so an impact whose entity the
//! content lacks leaves the slot empty. Within one hour the nuclear impacts
//! come before the critical bombardments; the game's order between the two
//! inside an hour is not recorded. Missile explosions start on the aircraft
//! presentation clock, which the flying aircraft share.
//!
//! The effect is shown while the camera's water-relative height is below
//! `NGraphics.DRAW_MAP_OBJECTS_CUTOFF`. An update that shows it while the
//! camera admits its province's box retires it once the root's played
//! animation has reached its length, read from the entity's own player; a root
//! that plays no animation reads zero against zero and retires.
//!
//! The impact's instant inside its hour, the renderer's clock and particle
//! seeds are not recorded. An effect starts on the presentation clock when the
//! hour holding its impact is presented, and during continuing playback also
//! for the built hours a forward skip passes; its state and events then follow
//! the authored timeline. Seeks, step backs, branch changes, gaps and
//! viewing-country changes clear the slots and start only the presented hour's
//! impacts.
use super::{
    entity_animation::{self, Clock, Request},
    models::{ModelInstance, ModelLibrary, WATER_HEIGHT},
};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_foundation::{
    map::{camera::Camera, province_admission::Admission},
    table_access::{Available, rows},
};

/// The entity the game creates for an impact of any type but the thermonuclear one.
pub const NUCLEAR_ENTITY: &str = "nuke_entity";
/// The entity the game creates for an impact of [`THERMONUCLEAR_TYPE`].
pub const THERMONUCLEAR_ENTITY: &str = "nuke_big_entity";
/// The recorded type ordinal of a thermonuclear strike (`NuclearStrikeRow::nuke_type`).
pub const THERMONUCLEAR_TYPE: u32 = 1;
/// The entity the game creates on a critical shore bombardment's province.
pub const CRITICAL_BOMBARDMENT_ENTITY: &str = "missile_explosion_entity_small";
/// The entity the game creates where a missile bombing scenario finishes.
pub const MISSILE_EXPLOSION_ENTITY: &str = "missile_explosion_entity";
/// The state the game requests on the new effect entity.
pub const IMPACT_STATE: &str = "attack";
/// Effect animation seeds are this XOR the impact id's low word.
const SEED: u32 = 0x4e55_4b45;
/// A critical bombardment's seed is this XOR its event sequence's low word.
const BOMBARDMENT_SEED: u32 = 0x4352_4954;
/// A missile explosion's seed is this XOR the high word of its mixed start.
const MISSILE_SEED: u32 = 0x4d49_5353;

/// Which effect entity an impact plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effect {
    Nuclear = 0,
    Thermonuclear = 1,
    CriticalBombardment = 2,
    MissileExplosion = 3,
}

/// The animation clocks the shown effects run on, keyed as the model pass keys players.
pub type Clocks = BTreeMap<(usize, u32, u32), Clock>;

/// One recorded impact, located.
#[derive(Clone, Debug, PartialEq)]
pub struct Impact {
    /// The NuclearStrikes id, the MapEvents sequence or the missile
    /// scenario's flight and cycle in its hour's plan, by [`Effect`]. The
    /// next hour's plan numbers its flights again, so a missile explosion's
    /// player is keyed by its start ([`Presentation`]), not by this.
    pub id: u64,
    pub province: u32,
    pub effect: Effect,
    /// The province's standstill unit-stack anchor.
    pub position: [f32; 3],
    /// The province's admission box `[min x, min z, max x, max z]`
    /// ([`viewer_foundation::map::province_admission::province_boxes`]).
    pub bounds: [f32; 4],
}

/// One hour's impacts in recorded order, and the ones it cannot place.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Impacts {
    pub impacts: Vec<Impact>,
    /// Named, counted reasons an impact or the table could not be placed.
    pub refused: Vec<String>,
}

impl Impacts {
    /// Reads the hour's `NuclearStrikes`. `province` gives a province's
    /// standstill unit-stack anchor, authored or default, and its admission
    /// box. Critical shore bombardments arrive through the map text consumer
    /// ([`super::animated_map_text::events::Consumer`]), which presents each
    /// recorded event once.
    pub fn read(
        frame: &DecodedFrame,
        province: impl Fn(u32) -> Option<([f32; 3], [f32; 4])>,
    ) -> io::Result<Self> {
        let mut out = Self::default();
        let available = rows::<tables::NuclearStrikes>(frame)?;
        match &available {
            Available::Complete(_) => {}
            Available::Truncated { missing, .. } => out.refused.push(format!(
                "NuclearStrikes truncated: {missing} impacts have no effect"
            )),
            Available::Unavailable { reason } => out.refused.push(format!(
                "NuclearStrikes unavailable ({reason}): impacts have no effect"
            )),
        }
        let mut unplaced = 0usize;
        for row in available.rows() {
            let Some((position, bounds)) = province(row.province) else {
                unplaced += 1;
                continue;
            };
            out.impacts.push(Impact {
                id: row.id,
                province: row.province,
                effect: if row.nuke_type == THERMONUCLEAR_TYPE {
                    Effect::Thermonuclear
                } else {
                    Effect::Nuclear
                },
                position,
                bounds,
            });
        }
        if unplaced > 0 {
            out.refused.push(format!(
                "{unplaced} nuclear impacts have no effect: their province has no standstill anchor or painted box"
            ));
        }
        Ok(out)
    }
}

/// The effect entities' slots in the frame library, by [`Effect`].
#[derive(Clone, Debug, Default)]
pub struct Assets {
    entities: [Option<usize>; 4],
}

impl Assets {
    /// The entity names the library must load, by [`Effect`].
    pub fn names() -> [&'static str; 4] {
        [
            NUCLEAR_ENTITY,
            THERMONUCLEAR_ENTITY,
            CRITICAL_BOMBARDMENT_ENTITY,
            MISSILE_EXPLOSION_ENTITY,
        ]
    }

    /// Resolves the loaded effect entities; an entity the content does not
    /// declare, or whose resources failed, is named in `warnings`.
    pub fn resolve(library: &ModelLibrary, warnings: &mut Vec<String>) -> Self {
        let entities = Self::names().map(|name| {
            let slot = library.slot(name);
            if slot.is_none() {
                warnings.push(format!(
                    "Impact effect entity {name} is not available: its impacts have no effect"
                ));
            }
            slot
        });
        Self { entities }
    }

    fn entity(&self, effect: Effect) -> Option<usize> {
        self.entities[effect as usize]
    }
}

#[derive(Clone, Debug)]
struct Active {
    impact: Impact,
    slot: usize,
    start: f32,
    /// The ordinal of this effect's start in its presentation.
    incarnation: u64,
}

/// A missile explosion's start ordinal as two words, through a bijective
/// mix, so distinct explosions keep distinct players.
fn missile_words(incarnation: u64) -> (u32, u32) {
    let mut x = incarnation;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^= x >> 33;
    ((x >> 32) as u32, x as u32)
}

impl Active {
    fn instance(&self) -> ModelInstance {
        let (seed, identity) = match self.impact.effect {
            Effect::CriticalBombardment => (
                BOMBARDMENT_SEED ^ self.impact.id as u32,
                self.impact.id as u32,
            ),
            Effect::MissileExplosion => {
                let (high, low) = missile_words(self.incarnation);
                (MISSILE_SEED ^ high, low)
            }
            Effect::Nuclear | Effect::Thermonuclear => {
                (SEED ^ self.impact.id as u32, self.impact.id as u32)
            }
        };
        ModelInstance {
            position: self.impact.position,
            scale: 1.0,
            animation: entity_animation::named_request(IMPACT_STATE),
            animation_seed: seed,
            animation_identity: identity.wrapping_add(1).max(1),
            ..Default::default()
        }
    }

    fn key(&self) -> (usize, u32, u32) {
        let (seed, identity) = self.instance().animation_key();
        (self.slot, seed, identity)
    }
}

impl Presentation {
    /// The effect a shown entity, keyed as [`Presentation::update`] places it, plays.
    pub fn effect(&self, key: (usize, u32, u32)) -> Option<Effect> {
        self.active
            .values()
            .find(|active| active.key() == key)
            .map(|active| active.impact.effect)
    }
}

/// Viewer-owned effect slots, one per struck province.
#[derive(Clone, Debug, Default)]
pub struct Presentation {
    assets: Assets,
    active: BTreeMap<u32, Active>,
    /// NuclearStrikes ids increase through a recording: a re-presented hour
    /// starts nothing twice.
    newest: Option<u64>,
    /// Effects started so far, never reset: each start is a new incarnation,
    /// so a missile explosion whose flight and cycle a later plan reuses,
    /// in its own or another province, never shares an earlier one's player.
    started: u64,
}

impl Presentation {
    pub fn new(assets: Assets) -> Self {
        Self {
            assets,
            ..Default::default()
        }
    }

    /// Starts the impacts of presented hours, oldest first, at `time`.
    /// `reset` clears every slot first.
    pub fn receive<'a>(
        &mut self,
        hours: impl IntoIterator<Item = &'a Impacts>,
        reset: bool,
        time: f32,
    ) {
        if reset {
            self.active.clear();
            self.newest = None;
        }
        for impact in hours.into_iter().flat_map(|hour| &hour.impacts) {
            if self.newest.is_some_and(|newest| impact.id <= newest) {
                continue;
            }
            self.newest = Some(impact.id);
            self.start(impact, time);
        }
    }

    /// Starts critical shore bombardments the map text consumer presents
    /// for the first time, after their hour's nuclear impacts, and the
    /// missile explosions of scenarios that finished since the last frame.
    pub fn receive_explosions<'a>(
        &mut self,
        impacts: impl IntoIterator<Item = &'a Impact>,
        time: f32,
    ) {
        for impact in impacts {
            self.start(impact, time);
        }
    }

    fn start(&mut self, impact: &Impact, time: f32) {
        // The previous entity goes even when the new one cannot be created.
        self.active.remove(&impact.province);
        let Some(slot) = self.assets.entity(impact.effect) else {
            return;
        };
        self.started = self.started.wrapping_add(1);
        self.active.insert(
            impact.province,
            Active {
                impact: impact.clone(),
                slot,
                start: time,
                incarnation: self.started,
            },
        );
    }

    /// Retires finished effects and returns the shown ones with their clocks.
    /// `played` reads an effect's player under its key, as
    /// [`super::models::ModelPass::root_play_position`] does.
    pub fn update(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        time: f32,
        played: impl Fn((usize, u32, u32)) -> Option<Option<(f32, f32)>>,
    ) -> (Vec<(usize, ModelInstance)>, Clocks) {
        let shown = camera.height - WATER_HEIGHT < camera.defines.map_objects_cutoff;
        let admission = Admission::new(camera, size);
        self.advance(shown, |bounds| admission.admits(bounds), played, time)
    }

    fn advance(
        &mut self,
        shown: bool,
        in_view: impl Fn([f32; 4]) -> bool,
        played: impl Fn((usize, u32, u32)) -> Option<Option<(f32, f32)>>,
        time: f32,
    ) -> (Vec<(usize, ModelInstance)>, Clocks) {
        let mut placed = Vec::new();
        let mut clocks = BTreeMap::new();
        if !shown {
            return (placed, clocks);
        }
        self.active.retain(|_, active| {
            // A presentation clock that starts again starts the effect with it.
            active.start = active.start.min(time);
            let finished = match played(active.key()) {
                // Not created yet: nothing has played.
                None => false,
                Some(None) => true,
                Some(Some((position, length))) => position >= length,
            };
            !(in_view(active.impact.bounds) && finished)
        });
        let requests = Arc::new(vec![Request {
            time: 0.0,
            phase_time: None,
            state: IMPACT_STATE.into(),
        }]);
        for active in self.active.values() {
            placed.push((active.slot, active.instance()));
            clocks.insert(
                active.key(),
                Clock {
                    time: time - active.start,
                    phase: None,
                    requests: Arc::clone(&requests),
                },
            );
        }
        (placed, clocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{
        entity_animation::{Clip, Node, Player, Rig},
        models::PaletteJoint,
    };
    use installed_content_core::{mesh, mesh::EntityState, mesh::Transform};
    use replay_data::{rows::NuclearStrikeRow, rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};

    fn empty_frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 42,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Metrics::default(),
        }
    }

    fn impact(id: u64, province: u32, thermonuclear: bool) -> Impact {
        Impact {
            id,
            province,
            effect: if thermonuclear {
                Effect::Thermonuclear
            } else {
                Effect::Nuclear
            },
            position: [province as f32, 10.0, 1.0],
            bounds: [province as f32, 0.0, province as f32 + 2.0, 2.0],
        }
    }

    fn hour(impacts: Vec<Impact>) -> Impacts {
        Impacts {
            impacts,
            refused: Vec::new(),
        }
    }

    fn presentation(slots: [Option<usize>; 2]) -> Presentation {
        Presentation::new(Assets {
            entities: [slots[0], slots[1], Some(5), Some(6)],
        })
    }

    /// A clip of `seconds` (at one sample a second) for `state`.
    fn clip(state: &str, seconds: u32, chance: f32, looping: bool) -> Clip {
        let mut animation = mesh::Animation::default();
        animation.fps = 1.0;
        animation.samples = seconds + 1;
        Clip {
            state: EntityState {
                name: state.into(),
                chance,
                speed: 1.0,
                looping,
                ..EntityState::default()
            },
            animation: Some(Arc::new(animation)),
        }
    }

    fn rig(clips: Vec<Clip>) -> Rig {
        Rig {
            nodes: vec![Node {
                mesh: Arc::new(mesh::Mesh::default()),
                native: None,
                clips,
                default: "idle".into(),
                follows_parent: false,
                parent: None,
                scale: 1.0,
                mesh_scale: 1.0,
                skin_base: None,
                rigid: 0,
                locators: BTreeMap::new(),
            }],
            start: 0,
            end: 1,
        }
    }

    /// Plays the presentation against real players of `rigs`, one frame a
    /// tenth of a second, and returns the time each impact's effect retired.
    fn retirements(rigs: &[Rig], mut shown: Presentation, until: f32) -> BTreeMap<u32, f32> {
        let mut players: BTreeMap<(usize, u32, u32), Player> = BTreeMap::new();
        let mut retired = BTreeMap::new();
        for step in 1..=(until * 10.0) as u32 {
            let time = step as f32 / 10.0;
            let live: std::collections::BTreeSet<u32> = shown
                .active
                .values()
                .map(|active| active.instance().animation_identity)
                .collect();
            let (placed, clocks) = shown.advance(
                true,
                |_| true,
                // As the model pass reads it: a clipless entity has no player.
                |key| {
                    let rig = &rigs[key.0];
                    if rig.nodes.iter().all(|node| node.clips.is_empty()) {
                        return Some(None);
                    }
                    Some(players.get(&key)?.root_play_position(rig))
                },
                time,
            );
            let now: std::collections::BTreeSet<u32> = placed
                .iter()
                .map(|(_, instance)| instance.animation_identity)
                .collect();
            for gone in live.difference(&now) {
                retired.insert(*gone, time);
            }
            for (slot, instance) in placed {
                let key = (slot, instance.animation_seed, instance.animation_identity);
                let player = players
                    .entry(key)
                    .or_insert_with(|| Player::new(&rigs[slot]));
                let mut palette = [PaletteJoint::from_transform(&Transform::IDENTITY)];
                player.evaluate_clock(
                    &rigs[slot],
                    instance,
                    &clocks[&key],
                    &mut palette,
                    &mut Vec::new(),
                );
            }
        }
        retired
    }

    #[test]
    fn the_type_ordinal_selects_the_entity_and_the_standstill_anchor_places_it() {
        let mut frame = empty_frame();
        *frame.tables[TableId::NuclearStrikes.index()] =
            TableView::Complete(TableRows::NuclearStrikes(vec![
                NuclearStrikeRow {
                    id: 4,
                    nuke_type: THERMONUCLEAR_TYPE,
                    province: 7,
                    ..Default::default()
                },
                NuclearStrikeRow {
                    id: 5,
                    nuke_type: 0,
                    province: 8,
                    ..Default::default()
                },
                NuclearStrikeRow {
                    id: 6,
                    nuke_type: 2,
                    province: 9,
                    ..Default::default()
                },
            ]));
        let read = Impacts::read(&frame, |province| {
            (province != 9).then_some(([province as f32, 2.0, 3.0], [0.0, 0.0, 4.0, 4.0]))
        })
        .unwrap();
        assert_eq!(
            read.impacts,
            [
                Impact {
                    id: 4,
                    province: 7,
                    effect: Effect::Thermonuclear,
                    position: [7.0, 2.0, 3.0],
                    bounds: [0.0, 0.0, 4.0, 4.0],
                },
                Impact {
                    id: 5,
                    province: 8,
                    effect: Effect::Nuclear,
                    position: [8.0, 2.0, 3.0],
                    bounds: [0.0, 0.0, 4.0, 4.0],
                },
            ]
        );
        assert_eq!(read.refused.len(), 1, "{:?}", read.refused);
        let mut shown = presentation([Some(3), Some(4)]);
        shown.receive([&read], false, 0.0);
        let (placed, clocks) = shown.advance(true, |_| true, |_| None, 1.0);
        assert_eq!(
            placed.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
            [4, 3]
        );
        let (slot, instance) = placed[0];
        assert_eq!(instance.position, [7.0, 2.0, 3.0]);
        assert_eq!((instance.rotation, instance.scale), (0.0, 1.0));
        let (seed, identity) = instance.animation_key();
        let clock = &clocks[&(slot, seed, identity)];
        assert_eq!(clock.time, 1.0);
        assert_eq!(clock.requests[0].state, IMPACT_STATE);
    }

    #[test]
    fn content_declares_the_effect_entities_and_a_missing_one_is_named() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let descriptors = installed_content_core::mesh::parse_descriptors(
            br#"entity={name=nuke_entity state={name=attack event={time=0.5 particle=mist keep_particle=yes sound={soundeffect=blast}}}}"#,
            "gfx/synthetic.asset",
        )
        .unwrap();
        let library = ModelLibrary::load(
            &installed_content_core::ResolvedLayers::default(),
            installed_content_image::native::Decode::default(),
            &descriptors,
            &[NUCLEAR_ENTITY.into()],
            &mut Vec::new(),
        )
        .unwrap();
        let mut warnings = Vec::new();
        let assets = Assets::resolve(&library, &mut warnings);
        assert_eq!(assets.entity(Effect::Nuclear), library.slot(NUCLEAR_ENTITY));
        assert_eq!(assets.entity(Effect::Thermonuclear), None);
        assert_eq!(assets.entity(Effect::CriticalBombardment), None);
        assert_eq!(assets.entity(Effect::MissileExplosion), None);
        assert_eq!(warnings.len(), 3);
        assert!(warnings[0].contains(THERMONUCLEAR_ENTITY), "{warnings:?}");
        assert!(
            warnings[1].contains(CRITICAL_BOMBARDMENT_ENTITY),
            "{warnings:?}"
        );
        assert!(
            warnings[2].contains(MISSILE_EXPLOSION_ENTITY),
            "{warnings:?}"
        );
        // The pass keys an effect's player by its identity, and the native
        // draw finds the pose under that same key.
        let mut shown = Presentation::new(assets);
        shown.receive([&hour(vec![impact(1, 7, false)])], false, 0.0);
        let mut pass = super::super::models::ModelPass::new(&library, "map frame");
        let (placed, clocks) = shown.advance(true, |_| true, |_| None, 1.0);
        pass.update_placed(placed.clone(), 256.0);
        let struck: std::collections::BTreeSet<_> =
            clocks.keys().map(|&key| pass.actor_address(key)).collect();
        pass.set_clocks(clocks);
        pass.animate(1.0, None, &[0.0]);
        // The impact's sound is a combat sound: its event is found under the
        // effect's clock key.
        let mut sounds = Vec::new();
        crate::map::sound_events::collect(
            &pass.events,
            &mut Default::default(),
            &mut sounds,
            |event| struck.contains(&event.key.player()),
        );
        assert_eq!(sounds.len(), 1);
        assert_eq!(&*sounds[0].effect, "blast");
        assert!(sounds[0].combat);
        let (slot, instance) = placed[0];
        assert_eq!(instance.animation_epoch, 0);
        assert!(pass.player(slot, &instance).is_some());
        // An event-only root plays no animation: it retires on its first check.
        let (seed, identity) = instance.animation_key();
        assert_eq!(pass.root_play_position((slot, seed, identity)), Some(None));
    }

    #[test]
    fn an_unavailable_table_is_named() {
        let mut frame = empty_frame();
        *frame.tables[TableId::NuclearStrikes.index()] = TableView::Unavailable {
            reason: "not captured".into(),
        };
        let read = Impacts::read(&frame, |_| Some(([0.0; 3], [0.0; 4]))).unwrap();
        assert!(read.impacts.is_empty());
        assert!(
            read.refused[0].contains("not captured"),
            "{:?}",
            read.refused
        );
    }

    #[test]
    fn an_effect_retires_when_the_variant_its_player_chose_ends() {
        // A one-second and a ten-second variant of the requested state.
        let rigs = [rig(vec![
            clip("idle", 1, 1.0, true),
            clip(IMPACT_STATE, 1, 0.5, false),
            clip(IMPACT_STATE, 10, 0.5, false),
        ])];
        let mut lengths = std::collections::BTreeSet::new();
        for id in 1..40 {
            let mut shown = presentation([Some(0), Some(0)]);
            shown.receive([&hour(vec![impact(id, 7, false)])], false, 0.0);
            let identity = shown.active[&7].instance().animation_identity;
            let retired = retirements(&rigs, shown, 12.0);
            // The variant this impact's player entered.
            let mut player = Player::new(&rigs[0]);
            let active = Active {
                impact: impact(id, 7, false),
                slot: 0,
                start: 0.0,
                incarnation: 1,
            };
            let mut palette = [PaletteJoint::from_transform(&Transform::IDENTITY)];
            player.evaluate_clock(
                &rigs[0],
                active.instance(),
                &Clock {
                    time: 0.5,
                    phase: None,
                    requests: Arc::new(vec![Request {
                        time: 0.0,
                        phase_time: None,
                        state: IMPACT_STATE.into(),
                    }]),
                },
                &mut palette,
                &mut Vec::new(),
            );
            let length = player.root_play_position(&rigs[0]).unwrap().1;
            let at = retired[&identity];
            // Retired on the first update at or past the chosen variant's end.
            assert!(
                at >= length && at <= length + 0.25,
                "{id}: {at} for {length}"
            );
            lengths.insert(length as u32);
        }
        assert_eq!(lengths, [1, 10].into(), "both variants were drawn");
    }

    #[test]
    fn an_absent_state_keeps_the_animated_default_until_it_ends() {
        let rigs = [rig(vec![clip("idle", 3, 1.0, false)]), rig(Vec::new())];
        let mut shown = presentation([Some(0), Some(1)]);
        shown.receive(
            [&hour(vec![impact(1, 7, false), impact(2, 8, true)])],
            false,
            0.0,
        );
        let retired = retirements(&rigs, shown, 5.0);
        // The default clip plays on and ends at three seconds.
        assert!((3.0..3.25).contains(&retired[&2]), "{retired:?}");
        // A root that plays nothing retires once its player reads it.
        assert!(retired[&3] <= 0.25, "{retired:?}");
    }

    #[test]
    fn retirement_waits_for_the_camera_to_admit_the_province() {
        let mut shown = presentation([Some(0), Some(0)]);
        shown.receive([&hour(vec![impact(1, 7, false)])], false, 0.0);
        let finished = |_| Some(Some((2.0, 1.0)));
        // Above the cutoff nothing is shown or retired.
        assert!(shown.advance(false, |_| true, finished, 3.0).0.is_empty());
        // A province the camera does not admit keeps its finished effect.
        assert_eq!(shown.advance(true, |_| false, finished, 3.0).0.len(), 1);
        // The province's box, not the anchor, is what the camera admits.
        let bounds = impact(1, 7, false).bounds;
        assert!(
            shown
                .advance(true, |admitted| admitted == bounds, finished, 3.0)
                .0
                .is_empty()
        );
    }

    #[test]
    fn a_critical_bombardment_plays_its_explosion_after_the_hours_nuclear_impacts() {
        let mut frame = empty_frame();
        *frame.tables[TableId::NuclearStrikes.index()] =
            TableView::Complete(TableRows::NuclearStrikes(vec![NuclearStrikeRow {
                id: 9,
                province: 7,
                ..Default::default()
            }]));
        let read = Impacts::read(&frame, |province| {
            Some(([province as f32, 0.0, 0.0], [0.0, 0.0, 1.0, 1.0]))
        })
        .unwrap();
        assert!(read.refused.is_empty(), "{:?}", read.refused);
        let critical = |id, province: u32| Impact {
            id,
            province,
            effect: Effect::CriticalBombardment,
            position: [province as f32, 0.0, 0.0],
            bounds: [0.0, 0.0, 1.0, 1.0],
        };
        // Sequence 2 after nuclear id 9 still starts: the map text consumer
        // presents each recorded event once.
        let mut shown = presentation([Some(3), Some(4)]);
        shown.receive([&read], false, 0.0);
        shown.receive_explosions(&[critical(2, 7), critical(5, 8)], 0.0);
        let placed = shown.advance(true, |_| true, |_| None, 1.0).0;
        assert_eq!(
            placed
                .iter()
                .map(|(slot, instance)| (*slot, instance.position[0]))
                .collect::<Vec<_>>(),
            [(5, 7.0), (5, 8.0)]
        );
        // Each shown entity names its effect, so a viewer can hide one kind.
        for (slot, instance) in &placed {
            let (seed, identity) = instance.animation_key();
            assert_eq!(
                shown.effect((*slot, seed, identity)),
                Some(Effect::CriticalBombardment)
            );
        }
        assert_eq!(shown.effect((5, 0, 0)), None);
        // Re-presenting the hour starts no nuclear impact again.
        shown.receive([&read], false, 3.0);
        let (_, clocks) = shown.advance(true, |_| true, |_| None, 4.0);
        assert!(clocks.values().all(|clock| clock.time == 4.0));
    }

    #[test]
    fn a_later_impact_replaces_the_province_effect_even_when_it_cannot_be_created() {
        let mut shown = presentation([Some(3), None]);
        shown.receive([&hour(vec![impact(1, 7, false)])], false, 0.0);
        assert_eq!(shown.advance(true, |_| true, |_| None, 1.0).0.len(), 1);
        // The thermonuclear entity is missing: the slot is emptied anyway.
        shown.receive([&hour(vec![impact(2, 7, true)])], false, 1.0);
        assert!(shown.advance(true, |_| true, |_| None, 2.0).0.is_empty());
    }

    #[test]
    fn a_later_impact_replaces_the_province_effect_and_none_starts_twice() {
        let mut shown = presentation([Some(3), Some(4)]);
        let struck = hour(vec![
            impact(1, 7, false),
            impact(2, 7, true),
            impact(3, 8, false),
        ]);
        shown.receive([&struck], false, 0.0);
        let identities = shown
            .advance(true, |_| true, |_| None, 1.0)
            .0
            .into_iter()
            .map(|(slot, instance)| (slot, instance.animation_identity))
            .collect::<Vec<_>>();
        assert_eq!(identities, [(4, 3), (3, 4)]);
        // Presenting the hour again keeps the running clocks.
        shown.receive([&struck], false, 5.0);
        let (_, clocks) = shown.advance(true, |_| true, |_| None, 6.0);
        assert!(clocks.values().all(|clock| clock.time == 6.0));
        // A seek clears the slots and starts the hour's impacts again.
        shown.receive([&struck], true, 7.0);
        let (_, clocks) = shown.advance(true, |_| true, |_| None, 8.0);
        assert!(clocks.values().all(|clock| clock.time == 1.0));
        // A clock that starts again restarts the effects.
        let (_, clocks) = shown.advance(true, |_| true, |_| None, 0.5);
        assert!(clocks.values().all(|clock| clock.time == 0.0));
    }
    #[test]
    fn a_missile_explosion_replaces_its_provinces_effect_with_its_own_entity() {
        let mut shown = presentation([Some(0), Some(1)]);
        shown.receive([&hour(vec![impact(1, 7, false)])], false, 0.0);
        let missile = Impact {
            id: 3 << 32 | 2,
            province: 7,
            effect: Effect::MissileExplosion,
            position: [7.0, 10.0, 1.0],
            bounds: [7.0, 0.0, 9.0, 2.0],
        };
        shown.receive_explosions(&[missile], 1.0);
        let (placed, clocks) = shown.advance(true, |_| true, |_| None, 1.5);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].0, 6);
        // The nuclear effect was the first start, the explosion the second.
        assert_eq!(
            placed[0].1.animation_seed,
            MISSILE_SEED ^ missile_words(2).0
        );
        assert_eq!(clocks.values().next().unwrap().time, 0.5);
        assert_eq!(
            Assets::names()[Effect::MissileExplosion as usize],
            MISSILE_EXPLOSION_ENTITY
        );
    }
    #[test]
    fn missile_explosions_in_two_provinces_keep_their_own_players() {
        let explosions = crate::map::aircraft::AircraftPlan::rocket_explosions_for_tests(
            &[(7, [7.0, 1.0, 1.0]), (9, [9.0, 1.0, 1.0])],
            &[1.0, 2.5],
        )
        .concat();
        // Both rockets end their first cycle on the first frame of the rig.
        assert_eq!(
            explosions
                .iter()
                .map(|impact| (impact.province, impact.effect))
                .collect::<Vec<_>>(),
            [(7, Effect::MissileExplosion), (9, Effect::MissileExplosion)]
        );
        let mut shown = presentation([Some(0), Some(1)]);
        shown.receive_explosions(&explosions, 2.5);
        let (placed, clocks) = shown.advance(true, |_| true, |_| None, 3.0);
        assert_eq!(placed.len(), 2);
        assert!(placed.iter().all(|(slot, _)| *slot == 6));
        let keys: std::collections::BTreeSet<_> = placed
            .iter()
            .map(|(slot, instance)| {
                let (seed, identity) = instance.animation_key();
                (*slot, seed, identity)
            })
            .collect();
        assert_eq!(keys.len(), 2);
        assert_eq!(clocks.len(), 2);
        // The next cycle in the same province takes a new player.
        let next = crate::map::aircraft::AircraftPlan::rocket_explosions_for_tests(
            &[(7, [7.0, 1.0, 1.0]), (9, [9.0, 1.0, 1.0])],
            &[1.0, 2.5, 4.5],
        )
        .pop()
        .unwrap();
        assert_eq!(next.len(), 2);
        assert!(
            next.iter()
                .all(|impact| !explosions.iter().any(|old| old.id == impact.id))
        );
    }

    /// The next hour's plan numbers its flights again: its first flight's
    /// first explosion has the earlier plan's flight and cycle, here in
    /// another province while the earlier explosion still plays. Each keeps
    /// its own player and clock, and the earlier one is not restarted; an
    /// explosion that replaces its province's effect takes a new player too.
    /// Before, the two shared one key, so one clock overwrote the other.
    #[test]
    fn missile_explosions_of_two_plans_keep_their_own_players() {
        let first = crate::map::aircraft::AircraftPlan::rocket_explosions_for_tests(
            &[(7, [7.0, 1.0, 1.0])],
            &[1.0, 2.5],
        )
        .concat();
        let second = crate::map::aircraft::AircraftPlan::rocket_explosions_for_tests(
            &[(9, [9.0, 1.0, 1.0])],
            &[1.0, 2.5],
        )
        .concat();
        assert_eq!((first.len(), second.len()), (1, 1));
        assert_eq!(first[0].id, second[0].id);
        assert_ne!(first[0].province, second[0].province);
        let key = |instance: &ModelInstance, slot: usize| {
            let (seed, identity) = instance.animation_key();
            (slot, seed, identity)
        };
        let mut shown = presentation([Some(0), Some(1)]);
        shown.receive_explosions(&first, 2.5);
        let (placed, _) = shown.advance(true, |_| true, |_| None, 3.0);
        let earlier = key(&placed[0].1, placed[0].0);
        shown.receive_explosions(&second, 4.0);
        let (placed, clocks) = shown.advance(true, |_| true, |_| None, 4.5);
        let keys: std::collections::BTreeSet<_> = placed
            .iter()
            .map(|(slot, instance)| key(instance, *slot))
            .collect();
        assert_eq!(keys.len(), 2);
        assert_eq!(clocks.len(), 2);
        assert_eq!(clocks[&earlier].time, 2.0);
        let later = *keys.iter().find(|&&other| other != earlier).unwrap();
        assert_eq!(clocks[&later].time, 0.5);
        // A third plan's explosion in the earlier province replaces its
        // effect with a new player.
        shown.receive_explosions(&first, 5.0);
        let (placed, clocks) = shown.advance(true, |_| true, |_| None, 5.5);
        let keys: std::collections::BTreeSet<_> = placed
            .iter()
            .map(|(slot, instance)| key(instance, *slot))
            .collect();
        assert_eq!(keys.len(), 2);
        assert!(!keys.contains(&earlier));
        assert!(keys.contains(&later));
        assert_eq!(clocks[&later].time, 1.5);
    }
}
