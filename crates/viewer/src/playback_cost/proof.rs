
use crate::{playhead_host, document::{Document, InitialHour}, playhead_host::{Frame, MapSide, mirror::Mirrors, units}};
use viewer_layer_core::layers::{Carried, Inputs, Layer, Settling, View};
use viewer_layers_counters::{counters::fed::Fed, naval_counters::NavalCounters};
use viewer_playback::proofs::{counters as counters_line, resource_icons as resource_icons_line, occupation_marks as occupation_marks_line, naval_counters as naval_counters_line, gun_counters as gun_counters_line, air_mission_counters as air_mission_counters_line};
use viewer_layers_political::political::{self, BUCKETS, LEFT_OUT, Political};
use viewer_layers_forces::units::{Figure, Ground, Units, Withheld};
use map_display_data::{
    landscape::Regions,
    province_field::{ProvinceField, Storage},
    value_index::ValueIndex,
};
use playhead::{Step, Timeline};
use replay_data::{Hour, rows::TableId};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    io::{self, BufWriter, Write as _},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use viewer_foundation::{perspective, tables};
use viewer_borders::{border_pass::{ClassPlan, land_class}, demilitarized::{Zone, ZonePlan}};
use viewer_map_modes::map_mode::MapMode;

mod intel_icons;
mod p6;
mod naval;
mod highlight;

const USAGE: &str = "viewer --playhead-proof <recording.battleplan> --out <dir> [--layers political] [--select <file>] [--every 6] [--seeks 20] [--seek-to H,...] [--from-hour H] [--hours N] [--timing] [--speed 1440] [--fog on|off] [--rows units,combat,world,triggers,all,details] [--camera fit|x,z,height] [--viewport 2048x1152] [--hovers all|none] [--map-mode operatives] [--scripts on|off[@tables.txt]] [--paced]";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

