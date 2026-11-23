use super::*;
use crate::{Emitter, Frame, Group, Listener, Mix};
use content_reader_layers::test_support::Stack;
use content_reader_sound::wav::{decode, encode_pcm16};

fn tone(frames: usize) -> Vec<u8> {
    encode_pcm16(RATE, 2, &vec![16_384; frames * 2])
}

struct Fixture {
    _stack: Stack,
    layers: content_reader_layers::ResolvedLayers,
    engine: Engine,
}

impl Fixture {
    fn new(name: &str, assets: &str) -> Self {
        let stack = Stack::new(name);
        let base = stack.base();
        stack.write(&base, "sound/test.asset", assets);
        std::fs::write(base.join("sound/tone.wav"), tone(RATE as usize)).unwrap();
        std::fs::write(base.join("sound/blip.wav"), tone(100)).unwrap();
        let layers = stack.layers(&[]);
        let catalog = content_reader_sound::sound::Catalog::read(&layers).unwrap();
        assert!(catalog.notes.is_empty(), "{:?}", catalog.notes.lines());
        let defines = content_reader_sound::sound::Defines {
            height_category: "Air".into(),
            height_min_altitude: 100.0,
            height_max_altitude: 2000.0,
            height_min_volume: 0.2,
            height_max_volume: 1.0,
            voice_category: "Voices".into(),
            voice_cool_down: 2.8,
        };
        let mut engine = Engine::new(7);
        engine.set_content(Some(Arc::new(Content::new(catalog, Some(defines)))));
        Self {
            _stack: stack,
            layers,
            engine,
        }
    }

    fn frame(&mut self, frame: Frame) {
        self.engine.frame(&frame);
        self.load();
    }

    fn load(&mut self) {
        for load in self.engine.take_wanted() {
            let Load::Sound(sound) = load else {
                continue;
            };
            let path = self.engine.content().unwrap().catalog.sounds[sound]
                .path
                .clone();
            let pcm = decode(&self.layers.read_uncovered(&path).unwrap()).unwrap();
            self.engine
                .provide(sound, Some(Arc::new(Samples::from_pcm(&pcm))));
        }
    }

    fn render(&mut self, callbacks: usize) -> Vec<i16> {
        let mut out = vec![0i16; callbacks * CALLBACK_FRAMES * 2];
        self.engine.render(&mut out);
        out
    }

    fn voices(&self) -> usize {
        self.engine.census().3
    }
}

fn mix() -> Mix {
    Mix {
        master: 1.0,
        dev_master: 1.0,
        ambient: 1.0,
        units: 1.0,
        combat: 1.0,
        interface: 1.0,
        voice: 1.0,
        music: 1.0,
        muted: false,
    }
}

fn listener() -> Listener {
    Listener {
        position: [0.0, 0.0, 0.0],
        right: [1.0, 0.0, 0.0],
        altitude: 100.0,
    }
}

fn emitter(key: u64, effect: &str, position: [f32; 3], age: f32) -> Emitter {
    Emitter {
        key,
        effect: effect.into(),
        position,
        age,
        group: Group::Units,
        restart: false,
    }
}

fn frame(emitters: Vec<Emitter>) -> Frame {
    Frame {
        listener: listener(),
        emitters,
        mix: mix(),
        reset: false,
        elapsed: 1.0 / 60.0,
        hour: None,
        music: None,
    }
}

