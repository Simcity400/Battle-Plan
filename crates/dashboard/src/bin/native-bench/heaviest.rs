use dashboard_base::Source;
use dashboard_core::config::LoadContext;
use dashboard_follow::{
    Follow, Identity, Message,
    live::{Live, cells_id},
    panel::{PanelData, ProductReader, ProductStep},
};
use dashboard_query::coverage::CoverageScope;
use playhead::{
    Declared, Hook, Lanes, Piece, Playhead, Settings, Step, Timeline,
    changes::Changes,
    pages::WorldPages,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
    time::Duration,
};

const SPEED: f64 = 1440.0;

#[cfg(windows)]
fn thread_cpu_ns() -> u64 {
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::{GetCurrentThread, GetThreadTimes}};
    let mut t = [FILETIME::default(); 4];
    let [c, e, k, u] = &mut t;
    // SAFETY: GetCurrentThread returns a valid pseudo-handle and the four FILETIME pointers are valid stack locals.
    assert_ne!(unsafe { GetThreadTimes(GetCurrentThread(), c, e, k, u) }, 0);
    [k, u].iter().map(|x| (u64::from(x.dwHighDateTime) << 32) | u64::from(x.dwLowDateTime)).sum::<u64>() * 100
}

#[cfg(not(windows))]
fn thread_cpu_ns() -> u64 { 0 }

#[cfg(windows)]
fn thread_cycles() -> u64 {
    use windows_sys::Win32::System::Threading::GetCurrentThread;
    unsafe extern "system" { fn QueryThreadCycleTime(thread: *mut std::ffi::c_void, cycles: *mut u64) -> i32; }
    let mut cycles = 0u64;
    // SAFETY: GetCurrentThread returns a valid pseudo-handle and cycles points to a valid local u64.
    assert_ne!(unsafe { QueryThreadCycleTime(GetCurrentThread(), &raw mut cycles) }, 0);
    cycles
}

#[cfg(not(windows))]
fn thread_cycles() -> u64 { 0 }

