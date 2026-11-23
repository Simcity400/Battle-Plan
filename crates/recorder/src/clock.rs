use crate::controller::{Probe, probe_withheld};
use engine_native::{Memory, cgame_state::{self, Clock}};
use engine_native_politics::camera_root::{self, Camera};
use engine_native_air::command_inputs;
use serde_json::{Map, Value, json};
use std::{
    ffi::OsString,
    io::{self, BufRead, Write},
};

pub(crate) const USAGE: &str = "recorder clock --pid <game process id>
  Reads the game's clock read-only and prints JSON lines: {\"ready\":true,\"withheld\":{..}} once,
  then {\"clock\":{..}|null,\"withheld\":{..}} for each line read on stdin, until stdin ends.
  A field this build withholds is left out of clock and named in withheld with its reason;
  a withheld clock prints {\"ready\":false,\"withheld\":{\"clock\":..}} and fails.";

const PRESENTATION: [&str; 4] = ["paused", "eye", "look_at", "viewport"];

#[derive(Debug)]
pub(crate) struct Sample {
    clock: Clock,
    paused: Option<bool>,
    camera: Option<Camera>,
}

fn once(memory: &impl Memory, base: usize, presentation: bool) -> Option<Sample> {
    let clock = engine_allocation::advisory(|| cgame_state::read_clock(memory, base))?;
    let idler = presentation
        .then(|| command_inputs::in_game_idler(memory, base).ok().flatten())
        .flatten();
    let paused = idler.and_then(|idler| command_inputs::idler_paused(memory, idler).ok());
    let camera = idler
        .filter(|_| paused.is_some())
        .and_then(|idler| camera_root::read_camera(memory, idler));
    Some(Sample {
        clock,
        paused,
        camera,
    })
}

pub(crate) fn campaign_loaded(memory: &impl Memory, base: usize) -> bool {
    engine_allocation::advisory(|| cgame_state::read_clock(memory, base)).is_some()
        && matches!(command_inputs::in_game_idler(memory, base), Ok(Some(_)))
}

pub(crate) fn sample(memory: &impl Memory, base: usize, presentation: bool) -> Option<Sample> {
    let first = once(memory, base, presentation)?;
    let second = once(memory, base, presentation)?;
    (first.clock == second.clock && first.paused == second.paused).then_some(second)
}

fn withheld(presentation: Result<(), &str>) -> Map<String, Value> {
    match presentation {
        Ok(()) => Map::new(),
        Err(reason) => PRESENTATION
            .iter()
            .map(|field| (field.to_string(), reason.into()))
            .collect(),
    }
}

fn clock_json(sample: &Sample) -> Value {
    let c = &sample.clock;
    let mut clock = json!({
        "game_state": c.game_state,
        "hour": c.hour,
        "play_time": c.play_time,
        "seed": c.seed,
        "speed": c.speed,
        "tick_busy": c.tick_busy,
        "play_session_started_at": c.play_session_started_at,
    });
    if let Some(paused) = sample.paused {
        clock["paused"] = paused.into();
    }
    if let Some(camera) = sample.camera.filter(|camera| {
        camera
            .eye
            .iter()
            .chain(&camera.look_at)
            .all(|v| v.is_finite())
    }) {
        clock["eye"] = json!(camera.eye);
        clock["look_at"] = json!(camera.look_at);
        clock["viewport"] = json!(camera.viewport);
    }
    clock
}

fn sample_line(sample: Option<&Sample>, withheld: &Map<String, Value>) -> Value {
    json!({ "clock": sample.map(clock_json), "withheld": withheld })
}

fn ready_line(opened: Result<&Map<String, Value>, &str>) -> Value {
    match opened {
        Ok(withheld) => json!({ "ready": true, "withheld": withheld }),
        Err(reason) => json!({ "ready": false, "withheld": { "clock": reason } }),
    }
}

pub(crate) fn command(args: Vec<OsString>) -> Result<(), String> {
    let pid = match args.as_slice() {
        [flag, pid] if flag == "--pid" => pid.to_str().and_then(|pid| pid.parse::<u32>().ok()),
        _ => None,
    }
    .ok_or(USAGE)?;
    let mut out = io::stdout().lock();
    let mut emit = |line: Value| {
        writeln!(out, "{line}")
            .and_then(|()| out.flush())
            .map_err(|e| e.to_string())
    };
    let probe = match Probe::open_pid(pid) {
        Ok(probe) => probe,
        Err(error) => {
            if probe_withheld(&error) {
                emit(ready_line(Err(error.as_str())))?;
            }
            return Err(error);
        }
    };
    let sites = probe.presentation_sites();
    let presentation = sites.is_ok();
    let withheld = withheld(sites);
    emit(ready_line(Ok(&withheld)))?;
    for line in io::stdin().lock().lines() {
        line.map_err(|e| e.to_string())?;
        emit(sample_line(probe.clock(presentation).as_ref(), &withheld))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgame_state::{
        CAMPAIGN_ID, CAMPAIGN_SEED, CURRENT_HOUR, PLAY_SESSION_STARTED_AT_RVA, PLAY_TIME,
        SINGLETON_RVA, SPEED, TICK_BUSY,
    };
    use command_inputs::{IDLER_PAUSED, IDLER_RVA, IN_GAME_IDLER_VTABLE_RVA};
    use std::cell::Cell;

    const STATE: usize = 0x1000;
    const IDLER: usize = 0x5000;
    const ROOT: usize = 0x7000;

    #[derive(Clone)]
    struct Fake(Vec<(usize, Vec<u8>)>);
    impl Memory for Fake {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.0.iter().any(|(at, bytes)| {
                let Some(source) = address
                    .checked_sub(*at)
                    .and_then(|start| bytes.get(start..start.checked_add(output.len())?))
                else {
                    return false;
                };
                output.copy_from_slice(source);
                true
            })
        }
    }
    impl Fake {
        fn put(&mut self, region: usize, offset: usize, bytes: &[u8]) {
            let (_, memory) = self.0.iter_mut().find(|(at, _)| *at == region).unwrap();
            memory[offset..offset + bytes.len()].copy_from_slice(bytes);
        }
    }

    fn eye_and_look_at() -> Vec<u8> {
        [1500.5_f32, 320.0, 900.25, 1500.5, 0.0, 1010.75]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }

    fn game() -> Fake {
        let mut state = vec![0; 0x800];
        state[CURRENT_HOUR..CURRENT_HOUR + 4].copy_from_slice(&1936_i32.to_le_bytes());
        state[PLAY_TIME..PLAY_TIME + 8].copy_from_slice(&77_u64.to_le_bytes());
        state[CAMPAIGN_SEED..CAMPAIGN_SEED + 4].copy_from_slice(&(-5_i32).to_le_bytes());
        state[SPEED..SPEED + 4].copy_from_slice(&3_i32.to_le_bytes());
        state[TICK_BUSY] = 0;
        state[CAMPAIGN_ID + 24..CAMPAIGN_ID + 32].copy_from_slice(&15_u64.to_le_bytes());
        let mut idler = vec![0; 0x800];
        idler[..8].copy_from_slice(&(IN_GAME_IDLER_VTABLE_RVA.0 as u64).to_le_bytes());
        idler[IDLER_PAUSED] = 1;
        idler[camera_root::IDLER_CAMERA..camera_root::IDLER_CAMERA + 8]
            .copy_from_slice(&(ROOT as u64).to_le_bytes());
        let mut root = vec![0; 0x800];
        root[camera_root::OWNER..camera_root::OWNER + 8]
            .copy_from_slice(&(IDLER as u64).to_le_bytes());
        root[camera_root::EYE..camera_root::EYE + 24].copy_from_slice(&eye_and_look_at());
        root[camera_root::RENDER_VIEW_EYE..camera_root::RENDER_VIEW_EYE + 24]
            .copy_from_slice(&eye_and_look_at());
        let viewport: Vec<u8> = [0_i32, 0, 2560, 1440]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        root[camera_root::VIEWPORT..camera_root::VIEWPORT + 16].copy_from_slice(&viewport);
        Fake(vec![
            (SINGLETON_RVA.0, (STATE as u64).to_le_bytes().to_vec()),
            (
                PLAY_SESSION_STARTED_AT_RVA.0,
                1_778_888_888_i64.to_le_bytes().to_vec(),
            ),
            (IDLER_RVA.0, (IDLER as u64).to_le_bytes().to_vec()),
            (STATE, state),
            (IDLER, idler),
            (ROOT, root),
        ])
    }

    fn line(memory: &impl Memory, presentation: Result<(), &str>) -> Value {
        let withheld = withheld(presentation);
        sample_line(sample(memory, 0, presentation.is_ok()).as_ref(), &withheld)
    }

    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<_> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn a_campaign_is_loaded_only_once_the_in_game_idler_runs() {
        let mut game = game();
        assert!(campaign_loaded(&game, 0));
        game.put(IDLER, 0, &0x1234_u64.to_le_bytes());
        assert!(!campaign_loaded(&game, 0));
        game.put(IDLER_RVA.0, 0, &0_u64.to_le_bytes());
        assert!(!campaign_loaded(&game, 0));
    }

    #[test]
    fn a_sample_carries_every_clock_field_and_the_verified_presentation() {
        let line = line(&game(), Ok(()));
        assert_eq!(
            line,
            json!({
                "clock": {
                    "game_state": STATE, "hour": 1936, "play_time": 77, "seed": -5, "speed": 3,
                    "tick_busy": 0, "play_session_started_at": 1_778_888_888_i64, "paused": true,
                    "eye": [1500.5, 320.0, 900.25], "look_at": [1500.5, 0.0, 1010.75],
                    "viewport": [0, 0, 2560, 1440],
                },
                "withheld": {},
            })
        );
        assert_eq!(
            ready_line(Ok(&Map::new())),
            json!({ "ready": true, "withheld": {} })
        );
    }

    #[test]
    fn withheld_presentation_sites_are_named_and_their_fields_left_out_not_zeroed() {
        let reason = "command_inputs::IDLER_RVA withheld: unresolved";
        let line = line(&game(), Err(reason));
        assert_eq!(
            keys(&line["clock"]),
            [
                "game_state",
                "hour",
                "play_session_started_at",
                "play_time",
                "seed",
                "speed",
                "tick_busy"
            ]
        );
        assert_eq!(
            keys(&line["withheld"]),
            ["eye", "look_at", "paused", "viewport"]
        );
        assert!(
            line["withheld"]
                .as_object()
                .unwrap()
                .values()
                .all(|why| why == reason)
        );
        let clock = "the loaded campaign is withheld on this unrecognised game build: struct layouts withheld: changed";
        assert_eq!(
            ready_line(Err(clock)),
            json!({ "ready": false, "withheld": { "clock": clock } })
        );
    }

    #[test]
    fn a_camera_root_that_does_not_verify_leaves_the_camera_out_and_keeps_the_pause() {
        let mut stranger = game();
        stranger.put(ROOT, camera_root::OWNER, &0x9000_u64.to_le_bytes());
        let mut moved = game();
        moved.put(ROOT, camera_root::RENDER_VIEW_EYE, &7.0_f32.to_le_bytes());
        let mut nan = game();
        let nan_eye = f32::NAN.to_le_bytes();
        nan.put(ROOT, camera_root::EYE, &nan_eye);
        nan.put(ROOT, camera_root::RENDER_VIEW_EYE, &nan_eye);
        for memory in [stranger, moved, nan] {
            let line = line(&memory, Ok(()));
            assert_eq!(line["clock"]["paused"], true);
            for field in ["eye", "look_at", "viewport"] {
                assert!(line["clock"].get(field).is_none(), "{field}");
            }
        }
    }

    #[test]
    fn another_idler_or_an_unreadable_pause_byte_leaves_the_presentation_out() {
        let mut menu = game();
        menu.put(IDLER, 0, &0x1234_u64.to_le_bytes());
        let mut torn = game();
        torn.put(IDLER, IDLER_PAUSED, &[7]);
        for memory in [menu, torn] {
            let line = line(&memory, Ok(()));
            assert_eq!(line["clock"]["hour"], 1936);
            for field in PRESENTATION {
                assert!(line["clock"].get(field).is_none(), "{field}");
            }
            assert_eq!(line["withheld"], json!({}));
        }
    }

    #[test]
    fn the_clock_is_null_without_a_game_state_or_when_two_reads_disagree() {
        let mut menu = game();
        menu.put(SINGLETON_RVA.0, 0, &0_u64.to_le_bytes());
        assert_eq!(line(&menu, Ok(()))["clock"], Value::Null);

        struct Changing {
            before: Fake,
            after: Fake,
            singleton_reads: Cell<u32>,
        }
        impl Memory for Changing {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == SINGLETON_RVA.0 {
                    self.singleton_reads.set(self.singleton_reads.get() + 1);
                }
                let memory = if self.singleton_reads.get() <= 1 {
                    &self.before
                } else {
                    &self.after
                };
                memory.copy(address, output)
            }
        }
        let mut advanced = game();
        advanced.put(STATE, CURRENT_HOUR, &1937_i32.to_le_bytes());
        let mut unpaused = game();
        unpaused.put(IDLER, IDLER_PAUSED, &[0]);
        for after in [advanced, unpaused] {
            let memory = Changing {
                before: game(),
                after,
                singleton_reads: Cell::new(0),
            };
            assert_eq!(line(&memory, Ok(()))["clock"], Value::Null);
        }
    }
}