const ASSETS: &str = r#"
    sound = { name = tone file = "tone.wav" volume = 0.5 }
    sound = { name = blip file = "blip.wav" }
    sound = { name = loud_tone file = "tone.wav" priority = 2 }
    falloff = { name = near min_distance = 10 max_distance = 30 type = linear }
    soundeffect = { name = shot sounds = { sound = tone } volume = 0.8 }
    soundeffect = { name = near_shot sounds = { sound = tone } falloff = near is3d = yes }
    soundeffect = { name = hum sounds = { sound = tone } loop = yes }
    soundeffect = { name = limited sounds = { sound = tone } max_audible = 2 max_audible_behaviour = fail }
    soundeffect = { name = wind sounds = { sound = tone } }
    soundeffect = { name = loud sounds = { sound = tone } volume = 3 }
    soundeffect = { name = quiet sounds = { sound = tone } volume = 0.01 }
    soundeffect = { name = urgent sounds = { sound = loud_tone } volume = 0.01 }
    soundeffect = { name = chord sounds = { sound = tone sound = blip } polyphony = 2 }
    soundeffect = { name = wide sounds = { sound = tone sound = blip } polyphony = 3 }
    soundeffect = { name = rise sounds = { sound = tone } fade_in = 1 }
    soundeffect = { name = late sounds = { sound = tone } fade_in = 1 delay_random_offset = { 0 10 } }
    soundeffect = { name = slow sounds = { sound = tone } playbackrate = 70 }
    soundeffect = { name = far_shot sounds = { sound = tone } falloff = near volume_random_offset = { 0 0.5 } }
    category = { name = Air soundeffects = { wind } }
"#;

fn level(volume: f32) -> i16 {
    scale(16_384, step(volume)) as i16
}

#[test]
fn a_fired_event_plays_its_sound_at_the_integer_chain_gain() {
    let mut fixture = Fixture::new("audio_fired", ASSETS);
    fixture.frame(frame(vec![emitter(1, "shot", [0.0; 3], 0.0)]));
    let out = fixture.render(1);
    assert_eq!(step(0.8 * 0.5), 6_554);
    assert!(out.iter().all(|value| *value == 6_554), "{:?}", &out[..4]);
    fixture.frame(frame(vec![emitter(2, "absent", [0.0; 3], 0.0)]));
    fixture.frame(frame(vec![
        emitter(2, "absent", [0.0; 3], 0.1),
        emitter(3, "absent", [0.0; 3], 0.0),
    ]));
    assert_eq!(
        fixture.engine.notes.lines(),
        ["Entity event names missing sound effect absent"]
    );
}

#[test]
fn volumes_clamp_and_a_mute_ramps_down_across_one_callback() {
    let mut fixture = Fixture::new("audio_clamp", ASSETS);
    fixture.frame(frame(vec![emitter(1, "loud", [0.0; 3], 0.0)]));
    assert!(fixture.render(1).iter().all(|value| *value == 8_192));
    let mut muted = frame(vec![emitter(1, "loud", [0.0; 3], 0.01)]);
    muted.mix.muted = true;
    fixture.frame(muted);
    let out = fixture.render(2);
    for frame in [0, 1, 511, 1_023] {
        let step = 8_192 + (frame as i32 * -8_192) / 1_024;
        assert_eq!(i32::from(out[frame * 2]), scale(16_384, step), "{frame}");
    }
    assert!(out[CALLBACK_FRAMES * 2..].iter().all(|value| *value == 0));
}

#[test]
fn an_event_seen_long_after_it_fired_keeps_only_its_loop() {
    let mut fixture = Fixture::new("audio_old", ASSETS);
    fixture.frame(frame(vec![
        emitter(1, "shot", [0.0; 3], 5.0),
        emitter(2, "hum", [0.0; 3], 5.0),
    ]));
    let counts: Vec<_> = fixture
        .engine
        .instances
        .iter()
        .map(|instance| instance.count)
        .collect();
    assert_eq!(counts, [0, 1]);
}

#[test]
fn a_reset_ends_voices_and_does_not_count_present_events_as_fired() {
    let mut fixture = Fixture::new("audio_reset", ASSETS);
    fixture.frame(frame(vec![emitter(1, "shot", [0.0; 3], 0.0)]));
    assert!(fixture.engine.active());
    let mut reset = frame(vec![emitter(2, "shot", [0.0; 3], 0.0)]);
    reset.reset = true;
    fixture.frame(reset);
    assert!(!fixture.engine.active());
    assert!(fixture.render(1).iter().all(|value| *value == 0));
    fixture.frame(frame(vec![emitter(2, "shot", [0.0; 3], 0.0)]));
    assert!(!fixture.engine.active(), "an equal age is the same firing");
    fixture.frame(frame(vec![emitter(2, "shot", [0.0; 3], 0.5)]));
    fixture.frame(frame(vec![emitter(2, "shot", [0.0; 3], 0.0)]));
    assert!(fixture.engine.active(), "a falling age fires again");
}

