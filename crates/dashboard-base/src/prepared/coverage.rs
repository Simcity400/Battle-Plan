//! Each branch's published coverage page: the accepted extent every prepared
//! query reads before its own pages. The pipeline writes the page, with the
//! families' refusals beside the extent.
//!
//! A family prepared on its own publishes its own coverage and refusals,
//! which only the pipeline's registry names. The dashboard installs its
//! [`Gates`] once, and readers here ask them through [`product_coverage`],
//! [`ranges`] and [`refusal`]; with none installed every family is prepared
//! hour by hour, so a product waits for the branch's coverage and no family
//! refuses it or leaves a hole in it. With them installed, every page they
//! answer from is decoded once per publication ([`super::gates`]).
//!
//! Every query is declared by its entry point before any reader runs, with the hours its request
//! names and the first hole its gate leaves there ([`Entry::declare`]). Readers name no hours that
//! reach past it: each gate they ask answers for the declared hours at most, its coverage ends
//! before that hole, and no sampled read reaches it ([`sampled_through`]).

use super::roots;
use crate::{Source, metadata::check_cancel, products::ContentOptions};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{OnceLock, atomic::AtomicBool},
};

/// What the pipeline answers for the families prepared on their own.
pub trait Gates: Sync {
    /// The refusal `query` answers with on `branch`, if a family it reads
    /// refused the branch at or before the hours declared ([`declared`]).
    fn refusal(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        query: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<()>;
    /// What `product`'s queries read on `branch` through `through` (the
    /// branch's end when None): the coverage they wait for, which ends before
    /// the first hole in those hours, and the holes the families they read
    /// leave there.
    fn ranges(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        product: &str,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<Ranges>;
    /// The hourly walk's coverage on `branch` ([`coverage`]).
    fn walk(
        &self,
        source: &Source,
        content: Option<&ContentOptions>,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Coverage>>;
}

static GATES: OnceLock<&'static dyn Gates> = OnceLock::new();

/// The right to declare queries ([`Entry::declare`]), which only the installer of the gates
/// holds: the query entry point.
pub struct Entry(());

/// Installs the pipeline's gates, and hands their installer the right to declare queries; later
/// installs keep the first and get none.
pub fn install_gates(gates: &'static dyn Gates) -> Option<Entry> {
    GATES.set(gates).ok().map(|()| Entry(()))
}

/// The request the query this thread answers asks, as its entry point declared it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Declared {
    /// The last hour it asks; None for every hour of its branch.
    pub through: Option<i32>,
    /// The first hour from which its gate answers nothing in those hours; None without a hole.
    pub before: Option<i32>,
}

thread_local! {
    static DECLARED: Cell<Declared> = const {
        Cell::new(Declared {
            through: None,
            before: None,
        })
    };
}

/// What the query this thread answers declared; outside a query, every hour of the branch.
pub fn declared() -> Declared {
    DECLARED.get()
}

impl Entry {
    /// Runs `query` as the request `declared`: until it returns, every gate a reader asks answers
    /// for those hours at most, its coverage ends before their first hole, and no sampled read
    /// reaches that hole.
    pub fn declare<T>(&self, declared: Declared, query: impl FnOnce() -> T) -> T {
        struct Restore(Declared);
        impl Drop for Restore {
            fn drop(&mut self) {
                DECLARED.set(self.0);
            }
        }
        let _restore = Restore(DECLARED.replace(declared));
        query()
    }
}

/// `through`, the last hour a reader asks (None: every hour), as the declared request allows it.
fn within(through: Option<i32>) -> Option<i32> {
    match (through, declared().through) {
        (Some(asked), Some(allowed)) => Some(asked.min(allowed)),
        (asked, allowed) => asked.or(allowed),
    }
}

/// `coverage` ending before the declared request's first hole.
fn before_hole(mut coverage: Coverage) -> Coverage {
    if let Some(before) = declared().before
        && coverage.last_hour >= before
    {
        coverage.last_hour = before
            .saturating_sub(1)
            .max(coverage.first_hour.saturating_sub(1));
        coverage.complete = false;
    }
    coverage
}

/// `through`, the last hour a sampled read would reach (None: the branch's end), ending before
/// the declared request's first hole.
pub fn sampled_through(through: Option<i32>) -> Option<i32> {
    match declared().before {
        Some(before) => Some(through.map_or(before.saturating_sub(1), |through| {
            through.min(before.saturating_sub(1))
        })),
        None => through,
    }
}

/// Whether a sampled read may read `hour`: not from the declared request's first hole on.
pub fn sampled(hour: i32) -> bool {
    declared().before.is_none_or(|before| hour < before)
}

/// The coverage `product`'s queries wait for on `branch`, through the declared hours
/// ([`ranges`]).
pub fn product_coverage(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    product: &str,
    cancelled: &AtomicBool,
) -> io::Result<Option<Coverage>> {
    Ok(ranges(source, content, branch, product, None, cancelled)?.coverage)
}

/// The status entry of the hourly walk's coverage in a reply's gate: the
/// preparation status lists it as `coverage`, and each family prepared on its
/// own under its name in `families`.
pub const WALK: &str = "walk";

thread_local! {
    /// The status entries whose coverage this thread's prepared query found
    /// short ([`waiting`]); None while no query collects them.
    static SHORT: RefCell<Option<BTreeSet<&'static str>>> = const { RefCell::new(None) };
}

/// Note that the query this thread runs waits for `entry`'s coverage: it
/// found none on a branch, or none complete.
pub fn note_short(entry: &'static str) {
    SHORT.with(|short| {
        if let Some(short) = short.borrow_mut().as_mut() {
            short.insert(entry);
        }
    });
}

/// Run `query`, with the status entries whose coverage it found short on the
/// branches it read ([`note_short`]): the hourly walk's ([`WALK`]) or a
/// family's prepared on its own. A reply that is pending or partial waits for
/// them, and its client re-asks when one of them moves.
pub fn waiting<T>(query: impl FnOnce() -> T) -> (T, BTreeSet<&'static str>) {
    // The collection this one runs inside, restored however `query` ends.
    struct Restore(Option<BTreeSet<&'static str>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SHORT.with(|short| *short.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SHORT.with(|short| short.borrow_mut().replace(BTreeSet::new())));
    let result = query();
    let short = SHORT.with(|short| short.borrow_mut().replace(BTreeSet::new()));
    (result, short.unwrap_or_default())
}

/// Hours a family's publication will not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedUnavailable")]
pub enum Unavailable {
    // The recording there could not be read, or the family's history has a gap there. (Plain
    // comments: the generated contract carries no documentation for these types.)
    Lost,
    Refused,
}

/// Requested hours a family's publication will not answer, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedHole")]
pub struct Hole {
    pub first_hour: i32,
    pub last_hour: i32,
    pub family: String,
    pub unavailable: Unavailable,
    pub reason: String,
}

impl Hole {
    /// `family`'s ledger `interval` as a hole; None for prepared hours.
    pub fn of(family: &str, interval: &roots::Interval) -> Option<Self> {
        let unavailable = match interval.held {
            roots::Held::Prepared => return None,
            roots::Held::Lost => Unavailable::Lost,
            roots::Held::Refused => Unavailable::Refused,
        };
        Some(Self {
            first_hour: interval.first_hour,
            last_hour: interval.last_hour,
            family: family.to_owned(),
            unavailable,
            reason: interval.reason.clone(),
        })
    }
}

/// What a gate answers a request with ([`ranges`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranges {
    /// The coverage the request waits for, ending before the first hole in the
    /// requested hours; None before any is accepted.
    pub coverage: Option<Coverage>,
    /// The requested hours the families it reads will not answer, in hour order.
    pub holes: Vec<Hole>,
}

/// What `product`'s queries read on `branch` through `through` ([`Gates::ranges`]), and the
/// declared hours at most: a reader names fewer hours, never more. Its coverage ends before the
/// declared request's first hole, and its holes are noted for the reply ([`with_holes`]).
pub fn ranges(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    product: &str,
    through: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Ranges> {
    let through = within(through);
    let mut ranges = match GATES.get() {
        Some(gates) => gates.ranges(source, content, branch, product, through, cancelled)?,
        None => Ranges {
            coverage: coverage(source, content, branch, cancelled)?,
            holes: Vec::new(),
        },
    };
    ranges.coverage = ranges.coverage.map(before_hole);
    HOLES.with(|found| {
        if let Some(found) = found.borrow_mut().as_mut() {
            for hole in &ranges.holes {
                if !found.contains(hole) {
                    found.push(hole.clone());
                }
            }
        }
    });
    Ok(ranges)
}

thread_local! {
    /// The holes the gates this thread's query asked found; None while no query collects them.
    static HOLES: RefCell<Option<Vec<Hole>>> = const { RefCell::new(None) };
}

/// Run `query`, with the holes the gates it asked found in the hours it read
/// ([`ranges`]), in hour order: its reply names them as never answered.
pub fn with_holes<T>(query: impl FnOnce() -> T) -> (T, Vec<Hole>) {
    // The collection this one runs inside, restored however `query` ends.
    struct Restore(Option<Vec<Hole>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            HOLES.with(|found| *found.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(HOLES.with(|found| found.borrow_mut().replace(Vec::new())));
    let result = query();
    let mut holes = HOLES
        .with(|found| found.borrow_mut().replace(Vec::new()))
        .unwrap_or_default();
    holes.sort_by(|a, b| {
        (a.first_hour, a.last_hour, &a.family).cmp(&(b.first_hour, b.last_hour, &b.family))
    });
    (result, holes)
}

/// `query`'s refusal on `branch`, when a family it reads refused at or before the declared
/// hours ([`Gates::refusal`]).
pub fn refusal(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    query: &str,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    match GATES.get() {
        Some(gates) => gates.refusal(source, content, branch, query, cancelled),
        None => check_cancel(cancelled),
    }
}

/// The version of the layout every prepared family shares: coverage pages,
/// the checkpoint, publication roots and ledgers. A checkpoint of another
/// version is prepared again whole, and coverage of another is refused. What
/// one family publishes has a version of its own in the driver's registry,
/// whose change prepares that family again alone.
pub const SCHEMA: u32 = 56;

/// Accepted extent, distinct from a complete source's timeline and recorded gaps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedCoverage")]
pub struct Coverage {
    pub schema: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub complete: bool,
}

const WALK_PAGE: &str = "preparation-coverage-";
const FAMILY_PAGE: &str = "preparation-family-";

/// The hourly walk's coverage page of a branch.
pub fn coverage_section(branch: u64) -> String {
    format!("{WALK_PAGE}{branch}")
}

/// The branch whose hourly walk's coverage page `section` is.
pub fn walk_of(section: &str) -> Option<u64> {
    branch_of(section.strip_prefix(WALK_PAGE)?)
}

/// A family's or job's coverage page of one branch, with its refusals there.
pub fn family_section(name: &str, branch: u64) -> String {
    format!("{FAMILY_PAGE}{name}-{branch}")
}

/// The family or job, and the branch, whose coverage page `section` is.
pub fn family_of(section: &str) -> Option<(&str, u64)> {
    let (name, branch) = section.strip_prefix(FAMILY_PAGE)?.rsplit_once('-')?;
    Some((name, branch_of(branch)?))
}

fn branch_of(digits: &str) -> Option<u64> {
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// A family's refusal of one branch's recorded data: the first refusing hour
/// and the family's own error text, which its queries answer with. A dependent
/// refusal names the family it came `from`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub hour: i32,
    pub error: String,
    pub from: Option<String>,
}

/// A published coverage page: the accepted extent, and the refusals of the
/// families it gates there, by name, so a query reads both from one small page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    #[serde(flatten)]
    pub coverage: Coverage,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refused: BTreeMap<String, Refusal>,
}

impl Page {
    /// A published page read back, refusing another layout's.
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let page: Self = serde_json::from_slice(bytes)?;
        if page.coverage.schema != SCHEMA || page.coverage.first_hour > page.coverage.last_hour {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported prepared coverage schema",
            ));
        }
        Ok(page)
    }
}

/// The section of the version stamps the hourly walk's and its families'
/// pages were published under.
pub const VERSIONS: &str = "preparation-versions-v1";

/// The version stamps a row preparation's pages were published under: the
/// hourly walk's, and each family prepared on its own's, by name. It is
/// written with every checkpoint, so it always names the pages beside it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versions {
    pub walk: u64,
    pub families: BTreeMap<String, u64>,
}

