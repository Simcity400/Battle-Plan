//! Country political state, prepared by the segment driver (family `politics_state`): election
//! schedules, governments in exile, balance-of-power membership and civil-war lists, as runs per
//! country holder (its native canonical slot) on the family's series publication
//! ([`facts`], [`emit`]).
//!
//! Each part is read at an hour that may change what it reads (the fields of the rows it
//! reads, and the identities naming its countries, [`Part::reads`]) or that follows a gap; at
//! every other hour it carries its held values, so a change elsewhere, such as a war's
//! casualties, costs it nothing. Each read is the part's whole state ([`observe`]), so a value
//! starts at the first read showing it and ends at the first read that does not. A gap ends
//! every value at its last read: nothing is carried across it, and an unreadable table leaves
//! its values unknown (no run), never zero.
//!
//! - Elections: the recorded game predicate (`has_elections`, 1 or 0) for a country owning
//!   states (the game holds no others' elections), the stored signed interval and the next date
//!   from the shared native calendar step ([`dashboard_base::products::country_people::next_election_date`]). A known "not
//!   held" and an unrecorded eligibility stay apart. An initial read establishes a schedule,
//!   never an election.
//! - Exiles: the exile cards' rule ([`dashboard_base::products::country_people::exiles`]): a positive recorded host makes
//!   an exile; its host holder, legitimacy, manpower, cap, leader resources and its own supplying
//!   read hour are independent runs. The first read in exile is a bound, never the exile's start.
//! - Balances: records have no persistent identity beyond their stored position, so the history
//!   follows what each holder belongs to: per holder and balance identity, the record's value,
//!   active range and trending side, and the record's stored position at the read (an identity
//!   within that read only: holders sharing it at one hour share one record); per holder, the
//!   balance the game shows (the first record listing it, which needs the whole membership
//!   resolved). Two records listing one holder under the same identity keep the earlier, as the
//!   game reads it.
//! - Civil wars: each holder's stored civil-war rivals, one run per rival holder while the
//!   holder's own list names it, and apart from them the shared game predicate
//!   ([`dashboard_text::identity::CivilWars`], indexed once per read) for every listed holder,
//!   and the game's existence check for every holder ever listed ([`super::civil_wars`]).
//!
//! Each part also keeps the hours its tables were read ([`Field::Known`]): a value ending at an
//! hour its part was not read ended unknown, not then. A refusal counts once for each hour it
//! held, once per row it refused. Holders keep their tag runs; a country's history is every
//! holder that held its tag, whole, so a holder renamed keeps its history and a reused tag never
//! joins an unrelated holder.

use super::{BalanceRange, BalanceSide};
use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
    invalid,
    prepared::engine,
    products::{ContentOptions, LoadContext, Product, country_groups::HourSpan},
    triggers::inputs::TableRead,
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, codec::Binary, family::Directory,
    query::EventView,
};
use dashboard_text::catalogue::View;
use emit::{
    BALANCE_RANGE, BALANCE_RECORD, BALANCE_SHOWN, BALANCE_TRENDING, BALANCE_VALUE,
    ELECTION_INTERVAL, ELECTIONS_HELD, ELECTIONS_REFUSED, EXILE_CAP, EXILE_HOST, EXILE_LEADERS,
    EXILE_LEGITIMACY, EXILE_MANPOWER, EXILE_READ, FAMILY as FAMILY_ACTOR, KNOWN, LAST_ELECTION,
    NEXT_ELECTION, REASONS, REFUSALS, TAGS, TOKENS,
};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub mod emit;
pub mod facts;
pub mod observe;
pub mod served;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod walk;

/// The content catalogue a history's names read, where the mode shows any.
pub fn presentation_catalogue(
    source: &Source,
    params: &PoliticsHistoryParams,
    context: &LoadContext,
) -> Option<Result<Arc<Catalogue>, String>> {
    matches!(
        params.mode,
        PoliticsHistoryMode::Country | PoliticsHistoryMode::Balances
    )
    .then(|| {
        context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
            .map_err(|error| error.to_string())
    })
}

/// The family's preparation name and its query's product key.
pub const FAMILY: &str = "politics_state";
pub const QUERY: &str = "politics-state-history";
/// The tables the operator reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Politics,
    TableId::CountryStabilityInputs,
    TableId::SliceReads,
    TableId::PowerBalances,
    TableId::PowerBalanceMembers,
    TableId::CountryCivilWars,
    TableId::CountryCounterInputs,
    TableId::CountryWars,
    TableId::WarRelations,
];

