
use super::super::views::{Need, Pace, Reader as ViewReader, Views};
use crate::{document::{Document, Hour, OverlayRows, same_quads}, map_views::CounterView};
use viewer_layer_core::scene_content::OverlayLayers;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, discovering};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_gpu::{camera::{Camera, MapSize}, province_map::HeightField};
use viewer_native_gui::counter_hovers::unit_hits::{Hit, Kind};
use viewer_unit_reads::counter_move::MovementFrame;
use viewer_foundation::perspective::Choice;
use viewer_runtime::unit_handle::Unit;
use viewer_map_modes::map_mode::{MapMode, Stock};

const VIEWPORT: [u32; 2] = [3840, 2160];
const MODES: [Stock; 5] = [Stock::StrategicAir, Stock::StrategicNavy, Stock::Resistance, Stock::Compliance, Stock::Resources];
const PICKS: usize = 400;

#[derive(Default)]
struct Picks {
    units: Vec<Unit>,
    wings: Vec<u64>,
    fleets: Vec<u64>,
}

pub(crate) struct WindowLine {
    pub(crate) reader: ViewReader,
    tables: Vec<TableId>,
    placement: String,
    camera: Option<Camera>,
    picks: Option<Picks>,
    reads: BTreeMap<String, BTreeSet<TableId>>,
    pub(crate) relaid: u64,
    spent: BTreeMap<String, Vec<(u64, u64)>>,
    hz: f64,
}

pub(crate) struct Output {
    pub(crate) name: String,
    pub(crate) entries: u64,
    pub(crate) differing: u64,
    pub(crate) first: Option<String>,
}

impl WindowLine {
    pub(crate) fn open(views: &Views, spec: &str, pace: Pace) -> io::Result<Self> {
        let (placement, listed) = spec.split_once('+').map_or((spec, None), |(placement, file)| (placement, Some(file)));
        let mut tables = Vec::new();
        if let Some(file) = listed {
            for line in std::fs::read_to_string(file)?.lines().map(str::trim).filter(|line| !line.is_empty()) {
                tables.push(TableId::ALL.into_iter().find(|table| format!("{table:?}") == line).ok_or_else(|| io::Error::other(format!("{file} names no table {line}")))?);
            }
        }
        Ok(Self { reader: views.reader_at("proof window", &tables.iter().copied().map(Need::every).collect::<Vec<Need>>(), pace), tables, placement: placement.replace(':', ","), camera: None, picks: None, reads: BTreeMap::new(), relaid: 0, spent: BTreeMap::new(), hz: cycle_rate() })
    }

    pub(crate) fn spent(&self, distribution: fn(Vec<f64>) -> serde_json::Value) -> serde_json::Value {
        let spent: serde_json::Map<String, serde_json::Value> = self
            .spent
            .iter()
            .map(|(name, spent)| {
                let wall = distribution(spent.iter().map(|&(ns, _)| ns as f64 / 1e6).collect());
                let cpu = distribution(spent.iter().map(|&(_, cycles)| cycles as f64 / self.hz * 1e3).collect());
                (name.clone(), serde_json::json!({"wall_ms": wall, "cpu_ms": cpu}))
            })
            .collect();
        serde_json::Value::Object(spent)
    }

    pub(crate) fn tables(&self) -> &[TableId] {
        &self.tables
    }

    pub(crate) fn reads(&self) -> &BTreeMap<String, BTreeSet<TableId>> {
        &self.reads
    }

