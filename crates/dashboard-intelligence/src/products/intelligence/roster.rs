//! Agencies' operatives: the asked countries' active lists at an hour ([`OperativesRoster`], rows at
//! an hour of the daily census), and the operatives that left them before the hour
//! ([`OperativesDeparted`]: the census's lifetimes, kept in the index at open, each one's
//! assignments by one history read and its last row by one reader moving forward).

use super::{
    Country, IDENTITY, Tags, branch, missing,
    operative_content::{self, Art, CAPTURED, DISBANDED, KILLED, OPERATION, OPERATIVE_CATEGORY, OperativeContent},
    placements::ACTIVE,
};
use crate::{
    Source,
    products::{
        LoadContext, Product,
        at_hour::{At, AtHour, Owner, Read},
        country_ids::{Names, RunLinks},
    },
};
use dashboard_base::products::validate_tag;
use dashboard_query::identity::{Holder, Holders};
use installed_content_country_names::country_names::CountryNames;
use replay_data::rows::{CharacterOperativeRow, CharacterPortraitRow, OperativeMembershipRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

/// What the census is named and drawn through: the operations its rows work in, the characters
/// that hold its portraits, and the countries that capture it.
const NAMING: [TableId; 5] = [TableId::CountryAppearance, TableId::ReservedCountryNaming, TableId::Politics, TableId::PoliticalParties, TableId::Autonomy];
const TABLES: [TableId; 13] = [
    TableId::OperativeMemberships,
    IDENTITY[0],
    IDENTITY[1],
    TableId::SimulationFeatures,
    TableId::CharacterOperatives,
    TableId::CharacterPortraits,
    TableId::Operations,
    TableId::LexicalTokens,
    NAMING[0],
    NAMING[1],
    NAMING[2],
    NAMING[3],
    NAMING[4],
];

/// The DLC mask bit that turns the game's intelligence agencies on.
const AGENCY_FEATURE: u32 = 1 << 5;

/// OperativeMemberships' key: id above the roster's country above its kind.
pub fn roster_of(key: u128) -> (u32, u8) {
    ((key >> 8) as u32, key as u8)
}

/// Whether the game has ended this membership: a disbanded or killed operative is departed.
pub fn terminal(assignment: u32) -> bool {
    matches!(assignment, DISBANDED | KILLED)
}

/// Why an operative left the list, from its last assignment.
pub fn departure(assignment: Option<u32>) -> &'static str {
    match assignment {
        Some(DISBANDED) => "Disbanded",
        Some(KILLED) => "Killed",
        _ => "Left (reason not recorded)",
    }
}

/// One operative as the census holds it, named and drawn through the recorded content.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RosterOperative {
    /// The full handle, as a decimal string.
    pub id: String,
    /// The asked country whose agency holds it.
    pub roster: String,
    /// The country it works for.
    pub owner: Country,
    pub name: Option<String>,
    /// The game saved its name empty.
    pub unnamed: bool,
    pub name_missing: Option<String>,
    /// Its portrait, an index into the reply's `artwork`.
    pub portrait: Option<u32>,
    pub portrait_missing: Option<String>,
    pub skill: Option<i32>,
    /// Fixed5, as recorded.
    pub experience_raw: String,
    /// The game's skill line and its progress to the next level (1 a whole level, never clamped).
    pub experience_text: Option<String>,
    pub experience_ratio: Option<f64>,
    pub experience_missing: Option<String>,
    /// As the game numbers it: 0 captured, 1 cooling down, 3 on a mission, 4 in an operation.
    pub assignment: u32,
    pub assignment_name: Option<String>,
    /// A captivity's or cooldown's remaining days, and its progress (never clamped).
    pub assignment_duration: Option<String>,
    pub assignment_progress: Option<f64>,
    pub assignment_missing: Option<String>,
    /// The assignment's icon, an index into the reply's `artwork`.
    pub assignment_icon: Option<u32>,
    pub assignment_icon_missing: Option<String>,
    /// Where it works, when the census records a target.
    pub target: Option<Country>,
    pub captured: bool,
    /// Who holds it captive, and since when, while captured.
    pub capturer: Option<Country>,
    pub captured_hour: Option<i32>,
}

