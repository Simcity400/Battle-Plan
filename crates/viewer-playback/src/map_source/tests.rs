use super::*;
use crate::playback::tests::fixture;
use replay::campaign::SeekResult;

fn source(replay: Replay) -> (Arc<Replay>, MapSource) {
    let replay = Arc::new(replay);
    let natives = Arc::new(Natives::new(replay.recording()).unwrap());
    let source = Columnar::new(natives).open(Arc::clone(&replay)).unwrap();
    (replay, source)
}

fn read(source: &mut MapSource, hour: Hour) -> Current {
    source.read(1, hour, true, &|| false).unwrap()
}

#[test]
fn an_hour_a_run_holds_is_the_reader_s_frame_and_never_a_gap() {
    let (replay, mut source) = source(fixture());
    let tables = dashboard_base::map_tables::selected_tables();
    let hours = replay.recorded_hours(1).unwrap();
    assert!(hours.len() > 2, "{hours:?}");
    for &hour in hours.iter().chain(hours.iter().rev()) {
        let SeekResult::Frame(expected) = replay.seek_selected(1, hour, Some(tables)).unwrap() else {
            panic!("the reader holds no frame of hour {hour}");
        };
        match read(&mut source, hour) {
            Current::Frame(frame) => assert!(*frame == *expected, "hour {hour}'s frame is not the reader's"),
            other => panic!("hour {hour}: {other:?}"),
        }
    }
    for hour in [99, 102, 103, 500, 821] {
        if hours.contains(&hour) {
            continue;
        }
        let reason = crate::map_source::gap_reason(&replay, 1, hour);
        assert_eq!(read(&mut source, hour), Current::Gap { branch: 1, hour, reason });
    }
}

#[test]
fn a_cursor_error_is_the_hour_s_failure_by_name_not_a_gap() {
    let clean = fixture();
    let damaged = Replay::open(clean.damaged_block(&clean.index[0]).unwrap()).unwrap();
    let tables = dashboard_base::map_tables::selected_tables();
    let block = damaged.index[0];
    let hour = block.first_hour;
    let error = match damaged.seek_selected(block.branch, hour, Some(tables)) {
        Err(error) => error.to_string(),
        Ok(other) => panic!("the damaged block's hour {hour} reads: {other:?}"),
    };
    let later = *damaged.index.last().unwrap();
    let SeekResult::Frame(expected) = damaged.seek_selected(later.branch, later.last_hour, Some(tables)).unwrap() else {
        panic!("the last block holds no frame");
    };
    let (_, mut source) = source(damaged);
    for _ in 0..2 {
        match source.read(block.branch, hour, true, &|| false).unwrap() {
            Current::Refused { branch, hour: at, reason } => {
                assert_eq!((branch, at), (block.branch, hour));
                assert!(reason.contains(&error), "{reason} does not carry {error}");
            }
            other => panic!("hour {hour}: {other:?}"),
        }
    }
    match source.read(later.branch, later.last_hour, true, &|| false).unwrap() {
        Current::Frame(frame) => assert!(*frame == *expected),
        other => panic!("hour {}: {other:?}", later.last_hour),
    }
}

fn fixture_source(name: &str) -> (Arc<Replay>, MapSource) {
    let path = std::env::var_os(name).unwrap_or_else(|| panic!("{name} names no recording"));
    source(Replay::open_path(std::path::Path::new(&path)).unwrap())
}

