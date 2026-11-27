use dashboard_base::Source;
use dashboard_core::config::LoadContext;
use dashboard_follow::{
    Follow, Identity,
    panel::{PanelData, ProductReader, ProductStep},
};
use playhead::{
    Declared, Hook, Lanes, Piece, Playhead, Settings, Step, Timeline,
    changes::Changes,
    pages::WorldPages,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
    time::Duration,
};

const SPEED: f64 = 1440.0;
const MAX_LCS_CELLS: usize = 256 * 1024;
const MAX_EDITS: usize = 32;

#[cfg(windows)]
fn thread_cpu_ns() -> u64 {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    // SAFETY: GetCurrentThread returns a valid pseudo-handle and the four FILETIME pointers are valid stack locals.
    let ok = unsafe { GetThreadTimes(GetCurrentThread(), created, exited, kernel, user) };
    assert_ne!(ok, 0);
    [kernel, user]
        .iter()
        .map(|t| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime))
        .sum::<u64>()
        * 100
}

#[cfg(not(windows))]
fn thread_cpu_ns() -> u64 {
    0
}

#[cfg(windows)]
fn thread_cycles() -> u64 {
    use windows_sys::Win32::System::Threading::GetCurrentThread;
    unsafe extern "system" {
        fn QueryThreadCycleTime(thread: *mut std::ffi::c_void, cycles: *mut u64) -> i32;
    }
    let mut cycles = 0u64;
    // SAFETY: GetCurrentThread returns a valid pseudo-handle and cycles points to a valid local u64.
    let ok = unsafe { QueryThreadCycleTime(GetCurrentThread(), &raw mut cycles) };
    assert_ne!(ok, 0);
    cycles
}

#[cfg(not(windows))]
fn thread_cycles() -> u64 {
    0
}

#[derive(Clone, Serialize)]
#[serde(untagged)]
enum Part {
    Key(String),
    Index(usize),
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
enum Op {
    Replace(Vec<Part>, Value),
    Remove(Vec<Part>),
    Splice(Vec<Part>, usize, usize, Vec<Value>),
}

fn diff_value(before: &Value, after: &Value, path: &mut Vec<Part>, out: &mut Vec<Op>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().filter(|k| !b.contains_key(*k)) {
                path.push(Part::Key(key.clone()));
                out.push(Op::Remove(path.clone()));
                path.pop();
            }
            for (key, value) in b {
                path.push(Part::Key(key.clone()));
                match a.get(key) {
                    Some(prev) => diff_value(prev, value, path, out),
                    None => out.push(Op::Replace(path.clone(), value.clone())),
                }
                path.pop();
            }
        }
        (Value::Array(a), Value::Array(b)) => diff_array(a, b, path, out),
        _ => out.push(Op::Replace(path.clone(), after.clone())),
    }
}

