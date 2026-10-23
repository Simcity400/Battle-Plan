//! Every country's division equipment reply at every hour of a branch, kept as keyed runs.
use super::{
    ArmyAttribution, Built, Context, DesignsOf, DivisionEquipmentParams, DivisionEquipmentReply,
    Labels, Members, NeedsOf, POOLS, Placed, STOCK, Shared, TABLES, build_in, manpower_sum,
};
use crate::products::{
    army_content::{self, Refusals, TemplateArtwork},
    division_needs,
};
use dashboard_base::{
    prepared::{
        frame_changes::{FrameChanges, field_mask},
        series::{Accumulator, Reader},
    },
    products::{Product, equipment::RawVariant},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_keyed::{
    FIRST, KNOWN, READ,
    query::{KeyedWindow, Query},
};
use dashboard_text::{catalogue::Catalogue, template_facts::TemplateNaming};
use replay::container::Replay;
use replay_data::{
    format::FormatVersion,
    rows::{RowCodec, TableId, TableSchema, tables},
};
use replay_frame::frame::{DecodedFrame, SharedView, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};
use ts_rs::TS;

/// The family's store name.
pub const NAME: &str = "division-equipment";
/// The prepared query reading it.
pub const QUERY: &str = "division-equipment-history";

/// How long a supplying read carries: the reached product's look-back.
const CADENCE: i32 = replay_data::cadence::STAGGER_PERIOD;

/// The designs' tables a reply reads ([`army_content::Designs`]).
const DESIGNS: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
];

/// Every table an hour's observation reads, sorted.
pub fn tables() -> &'static [TableId] {
    static ALL: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut all: Vec<TableId> = [TABLES, POOLS, STOCK, DESIGNS, &[TableId::Campaign]].concat();
        all.sort_unstable();
        all.dedup();
        all
    });
    &ALL
}

pub struct DivisionEquipmentHistory;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionEquipmentHistoryParams {
    pub branch_id: String,
    pub tag: String,
    pub count_for: ArmyAttribution,
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[ts(optional)]
    pub last_hour: Option<i32>,
}

/// An equipment type's key and its name in the shown language.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryKind {
    pub id: String,
    pub archetype: Option<String>,
    pub name: Option<String>,
    pub name_missing: Option<String>,
}

/// A refusal as the history keeps it. When a table an hour's replies read cannot be read
/// whole, its refusal starts `hour H <Table>`, as `DecodedFrame::complete` words it: that
/// part is kept as the table alone, so each reply names its own hour ([`KeptReason::at`]).
/// The text around it is kept as written: `text` holds one more part than `tables`, each
/// table going between two of them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct KeptReason {
    pub text: Vec<String>,
    pub tables: Vec<String>,
}

impl KeptReason {
    fn literal(text: impl Into<String>) -> Self {
        Self {
            text: vec![text.into()],
            tables: Vec::new(),
        }
    }

    /// The refusal `before` and the failed read of `table` at the frame's hour give: the
    /// read's own `hour H <Table>` start is kept as the table, the rest of its words as
    /// written. Made where the read fails, never found again in other text.
    fn unreadable(before: &str, frame: &DecodedFrame, table: TableId, error: &io::Error) -> Self {
        let text = error.to_string();
        match text.strip_prefix(&format!("hour {} {table:?}", frame.hour)) {
            Some(rest) => Self {
                text: vec![before.to_owned(), rest.to_owned()],
                tables: vec![format!("{table:?}")],
            },
            None => Self::literal(format!("{before}{text}")),
        }
    }

    /// The refusal as a reply at `hour` words it.
    pub fn at(&self, hour: i32) -> String {
        let mut text = String::new();
        for (at, part) in self.text.iter().enumerate() {
            if at > 0
                && let Some(table) = self.tables.get(at - 1)
            {
                text.push_str(&format!("hour {hour} {table}"));
            }
            text.push_str(part);
        }
        text
    }
}

/// A template's naming operands, a failed read of its tables kept as that read's refusal.
fn kept_naming(frame: &DecodedFrame, handle: u64) -> Result<TemplateNaming, KeptReason> {
    // In the naming's order: its dictionaries, its dictionary row, then its icons. A missing
    // row is refused before the icons are read, in the naming's own words.
    let dictionaries = match frame.complete::<tables::DivisionTemplates>() {
        Ok(rows) => rows,
        Err(error) => {
            return Err(KeptReason::unreadable(
                "",
                frame,
                TableId::DivisionTemplates,
                &error,
            ));
        }
    };
    if dictionaries
        .binary_search_by_key(&handle, |row| row.template)
        .is_ok()
        && let Err(error) = frame.complete::<tables::DivisionTemplateIcons>()
    {
        return Err(KeptReason::unreadable(
            "",
            frame,
            TableId::DivisionTemplateIcons,
            &error,
        ));
    }
    army_content::template_naming(frame, handle).map_err(KeptReason::literal)
}

/// A template's artwork operands, a failed read of its icons kept as that read's refusal.
fn kept_artwork(frame: &DecodedFrame, handle: u64) -> Result<TemplateArtwork, KeptReason> {
    if let Err(error) = frame.complete::<tables::DivisionTemplateIcons>() {
        return Err(KeptReason::unreadable(
            army_content::ARTWORK_UNAVAILABLE,
            frame,
            TableId::DivisionTemplateIcons,
            &error,
        ));
    }
    army_content::template_artwork(frame, handle).map_err(KeptReason::literal)
}

/// What `answer` gives for a kept operand: a kept refusal carried into its words.
fn answered<T: Clone, U>(
    operand: &Result<T, KeptReason>,
    answer: impl FnOnce(&Result<T, String>) -> Result<U, String>,
) -> (Option<U>, Option<KeptReason>) {
    // Stands for the kept refusal in this call alone, which words nothing else of its own.
    const KEPT: &str = "\u{0}kept refusal\u{0}";
    match answer(&operand.clone().map_err(|_| KEPT.to_owned())) {
        Ok(value) => (Some(value), None),
        Err(text) => match (operand, text.split_once(KEPT)) {
            (Err(kept), Some((before, after))) => {
                let mut text = kept.text.clone();
                text[0] = format!("{before}{}", text[0]);
                let last = text.len() - 1;
                text[last].push_str(after);
                (
                    None,
                    Some(KeptReason {
                        text,
                        tables: kept.tables.clone(),
                    }),
                )
            }
            _ => (None, Some(KeptReason::literal(text))),
        },
    }
}

/// A template's name from one of its naming operands.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryName {
    pub name: Option<String>,
    pub name_missing: Option<KeptReason>,
}

/// A template's icon from one of its artwork operands.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryIcon {
    pub icon: Option<String>,
    pub icon_missing: Option<KeptReason>,
}

/// A refusal counted at hours, worded for each.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRefusal {
    pub reason: KeptReason,
    pub times: usize,
}

/// Hours' counted refusals, and their line for artwork drawn in the recording's style. A
/// reply words, counts and orders the refusals for its own hour, as the reached product does.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryWarnings {
    pub first_hour: i32,
    pub last_hour: i32,
    pub refusals: Vec<HistoryRefusal>,
    pub fallback: Option<String>,
}

/// One country's history in a window; names lack their `TAG|mode|` part.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionEquipmentHistoryReply {
    pub branch_id: String,
    pub tag: String,
    pub count_for: ArmyAttribution,
    pub window: KeyedWindow,
    pub kinds: Vec<HistoryKind>,
    pub namings: BTreeMap<String, HistoryName>,
    pub artworks: BTreeMap<String, HistoryIcon>,
    pub designs: BTreeMap<String, Option<String>>,
    pub hours: Vec<HistoryWarnings>,
    /// The counted refusals of an hour the country does not exist.
    pub absent: Vec<String>,
    /// The lines every served reply adds, as of serving ([`served_lines`]).
    pub served: Vec<String>,
    pub warnings: Vec<String>,
}
dashboard_base::reply_warnings!(DivisionEquipmentHistoryReply);

impl Product for DivisionEquipmentHistory {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    type Params = DivisionEquipmentHistoryParams;
    type Reply = DivisionEquipmentHistoryReply;
    fn types() -> Vec<String> {
        vec![
            DivisionEquipmentHistoryParams::decl(),
            DivisionEquipmentHistoryReply::decl(),
            HistoryKind::decl(),
            HistoryName::decl(),
            HistoryIcon::decl(),
            KeptReason::decl(),
            HistoryRefusal::decl(),
            HistoryWarnings::decl(),
        ]
    }
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        dashboard_base::products::validate_tag(&p.tag)?;
        dashboard_keyed::query::validate_window(&p.branch_id, p.first_hour, p.last_hour)
    }
}

/// The lines a served reply adds to its own, as the reached product's server adds them.
pub fn served_lines(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
) -> Vec<String> {
    let mut lines = Vec::new();
    source.append_content_warnings(&mut lines);
    source.append_presentation_warnings(&mut lines, content);
    lines
}

fn mode(attribution: ArmyAttribution) -> &'static str {
    match attribution {
        ArmyAttribution::Owner => "owner",
        ArmyAttribution::Host => "host",
    }
}

/// Values numbered in the order first seen, found again by their text.
#[derive(Serialize, Deserialize)]
#[serde(bound = "T: Serialize + serde::de::DeserializeOwned")]
struct Numbered<T> {
    values: Vec<T>,
    #[serde(skip)]
    index: HashMap<String, usize>,
}

impl<T> Default for Numbered<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            index: HashMap::new(),
        }
    }
}

impl<T: Serialize> Numbered<T> {
    /// The value's number, and the text bytes it added when new.
    fn number(&mut self, value: T) -> io::Result<(usize, usize)> {
        if self.index.len() != self.values.len() {
            self.index = self
                .values
                .iter()
                .enumerate()
                .map(|(at, value)| Ok((serde_json::to_string(value)?, at)))
                .collect::<io::Result<_>>()?;
        }
        let text = serde_json::to_string(&value)?;
        if let Some(&at) = self.index.get(&text) {
            return Ok((at, 0));
        }
        let added = text.len();
        self.values.push(value);
        self.index.insert(text, self.values.len() - 1);
        Ok((self.values.len() - 1, added))
    }

    fn held_bytes(&self) -> usize {
        self.index.keys().map(|text| 2 * text.len() + 48).sum()
    }
}

/// What a query names the history's operands from.
#[derive(Default)]
pub struct Catalogued {
    names: Vec<String>,
    kinds: Numbered<Option<String>>,
    namings: Numbered<Result<TemplateNaming, KeptReason>>,
    artworks: Numbered<Result<TemplateArtwork, KeptReason>>,
    designs: Numbered<RawVariant>,
    notes: Numbered<KeptReason>,
}

/// How much of the catalogue earlier flushes published, in pages and entries, and the last
/// hour they flushed.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Published {
    pages: u64,
    counts: [usize; 6],
    through: Option<i32>,
}

/// Operand entries a page keeps together, read at once by a query.
const CHUNK: usize = 256;

