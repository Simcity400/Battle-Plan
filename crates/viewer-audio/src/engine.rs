//! The game's sound-effect instances and mixer, without a device.
//!
//! Each map emitter (an entity event that names a sound effect) owns one
//! scoped instance while the event is present. The instance follows the
//! game's per-frame update: its falloff gain from the listener, start and
//! loop restart, the out-of-range stop at twice the falloff's maximum
//! distance, the `max_audible` count and the stereo balance of `is3d`
//! effects. An instance has one slot per `polyphony` voice; starting fills
//! every free slot, drawing a weighted sound, then its delay and playback
//! rate and, for a voice it creates, its volume. Voices live in the mixer's
//! 256 slots, the first eight its own (music); a start finding none free
//! creates no voice.
//!
//! The mixer renders 1,024 frames per device callback in the game's 16-bit
//! integer arithmetic: each voice's gain is the category volume, the master
//! and group volumes, its falloff gain and its volume, in steps of 1/16384,
//! split by its balance, and ramps from the last callback's across the
//! callback. Only the 32 voices with the highest sound priority times gain
//! mix (music first); the rest advance silently. Voices of a category with
//! an enabled compressor mix into that category's buffer, which is
//! compressed when anything mixed into it; then the sum, music included,
//! goes through the master compressor and is clamped to 16 bits.
//!
//! The viewer splits the game's single effects volume into its Units and
//! Combat groups by the emitter's producer; both default to the game's
//! effects volume.

use crate::{Frame, Group, Listener, Mix};
use installed_content_sound::sound::{
    Catalog, Compressor, Curve, Defines, Effect, Falloff, Notes, Overflow,
};
use installed_content_sound::wav::Pcm;
use std::{collections::HashMap, sync::Arc};

/// The mixer's sample rate; compressor time constants use it too.
pub const RATE: u32 = 44_100;
/// Frames the mixer renders per device callback.
pub const CALLBACK_FRAMES: usize = crate::sdl::MIXER_FRAMES;
/// The mixer's voice slots; the first [`RESERVED_VOICES`] are its own.
const VOICE_SLOTS: usize = 256;
const RESERVED_VOICES: usize = 8;
/// Voices mixed per callback; the rest advance silently.
const MIXED_VOICES: usize = 32;
/// The mixer's unity gain step.
const UNITY: i32 = 16_384;
/// An event first seen at most this long after it fired starts its sound.
/// Older events (a recreated entity mid-state) only keep their loops.
const FIRED_WINDOW: f32 = 0.1;

/// A sound's samples as the game's loader converts them: interleaved
/// 16-bit stereo at [`RATE`].
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

/// One content stack's audio definitions.
#[derive(Debug)]
pub struct Content {
    pub catalog: Catalog,
    /// `None` when the content's sound defines are unreadable: no category
    /// takes the ambient or voice volume.
    pub defines: Option<Defines>,
    height_category: Option<usize>,
    voice_category: Option<usize>,
    pub music: installed_content_sound::music::Music,
    /// The user's enabled music stations; `None` enables every station.
    pub stations: Option<Vec<String>>,
    /// `NGame.MUSIC_PLAYER_RECENTLY_PLAYED_SIZE`.
    pub recently_played: usize,
    /// `NSound`'s battle ambience defines; `None` when unreadable, which
    /// leaves battles silent.
    pub battle: Option<installed_content_sound::combat_sounds::BattleDefines>,
    /// The battle ambience layers of `sound/combat_sounds`.
    pub layers: installed_content_sound::combat_sounds::Layers,
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

    /// Adds the battle ambience definitions.
    pub fn with_battles(
        mut self,
        battle: Option<installed_content_sound::combat_sounds::BattleDefines>,
        layers: installed_content_sound::combat_sounds::Layers,
    ) -> Self {
        self.battle = battle;
        self.layers = layers;
        self
    }

    /// Adds the music the player draws from.
    pub fn with_music(
        mut self,
        music: installed_content_sound::music::Music,
        stations: Option<Vec<String>>,
        recently_played: usize,
    ) -> Self {
        self.music = music;
        self.stations = stations;
        self.recently_played = recently_played;
        self
    }

    /// Whether the user enabled `song`'s station.
    pub fn enabled(&self, song: usize) -> bool {
        let station = &self.music.songs[song].station;
        self.stations
            .as_ref()
            .is_none_or(|stations| stations.contains(station))
    }
}