fn diff_array(a: &[Value], b: &[Value], path: &mut Vec<Part>, out: &mut Vec<Op>) {
    let mut prefix = 0;
    let limit = a.len().min(b.len());
    while prefix < limit && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < limit - prefix && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
        suffix += 1;
    }
    let am = &a[prefix..a.len() - suffix];
    let bm = &b[prefix..b.len() - suffix];
    if am.is_empty() && bm.is_empty() {
        return;
    }
    if am.len() == bm.len() && am.len() <= MAX_EDITS {
        let mark = out.len();
        for (i, (left, right)) in am.iter().zip(bm).enumerate() {
            path.push(Part::Index(prefix + i));
            diff_value(left, right, path, out);
            path.pop();
        }
        if out.len() - mark <= MAX_EDITS {
            return;
        }
        out.truncate(mark);
    }
    if am.len().saturating_mul(bm.len()) <= MAX_LCS_CELLS && !am.is_empty() && !bm.is_empty() {
        let cols = bm.len() + 1;
        let mut dp = vec![0u32; (am.len() + 1) * cols];
        for i in (0..am.len()).rev() {
            for j in (0..bm.len()).rev() {
                dp[i * cols + j] = if am[i] == bm[j] {
                    dp[(i + 1) * cols + (j + 1)] + 1
                } else {
                    dp[(i + 1) * cols + j].max(dp[i * cols + (j + 1)])
                };
            }
        }
        let mut i = 0;
        let mut j = 0;
        let mut pos = prefix;
        let mut ops = Vec::new();
        while i < am.len() || j < bm.len() {
            if i < am.len() && j < bm.len() && am[i] == bm[j] {
                i += 1;
                j += 1;
                pos += 1;
                continue;
            }
            let (si, sj) = (i, j);
            while i < am.len() || j < bm.len() {
                if i < am.len() && j < bm.len() && am[i] == bm[j] {
                    break;
                }
                if i < am.len() && (j == bm.len() || dp[(i + 1) * cols + j] >= dp[i * cols + (j + 1)]) {
                    i += 1;
                } else {
                    j += 1;
                }
            }
            let removed = i - si;
            let inserted = j - sj;
            if removed == inserted && removed <= 8 {
                for k in 0..removed {
                    path.push(Part::Index(pos + k));
                    diff_value(&am[si + k], &bm[sj + k], path, &mut ops);
                    path.pop();
                }
            } else {
                ops.push(Op::Splice(path.clone(), pos, removed, bm[sj..j].to_vec()));
            }
            pos += inserted;
            if ops.len() > MAX_EDITS {
                out.push(Op::Splice(path.clone(), prefix, am.len(), bm.to_vec()));
                return;
            }
        }
        out.extend(ops);
        return;
    }
    out.push(Op::Splice(path.clone(), prefix, am.len(), bm.to_vec()));
}

fn patch_bytes(before: &Value, after: &Value) -> usize {
    let mut ops = Vec::new();
    diff_value(before, after, &mut Vec::new(), &mut ops);
    rmp_serde::to_vec(&ops).map_or(usize::MAX, |b| b.len())
}

fn change_touches(c: &Changes, cols: &BTreeSet<usize>) -> bool {
    c.reset()
        || c.status().is_some()
        || c.status_left().is_some()
        || c.renumber().is_some()
        || c.read_rows_changed()
        || !c.inserted().is_empty()
        || !c.deleted().is_empty()
        || !c.passing().is_empty()
        || c.columns().iter().any(|w| !w.is_empty() && (cols.is_empty() || cols.contains(&w.column())))
        || c.log().is_some_and(|l| !l.is_empty() || !l.forgotten().is_empty())
}

struct PanelSub {
    name: &'static str,
    product: &'static str,
    duplicate: bool,
    read: ProductReader,
    tables: BTreeMap<usize, BTreeSet<usize>>,
    last_value: Option<Arc<PanelData>>,
    last_bytes: Vec<u8>,
    last_hour: i32,
}

#[derive(Default)]
struct ProductAgg {
    steps: u64,
    uses_lanes: bool,
    declared_touched: u64,
    ptr_changed: u64,
    bytes_changed: u64,
    unproven: u64,
    compute_cycles: u64,
    encode_cycles: u64,
    diff_cycles: u64,
    whole_bytes_sum: u64,
    emitted_bytes_sum: u64,
    patch_bytes_sum: u64,
    rows_sum: u64,
    seed_bytes: usize,
    seed_cycles: u64,
}

#[derive(Default)]
struct StepReport {
    last: i32,
    reset: bool,
    build_cycles: u64,
    build_ns: u64,
    panels_compute_cycles: u64,
    panels_encode_cycles: u64,
    panels_diff_cycles: u64,
    panels_emitted_bytes: u64,
    panels_patch_bytes: u64,
    panels_touched: u32,
    panels_changed: u32,
    panels_ptr_changed: u32,
    follow_cycles: u64,
    follow_bytes: u64,
    follow_answers: u32,
    follow_touched: u32,
}