/// The front of a page: where its entries are, so a query reads only those its window names.
#[derive(Default, Serialize, Deserialize)]
struct Directory {
    /// The last hour flushed before the page: its names first hold after it.
    after: Option<i32>,
    /// The catalogue's counts before the page.
    from: [usize; 6],
    /// Each tag's new names (`*` for the hour's own), sorted by tag, as spans of the body.
    tags: Vec<(String, u64, u64)>,
    /// Each operand list's new entries in chunks of [`CHUNK`], as spans of the body.
    chunks: [Vec<(u64, u64)>; 5],
}

/// The tag a value's name belongs to: its first part.
fn name_tag(name: &str) -> &str {
    name.split('|').next().unwrap_or_default()
}

/// Append `value` to `body`, as the span it takes there.
fn span<T: Serialize + ?Sized>(body: &mut Vec<u8>, value: &T) -> io::Result<(u64, u64)> {
    let at = body.len();
    serde_json::to_writer(&mut *body, value)?;
    Ok((at as u64, (body.len() - at) as u64))
}

/// Append `values` to `body` in chunks of [`CHUNK`], listing their spans in `spans`.
fn chunked<T: Serialize>(
    body: &mut Vec<u8>,
    values: &[T],
    spans: &mut Vec<(u64, u64)>,
) -> io::Result<()> {
    for chunk in values.chunks(CHUNK) {
        spans.push(span(body, chunk)?);
    }
    Ok(())
}

fn damaged() -> io::Error {
    io::Error::other("A division equipment catalogue page is damaged")
}

/// A page's directory, and where its body starts.
fn directory(range: &mut store::Range) -> io::Result<(Directory, u64)> {
    use io::Read;
    let mut length = [0; 8];
    range.read_exact(&mut length)?;
    let length = u64::from_le_bytes(length);
    if length > range.len().saturating_sub(8) {
        return Err(damaged());
    }
    let mut header = Vec::new();
    io::Read::take(&mut *range, length).read_to_end(&mut header)?;
    read_bytes(8 + length);
    Ok((serde_json::from_slice(&header)?, 8 + length))
}

/// The value one span of a page's body holds.
fn segment<T: serde::de::DeserializeOwned>(
    range: &mut store::Range,
    start: u64,
    (at, length): (u64, u64),
) -> io::Result<T> {
    use io::{Read, Seek};
    let from = start.checked_add(at).ok_or_else(damaged)?;
    if from.checked_add(length).is_none_or(|end| end > range.len()) {
        return Err(damaged());
    }
    range.seek(io::SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    range.take(length).read_to_end(&mut bytes)?;
    read_bytes(length);
    Ok(serde_json::from_slice(&bytes)?)
}

impl Catalogued {
    fn counts(&self) -> [usize; 6] {
        [
            self.names.len(),
            self.kinds.values.len(),
            self.namings.values.len(),
            self.artworks.values.len(),
            self.designs.values.len(),
            self.notes.values.len(),
        ]
    }

    /// The page of the entries added after `from`, as its flush writes it; none when there are
    /// none. Its directory leads, so a query reads one country's names and the operand
    /// chunks its window names without the rest.
    fn page(&self, from: [usize; 6], after: Option<i32>) -> io::Result<Option<Vec<u8>>> {
        if self.counts() == from {
            return Ok(None);
        }
        let mut tags: BTreeMap<&str, Vec<(u64, &str)>> = BTreeMap::new();
        for (at, name) in self.names.iter().enumerate().skip(from[0]) {
            tags.entry(name_tag(name))
                .or_default()
                .push((FIRST + at as u64, name));
        }
        let mut body = Vec::new();
        let mut directory = Directory {
            after,
            from,
            ..Directory::default()
        };
        for (tag, names) in &tags {
            let (at, length) = span(&mut body, names)?;
            directory.tags.push(((*tag).to_owned(), at, length));
        }
        let [kinds, namings, artworks, designs, notes] = &mut directory.chunks;
        chunked(&mut body, &self.kinds.values[from[1]..], kinds)?;
        chunked(&mut body, &self.namings.values[from[2]..], namings)?;
        chunked(&mut body, &self.artworks.values[from[3]..], artworks)?;
        chunked(&mut body, &self.designs.values[from[4]..], designs)?;
        chunked(&mut body, &self.notes.values[from[5]..], notes)?;
        let header = serde_json::to_vec(&directory)?;
        let mut page = Vec::with_capacity(8 + header.len() + body.len());
        page.extend_from_slice(&(header.len() as u64).to_le_bytes());
        page.extend_from_slice(&header);
        page.extend_from_slice(&body);
        Ok(Some(page))
    }

    /// The whole catalogue `read` gives as its first `pages` pages.
    fn read(
        mut read: impl FnMut(u64) -> io::Result<Option<store::Range>>,
        pages: u64,
    ) -> io::Result<Self> {
        fn extend<T: serde::de::DeserializeOwned>(
            range: &mut store::Range,
            start: u64,
            spans: &[(u64, u64)],
            into: &mut Vec<T>,
        ) -> io::Result<()> {
            for &span in spans {
                into.extend(segment::<Vec<T>>(range, start, span)?);
            }
            Ok(())
        }
        let mut catalogued = Self::default();
        for at in 0..pages {
            let mut range = read(at)?.filter(|range| !range.is_empty()).ok_or_else(|| {
                io::Error::other("A division equipment catalogue page is missing")
            })?;
            let (directory, start) = directory(&mut range)?;
            if directory.from != catalogued.counts() {
                return Err(damaged());
            }
            let mut names = Vec::new();
            for &(_, at, length) in &directory.tags {
                names.extend(segment::<Vec<(u64, String)>>(
                    &mut range,
                    start,
                    (at, length),
                )?);
            }
            names.sort_unstable_by_key(|(key, _)| *key);
            for (key, name) in names {
                if key != FIRST + catalogued.names.len() as u64 {
                    return Err(damaged());
                }
                catalogued.names.push(name);
            }
            let [kinds, namings, artworks, designs, notes] = &directory.chunks;
            extend(&mut range, start, kinds, &mut catalogued.kinds.values)?;
            extend(&mut range, start, namings, &mut catalogued.namings.values)?;
            extend(&mut range, start, artworks, &mut catalogued.artworks.values)?;
            extend(&mut range, start, designs, &mut catalogued.designs.values)?;
            extend(&mut range, start, notes, &mut catalogued.notes.values)?;
        }
        Ok(catalogued)
    }
}

/// The pages a query reads, by their directories.
struct Pages {
    pages: Vec<(store::Range, Directory, u64)>,
}

impl Pages {
    /// The pages whose entries first hold by `last`; a later page names only later hours.
    fn open(session: &store::Session, branch: u64, last: i32) -> io::Result<Self> {
        let section = catalogue_section(branch);
        let mut pages = Vec::new();
        for at in 0.. {
            let Some(mut range) = session.read(&section, at)? else {
                break;
            };
            if range.is_empty() {
                break;
            }
            let (directory, start) = directory(&mut range)?;
            if directory.after.is_some_and(|after| after >= last) {
                break;
            }
            pages.push((range, directory, start));
        }
        Ok(Self { pages })
    }

    /// `tag`'s names and the hour's own, with their keys.
    fn names(&mut self, tag: &str) -> io::Result<Vec<(u64, String)>> {
        let mut names = Vec::new();
        for (range, directory, start) in &mut self.pages {
            for wanted in [tag, "*"] {
                if let Ok(at) = directory
                    .tags
                    .binary_search_by(|(listed, _, _)| listed.as_str().cmp(wanted))
                {
                    let (_, offset, length) = directory.tags[at];
                    names.extend(segment::<Vec<(u64, String)>>(
                        range,
                        *start,
                        (offset, length),
                    )?);
                }
            }
        }
        Ok(names)
    }

    /// Operand list `list`'s entries `ids`, each read with its chunk alone.
    fn entries<T: serde::de::DeserializeOwned>(
        &mut self,
        list: usize,
        ids: &BTreeSet<usize>,
    ) -> io::Result<BTreeMap<usize, T>> {
        let unknown = || io::Error::other("Unknown division equipment history operand");
        let mut chunks = BTreeSet::new();
        for &id in ids {
            let page = self
                .pages
                .partition_point(|(_, directory, _)| directory.from[list + 1] <= id)
                .checked_sub(1)
                .ok_or_else(unknown)?;
            chunks.insert((page, (id - self.pages[page].1.from[list + 1]) / CHUNK));
        }
        let mut found = BTreeMap::new();
        for (page, chunk) in chunks {
            let (range, directory, start) = &mut self.pages[page];
            let span = *directory.chunks[list].get(chunk).ok_or_else(unknown)?;
            let first = directory.from[list + 1] + chunk * CHUNK;
            let values: Vec<T> = segment(range, *start, span)?;
            for (at, value) in values.into_iter().enumerate() {
                if ids.contains(&(first + at)) {
                    found.insert(first + at, value);
                }
            }
        }
        if found.len() != ids.len() {
            return Err(unknown());
        }
        Ok(found)
    }
}

/// The operands a window's runs name, read from their pages' chunks.
#[derive(Default)]
struct Operands {
    kinds: BTreeMap<usize, Option<String>>,
    namings: BTreeMap<usize, Result<TemplateNaming, KeptReason>>,
    artworks: BTreeMap<usize, Result<TemplateArtwork, KeptReason>>,
    designs: BTreeMap<usize, RawVariant>,
    notes: BTreeMap<usize, KeptReason>,
}

impl Operands {
    fn read(pages: &mut Pages, window: &KeyedWindow) -> io::Result<Self> {
        let number = |text: &str| {
            text.parse::<usize>()
                .map_err(|_| io::Error::other("Invalid history operand"))
        };
        let mut wanted: [BTreeSet<usize>; 5] = Default::default();
        for series in &window.series {
            let parts: Vec<&str> = series.name.split('|').collect();
            let values = || series.runs.iter().map(|run| number(&run.2));
            match parts.as_slice() {
                ["note", note] => {
                    wanted[4].insert(number(note)?);
                }
                ["template", _, "naming"] => {
                    for id in values() {
                        wanted[1].insert(id?);
                    }
                }
                ["template", _, "artwork"] => {
                    for id in values() {
                        wanted[2].insert(id?);
                    }
                }
                ["type", _, "design", _] | ["stockDesign", _] => {
                    for id in values() {
                        wanted[3].insert(id?);
                    }
                }
                _ => {}
            }
            if let ["type", kind, ..] = parts.as_slice() {
                wanted[0].insert(number(kind)?);
            }
        }
        Ok(Self {
            kinds: pages.entries(0, &wanted[0])?,
            namings: pages.entries(1, &wanted[1])?,
            artworks: pages.entries(2, &wanted[2])?,
            designs: pages.entries(3, &wanted[3])?,
            notes: pages.entries(4, &wanted[4])?,
        })
    }
}

/// What a continuation's counts after the flush and its compression frame can add to its JSON text.
const CONTINUATION_SLACK: usize = 4096;

/// The section a branch's catalogue pages are appended to.
pub fn catalogue_section(branch: u64) -> String {
    format!("{NAME}-catalogue-pages-{branch}")
}

/// The latest supplying read of one table, and whether it is proved the latest within a day.
#[derive(Default)]
struct Supply {
    read: Option<Arc<DecodedFrame>>,
    known: bool,
}

impl Supply {
    fn fresh() -> Self {
        Self {
            read: None,
            known: true,
        }
    }

    /// A complete read starts a carry of `kept`; an incomplete table or a day without a read ends it.
    fn step(
        &mut self,
        frame: &Arc<DecodedFrame>,
        table: TableId,
        kept: &[TableId],
        format: FormatVersion,
        resumed: Option<i32>,
    ) {
        let hour = frame.hour;
        if !matches!(&*frame.tables[table.index()], TableView::Complete(_)) {
            *self = Self::fresh();
            return;
        }
        if dashboard_text::metadata::slot_read(format, &frame.metrics, table, hour) {
            *self = Self {
                read: Some(Arc::new(self::kept(frame, kept))),
                known: true,
            };
            return;
        }
        if self
            .read
            .as_ref()
            .is_some_and(|read| i64::from(hour) - i64::from(read.hour) >= i64::from(CADENCE))
        {
            self.read = None;
        }
        if !self.known
            && resumed
                .is_some_and(|first| i64::from(hour) - i64::from(first) >= i64::from(CADENCE - 1))
        {
            self.known = true;
        }
    }
}

/// The hour's frame holding only `kept`, so a carry keeps alive no table it does not read.
fn kept(frame: &DecodedFrame, kept: &[TableId]) -> DecodedFrame {
    let unread = SharedView::from(TableView::Unavailable {
        reason: "Not kept".into(),
    });
    let mut frame = frame.clone();
    for table in TableId::ALL {
        if !kept.contains(&table) {
            frame.tables[table.index()] = unread.clone();
        }
    }
    frame
}

/// The tables an hour's changes are measured on against the hour before.
const MEASURED: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::CountryLocalSubunits,
    TableId::Divisions,
    TableId::DivisionStatus,
    TableId::DivisionLandInputs,
    TableId::NewDivisionEquipment,
    TableId::DivisionTemplateIcons,
    TableId::DivisionTemplates,
];

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Rows this thread's observations compared or listed, to prove a quiet hour lists none.
    pub(crate) static SCANNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Catalogue bytes this thread's queries read.
    pub(crate) static CATALOGUE_READ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Count `bytes` of the catalogue read.
