
use super::{Caches, Family, HoverRows, Inputs, Job, Key, Posted, Retry, What, id, make};
use viewer_layer_core::lane_rows::rosters::Lanes;
use viewer_counter_products::map::province_tooltips::air_region::{RosterHistory, SampleSource, Sampler, Span};
use crate::{
    app::picker,
    document::Document,
    playhead_host::{
        Entry,
        views::{Need, Pace, Reader as ViewReader, Views},
    },
};
use playhead::Step;
use replay::campaign::SeekResult;
use replay_data::{
    Hour,
    rows::{TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};
use viewer_counter_products::map::{counter_tooltips::Style, province_tooltips::ModeHover};
use viewer_gpu::{
    camera::{Camera, MapSize, Placement},
    layers::Omissions,
    province_geometry::ProvinceGeometry,
};
use viewer_native_gui::counter_hovers::{land::CounterHover, task_force, task_force_port};
use viewer_foundation::{perspective::{self, Vision}, tooltip_text::Tooltip};
use viewer_map_modes::map_mode::{MapMode, Stock};

const LIMIT: Duration = Duration::from_secs(120);
const VIEWPORT: [u32; 2] = [2048, 1152];
const NEAR: f32 = 400.0;
const CAMERAS: usize = 3;
const PROVINCES: usize = 16;

#[derive(Default)]
struct Targets {
    vision: Option<Arc<Vision>>,
    land: Vec<CounterHover>,
    task_forces: Vec<task_force::Hover>,
    ports: Vec<task_force_port::Hover>,
    air_bases: BTreeSet<(u32, i32)>,
    naval_bases: BTreeSet<u32>,
    naval: BTreeSet<(u32, bool)>,
    occupation: BTreeSet<(u32, String)>,
    provinces: Vec<u32>,
    modes: Vec<ModeHover>,
}

struct Reader {
    family: Family,
    reader: Option<ViewReader>,
    timed: Option<(Key, MakeJob)>,
    tables: Vec<TableId>,
    frames: Vec<(u64, u64)>,
    ages: Vec<f64>,
}

pub(crate) struct Line {
    views: Views,
    readers: Vec<Reader>,
    hovers: Option<(HoverRows, Targets)>,
    steps: Option<(playhead::Reader, playhead::Consumer, (Vec<u64>, Vec<u64>), BTreeMap<usize, u64>)>,
    entry: Arc<Entry>,
    geometry: ProvinceGeometry,
    caches: Caches,
    today_caches: Caches,
    totals: BTreeMap<String, (u64, u64, u64)>,
    cycle_hz: f64,
    rosters: Option<(Arc<Lanes>, RosterHistory, RosterHistory)>,
    scan: Option<Value>,
}

struct Shared(Arc<Lanes>);

impl SampleSource for Shared {
    fn span(&self, branch: replay_data::BranchId, hour: Hour, superseded: &dyn Fn() -> bool) -> Result<Span, String> {
        self.0.span(branch, hour, superseded)
    }
}

const COUNTRIES: usize = 4;

impl Line {
    pub(crate) fn open(entry: &Arc<Entry>, views: Views, document: &Document, which: &str, timing: bool, cycle_hz: f64) -> io::Result<Self> {
        let (which, seeded) = match which.split_once('@') {
            Some((which, file)) => (which, seeds(file)?),
            None => (which, BTreeMap::new()),
        };
        let families: Vec<Family> = match which {
            "all" => Family::ALL.to_vec(),
            "none" | "scan-naval" => Vec::new(),
            name => vec![Family::ALL.into_iter().find(|family| family.name() == name).ok_or_else(|| io::Error::other(format!("--hovers takes all, none or a family ({}), not {name}", Family::ALL.map(Family::name).join(", "))))?],
        };
        let seed = |family: Family| seeded.get(family.name()).cloned().unwrap_or_default();
        let readers: Vec<Reader> = if families.is_empty() || timing {
            Vec::new()
        } else {
            families
                .iter()
                .map(|&family| Reader {
                    family,
                    reader: Some(views.reader_at(family.name(), &seed(family).into_iter().map(Need::every).collect::<Vec<Need>>(), Pace::Step)),
                    timed: None,
                    tables: seed(family),
                    frames: Vec::new(),
                    ages: Vec::new(),
                })
                .collect()
        };
        let steps = timing.then(|| {
            let playhead = entry.playhead();
            playhead.declare(Vec::new(), None).map(|reader| (reader, playhead.consumer(reader), (Vec::new(), Vec::new()), BTreeMap::new()))
        });
        let rosters = (!timing && !families.is_empty())
            .then(|| Lanes::of(entry, document.playback.replay()))
            .flatten()
            .map(|lanes| {
                let lanes = Arc::new(lanes);
                let replay = document.playback.replay_handle();
                (Arc::clone(&lanes), RosterHistory::of(Arc::new(Sampler::new(Arc::clone(&replay), Box::new(Shared(lanes))))), RosterHistory::of(Arc::new(Sampler::today(replay, None))))
            });
        let scan = if which == "scan-naval" { Some(scan_naval(document)?) } else { None };
        let mut line = Self { views, readers, hovers: None, steps: steps.transpose()?, entry: Arc::clone(entry), geometry: ProvinceGeometry::build(&document.map), caches: Caches::default(), today_caches: Caches::default(), totals: BTreeMap::new(), cycle_hz, rosters, scan };
        if timing && !families.is_empty() {
            let targets = line.targets(document, None)?;
            let made = inputs(document, &targets);
            line.readers = families
                .iter()
                .filter(|&&family| family != Family::Capture)
                .map(|&family| Reader { family, reader: None, timed: made.as_ref().and_then(|inputs| all(family, document, &targets, inputs).into_iter().next().map(|(_, key, job)| (key, job))), tables: seed(family), frames: Vec::new(), ages: Vec::new() })
                .collect();
            line.hovers = Some((HoverRows::seeded(&seed), targets));
        }
        Ok(line)
    }

    pub(crate) fn frame(&mut self, head: Hour) {
        if let Some((_, consumer, built, events)) = &mut self.steps {
            let mut taken: Vec<Arc<Step>> = Vec::new();
            consumer.take(head, &mut taken);
            for step in &taken {
                if step.reset() {
                    built.1.push(step.built().ns);
                    continue;
                }
                built.0.push(step.built().ns);
                for changes in step.tables() {
                    let written: usize = changes.columns().iter().map(|written| written.slots().len()).sum();
                    let lives = changes.inserted().slots().len() + changes.deleted().keys().map_or(0, <[u128]>::len);
                    *events.entry(changes.table()).or_default() += (written + lives) as u64;
                }
            }
        }
        let Some((hovers, _)) = &mut self.hovers else { return };
        let head_at = &self.views;
        for reader in &mut self.readers {
            let Some((key, job)) = &reader.timed else { continue };
            let (began, cycles) = (Instant::now(), replay_frame::cpu::thread_cycles());
            hovers.ask(reader.family, head_at, key.clone(), |caches| job(caches));
            reader.frames.push((began.elapsed().as_nanos() as u64, replay_frame::cpu::thread_cycles().saturating_sub(cycles)));
            if let Some(made) = hovers.shown(reader.family) {
                reader.ages.push(f64::from(head - made.hour));
            }
        }
        hovers.end_frame(false);
    }

    pub(crate) fn compare(&mut self, document: &Document, hour: Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        if self.readers.is_empty() || self.hovers.is_some() {
            return Ok(());
        }
        let today: Arc<DecodedFrame> = match document.playback.replay().seek_selected(document.playback.branch(), hour, None)? {
            SeekResult::Frame(frame) => Arc::new(*frame),
            SeekResult::Gap { reason, .. } => return Err(io::Error::other(format!("Today's reader finds a gap at {hour}: {reason}"))),
        };
        let targets = self.targets(document, Some(&today))?;
        self.rosters(document, &today, hour, why, out)?;
        let Some(inputs) = inputs(document, &targets) else { return Ok(()) };
        for at in 0..self.readers.len() {
            let family = self.readers[at].family;
            let name = family.name();
            let Some(reader) = &self.readers[at].reader else { continue };
            if reader.frame().is_none() {
                self.note(format!("{name}:loading"), (0, 0, None), json!({}), hour, why, out)?;
                continue;
            }
            let Some(rows) = at_hour(reader, hour)? else {
                self.note(format!("{name}:ahead"), (0, 0, None), json!({}), hour, why, out)?;
                continue;
            };
            let jobs = all(family, document, &targets, &inputs);
            let (mut entries, mut differing, mut first, mut missed) = (0_u64, 0_u64, None, Vec::new());
            for (name, key, job) in jobs {
                let (wanted, _) = made_whole(&self.today_caches, key.clone(), &today, false, job(&self.today_caches));
                let wanted = wanted.as_deref().map(text).unwrap_or_default();
                let (got, lacked) = made_whole(&self.caches, key, &rows, true, job(&self.caches));
                let Some(got) = got.filter(|_| lacked.is_empty()) else {
                    missed.extend(lacked);
                    continue;
                };
                entries += 1;
                let got = text(&got);
                if wanted != got {
                    differing += 1;
                    first.get_or_insert_with(|| format!("{name}: {}", first_difference(&wanted, &got)));
                }
            }
            if !missed.is_empty() {
                missed.sort_unstable_by_key(|table| table.index());
                missed.dedup();
                let named: Vec<String> = missed.iter().map(|table| format!("{table:?}")).collect();
                let reader = &mut self.readers[at];
                reader.tables.extend(missed);
                reader.tables.sort_unstable_by_key(|table| table.index());
                reader.tables.dedup();
                if let Some(rows) = &reader.reader {
                    rows.declare_only(&reader.tables);
                }
                let declared = reader.tables.len();
                self.note(format!("{name}:declared"), (0, 0, None), json!({"declared": declared, "named": named}), hour, why, out)?;
                continue;
            }
            self.note(name.to_owned(), (entries, differing, first), json!({"tables": self.readers[at].tables.len()}), hour, why, out)?;
        }
        Ok(())
    }

    fn targets(&self, document: &Document, every: Option<&DecodedFrame>) -> io::Result<Targets> {
        let frame = document.frame()?;
        let every = every.unwrap_or(frame);
        let map = MapSize::of(&document.map);
        let mut cameras = vec![Camera::fit(map, document.map_defines, VIEWPORT)];
        if let Ok(movement) = frame.complete::<tables::DivisionMovement>() {
            let mut held: BTreeMap<u32, usize> = BTreeMap::new();
            for row in movement {
                *held.entry(row.source).or_default() += 1;
            }
            let mut busiest: Vec<(u32, usize)> = held.into_iter().collect();
            busiest.sort_by_key(|&(province, count)| (std::cmp::Reverse(count), province));
            cameras.extend(busiest.iter().filter_map(|&(province, _)| self.geometry.centroid(province)).take(CAMERAS).map(|center| Placement::map(center, NEAR).on(map, document.map_defines)));
        }
        let settings = viewer_counter_kit::CounterSettings { color_mode: viewer_counter_kit::CounterColorMode::of(document.game_options.country_counter_colours), nato: document.game_options.nato_counters };
        let choice = perspective::Choice::default();
        let mut targets = Targets::default();
        let mut navy_sources = None;
        for stock in [Stock::Default, Stock::StrategicAir, Stock::StrategicNavy, Stock::Resistance] {
            for camera in &cameras {
                let mut overlay = document.dump_overlay(*camera, VIEWPORT, 1.0, &choice, Omissions::default(), settings, &MapMode::Stock(stock))?;
                overlay.present(0.0)?;
                let report = &overlay.counters;
                targets.vision.get_or_insert_with(|| Arc::clone(&report.perspective));
                targets.land.extend(report.land_hovers.iter().cloned());
                targets.task_forces.extend(report.task_force_hovers.iter().filter(|hover| hover.supported).cloned());
                targets.ports.extend(report.task_force_port_hovers.iter().filter(|hover| hover.supported && hover.has_tooltip()).cloned());
                targets.air_bases.extend(report.air_base_hits.values().copied());
                targets.naval_bases.extend(report.naval_base_hits.values().copied());
                for hover in &report.naval_region_hovers {
                    use viewer_counter_kit::NavalRegionalControl;
                    match hover.control {
                        NavalRegionalControl::Mines => targets.naval.insert((hover.region, true)),
                        NavalRegionalControl::Accidents => targets.naval.insert((hover.region, false)),
                        _ => false,
                    };
                }
                for hit in &overlay.hits.occupation {
                    if let viewer_map_icons::occupation_icons::Hover::Modifier { state, token } = &hit.hover {
                        targets.occupation.insert((*state, token.clone()));
                    }
                }
                if stock == Stock::StrategicNavy && navy_sources.is_none() {
                    navy_sources = Some(overlay.naval_dominance_sources());
                }
            }
        }
        if let Ok(mines) = every.complete::<tables::NavalRegionalMines>() {
            targets.naval.extend(mines.iter().map(|row| (row.region, true)));
        }
        if let Ok(reports) = every.complete::<tables::NavalAccidentReports>() {
            targets.naval.extend(reports.iter().flat_map(|row| [(row.region, true), (row.region, false)]));
        }
        let ids: Vec<u32> = frame.complete::<tables::Provinces>().map(|rows| rows.iter().map(|row| row.id).collect()).unwrap_or_default();
        let stride = (ids.len() / PROVINCES).max(1);
        targets.provinces = ids.iter().step_by(stride).copied().take(PROVINCES).collect();
        for stock in Stock::ALL.into_iter().filter(|&stock| stock != Stock::Terrain) {
            let mut mode = picker::mode_hover(&MapMode::Stock(stock), false, document.map_modes.defines.opinion_thresholds(), || Err("the infrastructure level is the infrastructure hover's reader".into()));
            if let (ModeHover::StrategicNavy(sources), Some(prepared)) = (&mut mode, &navy_sources) {
                *sources = Arc::clone(prepared);
            }
            targets.modes.push(mode);
        }
        if let Some(prepared) = &navy_sources {
            targets.modes.push(ModeHover::SeaRegion(Arc::clone(prepared)));
        }
        Ok(targets)
    }

    fn rosters(&mut self, document: &Document, today: &DecodedFrame, hour: Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        let Some((_, lanes, reference)) = &self.rosters else { return Ok(()) };
        let ours = document.hour()?.vision(&perspective::Choice::default())?.ours();
        let mut slots = vec![ours];
        if let Ok(links) = today.complete::<tables::CountryLinks>() {
            slots.extend(links.iter().map(|link| link.canonical_slot).filter(|&slot| slot != ours).take(COUNTRIES));
        }
        let branch = document.playback.branch();
        let (mut entries, mut differing, mut outside, mut refused, mut first) = (0_u64, 0_u64, 0_u64, 0_u64, None);
        for slot in slots {
            let (got, wanted) = (lanes.visiting_pass(branch, hour, slot), reference.visiting_pass(branch, hour, slot));
            if got.as_ref().is_err_and(|why| why.contains("outside the air roster lanes")) {
                outside += 1;
                continue;
            }
            entries += 1;
            match (&got, &wanted) {
                (Ok(got), Ok(wanted)) if got == wanted => {}
                (Err(_), Err(_)) => refused += 1,
                _ => {
                    differing += 1;
                    first.get_or_insert_with(|| format!("slot {slot}: lanes {got:?}, today {wanted:?}"));
                }
            }
        }
        self.note("rosters".into(), (entries, differing, first), json!({"outside_the_lanes": outside, "refused_by_both": refused}), hour, why, out)
    }

    fn note(&mut self, name: String, (entries, differing, first): (u64, u64, Option<String>), notes: Value, hour: Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        let total = self.totals.entry(format!("hovers:{name}")).or_default();
        total.0 += entries;
        total.1 += differing;
        total.2 += 1;
        writeln!(out, "{}", json!({"hour": hour, "why": why, "compare": format!("hovers:{name}"), "entries": entries, "differing": differing, "first": first, "notes": notes}))
    }

    pub(crate) fn differing(&self) -> u64 {
        self.totals.values().map(|(_, differing, _)| differing).sum()
    }

    pub(crate) fn summary(&self, distribution: fn(Vec<f64>) -> Value) -> Value {
        let held = self.views.held();
        let bytes = |list: &[(TableId, usize)], tables: &[TableId]| list.iter().filter(|(table, _)| tables.contains(table)).map(|(_, bytes)| bytes).sum::<usize>();
        let hz = self.cycle_hz;
        let families: Vec<Value> = self
            .readers
            .iter()
            .map(|reader| {
                json!({
                    "family": reader.family.name(), "tables": reader.tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
                    "rows_bytes": bytes(&held, &reader.tables),
                    "window_ms": distribution(reader.frames.iter().map(|&(ns, _)| ns as f64 / 1e6).collect()),
                    "window_cpu_ms": distribution(reader.frames.iter().map(|&(_, cycles)| cycles as f64 / hz * 1e3).collect()),
                    "age_hours": distribution(reader.ages.clone()), "frames": reader.frames.len(), "frames_with_text": reader.ages.len(),
                })
            })
            .collect();
        let all = |list: &[(TableId, usize)]| list.iter().map(|(_, bytes)| bytes).sum::<usize>();
        let (_, peak, _) = self.entry.playhead().held();
        let events = self.steps.as_ref().map(|(_, _, built, events)| {
            let steps = built.0.len().max(1) as f64;
            let mut by: Vec<(&usize, &u64)> = events.iter().collect();
            by.sort_by_key(|(_, events)| std::cmp::Reverse(**events));
            by.iter().take(20).map(|&(&table, &count)| json!({"table": format!("{:?}", TableId::ALL[table]), "events_per_step": count as f64 / steps})).collect::<Vec<_>>()
        });
        json!({
            "compared": self.totals.iter().map(|(name, (entries, differing, samples))| json!({"compare": name, "samples": samples, "entries": entries, "differing": differing})).collect::<Vec<_>>(),
            "families": families,
            "rows_bytes": all(&held),
            "step_ms": self.steps.as_ref().map(|(_, _, built, _)| distribution(built.0.iter().map(|&ns| ns as f64 / 1e6).collect())),
            "steps": self.steps.as_ref().map(|(_, _, built, _)| built.0.len()),
            "reset_step_ms": self.steps.as_ref().map(|(_, _, built, _)| distribution(built.1.iter().map(|&ns| ns as f64 / 1e6).collect())),
            "events_by_table": events,
            "playhead_held_peak": peak.total,
            "naval_scan": self.scan,
            "rosters": self.rosters.as_ref().map(|(lanes, by_lanes, today)| {
                let reads = lanes.reads();
                let held = |history: &RosterHistory| { let (samples, spans, distinct, bytes) = history.sampler().held(); json!({"samples": samples, "spans": spans, "distinct": distinct, "bytes": bytes}) };
                json!({
                    "lane_reads": reads.len(), "read_ms": distribution(reads.iter().map(|&(_, ns)| ns as f64 / 1e6).collect()), "read_hours": distribution(reads.iter().map(|&(hours, _)| f64::from(hours)).collect()),
                    "held_by_lanes": held(by_lanes), "held_today": held(today),
                })
            }),
        })
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        if let Some((reader, _, _, _)) = self.steps.take() {
            self.entry.playhead().retire(reader);
        }
    }
}

fn scan_naval(document: &Document) -> io::Result<Value> {
    #[derive(Default)]
    struct Seen {
        hours: u64,
        first: Option<Hour>,
        last: Option<Hour>,
        most: usize,
        regions: BTreeSet<u32>,
        statuses: BTreeMap<String, u64>,
    }
    let replay = document.playback.replay();
    let (mut mines, mut accidents) = (Seen::default(), Seen::default());
    let began = Instant::now();
    let note = |seen: &mut Seen, hour: Hour, view: &replay_frame::frame::TableView, regions: Vec<u32>| {
        let status = match view {
            replay_frame::frame::TableView::Complete(_) => "complete".to_owned(),
            replay_frame::frame::TableView::Truncated { .. } => "truncated".to_owned(),
            replay_frame::frame::TableView::Unavailable { reason } => format!("unavailable: {reason}"),
            replay_frame::frame::TableView::NotRecorded { mode } => format!("not recorded ({mode})"),
        };
        *seen.statuses.entry(status).or_default() += 1;
        if !regions.is_empty() {
            seen.hours += 1;
            seen.first.get_or_insert(hour);
            seen.last = Some(hour);
            seen.most = seen.most.max(regions.len());
            seen.regions.extend(regions);
        }
    };
    for index in &replay.index {
        replay.read_block_alone(
            index,
            &[TableId::NavalRegionalMines, TableId::NavalAccidentReports],
            |_, decoded| {
                let regions = decoded.complete::<tables::NavalRegionalMines>().map(|rows| rows.iter().map(|row| row.region).collect()).unwrap_or_default();
                note(&mut mines, decoded.hour, &decoded.tables[TableId::NavalRegionalMines.index()], regions);
                let regions = decoded.complete::<tables::NavalAccidentReports>().map(|rows| rows.iter().map(|row| row.region).collect()).unwrap_or_default();
                note(&mut accidents, decoded.hour, &decoded.tables[TableId::NavalAccidentReports.index()], regions);
                Ok(())
            },
            || Ok(()),
        )?;
    }
    let seen = |seen: Seen| json!({"hours_with_rows": seen.hours, "first": seen.first, "last": seen.last, "most_rows": seen.most, "regions": seen.regions.len(), "statuses": seen.statuses});
    Ok(json!({"blocks": replay.index.len(), "seconds": began.elapsed().as_secs_f64(), "NavalRegionalMines": seen(mines), "NavalAccidentReports": seen(accidents)}))
}

fn seeds(file: &str) -> io::Result<BTreeMap<String, Vec<TableId>>> {
    let mut seeded: BTreeMap<String, Vec<TableId>> = BTreeMap::new();
    for line in std::fs::read_to_string(file)?.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (family, name) = line.split_once(' ').ok_or_else(|| io::Error::other(format!("{file}: {line} is not <family> <Table>")))?;
        let table = TableId::ALL.into_iter().find(|table| format!("{table:?}") == name.trim()).ok_or_else(|| io::Error::other(format!("{file} names no table {name}")))?;
        seeded.entry(family.to_owned()).or_default().push(table);
    }
    Ok(seeded)
}