struct Line {
    name: &'static str,
    make: fn(&Document) -> Box<dyn Layer>,
    today: fn(&dyn Layer, &Document) -> io::Result<Compared>,
    fresh: fn(&dyn Layer, &Mirrors, &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared>,
    marked: fn(&dyn Layer) -> Vec<(&'static str, u64)>,
}

const LINES: &[Line] = &[
    Line { name: "political", make: political_make, today: political_today, fresh: political_fresh, marked: political_marked },
    Line { name: "vision", make: p6::vision_make, today: p6::vision_today, fresh: p6::vision_fresh, marked: p6::vision_marked },
    Line { name: "intel", make: p6::intel_make, today: p6::intel_today, fresh: p6::intel_fresh, marked: p6::intel_marked },
    Line { name: "radar", make: p6::radar_make, today: p6::radar_today, fresh: p6::radar_fresh, marked: p6::radar_marked },
    Line { name: "weather", make: p6::weather_make, today: p6::weather_today, fresh: p6::weather_fresh, marked: p6::weather_marked },
    Line { name: "snow", make: p6::snow_make, today: p6::snow_today, fresh: p6::snow_fresh, marked: p6::snow_marked },
    Line { name: "sounds", make: sounds_line::make, today: sounds_line::today, fresh: sounds_line::fresh, marked: sounds_line::marked },
    Line { name: "strikes", make: strikes_line::make, today: strikes_line::today, fresh: strikes_line::fresh, marked: strikes_line::marked },
    Line { name: "map_text", make: viewer_playback::proofs::map_text::make, today: viewer_playback::proofs::map_text::today, fresh: viewer_playback::proofs::map_text::fresh, marked: viewer_playback::proofs::map_text::marked },
    Line { name: "engagements", make: engagements_line::make, today: engagements_line::today, fresh: engagements_line::fresh, marked: engagements_line::marked },
    Line { name: "units", make: units_make, today: units_today, fresh: units_fresh, marked: units_marked },
    Line { name: "convoys", make: convoys_make, today: convoys_today, fresh: convoys_fresh, marked: convoys_marked },
    Line { name: "orders", make: orders_make, today: orders_today, fresh: orders_fresh, marked: orders_marked },
    Line { name: "rails", make: p7::rails_make, today: p7::rails_today, fresh: p7::rails_fresh, marked: p7::rails_marked },
    Line { name: "names", make: p7::names_make, today: p7::names_today, fresh: p7::names_fresh, marked: p7::names_marked },
    Line { name: "labels", make: p7::labels_make, today: p7::labels_today, fresh: p7::labels_fresh, marked: p7::labels_marked },
    Line { name: "buildings", make: p7b::make, today: p7b::today, fresh: p7b::fresh, marked: p7b::marked },
    Line { name: "ships", make: p8c::ships_make, today: p8c::ships_today, fresh: p8c::ships_fresh, marked: p8c::ships_marked },
    Line { name: "aircraft", make: p8c::aircraft_make, today: p8c::aircraft_today, fresh: p8c::aircraft_fresh, marked: p8c::aircraft_marked },
    Line { name: "mode_fills", make: mode_fills::make, today: mode_fills::today, fresh: mode_fills::fresh, marked: mode_fills::marked },
    Line { name: "relations", make: viewer_playback::proofs::relations::make, today: viewer_playback::proofs::relations::today, fresh: viewer_playback::proofs::relations::fresh, marked: viewer_playback::proofs::relations::marked },
    Line { name: "counters", make: counters_line::make, today: counters_line::today, fresh: viewer_layers_counters::counters::fresh, marked: counters_line::marked },
    Line { name: "trains", make: p7::trains_make, today: p7::trains_today, fresh: p7::trains_fresh, marked: p7::trains_marked },
    Line { name: "resource_icons", make: resource_icons_line::make, today: resource_icons_line::today, fresh: resource_icons_line::fresh, marked: resource_icons_line::marked },
    Line { name: "occupation_marks", make: occupation_marks_line::make, today: occupation_marks_line::today, fresh: occupation_marks_line::fresh, marked: occupation_marks_line::marked },
    Line { name: "intel_icons", make: intel_icons::make, today: intel_icons::today, fresh: intel_icons::fresh, marked: intel_icons::marked },
    Line { name: "naval_counters", make: naval_counters_line::make, today: naval_counters_line::today, fresh: naval_counters_line::fresh, marked: naval_counters_line::marked },
    Line { name: "land_markers", make: land_markers_line::make, today: land_markers_line::today, fresh: viewer_playback::proofs::land_markers::fresh, marked: viewer_playback::proofs::land_markers::marked },
    Line { name: "base_counters", make: viewer_playback::proofs::base_counters::make, today: viewer_playback::proofs::base_counters::today, fresh: viewer_playback::proofs::base_counters::fresh, marked: viewer_playback::proofs::base_counters::marked },
    Line { name: "naval_markers", make: land_markers_line::make_naval, today: land_markers_line::today_naval, fresh: viewer_playback::proofs::naval_markers::fresh, marked: viewer_playback::proofs::naval_markers::marked },
    Line { name: "naval", make: naval::make, today: naval::today, fresh: naval::fresh, marked: naval::marked },
    Line { name: "highlight", make: highlight::make, today: highlight::today, fresh: highlight::fresh, marked: highlight::marked },
    Line { name: "gun_counters", make: gun_counters_line::make, today: gun_counters_line::today, fresh: gun_counters_line::fresh, marked: gun_counters_line::marked },
    Line { name: "air_mission_counters", make: air_mission_counters_line::make, today: air_mission_counters_line::today, fresh: air_mission_counters_line::fresh, marked: air_mission_counters_line::marked },
];

#[path = "engaged_proof.rs"]
mod engaged;

mod p7;
mod p7b;

mod p8c;

mod mode_fills;

pub(crate) use viewer_layer_core::comparison::{Compared, behind_at};

struct Options {
    recording: PathBuf,
    out: PathBuf,
    layers: Vec<&'static Line>,
    select: Option<PathBuf>,
    every: u64,
    seeks: usize,
    seek_to: Vec<Hour>,
    from: Option<Hour>,
    hours: Option<u32>,
    timing: bool,
    speed: f64,
    engaged: Option<String>,
    rows: Option<String>,
    details: bool,
    combat: Option<String>,
    hovers: Option<String>,
    scripts: Option<(bool, Option<String>)>,
    paced: bool,
    view: View,
    camera: Option<String>,
}

fn parse(args: &[OsString]) -> io::Result<Options> {
    let mut args = args.iter().map(|arg| arg.to_string_lossy().into_owned()).skip_while(|arg| arg != "--playhead-proof").skip(1);
    let recording = PathBuf::from(args.next().ok_or_else(|| invalid(USAGE))?);
    let mut options = Options {
        recording,
        out: PathBuf::new(),
        layers: LINES.iter().collect(),
        select: None,
        every: 6,
        seeks: 20,
        seek_to: Vec::new(),
        from: None,
        hours: None,
        timing: false,
        speed: 1440.0,
        engaged: Some("recorded".to_owned()),
        rows: None,
        details: false,
        combat: None,
        hovers: None,
        scripts: None,
        paced: false,
        view: View::default(),
        camera: None,
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| invalid(format!("{arg} needs a value; {USAGE}")));
        let number = |text: String| text.parse::<f64>().map_err(|_| invalid(format!("{arg}: {text} is not a number")));
        match arg.as_str() {
            "--out" => options.out = PathBuf::from(value()?),
            "--select" => options.select = Some(PathBuf::from(value()?)),
            "--every" => options.every = number(value()?)?.max(1.0) as u64,
            "--seeks" => options.seeks = number(value()?)? as usize,
            "--seek-to" => options.seek_to = value()?.split(',').map(|hour| number(hour.to_owned()).map(|hour| hour as Hour)).collect::<io::Result<_>>()?,
            "--from-hour" => options.from = Some(number(value()?)? as Hour),
            "--hours" => options.hours = Some(number(value()?)? as u32),
            "--speed" => options.speed = number(value()?)?,
            "--timing" => options.timing = true,
            "--engaged-fold" => options.engaged = Some(value()?),
            "--rows" => {
                let names = value()?;
                options.details |= names.split(',').any(|name| name == "details");
                let sets: Vec<&str> = names.split(',').filter(|name| *name != "details").collect();
                options.rows = (!sets.is_empty()).then(|| sets.join(","));
            }
            "--combat" => options.combat = Some(value()?),
            "--scripts" => {
                let text = value()?;
                let (on, primed) = text.split_once('@').map_or((text.as_str(), None), |(on, file)| (on, Some(file.to_owned())));
                options.scripts = Some((on == "on", primed));
            }
            "--paced" => options.paced = true,
            "--camera" => options.camera = Some(value()?),
            "--map-mode" => options.view.map_mode = MapMode::parse(&value()?).ok_or_else(|| invalid(format!("--map-mode takes a map mode's token; {USAGE}")))?,
            "--viewport" => {
                let text = value()?;
                let (width, height) = text.split_once('x').ok_or_else(|| invalid(format!("--viewport takes WxH, not {text}")))?;
                options.view.viewport = [number(width.to_owned())? as u32, number(height.to_owned())? as u32];
            }
            "--camera-height" => options.view.camera_height = number(value()?)? as f32,
            "--hovers" => options.hovers = Some(value()?),
            "--fog" => {
                let fog = perspective::Choice::parse_fog(&value()?)?;
                (options.view.perspective.fog, options.view.fog) = (fog, fog);
            }
            "--layers" => {
                let names = value()?;
                options.layers = names
                    .split(',')
                    .map(|name| LINES.iter().find(|line| line.name == name).ok_or_else(|| invalid(format!("No layer {name} in the proof tool"))))
                    .collect::<io::Result<_>>()?;
            }
            _ => return Err(invalid(format!("Unknown option {arg}; {USAGE}"))),
        }
    }
    if options.out.as_os_str().is_empty() {
        return Err(invalid(USAGE));
    }
    Ok(options)
}

pub(crate) fn run(args: &[OsString]) -> io::Result<()> {
    let mut options = parse(args)?;
    let captures = std::env::var_os("BATTLEPLAN_CAPTURES").or_else(|| std::env::var_os("HOI4_CAPTURES")).ok_or_else(|| invalid("Set BATTLEPLAN_CAPTURES to the folder --out must be under"))?;
    let parent = fs::canonicalize(options.out.parent().ok_or_else(|| invalid("--out needs a parent"))?)?;
    if !parent.starts_with(fs::canonicalize(captures)?) {
        return Err(invalid("--out must be a new directory under BATTLEPLAN_CAPTURES"));
    }
    fs::create_dir(&options.out)?;
    let out = fs::canonicalize(&options.out)?;
    let config = dashboard_core::config::Config::load()?;
    let mut resolve = config.resolve_options()?;
    resolve.cache_dir = out.join("cache/content");
    fs::create_dir_all(&resolve.cache_dir)?;
    let (local, roaming) = (out.join("cache/local"), out.join("cache/roaming"));
    fs::create_dir_all(&local)?;
    fs::create_dir_all(&roaming)?;
    unsafe {
        std::env::set_var("LOCALAPPDATA", local);
        std::env::set_var("APPDATA", roaming);
    }
    replay_frame::cpu::enable();
    let hz = tsc_hz();
    let (units, branch) = units::at_path(&options.recording, None)?;
    let playhead = units.name.clone();
    let timeline = units.timeline.clone();
    let runs: Vec<(usize, Hour, Hour)> = units.pages.runs().iter().enumerate().map(|(run, held)| (run, held.first_hour, held.last_hour())).collect();
    eprintln!("The recording's runs (run, first, last hour): {runs:?}; branch {branch} plays pieces {:?}", timeline.pieces());
    let first = options.from.unwrap_or(timeline.first());
    let within = |hour: Hour| timeline.first() <= hour && hour <= timeline.last();
    if let Some(&outside) = std::iter::once(&first).chain(&options.seek_to).find(|&&hour| !within(hour)) {
        return Err(invalid(format!("Hour {outside} is outside branch {branch}'s timeline, {} to {}", timeline.first(), timeline.last())));
    }
    let loaded = Instant::now();
    let initial = InitialHour { branch: Some(branch), hour: Some(first), hour_fraction: 0.0 };
    let mut document = Document::load(&options.recording, &resolve, false, &initial, None, 0)?;
    engine_executable::executable_tables::enter(document.content.tables);
    let load_seconds = loaded.elapsed().as_secs_f64();
    if let Some(spec) = options.camera.clone() {
        if options.view.viewport == [0, 0] {
            options.view.viewport = [2048, 1152];
        }
        let (size, defines) = (viewer_gpu::camera::MapSize::of(&document.content.map), document.content.map_defines);
        let camera = match spec.as_str() {
            "fit" => viewer_gpu::camera::Camera::fit(size, defines, options.view.viewport),
            placed => crate::parse_placement(placed)?.on(size, defines),
        };
        let placement = viewer_gpu::camera::Placement { center: camera.center, height: camera.height, pose: camera.pose };
        (options.view.camera, options.view.camera_height) = (Some(placement), camera.height);
    }
    let options = options;
    let last = options.hours.map_or(timeline.last(), |hours| (first + hours as Hour - 1).min(timeline.last()));
    let engaged = match &options.engaged {
        Some(viewers) => {
            let (results, folded) = engaged::run(&document, &units.pages, &timeline, viewers, last)?;
            serde_json::to_writer_pretty(File::create_new(out.join("engaged.json"))?, &results)?;
            if let Some(folded) = folded {
                document.playback.seek(document.playback.branch(), timeline.first())?;
                let viewer = document.hour()?.vision(&perspective::Choice::default())?.ours();
                *engagements_line::FOLDED.lock().unwrap_or_else(|p| p.into_inner()) = Some((folded, viewer));
            }
            Some(results)
        }
        None => None,
    };
    let mut layers: Vec<Box<dyn Layer>> = options.layers.iter().map(|line| (line.make)(&document)).collect();
    let select = match &options.select {
        Some(path) => Some(Declare::read(path)?),
        None => None,
    };
    if let Some(select) = select {
        layers.push(Box::new(select));
    }
    let entry = playhead_host::open(units, playhead_host::settings())?;
    entry.set_capture(document.capture_mode());
    document.playback.read_rosters_of(&entry);
    let mut side = MapSide::with_products(Arc::clone(&entry), layers, options.view.clone())?;
    if let Some((name, why)) = side.refusals().next() {
        return Err(invalid(format!("The {name} layer is refused: {why}")));
    }
    let mut proof = Proof {
        options: &options,
        frames: BufWriter::new(File::create_new(out.join("frames.jsonl"))?),
        compares: BufWriter::new(File::create_new(out.join("compare.jsonl"))?),
        records: Vec::new(),
        held: Vec::new(),
        totals: BTreeMap::new(),
        not_registered: BTreeMap::new(),
        sampled: BTreeMap::new(),
        mirror_tables: BTreeMap::new(),
        skipped: 0,
        steps: 0,
        resets: Vec::new(),
        started: Instant::now(),
        marked_before: Vec::new(),
        told: Instant::now(),
        hz,
        rows: match &options.rows {
            Some(sets) => Some(playhead_host::views::proof::Line::open(side.views().handle(), document.playback.branch(), sets, options.timing, &document.source_path, &resolve)?),
            None => None,
        },
        hovers: match &options.hovers {
            Some(which) => Some(crate::app::hover_rows::proof::Line::open(&entry, side.views().handle(), &document, which, options.timing, hz)?),
            None => None,
        },
        scripts: match &options.scripts {
            Some((on, primed)) => Some(viewer_playback::proofs::scripts::Line::open(&entry, playhead_host::views::Recording { branch: document.playback.branch() }, *on, primed.as_deref())?),
            None => None,
        },
        combat: match options.combat.as_deref().map(|value| value.split_once('@').unwrap_or((value, ""))) {
            None | Some(("none", _)) => None,
            Some((readers, learned)) => {
                use playhead_host::views::combat::TIMED;
                let timed: Vec<&'static str> = match readers {
                    "all" | "-" => TIMED.to_vec(),
                    names => names.split(',').map(|name| TIMED.iter().copied().find(|timed| *timed == name).ok_or_else(|| invalid(format!("No timed combat reader {name}; they are {TIMED:?}")))).collect::<io::Result<_>>()?,
                };
                Some(playhead_host::views::combat::Line::open(side.views().handle(), &timed, (!learned.is_empty()).then_some(learned), options.timing, hz)?)
            }
        },
        stepper: match &options.combat {
            Some(_) => Some(playhead_host::views::combat::StepTimer::open(&entry)?),
            None => None,
        },
    };
    let branch = document.playback.branch();
    for layer in side.parts().map_or(&[][..], |(_, layers)| layers) {
        if let Some(text) = layer.any().downcast_ref::<viewer_layers_political::map_text::MapText>() {
            text.attach(Arc::clone(&entry));
        }
    }
    give_vision(&side, &document, &options.view.perspective)?;
    side.anchor(first, true, options.speed, true);
    let flat = proof.play_flat(&mut side, &mut document, branch, &timeline, first, last)?;
    let seeks = proof.seek_all(&mut side, &mut document, branch, &timeline, last)?;
    proof.frames.flush()?;
    proof.compares.flush()?;
    let (report, peak, counters) = entry.playhead().held();
    let summary = json!({
        "recording": options.recording, "playhead": playhead, "first": first, "last": last,
        "layers": options.layers.iter().map(|line| line.name).collect::<Vec<_>>(), "select": options.select,
        "timing": options.timing, "fog": options.view.fog, "load_seconds": load_seconds,
        "flat": flat, "seeks": seeks, "steps": proof.steps, "skipped_samples": proof.skipped,
        "frames": proof.distributions(), "compared": proof.totals.iter().map(|(name, (entries, differing, samples))| {
            let [compared, loading] = proof.sampled.get(name).copied().unwrap_or_default();
            let mut line = json!({"layer": name, "samples": samples, "samples_compared": compared, "samples_loading": loading, "entries": entries, "differing": differing});
            if name == "limitations:today" {
                line["not_registered"] = json!(proof.not_registered);
            }
            line
        }).collect::<Vec<_>>(),
        "resets": proof.resets, "held_series": proof.held,
        "playhead_held_now": report.total, "playhead_held_peak": peak.total, "states_peak": [peak.states, peak.state_bytes],
        "stalls": counters.stalls, "seeks_counted": counters.seeks,
        "contents": political_content_bytes(&side), "engaged_fold": engaged,
        "mirror_refused": side.parts().map(|(mirrors, _)| mirrors.iter().map(|mirror| mirror.refused()).sum::<u64>()),
        "rows": proof.rows.as_ref().map(|rows| rows.summary(super::distribution)),
        "combat": proof.combat.as_ref().map(|combat| combat.summary(super::distribution)),
        "stepper": proof.stepper.as_ref().map(|stepper| stepper.summary(super::distribution)),
        "hovers": proof.hovers.as_ref().map(|hovers| hovers.summary(super::distribution)),
        "intel_icons": intel_icons::summary(&side),
        "scripts": proof.scripts.as_ref().map(|scripts| scripts.summary(&side, super::distribution)),
    });
    let mut file = File::create_new(out.join("summary.json"))?;
    serde_json::to_writer_pretty(&mut file, &summary)?;
    file.write_all(b"\n")?;
    println!("{}", serde_json::to_string(&json!({"compared": summary["compared"], "frames": summary["frames"], "resets_ms": summary["resets"], "out": out}))?);
    let differing: u64 = proof.totals.values().map(|(_, differing, _)| differing).sum::<u64>() + proof.rows.as_ref().map_or(0, |rows| rows.differing()) + proof.combat.as_ref().map_or(0, |combat| combat.differing()) + proof.hovers.as_ref().map_or(0, |hovers| hovers.differing()) + proof.scripts.as_ref().map_or(0, |scripts| scripts.differing());
    if differing > 0 {
        return Err(io::Error::other(format!("{differing} entries differ; see {}", out.join("compare.jsonl").display())));
    }
    Ok(())
}