fn read_bytes(bytes: u64) {
    #[cfg(any(test, feature = "test-support"))]
    CATALOGUE_READ.set(CATALOGUE_READ.get() + bytes);
    let _ = bytes;
}

/// Count `rows` compared or listed by an observation.
fn scanned(rows: usize) {
    #[cfg(any(test, feature = "test-support"))]
    SCANNED.set(SCANNED.get() + rows);
    let _ = rows;
}

/// The rows a table's view holds.
fn rows_in(view: &TableView) -> usize {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
        TableView::Unavailable { .. } | TableView::NotRecorded { .. } => 0,
    }
}

/// Where the hour's rebuild goes: every country, or those whose canonical slots changed.
enum Dirty {
    All,
    Slots(BTreeSet<u32>),
}

impl Dirty {
    fn contains(&self, slot: u32) -> bool {
        match self {
            Self::All => true,
            Self::Slots(slots) => slots.contains(&slot),
        }
    }

    fn slots(&mut self, slots: impl IntoIterator<Item = u32>) {
        if let Self::Slots(dirty) = self {
            dirty.extend(slots);
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Numeric {
    hp: Option<i128>,
    hp_max: Option<i128>,
    manpower: Option<i64>,
}

impl Numeric {
    fn read(frame: &DecodedFrame, id: u64) -> Option<Self> {
        let row = super::find(frame.complete::<tables::Divisions>().ok(), id, |row| row.id)?;
        Some(Self {
            hp: Some(i128::from(row.hit_points)),
            hp_max: super::find(frame.complete::<tables::DivisionStatus>().ok(), id, |row| {
                row.division
            })
            .map(|row| i128::from(row.maximum_hit_points)),
            manpower: super::find(
                frame.complete::<tables::DivisionLandInputs>().ok(),
                id,
                |row| row.division,
            )
            .map(|row| i64::from(manpower_sum(&row.manpower))),
        })
    }

    fn subtract(self, before: Self) -> Option<Self> {
        Some(Self {
            hp: Some(self.hp?.checked_sub(before.hp?)?),
            hp_max: Some(self.hp_max?.checked_sub(before.hp_max?)?),
            manpower: Some(self.manpower?.checked_sub(before.manpower?)?),
        })
    }

    fn add(self, delta: Self) -> Option<Self> {
        Some(Self {
            hp: Some(self.hp?.checked_add(delta.hp?)?),
            hp_max: Some(self.hp_max?.checked_add(delta.hp_max?)?),
            manpower: Some(self.manpower?.checked_add(delta.manpower?)?),
        })
    }

    fn values(self) -> Option<[(&'static str, i128); 3]> {
        Some([
            ("manpower", i128::from(self.manpower?)),
            ("hp", self.hp?),
            ("hpMax", self.hp_max?),
        ])
    }
}

#[derive(Default)]
struct NumericPatch {
    totals: BTreeMap<(u32, usize, u64), Numeric>,
    divisions: Vec<(u64, Numeric)>,
}

/// The reads carried from hour to hour, and what every country's reply shares.
#[derive(Default)]
struct Carried {
    pool: Supply,
    stock: Supply,
    designs: Supply,
    /// The designs as of the stockpile's read, and whether they were known then.
    stock_designs: (Option<Arc<DecodedFrame>>, bool),
    /// The first hour after a resumed run.
    resumed: Option<i32>,
    shared: Shared,
    /// The last hour observed known, its changes measured against.
    previous: Option<Arc<DecodedFrame>>,
    reads: [Option<usize>; 4],
    /// Each counted division's slots and template ([`Members`]), and each template's slots.
    placed: HashMap<u64, Placed>,
    contributions: HashMap<u64, Numeric>,
    totals: BTreeMap<(u32, usize, u64), Numeric>,
    users: HashMap<u64, BTreeMap<u32, usize>>,
    /// Each tag listed this hour by its holder's slot (none: no resolved holder).
    tags: BTreeMap<String, Option<u32>>,
    slots: BTreeMap<u32, Vec<String>>,
    /// Each template's naming and artwork numbers, and whether its naming holds only for its hour.
    labels: HashMap<u64, (usize, usize, bool)>,
    /// The templates whose naming holds only for its hour, named again each hour.
    hourly: BTreeSet<u64>,
    /// The links and local sub-units the needs resolve through, kept while their tables are.
    sources: Option<Arc<dashboard_text::subunit_requirements::SourceIndex>>,
    designs_numbered: HashMap<u64, Option<usize>>,
    stock_numbered: HashMap<u64, Option<usize>>,
}

impl Carried {
    fn fresh() -> Self {
        Self {
            pool: Supply::fresh(),
            stock: Supply::fresh(),
            designs: Supply::fresh(),
            stock_designs: (None, true),
            ..Self::default()
        }
    }

    fn resumed(hour: i32) -> Self {
        Self {
            stock_designs: (None, false),
            resumed: Some(hour),
            ..Self::default()
        }
    }

    fn known(&self) -> bool {
        self.pool.known
            && self.stock.known
            && self.designs.known
            && (self.stock.read.is_none() || self.stock_designs.1)
    }

    fn place(&mut self, id: u64, placed: Option<Placed>, dirty: &mut Dirty) {
        self.contributions.remove(&id);
        let members = self.shared.members.get_or_insert_with(Members::default);
        if let Some(before) = self.placed.remove(&id) {
            for (at, slot) in before.slots.into_iter().enumerate() {
                if let Some(slot) = slot {
                    self.totals.remove(&(slot, at, before.template));
                }
            }
            members.remove(id, before);
            for slot in before.slots.into_iter().flatten() {
                if let Some(users) = self.users.get_mut(&before.template)
                    && let Some(count) = users.get_mut(&slot)
                {
                    *count -= 1;
                    if *count == 0 {
                        users.remove(&slot);
                    }
                }
            }
            dirty.slots(before.slots.into_iter().flatten());
        }
        if let Some(placed) = placed {
            members.add(id, placed);
            for slot in placed.slots.into_iter().flatten() {
                *self
                    .users
                    .entry(placed.template)
                    .or_default()
                    .entry(slot)
                    .or_default() += 1;
            }
            dirty.slots(placed.slots.into_iter().flatten());
            self.placed.insert(id, placed);
        }
    }

    /// Index every counted division of the hour afresh.
    fn index(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        self.placed.clear();
        self.contributions.clear();
        self.totals.clear();
        self.users.clear();
        self.shared.members = Some(Members::default());
        let links = frame.complete::<tables::CountryLinks>()?;
        let mut none = Dirty::All;
        let rows = frame.complete::<tables::Divisions>()?;
        scanned(rows.len());
        for row in rows {
            self.place(row.id, Members::place(row, links), &mut none);
        }
        Ok(())
    }

    fn invalidate_numeric(&mut self, slot: u32, at: usize) {
        let keys: Vec<_> = self
            .totals
            .range((slot, at, 0)..=(slot, at, u64::MAX))
            .map(|(&key, _)| {
                scanned(1);
                key
            })
            .collect();
        for key in keys {
            self.totals.remove(&key);
        }
        if let Some(members) = &self.shared.members {
            for id in members.slots[at].get(&slot).into_iter().flatten() {
                self.contributions.remove(id);
            }
        }
    }

    fn seed_numeric(
        &mut self,
        frame: &DecodedFrame,
        slot: u32,
        at: usize,
        reply: &DivisionEquipmentReply,
    ) {
        self.invalidate_numeric(slot, at);
        if reply.absent {
            return;
        }
        for template in &reply.templates {
            let Ok(handle) = template.template.parse() else {
                continue;
            };
            self.totals.insert(
                (slot, at, handle),
                Numeric {
                    hp: template
                        .hp_raw
                        .as_ref()
                        .and_then(|value| value.parse().ok()),
                    hp_max: template
                        .hp_max_raw
                        .as_ref()
                        .and_then(|value| value.parse().ok()),
                    manpower: template
                        .manpower
                        .as_ref()
                        .and_then(|value| value.parse().ok()),
                },
            );
        }
        if let Some(members) = &self.shared.members {
            for &id in members.slots[at].get(&slot).into_iter().flatten() {
                if let Some(value) = Numeric::read(frame, id) {
                    self.contributions.insert(id, value);
                }
            }
        }
    }

    fn division_slots(&self, id: u64) -> impl Iterator<Item = u32> + '_ {
        self.placed
            .get(&id)
            .into_iter()
            .flat_map(|placed| placed.slots.into_iter().flatten())
    }

    fn template_slots(&self, template: u64) -> impl Iterator<Item = u32> + '_ {
        self.users
            .get(&template)
            .into_iter()
            .flat_map(|users| users.keys().copied())
    }

    fn armed(&self) -> Vec<u32> {
        let armed: Vec<u32> = self
            .shared
            .members
            .iter()
            .flat_map(|members| members.slots.iter().flat_map(|slots| slots.keys().copied()))
            .collect();
        scanned(armed.len());
        armed
    }

    /// Forget a template's labels, as when its rows change.
    fn relabel(&mut self, template: u64) {
        self.labels.remove(&template);
        self.hourly.remove(&template);
    }

    /// Which countries the hour's changes reach. The hour's divisions are indexed on the way.
    fn dirty(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        reads: &[Option<usize>; 4],
    ) -> io::Result<(Dirty, BTreeSet<u64>)> {
        let previous = self.previous.clone();
        let Some(previous) = previous.as_deref() else {
            return Ok((Dirty::All, BTreeSet::new()));
        };
        // Links, tokens, local clones and stockpile reads are shared by every reply. A block's
        // first hour marks every table changed; its rows are then compared with the hour before.
        if self.shared.members.is_none()
            || reads[1] != self.reads[1]
            || reads[3] != self.reads[3]
            || !unchanged::<tables::CountryLinks>(previous, frame, changes)
            || !unchanged::<tables::LexicalTokens>(previous, frame, changes)
            || !unchanged::<tables::CountryLocalSubunits>(previous, frame, changes)
        {
            return Ok((Dirty::All, BTreeSet::new()));
        }
        let previous = Some(previous);
        let countries = changed::<tables::Countries>(
            previous,
            frame,
            changes,
            field_mask(TableId::Countries, &["canonical_slot", "tag"]),
            |a, b| (a.canonical_slot, &a.tag) == (b.canonical_slot, &b.tag),
        );
        if countries.is_none_or(|keys| !keys.is_empty()) {
            return Ok((Dirty::All, BTreeSet::new()));
        }
        let mut dirty = Dirty::Slots(BTreeSet::new());
        let mut numeric = BTreeSet::new();
        let unlinked = self.shared.members.as_ref().map(|members| members.unlinked);
        let Some(divisions) = changed::<tables::Divisions>(
            previous,
            frame,
            changes,
            field_mask(
                TableId::Divisions,
                &[
                    "roster_owner",
                    "logical_owner",
                    "expeditionary_owner",
                    "template",
                    "hit_points",
                    "reveal_skip",
                ],
            ),
            |a, b| {
                (
                    a.roster_owner,
                    a.logical_owner,
                    a.expeditionary_owner,
                    a.template,
                    a.hit_points,
                    a.reveal_skip,
                ) == (
                    b.roster_owner,
                    b.logical_owner,
                    b.expeditionary_owner,
                    b.template,
                    b.hit_points,
                    b.reveal_skip,
                )
            },
        ) else {
            return Ok((Dirty::All, BTreeSet::new()));
        };
        let links = frame.complete::<tables::CountryLinks>()?;
        let rows = frame.complete::<tables::Divisions>()?;
        for key in divisions {
            let id = key as u64;
            let now = super::find(Some(rows), id, |row| row.id);
            let before = super::find(
                previous.and_then(|frame| frame.complete::<tables::Divisions>().ok()),
                id,
                |row| row.id,
            );
            let placed = now.and_then(|row| Members::place(row, links));
            if before.zip(now).is_some_and(|(before, now)| {
                (
                    before.roster_owner,
                    before.logical_owner,
                    before.expeditionary_owner,
                    before.template,
                    before.reveal_skip,
                ) == (
                    now.roster_owner,
                    now.logical_owner,
                    now.expeditionary_owner,
                    now.template,
                    now.reveal_skip,
                )
            }) && placed.is_some()
                && placed == self.placed.get(&id).copied()
            {
                numeric.insert(id);
            } else {
                self.place(id, placed, &mut dirty);
            }
        }
        // Every reply counts the divisions no country links.
        if self.shared.members.as_ref().map(|members| members.unlinked) != unlinked {
            return Ok((Dirty::All, BTreeSet::new()));
        }
        let by_division: [Option<BTreeSet<u128>>; 3] = [
            changed::<tables::DivisionStatus>(
                previous,
                frame,
                changes,
                field_mask(TableId::DivisionStatus, &["maximum_hit_points"]),
                |a, b| a.maximum_hit_points == b.maximum_hit_points,
            ),
            changed::<tables::DivisionLandInputs>(
                previous,
                frame,
                changes,
                field_mask(TableId::DivisionLandInputs, &["manpower"]),
                |a, b| a.manpower == b.manpower,
            ),
            changed::<tables::NewDivisionEquipment>(previous, frame, changes, u64::MAX, |a, b| {
                a == b
            })
            .map(|keys| keys.into_iter().map(|key| key >> 32).collect()),
        ];
        for (at, keys) in by_division.into_iter().enumerate() {
            let Some(keys) = keys else {
                return Ok((Dirty::All, BTreeSet::new()));
            };
            for key in keys {
                if at < 2 {
                    if self.placed.contains_key(&(key as u64)) {
                        numeric.insert(key as u64);
                    }
                } else {
                    let slots: Vec<u32> = self.division_slots(key as u64).collect();
                    dirty.slots(slots);
                }
            }
        }
        let by_template = [
            changed::<tables::DivisionTemplateIcons>(previous, frame, changes, u64::MAX, |a, b| {
                a == b
            }),
            changed::<tables::DivisionTemplates>(previous, frame, changes, u64::MAX, |a, b| a == b),
        ];
        for keys in by_template {
            let Some(keys) = keys else {
                return Ok((Dirty::All, BTreeSet::new()));
            };
            for key in keys {
                let template = key as u64;
                self.shared.needs.remove(&template);
                self.relabel(template);
                let slots: Vec<u32> = self.template_slots(template).collect();
                dirty.slots(slots);
            }
        }
        // A template named only at the hour it was sampled is named again each hour.
        let hourly: Vec<u64> = self.hourly.iter().copied().collect();
        for template in hourly {
            let slots: Vec<u32> = self.template_slots(template).collect();
            dirty.slots(slots);
        }
        if reads[2] != self.reads[2] {
            let armed = self.armed();
            dirty.slots(armed);
        }
        Ok((dirty, numeric))
    }
}

/// Keys of `T`'s rows changed this hour as `mask` or `same` tell; None when unknown.
fn changed<T: TableSchema>(
    previous: Option<&DecodedFrame>,
    frame: &DecodedFrame,
    changes: &FrameChanges,
    mask: u64,
    same: impl Fn(&T::Row, &T::Row) -> bool,
) -> Option<BTreeSet<u128>> {
    if !changes.rows_changed[T::ID.index()] {
        return Some(BTreeSet::new());
    }
    if let Some((rows, deleted)) = changes.rows(T::ID) {
        return Some(
            rows.iter()
                .filter(|row| row.mask & mask != 0)
                .map(|row| row.key)
                .chain(deleted.iter().copied())
                .collect(),
        );
    }
    let previous = previous?;
    let (Ok(before), Ok(now)) = (previous.complete::<T>(), frame.complete::<T>()) else {
        // A table unreadable alike on both sides gives every reply the same refusal, its
        // hour kept apart ([`KeptReason`]).
        let (before, now) = (
            &previous.tables[T::ID.index()],
            &frame.tables[T::ID.index()],
        );
        scanned(rows_in(before) + rows_in(now));
        return (before == now).then(BTreeSet::new);
    };
    Some(differ(before, now, same))
}

/// Whether `T`'s rows are the hour before's: told by the hour's changes, else compared.
fn unchanged<T: TableSchema>(
    previous: &DecodedFrame,
    frame: &DecodedFrame,
    changes: &FrameChanges,
) -> bool {
    let index = T::ID.index();
    if !changes.rows_changed[index] {
        return true;
    }
    if let Some((rows, deleted)) = changes.rows(T::ID) {
        return rows.is_empty() && deleted.is_empty();
    }
    scanned(rows_in(&previous.tables[index]) + rows_in(&frame.tables[index]));
    previous.tables[index] == frame.tables[index]
}

/// Keys of rows only one side holds, or that `same` tells apart; both sides key-ordered.
fn differ<R: RowCodec>(before: &[R], now: &[R], same: impl Fn(&R, &R) -> bool) -> BTreeSet<u128> {
    scanned(before.len() + now.len());
    let mut keys = BTreeSet::new();
    let (mut a, mut b) = (0, 0);
    while a < before.len() || b < now.len() {
        match (before.get(a), now.get(b)) {
            (Some(old), Some(new)) if old.key() == new.key() => {
                if !same(old, new) {
                    keys.insert(new.key());
                }
                a += 1;
                b += 1;
            }
            (Some(old), new) if new.is_none_or(|new| old.key() < new.key()) => {
                keys.insert(old.key());
                a += 1;
            }
            (_, Some(new)) => {
                keys.insert(new.key());
                b += 1;
            }
            _ => break,
        }
    }
    keys
}

/// The divisions whose pool or census membership differ between two pool reads.
fn pool_changes(before: &DecodedFrame, now: &DecodedFrame) -> Option<BTreeSet<u64>> {
    let rows = differ(
        before.complete::<tables::DivisionEquipment>().ok()?,
        now.complete::<tables::DivisionEquipment>().ok()?,
        |a, b| a == b,
    );
    let mut divisions: BTreeSet<u64> = rows.into_iter().map(|key| (key >> 32) as u64).collect();
    // Whether a division is in the read's census decides an unlisted pool.
    match (
        before.complete::<tables::Divisions>(),
        now.complete::<tables::Divisions>(),
    ) {
        (Ok(before), Ok(now)) => divisions.extend(
            differ(before, now, |_, _| true)
                .into_iter()
                .map(|key| key as u64),
        ),
        (Err(_), Err(_)) => {}
        _ => return None,
    }
    Some(divisions)
}

/// One branch's history as preparation carries it. Its continuation is the open state; the
/// catalogue is published as appended pages and read back from them ([`Prepared::restore`]).
#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    #[serde(skip)]
    catalogued: Catalogued,
    published: Published,
    /// Each country's published values by tag (`*` for the hour's own).
    owned: BTreeMap<String, BTreeMap<u64, i128>>,
    last_hour: Option<i32>,
    open: bool,
    batch: Accumulator,
    #[serde(skip)]
    index: HashMap<String, u64>,
    #[serde(skip)]
    carried: Option<Box<Carried>>,
    #[serde(skip)]
    cat: Option<Option<Arc<Catalogue>>>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub(crate) builds: usize,
    #[cfg(test)]
    #[serde(skip)]
    pub(crate) reference: bool,
}

impl Prepared {
    /// A branch with nothing prepared yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the content every hour's replies read.
    pub fn configure(&mut self, source: &Source, context: &LoadContext) {
        if self.cat.is_none() {
            self.cat = Some(army_content::content(source, context).ok());
        }
    }

