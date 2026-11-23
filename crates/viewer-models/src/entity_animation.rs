
use crate::{ModelInstance, PaletteJoint};
use content_reader_mesh::mesh::{self, EntityState, JointPose, Transform};
use std::sync::Mutex;
use std::sync::OnceLock;
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
pub const BUILDING_IDLE: u32 = 8;
pub const BUILDING_DAMAGED: u32 = 9;
pub const AMBIENT_IDLE: u32 = 10;
pub const STOPPED: u32 = 11;
pub const DEFAULT_STATE: u32 = 12;
pub const NAMED: u32 = 0x4000_0000;
const DIRECT: u32 = 0x8000_0000;

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub time: f32,
    pub phase_time: Option<f32>,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clock {
    pub time: f32,
    pub phase: Option<Phase>,
    pub requests: Arc<Vec<Request>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phase {
    pub time: f32,
    pub root_advances_hidden: bool,
    pub instance: ModelInstance,
}

pub fn named_stage(state: &str, intro: Option<&str>) -> u32 {
    let mut names = NAMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let index = match names
        .iter()
        .position(|(known, known_intro)| *known == state && *known_intro == intro)
    {
        Some(index) => index,
        None => {
            names.push((
                Box::leak(state.to_owned().into_boxed_str()),
                intro.map(|intro| &*Box::leak(intro.to_owned().into_boxed_str())),
            ));
            names.len() - 1
        }
    };
    NAMED + index as u32
}

pub fn named_request(state: &str) -> u32 {
    named_stage(state, None) | DIRECT
}

type Stage = (&'static str, Option<&'static str>);
static NAMES: OnceLock<Mutex<Vec<Stage>>> = OnceLock::new();

fn named(state: u32) -> Option<Stage> {
    let state = state & !DIRECT;
    let names = NAMES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    names.get(state.checked_sub(NAMED)? as usize).copied()
}

pub fn stage_intro(state: u32) -> Option<&'static str> {
    named(state).and_then(|(_, intro)| intro)
}

pub fn state_name(state: u32) -> &'static str {
    if state >= NAMED {
        return named(state).map_or("idle", |(name, _)| name);
    }
    match state {
        MOVE => "move",
        ATTACK => "attack",
        DEFEND => "defend",
        RETREAT => "retreat",
        SUPPORT => "support_attack",
        TRAINING => "training",
        TRAIN => "idle",
        BUILDING_IDLE => "idle",
        BUILDING_DAMAGED => "damaged",
        _ => "idle",
    }
}

#[derive(Debug, Clone)]
pub struct Clip {
    pub state: EntityState,
    pub animation: Option<Arc<mesh::Animation>>,
}
#[derive(Debug, Clone)]
pub struct Node {
    pub mesh: Arc<mesh::Mesh>,
    pub native: Option<Arc<super::mesh_pose::Source>>,
    pub clips: Vec<Clip>,
    pub default: String,
    pub follows_parent: bool,
    pub parent: Option<(usize, String, String)>,
    pub scale: f32,
    pub mesh_scale: f32,
    pub skin_base: Option<u32>,
    pub rigid: u32,
    pub locators: BTreeMap<String, (Transform, Option<String>)>,
}

pub fn joint_event_local(joint: &Transform, mesh_scale: f32) -> Transform {
    joint.compose(&Transform::from_trs(
        [0.0; 3],
        [0.0, 0.0, 0.0, 1.0],
        1.0 / mesh_scale.max(f32::EPSILON),
    ))
}

pub fn locator_local(locator: &Transform, joint: Option<&Transform>) -> Transform {
    joint.map_or(*locator, |joint| joint.compose(locator))
}
#[derive(Debug, Clone, Default)]
pub struct Rig {
    pub nodes: Vec<Node>,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Default)]
pub struct Random {
    pub seed: u32,
    pub index: u32,
}
impl Random {
    pub fn new(seed: u32) -> Self {
        Self { seed, index: 0 }
    }

    pub fn from_counter(nanoseconds: u32) -> Self {
        let mix = |factor: u32, add: u32, multiply: u32| {
            let value = nanoseconds.wrapping_mul(factor);
            let value = ((value >> 8) ^ value).wrapping_add(add);
            let value = ((value << 8) ^ value).wrapping_mul(multiply);
            (value >> 8) ^ value
        };
        Self {
            seed: mix(0x7eb8_da44, 0x10d8_365e, 0x6d22_f31b),
            index: mix(0x205a_d647, 0xb4f3_c315, 0x387a_9f3a),
        }
    }

    pub fn next_word(&mut self) -> u32 {
        let mut x = self.seed.wrapping_add(self.index.wrapping_mul(0xb529_7a4d));
        self.index = self.index.wrapping_add(1);
        x = (x ^ (x >> 8)).wrapping_add(0x68e3_1da4);
        x = (x ^ (x << 8)).wrapping_mul(0x1b56_c4e9);
        x = (x ^ (x >> 8)).wrapping_mul(0x92d6_8ca2);
        (x ^ (x >> 8)) & 0x7fff_ffff
    }
    fn fraction(&mut self) -> f32 {
        self.next_word() as f32 / 2147483648.0
    }
}

struct Playing {
    requested: String,
    clip: usize,
    start: f32,
    event_time: f32,
    phase_offset: f32,
    previous: Option<(usize, f32)>,
    admitted_kept: BTreeMap<usize, f32>,
    propagate: bool,
    random: Random,
    cycle: u64,
    completed: bool,
}
pub struct Player {
    nodes: Vec<Option<Playing>>,
    native_samples: Vec<Option<NativeSample>>,
    last: f32,
    phase: Option<(f32, f32)>,
    requested: usize,
    direct_state: Option<u32>,
    kept: Vec<Kept>,
    event_refusals: BTreeMap<String, usize>,
    native_choice: bool,
    scratch: EvaluationScratch,
    native_actors: std::cell::RefCell<Option<NativeActors>>,
}

#[derive(Default)]
struct EvaluationScratch {
    worlds: Vec<Vec<Transform>>,
    placements: Vec<Transform>,
    states: Vec<Option<(usize, f32, u64, bool)>>,
    nodes: Vec<NodeScratch>,
    memo: Vec<Option<Option<Transform>>>,
}

#[derive(Default)]
struct NodeScratch {
    bindings: Vec<Option<mesh::PoseBinding>>,
    crossfades: BTreeMap<(usize, usize), Vec<Option<usize>>>,
    locals: Vec<JointPose>,
    previous: Vec<JointPose>,
}

struct NativeActors {
    rig: usize,
    samples: Vec<Option<NativeSample>>,
    actors: Arc<[super::mesh_pose::Actor]>,
}

struct Kept {
    source: usize,
    definition: mesh::EntityEvent,
    start: f32,
    end: f32,
    seed: u32,
    key: EventKey,
    life: KeptLife,
}

#[derive(Clone, Default)]
pub struct KeptLife(Arc<std::sync::atomic::AtomicBool>);