    pub(crate) fn compare(&mut self, document: &Document, today: &Arc<DecodedFrame>, rows: &Arc<DecodedFrame>) -> io::Result<Result<Vec<Output>, Vec<TableId>>> {
        if self.camera.is_none() {
            let (size, defines) = (MapSize::of(&document.content.map), document.content.map_defines);
            self.camera = Some(match self.placement.as_str() {
                "" | "fit" => Camera::fit(size, defines, VIEWPORT),
                placed => crate::parse_placement(placed)?.on(size, defines),
            });
        }
        let camera = self.camera.expect("made above");
        let ours = Hour::new(&document.content, today, 0.0).vision(&Choice::default())?.ours();
        let picks = self.picks.take().unwrap_or_else(|| picks(today));
        let mut tries = 0;
        let (wanted, got, missed) = loop {
            let textures = document.icons.lazy.version();
            let wanted = evaluate(document, today, today, None, ours, &picks, camera)?;
            let (got, mut missed) = discovering(|| evaluate(document, today, rows, Some(rows), ours, &picks, camera));
            let got = got?;
            for &table in &got.missed {
                if !missed.contains(&table) {
                    missed.push(table);
                }
            }
            if !missed.is_empty() || document.icons.lazy.version() == textures || tries == 3 {
                break (wanted, got, missed);
            }
            tries += 1;
            self.relaid += 1;
        };
        for (name, tables) in &got.reads {
            self.reads.entry(name.clone()).or_default().extend(tables.iter().copied());
        }
        self.picks = Some(self::picks(today));
        if !missed.is_empty() {
            for &table in &missed {
                if !self.tables.contains(&table) {
                    self.tables.push(table);
                }
            }
            self.reader.declare_only(&self.tables);
            return Ok(Err(missed));
        }
        for (name, ns, cycles) in &got.spent {
            self.spent.entry(name.clone()).or_default().push((*ns, *cycles));
        }
        let mut outputs = Vec::new();
        for ((name, wanted), (_, got)) in wanted.texts.iter().zip(&got.texts) {
            outputs.push(Output { name: name.clone(), entries: 1, differing: u64::from(wanted != got), first: (wanted != got).then(|| super::proof::first_difference(wanted, got)) });
        }
        for ((name, wanted), (_, got)) in wanted.quads.iter().zip(&got.quads) {
            let same = same_quads(wanted, got);
            outputs.push(Output { name: name.clone(), entries: wanted.len().max(got.len()) as u64, differing: u64::from(!same), first: (!same).then(|| format!("{} quads today, {} at the head", wanted.len(), got.len())) });
        }
        Ok(Ok(outputs))
    }
}

fn picks(today: &DecodedFrame) -> Picks {
    fn spread<R>(rows: Result<&[R], impl Sized>, id: impl Fn(&R) -> u64) -> Vec<u64> {
        let Ok(rows) = rows else { return Vec::new() };
        let every = rows.len().div_ceil(PICKS).max(1);
        rows.iter().step_by(every).map(id).collect()
    }
    let mut units: Vec<Unit> = spread(today.complete::<tables::Divisions>(), |row| row.id).into_iter().map(Unit::Division).collect();
    units.extend(spread(today.complete::<tables::RailwayGunCounterInputs>(), |row| row.gun).into_iter().map(Unit::RailwayGun));
    units.extend(spread(today.complete::<tables::TaskForces>(), |row| row.id).into_iter().map(Unit::TaskForce));
    Picks { units, wings: spread(today.complete::<tables::AirWings>(), |row| row.id), fleets: spread(today.complete::<tables::Fleets>(), |row| row.id) }
}

struct Evaluated {
    texts: Vec<(String, String)>,
    spent: Vec<(String, u64, u64)>,
    reads: Vec<(String, Vec<TableId>)>,
    quads: Vec<(String, Vec<viewer_native_gui::sprites::Quad>)>,
    missed: Vec<TableId>,
}