    /// Bytes its next commit publishes and releases: the pending runs and the new catalogue page.
    pub fn bytes(&self) -> usize {
        let page = self
            .catalogued
            .page(self.published.counts, self.published.through)
            .map_or(usize::MAX, |page| page.map_or(0, |page| page.len()));
        self.batch.bytes().saturating_add(page)
    }

    /// What the branch state keeps after a flush, its continuation (written at every commit) included.
    pub fn held_bytes(&self) -> usize {
        let catalogued = &self.catalogued;
        let names: usize = catalogued
            .names
            .iter()
            .map(|name| 2 * name.len() + 64)
            .sum();
        let numbered = catalogued.kinds.held_bytes()
            + catalogued.namings.held_bytes()
            + catalogued.artworks.held_bytes()
            + catalogued.designs.held_bytes()
            + catalogued.notes.held_bytes();
        let owned: usize = self
            .owned
            .iter()
            .map(|(tag, values)| tag.len() + 48 + values.len() * 48)
            .sum();
        let carried = self.carried.as_ref().map_or(0, |carried| {
            let mut frames = dashboard_base::prepared::prepass::HeldBytes::default();
            for read in [
                &carried.pool.read,
                &carried.stock.read,
                &carried.designs.read,
                &carried.stock_designs.0,
                &carried.previous,
            ]
            .into_iter()
            .flatten()
            {
                frames.frame(read);
            }
            frames.total() as usize
                + carried.placed.len() * 48
                + carried.contributions.len() * (size_of::<(u64, Numeric)>() + 32)
                + carried.totals.len() * (size_of::<((u32, usize, u64), Numeric)>() + 48)
                + carried
                    .users
                    .values()
                    .map(|users| 48 + users.len() * 24)
                    .sum::<usize>()
                + carried.tags.keys().map(|tag| tag.len() + 48).sum::<usize>()
                + carried.labels.len() * 40
                + carried.hourly.len() * 32
                + carried
                    .sources
                    .as_ref()
                    .map_or(0, |index| 64 + index.len() * 96)
                + (carried.designs_numbered.len() + carried.stock_numbered.len()) * 32
                + carried.shared.needs.len() * 64
        });
        // The continuation is saved compressed; its JSON text bounds it.
        let continuation = serde_json::to_vec(self).map_or(usize::MAX, |text| text.len());
        (names + numbered + owned + carried)
            .saturating_add(continuation)
            .saturating_add(CONTINUATION_SLACK)
    }

