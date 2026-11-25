
pub mod service_record;
use super::{Content, Countries, Refusals, attributed, find, token_key};
use dashboard_base::{Source, catalogue, products::{LoadContext, at_hour::{At, AtHour}}};
use crate::products::army::ArmyAttribution;
use dashboard_query::coverage::witnessed;
use dashboard_text::division_history;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{CountryLinkRow, CountryRow, DivisionHistoryRow, DivisionLandInputsRow, DivisionRow, DivisionStatusRow, SharedDivisionHistoryRow, TableId, mask, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use content_reader_military::unit_medals::{Medal, Medals};
use dashboard_base::products::artwork::sprites::SpriteFrame;
use dashboard_world::products::styles::BranchLooks;
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

pub struct DecoratedDivisions;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum DecoratedRank {
    Medals,
    Experience,
    CasualtiesTaken,
    #[doc = " The division's name at the hour."]
    DivisionName,
    #[doc = " The name of the division's country at the hour, as the game shows it."]
    CountryName,
}

impl DecoratedRank {
    fn named(self) -> bool {
        matches!(self, Self::DivisionName | Self::CountryName)
    }
}

pub struct DecoratedContext {
    content: Content,
    looks: Result<BranchLooks, String>,
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

fn branch(id: &str) -> Result<(), &'static str> {
    id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
}

impl AtHour for DecoratedDivisions {
    const KEY: &'static str = "decorated-divisions";
    const TITLE: &'static str = "Most decorated divisions";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DecoratedDivisionsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]", "countries[].firstHour": Start, "countries[].lastHour": End, by, descending, count];
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Reply = DecoratedDivisionsReply;
    type Context = DecoratedContext;

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        let filter = filter_valid_through(params.countries.as_deref(), at.hour);
        match (&at.context.looks, params.by) {
            (Ok(looks), DecoratedRank::CountryName) => filter.min(looks.next_change(at.hour).saturating_sub(1)),
            _ => filter,
        }
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![DecoratedDivisionsParams::decl(), DecoratedDivisionsReply::decl(), DecoratedRank::decl(), CountryHours::decl(), CareerRow::decl(), CareerFacts::decl(), CareerAward::decl(), CareerMedal::decl(), CareerTemplate::decl()]
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

    fn columns(table: TableId) -> Option<Vec<usize>> {
        let fields = match table {
            TableId::Countries => mask::<CountryRow>(&["canonical_slot", "tag"]),
            TableId::CountryLinks => mask::<CountryLinkRow>(&["canonical_slot"]),
            TableId::Divisions => mask::<DivisionRow>(&["roster_owner", "expeditionary_owner", "template", "reveal_skip"]),
            TableId::DivisionStatus => mask::<DivisionStatusRow>(&["experience", "career_counter", "name", "naming", "country_ordinal"]),
            TableId::DivisionLandInputs => mask::<DivisionLandInputsRow>(&["manpower"]),
            _ => return None,
        };
        Some((0..64).filter(|column| fields & 1 << column != 0).collect())
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(DecoratedContext { content: Content::load(source, load), looks: Err("Country names need the campaign's branch".into()) })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let looks = if params.by == DecoratedRank::CountryName {
            let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
            dashboard_base::queries::query(source, branch)
                .and_then(|query| BranchLooks::load(source, &query.timeline, load, &mut Vec::new()))
                .map_err(|why| format!("Country names unavailable: {why}"))
        } else {
            Err("Country names are read only to rank by them".into())
        };
        Ok(DecoratedContext { content: Content::load(source, load), looks })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut refusals = Refusals::default();
        let mut reply = decorated(frame, params, at, &mut refusals)?;
        reply.warnings = refusals.lines();
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
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
    witnessed::<tables::DivisionStatus>(frame, u128::from(division)).row()
}

fn manpower_of(frame: &DecodedFrame, division: u64) -> Option<i32> {
    let row = witnessed::<tables::DivisionLandInputs>(frame, u128::from(division)).row()?;
    Some(row.manpower.iter().fold(0i32, |sum, part| sum.wrapping_add(part.amount)))
}

#[derive(Clone, Debug)]
pub(super) struct Measures {
    pub id: u64,
    pub medals: Option<i32>,
    pub experience: Option<i64>,
    pub casualties_taken: Option<i32>,
    #[doc = " The name the rank orders by, when it orders by a name: folded, then as shown."]
    pub name: Option<(String, String)>,
}

impl Measures {
    fn of(&self, by: DecoratedRank) -> Option<i64> {
        match by {
            DecoratedRank::Medals => self.medals.map(i64::from),
            DecoratedRank::Experience => self.experience,
            DecoratedRank::CasualtiesTaken => self.casualties_taken.map(i64::from),
            DecoratedRank::DivisionName | DecoratedRank::CountryName => None,
        }
    }

    fn known(&self, by: DecoratedRank) -> bool {
        if by.named() { self.name.is_some() } else { self.of(by).is_some() }
    }
}

pub(super) fn name_key(name: &str) -> (String, String) {
    (name.to_lowercase(), name.to_owned())
}

fn names_missing_last(a: Option<&(String, String)>, b: Option<&(String, String)>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) if descending => b.cmp(a),
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
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
    let first = if by.named() { names_missing_last(a.name.as_ref(), b.name.as_ref(), descending) } else { missing_last(a.of(by), b.of(by), descending) };
    first
        .then_with(|| RANKS.iter().filter(|&&other| other != by).map(|&other| missing_last(a.of(other), b.of(other), true)).find(|order| order.is_ne()).unwrap_or(Ordering::Equal))
        .then(a.id.cmp(&b.id))
}

pub(super) fn ranked(mut measured: Vec<Measures>, by: DecoratedRank, descending: bool, count: usize) -> (Vec<Measures>, bool) {
    let order = |a: &Measures, b: &Measures| rank_order(a, b, by, descending);
    let mut next = None;
    if count < measured.len() {
        measured.select_nth_unstable_by(count, order);
        next = Some(measured[count].clone());
        measured.truncate(count);
    }
    measured.sort_unstable_by(order);
    let unproven = measured.windows(2).any(|pair| decided_by_unread(&pair[0], &pair[1], by))
        || measured.last().zip(next.as_ref()).is_some_and(|(last, next)| decided_by_unread(last, next, by));
    (measured, unproven)
}

fn decided_by_unread(a: &Measures, b: &Measures, by: DecoratedRank) -> bool {
    let tied = if by.named() { a.name.is_some() && a.name == b.name } else { a.of(by).is_some() && a.of(by) == b.of(by) };
    if !tied {
        return false;
    }
    for &other in RANKS.iter().filter(|&&other| other != by) {
        match (a.of(other), b.of(other)) {
            (Some(x), Some(y)) if x == y => {}
            (Some(_), Some(_)) => return false,
            _ => return true,
        }
    }
    false
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
        DecoratedRank::DivisionName => "names",
        DecoratedRank::CountryName => "country names",
    }
}

