
use super::super::views::{Need, Pace, Reader as ViewReader, Views};
use crate::document::Document;
use replay::campaign::SeekResult;
use replay_data::rows::{Field, List, Opaque, RowRef, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView, discovering, undeclared_reads, view_reads};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, io::Write, sync::Arc, time::Duration};
use viewer_foundation::perspective;

const LIMIT: Duration = Duration::from_secs(120);

struct Set {
    name: &'static str,
    tables: &'static [TableId],
    pace: Pace,
}

const SETS: &[Set] = &[
    Set { name: "units", tables: &[TableId::Divisions, TableId::DivisionStatus, TableId::DivisionMovement, TableId::Ships, TableId::TaskForces, TableId::AirWings], pace: Pace::PerSecond(4) },
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
        pace: Pace::Step,
    },
    Set { name: "world", tables: &[TableId::Countries, TableId::CountryLinks, TableId::HumanControllers, TableId::Provinces, TableId::States, TableId::LexicalTokens], pace: Pace::PerSecond(4) },
    Set { name: "triggers", tables: &[TableId::Countries, TableId::CountryLinks, TableId::Provinces, TableId::States], pace: Pace::Step },
    Set { name: "all", tables: &TableId::ALL, pace: Pace::Step },
];

const COUNTRIES: usize = 4;

pub(crate) struct Line {
    views: Views,
    readers: Vec<(&'static Set, ViewReader)>,
    triggers: Vec<TableId>,
    totals: BTreeMap<String, (u64, u64, u64)>,
    roster: Option<super::roster::RosterLine>,
    window: Option<super::window::WindowLine>,
}

impl Line {
    pub(crate) fn open(views: Views, branch: replay_data::BranchId, names: &str, timing: bool, path: &std::path::Path, content: &content_reader_layers::ResolveOptions) -> io::Result<Self> {
        let mut readers = Vec::new();
        let mut roster = None;
        let mut window = None;
        let mut triggers = SETS.iter().find(|set| set.name == "triggers").map_or_else(Vec::new, |set| set.tables.to_vec());
        for name in names.split(',').filter(|name| !name.is_empty()) {
            let (name, listed) = name.split_once('@').map_or((name, None), |(name, file)| (name, Some(file)));
            if name == "roster" {
                let tag = listed.ok_or_else(|| io::Error::other("The roster set names its country: roster@<TAG>"))?;
                let pace = if timing { Pace::PerSecond(4) } else { Pace::Step };
                roster = Some(super::roster::RosterLine::open(&views, path, branch, tag, content, pace)?);
                continue;
            }
            if name == "window" {
                window = Some(super::window::WindowLine::open(&views, listed.unwrap_or("fit"), if timing { Pace::PerSecond(4) } else { Pace::Step })?);
                continue;
            }
            let set = SETS.iter().find(|set| set.name == name).ok_or_else(|| io::Error::other(format!("No row set {name}; the sets are {:?}", SETS.iter().map(|set| set.name).collect::<Vec<_>>())))?;
            let pace = if timing { set.pace } else { Pace::Step };
            if let Some(file) = listed {
                for line in std::fs::read_to_string(file)?.lines().map(str::trim).filter(|line| !line.is_empty()) {
                    let table = TableId::ALL.into_iter().find(|table| format!("{table:?}") == line).ok_or_else(|| io::Error::other(format!("{file} names no table {line}")))?;
                    if !triggers.contains(&table) {
                        triggers.push(table);
                    }
                }
            }
            let tables = if set.name == "triggers" { triggers.clone() } else { set.tables.to_vec() };
            readers.push((set, views.reader_at(set.name, &tables.iter().copied().map(Need::every).collect::<Vec<Need>>(), pace)));
        }
        Ok(Self { views, readers, triggers, totals: BTreeMap::new(), roster, window })
    }

    pub(crate) fn drive(&mut self, _hour: replay_data::Hour) {
        if let Some(roster) = &mut self.roster {
            roster.poll();
        }
    }

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
            self.readers[at].1.declare_only(&self.triggers);
            let skipped = Compared { notes: json!({"declared": self.triggers.len()}), ..Compared::default() };
            return self.note("triggers:declared".into(), &skipped, hour, why, out);
        }
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
            for &table in &missed {
                if !self.triggers.contains(&table) {
                    self.triggers.push(table);
                }
            }
            self.readers[at].1.declare_only(&self.triggers);
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

    pub(crate) fn summary(&self, distribution: fn(Vec<f64>) -> Value) -> Value {
        let held = self.views.held();
        json!({
            "compared": self.totals.iter().map(|(name, (entries, differing, samples))| json!({"compare": name, "samples": samples, "entries": entries, "differing": differing})).collect::<Vec<_>>(),
            "rows_bytes": held.iter().map(|(_, bytes)| bytes).sum::<usize>(),
            "tables": held.iter().map(|(table, bytes)| json!({"table": format!("{table:?}"), "rows_bytes": bytes})).collect::<Vec<Value>>(),
            "triggers_declared": self.triggers.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
            "undeclared_reads": undeclared_reads(),
            "roster": self.roster.as_ref().map(|roster| roster.summary(distribution)),
            "window_declared": self.window.as_ref().map(|window| window.tables().iter().map(|table| format!("{table:?}")).collect::<Vec<_>>()),
            "window_relaid": self.window.as_ref().map(|window| window.relaid),
            "window_spent": self.window.as_ref().map(|window| window.spent(distribution)),
            "window_reads": self.window.as_ref().map(|window| window.reads().iter().map(|(name, tables)| (name.clone(), tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>())).collect::<BTreeMap<_, _>>()),
        })
    }
}

fn at_hour(reader: &ViewReader, hour: replay_data::Hour) -> io::Result<Option<Arc<DecodedFrame>>> {
    match reader.wait(|frame| frame.hour >= hour, LIMIT) {
        Ok(frame) => Ok((frame.hour == hour).then_some(frame)),
        Err(error) => Err(io::Error::other(format!("{error} ({hour}); its newest rows are at {:?}", reader.frame().map(|frame| frame.hour)))),
    }
}

#[derive(Debug, Default)]
pub(crate) struct Compared {
    pub(crate) entries: u64,
    pub(crate) differing: u64,
    pub(crate) first: Option<String>,
    notes: Value,
}

fn evaluate(document: &Document, frame: &DecodedFrame, ours: u32, countries: &[u32]) -> (Vec<String>, Vec<String>) {
    let products = &document.content.products;
    let world = dashboard_text::triggers::World::read(frame, products.triggers.facts());
    let music = match &products.audio {
        Some(audio) => countries.iter().map(|&country| format!("{country}: {:?}", map_products::music_inputs::Memo::new().chances(frame, audio, country, &world))).collect(),
        None => Vec::new(),
    };
    let assets = &document.icons.raids;
    let paths = viewer_raids::map::raids::paths::HourPaths::new(assets, frame);
    let icons = viewer_raids::map::raids::icons(assets, frame, ours, &paths);
    (music, vec![format!("{:?}", icons.icons), format!("{:?}", icons.report)])
}

pub(crate) fn table_against(head: &DecodedFrame, today: &DecodedFrame, table: TableId) -> Compared {
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
