//! A real-recording oracle comparing fresh single-worker and concurrent aircraft claims.
use super::*;
use crate::document::{Content, Document, Hour as HourView, InitialHour};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::sync::{Mutex, mpsc};
use viewer_foundation::map::perspective;
use viewer_world::map::{aircraft::AircraftAssets, models::AssetKey};

struct Output(File);

impl Output {
    fn emit(&mut self, row: &Value) {
        serde_json::to_writer(&mut self.0, row).unwrap();
        writeln!(self.0).unwrap();
        self.0.flush().unwrap();
    }
}

struct Built {
    branch: u64,
    hour: i32,
    lines: Result<Vec<String>, String>,
}

fn build(content: &Content, frame: &DecodedFrame) -> Built {
    let view = HourView::new(content, frame, 0.0);
    Built {
        branch: frame.branch,
        hour: frame.hour,
        lines: view
            .aircraft_plan(&perspective::Choice::default())
            .map(|plan| plan.plan_report(&content.aircraft))
            .map_err(|error| error.to_string()),
    }
}

fn targets(path: &Path, grid: i32, limit: usize, mut visit: impl FnMut(usize, DecodedFrame)) {
    let replay = Replay::open_path(path).unwrap();
    let first = replay
        .index
        .first()
        .expect("recording has no blocks")
        .clone();
    let selected = dashboard_base::prepared_map::selected_tables();
    let mut decoder = FrameDecoder::for_tables(selected);
    let mut count = 0;
    for index in replay
        .index
        .iter()
        .take_while(|index| index.branch == first.branch)
    {
        for frame in &replay.read_block_selected(index, selected).unwrap() {
            let decoded = decoder.advance(frame).unwrap();
            if (i64::from(decoded.hour) - i64::from(first.first_hour)) % i64::from(grid) != 0 {
                continue;
            }
            if count == limit {
                return;
            }
            visit(count, decoded.clone());
            count += 1;
        }
    }
}

fn load(path: &Path, local: &Path) -> Document {
    Document::load_shared(
        path,
        &installed_content_core::ResolveOptions {
            game_dir: PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("set BATTLE_PLAN_GAME_DIR")),
            user_dir: PathBuf::from(
                std::env::var_os("BATTLE_PLAN_MAP_CENSUS_USER_DIR").or_else(|| std::env::var_os("HOI4_MAP_CENSUS_USER_DIR")).expect("set BATTLE_PLAN_MAP_CENSUS_USER_DIR"),
            ),
            cache_dir: local.join("cache"),
        },
        false,
        &InitialHour {
            branch: None,
            hour: None,
            hour_fraction: 0.0,
        },
        Arc::new(Replay::open_path(path).unwrap()),
        None,
        0,
        None,
    )
    .unwrap()
}