/// A sample file the engine asks the loader for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Load {
    /// A WAVE sound of [`Catalog::sounds`].
    Sound(usize),
    /// An Ogg music track of [`installed_content_sound::music::Music::tracks`].
    Track(usize),
}

/// The step each draw moves the counter through the seed.
const RANDOM_COUNTER_STEP: u32 = 0x4ad6_85b3;
/// The mix's offset after its first shift.
const RANDOM_MIX_OFFSET: u32 = 0x68e3_1da4;
/// The mix's multipliers after its second and third shifts.
const RANDOM_MIX_MULTIPLIERS: [u32; 2] = [0x1b56_c4e9, 0x92d6_8ca2];
/// A draw's scale to [0, 1).
const RANDOM_SCALE: f32 = 1.0 / 2_147_483_648.0;

/// The game's audio hash generator: a counter mixed with a seed.
#[derive(Clone, Debug)]
pub struct Random {
    seed: u32,
    count: u32,
}

impl Random {
    pub fn new(seed: u32) -> Self {
        Self { seed, count: 0 }
    }

    /// The next 31-bit draw.
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

    /// A draw in [0, 1) as the mixer scales it.
    pub fn fraction(&mut self) -> f32 {
        self.word() as f32 * RANDOM_SCALE
    }
}

/// The game's weighted pick: a draw over the positive weights' sum walks
/// them in order. Without a positive weight it returns `fallback`.
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

/// The uniform pick a weighted sound draw falls back to.
fn uniform(word: u32, count: usize) -> usize {
    (f64::from(word) * f64::from(RANDOM_SCALE) * count as f64) as usize
}

/// The falloff gain at `emitter` and whether it is beyond twice the
/// maximum distance, where an instance stops.
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

/// The stereo position of a positional effect: the listener's right axis
/// against the normalised direction to the emitter, from -1 (left) to 1.
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

