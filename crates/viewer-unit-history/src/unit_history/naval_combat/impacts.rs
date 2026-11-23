
use std::sync::Arc;

use content_reader_mesh::mesh::{NavalHitMissEffect, Transform};
use viewer_models::{
    entity_animation::{self, Clock, Event, Player, Rig},
    mesh_pose::{self, Actor, Binding, LookupMode, Matrix},
    ModelInstance,
    PaletteJoint,
};

pub struct Info {
    pub rig: Arc<Rig>,
    pub palette: Arc<Vec<PaletteJoint>>,
    timings: Result<Vec<NavalHitMissEffect>, String>,
    locators: [Result<Vec<String>, String>; 2],
}

impl Info {
    pub fn new(rig: Arc<Rig>, palette: Arc<Vec<PaletteJoint>>, normal_state: &str) -> Self {
        let timings = rig.nodes.first().map_or_else(
            || Err("naval impact hull has no root actor".to_owned()),
            |root| {
                root.clips
                    .iter()
                    .find(|clip| state_hash(&clip.state.name) == state_hash(normal_state))
                    .map_or_else(
                        || Ok(Vec::new()),
                        |clip| match &clip.state.naval_hitmiss_error {
                            Some(reason) => Err(reason.clone()),
                            None => Ok(clip.state.naval_hitmiss_effects.clone()),
                        },
                    )
            },
        );
        let locators = ["hit_effect_locator", "miss_effect_locator"]
            .map(|prefix| contiguous_locators(&rig, prefix));
        Self {
            rig,
            palette,
            timings,
            locators,
        }
    }
}

pub struct Tracker {
    info: Arc<Info>,
    player: Player,
    palette: Vec<PaletteJoint>,
    actors: Arc<[Actor]>,
    events: Vec<Event>,
    previous: f64,
}

impl Tracker {
    pub fn new(info: Arc<Info>) -> Self {
        Self {
            player: Player::new(&info.rig),
            palette: (*info.palette).clone(),
            info,
            actors: Arc::from([]),
            events: Vec::new(),
            previous: 0.0,
        }
    }

    pub fn advance_hidden(&mut self, clock: &Clock) {
        self.player.advance_hidden(&self.info.rig, clock);
    }

    #[cfg(test)]
    pub fn phase_advances_hidden(&self) -> bool {
        self.player.phase_advances_hidden(&self.info.rig)
    }

    #[cfg(test)]
    pub(super) fn root_entry(&self) -> Option<(usize, f32, u32, u64)> {
        self.player.root_entry()
    }

    pub fn sample(&mut self, instance: ModelInstance, clock: &Clock) {
        self.events.clear();
        self.player.evaluate_clock(
            &self.info.rig,
            instance,
            clock,
            &mut self.palette,
            &mut self.events,
        );
        self.actors = mesh_pose::actors(&self.info.rig, Some(&self.player), false);
    }

    pub fn probe(&mut self, clock: &Clock) -> Result<usize, String> {
        self.advance_hidden(clock);
        self.actors = mesh_pose::actors(&self.info.rig, Some(&self.player), false);
        let current = self.player.root_clip_phase(&self.info.rig);
        let previous = std::mem::replace(&mut self.previous, current);
        self.info
            .timings
            .as_ref()
            .map(|timings| crossings(timings, previous, current))
            .map_err(Clone::clone)
    }

    pub fn locator(&self, instance: ModelInstance, name: &str) -> Result<Matrix, String> {
        locator_matrix(
            &self.info.rig,
            &self.actors,
            instance,
            name,
            LookupMode::Naval,
        )?
        .ok_or_else(|| format!("naval impact locator {name:?} is unavailable"))
    }
}

pub struct TargetView<'a> {
    pub status: u8,
    pub instance: ModelInstance,
    pub tracker: &'a Tracker,
}

