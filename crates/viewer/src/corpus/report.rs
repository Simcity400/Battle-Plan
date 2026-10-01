//! A run's reports: one JSON file per stack, the summary tables, and the
//! difference between two runs.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs, io,
    path::Path,
};

use serde::{Deserialize, Serialize};

/// What kind of diagnostic a reader gave, from the channel it came through.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The reader refused the whole stack (its `io::Error`).
    ReaderError,
    /// A file whose reading stops short: the rest of it is unknown.
    Unsettled,
    /// A place the game's reading leaves the text, read as the game reads it.
    Misread,
    /// Text the tree parser cannot parse, or a file closed at its end.
    ParseFailure,
    /// A form the port does not model, refused by name.
    Unsupported,
    /// A definition, value or lookup refused by name.
    Refused,
    /// An error the game logs, then continues or skips as the message states.
    Logged,
    /// Any other note a reader returns about the content.
    Warning,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::ReaderError,
        Kind::Unsettled,
        Kind::Misread,
        Kind::ParseFailure,
        Kind::Unsupported,
        Kind::Refused,
        Kind::Logged,
        Kind::Warning,
    ];

    fn label(self) -> &'static str {
        match self {
            Kind::ReaderError => "error",
            Kind::Unsettled => "unsettled",
            Kind::Misread => "misread",
            Kind::ParseFailure => "parse",
            Kind::Unsupported => "unsupported",
            Kind::Refused => "refused",
            Kind::Logged => "logged",
            Kind::Warning => "warning",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Diagnostic {
    pub reader: String,
    pub kind: Kind,
    /// The content file the message names, if it names one.
    pub file: Option<String>,
    /// The line the message names, if it names one.
    pub line: Option<u32>,
    /// The message with its files, numbers and quoted text masked, so the
    /// same cause in many places counts once.
    pub cause: String,
    pub message: String,
}

impl Diagnostic {
    /// A diagnostic whose file and line are read from `message`.
    pub fn new(reader: &str, kind: Kind, message: String) -> Self {
        Self {
            reader: reader.to_owned(),
            kind,
            file: file_in(&message),
            line: line_in(&message),
            cause: cause(&message),
            message,
        }
    }

    /// A diagnostic about `subject` (a name the content defines), counted by
    /// `why` alone.
    pub fn about(reader: &str, kind: Kind, subject: &str, why: &str) -> Self {
        Self {
            cause: cause(why),
            ..Self::new(reader, kind, format!("{subject}: {why}"))
        }
    }

    /// A diagnostic in `file`, whose message names it already.
    pub fn in_file(reader: &str, kind: Kind, file: &str, message: String) -> Self {
        Self {
            file: Some(file.to_owned()),
            ..Self::new(reader, kind, message)
        }
    }

    /// A diagnostic about what a message names before its first `: `,
    /// counted by the rest, in `file` when it is known.
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

    /// A diagnostic whose file and line are known.
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
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReaderRun {
    pub reader: String,
    pub milliseconds: u128,
    /// Whether the reader returned (its diagnostics are then partial
    /// refusals) or refused the stack.
    pub completed: bool,
    /// Why the reader did not run: a reader whose result it needs refused
    /// the stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_run: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Mounted {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StackReport {
    pub stack: String,
    pub name: String,
    /// The mods mounted after the DLC, in mount order.
    pub mods: Vec<Mounted>,
    pub dlc: usize,
    /// Dependencies the descriptor names that are not mounted, and why.
    pub skipped_dependencies: Vec<String>,
    /// The gather's order notes (`GatheredContent::order_note`).
    pub order_note: Option<String>,
    /// Why the stack could not be gathered; no reader ran.
    pub gather_error: Option<String>,
    pub gather_milliseconds: u128,
    pub read_milliseconds: u128,
    pub readers: Vec<ReaderRun>,
    pub diagnostics: Vec<Diagnostic>,
}

impl StackReport {
    fn counts(&self) -> BTreeMap<&str, BTreeMap<Kind, usize>> {
        let mut counts: BTreeMap<&str, BTreeMap<Kind, usize>> = BTreeMap::new();
        for diagnostic in &self.diagnostics {
            *counts
                .entry(diagnostic.reader.as_str())
                .or_default()
                .entry(diagnostic.kind)
                .or_default() += 1;
        }
        counts
    }
}

/// The run's manifest, which names its stack reports: an output folder a
/// run reuses keeps files an earlier run wrote, which are not this run's.
const MANIFEST: &str = "manifest.json";

/// Writes each stack's report and the manifest naming them.
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

/// The stack reports the run in `directory` wrote, by stack.
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

/// A table header: `leading` columns, then one per kind.
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

/// The run's summary in Markdown: each stack's totals by kind with its
/// timing, the readers that did not run and how many of its diagnostics the
/// vanilla stack lacks, each stack by reader, and each stack's gravest and
/// most frequent causes.
pub fn summary(run: &BTreeMap<String, StackReport>, causes: usize) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## Stacks\n");
    out.push_str(&header(&[
        "stack",
        "name",
        "gather ms",
        "read ms",
        "not run",
        "not in vanilla",
    ]));
    let vanilla = run.get(super::VANILLA);
    for report in run.values() {
        let mut totals = BTreeMap::new();
        for diagnostic in &report.diagnostics {
            *totals.entry(diagnostic.kind).or_insert(0) += 1;
        }
        let name = match &report.gather_error {
            Some(error) => format!("{} (not gathered: {})", report.name, cell(error)),
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
                difference(&vanilla.diagnostics, &report.diagnostics)
                    .1
                    .len()
                    .to_string()
            })
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "| {} | {name} | {} | {} | {} | {beyond} | {} |",
            report.stack,
            report.gather_milliseconds,
            report.read_milliseconds,
            not_run.join(", "),
            kind_cells(&totals)
        );
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
    let _ = writeln!(
        out,
        "\n## Causes\n\nEach stack's causes, the gravest kinds first, then the most frequent.\n"
    );
    for report in run.values() {
        let mut by_cause: BTreeMap<(&str, Kind, &str), usize> = BTreeMap::new();
        for diagnostic in &report.diagnostics {
            *by_cause
                .entry((&diagnostic.reader, diagnostic.kind, &diagnostic.cause))
                .or_default() += 1;
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

/// What changed between two runs: per stack and reader, the diagnostics only
/// the base run has (gone) and only the head run has (new), as multisets of
/// (reader, kind, file, message).
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
                    truncate(&diagnostic.message, 400)
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

/// Text made safe for one Markdown table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn truncate(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_owned(),
    }
}

/// Extensions of the content files messages name.
const EXTENSIONS: [&str; 13] = [
    ".txt", ".yml", ".lua", ".gfx", ".gui", ".asset", ".csv", ".map", ".bmp", ".dds", ".tga",
    ".fnt", ".mod",
];

/// The top-level folders a content path starts with.
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

/// Each content path `message` names, as byte spans: a known extension
/// ending a word, back to the nearest content root that starts a word, so a
/// path holding spaces is one span; else the word ending in the extension,
/// when it holds a `/`.
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
            // A path holds no quote, colon or line break, and no list
            // separator: `, ` ends a list item, not a file name.
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

/// The `N` of a `path:N`, else the first `line N` in `message`.
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

/// `message` with its file paths, numbers and quoted text masked.
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

/// `word` with each number that starts at a word boundary masked, hex
/// digits and dots included (an address such as `1424AC5E0`, a date such
/// as `1936.1.1`); digits inside a name are kept.
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
            cause("common/x/N_1.txt line 4: key_2 has no operator: 1424AC5E0 reads 1936.1.1"),
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
        // A path holding spaces is one file, in the file, the line and the
        // cause alike.
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
        // A list of names is no path.
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
}