fn patch_bytes(before: &Value, after: &Value) -> usize {
    dashboard::patch::encode(&dashboard::patch::diff(before, after)).map_or(usize::MAX, |b| b.len())
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

type GroupKey = (usize, Vec<usize>, Vec<u128>, bool);

fn release_live(follow: &Follow, held: &mut Vec<(GroupKey, u32)>) {
    for (_, id) in held.drain(..) {
        let _ = follow.unfollow(id);
    }
}

#[expect(clippy::too_many_arguments, reason = "mirrors host::StepProduct::follow_cells transformation for delivered wire payloads")]
fn sync_live(
    follow: &Follow,
    lane: u32,
    held: &mut Vec<(GroupKey, u32)>,
    last_delivered: Option<&Arc<PanelData>>,
    value: &Arc<PanelData>,
    live: Option<&Live>,
    hour: i32,
    seed: bool,
    fresh: bool,
    new_lanes: &mut Vec<Declared>,
) -> Arc<PanelData> {
    let (Ok(reply @ Value::Object(_)), Some(live)) = (&**value, live.filter(|l| !l.is_empty())) else {
        release_live(follow, held);
        return value.clone();
    };
    let mut reply = reply.clone();
    let mut kept = Vec::with_capacity(live.groups.len());
    if let Some(groups) = reply.get_mut("live").and_then(|v| v.get_mut("groups")).and_then(Value::as_array_mut) {
        for (group, out) in live.groups.iter().zip(groups) {
            let Some(out) = out.as_object_mut() else { continue };
            let key: GroupKey = (
                group.table.index(),
                group.columns.clone(),
                group.keys.clone(),
                matches!(group.scope, CoverageScope::Row),
            );
            let exact = out
                .get("values")
                .and_then(Value::as_array)
                .is_some_and(|vals| vals.iter().all(|v| v.as_i64().is_none_or(|n| n.unsigned_abs() < 1 << 53)));
            let (id, is_new) = if let Some(pos) = held.iter().position(|(k, _)| *k == key) {
                let entry = held.swap_remove(pos);
                kept.push(entry);
                (kept.last().unwrap().1, false)
            } else {
                let id = cells_id(lane, 2);
                let _ = follow.follow(id, group.query());
                kept.push((key, id));
                (id, true)
            };
            new_lanes.push(Declared {
                table: group.table.index(),
                columns: group.columns.clone(),
                keyed: false,
                passing: true,
                hourly: false,
            });
            let seeded = (is_new || seed) && fresh && exact;
            out.insert("follow".into(), id.into());
            if seeded {
                out.insert("seedHour".into(), hour.into());
            } else {
                out.remove("values");
                out.remove("rawValues");
            }
        }
    }
    release_live(follow, held);
    *held = kept;
    let made = Arc::new(Ok(reply));
    match last_delivered {
        Some(last) if **last == *made => last.clone(),
        _ => made,
    }
}

struct PanelSub {
    name: &'static str,
    product: &'static str,
    duplicate: bool,
    read: ProductReader,
    tables: BTreeMap<usize, BTreeSet<usize>>,
    live_groups: Vec<(GroupKey, u32)>,
    last_input: Option<Arc<PanelData>>,
    last_delivered: Option<Arc<PanelData>>,
    last_bytes: Vec<u8>,
    last_hour: i32,
    resend: bool,
}

#[derive(Default)]
struct ProductAgg {
    steps: u64,
    pending: u64,
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
    raw_patch_bytes_sum: u64,
    wire_patch_bytes_sum: u64,
    rows_sum: u64,
    seed_bytes: usize,
    seed_cycles: u64,
    last_stride: Option<u32>,
    last_whole: u64,
    stride_flags: u8,
}

impl ProductAgg {
    fn mark(&mut self, bit: u8) -> bool {
        if self.stride_flags & bit != 0 {
            return false;
        }
        self.stride_flags |= bit;
        true
    }

    fn enter_stride(&mut self, stride: u32, whole_len: u64) {
        if self.last_stride != Some(stride) {
            self.steps += 1;
            self.last_stride = Some(stride);
            self.stride_flags = 0;
            self.last_whole = whole_len;
            self.whole_bytes_sum += whole_len;
        } else if whole_len > 0 {
            self.whole_bytes_sum = self.whole_bytes_sum - self.last_whole + whole_len;
            self.last_whole = whole_len;
        }
    }
}

struct FollowSub {
    id: u32,
    name: &'static str,
    steps: u64,
    answers: u64,
    changed: u64,
    bytes: u64,
    last_stride: Option<u32>,
    changed_in_stride: bool,
    last_body: Option<Vec<u8>>,
}

#[derive(Default)]
struct StepReport {
    last: i32,
    stride: u32,
    measured: bool,
    build_cycles: u64,
    build_ns: u64,
    panels_compute_cycles: u64,
    panels_encode_cycles: u64,
    panels_diff_cycles: u64,
    panels_emitted_bytes: u64,
    panels_raw_patch_bytes: u64,
    panels_wire_patch_bytes: u64,
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
        live_groups: Vec::new(),
        last_input: None,
        last_delivered: None,
        last_bytes: Vec::new(),
        last_hour: i32::MIN,
        resend: false,
    })
}

