//! Entity clocks and attachment poses. State selection: 1412796a0; weighted
//! variants/propagation: 14227b5e0; start offset/speed: 1422755a0.
//! Presentation seed is 0x414e494d XOR the stable unit handle. The engine's
//! shared RNG state and state-entry times are not recorded. Reconstructed
//! combat entry phases remain reconstructions. Clocks and state-entry history
//! belong to the viewer, never the recording client's renderer.

use super::models::{ModelInstance, PaletteJoint};
use installed_content::mesh::{self, EntityState, JointPose, Transform};
use std::{collections::BTreeMap, sync::Arc};

pub const SEED: u32 = 0x414e_494d;
pub const IDLE: u32 = 0;
pub const MOVE: u32 = 1;
pub const ATTACK: u32 = 2;
pub const DEFEND: u32 = 3;
pub const RETREAT: u32 = 4;
pub const SUPPORT: u32 = 5;
pub const TRAINING: u32 = 6;
pub const TRAIN: u32 = 7;
/// `141663e70` switches building entities between these authored states. They
/// use their own state-entry clock and authored random starts, without the
/// unit figure's reconstructed phase spread.
pub const BUILDING_IDLE: u32 = 8;
pub const BUILDING_DAMAGED: u32 = 9;
pub fn state_name(state: u32) -> &'static str {
    match state {
        MOVE => "move",
        ATTACK => "attack",
        DEFEND => "defend",
        RETREAT => "retreat",
        SUPPORT => "support_attack",
        TRAINING => "training",
        // 141c503b0 sends idle while the wagon is visible. The train .gfx
        // deliberately maps idle to train_locomotive_1_moving, and move to
        // its stationary clip. Choosing a state by its English name is wrong.
        TRAIN => "idle",
        BUILDING_IDLE => "idle",
        BUILDING_DAMAGED => "damaged",
        _ => "idle",
    }
}

#[derive(Clone)]
pub struct Clip {
    pub state: EntityState,
    pub animation: Option<Arc<mesh::Animation>>,
}
#[derive(Clone)]
pub struct Node {
    pub mesh: Arc<mesh::Mesh>,
    pub clips: Vec<Clip>,
    pub default: String,
    pub follows_parent: bool,
    pub parent: Option<(usize, String, String)>,
    pub scale: f32,
    /// The pdxmesh scale inside `scale`. The engine bakes it into the mesh
    /// and skeleton, so event particles do not carry it.
    pub mesh_scale: f32,
    pub skin_base: Option<u32>,
    pub rigid: u32,
    /// Entity locators in the node's frame, with their `parent_joint`.
    pub locators: BTreeMap<String, (Transform, Option<String>)>,
}

/// A joint's pose under which event particles take the entity scale only:
/// the engine's baked skeleton keeps the scaled joint position but not the
/// pdxmesh scale in its basis (`14226EF30` via `1422621F0`).
pub fn joint_event_local(joint: &Transform, mesh_scale: f32) -> Transform {
    joint.compose(&Transform::from_trs(
        [0.0; 3],
        [0.0, 0.0, 0.0, 1.0],
        1.0 / mesh_scale.max(f32::EPSILON),
    ))
}

/// A locator's node-space transform: under its parent joint's pose when the
/// skeleton has that joint (`14226EF30`, locator x joint world), otherwise in
/// entity space, as the engine keeps a locator whose joint it cannot find.
pub fn locator_local(locator: &Transform, joint: Option<&Transform>) -> Transform {
    joint.map_or(*locator, |joint| joint.compose(locator))
}
#[derive(Clone, Default)]
pub struct Rig {
    pub nodes: Vec<Node>,
    pub start: usize,
    pub end: usize,
}

/// The integer mixer used by 14227b5e0 and 1422755a0, including signed mask.
#[derive(Clone)]
struct Random {
    seed: u32,
    index: u32,
}
impl Random {
    fn next(&mut self) -> u32 {
        let mut x = self.seed.wrapping_add(self.index.wrapping_mul(0xb529_7a4d));
        self.index = self.index.wrapping_add(1);
        x = (x ^ (x >> 8)).wrapping_add(0x68e3_1da4);
        x = (x ^ (x << 8)).wrapping_mul(0x1b56_c4e9);
        x = (x ^ (x >> 8)).wrapping_mul(0x92d6_8ca2);
        (x ^ (x >> 8)) & 0x7fff_ffff
    }
    fn fraction(&mut self) -> f32 {
        self.next() as f32 / 2147483648.0
    }
}