pub fn choose(
    targets: &[TargetView<'_>],
    effects: &[Vec<Option<(usize, u32)>>; 4],
    root_scale: f32,
    random: &mut impl FnMut() -> u32,
) -> Result<Option<(usize, ModelInstance)>, String> {
    let count = targets.iter().filter(|target| target.status != 3).count();
    let Some(index) = target_index(count, random) else {
        return Ok(None);
    };
    let target = targets
        .iter()
        .filter(|target| target.status != 3)
        .nth(index)
        .unwrap();
    let sample = (random() & 0x7fff_ffff) as f32 / 2_147_483_648.0;
    let branch = usize::from(sample < 0.5);
    let locators = target.tracker.info.locators[branch]
        .as_ref()
        .map_err(Clone::clone)?;
    if locators.is_empty() {
        return Ok(None);
    }
    let locator = &locators[(random() & 0x7fff_ffff) as usize % locators.len()];
    let effects = &effects[branch * 2];
    if effects.is_empty() {
        return Err(format!(
            "naval impact {}-big effect list is empty",
            if branch == 0 { "hit" } else { "miss" }
        ));
    }
    let effect = (random() & 0x7fff_ffff) as usize % effects.len();
    let (mesh, animation) = effects[effect].ok_or_else(|| {
        format!(
            "naval impact {}-big effect entry {effect} is unavailable",
            if branch == 0 { "hit" } else { "miss" }
        )
    })?;
    let matrix = target.tracker.locator(target.instance, locator)?;
    Ok(Some((mesh, effect_instance(matrix, root_scale, animation))))
}

pub fn expired(now: f64, birth: f64, death_time: f32) -> bool {
    f64::from(death_time) < now - birth
}

fn crossings(timings: &[NavalHitMissEffect], previous: f64, current: f64) -> usize {
    timings
        .iter()
        .filter(|event| previous < f64::from(event.time) && f64::from(event.time) <= current)
        .count()
}

fn state_hash(name: &str) -> u32 {
    name.bytes()
        .take_while(|byte| *byte != 0)
        .fold(0x811c_9dc5, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        })
}

fn target_index(count: usize, random: &mut impl FnMut() -> u32) -> Option<usize> {
    (count != 0).then(|| {
        ((random() & 0x7fff_ffff) as usize % (count + 2)).min(count - 1)
    })
}

fn contiguous_locators(rig: &Rig, prefix: &str) -> Result<Vec<String>, String> {
    let source = rig
        .nodes
        .first()
        .and_then(|root| root.native.as_ref())
        .ok_or("naval impact hull has no native root source")?;
    let mut names = Vec::new();
    for number in 1.. {
        let name = format!("{prefix}{number}");
        if mesh_pose::lookup(source, &name, LookupMode::Naval).is_none() {
            break;
        }
        names.push(name);
    }
    Ok(names)
}

pub(super) fn locator_matrix(
    rig: &Rig,
    actors: &[Actor],
    instance: ModelInstance,
    name: &str,
    mode: LookupMode,
) -> Result<Option<Matrix>, String> {
    let source = rig
        .nodes
        .first()
        .and_then(|root| root.native.as_ref())
        .ok_or("naval combat actor has no native root source")?;
    let Some(binding) = mesh_pose::lookup(source, name, mode) else {
        return Ok(None);
    };
    let actor = actors
        .first()
        .ok_or("naval combat actor has no evaluated native actor")?;
    let local = match binding {
        Binding::Locator(locator) if locator.parent.is_none() => binding.local(Ok(None))?,
        _ => binding.local(
            actor
                .pose
                .as_ref()
                .map(|pose| Some(pose.joints.as_slice()))
                .map_err(Clone::clone),
        )?,
    };
    Ok(Some(mesh_pose::multiply(
        matrix(entity_animation::world_transform(instance)),
        mesh_pose::multiply(*actor.world.as_ref().map_err(Clone::clone)?, local),
    )))
}

fn matrix(transform: Transform) -> Matrix {
    [
        [
            transform.basis[0][0],
            transform.basis[0][1],
            transform.basis[0][2],
            0.0,
        ],
        [
            transform.basis[1][0],
            transform.basis[1][1],
            transform.basis[1][2],
            0.0,
        ],
        [
            transform.basis[2][0],
            transform.basis[2][1],
            transform.basis[2][2],
            0.0,
        ],
        [
            transform.translation[0],
            transform.translation[1],
            transform.translation[2],
            1.0,
        ],
    ]
}