/// The holder slot [`Field::Known`] keys are filed under; no country has it.
const PART_SLOT: u32 = u32::MAX;

/// The parts of the political state read from their own tables.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub enum Part {
    Elections,
    Exiles,
    Balances,
    CivilWars,
}

/// What names every part's countries: each holder's slot and tag.
fn identity_reads() -> [TableRead; 2] {
    [
        TableRead::fields(TableId::Countries, &["canonical_slot", "tag"]),
        TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
    ]
}

impl Part {
    pub const ALL: [Part; 4] = [Part::Elections, Part::Exiles, Part::Balances, Part::CivilWars];

    /// What the part reads of an hour, the identities naming its countries among it: an hour
    /// changing none of it since the hour before leaves the part as last read.
    pub fn reads(self) -> &'static [TableRead] {
        static READS: LazyLock<[Vec<TableRead>; 4]> = LazyLock::new(|| {
            Part::ALL.map(|part| {
                let mut reads = identity_reads().to_vec();
                match part {
                    Part::Elections => reads.extend([
                        TableRead::fields(
                            TableId::Politics,
                            &["last_election", "has_elections", "election_interval", "next_election_cache"],
                        ),
                        TableRead::fields(TableId::CountryCounterInputs, &["owned_state_count"]),
                    ]),
                    Part::Exiles => {
                        // The exile rows' own supplying reads, among every sliced table's.
                        let sliced = (TableId::CountryStabilityInputs.index() as u128) << 32;
                        reads.extend([
                            TableRead::fields(
                                TableId::CountryStabilityInputs,
                                &[
                                    "exile_host",
                                    "exile_legitimacy",
                                    "exile_leaders",
                                    "exile_manpower",
                                    "exile_manpower_cap",
                                ],
                            ),
                            TableRead::fields(TableId::SliceReads, &["sample_hour"])
                                .rows(sliced, sliced | u128::from(u32::MAX)),
                        ]);
                    }
                    Part::Balances => reads.extend([
                        TableRead::fields(
                            TableId::PowerBalances,
                            &["value_raw", "balance", "trending", "active_range"],
                        ),
                        TableRead::fields(TableId::PowerBalanceMembers, &["country"]),
                    ]),
                    Part::CivilWars => reads.extend(dashboard_text::identity::civil_war_reads()),
                }
                reads
            })
        });
        &READS[self as usize]
    }
}

/// What one value of a holder is.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub enum Field {
    /// 1 when the game holds elections, 0 when it does not.
    ElectionsHeld,
    /// 1 while the holder's schedule was read but whether it holds elections could not be: its
    /// eligibility is unknown, not denied.
    ElectionsRefused,
    ElectionInterval,
    NextElection,
    /// The stored last election's hour, where it is a date rather than an initializer the next
    /// date's cache stands in for.
    LastElection,
    /// The host's holder slot.
    ExileHost,
    ExileLegitimacy,
    ExileManpower,
    ExileCap,
    ExileLeaders,
    ExileRead,
    /// The fixed5 value of the record listing the holder under `balance`.
    BalanceValue(String),
    /// The record's active range, a token.
    BalanceRange(String),
    /// The record's trending side, a token.
    BalanceTrending(String),
    /// The stored position of the record listing the holder under `balance`.
    BalanceRecord(String),
    /// The balance the game shows the holder, a token.
    BalanceShown,
    /// 1 while the holder's stored civil-war list names this rival holder.
    CivilWarRival(u32),
    /// 1 while the game's civil-war predicate holds for the holder.
    CivilWar,
    /// 1 while a holder ever listed in a civil war owns a state, the game's existence check; an
    /// hour that cannot say has no run.
    Exists,
    /// 1 at each hour the part's tables were read.
    Known(Part),
}

/// Consecutive hours holding one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: i128,
}

/// The publication's runs of `key` over `from..=to`, clipped to it.
pub(crate) fn runs<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: u64,
    from: i32,
    to: i32,
) -> io::Result<Vec<Run>> {
    Ok(engine::runs(snapshot, key, from, to)?
        .into_iter()
        .map(|run| Run {
            first_hour: run.span.first,
            last_hour: run.span.last,
            value: run.value,
        })
        .collect())
}

/// The value `key` holds at `hour`.
fn at<P: PageRead>(snapshot: &mut Snapshot<'_, P>, key: Option<u64>, hour: i32) -> io::Result<Option<i128>> {
    match key {
        Some(key) => Ok(runs(snapshot, key, hour, hour)?.first().map(|run| run.value)),
        None => Ok(None),
    }
}