/// The processor's float-to-integer truncation: out of range is the
/// integer indefinite value.
fn truncate(value: f32) -> i32 {
    if !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

/// Rounds half away from zero, as the mixer rounds its gain steps.
fn round(value: f32) -> i32 {
    truncate(if value < 0.0 {
        value - 0.5
    } else {
        value + 0.5
    })
}

/// A voice's gain step: `gain` in steps of 1/16384, clamped to [0, unity].
fn step(gain: f32) -> i32 {
    round(gain * UNITY as f32).clamp(0, UNITY)
}

/// Splits a gain step by balance: the far side loses the pan's share.
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

/// Frame `frame` of `frames`' gain on the ramp from `old` to `new`.
pub fn ramp(old: i32, new: i32, frame: usize, frames: usize) -> i32 {
    old + (frame as i32 * (new - old)) / frames as i32
}

/// A 16-bit sample at a gain step, truncated toward zero.
pub fn scale(sample: i16, gain: i32) -> i32 {
    (i32::from(sample) * gain) / UNITY
}

/// A compressor's state in the mixer's integer domain; see [`Compressor`].
#[derive(Clone, Debug)]
struct Dynamics {
    settings: Compressor,
    attack: f32,
    release: f32,
    makeup_ratio: f32,
    threshold: f32,
    envelope: [f32; 2],
}

/// The level's floor and scale from 16-bit steps.
const LEVEL_SCALE: f32 = 1.0 / 32_768.0;

impl Dynamics {
    fn new(settings: Compressor) -> Self {
        // One-pole coefficients at the mixer rate; a zero time follows the input.
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

    /// Processes interleaved stereo sums in place. The level is detected
    /// after the pregain; the gain reduction and postgain apply to the sums.
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
    /// Waiting out its slot's delay.
    Delayed,
    Playing,
    FadingIn,
    FadingOut,
    Stopped,
}

#[derive(Debug)]
struct Voice {
    /// Tells this voice from a later one in the same mixer slot.
    id: u64,
    sound: usize,
    /// `None` until the loader provides the samples; the voice waits.
    samples: Option<Arc<Samples>>,
    /// Frames into the samples.
    cursor: usize,
    rate: f32,
    /// The volume a fade reaches: the effect volume with its offset, times
    /// the sound's volume.
    target: f32,
    volume: f32,
    /// The fade's seconds.
    fade: f32,
    state: State,
    /// The sample loops by itself.
    looping: bool,
    /// The instance's falloff gain, clamped to [0, 1]; zero while held silent.
    gain: f32,
    pan: f32,
    /// The last callback's gain step and balance; `None` before the first.
    previous: Option<(i32, f32)>,
    category: Option<usize>,
    group: Group,
    priority: f32,
}

impl Voice {
    /// Fades out over `fade` seconds, or stops at once without one.
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

/// A voice's mixer slot and id.
type Handle = (usize, u64);

/// The live voice `handle` names.
fn live(voices: &mut [Option<Voice>], handle: Handle) -> Option<&mut Voice> {
    voices[handle.0]
        .as_mut()
        .filter(|voice| voice.id == handle.1)
}

/// One of an instance's `polyphony` slots.
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    /// The voice's mixer slot and id.
    voice: Option<Handle>,
    /// The choice it last played.
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
    /// A start was requested: the event fired.
    pending: bool,
    /// Starts again whenever no voice plays ([`crate::Emitter::restart`]).
    restart: bool,
    slots: Vec<Slot>,
    /// Slots holding a voice.
    count: usize,
    /// Over the effect's `max_audible`: not counted, held silent.
    over: bool,
}

/// One started voice, for diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub struct Started {
    /// Seconds of mixed audio before the start.
    pub at: f64,
    /// The presented recorded hour.
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
    /// Sounds whose samples failed to load: their voices are not created.
    failed: Vec<bool>,
    /// Effect names entity events used that no asset declares, noted once each.
    missing: std::collections::BTreeSet<Arc<str>>,
    player: crate::music::Player,
    /// The last frame's song chances, which a song drawn when a track ends
    /// in a callback uses, as the game's next update would.
    chances: Option<Arc<crate::music::Chances>>,
    music_dynamics: Option<Dynamics>,
    /// Mixed-audio seconds at each sound's last use.
    last_used: Vec<f64>,
    /// In creation order, as the game updates them.
    instances: Vec<Instance>,
    voices: Vec<Option<Voice>>,
    next_voice: u64,
    /// Counted instances per effect, against its `max_audible`.
    audible: Vec<i32>,
    listener: Listener,
    mix: Mix,
    hour: Option<i32>,
    /// The last frame's emitter count, for the log.
    emitters: usize,
    random: Random,
    categories: Vec<Option<Dynamics>>,
    master: Option<Dynamics>,
    pub notes: Notes,
    pub log: Option<Vec<Started>>,
    time: f64,
    /// Per category, then music: a callback's buffer and whether anything
    /// mixed into it.
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

    /// Replaces the content; every instance and voice of the old one ends.
    pub fn set_content(&mut self, content: Option<Arc<Content>>) {
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
        // The history numbers the old content's songs; only the same song list keeps it.
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

    /// Sounds a started or prefetched instance needs, each asked for once,
    /// and the music track the player drew.
    pub fn take_wanted(&mut self) -> Vec<Load> {
        let mut wanted = std::mem::take(&mut self.wanted);
        wanted.extend(self.player.take_wanted().map(Load::Track));
        wanted
    }

    /// Draws the next song when none plays.
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

    /// How many songs the player has drawn.
    pub fn music_draws(&self) -> u64 {
        self.player.draws()
    }

    /// Whether the player drew a track it is still waiting to stream.
    pub fn music_waiting(&self) -> bool {
        self.player.waiting()
    }

    /// Waits up to `limit` for the playing track's first decoded samples, as
    /// the game's player opens its next track before mixing on.
    pub fn prime_music(&mut self, limit: std::time::Duration) {
        self.player.prime(limit);
    }

    /// A music track's stream, or `None` when it is unreadable.
    pub fn provide_track(&mut self, track: usize, stream: Option<crate::music::TrackStream>) {
        self.player.provide(track, stream);
    }

    /// The music track playing or loading.
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

    /// Whether `sound`'s samples are decoded and held.
    pub fn loaded(&self, sound: usize) -> bool {
        self.samples.get(sound).is_some_and(Option::is_some)
    }

    /// Drops decoded samples no voice holds and none used for `retain`
    /// seconds of mixed audio; a later start loads them again.
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

    /// Ends every sound and the music, as an ownership change does.
    pub fn silence(&mut self) {
        self.player.stop();
        self.silence_effects();
    }

    /// Ends every sound effect at once, as a seek does; music plays on.
    fn silence_effects(&mut self) {
        let effects = self.audible.len();
        self.instances.clear();
        self.voices.iter_mut().for_each(|voice| *voice = None);
        self.audible = vec![0; effects];
    }

    /// The last frame's hour and emitters, the instances and the sounding voices.
    pub fn census(&self) -> (Option<i32>, usize, usize, usize) {
        let voices = self
            .voices
            .iter()
            .flatten()
            .filter(|voice| voice.state != State::Stopped)
            .count();
        (self.hour, self.emitters, self.instances.len(), voices)
    }

    /// Whether anything still sounds or waits to.
    pub fn active(&self) -> bool {
        self.voices
            .iter()
            .flatten()
            .any(|voice| voice.state != State::Stopped)
    }

    /// Applies volumes and mute between frames; the next callback uses them.
    pub fn set_mix(&mut self, mix: Mix) {
        self.mix = mix;
    }

    /// Applies one presented frame's emitters, listener and volumes.
    pub fn frame(&mut self, frame: &Frame) {
        self.listener = frame.listener;
        self.mix = frame.mix;
        self.hour = frame.hour;
        self.emitters = frame.emitters.len();
        let Some(content) = self.content.clone() else {
            return;
        };
        if frame.reset {
            // A discontinuity ends what was sounding; loops of present events restart.
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
                if instance.effect == effect {
                    present[at] = true;
                    // The event's age falling back means it fired again.
                    if emitter.age + 1e-3 < instance.age && !definition.looping {
                        instance.pending = true;
                    }
                    instance.age = emitter.age;
                    instance.position = emitter.position;
                    instance.group = emitter.group;
                    instance.restart = emitter.restart;
                    continue;
                }
                replaced.push(at);
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
        // Departed instances leave in order; the rest keep theirs.
        let mut kept = Vec::with_capacity(self.instances.len() + created.len());
        for (instance, present) in std::mem::take(&mut self.instances).into_iter().zip(present) {
            if present {
                kept.push(instance);
            } else {
                self.retire(instance, &content);
            }
        }
        kept.extend(created);
        self.instances = kept;
        self.update(&content, frame.elapsed.max(0.0));
    }

    /// A departed event: its one-shot voices finish; its loops stop with
    /// their fade.
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

    /// The per-frame update: voice fades, then each instance in order.
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
            // `silent` overflow: an instance out of earshot gives up its
            // place; one in earshot takes a free place back.
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

    /// Fills `instance`'s free slots, as the game's start does. Out of range
    /// a fired start still draws but creates no voice.
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
            |range: installed_content_sound::sound::Range| range.first != 0.0 || range.second != 0.0;
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
        let width = |range: installed_content_sound::sound::Range| range.second - range.first;
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
                // A sample that failed to load creates no voice.
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
                // A fading-in voice rises from silence.
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

    /// A category's volume, by the game's volume settings and the camera.
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

    /// Mixes interleaved 16-bit stereo at [`RATE`] into `out`, one
    /// [`CALLBACK_FRAMES`] callback at a time.
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
        // Music, in the mixer's own slots, mixes first.
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
            // A track that ended draws the next song at once; it mixes from
            // the next callback, as the game's next update starts it.
            if self.player.playing().is_none() {
                self.draw_song(&content);
            }
        }
        // Effect voices with a gain or a gain to ramp down from, the highest
        // priority first; ties go to the later slot.
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
            // A category counts as mixed into by every voice within the 32,
            // whatever its gain, and by none past them.
            let buffer = if compressed && counted {
                let category = voice.category.expect("checked above");
                self.buses[category].1 = true;
                &mut self.buses[category].0
            } else if compressed {
                // Past the 32 the voice only advances: nothing reaches a buffer.
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

/// A voice not mixed this callback advances a callback's frames at its
/// natural rate, wrapping when it loops and stopping at its end otherwise.
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

/// Mixes one callback of `voice` into `buffer`, ramping each side's gain
/// step from `old` to `new` by frame. At its natural rate the samples add
/// frame for frame; otherwise the mixer steps a float position from the
/// cursor at the voice's rate, interpolating linearly, and keeps only the
/// whole frames passed when the callback ends.
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
            // Frames to the sample's end at this rate, in the callback's room.
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