    fn number(&mut self, name: &str) -> u64 {
        if self.index.len() != self.catalogued.names.len() {
            self.index = self
                .catalogued
                .names
                .iter()
                .enumerate()
                .map(|(at, name)| (name.clone(), FIRST + at as u64))
                .collect();
        }
        if let Some(&key) = self.index.get(name) {
            return key;
        }
        let key = FIRST + self.catalogued.names.len() as u64;
        self.catalogued.names.push(name.to_owned());
        self.index.insert(name.to_owned(), key);
        key
    }

    /// One accepted hour of the branch, with its changes from the hour before.
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        replay: &Replay,
    ) -> io::Result<()> {
        let hour = frame.hour;
        self.batch.advance(hour)?;
        let continuous = self.last_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        let in_gap = replay.gaps.iter().any(|gap| {
            gap.branch == frame.branch && gap.first_hour <= hour && hour <= gap.last_hour
        });
        // Only the branch's first hour, or one after a gap or a missing hour, reads nothing before it.
        let branch_start = replay
            .index
            .iter()
            .find(|block| block.branch == frame.branch)
            .is_some_and(|block| block.first_hour == hour);
        let mut carried = match (self.carried.take(), self.last_hour) {
            (Some(carried), _) if continuous && !in_gap => carried,
            _ if in_gap => Box::new(Carried::fresh()),
            (_, Some(_)) if !continuous => Box::new(Carried::fresh()),
            (_, None) if branch_start => Box::new(Carried::fresh()),
            _ => Box::new(Carried::resumed(hour)),
        };
        if in_gap {
            self.carried = Some(carried);
            return self.unknown(hour);
        }
        let frame = Arc::new(frame.clone());
        let format = replay.format;
        let resumed = carried.resumed;
        let pool_before = carried.pool.read.clone();
        carried.designs.step(
            &frame,
            TableId::VariantDefinitions,
            DESIGNS,
            format,
            resumed,
        );
        let stock_before = carried.stock.read.as_ref().map(Arc::as_ptr);
        carried
            .stock
            .step(&frame, TableId::Stockpiles, STOCK, format, resumed);
        if carried.stock.read.as_ref().map(Arc::as_ptr) != stock_before {
            carried.stock_designs = (carried.designs.read.clone(), carried.designs.known);
        }
        carried
            .pool
            .step(&frame, TableId::DivisionEquipment, POOLS, format, resumed);
        let result = self.observe_hour(&frame, changes, &mut carried, continuous, pool_before);
        self.carried = Some(carried);
        result
    }

    fn observe_hour(
        &mut self,
        frame: &Arc<DecodedFrame>,
        changes: &FrameChanges,
        carried: &mut Carried,
        continuous: bool,
        pool_before: Option<Arc<DecodedFrame>>,
    ) -> io::Result<()> {
        let hour = frame.hour;
        if !carried.known() {
            carried.previous = None;
            carried.contributions.clear();
            carried.totals.clear();
            return self.unknown(hour);
        }
        // The tables every reply reads its countries and divisions from, without reading them.
        if frame.complete::<tables::Countries>().is_err()
            || frame.complete::<tables::CountryLinks>().is_err()
            || frame.complete::<tables::Divisions>().is_err()
        {
            carried.previous = None;
            carried.contributions.clear();
            carried.totals.clear();
            return self.unknown(hour);
        }
        let starts = !self.open || !continuous;
        let reads = [
            carried
                .pool
                .read
                .as_ref()
                .map(|read| Arc::as_ptr(read) as usize),
            carried
                .stock
                .read
                .as_ref()
                .map(|read| Arc::as_ptr(read) as usize),
            carried
                .designs
                .read
                .as_ref()
                .map(|read| Arc::as_ptr(read) as usize),
            carried
                .stock_designs
                .0
                .as_ref()
                .map(|read| Arc::as_ptr(read) as usize),
        ];
        let (mut dirty, numeric) = if starts {
            (Dirty::All, BTreeSet::new())
        } else {
            carried.dirty(frame, changes, &reads)?
        };
        // A pool read differing from the last only for some divisions rebuilds their countries.
        if !matches!(dirty, Dirty::All) && reads[0] != carried.reads[0] {
            let divisions = match (pool_before.as_deref(), carried.pool.read.as_deref()) {
                (Some(before), Some(now)) => pool_changes(before, now),
                _ => None,
            };
            let slots: Vec<u32> = match divisions {
                Some(divisions) => divisions
                    .into_iter()
                    .flat_map(|id| carried.division_slots(id).collect::<Vec<_>>())
                    .collect(),
                None => carried.armed(),
            };
            dirty.slots(slots);
        }
        #[cfg(test)]
        if self.reference {
            for &id in &numeric {
                dirty.slots(carried.division_slots(id));
            }
        }
        let patch = self.stage_numeric(frame, carried, &numeric, &mut dirty);
        if reads[2] != carried.reads[2] {
            carried.shared.designs.clear();
            carried.designs_numbered.clear();
        }
        if reads[3] != carried.reads[3] {
            carried.shared.stock_designs.clear();
            carried.stock_numbered.clear();
        }
        if reads[1] != carried.reads[1] || carried.shared.stock.is_none() {
            carried.shared.stock = carried.stock.read.as_deref().map(Shared::group_stock);
            carried.shared.stock_ids = carried
                .stock
                .read
                .as_deref()
                .map(army_content::read_identities);
        }
        carried.reads = reads;
        carried.previous = Some(Arc::new(kept(frame, MEASURED)));
        if matches!(dirty, Dirty::All) {
            carried.index(frame)?;
            carried.shared.needs.clear();
            carried.labels.clear();
            carried.hourly.clear();
            carried.sources = None;
            let holders = army_content::holders(frame)?;
            scanned(holders.len());
            carried.tags.clear();
            carried.slots.clear();
            for (tag, holder) in &holders {
                let Ok(tag) = String::from_utf8(tag.clone()) else {
                    continue;
                };
                if dashboard_base::products::validate_tag(&tag).is_err() {
                    continue;
                }
                if let Some(holder) = holder {
                    carried
                        .slots
                        .entry(holder.slot)
                        .or_default()
                        .push(tag.clone());
                }
                carried.tags.insert(tag, holder.map(|holder| holder.slot));
            }
            carried.shared.holders = Some(holders);
        }
        for (&(slot, at, template), &total) in &patch.totals {
            if dirty.contains(slot) {
                continue;
            }
            let published = (|| {
                let values = total.values()?;
                let attribution = [ArmyAttribution::Owner, ArmyAttribution::Host]
                    .get(at)
                    .copied()?;
                for tag in carried.slots.get(&slot)? {
                    let values = values.map(|(measure, value)| {
                        let name = format!(
                            "{tag}|{}|template|{template}|{measure}",
                            mode(attribution)
                        );
                        (name, value)
                    });
                    self.publish_sparse(tag, values, hour)?;
                }
                Some(())
            })();
            if published.is_some() {
                carried.totals.insert((slot, at, template), total);
            } else {
                dirty.slots([slot]);
            }
        }
        let rebuilt: Vec<String> = match &dirty {
            Dirty::All => {
                scanned(carried.tags.len());
                carried.tags.keys().cloned().collect()
            }
            Dirty::Slots(slots) => slots
                .iter()
                .flat_map(|slot| carried.slots.get(slot).into_iter().flatten().cloned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        };
        let pool = carried.pool.read.as_ref().map(|read| read.hour);
        let mut global = Vec::new();
        if let Some(hour) = pool {
            global.push(("*|pool".to_owned(), i128::from(hour)));
        }
        self.publish("*", global, hour, starts);
        if !rebuilt.is_empty() {
            self.rebuild(frame, carried, &rebuilt, hour, starts)?;
        }
        for (id, value) in patch.divisions {
            if carried.division_slots(id).any(|slot| !dirty.contains(slot)) {
                carried.contributions.insert(id, value);
            }
        }
        if matches!(dirty, Dirty::All) {
            scanned(self.owned.len());
            let gone: Vec<String> = self
                .owned
                .keys()
                .filter(|tag| *tag != "*" && !carried.tags.contains_key(*tag))
                .cloned()
                .collect();
            for tag in gone {
                self.publish(&tag, Vec::new(), hour, false);
                self.owned.remove(&tag);
            }
        }
        if starts {
            self.batch.observe(KNOWN, hour, Some(1), true);
        }
        self.batch.through(hour);
        self.last_hour = Some(hour);
        self.open = true;
        Ok(())
    }

    fn stage_numeric(
        &self,
        frame: &DecodedFrame,
        carried: &Carried,
        candidates: &BTreeSet<u64>,
        dirty: &mut Dirty,
    ) -> NumericPatch {
        let mut patch = NumericPatch::default();
        for &id in candidates {
            let Some(placed) = carried.placed.get(&id) else {
                *dirty = Dirty::All;
                break;
            };
            if placed
                .slots
                .into_iter()
                .flatten()
                .all(|slot| dirty.contains(slot))
            {
                continue;
            }
            let staged = (|| {
                let previous = carried.previous.as_deref()?;
                let before = Numeric::read(previous, id)?;
                let now = Numeric::read(frame, id)?;
                if carried.contributions.get(&id) != Some(&before)
                    || Members::place(
                        super::find(frame.complete::<tables::Divisions>().ok(), id, |row| row.id)?,
                        frame.complete::<tables::CountryLinks>().ok()?,
                    ) != Some(*placed)
                    || placed.slots.iter().enumerate().any(|(at, slot)| {
                        slot.is_some_and(|slot| {
                            !carried.shared.members.as_ref().is_some_and(|members| {
                                members.slots[at]
                                    .get(&slot)
                                    .is_some_and(|ids| ids.contains(&id))
                            })
                        })
                    })
                {
                    return None;
                }
                Some((now, now.subtract(before)?))
            })();
            let Some((now, delta)) = staged else {
                dirty.slots(placed.slots.into_iter().flatten());
                continue;
            };
            patch.divisions.push((id, now));
            for (at, slot) in placed.slots.into_iter().enumerate() {
                let Some(slot) = slot.filter(|slot| !dirty.contains(*slot)) else {
                    continue;
                };
                let key = (slot, at, placed.template);
                let delta = match patch.totals.get(&key) {
                    Some(before) => before.add(delta),
                    None => Some(delta),
                };
                match delta {
                    Some(delta) => {
                        patch.totals.insert(key, delta);
                    }
                    None => dirty.slots([slot]),
                }
            }
        }
        patch.totals.retain(|&(slot, at, template), delta| {
            if dirty.contains(slot) {
                return false;
            }
            let total = (|| {
                let before = *carried.totals.get(&(slot, at, template))?;
                let total = before.add(*delta)?;
                let aliases = carried.slots.get(&slot).filter(|tags| !tags.is_empty())?;
                let attribution = [ArmyAttribution::Owner, ArmyAttribution::Host][at];
                for tag in aliases {
                    let owned = self.owned.get(tag)?;
                    for (measure, value) in before.values()? {
                        let name =
                            format!("{tag}|{}|template|{template}|{measure}", mode(attribution));
                        if owned.get(self.index.get(&name)?) != Some(&value) {
                            return None;
                        }
                    }
                }
                Some(total)
            })();
            match total {
                Some(total) => {
                    *delta = total;
                    true
                }
                None => {
                    dirty.slots([slot]);
                    false
                }
            }
        });
        patch
            .totals
            .retain(|&(slot, _, _), _| !dirty.contains(slot));
        patch
    }

    fn rebuild(
        &mut self,
        frame: &Arc<DecodedFrame>,
        carried: &mut Carried,
        tags: &[String],
        hour: i32,
        starts: bool,
    ) -> io::Result<()> {
        let cat = self.cat.clone().flatten();
        let needs = LazyNeeds {
            content: division_needs::content(cat.as_ref()),
            frame,
            index: carried.sources.clone(),
            sources: std::cell::OnceCell::new(),
            kept: std::cell::RefCell::new(BTreeMap::new()),
            hourly: std::cell::RefCell::new(Vec::new()),
        };
        let (daily, stock) = (carried.pool.read.clone(), carried.stock.read.clone());
        let (designs_read, stock_designs_read) = (
            carried.designs.read.clone(),
            carried.stock_designs.0.clone(),
        );
        let designs = LazyDesigns::new(designs_read.as_deref(), cat.as_ref());
        let stock_designs = LazyDesigns::new(stock_designs_read.as_deref(), cat.as_ref());
        let context = Context {
            daily: daily.as_deref(),
            stock: stock.as_deref(),
            cat: cat.as_ref(),
            designs: &designs,
            stock_designs: &stock_designs,
            needs: &needs,
            names: None,
            icon: &|_| Ok(String::new()),
            labels: Labels::Kept,
        };
        let never = AtomicBool::new(false);
        let mut refreshed = BTreeSet::new();
        for tag in tags {
            let mut values = Vec::new();
            for attribution in [ArmyAttribution::Owner, ArmyAttribution::Host] {
                let prefix = format!("{tag}|{}|", mode(attribution));
                let p = DivisionEquipmentParams {
                    branch_id: frame.branch.to_string(),
                    hour,
                    tag: tag.clone(),
                    count_for: attribution,
                    template: None,
                    division: None,
                };
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.builds += 1;
                }
                let mut refusals = Refusals::default();
                let built = build_in(
                    frame,
                    &p,
                    &context,
                    Some(&mut carried.shared),
                    &mut refusals,
                    &never,
                );
                let at = usize::from(attribution == ArmyAttribution::Host);
                if let Some(slot) = carried.tags.get(tag).copied().flatten()
                    && refreshed.insert((slot, at))
                {
                    match &built {
                        Ok(built) => carried.seed_numeric(frame, slot, at, &built.reply),
                        Err(_) => carried.invalidate_numeric(slot, at),
                    }
                }
                match built {
                    Ok(built) if built.reply.absent => {}
                    Ok(built) => self.flatten(
                        &prefix,
                        frame,
                        &needs.kept.borrow(),
                        built,
                        refusals,
                        carried,
                        &mut values,
                    )?,
                    Err(_) => values.push((format!("{prefix}refused"), 1)),
                }
            }
            self.publish(tag, values, hour, starts);
        }
        if carried.sources.is_none()
            && let Some(Ok(sources)) = needs.sources.get()
        {
            carried.sources = Some(Arc::clone(sources.index()));
        }
        // A need refused for this hour's unreadable tables is worded for it: asked again.
        for template in needs.hourly.take() {
            carried.shared.needs.remove(&template);
        }
        Ok(())
    }

