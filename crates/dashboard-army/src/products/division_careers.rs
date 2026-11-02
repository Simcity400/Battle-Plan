//! Division careers at a reached hour: every division's medal total, awards,
//! experience and kill counter (the ranking), and one division's recorded
//! career and awards (the service record).
//!
//! The medal total is the history summary's recorded amount, never a sum of
//! award entries. The kill counter is the game's career counter; battles won
//! or fought are not recorded per division and are not derived. Experience is
//! the stored experience normalised by the division's current manpower, with
//! its level from the content's experience defines. Entries keep their native
//! list order and repeats. Intelligence copies of divisions are left out.
use super::army_content::{self, Refusals};
use dashboard_base::{
    prepared::query::native,
    products::{Product, place_names},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_forces::products::army_size::owner;
use dashboard_text::{catalogue::Catalogue, division_history};
use installed_content_localisation::localisation::Argument;
use replay_data::rows::{DivisionHistoryRow, SharedDivisionHistoryRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc, sync::atomic::AtomicBool};
use ts_rs::TS;

pub struct DivisionCareers;
pub struct DivisionRecord;

/// The tables every career reply reads; a division's record also reads its places' names.
pub const TABLES: [TableId; 9] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionStatus,
    TableId::DivisionLandInputs,
    TableId::DivisionHistory,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::LexicalTokens,
];

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionCareersParams {
    pub branch_id: String,
    pub hour: i32,
    /// The countries whose divisions rank; none ranks every country's.
    pub tags: Option<Vec<String>>,
}

/// A medal the ranking names, by its content key.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CareerMedal {
    pub key: String,
    pub name: Option<String>,
}

/// A template the ranking names.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CareerTemplate {
    pub template: String,
    pub name: Option<String>,
    pub icon: Option<String>,
}

/// How many award entries a division holds of one medal.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CareerAward {
    /// Index into the reply's medals.
    pub medal: usize,
    pub count: u32,
}

