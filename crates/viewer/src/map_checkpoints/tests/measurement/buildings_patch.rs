//! Compare patched building fields against full builds in all three base directions.
use super::*;
use crate::document::{Document, Hour as HourView, InitialHour};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use viewer_foundation::map::perspective;
use viewer_foundation::profiling::BuilderSpans;
use viewer_world::map::map_buildings::{
    BuildPath, BuildingsBase, MapBuildingSummary, PreparedBuildings,
};

struct Output(File);

impl Output {
    fn emit(&mut self, row: &Value) {
        serde_json::to_writer(&mut self.0, row).unwrap();
        writeln!(self.0).unwrap();
        self.0.flush().unwrap();
    }
}

type Field = io::Result<(PreparedBuildings, MapBuildingSummary)>;

fn thread_ms() -> f64 {
    viewer_foundation::profiling::current_thread_cpu_time()
        .expect("thread CPU clock")
        .as_secs_f64()
        * 1e3
}

type Children = BTreeMap<&'static str, f64>;

fn children(spans: BuilderSpans) -> Children {
    let mut children = Children::new();
    for span in spans.into_records() {
        *children.entry(span.name).or_default() += span.cpu_ms.unwrap_or(f64::NAN);
    }
    children
}

fn full(view: &HourView<'_>, choice: &perspective::Choice) -> (Field, f64, Children) {
    let mut spans = BuilderSpans::new(true);
    let started = thread_ms();
    let field = view.map_building_field_profiled(choice, &mut spans);
    (field, thread_ms() - started, children(spans))
}

type Patched = io::Result<(PreparedBuildings, MapBuildingSummary, BuildingsBase)>;

fn patched(
    view: &HourView<'_>,
    choice: &perspective::Choice,
    base: Option<&BuildingsBase>,
) -> (Patched, f64, Children) {
    let mut spans = BuilderSpans::new(true);
    let started = thread_ms();
    let field = view.map_building_field_patched(choice, base, &mut spans);
    (field, thread_ms() - started, children(spans))
}

fn children_mean(children: &BTreeMap<&'static str, Vec<f64>>) -> Value {
    children
        .iter()
        .map(|(name, values)| {
            let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
            (
                name.to_string(),
                json!({"meanMs": mean, "hours": values.len()}),
            )
        })
        .collect::<Map<_, _>>()
        .into()
}

fn compare(field: Patched, reference: &Field) -> (bool, String, Option<Arc<BuildingsBase>>) {
    match (field, reference) {
        (Ok((prepared, summary, next)), Ok((full, full_summary))) => {
            let path = match next.path() {
                BuildPath::Full(reason) => format!("full: {reason}"),
                BuildPath::Patched { slots } => format!("patched {slots}"),
            };
            (
                prepared.bit_eq(full) && summary == *full_summary,
                path,
                Some(Arc::new(next)),
            )
        }
        (Err(patched), Err(full)) => (
            patched.to_string() == full.to_string(),
            "refused".into(),
            None,
        ),
        _ => (false, "refusal differs".into(), None),
    }
}

#[derive(Default)]
struct Tally {
    paths: BTreeMap<String, usize>,
    evaluated: Vec<usize>,
    successful_patches: usize,
    mismatches: Vec<i32>,
}

impl Tally {
    fn add(&mut self, hour: i32, equal: bool, path: &str) {
        let kind = match path.strip_prefix("patched ") {
            Some(slots) => {
                self.evaluated.push(slots.parse().unwrap());
                self.successful_patches += usize::from(equal);
                "patched"
            }
            None => path,
        };
        *self.paths.entry(kind.to_owned()).or_default() += 1;
        if !equal {
            self.mismatches.push(hour);
        }
    }

    fn json(&self) -> Value {
        let mut evaluated = self.evaluated.clone();
        evaluated.sort_unstable();
        let at = |percent: usize| {
            evaluated
                .get((evaluated.len().max(1) - 1) * percent / 100)
                .copied()
        };
        json!({"paths": self.paths, "successfulPatches": self.successful_patches,
            "mismatchedHours": self.mismatches,
            "evaluatedSlotsP50": at(50), "evaluatedSlotsP90": at(90),
            "evaluatedSlotsMax": evaluated.last()})
    }
}