/// Each holder's actor, by slot.
pub(crate) fn holders(directory: &Directory) -> BTreeMap<u32, u64> {
    directory
        .actors
        .iter()
        .enumerate()
        .filter_map(|(actor, named)| Some((emit::holder_of(&named.name)?, actor as u64)))
        .collect()
}

/// Each holder's tag from each hour it was first read with it.
pub(crate) fn tags<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
) -> io::Result<BTreeMap<u32, Vec<(i32, String)>>> {
    let malformed = || invalid("A prepared holder tag is not valid");
    let mut tags = BTreeMap::<u32, Vec<(i32, String)>>::new();
    let Some(key) = directory.key(FAMILY_ACTOR, TAGS, None) else {
        return Ok(tags);
    };
    let selection = [Selection {
        key,
        membership: None,
    }];
    let native = snapshot.publication().accepted;
    let mut cursor = None;
    loop {
        let mut events = Vec::new();
        let page = snapshot.event_page(
            EventQuery {
                selection: &selection,
                native,
                as_of: None,
                order: EventOrder::Native,
            },
            cursor,
            1 << 20,
            |event| {
                events.push(event);
                Ok(())
            },
        )?;
        for event in events {
            let EventView::Family(event) = event else {
                return Err(malformed());
            };
            if event.kind != emit::kind::TAG || event.fields.len != 2 * Cell::SIZE as u64 {
                return Err(malformed());
            }
            let slot = match snapshot.field(event.fields, 0)? {
                Cell::Unsigned(slot) => u32::try_from(slot).map_err(|_| malformed())?,
                _ => return Err(malformed()),
            };
            let tag = match snapshot.field(event.fields, 1)? {
                Cell::Text(blob) => {
                    let mut bytes = vec![0; usize::try_from(blob.len).map_err(|_| malformed())?];
                    snapshot.cell_bytes(blob, 0, &mut bytes)?;
                    String::from_utf8(bytes).map_err(|_| malformed())?
                }
                _ => return Err(malformed()),
            };
            tags.entry(slot).or_default().push((event.native_hour, tag));
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    Ok(tags)
}

/// The text of token `id`.
fn token(directory: &Directory, id: i128) -> Option<String> {
    let id = u64::try_from(id).ok().filter(|&id| id < REASONS)?;
    let text = directory.scalar(TOKENS + id)?.text()?;
    std::str::from_utf8(text).ok().map(str::to_owned)
}

/// Each refusal's hours through `through`, each counted as often as it held then.
pub(crate) fn refusals<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    through: i32,
) -> io::Result<BTreeMap<String, u64>> {
    let mut refusals = BTreeMap::new();
    for &(id, key) in directory.dimensions(FAMILY_ACTOR, REFUSALS) {
        let reason = directory
            .scalar(REASONS + id)
            .and_then(|scalar| scalar.text())
            .and_then(|text| std::str::from_utf8(text).ok())
            .ok_or_else(|| invalid("A prepared refusal names no reason"))?
            .to_owned();
        let count: u64 = runs(snapshot, key, i32::MIN, through)?
            .iter()
            .map(|run| {
                let hours = i64::from(run.last_hour) - i64::from(run.first_hour) + 1;
                u64::try_from(run.value).unwrap_or(0) * hours as u64
            })
            .sum();
        if count != 0 {
            *refusals.entry(reason).or_default() += count;
        }
    }
    Ok(refusals)
}

/// A holder's balance runs by identity (value, range, trending and record), and the runs of the
/// balance shown, gathered over `spans`.
#[derive(Default)]
struct Gathered {
    balances: BTreeMap<String, [Vec<Run>; 4]>,
    shown: Vec<Run>,
}

impl Gathered {
    fn gather<P: PageRead>(
        &mut self,
        snapshot: &mut Snapshot<'_, P>,
        directory: &Directory,
        actor: Option<u64>,
        spans: &[(i32, i32)],
        through: i32,
    ) -> io::Result<()> {
        let Some(actor) = actor else {
            return Ok(());
        };
        for (part, measure) in [BALANCE_VALUE, BALANCE_RANGE, BALANCE_TRENDING, BALANCE_RECORD]
            .into_iter()
            .enumerate()
        {
            for &(id, key) in directory.dimensions(actor, measure) {
                let balance = token(directory, i128::from(id))
                    .ok_or_else(|| invalid("A prepared balance names no token"))?;
                self.balances.entry(balance).or_default()[part]
                    .extend(clip(runs(snapshot, key, i32::MIN, through)?, spans, through));
            }
        }
        if let Some(key) = directory.key(actor, BALANCE_SHOWN, None) {
            self.shown
                .extend(clip(runs(snapshot, key, i32::MIN, through)?, spans, through));
        }
        Ok(())
    }

    /// The balances in identity order, with names and authored presentation, and the shown runs
    /// by identity token.
    fn finish(
        self,
        directory: &Directory,
        presentation: &Presentation<'_>,
        names: &mut BTreeMap<String, String>,
        refused: &mut u64,
    ) -> (Vec<BalanceHistory>, Vec<PoliticsRun>) {
        let token_text = |value: i128| token(directory, value);
        let sorted = |mut runs: Vec<Run>| {
            runs.sort_by_key(|run| run.first_hour);
            runs
        };
        let mut balances = Vec::new();
        for (token, [values, range, trending, record]) in self.balances {
            let (mut left, mut right, mut ranges) = presentation.authored(&token);
            presentation.name(&token, names, refused);
            present(presentation, names, refused, &mut left, &mut right, &mut ranges);
            let range = text_runs(sorted(range), token_text);
            let trending = text_runs(sorted(trending), token_text);
            for run in range.iter().chain(&trending) {
                presentation.name(&run.value, names, refused);
            }
            balances.push(BalanceHistory {
                name: names.get(&token).cloned(),
                values: text_runs(sorted(values), |value| Some(value.to_string())),
                record: text_runs(sorted(record), |value| Some(value.to_string())),
                range,
                trending,
                left,
                right,
                ranges,
                token,
            });
        }
        let shown = text_runs(sorted(self.shown), token_text);
        (balances, shown)
    }
}

/// The hours `part`'s tables were read, `from..=through`.
pub(crate) fn known<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    part: Part,
    from: i32,
    through: i32,
) -> io::Result<Vec<HourSpan>> {
    let Some(key) = directory.key(FAMILY_ACTOR, KNOWN, Some(part as u64)) else {
        return Ok(Vec::new());
    };
    Ok(runs(snapshot, key, from, through)?
        .into_iter()
        .map(|run| HourSpan {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
        })
        .collect())
}

