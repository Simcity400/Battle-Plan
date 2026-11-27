
use super::super::views::{AtHead, Need, Pace, Reader as ViewReader, Views};
use crate::{app::combat_rows::CombatRows, document::Document, playhead_host::Entry};
use replay::campaign::SeekResult;
use replay_data::{
    Hour,
    rows::{TableId, tables},
};
use replay_frame::frame::{DecodedFrame, discovering, view_reads};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};
use viewer_combat::{Selector, Window, division_panel, land_marker_tooltips, naval_marker_tooltips, terrain_mode};
use viewer_engagements::combat_selection::Selection;
use viewer_foundation::{perspective::Choice, tooltip_text::Tooltip};

const LIMIT: Duration = Duration::from_secs(120);

const READERS: [&str; 8] = ["window", "selector", "land_marker", "naval_marker", "division_panel", "terrain", "infrastructure", "raid"];

#[derive(Clone, Debug, Default)]
struct Targets {
    land: Vec<(u64, u32)>,
    naval: Vec<(u64, u32)>,
    results: Vec<u64>,
    regions: Vec<u32>,
    divisions: Vec<u64>,
    raids: Vec<viewer_raids::map::raids::tooltips::Key>,
}

impl Targets {
    fn of(document: &Document, today: &DecodedFrame, ours: u32) -> Self {
        let mut targets = Self::default();
        if let Ok(rows) = today.complete::<tables::LandCombats>() {
            targets.land = rows.iter().map(|row| (row.id, row.province)).collect();
        }
        if let Ok(rows) = today.complete::<tables::NavalCombats>() {
            targets.naval = rows.iter().map(|row| (row.id, row.province)).collect();
        }
        if let Ok(rows) = today.complete::<tables::NavalResults>() {
            targets.results = rows.iter().filter(|row| row.result_hour <= today.hour).map(|row| row.id).collect();
        }
        if let Ok(rows) = today.complete::<tables::AirWings>() {
            targets.regions = rows.iter().filter_map(|row| row.mission_region).collect::<BTreeSet<_>>().into_iter().collect();
        }
        if let Ok(rows) = today.complete::<tables::LandCombatDivisions>() {
            targets.divisions = rows.iter().map(|row| row.division).collect::<BTreeSet<_>>().into_iter().collect();
        }
        let assets = &document.icons.raids;
        let paths = viewer_raids::map::raids::paths::HourPaths::new(assets, today);
        targets.raids = viewer_raids::map::raids::icons(assets, today, ours, &paths).icons.into_iter().filter_map(|icon| icon.tooltip).collect();
        targets
    }

    fn provinces(&self) -> Vec<u32> {
        self.land.iter().chain(&self.naval).map(|&(_, province)| province).collect::<BTreeSet<_>>().into_iter().collect()
    }
}

fn outputs(name: &str, document: &Document, frame: &Arc<DecodedFrame>, targets: &Targets, ours: u32, vision: &viewer_foundation::perspective::Vision) -> Vec<String> {
    let content = &document.combat;
    match name {
        "window" => {
            let selections = targets.land.iter().map(|&(id, _)| Selection::Land(id)).chain(targets.naval.iter().map(|&(id, _)| Selection::Naval(id))).chain(targets.results.iter().map(|&id| Selection::NavalResult(id))).chain(targets.regions.iter().map(|&region| Selection::AirRegion(region)));
            selections
                .map(|selection| {
                    let mut window = Window::default();
                    window.set_battle_title_source(crate::battle_title_source(document));
                    window.open(selection);
                    format!("{selection:?}: {:?}", window.prepare(content, &document.playback.roster_sampler(), frame.branch, frame.hour, Ok(Arc::clone(frame)), &Choice::default(), None, false))
                })
                .collect()
        }
        "selector" => targets.land.iter().chain(&targets.naval).flat_map(|&(id, province)| [Selector::Id(id), Selector::Province(province)]).map(|selector| format!("{selector:?}: {:?}", selector.resolve(frame))).collect(),
        "land_marker" => targets.land.iter().map(|&(id, _)| format!("{id}: {:?}", land_marker_tooltips::build(content, frame, ours, id, None, false))).collect(),
        "naval_marker" => targets.naval.iter().map(|&(id, _)| format!("{id}: {:?}", naval_marker_tooltips::build(content, frame, ours, id, None))).collect(),
        "division_panel" => targets.divisions.iter().map(|&id| format!("{id}: {:?}", division_panel(content, frame, id))).collect(),
        "terrain" => targets.provinces().into_iter().map(|province| format!("{province}: {:?}", terrain_mode::hover(content, frame, province, vision))).collect(),
        "infrastructure" => targets.provinces().into_iter().map(|province| format!("{province}: {:?}", document.map_modes.infrastructure.hover_level(frame, province))).collect(),
        "raid" => targets.raids.iter().map(|key| format!("{key:?}: {:?}", viewer_raids::map::raids::tooltips::build(&document.raid_hover, frame, ours, false, key))).collect(),
        _ => Vec::new(),
    }
}