/// What every division's career shows in the ranking.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CareerFacts {
    /// The recorded medal total; none when the history was not read.
    pub medals: Option<i32>,
    pub awards: Vec<CareerAward>,
    pub history_missing: Option<String>,
    /// Manpower-normalised experience, fixed-point with five decimals.
    pub experience_raw: Option<String>,
    /// Zero-based experience level, and the level's in-game name.
    pub level: Option<i32>,
    pub level_name: Option<String>,
    /// The game's career kill counter.
    pub kills: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CareerRow {
    pub division: String,
    pub tag: Option<String>,
    pub template: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub facts: CareerFacts,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionCareersReply {
    pub branch_id: String,
    pub hour: i32,
    pub divisions: Vec<CareerRow>,
    pub medals: Vec<CareerMedal>,
    pub templates: Vec<CareerTemplate>,
    /// Why divisions are unknown at this hour, when they are.
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}
dashboard_base::reply_warnings!(DivisionCareersReply);

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionRecordParams {
    pub branch_id: String,
    pub hour: i32,
    /// The division's full native handle.
    pub division: String,
}

/// One recorded career entry or award.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecordEntry {
    pub award: bool,
    pub ordinal: u32,
    /// The entry's game date.
    pub hour: i32,
    pub description: Option<String>,
    pub description_missing: Option<String>,
    pub provinces: Vec<u32>,
    /// The awarded medal, by index into the reply's medals.
    pub medal: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionRecordReply {
    pub branch_id: String,
    pub hour: i32,
    pub division: String,
    /// The division is not in service at this hour.
    pub absent: bool,
    pub tag: Option<String>,
    pub template: Option<CareerTemplate>,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub facts: Option<CareerFacts>,
    pub medals: Vec<CareerMedal>,
    pub entries: Vec<RecordEntry>,
    pub warnings: Vec<String>,
}
dashboard_base::reply_warnings!(DivisionRecordReply);

impl Product for DivisionCareers {
    const KEY: &'static str = "division-careers";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = DivisionCareersParams;
    type Reply = DivisionCareersReply;
    fn types() -> Vec<String> {
        vec![
            DivisionCareersParams::decl(),
            DivisionCareersReply::decl(),
            CareerRow::decl(),
            CareerFacts::decl(),
            CareerAward::decl(),
            CareerMedal::decl(),
            CareerTemplate::decl(),
        ]
    }
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        for tag in p.tags.iter().flatten() {
            dashboard_base::products::validate_tag(tag)?;
        }
        p.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }
    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = p.branch_id.parse::<u64>().map_err(|_| io::Error::other("Invalid branch"))?;
        let reply = dashboard_land_battles::products::reached::cached(
            source,
            context,
            Self::KEY,
            branch,
            p.hour,
            &p.tags,
            cancelled,
            || {
                let frame = native::frame_at(source, branch, p.hour, &TABLES, cancelled)?
                    .ok_or_else(|| io::Error::other("This hour is not recorded"))?;
                let mut refusals = Refusals::default();
                let cat = refusals.keep(army_content::content(source, context)).0;
                let names = names(source, cat.as_ref(), &mut refusals);
                let mut reply = careers(
                    &frame,
                    p.tags.as_deref(),
                    &Naming {
                        cat: cat.as_ref(),
                        names: names.as_ref().map(|names| names.as_ref().map(Arc::as_ref).map_err(Clone::clone)),
                        icon: &|handle| army_content::template_icon(source, &frame, handle, cat.as_ref()),
                    },
                    &mut refusals,
                    cancelled,
                )?;
                reply.branch_id.clone_from(&p.branch_id);
                reply.warnings = refusals.lines();
                Ok(reply)
            },
        )?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

impl Product for DivisionRecord {
    const KEY: &'static str = "division-record";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = DivisionRecordParams;
    type Reply = DivisionRecordReply;
    fn types() -> Vec<String> {
        vec![
            DivisionRecordParams::decl(),
            DivisionRecordReply::decl(),
            RecordEntry::decl(),
        ]
    }
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        p.division.parse::<u64>().map_err(|_| "Invalid division")?;
        p.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }
    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = p.branch_id.parse::<u64>().map_err(|_| io::Error::other("Invalid branch"))?;
        let division = p.division.parse::<u64>().map_err(|_| io::Error::other("Invalid division"))?;
        let reply = dashboard_land_battles::products::reached::cached(
            source,
            context,
            Self::KEY,
            branch,
            p.hour,
            &p.division,
            cancelled,
            || {
                let mut tables = TABLES.to_vec();
                tables.extend(place_names::NAME_TABLES);
                tables.sort_unstable();
                tables.dedup();
                let frame = native::frame_at(source, branch, p.hour, &tables, cancelled)?
                    .ok_or_else(|| io::Error::other("This hour is not recorded"))?;
                let mut refusals = Refusals::default();
                let cat = refusals.keep(army_content::content(source, context)).0;
                let names = names(source, cat.as_ref(), &mut refusals);
                let naming = Naming {
                    cat: cat.as_ref(),
                    names: names.as_ref().map(|names| names.as_ref().map(Arc::as_ref).map_err(Clone::clone)),
                    icon: &|handle| army_content::template_icon(source, &frame, handle, cat.as_ref()),
                };
                let places = |provinces: &[u32]| -> Vec<Result<String, String>> {
                    let params = place_names::PlaceNamesParams {
                        branch_id: p.branch_id.clone(),
                        land: false,
                        places: provinces
                            .iter()
                            .map(|&province_id| place_names::PlaceNameRequest { province_id, hour: frame.hour })
                            .collect(),
                        states: Vec::new(),
                    };
                    match place_names::at_frame(source, &params, context, &frame, cancelled) {
                        Ok(reply) => reply
                            .places
                            .into_iter()
                            .map(|place| place.name.ok_or_else(|| place.missing.unwrap_or_else(|| "Province name unavailable".into())))
                            .collect(),
                        Err(why) => provinces.iter().map(|_| Err(why.to_string())).collect(),
                    }
                };
                let mut reply = record(&frame, division, &naming, &places, &mut refusals)?;
                reply.branch_id.clone_from(&p.branch_id);
                reply.warnings = refusals.lines();
                Ok(reply)
            },
        )?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn names(
    source: &Source,
    cat: Option<&Arc<Catalogue>>,
    refusals: &mut Refusals,
) -> Option<Result<Arc<installed_content_division_names::division_names::Database>, String>> {
    cat.map(|cat| {
        let (names, notes) = dashboard_land_battles::products::division_log::division_name_groups(source, cat);
        for note in notes {
            refusals.note(note);
        }
        names
    })
}

/// What division careers are named with.
pub struct Naming<'a> {
    pub cat: Option<&'a Arc<Catalogue>>,
    pub names: Option<Result<&'a installed_content_division_names::division_names::Database, String>>,
    pub icon: &'a dyn Fn(u64) -> Result<String, String>,
}

