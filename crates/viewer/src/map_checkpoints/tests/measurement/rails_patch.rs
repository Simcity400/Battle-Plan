//! Compare patched railways against full builds in all three base directions.
use super::*;
use crate::document::{Document, Hour as HourView, InitialHour};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use viewer_world::map::railways::{RailBase, RailMesh, RailNetwork};

struct Output(File);

impl Output {
    fn emit(&mut self, row: &Value) {
        serde_json::to_writer(&mut self.0, row).unwrap();
        writeln!(self.0).unwrap();
        self.0.flush().unwrap();
    }
}

fn thread_ms() -> f64 {
    viewer_foundation::profiling::current_thread_cpu_time()
        .expect("thread CPU clock")
        .as_secs_f64()
        * 1e3
}

fn full(view: &HourView<'_>, network: &RailNetwork) -> (io::Result<RailMesh>, f64) {
    let started = thread_ms();
    let mesh = view.rail_mesh_full_of(network);
    (mesh, thread_ms() - started)
}

type Patched = io::Result<(RailMesh, RailBase)>;

fn patched(
    view: &HourView<'_>,
    base: Option<&RailBase>,
    network: &Arc<RailNetwork>,
) -> (Patched, f64) {
    let started = thread_ms();
    let field = view.rail_mesh_from(base, network);
    (field, thread_ms() - started)
}

fn compare(field: Patched, reference: &io::Result<RailMesh>) -> (bool, Option<Arc<RailBase>>) {
    match (field, reference) {
        (Ok((mesh, next)), Ok(full)) => (mesh.bit_eq(full), Some(Arc::new(next))),
        (Err(patched), Err(full)) => (patched.to_string() == full.to_string(), None),
        _ => (false, None),
    }
}