#[test]
fn falloff_attenuates_pans_and_stops_beyond_twice_the_maximum() {
    let mut fixture = Fixture::new("audio_falloff", ASSETS);
    fixture.frame(frame(vec![emitter(1, "near_shot", [20.0, 0.0, 0.0], 0.0)]));
    let out = fixture.render(1);
    assert_eq!(out[0], 0);
    assert_eq!(out[1], level(0.5 * 0.5));
    fixture.frame(frame(vec![emitter(1, "near_shot", [61.0, 0.0, 0.0], 0.01)]));
    fixture.render(1);
    assert!(!fixture.engine.active());
    let near = content_reader_sound::sound::Falloff {
        min_distance: 0.0,
        max_distance: 100.0,
        ..Default::default()
    };
    let (gain, beyond) = falloff_gain(&near, [0.0; 3], [0.0, 100.0, 0.0]);
    assert!(gain.abs() < 1e-6 && !beyond);
    let (gain, _) = falloff_gain(&near, [0.0; 3], [50.0, 0.0, 0.0]);
    assert!((gain - -(0.55f32).log10()).abs() < 1e-6);
    let flat = content_reader_sound::sound::Falloff {
        height_scale: 0.5,
        ..near
    };
    assert!(!falloff_gain(&flat, [0.0; 3], [0.0, 350.0, 0.0]).1);
    assert!(falloff_gain(&flat, [0.0; 3], [0.0, 450.0, 0.0]).1);
}

#[test]
fn a_fired_start_out_of_range_draws_but_creates_no_voice() {
    let mut fixture = Fixture::new("audio_out_of_range", ASSETS);
    fixture.frame(frame(vec![emitter(1, "far_shot", [100.0, 0.0, 0.0], 0.0)]));
    assert_eq!(fixture.voices(), 0);
    assert_eq!(fixture.engine.random.count, 1);
    fixture.frame(frame(vec![emitter(2, "far_shot", [0.0; 3], 0.0)]));
    assert_eq!(fixture.voices(), 1);
    assert_eq!(fixture.engine.random.count, 3);
}

#[test]
fn fail_overflow_creates_no_voice_past_max_audible() {
    let mut fixture = Fixture::new("audio_fail", ASSETS);
    fixture.frame(frame(
        (1..=3)
            .map(|key| emitter(key, "limited", [0.0; 3], 0.0))
            .collect(),
    ));
    assert_eq!(fixture.voices(), 2);
    assert_eq!(fixture.engine.audible[fixture.effect("limited")], 2);
}

#[test]
fn the_height_category_follows_altitude_and_the_ambient_volume() {
    let mut fixture = Fixture::new("audio_height", ASSETS);
    let mut high = frame(vec![emitter(1, "wind", [0.0; 3], 0.0)]);
    high.listener.altitude = 1050.0;
    high.mix.ambient = 0.5;
    fixture.frame(high);
    let out = fixture.render(1);
    let defines = fixture.engine.content().unwrap().defines.clone().unwrap();
    let category = defines.height_volume(1050.0) * 0.5;
    assert_eq!(
        out[0],
        scale(16_384, step(category * 1.0 * 1.0 * 0.5)) as i16
    );
}

#[test]
fn a_departed_loop_stops_while_a_departed_one_shot_finishes() {
    let mut fixture = Fixture::new("audio_departed", ASSETS);
    fixture.frame(frame(vec![
        emitter(1, "hum", [0.0; 3], 0.0),
        emitter(2, "shot", [0.0; 3], 0.0),
    ]));
    fixture.frame(frame(Vec::new()));
    assert_eq!(fixture.voices(), 1);
    assert!(fixture.render(1)[0] > 0);
}

