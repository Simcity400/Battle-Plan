
use dashboard_base::products::Country;
use super::{Content, Countries, Refusals, attributed, find, token_key};
use dashboard_base::{Source, catalogue, products::{LoadContext, at_hour::{At, AtHour, Keys, Read}}};
use crate::products::army::ArmyAttribution;
use dashboard_query::identity::Holder;
use dashboard_text::division_history;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{DivisionHistoryRow, DivisionStatusRow, SharedDivisionHistoryRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use content_reader_military::unit_medals::{Medal, Medals};
use dashboard_base::products::artwork::sprites::SpriteFrame;
use std::{cmp::Ordering, collections::{BTreeMap, HashSet}, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 9] = [
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

pub struct DivisionCareers;
pub struct DivisionRecord;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionCareersParams {
    pub branch_id: String,
    #[doc = " The countries whose divisions rank; none ranks every country's."]
    pub countries: Option<Vec<Country>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CareerMedal {
    pub key: String,
    pub name: Option<String>,
    #[doc = " The medal's sprite and frame as its definition names them; none with `icon_missing` saying why."]
    pub icon: Option<SpriteFrame>,
    pub icon_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CareerTemplate {
    pub template: String,
    pub name: Option<String>,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CareerAward {
    #[doc = " Index into the reply's medals; none for the awards of the recorded total that could not be identified."]
    pub medal: Option<usize>,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CareerFacts {
    #[doc = " The recorded medal total; none when the history was not read."]
    pub medals: Option<i32>,
    pub awards: Vec<CareerAward>,
    pub history_missing: Option<String>,
    #[doc = " Manpower-normalised experience, fixed-point with five decimals."]
    pub experience_raw: Option<String>,
    #[doc = " Zero-based experience level, and the level's in-game name."]
    pub level: Option<i32>,
    pub level_name: Option<String>,
    #[doc = " The game's count of the men this division has lost (its panel's Casualties taken); excludes men lost at destruction and in paradrop disruption."]
    pub casualties_taken: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CareerRow {
    pub division: String,
    pub tag: Option<String>,
    pub template: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub facts: CareerFacts,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionCareersReply {
    pub divisions: Vec<CareerRow>,
    pub medals: Vec<CareerMedal>,
    pub templates: Vec<CareerTemplate>,
    #[doc = " Why divisions are unknown at this hour, when they are."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DivisionCareersReply);

pub struct DecoratedDivisions;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum DecoratedRank {
    Medals,
    Experience,
    CasualtiesTaken,
}

const RANKS: [DecoratedRank; 3] = [DecoratedRank::Medals, DecoratedRank::Experience, DecoratedRank::CasualtiesTaken];

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryHours {
    pub tag: String,
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecoratedDivisionsParams {
    pub branch_id: String,
    #[doc = " The countries whose divisions rank and the hours each counts; none ranks every country's."]
    pub countries: Option<Vec<CountryHours>>,
    pub by: DecoratedRank,
    pub descending: bool,
    #[doc = " How many divisions to return, from the first in order."]
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DecoratedDivisionsReply {
    #[doc = " The first `count` divisions in order: by the measure, then the other measures largest first, then by id; a division without the measure follows those with it."]
    pub divisions: Vec<CareerRow>,
    pub medals: Vec<CareerMedal>,
    pub templates: Vec<CareerTemplate>,
    #[doc = " Why the order holds only among the divisions it could place: a measure or a country not read for some."]
    pub partial: Option<String>,
    #[doc = " Why divisions are unknown at this hour, when they are."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DecoratedDivisionsReply);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionRecordParams {
    pub branch_id: String,
    #[doc = " The division's full native handle."]
    pub division: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RecordEntry {
    pub award: bool,
    pub ordinal: u32,
    #[doc = " The entry's game date."]
    pub hour: i32,
    pub description: Option<String>,
    pub description_missing: Option<String>,
    pub provinces: Vec<u32>,
    #[doc = " The awarded medal, by index into the reply's medals."]
    pub medal: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionRecordReply {
    pub division: String,
    #[doc = " The division is not in service at this hour."]
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

fn branch(id: &str) -> Result<(), &'static str> {
    id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
}

impl AtHour for DivisionCareers {
    const KEY: &'static str = "division-careers";
    const TITLE: &'static str = "Division careers";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = DivisionCareersParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]"];
    type Reply = DivisionCareersReply;
    type Context = Content;

    #[cfg(any(test, feature = "export-types"))]
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

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        for tag in params.countries.iter().flatten() {
            dashboard_base::products::validate_country(tag)?;
        }
        branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Content::load(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let slots = params.countries.as_ref().map(|tags| {
            tags.iter()
                .filter_map(|tag| match at.holders.at(tag, at.hour).map(|run| run.holder) {
                    Some(Holder::Found { slot, .. }) => Some(slot),
                    _ => None,
                })
                .collect::<Vec<u32>>()
        });
        let mut refusals = Refusals::default();
        let mut reply = careers(frame, slots.as_deref(), at, &mut refusals)?;
        reply.warnings = refusals.lines();
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

impl AtHour for DecoratedDivisions {
    const KEY: &'static str = "decorated-divisions";
    const TITLE: &'static str = "Most decorated divisions";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DecoratedDivisionsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]", by, descending, count];
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Reply = DecoratedDivisionsReply;
    type Context = Content;

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        filter_valid_through(params.countries.as_deref(), at.hour)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DecoratedDivisionsParams::decl(), DecoratedDivisionsReply::decl(), DecoratedRank::decl(), CountryHours::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        for span in params.countries.iter().flatten() {
            dashboard_core::protocol::validate_tag(&span.tag)?;
            if span.first_hour > span.last_hour {
                return Err("Invalid country hours");
            }
        }
        branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Content::load(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut refusals = Refusals::default();
        let mut reply = decorated(frame, params, at, &mut refusals)?;
        reply.warnings = refusals.lines();
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

impl AtHour for DivisionRecord {
    const KEY: &'static str = "division-record";
    const TITLE: &'static str = "Division record";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = DivisionRecordParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, division];
    type Reply = DivisionRecordReply;
    type Context = Content;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DivisionRecordParams::decl(), DivisionRecordReply::decl(), RecordEntry::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.division.parse::<u64>().map_err(|_| "Invalid division")?;
        branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = TABLES.to_vec();
        tables.extend([TableId::Provinces, TableId::States]);
        tables.extend_from_slice(dashboard_text::scripted_text::TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    fn reads(params: &Self::Params, _owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        if step != 0 {
            let template = held.complete::<tables::Divisions>().ok().and_then(|rows| find(Some(rows), division(params), |row| row.id)).map(|row| row.template);
            return match (step, template) {
                (1, Some(template)) => vec![Read::these(TableId::DivisionTemplateIcons, [u128::from(template)])],
                _ => Vec::new(),
            };
        }
        let id = division(params);
        let history = u128::from(id) << 34;
        let mut reads: Vec<Read> = Self::tables(params)
            .into_iter()
            .filter(|table| !matches!(table, TableId::Divisions | TableId::DivisionStatus | TableId::DivisionLandInputs | TableId::DivisionHistory | TableId::DivisionTemplateIcons))
            .map(Read::all)
            .collect();
        reads.extend([
            Read::these(TableId::Divisions, [u128::from(id)]),
            Read::these(TableId::DivisionStatus, [u128::from(id)]),
            Read::these(TableId::DivisionLandInputs, [u128::from(id)]),
            Read { table: TableId::DivisionHistory, keys: Keys::Range(history, history | ((1 << 34) - 1)) },
        ]);
        reads
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Content::load(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut refusals = Refusals::default();
        let mut reply = record(frame, division(params), at, &mut refusals)?;
        reply.warnings = refusals.lines();
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn division(params: &DivisionRecordParams) -> u64 {
    params.division.parse().unwrap_or(0)
}

#[derive(Default)]
struct MedalIndex {
    medals: Vec<CareerMedal>,
    by_identity: BTreeMap<(Option<i32>, i32), usize>,
}

impl MedalIndex {
    fn index(&mut self, frame: &DecodedFrame, token: i32, definition: Option<i32>, content: &Content, refusals: &mut Refusals) -> Option<usize> {
        if let Some(&at) = self.by_identity.get(&(definition, token)) {
            return Some(at);
        }
        let key = frame.complete::<tables::LexicalTokens>().map_err(|why| format!("Medal names unavailable: {why}")).and_then(|rows| token_key(rows, token as u32));
        let key = match key {
            Ok(key) => key,
            Err(why) => {
                refusals.note(format!("Medal identity unavailable: {why}"));
                return None;
            }
        };
        let (name, _) = refusals.keep(content.localised(&key).map_err(|_| "Medal name unavailable".to_owned()));
        let (icon, icon_missing) = refusals.keep(medal_icon(content, definition, &key));
        self.medals.push(CareerMedal { key, name, icon, icon_missing });
        self.by_identity.insert((definition, token), self.medals.len() - 1);
        Some(self.medals.len() - 1)
    }
}

fn medal_definitions(content: &Content) -> Result<std::sync::Arc<Medals>, String> {
    let cat = content.cat.as_ref().map_err(|_| "Medal artwork needs the recorded content stack".to_owned())?;
    cat.content
        .layers
        .derived::<catalogue::CachedResult<Medals>>()
        .get_or_init(|| Medals::load(&cat.content.layers, &|_| Some(true)))
        .map_err(|why| format!("Medal definitions unavailable: {why}"))
}

pub(super) fn sprite_of(definition: &Medal) -> Result<SpriteFrame, String> {
    let key = definition.icon.as_deref().filter(|icon| !icon.is_empty()).ok_or("The medal's definition names no icon")?;
    let key = std::str::from_utf8(key).map_err(|_| "The medal's icon is not UTF-8")?.to_owned();
    Ok(SpriteFrame { key, frame: definition.frame.unwrap_or(1).saturating_sub(1), recorded: None, text: None })
}

fn medal_icon(content: &Content, definition: Option<i32>, key: &str) -> Result<SpriteFrame, String> {
    let definition = definition.ok_or("The award's medal definition was not recorded")?;
    sprite_of(medal_definitions(content)?.get(definition, key.as_bytes())?)
}

pub(super) fn unidentified(total: i32, identified: u32) -> Option<CareerAward> {
    let total = u32::try_from(total).ok()?;
    (total > identified).then(|| CareerAward { medal: None, count: total - identified })
}

fn history_of(rows: &[SharedDivisionHistoryRow], division: u64) -> &[SharedDivisionHistoryRow] {
    let start = rows.partition_point(|row| row.division < division);
    &rows[start..start + rows[start..].partition_point(|row| row.division == division)]
}

struct Levels {
    experience: Result<content_reader_key_names::unit_leader::DivisionExperience, String>,
    names: BTreeMap<i32, Option<String>>,
}

impl Levels {
    fn read(content: &Content) -> Self {
        let experience = content
            .cat
            .as_ref()
            .map_err(|_| "Experience levels need the recorded content stack".to_owned())
            .and_then(|cat| content_reader_key_names::unit_leader::DivisionExperience::read(cat.defines()?).map_err(|why| why.to_string()));
        Self { experience, names: BTreeMap::new() }
    }
}

struct Facts<'a> {
    frame: &'a DecodedFrame,
    content: &'a Content,
    medals: MedalIndex,
    levels: Levels,
}

impl Facts<'_> {
    fn of(&mut self, history: &[SharedDivisionHistoryRow], status: Option<&DivisionStatusRow>, manpower: Option<i32>, refusals: &mut Refusals) -> CareerFacts {
        let mut out = CareerFacts { medals: None, awards: Vec::new(), history_missing: None, experience_raw: None, level: None, level_name: None, casualties_taken: status.and_then(|row| row.casualties_taken()) };
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
                    if let Some(at) = self.medals.index(self.frame, token, entry.medal_definition, self.content, refusals) {
                        let count = counts.entry(at).or_default();
                        if *count == 0 {
                            order.push(at);
                        }
                        *count += 1;
                    }
                }
                out.awards = order.into_iter().map(|medal| CareerAward { medal: Some(medal), count: counts[&medal] }).collect();
                if let Some(rest) = unidentified(summary.amount, counts.values().sum()) {
                    refusals.note("A division's medal total includes awards that were not identified");
                    out.awards.push(rest);
                }
            }
        }
        if let Some(experience) = status.zip(manpower).and_then(|(status, manpower)| status.normalized_experience(manpower)) {
            out.experience_raw = Some(experience.to_string());
            match &self.levels.experience {
                Ok(levels) => {
                    let level = levels.progress(experience).level;
                    out.level = Some(level);
                    let content = self.content;
                    out.level_name = self.levels.names.entry(level).or_insert_with(|| content.localised(&format!("UNIT_LEVEL_{level}")).ok()).clone();
                }
                Err(why) => refusals.note(why.clone()),
            }
        }
        out
    }
}

fn status_of(frame: &DecodedFrame, division: u64) -> Option<&DivisionStatusRow> {
    find(frame.complete::<tables::DivisionStatus>().ok(), division, |row| row.division)
}

fn manpower_of(frame: &DecodedFrame, division: u64) -> Option<i32> {
    let row = find(frame.complete::<tables::DivisionLandInputs>().ok(), division, |row| row.division)?;
    Some(row.manpower.iter().fold(0i32, |sum, part| sum.wrapping_add(part.amount)))
}

fn careers(frame: &DecodedFrame, slots: Option<&[u32]>, at: &At<'_, Content>, refusals: &mut Refusals) -> io::Result<DivisionCareersReply> {
    let mut reply = DivisionCareersReply { divisions: Vec::new(), medals: Vec::new(), templates: Vec::new(), missing: None, warnings: Vec::new() };
    let divisions = match frame.complete::<tables::Divisions>() {
        Ok(rows) => rows,
        Err(why) => {
            reply.missing = Some(format!("Divisions were not read at this hour: {why}"));
            return Ok(reply);
        }
    };
    let content = at.context;
    let countries = Countries::read(frame)?.named(at.holders, at.hour);
    let history = frame.complete::<tables::DivisionHistory>().unwrap_or(&[]);
    let mut facts = Facts { frame, content, medals: MedalIndex::default(), levels: Levels::read(content) };
    content.group_notes(refusals);
    let mut templates = BTreeMap::<u64, ()>::new();
    for division in divisions.iter().filter(|row| row.reveal_skip == 0) {
        if reply.divisions.len().is_multiple_of(1024) {
            dashboard_text::metadata::check_cancel(at.cancelled)?;
        }
        let raw = attributed(division, ArmyAttribution::Owner);
        if let Some(slots) = slots
            && !countries.slot(raw).is_some_and(|slot| slots.contains(&slot))
        {
            continue;
        }
        let status = status_of(frame, division.id);
        let (name, name_missing) = refusals.keep(content.division_name(status));
        let facts = facts.of(history_of(history, division.id), status, manpower_of(frame, division.id), refusals);
        templates.insert(division.template, ());
        reply.divisions.push(CareerRow { division: division.id.to_string(), tag: countries.tag(raw), template: division.template.to_string(), name, name_missing, facts });
    }
    for template in templates.into_keys() {
        reply.templates.push(CareerTemplate {
            template: template.to_string(),
            name: refusals.keep(content.template_name(frame, template)).0,
            icon: refusals.keep(content.template_icon(frame, template)).0,
        });
    }
    reply.medals = facts.medals.medals;
    Ok(reply)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Measures {
    pub id: u64,
    pub medals: Option<i32>,
    pub experience: Option<i64>,
    pub casualties_taken: Option<i32>,
}

impl Measures {
    fn of(&self, by: DecoratedRank) -> Option<i64> {
        match by {
            DecoratedRank::Medals => self.medals.map(i64::from),
            DecoratedRank::Experience => self.experience,
            DecoratedRank::CasualtiesTaken => self.casualties_taken.map(i64::from),
        }
    }
}

fn missing_last(a: Option<i64>, b: Option<i64>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) if descending => b.cmp(&a),
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn rank_order(a: &Measures, b: &Measures, by: DecoratedRank, descending: bool) -> Ordering {
    missing_last(a.of(by), b.of(by), descending)
        .then_with(|| RANKS.iter().filter(|&&other| other != by).map(|&other| missing_last(a.of(other), b.of(other), true)).find(|order| order.is_ne()).unwrap_or(Ordering::Equal))
        .then(a.id.cmp(&b.id))
}

pub(super) fn first_in_order(mut measured: Vec<Measures>, by: DecoratedRank, descending: bool, count: usize) -> Vec<Measures> {
    let order = |a: &Measures, b: &Measures| rank_order(a, b, by, descending);
    if count < measured.len() {
        measured.select_nth_unstable_by(count, order);
        measured.truncate(count);
    }
    measured.sort_unstable_by(order);
    measured
}

pub(super) fn filter_valid_through(countries: Option<&[CountryHours]>, hour: i32) -> i32 {
    countries
        .into_iter()
        .flatten()
        .flat_map(|span| [span.first_hour, span.last_hour.saturating_add(1)])
        .filter(|&boundary| boundary > hour)
        .min()
        .map_or(i32::MAX, |boundary| boundary - 1)
}

fn rank_word(by: DecoratedRank) -> &'static str {
    match by {
        DecoratedRank::Medals => "medals",
        DecoratedRank::Experience => "experience",
        DecoratedRank::CasualtiesTaken => "casualties taken",
    }
}

fn summary_medals(history: &[SharedDivisionHistoryRow]) -> Option<i32> {
    history.first().filter(|row| row.list == DivisionHistoryRow::SUMMARY && row.available != 0).map(|row| row.amount)
}

fn decorated(frame: &DecodedFrame, params: &DecoratedDivisionsParams, at: &At<'_, Content>, refusals: &mut Refusals) -> io::Result<DecoratedDivisionsReply> {
    let mut reply = DecoratedDivisionsReply { divisions: Vec::new(), medals: Vec::new(), templates: Vec::new(), partial: None, missing: None, warnings: Vec::new() };
    let divisions = match frame.complete::<tables::Divisions>() {
        Ok(rows) => rows,
        Err(why) => {
            reply.missing = Some(format!("Divisions were not read at this hour: {why}"));
            return Ok(reply);
        }
    };
    let content = at.context;
    let countries = Countries::read(frame)?.named(at.holders, at.hour);
    let passing = params.countries.as_ref().map(|spans| spans.iter().filter(|span| span.first_hour <= at.hour && at.hour <= span.last_hour).map(|span| span.tag.as_str()).collect::<HashSet<&str>>());
    let history = frame.complete::<tables::DivisionHistory>().unwrap_or(&[]);
    let mut measured = Vec::new();
    let (mut unplaced, mut unmeasured) = (0usize, 0usize);
    for (seen, division) in divisions.iter().filter(|row| row.reveal_skip == 0).enumerate() {
        if seen.is_multiple_of(4096) {
            dashboard_text::metadata::check_cancel(at.cancelled)?;
        }
        if let Some(passing) = &passing {
            match countries.tag(attributed(division, ArmyAttribution::Owner)) {
                Some(tag) if passing.contains(tag.as_str()) => {}
                Some(_) => continue,
                None => {
                    unplaced += 1;
                    continue;
                }
            }
        }
        let status = status_of(frame, division.id);
        let measures = Measures {
            id: division.id,
            medals: summary_medals(history_of(history, division.id)),
            experience: status.zip(manpower_of(frame, division.id)).and_then(|(status, manpower)| status.normalized_experience(manpower)),
            casualties_taken: status.and_then(DivisionStatusRow::casualties_taken),
        };
        unmeasured += usize::from(measures.of(params.by).is_none());
        measured.push(measures);
    }
    let mut partial = Vec::new();
    if unmeasured > 0 {
        partial.push(format!("{unmeasured} of {} divisions' {} were not read at this hour; they follow the others", measured.len(), rank_word(params.by)));
    }
    if unplaced > 0 {
        partial.push(format!("{unplaced} divisions' countries were not recorded at this hour; they are left out"));
    }
    reply.partial = (!partial.is_empty()).then(|| partial.join(". "));
    let first = first_in_order(measured, params.by, params.descending, usize::try_from(params.count).unwrap_or(usize::MAX));
    let mut facts = Facts { frame, content, medals: MedalIndex::default(), levels: Levels::read(content) };
    content.group_notes(refusals);
    let mut templates = BTreeMap::<u64, ()>::new();
    for measures in first {
        let Some(division) = find(Some(divisions), measures.id, |row| row.id) else { continue };
        let status = status_of(frame, division.id);
        let (name, name_missing) = refusals.keep(content.division_name(status));
        let row_facts = facts.of(history_of(history, division.id), status, manpower_of(frame, division.id), refusals);
        templates.insert(division.template, ());
        reply.divisions.push(CareerRow { division: division.id.to_string(), tag: countries.tag(attributed(division, ArmyAttribution::Owner)), template: division.template.to_string(), name, name_missing, facts: row_facts });
    }
    for template in templates.into_keys() {
        reply.templates.push(CareerTemplate {
            template: template.to_string(),
            name: refusals.keep(content.template_name(frame, template)).0,
            icon: refusals.keep(content.template_icon(frame, template)).0,
        });
    }
    reply.medals = facts.medals.medals;
    Ok(reply)
}

fn record(frame: &DecodedFrame, division: u64, at: &At<'_, Content>, refusals: &mut Refusals) -> io::Result<DivisionRecordReply> {
    let mut reply = DivisionRecordReply {
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
    let Some(row) = find(Some(divisions), division, |row| row.id) else {
        reply.absent = true;
        return Ok(reply);
    };
    let content = at.context;
    content.group_notes(refusals);
    let countries = Countries::read(frame)?.named(at.holders, at.hour);
    reply.tag = countries.tag(attributed(row, ArmyAttribution::Owner));
    let status = status_of(frame, division);
    (reply.name, reply.name_missing) = refusals.keep(content.division_name(status));
    reply.template = Some(CareerTemplate {
        template: row.template.to_string(),
        name: refusals.keep(content.template_name(frame, row.template)).0,
        icon: refusals.keep(content.template_icon(frame, row.template)).0,
    });
    let history = history_of(frame.complete::<tables::DivisionHistory>().unwrap_or(&[]), division);
    let mut facts = Facts { frame, content, medals: MedalIndex::default(), levels: Levels::read(content) };
    reply.facts = Some(facts.of(history, status, manpower_of(frame, division), refusals));
    if reply.facts.as_ref().is_some_and(|facts| facts.history_missing.is_none()) {
        for entry in history.iter().filter(|row| row.list != DivisionHistoryRow::SUMMARY) {
            let award = entry.list == DivisionHistoryRow::AWARDS;
            if award && entry.medal_enabled == 0 {
                continue;
            }
            let medal = if award { entry.medal_token.and_then(|token| facts.medals.index(frame, token, entry.medal_definition, content, refusals)) } else { None };
            let description = if award {
                medal.and_then(|at| facts.medals.medals[at].name.clone()).ok_or_else(|| "Medal name unavailable".to_owned())
            } else {
                describe(frame, entry, content, &countries)
            };
            let (description, description_missing) = refusals.keep(description);
            reply.entries.push(RecordEntry { award, ordinal: entry.ordinal, hour: entry.hour, description, description_missing, provinces: entry.provinces.clone(), medal });
        }
    }
    reply.medals = facts.medals.medals;
    Ok(reply)
}

fn province_name(frame: &DecodedFrame, text: &dashboard_text::scripted_text::ScriptedText<'_>, province: u32) -> Result<String, String> {
    if let Some(name) = text.province_display_name_primary(province)? {
        return catalogue::plain_text(&name);
    }
    let provinces = frame.complete::<tables::Provinces>().map_err(|why| format!("Provinces were not read: {why}"))?;
    let row = find(Some(provinces), u64::from(province), |row| u64::from(row.id)).ok_or_else(|| format!("Province {province} was not read"))?;
    text.plain_state_name(row.state).and_then(|name| catalogue::plain_text(&name))
}

fn describe(frame: &DecodedFrame, entry: &DivisionHistoryRow, content: &Content, countries: &Countries<'_>) -> Result<String, String> {
    let cat = content.cat.as_ref().map_err(|_| "Entry descriptions need the recorded content stack".to_owned())?;
    let loc = &cat.localisation;
    let text = |key: &[u8], arguments: &[(&str, Argument<'_>)]| -> Result<String, String> {
        loc.localise_with(key, arguments).map_err(|why| why.to_string()).and_then(|text| catalogue::plain_text(&text))
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
    let provinces = if entry.provinces.is_empty() {
        Vec::new()
    } else {
        let scripted = dashboard_text::scripted_text::ScriptedText::new(frame, loc, cat.scripted_content()?, 0);
        entry.provinces.iter().map(|&province| province_name(frame, &scripted, province)).collect::<Result<Vec<_>, _>>()?
    };
    let province = division_history::province_list(&provinces, text)?;
    let date = division_history::date(entry.hour);
    text(division_history::event_key(entry)?, &division_history::arguments(entry, &subject, &country, &province, &date))
}
