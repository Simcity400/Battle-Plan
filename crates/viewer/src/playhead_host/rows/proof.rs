//! The proof tool's line for rows at the head (`--rows units,combat,world,triggers`): one reader
//! per table set, driven to the proof's head; at every comparison each table's rows against today's
//! frame of the same hour, row by row with its status, and the trigger world over them against
//! today's (music chances and raid icons); per table what making its rows cost.

use super::{Cadence, Recording, RowReader, Rows};
use crate::{document::Document, playhead_host::Entry};
use replay::campaign::SeekResult;
use replay_data::rows::{Field, List, Opaque, RowRef, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView, discovering, undeclared_reads, view_reads};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, io::Write, sync::Arc, time::Duration};
use viewer_foundation::map::perspective;

const LIMIT: Duration = Duration::from_secs(120);

/// A set of tables one reader declares, and the cadence it reads them at while timed.
struct Set {
    name: &'static str,
    tables: &'static [TableId],
    cadence: Cadence,
}

const SETS: &[Set] = &[
    Set { name: "units", tables: &[TableId::Divisions, TableId::DivisionStatus, TableId::DivisionMovement, TableId::Ships, TableId::TaskForces, TableId::AirWings], cadence: Cadence::PerSecond(4) },
    Set {
        name: "combat",
        tables: &[
            TableId::LandCombats,
            TableId::LandCombatDetails,
            TableId::LandCombatDivisions,
            TableId::LandCombatModifiers,
            TableId::LandCombatSides,
            TableId::LandCombatDivisionModifiers,
        ],
        cadence: Cadence::EveryStep,
    },
    Set { name: "world", tables: &[TableId::Countries, TableId::CountryLinks, TableId::HumanControllers, TableId::Provinces, TableId::States, TableId::LexicalTokens], cadence: Cadence::PerSecond(4) },
    Set { name: "triggers", tables: &[TableId::Countries, TableId::CountryLinks, TableId::Provinces, TableId::States], cadence: Cadence::EveryStep },
    Set { name: "all", tables: &TableId::ALL, cadence: Cadence::EveryStep },
];

/// Countries whose music chances are compared besides the viewing one.
const COUNTRIES: usize = 4;

pub(crate) struct Line {
    rows: Arc<Rows>,
    readers: Vec<(&'static Set, RowReader)>,
    /// The tables the trigger reader declares: its core, and every table today's evaluation read.
    triggers: Vec<TableId>,
    /// Per comparison name: entries, differing, samples.
    totals: BTreeMap<String, (u64, u64, u64)>,
    /// `roster@<TAG>`: the template roster as a panel.
    roster: Option<super::roster::RosterLine>,
    /// `window@<x,z,height>`: the window's selection observers, overlay inputs and clicks.
    window: Option<super::window::WindowLine>,
}

impl Line {
    /// Readers of the sets named (comma separated) on `entry`'s playhead.
    /// `path` and `content` are the recording's and the content folders, for the roster panel.
    pub(crate) fn open(entry: &Arc<Entry>, recording: Recording, names: &str, timing: bool, path: &std::path::Path, content: &installed_content_core::ResolveOptions) -> io::Result<Self> {
        let rows = Rows::of(entry, recording)?;
        rows.time();
        let mut readers = Vec::new();
        let mut roster = None;
        let mut window = None;
        let mut triggers = SETS.iter().find(|set| set.name == "triggers").map_or_else(Vec::new, |set| set.tables.to_vec());
        for name in names.split(',').filter(|name| !name.is_empty()) {
            // `triggers@<file>`: the trigger reader also declares the tables the file names, one a line.
            let (name, listed) = name.split_once('@').map_or((name, None), |(name, file)| (name, Some(file)));
            if name == "roster" {
                let tag = listed.ok_or_else(|| io::Error::other("The roster set names its country: roster@<TAG>"))?;
                let cadence = if timing { Cadence::PerSecond(4) } else { Cadence::EveryStep };
                roster = Some(super::roster::RosterLine::open(&rows, path, recording.branch, tag, content, cadence)?);
                continue;
            }
            if name == "window" {
                window = Some(super::window::WindowLine::open(&rows, listed.unwrap_or("fit"), if timing { Cadence::PerSecond(4) } else { Cadence::EveryStep })?);
                continue;
            }
            let set = SETS.iter().find(|set| set.name == name).ok_or_else(|| io::Error::other(format!("No row set {name}; the sets are {:?}", SETS.iter().map(|set| set.name).collect::<Vec<_>>())))?;
            let cadence = if timing { set.cadence } else { Cadence::EveryStep };
            if let Some(file) = listed {
                for line in std::fs::read_to_string(file)?.lines().map(str::trim).filter(|line| !line.is_empty()) {
                    let table = TableId::ALL.into_iter().find(|table| format!("{table:?}") == line).ok_or_else(|| io::Error::other(format!("{file} names no table {line}")))?;
                    if !triggers.contains(&table) {
                        triggers.push(table);
                    }
                }
            }
            let tables = if set.name == "triggers" { triggers.clone() } else { set.tables.to_vec() };
            readers.push((set, rows.reader(&tables, cadence)));
        }
        rows.settled();
        Ok(Self { rows, readers, triggers, totals: BTreeMap::new(), roster, window })
    }

