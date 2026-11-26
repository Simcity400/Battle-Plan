
use crate::{Frame, Group, Listener, Mix};
use content_reader_sound::sound::{
    Catalog, Compressor, Curve, Defines, Effect, Falloff, Notes, Overflow,
};
use content_reader_sound::wav::Pcm;
use std::{collections::HashMap, sync::Arc};

pub const RATE: u32 = 44_100;
pub const CALLBACK_FRAMES: usize = crate::sdl::MIXER_FRAMES;
const VOICE_SLOTS: usize = 256;
const RESERVED_VOICES: usize = 8;
const MIXED_VOICES: usize = 32;
const UNITY: i32 = 16_384;
const FIRED_WINDOW: f32 = 0.1;

#[derive(Debug)]
pub struct Samples {
    pub data: Vec<i16>,
}

impl Samples {
    pub fn from_pcm(pcm: &Pcm) -> Self {
        Self {
            data: crate::sdl::to_mixer(pcm),
        }
    }

    pub fn frames(&self) -> usize {
        self.data.len() / 2
    }
}

#[derive(Debug)]
pub struct Content {
    pub catalog: Catalog,
    pub defines: Option<Defines>,
    height_category: Option<usize>,
    voice_category: Option<usize>,
    pub music: content_reader_sound::music::Music,
    pub stations: Option<Vec<String>>,
    pub recently_played: usize,
    pub battle: Option<content_reader_sound::combat_sounds::BattleDefines>,
    pub layers: content_reader_sound::combat_sounds::Layers,
}

impl Content {
    pub fn new(catalog: Catalog, defines: Option<Defines>) -> Self {
        let height_category = defines
            .as_ref()
            .and_then(|defines| catalog.category(&defines.height_category));
        let voice_category = defines
            .as_ref()
            .and_then(|defines| catalog.category(&defines.voice_category));
        Self {
            catalog,
            defines,
            height_category,
            voice_category,
            music: Default::default(),
            stations: None,
            recently_played: 0,
            battle: None,
            layers: Default::default(),
        }
    }

    pub fn with_battles(
        mut self,
        battle: Option<content_reader_sound::combat_sounds::BattleDefines>,
        layers: content_reader_sound::combat_sounds::Layers,
    ) -> Self {
        self.battle = battle;
        self.layers = layers;
        self
    }

    pub fn with_music(
        mut self,
        music: content_reader_sound::music::Music,
        stations: Option<Vec<String>>,
        recently_played: usize,
    ) -> Self {
        self.music = music;
        self.stations = stations;
        self.recently_played = recently_played;
        self
    }