impl Naming<'_> {
    fn division(&self, status: Option<&replay_data::rows::DivisionStatusRow>) -> Result<String, String> {
        let status = status.ok_or("The division's status was not read")?;
        let cat = self.cat.ok_or("Division names need the recorded content stack")?;
        dashboard_text::unit_names::army_name(status, &cat.localisation, &cat.recorded_language, || {
            self.names.clone().ok_or_else(|| "Division names need the recorded content stack".to_owned())?
        })
        .and_then(|name| dashboard_base::catalogue::plain_text(&name))
    }
}

/// Medals named once per reply, in first-seen order.
#[derive(Default)]
struct MedalIndex {
    medals: Vec<CareerMedal>,
    by_token: BTreeMap<i32, usize>,
}
impl MedalIndex {
    fn index(&mut self, frame: &DecodedFrame, token: i32, cat: Option<&Arc<Catalogue>>, refusals: &mut Refusals) -> Option<usize> {
        if let Some(&at) = self.by_token.get(&token) {
            return Some(at);
        }
        let key = match army_content::token_key(frame, token as u32) {
            Ok(key) => key,
            Err(why) => {
                refusals.note(format!("Medal identity unavailable: {why}"));
                return None;
            }
        };
        let (name, _) = refusals.keep(army_content::localised(cat, &key).map_err(|_| "Medal name unavailable".to_owned()));
        self.medals.push(CareerMedal { key, name });
        self.by_token.insert(token, self.medals.len() - 1);
        Some(self.medals.len() - 1)
    }
}

/// A division's history rows, summary first, in native order.
fn history_of(rows: &[SharedDivisionHistoryRow], division: u64) -> &[SharedDivisionHistoryRow] {
    let start = rows.partition_point(|row| row.division < division);
    &rows[start..start + rows[start..].partition_point(|row| row.division == division)]
}

/// The experience defines' levels, read once per reply.
struct Levels {
    experience: Result<installed_content_core::unit_leader::DivisionExperience, String>,
    names: BTreeMap<i32, Option<String>>,
}
impl Levels {
    fn read(cat: Option<&Arc<Catalogue>>) -> Self {
        let experience = cat
            .ok_or_else(|| "Experience levels need the recorded content stack".to_owned())
            .and_then(|cat| {
                installed_content_core::unit_leader::DivisionExperience::read(cat.defines()?).map_err(|why| why.to_string())
            });
        Self { experience, names: BTreeMap::new() }
    }
}

#[allow(clippy::too_many_arguments)]
fn facts(
    frame: &DecodedFrame,
    history: &[SharedDivisionHistoryRow],
    status: Option<&replay_data::rows::DivisionStatusRow>,
    manpower: Option<i32>,
    cat: Option<&Arc<Catalogue>>,
    medals: &mut MedalIndex,
    levels: &mut Levels,
    refusals: &mut Refusals,
) -> CareerFacts {
    let mut out = CareerFacts {
        medals: None,
        awards: Vec::new(),
        history_missing: None,
        experience_raw: None,
        level: None,
        level_name: None,
        kills: status.and_then(|row| row.career_counter()),
    };
    match history.first().filter(|row| row.list == DivisionHistoryRow::SUMMARY) {
        None => {
            refusals.note("A division's history was not read");
            out.history_missing = Some("The division's history was not read at this hour".into());
        }
        Some(summary) if summary.available == 0 => {
            let reason = String::from_utf8_lossy(&summary.reason).into_owned();
            refusals.note(format!("A division's history was refused: {reason}"));
            out.history_missing = Some(reason);
        }
        Some(summary) => {
            out.medals = Some(summary.amount);
            let mut counts = BTreeMap::<usize, u32>::new();
            let mut order = Vec::new();
            for entry in history.iter().filter(|row| row.list == DivisionHistoryRow::AWARDS && row.medal_enabled != 0) {
                let Some(token) = entry.medal_token else {
                    refusals.note("An award's medal identity was not recorded");
                    continue;
                };
                if let Some(at) = medals.index(frame, token, cat, refusals) {
                    let count = counts.entry(at).or_default();
                    if *count == 0 {
                        order.push(at);
                    }
                    *count += 1;
                }
            }
            out.awards = order.into_iter().map(|medal| CareerAward { medal, count: counts[&medal] }).collect();
        }
    }
    if let Some(experience) = status.zip(manpower).and_then(|(status, manpower)| status.normalized_experience(manpower)) {
        out.experience_raw = Some(experience.to_string());
        match &levels.experience {
            Ok(content) => {
                let level = content.progress(experience).level;
                out.level = Some(level);
                out.level_name = levels
                    .names
                    .entry(level)
                    .or_insert_with(|| army_content::localised(cat, &format!("UNIT_LEVEL_{level}")).ok())
                    .clone();
            }
            Err(why) => refusals.note(why.clone()),
        }
    }
    out
}

