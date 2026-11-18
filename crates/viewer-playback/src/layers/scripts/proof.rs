//! The proof tool's line for the script-driven readers at the head (`--scripts on|off`): the
//! map view's [`Scripts`] asked every frame as the window asks it; at every comparison, its raid
//! icons with the offer cycle complete and its music chances against today's over today's frame of
//! the same hour; what the frame thread spent asking, what the worker spent evaluating, how old
//! the icons were, what the rows and the playhead held, and every step's build time (`off` keeps
//! only the step times and holdings, the baseline the scripts are judged against).

use super::{Ask, Scripts};
use crate::{
    document::Document,
    playhead_host::{
        Entry,
        rows::{Recording, Rows},
    },
};
use replay::campaign::SeekResult;
use replay_data::Hour;
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{
    io,
    io::Write,
    sync::Arc,
    time::{Duration, Instant},
};
use viewer_foundation::map::perspective;

const LIMIT: Duration = Duration::from_secs(120);

pub struct Line {
    entry: Arc<Entry>,
    recording: Recording,
    scripts: Option<Scripts>,
    rows: Option<Arc<Rows>>,
    consumer: playhead::Consumer,
    reader: playhead::Reader,
    steps: Vec<Arc<playhead::Step>>,
    /// Every step's build nanoseconds, and resets apart.
    step_ns: Vec<u64>,
    /// Per frame: the frame thread's nanoseconds asking the scripts and taking their results.
    ask_ns: Vec<u64>,
    /// Per frame while playing: hours from the icons on screen to the head.
    age: Vec<f64>,
    /// The most the rows' worker held, mirrors and rows apart, and by table at the most.
    held: (usize, usize),
    by_table: Vec<(replay_data::rows::TableId, usize, usize)>,
    /// Per comparison name: entries, differing, samples.
    totals: std::collections::BTreeMap<String, (u64, u64, u64)>,
}

impl Line {
    /// `on` the scripts, declaring the tables `primed` names (a file, one a line) from the start.
    pub fn open(entry: &Arc<Entry>, recording: Recording, on: bool, primed: Option<&str>) -> io::Result<Self> {
        let mut tables = Vec::new();
        if let Some(file) = primed {
            for line in std::fs::read_to_string(file)?.lines().map(str::trim).filter(|line| !line.is_empty()) {
                tables.push(need(line).ok_or_else(|| io::Error::other(format!("{file}: {line} names no table and fields of it")))?);
            }
        }
        let reader = entry.playhead().declare(Vec::new(), None)?;
        let consumer = entry.playhead().consumer(reader);
        let scripts = on.then(|| {
            let scripts = Scripts::primed(&tables);
            scripts.time();
            scripts
        });
        let rows = if on { Some(Rows::of(entry, recording)?) } else { None };
        Ok(Self { entry: Arc::clone(entry), recording, scripts, rows, consumer, reader, steps: Vec::new(), step_ns: Vec::new(), ask_ns: Vec::new(), age: Vec::new(), held: (0, 0), by_table: Vec::new(), totals: Default::default() })
    }

    fn ask(&mut self, document: &Document, playing: bool) {
        let Some(scripts) = &mut self.scripts else { return };
        let began = Instant::now();
        scripts.show(Ask { entry: &self.entry, recording: self.recording, sources: super::Sources::of(&document.content), perspective: &perspective::Choice::default(), playing, raids: true, music: true }, || {});
        let raids = scripts.raids(self.recording.branch, u32::MAX);
        let music = scripts.music();
        scripts.end_frame();
        self.ask_ns.push(began.elapsed().as_nanos() as u64);
        drop((raids, music));
    }

    /// One frame of the map at `head`: the scripts asked as the window asks them, the steps
    /// reached taken and timed, the icons' age noted.
    pub fn frame(&mut self, document: &Document, head: Hour, playing: bool) {
        if let Some(rows) = &self.rows {
            rows.drive(head);
        }
        self.ask(document, playing);
        self.steps.clear();
        self.consumer.take(head, &mut self.steps);
        self.step_ns.extend(self.steps.iter().filter(|step| !step.reset()).map(|step| step.built().ns));
        if let Some(scripts) = &self.scripts
            && playing
            && let Some(hour) = scripts.newest_hour()
        {
            self.age.push(f64::from(head - hour));
        }
        if let Some(rows) = &self.rows {
            let held = rows.held();
            let mirrors: usize = held.mirrors.iter().map(|(_, bytes)| bytes).sum();
            let made: usize = held.rows.iter().map(|(_, bytes)| bytes).sum();
            if mirrors + made > self.held.0 + self.held.1 {
                self.by_table = held.mirrors.iter().map(|&(table, bytes)| (table, bytes, held.rows.iter().find(|(made, _)| *made == table).map_or(0, |(_, bytes)| *bytes))).collect();
                self.by_table.sort_by_key(|(_, mirror, rows)| std::cmp::Reverse(mirror + rows));
            }
            self.held.0 = self.held.0.max(mirrors);
            self.held.1 = self.held.1.max(made);
        }
    }