#[test]
#[ignore = "lead-run release oracle; needs BATTLE_PLAN_MAP_BENCH_REPLAY, BATTLE_PLAN_MAP_BENCH_EVIDENCE, BATTLE_PLAN_GAME_DIR, BATTLE_PLAN_MAP_CENSUS_USER_DIR and a private LOCALAPPDATA"]
fn buildings_patch_oracle() {
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
    let choice = perspective::Choice::default();
    let mut output = Output(File::create(evidence.join("buildings-patch.jsonl")).unwrap());
    output.emit(&json!({"kind": "input", "recording": path, "grid": grid,
        "perspective": "recorded country, fog on",
        "bases": {"preceding": "the target one grid step earlier", "skipping": "the target two grid steps earlier",
            "backward": "the following target, patched into the earlier one"}}));
    struct Kept {
        frame: DecodedFrame,
        full: Field,
        base: Option<Arc<BuildingsBase>>,
    }
    let mut kept: Vec<Kept> = Vec::new();
    let [mut preceding, mut skipping, mut backward] = [(); 3].map(|_| Tally::default());
    let (mut full_cpu, mut patched_cpu, mut vision_cpu) = (Vec::new(), Vec::new(), Vec::new());
    let mut full_children: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    let mut patched_children: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
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
            let vision_started = thread_ms();
            let _ = view.vision(&choice);
            let vision_ms = thread_ms() - vision_started;
            let base = kept.last().and_then(|kept| kept.base.clone());
            // Alternate order so neither path always sees cold memos.
            let full_first = targets % 2 == 1;
            let ((reference, full_ms, full_spans), (field, patched_ms, patched_spans)) =
                if full_first {
                    let reference = full(&view, &choice);
                    (reference, patched(&view, &choice, base.as_deref()))
                } else {
                    let field = patched(&view, &choice, base.as_deref());
                    (full(&view, &choice), field)
                };
            let (equal, path, next) = compare(field, &reference);
            preceding.add(frame.hour, equal, &path);
            full_cpu.push(full_ms);
            patched_cpu.push(patched_ms);
            vision_cpu.push(vision_ms);
            for (&name, &ms) in &full_spans {
                full_children.entry(name).or_default().push(ms);
            }
            if path.starts_with("patched") {
                for (&name, &ms) in &patched_spans {
                    patched_children.entry(name).or_default().push(ms);
                }
            }
            let mut row = json!({"kind": "target", "hour": frame.hour, "preceding": path,
                "equal": equal, "fullCpuMs": full_ms, "patchedCpuMs": patched_ms,
                "visionCpuMs": vision_ms, "fullFirst": full_first});
            row["fullChildrenCpuMs"] = json!(full_spans);
            row["patchedChildrenCpuMs"] = json!(patched_spans);
            if kept.len() >= 2 {
                let base = kept[kept.len() - 2].base.clone();
                let (field, _, _) = patched(&view, &choice, base.as_deref());
                let (equal, path, _) = compare(field, &reference);
                skipping.add(frame.hour, equal, &path);
                row["skipping"] = json!(path);
                row["skippingEqual"] = json!(equal);
            }
            if let (Some(earlier), Some(next)) = (kept.last(), &next) {
                let earlier_view = HourView::new(&document.content, &earlier.frame, 0.0);
                let (field, _, _) = patched(&earlier_view, &choice, Some(&**next));
                let (equal, path, _) = compare(field, &earlier.full);
                if earlier.full.is_ok() {
                    assert_eq!(path, "full: the base is not an earlier hour");
                }
                backward.add(earlier.frame.hour, equal, &path);
                row["backward"] = json!(path);
                row["backwardEqual"] = json!(equal);
            }
            drop(view);
            output.emit(&row);
            kept.push(Kept {
                frame,
                full: reference,
                base: next,
            });
            if kept.len() > 2 {
                kept.remove(0);
            }
        }
    }
    let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
    let mut summary = json!({"kind": "summary", "targets": targets,
        "preceding": preceding.json(), "skipping": skipping.json(), "backward": backward.json(),
        "fullCpuMeanMs": mean(&full_cpu), "patchedCpuMeanMs": mean(&patched_cpu),
        "visionCpuMeanMs": mean(&vision_cpu)});
    summary["fullChildrenMeanCpuMs"] = children_mean(&full_children);
    summary["patchedChildrenMeanCpuMs"] = children_mean(&patched_children);
    output.emit(&summary);
    assert_eq!(
        source_before,
        SourceKey::capture(&path, &[]).unwrap(),
        "input changed"
    );
    output.0.sync_all().unwrap();
    assert!(
        targets >= 3,
        "the oracle requires at least three grid targets"
    );
    assert!(
        preceding.successful_patches > 0,
        "no successful preceding-base patch was exercised"
    );
    assert!(
        skipping.successful_patches > 0,
        "no successful two-back patch was exercised"
    );
    assert!(!backward.paths.is_empty(), "no backward base was exercised");
    for (name, tally) in [
        ("preceding", &preceding),
        ("skipping", &skipping),
        ("backward", &backward),
    ] {
        assert!(
            tally.mismatches.is_empty(),
            "{name} bases mismatched the full build at hours {:?}",
            tally.mismatches
        );
    }
}