struct Playing {
    requested: String,
    clip: usize,
    start: f32,
    previous: Option<(usize, f32)>,
    random: Random,
    cycle: u64,
}
pub struct Player {
    nodes: Vec<Option<Playing>>,
    last: f32,
    position: Option<[f32; 3]>,
}

#[derive(Clone)]
pub struct Event {
    pub definition: mesh::EntityEvent,
    pub transform: Transform,
    pub age: f32,
    pub seed: u32,
    pub velocity: [f32; 3],
}

fn pick(node: &Node, name: &str, random: &mut Random) -> Option<usize> {
    let total: f32 = node
        .clips
        .iter()
        .filter(|c| c.state.name == name)
        .map(|c| c.state.chance.max(0.0))
        .sum();
    if total <= 0.0 {
        return None;
    }
    let mut draw = random.fraction() * total;
    for (i, clip) in node
        .clips
        .iter()
        .enumerate()
        .filter(|(_, c)| c.state.name == name)
    {
        draw -= clip.state.chance.max(0.0);
        if draw <= 0.0 {
            return Some(i);
        }
    }
    None
}
fn duration(clip: &Clip) -> f32 {
    clip.animation.as_ref().map_or(0.0, |a| a.duration()) / clip.state.speed.max(0.0001)
}
fn sample(clip: &Clip, age: f32) -> Vec<JointPose> {
    clip.animation.as_ref().map_or_else(Vec::new, |a| {
        a.sample(
            age * clip.state.speed + clip.state.start,
            clip.state.looping,
        )
    })
}