/// One government in exile at an hour: its holder slot and tag, its host's holder slot and tag,
/// and its legitimacy, manpower, cap, leaders and resources' read hour, each none where unread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExileAt {
    pub slot: u32,
    pub tag: Option<String>,
    pub host: u32,
    pub host_tag: Option<String>,
    pub values: [Option<i128>; 5],
}

/// One holder's political state at an hour, as the history holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AtHour {
    /// 1 when the game holds the holder's elections, 0 when it does not.
    pub held: Option<i128>,
    /// Whether its schedule was read but its eligibility was not.
    pub refused: bool,
    pub interval: Option<i128>,
    pub next: Option<i128>,
    /// Whether the exiles part was read at the hour.
    pub exiles_known: bool,
    /// The exiles the holder is or hosts, by holder slot.
    pub exiles: Vec<ExileAt>,
}

/// The exile measures an [`ExileAt`] holds, in its order.
const EXILE_FIELDS: [u64; 5] = [
    EXILE_LEGITIMACY,
    EXILE_MANPOWER,
    EXILE_CAP,
    EXILE_LEADERS,
    EXILE_READ,
];

/// `holder`'s elections and the exiles it is or hosts at `hour`, from the publication
/// `snapshot` pins.
pub fn read_at<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    holder: u32,
    hour: i32,
) -> io::Result<AtHour> {
    let holders = holders(directory);
    let actor = holders.get(&holder).copied();
    let own = |measure| actor.and_then(|actor| directory.key(actor, measure, None));
    let mut at_hour = AtHour {
        held: at(snapshot, own(ELECTIONS_HELD), hour)?,
        refused: at(snapshot, own(ELECTIONS_REFUSED), hour)? == Some(1),
        interval: at(snapshot, own(ELECTION_INTERVAL), hour)?,
        next: at(snapshot, own(NEXT_ELECTION), hour)?,
        exiles_known: at(
            snapshot,
            directory.key(FAMILY_ACTOR, KNOWN, Some(Part::Exiles as u64)),
            hour,
        )? == Some(1),
        exiles: Vec::new(),
    };
    // Its own exile, and those it hosts, by holder slot.
    let mut exiles = Vec::new();
    for (&slot, &actor) in &holders {
        let Some(key) = directory.key(actor, EXILE_HOST, None) else {
            continue;
        };
        let Some(host) = at(snapshot, Some(key), hour)?
            .and_then(|host| u32::try_from(host).ok())
            .filter(|&host| host > 0)
        else {
            continue;
        };
        if slot == holder || host == holder {
            exiles.push((slot, actor, host));
        }
    }
    if exiles.is_empty() {
        return Ok(at_hour);
    }
    let tags = tags(snapshot, directory)?;
    let tag = |slot: u32| {
        tags.get(&slot)
            .and_then(|tags| tag_at(tags, hour))
            .map(str::to_owned)
    };
    for (slot, actor, host) in exiles {
        let mut values = [None; 5];
        for (value, measure) in values.iter_mut().zip(EXILE_FIELDS) {
            *value = at(snapshot, directory.key(actor, measure, None), hour)?;
        }
        at_hour.exiles.push(ExileAt {
            slot,
            tag: tag(slot),
            host,
            host_tag: tag(host),
            values,
        });
    }
    Ok(at_hour)
}