    /// One country's reply as its names' values.
    #[allow(clippy::too_many_arguments)]
    fn flatten(
        &mut self,
        prefix: &str,
        frame: &DecodedFrame,
        kept: &BTreeMap<String, KeptReason>,
        built: Built,
        refusals: Refusals,
        carried: &mut Carried,
        values: &mut Vec<(String, i128)>,
    ) -> io::Result<()> {
        let parse = |text: &str| {
            text.parse::<i128>()
                .map_err(|_| io::Error::other("Division equipment value is not a number"))
        };
        let reply = built.reply;
        values.push((format!("{prefix}state"), 1));
        values.push((format!("{prefix}divisions"), i128::from(reply.divisions)));
        values.push((format!("{prefix}short"), i128::from(reply.divisions_short)));
        values.push((
            format!("{prefix}unknown"),
            i128::from(reply.unknown_divisions),
        ));
        for kind in &reply.types {
            let (id, _) = self.catalogued.kinds.number(kind.archetype.clone())?;
            let base = format!("{prefix}type|{id}|");
            values.push((format!("{base}held"), parse(&kind.held_raw)?));
            for (measure, value) in [
                ("needed", &kind.needed_raw),
                ("filled", &kind.filled_raw),
                ("missing", &kind.missing_raw),
                ("stock", &kind.stockpile_raw),
            ] {
                if let Some(value) = value {
                    values.push((format!("{base}{measure}"), parse(value)?));
                }
            }
            values.push((format!("{base}short"), i128::from(kind.divisions_short)));
            values.push((format!("{base}unknown"), i128::from(kind.unknown_divisions)));
            if let Some(hour) = kind.stockpile_hour {
                values.push((format!("{base}stockHour"), i128::from(hour)));
            }
            for variant in &kind.variants {
                let handle = variant
                    .variant
                    .parse::<u64>()
                    .map_err(|_| io::Error::other("Invalid design handle"))?;
                values.push((format!("{base}variant|{handle}"), parse(&variant.held_raw)?));
                let number = design_number(
                    &mut self.catalogued,
                    &carried.shared.designs,
                    &mut carried.designs_numbered,
                    handle,
                )?;
                if let Some(number) = number {
                    values.push((format!("{base}design|{handle}"), number as i128));
                }
            }
        }
        for template in &reply.templates {
            let handle = template
                .template
                .parse::<u64>()
                .map_err(|_| io::Error::other("Invalid template handle"))?;
            let base = format!("{prefix}template|{handle}|");
            values.push((format!("{base}divisions"), i128::from(template.divisions)));
            values.push((
                format!("{base}unknown"),
                i128::from(template.unknown_divisions),
            ));
            values.push((
                format!("{base}typesShort"),
                i128::from(template.types_short),
            ));
            for (measure, value) in [
                ("manpower", &template.manpower),
                ("required", &template.manpower_required),
                ("hp", &template.hp_raw),
                ("hpMax", &template.hp_max_raw),
                ("needed", &template.needed_raw),
                ("held", &template.held_raw),
            ] {
                if let Some(value) = value {
                    values.push((format!("{base}{measure}"), parse(value)?));
                }
            }
            let (naming, artwork) = match carried.labels.get(&handle) {
                Some(&(naming, artwork, false)) => (naming, artwork),
                _ => {
                    let hourly = army_content::template_naming_hourly(frame, handle);
                    let (naming, _) = self.catalogued.namings.number(kept_naming(frame, handle))?;
                    let (artwork, _) = self
                        .catalogued
                        .artworks
                        .number(kept_artwork(frame, handle))?;
                    carried.labels.insert(handle, (naming, artwork, hourly));
                    if hourly {
                        carried.hourly.insert(handle);
                    } else {
                        carried.hourly.remove(&handle);
                    }
                    (naming, artwork)
                }
            };
            values.push((format!("{base}naming"), naming as i128));
            values.push((format!("{base}artwork"), artwork as i128));
        }
        for (note, times) in refusals.counted() {
            let (id, _) = self.catalogued.notes.number(match kept.get(&note) {
                // A need this hour's composition read refused, noted as that need words it.
                Some(reason) => reason.clone(),
                None => KeptReason::literal(note),
            })?;
            values.push((format!("{prefix}note|{id}"), times as i128));
        }
        for handle in built.stock_designs {
            let number = design_number(
                &mut self.catalogued,
                &carried.shared.stock_designs,
                &mut carried.stock_numbered,
                handle,
            )?;
            if let Some(number) = number {
                values.push((format!("{prefix}stockDesign|{handle}"), number as i128));
            }
        }
        Ok(())
    }

    /// One country's values at `hour`, observed against those it last published.
    fn publish(&mut self, tag: &str, values: Vec<(String, i128)>, hour: i32, starts: bool) {
        let mut next = BTreeMap::new();
        for (name, value) in values {
            let key = self.number(&name);
            next.insert(key, value);
        }
        let current = self.owned.remove(tag).unwrap_or_default();
        for (&key, &value) in &next {
            if starts || current.get(&key) != Some(&value) {
                self.batch.observe(key, hour, Some(value), starts);
            }
        }
        for &key in current.keys() {
            if !next.contains_key(&key) {
                self.batch.observe(key, hour, None, false);
            }
        }
        self.owned.insert(tag.to_owned(), next);
    }

    fn publish_sparse(
        &mut self,
        tag: &str,
        values: [(String, i128); 3],
        hour: i32,
    ) -> Option<()> {
        let current = self.owned.get_mut(tag)?;
        for (name, value) in values {
            let key = *self.index.get(&name)?;
            let before = current.get_mut(&key)?;
            if *before != value {
                self.batch.observe(key, hour, Some(value), false);
                *before = value;
            }
        }
        Some(())
    }

