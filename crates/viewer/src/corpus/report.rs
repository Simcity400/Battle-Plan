
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs, io,
    path::Path,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    ReaderError,
    Unsettled,
    Misread,
    ParseFailure,
    Unsupported,
    Refused,
    Ambiguous,
    Fallback,
    Logged,
    Warning,
    Content,
    Offline,
}

impl From<content_reader_parser::DiagnosticKind> for Kind {
    fn from(kind: content_reader_parser::DiagnosticKind) -> Self {
        match kind {
            content_reader_parser::DiagnosticKind::ParseFailure => Self::ParseFailure,
            content_reader_parser::DiagnosticKind::Misread => Self::Misread,
            content_reader_parser::DiagnosticKind::Unsettled => Self::Unsettled,
            content_reader_parser::DiagnosticKind::Unsupported => Self::Unsupported,
            content_reader_parser::DiagnosticKind::Refused => Self::Refused,
            content_reader_parser::DiagnosticKind::Ambiguous => Self::Ambiguous,
            content_reader_parser::DiagnosticKind::Fallback => Self::Fallback,
            content_reader_parser::DiagnosticKind::Logged => Self::Logged,
            content_reader_parser::DiagnosticKind::Warning => Self::Warning,
        }
    }
}

impl From<dashboard_core::findings::Kind> for Kind {
    fn from(kind: dashboard_core::findings::Kind) -> Self {
        use dashboard_core::findings::Kind as ProductKind;
        match kind {
            ProductKind::ParseFailure => Self::ParseFailure,
            ProductKind::Misread => Self::Misread,
            ProductKind::Unsettled => Self::Unsettled,
            ProductKind::Logged => Self::Logged,
            ProductKind::Refused => Self::Refused,
            ProductKind::Unsupported => Self::Unsupported,
            ProductKind::Ambiguous => Self::Ambiguous,
            ProductKind::Fallback => Self::Fallback,
            ProductKind::Warning => Self::Warning,
            ProductKind::Content => Self::Content,
            ProductKind::Offline => Self::Offline,
        }
    }
}

impl Kind {
    pub const ALL: [Kind; 12] = [
        Kind::ReaderError,
        Kind::Unsettled,
        Kind::Misread,
        Kind::ParseFailure,
        Kind::Unsupported,
        Kind::Refused,
        Kind::Ambiguous,
        Kind::Fallback,
        Kind::Logged,
        Kind::Warning,
        Kind::Content,
        Kind::Offline,
    ];