/// Artwork a reply draws once each, by index.
#[derive(Default)]
pub struct Artwork {
    urls: Vec<String>,
    index: HashMap<String, u32>,
}

impl Artwork {
    fn add(&mut self, url: String) -> u32 {
        if let Some(&at) = self.index.get(&url) {
            return at;
        }
        let at = u32::try_from(self.urls.len()).unwrap_or(u32::MAX);
        self.index.insert(url.clone(), at);
        self.urls.push(url);
        at
    }

    /// `art` drawn, as its index, or why it is not shown.
    fn draw(&mut self, content: &OperativeContent, art: &Art) -> (Option<u32>, Option<String>) {
        match content.draw(art) {
            Ok(url) => (Some(self.add(url)), None),
            Err(reason) => (None, Some(reason)),
        }
    }
}

/// What a row is shown through besides its own cells, at the hour it is read.
pub struct Around<'a> {
    pub content: Result<&'a OperativeContent, &'a str>,
    /// The hour the row's cells are of: a captivity's or cooldown's days count to it.
    pub now: i32,
    /// A raw country id's tag at the hour.
    pub tag: &'a dyn Fn(u32) -> Option<String>,
    /// The capturer's name, by raw country id.
    pub capturer_name: &'a dyn Fn(u32) -> Result<String, String>,
    /// The operation token the row's operation runs, by operation id.
    pub operation: &'a dyn Fn(u64) -> Option<String>,
    /// The character holding the operative's portrait, and the portraits read.
    pub character: Option<u32>,
    pub portraits: Option<&'a [CharacterPortraitRow]>,
}

impl RosterOperative {
    pub fn of(row: &OperativeMembershipRow, roster: &str, around: &Around<'_>, artwork: &mut Artwork) -> Self {
        let country = |raw: u32| Country { id: raw, tag: (around.tag)(raw) };
        let content = around.content.ok();
        let unconfigured = |what: &str| around.content.err().map(|why| format!("{what} needs the recorded content: {why}"));
        let name = operative_content::name(content.map(OperativeContent::localisation), row);
        let (portrait, portrait_missing) = match content.map(|content| (content, content.portrait(around.character, around.portraits))) {
            Some((content, Ok(art))) => artwork.draw(content, &art),
            Some((_, Err(reason))) => (None, Some(reason)),
            None => (None, unconfigured("The portrait")),
        };
        let experience = content.map(|content| content.experience(row.skill, row.experience_raw)).unwrap_or_default();
        let captured = row.assignment == CAPTURED;
        let capturer_name = captured.then(|| (around.capturer_name)(row.capturer_country));
        let operation = (row.assignment == OPERATION).then(|| match row.operation {
            Recorded::Value(Some(id)) if id != 0 => (around.operation)(id),
            _ => None,
        });
        let assignment = content.map(|content| {
            let name = capturer_name.as_ref().and_then(|name| name.as_ref().ok()).map(String::as_str);
            let mut assignment = content.assignment(row, around.now, operation.flatten().as_deref(), name);
            if let Some(Err(why)) = &capturer_name {
                assignment.missing = Some(match assignment.missing.take() {
                    Some(missing) if missing.contains(why.as_str()) => missing,
                    Some(missing) => format!("{why} {missing}"),
                    None => why.clone(),
                });
            }
            assignment
        });
        let (assignment_icon, assignment_icon_missing) = match (content, assignment.as_ref().and_then(|assignment| assignment.icon.as_ref())) {
            (Some(content), Some(icon)) => artwork.draw(content, icon),
            _ => (None, None),
        };
        let assignment = assignment.unwrap_or_default();
        Self {
            id: row.id.to_string(),
            roster: roster.to_owned(),
            owner: country(row.owner_country),
            name: name.name,
            unnamed: name.unnamed,
            name_missing: name.missing,
            portrait,
            portrait_missing,
            skill: row.skill,
            experience_raw: row.experience_raw.to_string(),
            experience_text: experience.text,
            experience_ratio: experience.ratio,
            experience_missing: experience.missing.or_else(|| unconfigured("The skill level")),
            assignment: row.assignment,
            assignment_name: assignment.name,
            assignment_duration: assignment.duration,
            assignment_progress: assignment.progress,
            assignment_missing: assignment.missing.or_else(|| unconfigured("The assignment's text")),
            assignment_icon,
            assignment_icon_missing,
            target: match row.target_country {
                Recorded::Value(Some(target)) => Some(country(target)),
                _ => None,
            },
            captured,
            capturer: captured.then(|| country(row.capturer_country)),
            captured_hour: (captured && row.captured_hour != engine_native::cgame_date::NULL_RAW_HOUR).then_some(row.captured_hour),
        }
    }
}