fn build_panels(
    source: &Arc<Source>,
    load: &LoadContext,
    branch: &str,
    tag: &str,
    opened: i32,
    only_product: Option<&str>,
) -> io::Result<Vec<PanelSub>> {
    let world = json!({ "branchId": branch });
    let country = json!({ "branchId": branch, "country": tag });
    let specs: [(&'static str, &'static str, Option<&'static str>, Value); 30] = [
        ("most-decorated-divisions", "decorated-divisions", None, json!({ "branchId": branch, "countries": null, "by": "medals", "descending": true, "count": 12 })),
        ("air-superiority-timeline", "air-superiority-timeline", None, json!({ "branchId": branch, "mode": "sides", "top": 7 })),
        ("national-spirits", "idea-history", Some("idea-history"), country.clone()),
        ("national-focus", "focus-card", None, country.clone()),
        ("factions", "group-counts", None, json!({ "branchId": branch, "of": "factions" })),
        ("spirit-history", "idea-history", Some("idea-history"), country.clone()),
        ("intel-network-coverage", "intel-network-coverage", None, json!({ "branchId": branch, "top": 12, "touching": false, "measure": "covered", "basis": "controlled" })),
        ("weather-by-region", "regional-weather", None, json!({ "branchId": branch, "width": 768, "show": "conditions", "modifier": null, "regions": null, "top": 7, "windowDays": 7 })),
        ("lend-lease-agreements", "market-lendlease", None, json!({ "branchId": branch, "countries": [], "only": "agreements", "leaseSide": "sent" })),
        ("idea-timeline", "idea-history", Some("idea-history"), country.clone()),
        ("naval-supremacy:shares", "naval-supremacy", Some("naval-supremacy"), json!({ "branchId": branch, "country": tag, "top": 7 })),
        ("game-score", "game-score", None, world.clone()),
        ("diplomatic-relations", "diplomatic-relations", None, json!({ "branchId": branch, "top": 10, "order": "faction" })),
        ("who-spies-on-whom:matrix", "operative-matrix", None, json!({ "branchId": branch, "countries": [], "form": "matrix" })),
        ("naval-supremacy:timeline", "naval-supremacy", Some("naval-supremacy"), json!({ "branchId": branch, "country": tag, "top": 7 })),
        ("decryption:ciphers", "decryption", Some("decryption"), world.clone()),
        ("division-template", "division-templates", None, json!({ "branchId": branch, "country": tag, "countFor": "owner" })),
        ("convoy-health", "convoy-health", None, json!({ "branchId": branch, "view": "regions", "country": tag, "top": 7 })),
        ("active-research", "research-slots", None, country.clone()),
        ("casualties-between", "casualty-matrix", None, json!({ "branchId": branch, "countries": [], "between": false })),
        ("air-superiority-region", "air-superiority", None, json!({ "branchId": branch, "mode": "sides", "order": "contested" })),
        ("who-spies-on-whom:pairs", "operative-matrix", None, json!({ "branchId": branch, "countries": [], "form": "pairs" })),
        ("political-power-spending-breakdown", "power-spending-breakdown", None, json!({ "branchId": branch, "country": tag, "currency": "Political", "from": opened })),
        ("industrial-organisations", "industrial-organisations", None, country),
        ("governments-in-exile", "politics-state", None, json!({ "branchId": branch, "view": "exiles" })),
        ("occupied-states", "occupied-states", None, world.clone()),
        ("scoreboard", "scoreboard", None, json!({ "branchId": branch,
            "columns": ["factories", "armySize", "territory", "casualtiesDealt", "casualtiesTaken", "planes", "ships"],
            "top": { "count": 10, "sort": "factories", "descending": true } })),
        ("factories:race", "series-rank", None, json!({ "branchId": branch, "family": { "kind": "country-factories", "measure": "free7" }, "top": 10 })),
        ("decryption:matrix", "decryption", Some("decryption"), world),
        ("political-power:markers", "power-spend-markers", None, json!({ "branchId": branch, "country": tag, "currency": "Political" })),
    ];
    let mut seen_groups = BTreeSet::new();
    let mut out = Vec::new();
    for (name, product, group, params) in specs {
        if only_product.is_some_and(|want| want != name && want != product) {
            continue;
        }
        let duplicate = group.is_some_and(|g| !seen_groups.insert(g));
        out.push(open_panel(source, load, name, product, duplicate, params)?);
    }
    Ok(out)
}

fn build_follow(
    identity: &Identity,
    branch: &str,
    tag: &str,
    only_product: Option<&str>,
) -> io::Result<(Follow, Vec<Declared>, Vec<FollowSub>)> {
    let follow = Follow::new(0);
    let specs: [(&'static str, &'static str, Value); 1] = [
        ("political-power:series", "political-power", json!({ "product": "series-state", "coverage": "row", "params": { "branchId": branch, "family": { "kind": "country-measure", "measure": "politicalPower" }, "reads": [{ "country": tag }] } })),
    ];
    let mut subs = Vec::new();
    for (idx, (name, group, req)) in specs.into_iter().enumerate() {
        if only_product.is_some_and(|want| want != "follow" && want != name && want != group) {
            continue;
        }
        let id = (idx + 1) as u32;
        follow.follow(id, dashboard_follow::parse(&req, identity)?)?;
        subs.push(FollowSub {
            id,
            name,
            steps: 0,
            answers: 0,
            changed: 0,
            bytes: 0,
            last_stride: None,
            changed_in_stride: false,
            last_body: None,
        });
    }
    let declared = follow.declared()?;
    Ok((follow, declared, subs))
}

#[expect(clippy::too_many_arguments, reason = "benchmark entry point taking opened recording context and stride options")]
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
    let mut panels = source.in_scope(|| build_panels(source, load, branch_id, tag, opened, only_product))?;
    let (follow, follow_declared, follow_subs) = build_follow(identity, branch_id, tag, only_product)?;
    if panels.is_empty() && follow_subs.is_empty() {
        return Err(io::Error::other(format!("no Heaviest30 subscription matches --product {}", only_product.unwrap_or_default())));
    }
    let mut declared = BTreeMap::<usize, Declared>::new();
    widen(&mut declared, &follow_declared);

    let follow_out = Arc::new(Mutex::new(None::<(u32, Option<Message>)>));
    let follow_sink = Arc::clone(&follow_out);
    let mut follow_hook: Hook = follow.hook(Box::new(move |stats, msg| {
        *follow_sink.lock().unwrap() = Some((stats.touched, msg));
    }));

    let panel_meta: Vec<(&'static str, &'static str, bool)> = panels.iter().map(|p| (p.name, p.product, p.duplicate)).collect();
    let aggs = Arc::new(Mutex::new((0..panels.len()).map(|_| ProductAgg::default()).collect::<Vec<_>>()));
    let aggs_hook = Arc::clone(&aggs);
    let follow_aggs = Arc::new(Mutex::new(follow_subs));
    let follow_aggs_hook = Arc::clone(&follow_aggs);
    let hook_cpu = Arc::new(Mutex::new((0u64, 0u64)));
    let hook_cpu_ref = Arc::clone(&hook_cpu);
    let (send, recv) = mpsc::channel::<(Option<StepReport>, Vec<Declared>)>();
    let follow_live = follow.clone();
    let mut max_hour = i32::MIN;

    let hook: Hook = Box::new(move |step: &Step, lanes: &Lanes<'_>| {
        let is_forward = step.reset() || (step.first() <= step.last() && step.last() > max_hour);
        if !is_forward {
            follow_hook(step, lanes);
            let _ = follow_out.lock().unwrap().take();
            let _ = send.send((None, Vec::new()));
            return;
        }
        max_hour = step.last();
        let stride_idx = if step.reset() || step.last() <= first {
            0
        } else {
            ((step.last() - first - 1) / quantum) as u32
        };
        let measure = !step.reset() && (steps_target == 1 || stride_idx > 0);

        let cpu0 = thread_cpu_ns();
        let cyc0 = thread_cycles();
        let mut rep = StepReport {
            last: step.last(),
            stride: stride_idx,
            measured: measure,
            build_cycles: step.built().cycles,
            build_ns: step.built().ns,
            ..StepReport::default()
        };
        let mut new_lanes = Vec::new();
        let token = AtomicBool::new(false);

        let f0 = thread_cycles();
        follow_hook(step, lanes);
        rep.follow_cycles = thread_cycles().saturating_sub(f0);
        if let Some((touched, msg)) = follow_out.lock().unwrap().take() {
            rep.follow_touched = touched;
            rep.follow_answers = msg.as_ref().map_or(0, |m| m.answers);
            rep.follow_bytes = msg.as_ref().map_or(0, |m| m.bytes.len() as u64);
            let mut fguard = follow_aggs_hook.lock().unwrap();
            if measure {
                for sub in fguard.iter_mut() {
                    if sub.last_stride != Some(stride_idx) {
                        sub.steps += 1;
                        sub.last_stride = Some(stride_idx);
                        sub.changed_in_stride = false;
                    }
                }
            }
            if let Some(decoded) = msg.as_ref().and_then(|m| dashboard_follow::decode(&m.bytes).ok()) {
                for (id, _, body) in decoded.answers {
                    if let Some(sub) = fguard.iter_mut().find(|s| s.id == id) {
                        let changed = sub.last_body.as_deref() != Some(body);
                        sub.last_body = Some(body.to_vec());
                        if measure {
                            sub.answers += 1;
                            sub.bytes += body.len() as u64;
                            if changed && !sub.changed_in_stride {
                                sub.changed_in_stride = true;
                                sub.changed += 1;
                            }
                        }
                    }
                }
            }
        }

        let mut agg_guard = aggs_hook.lock().unwrap();
        for (i, panel) in panels.iter_mut().enumerate() {
            let agg = &mut agg_guard[i];
            let declared_touched = if panel.tables.is_empty() {
                true
            } else {
                panel.tables.iter().any(|(t, cols)| step.table(*t).is_some_and(|c| change_touches(c, cols)))
            };
            let send_now = step.reset() || std::mem::take(&mut panel.resend);
            let c0 = thread_cycles();
            let reading = (panel.read)(
                step.last(),
                send_now,
                &token,
                Some(ProductStep { step, lanes, held: None, wake: None }),
            );
            let reading = if reading.pending && panel.last_hour == i32::MIN {
                (panel.read)(step.last(), true, &token, None)
            } else {
                reading
            };

            if !reading.lanes.is_empty() {
                for l in &reading.lanes {
                    panel.tables.entry(l.table).or_default().extend(l.columns.iter().copied());
                }
                new_lanes.extend(reading.lanes.iter().cloned());
            }

            if reading.pending {
                let compute_c = thread_cycles().saturating_sub(c0);
                panel.resend = true;
                if measure {
                    agg.enter_stride(stride_idx, 0);
                    if agg.mark(1) { agg.pending += 1; }
                    agg.uses_lanes = !panel.tables.is_empty();
                    if declared_touched && agg.mark(2) { agg.declared_touched += 1; }
                    agg.compute_cycles += compute_c;
                    agg.rows_sum += reading.rows;
                    rep.panels_compute_cycles += compute_c;
                }
                continue;
            }

            let raw_ptr_same = !send_now && panel.last_input.as_ref().is_some_and(|prev| Arc::ptr_eq(prev, &reading.value));
            let delivered = if raw_ptr_same && let Some(prev) = &panel.last_delivered {
                prev.clone()
            } else {
                sync_live(
                    &follow_live,
                    (i as u32) % 2,
                    &mut panel.live_groups,
                    panel.last_delivered.as_ref(),
                    &reading.value,
                    reading.live.as_deref(),
                    step.last(),
                    send_now,
                    reading.fresh,
                    &mut new_lanes,
                )
            };
            let compute_c = thread_cycles().saturating_sub(c0);

            let ptr_same = !send_now && panel.last_delivered.as_ref().is_some_and(|prev| Arc::ptr_eq(prev, &delivered));
            let (encode_c, encoded, changed) = if ptr_same {
                (0, None, false)
            } else {
                let e0 = thread_cycles();
                let bytes = match &*delivered {
                    Ok(v) => rmp_serde::to_vec(v).expect("msgpack"),
                    Err(reason) => reason.as_bytes().to_vec(),
                };
                let ec = thread_cycles().saturating_sub(e0);
                let ch = send_now || panel.last_bytes != bytes;
                (ec, Some(bytes), ch)
            };

            let whole_len = encoded.as_ref().map_or(panel.last_bytes.len(), Vec::len);
            let (diff_c, raw_pb, wire_pb) = if measure && changed {
                match (panel.last_delivered.as_deref(), &*delivered) {
                    (Some(Ok(prev_v)), Ok(cur_v)) => {
                        let d0 = thread_cycles();
                        let pb = patch_bytes(prev_v, cur_v);
                        (thread_cycles().saturating_sub(d0), pb, pb.min(whole_len))
                    }
                    _ => (0, 0, whole_len),
                }
            } else {
                (0, 0, 0)
            };

            if step.reset() {
                agg.seed_bytes = whole_len;
                agg.seed_cycles = compute_c + encode_c;
                agg.uses_lanes = !panel.tables.is_empty();
            } else if measure {
                agg.enter_stride(stride_idx, whole_len as u64);
                agg.uses_lanes = !panel.tables.is_empty();
                if declared_touched && agg.mark(2) { agg.declared_touched += 1; }
                if !ptr_same && agg.mark(4) { agg.ptr_changed += 1; }
                if changed && agg.mark(8) { agg.bytes_changed += 1; }
                if panel.last_hour != i32::MIN && reading.from > panel.last_hour + 1 && agg.mark(16) { agg.unproven += 1; }
                agg.compute_cycles += compute_c;
                agg.encode_cycles += encode_c;
                agg.diff_cycles += diff_c;
                if changed {
                    agg.emitted_bytes_sum += whole_len as u64;
                    agg.raw_patch_bytes_sum += raw_pb as u64;
                    agg.wire_patch_bytes_sum += wire_pb as u64;
                    rep.panels_emitted_bytes += whole_len as u64;
                    rep.panels_raw_patch_bytes += raw_pb as u64;
                    rep.panels_wire_patch_bytes += wire_pb as u64;
                }
                agg.rows_sum += reading.rows;
                rep.panels_compute_cycles += compute_c;
                rep.panels_encode_cycles += encode_c;
                rep.panels_diff_cycles += diff_c;
            }

            if let Some(bytes) = encoded {
                panel.last_bytes = bytes;
            }
            panel.last_input = Some(reading.value);
            panel.last_delivered = Some(delivered);
            panel.last_hour = step.last();
        }
        drop(agg_guard);
        let cpu1 = thread_cpu_ns();
        let cyc1 = thread_cycles();
        let mut hc = hook_cpu_ref.lock().unwrap();
        hc.0 += cpu1.saturating_sub(cpu0);
        hc.1 += cyc1.saturating_sub(cyc0);
        let _ = send.send((Some(rep), new_lanes));
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
    let mut step_reports: Vec<StepReport> = Vec::new();
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
                if let Some(rep) = rep {
                    finished = rep.last;
                    if rep.measured {
                        if let Some(prev) = step_reports.last_mut().filter(|p| p.stride == rep.stride) {
                            prev.last = rep.last;
                            prev.build_cycles += rep.build_cycles;
                            prev.build_ns += rep.build_ns;
                            prev.panels_compute_cycles += rep.panels_compute_cycles;
                            prev.panels_encode_cycles += rep.panels_encode_cycles;
                            prev.panels_diff_cycles += rep.panels_diff_cycles;
                            prev.panels_emitted_bytes += rep.panels_emitted_bytes;
                            prev.panels_raw_patch_bytes += rep.panels_raw_patch_bytes;
                            prev.panels_wire_patch_bytes += rep.panels_wire_patch_bytes;
                            prev.follow_cycles += rep.follow_cycles;
                            prev.follow_bytes += rep.follow_bytes;
                            prev.follow_answers += rep.follow_answers;
                            prev.follow_touched = prev.follow_touched.max(rep.follow_touched);
                        } else {
                            step_reports.push(rep);
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(io::Error::other("playhead disconnected early")),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    drop(seat);
    ph.finish();

    let (total_cpu_ns, total_cycles) = *hook_cpu.lock().unwrap();
    let ns_per_cycle = (total_cpu_ns > 0 && total_cycles > 0).then(|| total_cpu_ns as f64 / total_cycles as f64);
    let to_ms = |cyc: f64| ns_per_cycle.map(|npc| (cyc * npc) / 1e6);

    let n = step_reports.len().max(1) as f64;
    let mean = |f: fn(&StepReport) -> u64| step_reports.iter().map(f).sum::<u64>() as f64 / n;
    let mean_u32 = |f: fn(&StepReport) -> u32| step_reports.iter().map(|s| u64::from(f(s))).sum::<u64>() as f64 / n;

    let aggs_final = aggs.lock().unwrap();
    let per_product: Vec<Value> = aggs_final
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let (name, product, duplicate) = panel_meta[i];
            let s = a.steps.max(1) as f64;
            let rows = if !a.uses_lanes && a.rows_sum == 0 {
                json!("not counted for history scans")
            } else {
                json!(a.rows_sum as f64 / s)
            };
            json!({
                "name": name,
                "product": product,
                "duplicate": duplicate,
                "usesLanes": a.uses_lanes,
                "steps": a.steps,
                "pendingSteps": a.pending,
                "touchedFrac": a.declared_touched as f64 / s,
                "ptrChangedFrac": a.ptr_changed as f64 / s,
                "changedFrac": a.bytes_changed as f64 / s,
                "unprovenFrac": a.unproven as f64 / s,
                "computeMs": to_ms(a.compute_cycles as f64 / s),
                "encodeMs": to_ms(a.encode_cycles as f64 / s),
                "diffMs": to_ms(a.diff_cycles as f64 / s),
                "computeCycles": a.compute_cycles as f64 / s,
                "encodeCycles": a.encode_cycles as f64 / s,
                "diffCycles": a.diff_cycles as f64 / s,
                "wholeBytes": a.whole_bytes_sum as f64 / s,
                "emittedBytes": a.emitted_bytes_sum as f64 / s,
                "rawPatchBytes": a.raw_patch_bytes_sum as f64 / s,
                "patchBytes": a.wire_patch_bytes_sum as f64 / s,
                "rowsPerStep": rows,
                "seedBytes": a.seed_bytes,
                "seedMs": to_ms(a.seed_cycles as f64),
            })
        })
        .collect();

    let follow_ms = to_ms(mean(|s| s.follow_cycles));
    let follow_touched_mean = mean_u32(|s| s.follow_touched);
    let follow_guard = follow_aggs.lock().unwrap();
    let isolated_follow = panel_meta.is_empty() && follow_guard.len() == 1;
    let follow_products: Vec<Value> = follow_guard
        .iter()
        .map(|sub| {
            let s = sub.steps.max(1) as f64;
            json!({
                "name": sub.name,
                "steps": sub.steps,
                "answersFrac": sub.answers as f64 / s,
                "changedFrac": sub.changed as f64 / s,
                "emittedBytes": sub.bytes as f64 / s,
                "computeMs": isolated_follow.then_some(follow_ms).flatten(),
                "touchedFrac": isolated_follow.then_some(follow_touched_mean),
                "sharedEngine": !isolated_follow,
            })
        })
        .collect();
    let follow_names: Vec<&str> = follow_guard.iter().map(|s| s.name).collect();
    drop(follow_guard);

    let dedup_cycles = |pick: fn(&ProductAgg) -> u64| -> f64 {
        aggs_final
            .iter()
            .enumerate()
            .filter(|(i, _)| !panel_meta[*i].2)
            .map(|(_, a)| pick(a) as f64 / a.steps.max(1) as f64)
            .sum()
    };
    let panels_whole_bytes: f64 = aggs_final.iter().map(|a| a.whole_bytes_sum as f64 / a.steps.max(1) as f64).sum();
    let panels_touched: f64 = aggs_final.iter().map(|a| a.declared_touched as f64 / a.steps.max(1) as f64).sum();
    let panels_ptr_changed: f64 = aggs_final.iter().map(|a| a.ptr_changed as f64 / a.steps.max(1) as f64).sum();
    let panels_changed: f64 = aggs_final.iter().map(|a| a.bytes_changed as f64 / a.steps.max(1) as f64).sum();
    let (pc, pe, pd, fc) = (mean(|s| s.panels_compute_cycles), mean(|s| s.panels_encode_cycles), mean(|s| s.panels_diff_cycles), mean(|s| s.follow_cycles));

    let out = json!({
        "recording": recording, "branch": branch_id, "tag": tag,
        "first": first, "last": last, "quantum": quantum, "steadySteps": step_reports.len(),
        "followQueries": follow_names,
        "calibration": {
            "hookThreadCpuMs": (total_cpu_ns > 0).then(|| total_cpu_ns as f64 / 1e6),
            "hookThreadCycles": total_cycles,
            "nsPerCycle": ns_per_cycle,
            "ghz": ns_per_cycle.map(|npc| 1.0 / npc),
        },
        "perStride": {
            "playheadBuildMs": to_ms(mean(|s| s.build_cycles)),
            "playheadBuildWallMs": mean(|s| s.build_ns) / 1e6,
            "panelsComputeMs": to_ms(pc), "panelsEncodeMs": to_ms(pe), "panelsDiffMs": to_ms(pd),
            "dedupPanelsComputeMs": to_ms(dedup_cycles(|a| a.compute_cycles)),
            "dedupPanelsEncodeMs": to_ms(dedup_cycles(|a| a.encode_cycles)),
            "dedupPanelsDiffMs": to_ms(dedup_cycles(|a| a.diff_cycles)),
            "followMs": follow_ms,
            "totalHookMs": to_ms(pc + pe + fc),
            "totalMeasuredWithDiffMs": to_ms(pc + pe + pd + fc),
            "totalPatchOnlyMs": to_ms(pc + pd + fc),
            "panelsWholeBytes": panels_whole_bytes,
            "panelsEmittedBytes": mean(|s| s.panels_emitted_bytes),
            "panelsRawPatchBytes": mean(|s| s.panels_raw_patch_bytes),
            "panelsPatchBytes": mean(|s| s.panels_wire_patch_bytes),
            "followBytes": mean(|s| s.follow_bytes),
            "panelsTouched": panels_touched, "panelsPtrChanged": panels_ptr_changed, "panelsChanged": panels_changed,
            "followTouched": follow_touched_mean, "followAnswers": mean_u32(|s| s.follow_answers),
        },
        "products": per_product,
        "followProducts": follow_products,
    });
    println!("{}", serde_json::to_string_pretty(&out).map_err(io::Error::other)?);
    Ok(())
}
