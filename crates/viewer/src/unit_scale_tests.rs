//! Opt-in layer measurements against the synthetic replay example's output.
use crate::{
    document::{Document, Hour, InitialHour},
    map::{
        camera::{MapSize, Placement},
        perspective, political,
    },
};
use replay::{container::SeekResult, frame::TableView, rows::TableId};
use std::{io, path::PathBuf, time::Instant};

fn lap<T>(name: &str, camera: &str, hour: i32, work: impl FnOnce() -> io::Result<T>) {
    let start = Instant::now();
    let result = work();
    let ms = start.elapsed().as_secs_f64() * 1e3;
    match result {
        Ok(value) => {
            println!(
                "UNIT_LAYER name={name} camera={camera} hour={hour} ms={ms:.6} status=complete"
            );
            std::hint::black_box(value);
        }
        Err(error) => panic!("{name} at {hour}: {error}"),
    }
}

#[test]
#[ignore = "requires BATTLE_PLAN_UNIT_REPLAY, BATTLE_PLAN_UNIT_GAME, BATTLE_PLAN_UNIT_USER and BATTLE_PLAN_UNIT_CACHE"]
fn unit_scale_replay_layers() {
    let path = |name| PathBuf::from(std::env::var_os(name).expect(name));
    let document = Document::load(
        &path("BATTLE_PLAN_UNIT_REPLAY"),
        &installed_content::ResolveOptions {
            game_dir: path("BATTLE_PLAN_UNIT_GAME"),
            user_dir: path("BATTLE_PLAN_UNIT_USER"),
            cache_dir: path("BATTLE_PLAN_UNIT_CACHE"),
        },
        false,
        &InitialHour {
            branch: None,
            hour: None,
            hour_fraction: 0.0,
        },
        None,
        0,
    )
    .unwrap();
    let replay = document.playback.replay();
    let first = replay.index[0].first_hour;
    let perspective = perspective::Choice {
        fog: false,
        ..Default::default()
    };
    for (camera_name, height) in [("close", 70.0), ("mid", 339.0), ("world", 2400.0)] {
        let camera = Placement {
            center: [3009.9868, 1537.9652],
            height,
        }
        .on(MapSize::of(&document.map), document.map_defines);
        for offset in 0..9 {
            let at = first + offset;
            let start = Instant::now();
            let SeekResult::Frame(frame) = replay.seek(1, at).unwrap() else {
                panic!("fixture gap")
            };
            println!(
                "UNIT_LAYER name=decode camera={camera_name} hour={at} ms={:.6} status=complete",
                start.elapsed().as_secs_f64() * 1e3
            );
            if camera_name == "close" && offset == 0 {
                println!("UNIT_REPLAY first_hour={first}");
                for table in TableId::ALL {
                    if let TableView::Complete(rows) = &*frame.tables[table.index()] {
                        // Row-vector allocation only; nested strings/lists are separate.
                        println!(
                            "UNIT_ROWS table={table:?} row_vector_bytes={}",
                            rows.allocated_bytes()
                        );
                    }
                }
            }
            let hour = Hour::new(&document.content, &frame, 0.0);
            lap("vision", camera_name, at, || hour.vision(&perspective));
            lap("political", camera_name, at, || {
                political::build(&document.map, &frame)
            });
            lap("names", camera_name, at, || hour.place_names());
            lap("overlay", camera_name, at, || {
                hour.prepare_overlay(camera, [1920, 1080], 1.0, &perspective)
            });
            lap("rails", camera_name, at, || hour.rail_mesh());
            lap("buildings", camera_name, at, || {
                hour.map_building_field(&perspective)
            });
            lap("aircraft", camera_name, at, || {
                hour.aircraft_plan(&perspective)
            });
            lap("unit_models", camera_name, at, || {
                hour.prepare_unit_models(&perspective)
            });
            lap("orders", camera_name, at, || hour.order_field(&perspective));
            lap("snow", camera_name, at, || hour.snow_mud());
            lap("limitations", camera_name, at, || {
                hour.limitations(&perspective)
            });
        }
        let mut pipeline = crate::hours::Pipeline::new(
            std::sync::Arc::clone(&document.content),
            document.playback.replay_handle(),
        )
        .unwrap();
        let started = Instant::now();
        pipeline
            .want(crate::hours::Want {
                branch: 1,
                hour: first,
                ahead: 8,
                behind: 0,
                playing: true,
                perspective: perspective.clone(),
                view: crate::hours::View {
                    camera,
                    viewport: [1920, 1080],
                    ui_scale: 1.0,
                },
                have: Vec::new(),
                received: 0,
            })
            .unwrap();
        let mut held = Vec::new();
        for _ in 0..9 {
            let built = pipeline.wait().unwrap();
            assert!(built.layers.is_ok(), "{:?}", built.layers.as_ref().err());
            println!(
                "UNIT_BUILDER camera={camera_name} hour={} wall_ms={:.6}",
                built.hour, built.wall
            );
            for (name, ms) in &built.laps {
                println!(
                    "UNIT_BUILDER_PART camera={camera_name} hour={} name={name} ms={ms:.6}",
                    built.hour
                );
            }
            held.push(built);
        }
        println!(
            "UNIT_BUILDER_BATCH camera={camera_name} hours={} wall_ms={:.6}",
            held.len(),
            started.elapsed().as_secs_f64() * 1e3
        );
        std::hint::black_box(held);
    }
}