#[test]
fn polyphony_fills_a_slot_per_voice_with_different_sounds() {
    let mut fixture = Fixture::new("audio_polyphony", ASSETS);
    fixture.frame(frame(vec![
        emitter(1, "chord", [0.0; 3], 0.0),
        emitter(2, "wide", [0.0; 3], 0.0),
    ]));
    let instances = &fixture.engine.instances;
    assert_eq!(instances[0].slots.len(), 2);
    assert_eq!(instances[0].count, 2);
    let mut chosen: Vec<_> = instances[0]
        .slots
        .iter()
        .map(|slot| slot.choice.unwrap())
        .collect();
    chosen.sort();
    assert_eq!(chosen, [0, 1]);
    assert_eq!(instances[1].slots.len(), 1);
    assert_eq!(fixture.voices(), 3);
}

#[test]
fn only_the_32_highest_priority_voices_mix() {
    let mut fixture = Fixture::new("audio_priority", ASSETS);
    let mut emitters: Vec<_> = (1..=40)
        .map(|key| emitter(key, "quiet", [0.0; 3], 0.0))
        .collect();
    fixture.frame(frame(emitters.clone()));
    let quiet = i32::from(level(0.01 * 0.5));
    assert_eq!(fixture.voices(), 40);
    assert_eq!(i32::from(fixture.render(1)[0]), 32 * quiet);
    emitters.push(emitter(41, "urgent", [0.0; 3], 0.0));
    let mut next = frame(emitters);
    for emitter in &mut next.emitters[..40] {
        emitter.age = 0.05;
    }
    fixture.frame(next);
    let urgent = i32::from(level(0.01));
    assert_eq!(i32::from(fixture.render(1)[0]), 31 * quiet + urgent);
}

#[test]
fn the_mixer_has_248_effect_voices() {
    let mut fixture = Fixture::new("audio_voice_slots", ASSETS);
    fixture.frame(frame(
        (1..=250)
            .map(|key| emitter(key, "quiet", [0.0; 3], 0.0))
            .collect(),
    ));
    assert_eq!(fixture.voices(), VOICE_SLOTS - RESERVED_VOICES);
    let without: Vec<_> = fixture
        .engine
        .instances
        .iter()
        .filter(|instance| instance.count == 0)
        .map(|instance| instance.key)
        .collect();
    assert_eq!(without, [249, 250]);
}

#[test]
fn fades_step_the_volume_by_frame_and_a_delayed_voice_skips_its_fade_in() {
    let mut fixture = Fixture::new("audio_fades", ASSETS);
    let mut first = frame(vec![
        emitter(1, "rise", [0.0; 3], 0.0),
        emitter(2, "late", [0.0; 3], 0.0),
    ]);
    first.elapsed = 0.25;
    fixture.frame(first.clone());
    let volume = |fixture: &Fixture, key: u64| {
        let instance = fixture
            .engine
            .instances
            .iter()
            .find(|instance| instance.key == key)
            .unwrap();
        let (slot, _) = instance.slots[0].voice.unwrap();
        let voice = fixture.engine.voices[slot].as_ref().unwrap();
        (voice.volume, voice.state)
    };
    assert_eq!(volume(&fixture, 1), (0.0, State::FadingIn));
    for emitter in &mut first.emitters {
        emitter.age = 0.25;
    }
    fixture.frame(first.clone());
    assert_eq!(volume(&fixture, 1), (0.25, State::FadingIn));
    for emitter in &mut first.emitters {
        emitter.age = 0.5;
    }
    fixture.frame(first.clone());
    assert_eq!(volume(&fixture, 1), (0.5, State::Playing));
    let mut random = Random::new(7);
    random.word();
    random.word();
    let delay = random.fraction() * 10.0;
    let mut waited = 0.75f32;
    let mut age = 0.5;
    while waited < delay {
        assert_eq!(
            volume(&fixture, 2),
            (0.5, State::Delayed),
            "{waited} of {delay}"
        );
        age += 0.25;
        for emitter in &mut first.emitters {
            emitter.age = age;
        }
        fixture.frame(first.clone());
        waited += 0.25;
    }
    assert_eq!(volume(&fixture, 2), (0.5, State::Playing));
}

