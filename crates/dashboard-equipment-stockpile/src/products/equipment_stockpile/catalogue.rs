//! The equipment stockpile's catalogue and chart (`equipment-stockpile-catalogue` and
//! `equipment-stockpile-chart`). Each holder's streams are replayed once per publication and
//! kept; from the replays, the holders' variants and logistics types with their names and
//! peaks, their coverage and reads, and each group's line over any view are answered without
//! reading the publication again.
//!
//! A group adds up as the widget's own at-hour figures do (`stockpileData.ts`): a type's or an
//! arm's stock is unknown wherever a variant of unknown type holds anything, and its surplus
//! and need follow the logistics record's types, with an unresolved type deciding a resolved
//! arm's completeness.

use super::{
    NEED_BOUND_RAW, StockType, UNRESOLVED_MARK,
    content::{Published, Reading, Types, publication, text_bytes},
    emit as layout,
    read::{self, Cells, DailyStream, Logistics, LogisticsStream},
};
use dashboard_base::{
    prepared::{
        engine,
        series::{summary_bins, summary_budget},
    },
    prepared_chart::{ChartBucket, ChartRun, ChartSummary, merge},
    products::{
        country_groups::HourSpan,
        equipment::{Domain, EQUIPMENT_DECIMALS, StockVariantName, UNRESOLVED_ARM},
        validate_tag,
    },
};
use dashboard_series::{Evidence, PageRead, Snapshot, family::Directory};
use dashboard_text::{catalogue::Refusals, metadata::check_cancel};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    mem::size_of,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

/// A holder whose history a request reads, only within `within` where it names hours.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileHolder {
    pub tag: String,
    #[serde(default)]
    #[ts(optional)]
    pub within: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileCatalogueParams {
    pub branch_id: String,
    pub holders: Vec<StockpileHolder>,
    /// The types a group counts; empty counts every type.
    #[serde(default)]
    pub included: Vec<String>,
}

/// What a chart line draws: a group's stock, its surplus as a percentage of its need, or
/// its need.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum StockpileMeasure {
    Stock,
    Share,
    Need,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileLine {
    /// The group's key: `[kind, key]` as JSON, a variant's key its handle.
    pub group: String,
    pub measure: StockpileMeasure,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileChartParams {
    pub branch_id: String,
    pub holders: Vec<StockpileHolder>,
    #[serde(default)]
    pub included: Vec<String>,
    pub lines: Vec<StockpileLine>,
    pub from: f64,
    pub to: f64,
    pub width: u32,
}

fn validate(branch_id: &str, holders: &[StockpileHolder]) -> Result<(), &'static str> {
    if branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    if holders.is_empty() {
        return Err("Invalid stockpile selection");
    }
    let mut seen = BTreeSet::new();
    for holder in holders {
        validate_tag(&holder.tag)?;
        if !seen.insert(holder.tag.as_str()) {
            return Err("Invalid stockpile selection");
        }
        if let Some(spans) = &holder.within
            && (spans.iter().any(|span| span.first_hour > span.last_hour)
                || spans
                    .windows(2)
                    .any(|pair| pair[0].last_hour >= pair[1].first_hour))
        {
            return Err("Invalid stockpile membership");
        }
    }
    Ok(())
}

impl StockpileCatalogueParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate(&self.branch_id, &self.holders)
    }
}

impl StockpileChartParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate(&self.branch_id, &self.holders)?;
        if !self.from.is_finite()
            || !self.to.is_finite()
            || self.from > self.to
            || self.from < f64::from(i32::MIN)
            || self.to > f64::from(i32::MAX)
            || self.width == 0
        {
            return Err("Invalid stockpile viewport");
        }
        Ok(())
    }
}

/// A group's largest stock or need at any read within the holders' hours, and whether its
/// stock and its share of need draw anything there.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileChoicePeak {
    pub raw: String,
    pub stock_drawable: bool,
    pub share_drawable: bool,
}

/// A variant the holders held or produced, with its naming over the whole branch.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileCatalogueVariant {
    /// Its native handle, as a decimal string.
    pub id: String,
    /// Its final nonempty equipment token; empty where no read named it.
    pub token: String,
    /// Its type's archetype key; null where content does not resolve its type.
    pub archetype: Option<String>,
    pub names: Vec<StockVariantName>,
    /// Every design recorded under its handle in the branch.
    pub designs: Vec<String>,
}