fn evaluate(document: &Document, today: &DecodedFrame, frame: &DecodedFrame, rows: Option<&Arc<DecodedFrame>>, ours: u32, picks: &Picks, camera: Camera) -> io::Result<Evaluated> {
    let content = &document.content;
    let (mut texts, mut reads, mut spent) = (Vec::new(), Vec::new(), Vec::new());
    let mut text = |name: &str, ((value, missed), ns, cycles): ((String, Vec<TableId>), u64, u64)| {
        texts.push((name.to_owned(), value));
        reads.push((name.to_owned(), missed));
        spent.push((name.to_owned(), ns, cycles));
    };

    let branch = document.playback.branch();
    let mut units = viewer_counters::unit_selection::Selection::default();
    units.context(branch);
    for (kind, of) in [(Kind::Division, 0), (Kind::RailwayGun, 1), (Kind::TaskForce, 2)] {
        let members: Vec<u64> = picks
            .units
            .iter()
            .filter_map(|unit| match (unit, of) {
                (Unit::Division(id), 0) | (Unit::RailwayGun(id), 1) | (Unit::TaskForce(id), 2) => Some(*id),
                _ => None,
            })
            .collect();
        units.click(Some(&Hit { ours: true, port: false, rect: [0.0; 4], kind, members: members.into(), layer: 0, plate: None }), true);
    }
    text("observe:units", timed(|| {
        units.observe(branch, frame);
        format!("{:?}", units.units().iter().collect::<BTreeSet<_>>())
    }));
    let selected: BTreeSet<Unit> = units.units().iter().copied().collect();
    let mut air = viewer_counters::air_selection::Selection::default();
    air.context(branch);
    air.click(&picks.wings, false);
    text("observe:air", timed(|| {
        air.observe(branch, ours, frame);
        format!("{:?}", air.wings())
    }));
    let mut fleets = viewer_counters::naval_selection::Selection::default();
    fleets.context(branch);
    for &fleet in &picks.fleets {
        fleets.click(fleet, true);
    }
    text("observe:fleets", timed(|| {
        fleets.observe(branch, frame);
        format!("{:?}", fleets.fleets())
    }));

    let heights = HeightField { width: content.assets.heightmap.width, height: content.assets.heightmap.height, values: content.assets.heightmap_texels() };
    text("arrows", timed(|| {
        let (routes, warnings) = viewer_counter_products::map::movement_routes::build(&content.counter_content, heights, content.map.width as f32, frame, ours, &selected);
        let routes: Vec<String> = routes.iter().map(|route| format!("{:?} {} {:?} {:?} {} {} {}", route.unit, route.province, route.style, route.controls, route.total_arc, route.fill_target, route.free_arc)).collect();
        format!("{routes:?} {warnings:?} {:?}", viewer_counter_products::map::movement_routes::selected_provinces(frame, &selected))
    }));
    text("clock", timed(|| format!("{:?}", MovementFrame::clock_speed(frame))));
    text("picker", timed(|| format!("{:?}", crate::app::picker::four_buttons_at(frame))));
    text("walk", timed(|| format!("{:?}", document.unit_models.infantry_figure(frame, ours))));
    text("clicks:fleet_choices", timed(|| format!("{:?}", viewer_counters::naval_selection::fleet_choices(frame, ours))));
    text("clicks:grid", timed(|| {
        let mut grid = viewer_counters::air_selection::Selection::default();
        let clicked = grid.grid_click(frame, &picks.wings, false).map_err(|error| error.to_string());
        format!("{clicked:?} {:?} {:?}", grid.wings(), grid.bases())
    }));

    let hour = Hour::new(content, today, 0.0);
    let overlay_tables = hour.hour_tables(&Choice::default(), None)?;
    overlay_tables.warm_counter_flags(&document.icons.counters);
    let mut quads = Vec::new();
    let mut missed = Vec::new();
    for stock in MODES {
        let overlay_rows = OverlayRows::at_head(rows.cloned());
        let mode = MapMode::Stock(stock);
        let mut view = CounterView::new(Default::default());
        view.set_mode(&mode);
        view.selected = Arc::new(selected.clone());
        view.air_selected = Arc::new(picks.wings.clone());
        view.fleets_selected = Arc::new(picks.fleets.clone());
        let mut prepared = hour.prepare_overlay_at(camera, VIEWPORT, 1.0, &overlay_tables, &view, &overlay_rows)?;
        let naval = &prepared.naval_selection;
        let inputs = format!("{:?} | {:?} | {:?} | {:?} | {:?} {:?} {:?}", prepared.air_ranges, prepared.air_assignment, prepared.occupation_stripes, prepared.air_orders, naval.assignment, naval.styles, naval.refused);
        texts.push((format!("overlay:{stock:?}"), inputs));
        reads.push((format!("overlay:{stock:?}"), overlay_rows.missed()));
        for (input, ns, cycles) in overlay_rows.spent() {
            spent.push((format!("overlay:{stock:?}:{input}"), ns, cycles));
        }
        missed.extend(overlay_rows.missed());
        let (mut batch, mut report) = (Default::default(), Default::default());
        prepared.draw_into(&hour, camera, VIEWPORT, 1.0, &OverlayLayers::omitting(Default::default()), 0.0, &mut batch, &mut report, &mut Default::default())?;
        quads.push((format!("overlay:{stock:?}:quads"), std::mem::take(&mut batch.quads)));
    }
    Ok(Evaluated { texts, spent, reads, quads, missed })
}

fn timed(read: impl FnOnce() -> String) -> ((String, Vec<TableId>), u64, u64) {
    let (started, cycles) = (std::time::Instant::now(), replay_frame::cpu::thread_cycles());
    let read = discovering(read);
    (read, started.elapsed().as_nanos() as u64, replay_frame::cpu::thread_cycles().saturating_sub(cycles))
}

fn cycle_rate() -> f64 {
    let (started, cycles) = (std::time::Instant::now(), replay_frame::cpu::thread_cycles());
    while started.elapsed() < std::time::Duration::from_millis(50) {
        std::hint::spin_loop();
    }
    replay_frame::cpu::thread_cycles().saturating_sub(cycles) as f64 / started.elapsed().as_secs_f64()
}