struct Proof<'a> {
    options: &'a Options,
    frames: BufWriter<File>,
    compares: BufWriter<File>,
    records: Vec<Record>,
    held: Vec<Value>,
    totals: BTreeMap<String, (u64, u64, u64)>,
    sampled: BTreeMap<String, [u64; 2]>,
    not_registered: BTreeMap<String, u64>,
    mirror_tables: BTreeMap<usize, (u64, u64)>,
    skipped: u64,
    steps: u64,
    resets: Vec<Value>,
    started: Instant,
    marked_before: Vec<Vec<(&'static str, u64)>>,
    told: Instant,
    hz: f64,
    rows: Option<playhead_host::views::proof::Line>,
    combat: Option<playhead_host::views::combat::Line>,
    stepper: Option<playhead_host::views::combat::StepTimer>,
    hovers: Option<crate::app::hover_rows::proof::Line>,
    scripts: Option<viewer_playback::proofs::scripts::Line>,
}

struct Record {
    frame_ns: u64,
    mirror_ns: u64,
    mirror_cycles: u64,
    cycles: Vec<(u64, u64)>,
    layers: Vec<(u64, u64, usize)>,
    marked: Vec<Vec<(&'static str, u64)>>,
}

impl Proof<'_> {
    fn tell(&mut self, side: &MapSide, what: &str) -> io::Result<()> {
        if self.told.elapsed() < Duration::from_secs(2) {
            return Ok(());
        }
        self.told = Instant::now();
        self.frames.flush()?;
        self.compares.flush()?;
        let playhead = side.entry().playhead();
        let (report, _, counters) = playhead.held();
        eprintln!(
            "{}",
            json!({"at_s": self.started.elapsed().as_secs_f64(), "doing": what, "newest": playhead.newest(), "holding": playhead.holding(), "composing": side.composing(), "pending": side.held().pending, "steps": self.steps, "records": self.records.len(), "lanes": report.lanes, "preparing": report.preparing, "states": report.states, "jobs": report.jobs_running, "queued": report.fetches_queued, "seeks": counters.seeks, "stalls": counters.stalls, "error": playhead.error()})
        );
        Ok(())
    }

    fn play_flat(&mut self, side: &mut MapSide, document: &mut Document, branch: replay_data::BranchId, timeline: &Timeline, first: Hour, last: Hour) -> io::Result<Value> {
        let started = Instant::now();
        let quantum = (self.options.speed / playhead_host::settings().rate).max(1.0) as Hour;
        let mut applied = None;
        let mut progress = Instant::now();
        let mut since = 0;
        let mut near_end = false;
        let mut frames = 0_u32;
        while applied.is_none_or(|hour| hour < last) {
            if self.options.paced {
                let due = started + Duration::from_secs(1) / 60 * frames;
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                frames += 1;
            }
            if let Some(error) = side.entry().playhead().error() {
                return Err(io::Error::other(format!("The playhead stopped: {error}")));
            }
            let newest = side.entry().playhead().newest().unwrap_or(first);
            let head = applied.map_or(newest, |hour| newest.min(hour + quantum));
            self.tell(side, "flat")?;
            if let Some(rows) = &mut self.rows {
                rows.drive(head);
            }
            if let Some(scripts) = &mut self.scripts {
                scripts.frame(side, head, true);
            }
            let began = Instant::now();
            let frame = side.frame(head)?;
            let frame_ns = began.elapsed().as_nanos() as u64;
            if self.options.timing
                && let (Some(combat), Some(shown)) = (&mut self.combat, frame.applied)
            {
                combat.frame(document, shown);
            }
            if let Some(stepper) = &mut self.stepper {
                stepper.frame(head);
            }
            if let Some(hovers) = &mut self.hovers {
                hovers.frame(head);
            }
            if frame.steps == 0 && frame.swapped.is_none() {
                stalled(&mut progress, "flat play")?;
                std::thread::sleep(Duration::from_micros(100));
                continue;
            }
            progress = Instant::now();
            self.record(side, &frame, frame_ns)?;
            let Some(hour) = frame.applied else { continue };
            let new = applied != Some(hour);
            applied = Some(hour);
            if !new || frame.composing {
                continue;
            }
            since += 1;
            let block_end = is_block_end(timeline, side, hour);
            if !self.options.timing && (since >= self.options.every || block_end || near_end) {
                if self.compare(side, document, branch, hour, if block_end { "block end" } else if near_end { "after a block end" } else { "flat" })? {
                    since = 0;
                }
            }
            near_end = block_end;
        }
        Ok(json!({"seconds": started.elapsed().as_secs_f64(), "last": applied}))
    }

