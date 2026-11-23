use super::entity_animation::{self, Clock, Request};
use crate::{ModelInstance, ModelLibrary, WATER_HEIGHT};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_gpu::{camera::Camera, province_admission::Admission};
use dashboard_text::table_access::{Available, rows};

pub const NUCLEAR_ENTITY: &str = "nuke_entity";
pub const THERMONUCLEAR_ENTITY: &str = "nuke_big_entity";
pub const THERMONUCLEAR_TYPE: u32 = 1;
pub const CRITICAL_BOMBARDMENT_ENTITY: &str = "missile_explosion_entity_small";
pub const MISSILE_EXPLOSION_ENTITY: &str = "missile_explosion_entity";
pub const IMPACT_STATE: &str = "attack";
const SEED: u32 = 0x4e55_4b45;
const BOMBARDMENT_SEED: u32 = 0x4352_4954;
const MISSILE_SEED: u32 = 0x4d49_5353;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effect {
    Nuclear = 0,
    Thermonuclear = 1,
    CriticalBombardment = 2,
    MissileExplosion = 3,
}

pub type Clocks = BTreeMap<(usize, u32, u32), Clock>;

#[derive(Clone, Debug, PartialEq)]
pub struct Impact {
    pub id: u64,
    pub province: u32,
    pub effect: Effect,
    pub position: [f32; 3],
    pub bounds: [f32; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Impacts {
    pub impacts: Vec<Impact>,
    pub refused: Vec<String>,
}

impl Impacts {
    pub fn read(
        frame: &DecodedFrame,
        province: impl Fn(u32) -> Option<([f32; 3], [f32; 4])>,
    ) -> io::Result<Self> {
        let available = rows::<tables::NuclearStrikes>(frame)?;
        let refused = match &available {
            Available::Complete(_) => None,
            Available::Truncated { missing, .. } => Some(truncated(*missing)),
            Available::Unavailable { reason } => Some(unavailable(reason)),
            Available::NotRecorded { mode } => Some(format!(
                "NuclearStrikes not recorded in a {mode} recording: impacts have no effect"
            )),
        };
        let mut out = Self {
            impacts: Vec::new(),
            refused: refused.into_iter().collect(),
        };
        let unplaced = available
            .rows()
            .iter()
            .filter(|row| !out.locate(row.id, row.province, row.nuke_type, &province))
            .count();
        out.refused.extend(unplaced_refusal(unplaced));
        Ok(out)
    }

    pub fn locate(
        &mut self,
        id: u64,
        province: u32,
        nuke_type: u32,
        place: impl Fn(u32) -> Option<([f32; 3], [f32; 4])>,
    ) -> bool {
        let Some(impact) = impact(id, province, nuke_type, place) else {
            return false;
        };
        self.impacts.push(impact);
        true
    }
}

pub fn impact(
    id: u64,
    province: u32,
    nuke_type: u32,
    place: impl Fn(u32) -> Option<([f32; 3], [f32; 4])>,
) -> Option<Impact> {
    let (position, bounds) = place(province)?;
    Some(Impact {
        id,
        province,
        effect: if nuke_type == THERMONUCLEAR_TYPE {
            Effect::Thermonuclear
        } else {
            Effect::Nuclear
        },
        position,
        bounds,
    })
}

pub fn truncated(missing: u32) -> String {
    format!("NuclearStrikes truncated: {missing} impacts have no effect")
}

pub fn unavailable(reason: impl std::fmt::Display) -> String {
    format!("NuclearStrikes unavailable ({reason}): impacts have no effect")
}

pub fn unplaced_refusal(unplaced: usize) -> Option<String> {
    (unplaced > 0).then(|| format!(
        "{unplaced} nuclear impacts have no effect: their province has no standstill anchor or painted box"
    ))
}

#[derive(Clone, Debug, Default)]
pub struct Assets {
    entities: [Option<usize>; 4],
}

impl Assets {
    pub fn names() -> [&'static str; 4] {
        [
            NUCLEAR_ENTITY,
            THERMONUCLEAR_ENTITY,
            CRITICAL_BOMBARDMENT_ENTITY,
            MISSILE_EXPLOSION_ENTITY,
        ]
    }

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
    incarnation: u64,
}

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
    pub fn effect(&self, key: (usize, u32, u32)) -> Option<Effect> {
        self.active
            .values()
            .find(|active| active.key() == key)
            .map(|active| active.impact.effect)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Presentation {
    assets: Assets,
    active: BTreeMap<u32, Active>,
    newest: Option<u64>,
    started: u64,
}

impl Presentation {
    pub fn new(assets: Assets) -> Self {
        Self {
            assets,
            ..Default::default()
        }
    }

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

    pub fn advance(
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
            active.start = active.start.min(time);
            let finished = match played(active.key()) {
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
    use super::test_support::presentation;
    use super::*;
    use crate::{entity_animation::{Clip, Node, Player, Rig}, PaletteJoint};
    use content_reader_mesh::mesh::{self, EntityState, Transform};
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
        content_reader_script::test_game::enter();
        let descriptors = content_reader_mesh::mesh::parse_descriptors(
            br#"entity={name=nuke_entity state={name=attack event={time=0.5 particle=mist keep_particle=yes sound={soundeffect=blast}}}}"#,
            "gfx/synthetic.asset",
        )
        .unwrap();
        let library = ModelLibrary::load(
            &content_reader_layers::ResolvedLayers::default(),
            content_reader_image::native::Decode::default(),
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
        let mut shown = Presentation::new(assets);
        shown.receive([&hour(vec![impact(1, 7, false)])], false, 0.0);
        let mut pass = crate::ModelPass::new(&library, "map frame");
        let (placed, clocks) = shown.advance(true, |_| true, |_| None, 1.0);
        pass.update_placed(placed.clone(), 256.0);
        let struck: std::collections::BTreeSet<_> =
            clocks.keys().map(|&key| pass.actor_address(key)).collect();
        pass.set_clocks(clocks);
        pass.animate(1.0, None, &[0.0]);
        let mut sounds = Vec::new();
        crate::sound_events::collect(
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
        assert!((3.0..3.25).contains(&retired[&2]), "{retired:?}");
        assert!(retired[&3] <= 0.25, "{retired:?}");
    }

    #[test]
    fn retirement_waits_for_the_camera_to_admit_the_province() {
        let mut shown = presentation([Some(0), Some(0)]);
        shown.receive([&hour(vec![impact(1, 7, false)])], false, 0.0);
        let finished = |_| Some(Some((2.0, 1.0)));
        assert!(shown.advance(false, |_| true, finished, 3.0).0.is_empty());
        assert_eq!(shown.advance(true, |_| false, finished, 3.0).0.len(), 1);
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
        for (slot, instance) in &placed {
            let (seed, identity) = instance.animation_key();
            assert_eq!(
                shown.effect((*slot, seed, identity)),
                Some(Effect::CriticalBombardment)
            );
        }
        assert_eq!(shown.effect((5, 0, 0)), None);
        shown.receive([&read], false, 3.0);
        let (_, clocks) = shown.advance(true, |_| true, |_| None, 4.0);
        assert!(clocks.values().all(|clock| clock.time == 4.0));
    }

    #[test]
    fn a_later_impact_replaces_the_province_effect_even_when_it_cannot_be_created() {
        let mut shown = presentation([Some(3), None]);
        shown.receive([&hour(vec![impact(1, 7, false)])], false, 0.0);
        assert_eq!(shown.advance(true, |_| true, |_| None, 1.0).0.len(), 1);
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
        shown.receive([&struck], false, 5.0);
        let (_, clocks) = shown.advance(true, |_| true, |_| None, 6.0);
        assert!(clocks.values().all(|clock| clock.time == 6.0));
        shown.receive([&struck], true, 7.0);
        let (_, clocks) = shown.advance(true, |_| true, |_| None, 8.0);
        assert!(clocks.values().all(|clock| clock.time == 1.0));
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
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