fn widen<'a>(declared: &mut BTreeMap<usize, Declared>, lanes: impl IntoIterator<Item = &'a Declared>) -> bool {
    let mut widened = false;
    for lane in lanes {
        let held = declared.entry(lane.table).or_insert_with(|| {
            widened = true;
            lane.clone()
        });
        let before = (held.columns.len(), held.keyed, held.passing);
        held.columns.extend(&lane.columns);
        held.columns.sort_unstable();
        held.columns.dedup();
        held.keyed |= lane.keyed;
        held.passing |= lane.passing;
        widened |= before != (held.columns.len(), held.keyed, held.passing);
    }
    widened
}

fn open_panel(
    source: &Arc<Source>,
    load: &LoadContext,
    name: &'static str,
    product: &'static str,
    duplicate: bool,
    mut params: Value,
) -> io::Result<PanelSub> {
    params["panel"] = product.into();
    let panel = dashboard::panel::open(source, load, &params).map_err(io::Error::other)?;
    let exact = panel.exact().ok_or_else(|| io::Error::other(format!("{name}: no exact reader")))?;
    Ok(PanelSub {
        name,
        product,
        duplicate,
        read: exact.advance.clone(),
        tables: BTreeMap::new(),
        last_value: None,
        last_bytes: Vec::new(),
        last_hour: i32::MIN,
    })
}

fn build_panels(source: &Arc<Source>, load: &LoadContext, branch: &str, tag: &str, opened: i32) -> io::Result<Vec<PanelSub>> {
    let world = json!({ "branchId": branch });
    let country = json!({ "branchId": branch, "country": tag });
    let mut out = Vec::with_capacity(29);
    out.push(open_panel(source, load, "most-decorated-divisions", "decorated-divisions", false, json!({ "branchId": branch, "countries": null, "by": "medals", "descending": true, "count": 12 }))?);
    out.push(open_panel(source, load, "air-superiority-timeline", "air-superiority-timeline", false, json!({ "branchId": branch, "mode": "sides", "top": 7 }))?);
    out.push(open_panel(source, load, "national-spirits", "idea-history", false, country.clone())?);
    out.push(open_panel(source, load, "national-focus", "focus-card", false, country.clone())?);
    out.push(open_panel(source, load, "factions", "group-counts", false, json!({ "branchId": branch, "of": "factions" }))?);
    out.push(open_panel(source, load, "spirit-history", "idea-history", true, country.clone())?);
    out.push(open_panel(source, load, "intel-network-coverage", "intel-network-coverage", false, json!({ "branchId": branch, "top": 12, "touching": false, "measure": "covered", "basis": "controlled" }))?);
    out.push(open_panel(source, load, "weather-by-region", "regional-weather", false, json!({ "branchId": branch, "width": 768, "show": "conditions", "modifier": null, "regions": null, "top": 7, "windowDays": 7 }))?);
    out.push(open_panel(source, load, "lend-lease-agreements", "market-lendlease", false, json!({ "branchId": branch, "countries": [], "only": "agreements", "leaseSide": "sent" }))?);
    out.push(open_panel(source, load, "idea-timeline", "idea-history", true, country.clone())?);
    out.push(open_panel(source, load, "naval-supremacy:shares", "naval-supremacy", false, json!({ "branchId": branch, "country": tag, "top": 7 }))?);
    out.push(open_panel(source, load, "game-score", "game-score", false, world.clone())?);
    out.push(open_panel(source, load, "diplomatic-relations", "diplomatic-relations", false, json!({ "branchId": branch, "top": 10, "order": "faction" }))?);
    out.push(open_panel(source, load, "who-spies-on-whom:matrix", "operative-matrix", false, json!({ "branchId": branch, "countries": [], "form": "matrix" }))?);
    out.push(open_panel(source, load, "naval-supremacy:timeline", "naval-supremacy", true, json!({ "branchId": branch, "country": tag, "top": 7 }))?);
    out.push(open_panel(source, load, "decryption:ciphers", "decryption", false, world.clone())?);
    out.push(open_panel(source, load, "division-template", "division-templates", false, json!({ "branchId": branch, "country": tag, "countFor": "owner" }))?);
    out.push(open_panel(source, load, "convoy-health", "convoy-health", false, json!({ "branchId": branch, "view": "regions", "country": tag, "top": 7 }))?);
    out.push(open_panel(source, load, "active-research", "research-slots", false, country.clone())?);
    out.push(open_panel(source, load, "casualties-between", "casualty-matrix", false, json!({ "branchId": branch, "countries": [], "between": false }))?);
    out.push(open_panel(source, load, "air-superiority-region", "air-superiority", false, json!({ "branchId": branch, "mode": "sides", "order": "contested" }))?);
    out.push(open_panel(source, load, "who-spies-on-whom:pairs", "operative-matrix", false, json!({ "branchId": branch, "countries": [], "form": "pairs" }))?);
    out.push(open_panel(source, load, "political-power-spending-breakdown", "power-spending-breakdown", false, json!({ "branchId": branch, "country": tag, "currency": "Political", "from": opened }))?);
    out.push(open_panel(source, load, "industrial-organisations", "industrial-organisations", false, country.clone())?);
    out.push(open_panel(source, load, "governments-in-exile", "politics-state", false, json!({ "branchId": branch, "view": "exiles" }))?);
    out.push(open_panel(source, load, "occupied-states", "occupied-states", false, world.clone())?);
    out.push(open_panel(source, load, "factories:race", "series-rank", false, json!({ "branchId": branch, "family": { "kind": "country-factories", "measure": "free7" }, "top": 10 }))?);
    out.push(open_panel(source, load, "decryption:matrix", "decryption", true, world)?);
    out.push(open_panel(source, load, "political-power:markers", "power-spend-markers", false, json!({ "branchId": branch, "country": tag, "currency": "Political" }))?);
    Ok(out)
}

