//! Opt-in layer measurements against the synthetic replay example's output.
use replay::container::SeekResult;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use viewer_foundation::engagement_history::LAND;
use viewer_foundation::engagement_history::combats;
use {
    crate::document::Document, crate::document::Hour, crate::document::InitialHour,
    crate::engaged_index::EngagedIndex, viewer_foundation::map::camera::MapSize,
    viewer_foundation::map::camera::Placement, viewer_foundation::map::perspective,
    viewer_foundation::map::tables::core, viewer_foundation::map::visibility::Relations,
    viewer_world::map::political,
};

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
        &installed_content_core::ResolveOptions {
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
    // The index the window starts for its perspective country
    // (`hours::Pipeline::engaged`), timed until it covers the last hour.
    let started = Instant::now();
    let engaged = Arc::new(
        EngagedIndex::start(
            document.playback.replay_handle(),
            perspective.country.clone(),
        )
        .unwrap(),
    );
    let last = replay
        .index
        .iter()
        .filter(|block| block.branch == 1)
        .map(|block| block.last_hour)
        .max()
        .unwrap();
    while !engaged.shared.covers(1, last) {
        thread::sleep(Duration::from_millis(1));
    }
    let ms = started.elapsed().as_secs_f64() * 1e3;
    let hours = last - first + 1;
    println!(
        "UNIT_ENGAGED_FOLD hours={hours} ms={ms:.6} per_hour_ms={:.6}",
        ms / f64::from(hours)
    );
    for (camera_name, height) in [("close", 70.0), ("mid", 339.0), ("world", 2400.0)] {
        let camera = Placement {
            center: [3009.9868, 1537.9652],
            height,
            pose: viewer_foundation::map::camera::Pose::Map,
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
            lap("names", camera_name, at, || {
                hour.place_names(&Default::default())
            });
            // The land combat flags alone: from the covering index, as the
            // window reads them, and rebuilt as a load (or a dump) does.
            let viewer = hour.vision(&perspective).unwrap().ours();
            let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(&frame)
                .unwrap()
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            let relations = Relations::read(&frame, &links);
            assert!(
                engaged.shared.answers(&frame, viewer),
                "the index does not answer hour {at}"
            );
            let flags = |index: Option<&EngagedIndex>| {
                combats(
                    &frame,
                    LAND,
                    viewer,
                    &links,
                    relations.as_ref().map_err(|reason| *reason),
                    index.map(|index| index.shared.as_ref()),
                )
            };
            if camera_name == "close" && offset == 0 {
                let mut counts = BTreeMap::new();
                for flag in flags(Some(&*engaged)).unwrap().0.values() {
                    *counts.entry(format!("{flag:?}")).or_insert(0) += 1;
                }
                println!("UNIT_ENGAGED hour={at} flags={counts:?}");
                // The fixture covers the engaged index, not every marker:
                // its added combats have no LandCombatDetails row, so a seen
                // one draws no plate or wedge. Every combat such a gap names
                // must be one of those.
                let tables = hour.hour_tables(&perspective, Some(&*engaged)).unwrap();
                let (gaps, limits) = tables.land_combat_gaps();
                let details: BTreeSet<u64> = core::<tables::LandCombatDetails>(&frame)
                    .map(|rows| rows.iter().map(|row| row.combat).collect())
                    .unwrap_or_default();
                let without_details: BTreeSet<u64> = core::<tables::LandCombats>(&frame)
                    .unwrap()
                    .iter()
                    .map(|combat| combat.id)
                    .filter(|id| !details.contains(id))
                    .collect();
                let sizes = |causes: &BTreeMap<String, BTreeSet<u64>>| {
                    causes
                        .iter()
                        .map(|(cause, combats)| (cause.clone(), combats.len()))
                        .collect::<BTreeMap<_, _>>()
                };
                println!(
                    "UNIT_LAND_MARKERS hour={at} without_details={} gaps={:?} limits={:?}",
                    without_details.len(),
                    sizes(gaps),
                    sizes(limits)
                );
                for (cause, combats) in gaps {
                    if cause.starts_with("plate and wedge omitted") {
                        assert!(
                            combats.is_subset(&without_details),
                            "{cause}: {combats:?} has a combat with details"
                        );
                    }
                }
                // The covering index answers every flag: no load-rebuilt
                // provenance. (Marker value limits, such as an unseen
                // icon's withheld frame, are not provenance.)
                assert!(
                    !limits.keys().any(|limit| limit.starts_with("engaged flag")),
                    "the covering index answers: {:?}",
                    sizes(limits)
                );
            }
            lap("engaged_indexed", camera_name, at, || {
                flags(Some(&*engaged))
            });
            lap("engaged_rebuilt", camera_name, at, || flags(None));
            lap("overlay", camera_name, at, || {
                hour.hour_tables(&perspective, Some(&*engaged))
                    .and_then(|tables| {
                        hour.prepare_overlay_cached(
                            camera,
                            [1920, 1080],
                            1.0,
                            &tables,
                            &Default::default(),
                        )
                    })
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
            Arc::clone(&document.content),
            document.playback.replay_handle(),
        )
        .unwrap();
        // The window carries its index into a new pipeline the same way.
        pipeline.adopt_engaged(Arc::clone(&engaged));
        let started = Instant::now();
        pipeline
            .want(crate::hours::Want {
                branch: 1,
                hour: first,
                ahead: 8,
                behind: 0,
                playing: true,
                perspective: perspective.clone(),
                map_mode: Default::default(),
                show_allied_plans: false,
                counters: Default::default(),
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