#[test]
#[ignore = "reads a recording named by BATTLEPLAN_MAP_FIXTURE at the hour BATTLEPLAN_MAP_FIXTURE_HOUR"]
fn a_recorded_hour_through_the_map_source_is_the_row_frames() {
    let (replay, mut source) = fixture_source("BATTLEPLAN_MAP_FIXTURE");
    let at = std::env::var("BATTLEPLAN_MAP_FIXTURE_HOUR").or_else(|_| std::env::var("HOI4_MAP_FIXTURE_HOUR")).expect("BATTLEPLAN_MAP_FIXTURE_HOUR names no hour");
    let (branch, hour) = at.split_once(':').expect("branch:hour");
    let (branch, hour): (BranchId, Hour) = (branch.parse().unwrap(), hour.parse().unwrap());
    let tables = dashboard_base::map_tables::selected_tables();
    let SeekResult::Frame(expected) = replay.seek_selected(branch, hour, Some(tables)).unwrap() else {
        panic!("the row frames hold no frame of hour {hour} of branch {branch}");
    };
    let started = std::time::Instant::now();
    let read = source.read(branch, hour, true, &|| false).unwrap();
    let elapsed = started.elapsed();
    let Current::Frame(frame) = read else { panic!("hour {hour} of branch {branch}: {read:?}") };
    assert!(*frame == *expected, "hour {hour} of branch {branch}'s frame is not the row frames'");
    let tables_read = frame.tables.iter().filter(|view| !matches!(&***view, TableView::NotRecorded { .. })).count();
    println!("branch {branch} hour {hour}: a frame equal to the row frames' ({} of the map's tables, {tables_read} views not 'not recorded'), read in {:.1} ms", tables.len(), elapsed.as_secs_f64() * 1e3);
}

#[test]
#[ignore = "reads the recordings named by BATTLEPLAN_MAP_FIXTURES and prints timings"]
fn map_source_steps_and_seeks_beside_the_hour_cursor() {
    let paths = std::env::var("BATTLEPLAN_MAP_FIXTURES").or_else(|_| std::env::var("HOI4_MAP_FIXTURES")).expect("BATTLEPLAN_MAP_FIXTURES names no recording");
    let tables = dashboard_base::map_tables::selected_tables();
    for path in paths.split(';').filter(|path| !path.is_empty()) {
        let replay = Arc::new(Replay::open_path(std::path::Path::new(path)).unwrap());
        let mut source = Columnar::new(Arc::new(Natives::new(replay.recording()).unwrap())).open(Arc::clone(&replay)).unwrap();
        let mut cursor = replay::campaign::HourCursor::new(Arc::clone(&replay), tables, false);
        let mut held = Vec::new();
        for block in &replay.index {
            held.extend((block.first_hour..=block.last_hour).map(|hour| (block.branch, hour)));
        }
        let mut compare = |branch: BranchId, hour: Hour, times: &mut [f64; 2]| {
            let started = std::time::Instant::now();
            let read = source.read(branch, hour, true, &|| false).unwrap();
            times[0] += started.elapsed().as_secs_f64();
            let started = std::time::Instant::now();
            let SeekResult::Frame(expected) = cursor.seek(branch, hour).unwrap() else { panic!("HourCursor holds no hour {hour}") };
            times[1] += started.elapsed().as_secs_f64();
            let Current::Frame(frame) = read else { panic!("{path}: hour {hour} of branch {branch}: {read:?}") };
            assert!(*frame == *expected, "{path}: hour {hour} of branch {branch} differs");
        };
        let mut stepped = [0.0; 2];
        for &(branch, hour) in &held {
            compare(branch, hour, &mut stepped);
        }
        let mut sought = [0.0; 2];
        let mut seed = 5_u64;
        for _ in 0..60 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let (branch, hour) = held[(seed >> 33) as usize % held.len()];
            compare(branch, hour, &mut sought);
        }
        let ms = |total: f64, count: usize| total * 1e3 / count as f64;
        println!(
            "{path}: {} map tables; {} hours stepped, 0 gaps, every frame HourCursor's: map source {:.3} ms/hour, HourCursor {:.3} ms/hour; 60 seeks: map source {:.1} ms/seek, HourCursor {:.1} ms/seek",
            tables.len(),
            held.len(),
            ms(stepped[0], held.len()),
            ms(stepped[1], held.len()),
            ms(sought[0], 60),
            ms(sought[1], 60),
        );
    }
}