fn build_follow(identity: &Identity, branch: &str, tag: &str) -> io::Result<(Follow, Vec<Declared>, Vec<&'static str>)> {
    let follow = Follow::new(0);
    let specs: [(&'static str, Value); 10] = [
        ("scoreboard:rank", json!({ "product": "series-state", "params": { "branchId": branch, "family": { "kind": "country-factories", "measure": "free7" }, "top": 10 } })),
        ("scoreboard:factories", json!({ "product": "series-state", "params": { "branchId": branch, "family": { "kind": "country-factories", "measure": "free7" }, "reads": "every" } })),
        ("scoreboard:army", json!({ "product": "series-state", "params": { "branchId": branch, "family": { "kind": "army-totals", "measure": "ownerManpower" }, "reads": "every" } })),
        ("scoreboard:territory", json!({ "product": "series-state", "params": { "branchId": branch, "family": { "kind": "territory", "measure": "ownedStates" }, "reads": "every" } })),
        ("scoreboard:cas-dealt", json!({ "product": "series-state", "params": { "branchId": branch, "family": { "kind": "casualty-totals", "measure": "dealt" }, "reads": "every" } })),
        ("scoreboard:cas-taken", json!({ "product": "series-state", "params": { "branchId": branch, "family": { "kind": "casualty-totals", "measure": "taken" }, "reads": "every" } })),
        ("scoreboard:cas-partial", json!({ "product": "series-state", "params": { "branchId": branch, "family": { "kind": "casualty-totals", "measure": "partial" }, "reads": "every" } })),
        ("scoreboard:planes", json!({ "product": "series-state", "params": { "branchId": branch, "family": { "kind": "air-force-totals", "measure": "rosterPlanes" }, "reads": "every" } })),
        ("scoreboard:ships", json!({ "product": "series-state", "params": { "branchId": branch, "family": { "kind": "navy-totals", "measure": "rosterShips" }, "reads": "every" } })),
        ("political-power:series", json!({ "product": "series-state", "coverage": "row", "params": { "branchId": branch, "family": { "kind": "country-measure", "measure": "politicalPower" }, "reads": [{ "country": tag }] } })),
    ];
    let mut names = Vec::with_capacity(specs.len());
    for (idx, (name, req)) in specs.into_iter().enumerate() {
        let query = dashboard_follow::parse(&req, identity)?;
        follow.follow((idx + 1) as u32, query)?;
        names.push(name);
    }
    let declared = follow.declared()?;
    Ok((follow, declared, names))
}

pub fn run(
    recording: &str,
    source: &Arc<Source>,
    load: &LoadContext,
    branch_id: &str,
    tag: &str,
    opened: i32,
    pages: Arc<WorldPages>,
    identity: &Identity,
    first: i32,
    quantum: i32,
    steps_target: i32,
    only_product: Option<&str>,
) -> io::Result<()> {
    replay_frame::cpu::enable();
    let last = first + quantum * steps_target;
    let mut panels = source.in_scope(|| build_panels(source, load, branch_id, tag, opened))?;
    let include_follow = only_product.is_none_or(|want| want == "follow" || want == "scoreboard" || want == "political-power:series");
    if let Some(want) = only_product {
        panels.retain(|p| p.name == want || p.product == want);
        if panels.is_empty() && !include_follow {
            return Err(io::Error::other(format!("no Heaviest30 subscription matches --product {want}")));
        }
    }
    let (follow, follow_declared, follow_names) = if include_follow {
        build_follow(identity, branch_id, tag)?
    } else {
        (Follow::new(0), Vec::new(), Vec::new())
    };
    let mut declared = BTreeMap::<usize, Declared>::new();
    widen(&mut declared, &follow_declared);

    let follow_out = Arc::new(Mutex::new(None::<(u32, u32, u64)>));
    let follow_sink = Arc::clone(&follow_out);
    let mut follow_hook: Hook = follow.hook(Box::new(move |stats, msg| {
        let bytes = msg.as_ref().map_or(0, |m| m.bytes.len() as u64);
        let answers = msg.as_ref().map_or(0, |m| m.answers);
        *follow_sink.lock().unwrap() = Some((stats.touched, answers, bytes));
    }));

    let panel_meta: Vec<(&'static str, &'static str, bool)> = panels.iter().map(|p| (p.name, p.product, p.duplicate)).collect();
    let aggs = Arc::new(Mutex::new((0..panels.len()).map(|_| ProductAgg::default()).collect::<Vec<_>>()));
    let aggs_hook = Arc::clone(&aggs);
    let hook_cpu = Arc::new(Mutex::new((0u64, 0u64)));
    let hook_cpu_ref = Arc::clone(&hook_cpu);
    let (send, recv) = mpsc::channel::<(StepReport, Vec<Declared>)>();
    let live_ids = Arc::new(Mutex::new((1000u32, BTreeMap::<(usize, Vec<usize>, Vec<u128>, bool), u32>::new())));
    let follow_live = follow.clone();

    let hook: Hook = Box::new(move |step: &Step, lanes: &Lanes<'_>| {
        let cpu0 = thread_cpu_ns();
        let cyc0 = thread_cycles();
        let mut rep = StepReport {
            last: step.last(),
            reset: step.reset(),
            build_cycles: step.built().cycles,
            build_ns: step.built().ns,
            ..StepReport::default()
        };
        let mut new_lanes = Vec::new();
        let token = AtomicBool::new(false);

        let f0 = thread_cycles();
        follow_hook(step, lanes);
        rep.follow_cycles = thread_cycles().saturating_sub(f0);
        if let Some((touched, answers, bytes)) = follow_out.lock().unwrap().take() {
            rep.follow_touched = touched;
            rep.follow_answers = answers;
            rep.follow_bytes = bytes;
        }

        let mut agg_guard = aggs_hook.lock().unwrap();
        for (i, panel) in panels.iter_mut().enumerate() {
            let agg = &mut agg_guard[i];
            let declared_touched = if panel.tables.is_empty() {
                true
            } else {
                panel.tables.iter().any(|(t, cols)| step.table(*t).is_some_and(|c| change_touches(c, cols)))
            };
            let c0 = thread_cycles();
            let reading = (panel.read)(
                step.last(),
                step.reset(),
                &token,
                Some(ProductStep { step, lanes, held: None, wake: None }),
            );
            let reading = if reading.pending && panel.last_hour == i32::MIN {
                (panel.read)(step.last(), true, &token, None)
            } else {
                reading
            };
            let compute_c = thread_cycles().saturating_sub(c0);

            if !reading.lanes.is_empty() {
                for l in &reading.lanes {
                    let entry = panel.tables.entry(l.table).or_default();
                    entry.extend(l.columns.iter().copied());
                }
                new_lanes.extend(reading.lanes.iter().cloned());
            }
            if let Some(live) = reading.live.as_deref() {
                let mut reg = live_ids.lock().unwrap();
                for group in &live.groups {
                    let key = (group.table.index(), group.columns.clone(), group.keys.clone(), matches!(group.scope, dashboard_query::coverage::CoverageScope::Row));
                    if !reg.1.contains_key(&key) {
                        let id = reg.0;
                        reg.0 += 1;
                        reg.1.insert(key, id);
                        let _ = follow_live.follow(id, group.query());
                        new_lanes.push(Declared {
                            table: group.table.index(),
                            columns: group.columns.clone(),
                            keyed: false,
                            passing: true,
                            hourly: false,
                        });
                    }
                }
            }

            if reading.pending {
                continue;
            }
            let ptr_same = !step.reset() && panel.last_value.as_ref().is_some_and(|prev| Arc::ptr_eq(prev, &reading.value));
            let (encode_c, encoded, changed) = if ptr_same {
                (0, None, false)
            } else {
                let e0 = thread_cycles();
                let bytes = match &*reading.value {
                    Ok(v) => rmp_serde::to_vec(v).expect("msgpack"),
                    Err(reason) => reason.as_bytes().to_vec(),
                };
                let ec = thread_cycles().saturating_sub(e0);
                let ch = step.reset() || panel.last_bytes != bytes;
                (ec, Some(bytes), ch)
            };

            let (diff_c, pbytes) = if !step.reset() && changed {
                match (panel.last_value.as_deref(), &*reading.value) {
                    (Some(Ok(prev_v)), Ok(cur_v)) => {
                        let d0 = thread_cycles();
                        let pb = patch_bytes(prev_v, cur_v);
                        let dc = thread_cycles().saturating_sub(d0);
                        let wb = encoded.as_ref().map_or(0, Vec::len);
                        (dc, pb.min(wb))
                    }
                    _ => (0, encoded.as_ref().map_or(0, Vec::len)),
                }
            } else {
                (0, 0)
            };

            let whole_len = encoded.as_ref().map_or(panel.last_bytes.len(), Vec::len);
            if step.reset() {
                agg.seed_bytes = whole_len;
                agg.seed_cycles = compute_c + encode_c;
                agg.uses_lanes = !panel.tables.is_empty();
            } else {
                agg.steps += 1;
                agg.uses_lanes = !panel.tables.is_empty();
                if declared_touched {
                    agg.declared_touched += 1;
                    rep.panels_touched += 1;
                }
                if !ptr_same {
                    agg.ptr_changed += 1;
                    rep.panels_ptr_changed += 1;
                }
                if changed {
                    agg.bytes_changed += 1;
                    rep.panels_changed += 1;
                }
                if panel.last_hour != i32::MIN && reading.from > panel.last_hour + 1 {
                    agg.unproven += 1;
                }
                agg.compute_cycles += compute_c;
                agg.encode_cycles += encode_c;
                agg.diff_cycles += diff_c;
                agg.whole_bytes_sum += whole_len as u64;
                if changed {
                    agg.emitted_bytes_sum += whole_len as u64;
                    agg.patch_bytes_sum += pbytes as u64;
                    rep.panels_emitted_bytes += whole_len as u64;
                    rep.panels_patch_bytes += pbytes as u64;
                }
                agg.rows_sum += reading.rows;
                rep.panels_compute_cycles += compute_c;
                rep.panels_encode_cycles += encode_c;
                rep.panels_diff_cycles += diff_c;
            }

            if let Some(bytes) = encoded {
                panel.last_bytes = bytes;
            }
            panel.last_value = Some(reading.value);
            panel.last_hour = step.last();
        }
        drop(agg_guard);
        let cpu1 = thread_cpu_ns();
        let cyc1 = thread_cycles();
        let mut hc = hook_cpu_ref.lock().unwrap();
        hc.0 += cpu1.saturating_sub(cpu0);
        hc.1 += cyc1.saturating_sub(cyc0);
        let _ = send.send((rep, new_lanes));
    });

    let timeline = Timeline::new(&*pages, vec![Piece { run: 0, first, last }])?;
    let rate = SPEED / f64::from(quantum);
    let ph = Playhead::new(
        pages,
        timeline,
        Settings { step_threads: 2, rate, ..Settings::default() },
        Some(hook),
    )?;
    ph.lead(dashboard_follow::lead::LEAD);
    let reader = ph.declare(declared.values().cloned().collect(), None)?;
    let mut seat = ph.consumer(reader);
    ph.seek(first);

    let mut started = false;
    let mut finished = first - 1;
    let mut taken = Vec::new();
    let mut step_reports = Vec::new();
    while finished < last {
        if let Some(why) = ph.error() {
            return Err(io::Error::other(why));
        }
        let newest = ph.newest().unwrap_or(first);
        seat.advance(newest);
        seat.take(newest, &mut taken);
        taken.clear();
        if !started && ph.newest().is_some() {
            started = true;
            ph.anchor(f64::from(first), true, SPEED, std::time::Instant::now(), false);
        }
        match recv.recv_timeout(Duration::from_millis(2)) {
            Ok((rep, new_lanes)) => {
                if widen(&mut declared, &new_lanes) {
                    ph.redeclare(reader, declared.values().cloned().collect())?;
                }
                finished = rep.last;
                step_reports.push(rep);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(io::Error::other("playhead disconnected early")),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    drop(seat);
    ph.finish();

    let (total_cpu_ns, total_cycles) = *hook_cpu.lock().unwrap();
    let ns_per_cycle = if total_cycles > 0 { total_cpu_ns as f64 / total_cycles as f64 } else { 0.0 };
    let to_ms = |cyc: u64| (cyc as f64 * ns_per_cycle) / 1e6;

    let steady: Vec<&StepReport> = step_reports.iter().filter(|s| !s.reset).collect();
    let n = steady.len().max(1) as f64;
    let mean = |f: fn(&StepReport) -> u64| steady.iter().map(|s| f(s)).sum::<u64>() as f64 / n;
    let mean_u32 = |f: fn(&StepReport) -> u32| steady.iter().map(|s| u64::from(f(s))).sum::<u64>() as f64 / n;

    let aggs_final = aggs.lock().unwrap();
    let per_product: Vec<Value> = aggs_final
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let (name, product, duplicate) = panel_meta[i];
            let s = a.steps.max(1) as f64;
            json!({
                "name": name,
                "product": product,
                "duplicate": duplicate,
                "usesLanes": a.uses_lanes,
                "steps": a.steps,
                "touchedFrac": a.declared_touched as f64 / s,
                "ptrChangedFrac": a.ptr_changed as f64 / s,
                "changedFrac": a.bytes_changed as f64 / s,
                "unprovenFrac": a.unproven as f64 / s,
                "computeMs": to_ms(a.compute_cycles) / s,
                "encodeMs": to_ms(a.encode_cycles) / s,
                "diffMs": to_ms(a.diff_cycles) / s,
                "computeCycles": a.compute_cycles as f64 / s,
                "encodeCycles": a.encode_cycles as f64 / s,
                "diffCycles": a.diff_cycles as f64 / s,
                "wholeBytes": a.whole_bytes_sum as f64 / s,
                "emittedBytes": a.emitted_bytes_sum as f64 / s,
                "patchBytes": a.patch_bytes_sum as f64 / s,
                "rowsPerStep": a.rows_sum as f64 / s,
                "seedBytes": a.seed_bytes,
                "seedMs": to_ms(a.seed_cycles),
            })
        })
        .collect();

    let dedup_compute_ms: f64 = aggs_final
        .iter()
        .enumerate()
        .filter(|(i, _)| !panel_meta[*i].2)
        .map(|(_, a)| to_ms(a.compute_cycles) / a.steps.max(1) as f64)
        .sum();
    let dedup_encode_ms: f64 = aggs_final
        .iter()
        .enumerate()
        .filter(|(i, _)| !panel_meta[*i].2)
        .map(|(_, a)| to_ms(a.encode_cycles) / a.steps.max(1) as f64)
        .sum();
    let dedup_diff_ms: f64 = aggs_final
        .iter()
        .enumerate()
        .filter(|(i, _)| !panel_meta[*i].2)
        .map(|(_, a)| to_ms(a.diff_cycles) / a.steps.max(1) as f64)
        .sum();

    let out = json!({
        "recording": recording,
        "branch": branch_id,
        "tag": tag,
        "first": first,
        "last": last,
        "quantum": quantum,
        "steadySteps": steady.len(),
        "followQueries": follow_names,
        "calibration": {
            "hookThreadCpuMs": total_cpu_ns as f64 / 1e6,
            "hookThreadCycles": total_cycles,
            "nsPerCycle": ns_per_cycle,
            "ghz": if ns_per_cycle > 0.0 { 1.0 / ns_per_cycle } else { 0.0 },
        },
        "perStride": {
            "playheadBuildMs": mean(|s| s.build_cycles) * ns_per_cycle / 1e6,
            "playheadBuildWallMs": mean(|s| s.build_ns) / 1e6,
            "panelsComputeMs": mean(|s| s.panels_compute_cycles) * ns_per_cycle / 1e6,
            "panelsEncodeMs": mean(|s| s.panels_encode_cycles) * ns_per_cycle / 1e6,
            "panelsDiffMs": mean(|s| s.panels_diff_cycles) * ns_per_cycle / 1e6,
            "dedupPanelsComputeMs": dedup_compute_ms,
            "dedupPanelsEncodeMs": dedup_encode_ms,
            "dedupPanelsDiffMs": dedup_diff_ms,
            "followMs": mean(|s| s.follow_cycles) * ns_per_cycle / 1e6,
            "totalHookMs": (mean(|s| s.panels_compute_cycles) + mean(|s| s.panels_encode_cycles) + mean(|s| s.follow_cycles)) * ns_per_cycle / 1e6,
            "totalWithDiffMs": (mean(|s| s.panels_compute_cycles) + mean(|s| s.panels_diff_cycles) + mean(|s| s.follow_cycles)) * ns_per_cycle / 1e6,
            "panelsEmittedBytes": mean(|s| s.panels_emitted_bytes),
            "panelsPatchBytes": mean(|s| s.panels_patch_bytes),
            "followBytes": mean(|s| s.follow_bytes),
            "panelsTouched": mean_u32(|s| s.panels_touched),
            "panelsPtrChanged": mean_u32(|s| s.panels_ptr_changed),
            "panelsChanged": mean_u32(|s| s.panels_changed),
            "followTouched": mean_u32(|s| s.follow_touched),
            "followAnswers": mean_u32(|s| s.follow_answers),
        },
        "products": per_product,
    });
    println!("{}", serde_json::to_string_pretty(&out).map_err(io::Error::other)?);
    Ok(())
}