struct Reader {
    name: &'static str,
    reader: ViewReader,
    tables: Vec<TableId>,
}

struct Timing {
    rows: CombatRows,
    window: Window,
    window_rows: AtHead,
    window_reads: Vec<u64>,
    battle: AtHead,
    targets: Option<(u64, u32, Option<u64>, Option<u64>)>,
    base: Arc<Tooltip>,
    frames: Vec<(u64, u64)>,
    each: [Vec<(u64, u64)>; 6],
    hz: f64,
    behind: Vec<f64>,
    loading: u64,
}

pub(crate) const TIMED: [&str; 6] = ["window", "land_marker", "naval_marker", "division_panel", "terrain", "infrastructure"];

pub(crate) struct StepTimer {
    _entry: Arc<Entry>,
    reader: playhead::Reader,
    consumer: playhead::Consumer,
    steps: Vec<Arc<playhead::Step>>,
    ns: Vec<u64>,
    last: Option<u64>,
}

impl StepTimer {
    pub(crate) fn open(entry: &Arc<Entry>) -> io::Result<Self> {
        let playhead = entry.playhead();
        let reader = playhead.declare(Vec::new(), None)?;
        let consumer = playhead.consumer(reader);
        Ok(Self { _entry: Arc::clone(entry), reader, consumer, steps: Vec::new(), ns: Vec::new(), last: None })
    }

    pub(crate) fn frame(&mut self, head: Hour) {
        self.steps.clear();
        self.consumer.take(head, &mut self.steps);
        for step in &self.steps {
            if self.last.is_none_or(|last| step.seq() > last) {
                self.last = Some(step.seq());
                if !step.reset() {
                    self.ns.push(step.built().ns);
                }
            }
        }
    }

    pub(crate) fn summary(&self, distribution: fn(Vec<f64>) -> Value) -> Value {
        json!({"steps": self.ns.len(), "step_ms": distribution(self.ns.iter().map(|&ns| ns as f64 / 1e6).collect())})
    }
}

impl Drop for StepTimer {
    fn drop(&mut self) {
        self._entry.playhead().retire(self.reader);
    }
}

pub(crate) struct Line {
    views: Views,
    readers: Vec<Reader>,
    timing: Timing,
    timed: Vec<&'static str>,
    battles: Vec<(u64, Hour, Hour, usize)>,
    totals: BTreeMap<String, (u64, u64, u64)>,
}

