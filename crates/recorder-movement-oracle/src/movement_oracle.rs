use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use engine_dlc::{cdlc_manager, cdlc_manager::mounts_match};
use engine_executable::build::hash_executable;
use engine_native_memory::{capture_text::Owned as Descriptor, walk::Outcome};
use engine_native::cprovince::definition::{MovementCensus, read_movement_census};
use content_reader_layers::content::{GatherOptions, gather};
use content_reader_map::movement::MovementMap;
use serde_json::{Value, json};

use crate::windows::{self, Process, ProcessMemory};
use recorder_platform::games::{Discovery, choose_game};

pub const USAGE: &str =
    "recorder movement-oracle [--user-dir <directory>] [--report <file.json>] [--show <n>]";

pub fn command(args: Vec<OsString>) -> Result<(), String> {
    let started = Instant::now();
    let mut user = None;
    let mut report_path = None;
    let mut show = 20_usize;
    let mut args = args.into_iter();
    while let Some(option) = args.next() {
        if option == "--help" || option == "-h" {
            println!("{USAGE}");
            return Ok(());
        }
        let value = args.next().ok_or(USAGE)?;
        match option.to_str() {
            Some("--user-dir") if user.is_none() => user = Some(PathBuf::from(value)),
            Some("--report") if report_path.is_none() => report_path = Some(PathBuf::from(value)),
            Some("--show") => {
                show = value
                    .to_str()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--show requires a count")?
            }
            _ => return Err(USAGE.into()),
        }
    }
    let process = loop {
        let game = match choose_game(&windows::game_processes()?) {
            Discovery::One(game) => game,
            Discovery::None => return Err("no hoi4.exe is running; load a save first".into()),
            Discovery::Several(several) => return Err(several.to_string()),
        };
        if let Some(process) = Process::open_selected(&game, false)? {
            break process;
        }
    };
    let pid = process.pid;
    engine_executable::executable_tables::initialize_executable(&process.path)
        .map_err(|error| format!("Cannot prepare game tables: {error}"))?;
    let file = fs::read(&process.path).map_err(|e| e.to_string())?;
    let exe = hash_executable(file.as_slice()).map_err(|e| e.to_string())?;
    let name = process
        .path
        .file_name()
        .ok_or("game executable has no name")?;
    let base = windows::modules(pid)?
        .into_iter()
        .find(|module| module.path.file_name() == Some(name))
        .ok_or("game module not found")?
        .base;
    engine_sites::install_file(&file, &exe.sha256, base)
        .map_err(|w| w.to_string())?
        .layouts()
        .map_err(|withheld| format!("the movement oracle is withheld: {withheld}"))?;
    let user = match user {
        Some(user) => user,
        None => crate::directories::resolve_user_directory()?,
    };
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is unset")?;
    let content = gather(&GatherOptions {
        game_dir: process
            .path
            .parent()
            .ok_or("game executable has no folder")?
            .to_owned(),
        user_dir: user.clone(),
        cache_dir: local.join("BattlePlan").join("recorder"),
        dlc_order: None,
    })
    .map_err(|e| e.to_string())?;
    println!(
        "Content: {} files, user {}, {} mods.",
        content.stats.files,
        user.display(),
        content.mods.len()
    );
    let memory = ProcessMemory(&process.handle);
    let mut mounts =
        vec![Descriptor::default(); cdlc_manager::observe_counts(&memory, base).mounts];
    let outcome = cdlc_manager::observe_search_path_owned(&memory, base, &mut mounts, true);
    verify_content_order(
        content.order_note.as_deref(),
        outcome,
        &mounts,
        &content.expected_mounts,
    )?;
    let defines = content_reader_defines::defines::read(&content.layers).map_err(|e| e.to_string())?;
    let map = MovementMap::read(&content.layers, &defines)
        .map_err(|e| format!("movement map refused: {e}"))?;
    let census =
        read_movement_census(&memory, base).map_err(|e| format!("game province census: {e}"))?;
    let report = compare(&census, &map);
    report.print(show);
    let report_path = match report_path {
        Some(path) => path,
        None => crate::directories::default_capture_directory()
            .join("movement_oracle")
            .join(format!(
                "movement-oracle-{}.json",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs()
            )),
    };
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut value = report.json();
    value["process"] = json!({"pid": pid, "path": process.path, "module_base": base});
    fs::write(
        &report_path,
        serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{}: {e}", report_path.display()))?;
    println!(
        "Report: {} ({:.1} s)",
        report_path.display(),
        started.elapsed().as_secs_f64()
    );
    report.outcome()
}

fn verify_content_order(
    order_note: Option<&str>,
    outcome: Outcome,
    observed: &[Descriptor],
    expected: &[String],
) -> Result<(), String> {
    let refuse = |reason: String| Err(format!("content load order unverified: {reason}"));
    if let Some(note) = order_note {
        return refuse(note.to_owned());
    }
    match outcome {
        Outcome::Rows { len } if mounts_match(&observed[..len], expected) => Ok(()),
        Outcome::Rows { len } => refuse(format!(
            "the game's {len} successful mounts differ from the content stack's {} in membership or priority order",
            expected.len()
        )),
        Outcome::Truncated { len, unread_rows } => refuse(format!(
            "the game's search path grew while read ({len} read, {unread_rows} unread)"
        )),
        Outcome::Unavailable { reason } => refuse(format!("game search path {reason}")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Difference {
    province: u32,
    entry: Option<usize>,
    field: &'static str,
    game: String,
    content: String,
}

#[derive(Debug, Default)]
struct Report {
    census_records: u32,
    content_records: usize,
    provinces: usize,
    entries: usize,
    unreadable: Vec<(u32, &'static str)>,
    counts: BTreeMap<&'static str, usize>,
    differences: Vec<Difference>,
}

impl Report {
    fn differ(
        &mut self,
        province: u32,
        entry: Option<usize>,
        field: &'static str,
        game: impl ToString,
        content: impl ToString,
    ) {
        *self.counts.entry(field).or_default() += 1;
        self.differences.push(Difference {
            province,
            entry,
            field,
            game: game.to_string(),
            content: content.to_string(),
        });
    }

    fn total(&self) -> usize {
        self.counts.values().sum()
    }

    fn print(&self, show: usize) {
        println!(
            "Movement oracle: {} game records, {} content records; {} provinces and {} neighbour entries compared.",
            self.census_records, self.content_records, self.provinces, self.entries
        );
        println!(
            "Unreadable provinces: {}; differences: {}.",
            self.unreadable.len(),
            self.total()
        );
        for (field, count) in &self.counts {
            println!("  {field}: {count}");
        }
        for (province, reason) in self.unreadable.iter().take(show) {
            println!("  unreadable province {province}: {reason}");
        }
        for difference in self.differences.iter().take(show) {
            println!(
                "  province {}{} {}: game {} / content {}",
                difference.province,
                difference
                    .entry
                    .map_or(String::new(), |entry| format!(" entry {entry}")),
                difference.field,
                difference.game,
                difference.content
            );
        }
    }

    fn json(&self) -> Value {
        json!({
            "census_records": self.census_records,
            "content_records": self.content_records,
            "provinces_compared": self.provinces,
            "entries_compared": self.entries,
            "unreadable": self.unreadable.iter().map(|(province, reason)| json!({"province": province, "reason": reason})).collect::<Vec<_>>(),
            "difference_counts": self.counts,
            "differences_total": self.total(),
            "differences": self.differences.iter().map(|d| json!({"province": d.province, "entry": d.entry, "field": d.field, "game": d.game, "content": d.content})).collect::<Vec<_>>(),
        })
    }

    fn outcome(&self) -> Result<(), String> {
        if self.total() == 0 && self.unreadable.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "movement oracle failed: {} differences, {} unreadable provinces",
                self.total(),
                self.unreadable.len()
            ))
        }
    }
}

fn compare(census: &MovementCensus, map: &MovementMap) -> Report {
    let mut report = Report {
        census_records: census.records,
        content_records: map.records(),
        ..Default::default()
    };
    if census.records as usize != map.records() {
        report.differ(0, None, "record_count", census.records, map.records());
    }
    for (index, result) in census.provinces.iter().enumerate() {
        let province = index as u32 + 1;
        let definition = match result {
            Ok(definition) => definition,
            Err(reason) => {
                report.unreadable.push((province, reason));
                continue;
            }
        };
        let Some(derived) = map.neighbour_records(province) else {
            report.differ(province, None, "province", "present", "unknown");
            continue;
        };
        let derived: Vec<_> = derived.collect();
        report.provinces += 1;
        match map.terrain(province) {
            Some(terrain) => {
                if terrain.flags != definition.terrain_flags {
                    report.differ(
                        province,
                        None,
                        "terrain_flags",
                        definition.terrain_flags,
                        terrain.flags,
                    );
                }
                if terrain.index != definition.terrain_index {
                    report.differ(
                        province,
                        None,
                        "terrain_index",
                        definition.terrain_index,
                        terrain.index,
                    );
                }
            }
            None => report.differ(
                province,
                None,
                "unclaimed_terrain",
                format!(
                    "flags {} ordinal {}",
                    definition.terrain_flags, definition.terrain_index
                ),
                "not derived: no definition row claims this record",
            ),
        }
        if definition.neighbours.len() != derived.len() {
            report.differ(
                province,
                None,
                "neighbour_count",
                definition.neighbours.len(),
                derived.len(),
            );
        }
        for (entry, (game, content)) in definition.neighbours.iter().zip(&derived).enumerate() {
            report.entries += 1;
            let at = Some(entry);
            if game.id != content.to {
                report.differ(province, at, "neighbour_id", game.id, content.to);
                continue;
            }
            if Some(game.crossing_kind) != content.kind {
                report.differ(
                    province,
                    at,
                    "crossing_kind",
                    game.crossing_kind,
                    content
                        .kind
                        .map_or_else(|| "unavailable".into(), |kind| kind.to_string()),
                );
            }
            if game.flag != u8::from(content.no_midpoint) {
                report.differ(
                    province,
                    at,
                    "midpoint_flag",
                    game.flag,
                    u8::from(content.no_midpoint),
                );
            }
            match content.distance {
                Some(distance) if distance == game.distance => {}
                Some(distance) => report.differ(province, at, "distance", game.distance, distance),
                None => {
                    let pair = (province.min(content.to), province.max(content.to));
                    let reason = map
                        .unavailable_edges()
                        .get(&pair)
                        .map_or("no reason recorded", String::as_str);
                    report.differ(
                        province,
                        at,
                        "distance",
                        game.distance,
                        format!("unavailable: {reason}"),
                    );
                }
            }
            if game.crossing != content.crossing {
                report.differ(
                    province,
                    at,
                    "crossing_point",
                    format!("{:?}", game.crossing),
                    format!("{:?}", content.crossing),
                );
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::{
        cgame_state,
        cprovince::{
            self,
            definition::{self as definition, MovementDefinition},
        },
    };
    use engine_native_parts::{
        cterrain_type,
        province_neighbour::{self as neighbour, Record},
    };
    use content_reader_map::movement::{Neighbour, ProvinceTerrain};

    fn record(id: u32, kind: u8, flag: u8, distance: i64, crossing: [i32; 2]) -> Record {
        Record {
            id,
            crossing_kind: kind,
            flag,
            distance,
            crossing,
        }
    }

    fn map() -> MovementMap {
        let land = ProvinceTerrain { flags: 9, index: 2 };
        MovementMap::from_parts(&[
            (
                1,
                land,
                vec![
                    Neighbour {
                        to: 2,
                        distance: Some(233_333),
                        kind: 2,
                        midpoint: None,
                    },
                    Neighbour {
                        to: 3,
                        distance: Some(100_000),
                        kind: 0,
                        midpoint: Some([40, 80]),
                    },
                ],
            ),
            (
                2,
                ProvinceTerrain { flags: 1, index: 3 },
                vec![Neighbour {
                    to: 1,
                    distance: Some(233_333),
                    kind: 2,
                    midpoint: None,
                }],
            ),
        ])
    }

    fn census() -> MovementCensus {
        MovementCensus {
            records: 3,
            provinces: vec![
                Ok(MovementDefinition {
                    terrain_flags: 9,
                    terrain_index: 2,
                    neighbours: vec![
                        record(2, 2, 1, 233_333, [-1, 0]),
                        record(3, 0, 0, 100_000, [40, 80]),
                    ],
                }),
                Ok(MovementDefinition {
                    terrain_flags: 1,
                    terrain_index: 3,
                    neighbours: vec![record(1, 2, 1, 233_333, [-1, 0])],
                }),
            ],
        }
    }

    #[test]
    fn an_identical_census_passes() {
        let report = compare(&census(), &map());
        assert_eq!(report.total(), 0, "{:?}", report.differences);
        assert_eq!((report.provinces, report.entries), (2, 3));
        assert!(report.outcome().is_ok());
    }

    #[test]
    fn every_stored_field_difference_is_counted_and_fails() {
        let mut changed = census();
        let first = changed.provinces[0].as_mut().unwrap();
        first.terrain_flags = 13;
        first.terrain_index = 5;
        first.neighbours[0].crossing_kind = 3;
        first.neighbours[0].flag = 2;
        first.neighbours[0].distance = 233_334;
        first.neighbours[1].crossing = [41, 80];
        let second = changed.provinces[1].as_mut().unwrap();
        second.neighbours[0].id = 4;
        second.neighbours.push(record(3, 0, 0, 0, [0, 0]));
        let report = compare(&changed, &map());
        let counts: Vec<_> = report.counts.iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(
            counts,
            [
                ("crossing_kind", 1),
                ("crossing_point", 1),
                ("distance", 1),
                ("midpoint_flag", 1),
                ("neighbour_count", 1),
                ("neighbour_id", 1),
                ("terrain_flags", 1),
                ("terrain_index", 1),
            ]
        );
        assert_eq!(
            report.differences[0],
            Difference {
                province: 1,
                entry: None,
                field: "terrain_flags",
                game: "13".into(),
                content: "9".into(),
            }
        );
        assert!(report.outcome().is_err());
    }

    #[test]
    fn unreadable_unknown_unavailable_and_miscounted_records_fail() {
        let mut changed = census();
        changed.records = 4;
        changed.provinces[1] = Err("definition identity mismatch");
        changed.provinces.push(Ok(MovementDefinition {
            terrain_flags: 5,
            terrain_index: 0,
            neighbours: vec![record(1, 0, 0, 100_000, [40, 80])],
        }));
        let mut map = map();
        let report = compare(&changed, &map);
        assert_eq!(report.unreadable, [(2, "definition identity mismatch")]);
        assert_eq!(report.counts["record_count"], 1);
        assert_eq!(report.counts["province"], 1);
        assert!(report.outcome().is_err());

        map = MovementMap::from_parts(&[(
            1,
            ProvinceTerrain { flags: 9, index: 2 },
            vec![Neighbour {
                to: 2,
                distance: None,
                kind: 2,
                midpoint: None,
            }],
        )]);
        let report = compare(
            &MovementCensus {
                records: 2,
                provinces: vec![Ok(MovementDefinition {
                    terrain_flags: 9,
                    terrain_index: 2,
                    neighbours: vec![record(2, 2, 1, 233_333, [-1, 0])],
                })],
            },
            &map,
        );
        assert_eq!(report.counts["distance"], 1);
        assert!(report.differences[0].content.starts_with("unavailable"));
    }

    #[test]
    fn an_unclaimed_record_fails_on_terrain_and_still_compares_its_entries() {
        let land = ProvinceTerrain { flags: 9, index: 2 };
        let edge = |to| Neighbour {
            to,
            distance: Some(100_000),
            kind: 0,
            midpoint: Some([40, 80]),
        };
        let map =
            MovementMap::from_parts(&[(1, Some(land), vec![edge(2)]), (2, None, vec![edge(1)])]);
        let entry = record(1, 0, 0, 100_000, [40, 80]);
        let census = MovementCensus {
            records: 3,
            provinces: vec![
                Ok(MovementDefinition {
                    terrain_flags: 9,
                    terrain_index: 2,
                    neighbours: vec![record(2, 0, 0, 100_000, [40, 80])],
                }),
                Ok(MovementDefinition {
                    terrain_flags: 5,
                    terrain_index: 0,
                    neighbours: vec![entry],
                }),
            ],
        };
        let report = compare(&census, &map);
        assert_eq!((report.provinces, report.entries), (2, 2));
        assert_eq!(report.total(), 1, "{:?}", report.differences);
        assert_eq!(
            report.differences[0],
            Difference {
                province: 2,
                entry: None,
                field: "unclaimed_terrain",
                game: "flags 5 ordinal 0".into(),
                content: "not derived: no definition row claims this record".into(),
            }
        );
        assert!(report.outcome().is_err());

        let mut changed = census;
        changed.provinces[1].as_mut().unwrap().neighbours[0].distance = 100_001;
        assert_eq!(compare(&changed, &map).counts["distance"], 1);
    }

    #[test]
    fn an_unproved_content_load_order_is_refused_before_comparing() {
        let mount = |path: &[u8]| {
            let mut text = Descriptor::default();
            text.bytes.extend_from_slice(path);
            text.len = path.len();
            text
        };
        let observed = [
            mount(b"C:/user"),
            mount(b"C:/game/dlc/d1"),
            mount(b"C:/game"),
        ];
        let expected: Vec<String> = [r"C:\user", r"C:\game\dlc\d1", r"C:\game"]
            .map(String::from)
            .into();
        let rows = Outcome::Rows { len: 3 };
        assert_eq!(
            verify_content_order(None, rows, &observed, &expected),
            Ok(())
        );
        let refused = |result: Result<(), String>, reason: &str| {
            let error = result.unwrap_err();
            assert!(
                error.starts_with("content load order unverified") && error.contains(reason),
                "{error}"
            );
        };
        refused(
            verify_content_order(Some("mods not measured"), rows, &observed, &expected),
            "mods not measured",
        );
        let mut swapped = expected.clone();
        swapped.swap(0, 1);
        refused(
            verify_content_order(None, rows, &observed, &swapped),
            "priority order",
        );
        refused(
            verify_content_order(None, Outcome::Rows { len: 2 }, &observed, &expected),
            "membership",
        );
        refused(
            verify_content_order(
                None,
                Outcome::Truncated {
                    len: 3,
                    unread_rows: 1,
                },
                &observed,
                &expected,
            ),
            "grew",
        );
        refused(
            verify_content_order(
                None,
                Outcome::Unavailable {
                    reason: "search path cache unreadable or not constructed",
                },
                &observed,
                &expected,
            ),
            "not constructed",
        );
        refused(
            verify_content_order(None, Outcome::Rows { len: 0 }, &[], &[]),
            "membership",
        );
    }

    #[test]
    fn process_memory_reads_a_synthetic_census_end_to_end() {
        let mut image = vec![0_u8; 0x20_000];
        let start = image.as_ptr() as usize;
        let base = start
            .checked_sub(cgame_state::SINGLETON_RVA.0)
            .expect("heap image above the module RVA");
        let mut put = |at: usize, bytes: &[u8]| {
            image[at..at + bytes.len()].copy_from_slice(bytes);
        };
        let absolute = |offset: usize| (start + offset) as u64;
        let (game_state, census_data, terrains) = (0x100, 0x200, 0x300);
        put(0, &absolute(game_state).to_le_bytes());
        put(
            game_state + cgame_state::PROVINCES,
            &absolute(census_data).to_le_bytes(),
        );
        put(
            game_state + cgame_state::PROVINCES + 8,
            &[3, 0, 0, 0, 3, 0, 0, 0],
        );
        let expected = census();
        for (slot, result) in expected.provinces.iter().enumerate() {
            let id = slot as u32 + 1;
            let definition_row = result.as_ref().unwrap();
            let province = 0x1000 * (2 * slot + 2);
            let definition = province + 0x400;
            let entries = province + 0x800;
            let terrain = terrains + slot * 0x100;
            put(
                census_data + 8 * id as usize,
                &absolute(province).to_le_bytes(),
            );
            put(
                province,
                &((base + cprovince::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(province + cprovince::ID, &id.to_le_bytes());
            put(
                province + cprovince::DEFINITION,
                &absolute(definition).to_le_bytes(),
            );
            put(definition + definition::ID, &id.to_le_bytes());
            put(
                definition + definition::TERRAIN_FLAGS,
                &[definition_row.terrain_flags],
            );
            put(
                definition + definition::TERRAIN,
                &absolute(terrain).to_le_bytes(),
            );
            put(
                terrain,
                &((base + cterrain_type::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                terrain + cterrain_type::DATABASE_INDEX,
                &definition_row.terrain_index.to_le_bytes(),
            );
            let count = definition_row.neighbours.len() as u32;
            put(
                definition + definition::NEIGHBOURS,
                &absolute(entries).to_le_bytes(),
            );
            put(
                definition + definition::NEIGHBOURS + 8,
                &count.to_le_bytes(),
            );
            put(
                definition + definition::NEIGHBOURS + 12,
                &count.to_le_bytes(),
            );
            for (index, row) in definition_row.neighbours.iter().enumerate() {
                let at = entries + index * neighbour::STRIDE;
                put(at + neighbour::PROVINCE_ID, &row.id.to_le_bytes());
                put(
                    at + neighbour::CROSSING_KIND,
                    &[row.crossing_kind, row.flag],
                );
                put(
                    at + neighbour::MOVEMENT_DISTANCE,
                    &row.distance.to_le_bytes(),
                );
                put(
                    at + neighbour::CROSSING_POINT,
                    &row.crossing[0].to_le_bytes(),
                );
                put(
                    at + neighbour::CROSSING_POINT + 4,
                    &row.crossing[1].to_le_bytes(),
                );
            }
        }
        let process = Process::open(std::process::id(), false).unwrap();
        let read = read_movement_census(&ProcessMemory(&process.handle), base).unwrap();
        assert_eq!(read, expected);
        assert_eq!(compare(&read, &map()).total(), 0);
        let mut byte = [0];
        assert!(!engine_native_memory::Memory::copy(
            &ProcessMemory(&process.handle),
            8,
            &mut byte
        ));
        drop(image);
    }
}
