use super::*;
use replay::{
    frame::FrameEncoder,
    identity::{Roster, Verification},
    rows::{DayNightClock, ProvinceTemperature, TableRows, TemperaturePass, WeatherProvinceRow},
    spool::{ControlRecord, Record},
    temperature::State,
};

#[test]
fn derived_command_fails_on_wrong_current_checksum_and_incomplete_comparison() {
    let root = crate::test_support::allocation_test_directory("derived-verdict");
    let game = root.join("game");
    let user = root.join("user");
    for directory in ["common/defines", "common/terrain", "map", "dlc"] {
        fs::create_dir_all(game.join(directory)).unwrap();
    }
    fs::create_dir_all(&user).unwrap();
    fs::write(game.join("hoi4.exe"), b"synthetic executable").unwrap();
    fs::write(user.join("settings.txt"), "language=\"english\"").unwrap();
    fs::write(
        user.join("dlc_load.json"),
        r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
    )
    .unwrap();
    fs::write(
        game.join("common/defines/test.lua"),
        "NDefines={NGraphics={GMT_OFFSET=0,SOUTH_POLE_OFFSET=0,NORTH_POLE_OFFSET=1,DAY_NIGHT_FEATHER=0.1}}",
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
    let gathered = installed_content::gather(&installed_content::GatherOptions {
        game_dir: game.clone(),
        user_dir: user.clone(),
        cache_dir: root.join("cache"),
        dlc_order: Some(Vec::new()),
    })
    .unwrap();
    let executable = engine::build::hash_executable(&b"synthetic executable"[..]).unwrap();
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
                                        checksum: replay::temperature::checksum([0, 0]),
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
    for (bad_checksum, missing_pass, bad_clock) in [
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
                } else if bad_checksum {
                    let Recorded::Value(Some(pass)) = &mut rows[0].temperature_pass else {
                        panic!("fixture must hold its pass");
                    };
                    pass.checksum ^= 1;
                } else if bad_clock {
                    rows[1].clock = DayNightClock::held(Clock {
                        local_hour: i32::MIN,
                        night_factor: 0,
                    });
                }
            }
        }
        let replay = Replay::from_records(
            crate::test_support::ALLOCATION_CAMPAIGN.to_vec(),
            &[records],
        )
        .unwrap();
        fs::write(&recording, replay.bytes()).unwrap();
        let outcome = crate::recording::command(args());
        if bad_checksum || missing_pass || bad_clock {
            let error = outcome.unwrap_err();
            let expected = if bad_checksum || bad_clock {
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