    fn seek_all(&mut self, side: &mut MapSide, document: &mut Document, branch: replay_data::BranchId, timeline: &Timeline, last: Hour) -> io::Result<Value> {
        let first = timeline.first();
        let mut targets = Vec::new();
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let ends: Vec<Hour> = (1..).map(|block| first + block * replay_columnar::unit::BLOCK as Hour - 1).take_while(|&end| end < last).collect();
        for (at, &end) in ends.iter().enumerate() {
            if targets.len() + 3 > self.options.seeks / 2 + 3 {
                break;
            }
            match at % 3 {
                0 => targets.push(end - 1),
                1 => targets.push(end),
                _ => targets.push(end + 1),
            }
        }
        while targets.len() < self.options.seeks {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            targets.push(first + ((seed >> 33) % (last - first + 1) as u64) as Hour);
        }
        targets.truncate(self.options.seeks);
        if !self.options.seek_to.is_empty() {
            targets.clone_from(&self.options.seek_to);
        }
        let mut done = Vec::new();
        for target in targets {
            side.anchor(target, false, self.options.speed, true);
            let called = Instant::now();
            let mut taken_at = None;
            let mut sought = None;
            let mut composed = None;
            let mut progress = Instant::now();
            loop {
                if let Some(error) = side.entry().playhead().error() {
                    return Err(io::Error::other(format!("The playhead stopped: {error}")));
                }
                self.tell(side, "seek")?;
                if let Some(rows) = &mut self.rows {
                    rows.drive(target);
                }
                if let Some(stepper) = &mut self.stepper {
                    stepper.frame(target);
                }
                if let Some(hovers) = &mut self.hovers {
                    hovers.frame(target);
                }
                if let Some(scripts) = &mut self.scripts {
                    scripts.frame(side, target, false);
                }
                let began = Instant::now();
                let frame = side.frame(target)?;
                let frame_ns = began.elapsed().as_nanos() as u64;
                if frame.steps > 0 || frame.swapped.is_some() {
                    self.record(side, &frame, frame_ns)?;
                }
                if frame.resets > 0 && taken_at.is_none() {
                    taken_at = Some(called.elapsed().as_secs_f64() * 1e3);
                    sought = frame.sought;
                }
                if let Some(swapped) = &frame.swapped {
                    composed = Some(swapped.clone());
                }
                if taken_at.is_some() && composed.is_some() && !side.composing() {
                    break;
                }
                if frame.steps == 0 && frame.swapped.is_none() {
                    stalled(&mut progress, "a seek")?;
                    std::thread::sleep(Duration::from_micros(100));
                } else {
                    progress = Instant::now();
                }
            }
            let shown_ms = called.elapsed().as_secs_f64() * 1e3;
            let record = json!({
                "target": target, "taken_ms": taken_at, "shown_ms": shown_ms,
                "compose_ms": composed.as_ref().map(|ns| ns.iter().map(|&ns| ns as f64 / 1e6).collect::<Vec<_>>()), "composed_on": "playhead host worker",
                "rebuild_ms": sought.map(|sought| sought.rebuild_ns as f64 / 1e6), "rebuild_most_ms": sought.map(|sought| sought.rebuild_most_ns as f64 / 1e6),
                "seek_ms": sought.map(|sought| sought.ns as f64 / 1e6), "rebuilt_on": "playhead fetchers", "states": sought.map(|sought| (sought.states, sought.state_bytes)),
            });
            self.resets.push(record.clone());
            if !self.options.timing {
                self.compare(side, document, branch, target, "after a seek")?;
            }
            done.push(record);
        }
        Ok(Value::Array(done))
    }

