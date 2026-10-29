use super::*;
use crate::{Emitter, Group, Listener};
use installed_content_core::test_support::Stack;

const ASSETS: &str = r#"
    sound = { name = tone file = "tone.wav" }
    sound = { name = broken file = "broken.wav" }
    soundeffect = { name = shot sounds = { sound = tone } }
    soundeffect = { name = crackle sounds = { sound = broken } }
"#;

/// A content stack with one playable tone and one sound whose file is garbage.
fn source(name: &str) -> (Stack, Arc<Source>) {
    let stack = Stack::new(name);
    let base = stack.base();
    stack.write(&base, "sound/test.asset", ASSETS);
    std::fs::write(
        base.join("sound/tone.wav"),
        installed_content_sound::wav::encode_pcm16(RATE, 1, &vec![8_192; RATE as usize / 4]),
    )
    .unwrap();
    std::fs::write(base.join("sound/broken.wav"), b"RIFF\x04\0\0\0WAVEjunk").unwrap();
    let layers = stack.layers(&[]);
    let catalog = installed_content_sound::sound::Catalog::read(&layers).unwrap();
    let source = Arc::new(Source {
        content: Arc::new(Content::new(catalog, None)),
        layers,
    });
    (stack, source)
}

fn frame(effect: &str, key: u64) -> Frame {
    Frame {
        listener: Listener {
            position: [0.0; 3],
            right: [1.0, 0.0, 0.0],
            altitude: 100.0,
        },
        emitters: vec![Emitter {
            key,
            effect: effect.into(),
            position: [0.0; 3],
            age: 0.0,
            group: Group::Units,
            restart: false,
        }],
        mix: Mix::default(),
        reset: false,
        elapsed: 0.0,
        hour: None,
        music: None,
    }
}