    pub fn enabled(&self, song: usize) -> bool {
        let station = &self.music.songs[song].station;
        self.stations
            .as_ref()
            .is_none_or(|stations| stations.contains(station))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Load {
    Sound(usize),
    Track(usize),
}

const RANDOM_COUNTER_STEP: u32 = 0x4ad6_85b3;
const RANDOM_MIX_OFFSET: u32 = 0x68e3_1da4;
const RANDOM_MIX_MULTIPLIERS: [u32; 2] = [0x1b56_c4e9, 0x92d6_8ca2];
const RANDOM_SCALE: f32 = 1.0 / 2_147_483_648.0;

#[derive(Clone, Debug)]
pub struct Random {
    seed: u32,
    count: u32,
}

impl Random {
    pub fn new(seed: u32) -> Self {
        Self { seed, count: 0 }
    }

    pub fn word(&mut self) -> u32 {
        let mut x = self
            .seed
            .wrapping_sub(self.count.wrapping_mul(RANDOM_COUNTER_STEP));
        self.count = self.count.wrapping_add(1);
        x = (x ^ (x >> 8)).wrapping_add(RANDOM_MIX_OFFSET);
        x = (x ^ (x << 8)).wrapping_mul(RANDOM_MIX_MULTIPLIERS[0]);
        x = (x ^ (x >> 8)).wrapping_mul(RANDOM_MIX_MULTIPLIERS[1]);
        (x ^ (x >> 8)) & 0x7fff_ffff
    }

    pub fn fraction(&mut self) -> f32 {
        self.word() as f32 * RANDOM_SCALE
    }
}

pub fn weighted(weights: &[i64], word: u32, fallback: Option<usize>) -> Option<usize> {
    let total: i64 = weights.iter().filter(|weight| **weight > 0).sum();
    if total == 0 {
        return fallback;
    }
    let target = (f64::from(word) * f64::from(RANDOM_SCALE) * total as f64) as u64;
    let mut sum = 0u64;
    for (index, weight) in weights.iter().enumerate() {
        if *weight > 0 {
            sum += *weight as u64;
            if target < sum {
                return Some(index);
            }
        }
    }
    fallback
}

fn uniform(word: u32, count: usize) -> usize {
    (f64::from(word) * f64::from(RANDOM_SCALE) * count as f64) as usize
}

pub fn falloff_gain(falloff: &Falloff, listener: [f32; 3], emitter: [f32; 3]) -> (f32, bool) {
    let dx = emitter[0] - listener[0];
    let mut dy = emitter[1] - listener[1];
    let dz = emitter[2] - listener[2];
    if falloff.height_scale > 0.0 {
        dy *= falloff.height_scale;
    }
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();
    if distance > falloff.max_distance + falloff.max_distance {
        return (0.0, true);
    }
    let span = falloff.max_distance - falloff.min_distance;
    let mut t = distance - falloff.min_distance;
    if span > 0.0 {
        t /= span;
    }
    let t = if t < 0.0 { 0.0 } else { t.min(1.0) };
    let gain = match falloff.curve {
        Curve::Linear => 1.0 - t,
        Curve::Logarithmic => -(t * 0.9 + 0.1).log10(),
    };
    (gain, false)
}

pub fn balance(listener: &Listener, emitter: [f32; 3]) -> f32 {
    let d = [
        emitter[0] - listener.position[0],
        emitter[1] - listener.position[1],
        emitter[2] - listener.position[2],
    ];
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if length <= 0.0 || !length.is_finite() {
        return 0.0;
    }
    let pan =
        (listener.right[0] * d[0] + listener.right[1] * d[1] + listener.right[2] * d[2]) / length;
    pan.clamp(-1.0, 1.0)
}

fn truncate(value: f32) -> i32 {
    if !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

fn round(value: f32) -> i32 {
    truncate(if value < 0.0 {
        value - 0.5
    } else {
        value + 0.5
    })
}

fn step(gain: f32) -> i32 {
    round(gain * UNITY as f32).clamp(0, UNITY)
}

fn sides(gain: i32, pan: f32) -> [i32; 2] {
    if pan == 0.0 {
        return [gain, gain];
    }
    let (left, right) = if pan < 0.0 {
        (1.0, pan + 1.0)
    } else {
        (1.0 - pan, 1.0)
    };
    let gain = gain as f32;
    [round(left * gain), round(right * gain)]
}

pub fn ramp(old: i32, new: i32, frame: usize, frames: usize) -> i32 {
    old + (frame as i32 * (new - old)) / frames as i32
}

pub fn scale(sample: i16, gain: i32) -> i32 {
    (i32::from(sample) * gain) / UNITY
}

#[derive(Clone, Debug)]
struct Dynamics {
    settings: Compressor,
    attack: f32,
    release: f32,
    makeup_ratio: f32,
    threshold: f32,
    envelope: [f32; 2],
}

const LEVEL_SCALE: f32 = 1.0 / 32_768.0;

impl Dynamics {
    fn new(settings: Compressor) -> Self {
        let coefficient = |time: f32| {
            if time == 0.0 {
                0.0
            } else {
                (-1.0 / (RATE as f32 * time)).exp()
            }
        };
        Self {
            attack: coefficient(settings.attack),
            release: coefficient(settings.release),
            makeup_ratio: 1.0 - 1.0 / settings.ratio,
            threshold: 10f32.powf(settings.threshold / 20.0),
            settings,
            envelope: [0.0; 2],
        }
    }

    fn process(&mut self, buffer: &mut [i32]) {
        if !self.settings.enabled {
            return;
        }
        let follows = self.attack != 0.0 || self.release != 0.0;
        for frame in buffer.chunks_exact_mut(2) {
            let input = [
                frame[0].wrapping_abs() as f32 * self.settings.pregain,
                frame[1].wrapping_abs() as f32 * self.settings.pregain,
            ];
            let level = if follows {
                for (envelope, input) in self.envelope.iter_mut().zip(input) {
                    let coefficient = if input > *envelope {
                        self.attack
                    } else {
                        self.release
                    };
                    *envelope = (*envelope - input) * coefficient + input;
                }
                self.envelope[0].max(self.envelope[1])
            } else {
                input[0].max(input[1])
            } * LEVEL_SCALE;
            if level <= self.threshold && self.settings.postgain == 1.0 {
                continue;
            }
            let decibels = level.max(LEVEL_SCALE).log10() * 20.0;
            let reduction = 0f32.min((self.settings.threshold - decibels) * self.makeup_ratio);
            let gain = 10f32.powf(reduction / 20.0) * self.settings.postgain;
            frame[0] = truncate(frame[0] as f32 * gain);
            frame[1] = truncate(frame[1] as f32 * gain);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Delayed,
    Playing,
    FadingIn,
    FadingOut,
    Stopped,
}

#[derive(Debug)]
struct Voice {
    id: u64,
    sound: usize,
    samples: Option<Arc<Samples>>,
    cursor: usize,
    rate: f32,
    target: f32,
    volume: f32,
    fade: f32,
    state: State,
    looping: bool,
    gain: f32,
    pan: f32,
    previous: Option<(i32, f32)>,
    category: Option<usize>,
    group: Group,
    priority: f32,
}

impl Voice {
    fn stop(&mut self, fade: f32) {
        self.looping = false;
        self.fade = fade;
        self.state = if fade > 0.0 {
            State::FadingOut
        } else {
            State::Stopped
        };
    }

    fn mixes(&self) -> bool {
        matches!(
            self.state,
            State::Playing | State::FadingIn | State::FadingOut
        ) && self.samples.is_some()
    }
}

type Handle = (usize, u64);

fn live(voices: &mut [Option<Voice>], handle: Handle) -> Option<&mut Voice> {
    voices[handle.0]
        .as_mut()
        .filter(|voice| voice.id == handle.1)
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    voice: Option<Handle>,
    choice: Option<usize>,
    delay: f32,
    rate: f32,
}

#[derive(Debug)]
struct Instance {
    key: u64,
    effect: usize,
    group: Group,
    position: [f32; 3],
    age: f32,
    pending: bool,
    restart: bool,
    slots: Vec<Slot>,
    count: usize,
    over: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Started {
    pub at: f64,
    pub hour: Option<i32>,
    pub effect: String,
    pub sound: String,
    pub position: [f32; 3],
    pub gain: f32,
}

pub struct Engine {
    content: Option<Arc<Content>>,
    samples: Vec<Option<Arc<Samples>>>,
    wanted: Vec<Load>,
    requested: Vec<bool>,
    failed: Vec<bool>,
    missing: std::collections::BTreeSet<Arc<str>>,
    player: crate::music::Player,
    chances: Option<Arc<crate::music::Chances>>,
    music_dynamics: Option<Dynamics>,
    last_used: Vec<f64>,
    instances: Vec<Instance>,
    voices: Vec<Option<Voice>>,
    next_voice: u64,
    audible: Vec<i32>,
    listener: Listener,
    mix: Mix,
    hour: Option<i32>,
    emitters: usize,
    random: Random,
    categories: Vec<Option<Dynamics>>,
    master: Option<Dynamics>,
    pub notes: Notes,
    pub log: Option<Vec<Started>>,
    time: f64,
    buses: Vec<(Vec<i32>, bool)>,
}

impl Engine {
    pub fn new(seed: u32) -> Self {
        Self {
            content: None,
            samples: Vec::new(),
            wanted: Vec::new(),
            requested: Vec::new(),
            failed: Vec::new(),
            missing: Default::default(),
            player: Default::default(),
            chances: None,
            music_dynamics: None,
            last_used: Vec::new(),
            instances: Vec::new(),
            voices: (0..VOICE_SLOTS).map(|_| None).collect(),
            next_voice: 0,
            audible: Vec::new(),
            listener: Listener::default(),
            mix: Mix::default(),
            hour: None,
            emitters: 0,
            random: Random::new(seed),
            categories: Vec::new(),
            master: None,
            notes: Notes::default(),
            log: None,
            time: 0.0,
            buses: Vec::new(),
        }
    }

    pub fn content(&self) -> Option<&Arc<Content>> {
        self.content.as_ref()
    }

    pub fn set_content(&mut self, content: Option<Arc<Content>>) {
        self.notes = Notes::default();
        self.player.failures.clear();
        self.instances.clear();
        self.voices.iter_mut().for_each(|voice| *voice = None);
        self.wanted.clear();
        let (sounds, effects, categories) = content.as_ref().map_or((0, 0, Vec::new()), |c| {
            (
                c.catalog.sounds.len(),
                c.catalog.effects.len(),
                c.catalog.categories.clone(),
            )
        });
        self.samples = vec![None; sounds];
        self.requested = vec![false; sounds];
        self.failed = vec![false; sounds];
        self.missing.clear();
        self.last_used = vec![0.0; sounds];
        self.audible = vec![0; effects];
        self.categories = categories
            .iter()
            .map(|category| category.compressor.map(Dynamics::new))
            .collect();
        self.master = content
            .as_ref()
            .and_then(|content| content.catalog.master_compressor)
            .map(Dynamics::new);
        self.music_dynamics = content
            .as_ref()
            .and_then(|content| content.catalog.music_compressor)
            .map(Dynamics::new);
        self.player.stop();
        let same_songs = match (&self.content, &content) {
            (Some(old), Some(new)) => {
                Arc::ptr_eq(old, new)
                    || (old.music.songs == new.music.songs && old.music.tracks == new.music.tracks)
            }
            _ => false,
        };
        if !same_songs {
            self.player.forget_history();
        }
        self.buses = vec![(Vec::new(), false); self.categories.len() + 1];
        self.content = content;
    }

    pub fn take_wanted(&mut self) -> Vec<Load> {
        let mut wanted = std::mem::take(&mut self.wanted);
        wanted.extend(self.player.take_wanted().map(Load::Track));
        wanted
    }

    fn draw_song(&mut self, content: &Content) {
        if let Some(name) = self.player.update(
            &content.music,
            self.chances.as_deref(),
            content.recently_played,
        ) && let Some(log) = &mut self.log
        {
            log.push(Started {
                at: self.time,
                hour: self.hour,
                effect: "music".into(),
                sound: name.to_owned(),
                position: [0.0; 3],
                gain: 1.0,
            });
        }
    }

    pub fn music_draws(&self) -> u64 {
        self.player.draws()
    }

    pub fn music_waiting(&self) -> bool {
        self.player.waiting()
    }

    pub fn prime_music(&mut self, limit: std::time::Duration) {
        self.player.prime(limit);
    }

    pub fn provide_track(&mut self, track: usize, stream: Option<crate::music::TrackStream>) {
        self.player.provide(track, stream);
    }

    pub fn music_playing(&self) -> Option<usize> {
        self.player.playing()
    }

    pub fn provide(&mut self, sound: usize, samples: Option<Arc<Samples>>) {
        let Some(slot) = self.samples.get_mut(sound) else {
            return;
        };
        *slot = samples.clone();
        self.failed[sound] = samples.is_none();
        for voice in self
            .voices
            .iter_mut()
            .flatten()
            .filter(|voice| voice.sound == sound && voice.samples.is_none())
        {
            match &samples {
                Some(samples) => voice.samples = Some(Arc::clone(samples)),
                None => voice.state = State::Stopped,
            }
        }
    }

    pub fn loaded(&self, sound: usize) -> bool {
        self.samples.get(sound).is_some_and(Option::is_some)
    }

    pub fn evict(&mut self, retain: f64) {
        for (sound, slot) in self.samples.iter_mut().enumerate() {
            if let Some(samples) = slot
                && Arc::strong_count(samples) == 1
                && self.time - self.last_used[sound] > retain
            {
                *slot = None;
                self.requested[sound] = false;
            }
        }
    }

    fn want(&mut self, sound: usize) {
        if let Some(used) = self.last_used.get_mut(sound) {
            *used = self.time;
        }
        if let Some(requested) = self.requested.get_mut(sound)
            && !*requested
        {
            *requested = true;
            self.wanted.push(Load::Sound(sound));
        }
    }

    pub fn silence(&mut self) {
        self.player.stop();
        self.silence_effects();
    }

    fn silence_effects(&mut self) {
        let effects = self.audible.len();
        self.instances.clear();
        self.voices.iter_mut().for_each(|voice| *voice = None);
        self.audible = vec![0; effects];
    }

    pub fn census(&self) -> (Option<i32>, usize, usize, usize) {
        let voices = self
            .voices
            .iter()
            .flatten()
            .filter(|voice| voice.state != State::Stopped)
            .count();
        (self.hour, self.emitters, self.instances.len(), voices)
    }

    pub fn needs_frame(&self) -> bool {
        self.voices.iter().flatten().any(|voice| {
            matches!(
                voice.state,
                State::FadingIn | State::FadingOut | State::Delayed
            )
        }) || self.instances.iter().any(|instance| {
            (instance.restart
                || self
                    .content
                    .as_ref()
                    .is_some_and(|content| content.catalog.effects[instance.effect].looping))
                && instance
                    .slots
                    .iter()
                    .filter_map(|slot| slot.voice)
                    .any(|(at, id)| {
                        self.voices[at]
                            .as_ref()
                            .is_none_or(|voice| voice.id != id || voice.state == State::Stopped)
                    })
        })
    }

    pub fn active(&self) -> bool {
        self.voices
            .iter()
            .flatten()
            .any(|voice| voice.state != State::Stopped)
    }

    pub fn set_mix(&mut self, mix: Mix) {
        self.mix = mix;
    }

    pub fn frame(&mut self, frame: &Frame) {
        self.listener = frame.listener;
        self.mix = frame.mix;
        self.hour = frame.hour;
        self.emitters = frame.emitters.len();
        let Some(content) = self.content.clone() else {
            return;
        };
        if frame.reset {
            self.silence_effects();
        }
        self.chances = frame.music.clone();
        self.draw_song(&content);
        let window = frame.elapsed.max(0.0) + FIRED_WINDOW;
        let index: HashMap<u64, usize> = self
            .instances
            .iter()
            .enumerate()
            .map(|(index, instance)| (instance.key, index))
            .collect();
        let mut present = vec![false; self.instances.len()];
        let mut created = Vec::new();
        let mut replaced = Vec::new();
        let mut stopped = Vec::new();
        for emitter in &frame.emitters {
            let Some(effect) = content.catalog.effect(&emitter.effect) else {
                if self.missing.insert(Arc::clone(&emitter.effect)) {
                    self.notes.note(format!(
                        "Entity event names missing sound effect {}",
                        emitter.effect
                    ));
                }
                continue;
            };
            let definition = &content.catalog.effects[effect];
            if let Some(&at) = index.get(&emitter.key) {
                let instance = &mut self.instances[at];
                if emitter.group == Group::Interface {
                    stopped.push(at);
                } else if instance.effect == effect {
                    present[at] = true;
                    if emitter.age + 1e-3 < instance.age && !definition.looping {
                        instance.pending = true;
                    }
                    instance.age = emitter.age;
                    instance.position = emitter.position;
                    instance.group = emitter.group;
                    instance.restart = emitter.restart;
                    continue;
                } else {
                    replaced.push(at);
                }
            }
            let fired = !frame.reset && emitter.age <= window;
            let polyphony = usize::try_from(definition.polyphony).unwrap_or(0);
            let slots = if polyphony > 0 && polyphony <= definition.choices.len() {
                polyphony
            } else {
                1
            };
            created.push(Instance {
                key: emitter.key,
                effect,
                group: emitter.group,
                position: emitter.position,
                age: emitter.age,
                pending: fired || definition.looping,
                restart: emitter.restart,
                slots: vec![Slot::default(); slots],
                count: 0,
                over: false,
            });
            if fired || definition.looping || emitter.restart {
                for choice in definition.choices.clone() {
                    self.want(choice.sound);
                }
            }
        }
        for at in replaced {
            present[at] = false;
        }
        let mut stopping = vec![false; present.len()];
        for at in stopped {
            present[at] = false;
            stopping[at] = true;
        }
        let mut kept = Vec::with_capacity(self.instances.len() + created.len());
        for ((instance, present), stopping) in std::mem::take(&mut self.instances)
            .into_iter()
            .zip(present)
            .zip(stopping)
        {
            if stopping {
                let fade = content.catalog.effects[instance.effect].fade_out;
                for slot in &instance.slots {
                    if let Some(voice) = slot.voice.and_then(|voice| live(&mut self.voices, voice)) {
                        voice.stop(fade);
                    }
                }
                self.retire(instance, &content);
                continue;
            }
            let sounding = instance.group == Group::Interface
                && instance.slots.iter().any(|slot| {
                    slot.voice
                        .is_some_and(|voice| live(&mut self.voices, voice).is_some())
                });
            if present || sounding {
                kept.push(instance);
            } else {
                self.retire(instance, &content);
            }
        }
        kept.extend(created);
        self.instances = kept;
        self.update(&content, frame.elapsed.max(0.0));
    }

    fn retire(&mut self, instance: Instance, content: &Content) {
        let effect = &content.catalog.effects[instance.effect];
        if instance.count > 0 && !instance.over {
            self.audible[instance.effect] -= 1;
        }
        for slot in &instance.slots {
            if let Some(voice) = slot.voice.and_then(|voice| live(&mut self.voices, voice))
                && effect.looping
            {
                voice.stop(effect.fade_out);
            }
        }
    }

    fn update(&mut self, content: &Content, elapsed: f32) {
        for voice in self.voices.iter_mut().flatten() {
            let step = elapsed / voice.fade;
            match voice.state {
                State::FadingOut => {
                    voice.volume = 0f32.max(voice.volume - step);
                    if voice.volume <= 0.0 {
                        if voice.looping {
                            voice.cursor = 0;
                            voice.volume = voice.target;
                        } else {
                            voice.state = State::Stopped;
                        }
                    }
                }
                State::FadingIn => {
                    voice.volume = voice.target.min(step + voice.volume);
                    if voice.volume >= voice.target {
                        voice.state = State::Playing;
                    }
                }
                _ => {}
            }
        }
        for voice in &mut self.voices {
            if voice
                .as_ref()
                .is_some_and(|voice| voice.state == State::Stopped)
            {
                *voice = None;
            }
        }
        let listener = self.listener;
        let mut instances = std::mem::take(&mut self.instances);
        for instance in &mut instances {
            let effect = &content.catalog.effects[instance.effect];
            if instance.count > 0 {
                for slot in &mut instance.slots {
                    if let Some(voice) = slot.voice
                        && live(&mut self.voices, voice).is_none()
                    {
                        slot.voice = None;
                        instance.count -= 1;
                    }
                }
                if instance.count == 0 && !instance.over {
                    self.audible[instance.effect] -= 1;
                }
            }
            let start = instance.pending
                || ((effect.looping || instance.restart) && instance.count != instance.slots.len());
            let (mut gain, beyond) = match effect.falloff {
                Some(falloff) => falloff_gain(
                    &content.catalog.falloffs[falloff],
                    listener.position,
                    instance.position,
                ),
                None => (1.0, false),
            };
            if start {
                self.start(instance, content, !beyond);
            }
            if beyond {
                instance.pending = false;
                if instance.count != 0 {
                    for slot in &mut instance.slots {
                        if let Some(voice) = slot.voice.take()
                            && let Some(voice) = live(&mut self.voices, voice)
                        {
                            voice.stop(effect.fade_out);
                        }
                    }
                    instance.count = 0;
                    if !instance.over {
                        self.audible[instance.effect] -= 1;
                    }
                }
            }
            if instance.count == 0 {
                continue;
            }
            if effect.max_audible >= 0 && effect.overflow == Overflow::Silent {
                if !instance.over && gain <= 0.0 {
                    self.audible[instance.effect] -= 1;
                    instance.over = true;
                }
                if gain > 0.0 && instance.over && self.audible[instance.effect] < effect.max_audible
                {
                    instance.over = false;
                    self.audible[instance.effect] += 1;
                }
            }
            if instance.over {
                gain = 0.0;
            }
            let pan = effect
                .positional
                .then(|| balance(&listener, instance.position));
            for slot in &mut instance.slots {
                let Some(voice) = slot.voice.and_then(|voice| live(&mut self.voices, voice)) else {
                    continue;
                };
                if slot.delay > 0.0 {
                    slot.delay -= elapsed;
                    if slot.delay > 0.0 {
                        continue;
                    }
                    voice.state = State::Playing;
                }
                voice.gain = gain.clamp(0.0, 1.0);
                if let Some(pan) = pan {
                    voice.pan = pan.clamp(-1.0, 1.0);
                }
                voice.group = instance.group;
            }
        }
        self.instances = instances;
    }

    fn start(&mut self, instance: &mut Instance, content: &Content, in_range: bool) {
        let pending = instance.pending;
        instance.pending = false;
        let effect: &Effect = &content.catalog.effects[instance.effect];
        let prior = instance.count;
        let mut create = in_range;
        if prior == 0 {
            instance.over =
                effect.max_audible >= 0 && effect.max_audible <= self.audible[instance.effect];
            if instance.over && effect.overflow == Overflow::Fail {
                create = false;
            }
        }
        if !pending && !create {
            return;
        }
        let offset =
            |range: content_reader_sound::sound::Range| range.first != 0.0 || range.second != 0.0;
        let volume_offset = offset(effect.volume_offset);
        let delay_offset = offset(effect.delay_offset);
        let rate_offset = offset(effect.playback_rate_offset);
        let choices = effect.choices.len();
        if choices == 0 {
            return;
        }
        let looping = effect.looping
            && !(effect.random_sound_when_looping && choices > instance.slots.len())
            && !(effect.looping_volume_offset && volume_offset)
            && !(effect.looping_delay_offset && delay_offset)
            && !(effect.looping_playback_rate_offset && rate_offset);
        let mut weights: Vec<i64> = effect
            .choices
            .iter()
            .map(|choice| i64::from(choice.weight as i32))
            .collect();
        for slot in &instance.slots {
            if let Some(choice) = slot.choice
                && (slot.voice.is_some() || effect.prevent_random_repetition)
            {
                weights[choice] = 0;
            }
        }
        let width = |range: content_reader_sound::sound::Range| range.second - range.first;
        for index in 0..instance.slots.len() {
            if instance.slots[index].voice.is_some() {
                continue;
            }
            let slot = &mut instance.slots[index];
            let choice = match slot.choice {
                Some(choice) if !effect.random_sound_when_looping => choice,
                _ => {
                    let word = self.random.word();
                    let choice = weighted(&weights, word, Some(uniform(word, choices)))
                        .expect("a fallback is given");
                    weights[choice] = 0;
                    choice
                }
            };
            slot.choice = Some(choice);
            slot.delay = 0.0;
            if delay_offset && (pending || effect.looping_delay_offset) {
                slot.delay = 0f32.max(self.random.fraction() * width(effect.delay_offset));
            }
            slot.rate = effect.playback_rate;
            if rate_offset && (pending || effect.looping_playback_rate_offset) {
                slot.rate += self.random.fraction() * width(effect.playback_rate_offset)
                    + effect.playback_rate_offset.first;
            }
            if !create {
                continue;
            }
            let sound = effect.choices[choice].sound;
            self.want(sound);
            if self.failed[sound] {
                self.notes.note(format!(
                    "Sound {} of effect {} failed to load; start refused",
                    content.catalog.sounds[sound].name, effect.name
                ));
                continue;
            }
            let Some(free) = (RESERVED_VOICES..VOICE_SLOTS).find(|at| self.voices[*at].is_none())
            else {
                continue;
            };
            let id = self.next_voice;
            self.next_voice += 1;
            let slot = &mut instance.slots[index];
            slot.voice = Some((free, id));
            instance.count += 1;
            let mut volume = effect.volume;
            if volume_offset && (pending || effect.looping_volume_offset) {
                volume += self.random.fraction() * width(effect.volume_offset)
                    + effect.volume_offset.first;
            }
            let volume = if volume < 0.0 { 0.0 } else { volume.min(1.0) };
            let definition = &content.catalog.sounds[sound];
            let target = volume * definition.volume;
            let state = if slot.delay > 0.0 {
                State::Delayed
            } else if effect.fade_in > 0.0 {
                State::FadingIn
            } else {
                State::Playing
            };
            if let Some(log) = &mut self.log {
                log.push(Started {
                    at: self.time,
                    hour: self.hour,
                    effect: effect.name.clone(),
                    sound: definition.name.clone(),
                    position: instance.position,
                    gain: effect.falloff.map_or(1.0, |f| {
                        falloff_gain(
                            &content.catalog.falloffs[f],
                            self.listener.position,
                            instance.position,
                        )
                        .0
                    }),
                });
            }
            self.voices[free] = Some(Voice {
                id,
                sound,
                samples: self.samples[sound].clone(),
                cursor: 0,
                rate: slot.rate,
                target,
                volume: if state == State::FadingIn {
                    0.0
                } else {
                    target
                },
                fade: effect.fade_in,
                state,
                looping,
                gain: 1.0,
                pan: 0.0,
                previous: None,
                category: effect.category,
                group: instance.group,
                priority: definition.priority,
            });
        }
        if prior == 0 && instance.count > 0 && !instance.over {
            self.audible[instance.effect] += 1;
        }
    }

    fn category_volume(&self, content: &Content, category: Option<usize>) -> f32 {
        let Some(category) = category else {
            return 1.0;
        };
        if Some(category) == content.height_category {
            let altitude = content
                .defines
                .as_ref()
                .map_or(1.0, |defines| defines.height_volume(self.listener.altitude));
            altitude * self.mix.ambient
        } else if Some(category) == content.voice_category {
            self.mix.voice
        } else {
            1.0
        }
    }

    pub fn render(&mut self, out: &mut [i16]) {
        for chunk in out.chunks_mut(CALLBACK_FRAMES * 2) {
            self.callback(chunk);
        }
    }

    fn callback(&mut self, out: &mut [i16]) {
        out.fill(0);
        let Some(content) = self.content.clone() else {
            return;
        };
        let frames = out.len() / 2;
        let mut main = vec![0i32; out.len()];
        for (buffer, active) in &mut self.buses {
            buffer.clear();
            buffer.resize(out.len(), 0);
            *active = false;
        }
        let master = if self.mix.muted {
            0.0
        } else {
            self.mix.master * self.mix.dev_master
        };
        let volumes: Vec<f32> = (0..self.categories.len())
            .map(|category| self.category_volume(&content, Some(category)))
            .collect();
        let music_bus = self.buses.len() - 1;
        let mut mixed = 0usize;
        if let Some(volume) = self.player.volume() {
            let gain = step(1.0 * (master * self.mix.music) * volume);
            let buffer = match self
                .music_dynamics
                .as_ref()
                .is_some_and(|d| d.settings.enabled)
            {
                true => {
                    self.buses[music_bus].1 = true;
                    &mut self.buses[music_bus].0
                }
                false => &mut main,
            };
            self.player
                .mix(buffer, if mixed < MIXED_VOICES { gain } else { 0 });
            mixed += 1;
            for failure in self.player.failures.drain(..) {
                self.notes.note(format!("Unreadable music {failure}"));
            }
            if self.player.playing().is_none() {
                self.draw_song(&content);
            }
        }
        let mut listed = Vec::new();
        for (at, voice) in self.voices.iter_mut().enumerate().skip(RESERVED_VOICES) {
            let Some(voice) = voice else { continue };
            if !voice.mixes() {
                continue;
            }
            self.last_used[voice.sound] = self.time;
            let previous = voice.previous.map_or(-1, |(gain, _)| gain);
            if voice.gain > 0.0 || previous > 0 {
                let priority = voice.priority * voice.gain;
                listed.push((at, priority));
            } else {
                voice.previous = Some((0, voice.pan));
                advance(voice, frames);
            }
        }
        listed.sort_by(|a, b| a.1.total_cmp(&b.1));
        for &(at, _) in listed.iter().rev() {
            let voice = self.voices[at].as_mut().expect("listed above");
            let group = match voice.group {
                Group::Units => self.mix.units,
                Group::Combat => self.mix.combat,
                Group::Interface => self.mix.interface,
            };
            let bus = master * group;
            let category = voice.category.map_or(1.0, |category| volumes[category]);
            let mut new = step(category * bus * voice.gain * voice.volume);
            let counted = mixed < MIXED_VOICES;
            if !counted {
                new = 0;
            }
            let (old, old_pan) = voice.previous.unwrap_or((new, voice.pan));
            let old = if counted { old } else { 0 };
            voice.previous = Some((new, voice.pan));
            let compressed = voice.category.is_some_and(|category| {
                self.categories[category]
                    .as_ref()
                    .is_some_and(|d| d.settings.enabled)
            });
            let buffer = if compressed && counted {
                let category = voice.category.expect("checked above");
                self.buses[category].1 = true;
                &mut self.buses[category].0
            } else if compressed {
                &mut self.buses[voice.category.expect("checked above")].0
            } else {
                &mut main
            };
            if old == 0 && new == 0 {
                advance(voice, frames);
            } else {
                mix_voice(voice, buffer, sides(old, old_pan), sides(new, voice.pan));
            }
            mixed += 1;
        }
        for (index, (buffer, active)) in self.buses.iter_mut().enumerate() {
            if !*active {
                continue;
            }
            let dynamics = if index == music_bus {
                self.music_dynamics.as_mut()
            } else {
                self.categories[index].as_mut()
            };
            if let Some(dynamics) = dynamics {
                dynamics.process(buffer);
            }
            for (sum, value) in main.iter_mut().zip(buffer.iter()) {
                *sum = sum.wrapping_add(*value);
            }
        }
        if let Some(master) = &mut self.master {
            master.process(&mut main);
        }
        for (out, sum) in out.iter_mut().zip(main) {
            *out = sum.clamp(-32_768, 32_767) as i16;
        }
        for voice in &mut self.voices {
            if voice
                .as_ref()
                .is_some_and(|voice| voice.state == State::Stopped)
            {
                *voice = None;
            }
        }
        self.time += frames as f64 / f64::from(RATE);
    }
}

fn advance(voice: &mut Voice, frames: usize) {
    let length = voice.samples.as_ref().map_or(0, |samples| samples.frames());
    if length == 0 {
        voice.state = State::Stopped;
        return;
    }
    let mut left = frames;
    loop {
        let take = (length - voice.cursor).min(left);
        voice.cursor += take;
        left -= take;
        if voice.cursor < length {
            return;
        }
        if !voice.looping {
            voice.state = State::Stopped;
            return;
        }
        voice.cursor = 0;
        if left == 0 {
            return;
        }
    }
}

fn mix_voice(voice: &mut Voice, buffer: &mut [i32], old: [i32; 2], new: [i32; 2]) {
    let samples = Arc::clone(
        voice
            .samples
            .as_ref()
            .expect("only voices with samples mix"),
    );
    let data = &samples.data;
    let length = samples.frames();
    let frames = buffer.len() / 2;
    if length == 0 {
        voice.state = State::Stopped;
        return;
    }
    let mut frame = 0;
    while frame < frames {
        let remaining = length - voice.cursor;
        if voice.rate == 1.0 {
            let take = remaining.min(frames - frame);
            for offset in 0..take {
                let at = frame + offset;
                for side in 0..2 {
                    let gain = ramp(old[side], new[side], at, frames);
                    buffer[at * 2 + side] += scale(data[(voice.cursor + offset) * 2 + side], gain);
                }
            }
            voice.cursor += take;
            frame += take;
        } else {
            let needed = truncate((remaining as f32 / voice.rate).ceil()).wrapping_mul(2);
            let count = needed.min(((frames - frame) * 2) as i32) / 2;
            if count <= 0 {
                return;
            }
            let base = &data[voice.cursor * 2..];
            let last = remaining * 2 - 1;
            let mut position = 0.0f32;
            let mut fraction = 0.0f32;
            let mut index = 0usize;
            for offset in 0..count as usize {
                let at = frame + offset;
                for side in 0..2 {
                    let a = i32::from(base[(index + side).min(last)]);
                    let b = i32::from(base[(index + side + 2).min(last)]);
                    let value = a + truncate((b - a) as f32 * fraction);
                    let gain = ramp(old[side], new[side], at, frames);
                    buffer[at * 2 + side] += (value * gain) / UNITY;
                }
                position += voice.rate;
                let whole = truncate(position);
                fraction = position - whole as f32;
                index = (whole.max(0) as usize) * 2;
            }
            voice.cursor += index / 2;
            frame += count as usize;
        }
        if voice.cursor >= length {
            if !voice.looping {
                voice.state = State::Stopped;
                return;
            }
            voice.cursor = 0;
        }
    }
}

#[cfg(test)]
mod tests;