/// A logistics type the holders' record listed, and whether it ever held anything but zero.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileCatalogueLogistics {
    /// The archetype key, or the unresolved mark and the recorded token's spelling.
    pub key: String,
    pub nonzero: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileCatalogueReply {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub campaign_key: String,
    pub decimals: u8,
    /// Every archetype a variant or logistics type belongs to, in the equipment database's order.
    pub types: Vec<StockType>,
    pub variants: Vec<StockpileCatalogueVariant>,
    pub logistics: Vec<StockpileCatalogueLogistics>,
    /// The hours the stockpile, the production lines and the logistics record (while the
    /// game computed a balance) were read for the holders within their hours, ascending.
    pub reads: Vec<i32>,
    pub line_reads: Vec<i32>,
    pub logistics_reads: Vec<i32>,
    pub stock_hours: Vec<HourSpan>,
    pub line_hours: Vec<HourSpan>,
    pub balance_hours: Vec<HourSpan>,
    /// Hours the holder was AI-controlled, so the game computed no balance.
    pub ai_hours: Vec<HourSpan>,
    pub need_bound_raw: String,
    /// Each group's peak and drawability, by its key (`[kind, key]` as JSON).
    pub choices: BTreeMap<String, StockpileChoicePeak>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileChartLine {
    pub group: String,
    pub measure: StockpileMeasure,
    pub summary: ChartSummary,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileChartReply {
    pub branch_id: String,
    pub lines: Vec<StockpileChartLine>,
    pub warnings: Vec<String>,
}

/// A variant's stock at a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stock {
    /// No stock row: it holds none.
    Absent,
    /// A row whose stock is not computable.
    Unknown,
    Amount(i64),
}

impl Stock {
    fn of(row: Option<read::Daily>) -> Self {
        match row {
            Some(row) if row.stock_present => row.stock.map_or(Self::Unknown, Self::Amount),
            _ => Self::Absent,
        }
    }

    /// What it adds to a sum; none leaves the sum unknown.
    fn raw(self) -> Option<i128> {
        match self {
            Self::Absent => Some(0),
            Self::Unknown => None,
            Self::Amount(amount) => Some(i128::from(amount)),
        }
    }

    /// Whether a row holds anything but zero: a variant of unknown type holding it leaves
    /// every type's and arm's total unknown.
    fn holds(self) -> bool {
        matches!(self, Self::Unknown) || matches!(self, Self::Amount(amount) if amount != 0)
    }
}

/// One holder's streams replayed over the publication.
#[derive(Default)]
struct Replay {
    stock: Vec<HourSpan>,
    lines: Vec<HourSpan>,
    /// The logistics record's known hours, while it held the holder's rows and while it held
    /// none (AI-controlled).
    balance: Vec<HourSpan>,
    ai: Vec<HourSpan>,
    /// Every variant with a row at some read, and its stock from each hour it changed.
    variants: BTreeMap<u64, Vec<(i32, Stock)>>,
    /// Every logistics token with a row at some read, and its row from each hour it changed.
    logistics: BTreeMap<u64, Vec<(i32, Option<Logistics>)>>,
    counts: Vec<(i32, [u64; 11])>,
}

impl Replay {
    fn bytes(&self) -> usize {
        let spans = [&self.stock, &self.lines, &self.balance, &self.ai]
            .iter()
            .map(|spans| spans.capacity() * size_of::<HourSpan>())
            .sum::<usize>();
        let variants = self
            .variants
            .values()
            .map(|changes| 64 + changes.capacity() * size_of::<(i32, Stock)>())
            .sum::<usize>();
        let logistics = self
            .logistics
            .values()
            .map(|changes| 64 + changes.capacity() * size_of::<(i32, Option<Logistics>)>())
            .sum::<usize>();
        size_of::<Self>()
            + spans
            + variants
            + logistics
            + self.counts.capacity() * size_of::<(i32, [u64; 11])>()
    }
}

fn known_spans(rows: Vec<Evidence>) -> Vec<HourSpan> {
    rows.iter()
        .filter(|row| read::known(Some(row)))
        .map(|row| HourSpan {
            first_hour: row.span.first,
            last_hour: row.span.last,
        })
        .collect()
}

/// The logistics record's known hours split where the holder's rows start and stop: hours it
/// held some, and hours it held none.
fn split(spans: &[HourSpan], held: &[(i32, bool)]) -> io::Result<(Vec<HourSpan>, Vec<HourSpan>)> {
    let (mut rows, mut none) = (Vec::new(), Vec::new());
    for span in spans {
        let mut at = held.partition_point(|(hour, _)| *hour <= span.first_hour);
        // A known span starts at a read, which writes a keyframe.
        let mut state = at
            .checked_sub(1)
            .map(|before| held[before].1)
            .ok_or_else(read::malformed)?;
        let mut first = span.first_hour;
        while let Some(&(hour, next)) = held.get(at).filter(|(hour, _)| *hour <= span.last_hour) {
            if next != state {
                let piece = HourSpan {
                    first_hour: first,
                    last_hour: hour - 1,
                };
                if state { &mut rows } else { &mut none }.push(piece);
                (first, state) = (hour, next);
            }
            at += 1;
        }
        let piece = HourSpan {
            first_hour: first,
            last_hour: span.last_hour,
        };
        if state { &mut rows } else { &mut none }.push(piece);
    }
    Ok((rows, none))
}

/// `tag`'s streams over the publication, in one forward scan of each.
fn replay<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<Replay> {
    let accepted = snapshot.publication().accepted;
    let mut replay = Replay::default();
    let Some(actor) = directory.actor(tag.as_bytes()) else {
        return Ok(replay);
    };
    let key = |measure| directory.key(actor, measure, None);
    replay.stock = known_spans(read::evidence(snapshot, key(layout::STOCK_HOURS), accepted)?);
    replay.lines = known_spans(read::evidence(snapshot, key(layout::LINES_HOURS), accepted)?);
    let logistics = known_spans(read::evidence(
        snapshot,
        key(layout::LOGISTICS_HOURS),
        accepted,
    )?);
    let mut cells = Cells::default();
    if let Some(daily) = key(layout::DAILY) {
        let mut rows = BTreeMap::new();
        let variants = &mut replay.variants;
        read::events(snapshot, daily, accepted, |snapshot, event| {
            check_cancel(cancelled)?;
            let fields = cells.read(snapshot, event.fields)?;
            read::apply::<DailyStream>(event.kind, fields, &mut rows, |id, row| {
                let stock = Stock::of(row);
                let changes = variants.entry(id).or_default();
                if changes.last().is_none_or(|(_, last)| *last != stock) {
                    changes.push((event.native_hour, stock));
                }
            })
        })?;
    }
    let mut held = Vec::<(i32, bool)>::new();
    if let Some(key) = key(layout::LOGISTICS) {
        let mut rows = BTreeMap::new();
        let tokens = &mut replay.logistics;
        read::events(snapshot, key, accepted, |snapshot, event| {
            check_cancel(cancelled)?;
            let fields = cells.read(snapshot, event.fields)?;
            read::apply::<LogisticsStream>(event.kind, fields, &mut rows, |id, row| {
                let changes = tokens.entry(id).or_default();
                if changes.last().is_none_or(|(_, last)| *last != row) {
                    changes.push((event.native_hour, row));
                }
            })?;
            let any = !rows.is_empty();
            if held.last().is_none_or(|(_, last)| *last != any) {
                held.push((event.native_hour, any));
            }
            Ok(())
        })?;
    }
    (replay.balance, replay.ai) = split(&logistics, &held)?;
    if let Some(key) = key(layout::COUNTS) {
        let counts = &mut replay.counts;
        read::events(snapshot, key, accepted, |snapshot, event| {
            counts.push((event.native_hour, read::counts(snapshot, &event, &mut cells)?));
            Ok(())
        })?;
    }
    Ok(replay)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Type,
    Domain,
    Variant,
}

/// What a group adds up: its variants and its logistics types.
struct Group {
    /// `[kind, key]` as JSON, as the widget keys it.
    wire: String,
    kind: Kind,
    key: String,
    variants: Vec<u64>,
    types: Vec<String>,
}

impl Group {
    /// A resolved arm, whose sums an unresolved logistics type can leave unknown.
    fn resolved_arm(&self) -> bool {
        self.kind == Kind::Domain && self.key != UNRESOLVED_ARM
    }
}

/// A stretch of a line inside a coverage span: one value, or unknown; `starts` where the span
/// starts.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Piece<T> {
    first: i32,
    last: i32,
    value: Option<T>,
    starts: bool,
}