#[test]
#[ignore = "lead-run release oracle; needs BATTLE_PLAN_MAP_BENCH_REPLAY, BATTLE_PLAN_MAP_BENCH_EVIDENCE, BATTLE_PLAN_GAME_DIR, BATTLE_PLAN_MAP_CENSUS_USER_DIR and a private LOCALAPPDATA"]
fn rails_patch_oracle() {
    if cfg!(debug_assertions) {
        panic!("measure with --release");
    }
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
    let grid = number("BATTLE_PLAN_MAP_CENSUS_GRID", 40) as i32;
    let limit = number("BATTLE_PLAN_MAP_CENSUS_TARGETS", usize::MAX);
    assert!(grid > 0);
    let document = Document::load_shared(
        &path,
        &installed_content_core::ResolveOptions {
            game_dir: PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap()),
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
        Arc::new(Replay::open_path(&path).unwrap()),
        None,
        0,
        None,
    )
    .unwrap();
    let replay = Replay::open_path(&path).unwrap();
    let first = replay
        .index
        .first()
        .expect("recording has no blocks")
        .clone();
    let blocks: Vec<_> = replay
        .index
        .iter()
        .take_while(|index| index.branch == first.branch)
        .cloned()
        .collect();
    let selected = dashboard_base::prepared_map::selected_tables();
    let mut output = Output(File::create(evidence.join("rails-patch.jsonl")).unwrap());
    output.emit(&json!({"kind": "input", "recording": path, "grid": grid,
        "bases": {"preceding": "the target one grid step earlier", "skipping": "the target two grid steps earlier",
            "backward": "the following target, patched into the earlier one"}}));
    struct Kept {
        network: Arc<RailNetwork>,
        full: io::Result<RailMesh>,
        base: Option<Arc<RailBase>>,
    }
    let mut kept: Vec<Kept> = Vec::new();
    let mut mismatches: BTreeMap<&str, Vec<i32>> = BTreeMap::new();
    let (mut full_cpu, mut patched_cpu, mut read_cpu) = (Vec::new(), Vec::new(), Vec::new());
    let mut comparisons = [0usize; 3];
    let mut successful_base_comparisons = [0usize; 3];
    let mut unread = 0;
    let mut decoder = FrameDecoder::for_tables(selected);
    let mut targets = 0;
    'blocks: for index in &blocks {
        let frames = replay.read_block_selected(index, selected).unwrap();
        for frame in &frames {
            let decoded = decoder.advance(frame).unwrap();
            if (decoded.hour - first.first_hour) % grid != 0 {
                continue;
            }
            if targets == limit {
                break 'blocks;
            }
            targets += 1;
            let frame = decoded.clone();
            let view = HourView::new(&document.content, &frame, 0.0);
            let read_started = thread_ms();
            let network = match RailNetwork::read(&frame) {
                Ok(network) => Arc::new(network),
                Err(error) => {
                    unread += 1;
                    output.emit(&json!({"kind": "target", "hour": frame.hour,
                        "unread": error.to_string()}));
                    continue;
                }
            };
            read_cpu.push(thread_ms() - read_started);
            let base = kept.last().and_then(|kept| kept.base.clone());
            // Alternate order so neither path always sees cold memos.
            let full_first = targets % 2 == 1;
            let ((reference, full_ms), (field, patched_ms)) = if full_first {
                let reference = full(&view, &network);
                (reference, patched(&view, base.as_deref(), &network))
            } else {
                let field = patched(&view, base.as_deref(), &network);
                (full(&view, &network), field)
            };
            let (equal, next) = compare(field, &reference);
            comparisons[0] += 1;
            let successful_base = equal && base.is_some() && next.is_some();
            successful_base_comparisons[0] += usize::from(successful_base);
            if !equal {
                mismatches.entry("preceding").or_default().push(frame.hour);
            }
            full_cpu.push(full_ms);
            patched_cpu.push(patched_ms);
            let mut row = json!({"kind": "target", "hour": frame.hour, "equal": equal,
                "baseOffered": base.is_some(), "successfulBaseComparison": successful_base,
                "fullCpuMs": full_ms, "patchedCpuMs": patched_ms, "fullFirst": full_first,
                "vertices": reference.as_ref().ok().map(|mesh| mesh.native_vertices.len()),
                "bridges": reference.as_ref().ok().map(|mesh| mesh.bridges.len())});
            if kept.len() >= 2 {
                let base = kept[kept.len() - 2].base.clone();
                let (field, _) = patched(&view, base.as_deref(), &network);
                let (equal, next) = compare(field, &reference);
                let successful_base = equal && base.is_some() && next.is_some();
                successful_base_comparisons[1] += usize::from(successful_base);
                if !equal {
                    mismatches.entry("skipping").or_default().push(frame.hour);
                }
                comparisons[1] += 1;
                row["skippingEqual"] = json!(equal);
                row["skippingBaseOffered"] = json!(base.is_some());
                row["skippingSuccessfulBaseComparison"] = json!(successful_base);
            }
            if let (Some(earlier), Some(next)) = (kept.last(), &next) {
                let (field, _) = patched(&view, Some(&**next), &earlier.network);
                let (equal, next) = compare(field, &earlier.full);
                let successful_base = equal && next.is_some();
                successful_base_comparisons[2] += usize::from(successful_base);
                if !equal {
                    mismatches.entry("backward").or_default().push(frame.hour);
                }
                comparisons[2] += 1;
                row["backwardEqual"] = json!(equal);
                row["backwardSuccessfulBaseComparison"] = json!(successful_base);
            }
            drop(view);
            output.emit(&row);
            kept.push(Kept {
                network,
                full: reference,
                base: next,
            });
            if kept.len() > 2 {
                kept.remove(0);
            }
        }
    }
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
    let summary = json!({"kind": "summary", "targets": targets, "unread": unread,
        "patchedTargets": successful_base_comparisons[0], "comparisons": comparisons,
        "successfulBaseComparisons": {"preceding": successful_base_comparisons[0],
            "skipping": successful_base_comparisons[1], "backward": successful_base_comparisons[2]},
        "mismatchedHours": mismatches,
        "fullCpuMeanMs": mean(&full_cpu), "patchedCpuMeanMs": mean(&patched_cpu),
        "readCpuMeanMs": mean(&read_cpu)});
    output.emit(&summary);
    assert_eq!(
        source_before,
        SourceKey::capture(&path, &[]).unwrap(),
        "input changed"
    );
    output.0.sync_all().unwrap();
    assert!(
        comparisons.iter().all(|&count| count > 0),
        "some base directions were never exercised: {comparisons:?}"
    );
    assert!(
        successful_base_comparisons[0] > 0,
        "no successful comparison from a preceding base"
    );
    assert!(
        successful_base_comparisons[1] > 0,
        "no successful comparison from an offered two-back base"
    );
    assert!(
        mismatches.is_empty(),
        "patched railways mismatched the full build: {mismatches:?}"
    );
}