/// Steps `control` until `done` holds, for at most five seconds.
fn step_until(control: &mut Control, done: impl Fn(&Control) -> bool) {
    let start = Instant::now();
    while !done(control) {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        control.step();
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn one_owner_sounds_and_a_release_or_drop_frees_the_audio() {
    let system = System::spawn(true);
    let mut a = system.owner();
    let mut b = system.owner();
    a.claim();
    assert!(a.owns() && !b.owns());
    b.claim();
    assert!(!a.owns() && b.owns(), "the last claim wins");
    a.release();
    assert!(b.owns(), "a view that does not own cannot release");
    b.release();
    assert!(!a.owns() && !b.owns());
    a.claim();
    drop(a);
    assert_eq!(
        system.owner.load(Ordering::Acquire),
        0,
        "a dropped owner releases"
    );
    assert_eq!(
        system.status().device,
        None,
        "a silent system opens no device"
    );
}

/// Without a device, a claimed view's frames still start voices, which load,
/// play by the clock and finish; a release silences everything at once.
#[test]
fn the_silent_path_plays_by_the_clock_and_a_release_silences() {
    let (_stack, source) = source("audio_system_silent");
    let status = Arc::new(Mutex::new(Status::default()));
    let mut control = Control::new(Arc::clone(&status), None, true);
    control.apply(Command::Claim(1));
    control.apply(Command::Frame(1, Arc::clone(&source), frame("shot", 1)));
    assert!(control.engine.active());
    step_until(&mut control, |control| !control.engine.active());
    assert!(control.output.is_none());
    assert_eq!(status.lock().unwrap().device, None);
    // A start, then a release: nothing sounds afterwards.
    control.apply(Command::Frame(1, Arc::clone(&source), frame("shot", 2)));
    assert!(control.engine.active());
    control.apply(Command::Release(1));
    assert!(!control.engine.active());
    // Frames of a view that no longer owns are dropped.
    control.apply(Command::Frame(1, Arc::clone(&source), frame("shot", 3)));
    assert!(!control.engine.active());
}

/// A mute sent between frames applies to the next mixed block.
#[test]
fn a_mix_applies_without_a_frame() {
    let (_stack, source) = source("audio_system_mix");
    let mut control = Control::new(Arc::new(Mutex::new(Status::default())), None, true);
    control.apply(Command::Claim(4));
    let mut loud = frame("shot", 1);
    loud.mix.dev_master = 1.0;
    control.apply(Command::Frame(4, Arc::clone(&source), loud));
    step_until(&mut control, |control| {
        control.engine.census().3 == 1 && control.engine.loaded(0)
    });
    let mut muted = Mix {
        muted: true,
        ..Mix::default()
    };
    control.apply(Command::Mix(9, muted));
    let mut out = vec![0i16; 128];
    control.engine.render(&mut out);
    assert!(
        out.iter().any(|value| *value != 0),
        "another view's mix is ignored"
    );
    muted.dev_master = 1.0;
    control.apply(Command::Mix(4, muted));
    control.engine.render(&mut out);
    control.engine.render(&mut out);
    assert!(out.iter().all(|value| *value == 0));
}

/// A file the decoder refuses is named, its voices finish, and the loader
/// keeps serving later requests.
#[test]
fn an_unreadable_sample_is_refused_and_the_loader_keeps_running() {
    let (_stack, source) = source("audio_system_unreadable");
    let mut control = Control::new(Arc::new(Mutex::new(Status::default())), None, true);
    control.apply(Command::Claim(1));
    control.apply(Command::Frame(1, Arc::clone(&source), frame("crackle", 1)));
    step_until(&mut control, |control| !control.engine.active());
    assert!(
        control
            .engine
            .notes
            .lines()
            .iter()
            .any(|line| line.starts_with("Unreadable sample sound/broken.wav")),
        "{:?}",
        control.engine.notes.lines()
    );
    control.apply(Command::Frame(1, Arc::clone(&source), frame("shot", 2)));
    step_until(&mut control, |control| control.engine.loaded(0));
}

/// Notes describe the content stack they came from: another stack starts,
/// and publishes, without the old one's.
#[test]
fn another_content_stack_drops_the_old_ones_notes() {
    let (_old_stack, old) = source("audio_system_notes_old");
    let (_new_stack, new) = source("audio_system_notes_new");
    let status = Arc::new(Mutex::new(Status::default()));
    let mut control = Control::new(Arc::clone(&status), None, true);
    let publish = |control: &mut Control| {
        control.last_evict = Instant::now() - Duration::from_secs(6);
        control.step();
    };
    control.apply(Command::Claim(1));
    control.apply(Command::Frame(1, Arc::clone(&old), frame("absent", 1)));
    control.apply(Command::Frame(1, Arc::clone(&old), frame("crackle", 2)));
    step_until(&mut control, |control| control.engine.notes.lines().len() == 2);
    publish(&mut control);
    assert_eq!(status.lock().unwrap().notes.len(), 2);
    control.apply(Command::Frame(1, Arc::clone(&new), frame("shot", 3)));
    assert!(
        control.engine.notes.is_empty(),
        "{:?}",
        control.engine.notes.lines()
    );
    assert!(status.lock().unwrap().notes.is_empty());
    step_until(&mut control, |control| control.engine.loaded(0));
    publish(&mut control);
    assert!(status.lock().unwrap().notes.is_empty());
}

/// A loader that is gone refuses its files by name instead of leaving
/// voices waiting forever.
#[test]
fn a_request_to_a_stopped_loader_is_refused_by_name() {
    let (_stack, source) = source("audio_system_dead_loader");
    let mut control = Control::new(Arc::new(Mutex::new(Status::default())), None, true);
    let (sounds, requests) = mpsc::channel();
    drop(requests);
    control.sounds = sounds;
    control.apply(Command::Claim(1));
    control.apply(Command::Frame(1, Arc::clone(&source), frame("shot", 1)));
    assert!(control.engine.active());
    control.step();
    assert!(!control.engine.active());
    assert_eq!(
        control.engine.notes.lines(),
        ["Unreadable sample sound/tone.wav: its loader stopped"]
    );
}

/// A decoder that panics refuses that file by name instead of ending the
/// loader thread.
#[test]
fn a_decoder_panic_is_refused_by_name() {
    let (_stack, source) = source("audio_system_panic");
    let refused: Result<Arc<Samples>, String> =
        guarded(&source, Load::Sound(1), |_, _| panic!("a decoder fault"));
    assert_eq!(
        refused.unwrap_err(),
        "sound/broken.wav: the decoder failed on it"
    );
    let decoded = guarded(&source, Load::Sound(0), decode).unwrap();
    assert_eq!(decoded.frames(), RATE as usize / 4);
}

fn fake_output(ring: Ring, progress: Instant) -> (Output, Stream) {
    (
        Output {
            ring: Arc::new(ring),
            rate: 48_000,
            channels: 2,
            format: DeviceFormat::F32,
            alive: Arc::new(AtomicBool::new(true)),
            failure: Arc::new(AtomicU8::new(0)),
            progress: (0, progress),
        },
        Stream::new(48_000, 2, DeviceFormat::F32).unwrap(),
    )
}

#[test]
fn the_ring_hands_whole_frames_over_in_order_and_refuses_when_full() {
    let ring = Ring::new(2, 2);
    assert!(ring.push_frame(&[1.0, 2.0]) && ring.push_frame(&[3.0, 4.0]));
    assert!(!ring.push_frame(&[5.0, 6.0]), "full");
    assert_eq!(ring.len(), 4);
    let mut frame = [0.0; 2];
    assert!(ring.pop_frame(|channel, value| frame[channel] = value));
    assert_eq!(frame, [1.0, 2.0]);
    assert!(ring.push_frame(&[5.0, 6.0]));
    let mut taken = Vec::new();
    while ring.pop_frame(|_, value| taken.push(value)) {}
    assert_eq!(taken, [3.0, 4.0, 5.0, 6.0]);
    assert_eq!(ring.consumed(), 6);
}

/// The callback takes only whole frames: an underrun, or a buffer that ends
/// mid-frame, is silence for whole frames, and the next frame still starts
/// on its left channel.
#[test]
fn the_device_callback_keeps_channels_aligned_across_underruns() {
    let ring = Ring::new(8, 2);
    ring.push_frame(&[0.1, 0.2]);
    // An underrun: one frame queued, two asked for.
    let mut out = [9.0f32; 4];
    drain(&ring, &mut out, |value| value);
    assert_eq!(out, [0.1, 0.2, 0.0, 0.0]);
    ring.push_frame(&[0.3, 0.4]);
    ring.push_frame(&[0.5, 0.6]);
    // A buffer ending mid-frame takes no half frame...
    let mut odd = [9.0f32; 3];
    drain(&ring, &mut odd, |value| value);
    assert_eq!(odd, [0.3, 0.4, 0.0]);
    // ...so the next frame is still left then right.
    let mut next = [9.0f32; 2];
    drain(&ring, &mut next, |value| value);
    assert_eq!(next, [0.5, 0.6]);
    assert_eq!(ring.len() % 2, 0);
}

/// The stream error callback records a failure without locking or
/// allocating; errors after which the stream plays on are not failures.
#[test]
fn stream_errors_that_leave_the_stream_playing_are_not_failures() {
    let failure = AtomicU8::new(0);
    record_failure(&failure, cpal::ErrorKind::DeviceChanged);
    record_failure(&failure, cpal::ErrorKind::Xrun);
    assert_eq!(failure.load(Ordering::Acquire), 0);
    record_failure(&failure, cpal::ErrorKind::DeviceNotAvailable);
    record_failure(&failure, cpal::ErrorKind::BackendError);
    let code = failure.load(Ordering::Acquire);
    assert_eq!(
        STREAM_FAILURES[usize::from(code) - 1],
        cpal::ErrorKind::DeviceNotAvailable,
        "the first failure is kept"
    );
}

/// A device that reports an error, or stops taking samples, is dropped: the
/// status says why, mixing continues by the clock and the device is opened
/// again later.
#[test]
fn a_lost_device_is_reported_and_dropped() {
    let status = Arc::new(Mutex::new(Status::default()));
    let mut control = Control::new(Arc::clone(&status), None, false);
    let ring = Ring::new(8, 2);
    ring.push_frame(&[0.5, 0.5]);
    control.output = Some(fake_output(ring, Instant::now() - STALLED * 2));
    control.step();
    assert!(control.output.is_none());
    assert_eq!(
        status.lock().unwrap().device,
        Some(Err(
            "The audio output was lost (it stopped taking samples)".into()
        ))
    );
    assert!(control.next_open > Instant::now(), "the reopen waits");
    // A reported stream error is lost at once.
    let (output, converter) = fake_output(Ring::new(8, 2), Instant::now());
    record_failure(&output.failure, cpal::ErrorKind::DeviceNotAvailable);
    control.output = Some((output, converter));
    control.step();
    assert!(control.output.is_none());
    assert_eq!(
        status.lock().unwrap().device,
        Some(Err(format!(
            "The audio output was lost ({})",
            cpal::ErrorKind::DeviceNotAvailable
        )))
    );
    // A device that keeps taking samples stays.
    control.output = Some(fake_output(Ring::new(8, 2), Instant::now() - STALLED * 2));
    control.step();
    assert!(control.output.is_some());
}