/// Each operative reference's character, read from the characters an operative category links:
/// none where two characters claim one reference.
fn characters(rows: &[CharacterOperativeRow]) -> HashMap<u64, Option<u32>> {
    let mut out: HashMap<u64, Option<u32>> = HashMap::new();
    for row in rows.iter().filter(|row| row.resolved && row.category == OPERATIVE_CATEGORY) {
        let linked = out.entry(row.reference).or_insert(Some(row.character));
        if *linked != Some(row.character) {
            *linked = None;
        }
    }
    out
}

/// Each operation's definition token, by operation id.
fn operation_tokens(frame: &DecodedFrame) -> HashMap<u64, String> {
    let (Ok(operations), Ok(lexical)) = (frame.complete::<tables::Operations>(), frame.complete::<tables::LexicalTokens>()) else { return HashMap::new() };
    let literals: HashMap<u32, &[u8]> = lexical.iter().map(|row| (row.token, row.literal.as_slice())).collect();
    operations.iter().filter_map(|row| Some((row.id, String::from_utf8(literals.get(&row.definition_token)?.to_vec()).ok()?))).collect()
}

/// The agency feature at the frame's hour; none where it was not recorded.
fn agency_feature(frame: &DecodedFrame) -> Option<bool> {
    let [features] = frame.complete::<tables::SimulationFeatures>().ok()? else { return None };
    Some(features.dlc_mask & AGENCY_FEATURE != 0)
}

pub struct OperativesRoster;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperativesRosterParams {
    pub branch_id: String,
    /// The countries whose agencies are shown, each once.
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativesRosterReply {
    /// The asked agencies' active lists, by asked country, each in roster order where recorded,
    /// else by handle.
    pub operatives: Vec<RosterOperative>,
    /// Artwork URLs the rows name by index.
    pub artwork: Vec<String>,
    /// Asked countries no country holds at the hour.
    pub unheld: Vec<String>,
    /// Whether the game's intelligence agencies exist: false where the DLC is absent, none where
    /// it was not recorded.
    pub agency_feature: Option<bool>,
    /// The census's read hour the rows hold.
    pub read_hour: Option<i32>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OperativesRosterReply, OperativesDepartedReply);

fn validate_tags(tags: &[String]) -> Result<(), &'static str> {
    if tags.is_empty() {
        return Err("Choose a country to see its operatives");
    }
    tags.iter().try_for_each(|tag| validate_tag(tag))?;
    if tags.iter().collect::<BTreeSet<_>>().len() != tags.len() {
        return Err("A country is asked twice");
    }
    Ok(())
}

/// The context of every roster reply: the operative content, or why there is none.
pub type RosterContext = Result<Arc<OperativeContent>, String>;