fn effect_instance(matrix: Matrix, root_scale: f32, animation: u32) -> ModelInstance {
    ModelInstance {
        position: matrix[3][..3].try_into().unwrap(),
        scale: 1.0,
        native_flags: 4,
        native_basis: std::array::from_fn(|axis| {
            std::array::from_fn(|component| matrix[axis][component] * root_scale)
        }),
        animation,
        ..ModelInstance::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_mesh::mesh::{EntityState, Mesh};
    use std::collections::BTreeMap;
    use viewer_models::entity_animation::{Clip, Node};

    fn rig(clips: Vec<Clip>, locators: BTreeMap<String, (Transform, Option<String>)>) -> Arc<Rig> {
        let entity_locators = locators
            .iter()
            .map(|(name, (transform, _))| mesh_pose::Locator {
                name: name.clone(),
                local: matrix(*transform),
                parent: None,
                error: None,
            })
            .collect();
        Arc::new(Rig {
            nodes: vec![Node {
                mesh: Arc::new(Mesh::default()),
                native: Some(Arc::new(mesh_pose::Source {
                    pdxmesh: false,
                    skeleton: Vec::new(),
                    entity_locators,
                    binary_locators: Vec::new(),
                    mesh_scale: 1.0,
                    entity_scale: 1.0,
                    frozen: None,
                    game_data: None,
                })),
                clips,
                default: "idle".to_owned(),
                follows_parent: false,
                parent: None,
                scale: 1.0,
                mesh_scale: 1.0,
                skin_base: None,
                rigid: 0,
                locators,
            }],
            start: 0,
            end: 1,
        })
    }

    #[test]
    fn normal_timings_come_from_first_state_hash_not_the_chosen_variant() {
        let clip = |name: &str, time| Clip {
            state: EntityState {
                name: name.to_owned(),
                naval_hitmiss_effects: vec![NavalHitMissEffect { time, kind: 0 }],
                ..Default::default()
            },
            animation: None,
        };
        let info = Arc::new(Info::new(
            rig(
                vec![
                    clip("attack", 1.0),
                    clip("attack", 2.0),
                    clip("damaged_attack", 3.0),
                ],
                BTreeMap::new(),
            ),
            Arc::new(vec![PaletteJoint::IDENTITY]),
            "attack",
        ));
        let tracker = Tracker::new(Arc::clone(&info));
        assert_eq!(tracker.info.timings.as_ref().unwrap()[0].time, 1.0);
        assert_eq!(state_hash("attack\0ignored"), state_hash("attack"));
    }

    #[test]
    fn locator_scan_stops_at_first_hole_and_empty_hit_has_no_miss_fallback() {
        let info = Arc::new(Info::new(
            rig(
                Vec::new(),
                ["miss_effect_locator1", "miss_effect_locator3"]
                    .map(|name| (name.to_owned(), (Transform::IDENTITY, None)))
                    .into_iter()
                    .collect(),
            ),
            Arc::new(vec![PaletteJoint::IDENTITY]),
            "defend",
        ));
        assert!(info.locators[0].as_ref().unwrap().is_empty());
        assert_eq!(
            info.locators[1].as_ref().unwrap(),
            &["miss_effect_locator1"]
        );
        let tracker = Tracker::new(info);
        let mut draws = [0, 0x4000_0000].into_iter();
        let effects = [
            vec![Some((11, 0))],
            Vec::new(),
            vec![Some((22, 0))],
            Vec::new(),
        ];
        let result = choose(
            &[TargetView {
                status: 1,
                instance: ModelInstance::default(),
                tracker: &tracker,
            }],
            &effects,
            1.0,
            &mut || {
                draws
                    .next()
                    .expect("missing hit must stop after branch draw")
            },
        )
        .unwrap();
        assert!(result.is_none());
        assert!(draws.next().is_none());
    }

    #[test]
    fn copied_timings_keep_order_duplicates_and_skip_phase_wrap() {
        let times =
            [0.0, 2.0, 1.0, 2.0, -1.0, f32::NAN].map(|time| NavalHitMissEffect { time, kind: 1 });
        assert_eq!(crossings(&times, 0.0, 2.0), 3);
        assert_eq!(crossings(&times, 1.0, 2.0), 2);
        assert_eq!(crossings(&times, 2.0, 0.5), 0);
        assert_eq!(crossings(&times, 0.5, 1.0), 1);
        assert_eq!(crossings(&times, 1.0, 1.0), 0);
    }

    #[test]
    fn last_surviving_target_has_three_tickets_and_empty_has_no_draw() {
        let choices: Vec<_> = (0..6)
            .map(|draw| target_index(4, &mut || draw).unwrap())
            .collect();
        assert_eq!(choices, [0, 1, 2, 3, 3, 3]);
        assert_eq!(
            target_index(0, &mut || panic!("empty side must not draw")),
            None
        );
        assert_eq!(target_index(1, &mut || 2), Some(0));
    }

    #[test]
    fn current_raw_locator_uses_first_or_last_mode_and_never_hides_pose_failure() {
        let mut rig = rig(Vec::new(), BTreeMap::new());
        let source = Arc::make_mut(Arc::make_mut(&mut rig).nodes[0].native.as_mut().unwrap());
        let mut first = mesh_pose::IDENTITY;
        first[3][0] = 4.0;
        let mut last = first;
        last[3][0] = 9.0;
        source.entity_locators = [first, last]
            .map(|local| mesh_pose::Locator {
                name: "hit_effect_locator1".to_owned(),
                local,
                parent: None,
                error: None,
            })
            .into_iter()
            .collect();
        source.mesh_scale = 7.0;
        let actors = [Actor {
            world: Ok(mesh_pose::scale(2.0)),
            pose: Err("native palette uninitialized".to_owned()),
        }];
        let instance = ModelInstance {
            position: [10.0, 0.0, 0.0],
            scale: 1.0,
            ..Default::default()
        };
        let naval = locator_matrix(
            &rig,
            &actors,
            instance,
            "hit_effect_locator1",
            LookupMode::Naval,
        )
        .unwrap()
        .unwrap();
        let attached = locator_matrix(
            &rig,
            &actors,
            instance,
            "hit_effect_locator1",
            LookupMode::Attachment,
        )
        .unwrap()
        .unwrap();
        assert_eq!(naval[3][0], 18.0);
        assert_eq!(attached[3][0], 28.0);
        let source = Arc::make_mut(Arc::make_mut(&mut rig).nodes[0].native.as_mut().unwrap());
        source.entity_locators[0].parent = Some(0);
        assert_eq!(
            locator_matrix(
                &rig,
                &actors,
                instance,
                "hit_effect_locator1",
                LookupMode::Naval
            ),
            Err("native palette uninitialized".to_owned()),
        );
        let actors = [Actor {
            world: Ok(mesh_pose::scale(2.0)),
            pose: Ok(mesh_pose::Pose {
                joints: vec![mesh_pose::scale(3.0)],
                bones: Vec::new(),
            }),
        }];
        let posed = locator_matrix(
            &rig,
            &actors,
            instance,
            "hit_effect_locator1",
            LookupMode::Naval,
        )
        .unwrap()
        .unwrap();
        assert_eq!(posed[3][0], 34.0);
        let source = Arc::make_mut(Arc::make_mut(&mut rig).nodes[0].native.as_mut().unwrap());
        source.entity_locators.clear();
        source.binary_locators = [first, last]
            .map(|local| mesh_pose::Locator {
                name: "hit_effect_locator1".to_owned(),
                local,
                parent: Some(0),
                error: None,
            })
            .into_iter()
            .collect();
        let binary = locator_matrix(
            &rig,
            &actors,
            instance,
            "hit_effect_locator1",
            LookupMode::Naval,
        )
        .unwrap()
        .unwrap();
        assert_eq!(binary[3][0], 34.0);
    }

    #[test]
    fn effect_choice_excludes_dead_actors_and_keeps_registered_default_state() {
        let info = Arc::new(Info::new(
            rig(
                Vec::new(),
                BTreeMap::from([(
                    "hit_effect_locator1".to_owned(),
                    (
                        Transform {
                            translation: [4.0, 0.0, 0.0],
                            ..Transform::IDENTITY
                        },
                        None,
                    ),
                )]),
            ),
            Arc::new(vec![PaletteJoint::IDENTITY]),
            "attack",
        ));
        let mut tracker = Tracker::new(info);
        tracker.actors = Arc::from([Actor {
            world: Ok(mesh_pose::scale(2.0)),
            pose: Ok(mesh_pose::Pose::default()),
        }]);
        let targets = [(3, 999.0), (1, 10.0), (2, 20.0)].map(|(status, x)| TargetView {
            status,
            instance: ModelInstance {
                position: [x, 0.0, 0.0],
                scale: 1.0,
                ..Default::default()
            },
            tracker: &tracker,
        });
        let effects = [
            vec![None, Some((101, 1001))],
            vec![Some((999, 9999))],
            vec![Some((202, 2002))],
            Vec::new(),
        ];
        let mut draws = [2, 0x4000_0000, 0, 1].into_iter();
        let (mesh, effect) = choose(&targets, &effects, 3.0, &mut || draws.next().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(mesh, 101);
        assert_eq!(effect.animation, 1001);
        assert_eq!(effect.position, [28.0, 0.0, 0.0]);
        assert_eq!(
            effect.native_basis,
            [[6.0, 0.0, 0.0], [0.0, 6.0, 0.0], [0.0, 0.0, 6.0]]
        );
        assert!(draws.next().is_none());
        let mut draws = [2, 0x4000_0000, 0, 0].into_iter();
        assert_eq!(
            choose(&targets, &effects, 3.0, &mut || draws.next().unwrap()),
            Err("naval impact hit-big effect entry 0 is unavailable".to_owned()),
        );
        assert!(draws.next().is_none());
    }

    #[test]
    fn effect_scale_preserves_translation_and_strict_retirement() {
        let mut placement = mesh_pose::scale(2.0);
        placement[3] = [12.0, 7.0, -9.0, 1.0];
        let effect = effect_instance(placement, 3.0, 77);
        assert_eq!(effect.position, [12.0, 7.0, -9.0]);
        assert_eq!(effect.animation, 77);
        assert_eq!(
            effect.native_basis,
            [[6.0, 0.0, 0.0], [0.0, 6.0, 0.0], [0.0, 0.0, 6.0]]
        );
        assert!(!expired(20.0, 10.0, 10.0));
        assert!(expired(20.001, 10.0, 10.0));
        assert!(!expired(f64::NAN, 10.0, 10.0));
    }
}
