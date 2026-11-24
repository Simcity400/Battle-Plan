
#[path = "native-bench/widgets.rs"]
mod widgets;

use dashboard_base::{Source, SourceKey, products::at_hour};
use dashboard_core::config::{ContentOptions, LoadContext};
use dashboard_follow::{Identity, lead, panel::dated};
use playhead::{Piece, Playhead, Settings, Timeline, pages::{Span, WorldPages}};
use replay_data::rows::{TableId, tables};
use serde_json::json;
use std::{collections::VecDeque, io, path::PathBuf, sync::{Arc, mpsc}, time::{Duration, Instant}};
use widgets::Packet;

#[global_allocator]
static ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

const SPEED: f64 = 1440.0;
const QUANTUM: i32 = 24;
const LEAD: i32 = 720;

fn widen<'a>(declared: &mut std::collections::BTreeMap<usize, playhead::Declared>, lanes: impl IntoIterator<Item = &'a playhead::Declared>) -> bool {
    let mut widened = false;
    for lane in lanes {
        let held = declared.entry(lane.table).or_insert_with(|| { widened = true; lane.clone() });
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

#[cfg(windows)]
fn cpu() -> u64 {
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::{GetCurrentProcess, GetProcessTimes}};
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    assert_ne!(unsafe { GetProcessTimes(GetCurrentProcess(), created, exited, kernel, user) }, 0);
    [kernel, user].iter().map(|t| u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime)).sum::<u64>() * 100
}

#[cfg(not(windows))]
fn cpu() -> u64 { panic!("This benchmark measures Windows process CPU time") }

#[cfg(windows)]
fn cycles() -> u64 {
    use windows_sys::Win32::System::{Threading::GetCurrentProcess, WindowsProgramming::QueryProcessCycleTime};
    let mut count = 0;
    assert_ne!(unsafe { QueryProcessCycleTime(GetCurrentProcess(), &raw mut count) }, 0);
    count
}

#[cfg(not(windows))]
fn cycles() -> u64 { unreachable!() }

struct Batch {
    hour: i32,
    packets: Vec<Vec<u8>>,
    bytes: usize,
}

#[derive(Default)]
struct Window {
    queued: VecDeque<Batch>,
    bytes: usize,
}

impl Window {
    fn consume(&mut self, clock: i32) {
        while self.queued.front().is_some_and(|batch| batch.hour <= clock) {
            self.bytes -= self.queued.pop_front().unwrap().bytes;
        }
    }

    fn offer(&mut self, hour: i32, packets: Vec<Vec<u8>>, lane_bytes: usize) -> io::Result<usize> {
        let bytes = packets.iter().map(Vec::len).sum();
        self.bytes += bytes;
        self.queued.push_back(Batch { hour, packets, bytes });
        let mut merged = 0;
        while self.bytes + lane_bytes > lead::BYTES && self.queued.len() >= 2 {
            let older = self.queued.pop_front().unwrap();
            let newer = self.queued.front_mut().unwrap();
            self.bytes -= older.bytes + newer.bytes;
            for (old, new) in older.packets.iter().zip(&mut newer.packets) {
                let merge = if new.starts_with(dated::MAGIC) { dated::merge } else { dashboard_follow::merge };
                *new = merge(old, new)?.0;
            }
            newer.bytes = newer.packets.iter().map(Vec::len).sum();
            self.bytes += newer.bytes;
            merged += 1;
        }
        Ok(merged)
    }
}

