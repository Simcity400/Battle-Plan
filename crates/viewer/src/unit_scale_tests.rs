use replay::campaign::SeekResult;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::TableView;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};
use viewer_engagements::engagement_history::LAND;
use viewer_engagements::engagement_history::combats;
use crate::{document::{Document, Hour, InitialHour}, engaged_index::EngagedIndex};
use viewer_gpu::camera::{MapSize, Placement};
use viewer_foundation::{perspective, tables::core, visibility::Relations};
use viewer_map_modes::political;

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
#[ignore = "requires BATTLEPLAN_UNIT_REPLAY, BATTLEPLAN_UNIT_GAME, BATTLEPLAN_UNIT_USER and BATTLEPLAN_UNIT_CACHE"]
fn unit_scale_replay_layers() {
    let path = |name| PathBuf::from(std::env::var_os(name).expect(name));
    let document = Document::load(
        &path("BATTLEPLAN_UNIT_REPLAY"),
        &content_reader_layers::ResolveOptions {
            game_dir: path("BATTLEPLAN_UNIT_GAME"),
            user_dir: path("BATTLEPLAN_UNIT_USER"),
            cache_dir: path("BATTLEPLAN_UNIT_CACHE"),
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
    let mut folded = Vec::new();
    for offset in 0..9 {
        let SeekResult::Frame(frame) = replay.seek(1, first + offset).unwrap() else {
            panic!("fixture gap")
        };
        folded.push(*frame);
    }
    let started = Instant::now();
    let engaged = Arc::new(EngagedIndex::folded(&folded, perspective.country.clone(), None));
    let ms = started.elapsed().as_secs_f64() * 1e3;
    let hours = folded.len() as u32;
    println!(
        "UNIT_ENGAGED_FOLD hours={hours} ms={ms:.6} per_hour_ms={:.6}",
        ms / f64::from(hours)
    );
    for (camera_name, height) in [("close", 70.0), ("mid", 339.0), ("world", 2400.0)] {
        let camera = Placement {
            center: [3009.9868, 1537.9652],
            height,
            pose: viewer_gpu::camera::Pose::Map,
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
    }
}