fn at_hour(reader: &ViewReader, hour: Hour) -> io::Result<Option<Arc<DecodedFrame>>> {
    match reader.wait(|frame| frame.hour >= hour, LIMIT) {
        Ok(frame) => Ok((frame.hour == hour).then_some(frame)),
        Err(error) => Err(io::Error::other(format!("{error} ({hour}); its newest rows are at {:?}", reader.frame().map(|frame| frame.hour)))),
    }
}

fn inputs(document: &Document, targets: &Targets) -> Option<Inputs> {
    Some(Inputs { content: Arc::clone(&document.counter_content), raids: Arc::clone(&document.icons.raids), vision: Arc::clone(targets.vision.as_ref()?), style: Style { nato: document.game_options.nato_counters, diagnostics: false }, wake: Arc::new(|| {}) })
}

type MakeJob = Box<dyn Fn(&Caches) -> Job>;

fn all(family: Family, document: &Document, targets: &Targets, inputs: &Inputs) -> Vec<(String, Key, MakeJob)> {
    let mut out: Vec<(String, Key, MakeJob)> = Vec::new();
    let replay = document.playback.roster_sampler();
    let selected: Arc<Vec<viewer_runtime::unit_handle::Unit>> = Arc::default();
    match family {
        Family::Counters => {
            for hover in &targets.land {
                let (inputs, hover) = (inputs.clone(), hover.clone());
                out.push((format!("land {:?}", hover.members), inputs.key(What::Land(hover.clone()), [0, 0]), Box::new(move |caches| inputs.land(caches, hover.clone()))));
            }
            for hover in &targets.task_forces {
                let (inputs, hover) = (inputs.clone(), hover.clone());
                out.push((format!("task force {:?}", hover.members), inputs.key(What::TaskForce(hover.clone()), [0, 0]), Box::new(move |caches| inputs.task_force(caches, hover.clone()))));
            }
            for hover in &targets.ports {
                let (inputs, hover) = (inputs.clone(), hover.clone());
                out.push((format!("port {:?}", hover.members), inputs.key(What::Port(hover.clone()), [0, 0]), Box::new(move |caches| inputs.port(caches, hover.clone()))));
            }
            for &(province, kind) in &targets.air_bases {
                let inputs = inputs.clone();
                out.push((format!("air base {province}/{kind}"), inputs.key(What::AirBase { province, kind }, [0, 0]), Box::new(move |_| inputs.air_base(province, kind, Arc::default()))));
            }
            for &province in &targets.naval_bases {
                let inputs = inputs.clone();
                out.push((format!("naval base {province}"), inputs.key(What::NavalBase { province, fleet_selected: false }, [0, 0]), Box::new(move |_| inputs.naval_base(province, false))));
            }
        }
        Family::Province => {
            for mode in &targets.modes {
                for &province in &targets.provinces {
                    let (inputs, mode, selected, replay) = (inputs.clone(), mode.clone(), Arc::clone(&selected), Arc::clone(&replay));
                    let label = format!("province {province} {}", format!("{mode:?}").split('(').next().unwrap_or_default());
                    let key = inputs.key(What::Province { province, mode: mode.clone(), routes: Vec::new() }, [id(&selected), id(&replay)]);
                    out.push((label, key, Box::new(move |caches| inputs.province(caches, province, mode.clone(), Vec::new(), Arc::clone(&selected), Arc::clone(&replay)))));
                }
            }
        }
        Family::Naval => {
            for &(region, mines) in &targets.naval {
                let inputs = inputs.clone();
                let key = inputs.key(if mines { What::Mines(region) } else { What::Accidents(region) }, [0, 0]);
                out.push((format!("naval {region} {}", if mines { "mines" } else { "accidents" }), key, Box::new(move |_| inputs.naval(region, mines))));
            }
        }
        Family::Occupation => {
            for (state, token) in &targets.occupation {
                let Ok(Some(definition)) = document.icons.occupation.as_ref().map_err(Clone::clone).and_then(|icons| icons.definitions.find(token)) else { continue };
                let (inputs, definition, state) = (inputs.clone(), definition.clone(), *state);
                out.push((format!("occupation {state} {token}"), inputs.key(What::Occupation { state, token: token.clone() }, [0, 0]), Box::new(move |_| inputs.occupation(state, definition.clone()))));
            }
        }
        Family::Capture => {
            let (inputs, land, task_forces, ports) = (inputs.clone(), targets.land.clone(), targets.task_forces.clone(), targets.ports.clone());
            out.push(("capture".into(), inputs.key(What::Capture, [0, 0]), Box::new(move |_| inputs.capture(land.clone(), task_forces.clone(), ports.clone()))));
        }
    }
    out
}

fn made_whole(caches: &Caches, key: Key, frame: &Arc<DecodedFrame>, at_head: bool, job: Job) -> (Option<Arc<Tooltip>>, Vec<TableId>) {
    let retry: Retry = Arc::new(|| {});
    let mut posted = Posted { key, frame: id(frame), rows: Arc::clone(frame), at_head, job };
    loop {
        let made = make(&mut posted, &retry);
        let provinces = caches.provinces.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !provinces.pending() {
            return (made.text, made.missed);
        }
        provinces.wait();
    }
}

fn text(tooltip: &Tooltip) -> String {
    format!("{}\n--\n{}\n--\n{:?}", tooltip.title, tooltip.body, tooltip.refused)
}

fn first_difference(wanted: &str, got: &str) -> String {
    let at = wanted.bytes().zip(got.bytes()).position(|(a, b)| a != b).unwrap_or(wanted.len().min(got.len()));
    let around = |text: &str| {
        let start = text.char_indices().map(|(at, _)| at).take_while(|&from| from + 160 <= at).last().unwrap_or(0);
        text[start..].chars().take(400).collect::<String>()
    };
    format!("at byte {at} of {} / {}: today {} / at the head {}", wanted.len(), got.len(), around(wanted), around(got))
}