#[test]
fn a_voice_at_another_rate_loses_its_fraction_each_callback() {
    let mut fixture = Fixture::new("audio_rate", ASSETS);
    fixture.frame(frame(vec![emitter(1, "slow", [0.0; 3], 0.0)]));
    fixture.render(1);
    let rate = 0.7f32;
    let mut position = 0.0f32;
    for _ in 0..CALLBACK_FRAMES {
        position += rate;
    }
    let whole = position as usize;
    assert!(position.fract() > 0.0);
    let (slot, _) = fixture.engine.instances[0].slots[0].voice.unwrap();
    assert_eq!(fixture.engine.voices[slot].as_ref().unwrap().cursor, whole);
    fixture.render(1);
    assert_eq!(
        fixture.engine.voices[slot].as_ref().unwrap().cursor,
        whole * 2
    );
}

#[test]
fn the_compressor_reduces_levels_above_its_threshold() {
    let mut dynamics = Dynamics::new(Compressor {
        threshold: -12.0,
        ratio: 4.0,
        ..Compressor::default()
    });
    let mut loud = vec![16_384i32; 64];
    dynamics.process(&mut loud);
    let level = 20.0 * 0.5f32.log10();
    let expected = 16_384.0 * 10f32.powf((-12.0 - level) * 0.75 / 20.0);
    assert!((loud[63] as f32 - expected).abs() < 2.0, "{}", loud[63]);
    assert!(loud[63] < 10_000);
    let mut quiet = vec![3_000i32; 64];
    dynamics.process(&mut quiet);
    assert_eq!(quiet[0], 3_000);
}

#[test]
fn a_category_compressor_holds_while_nothing_mixes_into_it() {
    let assets = r#"
        sound = { name = tone file = "blip.wav" }
        soundeffect = { name = beep sounds = { sound = tone } }
        category = { name = Beeps soundeffects = { beep }
            compressor = { threshold = -40 ratio = 2 attacktime = 0.01 releasetime = 1 } }
    "#;
    let mut fixture = Fixture::new("audio_category_hold", assets);
    fixture.frame(frame(vec![emitter(1, "beep", [0.0; 3], 0.0)]));
    fixture.render(1);
    let envelope = fixture.engine.categories[0].as_ref().unwrap().envelope;
    assert!(envelope[0] > 0.0);
    fixture.render(4);
    assert_eq!(
        fixture.engine.categories[0].as_ref().unwrap().envelope,
        envelope
    );
}

#[test]
fn the_weighted_pick_walks_positive_weights_and_falls_back() {
    let half = 1u32 << 30;
    assert_eq!(weighted(&[0, 3, -5, 1], half, None), Some(1));
    assert_eq!(weighted(&[0, 1, -5, 3], half, None), Some(3));
    assert_eq!(weighted(&[0, -2], half, Some(1)), Some(1));
    assert_eq!(uniform(half, 4), 2);
}

#[test]
fn music_mixes_first_through_the_master_compressor_and_survives_a_reset() {
    let assets = r#"
        sound = { name = tone file = "tone.wav" }
        master_compressor = { postgain = -6 }
    "#;
    let mut fixture = Fixture::new("audio_music", assets);
    let content = fixture.engine.content().unwrap();
    let mut music = content_reader_sound::music::Music::default();
    music.tracks.push(content_reader_sound::music::Track {
        name: "march".into(),
        path: "music/march.ogg".into(),
        volume: 0.5,
        source: String::new(),
    });
    music.songs.push(content_reader_sound::music::Song {
        track: 0,
        station: "base_music".into(),
        chance: Vec::new(),
        source: String::new(),
    });
    let content = Content::new(content.catalog.clone(), content.defines.clone()).with_music(
        music,
        Some(vec!["base_music".into()]),
        10,
    );
    assert!(content.enabled(0));
    fixture.engine.set_content(Some(Arc::new(content)));
    let mut first = frame(Vec::new());
    first.mix.music = 0.5;
    first.music = Some(Arc::new(crate::Chances {
        weights: vec![Some(1.0)],
        refusals: Vec::new(),
    }));
    fixture.engine.frame(&first);
    assert_eq!(fixture.engine.take_wanted(), [Load::Track(0)]);
    let (sender, stream) = crate::music::track_channel();
    sender.send(Ok(vec![16_384; 4_000])).unwrap();
    fixture.engine.provide_track(0, Some(stream));
    let out = fixture.render(1);
    let mixed = scale(16_384, step(1.0 * 0.5 * 0.5));
    let postgain = 10f32.powf(-6.0 / 20.0);
    assert_eq!(i32::from(out[0]), truncate(mixed as f32 * postgain));
    let mut reset = first.clone();
    reset.reset = true;
    fixture.engine.frame(&reset);
    assert_eq!(
        fixture.engine.music_playing(),
        Some(0),
        "a seek keeps the music"
    );
    fixture.engine.silence();
    assert_eq!(
        fixture.engine.music_playing(),
        None,
        "losing the audio stops it"
    );
}