    pub(crate) fn drive(&mut self, hour: replay_data::Hour) {
        self.rows.drive(hour);
        if let Some(roster) = &mut self.roster {
            roster.poll();
        }
    }

    /// Every set's rows at `hour` against today's reader of every table at that hour; each set's
    /// wait for its rows at the hour, timed from the call (the map has shown the hour by then).
    pub(crate) fn compare(&mut self, document: &Document, hour: replay_data::Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        let called = std::time::Instant::now();
        let mut frames = Vec::with_capacity(self.readers.len());
        for (set, reader) in &self.readers {
            frames.push(if set.name == "triggers" { None } else { Some((at_hour(reader, hour)?, called.elapsed().as_secs_f64() * 1e3)) });
        }
        if self.roster.is_some() {
            let frame = at_hour(&self.roster.as_ref().expect("roster").reader, hour)?;
            let compared = match frame {
                Some(frame) => {
                    let (entries, differing, first) = self.roster.as_mut().expect("roster").compare(&frame, hour);
                    Compared { entries, differing, first, notes: json!({}) }
                }
                None => Compared::default(),
            };
            self.note("roster:text".into(), &compared, hour, why, out)?;
        }
        let today: Arc<DecodedFrame> = match document.playback.replay().seek_selected(document.playback.branch(), hour, None)? {
            SeekResult::Frame(frame) => Arc::new(*frame),
            SeekResult::Gap { reason, .. } => return Err(io::Error::other(format!("Today's reader finds a gap at {hour}: {reason}"))),
        };
        if self.window.is_some() {
            self.window(document, &today, hour, why, out)?;
        }
        for (at, frame) in frames.into_iter().enumerate() {
            let set = self.readers[at].0;
            let Some((frame, waited_ms)) = frame else {
                self.triggers(document, &today, hour, why, out)?;
                continue;
            };
            let Some(frame) = frame else {
                self.note(format!("{}:ahead", set.name), &Compared::default(), hour, why, out)?;
                continue;
            };
            for &table in set.tables {
                let mut compared = table_against(&frame, &today, table);
                compared.notes["waited_ms"] = json!(waited_ms);
                self.note(format!("{}:{table:?}", set.name), &compared, hour, why, out)?;
            }
        }
        Ok(())
    }