    /// The raid icons with the cycle complete and the music chances at `hour` against today's;
    /// skipped (and noted) while the scripts' tables are still arriving.
    pub fn compare(&mut self, document: &Document, hour: Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        if self.scripts.is_none() {
            return Ok(());
        }
        let ours = document.hour()?.vision(&perspective::Choice::default())?.ours();
        let today: Arc<DecodedFrame> = match document.playback.replay().seek_selected(document.playback.branch(), hour, None)? {
            SeekResult::Frame(frame) => Arc::new(*frame),
            SeekResult::Gap { reason, .. } => return Err(io::Error::other(format!("Today's reader finds a gap at {hour}: {reason}"))),
        };
        let until = Instant::now() + LIMIT;
        let learned = self.scripts.as_ref().map(Scripts::needs).unwrap_or_default();
        let reached = loop {
            self.ask(document, false);
            let scripts = self.scripts.as_ref().expect("checked above");
            let raids = scripts.raids(self.recording.branch, ours).filter(|icons| icons.built_for.is_some_and(|(_, at, _)| at == hour) && icons.report.cycle_complete());
            let music = scripts.music_at().filter(|(at, _)| *at == hour);
            if let (Some(raids), Some((_, music))) = (raids, music) {
                break Some((raids, music));
            }
            // A table learned now joins past the head this compare holds: it arrives only later.
            if Instant::now() >= until || scripts.newest_hour().is_some_and(|at| at > hour) || scripts.needs() != learned {
                break None;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let tables = self.scripts.as_ref().map_or(0, |scripts| scripts.tables().len());
        let Some((raids, music)) = reached else {
            return self.note("scripts:loading".into(), (0, 0, None), json!({"tables": tables}), hour, why, out);
        };
        let content = &document.content;
        let wanted_raids = {
            let assets = &document.icons.raids;
            let paths = viewer_raids::map::raids::paths::HourPaths::new(assets, &today);
            viewer_raids::map::raids::icons(assets, &today, ours, &paths)
        };
        let wanted_music = content.products.audio.as_ref().map(|audio| {
            let world = dashboard_text::triggers::World::read(&*today, content.products.triggers.facts());
            map_products::music_inputs::Memo::new().chances(&today, audio, ours, &world)
        });
        let today_refusals: Vec<String> = wanted_music.as_ref().map(|music| music.refusals.clone()).unwrap_or_default();
        let head_only: Vec<&String> = music.refusals.iter().filter(|refusal| !today_refusals.contains(refusal)).take(4).collect();
        let pairs = [
            ("scripts:raids:icons", format!("{:?}", wanted_raids.icons), format!("{:?}", raids.icons)),
            ("scripts:raids:report", format!("{:?}", wanted_raids.report), format!("{:?}", raids.report)),
            ("scripts:music", format!("{:?}", wanted_music.map(|music| (music.weights, music.refusals))), format!("{:?}", Some((music.weights.clone(), music.refusals.clone())))),
        ];
        for (name, wanted, got) in pairs {
            let differs = wanted != got;
            let first = differs.then(|| first_difference(&wanted, &got));
            let entries = if name == "scripts:raids:icons" { wanted_raids.icons.len().max(1) as u64 } else { 1 };
            let notes = if name == "scripts:music" && differs { json!({"tables": tables, "refused_at_the_head_only": head_only}) } else { json!({"tables": tables}) };
            self.note(name.into(), (entries, u64::from(differs), first), notes, hour, why, out)?;
        }
        Ok(())
    }

    fn note(&mut self, name: String, (entries, differing, first): (u64, u64, Option<String>), notes: Value, hour: Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        let total = self.totals.entry(name.clone()).or_default();
        total.0 += entries;
        total.1 += differing;
        total.2 += 1;
        writeln!(out, "{}", json!({"hour": hour, "why": why, "compare": name, "entries": entries, "differing": differing, "first": first, "notes": notes}))
    }

    pub fn differing(&self) -> u64 {
        self.totals.values().map(|(_, differing, _)| differing).sum()
    }

    /// The scripts reader's raid arrow notes for canonical viewing country `ours`; none without a reader.
    pub fn raid_notes(&self, ours: u32) -> Option<Arc<Vec<String>>> {
        self.scripts.as_ref()?.raid_notes(ours)
    }

    pub fn summary(&self, distribution: fn(Vec<f64>) -> Value) -> Value {
        let ms = |values: &[u64]| distribution(values.iter().map(|&ns| ns as f64 / 1e6).collect());
        let timings = self.scripts.as_ref().and_then(Scripts::timings).unwrap_or_default();
        let batches: Vec<u64> = timings.frames.iter().filter_map(|frame| frame.1).collect();
        let music: Vec<u64> = timings.frames.iter().filter_map(|frame| frame.2).collect();
        json!({
            "on": self.scripts.is_some(),
            "compared": self.totals.iter().map(|(name, (entries, differing, samples))| json!({"compare": name, "samples": samples, "entries": entries, "differing": differing})).collect::<Vec<_>>(),
            "step_ms": ms(&self.step_ns), "steps": self.step_ns.len(),
            "ask_ms": ms(&self.ask_ns), "frames": self.ask_ns.len(),
            "icon_age_hours": distribution(self.age.clone()),
            "frames_evaluated": timings.frames.len(), "skipped": timings.skipped, "learned": timings.learned, "restarts": timings.restarts, "ran_on": timings.ran_on,
            "raid_batch_ms": ms(&batches), "paused_batch_ms": ms(&timings.paused), "music_ms": ms(&music),
            "rows_mirrors_peak_bytes": self.held.0, "rows_rows_peak_bytes": self.held.1,
            "held_by_table_at_peak": self.by_table.iter().map(|(table, mirror, rows)| json!({"table": format!("{table:?}"), "mirror_bytes": mirror, "rows_bytes": rows})).collect::<Vec<_>>(),
            "tables": self.scripts.as_ref().map(|scripts| scripts.tables().iter().map(|table| format!("{table:?}")).collect::<Vec<_>>()),
            "needs": self.scripts.as_ref().map(|scripts| scripts.needs().iter().map(|need| json!({"table": format!("{:?}", need.table), "every": need.columns == u64::MAX, "fields": (0..64).filter(|bit| need.columns & (1 << bit) != 0).filter_map(|bit| need.table.field_name(bit)).collect::<Vec<_>>()})).collect::<Vec<_>>()),
            "outside_reads": replay_frame::frame::outside_reads(),
        })
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        self.scripts = None;
        self.entry.playhead().retire(self.reader);
    }
}

/// A line of a primed file: `Table` (its keys), `Table *` (every field) or `Table a,b` (those
/// fields, by their schema names).
fn need(line: &str) -> Option<crate::playhead_host::rows::Need> {
    use crate::playhead_host::rows::Need;
    let (name, fields) = line.split_once(' ').map_or((line, None), |(name, fields)| (name, Some(fields.trim())));
    let table = replay_data::rows::TableId::ALL.into_iter().find(|table| format!("{table:?}") == name)?;
    match fields {
        None => Some(Need::keys(table)),
        Some("*") => Some(Need::every(table)),
        Some(fields) => fields.split(',').map(|field| (0..64).find(|&bit| table.field_name(bit) == Some(field.trim())).map(|bit| 1_u64 << bit)).sum::<Option<u64>>().map(|mask| Need::columns(table, mask)),
    }
}

/// Where two texts first differ, with what each holds around it.
fn first_difference(wanted: &str, got: &str) -> String {
    let at = wanted.bytes().zip(got.bytes()).position(|(a, b)| a != b).unwrap_or(wanted.len().min(got.len()));
    let around = |text: &str| {
        let start = text.char_indices().map(|(at, _)| at).take_while(|&from| from + 160 <= at).last().unwrap_or(0);
        text[start..].chars().take(400).collect::<String>()
    };
    format!("at byte {at} of {} / {}: today {} / at the head {}", wanted.len(), got.len(), around(wanted), around(got))
}
