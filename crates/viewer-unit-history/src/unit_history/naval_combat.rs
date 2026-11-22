
use std::collections::BTreeSet;
use std::{collections::BTreeMap, sync::Arc};

use super::{Budget, Frame, Part, Target};
use viewer_models::{
    entity_animation::{self, Clock, Phase, Player, Random, Request, Rig},
    mesh_pose::{self, Matrix},
    ModelInstance,
    PaletteJoint,
};

pub mod impacts;
#[cfg(test)]
mod tests;

#[derive(Clone, Default)]
pub struct Sources {
    pub battles: BTreeMap<u64, Battle>,
    pub links: BTreeMap<u64, [u64; 2]>,
    pub roots: BTreeMap<u32, ModelInstance>,
}

#[derive(Clone)]
pub struct Battle {
    pub id: u64,
    pub stamp: i32,
    pub root: Target,
    pub rig: Arc<Rig>,
    pub palette: Arc<Vec<PaletteJoint>>,
    pub locators: [Option<String>; 8],
    pub hulls: [Option<Target>; 8],
    pub offered: [bool; 8],
    pub impacts: [Option<Arc<impacts::Info>>; 8],
    pub effects: [Vec<Option<(usize, u32)>>; 4],
    pub health: [Health; 2],
    pub death_time: f32,
}

impl Battle {
    pub fn targets(&self) -> impl Iterator<Item = Target> + '_ {
        std::iter::once(self.root)
            .chain(self.hulls.iter().copied().flatten())
            .chain(
                self.effects
                    .iter()
                    .flatten()
                    .filter_map(|effect| *effect)
                    .map(|(mesh, animation)| Target {
                        mesh,
                        instance: ModelInstance {
                            animation,
                            ..self.root.instance
                        },
                        ..self.root
                    }),
            )
    }
}

#[derive(Clone, Copy, Default)]
pub struct Health {
    pub capital: i64,
    pub screens: i64,
    pub sunk_fraction: f32,
}

#[derive(Clone, Copy)]
pub(super) struct Selection<'a> {
    pub battle: &'a Battle,
    pub first: u64,
    pub root: ModelInstance,
}

struct Template {
    root: Part,
    rig: Arc<Rig>,
    player: Player,
    palette: Vec<PaletteJoint>,
    clock: Clock,
}

impl Template {
    fn new(battle: &Battle, instance: ModelInstance, epoch: &mut u32, steady: bool) -> Self {
        let root = Part::new(
            Target {
                instance,
                ..battle.root
            },
            epoch,
            steady,
        );
        let clock = actor_clock(&root, "idle", true);
        Self {
            root,
            player: Player::new(&battle.rig),
            rig: Arc::clone(&battle.rig),
            palette: (*battle.palette).clone(),
            clock,
        }
    }

    fn root_instance(&self) -> ModelInstance {
        ModelInstance {
            animation_epoch: self.root.epoch,
            animation_identity: self.root.identity,
            ..self.root.target.instance
        }
    }

    fn sample(&mut self) {
        let instance = self.root_instance();
        self.player.evaluate_clock(
            &self.rig,
            instance,
            &self.clock,
            &mut self.palette,
            &mut Vec::new(),
        );
    }

    fn attachments(&mut self, names: &[Option<String>; 8]) -> Result<[Option<Matrix>; 8], String> {
        let instance = self.root_instance();
        if let Some(phase) = &mut self.clock.phase {
            phase.instance = instance;
        }
        self.player.advance_hidden(&self.rig, &self.clock);
        let actors = mesh_pose::actors(&self.rig, Some(&self.player), false);
        let mut placements = [None; 8];
        for (index, name) in names.iter().enumerate() {
            let Some(name) = name else { continue };
            placements[index] = impacts::locator_matrix(
                &self.rig,
                &actors,
                instance,
                name,
                mesh_pose::LookupMode::Attachment,
            )?;
            if placements[index].is_none() {
                return Err(format!("combat attachment {name:?} is unavailable"));
            }
        }
        Ok(placements)
    }
}

struct Entity {
    active: bool,
    stamp: i32,
    template: Template,
    parts: [Option<Hull>; 8],
    status: [u8; 8],
    locators: [Option<String>; 8],
    random: Random,
    effects: [Vec<Option<(usize, u32)>>; 4],
    death_time: f32,
}

struct Hull {
    part: Part,
    tracker: impacts::Tracker,
    clock: Clock,
}

struct Death {
    template: Template,
    hull: Hull,
    locators: [Option<String>; 8],
    birth: f64,
    death_time: f32,
}

struct Effect {
    part: Part,
    clock: Clock,
    birth: f64,
    death_time: f32,
}