fn push<T: PartialEq + Copy>(
    pieces: &mut Vec<Piece<T>>,
    (first, last): (i32, i32),
    value: Option<T>,
    starts: bool,
) {
    if let Some(previous) = pieces.last_mut()
        && !starts
        && previous.value == value
        && i64::from(previous.last) + 1 == i64::from(first)
    {
        previous.last = last;
        return;
    }
    pieces.push(Piece {
        first,
        last,
        value,
        starts,
    });
}

/// A group's surplus and need at an hour, the need none where it is not derived.
type Balance = (i128, Option<i128>);

/// Each logistics key, its tokens ascending (an archetype's row at an hour is its greatest
/// token holding one, as an hour's reply reads it), and whether it ever held anything but zero.
type TypeTokens = BTreeMap<String, (Vec<u64>, bool)>;

/// One holder's lines of a group.
#[derive(Default)]
struct Lines {
    stock: Vec<Piece<i128>>,
    logistics: Vec<Piece<Balance>>,
}

/// A variant of the holders' catalogue.
struct Member {
    token: String,
    archetype: Option<String>,
    domain: Option<Domain>,
    names: Vec<StockVariantName>,
    designs: Vec<String>,
}

/// The holders' catalogue and every type's and arm's lines, built once per publication,
/// selection and included types.
struct Model {
    tags: Vec<String>,
    replays: Vec<Arc<Replay>>,
    variants: BTreeMap<u64, Member>,
    logistics: TypeTokens,
    types: Vec<StockType>,
    groups: Vec<Group>,
    /// Per holder, each group's lines, by its place in `groups`.
    lines: Vec<Vec<Lines>>,
    /// The daily and logistics schedules: each run's first read, last hour and period.
    schedules: [Vec<(i32, i32, i32)>; 2],
    first: i32,
    last: i32,
    warnings: Vec<String>,
}

impl Model {
    fn bytes(&self) -> usize {
        // A map entry's node share, conservatively.
        const ENTRY: usize = 64;
        let names = self
            .variants
            .values()
            .map(|member| {
                ENTRY
                    + size_of::<(u64, Member)>()
                    + member.token.capacity()
                    + member.archetype.as_ref().map_or(0, String::capacity)
                    + member.names.capacity() * size_of::<StockVariantName>()
                    + member
                        .names
                        .iter()
                        .map(|name| {
                            name.name.as_ref().map_or(0, String::capacity)
                                + name.design.as_ref().map_or(0, String::capacity)
                        })
                        .sum::<usize>()
                    + text_bytes(&member.designs)
                    + (member.designs.capacity() - member.designs.len()) * size_of::<String>()
            })
            .sum::<usize>();
        let groups = self.groups.capacity() * size_of::<Group>()
            + self
                .groups
                .iter()
                .map(|group| {
                    group.wire.capacity()
                        + group.key.capacity()
                        + group.variants.capacity() * size_of::<u64>()
                        + text_bytes(&group.types)
                        + (group.types.capacity() - group.types.len()) * size_of::<String>()
                })
                .sum::<usize>();
        let lines = self.lines.capacity() * size_of::<Vec<Lines>>()
            + self
                .lines
                .iter()
                .map(|holder| holder.capacity() * size_of::<Lines>())
                .sum::<usize>()
            + self
                .lines
                .iter()
                .flatten()
                .map(|lines| {
                    lines.stock.capacity() * size_of::<Piece<i128>>()
                        + lines.logistics.capacity() * size_of::<Piece<Balance>>()
                })
                .sum::<usize>();
        let types = self.types.capacity() * size_of::<StockType>()
            + self
                .types
                .iter()
                .map(|kind| {
                    kind.key.capacity()
                        + kind.name.as_ref().map_or(0, String::capacity)
                        + kind.need_note.as_ref().map_or(0, String::capacity)
                })
                .sum::<usize>();
        // Its replays stay charged while it holds them, however long theirs are kept.
        size_of::<Self>()
            + self.replays.capacity() * size_of::<Arc<Replay>>()
            + self.replays.iter().map(|replay| replay.bytes()).sum::<usize>()
            + text_bytes(&self.tags)
            + types
            + names
            + groups
            + lines
            + self
                .logistics
                .iter()
                .map(|(key, (tokens, _))| {
                    ENTRY
                        + size_of::<(String, (Vec<u64>, bool))>()
                        + key.capacity()
                        + tokens.capacity() * size_of::<u64>()
                })
                .sum::<usize>()
            + text_bytes(&self.warnings)
            + (self.warnings.capacity() - self.warnings.len()) * size_of::<String>()
            + self
                .schedules
                .iter()
                .map(|runs| runs.capacity() * size_of::<(i32, i32, i32)>())
                .sum::<usize>()
    }

    fn group(&self, wire: &str) -> Option<usize> {
        self.groups.iter().position(|group| group.wire == wire)
    }

    /// A holder's lines of the group at `index`.
    fn lines(&self, holder: usize, index: usize) -> &Lines {
        &self.lines[holder][index]
    }
}

fn domain_key(domain: Option<Domain>) -> &'static str {
    match domain {
        Some(Domain::Land) => "land",
        Some(Domain::Air) => "air",
        Some(Domain::Naval) => "naval",
        None => UNRESOLVED_ARM,
    }
}

fn wire(kind: Kind, key: &str) -> String {
    let kind = match kind {
        Kind::Type => "type",
        Kind::Domain => "domain",
        Kind::Variant => "variant",
    };
    serde_json::to_string(&(kind, key)).expect("a group key serializes")
}

/// Every group the widget can show, as `stockGroups` builds them: by type, by arm and by
/// variant, each counting only the included types; a logistics type that held anything but
/// zero adds a row of its own, and every listed type of a row's adds to its sums.
fn groups(
    variants: &BTreeMap<u64, Member>,
    logistics: &TypeTokens,
    domains: &BTreeMap<String, Domain>,
    included: &[String],
) -> Vec<Group> {
    let shown = |key: &str| included.is_empty() || included.iter().any(|item| item == key);
    let type_key = |id: u64, member: &Member| {
        member.archetype.clone().unwrap_or_else(|| {
            let token = if member.token.is_empty() {
                id.to_string()
            } else {
                member.token.clone()
            };
            format!("{UNRESOLVED_MARK}{token}")
        })
    };
    fn group(grouped: &mut BTreeMap<String, Group>, kind: Kind, key: String) -> &mut Group {
        grouped.entry(key.clone()).or_insert_with(|| Group {
            wire: wire(kind, &key),
            kind,
            key,
            variants: Vec::new(),
            types: Vec::new(),
        })
    }
    let mut result = Vec::new();
    for kind in [Kind::Type, Kind::Domain] {
        let mut grouped = BTreeMap::<String, Group>::new();
        for (&id, member) in variants {
            let key = type_key(id, member);
            if !shown(&key) {
                continue;
            }
            let key = match kind {
                Kind::Type => key,
                _ => domain_key(member.domain).to_owned(),
            };
            group(&mut grouped, kind, key).variants.push(id);
        }
        let listed: Vec<_> = logistics
            .iter()
            .filter(|(key, _)| shown(key))
            .map(|(key, &(_, nonzero))| {
                let row = match kind {
                    Kind::Type => key.clone(),
                    _ => domain_key(domains.get(key).copied()).to_owned(),
                };
                (key, row, nonzero)
            })
            .collect();
        for (_, row, nonzero) in &listed {
            if *nonzero {
                group(&mut grouped, kind, row.clone());
            }
        }
        for (key, row, _) in listed {
            if let Some(group) = grouped.get_mut(&row)
                && !group.types.contains(key)
            {
                group.types.push(key.clone());
            }
        }
        result.extend(grouped.into_values());
    }
    for (&id, member) in variants {
        if shown(&type_key(id, member)) {
            let key = id.to_string();
            result.push(Group {
                wire: wire(Kind::Variant, &key),
                kind: Kind::Variant,
                key,
                variants: vec![id],
                types: Vec::new(),
            });
        }
    }
    result
}