    fn record(&mut self, side: &MapSide, frame: &Frame, frame_ns: u64) -> io::Result<()> {
        self.steps += u64::from(frame.steps);
        let marked: Vec<Vec<(&'static str, u64)>> = match side.parts() {
            Some((_, layers)) => self.options.layers.iter().zip(layers).map(|(line, layer)| (line.marked)(&**layer)).collect(),
            None => self.marked_before.clone(),
        };
        let deltas: Vec<Vec<(&'static str, u64)>> = marked
            .iter()
            .enumerate()
            .map(|(at, now)| now.iter().map(|&(name, count)| (name, count - self.marked_before.get(at).and_then(|before| before.iter().find(|held| held.0 == name)).map_or(0, |held| held.1).min(count))).collect())
            .collect();
        self.marked_before = marked;
        let layers: Vec<(u64, u64, usize)> = frame.layers.iter().map(|part| (part.apply_ns, part.settle_ns, part.dirty)).collect();
        if frame.steps > 0 && frame.swapped.is_none() && frame.resets == 0 {
            for (table, &(ns, _)) in frame.mirror_tables.iter().enumerate().filter(|(_, (ns, _))| *ns > 0) {
                let held = self.mirror_tables.entry(table).or_default();
                held.0 += ns;
                held.1 = held.1.max(ns);
            }
            let cycles = frame.layers.iter().map(|part| (part.apply_cycles, part.settle_cycles)).collect();
            self.records.push(Record { frame_ns, mirror_ns: frame.mirror_ns, mirror_cycles: frame.mirror_cycles, cycles, layers: layers.clone(), marked: deltas.clone() });
        }
        if self.records.len() % 50 == 1 {
            let held = side.held();
            self.held.push(json!({"at_s": self.started.elapsed().as_secs_f64(), "hour": frame.applied, "mirrors": held.mirrors.iter().map(|(_, bytes)| bytes).sum::<usize>(), "layers": held.layers, "pending": held.pending, "states": held.states, "playhead": held.playhead, "total": held.total()}));
        }
        let line = json!({
            "head": frame.head, "applied": frame.applied, "steps": frame.steps, "cells": frame.cells, "rows": frame.rows, "frame_ns": frame_ns, "step_ns": frame.step_ns, "mirror_ns": frame.mirror_ns,
            "layers": layers, "marked": deltas.iter().map(|kinds| kinds.iter().map(|(name, count)| json!({name.to_owned(): count})).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "composing": frame.composing, "swapped": frame.swapped, "resets": frame.resets,
        });
        writeln!(self.frames, "{line}")
    }

    fn compare(&mut self, side: &mut MapSide, document: &mut Document, branch: replay_data::BranchId, hour: Hour, why: &str) -> io::Result<bool> {
        document.playback.seek(branch, hour)?;
        give_vision(side, document, &self.options.view.perspective)?;
        side.settle_all()?;
        let progress = Instant::now();
        while side.parts().is_some_and(|(_, layers)| layers.iter().any(|layer| layer.carried().entities > 0)) {
            if progress.elapsed() > Duration::from_secs(60) {
                let carrying: Vec<String> = side.layer_states().into_iter().zip(side.parts().map(|(_, layers)| layers).unwrap_or_default()).filter(|(_, layer)| layer.carried().entities > 0).map(|((name, draws, ..), layer)| format!("{name} carries {} ({})", layer.carried().entities, if draws { "drawn" } else { "not drawn" })).collect();
                return Err(io::Error::other(format!("The layers did not settle at hour {hour} within a minute: {}", carrying.join(", "))));
            }
            std::thread::sleep(Duration::from_micros(200));
            side.settle_all()?;
        }
        let progress = Instant::now();
        loop {
            let waiting = behind_at(side, hour);
            if waiting.is_empty() {
                break;
            }
            if progress.elapsed() > Duration::from_secs(60) {
                return Err(io::Error::other(format!("The layers' products did not reach hour {hour} within a minute: {}", waiting.join(", "))));
            }
            std::thread::sleep(Duration::from_micros(200));
            side.settle_all()?;
        }
        let Some((mirrors, layers)) = side.parts() else {
            self.skipped += 1;
            return Ok(false);
        };
        let behind: Vec<String> = mirrors.iter().filter(|mirror| mirror.head() != Some(hour)).map(|mirror| format!("{:?} at {:?}", mirror.table(), mirror.head())).collect();
        if !behind.is_empty() {
            self.skipped += 1;
            return Ok(false);
        }
        let source = source_check(mirrors, document)?;
        for (at, (line, layer)) in self.options.layers.iter().zip(layers).enumerate() {
            let upstream = viewer_layer_core::layers::Upstream::new(&layers[..at]);
            for (kind, compared) in [("today", (line.today)(&**layer, document)?), ("composed", (line.fresh)(&**layer, mirrors, &upstream)?)] {
                let name = format!("{}:{kind}", line.name);
                let total = self.totals.entry(name.clone()).or_default();
                total.0 += compared.entries;
                total.1 += compared.differing;
                total.2 += 1;
                let sampled = self.sampled.entry(name.clone()).or_default();
                sampled[0] += u64::from(compared.entries > 0);
                sampled[1] += u64::from(compared.notes.get("loading").is_some());
                writeln!(self.compares, "{}", json!({"hour": hour, "why": why, "compare": name, "entries": compared.entries, "differing": compared.differing, "first": compared.first, "notes": compared.notes, "source": source}))?;
            }
        }
        let every = match document.playback.replay().seek_selected(branch, hour, None)? {
            replay::campaign::SeekResult::Frame(frame) => frame,
            replay::campaign::SeekResult::Gap { reason, .. } => return Err(io::Error::other(format!("Today's reader finds a gap at {hour}: {reason}"))),
        };
        if self.options.details {
            let (entries, differing, first, absent) = playhead_host::details::differ(side, &every);
            let total = self.totals.entry("details:today".into()).or_default();
            (total.0, total.1, total.2) = (total.0 + entries, total.1 + differing, total.2 + 1);
            let absent: Vec<String> = absent.iter().map(|table| format!("{table:?}")).collect();
            writeln!(self.compares, "{}", json!({"hour": hour, "why": why, "compare": "details:today", "entries": entries, "differing": differing, "first": first, "not_in_the_units": absent}))?;
        }
        let today = document.limitations(&self.options.view.perspective)?.into_lines();
        let compared = viewer_playback::limitations::differ(side, &every, &today);
        let total = self.totals.entry("limitations:today".into()).or_default();
        (total.0, total.1, total.2) = (total.0 + compared.entries, total.1 + compared.differing, total.2 + 1);
        for (owner, &count) in &compared.not_registered {
            *self.not_registered.entry(owner.clone()).or_default() += count as u64;
        }
        let apart: Vec<String> = compared.apart.iter().map(|table| format!("{table:?}")).collect();
        writeln!(self.compares, "{}", json!({"hour": hour, "why": why, "compare": "limitations:today", "entries": compared.entries, "differing": compared.differing, "first": compared.first, "notes": {"missing_at_the_head": compared.missing, "not_today": compared.extra, "apart_not_in_the_units": apart, "not_given": compared.not_given, "not_registered": compared.not_registered, "raid_lines_apart_no_scripts_layer": compared.raids_apart, "refused_layers": compared.refused_layers, "today_lines": today.len()}}))?;
        let total = self.totals.entry("limitations:order".into()).or_default();
        (total.0, total.1, total.2) = (total.0 + compared.matched, total.1 + compared.reordered, total.2 + 1);
        writeln!(self.compares, "{}", json!({"hour": hour, "why": why, "compare": "limitations:order", "entries": compared.matched, "differing": compared.reordered}))?;
        let (entries, differing, first) = viewer_playback::limitations::differ_composed(side)?;
        let total = self.totals.entry("limitations:composed".into()).or_default();
        (total.0, total.1, total.2) = (total.0 + entries, total.1 + differing, total.2 + 1);
        writeln!(self.compares, "{}", json!({"hour": hour, "why": why, "compare": "limitations:composed", "entries": entries, "differing": differing, "first": first}))?;
        if let Some(rows) = &mut self.rows {
            rows.compare(document, hour, why, &mut self.compares)?;
        }
        if let Some(combat) = &mut self.combat {
            combat.compare(document, hour, why, &mut self.compares)?;
        }
        if let Some(hovers) = &mut self.hovers {
            hovers.compare(document, hour, why, &mut self.compares)?;
        }
        if let Some(scripts) = &mut self.scripts {
            scripts.compare(side, document, hour, why, &mut self.compares)?;
        }
        Ok(true)
    }

    fn distributions(&self) -> Value {
        let take = |values: Vec<f64>| super::distribution(values);
        let frames = &self.records;
        let mut layers = Vec::new();
        for (at, line) in self.options.layers.iter().enumerate() {
            let both: Vec<f64> = frames.iter().map(|record| (record.layers[at].0 + record.layers[at].1) as f64 / 1e6).collect();
            let mut marked: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
            for record in frames {
                for &(name, count) in record.marked.get(at).into_iter().flatten() {
                    marked.entry(name).or_default().push(count as f64);
                }
            }
            layers.push(json!({
                "layer": line.name,
                "apply_ms": take(frames.iter().map(|record| record.layers[at].0 as f64 / 1e6).collect()),
                "settle_ms": take(frames.iter().map(|record| record.layers[at].1 as f64 / 1e6).collect()),
                "apply_settle_ms": take(both),
                "apply_settle_cpu_ms": take(frames.iter().map(|record| (record.cycles[at].0 + record.cycles[at].1) as f64 / self.hz * 1e3).collect()),
                "dirty": take(frames.iter().map(|record| record.layers[at].2 as f64).collect()),
                "marked_per_frame": marked.into_iter().map(|(name, counts)| json!({name: take(counts)})).collect::<Vec<_>>(),
            }));
        }
        let mut tables: Vec<(&usize, &(u64, u64))> = self.mirror_tables.iter().collect();
        tables.sort_by_key(|(_, (sum, _))| std::cmp::Reverse(*sum));
        json!({
            "mirror_by_table": tables.iter().map(|&(&table, &(sum, most))| json!({"table": format!("{:?}", TableId::ALL[table]), "mean_ms": sum as f64 / 1e6 / frames.len().max(1) as f64, "most_ms": most as f64 / 1e6})).collect::<Vec<_>>(),
            "count": frames.len(),
            "frame_ms": take(frames.iter().map(|record| record.frame_ns as f64 / 1e6).collect()),
            "mirror_ms": take(frames.iter().map(|record| record.mirror_ns as f64 / 1e6).collect()),
            "mirror_cpu_ms": take(frames.iter().map(|record| record.mirror_cycles as f64 / self.hz * 1e3).collect()),
            "cpu_note": "the frame thread's own cycles (QueryThreadCycleTime), which time preempted does not accrue, at the calibrated counter rate",
            "layers": layers,
        })
    }
}

fn tsc_hz() -> f64 {
    let (wall, cycles) = (Instant::now(), replay_frame::cpu::stamp());
    std::thread::sleep(Duration::from_millis(100));
    (replay_frame::cpu::stamp() - cycles) as f64 / wall.elapsed().as_secs_f64()
}

fn stalled(progress: &mut Instant, what: &str) -> io::Result<()> {
    if progress.elapsed() > Duration::from_secs(60) {
        return Err(io::Error::other(format!("No step arrived for a minute in {what}")));
    }
    Ok(())
}

fn is_block_end(timeline: &Timeline, side: &MapSide, hour: Hour) -> bool {
    let runs = side.entry().pages().runs();
    timeline.pieces().iter().find(|piece| piece.first <= hour && hour <= piece.last).is_some_and(|piece| {
        let first = runs[piece.run].first_hour;
        (hour - first + 1) % replay_columnar::unit::BLOCK as Hour == 0
    })
}

fn source_check(mirrors: &Mirrors, document: &Document) -> io::Result<Value> {
    let frame = document.frame()?;
    let mut differing = 0u64;
    let mut first = None;
    let mut compared = 0u64;
    let mut check = |table: TableId, rows: Vec<(u128, Vec<(&str, u64)>)>| {
        let Some(mirror) = mirrors.get(table) else { return };
        let kinds_ok = |field: &str| (0..64).find(|&column| table.field_name(column) == Some(field));
        let mut seen = 0;
        for (key, fields) in &rows {
            compared += 1;
            let Some(slot) = mirror.slot(*key) else {
                differing += 1;
                first.get_or_insert_with(|| format!("{table:?} row {key} is not in the playhead"));
                continue;
            };
            seen += 1;
            for &(field, value) in fields {
                let Some(column) = kinds_ok(field) else { continue };
                let held = mirror.raw_cell(column, slot);
                if held != Some(value) {
                    differing += 1;
                    first.get_or_insert_with(|| format!("{table:?} row {key} {field}: today {value}, playhead {held:?}"));
                }
            }
        }
        let live = mirror.rows().count();
        if live != seen {
            differing += 1;
            first.get_or_insert_with(|| format!("{table:?}: the playhead holds {live} rows, today {}", rows.len()));
        }
    };
    let provinces = tables::core::<replay_data::rows::tables::Provinces>(frame)?;
    check(TableId::Provinces, provinces.iter().map(|row| (u128::from(row.id), vec![("state", u64::from(row.state)), ("owner", u64::from(row.owner)), ("controller", u64::from(row.controller))])).collect());
    let countries = tables::core::<replay_data::rows::tables::Countries>(frame)?;
    check(TableId::Countries, countries.iter().map(|row| (u128::from(row.id), vec![("canonical_slot", u64::from(row.canonical_slot))])).collect());
    let links = tables::core::<replay_data::rows::tables::CountryLinks>(frame)?;
    check(TableId::CountryLinks, links.iter().map(|row| (u128::from(row.id), vec![("canonical_slot", u64::from(row.canonical_slot))])).collect());
    Ok(json!({"rows": compared, "differing": differing, "first": first}))
}

fn political_make(document: &Document) -> Box<dyn Layer> {
    Box::new(Political::new(Arc::new(political::Content::new(Arc::clone(&document.content.map), Arc::clone(&document.border_meshes)))))
}

fn political(layer: &dyn Layer) -> &Political {
    layer.any().downcast_ref::<Political>().expect("the political line's layer")
}

fn political_content_bytes(side: &MapSide) -> Value {
    side.parts()
        .and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<Political>()))
        .map_or(Value::Null, |layer| json!({"political_content": layer.content_bytes()}))
}