fn summary_medals(history: &[SharedDivisionHistoryRow]) -> Option<i32> {
    history.first().filter(|row| row.list == DivisionHistoryRow::SUMMARY && row.available != 0).map(|row| row.amount)
}

fn rank_name(by: DecoratedRank, content: &Content, looks: &Result<BranchLooks, String>, status: Option<&DivisionStatusRow>, tag: Option<&str>, hour: i32) -> Option<(String, String)> {
    match by {
        DecoratedRank::DivisionName => content.division_name(status).ok().map(|name| name_key(&name)),
        DecoratedRank::CountryName => looks.as_ref().ok().zip(tag).and_then(|(looks, tag)| looks.at(tag, hour).name).map(|name| name_key(&name)),
        _ => None,
    }
}

fn decorated(frame: &DecodedFrame, params: &DecoratedDivisionsParams, at: &At<'_, DecoratedContext>, refusals: &mut Refusals) -> io::Result<DecoratedDivisionsReply> {
    let mut reply = DecoratedDivisionsReply { divisions: Vec::new(), medals: Vec::new(), templates: Vec::new(), partial: None, missing: None, warnings: Vec::new() };
    let divisions = match frame.complete::<tables::Divisions>() {
        Ok(rows) => rows,
        Err(why) => {
            reply.missing = Some(format!("Divisions were not read at this hour: {why}"));
            return Ok(reply);
        }
    };
    dashboard_base::products::at_hour::count_rows(|| divisions.len() as u64);
    let content = &at.context.content;
    let (countries, links_missing) = Countries::witnessed(frame);
    let countries = countries.named(at.holders, at.hour);
    if let Some(why) = links_missing {
        refusals.note(format!("Division countries unavailable: {why}"));
    }
    let passing = params.countries.as_ref().map(|spans| spans.iter().filter(|span| span.first_hour <= at.hour && at.hour <= span.last_hour).map(|span| span.tag.as_str()).collect::<HashSet<&str>>());
    let history = frame.complete::<tables::DivisionHistory>().unwrap_or(&[]);
    let mut measured = Vec::new();
    let (mut unplaced, mut unmeasured) = (0usize, 0usize);
    for (seen, division) in divisions.iter().filter(|row| row.reveal_skip == 0).enumerate() {
        if seen.is_multiple_of(4096) {
            dashboard_text::metadata::check_cancel(at.cancelled)?;
        }
        let tag = (passing.is_some() || params.by == DecoratedRank::CountryName).then(|| countries.tag(attributed(division, ArmyAttribution::Owner))).flatten();
        if let Some(passing) = &passing {
            match &tag {
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
            name: rank_name(params.by, content, &at.context.looks, status, tag.as_deref(), at.hour),
        };
        unmeasured += usize::from(!measures.known(params.by));
        measured.push(measures);
    }
    if passing.is_some() && measured.is_empty() && unplaced > 0 {
        reply.missing = Some(format!("{unplaced} divisions' countries were not recorded at this hour, so which belong to the chosen countries is not known"));
        return Ok(reply);
    }
    let mut partial = Vec::new();
    if unmeasured > 0 {
        partial.push(format!("{unmeasured} of {} divisions' {} were not read at this hour; they follow the others", measured.len(), rank_word(params.by)));
    }
    if unplaced > 0 {
        partial.push(format!("{unplaced} divisions' countries were not recorded at this hour; they are left out"));
    }
    let (first, unproven) = ranked(measured, params.by, params.descending, usize::try_from(params.count).unwrap_or(usize::MAX));
    if unproven {
        partial.push(format!("Divisions with equal {} are ordered by a measure not read for some of them", rank_word(params.by)));
    }
    reply.partial = (!partial.is_empty()).then(|| partial.join(". "));
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
    if matches!(entry.kind, DivisionHistoryRow::OPERATION | DivisionHistoryRow::OPERATION_EXCELLED) && !subject.is_empty() {
        return Ok(subject);
    }
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