    /// The music chances of the viewing country and a few others, and the viewing country's raid
    /// icons, over rows at the head against today's frame; the tables today's evaluation read are
    /// declared first.
    fn triggers(&mut self, document: &Document, today: &Arc<DecodedFrame>, hour: replay_data::Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        let at = self.readers.iter().position(|(set, _)| set.name == "triggers").expect("the triggers set");
        let ours = document.hour()?.vision(&perspective::Choice::default())?.ours();
        let mut countries = vec![ours];
        if let Ok(links) = today.complete::<replay_data::rows::tables::CountryLinks>() {
            countries.extend(links.iter().map(|link| link.canonical_slot).filter(|&slot| slot != ours).take(COUNTRIES));
        }
        let (wanted, reads) = view_reads::record(|| evaluate(document, today, ours, &countries));
        let read: Vec<TableId> = TableId::ALL.into_iter().filter(|table| !today.tables[table.index()].is_not_selected() && reads.iter().any(|identity| identity.is(&today.tables[table.index()]))).collect();
        let before = self.triggers.len();
        for table in read {
            if !self.triggers.contains(&table) {
                self.triggers.push(table);
            }
        }
        if self.triggers.len() != before {
            // The tables join at a step past the head, which waits here: compared from the next sample.
            self.readers[at].1.declare(&self.triggers);
            self.rows.settled();
            let skipped = Compared { notes: json!({"declared": self.triggers.len()}), ..Compared::default() };
            return self.note("triggers:declared".into(), &skipped, hour, why, out);
        }
        // Tables declared since join at the playhead's newest step, past this head: not waited for.
        if self.readers[at].1.frame().is_none() {
            return self.note("triggers:loading".into(), &Compared::default(), hour, why, out);
        }
        let Some(frame) = at_hour(&self.readers[at].1, hour)? else {
            return self.note("triggers:ahead".into(), &Compared::default(), hour, why, out);
        };
        let undeclared = undeclared_reads();
        let (got, missed) = discovering(|| evaluate(document, &frame, ours, &countries));
        let undeclared = undeclared_reads() - undeclared;
        if !missed.is_empty() {
            // What the scripts read at the head names what today's reads did not: declared, as a
            // script reader at the head declares, and compared from the next sample.
            for &table in &missed {
                if !self.triggers.contains(&table) {
                    self.triggers.push(table);
                }
            }
            self.readers[at].1.declare(&self.triggers);
            self.rows.settled();
            let skipped = Compared { notes: json!({"declared": self.triggers.len(), "named_at_the_head": missed.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>()}), ..Compared::default() };
            return self.note("triggers:declared".into(), &skipped, hour, why, out);
        }
        for table in self.triggers.clone() {
            let compared = table_against(&frame, today, table);
            self.note(format!("triggers:{table:?}"), &compared, hour, why, out)?;
        }
        for (name, (wanted, got)) in [("music", (&wanted.0, &got.0)), ("raids", (&wanted.1, &got.1))] {
            let differing = wanted.iter().zip(got).filter(|(wanted, got)| wanted != got).count() as u64;
            let first = wanted.iter().zip(got).find(|(wanted, got)| wanted != got).map(|(wanted, got)| first_difference(wanted, got));
            let compared = Compared { entries: wanted.len() as u64, differing, first, notes: json!({"undeclared_reads": undeclared, "undeclared": missed.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(), "tables": self.triggers.len()}) };
            self.note(format!("triggers:{name}"), &compared, hour, why, out)?;
        }
        Ok(())
    }

    /// The window's readers over rows at the head against over today's frame; a sample whose rows
    /// lacked a table the readers read declares it and is compared from the next.
    fn window(&mut self, document: &Document, today: &Arc<DecodedFrame>, hour: replay_data::Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        let window = self.window.as_mut().expect("the window set");
        if window.reader.frame().is_none() {
            return self.note("window:loading".into(), &Compared::default(), hour, why, out);
        }
        let Some(frame) = at_hour(&window.reader, hour)? else {
            return self.note("window:ahead".into(), &Compared::default(), hour, why, out);
        };
        let compared = window.compare(document, today, &frame)?;
        let declared = window.tables().len();
        match compared {
            Err(missed) => {
                self.rows.settled();
                let skipped = Compared { notes: json!({"declared": declared, "named_at_the_head": missed.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>()}), ..Compared::default() };
                self.note("window:declared".into(), &skipped, hour, why, out)
            }
            Ok(outputs) => {
                for output in outputs {
                    let compared = Compared { entries: output.entries, differing: output.differing, first: output.first, notes: json!({"tables": declared}) };
                    self.note(format!("window:{}", output.name), &compared, hour, why, out)?;
                }
                Ok(())
            }
        }
    }

    fn note(&mut self, name: String, compared: &Compared, hour: replay_data::Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        let total = self.totals.entry(name.clone()).or_default();
        total.0 += compared.entries;
        total.1 += compared.differing;
        total.2 += 1;
        writeln!(out, "{}", json!({"hour": hour, "why": why, "compare": name, "entries": compared.entries, "differing": compared.differing, "first": compared.first, "notes": compared.notes}))
    }

    pub(crate) fn differing(&self) -> u64 {
        self.totals.values().map(|(_, differing, _)| differing).sum()
    }