#[derive(Debug, PartialEq)]
struct Library {
    actors: BTreeMap<AssetKey, Vec<String>>,
    native_draws: BTreeMap<AssetKey, String>,
    textures: Vec<String>,
    materials: Vec<[String; 3]>,
    failed: BTreeMap<String, String>,
    animation_refusals: BTreeMap<String, String>,
    decode: String,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sorted<T: Ord>(mut values: Vec<T>) -> Vec<T> {
    values.sort();
    values
}

// Normalize merge-local bindings and palette offsets, but retain authored order inside each actor.
fn library(assets: &AircraftAssets) -> Library {
    let library = assets.library();
    let textures: Vec<_> = library
        .textures
        .iter()
        .zip(&library.spaces)
        .map(|(texture, space)| {
            let mut hash = Sha256::new();
            hash.update(
                format!(
                    "{:?}:{:?}:{}:{}:{}",
                    space, texture.format, texture.width, texture.height, texture.faces
                )
                .as_bytes(),
            );
            for face in &texture.mips {
                hash.update(face.len().to_le_bytes());
                for mip in face {
                    hash.update(mip.width.to_le_bytes());
                    hash.update(mip.height.to_le_bytes());
                    hash.update(mip.data.len().to_le_bytes());
                    hash.update(&mip.data);
                }
            }
            format!("{:x}", hash.finalize())
        })
        .collect();
    assert_eq!(textures.len(), library.textures.len());
    assert_eq!(textures.len(), library.spaces.len());
    let materials = library
        .materials
        .iter()
        .map(|material| {
            [material.diffuse, material.normal, material.specular]
                .map(|slot| textures[slot].clone())
        })
        .collect();
    let mut actors = BTreeMap::new();
    let mut native_draws = BTreeMap::new();
    for (slot, mesh) in library.meshes.iter().enumerate() {
        let (rig, palette) = library.rig_source(slot);
        let mut rig = (*rig).clone();
        let start = rig.start;
        rig.start = 0;
        rig.end -= start;
        for node in &mut rig.nodes {
            node.rigid -= u32::try_from(start).unwrap();
            if let Some(base) = &mut node.skin_base {
                *base -= u32::try_from(start).unwrap();
            }
        }
        let palette: Vec<_> = palette
            .iter()
            .map(|joint| joint.rows.map(|row| row.map(f32::to_bits)))
            .collect();
        let bound = library
            .index
            .iter()
            .filter(|(_, at)| **at == slot)
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        let counterpart = library
            .alternatives
            .get(&slot)
            .map(|&at| &library.meshes[at].asset);
        let row = vec![
            format!("{:?}", (&mesh.name, &mesh.path)),
            format!(
                "{:?}",
                [
                    mesh.scale,
                    mesh.radius,
                    mesh.height,
                    mesh.own_radius,
                    mesh.bound
                ]
                .map(f32::to_bits)
            ),
            format!(
                "{:?}",
                mesh.lod_distances
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>()
            ),
            format!(
                "{:?}",
                (
                    mesh.visual,
                    mesh.attachments,
                    mesh.convoy_radius.map(f32::to_bits),
                    library.has_geometry(slot)
                )
            ),
            digest(format!("{rig:?}").as_bytes()),
            format!("{palette:?}"),
            format!("{bound:?}"),
            format!("{counterpart:?}"),
        ];
        assert!(
            native_draws
                .insert(
                    mesh.asset.clone(),
                    digest(&library.native_draw_snapshot(slot, &textures)),
                )
                .is_none(),
            "each aircraft native draw asset bakes once"
        );
        assert!(
            actors.insert(mesh.asset.clone(), row).is_none(),
            "each aircraft asset bakes once"
        );
    }
    Library {
        actors,
        native_draws,
        textures: sorted(textures),
        materials: sorted(materials),
        failed: library.failed.clone(),
        animation_refusals: library.animation_refusals.clone(),
        decode: format!("{:?}", library.decode()),
    }
}

struct Pass {
    built: Vec<Built>,
    refusals: Vec<String>,
    lazy: Vec<String>,
    library: Library,
    at_load: usize,
}

fn pass((path, local): (&Path, &Path), (grid, limit): (i32, usize), threads: usize) -> Pass {
    let document = load(path, local);
    let content: &Content = &document.content;
    let at_load = content.aircraft.library().meshes.len();
    let mut built = Vec::new();
    if threads == 1 {
        targets(path, grid, limit, |index, frame| {
            built.push((index, build(content, &frame)))
        });
    } else {
        let (sender, receiver) = mpsc::sync_channel::<(usize, DecodedFrame)>(threads);
        let receiver = Mutex::new(receiver);
        let done = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let next = receiver.lock().unwrap().recv();
                        let Ok((index, frame)) = next else { break };
                        let one = build(content, &frame);
                        done.lock().unwrap().push((index, one));
                    }
                });
            }
            targets(path, grid, limit, |index, frame| {
                sender.send((index, frame)).unwrap()
            });
            drop(sender);
        });
        built = done.into_inner().unwrap();
    }
    built.sort_by_key(|(index, _)| *index);
    let aircraft = &content.aircraft;
    Pass {
        built: built.into_iter().map(|(_, one)| one).collect(),
        refusals: sorted(aircraft.refusals()),
        lazy: sorted(
            aircraft
                .lazy_diagnostics()
                .into_iter()
                .map(|(kind, line)| format!("{kind:?} {line}"))
                .collect(),
        ),
        library: library(aircraft),
        at_load,
    }
}

fn difference(one: &Built, expected: &Built) -> Value {
    match (&one.lines, &expected.lines) {
        (Ok(lines), Ok(reference)) => {
            let at = lines
                .iter()
                .zip(reference)
                .position(|(line, expected)| line != expected)
                .unwrap_or(lines.len().min(reference.len()));
            json!({"line": at, "pass": lines.get(at), "reference": reference.get(at),
                "lines": lines.len(), "referenceLines": reference.len()})
        }
        (lines, reference) => {
            json!({"pass": lines.as_ref().err(), "reference": reference.as_ref().err()})
        }
    }
}