    /// An hour whose replies are not all known: every name is unknown then.
    fn unknown(&mut self, hour: i32) -> io::Result<()> {
        if let Some(carried) = &mut self.carried {
            carried.contributions.clear();
            carried.totals.clear();
        }
        if self.open {
            let keys: Vec<u64> = [READ, KNOWN]
                .into_iter()
                .chain(
                    self.owned
                        .values()
                        .flat_map(|values| values.keys().copied()),
                )
                .collect();
            for key in keys {
                self.batch.observe(key, hour, None, false);
            }
        }
        self.owned.clear();
        self.open = false;
        self.last_hour = Some(hour);
        self.batch.through(hour);
        Ok(())
    }

    /// Publish the batch and the catalogue.
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.batch.flush(
            writer,
            &dashboard_keyed::series_section(NAME, branch),
            0,
            cancelled,
        )?;
        let counts = self.catalogued.counts();
        if let Some(page) = self
            .catalogued
            .page(self.published.counts, self.published.through)?
        {
            writer.write_at(
                &catalogue_section(branch),
                self.published.pages,
                page.as_slice(),
            )?;
            #[cfg(any(test, feature = "test-support"))]
            dashboard_base::prepared::pages::Pages::note_framing(
                writer,
                &catalogue_section(branch),
                self.published.pages,
                8 + u64::from_le_bytes(page[..8].try_into().expect("catalogue directory length"))
                    as usize,
            );
            self.published = Published {
                pages: self.published.pages + 1,
                counts,
                through: self.last_hour,
            };
        }
        Ok(())
    }

    /// The catalogue a restored continuation published, read back from its pages.
    pub fn restore(
        &mut self,
        read: impl FnMut(u64) -> io::Result<Option<store::Range>>,
    ) -> io::Result<()> {
        let catalogued = Catalogued::read(read, self.published.pages)?;
        if catalogued.counts() != self.published.counts {
            return Err(io::Error::other(
                "Division equipment catalogue pages differ from their continuation",
            ));
        }
        self.catalogued = catalogued;
        self.index.clear();
        self.carried = None;
        Ok(())
    }
}

/// Needs whose hour's sources are read only at a template's first need, and then from the
/// links and local sub-units kept since their tables last changed.
struct LazyNeeds<'f> {
    content: Result<Arc<dashboard_text::subunit_requirements::Content>, String>,
    frame: &'f DecodedFrame,
    index: Option<Arc<dashboard_text::subunit_requirements::SourceIndex>>,
    sources: std::cell::OnceCell<Result<dashboard_text::subunit_requirements::Sources<'f>, String>>,
    /// Each need refused by this hour's failed composition read, as that read's refusal.
    kept: std::cell::RefCell<BTreeMap<String, KeptReason>>,
    /// Templates whose need this hour's unreadable tables refused.
    hourly: std::cell::RefCell<Vec<u64>>,
}

impl<'f> LazyNeeds<'f> {
    fn sources(&self) -> &Result<dashboard_text::subunit_requirements::Sources<'f>, String> {
        self.sources.get_or_init(|| match &self.index {
            Some(index) => division_needs::reused_sources(self.frame, Arc::clone(index)),
            None => {
                // Reading them lists every link, local sub-unit and token of the hour.
                scanned(
                    [
                        TableId::LexicalTokens,
                        TableId::CountryLinks,
                        TableId::CountryLocalSubunits,
                    ]
                    .iter()
                    .map(|table| rows_in(&self.frame.tables[table.index()]))
                    .sum(),
                );
                division_needs::sources(self.frame)
            }
        })
    }
}

impl NeedsOf for LazyNeeds<'_> {
    fn warnings(&self) -> Vec<&str> {
        division_needs::warnings(&self.content).collect()
    }

    fn template(&self, frame: &DecodedFrame, handle: u64) -> Result<Vec<(String, i64)>, String> {
        let need = division_needs::template(&self.content, || self.sources(), frame, handle);
        // Refused at its composition read, after its content and sources were read: that
        // read's refusal, which names the hour.
        if let Err(why) = &need
            && self.content.is_ok()
            && self.sources().is_ok()
            && let Err(error) = frame.complete::<tables::DivisionTemplateIcons>()
        {
            let kept = KeptReason::unreadable(
                division_needs::COMPOSITION_UNAVAILABLE,
                frame,
                TableId::DivisionTemplateIcons,
                &error,
            );
            if kept.at(frame.hour) == *why {
                self.kept.borrow_mut().insert(why.clone(), kept);
                self.hourly.borrow_mut().push(handle);
            }
        }
        need
    }
}

/// Designs read only once a design missing from the shared lookups is asked.
struct LazyDesigns<'f> {
    read: Option<&'f DecodedFrame>,
    cat: Option<&'f Arc<Catalogue>>,
    designs: std::cell::OnceCell<army_content::Designs<'f>>,
}

impl<'f> LazyDesigns<'f> {
    fn new(read: Option<&'f DecodedFrame>, cat: Option<&'f Arc<Catalogue>>) -> Self {
        Self {
            read,
            cat,
            designs: std::cell::OnceCell::new(),
        }
    }
}

impl DesignsOf for LazyDesigns<'_> {
    fn operand(
        &self,
        variant: u64,
        refusals: &mut Refusals,
    ) -> Option<army_content::DesignOperand> {
        self.designs
            .get_or_init(|| army_content::Designs::new(self.read, self.cat))
            .operand(variant, refusals)
    }

    fn name(&self, raw: &RawVariant) -> Option<String> {
        army_content::design_name(raw, self.cat)
    }
}

/// A design's naming number, from the operands its lookup read; none when it has none.
fn design_number(
    catalogued: &mut Catalogued,
    looked: &BTreeMap<u64, super::Looked>,
    numbered: &mut HashMap<u64, Option<usize>>,
    handle: u64,
) -> io::Result<Option<usize>> {
    if let Some(&number) = numbered.get(&handle) {
        return Ok(number);
    }
    let number = match looked
        .get(&handle)
        .and_then(|looked| looked.operand.as_ref())
    {
        Some(operand) => {
            let (number, _) = catalogued.designs.number(operand.raw.clone())?;
            Some(number)
        }
        None => None,
    };
    numbered.insert(handle, number);
    Ok(number)
}

/// The value `runs` hold at `hour`.
fn at(runs: &[dashboard_keyed::query::KeyedRun], hour: i32) -> Option<&str> {
    let at = runs.partition_point(|run| run.1 < hour);
    runs.get(at)
        .filter(|run| run.0 <= hour)
        .map(|run| run.2.as_str())
}

/// The history of one country's replies, as JSON.
pub fn query(query: &Query<'_>) -> io::Result<serde_json::Value> {
    dashboard_keyed::query::finish(query, &history(query)?)
}

/// The history of one country's replies, through the gate's accepted prefix.
pub fn history(query: &Query<'_>) -> io::Result<DivisionEquipmentHistoryReply> {
    let p = dashboard_keyed::query::params::<DivisionEquipmentHistory>(query)?;
    let (first, last) = query.window()?;
    let context = LoadContext {
        content: query.content.cloned(),
    };
    let mut base = Refusals::default();
    let cat = base.keep(army_content::content(query.source, &context)).0;
    if let Some(cat) = &cat {
        let (_, notes) =
            dashboard_land_battles::products::division_log::division_name_groups(query.source, cat);
        for note in notes {
            base.note(note);
        }
    }
    let mut reply = DivisionEquipmentHistoryReply {
        branch_id: p.branch_id.clone(),
        tag: p.tag.clone(),
        count_for: p.count_for,
        window: dashboard_keyed::query::empty(first, last),
        kinds: Vec::new(),
        namings: BTreeMap::new(),
        artworks: BTreeMap::new(),
        designs: BTreeMap::new(),
        hours: Vec::new(),
        absent: Vec::new(),
        served: Vec::new(),
        warnings: Vec::new(),
    };
    let session = query.store.session(query.key)?;
    // Only the pages naming an hour by the window's end, and of them this country's names
    // and the operands its runs name.
    let through = query.through().map_or(last, |through| through.min(last));
    let mut pages = match &session {
        Some(session) => Pages::open(session, query.branch, through)?,
        None => Pages { pages: Vec::new() },
    };
    if !pages.pages.is_empty() {
        let mut reader = Reader::new(
            query.store,
            query.key,
            dashboard_keyed::series_section(NAME, query.branch),
            query.cancelled,
        )?;
        let prefix = format!("{}|{}|", p.tag, mode(p.count_for));
        let keyed: Vec<(String, u64)> = pages
            .names(&p.tag)?
            .into_iter()
            .filter_map(|(key, name)| match name.strip_prefix(&prefix) {
                Some(rest) => Some((rest.to_owned(), key)),
                None => name.starts_with("*|").then_some((name, key)),
            })
            .collect();
        reply.window = dashboard_keyed::query::window(
            &mut reader,
            keyed,
            first,
            last,
            query.through(),
            query.cancelled,
        )?;
        let operands = Operands::read(&mut pages, &reply.window)?;
        let drawn = name_operands(query.source, &operands, cat.as_ref(), &mut reply)?;
        reply.hours = hours(&operands, &reply, &base, &drawn)?;
    }
    reply.absent = base.lines();
    Ok(reply)
}

/// The window's operands named in the shown language and art, with each icon's fallbacks.
fn name_operands(
    source: &Source,
    operands: &Operands,
    cat: Option<&Arc<Catalogue>>,
    reply: &mut DivisionEquipmentHistoryReply,
) -> io::Result<BTreeMap<String, BTreeSet<String>>> {
    let mut drawn = BTreeMap::new();
    let mut kinds = BTreeSet::new();
    let number = |text: &str| {
        text.parse::<usize>()
            .map_err(|_| io::Error::other("Invalid history operand"))
    };
    for series in &reply.window.series {
        let parts: Vec<&str> = series.name.split('|').collect();
        if let ["type", kind, ..] = parts.as_slice() {
            kinds.insert(number(kind)?);
        }
        let ids = series.runs.iter().map(|run| number(&run.2));
        match parts.as_slice() {
            ["template", _, "naming"] => {
                for id in ids {
                    let id = id?;
                    let naming = operands
                        .namings
                        .get(&id)
                        .ok_or_else(|| io::Error::other("Unknown template naming"))?;
                    reply.namings.entry(id.to_string()).or_insert_with(|| {
                        let (name, name_missing) =
                            answered(naming, |naming| army_content::template_name_of(naming, cat));
                        HistoryName { name, name_missing }
                    });
                }
            }
            ["template", _, "artwork"] => {
                for id in ids {
                    let id = id?;
                    let artwork = operands
                        .artworks
                        .get(&id)
                        .ok_or_else(|| io::Error::other("Unknown template artwork"))?;
                    if !reply.artworks.contains_key(&id.to_string()) {
                        // An hour names only the fallbacks of the icons it shows.
                        let ((icon, icon_missing), own) =
                            dashboard_text::catalogue::capture_isolated(|| {
                                answered(artwork, |artwork| {
                                    army_content::template_icon_of(source, artwork, cat)
                                })
                            });
                        reply
                            .artworks
                            .insert(id.to_string(), HistoryIcon { icon, icon_missing });
                        drawn.insert(id.to_string(), own.fallbacks().clone());
                    }
                }
            }
            ["type", _, "design", _] | ["stockDesign", _] => {
                for id in ids {
                    let id = id?;
                    let raw = operands
                        .designs
                        .get(&id)
                        .ok_or_else(|| io::Error::other("Unknown design operands"))?;
                    reply
                        .designs
                        .entry(id.to_string())
                        .or_insert_with(|| army_content::design_name(raw, cat));
                }
            }
            _ => {}
        }
    }
    // Types in the reply's key order, the unclassified first.
    let mut listed: Vec<(Option<String>, usize)> = kinds
        .into_iter()
        .map(|id| {
            operands
                .kinds
                .get(&id)
                .cloned()
                .map(|key| (key, id))
                .ok_or_else(|| io::Error::other("Unknown equipment type"))
        })
        .collect::<io::Result<_>>()?;
    listed.sort();
    for (archetype, id) in listed {
        let (name, name_missing) = match &archetype {
            Some(key) => keep(army_content::localised(cat, key)),
            None => (None, Some("Equipment archetype is unclassified".into())),
        };
        reply.kinds.push(HistoryKind {
            id: id.to_string(),
            archetype,
            name,
            name_missing,
        });
    }
    Ok(drawn)
}