    /// What was compared, and per table what making its rows cost: build milliseconds, rows,
    /// bytes held, and the steps that changed it.
    pub(crate) fn summary(&self, distribution: fn(Vec<f64>) -> Value) -> Value {
        let timings = self.rows.timings().unwrap_or_default();
        let held = self.rows.held();
        let steps = timings.steps.max(1);
        let tables: Vec<Value> = timings
            .builds
            .iter()
            .map(|(&table, builds)| {
                let id = TableId::ALL[table];
                let bytes = |list: &[(TableId, usize)]| list.iter().find(|(held, _)| *held == id).map(|(_, bytes)| *bytes);
                let changed = timings.changed.get(&table).copied().unwrap_or(0);
                json!({
                    "table": format!("{id:?}"), "builds": builds.len(),
                    "build_ms": distribution(builds.iter().map(|&(ns, _)| ns as f64 / 1e6).collect()),
                    "rows": distribution(builds.iter().map(|&(_, rows)| rows as f64).collect()),
                    "ns_per_row": distribution(builds.iter().filter(|(_, rows)| *rows > 0).map(|&(ns, rows)| ns as f64 / rows as f64).collect()),
                    "rows_bytes": bytes(&held.rows), "mirror_bytes": bytes(&held.mirrors),
                    "changed_steps": changed, "changed_share": changed as f64 / steps as f64,
                })
            })
            .collect();
        json!({
            "compared": self.totals.iter().map(|(name, (entries, differing, samples))| json!({"compare": name, "samples": samples, "entries": entries, "differing": differing})).collect::<Vec<_>>(),
            "steps": timings.steps, "landed": timings.landed, "resets": timings.resets, "published": timings.published, "untold_text_cells": timings.untold, "built_on": timings.built_on,
            "apply_ms": distribution(timings.apply.iter().map(|&ns| ns as f64 / 1e6).collect()),
            "mirrors_bytes": held.mirrors.iter().map(|(_, bytes)| bytes).sum::<usize>(), "rows_bytes": held.rows.iter().map(|(_, bytes)| bytes).sum::<usize>(),
            "tables": tables, "triggers_declared": self.triggers.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
            "undeclared_reads": undeclared_reads(), "error": self.rows.error(),
            "roster": self.roster.as_ref().map(|roster| roster.summary(distribution)),
            "window_declared": self.window.as_ref().map(|window| window.tables().iter().map(|table| format!("{table:?}")).collect::<Vec<_>>()),
            "window_relaid": self.window.as_ref().map(|window| window.relaid),
            "window_spent": self.window.as_ref().map(|window| window.spent(distribution)),
            "window_reads": self.window.as_ref().map(|window| window.reads().iter().map(|(name, tables)| (name.clone(), tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>())).collect::<BTreeMap<_, _>>()),
        })
    }
}

/// The reader's frame at `hour`; none when its rows are already past it.
fn at_hour(reader: &RowReader, hour: replay_data::Hour) -> io::Result<Option<Arc<DecodedFrame>>> {
    match reader.wait(|frame| frame.hour >= hour, LIMIT) {
        Ok(frame) => Ok((frame.hour == hour).then_some(frame)),
        Err(error) => Err(io::Error::other(format!("{error} ({hour}); its newest rows are at {:?}", reader.frame().map(|frame| frame.hour)))),
    }
}

#[derive(Debug, Default)]
pub(super) struct Compared {
    pub(super) entries: u64,
    pub(super) differing: u64,
    pub(super) first: Option<String>,
    notes: Value,
}

/// The music chances of each of `countries` and the raid icons of `ours`, each as its text.
fn evaluate(document: &Document, frame: &DecodedFrame, ours: u32, countries: &[u32]) -> (Vec<String>, Vec<String>) {
    let products = &document.content.products;
    let world = dashboard::triggers::World::read(frame, products.triggers.facts());
    let music = match &products.audio {
        Some(audio) => countries.iter().map(|&country| format!("{country}: {:?}", map_products::music_inputs::Memo::new().chances(frame, audio, country, &world))).collect(),
        None => Vec::new(),
    };
    let assets = &document.icons.raids;
    let paths = viewer_raids::map::raids::paths::HourPaths::new(assets, frame);
    let icons = viewer_raids::map::raids::icons(assets, frame, ours, &paths);
    (music, vec![format!("{:?}", icons.icons), format!("{:?}", icons.report)])
}

/// One table at the head against today's: rows by key, every field, and the status.
pub(super) fn table_against(head: &DecodedFrame, today: &DecodedFrame, table: TableId) -> Compared {
    let (got, wanted) = (&*head.tables[table.index()], &*today.tables[table.index()]);
    let sampled = (head.tables[table.index()].sampled_at(), today.tables[table.index()].sampled_at());
    let notes = json!({"status": status(wanted), "sampled_differs": sampled.0 != sampled.1});
    let rows = |view: &TableView| match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => Some(rows.len()),
        _ => None,
    };
    if got == wanted {
        return Compared { entries: rows(wanted).unwrap_or(1) as u64, differing: 0, first: None, notes };
    }
    let (TableView::Complete(got_rows) | TableView::Truncated { rows: got_rows, .. }, TableView::Complete(wanted_rows) | TableView::Truncated { rows: wanted_rows, .. }) = (got, wanted) else {
        return Compared { entries: 1, differing: 1, first: Some(format!("status: at the head {}, today {}", status(got), status(wanted))), notes };
    };
    let (differing, first) = rows_differing(got_rows, wanted_rows);
    let status_differs = std::mem::discriminant(got) != std::mem::discriminant(wanted) || status(got) != status(wanted);
    let first = if status_differs { Some(format!("status: at the head {}, today {}", status(got), status(wanted))) } else { first };
    Compared { entries: wanted_rows.len().max(got_rows.len()) as u64, differing: differing + u64::from(status_differs), first, notes }
}