#[test]
fn a_sample_that_fails_to_load_refuses_its_starts() {
    let assets = r#"
        sound = { name = broken file = "broken.wav" }
        soundeffect = { name = crackle sounds = { sound = broken } max_audible = 1 max_audible_behaviour = fail }
    "#;
    let mut fixture = Fixture::new("audio_failed_sample", assets);
    fixture
        .engine
        .frame(&frame(vec![emitter(1, "crackle", [0.0; 3], 0.0)]));
    assert!(
        fixture.engine.active(),
        "the first voice waits for its samples"
    );
    assert_eq!(fixture.engine.take_wanted(), [Load::Sound(0)]);
    fixture.engine.provide(0, None);
    assert!(!fixture.engine.active());
    fixture.render(1);
    for key in 2..=4 {
        fixture
            .engine
            .frame(&frame(vec![emitter(key, "crackle", [0.0; 3], 0.0)]));
    }
    assert!(!fixture.engine.active());
    assert!(
        fixture.engine.take_wanted().is_empty(),
        "no reload is asked for"
    );
    assert_eq!(
        fixture.engine.notes.lines(),
        ["Sound broken of effect crackle failed to load; start refused (x3)"]
    );
    assert_eq!(fixture.engine.audible, [0]);
    assert_eq!(fixture.voices(), 0);
}

#[test]
fn a_silent_place_goes_to_an_instance_that_can_still_play() {
    let assets = r#"
        sound = { name = tone file = "tone.wav" }
        soundeffect = { name = one sounds = { sound = tone } max_audible = 1 }
    "#;
    let mut fixture = Fixture::new("audio_silent_slot", assets);
    fixture.frame(frame(vec![emitter(1, "one", [0.0; 3], 0.0)]));
    fixture.render(22);
    fixture.frame(frame(vec![
        emitter(1, "one", [0.0; 3], 0.5),
        emitter(2, "one", [0.0; 3], 0.0),
    ]));
    assert!(fixture.engine.instances[1].over);
    fixture.render(25);
    fixture.frame(frame(vec![
        emitter(1, "one", [0.0; 3], 1.1),
        emitter(2, "one", [0.0; 3], 0.6),
    ]));
    fixture.render(1);
    let out = fixture.render(1);
    assert_eq!(out[0], level(1.0), "the sounding instance is heard");
    assert_eq!(fixture.engine.audible, [1]);
}

#[test]
fn a_restarting_emitter_plays_again_when_its_sound_ends() {
    let mut fixture = Fixture::new("audio_restart", ASSETS);
    let mut battle = emitter(1, "shot", [0.0; 3], 0.0);
    battle.restart = true;
    let shot = emitter(2, "shot", [0.0; 3], 0.0);
    fixture.frame(frame(vec![battle.clone(), shot.clone()]));
    fixture.render(44);
    fixture.frame(frame(vec![battle, shot]));
    let counts: Vec<_> = fixture
        .engine
        .instances
        .iter()
        .map(|instance| instance.count)
        .collect();
    assert_eq!(counts, [1, 0]);
}

impl Fixture {
    fn effect(&self, name: &str) -> usize {
        self.engine.content().unwrap().catalog.effect(name).unwrap()
    }
}