fn keep<T>(value: Result<T, String>) -> (Option<T>, Option<String>) {
    match value {
        Ok(value) => (Some(value), None),
        Err(reason) => (None, Some(reason)),
    }
}

/// Each present hour's refusals, naming ones counted again as the reached product counts them.
fn hours(
    operands: &Operands,
    reply: &DivisionEquipmentHistoryReply,
    base: &Refusals,
    drawn: &BTreeMap<String, BTreeSet<String>>,
) -> io::Result<Vec<HistoryWarnings>> {
    let window = &reply.window;
    let Some(state) = window.series.iter().find(|series| series.name == "state") else {
        return Ok(Vec::new());
    };
    let read: Vec<&dashboard_keyed::query::KeyedSeries> = window
        .series
        .iter()
        .filter(|series| {
            let parts: Vec<&str> = series.name.split('|').collect();
            matches!(
                parts.as_slice(),
                ["state"]
                    | ["note", _]
                    | ["type", _, "held"]
                    | ["type", _, "design", _]
                    | ["template", _, "naming" | "artwork"]
                    | ["stockDesign", _]
            )
        })
        .collect();
    let mut cuts = BTreeSet::new();
    for series in &read {
        for run in &series.runs {
            cuts.insert(run.0);
            if let Some(after) = run.1.checked_add(1) {
                cuts.insert(after);
            }
        }
    }
    let ends: Vec<i32> = cuts.iter().copied().collect();
    let mut spans: Vec<HistoryWarnings> = Vec::new();
    for (index, &start) in ends.iter().enumerate() {
        let Some(&next) = ends.get(index + 1) else {
            break;
        };
        if at(&state.runs, start).is_none() {
            continue;
        }
        let mut refusals = BTreeMap::<KeptReason, usize>::new();
        let mut count = |reason: KeptReason, times: usize| {
            if times > 0 {
                *refusals.entry(reason).or_default() += times;
            }
        };
        for (reason, times) in base.clone().counted() {
            count(KeptReason::literal(reason), times);
        }
        let mut fallbacks = BTreeSet::new();
        for series in &read {
            let Some(value) = at(&series.runs, start) else {
                continue;
            };
            let parts: Vec<&str> = series.name.split('|').collect();
            let number = || {
                value
                    .parse::<usize>()
                    .map_err(|_| io::Error::other("Invalid history operand"))
            };
            match parts.as_slice() {
                ["note", note] => {
                    let id = note
                        .parse::<usize>()
                        .map_err(|_| io::Error::other("Invalid history note"))?;
                    let text = operands
                        .notes
                        .get(&id)
                        .ok_or_else(|| io::Error::other("Unknown history note"))?;
                    count(text.clone(), number()?);
                }
                ["type", kind, "held"] => {
                    if let Some(found) = reply.kinds.iter().find(|listed| listed.id == *kind)
                        && found.archetype.is_some()
                        && let Some(missing) = &found.name_missing
                    {
                        count(KeptReason::literal(missing.clone()), 1);
                    }
                }
                ["template", _, "naming"] => {
                    if let Some(missing) = reply
                        .namings
                        .get(&number()?.to_string())
                        .and_then(|name| name.name_missing.as_ref())
                    {
                        count(missing.clone(), 1);
                    }
                }
                ["template", _, "artwork"] => {
                    let id = number()?.to_string();
                    if let Some(missing) = reply
                        .artworks
                        .get(&id)
                        .and_then(|icon| icon.icon_missing.as_ref())
                    {
                        count(missing.clone(), 1);
                    }
                    fallbacks.extend(drawn.get(&id).into_iter().flatten().cloned());
                }
                ["type", _, "design", _] | ["stockDesign", _] => {
                    if reply
                        .designs
                        .get(&number()?.to_string())
                        .is_some_and(Option::is_none)
                    {
                        count(KeptReason::literal(army_content::UNNAMED_DESIGN), 1);
                    }
                }
                _ => {}
            }
        }
        let refusals: Vec<HistoryRefusal> = refusals
            .into_iter()
            .map(|(reason, times)| HistoryRefusal { reason, times })
            .collect();
        let fallback = installed_content_core::content::overlay_refusal_line(&fallbacks);
        match spans.last_mut() {
            Some(span)
                if (&span.refusals, &span.fallback) == (&refusals, &fallback)
                    && span.last_hour.checked_add(1) == Some(start) =>
            {
                span.last_hour = next - 1
            }
            _ => spans.push(HistoryWarnings {
                first_hour: start,
                last_hour: next - 1,
                refusals,
                fallback,
            }),
        }
    }
    Ok(spans)
}

/// The reply a history holds for `hour`, served; none where it does not answer then.
pub fn reply_at(
    history: &DivisionEquipmentHistoryReply,
    hour: i32,
) -> Option<DivisionEquipmentReply> {
    use super::{EquipmentTemplate, EquipmentType, EquipmentVariant};
    let window = &history.window;
    if hour < window.first_hour
        || hour > window.last_hour
        || !window
            .known
            .iter()
            .any(|span| span.first_hour <= hour && hour <= span.last_hour)
    {
        return None;
    }
    let values: BTreeMap<&str, &str> = window
        .series
        .iter()
        .filter_map(|series| Some((series.name.as_str(), at(&series.runs, hour)?)))
        .collect();
    if values.contains_key("refused") {
        return None;
    }
    let number = |name: &str| {
        values
            .get(name)
            .and_then(|value| value.parse::<i128>().ok())
    };
    let count = |name: &str| number(name).and_then(|value| u32::try_from(value).ok());
    let mut reply = DivisionEquipmentReply {
        branch_id: history.branch_id.clone(),
        hour,
        tag: history.tag.clone(),
        count_for: history.count_for,
        absent: false,
        pool_hour: number("*|pool").map(|hour| hour as i32),
        divisions: 0,
        divisions_short: 0,
        unknown_divisions: 0,
        types: Vec::new(),
        templates: Vec::new(),
        division_rows: None,
        pool: None,
        warnings: Vec::new(),
    };
    if !values.contains_key("state") {
        reply.absent = true;
        reply.warnings = [history.absent.as_slice(), &history.served].concat();
        return Some(reply);
    }
    reply.divisions = count("divisions")?;
    reply.divisions_short = count("short")?;
    reply.unknown_divisions = count("unknown")?;
    let text = |name: String| values.get(name.as_str()).map(|value| (*value).to_owned());
    for kind in &history.kinds {
        let base = format!("type|{}|", kind.id);
        let Some(held) = text(format!("{base}held")) else {
            continue;
        };
        let prefix = format!("{base}variant|");
        let mut variants: Vec<(u64, EquipmentVariant)> = values
            .iter()
            .filter_map(|(name, value)| Some((name.strip_prefix(prefix.as_str())?, value)))
            .map(|(handle, value)| {
                let name = text(format!("{base}design|{handle}"))
                    .and_then(|id| history.designs.get(&id).cloned().flatten());
                Some((
                    handle.parse().ok()?,
                    EquipmentVariant {
                        variant: handle.to_owned(),
                        name,
                        held_raw: (*value).to_owned(),
                    },
                ))
            })
            .collect::<Option<_>>()?;
        variants.sort_by_key(|(handle, _)| *handle);
        reply.types.push(EquipmentType {
            archetype: kind.archetype.clone(),
            name: kind.name.clone(),
            name_missing: kind.name_missing.clone(),
            held_raw: held,
            needed_raw: text(format!("{base}needed")),
            filled_raw: text(format!("{base}filled")),
            missing_raw: text(format!("{base}missing")),
            divisions_short: count(&format!("{base}short"))?,
            unknown_divisions: count(&format!("{base}unknown"))?,
            stockpile_raw: text(format!("{base}stock")),
            stockpile_hour: number(&format!("{base}stockHour")).map(|hour| hour as i32),
            variants: variants.into_iter().map(|(_, variant)| variant).collect(),
        });
    }
    let mut templates: Vec<u64> = values
        .keys()
        .filter_map(|name| name.strip_prefix("template|")?.strip_suffix("|divisions"))
        .map(|handle| handle.parse().ok())
        .collect::<Option<_>>()?;
    templates.sort_unstable();
    for handle in templates {
        let base = format!("template|{handle}|");
        let naming = text(format!("{base}naming")).and_then(|id| history.namings.get(&id));
        let artwork = text(format!("{base}artwork")).and_then(|id| history.artworks.get(&id));
        reply.templates.push(EquipmentTemplate {
            template: handle.to_string(),
            name: naming.and_then(|name| name.name.clone()),
            name_missing: naming
                .and_then(|name| name.name_missing.as_ref().map(|kept| kept.at(hour))),
            icon: artwork.and_then(|icon| icon.icon.clone()),
            icon_missing: artwork
                .and_then(|icon| icon.icon_missing.as_ref().map(|kept| kept.at(hour))),
            divisions: count(&format!("{base}divisions"))?,
            manpower: text(format!("{base}manpower")),
            manpower_required: text(format!("{base}required")),
            hp_raw: text(format!("{base}hp")),
            hp_max_raw: text(format!("{base}hpMax")),
            needed_raw: text(format!("{base}needed")),
            held_raw: text(format!("{base}held")),
            types_short: count(&format!("{base}typesShort"))?,
            unknown_divisions: count(&format!("{base}unknown"))?,
        });
    }
    let span = history
        .hours
        .iter()
        .find(|span| span.first_hour <= hour && hour <= span.last_hour)?;
    // Worded for the hour, then counted and ordered by those words, as the reached product's.
    let mut refusals = Refusals::default();
    for refusal in &span.refusals {
        refusals.note_times(refusal.reason.at(hour), refusal.times);
    }
    reply.warnings = refusals.lines();
    reply.warnings.extend(history.served.iter().cloned());
    reply.warnings.extend(span.fallback.clone());
    // The reached product refuses a reply over its transport budget.
    dashboard_base::products::check_reply_budget(&reply).ok()?;
    Some(reply)
}