    fn label(self) -> &'static str {
        match self {
            Kind::ReaderError => "error",
            Kind::Unsettled => "unsettled",
            Kind::Misread => "misread",
            Kind::ParseFailure => "parse",
            Kind::Unsupported => "unsupported",
            Kind::Refused => "refused",
            Kind::Ambiguous => "ambiguous",
            Kind::Fallback => "fallback",
            Kind::Logged => "logged",
            Kind::Warning => "warning",
            Kind::Content => "content",
            Kind::Offline => "offline",
        }
    }

    fn actionable(self) -> bool {
        !matches!(self, Self::Content | Self::Offline)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Diagnostic {
    pub reader: String,
    pub kind: Kind,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub cause: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

impl Diagnostic {
    fn occurrences(&self) -> usize {
        self.count.unwrap_or(1)
    }

    fn details(&self) -> String {
        match self.count {
            Some(count) => format!(
                "{}; {count} items; examples: {}",
                self.message,
                self.examples.join(", ")
            ),
            None => self.message.clone(),
        }
    }

    pub fn new(reader: &str, kind: Kind, message: String) -> Self {
        Self {
            reader: reader.to_owned(),
            kind,
            file: file_in(&message),
            line: line_in(&message),
            cause: cause(&message),
            message,
            count: None,
            examples: Vec::new(),
        }
    }

    pub fn about(reader: &str, kind: Kind, subject: &str, why: &str) -> Self {
        Self {
            cause: cause(why),
            ..Self::new(reader, kind, format!("{subject}: {why}"))
        }
    }

    #[cfg(test)]
    pub fn in_file(reader: &str, kind: Kind, file: &str, message: String) -> Self {
        Self {
            file: Some(file.to_owned()),
            ..Self::new(reader, kind, message)
        }
    }

    pub fn split(reader: &str, kind: Kind, file: Option<&str>, message: &str) -> Self {
        let diagnostic = match message.split_once(": ") {
            Some((subject, why)) => Self::about(reader, kind, subject, why),
            None => Self::new(reader, kind, message.to_owned()),
        };
        match file {
            Some(file) => Self {
                file: Some(file.to_owned()),
                ..diagnostic
            },
            None => diagnostic,
        }
    }

    pub fn at(reader: &str, kind: Kind, file: &str, line: Option<u32>, what: &str) -> Self {
        let message = match line {
            Some(line) => format!("{file}: line {line}: {what}"),
            None => format!("{file}: {what}"),
        };
        Self {
            reader: reader.to_owned(),
            kind,
            file: Some(file.to_owned()),
            line,
            cause: cause(what),
            message,
            count: None,
            examples: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderRun {
    pub reader: String,
    pub milliseconds: u128,
    pub completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Mounted {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StateHistoryDiagnostic {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub kind: Kind,
    pub message: String,
}

impl From<&Diagnostic> for StateHistoryDiagnostic {
    fn from(note: &Diagnostic) -> Self {
        Self {
            file: note.file.clone(),
            line: note.line,
            kind: note.kind,
            message: note.message.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StateHistoryState {
    pub id: u32,
    pub file: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(Default)]
pub struct StateHistory {
    pub notes: Vec<StateHistoryDiagnostic>,
    pub states: Vec<StateHistoryState>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StackReport {
    pub stack: String,
    pub name: String,
    #[serde(default)]
    pub descriptor_file: Option<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub supported_version: String,
    pub mods: Vec<Mounted>,
    pub dlc: usize,
    pub skipped_dependencies: Vec<String>,
    pub order_note: Option<String>,
    pub gather_error: Option<String>,
    pub gather_milliseconds: u128,
    pub read_milliseconds: u128,
    pub readers: Vec<ReaderRun>,
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    pub state_history: Vec<StateHistoryDiagnostic>,
    #[serde(default)]
    pub state_history_states: Vec<StateHistoryState>,
}

impl StackReport {
    fn counts(&self) -> BTreeMap<&str, BTreeMap<Kind, usize>> {
        let mut counts: BTreeMap<&str, BTreeMap<Kind, usize>> = BTreeMap::new();
        for diagnostic in &self.diagnostics {
            *counts
                .entry(diagnostic.reader.as_str())
                .or_default()
                .entry(diagnostic.kind)
                .or_default() += diagnostic.occurrences();
        }
        counts
    }
}

const MANIFEST: &str = "manifest.json";

pub fn write_run(directory: &Path, run: &BTreeMap<String, StackReport>) -> io::Result<()> {
    for report in run.values() {
        fs::write(
            directory.join(format!("{}.json", report.stack)),
            serde_json::to_vec_pretty(report)?,
        )?;
    }
    fs::write(
        directory.join(MANIFEST),
        serde_json::to_vec_pretty(&run.keys().collect::<Vec<_>>())?,
    )
}

pub fn read_run(directory: &Path) -> io::Result<BTreeMap<String, StackReport>> {
    let manifest = directory.join(MANIFEST);
    let stacks: Vec<String> = serde_json::from_slice(&fs::read(&manifest)?)
        .map_err(|error| io::Error::other(format!("{}: {error}", manifest.display())))?;
    stacks
        .into_iter()
        .map(|stack| {
            let path = directory.join(format!("{stack}.json"));
            let report: StackReport = serde_json::from_slice(&fs::read(&path)?)
                .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))?;
            Ok((stack, report))
        })
        .collect()
}

fn kind_cells(counts: &BTreeMap<Kind, usize>) -> String {
    Kind::ALL
        .iter()
        .map(|kind| match counts.get(kind) {
            Some(count) => count.to_string(),
            None => String::new(),
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn header(leading: &[&str]) -> String {
    let columns: Vec<_> = leading
        .iter()
        .copied()
        .chain(Kind::ALL.iter().map(|kind| kind.label()))
        .collect();
    format!(
        "| {} |\n|{}\n",
        columns.join(" | "),
        "---|".repeat(columns.len())
    )
}

pub fn summary(run: &BTreeMap<String, StackReport>, causes: usize) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## Stacks found\n");
    out.push_str(&header(&[
        "stack",
        "descriptor",
        "name",
        "version",
        "supported_version",
        "dependencies mounted",
        "dependencies not found",
        "DLC",
        "gather ms",
        "read ms",
        "not run",
        "actionable not in vanilla",
    ]));
    let vanilla = run.get(super::VANILLA);
    for report in run.values() {
        let mut totals = BTreeMap::new();
        for diagnostic in &report.diagnostics {
            *totals.entry(diagnostic.kind).or_insert(0) += diagnostic.occurrences();
        }
        let name = match &report.gather_error {
            Some(error) => format!("{} (not gathered: {})", cell(&report.name), cell(error)),
            None => cell(&report.name),
        };
        let not_run: Vec<String> = report
            .readers
            .iter()
            .filter_map(|run| Some(format!("{} ({})", run.reader, run.not_run.as_ref()?)))
            .collect();
        let beyond = vanilla
            .filter(|vanilla| vanilla.stack != report.stack)
            .map(|vanilla| {
                growth(&vanilla.diagnostics, &report.diagnostics)
                    .into_iter()
                    .filter(|(diagnostic, _)| diagnostic.kind.actionable())
                    .map(|(_, count)| count)
                    .sum::<usize>()
                    .to_string()
            })
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "| {} | {} | {name} | {} | {} | {} | {} | {} | {} | {} | {} | {beyond} | {} |",
            cell(&report.stack),
            cell(report.descriptor_file.as_deref().unwrap_or("")),
            cell(&report.version),
            cell(&report.supported_version),
            cell(
                &report
                    .mods
                    .iter()
                    .filter(|mounted| mounted.name != report.name)
                    .map(|mounted| format!(
                        "{} ({}, {})",
                        mounted.name, mounted.id, mounted.version
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            cell(&report.skipped_dependencies.join(", ")),
            report.dlc,
            report.gather_milliseconds,
            report.read_milliseconds,
            cell(&not_run.join(", ")),
            kind_cells(&totals)
        );
    }
    let products = dashboard::products::PRODUCTS;
    let _ = writeln!(out, "\n## Dashboard products\n");
    let _ = writeln!(
        out,
        "| stack | {} | content | offline |\n|{}",
        products
            .iter()
            .map(|product| product.key)
            .collect::<Vec<_>>()
            .join(" | "),
        "---|".repeat(products.len() + 3)
    );
    for report in run.values() {
        let counts = report.counts();
        let beyond = vanilla
            .filter(|base| base.stack != report.stack)
            .map(|base| growth(&base.diagnostics, &report.diagnostics));
        let cells: Vec<_> = products
            .iter()
            .map(|product| {
                let reader = format!("dashboard.{}", product.key);
                let kinds = counts
                    .get(reader.as_str())
                    .map(|counts| {
                        Kind::ALL
                            .iter()
                            .filter(|kind| kind.actionable())
                            .filter_map(|kind| {
                                counts
                                    .get(kind)
                                    .map(|count| format!("{count} {}", kind.label()))
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                if !kinds.is_empty() {
                    match &beyond {
                        Some(beyond) => format!(
                            "{kinds} (+{} vs vanilla)",
                            beyond
                                .iter()
                                .filter(|(diagnostic, _)| diagnostic.reader == reader
                                    && diagnostic.kind.actionable())
                                .map(|(_, count)| count)
                                .sum::<usize>()
                        ),
                        None => kinds,
                    }
                } else if report
                    .readers
                    .iter()
                    .any(|run| run.reader == reader && run.completed)
                {
                    "clean".to_owned()
                } else {
                    "not run".to_owned()
                }
            })
            .collect();
        let informational = |kind| {
            report
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    products
                        .iter()
                        .any(|product| diagnostic.reader == format!("dashboard.{}", product.key))
                        && diagnostic.kind == kind
                })
                .map(Diagnostic::occurrences)
                .sum::<usize>()
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} |",
            cell(&report.stack),
            cells.join(" | "),
            informational(Kind::Content),
            informational(Kind::Offline)
        );
    }
    let _ = writeln!(out, "\n## New in mods\n");
    let _ = writeln!(
        out,
        "Actionable dashboard product causes beyond vanilla; occurrences include aggregate count growth, matched by reader, kind, file and message.\n"
    );
    for report in run.values().filter(|report| report.stack != super::VANILLA) {
        let _ = writeln!(out, "### {}\n", cell(&report.stack));
        let Some(base) = vanilla else {
            let _ = writeln!(out, "Vanilla baseline unavailable.\n");
            continue;
        };
        let beyond = growth(&base.diagnostics, &report.diagnostics);
        let mut causes: BTreeMap<(&str, Kind, String), (usize, &Diagnostic)> = BTreeMap::new();
        for (diagnostic, count) in &beyond {
            if diagnostic.kind.actionable()
                && products
                    .iter()
                    .any(|product| diagnostic.reader == format!("dashboard.{}", product.key))
            {
                let entry = causes
                    .entry((
                        &diagnostic.reader,
                        diagnostic.kind,
                        diagnostic.cause.clone(),
                    ))
                    .or_insert((0, diagnostic));
                entry.0 += count;
            }
        }
        if causes.is_empty() {
            let _ = writeln!(out, "No new actionable product diagnostics.\n");
            continue;
        }
        let _ = writeln!(
            out,
            "| product / reader | kind | count | cause | example |\n|---|---|---|---|---|"
        );
        let mut causes: Vec<_> = causes.into_iter().collect();
        causes.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
        for ((reader, kind, cause), (count, example)) in causes {
            let _ = writeln!(
                out,
                "| {} | {} | {count} | {} | {} |",
                cell(reader.trim_start_matches("dashboard.")),
                kind.label(),
                cell(&cause),
                cell(&example.details())
            );
        }
        out.push('\n');
    }
    let _ = writeln!(out, "\n## Stacks by reader\n");
    out.push_str(&header(&["stack", "reader"]));
    for report in run.values() {
        for (reader, counts) in report.counts() {
            let _ = writeln!(
                out,
                "| {} | {reader} | {} |",
                report.stack,
                kind_cells(&counts)
            );
        }
    }
    out.push_str(&state_history(run));
    let _ = writeln!(
        out,
        "\n## Causes\n\nEach stack's causes, the gravest kinds first, then the most frequent.\n"
    );
    for report in run.values() {
        let mut by_cause: BTreeMap<(&str, Kind, &str), usize> = BTreeMap::new();
        for diagnostic in &report.diagnostics {
            *by_cause
                .entry((&diagnostic.reader, diagnostic.kind, &diagnostic.cause))
                .or_default() += diagnostic.occurrences();
        }
        if by_cause.is_empty() {
            continue;
        }
        let mut ranked: Vec<_> = by_cause.into_iter().collect();
        ranked.sort_by(|a, b| (a.0.1, b.1, a.0.0, a.0.2).cmp(&(b.0.1, a.1, b.0.0, b.0.2)));
        let _ = writeln!(out, "### {} ({})\n", report.stack, cell(&report.name));
        let _ = writeln!(out, "| count | reader | kind | cause |\n|---|---|---|---|");
        for ((reader, kind, cause), count) in ranked.into_iter().take(causes) {
            let _ = writeln!(
                out,
                "| {count} | {reader} | {} | {} |",
                kind.label(),
                cell(&truncate(cause, 240))
            );
        }
        let _ = writeln!(out);
    }
    out
}

pub fn state_history(run: &BTreeMap<String, StackReport>) -> String {
    let mut out = String::from("\n## State history\n\n");
    for report in run.values() {
        let _ = writeln!(
            out,
            "### {} ({})\n",
            cell(&report.stack),
            cell(&report.name)
        );
        let reader = report
            .readers
            .iter()
            .find(|reader| reader.reader == "states");
        if let Some(error) = &report.gather_error {
            let _ = writeln!(
                out,
                "Unavailable: stack was not gathered ({}).\n",
                cell(error)
            );
            continue;
        }
        if let Some(needs) = reader.and_then(|reader| reader.not_run.as_deref()) {
            let _ = writeln!(
                out,
                "Unavailable: states reader did not run ({}).\n",
                cell(needs)
            );
            continue;
        }
        if reader.is_none() {
            let _ = writeln!(out, "Unavailable: states reader result was not recorded.\n");
            continue;
        }
        if reader.is_some_and(|reader| reader.completed) {
            let _ = writeln!(
                out,
                "Affected states: {}.\n",
                report.state_history_states.len()
            );
            if !report.state_history_states.is_empty() {
                let _ = writeln!(
                    out,
                    "| stack | name | state id | file | reasons |\n|---|---|---|---|---|"
                );
                for state in &report.state_history_states {
                    let _ = writeln!(
                        out,
                        "| {} | {} | {} | {} | {} |",
                        cell(&report.stack),
                        cell(&report.name),
                        state.id,
                        cell(state.file.as_deref().unwrap_or("unknown")),
                        cell(&state.reasons.join("; "))
                    );
                }
                let _ = writeln!(out);
            }
        } else {
            let _ = writeln!(out, "Affected states: unavailable.\n");
        }
        if report.state_history.is_empty() {
            let message = if reader.is_some_and(|reader| reader.completed) {
                "No states reader diagnostics. Refused: no."
            } else {
                "Unavailable: states reader failed. Refused: yes."
            };
            let _ = writeln!(out, "{message}\n");
            continue;
        }
        let mut kinds: BTreeMap<Kind, usize> = BTreeMap::new();
        for note in &report.state_history {
            *kinds.entry(note.kind).or_default() += 1;
        }
        let counts = kinds
            .iter()
            .map(|(kind, count)| format!("{}: {count}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        let refused = if reader.is_some_and(|reader| !reader.completed) {
            "yes"
        } else {
            "no"
        };
        let _ = writeln!(out, "Notes per kind: {counts}. Refused: {refused}.\n");
        if reader.is_some_and(|reader| !reader.completed) {
            let _ = writeln!(
                out,
                "The states reader failed; the state inventory is unavailable.\n"
            );
        }
        let _ = writeln!(
            out,
            "Returned reader diagnostics, including summaries and notes not attributable to a kept state:\n"
        );
        let _ = writeln!(
            out,
            "| stack | name | file | line | producer kind | message |\n|---|---|---|---|---|---|"
        );
        for note in &report.state_history {
            let line = note
                .line
                .map_or_else(|| "unknown".to_owned(), |line| line.to_string());
            let _ = writeln!(
                out,
                "| {} | {} | {} | {line} | {} | {} |",
                cell(&report.stack),
                cell(&report.name),
                cell(note.file.as_deref().unwrap_or("unknown")),
                note.kind.label(),
                cell(&note.message)
            );
        }
        let _ = writeln!(out);
    }
    out
}

pub fn diff(
    base: &BTreeMap<String, StackReport>,
    head: &BTreeMap<String, StackReport>,
    examples: usize,
) -> String {
    let mut out = String::new();
    let mut changed = false;
    let stacks: BTreeSet<&String> = base.keys().chain(head.keys()).collect();
    for stack in stacks {
        let (before, after) = (base.get(stack), head.get(stack));
        let (Some(before), Some(after)) = (before, after) else {
            changed = true;
            let _ = writeln!(
                out,
                "- {stack}: only in the {} run",
                if before.is_some() { "base" } else { "head" }
            );
            continue;
        };
        if before.gather_error != after.gather_error {
            changed = true;
            let _ = writeln!(
                out,
                "- {stack}: gather {:?} -> {:?}",
                before.gather_error, after.gather_error
            );
        }
        let mut previous: BTreeMap<Key<'_>, Vec<&Diagnostic>> = BTreeMap::new();
        for diagnostic in &before.diagnostics {
            previous
                .entry(key(diagnostic))
                .or_default()
                .push(diagnostic);
        }
        for diagnostic in &after.diagnostics {
            if let Some(old) = previous.get_mut(&key(diagnostic)).and_then(Vec::pop)
                && (old.count != diagnostic.count || old.examples != diagnostic.examples)
            {
                changed = true;
                let _ = writeln!(
                    out,
                    "- {stack}: {} ({}) inventory {:?} -> {:?}; examples: {}",
                    diagnostic.reader,
                    diagnostic.kind.label(),
                    old.count,
                    diagnostic.count,
                    diagnostic.examples.join(", ")
                );
            }
        }
        let (gone, new) = difference(&before.diagnostics, &after.diagnostics);
        if gone.is_empty() && new.is_empty() {
            continue;
        }
        changed = true;
        let _ = writeln!(
            out,
            "\n### {stack} ({}): {} gone, {} new\n",
            cell(&after.name),
            gone.len(),
            new.len()
        );
        let mut readers: BTreeMap<(&str, Kind), (usize, usize)> = BTreeMap::new();
        for diagnostic in &gone {
            readers
                .entry((&diagnostic.reader, diagnostic.kind))
                .or_default()
                .0 += 1;
        }
        for diagnostic in &new {
            readers
                .entry((&diagnostic.reader, diagnostic.kind))
                .or_default()
                .1 += 1;
        }
        let _ = writeln!(out, "| reader | kind | gone | new |\n|---|---|---|---|");
        for ((reader, kind), (gone, new)) in readers {
            let _ = writeln!(out, "| {reader} | {} | {gone} | {new} |", kind.label());
        }
        for (label, list) in [("gone", &gone), ("new", &new)] {
            for diagnostic in list.iter().take(examples) {
                let _ = writeln!(
                    out,
                    "- {label}: {} {}: {}",
                    diagnostic.reader,
                    diagnostic.kind.label(),
                    truncate(&diagnostic.details(), 400)
                );
            }
            if list.len() > examples {
                let _ = writeln!(out, "- {label}: ... and {} more", list.len() - examples);
            }
        }
    }
    if !changed {
        out.push_str("No stack's diagnostics changed.\n");
    }
    out
}

type Key<'a> = (&'a str, Kind, Option<&'a str>, &'a str);

fn key(diagnostic: &Diagnostic) -> Key<'_> {
    (
        &diagnostic.reader,
        diagnostic.kind,
        diagnostic.file.as_deref(),
        &diagnostic.message,
    )
}

fn growth<'a>(base: &[Diagnostic], head: &'a [Diagnostic]) -> Vec<(&'a Diagnostic, usize)> {
    let mut remaining: BTreeMap<Key<'_>, usize> = BTreeMap::new();
    for diagnostic in base {
        *remaining.entry(key(diagnostic)).or_default() += diagnostic.occurrences();
    }
    let mut new = Vec::new();
    for diagnostic in head {
        let baseline = remaining.entry(key(diagnostic)).or_default();
        let matched = (*baseline).min(diagnostic.occurrences());
        *baseline -= matched;
        let added = diagnostic.occurrences() - matched;
        if added > 0 {
            new.push((diagnostic, added));
        }
    }
    new
}

fn difference<'a>(
    base: &'a [Diagnostic],
    head: &'a [Diagnostic],
) -> (Vec<&'a Diagnostic>, Vec<&'a Diagnostic>) {
    let mut counts: BTreeMap<Key<'a>, (Vec<&'a Diagnostic>, usize)> = BTreeMap::new();
    for diagnostic in base {
        counts
            .entry(key(diagnostic))
            .or_default()
            .0
            .push(diagnostic);
    }
    let mut new = Vec::new();
    for diagnostic in head {
        let entry = counts.entry(key(diagnostic)).or_default();
        if entry.1 < entry.0.len() {
            entry.1 += 1;
        } else {
            new.push(diagnostic);
        }
    }
    let gone = counts
        .into_values()
        .flat_map(|(listed, matched)| listed.into_iter().skip(matched))
        .collect();
    (gone, new)
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn truncate(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_owned(),
    }
}

const EXTENSIONS: [&str; 13] = [
    ".txt", ".yml", ".lua", ".gfx", ".gui", ".asset", ".csv", ".map", ".bmp", ".dds", ".tga",
    ".fnt", ".mod",
];

const ROOTS: [&str; 12] = [
    "common/",
    "history/",
    "map/",
    "localisation/",
    "interface/",
    "gfx/",
    "events/",
    "portraits/",
    "music/",
    "sound/",
    "tests/",
    "tweakergui_assets/",
];

fn path_spans(message: &str) -> Vec<(usize, usize)> {
    let lower = message.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'/';
    let boundary = |at: usize| {
        at == 0
            || matches!(
                bytes[at - 1],
                b' ' | b'\t' | b'"' | b'\'' | b'`' | b'(' | b'['
            )
    };
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for extension in EXTENSIONS {
        for (at, _) in lower.match_indices(extension) {
            let end = at + extension.len();
            let after = bytes.get(end).copied();
            let ends = match after {
                None => true,
                Some(b'.') => bytes
                    .get(end + 1)
                    .is_none_or(|&c| !c.is_ascii_alphanumeric()),
                Some(c) => !word(c),
            };
            let earlier = spans
                .iter()
                .map(|&(_, end)| end)
                .filter(|&e| e <= at)
                .max()
                .unwrap_or(0);
            if !ends || spans.iter().any(|&(start, stop)| start <= at && at < stop) {
                continue;
            }
            let rooted = (earlier..at).rev().find(|&start| {
                boundary(start)
                    && ROOTS
                        .iter()
                        .any(|root| bytes[start..].starts_with(root.as_bytes()))
            });
            let start = rooted.or_else(|| {
                let start = lower[..at]
                    .rfind(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | '['))
                    .map_or(0, |space| space + 1);
                lower[start..at].contains('/').then_some(start)
            });
            if let Some(start) = start.filter(|&start| {
                let span = &message[start..end];
                !span.contains(['"', ':', '\n']) && !span.contains(", ")
            }) {
                spans.push((start, end));
            }
        }
    }
    spans.sort_unstable();
    spans
}

fn file_in(message: &str) -> Option<String> {
    path_spans(message)
        .first()
        .map(|&(start, end)| message[start..end].to_owned())
}

fn line_in(message: &str) -> Option<u32> {
    let digits = |text: &str| -> Option<u32> {
        let end = text
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len());
        text[..end].parse().ok()
    };
    for (_, end) in path_spans(message) {
        if let Some(line) = message[end..].strip_prefix(':').and_then(digits) {
            return Some(line);
        }
    }
    let lower = message.to_ascii_lowercase();
    lower
        .match_indices("line ")
        .find_map(|(at, found)| digits(&message[at + found.len()..]))
}

fn cause(message: &str) -> String {
    let mut unfiled = String::with_capacity(message.len());
    let mut from = 0;
    for (start, end) in path_spans(message) {
        unfiled.push_str(&message[from..start]);
        unfiled.push_str("<file>");
        from = end;
    }
    unfiled.push_str(&message[from..]);
    let mut unquoted = String::with_capacity(unfiled.len());
    let mut chars = unfiled.chars();
    while let Some(c) = chars.next() {
        if matches!(c, '"' | '`') {
            chars.by_ref().find(|&inner| inner == c);
            unquoted.push_str("<text>");
        } else {
            unquoted.push(c);
        }
    }
    unquoted
        .split_inclusive(char::is_whitespace)
        .map(numbers)
        .collect()
}

fn numbers(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut chars = word.chars().peekable();
    let mut previous: Option<char> = None;
    while let Some(c) = chars.next() {
        let boundary = previous.is_none_or(|p| !(p.is_alphanumeric() || p == '_'));
        if c.is_ascii_digit() && boundary {
            while chars
                .peek()
                .is_some_and(|next| next.is_ascii_hexdigit() || *next == '.')
            {
                chars.next();
            }
            out.push('N');
            previous = Some('N');
        } else {
            out.push(c);
            previous = Some(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    mod state_history;

    fn product_cell(summary: &str, stack: &str, key: &str) -> String {
        let matrix = summary
            .split("## Dashboard products\n")
            .nth(1)
            .unwrap()
            .split("## New in mods\n")
            .next()
            .unwrap();
        let mut lines = matrix.lines();
        let column = lines
            .find(|line| line.starts_with("| stack |"))
            .unwrap()
            .split('|')
            .position(|heading| heading.trim() == key)
            .unwrap_or_else(|| panic!("no column {key}: {matrix}"));
        let row = lines
            .find(|line| line.starts_with(&format!("| {stack} |")))
            .unwrap_or_else(|| panic!("no row {stack}: {matrix}"));
        row.split('|').nth(column).unwrap().trim().to_owned()
    }

    #[test]
    fn aggregate_inventory_changes_keep_the_diff_identity() {
        let mut before = Diagnostic::about(
            "dashboard.national-focus",
            Kind::Unsupported,
            "product coverage",
            "country script is not evaluated",
        );
        before.count = Some(2);
        before.examples = vec!["focus a".into(), "focus b".into()];
        let mut after = before.clone();
        after.count = Some(8);
        after.examples.push("focus c".into());
        let before = [before];
        let after = [after];
        let (gone, new) = difference(&before, &after);
        assert!(gone.is_empty() && new.is_empty());
        assert!(after[0].details().contains("8 items"));
        let decoded: Diagnostic =
            serde_json::from_str(&serde_json::to_string(&after[0]).unwrap()).unwrap();
        assert_eq!(decoded.count, Some(8));
        let report = |diagnostic| StackReport {
            stack: "mod".into(),
            diagnostics: vec![diagnostic],
            ..Default::default()
        };
        let inventory = diff(
            &BTreeMap::from([("mod".into(), report(before[0].clone()))]),
            &BTreeMap::from([("mod".into(), report(after[0].clone()))]),
            5,
        );
        assert!(
            inventory.contains("inventory Some(2) -> Some(8)"),
            "{inventory}"
        );
        assert!(
            !inventory.contains("new:") && !inventory.contains("gone:"),
            "{inventory}"
        );
    }

    #[test]
    fn summary_counts_aggregate_growth_and_reader_failures_as_occurrences() {
        let reader = "dashboard.national-focus";
        let mut shared = Diagnostic::new(reader, Kind::Unsupported, "country script".into());
        shared.count = Some(2);
        shared.examples = vec!["old focus".into()];
        let mut larger = shared.clone();
        larger.count = Some(8);
        larger.examples = vec!["different example".into()];
        assert_eq!(key(&shared), key(&larger));
        assert!(
            difference(&[shared.clone()], &[larger.clone()])
                .1
                .is_empty()
        );
        let mut info = Diagnostic::new(reader, Kind::Offline, "holder tag".into());
        info.count = Some(12);
        let baseline = StackReport {
            stack: "vanilla".into(),
            diagnostics: vec![shared],
            ..Default::default()
        };
        let modded = StackReport {
            stack: "mod".into(),
            diagnostics: vec![
                larger,
                info,
                Diagnostic::new(reader, Kind::Unsettled, "unread suffix".into()),
                Diagnostic::new(reader, Kind::ParseFailure, "parser failure".into()),
            ],
            ..Default::default()
        };
        let out = summary(
            &BTreeMap::from([("vanilla".into(), baseline), ("mod".into(), modded)]),
            10,
        );
        assert_eq!(
            product_cell(&out, "mod", "national-focus"),
            "1 unsettled, 1 parse, 8 unsupported (+8 vs vanilla)",
            "{out}"
        );
        assert!(out.contains("| 0 | 12 |"), "{out}");
        assert!(
            out.contains("| national-focus | unsupported | 6 | country script |"),
            "{out}"
        );
        assert!(
            out.contains("| national-focus | unsettled | 1 | unread suffix |"),
            "{out}"
        );
        assert!(
            out.contains("| national-focus | parse | 1 | parser failure |"),
            "{out}"
        );
        for kind in Kind::ALL {
            assert_eq!(
                kind.actionable(),
                !matches!(kind, Kind::Content | Kind::Offline)
            );
        }
        let mut small = Diagnostic::new(reader, Kind::Unsupported, "country script".into());
        small.count = Some(1);
        let mut big = small.clone();
        big.count = Some(3);
        assert!(growth(&[big], &[small]).is_empty());
    }

    #[test]
    fn a_message_gives_its_file_line_and_masked_cause() {
        let diagnostic = Diagnostic::new(
            "ideas",
            Kind::Unsettled,
            "Ideas unavailable in common/ideas/GER.txt: line 12: a block \"x\" at 3.5".into(),
        );
        assert_eq!(diagnostic.file.as_deref(), Some("common/ideas/GER.txt"));
        assert_eq!(diagnostic.line, Some(12));
        assert_eq!(
            diagnostic.cause,
            "Ideas unavailable in <file>: line N: a block <text> at N"
        );
        assert_eq!(
            cause("common/x/N_1.txt line 4: key_2 has no operator: 123ABCDEF reads 1936.1.1"),
            "<file> line N: key_2 has no operator: N reads N"
        );
        let about = Diagnostic::about("r", Kind::Refused, "raid GER_1", "3 is bad");
        assert_eq!(about.message, "raid GER_1: 3 is bad");
        assert_eq!(about.cause, "N is bad");
        let colon = Diagnostic::new("x", Kind::Logged, "common/a/b.txt:40: bad".into());
        assert_eq!(
            (colon.file.as_deref(), colon.line),
            (Some("common/a/b.txt"), Some(40))
        );
        let spaced = Diagnostic::new(
            "ideas",
            Kind::Misread,
            "Ideas in common/ideas/My Mod (OLD).txt: line 12: key 4".into(),
        );
        assert_eq!(
            (spaced.file.as_deref(), spaced.line),
            (Some("common/ideas/My Mod (OLD).txt"), Some(12))
        );
        assert_eq!(spaced.cause, "Ideas in <file>: line N: key N");
        let colon = Diagnostic::new("x", Kind::Logged, "gfx/interface/A B.gfx:7: bad".into());
        assert_eq!(
            (colon.file.as_deref(), colon.line),
            (Some("gfx/interface/A B.gfx"), Some(7))
        );
        assert_eq!(colon.cause, "<file>:N: bad");
        let list = Diagnostic::new(
            "x",
            Kind::Warning,
            "resolve to no mesh (gfx/models textures), a (no entity), w_2.dds".into(),
        );
        assert_eq!(list.file, None);
        let split = Diagnostic::split("x", Kind::Refused, Some("gfx/a.txt"), "entity E_1: bad 2");
        assert_eq!(
            (split.file.as_deref(), split.cause.as_str()),
            (Some("gfx/a.txt"), "bad N")
        );
    }

    #[test]
    fn the_difference_of_two_runs_is_a_multiset_difference() {
        let one = |message: &str| Diagnostic::new("r", Kind::Warning, message.into());
        let base = [one("a"), one("a"), one("b")];
        let head = [one("a"), one("c")];
        let (gone, new) = difference(&base, &head);
        let messages = |list: &[&Diagnostic]| {
            list.iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(messages(&gone), ["a", "b"]);
        assert_eq!(messages(&new), ["c"]);
    }

    #[test]
    fn summary_lists_stack_descriptors_dependencies_and_every_product() {
        use dashboard_base::products::Product;
        use dashboard_land::products::doctrines::Doctrines;
        use dashboard_politics::products::national_focus::NationalFocus;
        let reader = format!("dashboard.{}", NationalFocus::KEY);
        let mut report = StackReport {
            stack: "local-fixture".into(),
            name: "Fixture | conversion".into(),
            descriptor_file: Some("mod/fixture.mod".into()),
            version: "2.4".into(),
            supported_version: "1.17.*".into(),
            mods: vec![Mounted {
                id: "123".into(),
                name: "Dependency".into(),
                version: "1.0".into(),
            }],
            skipped_dependencies: vec!["Other: not installed".into()],
            ..Default::default()
        };
        report.readers = dashboard::products::PRODUCTS
            .iter()
            .map(|product| ReaderRun {
                reader: format!("dashboard.{}", product.key),
                milliseconds: 1,
                completed: true,
                not_run: None,
                note: Some("coverage".into()),
            })
            .collect();
        report.diagnostics = vec![
            Diagnostic::new(&reader, Kind::Fallback, "key shown".into()),
            Diagnostic::new(&reader, Kind::Ambiguous, "two portraits".into()),
        ];
        let summary = summary(&BTreeMap::from([(report.stack.clone(), report)]), 10);
        assert!(summary.contains("## Stacks found\n"));
        assert!(summary.contains("mod/fixture.mod | Fixture \\| conversion | 2.4 | 1.17.* | Dependency (123, 1.0) | Other: not installed"), "{summary}");
        assert!(summary.contains("## Dashboard products\n"));
        for product in dashboard::products::PRODUCTS {
            let expected = if product.key == NationalFocus::KEY {
                "1 ambiguous, 1 fallback"
            } else {
                "clean"
            };
            assert_eq!(
                product_cell(&summary, "local-fixture", product.key),
                expected,
                "{summary}"
            );
        }
        let diagnostic = Diagnostic::new(
            &format!("dashboard.{}", Doctrines::KEY),
            Kind::Fallback,
            "sprite missing".into(),
        );
        let encoded = serde_json::to_string(&diagnostic).unwrap();
        assert!(encoded.contains("\"fallback\""));
        let decoded: Diagnostic = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, diagnostic);
        assert_eq!(difference(&[], &[diagnostic]).1.len(), 1);
    }

    #[test]
    fn product_matrix_and_mod_causes_count_only_actionable_multiset_delta() {
        let reader = "dashboard.national-focus";
        let shared = Diagnostic::in_file(
            reader,
            Kind::Fallback,
            "interface/fixture.gfx",
            "same fallback".into(),
        );
        let added = Diagnostic::about(reader, Kind::Refused, "focus added", "unsupported texture");
        let runs = vec![ReaderRun {
            reader: reader.into(),
            milliseconds: 0,
            completed: true,
            not_run: None,
            note: None,
        }];
        let base = StackReport {
            stack: super::super::VANILLA.into(),
            readers: runs.clone(),
            diagnostics: vec![shared.clone()],
            ..Default::default()
        };
        let modded = StackReport {
            stack: "mod".into(),
            readers: runs.clone(),
            diagnostics: vec![
                shared.clone(),
                shared,
                added,
                Diagnostic::new(reader, Kind::Content, "authored omission".into()),
                Diagnostic::new(reader, Kind::Offline, "recorded context; 4000 items".into()),
                Diagnostic::new("other", Kind::Warning, "not a product".into()),
            ],
            ..Default::default()
        };
        let clean = StackReport {
            stack: "clean-mod".into(),
            readers: runs,
            diagnostics: vec![
                Diagnostic::new(reader, Kind::Content, "same game recovery".into()),
                Diagnostic::new(reader, Kind::Offline, "one aggregated reason".into()),
            ],
            ..Default::default()
        };
        let summary = summary(
            &BTreeMap::from([
                (base.stack.clone(), base),
                (modded.stack.clone(), modded),
                (clean.stack.clone(), clean),
            ]),
            10,
        );
        let matrix = summary
            .split("## Dashboard products\n")
            .nth(1)
            .unwrap()
            .split("## New in mods\n")
            .next()
            .unwrap();
        let stacks = summary
            .split("## Stacks found\n")
            .nth(1)
            .unwrap()
            .split("## Dashboard products\n")
            .next()
            .unwrap();
        let heading = stacks
            .lines()
            .find(|line| line.starts_with("| stack |"))
            .unwrap();
        let column = heading
            .split('|')
            .position(|cell| cell.trim() == "actionable not in vanilla")
            .unwrap();
        for (stack, expected) in [("mod", "3"), ("clean-mod", "0")] {
            let row = stacks
                .lines()
                .find(|line| line.starts_with(&format!("| {stack} |")))
                .unwrap();
            assert_eq!(
                row.split('|').nth(column).unwrap().trim(),
                expected,
                "{stacks}"
            );
        }
        assert_eq!(summary.matches("## New in mods\n").count(), 1);
        assert_eq!(
            summary
                .matches("Actionable dashboard product causes beyond vanilla;")
                .count(),
            1
        );
        assert!(matrix.contains("| content | offline |"));
        assert_eq!(
            product_cell(&summary, "mod", "national-focus"),
            "1 refused, 2 fallback (+2 vs vanilla)",
            "{matrix}"
        );
        assert_eq!(
            product_cell(&summary, "clean-mod", "national-focus"),
            "clean",
            "{matrix}"
        );
        assert!(matrix.contains("| 1 | 1 |"));
        let new = summary
            .split("## New in mods\n")
            .nth(1)
            .unwrap()
            .split("## Stacks by reader\n")
            .next()
            .unwrap();
        assert!(
            new.contains("| national-focus | fallback | 1 | same fallback | same fallback |"),
            "{new}"
        );
        assert!(new.contains("| national-focus | refused | 1 | unsupported texture | focus added: unsupported texture |"), "{new}");
        assert!(new.contains("No new actionable product diagnostics."));
        assert!(
            !new.contains("authored omission")
                && !new.contains("recorded context")
                && !new.contains("not a product")
        );
        for kind in [Kind::Content, Kind::Offline] {
            let diagnostic = Diagnostic::new(reader, kind, "info".into());
            let json = serde_json::to_string(&diagnostic).unwrap();
            assert!(json.contains(kind.label()));
            assert_eq!(
                serde_json::from_str::<Diagnostic>(&json).unwrap(),
                diagnostic
            );
            assert_eq!(difference(&[], &[diagnostic]).1.len(), 1);
            assert!(Kind::Warning < kind);
        }
    }

    #[test]
    fn causes_keep_producer_reasons_without_wording_specific_grouping() {
        assert_eq!(
            cause("localisation key_a holds the scripted expression \"[SCOPE.ExampleLabel]\""),
            "localisation key_a holds the scripted expression <text>"
        );
        assert_eq!(
            cause(
                "Characters in common/characters/mod.txt: 12 character roles unavailable: one: unknown role."
            ),
            "Characters in <file>: N character roles unavailable: one: unknown role."
        );
        let one = Diagnostic::about(
            "dashboard.idea-history",
            Kind::Refused,
            "product coverage",
            "idea artwork unavailable",
        );
        let mut two = one.clone();
        two.count = Some(4);
        two.examples = vec!["different idea".into()];
        assert_eq!(one.cause, two.cause);
        assert_eq!(key(&one), key(&two));
    }

    #[test]
    fn product_notes_group_by_cause_and_other_dashboard_readers_stay_out_of_product_tables() {
        let other = "dashboard.scripted_content";
        assert!(
            !dashboard::products::PRODUCTS
                .iter()
                .any(|product| other == format!("dashboard.{}", product.key))
        );
        let readers: Vec<String> = dashboard::products::PRODUCTS
            .iter()
            .map(|product| format!("dashboard.{}", product.key))
            .chain([other.to_owned()])
            .collect();
        let notes: Vec<_> = readers
            .iter()
            .enumerate()
            .map(|(index, reader)| {
                let key = reader.trim_start_matches("dashboard.");
                let kind = if index % 2 == 0 {
                    Kind::Refused
                } else {
                    Kind::Misread
                };
                let mut note = Diagnostic::about(
                    reader,
                    kind,
                    "product coverage",
                    &format!("{key} producer reason"),
                );
                note.count = Some(3);
                note.examples = vec!["distinct subject: detailed producer reason".into()];
                assert_eq!(note.cause, format!("{key} producer reason"));
                note
            })
            .collect();
        let base_notes = notes
            .iter()
            .map(|note| {
                let mut base = note.clone();
                base.count = Some(1);
                base.examples.clear();
                assert_eq!(key(note), key(&base));
                base
            })
            .collect();
        let report = StackReport {
            stack: "mod".into(),
            readers: readers
                .iter()
                .map(|reader| ReaderRun {
                    reader: reader.clone(),
                    milliseconds: 0,
                    completed: true,
                    not_run: None,
                    note: None,
                })
                .collect(),
            diagnostics: notes.clone(),
            ..Default::default()
        };
        let out = summary(
            &BTreeMap::from([
                (
                    "vanilla".into(),
                    StackReport {
                        stack: "vanilla".into(),
                        diagnostics: base_notes,
                        ..Default::default()
                    },
                ),
                ("mod".into(), report),
            ]),
            10,
        );
        let section = |from: &str, to: &str| {
            out.split(from)
                .nth(1)
                .unwrap()
                .split(to)
                .next()
                .unwrap()
                .to_owned()
        };
        let products = section("## Dashboard products
", "## New in mods
");
        let new = section("## New in mods
", "## Stacks by reader
");
        for (product, note) in dashboard::products::PRODUCTS.iter().zip(&notes) {
            let kind = note.kind.label();
            assert_eq!(
                product_cell(&out, "mod", product.key),
                format!("3 {kind} (+2 vs vanilla)"),
                "{products}"
            );
            assert!(
                new.contains(&format!(
                    "| {key} | {kind} | 2 | {key} producer reason |",
                    key = product.key
                )),
                "{new}"
            );
        }
        assert!(!products.contains("scripted_content"), "{products}");
        assert!(!new.contains("scripted_content"), "{new}");
        assert!(out.contains(&format!("| mod | {other} |")), "{out}");
    }
}