/// Publishes accepted coverage for family tests that write their own prepared files.
#[cfg(any(test, feature = "test-support"))]
pub fn publish_coverage(
    writer: &mut store::Writer,
    branch: u64,
    first_hour: i32,
    last_hour: i32,
    complete: bool,
) -> io::Result<()> {
    let page = Coverage {
        schema: SCHEMA,
        first_hour,
        last_hour,
        complete,
    };
    writer.write_at(
        &coverage_section(branch),
        0,
        serde_json::to_vec(&page)?.as_slice(),
    )
}

/// The hourly walk's coverage on `branch`, from the gates installed ([`Gates::walk`]), ending
/// before the declared request's first hole.
pub fn coverage(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<Coverage>> {
    let coverage = match GATES.get() {
        Some(gates) => gates.walk(source, content, branch, cancelled)?,
        None => published_walk(source, content, branch, cancelled)?,
    };
    Ok(coverage.map(before_hole))
}

/// The hourly walk's coverage on `branch`, read from the published store.
pub fn published_walk(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<Coverage>> {
    check_cancel(cancelled)?;
    let (store, key) = super::files(source, content)?;
    let Some(mut file) = store.read(&key, &coverage_section(branch), 0)? else {
        note_short(WALK);
        return Ok(None);
    };
    let mut bytes = Vec::new();
    io::Read::read_to_end(&mut file, &mut bytes)?;
    let range = Page::parse(&bytes)?.coverage;
    if !range.complete {
        note_short(WALK);
    }
    Ok(Some(range))
}