/// `holder`'s elections and the exiles it is or hosts at `hour`; none before the history's
/// first publication or past the hours it has accepted.
pub fn at_hour(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
    holder: u32,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<AtHour>> {
    let Some(opened) =
        dashboard_base::prepared::published::open(source, content, FAMILY, emit::SCHEMA, branch)?
    else {
        return Ok(None);
    };
    opened.read(cancelled, |snapshot, directory| {
        if hour > snapshot.publication().accepted.last {
            return Ok(None);
        }
        read_at(snapshot, directory, holder, hour).map(Some)
    })
}

pub(crate) fn tag_at(runs: &[(i32, String)], hour: i32) -> Option<&str> {
    runs.iter()
        .rev()
        .find(|(from, _)| *from <= hour)
        .map(|(_, tag)| tag.as_str())
}

fn clip(runs: Vec<Run>, spans: &[(i32, i32)], through: i32) -> Vec<Run> {
    let mut out = Vec::new();
    for run in runs {
        for &(from, to) in spans {
            let first = run.first_hour.max(from);
            let last = run.last_hour.min(to).min(through);
            if first <= last {
                out.push(Run {
                    first_hour: first,
                    last_hour: last,
                    value: run.value,
                });
            }
        }
    }
    out.sort_by_key(|run| run.first_hour);
    out
}

fn text_runs(runs: Vec<Run>, text: impl Fn(i128) -> Option<String>) -> Vec<PoliticsRun> {
    runs.into_iter()
        .filter_map(|run| {
            Some(PoliticsRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                value: text(run.value)?,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum PoliticsHistoryMode {
    /// One country's balances over the whole branch.
    Country,
    /// Every election schedule over the whole branch.
    Elections,
    /// Every government in exile over the whole branch.
    Exiles,
    /// Every holder's balances over the whole branch.
    Balances,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoliticsHistoryParams {
    pub branch_id: String,
    pub mode: PoliticsHistoryMode,
    /// The country, for [`PoliticsHistoryMode::Country`].
    #[serde(default)]
    #[ts(optional)]
    pub tag: Option<String>,
    /// The window [`PoliticsHistoryMode::Exiles`] answers, both ends included.
    #[serde(default)]
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[serde(default)]
    #[ts(optional)]
    pub last_hour: Option<i32>,
}

/// Consecutive reads holding one value, as an exact decimal string.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoliticsRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: String,
}

/// A holder's tag from `hour` until its next entry.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HolderTag {
    pub hour: i32,
    pub tag: String,
}

/// One holder's election schedule over the branch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ElectionHistory {
    pub tags: Vec<HolderTag>,
    /// "1" when the game holds elections, "0" when it does not.
    pub held: Vec<PoliticsRun>,
    /// "1" while its schedule was read but its eligibility was not: unknown, not denied.
    pub refused: Vec<PoliticsRun>,
    /// Signed months.
    pub interval: Vec<PoliticsRun>,
    /// The next election's hour.
    pub next: Vec<PoliticsRun>,
    /// The term's start: the stored last election's hour, where it is a date.
    pub last: Vec<PoliticsRun>,
}

/// One holder's exile over the branch: each field's runs while a positive
/// host was read, each independent of the others.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExileHistory {
    pub tags: Vec<HolderTag>,
    /// The host's holder id.
    pub host: Vec<PoliticsRun>,
    /// Fixed5 legitimacy.
    pub legitimacy: Vec<PoliticsRun>,
    pub manpower: Vec<PoliticsRun>,
    pub cap: Vec<PoliticsRun>,
    pub leaders: Vec<PoliticsRun>,
    /// The hour the exile's resources were last read.
    pub read: Vec<PoliticsRun>,
}

/// One holder's balances over the branch.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HolderBalances {
    pub holder: String,
    pub tags: Vec<HolderTag>,
    pub balances: Vec<BalanceHistory>,
    /// The balance the game shows the holder, by identity token.
    pub shown: Vec<PoliticsRun>,
}

/// One balance identity a country belonged to. Range and trending runs
/// hold content tokens.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BalanceHistory {
    pub token: String,
    pub name: Option<String>,
    pub left: Option<BalanceSide>,
    pub right: Option<BalanceSide>,
    /// The authored ranges in authored order; empty when content refuses.
    pub ranges: Vec<BalanceRange>,
    pub values: Vec<PoliticsRun>,
    /// The record's stored position: holders sharing it at an hour share that
    /// record. Not an identity across hours.
    pub record: Vec<PoliticsRun>,
    pub range: Vec<PoliticsRun>,
    pub trending: Vec<PoliticsRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoliticsHistoryReply {
    pub branch_id: String,
    pub mode: PoliticsHistoryMode,
    /// Through the last prepared hour; later hours are still being prepared.
    pub last_hour: Option<i32>,
    pub tag: Option<String>,
    /// The hours the mode's part was read; others are unknown.
    pub known: Vec<HourSpan>,
    pub elections: Vec<ElectionHistory>,
    /// Content tokens to their in-game names, where they resolve.
    pub names: BTreeMap<String, String>,
    /// Every government in exile over the branch.
    pub exile_history: Vec<ExileHistory>,
    /// Each holder id's tags, for the holders runs name.
    pub holders: BTreeMap<String, Vec<HolderTag>>,
    /// Every holder's balances over the branch; for a country, every holder
    /// that held its tag, whole.
    pub holder_balances: Vec<HolderBalances>,
    pub warnings: Vec<String>,
}

fn holder_tags(runs: &[(i32, String)]) -> Vec<HolderTag> {
    runs.iter()
        .map(|(hour, tag)| HolderTag {
            hour: *hour,
            tag: tag.clone(),
        })
        .collect()
}

/// Names and authored presentation from the recording's content stack.
struct Presentation<'a> {
    catalogue: Option<&'a Catalogue>,
    balances: Option<&'a installed_content_core::power_balance::Database>,
}
impl Presentation<'_> {
    fn name(&self, token: &str, names: &mut BTreeMap<String, String>, refused: &mut u64) {
        if token.is_empty() || names.contains_key(token) {
            return;
        }
        match self
            .catalogue
            .and_then(|catalogue| catalogue.localised(token, View::Plain))
        {
            Some(text) if !(text.contains('[') && text.contains(']')) => {
                names.insert(token.to_owned(), text);
            }
            _ if self.catalogue.is_some() => *refused += 1,
            _ => {}
        }
    }
    fn side(&self, token: &[u8]) -> BalanceSide {
        let token = String::from_utf8_lossy(installed_content_core::parser::c_string(token)).into_owned();
        BalanceSide {
            name: None,
            token,
        }
    }
    fn authored(
        &self,
        token: &str,
    ) -> (Option<BalanceSide>, Option<BalanceSide>, Vec<BalanceRange>) {
        let Some(balances) = self.balances else {
            return (None, None, Vec::new());
        };
        let definition = balances.definition(token.as_bytes());
        let ranges = balances
            .ranges(token.as_bytes())
            .unwrap_or_default()
            .iter()
            .map(|range| BalanceRange {
                token: String::from_utf8_lossy(installed_content_core::parser::c_string(&range.id))
                    .into_owned(),
                name: None,
                min: Some(range.min.to_string()),
                max: Some(range.max.to_string()),
            })
            .collect();
        (
            definition.map(|definition| self.side(&definition.left)),
            definition.map(|definition| self.side(&definition.right)),
            ranges,
        )
    }
}