fn political_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [provinces, keys, edges] = political(layer).marked();
    vec![("provinces", provinces as u64), ("fill keys", keys as u64), ("edges", edges as u64)]
}

fn field(values: &[u32]) -> ProvinceField {
    ProvinceField(Arc::new(Storage::Dense(Arc::new(values.to_vec()))))
}

fn political_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = political(layer);
    if !layer.drawn() {
        return Ok(Compared { entries: 1, notes: json!({"drawn": false}), ..Compared::default() });
    }
    let hour = document.hour()?;
    let mut compared = Compared::default();
    let note = |compared: &mut Compared, what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    let product = match hour.political_product(&MapMode::default(), &perspective::Choice::default(), None, None) {
        Ok(product) => product,
        Err(error) => {
            compared.entries = 1;
            if layer.refused().is_none() {
                note(&mut compared, format!("today refuses the hour ({error}), the layer draws it"));
            }
            compared.notes = json!({"today_refused": error.to_string(), "layer_refused": layer.refused()});
            return Ok(compared);
        }
    };
    if let Some(why) = layer.refused() {
        compared.entries = 1;
        note(&mut compared, format!("the layer refuses the hour ({why}), today draws it"));
        return Ok(compared);
    }
    let regions = &product.layers.regions;
    let zone: &Zone = &product.zone;
    for (name, today, ours) in [("country_of", &regions.country_of, layer.country_of()), ("owner_of", &regions.owner_of, layer.owner_of()), ("state_of", &regions.state_of, layer.state_of())] {
        compared.entries += today.len() as u64;
        if today.len() != ours.len() {
            note(&mut compared, format!("{name}: today {} provinces, the layer {}", today.len(), ours.len()));
        }
        for (id, (a, b)) in today.iter().zip(ours).enumerate() {
            if a != b {
                note(&mut compared, format!("{name}[{id}]: today {a}, the layer {b}"));
            }
        }
    }
    let ours: BTreeMap<u32, [u8; 3]> = layer.colours().collect();
    let today: BTreeMap<u32, [u8; 3]> = regions.country_colors.iter().map(|(&key, &colour)| (key, colour)).collect();
    compared.entries += today.len().max(ours.len()) as u64;
    for key in today.keys().chain(ours.keys()).collect::<std::collections::BTreeSet<_>>() {
        if today.get(key) != ours.get(key) {
            note(&mut compared, format!("colour of key {key}: today {:?}, the layer {:?}", today.get(key), ours.get(key)));
        }
    }
    let warnings = layer.warnings();
    compared.entries += warnings.len() as u64;
    if regions.warnings.len() < warnings.len() || regions.warnings[..warnings.len()] != warnings[..] {
        note(&mut compared, format!("warnings: today {:?}, the layer {:?}", regions.warnings, warnings));
    }
    let meshes = &document.border_meshes;
    let (edges, starts) = layer.content().edges();
    let classes = layer.classes();
    for (edge, &(a, b)) in edges.iter().enumerate() {
        let bucket = starts.partition_point(|&start| start <= edge) - 1;
        let expected = if zone.splits(a, b) {
            LEFT_OUT
        } else if bucket == 0 {
            land_class(regions, a, b).bucket() as u8
        } else {
            BUCKETS[bucket].bucket() as u8
        };
        compared.entries += 1;
        if classes[edge] != expected {
            note(&mut compared, format!("edge {edge} ({a}, {b}): today class {expected}, the layer {}", classes[edge]));
        }
    }
    let mine = Regions {
        country_of: field(layer.country_of()),
        owner_of: field(layer.owner_of()),
        state_of: field(layer.state_of()),
        country_colors: ValueIndex::try_from_iter(ours.iter().map(|(&key, &colour)| (key, colour)))?,
        warnings: Vec::new(),
        strategic_attributes: BTreeMap::new(),
        mode: None,
    };
    let my_zone = Zone::new(&document.content.map, &mine, &layer.demilitarized());
    compared.entries += 3;
    if my_zone != *zone {
        note(&mut compared, "the demilitarised zone differs".into());
    }
    if ClassPlan::new(meshes, &mine, &my_zone) != ClassPlan::new(meshes, regions, zone) {
        note(&mut compared, "the border class plan differs".into());
    }
    if ZonePlan::new(meshes, &mine, &my_zone) != ZonePlan::new(meshes, regions, zone) {
        note(&mut compared, "the demilitarised strips differ".into());
    }
    compared.notes = json!({
        "left_out": {"strategic_attributes": regions.strategic_attributes.len(), "naval_and_mode_warnings": regions.warnings.len().saturating_sub(warnings.len())},
        "provinces": regions.country_of.len(), "fill_keys": today.len(), "edges": edges.len(), "warnings": warnings.len(),
    });
    Ok(compared)
}

fn political_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = political(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: (layer.country_of().len() * 3 + layer.classes().len()) as u64, differing, first, notes: Value::Null })
}

fn give_vision(side: &MapSide, document: &Document, choice: &perspective::Choice) -> io::Result<()> {
    for layer in side.parts().map_or(&[][..], |(_, layers)| layers) {
        if let Some(units) = layer.any().downcast_ref::<Units>() {
            units.set_vision(choice.clone(), document.vision(choice)?);
        }
        p7b::give(&**layer, document, choice)?;
        if let Some(convoys) = layer.any().downcast_ref::<viewer_layers_forces::convoys::Convoys>() {
            convoys.set_vision(choice.clone(), document.vision(choice)?);
        }
        p8c::give(&**layer, side, document, choice)?;
        mode_fills::give(&**layer, side, document, choice)?;
        if let Some(counters) = layer.any().downcast_ref::<Fed>() {
            counters.set_vision(choice.clone(), document.vision(choice)?);
        }
        if let Some(markers) = layer.any().downcast_ref::<viewer_layers_counters::land_markers::LandMarkers>() {
            markers.set_vision(choice.clone(), document.vision(choice)?);
        }
        if let Some(markers) = layer.any().downcast_ref::<viewer_layers_counters::naval_markers::NavalMarkers>() {
            markers.set_vision(choice.clone(), document.vision(choice)?);
        }
        if let Some(bases) = layer.any().downcast_ref::<viewer_layers_counters::base_counters::BaseCountersFed>() {
            bases.set_vision(choice.clone(), document.vision(choice)?);
        }
        if let Some(naval) = layer.any().downcast_ref::<NavalCounters>() {
            naval.set_vision(choice.clone(), document.vision(choice)?);
            naval.set_relations(Arc::clone(document.hour()?.hour_tables(choice, None)?.counters().type_tints()));
        }
        if let Some(orders) = layer.any().downcast_ref::<viewer_layers_forces::orders::Orders>() {
            orders.set_vision(choice.clone(), document.vision(choice)?);
        }
        gun_counters_line::give(&**layer, document, choice)?;
        air_mission_counters_line::give(&**layer, document, choice)?;
    }
    Ok(())
}

fn convoys_make(document: &Document) -> Box<dyn Layer> {
    use viewer_layers_forces::convoys::{Convoys, Routes};
    Box::new(Convoys::new(Arc::clone(&document.content) as Arc<dyn Routes>))
}

fn convoys_of(layer: &dyn Layer) -> &viewer_layers_forces::convoys::Convoys {
    layer.any().downcast_ref().expect("the convoys line's layer")
}

fn convoys_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [rows, products, reused, made, micros] = convoys_of(layer).marked();
    vec![("rows", rows as u64), ("products", products as u64), ("ribbons kept", reused as u64), ("ribbons made", made as u64), ("product us", micros as u64)]
}

fn convoys_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = convoys_of(layer);
    if layer.hidden() {
        let product = layer.product();
        let mut compared = Compared { entries: 1, ..Compared::default() };
        if !product.convoys.is_empty() || !product.field.draws.is_empty() || product.report.drawn != 0 {
            compared.differing = 1;
            compared.first = Some(format!("no trade route is drawn in this map mode, the layer holds {} lines and {} convoy lines", product.report.drawn, product.convoys.len()));
        }
        compared.notes = json!({"not_drawn_at_this_view": layer.refused()});
        return Ok(compared);
    }
    let mut compared = Compared { entries: 2, ..Compared::default() };
    let choice = layer.perspective().cloned().unwrap_or_default();
    let (field, convoys) = document.hour()?.orders_and_convoys(&choice, false)?;
    let product = layer.product();
    let mut note = |what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    if layer.refused().is_some() {
        note(format!("the layer refuses the hour: {:?}", layer.refused()));
    }
    compared.entries += convoys.len() as u64;
    if convoys != product.convoys {
        note(format!("convoys: today {} lines, the layer {}; today {:?}, the layer {:?}", convoys.len(), product.convoys.len(), convoys.first(), product.convoys.first()));
    }
    if field.report.trade_routes != product.report.warnings() {
        note(format!("warnings: today {:?}, the layer {:?}", field.report.trade_routes, product.report.warnings()));
    }
    compared.notes = json!({"drawn": product.report.drawn, "convoy_lines": product.report.convoy_lines, "convoys": product.report.convoys, "unavailable": product.report.unavailable, "refused": product.report.refused});
    Ok(compared)
}