fn status_of(frame: &DecodedFrame, division: u64) -> Option<&replay_data::rows::DivisionStatusRow> {
    let rows = frame.complete::<tables::DivisionStatus>().ok()?;
    rows.binary_search_by_key(&division, |row| row.division).ok().map(|at| &rows[at])
}

fn manpower_of(frame: &DecodedFrame, division: u64) -> Option<i32> {
    let rows = frame.complete::<tables::DivisionLandInputs>().ok()?;
    let row = &rows[rows.binary_search_by_key(&division, |row| row.division).ok()?];
    Some(row.manpower.iter().fold(0i32, |sum, part| sum.wrapping_add(part.amount)))
}

/// Every division's career facts, for the selected countries.
pub fn careers(
    frame: &DecodedFrame,
    tags: Option<&[String]>,
    naming: &Naming<'_>,
    refusals: &mut Refusals,
    cancelled: &AtomicBool,
) -> io::Result<DivisionCareersReply> {
    let mut reply = DivisionCareersReply {
        branch_id: frame.branch.to_string(),
        hour: frame.hour,
        divisions: Vec::new(),
        medals: Vec::new(),
        templates: Vec::new(),
        missing: None,
        warnings: Vec::new(),
    };
    let divisions = match frame.complete::<tables::Divisions>() {
        Ok(rows) => rows,
        Err(why) => {
            reply.missing = Some(format!("Divisions were not read at this hour: {why}"));
            return Ok(reply);
        }
    };
    let countries = army_content::Countries::read(frame)?;
    let history = frame.complete::<tables::DivisionHistory>().unwrap_or(&[]);
    let mut medals = MedalIndex::default();
    let mut levels = Levels::read(naming.cat);
    let mut templates = BTreeMap::<u64, ()>::new();
    for division in divisions.iter().filter(|row| row.reveal_skip == 0) {
        if reply.divisions.len().is_multiple_of(1024) {
            dashboard_text::metadata::check_cancel(cancelled)?;
        }
        let tag = countries.tag(owner(division));
        if let Some(tags) = tags
            && !tag.as_ref().is_some_and(|tag| tags.contains(tag))
        {
            continue;
        }
        let status = status_of(frame, division.id);
        let (name, name_missing) = refusals.keep(naming.division(status));
        let facts = facts(
            frame,
            history_of(history, division.id),
            status,
            manpower_of(frame, division.id),
            naming.cat,
            &mut medals,
            &mut levels,
            refusals,
        );
        templates.insert(division.template, ());
        reply.divisions.push(CareerRow {
            division: division.id.to_string(),
            tag,
            template: division.template.to_string(),
            name,
            name_missing,
            facts,
        });
    }
    for template in templates.into_keys() {
        reply.templates.push(CareerTemplate {
            template: template.to_string(),
            name: refusals.keep(army_content::template_name(frame, template, naming.cat)).0,
            icon: refusals.keep((naming.icon)(template)).0,
        });
    }
    reply.medals = medals.medals;
    Ok(reply)
}