fn status(view: &TableView) -> String {
    match view {
        TableView::Complete(_) => "complete".into(),
        TableView::Truncated { missing, .. } => format!("truncated, {missing} missing"),
        TableView::Unavailable { reason } => format!("unavailable: {reason}"),
        TableView::NotRecorded { mode } => format!("not recorded ({mode})"),
    }
}

/// Rows present in one only or differing in a field, and the first such.
fn rows_differing(got: &TableRows, wanted: &TableRows) -> (u64, Option<String>) {
    if got.table() != wanted.table() {
        return (1, Some("the two hold rows of different tables".into()));
    }
    let fields = |row: RowRef<'_>| {
        let mut out = Vec::new();
        let _ = row.visit_wire(&mut |field, value| {
            out.push(format!("{field}={}", describe(value)));
            Ok(())
        });
        out
    };
    let (mut got, mut wanted) = (got.iter().peekable(), wanted.iter().peekable());
    let (mut differing, mut first) = (0_u64, None);
    while got.peek().is_some() || wanted.peek().is_some() {
        let (key_got, key_wanted) = (got.peek().map(RowRef::key), wanted.peek().map(RowRef::key));
        let note = match (key_got, key_wanted) {
            (Some(a), Some(b)) if a == b => {
                let (left, right) = (fields(got.next().unwrap()), fields(wanted.next().unwrap()));
                left.iter().zip(&right).find(|(left, right)| left != right)
                    .map(|(left, right)| format!("key {a}: at the head {left}, today {right}"))
            }
            (Some(a), b) if b.is_none_or(|b| a < b) => {
                got.next();
                Some(format!("key {a} only at the head"))
            }
            (_, Some(b)) => {
                wanted.next();
                Some(format!("key {b} only today"))
            }
            _ => None,
        };
        if let Some(note) = note {
            differing += 1;
            first.get_or_insert(note);
        }
    }
    (differing, first)
}

/// A wire field as text, for the first difference a comparison names.
fn describe(field: Field<'_>) -> String {
    match field {
        Field::Empty => "empty".into(),
        Field::NotRecorded => "not recorded".into(),
        Field::U8(value) => value.to_string(),
        Field::I8(value) => value.to_string(),
        Field::U16(value) => value.to_string(),
        Field::I16(value) => value.to_string(),
        Field::U32(value) => value.to_string(),
        Field::I32(value) => value.to_string(),
        Field::U64(value) => value.to_string(),
        Field::I64(value) => value.to_string(),
        Field::Bool(value) => value.to_string(),
        Field::OptionalI32(value) => format!("{value:?}"),
        Field::OptionalU32(value) => format!("{value:?}"),
        Field::OptionalI64(value) => format!("{value:?}"),
        Field::OptionalU64(value) => format!("{value:?}"),
        Field::Bytes(bytes) => format!("{:?}", String::from_utf8_lossy(bytes)),
        Field::U32List(List::Native(items)) => format!("{items:?}"),
        Field::U64List(List::Native(items)) => format!("{items:?}"),
        Field::U32List(List::Encoded(bytes)) | Field::U64List(List::Encoded(bytes)) => format!("list bytes {bytes:?}"),
        Field::Opaque(Opaque::Encoded(bytes, _)) => format!("opaque {bytes:?}"),
        Field::Opaque(Opaque::Native(_)) => "opaque value".into(),
    }
}

/// Where two texts first differ, with what each holds around it.
pub(super) fn first_difference(wanted: &str, got: &str) -> String {
    let at = wanted.bytes().zip(got.bytes()).position(|(a, b)| a != b).unwrap_or(wanted.len().min(got.len()));
    let around = |text: &str| {
        let start = text.char_indices().map(|(at, _)| at).take_while(|&from| from + 160 <= at).last().unwrap_or(0);
        text[start..].chars().take(400).collect::<String>()
    };
    format!("at byte {at} of {} / {}: today {} / at the head {}", wanted.len(), got.len(), around(wanted), around(got))
}

#[cfg(test)]
mod tests;