#[test]
fn a_draw_without_positive_weights_picks_uniformly_over_every_sound() {
    let assets = r#"
        sound = { name = tone file = "tone.wav" }
        sound = { name = blip file = "blip.wav" }
        soundeffect = { name = pair polyphony = 2 sounds = {
            weighted_sound = { sound = tone weight = 1 }
            weighted_sound = { sound = blip weight = 0 } } }
    "#;
    let mut fixture = Fixture::new("audio_uniform_fallback", assets);
    fixture.frame(frame(vec![emitter(1, "pair", [0.0; 3], 0.0)]));
    let mut random = Random::new(7);
    let first = random.word();
    let second = random.word();
    assert_eq!(weighted(&[1, 0], first, None), Some(0));
    let chosen: Vec<_> = fixture.engine.instances[0]
        .slots
        .iter()
        .map(|slot| slot.choice.unwrap())
        .collect();
    assert_eq!(chosen, [0, uniform(second, 2)]);
}

#[test]
fn a_voice_past_the_32_leaves_its_category_compressor_holding() {
    let assets = r#"
        sound = { name = tone file = "tone.wav" }
        sound = { name = high file = "tone.wav" priority = 2 }
        soundeffect = { name = beep sounds = { sound = tone } volume = 0.01 }
        soundeffect = { name = top sounds = { sound = high } volume = 0.01 }
        category = { name = Beeps soundeffects = { beep }
            compressor = { threshold = -60 ratio = 2 attacktime = 0.01 releasetime = 1 } }
    "#;
    let mut fixture = Fixture::new("audio_category_past_32", assets);
    fixture.frame(frame(vec![emitter(1, "beep", [0.0; 3], 0.0)]));
    fixture.render(1);
    let envelope = fixture.engine.categories[0].as_ref().unwrap().envelope;
    assert!(envelope[0] > 0.0);
    let mut emitters = vec![emitter(1, "beep", [0.0; 3], 0.05)];
    emitters.extend((2..=33).map(|key| emitter(key, "top", [0.0; 3], 0.0)));
    fixture.frame(frame(emitters));
    fixture.render(2);
    assert_eq!(
        fixture.engine.categories[0].as_ref().unwrap().envelope,
        envelope
    );
}

#[test]
fn a_track_that_ends_draws_the_next_song_in_its_callback() {
    let mut fixture = Fixture::new("audio_music_handoff", ASSETS);
    let content = fixture.engine.content().unwrap();
    let mut music = content_reader_sound::music::Music::default();
    for (index, name) in ["march", "hymn"].into_iter().enumerate() {
        music.tracks.push(content_reader_sound::music::Track {
            name: name.into(),
            path: format!("music/{name}.ogg"),
            volume: 1.0,
            source: String::new(),
        });
        music.songs.push(content_reader_sound::music::Song {
            track: index,
            station: "base_music".into(),
            chance: Vec::new(),
            source: String::new(),
        });
    }
    let content =
        Content::new(content.catalog.clone(), content.defines.clone()).with_music(music, None, 2);
    fixture.engine.set_content(Some(Arc::new(content)));
    let mut first = frame(Vec::new());
    first.music = Some(Arc::new(crate::Chances {
        weights: vec![Some(1.0), Some(1.0)],
        refusals: Vec::new(),
    }));
    fixture.engine.frame(&first);
    let Load::Track(track) = fixture.engine.take_wanted()[0] else {
        panic!("a track is asked for");
    };
    assert!(fixture.engine.music_waiting());
    let (sender, stream) = crate::music::track_channel();
    sender.send(Ok(vec![1_000; 200])).unwrap();
    drop(sender);
    fixture.engine.provide_track(track, Some(stream));
    fixture
        .engine
        .prime_music(std::time::Duration::from_secs(1));
    assert!(!fixture.engine.music_waiting(), "the first samples are in");
    fixture.render(1);
    assert_eq!(fixture.engine.music_draws(), 2);
    assert!(fixture.engine.music_playing().is_some());
    assert!(matches!(fixture.engine.take_wanted()[..], [Load::Track(_)]));
}
