//! Stability and war support beyond their declared series: the breakdown behind a country's
//! values at an hour (formula terms, modifier tree lines), and the names of the parts of the
//! declared `stability-parts` series.

use super::at_hour::{At, AtHour, Keys, Owner, Read};
use super::{LoadContext, validate_tag};
use crate::Source;
use dashboard_base::products::politics_content::{PoliticsContent, politics_content};
use dashboard_text::idea_names::IdeaCountry;
use dashboard_text::identity::{Look, built_template_in, character_script_token_in};
use dashboard_text::stability as formula;
use installed_content_core::characters::BuiltTemplate;
use replay_data::rows::{
    CharacterRow, CountryStabilityInputRow, CountryStabilityLineRow, Recorded, StabilityPartKeyRow, StabilityPartRow, TableId, tables,
};
use replay_frame::frame::{DecodedFrame, Rowset};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io, sync::Arc};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub mod naming;
pub mod reads;
#[cfg(test)]
mod tests;

use naming::{RecordedCharacter, line_group, line_name};

/// One additive term of a displayed value; none when an input it needs was not recorded.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FormulaTerm {
    pub term: String,
    pub value_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FormulaBreakdown {
    /// The displayed value, fixed5.
    pub value_raw: Option<String>,
    pub terms: Vec<FormulaTerm>,
    /// Displayed minus the unclamped sum: the clamp's contribution.
    pub clamp_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BreakdownLine {
    /// The child's position in the modifier tree (the tooltip's order); 0 for a part's line.
    pub line: u32,
    /// `unpaired`, `idea`, `countryLeader`, `ideology`, `advisor`, `static`, `staticScaled`,
    /// `dynamicModifier` or `dynamicModifierValues`.
    pub kind: String,
    /// Script identity; empty for a leader or an unpaired line.
    pub identity: String,
    /// Character id or dynamic modifier scope country; zero otherwise.
    pub identity_id: u32,
    /// Dynamic modifier scope state; zero otherwise.
    pub secondary_raw: String,
    pub value_raw: String,
    /// An unpaired line's recorded display text, as plain text.
    pub text: Option<String>,
    /// A paired line's name in the viewer's language; none when content does not name it.
    pub label: Option<String>,
    /// An idea slot's or advisor role's name in the viewer's language.
    pub group: Option<String>,
    /// For an idea in a category rather than a slot, the category's content type.
    pub group_kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ModifierBreakdown {
    /// The modifier's script name.
    pub key: String,
    /// The recorded key index; none when the game registers no such key.
    pub index: Option<u32>,
    /// ModifierDefinitions' `good` flag: only for good keys does every line equal its
    /// contribution to the total.
    pub good: Option<bool>,
    pub total_raw: Option<String>,
    pub lines: Vec<BreakdownLine>,
    /// Total minus the lines, when it is not zero.
    pub remainder_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StabilityBreakdown {
    pub hour: i32,
    /// The hour the inputs were read at.
    pub read_hour: i32,
    /// The game language of unpaired lines' text.
    pub language: String,
    pub stability: FormulaBreakdown,
    pub war_support: FormulaBreakdown,
    pub max_command_power: FormulaBreakdown,
    /// False when the lines were not recorded by the same read.
    pub lines_available: bool,
    pub modifiers: Vec<ModifierBreakdown>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StabilityBreakdownParams {
    pub branch_id: String,
    pub tag: String,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StabilityBreakdownReply {
    pub tag: String,
    pub hour: i32,
    /// None where the hour holds no breakdown; `missing` says why.
    pub breakdown: Option<StabilityBreakdown>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StabilityPartNamesParams {
    pub branch_id: String,
    pub tag: String,
    /// Parts of the `stability-parts` series, as `series-parts` names them: measure and handle.
    pub parts: Vec<String>,
}

/// One part of a country's stability or war support, named.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StabilityPartName {
    /// The part as asked.
    pub part: String,
    /// `stability` or `warSupport`.
    pub measure: String,
    /// `term`, `line`, `remainder`, `clamp` or `total` (the displayed value: hours it is recorded
    /// are the hours the parts are known).
    pub kind: String,
    /// Its source, the same for the same source whatever its handle: `term:<term>`,
    /// `line:<kind>:<identity>:<id>:<secondary>[:<script>:<saved>]`, `unpaired:<text>`,
    /// `unpaired-refused:<escaped label>`, `remainder`, `clamp` or `total`.
    pub id: Option<String>,
    /// The formula term of a term part.
    pub term: Option<String>,
    /// A line part's line, named; its tree position and value are not a part's.
    pub line: Option<BreakdownLine>,
    /// Why it is not named.
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StabilityPartNamesReply {
    pub tag: String,
    pub hour: i32,
    pub parts: Vec<StabilityPartName>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(StabilityBreakdownReply, StabilityPartNamesReply);

/// The recorded content names are read from, or none and why.
pub struct Naming {
    content: Option<Arc<PoliticsContent>>,
    warnings: Vec<String>,
}

fn naming(source: &Source, load: &LoadContext) -> io::Result<Naming> {
    // Without content, lines keep their identities and recorded text.
    let Some(options) = load.content.as_ref() else {
        return Ok(Naming { content: None, warnings: Vec::new() });
    };
    Ok(match politics_content(source, options) {
        Ok(content) => Naming { warnings: content.warnings.clone(), content: Some(content) },
        Err(error) => Naming { content: None, warnings: vec![format!("Stability lines are not named: {error}")] },
    })
}

impl Naming {
    fn finish(&self, warnings: &mut Vec<String>) {
        warnings.extend(self.warnings.iter().cloned());
        if let Some(content) = &self.content {
            warnings.extend(content.catalogue.refusal_lines());
        }
    }
}

const GOOD_FLAG: u32 = 0x2;
/// The modifier key whose lines make up each measure's modifier term.
pub const LINES_OF: [(&str, &str); 2] = [("stabilityFactor", "stability_factor"), ("warSupportFactor", "war_support_factor")];

/// The tables a dynamic country's civil war is read from, for its idea names.
const CIVIL_WAR: [TableId; 6] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryCivilWars,
    TableId::CountryCounterInputs,
    TableId::CountryWars,
    TableId::WarRelations,
];

fn sliced(table: TableId, owner: u32) -> u128 {
    u128::from(table.index() as u32) << 32 | u128::from(owner)
}

/// Whether the owner's appearance names a dynamic country, whose idea names need its civil war.
fn dynamic(held: &DecodedFrame, owner: Owner) -> bool {
    held.complete::<tables::CountryAppearance>()
        .is_ok_and(|rows| rows.iter().any(|row| row.country == owner.id && !row.name_tag.is_empty()))
}

/// The owner's naming country at the frame's hour: its tag, slot and look.
fn look(frame: &DecodedFrame, owner: Owner) -> Option<Look> {
    let countries = frame.complete::<tables::Countries>().ok()?;
    let row = countries.iter().find(|row| row.id == owner.id)?;
    Look::read(frame, row)
}

pub fn kind(kind: u8) -> &'static str {
    match kind {
        CountryStabilityLineRow::IDEA => "idea",
        CountryStabilityLineRow::COUNTRY_LEADER => "countryLeader",
        CountryStabilityLineRow::IDEOLOGY => "ideology",
        CountryStabilityLineRow::ADVISOR => "advisor",
        CountryStabilityLineRow::STATIC => "static",
        CountryStabilityLineRow::STATIC_SCALED => "staticScaled",
        CountryStabilityLineRow::DYNAMIC_MODIFIER => "dynamicModifier",
        CountryStabilityLineRow::DYNAMIC_MODIFIER_VALUES => "dynamicModifierValues",
        _ => "unpaired",
    }
}

fn breakdown(terms: formula::Terms) -> FormulaBreakdown {
    FormulaBreakdown {
        value_raw: terms.value.map(|value| value.to_string()),
        clamp_raw: terms.value.zip(terms.unclamped).map(|(value, unclamped)| value.wrapping_sub(unclamped).to_string()),
        terms: terms.terms.into_iter().map(|(term, value)| FormulaTerm { term: term.into(), value_raw: value.map(|value| value.to_string()) }).collect(),
    }
}

/// One key's lines of a read: `name` and `index` its script name and recorded index (`u32::MAX`
/// where the game registers none), `total` its total, `lines` the country's lines of the read.
fn modifier(
    (name, index): (&[u8], u32),
    total: Option<i64>,
    lines: Option<&[CountryStabilityLineRow]>,
    good: &impl Fn(u32) -> Option<bool>,
    refused: &mut dashboard_text::catalogue::Refusals,
    tag: &str,
) -> ModifierBreakdown {
    let index = (index != u32::MAX).then_some(index);
    let rows: Vec<&CountryStabilityLineRow> = match (lines, index) {
        (Some(lines), Some(index)) => lines.iter().filter(|line| line.key == index).collect(),
        _ => Vec::new(),
    };
    let sum = rows.iter().fold(0_i64, |sum, line| sum.wrapping_add(line.value));
    ModifierBreakdown {
        key: String::from_utf8_lossy(name).into_owned(),
        index,
        good: index.and_then(good),
        total_raw: total.map(|total| total.to_string()),
        remainder_raw: total.filter(|_| lines.is_some()).map(|total| total.wrapping_sub(sum)).filter(|remainder| *remainder != 0).map(|remainder| remainder.to_string()),
        lines: rows
            .into_iter()
            .map(|line| {
                let identity = String::from_utf8_lossy(&line.identity).into_owned();
                let text = (line.kind == CountryStabilityLineRow::UNPAIRED)
                    .then(|| dashboard_text::catalogue::display_text(&line.text))
                    .and_then(|text| text.inspect_err(|reason| refused.note(format!("the recorded tree label {reason}"), format!("{tag} {identity}"))).ok());
                BreakdownLine {
                    line: line.line,
                    kind: kind(line.kind).into(),
                    identity,
                    identity_id: line.identity_id,
                    secondary_raw: line.secondary.to_string(),
                    value_raw: line.value.to_string(),
                    text,
                    label: None,
                    group: None,
                    group_kind: None,
                }
            })
            .collect(),
    }
}

/// The breakdown of `row` (the owner's inputs, read at `read`) at the frame's hour: formula terms
/// and each recorded key's lines, unnamed. `lines` is none when the read recorded no lines with
/// its inputs.
pub fn breakdown_of(frame: &DecodedFrame, row: &CountryStabilityInputRow, slot: u32, read: i32, lines: Option<&[CountryStabilityLineRow]>, tag: &str, warnings: &mut Vec<String>) -> StabilityBreakdown {
    let definitions = frame.complete::<tables::ModifierDefinitions>().ok();
    let good = |index: u32| definitions.and_then(|rows| rows.binary_search_by_key(&index, |row| row.index).ok().map(|at| rows[at].flags & GOOD_FLAG != 0));
    let command = formula::command_power(frame, slot);
    let stability = row.totals();
    // The last two keys' totals are CountryCommandPower's, their one record.
    let totals: [Option<i64>; 10] = std::array::from_fn(|index| match index {
        8 => command.and_then(|command| command.max_command_power),
        9 => command.and_then(|command| command.max_command_power_factor),
        index => stability[index],
    });
    let mut refused = dashboard_text::catalogue::Refusals::default();
    let modifiers = CountryStabilityInputRow::KEY_NAMES
        .iter()
        .zip(&row.keys)
        .zip(totals)
        .map(|((&name, &index), total)| modifier((name, index), total, lines, &good, &mut refused, tag))
        .collect();
    warnings.extend(refused.lines("stability tree labels are"));
    StabilityBreakdown {
        hour: frame.hour,
        read_hour: read,
        language: String::from_utf8_lossy(&row.language).into_owned(),
        stability: breakdown(formula::stability(row)),
        war_support: breakdown(formula::war_support(row)),
        max_command_power: breakdown(formula::max_command_power(row, command)),
        lines_available: lines.is_some(),
        modifiers,
    }
}

/// A character line's recorded character at the frame's hour.
fn frame_character<'a>(line: &BreakdownLine, characters: Option<&'a [CharacterRow]>, lexical: Option<&'a [replay_data::rows::LexicalTokenRow]>) -> Option<RecordedCharacter<'a>> {
    let rows = characters?;
    let row = &rows[rows.binary_search_by_key(&line.identity_id, |row| row.id).ok()?];
    Some(RecordedCharacter {
        script: lexical.and_then(|rows| character_script_token_in(row, Rowset::Decoded(rows))),
        saved: std::str::from_utf8(&row.token).ok().unwrap_or_default(),
        template: built_template_in(row, lexical.map(Rowset::Decoded)),
    })
}

fn is_character(kind: u8) -> bool {
    kind == CountryStabilityLineRow::ADVISOR || kind == CountryStabilityLineRow::COUNTRY_LEADER
}

/// The owner's stability lines of the frame (its key range), when the frame holds them.
fn owner_lines(frame: &DecodedFrame) -> Option<&[CountryStabilityLineRow]> {
    frame.complete::<tables::CountryStabilityLines>().ok()
}

pub struct CountryStabilityBreakdown;

impl AtHour for CountryStabilityBreakdown {
    const KEY: &'static str = "country-stability-breakdown";
    const TITLE: &'static str = "Stability breakdown";
    const CONTENT_DEPENDENT: bool = true;
    type Params = StabilityBreakdownParams;
    type Reply = StabilityBreakdownReply;
    type Context = Naming;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            StabilityBreakdownParams::decl(),
            StabilityBreakdownReply::decl(),
            StabilityBreakdown::decl(),
            FormulaBreakdown::decl(),
            FormulaTerm::decl(),
            ModifierBreakdown::decl(),
            BreakdownLine::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid stability branch");
        }
        validate_tag(&params.tag)
    }

    fn owner(params: &Self::Params) -> Option<&str> {
        Some(&params.tag)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![
            TableId::CountryStabilityInputs,
            TableId::CountryStabilityLines,
            TableId::SliceReads,
            TableId::CountryCommandPower,
            TableId::ModifierDefinitions,
            TableId::CountryAppearance,
            TableId::Characters,
            TableId::LexicalTokens,
        ];
        tables.extend(CIVIL_WAR);
        tables
    }

    /// The owner's inputs, lines, read hours and command power, the key definitions and its
    /// appearance; then its lines' characters (and a dynamic country's civil war); then their tokens.
    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        let id = u128::from(owner.id);
        match step {
            0 => vec![
                Read::these(TableId::Countries, [id]),
                Read::these(TableId::CountryStabilityInputs, [id]),
                Read { table: TableId::CountryStabilityLines, keys: Keys::Range(id << 64, id << 64 | u128::from(u64::MAX)) },
                Read::these(TableId::SliceReads, [sliced(TableId::CountryStabilityInputs, owner.id), sliced(TableId::CountryStabilityLines, owner.id)]),
                Read::these(TableId::CountryCommandPower, [u128::from(owner.slot)]),
                Read::all(TableId::ModifierDefinitions),
                Read::these(TableId::CountryAppearance, [id]),
            ],
            1 => {
                let characters: BTreeSet<u128> = owner_lines(held)
                    .into_iter()
                    .flatten()
                    .filter(|line| is_character(line.kind))
                    .map(|line| u128::from(line.identity_id))
                    .collect();
                let mut reads = vec![Read::these(TableId::Characters, characters)];
                if dynamic(held, owner) {
                    reads.extend(CIVIL_WAR.map(Read::all));
                }
                reads
            }
            2 => {
                let tokens: BTreeSet<u128> = held
                    .complete::<tables::Characters>()
                    .into_iter()
                    .flatten()
                    .flat_map(|row| [u32::try_from(row.name_token_id).ok(), Some(row.template_tag)])
                    .flatten()
                    .filter(|&token| token != 0)
                    .map(u128::from)
                    .collect();
                if tokens.is_empty() { Vec::new() } else { vec![Read::these(TableId::LexicalTokens, tokens)] }
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        naming(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Stability breakdown country is unavailable at this hour"))?;
        let mut warnings = Vec::new();
        let (breakdown, missing) = match breakdown_at(frame, owner, &params.tag, &at.context.content, &mut warnings) {
            Ok(breakdown) => (Some(breakdown), None),
            Err(why) => (None, Some(why)),
        };
        at.context.finish(&mut warnings);
        let reply = StabilityBreakdownReply { tag: params.tag.clone(), hour: frame.hour, breakdown, missing, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

/// The owner's breakdown at the frame's hour, its lines named where content is held; why not
/// where the hour holds none.
fn breakdown_at(frame: &DecodedFrame, owner: Owner, tag: &str, content: &Option<Arc<PoliticsContent>>, warnings: &mut Vec<String>) -> Result<StabilityBreakdown, String> {
    let inputs = frame.complete::<tables::CountryStabilityInputs>().map_err(|_| "The stability inputs are not recorded at this hour.".to_owned())?;
    let row = inputs.iter().find(|row| row.country == owner.id).ok_or_else(|| "No stability inputs were recorded for this country at this hour.".to_owned())?;
    let reads = frame.complete::<tables::SliceReads>().map_err(|_| "The read behind this hour's values is not recorded.".to_owned())?;
    let read = replay_data::sliced::read_hour(reads, TableId::CountryStabilityInputs, owner.id)
        .ok_or_else(|| "The read behind this hour's values is not recorded.".to_owned())?;
    // SliceReads names an owner only while it has rows: a read that found no lines has none.
    let lines_read = replay_data::sliced::read_hour(reads, TableId::CountryStabilityLines, owner.id);
    let lines = owner_lines(frame).filter(|_| lines_read.is_none_or(|lines_read| lines_read == read));
    if lines.is_none() {
        warnings.push(format!("{tag}: the stability lines of hour {} were not recorded with its inputs", frame.hour));
    }
    let mut breakdown = breakdown_of(frame, row, owner.slot, read, lines, tag, warnings);
    if let Some(content) = content {
        let characters = frame.complete::<tables::Characters>().ok();
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        let look = look(frame, owner);
        let country = IdeaCountry { tag, slot: owner.slot, look: look.as_ref().map(Look::country_look) };
        for line in breakdown.modifiers.iter_mut().flat_map(|modifier| modifier.lines.iter_mut()) {
            let character = frame_character(line, characters, lexical);
            (line.group, line.group_kind) = line_group(line, content, character.as_ref());
            line.label = line_name(line, content, character, country);
        }
    }
    Ok(breakdown)
}

pub struct StabilityPartNames;

/// A part as asked: its measure and handle, from the low 40 bits of its row key.
fn part_of(text: &str) -> Option<(u8, u32)> {
    let part: u64 = text.parse().ok()?;
    let measure = u8::try_from(part >> 32).ok()?;
    (measure <= StabilityPartRow::WAR_SUPPORT).then_some((measure, part as u32))
}

impl AtHour for StabilityPartNames {
    const KEY: &'static str = "stability-part-names";
    const TITLE: &'static str = "Stability parts";
    const CONTENT_DEPENDENT: bool = true;
    type Params = StabilityPartNamesParams;
    type Reply = StabilityPartNamesReply;
    type Context = Naming;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![StabilityPartNamesParams::decl(), StabilityPartNamesReply::decl(), StabilityPartName::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid stability branch");
        }
        if params.parts.iter().any(|part| part_of(part).is_none()) {
            return Err("Invalid stability part");
        }
        validate_tag(&params.tag)
    }

    fn owner(params: &Self::Params) -> Option<&str> {
        Some(&params.tag)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![TableId::StabilityPartKeys, TableId::CountryAppearance];
        tables.extend(CIVIL_WAR);
        tables
    }

    /// The asked parts' dictionary rows and the owner's appearance; then a dynamic country's civil war.
    fn reads(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => vec![
                Read::these(TableId::StabilityPartKeys, params.parts.iter().filter_map(|part| part_of(part)).map(|(_, handle)| u128::from(handle))),
                Read::these(TableId::Countries, [u128::from(owner.id)]),
                Read::these(TableId::CountryAppearance, [u128::from(owner.id)]),
            ],
            1 if dynamic(held, owner) => CIVIL_WAR.map(Read::all).into(),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        naming(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Stability parts country is unavailable at this hour"))?;
        let mut warnings = Vec::new();
        let keys = frame.complete::<tables::StabilityPartKeys>();
        let look = look(frame, owner);
        let country = IdeaCountry { tag: &params.tag, slot: owner.slot, look: look.as_ref().map(Look::country_look) };
        let parts = params
            .parts
            .iter()
            .map(|part| {
                let (measure, handle) = part_of(part).expect("validated part");
                let row = match &keys {
                    Ok(rows) => rows.binary_search_by_key(&handle, |row| row.handle).ok().map(|at| &rows[at]).ok_or("This part is not in the parts dictionary at this hour."),
                    Err(_) => Err("The parts dictionary is not recorded at this hour."),
                };
                match row {
                    Ok(row) => named_part(part, measure, row, at.context.content.as_deref(), country),
                    Err(why) => StabilityPartName {
                        part: part.clone(),
                        measure: measure_name(measure).into(),
                        kind: String::new(),
                        id: None,
                        term: None,
                        line: None,
                        missing: Some(why.into()),
                    },
                }
            })
            .collect();
        at.context.finish(&mut warnings);
        let reply = StabilityPartNamesReply { tag: params.tag.clone(), hour: frame.hour, parts, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn measure_name(measure: u8) -> &'static str {
    if measure == StabilityPartRow::WAR_SUPPORT { "warSupport" } else { "stability" }
}

fn recorded<T>(value: &Recorded<T>) -> Option<&T> {
    match value {
        Recorded::Value(value) => Some(value),
        Recorded::NotRecorded => None,
    }
}

/// A dictionary row's character: none when neither its script nor saved name was recorded.
fn part_character(row: &StabilityPartKeyRow) -> Option<RecordedCharacter<'_>> {
    let script = recorded(&row.script).and_then(|bytes| std::str::from_utf8(bytes).ok());
    let saved = recorded(&row.saved).map(|bytes| std::str::from_utf8(bytes).ok().unwrap_or_default());
    if script.is_none() && saved.is_none() {
        return None;
    }
    let template = match (recorded(&row.template_present), recorded(&row.template_tag), recorded(&row.template_file), recorded(&row.template_line), recorded(&row.template_generated)) {
        (Some(0), ..) => Err(format!("character {} holds no template (recorded template is null)", row.identity_id)),
        (Some(_), Some(tag), Some(file), Some(&line), Some(&generated)) => Ok(BuiltTemplate { tag: String::from_utf8_lossy(tag).into_owned(), file: file.clone(), line, generated: generated != 0 }),
        _ => Err(format!("character {}'s template is not recorded", row.identity_id)),
    };
    Some(RecordedCharacter { script, saved: saved.unwrap_or_default(), template })
}

/// A part's identity and names from its dictionary row, the ids main's parts carried.
pub fn named_part(part: &str, measure: u8, row: &StabilityPartKeyRow, content: Option<&PoliticsContent>, country: IdeaCountry<'_>) -> StabilityPartName {
    let identity = String::from_utf8_lossy(&row.identity).into_owned();
    let plain = |kind: &str, id: String, term: Option<String>| StabilityPartName {
        part: part.to_owned(),
        measure: measure_name(measure).into(),
        kind: kind.into(),
        id: Some(id),
        term,
        line: None,
        missing: None,
    };
    match row.kind {
        StabilityPartKeyRow::TERM => return plain("term", format!("term:{identity}"), Some(identity)),
        StabilityPartKeyRow::REMAINDER => return plain("remainder", "remainder".into(), None),
        StabilityPartKeyRow::CLAMP => return plain("clamp", "clamp".into(), None),
        StabilityPartKeyRow::TOTAL => return plain("total", "total".into(), None),
        _ => {}
    }
    let refused = row.kind == StabilityPartKeyRow::REFUSED_LABEL;
    let mut line = BreakdownLine {
        line: 0,
        kind: if refused { "unpaired".into() } else { kind(row.kind).into() },
        identity: if row.kind == CountryStabilityLineRow::UNPAIRED || refused { String::new() } else { identity.clone() },
        identity_id: row.identity_id,
        secondary_raw: row.secondary.to_string(),
        value_raw: "0".into(),
        text: (row.kind == CountryStabilityLineRow::UNPAIRED).then(|| identity.clone()),
        label: None,
        group: None,
        group_kind: None,
    };
    let character = is_character(row.kind).then(|| part_character(row)).flatten();
    let id = if refused {
        format!("unpaired-refused:{}", row.identity.escape_ascii())
    } else if row.kind == CountryStabilityLineRow::UNPAIRED {
        format!("unpaired:{identity}")
    } else {
        let base = format!("line:{}:{}:{}:{}", line.kind, line.identity, line.identity_id, line.secondary_raw);
        match &character {
            Some(character) => format!("{base}:{}:{}", character.script.unwrap_or_default(), character.saved),
            None => base,
        }
    };
    if let Some(content) = content {
        (line.group, line.group_kind) = line_group(&line, content, character.as_ref());
        line.label = line_name(&line, content, character, country);
    }
    StabilityPartName { part: part.to_owned(), measure: measure_name(measure).into(), kind: "line".into(), id: Some(id), term: None, line: Some(line), missing: None }
}