/// The asked agencies' active lists at `frame`'s hour, each asked tag's holder resolved at `hour`.
pub fn roster(frame: &DecodedFrame, tags_asked: &[String], holders: &Holders, hour: i32, context: &RosterContext) -> io::Result<OperativesRosterReply> {
    let tags = Tags::read(frame)?;
    let rows = frame.complete::<tables::OperativeMemberships>()?;
    let mut unheld = Vec::new();
    let mut slots: HashMap<u32, &str> = HashMap::new();
    for tag in tags_asked {
        match holders.at(tag, hour).map(|run| run.holder) {
            Some(Holder::Found { slot, .. }) => {
                slots.insert(slot, tag);
            }
            _ => unheld.push(tag.clone()),
        }
    }
    let characters = frame.complete::<tables::CharacterOperatives>().map(characters).unwrap_or_default();
    let portraits = frame.complete::<tables::CharacterPortraits>().ok();
    let operations = operation_tokens(frame);
    let content = context.as_deref().map_err(String::as_str);
    let mut captors: BTreeSet<u32> = BTreeSet::new();
    let mut mine: Vec<(&OperativeMembershipRow, &str)> = Vec::new();
    for row in rows.iter().filter(|row| row.roster_kind == ACTIVE && !terminal(row.assignment)) {
        if let Some(tag) = tags.slot(row.roster_country).and_then(|slot| slots.get(&slot)) {
            if row.assignment == CAPTURED {
                captors.extend(tags.slot(row.capturer_country).or((row.capturer_country == 0).then_some(0)));
            }
            mine.push((row, tag));
        }
    }
    let order: HashMap<&str, usize> = tags_asked.iter().enumerate().map(|(at, tag)| (tag.as_str(), at)).collect();
    mine.sort_by_key(|(row, tag)| (order.get(tag).copied(), match row.ordinal {
        Recorded::Value(ordinal) => (false, ordinal),
        Recorded::NotRecorded => (true, 0),
    }, row.id));
    let names = content.ok().map(|content| CountryNames::read_selected(frame, content.localisation(), captors));
    let read_hour = frame.tables[TableId::OperativeMemberships.index()].sampled_at();
    let now = read_hour.unwrap_or(frame.hour);
    let tag = |raw: u32| tags.tag(raw);
    let capturer_name = |raw: u32| -> Result<String, String> {
        let slot = tags.slot(raw).or((raw == 0).then_some(0)).ok_or("The captured operative's country identity is unknown at this hour")?;
        names.as_ref().ok_or("Country names need the recorded content")?.name(slot)
    };
    let operation = |id: u64| operations.get(&id).cloned();
    let mut artwork = Artwork::default();
    let operatives = mine
        .into_iter()
        .map(|(row, roster)| {
            let around = Around {
                content,
                now,
                tag: &tag,
                capturer_name: &capturer_name,
                operation: &operation,
                character: characters.get(&row.id).copied().flatten(),
                portraits,
            };
            RosterOperative::of(row, roster, &around, &mut artwork)
        })
        .collect();
    let mut warnings = context.as_ref().map(|content| content.warnings.clone()).unwrap_or_default();
    for (table, what) in [(TableId::CharacterOperatives, "Operative portraits"), (TableId::CharacterPortraits, "Operative portraits"), (TableId::Operations, "Operation names")] {
        if let Some(why) = missing(frame, table) {
            warnings.push(format!("{what} are not shown: {why}"));
        }
    }
    Ok(OperativesRosterReply { operatives, artwork: artwork.urls, unheld, agency_feature: agency_feature(frame), read_hour, missing: missing(frame, TableId::OperativeMemberships), warnings })
}