/// A change list's value at each hour, taken in ascending hours.
struct Cursor<'a, T> {
    changes: &'a [(i32, T)],
    taken: usize,
}

impl<'a, T: Copy> Cursor<'a, T> {
    fn new(changes: &'a [(i32, T)]) -> Self {
        Self { changes, taken: 0 }
    }

    fn advance(&mut self, hour: i32) {
        while self
            .changes
            .get(self.taken)
            .is_some_and(|(at, _)| *at <= hour)
        {
            self.taken += 1;
        }
    }

    fn value(&self) -> Option<T> {
        self.taken
            .checked_sub(1)
            .map(|taken| self.changes[taken].1)
    }
}

/// Each span's first hour, then every hour in it that one of `changes` changes at.
fn points(spans: &[HourSpan], mut hours: Vec<i32>) -> Vec<(i32, i32, bool)> {
    hours.sort_unstable();
    hours.dedup();
    let mut points = Vec::new();
    for span in spans {
        let start = hours.partition_point(|hour| *hour <= span.first_hour);
        let inside = hours[start..]
            .iter()
            .copied()
            .take_while(|hour| *hour <= span.last_hour);
        let mut first = span.first_hour;
        let mut starts = true;
        for hour in inside {
            points.push((first, hour - 1, starts));
            (first, starts) = (hour, false);
        }
        points.push((first, span.last_hour, starts));
    }
    points
}

/// Every type's and arm's stock over the holder's stock hours.
fn stock_lines(
    replay: &Replay,
    groups: &[(usize, &Group)],
    untyped: &[u64],
    cancelled: &AtomicBool,
) -> io::Result<Vec<Vec<Piece<i128>>>> {
    let mut ids: Vec<u64> = groups
        .iter()
        .flat_map(|(_, group)| group.variants.iter().copied())
        .chain(untyped.iter().copied())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let place: HashMap<u64, usize> = ids.iter().enumerate().map(|(at, id)| (*id, at)).collect();
    let mut cursors: Vec<_> = ids
        .iter()
        .map(|id| Cursor::new(replay.variants.get(id).map_or(&[][..], Vec::as_slice)))
        .collect();
    let members: Vec<Vec<usize>> = groups
        .iter()
        .map(|(_, group)| group.variants.iter().map(|id| place[id]).collect())
        .collect();
    let untyped: Vec<usize> = untyped.iter().map(|id| place[id]).collect();
    // A type or an arm holding an unresolved logistics type cannot tell its variants.
    let refused: Vec<bool> = groups
        .iter()
        .map(|(_, group)| group.types.iter().any(|key| key.starts_with(UNRESOLVED_MARK)))
        .collect();
    let hours = cursors
        .iter()
        .flat_map(|cursor| cursor.changes.iter().map(|(hour, _)| *hour))
        .collect();
    let mut lines = vec![Vec::new(); groups.len()];
    for (count, (first, last, starts)) in points(&replay.stock, hours).into_iter().enumerate() {
        if count % 1024 == 0 {
            check_cancel(cancelled)?;
        }
        for cursor in &mut cursors {
            cursor.advance(first);
        }
        let stock = |at: usize| cursors[at].value().unwrap_or(Stock::Absent);
        let blocked = untyped.iter().any(|&at| stock(at).holds());
        for (index, members) in members.iter().enumerate() {
            let sum = if blocked || refused[index] {
                None
            } else {
                members
                    .iter()
                    .try_fold(0_i128, |sum, &at| stock(at).raw().map(|raw| sum + raw))
            };
            push(&mut lines[index], (first, last), sum, starts);
        }
    }
    Ok(lines)
}

/// A variant's stock over the holder's stock hours.
fn variant_line(replay: &Replay, id: u64) -> Vec<Piece<i128>> {
    let changes = replay.variants.get(&id).map_or(&[][..], Vec::as_slice);
    let mut cursor = Cursor::new(changes);
    let mut line = Vec::new();
    let hours = changes.iter().map(|(hour, _)| *hour).collect();
    for (first, last, starts) in points(&replay.stock, hours) {
        cursor.advance(first);
        let stock = cursor.value().unwrap_or(Stock::Absent).raw();
        push(&mut line, (first, last), stock, starts);
    }
    line
}

/// What decides each type's and arm's surplus and need: the logistics keys' tokens, their
/// types' need notes, and the unresolved keys the included types admit.
struct Record<'a> {
    logistics: &'a TypeTokens,
    notes: BTreeMap<&'a str, Option<&'a str>>,
    unresolved: Vec<&'a str>,
}

const NEED_UNAVAILABLE: &str = "Need unavailable.";