impl Player {
    pub fn new(rig: &Rig) -> Self {
        Self {
            nodes: (0..rig.nodes.len()).map(|_| None).collect(),
            last: -1.0,
            position: None,
        }
    }
    /// Work is bounded by the current rig and elapsed state boundaries. The
    /// caller discards players for despawned instances, never retaining replay history.
    pub fn evaluate(
        &mut self,
        rig: &Rig,
        instance: ModelInstance,
        time: f32,
        palette: &mut [PaletteJoint],
        events: &mut Vec<Event>,
    ) {
        if time < self.last {
            self.nodes.iter_mut().for_each(|n| *n = None);
        }
        let velocity = self
            .position
            .filter(|_| time > self.last)
            .map_or([0.0; 3], |p| {
                std::array::from_fn(|i| (instance.position[i] - p[i]) / (time - self.last))
            });
        self.position = Some(instance.position);
        self.last = time;
        let mut worlds: Vec<Vec<Transform>> = Vec::with_capacity(rig.nodes.len());
        let mut placements = Vec::<Transform>::with_capacity(rig.nodes.len());
        let mut states: Vec<Option<(usize, f32, u64)>> = Vec::with_capacity(rig.nodes.len());
        for (index, node) in rig.nodes.iter().enumerate() {
            let parent_state = node.parent.as_ref().and_then(|(parent, _, handle)| {
                let (clip, age, cycle) = states[*parent]?;
                let state = &rig.nodes[*parent].clips[clip].state;
                let name = state
                    .propagate
                    .iter()
                    .find(|(h, _)| h == handle)
                    .or_else(|| state.propagate.iter().find(|(h, _)| h == "all"))
                    .map_or(state.name.as_str(), |(_, name)| name.as_str());
                Some((name.to_owned(), age, cycle))
            });
            let requested = if index == 0 {
                state_name(instance.animation).to_owned()
            } else if node.follows_parent {
                parent_state
                    .as_ref()
                    .map_or_else(|| node.default.clone(), |p| p.0.clone())
            } else {
                node.default.clone()
            };
            let chosen = if node.clips.iter().any(|c| c.state.name == requested) {
                requested
            } else {
                node.default.clone()
            };
            let changed = self.nodes[index]
                .as_ref()
                .is_none_or(|p| p.requested != chosen);
            if changed {
                let seed =
                    SEED ^ instance.animation_seed ^ (index as u32).wrapping_mul(0x9e37_79b9);
                let mut random = self.nodes[index]
                    .as_ref()
                    .map_or(Random { seed, index: 0 }, |p| p.random.clone());
                let previous = self.nodes[index].as_ref().map(|p| (p.clip, time - p.start));
                self.nodes[index] = pick(node, &chosen, &mut random).map(|clip| {
                    // Native idle has NO random start: 14127f000 copies
                    // 1422814f0's defaults and sets only request flag 4, whose
                    // range field stays 0, so 1422755a0 falls back to the
                    // authored `animation_random_start_time`, which no shipped
                    // entity sets. The game still shows neighbouring figures in
                    // different poses, so the spread is their unrecorded
                    // state-entry times. This five-second range is that
                    // stand-in, not a native constant.
                    let random_range = if index == 0 && matches!(instance.animation, IDLE | DEFEND)
                    {
                        5.0
                    } else {
                        node.clips[clip].state.random_start
                    };
                    // Unknown combat entry time: deterministic distribution over its clip.
                    let phase = if previous.is_none()
                        && index == 0
                        && matches!(instance.animation, ATTACK | SUPPORT)
                    {
                        random.fraction() * duration(&node.clips[clip])
                    } else {
                        random.fraction() * random_range
                    };
                    Playing {
                        requested: chosen,
                        clip,
                        start: if previous.is_none()
                            && !matches!(instance.animation, BUILDING_IDLE | BUILDING_DAMAGED)
                        {
                            -phase
                        } else {
                            time - phase
                        },
                        previous,
                        random,
                        cycle: 0,
                    }
                });
            }
            let mut state_info = None;
            let mut pose = Vec::new();
            if let Some(playing) = &mut self.nodes[index] {
                let mut age = time - playing.start;
                if index == 0 && instance.animation == TRAIN {
                    age = instance.animation_phase + time;
                }
                if node.follows_parent
                    && let Some((_, parent_age, cycle)) = &parent_state
                {
                    age = *parent_age;
                    playing.cycle = *cycle;
                } else {
                    // A looping state repeats its clip; nonlooping variants re-enter
                    // next_state (or the requested state) and redraw the authored chance.
                    for _ in 0..128 {
                        let clip = &node.clips[playing.clip];
                        let length = duration(clip);
                        if clip.state.looping || length <= 0.0 || age < length {
                            break;
                        }
                        let next = clip
                            .state
                            .next_state
                            .as_deref()
                            .unwrap_or(&playing.requested);
                        let Some(next_clip) = pick(node, next, &mut playing.random) else {
                            break;
                        };
                        playing.previous = Some((playing.clip, length));
                        playing.clip = next_clip;
                        playing.start += length;
                        playing.cycle += 1;
                        age = time - playing.start;
                    }
                }
                let clip = &node.clips[playing.clip];
                let length = duration(clip);
                let cycle = if clip.state.looping && length > 0.0 {
                    (age / length).floor().max(0.0) as u64
                } else {
                    playing.cycle
                };
                let local_age = if clip.state.looping && length > 0.0 {
                    age.rem_euclid(length)
                } else {
                    age
                };
                let mut locals = sample(clip, age);
                if let Some((old, old_age)) = playing.previous {
                    let blend = clip.state.blend;
                    let t = if blend > 0.0 {
                        (time - playing.start) / blend
                    } else {
                        1.0
                    };
                    if t < 1.0
                        && let (Some(old_animation), Some(animation)) =
                            (&node.clips[old].animation, &clip.animation)
                    {
                        let old_values = sample(&node.clips[old], old_age);
                        let old_by_name: BTreeMap<_, _> = old_animation
                            .joints
                            .iter()
                            .zip(old_values)
                            .map(|(j, v)| (j.name.as_str(), v))
                            .collect();
                        for (joint, value) in animation.joints.iter().zip(&mut locals) {
                            if let Some(old) = old_by_name.get(joint.name.as_str()) {
                                *value = old.blend(*value, t);
                            }
                        }
                    }
                }
                if let Some(animation) = &clip.animation {
                    pose = node.mesh.sampled_pose(animation, &locals);
                }
                state_info = Some((playing.clip, local_age, cycle));
            }
            if pose.is_empty() {
                let count = node
                    .mesh
                    .skeleton
                    .iter()
                    .map(|j| j.index as usize + 1)
                    .max()
                    .unwrap_or(0);
                pose = vec![Transform::IDENTITY; count];
                for joint in &node.mesh.skeleton {
                    pose[joint.index as usize] = node
                        .mesh
                        .bind_pose(&joint.name)
                        .unwrap_or(Transform::IDENTITY);
                }
            }
            let parent = node
                .parent
                .as_ref()
                .map_or(Transform::IDENTITY, |(parent, joint, _)| {
                    let joint = rig.nodes[*parent]
                        .mesh
                        .joint(joint)
                        .and_then(|j| worlds[*parent].get(j.index as usize))
                        .copied()
                        .unwrap_or(Transform::IDENTITY);
                    placements[*parent].compose(&joint)
                });
            let own = parent.compose(&Transform::from_trs(
                [0.0; 3],
                [0.0, 0.0, 0.0, 1.0],
                node.scale,
            ));
            palette[node.rigid as usize - rig.start] = PaletteJoint::from_transform(&own);
            if let Some(base) = node.skin_base {
                for joint in &node.mesh.skeleton {
                    let matrix = joint
                        .inverse_bind
                        .map(|inverse| pose[joint.index as usize].compose(&inverse))
                        .filter(Transform::is_finite)
                        .unwrap_or(Transform::IDENTITY);
                    palette[base as usize + joint.index as usize - rig.start] =
                        PaletteJoint::from_transform(&own.compose(&matrix));
                }
            }
            if let Some((clip, age, _cycle)) = state_info {
                let mut authored = node.clips[clip]
                    .state
                    .events
                    .iter()
                    .map(|event| (event, age))
                    .collect::<Vec<_>>();
                // Before entering the spline's visible interval, 141c503b0
                // sends move (14299069c); its keep_particle exhaust survives
                // the switch to idle (142952aec). Reconstruct that entry from
                // the same seeded departure that places the locomotive.
                if index == 0 && instance.animation == TRAIN {
                    authored.extend(
                        node.clips
                            .iter()
                            .filter(|c| c.state.name == "move")
                            .flat_map(|c| c.state.events.iter())
                            .filter(|e| e.keep_particle)
                            .map(|e| (e, time + instance.animation_phase)),
                    );
                }
                for (event_index, (event, age)) in authored.into_iter().enumerate() {
                    let event_clock = if event.trigger_once {
                        if matches!(instance.animation, BUILDING_IDLE | BUILDING_DAMAGED) {
                            self.nodes[index].as_ref().map_or(age, |p| time - p.start)
                        } else {
                            time + instance.animation_phase
                        }
                    } else {
                        age
                    };
                    if event_clock < event.time {
                        continue;
                    }
                    let local = node
                        .mesh
                        .joint(&event.node)
                        .and_then(|j| pose.get(j.index as usize))
                        .map(|joint| joint_event_local(joint, node.mesh_scale))
                        .or_else(|| {
                            let (locator, joint) = node.locators.get(&event.node)?;
                            let joint = joint
                                .as_deref()
                                .and_then(|name| node.mesh.joint(name))
                                .and_then(|joint| pose.get(joint.index as usize));
                            Some(locator_local(locator, joint))
                        })
                        .or_else(|| {
                            node.mesh
                                .bind_pose(&event.node)
                                .map(|joint| joint_event_local(&joint, node.mesh_scale))
                        })
                        .unwrap_or(Transform::IDENTITY);
                    let transform = own.compose(&local);
                    if transform.uniform_scale() <= 1e-6 {
                        continue;
                    }
                    events.push(Event {
                        definition: event.clone(),
                        transform: world_transform(instance).compose(&transform),
                        age: event_clock - event.time,
                        seed: instance.animation_seed
                            ^ (index as u32).rotate_left(8)
                            ^ event_index as u32,
                        velocity,
                    });
                }
            }
            states.push(state_info);
            placements.push(own);
            worlds.push(pose);
        }
    }
}