#[test]
#[ignore = "needs BATTLE_PLAN_MAP_BENCH_REPLAY, BATTLE_PLAN_MAP_BENCH_EVIDENCE, BATTLE_PLAN_GAME_DIR, BATTLE_PLAN_MAP_CENSUS_USER_DIR and private LOCALAPPDATA"]
fn aircraft_resolve_oracle() {
    engine_executable::executable_tables::initialize_for_tests().expect("set BATTLE_PLAN_GAME_DIR");
    let path = PathBuf::from(std::env::var_os("BATTLE_PLAN_MAP_BENCH_REPLAY").or_else(|| std::env::var_os("HOI4_MAP_BENCH_REPLAY")).expect("set replay"))
        .canonicalize()
        .unwrap();
    let evidence = PathBuf::from(
        std::env::var_os("BATTLE_PLAN_MAP_BENCH_EVIDENCE").or_else(|| std::env::var_os("HOI4_MAP_BENCH_EVIDENCE")).expect("set evidence outside Git"),
    );
    std::fs::create_dir_all(&evidence).unwrap();
    let evidence = evidence.canonicalize().unwrap();
    let mut worktree = std::env::current_dir().unwrap();
    while !worktree.join(".git").exists() && worktree.pop() {}
    if worktree.join(".git").exists() {
        assert!(!evidence.starts_with(worktree.canonicalize().unwrap()));
    }
    assert!(
        !path.starts_with(&evidence),
        "recording must remain outside scratch"
    );
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("set private LOCALAPPDATA"));
    std::fs::create_dir_all(&local).unwrap();
    assert!(local.canonicalize().unwrap().starts_with(&evidence));
    let source_before = SourceKey::capture(&path, &[]).unwrap();
    let number = |name: &str, default: usize| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .map_or(default, |value| {
                value
                    .parse()
                    .unwrap_or_else(|_| panic!("{name} must be a whole number"))
            })
    };
    let grid = i32::try_from(number("BATTLE_PLAN_MAP_CENSUS_GRID", 40)).expect("grid must fit i32");
    let limit = number("BATTLE_PLAN_MAP_CENSUS_TARGETS", usize::MAX);
    let threads = number(
        "BATTLE_PLAN_MAP_CENSUS_THREADS",
        std::thread::available_parallelism().map_or(2, |cores| cores.get().max(2)),
    );
    assert!(grid > 0 && limit > 0 && threads > 1);
    let mut output = Output(File::create(evidence.join("aircraft-resolve.jsonl")).unwrap());
    output.emit(
        &json!({"kind": "input", "recording": path, "grid": grid, "threads": threads,
        "passes": {"reference": "fresh single-worker claimed resolves in target order",
            "concurrent": "fresh claimed resolves shared by independent workers"}}),
    );
    let files = (path.as_path(), local.as_path());
    let reference = pass(files, (grid, limit), 1);
    let concurrent = pass(files, (grid, limit), threads);
    assert!(!reference.built.is_empty(), "oracle must visit targets");
    assert!(
        reference.built.iter().any(|one| one.lines.is_ok()),
        "oracle must build an aircraft plan"
    );
    assert!(
        reference.library.actors.len() > reference.at_load,
        "oracle must exercise a lazy bake"
    );
    assert!(
        reference
            .built
            .iter()
            .any(|one| one.lines.as_ref().is_ok_and(|lines| {
                lines
                    .first()
                    .is_some_and(|line| !line.starts_with("Aircraft: 0 generated flight slots"))
            })),
        "oracle must generate aircraft flights"
    );
    let mut mismatches = Vec::new();
    if concurrent.built.len() != reference.built.len() {
        mismatches.push("target count".to_owned());
    }
    for (one, expected) in concurrent.built.iter().zip(&reference.built) {
        if (one.branch, one.hour) != (expected.branch, expected.hour) || one.lines != expected.lines
        {
            mismatches.push(format!("branch {} hour {}", one.branch, one.hour));
            output.emit(
                &json!({"kind": "mismatch", "branch": one.branch, "hour": one.hour,
                "difference": difference(one, expected)}),
            );
        }
    }
    for (what, equal) in [
        ("refusals", concurrent.refusals == reference.refusals),
        ("lazy diagnostics", concurrent.lazy == reference.lazy),
        ("baked library", concurrent.library == reference.library),
    ] {
        if !equal {
            mismatches.push(what.to_owned());
        }
    }
    for (name, run) in [("reference", &reference), ("concurrent", &concurrent)] {
        output.emit(&json!({"kind": "summary", "pass": name, "targets": run.built.len(),
            "unbuilt": run.built.iter().filter(|one| one.lines.is_err()).count(),
            "assets": run.library.actors.len(), "lazilyBaked": run.library.actors.len() - run.at_load, "refusals": run.refusals.len(), "lazyDiagnostics": run.lazy.len()}));
    }
    output.emit(&json!({"kind": "verdict", "mismatches": mismatches}));
    assert_eq!(
        source_before,
        SourceKey::capture(&path, &[]).unwrap(),
        "input changed"
    );
    output.0.sync_all().unwrap();
    assert!(
        mismatches.is_empty(),
        "concurrent claims differed from the reference: {mismatches:?}"
    );
}