/// Every type's and arm's surplus and need over the hours the holder's record held its rows,
/// as `balanceOperandsAt` derives them.
fn logistics_lines(
    replay: &Replay,
    groups: &[(usize, &Group)],
    record: &Record<'_>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<Vec<Piece<Balance>>>> {
    let mut tokens: Vec<u64> = groups
        .iter()
        .flat_map(|(_, group)| group.types.iter().map(String::as_str))
        .chain(record.unresolved.iter().copied())
        .filter_map(|key| record.logistics.get(key))
        .flat_map(|(tokens, _)| tokens.iter().copied())
        .collect();
    tokens.sort_unstable();
    tokens.dedup();
    let place: HashMap<u64, usize> = tokens
        .iter()
        .enumerate()
        .map(|(at, token)| (*token, at))
        .collect();
    let mut cursors: Vec<_> = tokens
        .iter()
        .map(|token| Cursor::new(replay.logistics.get(token).map_or(&[][..], Vec::as_slice)))
        .collect();
    // A key's tokens by their place, ascending.
    let places = |key: &str| -> Vec<usize> {
        record
            .logistics
            .get(key)
            .map(|(tokens, _)| tokens.iter().map(|token| place[token]).collect())
            .unwrap_or_default()
    };
    // Each group's types: their tokens' places and their need note.
    let members: Vec<Vec<(Vec<usize>, Option<&str>)>> = groups
        .iter()
        .map(|(_, group)| {
            group
                .types
                .iter()
                .map(|key| {
                    let tokens = places(key);
                    let note = record
                        .notes
                        .get(key.as_str())
                        .copied()
                        .unwrap_or(Some(NEED_UNAVAILABLE));
                    (tokens, note)
                })
                .collect()
        })
        .collect();
    let unresolved: Vec<Vec<usize>> = record.unresolved.iter().map(|key| places(key)).collect();
    let hours = cursors
        .iter()
        .flat_map(|cursor| cursor.changes.iter().map(|(hour, _)| *hour))
        .collect();
    let bound = NEED_BOUND_RAW.unsigned_abs();
    let mut lines = vec![Vec::new(); groups.len()];
    for (count, (first, last, starts)) in points(&replay.balance, hours).into_iter().enumerate() {
        if count % 1024 == 0 {
            check_cancel(cancelled)?;
        }
        for cursor in &mut cursors {
            cursor.advance(first);
        }
        // A key's row: its greatest token holding one.
        let row = |tokens: &[usize]| tokens.iter().rev().find_map(|&at| cursors[at].value().flatten());
        // An unresolved type could belong to any resolved arm.
        let unattributed = unresolved
            .iter()
            .any(|tokens| row(tokens).is_some_and(|row| row.balance != Some(0) || row.stock != Some(0)));
        let listed = unresolved.iter().any(|tokens| row(tokens).is_some());
        for (index, (_, group)) in groups.iter().enumerate() {
            let types = &members[index];
            let value = if types.is_empty() || (group.resolved_arm() && unattributed) {
                None
            } else {
                (|| {
                    let (mut balance, mut stock_need) = (0_i128, Some(0_i128));
                    for (tokens, note) in types {
                        let held = row(tokens);
                        let (type_balance, type_stock) = match held {
                            Some(held) => (held.balance?, held.stock?),
                            None => (0, 0),
                        };
                        balance += i128::from(type_balance);
                        let available = held.and_then(|held| held.available);
                        let note = if available.is_some() { None } else { *note };
                        let native = available.unwrap_or(type_stock.max(0));
                        // The native subtraction wraps at signed 64 bits.
                        let need = native.wrapping_sub(type_balance);
                        stock_need = if note.is_some() || need.unsigned_abs() >= bound {
                            None
                        } else {
                            stock_need.map(|sum| sum + i128::from(need))
                        };
                    }
                    if group.resolved_arm() && listed {
                        stock_need = None;
                    }
                    Some((balance, stock_need))
                })()
            };
            push(&mut lines[index], (first, last), value, starts);
        }
    }
    Ok(lines)
}