pub fn world_transform(instance: ModelInstance) -> Transform {
    let (s, c) = instance.rotation.sin_cos();
    let (sp, cp) = instance.pitch.sin_cos();
    Transform {
        basis: [[c, 0.0, s], [-s * sp, cp, c * sp], [-s * cp, -sp, c * cp]],
        translation: instance.position,
    }
    .compose(&Transform::from_trs(
        [0.0; 3],
        [0.0, 0.0, 0.0, 1.0],
        instance.scale,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn building_rig() -> Rig {
        let clip = |name: &str, events| Clip {
            state: EntityState {
                name: name.into(),
                chance: 1.0,
                speed: 1.0,
                events,
                ..EntityState::default()
            },
            animation: None,
        };
        Rig {
            nodes: vec![Node {
                mesh: Arc::new(mesh::Mesh::default()),
                clips: vec![
                    clip("idle", Vec::new()),
                    clip(
                        "damaged",
                        vec![mesh::EntityEvent {
                            time: 0.1,
                            particle: Some("building_damaged_particle".into()),
                            trigger_once: true,
                            ..mesh::EntityEvent::default()
                        }],
                    ),
                ],
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

    #[test]
    fn a_joint_event_keeps_the_joint_position_but_not_the_mesh_scale() {
        let mesh_scale = 0.14;
        let own = Transform::from_trs([0.0; 3], [0.0, 0.0, 0.0, 1.0], 0.8 * mesh_scale);
        let joint = Transform::from_trs([0.0, 10.0, 0.0], [0.0, 0.0, 0.0, 1.0], 1.0);
        let world = own.compose(&joint_event_local(&joint, mesh_scale));
        let at = world.transform_point([0.0; 3]);
        assert!((at[1] - 10.0 * 0.8 * mesh_scale).abs() < 1e-5, "{at:?}");
        assert!((world.uniform_scale() - 0.8).abs() < 1e-5);
    }

    #[test]
    fn a_locator_follows_its_parent_joint_pose() {
        let mut locator = Transform::IDENTITY;
        locator.translation = [0.0, 1.0, 0.0];
        let joint = Transform::from_trs([5.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], 1.0);
        assert_eq!(
            locator_local(&locator, Some(&joint)).transform_point([0.0; 3]),
            [5.0, 1.0, 0.0]
        );
        assert_eq!(
            locator_local(&locator, None).transform_point([0.0; 3]),
            [0.0, 1.0, 0.0],
            "a joint the skeleton lacks leaves the locator in entity space"
        );
    }

    #[test]
    fn building_damage_event_uses_state_entry_and_repair_stops_it() {
        let rig = building_rig();
        let mut player = Player::new(&rig);
        let mut palette = [PaletteJoint::from_transform(&Transform::IDENTITY)];
        let mut events = Vec::new();
        let mut instance = ModelInstance {
            animation: BUILDING_IDLE,
            animation_seed: 81,
            scale: 1.0,
            ..ModelInstance::default()
        };
        player.evaluate(&rig, instance, 50.0, &mut palette, &mut events);
        assert_eq!(player.nodes[0].as_ref().unwrap().start, 50.0);
        assert!(events.is_empty());

        instance.animation = BUILDING_DAMAGED;
        player.evaluate(&rig, instance, 60.0, &mut palette, &mut events);
        assert!(events.is_empty());
        player.evaluate(&rig, instance, 60.2, &mut palette, &mut events);
        assert_eq!(events.len(), 1);
        assert!((events[0].age - 0.1).abs() < 0.00001);

        events.clear();
        instance.animation = BUILDING_IDLE;
        player.evaluate(&rig, instance, 61.0, &mut palette, &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn newly_created_damaged_building_does_not_use_the_global_elapsed_clock() {
        let rig = building_rig();
        let mut player = Player::new(&rig);
        let mut palette = [PaletteJoint::from_transform(&Transform::IDENTITY)];
        let mut events = Vec::new();
        let instance = ModelInstance {
            animation: BUILDING_DAMAGED,
            scale: 1.0,
            ..ModelInstance::default()
        };
        player.evaluate(&rig, instance, 120.0, &mut palette, &mut events);
        assert!(events.is_empty());
        player.evaluate(&rig, instance, 120.2, &mut palette, &mut events);
        assert_eq!(events.len(), 1);
        assert!((events[0].age - 0.1).abs() < 0.00001);
    }
}