impl AtHour for OperativesRoster {
    const KEY: &'static str = "operatives-roster";
    const TITLE: &'static str = "Operatives roster";
    const CONTENT_DEPENDENT: bool = true;
    type Params = OperativesRosterParams;
    type Reply = OperativesRosterReply;
    type Context = RosterContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![OperativesRosterParams::decl(), Country::decl(), RosterOperative::decl(), OperativesRosterReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        validate_tags(&params.tags)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    /// The census, identity and characters first; then the operations, portraits and names its
    /// rows need; then the operations' tokens.
    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let rows = || held.complete::<tables::OperativeMemberships>().unwrap_or_default().iter().filter(|row| row.roster_kind == ACTIVE);
        match step {
            0 => [TableId::OperativeMemberships, IDENTITY[0], IDENTITY[1], TableId::SimulationFeatures, TableId::CharacterOperatives].into_iter().map(Read::all).collect(),
            1 => {
                let operations = rows().filter_map(|row| match row.operation {
                    Recorded::Value(Some(id)) if row.assignment == OPERATION && id != 0 => Some(u128::from(id)),
                    _ => None,
                });
                let ids: BTreeSet<u64> = rows().map(|row| row.id).collect();
                let characters = held
                    .complete::<tables::CharacterOperatives>()
                    .unwrap_or_default()
                    .iter()
                    .filter(|row| row.resolved && row.category == OPERATIVE_CATEGORY && ids.contains(&row.reference))
                    .map(|row| u128::from(row.character));
                let mut reads = vec![Read::these(TableId::Operations, operations), Read::these(TableId::CharacterPortraits, characters)];
                if rows().any(|row| row.assignment == CAPTURED) {
                    reads.extend(NAMING.into_iter().map(Read::all));
                }
                reads
            }
            2 => {
                let tokens: BTreeSet<u32> = held.complete::<tables::Operations>().map(|rows| rows.iter().map(|row| row.definition_token).collect()).unwrap_or_default();
                vec![Read::these(TableId::LexicalTokens, tokens.into_iter().map(u128::from))]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(operative_content::content(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        Ok(match roster(frame, &params.tags, at.holders, at.hour, at.context) {
            Ok(reply) => reply,
            Err(error) => OperativesRosterReply {
                operatives: Vec::new(),
                artwork: Vec::new(),
                unheld: Vec::new(),
                agency_feature: agency_feature(frame),
                read_hour: frame.tables[TableId::OperativeMemberships.index()].sampled_at(),
                missing: Some(error.to_string()),
                warnings: Vec::new(),
            },
        })
    }
}

pub struct OperativesDeparted;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperativesDepartedParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub hour: i32,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DepartedOperative {
    /// As the census last held it on the list.
    #[serde(flatten)]
    pub operative: RosterOperative,
    /// The first and last hour the census held it on the list.
    pub first_hour: i32,
    pub last_hour: i32,
    /// The first read showing it gone or disbanded or killed: an upper bound of when it left.
    pub departed_hour: i32,
    pub departed_reason: String,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativesDepartedReply {
    /// Operatives the asked lists held in the hour's stretch that left them by the hour, newest
    /// first.
    pub departed: Vec<DepartedOperative>,
    /// Artwork URLs the rows name by index.
    pub artwork: Vec<String>,
    pub warnings: Vec<String>,
}

/// A departed membership as its lifetime and its assignments place it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Leaving {
    pub key: u128,
    pub first: i32,
    /// The last hour it was held on the list, alive and serving.
    pub last: i32,
    pub departed: i32,
    pub assignment: Option<u32>,
}

/// When each membership of `lives` (key, first and last hour held, within the stretch from
/// `stretch_first`) left by `hour`, from its assignment steps: the first step at which it is
/// disbanded or killed, else the hour after its last. One still serving at `hour` has not left.
pub fn leavings(lives: &[(u128, i32, i32)], steps: &HashMap<u128, Vec<(i32, Option<u32>)>>, hour: i32) -> Vec<Leaving> {
    let mut out = Vec::new();
    for &(key, first, last) in lives {
        let held = steps.get(&key).map(Vec::as_slice).unwrap_or_default();
        let ended = held.iter().find(|(at, assignment)| *at <= hour && assignment.is_some_and(terminal));
        let leaving = match ended {
            Some(&(at, assignment)) => Leaving { key, first, last: (at - 1).max(first), departed: at, assignment },
            None if last < hour => {
                let assignment = held.iter().rev().find(|(at, assignment)| *at <= last && assignment.is_some()).and_then(|(_, assignment)| *assignment);
                Leaving { key, first, last, departed: last + 1, assignment }
            }
            None => continue,
        };
        out.push(leaving);
    }
    out.sort_by_key(|leaving| std::cmp::Reverse((leaving.departed, leaving.key)));
    out
}

impl Product for OperativesDeparted {
    const KEY: &'static str = "operatives-departed";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = OperativesDepartedParams;
    type Reply = OperativesDepartedReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![OperativesDepartedParams::decl(), DepartedOperative::decl(), OperativesDepartedReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        validate_tags(&params.tags)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        crate::metadata::check_cancel(cancelled)?;
        let query = crate::answer::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let stretch = *query.timeline.at(params.hour).ok_or_else(|| dashboard_core::refused("Operatives roster hour is not recorded"))?;
        let links = crate::answer::links(source, stretch.run)?;
        let mut slots: HashMap<u32, &str> = HashMap::new();
        for tag in &params.tags {
            if let Some(Holder::Found { slot, .. }) = query.holders.at(tag, params.hour).map(|run| run.holder) {
                slots.insert(slot, tag);
            }
        }
        let lives = crate::answer::lifetimes(source, stretch.run, TableId::OperativeMemberships)?;
        let roster = |key: u128, hour: i32| -> Option<&str> {
            let (raw, kind) = roster_of(key);
            (kind == ACTIVE).then(|| links.slot_at(raw, hour)).flatten().and_then(|slot| slots.get(&slot).copied())
        };
        let mine: Vec<(u128, i32, i32)> = lives
            .iter()
            .filter_map(|&(key, first, last)| {
                let (first, last) = (i32::try_from(first).ok()?.max(stretch.first), i32::try_from(last).ok()?.min(params.hour));
                (first <= last && roster(key, last).is_some()).then_some((key, first, last))
            })
            .collect();
        let recording = query.replay.recording();
        let keys: Vec<u128> = mine.iter().map(|&(key, ..)| key).collect();
        let assignment = dashboard_declare::field(TableId::OperativeMemberships, "assignment")?;
        let history = dashboard_query::cost::storage(|| replay::lanes::history(recording, stretch.run, TableId::OperativeMemberships, &[assignment], replay::lanes::Keys::These(&keys), stretch.first, params.hour))?;
        let steps: HashMap<u128, Vec<(i32, Option<u32>)>> = history
            .keys
            .into_iter()
            .map(|(key, steps)| {
                let steps = steps.into_iter().map(|step| (step.hour, match step.cells.as_deref() {
                    Some([replay::points::Cell::Number(value), ..]) => u32::try_from(*value).ok(),
                    _ => None,
                })).collect();
                (key, steps)
            })
            .collect();
        let leaving = leavings(&mine, &steps, params.hour);
        crate::metadata::check_cancel(cancelled)?;
        let format = recording.format();
        let asks: Vec<(i32, u128)> = leaving.iter().map(|leaving| (leaving.last, leaving.key)).collect();
        let mut rows: HashMap<u128, OperativeMembershipRow> = HashMap::new();
        dashboard_query::cost::storage(|| {
            replay::lanes::at_each(recording, stretch.run, TableId::OperativeMemberships, None, &asks, &mut |_, key, row| {
                if let Some((view, slot)) = row
                    && let Some(row) = replay_frame::frame::cells_rows(TableId::OperativeMemberships, &view, &[(key, slot)], format)
                        .ok()
                        .and_then(|rows| rows.rows::<tables::OperativeMemberships>().and_then(|rows| rows.first().cloned()))
                {
                    rows.insert(key, row);
                }
                Ok(())
            })
        })?;
        let content = operative_content::content(source, context);
        let names = Names::new(&query.holders);
        let mut artwork = Artwork::default();
        let mut departed = Vec::new();
        let mut unread = 0usize;
        for leaving in leaving {
            let Some(row) = rows.get(&leaving.key) else {
                unread += 1;
                continue;
            };
            let Some(tag) = roster(leaving.key, leaving.last) else { continue };
            departed.push(DepartedOperative {
                operative: departed_operative(row, tag, leaving.last, &content, &names, &links, &mut artwork),
                first_hour: leaving.first,
                last_hour: leaving.last,
                departed_hour: leaving.departed,
                departed_reason: departure(leaving.assignment).to_owned(),
            });
        }
        let mut warnings = content.as_ref().map(|content| content.warnings.clone()).unwrap_or_default();
        if unread != 0 {
            warnings.push(format!("{unread} departed operatives are not shown: their last row was not recorded"));
        }
        if departed.iter().any(|operative| operative.operative.portrait.is_none()) {
            warnings.push("Departed operatives' portraits and operations are not read at their departure.".to_owned());
        }
        Ok(OperativesDepartedReply { departed, artwork: artwork.urls, warnings })
    }
}

/// A departed row named at its last hour through the run's links; its portrait and operation are
/// not read at that hour, and say so.
fn departed_operative(row: &OperativeMembershipRow, roster: &str, last: i32, content: &RosterContext, names: &Names, links: &RunLinks, artwork: &mut Artwork) -> RosterOperative {
    let tag = |raw: u32| names.tag_at(links, raw, last).map(str::to_owned);
    let capturer_name = |_: u32| -> Result<String, String> { Err("The capturer's name is not read at a departure.".to_owned()) };
    let operation = |_: u64| None;
    let around = Around { content: content.as_deref().map_err(String::as_str), now: last, tag: &tag, capturer_name: &capturer_name, operation: &operation, character: None, portraits: None };
    let mut operative = RosterOperative::of(row, roster, &around, artwork);
    if operative.portrait.is_none() && content.is_ok() {
        operative.portrait_missing = Some("The portrait is not read at a departure.".to_owned());
    }
    operative
}

pub struct OperativeTenures;

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Tenure {
    /// The full handle, as a decimal string.
    pub id: String,
    pub roster: String,
    /// The first and last hour the census held it on the list within the stretch.
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativeTenuresReply {
    /// The stretch of continuous history holding the hour asked: what the tenures cover.
    pub first_hour: i32,
    pub last_hour: i32,
    /// Every membership of the asked active lists the stretch held, by roster then handle.
    pub tenures: Vec<Tenure>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OperativeTenuresReply);

/// When each operative of the asked agencies was on their lists over the stretch holding an hour:
/// the census's lifetimes, kept in the index at open, named by the run's links.
impl Product for OperativeTenures {
    const KEY: &'static str = "operative-tenures";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = OperativesDepartedParams;
    type Reply = OperativeTenuresReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![Tenure::decl(), OperativeTenuresReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        validate_tags(&params.tags)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        crate::metadata::check_cancel(cancelled)?;
        let query = crate::answer::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let stretch = *query.timeline.at(params.hour).ok_or_else(|| dashboard_core::refused("Operatives roster hour is not recorded"))?;
        let links = crate::answer::links(source, stretch.run)?;
        let lives = crate::answer::lifetimes(source, stretch.run, TableId::OperativeMemberships)?;
        let mut tenures = Vec::new();
        for &(key, first, last) in lives.iter() {
            let (raw, kind) = roster_of(key);
            let (Ok(first), Ok(last)) = (i32::try_from(first), i32::try_from(last)) else { continue };
            let (first, last) = (first.max(stretch.first), last.min(stretch.last));
            if kind != ACTIVE || first > last {
                continue;
            }
            let Some(slot) = links.slot_at(raw, last) else { continue };
            let held = params.tags.iter().find(|tag| query.holders.at(tag, last).is_some_and(|run| matches!(run.holder, Holder::Found { slot: held, .. } if held == slot)));
            if let Some(tag) = held {
                tenures.push(Tenure { id: (key >> 40).to_string(), roster: tag.clone(), first_hour: first, last_hour: last });
            }
        }
        let order: HashMap<&str, usize> = params.tags.iter().enumerate().map(|(at, tag)| (tag.as_str(), at)).collect();
        tenures.sort_by(|a, b| (order.get(a.roster.as_str()), a.first_hour, &a.id).cmp(&(order.get(b.roster.as_str()), b.first_hour, &b.id)));
        Ok(OperativeTenuresReply { first_hour: stretch.first, last_hour: stretch.last, tenures, warnings: Vec::new() })
    }
}