/// The holders' model: their replays, kept per holder, and their catalogue and lines.
#[allow(clippy::too_many_arguments)]
fn model<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    branch: u64,
    tags: &[String],
    included: &[String],
    reading: &Reading<'_>,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Model>> {
    let publication: Published = publication(snapshot);
    reading.keep(
        "stockpile:model",
        branch,
        &(tags, included),
        &publication,
        cancelled,
        Model::bytes,
        || {
            let mut replays = Vec::with_capacity(tags.len());
            for tag in tags {
                replays.push(reading.keep(
                    "stockpile:replay",
                    branch,
                    tag,
                    &publication,
                    cancelled,
                    Replay::bytes,
                    || replay(snapshot, directory, tag, cancelled),
                )?);
            }
            build(snapshot, directory, branch, tags, included, replays, reading, cancelled)
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn build<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    branch: u64,
    tags: &[String],
    included: &[String],
    replays: Vec<Arc<Replay>>,
    reading: &Reading<'_>,
    cancelled: &AtomicBool,
) -> io::Result<Model> {
    let accepted = snapshot.publication().accepted;
    let content = &reading.content;
    let mut warnings = content.warnings.clone();
    let mut types = Types::default();
    let mut cells = Cells::default();
    let forgets = read::forgets(snapshot, directory, accepted.last)?;
    let ids: BTreeSet<u64> = replays
        .iter()
        .flat_map(|replay| replay.variants.keys().copied())
        .collect();
    let (mut unnamed, mut refused) = (0, Refusals::default());
    let mut variants = BTreeMap::new();
    for id in ids {
        check_cancel(cancelled)?;
        let token = read::token(snapshot, directory, id, &mut cells)?.unwrap_or_default();
        let history = read::naming(snapshot, directory, id, &forgets, accepted.last, &mut cells)?;
        if history.is_empty() {
            unnamed += 1;
        }
        let mut names: Vec<StockVariantName> = Vec::new();
        let mut designs = BTreeSet::new();
        for (first, raw) in &history {
            for (_, why) in raw.iter().flat_map(|raw| &raw.refusals) {
                refused.note(why.as_str(), id.to_string());
            }
            if let Some(design) = raw.as_ref().and_then(|raw| raw.design.clone()) {
                designs.insert(design);
            }
            let name = read::name(*first, raw.as_ref(), content.texts());
            if names.last().is_some_and(|last| {
                last.name == name.name && last.known == name.known && last.design == name.design
            }) {
                continue;
            }
            names.push(name);
        }
        let archetype = types.variant(content, &token);
        variants.insert(
            id,
            Member {
                domain: archetype.as_deref().and_then(|archetype| types.domain(archetype)),
                token,
                archetype,
                names,
                designs: designs.into_iter().collect(),
            },
        );
    }
    let spellings = read::tokens(snapshot, directory, branch, reading, cancelled)?;
    let mut logistics = TypeTokens::new();
    let mut domains = BTreeMap::new();
    let tokens: BTreeSet<u64> = replays
        .iter()
        .flat_map(|replay| replay.logistics.keys().copied())
        .collect();
    for token in tokens {
        let spelling = spellings
            .get(&token)
            .cloned()
            .unwrap_or_else(|| format!("token {token}"));
        let key = types.logistics(content, &spelling);
        if let Some(domain) = types.domain(&key) {
            domains.insert(key.clone(), domain);
        }
        // A recorded zero is a value; anything else, unknown included, is not.
        let nonzero = replays.iter().any(|replay| {
            replay.logistics.get(&token).is_some_and(|changes| {
                changes.iter().any(|(_, row)| {
                    row.is_some_and(|row| {
                        row.balance != Some(0) || row.stock != Some(0) || row.available != Some(0)
                    })
                })
            })
        });
        let entry = logistics.entry(key).or_default();
        entry.0.push(token);
        entry.1 |= nonzero;
    }
    if unnamed > 0 {
        warnings.push(format!(
            "{unnamed} variants in stock or production have no recorded definition in this branch; their type is unknown."
        ));
    }
    warnings.extend(refused.diagnostic_lines("equipment variants with unknown text/design operands"));
    let types = types.finish(content, &mut warnings);
    warnings.extend(content.refusals());
    let groups = groups(&variants, &logistics, &domains, included);
    let untyped: Vec<u64> = variants
        .iter()
        .filter(|(_, member)| member.archetype.is_none())
        .map(|(&id, _)| id)
        .collect();
    let notes = logistics
        .keys()
        .map(|key| {
            let note = types
                .iter()
                .find(|kind| kind.key == *key)
                .map_or(Some(NEED_UNAVAILABLE), |kind| kind.need_note.as_deref());
            (key.as_str(), note)
        })
        .collect();
    let record = Record {
        logistics: &logistics,
        notes,
        unresolved: logistics
            .keys()
            .filter(|key| {
                key.starts_with(UNRESOLVED_MARK)
                    && (included.is_empty() || included.iter().any(|item| item == *key))
            })
            .map(String::as_str)
            .collect(),
    };
    let summed: Vec<(usize, &Group)> = groups
        .iter()
        .enumerate()
        .filter(|(_, group)| group.kind != Kind::Variant)
        .collect();
    let mut lines = Vec::with_capacity(replays.len());
    for replay in &replays {
        let stock = stock_lines(replay, &summed, &untyped, cancelled)?;
        let logistics = logistics_lines(replay, &summed, &record, cancelled)?;
        let mut holder: Vec<Lines> = groups
            .iter()
            .map(|group| Lines {
                stock: match group.kind {
                    Kind::Variant => variant_line(replay, group.variants[0]),
                    _ => Vec::new(),
                },
                logistics: Vec::new(),
            })
            .collect();
        for (((index, _), stock), logistics) in summed.iter().zip(stock).zip(logistics) {
            holder[*index] = Lines { stock, logistics };
        }
        lines.push(holder);
    }
    drop(record);
    let schedule = |snapshot: &mut Snapshot<'_, P>, measure| -> io::Result<Vec<(i32, i32, i32)>> {
        let Some(key) = directory.key(layout::FAMILY, measure, None) else {
            return Ok(Vec::new());
        };
        engine::runs(snapshot, key, accepted.first, accepted.last)?
            .into_iter()
            .map(|run| {
                let period = i32::try_from(run.value)
                    .ok()
                    .filter(|period| *period >= 0)
                    .ok_or_else(read::malformed)?;
                Ok((run.span.first, run.span.last, period))
            })
            .collect()
    };
    let schedules = [
        schedule(snapshot, layout::DAILY_READS)?,
        schedule(snapshot, layout::LOGISTICS_READS)?,
    ];
    Ok(Model {
        tags: tags.to_vec(),
        replays,
        variants,
        logistics,
        types,
        groups,
        lines,
        schedules,
        first: accepted.first,
        last: accepted.last,
        warnings,
    })
}

/// `spans` within `within`, where it names hours.
fn within(spans: &[HourSpan], within: Option<&[HourSpan]>) -> Vec<HourSpan> {
    let Some(within) = within else {
        return spans.to_vec();
    };
    let mut result = Vec::new();
    for span in spans {
        let start = within.partition_point(|member| member.last_hour < span.first_hour);
        for member in within[start..]
            .iter()
            .take_while(|member| member.first_hour <= span.last_hour)
        {
            result.push(HourSpan {
                first_hour: span.first_hour.max(member.first_hour),
                last_hour: span.last_hour.min(member.last_hour),
            });
        }
    }
    result
}

fn overlaps(spans: Option<&[HourSpan]>, first: i32, last: i32) -> bool {
    spans.is_none_or(|spans| {
        spans
            .get(spans.partition_point(|span| span.last_hour < first))
            .is_some_and(|span| span.first_hour <= last)
    })
}

/// A schedule's reads within `spans`: each run's first hour and every period after; a period
/// of 0 reads at the first hour alone.
fn reads(schedule: &[(i32, i32, i32)], spans: &[HourSpan], into: &mut Vec<i32>) {
    for span in spans {
        let start = schedule.partition_point(|run| run.1 < span.first_hour);
        for &(first, last, period) in schedule[start..]
            .iter()
            .take_while(|run| run.0 <= span.last_hour)
        {
            let (from, to) = (
                i64::from(span.first_hour.max(first)),
                i64::from(span.last_hour.min(last)),
            );
            let (first, period) = (i64::from(first), i64::from(period));
            if period == 0 {
                if (from..=to).contains(&first) {
                    into.push(first as i32);
                }
                continue;
            }
            let mut hour = first + (from - first + period - 1).div_euclid(period) * period;
            while hour <= to {
                into.push(hour as i32);
                hour += period;
            }
        }
    }
}

/// How many of a schedule's reads fall within `spans` and `from..=to`.
fn read_count(schedule: &[(i32, i32, i32)], spans: &[HourSpan], from: i32, to: i32) -> u64 {
    let mut count = 0;
    let start = spans.partition_point(|span| span.last_hour < from);
    for span in spans[start..].iter().take_while(|span| span.first_hour <= to) {
        let (lower, upper) = (span.first_hour.max(from), span.last_hour.min(to));
        let runs = schedule.partition_point(|run| run.1 < lower);
        for &(first, last, period) in schedule[runs..].iter().take_while(|run| run.0 <= upper) {
            let (low, high) = (i64::from(lower.max(first)), i64::from(upper.min(last)));
            let (first, period) = (i64::from(first), i64::from(period));
            if low > high {
                continue;
            }
            if period == 0 {
                count += u64::from((low..=high).contains(&first));
                continue;
            }
            let earliest = (low - first + period - 1).div_euclid(period);
            let latest = (high - first).div_euclid(period);
            if latest >= earliest {
                count += (latest - earliest + 1) as u64;
            }
        }
    }
    count
}

fn sorted(mut spans: Vec<HourSpan>) -> Vec<HourSpan> {
    spans.sort_by_key(|span| span.first_hour);
    spans
}