fn convoys_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = convoys_of(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: layer.product().convoys.len() as u64 + 1, differing, first, notes: Value::Null })
}

fn orders_make(document: &Document) -> Box<dyn Layer> {
    use viewer_layers_forces::orders::{Orders, Plans};
    Box::new(Orders::new(Arc::clone(&document.content) as Arc<dyn Plans>))
}

fn orders_of(layer: &dyn Layer) -> &viewer_layers_forces::orders::Orders {
    layer.any().downcast_ref().expect("the orders line's layer")
}

fn orders_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [plans, learned, micros] = orders_of(layer).marked();
    vec![("plans", plans), ("rebuilt for tables learned", learned), ("plan us", micros)]
}

fn orders_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = orders_of(layer);
    let choice = layer.perspective().cloned().unwrap_or_default();
    let today = document.hour()?.plan_field(&choice, layer.show_allied())?;
    let mut compared = Compared { entries: today.draws.len() as u64 + 1, ..Compared::default() };
    let mut note = |what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    match layer.plan() {
        None => note(format!("the layer has no plan: {:?}", layer.refused())),
        Some((_, plan)) => {
            let parts = |field: &viewer_gpu::ribbon_content::OrderField| {
                [
                    ("vertices", format!("{:?}", field.vertices)),
                    ("native vertices", format!("{:?}", field.native_vertices)),
                    ("draws", format!("{:?}", field.draws)),
                    ("trade-route hovers", format!("{:?}", field.trade_route_hovers)),
                    ("raid units", format!("{:?}", field.raid_units)),
                    ("report", format!("{:?}", field.report)),
                ]
            };
            for ((name, ours), (_, theirs)) in parts(plan).into_iter().zip(parts(&today)) {
                if ours != theirs {
                    note(format!("{name}: today {} the layer {}", theirs.chars().take(400).collect::<String>(), ours.chars().take(400).collect::<String>()));
                }
            }
            if let Some(part) = viewer_layers_forces::orders::differ_labels(&today.labels, &plan.labels) {
                note(format!("labels: {part}"));
            }
        }
    }
    compared.notes = json!({"draws": today.draws.len(), "drawn": today.report.drawn, "labels": today.report.labels, "refused": format!("{:?}", today.report.refused)});
    Ok(compared)
}

fn orders_fresh(layer: &dyn Layer, _mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = orders_of(layer);
    Ok(Compared { entries: 1, differing: u64::from(layer.plan().is_none()), first: layer.plan().is_none().then(|| "no plan".to_owned()), notes: json!({"built": "whole, from rows at the head"}) })
}

fn units_make(document: &Document) -> Box<dyn Layer> {
    Box::new(Units::reading_vision(Arc::clone(&document.content) as Arc<dyn Ground>))
}

fn units(layer: &dyn Layer) -> &Units {
    layer.any().downcast_ref::<Units>().expect("the units line's layer")
}

fn units_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [divisions, stacks, figures] = units(layer).marked();
    vec![("divisions", divisions as u64), ("stacks", stacks as u64), ("figures placed", figures as u64)]
}

fn units_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    use viewer_unit_history::unit_history::Kind;
    let layer = units(layer);
    let mut compared = Compared::default();
    if !layer.drawn() {
        compared.entries = 1;
        compared.notes = json!({"drawn": false, "why": layer.refused()});
        return Ok(compared);
    }
    let hour = document.hour()?;
    let note = |compared: &mut Compared, what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    let choice = layer.perspective().cloned().unwrap_or_default();
    let prepared = match hour.prepare_unit_models(&choice) {
        Ok(prepared) => prepared,
        Err(error) => {
            compared.entries = 1;
            if layer.refused().is_none() {
                note(&mut compared, format!("today refuses the hour ({error}), the layer draws it"));
            }
            compared.notes = json!({"today_refused": error.to_string(), "layer_refused": layer.refused()});
            return Ok(compared);
        }
    };
    let report = &prepared.report;
    let field = prepared.field(&document.content.unit_models);
    let today: Vec<_> = field.targets.iter().filter(|target| target.kind == Kind::Land).copied().collect();
    if let Some(why) = layer.refused() {
        compared.entries = 1;
        if !today.is_empty() {
            note(&mut compared, format!("the layer refuses the hour ({why}), today places {} land figures", today.len()));
        }
        return Ok(compared);
    }
    let figures = layer.figures();
    let by_identity: BTreeMap<u64, &Figure> = figures.values().filter(|figure| !figure.admitted).chain(figures.values().filter(|figure| figure.admitted)).map(|figure| (figure.selection.oldest, *figure)).collect();
    let withheld = |why: Withheld| by_identity.values().filter(|figure| matches!(&figure.placed, Some(Err(held)) if *held == why)).count();
    compared.entries += 2;
    let whole = layer.deferred() == 0;
    if whole && report.stacks != layer.stack_count() {
        note(&mut compared, format!("stacks: today {}, the layer {}", report.stacks, layer.stack_count()));
    }
    let unseen = withheld(Withheld::Unseen);
    if whole && report.unseen != unseen {
        note(&mut compared, format!("unseen stacks: today {}, the layer {unseen}", report.unseen));
    }
    let mut outside = 0;
    for target in &today {
        if !layer.admits(target.province) {
            outside += 1;
            continue;
        }
        let figure = by_identity.get(&target.identity);
        compared.entries += 1;
        match figure.map(|figure| &figure.placed) {
            Some(Some(Ok((ours, _)))) => {
                let ours = viewer_unit_history::unit_history::Target { mesh: target.mesh, ..*ours };
                if ours != *target {
                    note(&mut compared, format!("stack of {}: today {target:?}, the layer {ours:?}", target.identity));
                }
            }
            Some(Some(Err(why))) => note(&mut compared, format!("stack of {}: today places it, the layer withholds it ({why:?})", target.identity)),
            Some(None) => note(&mut compared, format!("stack of {}: admitted, its figure is not made once settled", target.identity)),
            None => note(&mut compared, format!("stack of {}: today places it, the layer has no such stack", target.identity)),
        }
    }
    let placed: std::collections::BTreeSet<u64> = today.iter().map(|target| target.identity).collect();
    let extra = by_identity.values().filter(|figure| figure.admitted && matches!(figure.placed, Some(Ok(_))) && !placed.contains(&figure.selection.oldest)).count();
    let own = ["division acclimatization status is unavailable", "division exercise is unavailable this hour"];
    let selection: usize = report.selection_errors.iter().filter(|(reason, _)| !own.contains(&reason.as_str())).map(|(_, count)| count).sum();
    let omitted = report.unknown_identity + report.untemplated + selection;
    compared.entries += 1;
    if extra > omitted {
        note(&mut compared, format!("the layer places {extra} stacks today omits, beyond today's {omitted} selection omissions"));
    }
    compared.notes = json!({
        "stacks": report.stacks, "land_targets": today.len(), "placed_not_selected": extra, "today_selection_omissions": omitted,
        "withheld_status": withheld(Withheld::Status), "withheld_unanchored": withheld(Withheld::Unplaced(viewer_unit_models::unit_models::Unplaced::Unanchored)),
        "withheld_exercise": withheld(Withheld::Unplaced(viewer_unit_models::unit_models::Unplaced::Exercise)),
        "today_unanchored": report.unanchored, "today_errors": report.selection_errors, "today_moving": report.moving, "today_transports": report.transports,
        "admitted": layer.admitted(), "outside_compared": outside, "deferred": layer.deferred(), "front": layer.front(256.0), "units_cutoff": layer.cutoff(),
        "left_out": {"ships": field.targets.len() - today.len(), "raid_units": "P9", "convoys": "trade routes"},
    });
    Ok(compared)
}

fn units_fresh(layer: &dyn Layer, mirrors: &Mirrors, upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = units(layer);
    let mut fresh = layer.fresh();
    fresh.compose_with(mirrors, upstream)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: layer.stack_count() as u64 + 1, differing, first, notes: Value::Null })
}

struct Declare {
    lines: Vec<(TableId, Option<String>)>,
}

impl Declare {
    fn read(path: &std::path::Path) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut lines = Vec::new();
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
            let (name, columns) = line.split_once(':').map_or((line, None), |(name, columns)| (name, Some(columns.to_owned())));
            let table = TableId::ALL.iter().copied().find(|table| format!("{table:?}") == name).ok_or_else(|| invalid(format!("No table {name}")))?;
            lines.push((table, columns));
        }
        Ok(Self { lines })
    }
}