/// Fills every side and range name from `names`, gathering them first.
fn present(
    presentation: &Presentation<'_>,
    names: &mut BTreeMap<String, String>,
    refused: &mut u64,
    left: &mut Option<BalanceSide>,
    right: &mut Option<BalanceSide>,
    ranges: &mut [BalanceRange],
) {
    for side in [left.as_mut(), right.as_mut()].into_iter().flatten() {
        presentation.name(&side.token, names, refused);
        side.name = names.get(&side.token).cloned();
    }
    for range in ranges {
        presentation.name(&range.token, names, refused);
        range.name = names.get(&range.token).cloned();
    }
}

/// The reply before the family's first publication on the branch: no hour is prepared yet.
pub fn unpublished(params: &PoliticsHistoryParams, through: Option<i32>) -> io::Result<PoliticsHistoryReply> {
    params
        .branch_id
        .parse::<u64>()
        .map_err(|_| invalid("Invalid politics history branch"))?;
    Ok(PoliticsHistoryReply {
        branch_id: params.branch_id.clone(),
        mode: params.mode,
        last_hour: through,
        tag: params.tag.clone(),
        known: Vec::new(),
        elections: Vec::new(),
        names: BTreeMap::new(),
        exile_history: Vec::new(),
        holders: BTreeMap::new(),
        holder_balances: Vec::new(),
        warnings: Vec::new(),
    })
}

