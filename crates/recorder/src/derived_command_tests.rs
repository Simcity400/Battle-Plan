use replay::campaign::Replay;
use replay_recording::control::ControlRecord;
use replay_capture_spool::spool::Record;
use replay_data::{day_night, day_night::Clock, rows::Recorded, rows::TableId};
use replay_data::{
    identity::Roster, identity::Verification, rows::DayNightClock, rows::ProvinceTemperature,
    rows::TableRows, rows::TemperaturePass, rows::WeatherProvinceRow, temperature::State,
};
use replay_frame::frame::FrameEncoder;
use replay_frame::{frame::FrameDecoder, frame::TableUpdate, frame::TableView};
use std::fs;

#[test]
fn derived_command_fails_on_wrong_current_witness_and_incomplete_comparison() {
    const CHILD: &str = "BATTLEPLAN_DERIVED_TEST_ROOT";
    let Some(root) = std::env::var_os(CHILD).map(std::path::PathBuf::from) else {
        let sandbox = synthetic_game::Installation::new("derived-cache").build();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "derived_command_tests::derived_command_fails_on_wrong_current_witness_and_incomplete_comparison",
                "--nocapture",
            ])
            .env(CHILD, &sandbox.cache_dir)
            .env("LOCALAPPDATA", &sandbox.cache_dir)
            .env("BATTLEPLAN_CACHE", sandbox.cache_dir.join("private"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!sandbox.cache_dir.join("BattlePlan").exists());
        assert!(sandbox.cache_dir.join("private/content-hashes-v2.bin").is_file());
        return;
    };
    content_reader_script::test_game::enter();
    let installation = synthetic_game::Installation::new("derived-verdict").build();
    let game = installation.game_dir.clone();
    let user = installation.user_dir.clone();
    for directory in ["common/defines", "common/terrain", "map", "dlc"] {
        fs::create_dir_all(game.join(directory)).unwrap();
    }
    fs::write(
        game.join("common/defines/test.lua"),
        r#"NDefines={
            NGraphics={GMT_OFFSET=0,SOUTH_POLE_OFFSET=0,NORTH_POLE_OFFSET=1,DAY_NIGHT_FEATHER=0.1},
            NMilitary={
                AMPHIBIOUS_INVADE_MOVEMENT_COST=1,LAND_SPEED_MODIFIER=1,
                RIVER_CROSSING_SPEED_PENALTY=0,RIVER_CROSSING_SPEED_PENALTY_LARGE=0,
                RIVER_SMALL_START_INDEX=1,RIVER_SMALL_STOP_INDEX=1,RIVER_LARGE_STOP_INDEX=2
            },
            NNavy={NAVAL_SPEED_MODIFIER=1,AMPHIBIOUS_INVADE_SPEED_BASE=1}
        }"#,
    )
    .unwrap();
    fs::write(
        game.join("common/terrain/test.txt"),
        "categories={test={movement_cost=1}} terrain={test={type=test color={1}}}",
    )
    .unwrap();
    fs::write(game.join("common/weather.txt"), "settings={}").unwrap();
    fs::write(
        game.join("map/default.map"),
        "definitions=\"definition.csv\" provinces=\"provinces.bmp\" terrain=\"terrain.bmp\" rivers=\"rivers.bmp\" heightmap=\"heightmap.bmp\" tree_definition=\"trees.bmp\" adjacencies=\"adjacencies.csv\" adjacency_rules=\"adjacency_rules.txt\"",
    )
    .unwrap();
    fs::write(
        game.join("map/definition.csv"),
        "0;0;0;0;land;false;test;0\n1;1;0;0;land;false;test;0\n",
    )
    .unwrap();
    fs::write(
        game.join("map/adjacencies.csv"),
        "From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment\n-1;-1;;-1;-1;-1;-1;-1;-1;\n",
    )
    .unwrap();
    fs::write(game.join("map/adjacency_rules.txt"), "").unwrap();
    let mut bitmap = vec![0_u8; 58];
    bitmap[..2].copy_from_slice(b"BM");
    for (offset, value) in [(2, 58_u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
        bitmap[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bitmap[56] = 1;
    fs::write(game.join("map/provinces.bmp"), &bitmap).unwrap();
    fs::write(game.join("map/terrain.bmp"), &bitmap).unwrap();
    bitmap[28..30].copy_from_slice(&8_u16.to_le_bytes());
    bitmap[54..].fill(0);
    fs::write(game.join("map/rivers.bmp"), &bitmap).unwrap();
    let gathered = content_reader_layers::gather(&content_reader_layers::GatherOptions {
        game_dir: game.clone(),
        user_dir: user.clone(),
        cache_dir: installation.cache_dir.clone(),
        dlc_order: Some(Vec::new()),
    })
    .unwrap();
    let executable =
        engine_executable::build::hash_executable(fs::File::open(game.join("hoi4.exe")).unwrap())
            .unwrap();
    let mut identity = gathered
        .identity(
            "synthetic".into(),
            executable.sha256,
            executable.size,
            "test".into(),
            [0; 32],
        )
        .unwrap();
    identity.verification = Verification::Verified {
        launcher: Roster::default(),
        memory: Roster::default(),
    };
    let mut records = crate::test_support::allocation_records();
    let mut encoder = FrameEncoder::new();
    for record in &mut records {
        match record {
            Record::SessionStart(session) => session.identity = identity.clone(),
            Record::BranchStart(branch) => {
                branch.loaded_hour = branch.loaded_hour.map(|hour| hour + day_night::EPOCH);
                branch.first_hour += day_night::EPOCH;
            }
            Record::Control(ControlRecord::ClockWitness { hour, .. }) => {
                *hour += day_night::EPOCH;
            }
            Record::Control(ControlRecord::Detach { last_hour, .. }) => {
                *last_hour = last_hour.map(|hour| hour + day_night::EPOCH);
            }
            Record::Frame(encoded) => {
                let mut frame = FrameDecoder::new().apply(encoded).unwrap();
                frame.hour = encoded.hour + day_night::EPOCH;
                if let TableView::Complete(TableRows::Clock(rows)) =
                    &mut *frame.tables[TableId::Clock.index()]
                {
                    rows[0].hour = frame.hour;
                }
                if let TableView::Complete(TableRows::SimulationFeatures(rows)) =
                    &mut *frame.tables[TableId::SimulationFeatures.index()]
                {
                    rows[0].weather_suspend = Recorded::Value(Some([0, 0]));
                }
                *frame.tables[TableId::WeatherProvinces.index()] =
                    TableView::Complete(TableRows::WeatherProvinces(
                        (0..2)
                            .map(|id| WeatherProvinceRow {
                                id,
                                temperature: ProvinceTemperature::anchored(State::default()),
                                clock: if id == 0 {
                                    DayNightClock::held(Clock::default())
                                } else {
                                    DayNightClock::UPDATED
                                },
                                temperature_pass: Recorded::Value((id == 0).then_some(
                                    TemperaturePass {
                                        anchor: frame.hour,
                                        witness: replay_data::rows::TemperatureWitness::new(
                                            replay_data::temperature::witness([0, 0]),
                                        ),
                                        ..Default::default()
                                    },
                                )),
                                ..Default::default()
                            })
                            .collect(),
                    ));
                *encoded = encoder.encode(frame, true).unwrap();
            }
            _ => {}
        }
    }
    let recording = root.join("test.battleplan");
    let args = || {
        vec![
            "derived".into(),
            recording.clone().into_os_string(),
            "--game-dir".into(),
            game.clone().into_os_string(),
            "--user-dir".into(),
            user.clone().into_os_string(),
        ]
    };
    for (bad_witness, missing_pass, bad_clock) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let mut records = records.clone();
        for record in &mut records {
            if let Record::Frame(frame) = record
                && let TableUpdate::Snapshot(TableRows::WeatherProvinces(rows)) =
                    &mut frame.tables[TableId::WeatherProvinces.index()]
            {
                if missing_pass {
                    rows[0].temperature_pass = Recorded::Value(None);
                } else if bad_witness {
                    let Recorded::Value(Some(pass)) = &mut rows[0].temperature_pass else {
                        panic!("fixture must hold its pass");
                    };
                    pass.witness =
                        replay_data::rows::TemperatureWitness::new(pass.witness.get().unwrap() ^ 1);
                } else if bad_clock {
                    rows[1].clock = DayNightClock::held(Clock {
                        local_hour: i32::MIN,
                        night_factor: 0,
                    });
                }
            }
        }
        let replay = Replay::from_records(
            recorder_derive::writers(),
            crate::test_support::ALLOCATION_CAMPAIGN.to_vec(),
            &[records],
        )
        .unwrap();
        fs::write(&recording, replay.bytes()).unwrap();
        let outcome = crate::recording::command(args());
        if bad_witness || missing_pass || bad_clock {
            let error = outcome.unwrap_err();
            let expected = if bad_witness || bad_clock {
                "3 mismatches; 0 incomplete comparisons"
            } else {
                "0 mismatches; 3 incomplete comparisons"
            };
            assert!(error.contains(expected), "{error}");
        } else {
            outcome.unwrap();
        }
    }
}