#[derive(Default)]
pub(super) struct History {
    lifetime_time: f64,
    entities: BTreeMap<u64, Entity>,
    provinces: BTreeMap<u32, u64>,
    deaths: Vec<Death>,
    effects: Vec<Effect>,
    refusals: BTreeMap<(u32, String), usize>,
}

impl History {
    pub(super) fn begin_frame(&mut self, delta: f32) {
        self.lifetime_time += f64::from(delta);
        self.refusals.clear();
    }

    pub(super) fn advance(&mut self, delta: f32) {
        self.deaths
            .retain(|death| !impacts::expired(self.lifetime_time, death.birth, death.death_time));
        self.effects.retain(|effect| {
            !impacts::expired(self.lifetime_time, effect.birth, effect.death_time)
        });
        for entity in self.entities.values_mut() {
            if entity.active {
                entity.template.clock.time += delta;
            }
            let root_instance = entity.template.root_instance();
            if let Some(phase) = &mut entity.template.clock.phase {
                phase.time += delta;
                phase.instance = root_instance;
            }
            if entity.active {
                entity.template.sample();
            } else {
                entity
                    .template
                    .player
                    .advance_hidden(&entity.template.rig, &entity.template.clock);
            }
            for hull in entity.parts.iter_mut().flatten() {
                if entity.active {
                    hull.clock.time += delta;
                }
                if let Some(phase) = &mut hull.clock.phase {
                    phase.time += delta;
                    phase.instance = actor_instance(&hull.part);
                }
                if entity.active {
                    hull.tracker.sample(actor_instance(&hull.part), &hull.clock);
                } else {
                    hull.tracker.advance_hidden(&hull.clock);
                }
            }
        }
        for death in &mut self.deaths {
            death.template.clock.time += delta;
            death.hull.clock.time += delta;
            let instance = death.template.root_instance();
            if let Some(phase) = &mut death.template.clock.phase {
                phase.time += delta;
                phase.instance = instance;
            }
            if let Some(phase) = &mut death.hull.clock.phase {
                phase.time += delta;
                phase.instance = actor_instance(&death.hull.part);
            }
            death.template.sample();
            death
                .hull
                .tracker
                .sample(actor_instance(&death.hull.part), &death.hull.clock);
        }
        for effect in &mut self.effects {
            effect.clock.time += delta;
        }
    }

    pub(super) fn reset(&mut self, provinces: Option<&BTreeSet<u32>>) {
        match provinces {
            Some(provinces) => {
                for &province in provinces {
                    self.release(province);
                }
            }
            None => {
                self.entities.clear();
                self.provinces.clear();
                self.deaths.clear();
                self.effects.clear();
            }
        }
        self.refusals.clear();
    }

    pub(super) fn holds(&self, province: u32) -> bool {
        !self.provinces.is_empty() && self.provinces.contains_key(&province)
    }

    pub(super) fn leave(&mut self, province: u32, land_was_shown: bool, steady: bool) {
        if land_was_shown || steady {
            self.release(province);
        }
    }

    fn release(&mut self, province: u32) {
        if let Some(id) = self.provinces.remove(&province)
            && let Some(entity) = self.entities.get_mut(&id)
        {
            entity.active = false;
        }
    }