/// The holders' catalogue: their variants, logistics types, coverage, reads, groups' peaks and
/// warnings, within their hours; none past what preparation published, which is pending.
#[allow(clippy::too_many_arguments)]
pub fn query<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &StockpileCatalogueParams,
    reading: &Reading<'_>,
    cancelled: &AtomicBool,
) -> io::Result<StockpileCatalogueReply> {
    let branch = params.branch_id.parse().expect("validated stockpile branch");
    let publication = publication(snapshot);
    let reply = reading.keep(
        "stockpile:catalogue",
        branch,
        &(&params.holders, &params.included),
        &publication,
        cancelled,
        |reply: &StockpileCatalogueReply| {
            dashboard_base::prepared::query::observations::serde_bytes(reply)
        },
        || catalogue(snapshot, directory, branch, params, reading, cancelled),
    )?;
    Ok(StockpileCatalogueReply::clone(&reply))
}

fn catalogue<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    branch: u64,
    params: &StockpileCatalogueParams,
    reading: &Reading<'_>,
    cancelled: &AtomicBool,
) -> io::Result<StockpileCatalogueReply> {
    let tags: Vec<String> = params.holders.iter().map(|holder| holder.tag.clone()).collect();
    let model = model(
        snapshot,
        directory,
        branch,
        &tags,
        &params.included,
        reading,
        cancelled,
    )?;
    let mut reply = StockpileCatalogueReply {
        branch_id: params.branch_id.clone(),
        tags: model.tags.clone(),
        campaign_key: reading.campaign_key.clone(),
        decimals: EQUIPMENT_DECIMALS,
        types: model.types.clone(),
        variants: model
            .variants
            .iter()
            .map(|(id, member)| StockpileCatalogueVariant {
                id: id.to_string(),
                token: member.token.clone(),
                archetype: member.archetype.clone(),
                names: member.names.clone(),
                designs: member.designs.clone(),
            })
            .collect(),
        logistics: model
            .logistics
            .iter()
            .map(|(key, &(_, nonzero))| StockpileCatalogueLogistics {
                key: key.clone(),
                nonzero,
            })
            .collect(),
        reads: Vec::new(),
        line_reads: Vec::new(),
        logistics_reads: Vec::new(),
        stock_hours: Vec::new(),
        line_hours: Vec::new(),
        balance_hours: Vec::new(),
        ai_hours: Vec::new(),
        need_bound_raw: NEED_BOUND_RAW.to_string(),
        choices: BTreeMap::new(),
        warnings: model.warnings.clone(),
    };
    // The reads each part of a holder's counts covers, and their totals.
    let (mut costless, mut other, mut missing, mut overflowed) = (0_u64, 0_u64, 0_u64, 0_u64);
    for (holder, replay) in params.holders.iter().zip(&model.replays) {
        let membership = holder.within.as_deref();
        let [stock, lines, balance, ai] =
            [&replay.stock, &replay.lines, &replay.balance, &replay.ai]
                .map(|spans| within(spans, membership));
        let [daily, logistics] = &model.schedules;
        reads(daily, &stock, &mut reply.reads);
        reads(daily, &lines, &mut reply.line_reads);
        reads(logistics, &balance, &mut reply.logistics_reads);
        for (at, (hour, counts)) in replay.counts.iter().enumerate() {
            let until = replay
                .counts
                .get(at + 1)
                .map_or(model.last, |(next, _)| next - 1);
            let [stock_reads, line_reads, logistics_reads] = [
                (daily, &stock),
                (daily, &lines),
                (logistics, &balance),
            ]
            .map(|(schedule, spans)| read_count(schedule, spans, *hour, until));
            costless += counts[1] * line_reads;
            other += counts[2] * line_reads;
            missing += counts[7] * logistics_reads;
            overflowed += counts[0] * stock_reads
                + (counts[3] + counts[4]) * line_reads
                + (counts[8] + counts[9] + counts[10]) * logistics_reads;
        }
        reply.stock_hours.extend(stock);
        reply.line_hours.extend(lines);
        reply.balance_hours.extend(balance);
        reply.ai_hours.extend(ai);
    }
    for reads in [
        &mut reply.reads,
        &mut reply.line_reads,
        &mut reply.logistics_reads,
    ] {
        reads.sort_unstable();
        reads.dedup();
    }
    for spans in [
        &mut reply.stock_hours,
        &mut reply.line_hours,
        &mut reply.balance_hours,
        &mut reply.ai_hours,
    ] {
        *spans = sorted(std::mem::take(spans));
    }
    for (index, group) in model.groups.iter().enumerate() {
        check_cancel(cancelled)?;
        let mut choice = (0_i128, false, false);
        for (holder, params) in params.holders.iter().enumerate() {
            let membership = params.within.as_deref();
            let lines = model.lines(holder, index);
            for piece in &lines.stock {
                if let Some(value) = piece.value
                    && overlaps(membership, piece.first, piece.last)
                {
                    choice.0 = choice.0.max(value);
                    choice.1 = true;
                }
            }
            for piece in &lines.logistics {
                if let Some((_, Some(need))) = piece.value
                    && overlaps(membership, piece.first, piece.last)
                {
                    choice.0 = choice.0.max(need);
                    choice.2 |= need > 0;
                }
            }
        }
        reply.choices.insert(
            group.wire.clone(),
            StockpileChoicePeak {
                raw: choice.0.to_string(),
                stock_drawable: choice.1,
                share_drawable: choice.2,
            },
        );
    }
    if costless > 0 {
        reply.warnings.push(format!(
            "{costless} production line reads have no positive cost, so their variant's daily output and cost per unit are unknown at those reads (the game shows only a placeholder rate for them)."
        ));
    }
    if reply.balance_hours.is_empty() && reply.ai_hours.is_empty() {
        reply.warnings.push("The surplus or shortage is not in this recording: it has no logistics record (LogisticsEquipment).".to_owned());
    }
    if missing > 0 {
        reply.warnings.push(format!("{missing} logistics archetype reads have no matching recorded available-stock operand; convoy, truck and train need remains unknown at those reads."));
    }
    if overflowed > 0 {
        reply.warnings.push(format!(
            "{overflowed} recorded totals do not fit a 64-bit amount; they are shown as unknown, never clamped."
        ));
    }
    if other > 0 {
        reply.warnings.push(format!(
            "{other} naval, refit or railway-gun line reads make equipment whose daily output is not established, so it, its IC and its cost are unknown at those reads."
        ));
    }
    if reply.stock_hours.is_empty() {
        reply.warnings.push(format!(
            "{}: no recorded stockpile in this branch",
            tags.join(", ")
        ));
    }
    Ok(reply)
}