impl Layer for Declare {
    fn name(&self) -> &'static str {
        "select"
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        let kinds = replay_recording::schema::schema()?;
        for (table, columns) in &self.lines {
            inputs.table(*table, false)?;
            let held = &kinds[table.index()];
            let list = |kind| matches!(kind, replay_columnar::state::Kind::U32List | replay_columnar::state::Kind::U64List | replay_columnar::state::Kind::Opaque);
            match columns.as_deref() {
                None => (0..held.len()).try_for_each(|column| inputs.index(*table, column))?,
                Some("numeric") => (0..held.len()).filter(|&column| !list(held[column])).try_for_each(|column| inputs.index(*table, column))?,
                Some(names) => {
                    for name in names.split(',') {
                        let column = (0..held.len()).find(|&column| table.field_name(column) == Some(name)).ok_or_else(|| invalid(format!("No column {name} in {table:?}")))?;
                        inputs.index(*table, column)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        Ok(())
    }

    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}

    fn settle(&mut self, _mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        Ok(())
    }


    fn carried(&self) -> Carried {
        Carried::default()
    }

    fn held(&self) -> usize {
        0
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

mod sounds_line {
    use super::{Compared, Document, Layer, Mirrors};
    use viewer_layers_infrastructure::sounds::{Sounds, SoundsFed, differ_battles};
    use serde_json::json;
    use std::{io, sync::Arc};

    fn of(layer: &dyn Layer) -> &SoundsFed {
        layer.any().downcast_ref::<SoundsFed>().expect("the sounds line's layer")
    }

    pub(super) fn make(document: &Document) -> Box<dyn Layer> {
        Box::new(SoundsFed::new(Sounds::new(document.content.audio.as_ref().map(|source| Arc::clone(&source.content)))))
    }

    pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
        let fog = document.hour()?.fog_products(&viewer_foundation::perspective::Choice::default(), &map_products::FogMemos::new(), &mut viewer_runtime::profiling::BuilderSpans::new(false), true)?;
        let (today, ours) = (fog.battles.as_deref(), of(layer).battles());
        let (differing, first) = differ_battles(ours.map(Arc::as_ref), today);
        let battles = today.map_or(0, |battles| battles.battles.len());
        Ok(Compared { entries: battles as u64 + 1, differing, first, notes: json!({"battles": battles, "divisions": today.map(|battles| battles.battles.iter().map(|battle| battle.divisions).sum::<usize>()), "refusals": today.map(|battles| battles.refusals.clone())}) })
    }

    pub(super) fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
        let layer = of(layer);
        let mut fresh = layer.fresh();
        fresh.compose(mirrors)?;
        let (differing, first) = differ_battles(layer.battles().map(Arc::as_ref), fresh.product().as_ref());
        Ok(Compared { entries: layer.battles().map_or(0, |battles| battles.battles.len()) as u64 + 1, differing, first, notes: serde_json::Value::Null })
    }

    pub(super) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
        let [combats, provinces, battles] = of(layer).marked();
        vec![("combats", combats as u64), ("provinces", provinces as u64), ("battles", battles as u64)]
    }
}

mod strikes_line {
    use super::{Compared, Document, Layer, Mirrors};
    use viewer_layers_infrastructure::strikes::{Strikes, differ_impacts};
    use serde_json::json;
    use std::{io, sync::Arc};

    fn of(layer: &dyn Layer) -> &Strikes {
        layer.any().downcast_ref::<Strikes>().expect("the strikes line's layer")
    }

    pub(super) fn make(document: &Document) -> Box<dyn Layer> {
        let content = Arc::clone(&document.content);
        Box::new(Strikes::new(Arc::new(move |province| content.products.impact_place(province))))
    }

    pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
        let today = document.nuclear_impacts()?;
        let (differing, first) = differ_impacts(&of(layer).product(), &today);
        Ok(Compared { entries: today.impacts.len() as u64 + 1, differing, first, notes: json!({"impacts": today.impacts.len(), "refused": today.refused}) })
    }

    pub(super) fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
        let layer = of(layer);
        let mut fresh = layer.fresh();
        fresh.compose(mirrors)?;
        let (differing, first) = layer.differ(&fresh);
        Ok(Compared { entries: layer.product().impacts.len() as u64 + 1, differing, first, notes: serde_json::Value::Null })
    }

    pub(super) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
        let [rows, born] = of(layer).marked();
        vec![("strike rows", rows as u64), ("impacts born", born as u64)]
    }
}

mod land_markers_line {
    use super::{Compared, Document, Layer, engagements_line::FOLDED};
    use viewer_layer_core::engagements::Index;
    use viewer_playback::proofs::land_markers as proof;
    use std::{io, sync::Arc};

    pub(super) fn make(document: &Document) -> Box<dyn Layer> {
        let held = FOLDED.lock().unwrap_or_else(|p| p.into_inner());
        proof::make_with(document, held.as_ref().map(|(folded, _)| Index { history: Arc::clone(&folded.lanes), branch: folded.branch }))
    }

    pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
        let held = FOLDED.lock().unwrap_or_else(|p| p.into_inner());
        let Some((folded, _)) = held.as_ref() else { return Err(io::Error::other("the land markers line needs --engaged-fold recorded")) };
        proof::today_with(layer, document, &folded.today)
    }

    pub(super) fn make_naval(document: &Document) -> Box<dyn Layer> {
        let held = FOLDED.lock().unwrap_or_else(|p| p.into_inner());
        viewer_playback::proofs::naval_markers::make_with(document, held.as_ref().map(|(folded, _)| Index { history: Arc::clone(&folded.lanes), branch: folded.branch }))
    }

    pub(super) fn today_naval(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
        let held = FOLDED.lock().unwrap_or_else(|p| p.into_inner());
        let Some((folded, _)) = held.as_ref() else { return Err(io::Error::other("the naval markers line needs --engaged-fold recorded")) };
        viewer_playback::proofs::naval_markers::today_with(layer, document, &folded.today)
    }
}

mod engagements_line {
    use super::{Compared, Document, Layer, Mirrors, engaged::Folded};
    use viewer_layer_core::engagements::{Engagements, Index};
    use serde_json::json;
    use std::{collections::BTreeMap, io, sync::{Arc, Mutex}};
    use viewer_engagements::engagement_history::CombatFlags;
    use viewer_foundation::{tables::core, visibility::Relations};

    pub(super) static FOLDED: Mutex<Option<(Folded, u32)>> = Mutex::new(None);

    fn of(layer: &dyn Layer) -> &Engagements {
        layer.any().downcast_ref::<Engagements>().expect("the engagements line's layer")
    }

    pub(super) fn make(_: &Document) -> Box<dyn Layer> {
        let held = FOLDED.lock().unwrap_or_else(|p| p.into_inner());
        Box::new(match held.as_ref() {
            Some((folded, viewer)) => Engagements::new(Some(Index { history: Arc::clone(&folded.lanes), branch: folded.branch }), Some(*viewer)),
            None => Engagements::new(None, None),
        })
    }

    pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
        let layer = of(layer);
        let held = FOLDED.lock().unwrap_or_else(|p| p.into_inner());
        let Some((folded, viewer)) = held.as_ref() else { return Err(io::Error::other("the engagements line needs --engaged-fold recorded")) };
        let frame = document.frame()?;
        let links: BTreeMap<u32, u32> = core::<replay_data::rows::tables::CountryLinks>(frame)?.iter().map(|link| (link.id, link.canonical_slot)).collect();
        let relations = Relations::read(frame, &links);
        let today = CombatFlags::read(frame, *viewer, &links, relations.as_ref().map_err(|reason| *reason), Some(&folded.today))?;
        let (land, naval, provenance) = layer.flags();
        let mut compared = Compared { entries: (today.land.len() + today.naval.len() + 2) as u64, ..Compared::default() };
        for (what, ours, theirs) in [("land", land, &today.land), ("naval", naval, &today.naval)] {
            for combat in ours.keys().chain(theirs.keys()).collect::<std::collections::BTreeSet<_>>() {
                if ours.get(combat) != theirs.get(combat) {
                    compared.differing += 1;
                    compared.first.get_or_insert(format!("{what} combat {combat}: {:?} against today's {:?}", ours.get(combat), theirs.get(combat)));
                }
            }
        }
        if provenance != [today.land_provenance, today.naval_provenance] {
            compared.differing += 1;
            compared.first.get_or_insert(format!("provenance {provenance:?} against today's {:?}", [today.land_provenance, today.naval_provenance]));
        }
        compared.notes = json!({"land": today.land.len(), "naval": today.naval.len(), "engaged": today.land.values().chain(today.naval.values()).filter(|flag| **flag == viewer_engagements::engaged::Engaged::Yes).count()});
        Ok(compared)
    }

    pub(super) fn fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
        let layer = of(layer);
        let mut fresh = layer.fresh();
        fresh.compose(mirrors)?;
        let differing = u64::from(layer.flags() != fresh.flags());
        Ok(Compared { entries: (layer.flags().0.len() + layer.flags().1.len()) as u64, differing, first: (differing > 0).then(|| "the composed flags differ".into()), notes: serde_json::Value::Null })
    }

    pub(super) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
        vec![("combats read", of(layer).marked() as u64)]
    }
}