impl KeptLife {
    pub fn finish(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn finished(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct NativeSample {
    clip: usize,
    age: f32,
    time: f32,
    previous: Option<(usize, f32, f32)>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventKey {
    pub pass: u64,
    pub model: crate::AssetKey,
    actor: (u32, u32),
    node: usize,
    state: (usize, u32),
    event: usize,
}

impl EventKey {
    #[cfg(any(test, feature = "test-support"))]
    pub fn without_pass(&self) -> Self {
        Self {
            pass: 0,
            ..self.clone()
        }
    }

    pub fn actor(&self) -> crate::ActorAddress {
        (self.pass, self.model.clone(), self.actor.0, self.actor.1)
    }
}

#[derive(Clone)]
pub struct Event {
    pub definition: mesh::EntityEvent,
    pub transform: Transform,
    pub age: f32,
    pub seed: u32,
    pub key: EventKey,
    pub emission_end: Option<f32>,
    pub hidden: bool,
    pub state: u32,
    pub kept: Option<KeptLife>,
}

impl Event {
    pub fn in_combat(&self) -> bool {
        matches!(self.state & !DIRECT, ATTACK | DEFEND)
    }
}

impl EventKey {
    pub fn player(&self) -> crate::ActorAddress {
        self.actor()
    }
}

fn event_key(
    instance: ModelInstance,
    node: usize,
    event: usize,
    playing: Option<&Playing>,
) -> EventKey {
    EventKey {
        pass: 0,
        model: Default::default(),
        actor: instance.animation_key(),
        node,
        state: playing.map_or((0, 0), |playing| {
            (playing.clip, playing.event_time.to_bits())
        }),
        event,
    }
}

fn event_seed(
    instance: ModelInstance,
    node: usize,
    event: usize,
    playing: Option<&Playing>,
) -> u32 {
    let state = if instance.animation & DIRECT != 0 {
        playing.map_or(0, |playing| {
            (playing.clip as u32).rotate_left(16) ^ playing.event_time.to_bits().rotate_left(3)
        }) ^ instance.animation_epoch.rotate_left(11)
    } else {
        0
    };
    instance.animation_seed ^ (node as u32).rotate_left(8) ^ event as u32 ^ state
}

fn keep_events(
    kept: &mut Vec<Kept>,
    node: &Node,
    index: usize,
    playing: &Playing,
    time: f32,
    instance: ModelInstance,
    phase_time: Option<f32>,
) {
    let clip = &node.clips[playing.clip];
    let elapsed = time - playing.start + playing.phase_offset;
    let length = duration(clip);
    for (ordinal, event) in clip.state.events.iter().enumerate() {
        if !event.keep_particle || event.particle.is_none() {
            continue;
        }
        let clock = if !event.trigger_once && clip.state.looping && length > 0. {
            elapsed.rem_euclid(length)
        } else {
            elapsed
        };
        let event_time = (event.time - clip.state.start) / clip.state.speed;
        let retained = if let Some(phase) = phase_time {
            playing
                .admitted_kept
                .get(&ordinal)
                .map(|origin| (phase - origin, *origin))
        } else {
            (clock >= event_time).then_some((clock - event_time, time - (clock - event_time)))
        };
        if let Some((age, origin)) = retained {
            let mut definition = event.clone();
            definition.time = 0.;
            definition.trigger_once = false;
            definition.sound = None;
            definition.id = None;
            kept.push(Kept {
                source: index,
                definition,
                start: origin,
                end: age,
                seed: event_seed(instance, index, ordinal, Some(playing)),
                key: event_key(instance, index, ordinal, Some(playing)),
                life: KeptLife::default(),
            });
        }
    }
}

fn pick(node: &Node, name: &str, random: &mut Random, native: bool) -> Option<usize> {
    if native {
        let variants = node
            .clips
            .iter()
            .enumerate()
            .filter(|(_, clip)| clip.state.name == name);
        let count = variants.clone().count();
        if count <= 1 {
            return variants.clone().next().map(|(index, _)| index);
        }
        let fraction = f64::from(random.next_word()) / 2_147_483_648.0;
        let total: i64 = variants
            .clone()
            .map(|(_, clip)| i64::from(chance_weight(clip).max(0)))
            .sum();
        if total > 0 {
            let target = (fraction * total as f64) as i64;
            let mut cumulative = 0_i64;
            for (index, clip) in variants.clone() {
                cumulative += i64::from(chance_weight(clip).max(0));
                if target < cumulative {
                    return Some(index);
                }
            }
        }
        return variants
            .clone()
            .nth((fraction * count as f64) as usize)
            .map(|(index, _)| index);
    }
    let total: f32 = node
        .clips
        .iter()
        .filter(|c| c.state.name == name)
        .map(|c| c.state.chance.max(0.0))
        .sum();
    if total <= 0.0 {
        let mut variants = node
            .clips
            .iter()
            .enumerate()
            .filter(|(_, c)| c.state.name == name)
            .map(|(index, _)| index);
        let count = variants.clone().count();
        if count <= 1 {
            return variants.next();
        }
        let fraction = f64::from(random.next_word()) / 2_147_483_648.0;
        return variants.nth((fraction * count as f64) as usize);
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

fn played_animation(node: &Node, clip: usize) -> bool {
    node.clips[clip].animation.is_some() && node.native.as_ref().is_none_or(|source| source.pdxmesh)
}
fn chance_weight(clip: &Clip) -> i32 {
    clip.state
        .chance_weight
        .unwrap_or_else(|| (f64::from(clip.state.chance.max(0.0)) * 100_000.0) as i32)
}
fn clip_phase(clip: &Clip, age: f32) -> f32 {
    let Some(animation) = &clip.animation else {
        return age * clip.state.speed + clip.state.start;
    };
    let length = animation.samples.saturating_sub(1).max(1) as f32 / animation.fps;
    let phase = age * clip.state.speed + clip.state.start;
    if clip.state.looping {
        phase.rem_euclid(length)
    } else {
        phase.clamp(0.0, length)
    }
}

fn advance_sample(node: &Node, mut sample: NativeSample, time: f32) -> NativeSample {
    let delta = time - sample.time;
    sample.age += delta;
    sample.previous = sample.previous.and_then(|(old, age, weight)| {
        let blend = node.clips[sample.clip].state.blend;
        let weight = if blend > 0.0 {
            weight + delta / blend
        } else {
            1.0
        };
        (weight < 1.0).then_some((old, age + delta, weight))
    });
    sample.time = time;
    sample
}
fn sample(clip: &Clip, age: f32, out: &mut Vec<JointPose>) {
    out.clear();
    if let Some(animation) = &clip.animation {
        animation.sample_into(
            age * clip.state.speed + clip.state.start,
            clip.state.looping,
            out,
        );
    }
}

impl Player {
    pub fn new(rig: &Rig) -> Self {
        Self {
            nodes: (0..rig.nodes.len()).map(|_| None).collect(),
            native_samples: vec![None; rig.nodes.len()],
            last: -1.0,
            phase: None,
            requested: 0,
            direct_state: None,
            kept: Vec::new(),
            event_refusals: BTreeMap::new(),
            native_choice: false,
            scratch: EvaluationScratch {
                worlds: vec![Vec::new(); rig.nodes.len()],
                nodes: rig
                    .nodes
                    .iter()
                    .map(|node| NodeScratch {
                        bindings: (0..node.clips.len()).map(|_| None).collect(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
            native_actors: Default::default(),
        }
    }

    pub fn native_choice(rig: &Rig) -> Self {
        Self {
            native_choice: true,
            ..Self::new(rig)
        }
    }

    pub fn evaluate_clock(
        &mut self,
        rig: &Rig,
        instance: ModelInstance,
        clock: &Clock,
        palette: &mut [PaletteJoint],
        events: &mut Vec<Event>,
    ) {
        self.evaluate_clock_inner(rig, instance, clock, palette, events, true);
    }

    pub fn advance_hidden(&mut self, rig: &Rig, clock: &Clock) {
        if let Some(phase) = clock.phase {
            self.evaluate_clock_inner(rig, phase.instance, clock, &mut [], &mut Vec::new(), false);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn phase_advances_hidden(&self, rig: &Rig) -> bool {
        self.native_samples
            .first()
            .copied()
            .flatten()
            .zip(rig.nodes.first())
            .is_some_and(|(sample, root)| played_animation(root, sample.clip))
    }

    pub fn event_refusals(&self) -> &BTreeMap<String, usize> {
        &self.event_refusals
    }

    fn evaluate_clock_inner(
        &mut self,
        rig: &Rig,
        mut instance: ModelInstance,
        clock: &Clock,
        palette: &mut [PaletteJoint],
        events: &mut Vec<Event>,
        admitted: bool,
    ) {
        if clock.time < self.last
            || clock
                .phase
                .zip(self.phase)
                .is_some_and(|(phase, (_, previous))| phase.time < previous)
        {
            *self = Self::new(rig);
        }
        self.event_refusals.clear();
        let Some(root) = rig.nodes.first() else {
            return;
        };
        let initial = self
            .direct_state
            .unwrap_or_else(|| named_stage(&root.default, None) | DIRECT);
        let mut cursor = self.phase.unwrap_or((0.0, 0.0));
        self.phase = clock.phase.map(|_| cursor);
        let native_flags = instance.native_flags;
        instance.native_flags &= !8;
        instance.animation = initial;
        let mut discarded = Vec::new();
        if self.direct_state.is_none() {
            self.evaluate_inner(
                rig,
                instance,
                0.0,
                palette,
                &mut discarded,
                admitted && clock.phase.is_none(),
            );
            self.direct_state = Some(initial);
            discarded.clear();
        }
        for request in &clock.requests[self.requested..] {
            if request.time > clock.time
                || clock
                    .phase
                    .is_some_and(|phase| request.phase_time.is_some_and(|time| time > phase.time))
            {
                break;
            }
            if clock.phase.is_some() {
                let phase_time = request.phase_time.unwrap_or(request.time);
                self.offset_phase(
                    rig,
                    (phase_time - cursor.1) - (request.time - cursor.0),
                    clock.phase.is_some_and(|phase| phase.root_advances_hidden),
                );
                cursor = (request.time, phase_time);
                self.phase = Some(cursor);
            }
            self.evaluate_inner(
                rig,
                instance,
                request.time,
                palette,
                &mut discarded,
                admitted && clock.phase.is_none(),
            );
            let exists = root
                .clips
                .iter()
                .any(|clip| clip.state.name == request.state);
            let current = self.nodes[0]
                .as_ref()
                .map(|p| root.clips[p.clip].state.name.as_str());
            if exists && (clock.phase.is_some() || current != Some(request.state.as_str())) {
                if let Some(playing) = &mut self.nodes[0] {
                    playing.requested.clear();
                }
                instance.animation = named_stage(&request.state, None) | DIRECT;
                instance.native_flags = if self.requested == 0 {
                    native_flags
                } else {
                    native_flags & !8
                };
                self.direct_state = Some(instance.animation);
                self.evaluate_inner(
                    rig,
                    instance,
                    request.time,
                    palette,
                    &mut discarded,
                    admitted && clock.phase.is_none(),
                );
            }
            self.requested += 1;
            discarded.clear();
        }
        if let Some(phase) = clock.phase {
            self.offset_phase(
                rig,
                (phase.time - cursor.1) - (clock.time - cursor.0),
                phase.root_advances_hidden,
            );
            self.phase = Some((clock.time, phase.time));
        }
        instance.native_flags = native_flags & !8;
        self.evaluate_inner(rig, instance, clock.time, palette, events, admitted);
        if admitted && clock.phase.is_some() {
            self.finish_phase(rig, instance, clock.time);
        }
    }

    fn offset_phase(&mut self, rig: &Rig, extra: f32, root_advances: bool) {
        for (index, playing) in self.nodes.iter_mut().enumerate() {
            if let Some(playing) = playing
                && ((index == 0 && root_advances)
                    || played_animation(&rig.nodes[index], playing.clip))
            {
                playing.phase_offset += extra;
            }
        }
    }

    fn finish_phase(&mut self, rig: &Rig, instance: ModelInstance, time: f32) {
        for (index, node) in rig.nodes.iter().enumerate().rev() {
            let Some(playing) = &mut self.nodes[index] else {
                continue;
            };
            let clip = &node.clips[playing.clip];
            let age = time - playing.start + playing.phase_offset;
            let played =
                self.native_samples[index].filter(|sample| played_animation(node, sample.clip));
            let finished = if let Some(sample) = played {
                let actual = &node.clips[sample.clip];
                let length = actual
                    .animation
                    .as_ref()
                    .map_or(0.0, |animation| animation.duration());
                clip_phase(actual, sample.age) >= length
            } else {
                age * clip.state.speed + clip.state.start >= duration(clip) * clip.state.speed
            };
            if clip.state.looping || !finished {
                continue;
            }
            if clip.state.next_state.is_none() && chance_weight(clip) < 1 {
                continue;
            }
            let next = clip.state.next_state.as_deref().unwrap_or(&clip.state.name);
            let Some(next_clip) = pick(node, next, &mut playing.random, true) else {
                continue;
            };
            if instance.animation & DIRECT != 0 {
                keep_events(
                    &mut self.kept,
                    node,
                    index,
                    playing,
                    time,
                    instance,
                    self.phase.map(|(_, phase)| phase),
                );
            }
            playing.previous =
                Some(played.map_or((playing.clip, age), |sample| (sample.clip, sample.age)));
            playing.clip = next_clip;
            playing.propagate = true;
            playing.admitted_kept.clear();
            playing.completed = false;
            playing.start = time;
            playing.event_time = self.phase.map_or(time, |(_, phase)| phase);
            let range = node.clips[next_clip].state.random_start;
            playing.phase_offset = if range > 0.0 {
                playing.random.fraction() * range / node.clips[next_clip].state.speed
            } else {
                0.0
            };
            playing.cycle += 1;
            self.evaluate_inner(rig, instance, time, &mut [], &mut Vec::new(), false);
        }
    }

    pub(super) fn native_actors(&self, rig: &Rig) -> Arc<[super::mesh_pose::Actor]> {
        self.native_actors_held(rig).0
    }

    pub(super) fn sample_bits(&self) -> u64 {
        let mut bits = 0xcbf2_9ce4_8422_2325_u64;
        let mut eat = |value: u64| bits = (bits ^ value).wrapping_mul(0x100_0000_01b3);
        for sample in &self.native_samples {
            match sample {
                None => eat(1),
                Some(sample) => {
                    eat(sample.clip as u64 + 2);
                    eat(u64::from(sample.age.to_bits()));
                    match sample.previous {
                        None => eat(3),
                        Some((clip, age, weight)) => {
                            eat(clip as u64 + 4);
                            eat(u64::from(age.to_bits()));
                            eat(u64::from(weight.to_bits()));
                        }
                    }
                }
            }
        }
        bits
    }

    pub(super) fn native_actors_held(&self, rig: &Rig) -> (Arc<[super::mesh_pose::Actor]>, bool) {
        if let Some(held) = self.held_actors(rig) {
            return (held, true);
        }
        let actors: Arc<[super::mesh_pose::Actor]> = super::mesh_pose::actors_uncached(rig, Some(self), false).into();
        self.hold_actors(rig, &actors);
        (actors, false)
    }

    fn held_actors(&self, rig: &Rig) -> Option<Arc<[super::mesh_pose::Actor]>> {
        fn same_sample(node: &Node, a: Option<NativeSample>, b: Option<NativeSample>) -> bool {
            if node
                .native
                .as_ref()
                .is_none_or(|source| source.skeleton.is_empty())
            {
                return true;
            }
            let same_age = |clip: usize, a: f32, b: f32| {
                a.to_bits() == b.to_bits()
                    || node.clips[clip].animation.as_ref().is_none_or(|animation| {
                        (a * node.clips[clip].state.speed + node.clips[clip].state.start)
                            .is_finite()
                            && (b * node.clips[clip].state.speed + node.clips[clip].state.start)
                                .is_finite()
                            && animation
                                .joints
                                .iter()
                                .all(|joint| joint.channels.is_empty())
                    })
            };
            match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) if a.clip == b.clip && same_age(a.clip, a.age, b.age) => {
                    match (a.previous, b.previous) {
                        (None, None) => true,
                        (Some((ac, aa, aw)), Some((bc, ba, bw))) => {
                            ac == bc && same_age(ac, aa, ba) && aw.to_bits() == bw.to_bits()
                        }
                        _ => false,
                    }
                }
                _ => false,
            }
        }
        let cache = self.native_actors.borrow();
        let rig_key = std::ptr::from_ref(rig) as usize;
        cache
            .as_ref()
            .filter(|held| {
                held.rig == rig_key
                    && held
                        .samples
                        .iter()
                        .zip(&self.native_samples)
                        .zip(&rig.nodes)
                        .all(|((&a, &b), node)| same_sample(node, a, b))
            })
            .map(|held| Arc::clone(&held.actors))
    }

    fn hold_actors(&self, rig: &Rig, actors: &Arc<[super::mesh_pose::Actor]>) {
        let mut cache = self.native_actors.borrow_mut();
        let rig_key = std::ptr::from_ref(rig) as usize;
        let held = cache.get_or_insert_with(|| NativeActors {
            rig: rig_key,
            samples: Vec::new(),
            actors: Arc::clone(actors),
        });
        held.rig = rig_key;
        held.samples.clone_from(&self.native_samples);
        held.actors = Arc::clone(actors);
    }

    pub fn native_locals(
        &self,
        rig: &Rig,
        index: usize,
    ) -> std::io::Result<Option<super::mesh_pose::Locals>> {
        locals_of(&self.native_samples, rig, index)
    }

    pub(super) fn samples(&self) -> &[Option<NativeSample>] {
        &self.native_samples
    }
}

pub(super) fn locals_at(
    samples: &[Option<NativeSample>],
    rig: &Rig,
    index: usize,
) -> std::io::Result<Option<super::mesh_pose::Locals>> {
    if index >= samples.len() {
        return Ok(None);
    }
    locals_of(samples, rig, index)
}

fn locals_of(
    samples: &[Option<NativeSample>],
    rig: &Rig,
    index: usize,
) -> std::io::Result<Option<super::mesh_pose::Locals>> {
        let Some(playing) = samples[index] else {
            return Ok(None);
        };
        let node = &rig.nodes[index];
        let sample = |clip: usize, age| {
            let clip = &node.clips[clip];
            clip.animation
                .as_ref()
                .map(|a| {
                    super::mesh_pose::sample(
                        a,
                        age * clip.state.speed + clip.state.start,
                        clip.state.looping,
                    )
                })
                .transpose()
        };
        let Some(mut locals) = sample(playing.clip, playing.age)? else {
            return Ok(None);
        };
        if let Some((old, age, weight)) = playing.previous {
            let old = sample(old, age)?.ok_or_else(|| {
                std::io::Error::other("native crossfade source animation unavailable")
            })?;
            if old.len() < locals.len() {
                return Err(std::io::Error::other(
                    "native crossfade source has fewer records than destination",
                ));
            }
            for (new, old) in locals.iter_mut().zip(old) {
                *new = super::mesh_pose::blend(old, *new, weight);
            }
        }
        Ok(Some(super::mesh_pose::Locals {
            poses: locals,
            crossfade: playing.previous.is_some(),
        }))
}

impl Player {
    pub fn root_clip_phase(&self, rig: &Rig) -> f64 {
        let Some((sample, root)) = self
            .native_samples
            .first()
            .copied()
            .flatten()
            .zip(rig.nodes.first())
        else {
            return 0.0;
        };
        let clip = &root.clips[sample.clip];
        if !played_animation(root, sample.clip) {
            return 0.0;
        }
        let Some(animation) = &clip.animation else {
            return 0.0;
        };
        if !animation.fps.is_finite() || animation.fps <= 0.0 || animation.samples == 0 {
            return 0.0;
        }
        let duration =
            f64::from(animation.samples.saturating_sub(1).max(1)) / f64::from(animation.fps);
        let phase = f64::from(sample.age * clip.state.speed + clip.state.start);
        if clip.state.looping {
            phase.rem_euclid(duration)
        } else {
            phase.clamp(0.0, duration)
        }
    }
    pub fn evaluate(
        &mut self,
        rig: &Rig,
        instance: ModelInstance,
        time: f32,
        palette: &mut [PaletteJoint],
        events: &mut Vec<Event>,
    ) {
        self.event_refusals.clear();
        self.evaluate_inner(rig, instance, time, palette, events, true);
    }

    fn evaluate_inner(
        &mut self,
        rig: &Rig,
        instance: ModelInstance,
        time: f32,
        palette: &mut [PaletteJoint],
        events: &mut Vec<Event>,
        admitted: bool,
    ) {
        let event_start = events.len();
        self.kept.retain(|kept| !kept.life.finished());
        let pose_required =
            !palette.is_empty() || rig.nodes.iter().any(|node| node.native.is_none());
        if time < self.last {
            self.nodes.iter_mut().for_each(|n| *n = None);
            self.native_samples.fill(None);
        }
        self.last = time;
        let mut scratch = std::mem::take(&mut self.scratch);
        let EvaluationScratch {
            worlds,
            placements,
            states,
            nodes,
            memo,
        } = &mut scratch;
        placements.clear();
        states.clear();
        for (index, node) in rig.nodes.iter().enumerate() {
            let timeline = self.phase.map_or(time, |(_, phase)| phase);
            if node.native.as_ref().is_some_and(|source| source.pdxmesh)
                && let Some(sample) =
                    self.native_samples[index].filter(|sample| played_animation(node, sample.clip))
            {
                self.native_samples[index] = Some(advance_sample(node, sample, timeline));
            }
            let parent_state = node.parent.as_ref().and_then(|(parent, _, handle)| {
                let (clip, age, cycle, entered) = states[*parent]?;
                let state = &rig.nodes[*parent].clips[clip].state;
                let name = state
                    .propagate
                    .iter()
                    .find(|(h, _)| h == handle)
                    .or_else(|| state.propagate.iter().find(|(h, _)| h == "all"))
                    .map_or(state.name.as_str(), |(_, name)| name.as_str());
                Some((
                    name.to_owned(),
                    age,
                    cycle,
                    entered,
                    age * state.speed + state.start,
                ))
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
            let exists = node.clips.iter().any(|c| c.state.name == requested);
            let chosen = if exists {
                requested
            } else if (self.phase.is_some() || node.native.is_some())
                && index != 0
                && let Some(playing) = &self.nodes[index]
            {
                playing.requested.clone()
            } else {
                node.default.clone()
            };
            let stage =
                (index == 0 && instance.animation >= NAMED && instance.animation & DIRECT == 0)
                    .then(|| stage_intro(instance.animation));
            let (changed, chosen) = match (stage, &self.nodes[index]) {
                (Some(intro), Some(playing)) => {
                    let current = node.clips[playing.clip].state.name.as_str();
                    let settled = current == chosen || Some(current) == intro;
                    match intro {
                        _ if settled => (false, chosen),
                        None => (true, chosen),
                        Some(intro) if node.clips.iter().any(|c| c.state.name == intro) => {
                            (true, intro.to_owned())
                        }
                        Some(_) => (false, chosen),
                    }
                }
                (Some(_), None) => (true, chosen),
                (None, playing) => (
                    playing.as_ref().is_none_or(|p| p.requested != chosen)
                        || (self.phase.is_some()
                            && node.follows_parent
                            && exists
                            && parent_state.as_ref().is_some_and(|parent| parent.3)),
                    chosen,
                ),
            };
            if changed {
                if instance.animation & DIRECT != 0
                    && let Some(playing) = &self.nodes[index]
                {
                    keep_events(
                        &mut self.kept,
                        node,
                        index,
                        playing,
                        time,
                        instance,
                        self.phase.map(|(_, phase)| phase),
                    );
                }
                let seed =
                    SEED ^ instance.animation_seed ^ (index as u32).wrapping_mul(0x9e37_79b9);
                let mut random = self.nodes[index]
                    .as_ref()
                    .map_or_else(|| Random::new(seed), |p| p.random.clone());
                let previous = if node.native.as_ref().is_some_and(|source| source.pdxmesh) {
                    self.native_samples[index]
                        .filter(|sample| played_animation(node, sample.clip))
                        .map(|sample| (sample.clip, sample.age))
                } else {
                    self.nodes[index]
                        .as_ref()
                        .map(|p| (p.clip, time - p.start + p.phase_offset))
                };
                self.nodes[index] = pick(
                    node,
                    &chosen,
                    &mut random,
                    self.phase.is_some() || self.native_choice,
                )
                .map(|clip| {
                    let random_range = if index == 0
                        && instance.native_flags & 8 != 0
                        && instance.animation_phase > 0.0
                    {
                        instance.animation_phase
                    } else if index == 0 && matches!(instance.animation, IDLE | DEFEND) {
                        5.0
                    } else {
                        node.clips[clip].state.random_start
                    };
                    let random_range = if self.phase.is_some() {
                        if random_range > 0.0 {
                            random_range
                        } else {
                            0.0
                        }
                    } else {
                        random_range
                    };
                    let phase = if self.phase.is_some() && random_range == 0.0 {
                        0.0
                    } else if previous.is_none()
                        && index == 0
                        && matches!(instance.animation, ATTACK | SUPPORT)
                    {
                        random.fraction() * duration(&node.clips[clip])
                    } else {
                        random.fraction() * random_range
                    };
                    let start = if self.phase.is_some() {
                        time
                    } else if previous.is_none()
                        && instance.animation_epoch == 0
                        && !matches!(
                            instance.animation,
                            BUILDING_IDLE | BUILDING_DAMAGED | AMBIENT_IDLE
                        )
                        && instance.animation < NAMED
                    {
                        -phase
                    } else {
                        time - if instance.animation & DIRECT != 0 {
                            phase / node.clips[clip].state.speed
                        } else {
                            phase
                        }
                    };
                    Playing {
                        requested: chosen,
                        clip,
                        start,
                        event_time: self.phase.map_or(start, |(_, phase)| phase),
                        previous,
                        admitted_kept: BTreeMap::new(),
                        propagate: self.phase.is_some(),
                        phase_offset: if self.phase.is_some() {
                            phase / node.clips[clip].state.speed
                        } else {
                            0.0
                        },
                        random,
                        cycle: 0,
                        completed: false,
                    }
                });
            }
            let mut state_info = None;
            let mut pose = std::mem::take(&mut worlds[index]);
            pose.clear();
            if let Some(playing) = &mut self.nodes[index] {
                let entered = std::mem::take(&mut playing.propagate);
                let mut age = time - playing.start + playing.phase_offset;
                if index == 0 && instance.animation == TRAIN {
                    age = instance.animation_phase + time;
                }
                let has_played = played_animation(node, playing.clip)
                    || self.native_samples[index]
                        .is_some_and(|sample| played_animation(node, sample.clip));
                if admitted
                    && node.follows_parent
                    && let Some((_, parent_age, cycle, _, parent_phase)) = &parent_state
                    && (self.phase.is_none() || !has_played)
                {
                    age = if self.phase.is_some() {
                        let state = &node.clips[playing.clip].state;
                        (*parent_phase - state.start) / state.speed
                    } else {
                        *parent_age
                    };
                    if self.phase.is_some() {
                        playing.phase_offset = age - (time - playing.start);
                    }
                    playing.cycle = *cycle;
                } else if admitted && self.phase.is_none() {
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
                        let Some(next_clip) =
                            pick(node, next, &mut playing.random, self.native_choice)
                        else {
                            break;
                        };
                        if instance.animation & DIRECT != 0 {
                            keep_events(
                                &mut self.kept,
                                node,
                                index,
                                playing,
                                playing.start + length,
                                instance,
                                None,
                            );
                        }
                        playing.previous = Some((playing.clip, length));
                        playing.clip = next_clip;
                        playing.start += length;
                        playing.event_time = playing.start;
                        playing.cycle += 1;
                        playing.completed = true;
                        age = time - playing.start + playing.phase_offset;
                    }
                }
                let clip = &node.clips[playing.clip];
                let length = duration(clip);
                let cycle = if clip.state.looping && length > 0.0 {
                    (age / length).floor().max(0.0) as u64
                } else {
                    playing.cycle
                };
                let mut local_age = if self.phase.is_some() && clip.animation.is_some() {
                    (clip_phase(clip, age) - clip.state.start) / clip.state.speed
                } else if clip.state.looping && length > 0.0 {
                    age.rem_euclid(length)
                } else {
                    age
                };
                let native_previous = playing.previous.and_then(|(old, old_age)| {
                    let elapsed = self.phase.map_or(time - playing.start, |(_, phase)| {
                        phase - playing.event_time
                    });
                    let blend = clip.state.blend;
                    let weight = if blend > 0.0 { elapsed / blend } else { 1.0 };
                    (weight < 1.0
                        && node.clips[old].animation.is_some()
                        && clip.animation.is_some())
                    .then_some((
                        old,
                        old_age + if self.phase.is_some() { elapsed } else { 0.0 },
                        weight,
                    ))
                });
                let retained = (clip.animation.is_none()
                    && node.native.as_ref().is_some_and(|source| source.pdxmesh))
                .then(|| {
                    self.native_samples[index].filter(|sample| played_animation(node, sample.clip))
                })
                .flatten()
                .map(|sample| advance_sample(node, sample, timeline));
                let native = retained.unwrap_or(NativeSample {
                    clip: playing.clip,
                    age,
                    time: timeline,
                    previous: native_previous,
                });
                if retained.is_some() {
                    local_age = (clip_phase(&node.clips[native.clip], native.age)
                        - clip.state.start)
                        / clip.state.speed;
                }
                self.native_samples[index] = Some(native);
                if !admitted {
                    states.push(Some((playing.clip, local_age, cycle, entered)));
                    continue;
                }
                let played = &node.clips[native.clip];
                if pose_required {
                    let scratch = &mut nodes[index];
                    sample(played, native.age, &mut scratch.locals);
                    if let Some((old, old_age, t)) = native.previous
                        && let (Some(old_animation), Some(animation)) =
                            (&node.clips[old].animation, &played.animation)
                    {
                        sample(&node.clips[old], old_age, &mut scratch.previous);
                        let join =
                            scratch
                                .crossfades
                                .entry((old, native.clip))
                                .or_insert_with(|| {
                                    animation
                                        .joints
                                        .iter()
                                        .map(|joint| {
                                            old_animation
                                                .joints
                                                .iter()
                                                .rposition(|old| old.name == joint.name)
                                        })
                                        .collect()
                                });
                        for (old, value) in join.iter().zip(&mut scratch.locals) {
                            if let Some(old) = old.and_then(|at| scratch.previous.get(at)) {
                                *value = old.blend(*value, t);
                            }
                        }
                    }
                    if let Some(animation) = &played.animation {
                        let binding = scratch.bindings[native.clip]
                            .get_or_insert_with(|| mesh::PoseBinding::new(&node.mesh, animation));
                        binding.sample_into(&scratch.locals, &mut pose, memo);
                    }
                }
                state_info = Some((playing.clip, local_age, cycle, entered));
            } else {
                self.native_samples[index] = None;
            }
            if !admitted {
                states.push(None);
                continue;
            }
            let own = if pose_required {
                if pose.is_empty() {
                    let count = node
                        .mesh
                        .skeleton
                        .iter()
                        .map(|j| j.index as usize + 1)
                        .max()
                        .unwrap_or(0);
                    pose.resize(count, Transform::IDENTITY);
                    for joint in &node.mesh.skeleton {
                        pose[joint.index as usize] = node
                            .mesh
                            .bind_pose(&joint.name)
                            .unwrap_or(Transform::IDENTITY);
                    }
                }
                let parent =
                    node.parent
                        .as_ref()
                        .map_or(Transform::IDENTITY, |(parent, joint, _)| {
                            let joint = rig.nodes[*parent]
                                .mesh
                                .joint(joint)
                                .and_then(|j| worlds[*parent].get(j.index as usize))
                                .copied()
                                .or_else(|| {
                                    rig.nodes[*parent]
                                        .locators
                                        .get(joint)
                                        .map(|(local, _)| *local)
                                })
                                .unwrap_or(Transform::IDENTITY);
                            placements[*parent].compose(&joint)
                        });
                let own = parent.compose(&Transform::from_trs(
                    [0.0; 3],
                    [0.0, 0.0, 0.0, 1.0],
                    node.scale,
                ));
                if !palette.is_empty() {
                    palette[node.rigid as usize - rig.start] = PaletteJoint::from_transform(&own);
                }
                if let Some(base) = node.skin_base.filter(|_| !palette.is_empty()) {
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
                own
            } else {
                Transform::IDENTITY
            };
            if let Some((clip, age, _cycle, _entered)) = state_info {
                let mut authored = node.clips[clip]
                    .state
                    .events
                    .iter()
                    .map(|event| (event, age, None, None, None, None))
                    .collect::<Vec<_>>();
                if index == 0 && instance.animation == TRAIN {
                    authored.extend(
                        node.clips
                            .iter()
                            .filter(|c| c.state.name == "move")
                            .flat_map(|c| c.state.events.iter())
                            .filter(|e| e.keep_particle)
                            .map(|e| {
                                (e, time + instance.animation_phase, None, None, None, None)
                            }),
                    );
                }
                authored.extend(
                    self.kept
                        .iter()
                        .filter(|kept| kept.source == index)
                        .map(|kept| {
                            (
                                &kept.definition,
                                self.phase.map_or(time, |(_, phase)| phase) - kept.start,
                                Some(kept.end),
                                Some(kept.seed),
                                Some(kept.key.clone()),
                                Some(kept.life.clone()),
                            )
                        }),
                );
                for (event_index, (event, age, emission_end, seed, key, kept)) in
                    authored.into_iter().enumerate()
                {
                    if self.phase.is_some()
                        && event.trigger_once
                        && emission_end.is_none()
                        && let Some(playing) = &self.nodes[index]
                        && time - playing.start > duration(&node.clips[clip])
                        && (!self.native_samples[index]
                            .is_some_and(|sample| played_animation(node, sample.clip))
                            || node.clips[clip].state.looping)
                    {
                        continue;
                    }
                    let event_clock = if self.phase.is_some() && emission_end.is_none() {
                        age
                    } else if event.trigger_once {
                        if instance.animation_epoch != 0
                            || matches!(
                                instance.animation,
                                BUILDING_IDLE | BUILDING_DAMAGED | AMBIENT_IDLE
                            )
                            || instance.animation >= NAMED
                        {
                            self.nodes[index]
                                .as_ref()
                                .map_or(age, |p| time - p.start + p.phase_offset)
                        } else {
                            time + instance.animation_phase
                        }
                    } else {
                        age
                    };
                    let event_time = if instance.animation & DIRECT != 0 && emission_end.is_none() {
                        (event.time - node.clips[clip].state.start) / node.clips[clip].state.speed
                    } else {
                        event.time
                    };
                    if event_clock < event_time {
                        continue;
                    }
                    let local = if node.native.is_some() {
                        Transform::IDENTITY
                    } else {
                        node.mesh
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
                            .unwrap_or(Transform::IDENTITY)
                    };
                    let transform = own.compose(&local);
                    if node.native.is_none() && transform.uniform_scale() <= 1e-6 {
                        continue;
                    }
                    events.push(Event {
                        definition: event.clone(),
                        transform: world_transform(instance).compose(&transform),
                        age: event_clock - event_time,
                        seed: seed.unwrap_or_else(|| {
                            event_seed(instance, index, event_index, self.nodes[index].as_ref())
                        }),
                        key: key.unwrap_or_else(|| {
                            event_key(instance, index, event_index, self.nodes[index].as_ref())
                        }),
                        emission_end,
                        hidden: false,
                        state: instance.animation,
                        kept,
                    });
                }
            }
            states.push(state_info);
            placements.push(own);
            worlds[index] = pose;
        }
        self.scratch = scratch;
        if admitted
            && events[event_start..]
                .iter()
                .any(|event| rig.nodes[event.key.node].native.is_some())
        {
            self.resolve_events(rig, instance, event_start, events);
        }
        if admitted && let Some((_, phase)) = self.phase {
            for event in &events[event_start..] {
                if event.definition.keep_particle
                    && event.definition.particle.is_some()
                    && event.emission_end.is_none()
                    && let Some(playing) = &mut self.nodes[event.key.node]
                {
                    playing
                        .admitted_kept
                        .insert(event.key.event, phase - event.age);
                }
            }
        }
    }

    fn resolve_events(
        &mut self,
        rig: &Rig,
        instance: ModelInstance,
        start: usize,
        events: &mut Vec<Event>,
    ) {
        use super::mesh_pose::{self, Binding, LookupMode};
        let actors = mesh_pose::actors(rig, Some(self), false);
        let mut ordinal = 0;
        events.retain_mut(|event| {
            let current = ordinal;
            ordinal += 1;
            if current < start { return true; }
            let index = event.key.node;
            let node = &rig.nodes[index];
            let Some(source) = &node.native else { return true; };
            let resolve = || {
                let actor = &actors[index];
                let local = match mesh_pose::lookup(source, &event.definition.node, LookupMode::Event) {
                    None => mesh_pose::IDENTITY,
                    Some(binding) => {
                        let no_palette = matches!(binding, Binding::Locator(locator) if locator.parent.is_none())
                            || self.native_samples[index].is_none_or(|sample| !played_animation(node, sample.clip));
                        binding.local(if no_palette { Ok(None) } else {
                            actor.pose.as_ref().map(|pose| Some(pose.joints.as_slice())).map_err(Clone::clone)
                        })?
                    }
                };
                let matrix = mesh_pose::multiply(*actor.world.as_ref().map_err(Clone::clone)?, local);
                Ok::<_, String>(Transform {
                    basis: std::array::from_fn(|column| std::array::from_fn(|row| matrix[column][row])),
                    translation: [matrix[3][0], matrix[3][1], matrix[3][2]],
                })
            };
            match resolve() {
                Ok(transform) if transform.is_finite() => {
                    event.transform = world_transform(instance).compose(&transform);
                    event.transform.uniform_scale() > 1e-6
                }
                result => {
                    let why = result.err().unwrap_or_else(|| "native event transform is nonfinite".to_owned());
                    *self.event_refusals.entry(format!("refused state event node {:?}: {why}", event.definition.node)).or_default() += 1;
                    false
                }
            }
        });
    }
}

pub fn world_transform(instance: ModelInstance) -> Transform {
    if instance.native_flags & 4 != 0 {
        return Transform {
            basis: instance
                .native_basis
                .map(|axis| axis.map(|v| v * instance.scale)),
            translation: instance.position,
        };
    }
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

#[cfg(any(test, feature = "test-support"))]
pub fn stage_rig(dive_intro: bool) -> Rig {
    let mut animation = mesh::Animation::default();
    animation.fps = 1.0;
    animation.samples = 3;
    let animation = Arc::new(animation);
    let clip = |name: &str, looping: bool, next: Option<&str>| Clip {
        state: EntityState {
            name: name.into(),
            chance: 1.0,
            speed: 1.0,
            looping,
            next_state: next.map(str::to_owned),
            ..EntityState::default()
        },
        animation: Some(Arc::clone(&animation)),
    };
    let mut clips = vec![clip("cruise", true, None), clip("dive", true, None)];
    if dive_intro {
        clips.push(clip("dive_intro", false, Some("dive")));
    }
    Rig {
        nodes: vec![Node {
            mesh: Arc::new(mesh::Mesh::default()),
            native: None,
            clips,
            default: "cruise".into(),
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

impl Player {
    pub fn root_play_position(&self, rig: &Rig) -> Option<(f32, f32)> {
        let node = rig.nodes.first()?;
        let playing = self.nodes.first()?.as_ref()?;
        let (clip, age) = match self.native_samples.first().copied().flatten() {
            Some(sample) => (sample.clip, sample.age),
            None => (
                playing.clip,
                self.last - playing.start + playing.phase_offset,
            ),
        };
        if !played_animation(node, clip) {
            return None;
        }
        let clip = &node.clips[clip];
        let length = clip.animation.as_ref()?.duration();
        if playing.completed {
            return Some((length, length));
        }
        Some((age.max(0.0) * clip.state.speed + clip.state.start, length))
    }

    pub fn root_play_clip(&self, rig: &Rig) -> Option<(StateTiming, f32, bool)> {
        let node = rig.nodes.first()?;
        let playing = self.nodes.first()?.as_ref()?;
        let (clip, age) = match self.native_samples.first().copied().flatten() {
            Some(sample) => (sample.clip, sample.age),
            None => (
                playing.clip,
                self.last - playing.start + playing.phase_offset,
            ),
        };
        if !played_animation(node, clip) {
            return None;
        }
        let clip = &node.clips[clip];
        Some((
            StateTiming {
                start: clip.state.start,
                speed: clip.state.speed,
                length: clip.animation.as_ref()?.duration(),
                looping: clip.state.looping,
            },
            age.max(0.0),
            playing.completed,
        ))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn root_entry(&self) -> Option<(usize, f32, u32, u64)> {
        let playing = self.nodes.first()?.as_ref()?;
        Some((
            playing.clip,
            playing.event_time,
            playing.random.index,
            playing.cycle,
        ))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn root_playing<'r>(&self, rig: &'r Rig) -> Option<&'r str> {
        let playing = self.nodes.first()?.as_ref()?;
        Some(rig.nodes[0].clips[playing.clip].state.name.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateTiming {
    pub start: f32,
    pub speed: f32,
    pub length: f32,
    pub looping: bool,
}

impl StateTiming {
    pub fn reads_at_least(self, threshold: f32) -> Option<(f32, Option<f32>)> {
        let completes = (self.speed > 0.0 && !self.looping && self.length > 0.0)
            .then(|| self.length / self.speed);
        let until = completes.filter(|_| threshold > self.length);
        if threshold <= self.start {
            return Some((0.0, until));
        }
        if self.speed <= 0.0 {
            return None;
        }
        let at = (threshold - self.start) / self.speed;
        match completes {
            Some(completes) if at >= completes => {
                (threshold <= self.length).then_some((completes, None))
            }
            _ => Some((at, until)),
        }
    }

    pub fn reaches(self, threshold: f32) -> Option<f32> {
        self.reads_at_least(threshold).map(|(from, _)| from)
    }
}

#[cfg(test)]
mod naval_clock_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aircraft_callbacks_allow_automatic_states_and_finish_kept_events() {
        let mut rig = stage_rig(true);
        rig.nodes[0].clips[2].state.events.push(mesh::EntityEvent {
            time: 0.1,
            particle: Some("drop".into()),
            keep_particle: true,
            ..Default::default()
        });
        let mut player = Player::new(&rig);
        let mut palette = [PaletteJoint::IDENTITY];
        let instance = ModelInstance {
            scale: 1.,
            animation_seed: 7,
            ..Default::default()
        };
        let mut clock = Clock {
            time: 1.2,
            phase: None,
            requests: Arc::new(vec![
                Request {
                    time: 1.,
                    phase_time: None,
                    state: "dive_intro".into(),
                },
                Request {
                    time: 4.,
                    phase_time: None,
                    state: "dive".into(),
                },
                Request {
                    time: 4.1,
                    phase_time: None,
                    state: "missing".into(),
                },
            ]),
        };
        let mut events = Vec::new();
        player.evaluate_clock(&rig, instance, &clock, &mut palette, &mut events);
        assert_eq!(player.root_playing(&rig), Some("dive_intro"));
        assert!((events[0].age - 0.1).abs() < 1e-5);
        clock.time = 4.2;
        events.clear();
        player.evaluate_clock(&rig, instance, &clock, &mut palette, &mut events);
        assert_eq!(player.root_playing(&rig), Some("dive"));
        assert_eq!(player.nodes[0].as_ref().unwrap().start, 3.);
        assert_eq!(events.len(), 1);
        assert!((events[0].age - 3.1).abs() < 1e-5);
        assert!((events[0].emission_end.unwrap() - 1.9).abs() < 1e-5);
    }

    #[test]
    fn callback_players_enter_a_lone_zero_chance_variant_and_draw_among_unweighted_ones() {
        let mut rig = stage_rig(false);
        rig.nodes[0].clips[1].state.chance = 0.0;
        rig.nodes[0].clips[0].state.chance = 0.0;
        let second = rig.nodes[0].clips[0].clone();
        rig.nodes[0].clips.push(second);
        let instance = ModelInstance {
            scale: 1.,
            animation_seed: 7,
            ..Default::default()
        };
        for (state, entered) in [("dive", &[1][..]), ("cruise", &[0, 2][..])] {
            let clock = Clock {
                time: 1.5,
                phase: None,
                requests: Arc::new(vec![Request {
                    time: 1.,
                    phase_time: None,
                    state: state.into(),
                }]),
            };
            let mut player = Player::new(&rig);
            player.evaluate_clock(
                &rig,
                instance,
                &clock,
                &mut [PaletteJoint::IDENTITY],
                &mut Vec::new(),
            );
            let playing = player.nodes[0].as_ref().map(|playing| playing.clip);
            assert!(
                playing.is_some_and(|clip| entered.contains(&clip)),
                "{state}: {playing:?}"
            );
        }
    }

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
                native: None,
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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

    fn playing_state<'r>(player: &Player, rig: &'r Rig) -> &'r str {
        let playing = player.nodes[0].as_ref().expect("the root node plays");
        &rig.nodes[0].clips[playing.clip].state.name
    }

    fn raid_unit(stage: u32) -> ModelInstance {
        ModelInstance {
            animation: stage,
            animation_seed: 7,
            scale: 1.0,
            ..ModelInstance::default()
        }
    }

    #[test]
    fn a_stage_crossed_during_playback_plays_its_intro_then_its_state() {
        content_reader_script::test_game::enter();
        let rig = stage_rig(true);
        let mut player = Player::new(&rig);
        let mut palette = [PaletteJoint::from_transform(&Transform::IDENTITY)];
        let mut events = Vec::new();
        let cruise = raid_unit(named_stage("cruise", None));
        let dive = raid_unit(named_stage("dive", Some("dive_intro")));
        player.evaluate(&rig, cruise, 10.0, &mut palette, &mut events);
        assert_eq!(playing_state(&player, &rig), "cruise");
        player.evaluate(&rig, dive, 11.0, &mut palette, &mut events);
        assert_eq!(playing_state(&player, &rig), "dive_intro");
        assert_eq!(player.nodes[0].as_ref().unwrap().start, 11.0);
        player.evaluate(&rig, dive, 12.9, &mut palette, &mut events);
        assert_eq!(playing_state(&player, &rig), "dive_intro");
        player.evaluate(&rig, dive, 13.1, &mut palette, &mut events);
        assert_eq!(playing_state(&player, &rig), "dive");
        player.evaluate(&rig, dive, 30.0, &mut palette, &mut events);
        assert_eq!(playing_state(&player, &rig), "dive");
    }

    #[test]
    fn a_fresh_entity_mid_stage_shows_the_state_from_its_own_entry() {
        content_reader_script::test_game::enter();
        let rig = stage_rig(true);
        let mut player = Player::new(&rig);
        let mut palette = [PaletteJoint::from_transform(&Transform::IDENTITY)];
        let mut events = Vec::new();
        let dive = raid_unit(named_stage("dive", Some("dive_intro")));
        player.evaluate(&rig, dive, 50.0, &mut palette, &mut events);
        assert_eq!(playing_state(&player, &rig), "dive");
        assert_eq!(player.nodes[0].as_ref().unwrap().start, 50.0);
    }

    #[test]
    fn a_stage_whose_intro_the_entity_lacks_keeps_the_current_state() {
        content_reader_script::test_game::enter();
        let rig = stage_rig(false);
        let mut player = Player::new(&rig);
        let mut palette = [PaletteJoint::from_transform(&Transform::IDENTITY)];
        let mut events = Vec::new();
        player.evaluate(
            &rig,
            raid_unit(named_stage("cruise", None)),
            10.0,
            &mut palette,
            &mut events,
        );
        let dive = raid_unit(named_stage("dive", Some("dive_intro")));
        player.evaluate(&rig, dive, 11.0, &mut palette, &mut events);
        assert_eq!(playing_state(&player, &rig), "cruise");
        assert_eq!(player.nodes[0].as_ref().unwrap().start, 10.0);
        player.evaluate(&rig, dive, 30.0, &mut palette, &mut events);
        assert_eq!(playing_state(&player, &rig), "cruise");
        let plain = raid_unit(named_stage("dive", None));
        player.evaluate(&rig, plain, 31.0, &mut palette, &mut events);
        assert_eq!(playing_state(&player, &rig), "dive");
        let mut fresh = Player::new(&rig);
        fresh.evaluate(&rig, dive, 40.0, &mut palette, &mut events);
        assert_eq!(playing_state(&fresh, &rig), "dive");
        assert_eq!(stage_intro(dive.animation), Some("dive_intro"));
        assert_eq!(state_name(dive.animation), "dive");
        assert_eq!(stage_intro(IDLE), None);
    }
}

#[cfg(test)]
mod pose_tests;

#[cfg(any(test, feature = "test-support"))]
pub use pose_fixture::rig as pose_rig;

#[cfg(any(test, feature = "test-support"))]
impl Player {
    pub fn kept_events_empty(&self) -> bool {
        self.kept.is_empty()
    }
}

#[cfg(any(test, feature = "test-support"))]
mod pose_fixture;