    pub(super) fn visit(
        &mut self,
        province: u32,
        selection: Option<Selection<'_>>,
        frame: Frame,
        land_was_shown: bool,
        epoch: &mut u32,
        budget: &mut Budget,
    ) {
        if !frame.shown[super::Kind::Land as usize] {
            self.leave(province, land_was_shown, frame.steady);
            return;
        }
        let Some(selection) = selection else {
            self.release(province);
            return;
        };
        let battle = selection.battle;
        let fresh = self.provinces.get(&province).copied() != Some(selection.first);
        if fresh {
            self.release(province);
            let entity = self.entities.entry(battle.id).or_insert_with(|| Entity {
                active: false,
                stamp: -1,
                template: Template::new(battle, selection.root, epoch, frame.steady),
                parts: std::array::from_fn(|_| None),
                status: [0; 8],
                locators: battle.locators.clone(),
                random: Random::new(battle.id as u32 ^ (battle.id >> 32) as u32),
                effects: battle.effects.clone(),
                death_time: battle.death_time,
            });
            entity.active = true;
            entity.template.root.target.instance = selection.root;
            entity.template.root.target.province = province;
            self.provinces.insert(province, battle.id);
            if !frame.steady || selection.first != battle.id {
                return;
            }
        }
        let entity = self.entities.get_mut(&battle.id).unwrap();
        if entity.stamp != battle.stamp {
            if entity.stamp < 0 || battle.stamp.wrapping_sub(entity.stamp) > 48 {
                entity.parts.fill_with(|| None);
            }
            for index in 0..8 {
                if entity.status[index] == 0 && battle.offered[index] {
                    if let Some(target) = battle.hulls[index]
                        && budget.take()
                        && let Some(info) = &battle.impacts[index]
                    {
                        let part = Part::new(target, epoch, frame.steady);
                        let clock =
                            actor_clock(&part, if index < 4 { "defend" } else { "attack" }, false);
                        entity.parts[index] = Some(Hull {
                            part,
                            tracker: impacts::Tracker::new(Arc::clone(info)),
                            clock,
                        });
                    }
                    entity.status[index] = 1;
                }
                let health = battle.health[index / 4];
                let slot = index % 4;
                let dying = if slot == 0 {
                    health.capital == 0
                } else {
                    matches!(
                        health
                            .sunk_fraction
                            .partial_cmp(&[0.95_f32, 0.55, 0.35][slot - 1]),
                        None | Some(std::cmp::Ordering::Greater)
                    )
                };
                if dying && matches!(entity.status[index], 1 | 2) {
                    if let Some(mut hull) = entity.parts[index].take() {
                        hull.part.target.instance.animation =
                            entity_animation::named_request("death");
                        Arc::make_mut(&mut hull.clock.requests).push(Request {
                            time: hull.clock.time,
                            state: "death".to_owned(),
                            phase_time: hull.clock.phase.map(|phase| phase.time),
                        });
                        let mut template = Template::new(
                            battle,
                            entity.template.root.target.instance,
                            epoch,
                            frame.steady,
                        );
                        template.root.target = entity.template.root.target;
                        self.deaths.push(Death {
                            template,
                            hull,
                            locators: entity.locators.clone(),
                            birth: self.lifetime_time,
                            death_time: battle.death_time,
                        });
                        entity.status[index] = 3;
                    }
                } else if entity.status[index] == 1 {
                    let damaged = if slot == 0 {
                        health.capital < 50_000
                    } else {
                        health.screens < [25_000, 50_000, 75_000][slot - 1]
                    };
                    if damaged && let Some(hull) = &mut entity.parts[index] {
                        let state = if index < 4 {
                            "damaged_defend"
                        } else {
                            "damaged_attack"
                        };
                        hull.part.target.instance.animation =
                            entity_animation::named_request(state);
                        Arc::make_mut(&mut hull.clock.requests).push(Request {
                            time: hull.clock.time,
                            state: state.to_owned(),
                            phase_time: hull.clock.phase.map(|phase| phase.time),
                        });
                        entity.status[index] = 2;
                    }
                }
            }
            entity.stamp = battle.stamp;
        }
        Self::probes(
            entity,
            province,
            epoch,
            frame.steady,
            self.lifetime_time,
            &mut self.effects,
            &mut self.refusals,
        );
    }

    fn probes(
        entity: &mut Entity,
        province: u32,
        epoch: &mut u32,
        steady: bool,
        now: f64,
        effects: &mut Vec<Effect>,
        refusals: &mut BTreeMap<(u32, String), usize>,
    ) {
        match entity.template.attachments(&entity.locators) {
            Ok(matrices) => {
                let mut instances = [None; 8];
                let mut crossings = [0; 8];
                for (index, hull) in entity.parts.iter_mut().enumerate() {
                    let Some(hull) = hull else { continue };
                    let Some(matrix) = matrices[index] else {
                        continue;
                    };
                    let instance = placed_part(&hull.part, matrix);
                    instances[index] = Some(instance);
                    match hull.tracker.probe(&hull.clock) {
                        Ok(count) => crossings[index] = count,
                        Err(reason) => {
                            *refusals.entry((province, reason)).or_default() += 1;
                        }
                    }
                }
                for (index, count) in crossings.into_iter().enumerate() {
                    if !matches!(entity.status[index], 1 | 2) {
                        continue;
                    }
                    let start = if index < 4 { 4 } else { 0 };
                    let targets: Vec<_> = (start..start + 4)
                        .filter_map(|target| {
                            Some(impacts::TargetView {
                                status: entity.status[target],
                                instance: instances[target]?,
                                tracker: &entity.parts[target].as_ref()?.tracker,
                            })
                        })
                        .collect();
                    for _ in 0..count {
                        match impacts::choose(
                            &targets,
                            &entity.effects,
                            entity.template.root.target.instance.scale,
                            &mut || entity.random.next_word(),
                        ) {
                            Ok(Some((mesh, mut instance))) => {
                                instance.animation_seed = entity.random.next_word();
                                effects.push(Effect {
                                    part: Part::new(
                                        Target {
                                            mesh,
                                            instance,
                                            ..entity.template.root.target
                                        },
                                        epoch,
                                        steady,
                                    ),
                                    clock: Clock {
                                        time: 0.0,
                                        phase: None,
                                        requests: Arc::new(Vec::new()),
                                    },
                                    birth: now + 10.0,
                                    death_time: entity.death_time,
                                });
                            }
                            Ok(None) => {}
                            Err(reason) => {
                                *refusals.entry((province, reason)).or_default() += 1;
                            }
                        }
                    }
                }
            }
            Err(reason) => {
                *refusals.entry((province, reason)).or_default() +=
                    entity.parts.iter().flatten().count();
            }
        }
    }