/// The history `params` asks for, through `through`, the last prepared hour, from the
/// publication `snapshot` pins.
pub fn query<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    params: &PoliticsHistoryParams,
    through: Option<i32>,
    catalogue: Option<Result<Arc<Catalogue>, String>>,
    cancelled: &AtomicBool,
) -> io::Result<PoliticsHistoryReply> {
    let mut reply = unpublished(params, through)?;
    let Some(through) = through else {
        return Ok(reply);
    };
    let prepared = through;
    let check = || {
        if cancelled.load(Ordering::Acquire) {
            Err(invalid("Politics history request cancelled"))
        } else {
            Ok(())
        }
    };
    let catalogue = match catalogue {
        Some(Ok(catalogue)) => Some(catalogue),
        Some(Err(error)) => {
            reply.warnings.push(format!("1 refused balance names: {error}"));
            None
        }
        None => None,
    };
    let effects = catalogue
        .as_deref()
        .map(|catalogue| catalogue.scripted_content().and_then(|content| content.effects()));
    if let Some(Err(error)) = &effects {
        reply
            .warnings
            .push(format!("1 refused balance ranges: {error}"));
    }
    let presentation = Presentation {
        catalogue: catalogue.as_deref(),
        balances: effects
            .as_ref()
            .and_then(|effects| effects.as_ref().ok())
            .and_then(|effects| effects.view(None).power_balances),
    };
    let holders = holders(directory);
    let tags = tags(snapshot, directory)?;
    let part = match params.mode {
        PoliticsHistoryMode::Elections => Part::Elections,
        PoliticsHistoryMode::Exiles => Part::Exiles,
        PoliticsHistoryMode::Country | PoliticsHistoryMode::Balances => Part::Balances,
    };
    // The exile history is one window at a time: every exile's runs over a whole branch
    // outgrow a reply.
    let (from, through) = match params.mode {
        PoliticsHistoryMode::Exiles => match (params.first_hour, params.last_hour) {
            (Some(first), Some(last)) if first <= last => (first, last.min(through)),
            _ => return Err(invalid("An exile history names its window")),
        },
        _ => (i32::MIN, through),
    };
    let window = [(from, through)];
    reply.known = known(snapshot, directory, part, from, through)?;
    let holder_tags_of =
        |slot: u32| holder_tags(tags.get(&slot).map(Vec::as_slice).unwrap_or_default());
    let mut refused_names = 0_u64;
    match params.mode {
        PoliticsHistoryMode::Elections => {
            for (slot, held) in &tags {
                check()?;
                let mut history = ElectionHistory {
                    tags: holder_tags(held),
                    held: Vec::new(),
                    refused: Vec::new(),
                    interval: Vec::new(),
                    next: Vec::new(),
                    last: Vec::new(),
                };
                if let Some(&actor) = holders.get(slot) {
                    for (target, measure) in [
                        (&mut history.held, ELECTIONS_HELD),
                        (&mut history.refused, ELECTIONS_REFUSED),
                        (&mut history.interval, ELECTION_INTERVAL),
                        (&mut history.next, NEXT_ELECTION),
                        (&mut history.last, LAST_ELECTION),
                    ] {
                        if let Some(key) = directory.key(actor, measure, None) {
                            *target = text_runs(runs(snapshot, key, i32::MIN, through)?, |value| {
                                Some(value.to_string())
                            });
                        }
                    }
                }
                if !history.held.is_empty()
                    || !history.refused.is_empty()
                    || !history.interval.is_empty()
                {
                    reply.elections.push(history);
                }
            }
        }
        PoliticsHistoryMode::Country => {
            let tag = params
                .tag
                .as_deref()
                .ok_or_else(|| invalid("A country history names its country"))?;
            // Every holder that held the tag, whole: the caller picks the one holding it at its
            // hour and follows that holder through renames.
            for (&slot, held) in &tags {
                check()?;
                if !held.iter().any(|(_, held)| held == tag) {
                    continue;
                }
                let mut gathered = Gathered::default();
                gathered.gather(snapshot, directory, holders.get(&slot).copied(), &window, through)?;
                let (balances, shown) =
                    gathered.finish(directory, &presentation, &mut reply.names, &mut refused_names);
                reply.holder_balances.push(HolderBalances {
                    holder: slot.to_string(),
                    tags: holder_tags(held),
                    balances,
                    shown,
                });
            }
        }
        PoliticsHistoryMode::Balances => {
            for (&slot, &actor) in &holders {
                check()?;
                if directory.dimensions(actor, BALANCE_VALUE).is_empty() {
                    continue;
                }
                let mut gathered = Gathered::default();
                gathered.gather(snapshot, directory, Some(actor), &window, through)?;
                let (balances, shown) =
                    gathered.finish(directory, &presentation, &mut reply.names, &mut refused_names);
                reply.holder_balances.push(HolderBalances {
                    holder: slot.to_string(),
                    tags: holder_tags_of(slot),
                    balances,
                    shown,
                });
            }
        }
        PoliticsHistoryMode::Exiles => {
            for (&slot, &actor) in &holders {
                check()?;
                if directory.key(actor, EXILE_HOST, None).is_none() {
                    continue;
                }
                let mut history = ExileHistory {
                    tags: holder_tags_of(slot),
                    host: Vec::new(),
                    legitimacy: Vec::new(),
                    manpower: Vec::new(),
                    cap: Vec::new(),
                    leaders: Vec::new(),
                    read: Vec::new(),
                };
                for (target, measure) in [
                    (&mut history.host, EXILE_HOST),
                    (&mut history.legitimacy, EXILE_LEGITIMACY),
                    (&mut history.manpower, EXILE_MANPOWER),
                    (&mut history.cap, EXILE_CAP),
                    (&mut history.leaders, EXILE_LEADERS),
                    (&mut history.read, EXILE_READ),
                ] {
                    if let Some(key) = directory.key(actor, measure, None) {
                        *target = text_runs(runs(snapshot, key, from, through)?, |value| {
                            Some(value.to_string())
                        });
                    }
                }
                if history.host.is_empty() {
                    continue;
                }
                for run in &history.host {
                    if let Ok(host) = run.value.parse::<u32>()
                        && let Some(held) = tags.get(&host)
                    {
                        reply
                            .holders
                            .entry(run.value.clone())
                            .or_insert_with(|| holder_tags(held));
                    }
                }
                reply.exile_history.push(history);
            }
        }
    }
    if refused_names != 0 {
        reply.warnings.push(format!(
            "{refused_names} refused balance names: no plain localisation in the recorded content"
        ));
    }
    for (reason, count) in refusals(snapshot, directory, prepared)? {
        reply
            .warnings
            .push(format!("{count} refused political state reads: {reason}"));
    }
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

/// The prepared history's product metadata; its query is routed by key.
pub struct PoliticsStateHistory;
impl Product for PoliticsStateHistory {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = PoliticsHistoryParams;
    type Reply = PoliticsHistoryReply;
    fn types() -> Vec<String> {
        vec![
            PoliticsHistoryParams::decl(),
            PoliticsHistoryMode::decl(),
            PoliticsRun::decl(),
            HolderTag::decl(),
            ElectionHistory::decl(),
            ExileHistory::decl(),
            BalanceHistory::decl(),
            HolderBalances::decl(),
            PoliticsHistoryReply::decl(),
        ]
    }
    fn validate(p: &PoliticsHistoryParams) -> Result<(), &'static str> {
        p.branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid politics history branch")?;
        match p.mode {
            PoliticsHistoryMode::Country => {
                dashboard_base::products::validate_tag(p.tag.as_deref().ok_or("Missing country")?)
            }
            PoliticsHistoryMode::Exiles => match (p.first_hour, p.last_hour) {
                (Some(first), Some(last)) if first <= last => Ok(()),
                _ => Err("An exile history names its window"),
            },
            _ => Ok(()),
        }
    }
}