/// One division's header facts and recorded entries. `places` names provinces.
#[allow(clippy::type_complexity)]
pub fn record(
    frame: &DecodedFrame,
    division: u64,
    naming: &Naming<'_>,
    places: &dyn Fn(&[u32]) -> Vec<Result<String, String>>,
    refusals: &mut Refusals,
) -> io::Result<DivisionRecordReply> {
    let mut reply = DivisionRecordReply {
        branch_id: frame.branch.to_string(),
        hour: frame.hour,
        division: division.to_string(),
        absent: false,
        tag: None,
        template: None,
        name: None,
        name_missing: None,
        facts: None,
        medals: Vec::new(),
        entries: Vec::new(),
        warnings: Vec::new(),
    };
    let divisions = frame.complete::<tables::Divisions>()?;
    let Some(row) = divisions.binary_search_by_key(&division, |row| row.id).ok().map(|at| &divisions[at]) else {
        reply.absent = true;
        return Ok(reply);
    };
    let countries = army_content::Countries::read(frame)?;
    reply.tag = countries.tag(owner(row));
    let status = status_of(frame, division);
    (reply.name, reply.name_missing) = refusals.keep(naming.division(status));
    reply.template = Some(CareerTemplate {
        template: row.template.to_string(),
        name: refusals.keep(army_content::template_name(frame, row.template, naming.cat)).0,
        icon: refusals.keep((naming.icon)(row.template)).0,
    });
    let history = history_of(frame.complete::<tables::DivisionHistory>().unwrap_or(&[]), division);
    let mut medals = MedalIndex::default();
    let mut levels = Levels::read(naming.cat);
    reply.facts = Some(facts(
        frame,
        history,
        status,
        manpower_of(frame, division),
        naming.cat,
        &mut medals,
        &mut levels,
        refusals,
    ));
    if reply.facts.as_ref().is_some_and(|facts| facts.history_missing.is_none()) {
        for entry in history.iter().filter(|row| row.list != DivisionHistoryRow::SUMMARY) {
            let award = entry.list == DivisionHistoryRow::AWARDS;
            if award && entry.medal_enabled == 0 {
                // A retained empty award slot is known absence, not an entry.
                continue;
            }
            let medal = if award {
                entry.medal_token.and_then(|token| medals.index(frame, token, naming.cat, refusals))
            } else {
                None
            };
            let description = if award {
                medal
                    .and_then(|at| medals.medals[at].name.clone())
                    .ok_or_else(|| "Medal name unavailable".to_owned())
            } else {
                describe(frame, entry, naming.cat, &countries, places)
            };
            let (description, description_missing) = refusals.keep(description);
            reply.entries.push(RecordEntry {
                award,
                ordinal: entry.ordinal,
                hour: entry.hour,
                description,
                description_missing,
                provinces: entry.provinces.clone(),
                medal,
            });
        }
    }
    reply.medals = medals.medals;
    Ok(reply)
}

/// A career entry in the game's own wording.
#[allow(clippy::type_complexity)]
fn describe(
    _frame: &DecodedFrame,
    entry: &DivisionHistoryRow,
    cat: Option<&Arc<Catalogue>>,
    countries: &army_content::Countries<'_>,
    places: &dyn Fn(&[u32]) -> Vec<Result<String, String>>,
) -> Result<String, String> {
    let cat = cat.ok_or("Entry descriptions need the recorded content stack")?;
    let loc = &cat.localisation;
    let text = |key: &[u8], arguments: &[(&str, Argument<'_>)]| -> Result<String, String> {
        loc.localise_with(key, arguments)
            .map_err(|why| why.to_string())
            .and_then(|text| dashboard_base::catalogue::plain_text(&text))
    };
    if let Some(ship) = &entry.sunk_ship {
        return dashboard_text::scripted_text::stored_text(loc, &cat.recorded_language, &ship.ship_name, "Sunk ship name");
    }
    let subject = dashboard_text::scripted_text::stored_text(loc, &cat.recorded_language, &entry.name, "Entry subject")?;
    let country = if entry.target_country == 0 {
        String::new()
    } else {
        let tag = countries.tag(entry.target_country).ok_or("The entry's country is not recorded")?;
        text(tag.as_bytes(), &[])?
    };
    let provinces = places(&entry.provinces).into_iter().collect::<Result<Vec<_>, _>>()?;
    let province = division_history::province_list(&provinces, text)?;
    let date = division_history::date(entry.hour);
    text(
        division_history::event_key(entry)?,
        &division_history::arguments(entry, &subject, &country, &province, &date),
    )
}

#[cfg(test)]
mod tests;