fn standing_cost() {
    use dashboard_follow::{panel::product::take_due, push::{PANEL, Push}};
    use std::{collections::BTreeSet, hint::black_box};

    let push = Push::default();
    let (session, mut released) = (push.session(), 0u64);
    let mut samples = Vec::new();
    const STEPS: i32 = 100_000;
    for round in 0..9 {
        for index in 0..3 {
            let subscriptions = [1, 15, 114][(round + index) % 3];
            let mut due: BTreeSet<_> = (0..subscriptions).map(|id| (STEPS + 1, id)).collect();
            let start = Instant::now();
            for hour in 0..STEPS {
                assert!(take_due(black_box(&mut due), hour).is_none());
                push.offer_kind(PANEL, dated::ProductBatch::new((0, 1, 0)).finish(Some(hour)));
                if let Some((_, bytes, ..)) = black_box(push.take_kind()) {
                    released += dashboard_follow::charge(&bytes) as u64;
                    push.credit(session, released);
                }
            }
            samples.push(json!({"subscriptions": subscriptions, "round": round,
                "nsPerStep": start.elapsed().as_nanos() as f64 / f64::from(STEPS)}));
        }
    }
    println!("{}", json!({"stepsPerSample": STEPS, "samples": samples, "pressureMerges": push.counters().pressure_merges}));
}

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--standing-cost"] {
        standing_cost();
        return Ok(());
    }
    let [recording, game, user, cache, tag, flags @ ..] = &args[..] else {
        return Err(io::Error::other("native-bench RECORDING GAME_DIR USER_DIR CACHE_DIR TAG [--audit|--capacity|--hourly-check]"));
    };
    let audit = flags == ["--audit"];
    let capacity = flags == ["--capacity"];
    let hourly = flags == ["--hourly-check"];
    if !flags.is_empty() && !hourly && !audit && !capacity { return Err(io::Error::other("unknown option")); }
    let path = PathBuf::from(recording);
    let source = Source::open(SourceKey { source_id: "native-bench".into(), revision: dashboard_core::protocol::replay_revision(&path)? }, path.clone(), &false.into())?;
    engine_executable::executable_tables::enter(source.tables());
    let load = LoadContext { content: Some(ContentOptions {
        options: content_reader_layers::ResolveOptions { game_dir: game.into(), user_dir: user.into(), cache_dir: cache.into() },
        allow_unverified: false, presentation: Default::default(),
    }) };
    let metadata = source.metadata().map_err(io::Error::other)?;
    let branch = metadata.branches.iter().find(|b| b.main).ok_or_else(|| io::Error::other("no main branch"))?;
    let q = dashboard_base::queries::query(&source, branch.id.parse().map_err(io::Error::other)?)?;
    let opened = q.timeline.stretches.iter().map(|s| s.first).min().ok_or_else(|| io::Error::other("the branch holds no recording"))?;
    let closed = q.timeline.stretches.iter().map(|s| s.last).max().unwrap_or(opened);
    let start = opened + (closed - opened) * 3 / 10;
    let span = q.timeline.stretches.iter().find(|s| s.first <= start && start + 3600 + QUANTUM <= s.last)
        .ok_or_else(|| io::Error::other("no continuous recording for 3,600 hours from where the acceptance suite plays"))?;
    let pages = Arc::new(WorldPages::open(&path, Span::Run(span.run as usize))?);
    let first = start + (pages.pieces()[0].first - 1 - start).rem_euclid(QUANTUM);
    let end = first + 3600;
    let last = if hourly { first + QUANTUM } else { end };
    let reads = [TableId::Countries, TableId::Projects, TableId::ProjectHistory, TableId::ProductionLines].map(at_hour::Read::all);
    let census = at_hour::frame(&q, first, &reads, "country census")?;
    let countries = census.complete::<tables::Countries>()?.len();
    let tag = if tag == "auto" {
        let mut activity = std::collections::BTreeMap::<u32, (usize, usize, usize)>::new();
        for row in census.complete::<tables::Projects>()? {
            activity.entry(row.owner).or_default().0 += usize::from(row.started != 0);
        }
        for row in census.complete::<tables::ProjectHistory>()? {
            activity.entry(row.owner).or_default().1 += 1;
        }
        for row in census.complete::<tables::ProductionLines>()? {
            activity.entry(row.country).or_default().2 += 1;
        }
        q.holders.tags.keys().filter_map(|tag| {
            let dashboard_query::identity::Holder::Found { id, .. } = q.holders.at(tag, first)?.holder else { return None };
            Some((activity.get(&id)?, tag))
        }).max().ok_or_else(|| io::Error::other("no held tag with recorded activity; supply a tag"))?.1
    } else { tag };
    let identity = Identity { timeline: q.timeline.clone(), holders: q.holders.clone() };
    let mut products = source.in_scope(|| widgets::page(&source, &load, &identity, tag))?;
    let names: Vec<_> = products.iter().map(|w| w.name).collect();
    let mut declared = std::collections::BTreeMap::<usize, playhead::Declared>::new();
    widen(&mut declared, products.iter().flat_map(|w| &w.declared));
    let timeline = Timeline::new(&*pages, vec![Piece { run: 0, first, last }])?;
    eprintln!("{countries} countries; {tag}, branch {}, {first}..{last}, measuring {} products", branch.id, products.len());
    let origin = Instant::now();
    let ms = |time: Instant| time.duration_since(origin).as_secs_f64() * 1000.0;
    let (send, receive) = mpsc::channel();
    let audit_pages = audit.then(|| pages.clone());
    let hook: playhead::Hook = Box::new(move |step, lanes| {
        let metrics: Vec<_> = products.iter_mut().enumerate().map(|(i, w)| {
            if audit { return (0, 0, 0, w.step(i as u32, step, lanes, audit_pages.as_ref())); }
            let before = cpu();
            let ticks = cycles();
            let start = Instant::now();
            let packet = w.step(i as u32, step, lanes, None);
            (start.elapsed().as_nanos() as u64, cycles() - ticks, cpu() - before, packet)
        }).collect();
        send.send((step.first(), step.last(), step.reset(), step.built().ns, Instant::now(), metrics)).unwrap();
    });
    let ph = Playhead::new(pages, timeline, Settings { step_threads: 2, rate: if hourly { SPEED } else { 60.0 }, ..Settings::default() }, Some(hook))?;
    ph.lead(lead::LEAD);
    let reader = ph.declare(declared.values().cloned().collect(), None)?;
    let mut seat = ph.consumer(reader);
    ph.seek(first);
    let mut start = None;
    let mut trace = Vec::new();
    let mut seed = Vec::new();
    let mut bodies = vec![None; names.len()];
    let mut window = Window::default();
    let mut through = vec![i32::MIN; names.len()];
    let mut taken = Vec::new();
    let mut finished = first - 1;
    let mut mismatches = 0;
    let mut full_lead_at_start = None;
    while finished < last {
        if let Some(why) = ph.error() { return Err(io::Error::other(why)); }
        let now = Instant::now();
        let newest = ph.newest().unwrap_or(first);
        let head = match start {
            None => first,
            Some(_) if capacity => newest,
            Some(_) => ph.head(now),
        };
        let consumed = head.min(newest);
        seat.advance(consumed);
        seat.take(consumed, &mut taken);
        taken.clear();
        window.consume(head);
        let ring = ph.lead_held();
        if start.is_none() && (ring.past >= ring.steps || ring.capped || hourly && ph.newest().is_some()) {
            start = Some(now);
            full_lead_at_start = Some(newest - first);
            ph.anchor(f64::from(first), true, SPEED, now, false);
        }
        match receive.recv_timeout(Duration::from_millis(1)) {
            Ok((from, hour, reset, build_ns, ready, values)) => {
                if hour - from + 1 > if hourly { 1 } else { QUANTUM } { return Err(io::Error::other("oversized step")); }
                let clock = if capacity { head.min(hour) } else { start.map_or(first, |s: Instant| first + ((ms(ready) - ms(s)).max(0.0) * SPEED / 1000.0) as i32) };
                let mut rows = Vec::new();
                let mut packets = Vec::new();
                let mut widened = false;
                for (i, (wall_ns, cycles, cpu_ns, Packet { bytes, through: confirmed, data, refused, rows: read, cold_rows, mismatch, lanes })) in values.into_iter().enumerate() {
                    widened |= widen(&mut declared, &lanes);
                    if let Some(why) = &mismatch { mismatches += 1; eprintln!("{} at {hour}: {why}", names[i]); }
                    through[i] = confirmed;
                    if data {
                        let body = if bytes.starts_with(dated::MAGIC) { dated::records(&bytes)?[0].body }
                            else { dashboard_follow::decode(&bytes)?.answers[0].2 };
                        bodies[i] = Some(blake3::hash(body).to_hex().to_string());
                    }
                    if reset && bytes.starts_with(dated::MAGIC) {
                        let records = dated::records(&bytes)?;
                        let record = &records[0];
                        if record.kind == dated::DATA {
                            let value: serde_json::Value = rmp_serde::from_slice(record.body).map_err(io::Error::other)?;
                            let counts: std::collections::BTreeMap<_, _> = value.as_object().into_iter().flatten()
                                .filter_map(|(k, v)| Some((k, v.as_array()?.len()))).collect();
                            seed.push(json!({ "name": names[i], "arrays": counts, "missing": value["missing"], "unavailable": value["unavailable"] }));
                        } else {
                            seed.push(json!({ "name": names[i], "refused": String::from_utf8_lossy(record.body) }));
                        }
                    } else if reset {
                        let decoded = dashboard_follow::decode(&bytes)?;
                        if let Some((_, _, body)) = decoded.answers.first() {
                            let answer = dashboard_follow::Answer::parse(body)?;
                            seed.push(json!({ "name": names[i], "rows": answer.rows(), "refused": answer.not_recorded }));
                        }
                    }
                    rows.push(json!([wall_ns, cpu_ns, bytes.len(), data, refused, confirmed, bodies[i], cycles, read, cold_rows, mismatch]));
                    packets.push(bytes);
                }
                if widened { ph.redeclare(reader, declared.values().cloned().collect())?; }
                let merges = window.offer(hour, packets, ring.bytes)?;
                finished = hour;
                if audit && trace.len() % 30 == 0 { eprintln!("step {} at {hour}, {:.1}s elapsed", trace.len(), ms(ready) / 1000.0); }
                trace.push(json!({ "from": from, "hour": hour, "reset": reset, "readyMs": ms(ready), "buildNs": build_ns,
                    "leadHours": i64::from(*through.iter().min().unwrap()) - i64::from(clock), "clock": clock,
                    "cap": ring.capped || window.bytes + ring.bytes > lead::BYTES, "merges": merges,
                    "wireHeld": window.bytes, "laneBytes": ring.bytes, "products": rows }));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(io::Error::other("playhead stopped before the last step")),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    drop(seat);
    ph.finish();
    println!("{}", json!({ "recording": recording, "recordingBytes": path.metadata()?.len(), "format": metadata.format,
        "branch": branch.id, "branchFirst": branch.first_hour, "branchLast": branch.last_hour,
        "first": first, "last": last, "targetLeadHours": LEAD, "countriesAtStart": countries, "catalogue": metadata.countries.len(), "tag": tag,
        "reportedParallelism": std::thread::available_parallelism()?.get(), "prefillMs": ms(start.expect("clock started")),
        "prefillHours": full_lead_at_start, "byteBound": lead::BYTES, "audit": audit, "capacity": capacity, "hourly": hourly, "names": names, "seed": seed,
        "columns": ["wallNs","processCpuNs","wireBytes","data","refused","through","bodyBlake3","processCycles","rowsRead","coldRows","mismatch"], "steps": trace }));
    if mismatches != 0 { return Err(io::Error::other(format!("{mismatches} payloads differ from exact reads"))); }
    Ok(())
}