    pub(super) fn placed(&mut self) -> Vec<(usize, ModelInstance)> {
        let mut placed = Vec::new();
        for entity in self.entities.values_mut().filter(|entity| entity.active) {
            let province = entity.template.root.target.province;
            placed.push((
                entity.template.root.target.mesh,
                entity.template.root_instance(),
            ));
            match entity.template.attachments(&entity.locators) {
                Ok(matrices) => {
                    for (index, hull) in entity.parts.iter().enumerate() {
                        if let (Some(hull), Some(matrix)) = (hull, matrices[index]) {
                            placed.push((hull.part.target.mesh, placed_part(&hull.part, matrix)));
                        }
                    }
                }
                Err(reason) => {
                    *self.refusals.entry((province, reason)).or_default() +=
                        entity.parts.iter().flatten().count();
                }
            }
        }
        for death in &mut self.deaths {
            let province = death.template.root.target.province;
            match death.template.attachments(&death.locators) {
                Ok(matrices) => {
                    if let Some(matrix) = matrices[death.hull.part.target.part as usize] {
                        placed.push((
                            death.template.root.target.mesh,
                            death.template.root_instance(),
                        ));
                        placed.push((
                            death.hull.part.target.mesh,
                            placed_part(&death.hull.part, matrix),
                        ));
                    }
                }
                Err(reason) => {
                    *self.refusals.entry((province, reason)).or_default() += 1;
                }
            }
        }
        placed.extend(self.effects.iter().map(|effect| {
            (
                effect.part.target.mesh,
                ModelInstance {
                    animation_epoch: effect.part.epoch,
                    animation_identity: effect.part.identity,
                    ..effect.part.target.instance
                },
            )
        }));
        placed
    }

    pub(super) fn clocks(&self) -> BTreeMap<(usize, u32, u32), Clock> {
        let mut clocks = BTreeMap::new();
        let mut insert = |part: &Part, clock: &Clock| {
            let instance = ModelInstance {
                animation_identity: part.identity,
                animation_epoch: part.epoch,
                ..part.target.instance
            };
            let (seed, identity) = instance.animation_key();
            let mut clock = clock.clone();
            if let Some(phase) = &mut clock.phase {
                phase.instance = instance;
            }
            clocks.insert((part.target.mesh, seed, identity), clock);
        };
        for entity in self.entities.values() {
            insert(&entity.template.root, &entity.template.clock);
            for hull in entity.parts.iter().flatten() {
                insert(&hull.part, &hull.clock);
            }
        }
        for death in &self.deaths {
            insert(&death.template.root, &death.template.clock);
            insert(&death.hull.part, &death.hull.clock);
        }
        for effect in &self.effects {
            insert(&effect.part, &effect.clock);
        }
        clocks
    }

    pub(super) fn warnings(&self) -> Vec<String> {
        self.refusals.iter().map(|((province, reason), count)|
            format!("Naval combat models: {count} attachments withheld in province {province}: {reason}.")
        ).collect()
    }
}

fn actor_instance(part: &Part) -> ModelInstance {
    ModelInstance {
        animation_epoch: part.epoch,
        animation_identity: part.identity,
        ..part.target.instance
    }
}

fn actor_clock(part: &Part, state: &str, root_advances_hidden: bool) -> Clock {
    Clock {
        time: 0.0,
        phase: Some(Phase {
            time: 0.0,
            instance: actor_instance(part),
            root_advances_hidden,
        }),
        requests: Arc::new(vec![Request {
            time: 0.0,
            state: state.to_owned(),
            phase_time: Some(0.0),
        }]),
    }
}

fn placed_part(part: &Part, matrix: Matrix) -> ModelInstance {
    ModelInstance {
        position: matrix[3][..3].try_into().unwrap(),
        scale: 1.0,
        native_flags: part.target.instance.native_flags | 4,
        native_basis: [
            matrix[0][..3].try_into().unwrap(),
            matrix[1][..3].try_into().unwrap(),
            matrix[2][..3].try_into().unwrap(),
        ],
        animation_epoch: part.epoch,
        animation_identity: part.identity,
        ..part.target.instance
    }
}