impl Line {
    pub(crate) fn open(document: &Document, views: Views, timed: &[&'static str], learned: Option<&str>, timing: bool, hz: f64) -> io::Result<Self> {
        let mut known: BTreeMap<&str, Vec<TableId>> = BTreeMap::new();
        if let Some(file) = learned {
            for line in std::fs::read_to_string(file)?.lines().map(str::trim).filter(|line| !line.is_empty()) {
                let (name, table) = line.split_once(':').ok_or_else(|| io::Error::other(format!("{file}: {line} is not reader: Table")))?;
                let name = READERS.iter().copied().find(|reader| *reader == name.trim()).ok_or_else(|| io::Error::other(format!("{file}: no reader {name}")))?;
                let table = TableId::ALL.into_iter().find(|id| format!("{id:?}") == table.trim()).ok_or_else(|| io::Error::other(format!("{file}: no table {table}")))?;
                known.entry(name).or_default().push(table);
            }
        }
        let readers = READERS.iter().map(|&name| {
            let tables = if timing { Vec::new() } else { known.get(name).cloned().unwrap_or_default() };
            Reader { name, reader: views.reader_at(name, &tables.iter().copied().map(Need::every).collect::<Vec<Need>>(), Pace::Step), tables }
        });
        let readers: Vec<Reader> = readers.collect();
        let mut combat = CombatRows::default();
        combat.time();
        for name in TIMED {
            combat.learned(name, known.get(name).map_or(&[][..], Vec::as_slice));
        }
        let mut window_rows = AtHead::learning("combat window");
        window_rows.learn(known.get("window").map_or(&[][..], Vec::as_slice));
        let battle = AtHead::new("proof battle", &[TableId::LandCombats, TableId::LandCombatDivisions, TableId::NavalCombats], Pace::PerSecond(4));
        let mut window = Window::default();
        window.set_battle_title_source(crate::battle_title_source(document));
        let timing = Timing { rows: combat, window, window_rows, window_reads: Vec::new(), battle, targets: None, base: Arc::new(Tooltip::default()), frames: Vec::new(), each: Default::default(), hz, behind: Vec::new(), loading: 0 };
        Ok(Self { views, readers, timing, timed: timed.to_vec(), battles: Vec::new(), totals: BTreeMap::new() })
    }

    pub(crate) fn compare(&mut self, document: &Document, hour: Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        let today: Arc<DecodedFrame> = match document.playback.replay().seek_selected(document.playback.branch(), hour, None)? {
            SeekResult::Frame(frame) => Arc::new(*frame),
            SeekResult::Gap { reason, .. } => return Err(io::Error::other(format!("Today's reader finds a gap at {hour}: {reason}"))),
        };
        let vision = document.hour()?.vision(&Choice::default())?;
        let ours = vision.ours();
        let targets = Targets::of(document, &today, ours);
        let mut declared = false;
        for at in 0..self.readers.len() {
            let name = self.readers[at].name;
            let (wanted, reads) = view_reads::record(|| outputs(name, document, &today, &targets, ours, &vision));
            let read: Vec<TableId> = TableId::ALL.into_iter().filter(|table| !today.tables[table.index()].is_not_selected() && reads.iter().any(|identity| identity.is(&today.tables[table.index()]))).collect();
            if self.learn(at, &read) {
                declared = true;
                let notes = json!({"declared": self.readers[at].tables.len(), "from": "today's reads"});
                self.note(format!("{name}:declared"), (0, 0, None), hour, why, notes, out)?;
                continue;
            }
            if self.readers[at].reader.frame().is_none() {
                self.note(format!("{name}:loading"), (0, 0, None), hour, why, json!({}), out)?;
                continue;
            }
            let Some(frame) = at_hour(&self.readers[at].reader, hour)? else {
                self.note(format!("{name}:ahead"), (0, 0, None), hour, why, json!({}), out)?;
                continue;
            };
            let (got, missed) = discovering(|| outputs(name, document, &frame, &targets, ours, &vision));
            if self.learn(at, &missed) {
                declared = true;
                let notes = json!({"declared": self.readers[at].tables.len(), "named_at_the_head": missed.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>()});
                self.note(format!("{name}:declared"), (0, 0, None), hour, why, notes, out)?;
                continue;
            }
            let differing = wanted.iter().zip(&got).filter(|(wanted, got)| wanted != got).count() + wanted.len().abs_diff(got.len());
            let first = wanted.iter().zip(&got).find(|(wanted, got)| wanted != got).map(|(wanted, got)| super::proof::first_difference(wanted, got));
            let tables: Vec<Value> = if differing == 0 {
                Vec::new()
            } else {
                self.readers[at].tables.iter().map(|&table| (table, super::proof::table_against(&frame, &today, table))).filter(|(_, compared)| compared.differing > 0).map(|(table, compared)| json!({"table": format!("{table:?}"), "differing": compared.differing, "first": compared.first})).collect()
            };
            let notes = json!({"tables": self.readers[at].tables.len(), "differing_tables": tables});
            self.note(name.to_owned(), (wanted.len().max(got.len()) as u64, differing as u64, first), hour, why, notes, out)?;
        }
        if declared {
        }
        Ok(())
    }

    fn learn(&mut self, at: usize, tables: &[TableId]) -> bool {
        let reader = &mut self.readers[at];
        let before = reader.tables.len();
        reader.tables.extend(tables.iter().copied());
        reader.tables.sort_unstable_by_key(|table| table.index());
        reader.tables.dedup();
        if reader.tables.len() == before {
            return false;
        }
        reader.reader.declare_only(&reader.tables);
        true
    }

    fn note(&mut self, name: String, (entries, differing, first): (u64, u64, Option<String>), hour: Hour, why: &str, notes: Value, out: &mut impl Write) -> io::Result<()> {
        let total = self.totals.entry(name.clone()).or_default();
        total.0 += entries;
        total.1 += differing;
        total.2 += 1;
        writeln!(out, "{}", json!({"hour": hour, "why": why, "compare": format!("combat:{name}"), "entries": entries, "differing": differing, "first": first, "notes": notes}))
    }

    pub(crate) fn differing(&self) -> u64 {
        self.totals.iter().filter(|(name, _)| !name.contains(':')).map(|(_, (_, differing, _))| differing).sum()
    }

    pub(crate) fn frame(&mut self, document: &Document, head: Hour) {
        let Ok(vision) = document.hour().and_then(|hour| hour.vision(&Choice::default())).map(Arc::new) else { return };
        let ours = vision.ours();
        let at = &self.views;
        let timing = &mut self.timing;
        if let Some(frame) = timing.battle.frame(&self.views) {
            let held = self.battles.last().copied();
            match largest(&frame, held, head) {
                Some(battle) if held.is_some_and(|held| held.0 == battle.0) => self.battles.last_mut().expect("held").2 = head,
                Some(battle) => {
                    timing.window.open(Selection::Land(battle.0));
                    let targets = targets_at(&frame, battle.0);
                    timing.targets = targets.land.first().map(|&(id, province)| (id, province, targets.divisions.first().copied(), targets.naval.first().map(|&(id, _)| id)));
                    self.battles.push(battle);
                }
                None => {}
            }
        }
        timing.battle.end_frame();
        if timing.targets.is_some()
            && self.timed.contains(&TIMED[0])
            && let Some(frame) = timing.window_rows.frame(at)
        {
            let began = Instant::now();
            let (prepared, missed) = discovering(|| timing.window.prepare(&document.combat, &document.playback.roster_sampler(), frame.branch, frame.hour, Ok(Arc::clone(&frame)), &Choice::default(), None, false));
            if matches!(prepared, Ok(viewer_combat::Prepared::Read(_))) {
                timing.window_reads.push(began.elapsed().as_nanos() as u64);
            }
            match prepared {
                Ok(prepared) if missed.is_empty() => drop(timing.window.take_in(&document.combat, prepared)),
                _ => timing.window_rows.learn(&missed),
            }
        }
        let (started, cycles) = (Instant::now(), replay_frame::cpu::thread_cycles());
        if let Some((combat, province, division, naval)) = timing.targets {
            let rows = &mut timing.rows;
            let asked = &self.timed;
            let mut each = |at: usize, work: &mut dyn FnMut()| {
                if !asked.contains(&TIMED[at]) {
                    return;
                }
                let (began, cycles) = (Instant::now(), replay_frame::cpu::thread_cycles());
                work();
                timing.each[at].push((began.elapsed().as_nanos() as u64, replay_frame::cpu::thread_cycles().saturating_sub(cycles)));
            };
            each(1, &mut || {
                rows.land_marker(at, document, ours, combat, None, false, || {});
            });
            if let Some(naval) = naval {
                each(2, &mut || {
                    rows.naval_marker(at, document, ours, naval, None, || {});
                });
            }
            if let Some(division) = division {
                let base = &timing.base;
                each(3, &mut || {
                    rows.division_panel(at, document, Arc::clone(base), division, || {});
                });
            }
            each(4, &mut || {
                rows.terrain(at, document, province, &vision, || {});
            });
            each(5, &mut || {
                let _ = rows.infrastructure(at, document, province, || {});
            });
        }
        timing.rows.end_frame();
        timing.window_rows.end_frame();
        timing.frames.push((started.elapsed().as_nanos() as u64, replay_frame::cpu::thread_cycles().saturating_sub(cycles)));
        match timing.window.model() {
            Some((hour, _)) => timing.behind.push(f64::from(head - hour)),
            None if timing.window.selection.is_some() => timing.loading += 1,
            None => {}
        }
        drop(timing.window.take_retired());
    }

    pub(crate) fn summary(&self, distribution: fn(Vec<f64>) -> Value) -> Value {
        let held = self.views.held();
        let bytes = |list: &[(TableId, usize)], tables: &[TableId]| list.iter().filter(|(table, _)| tables.contains(table)).map(|(_, bytes)| bytes).sum::<usize>();
        let ms = |values: &[u64]| distribution(values.iter().map(|&ns| ns as f64 / 1e6).collect());
        let hz = self.timing.hz;
        let both = |values: &[(u64, u64)]| json!({"wall": distribution(values.iter().map(|&(ns, _)| ns as f64 / 1e6).collect()), "cpu": distribution(values.iter().map(|&(_, cycles)| cycles as f64 / hz * 1e3).collect())});
        let mut workers = serde_json::Map::new();
        workers.insert("window".into(), ms(&self.timing.window_reads));
        for (name, times) in self.timing.rows.times() {
            workers.insert(name.into(), ms(&times));
        }
        let readers: Vec<Value> = self
            .readers
            .iter()
            .map(|reader| {
                let timed = TIMED.iter().position(|name| *name == reader.name);
                json!({
                    "reader": reader.name, "tables": reader.tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
                    "rows_bytes": bytes(&held, &reader.tables),
                    "frame_thread_ms": timed.map(|at| both(&self.timing.each[at])), "worker_ms": workers.get(reader.name),
                })
            })
            .collect();
        let learned: Vec<String> = self.readers.iter().flat_map(|reader| reader.tables.iter().map(|table| format!("{}: {table:?}", reader.name))).collect();
        json!({
            "compared": self.totals.iter().map(|(name, (entries, differing, samples))| json!({"compare": name, "samples": samples, "entries": entries, "differing": differing})).collect::<Vec<_>>(),
            "readers": readers, "learned": learned,
            "frame_thread_ms": both(&self.timing.frames), "frames": self.timing.frames.len(),
            "window_hours_behind_head": distribution(self.timing.behind.clone()), "window_loading_frames": self.timing.loading,
            "battles": self.battles.iter().map(|&(id, from, to, divisions)| json!({"combat": id, "from": from, "to": to, "divisions": divisions})).collect::<Vec<_>>(),
            "rows_bytes": held.iter().map(|(_, bytes)| bytes).sum::<usize>(),
        })
    }
}

fn largest(frame: &DecodedFrame, held: Option<(u64, Hour, Hour, usize)>, head: Hour) -> Option<(u64, Hour, Hour, usize)> {
    let combats = frame.complete::<tables::LandCombats>().ok()?;
    if let Some(held) = held
        && combats.iter().any(|row| row.id == held.0)
    {
        return Some(held);
    }
    combats.iter().max_by_key(|row| (row.attackers.len() + row.defenders.len(), row.id)).map(|row| (row.id, head, head, row.attackers.len() + row.defenders.len()))
}

fn targets_at(frame: &DecodedFrame, id: u64) -> Targets {
    let mut targets = Targets::default();
    if let Ok(rows) = frame.complete::<tables::LandCombats>()
        && let Some(row) = rows.iter().find(|row| row.id == id)
    {
        targets.land = vec![(row.id, row.province)];
    }
    if let Ok(rows) = frame.complete::<tables::LandCombatDivisions>() {
        targets.divisions = rows.iter().filter(|row| row.combat == id).map(|row| row.division).take(1).collect();
    }
    if let Ok(rows) = frame.complete::<tables::NavalCombats>() {
        targets.naval = rows.first().map(|row| (row.id, row.province)).into_iter().collect();
    }
    targets
}

fn at_hour(reader: &ViewReader, hour: Hour) -> io::Result<Option<Arc<DecodedFrame>>> {
    match reader.wait(|frame| frame.hour >= hour, LIMIT) {
        Ok(frame) => Ok((frame.hour == hour).then_some(frame)),
        Err(error) => Err(io::Error::other(format!("{error} ({hour}); its newest rows are at {:?}", reader.frame().map(|frame| frame.hour)))),
    }
}