/// A line to draw: a value held over its hours, with its raw magnitude per hour.
struct Drawn {
    first: i32,
    last: i32,
    value: f64,
    absolute: u128,
    break_before: bool,
}

/// The pieces a measure draws over `from..=to`, read only where the hours overlap them: breaks
/// where a span starts and after an unknown stretch. Equal neighbours stay apart, so a view
/// starts at its first piece; joined without a break, they draw as one.
fn drawn<T: Copy>(
    pieces: &[Piece<T>],
    measure: impl Fn(T) -> Option<(f64, u128)>,
    (from, to): (i32, i32),
    membership: Option<&[HourSpan]>,
    into: &mut Vec<Drawn>,
) {
    let measured = |piece: &Piece<T>| piece.value.and_then(&measure);
    let start = pieces.partition_point(|piece| piece.last < from);
    let end = pieces.partition_point(|piece| piece.first <= to);
    if start >= end {
        return;
    }
    let mut line: Vec<Drawn> = Vec::new();
    let mut joined = start > 0 && measured(&pieces[start - 1]).is_some();
    for piece in &pieces[start..end] {
        let Some((value, absolute)) = measured(piece) else {
            joined = false;
            continue;
        };
        line.push(Drawn {
            first: piece.first,
            last: piece.last,
            value,
            absolute,
            break_before: !joined || piece.starts,
        });
        joined = true;
    }
    let Some(spans) = membership else {
        into.extend(line);
        return;
    };
    for drawn in line {
        let start = spans.partition_point(|span| span.last_hour < drawn.first);
        for span in spans[start..]
            .iter()
            .take_while(|span| span.first_hour <= drawn.last)
        {
            let first = drawn.first.max(span.first_hour);
            into.push(Drawn {
                first,
                last: drawn.last.min(span.last_hour),
                value: drawn.value,
                absolute: drawn.absolute,
                break_before: drawn.break_before || first != drawn.first,
            });
        }
    }
}

/// `lines` as a chart over `view`, its bins laid from `bounds`; hourly views list the lines
/// themselves.
fn summary(mut lines: Vec<Drawn>, bounds: (i32, i32), view: (f64, f64, u32)) -> io::Result<ChartSummary> {
    lines.sort_by_key(|line| line.first);
    let (exact, bins) = summary_bins(bounds, view)?;
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; bins.len()];
    let (from, to) = match (bins.first(), bins.last()) {
        (Some(first), Some(last)) => (first.0, last.1),
        _ => (1, 0),
    };
    let start = lines.partition_point(|line| line.last < from);
    let mut runs = exact.then(Vec::new);
    for line in lines[start..].iter().take_while(|line| line.first <= to) {
        let bin = bins.partition_point(|(_, end)| *end < line.first);
        for (index, &(first, last)) in bins.iter().enumerate().skip(bin) {
            if first > line.last {
                break;
            }
            let (first, last) = (first.max(line.first), last.min(line.last));
            let piece = ChartBucket {
                absolute_total: line.absolute * (i64::from(last) - i64::from(first) + 1) as u128,
                first_hour: first,
                last_hour: last,
                first_value: line.value,
                last_value: line.value,
                last_value_hour: first,
                min_hour: first,
                min_value: line.value,
                max_hour: first,
                max_value: line.value,
                first_break: line.break_before && first == line.first,
                has_break: false,
            };
            // Merging into no bucket would mark a gap that is not there.
            buckets[index] = if buckets[index].is_some() {
                merge(buckets[index], Some(piece))
            } else {
                Some(piece)
            };
        }
        if let Some(runs) = &mut runs {
            runs.push(ChartRun {
                first_hour: line.first,
                last_hour: line.last,
                value: line.value,
                break_before: line.break_before,
            });
        }
    }
    for (bucket, &(first, last)) in buckets.iter_mut().zip(&bins) {
        if let Some(bucket) = bucket {
            bucket.has_break |= bucket.first_hour > first || bucket.last_hour < last;
        }
    }
    Ok(ChartSummary {
        from: view.0,
        to: view.1,
        width: view.2,
        buckets,
        runs,
        approximate: Vec::new(),
    })
}

/// The holders' groups as chart lines over a view, each in its measure.
#[allow(clippy::too_many_arguments)]
pub fn chart<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &StockpileChartParams,
    reading: &Reading<'_>,
    cancelled: &AtomicBool,
) -> io::Result<StockpileChartReply> {
    summary_budget(params.from, params.to, params.width, params.lines.len())?;
    let branch = params.branch_id.parse().expect("validated stockpile branch");
    let tags: Vec<String> = params.holders.iter().map(|holder| holder.tag.clone()).collect();
    let model = model(
        snapshot,
        directory,
        branch,
        &tags,
        &params.included,
        reading,
        cancelled,
    )?;
    let scale = 10_f64.powi(i32::from(EQUIPMENT_DECIMALS));
    let view = (params.from, params.to, params.width);
    let hours = (
        (params.from.floor() as i32).max(model.first),
        (params.to.floor() as i32).min(model.last),
    );
    let mut reply = StockpileChartReply {
        branch_id: params.branch_id.clone(),
        lines: Vec::with_capacity(params.lines.len()),
        warnings: Vec::new(),
    };
    for line in &params.lines {
        check_cancel(cancelled)?;
        let mut drawn_lines = Vec::new();
        if let Some(index) = model.group(&line.group) {
            for (holder, params) in params.holders.iter().enumerate() {
                let lines = model.lines(holder, index);
                let membership = params.within.as_deref();
                match line.measure {
                    StockpileMeasure::Stock => drawn(
                        &lines.stock,
                        |raw| Some((raw as f64 / scale, raw.unsigned_abs())),
                        hours,
                        membership,
                        &mut drawn_lines,
                    ),
                    StockpileMeasure::Share => drawn(
                        &lines.logistics,
                        |(balance, need)| {
                            need.filter(|need| *need > 0)
                                .map(|need| (balance as f64 / need as f64 * 100.0, 0))
                        },
                        hours,
                        membership,
                        &mut drawn_lines,
                    ),
                    StockpileMeasure::Need => drawn(
                        &lines.logistics,
                        |(_, need)| need.map(|need| (need as f64 / scale, need.unsigned_abs())),
                        hours,
                        membership,
                        &mut drawn_lines,
                    ),
                }
            }
        } else {
            reply
                .warnings
                .push(format!("The stockpile has no group {}.", line.group));
        }
        reply.lines.push(StockpileChartLine {
            group: line.group.clone(),
            measure: line.measure,
            summary: summary(drawn_lines, (model.first, model.last), view)?,
        });
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;
