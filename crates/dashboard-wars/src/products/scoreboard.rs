
use dashboard_base::products::Country;
use dashboard_base::products::{BranchParams, LoadContext, Product};
use dashboard_base::{Source, catalogue};
use dashboard_query::{
    Query,
    series::{
        Line,
        board::{self, Rows},
    },
};
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ScoreboardColumnKey {
    Factories,
    ArmySize,
    Territory,
    Casualties,
    CasualtiesDealt,
    CasualtiesTaken,
    Planes,
    Ships,
    FactoriesBuilt,
    FactoriesTopbar,
    EquipmentProduced,
    PoliticalPower,
    CommandPower,
    Stability,
    WarSupport,
    Nukes,
}

struct Def {
    key: ScoreboardColumnKey,
    label: &'static str,
    localised: Option<&'static str>,
    unit: &'static str,
    measure: Option<(&'static str, &'static str)>,
    partial: Option<(&'static str, &'static str)>,
    reason: Option<&'static str>,
    note: Option<&'static str>,
}

const DEALT_NOTE: &str = "Recorded casualties dealt across land, naval and air. † marks a partial recorded subtotal; missing history may omit positive losses or negative corrections.";
const TAKEN_NOTE: &str = "Recorded casualties taken across land, naval and air with a known attacker, as in the Casualties widget's default. Losses without a recorded attacker are excluded; † marks a partial recorded subtotal.";
const PARTIAL: Option<(&str, &str)> = Some(("casualty-totals", "partial"));

const fn def(key: ScoreboardColumnKey, label: &'static str, unit: &'static str, measure: (&'static str, &'static str)) -> Def {
    Def { key, label, localised: None, unit, measure: Some(measure), partial: None, reason: None, note: None }
}

static DEFS: [Def; 16] = {
    use ScoreboardColumnKey::*;
    [
        def(Factories, "Factories", "factories", ("country-factories", "free7")),
        Def {
            note: Some("Manpower in the field, attributed to each division's owner, as in the Army size widget's default."),
            ..def(ArmySize, "Army size", "people", ("army-totals", "ownerManpower"))
        },
        def(Territory, "States owned", "states", ("territory", "ownedStates")),
        Def {
            key: Casualties,
            label: "Casualties",
            localised: None,
            unit: "people",
            measure: None,
            partial: None,
            reason: Some("Choose Casualties dealt or Casualties taken in settings."),
            note: None,
        },
        Def { partial: PARTIAL, note: Some(DEALT_NOTE), ..def(CasualtiesDealt, "Casualties dealt", "people", ("casualty-totals", "dealt")) },
        Def { partial: PARTIAL, note: Some(TAKEN_NOTE), ..def(CasualtiesTaken, "Casualties taken", "people", ("casualty-totals", "taken")) },
        def(Planes, "Number of planes", "planes", ("air-force-totals", "rosterPlanes")),
        def(Ships, "Number of ships", "ships", ("navy-totals", "rosterShips")),
        def(FactoriesBuilt, "Factories built", "factories", ("country-factories", "built7")),
        def(FactoriesTopbar, "Factories in top bar", "factories", ("country-factories", "topBar7")),
        def(EquipmentProduced, "Equipment produced", "units", ("production-country-totals", "unitsWithoutRefits")),
        Def { localised: Some("POLITICAL_POWER"), ..def(PoliticalPower, "Political power", "points", ("country-measure", "politicalPower")) },
        Def { localised: Some("COMMAND_POWER"), ..def(CommandPower, "Command power", "points", ("country-measure", "commandPower")) },
        Def { localised: Some("STABILITY"), ..def(Stability, "Stability", "percent", ("country-stability", "stability")) },
        Def { localised: Some("WAR_SUPPORT"), ..def(WarSupport, "War support", "percent", ("country-stability", "warSupport")) },
        def(Nukes, "Nukes", "nukes", ("country-nuke-totals", "total")),
    ]
};

const DEFAULT_COLUMNS: [ScoreboardColumnKey; 7] = {
    use ScoreboardColumnKey::*;
    [Factories, ArmySize, Territory, CasualtiesDealt, CasualtiesTaken, Planes, Ships]
};
const DEFAULT_SORT: ScoreboardColumnKey = ScoreboardColumnKey::Factories;

pub fn localised_keys() -> impl Iterator<Item = &'static str> {
    DEFS.iter().filter_map(|def| def.localised)
}

fn def_of(key: ScoreboardColumnKey) -> &'static Def {
    DEFS.iter().find(|def| def.key == key).expect("every column is defined")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardFamily {
    pub kind: String,
    pub measure: String,
}

fn declared(measure: (&str, &str)) -> Result<(&'static dashboard_declare::Series, &'static dashboard_declare::Measure), String> {
    let series = dashboard_declare::series(measure.0).ok_or_else(|| format!("The series {} is not declared", measure.0))?;
    if series.parts.is_some() || series.key == dashboard_declare::Key::Singleton {
        return Err(format!("{} is not one value a country", measure.0));
    }
    let declared = series.measure(measure.1).ok_or_else(|| format!("{} declares no measure {}", measure.0, measure.1))?;
    Ok((series, declared))
}

fn family(measure: (&str, &str)) -> ScoreboardFamily {
    ScoreboardFamily { kind: measure.0.to_owned(), measure: measure.1.to_owned() }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardColumn {
    pub key: ScoreboardColumnKey,
    pub label: String,
    pub unit: String,
    #[doc = " Fixed-point decimals of its raw values."]
    pub decimals: u8,
    pub available: bool,
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub note: Option<String>,
    #[doc = " The declared measure its values are, followed at the head."]
    pub family: Option<ScoreboardFamily>,
    #[doc = " The declared measure whose nonzero value marks its value a partial subtotal."]
    pub partial: Option<ScoreboardFamily>,
    pub none_at_zero: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardColumnsReply {
    pub branch_id: String,
    pub default_columns: Vec<ScoreboardColumnKey>,
    pub default_sort: ScoreboardColumnKey,
    pub columns: Vec<ScoreboardColumn>,
    pub warnings: Vec<String>,
}

fn registry(label: impl Fn(&str) -> Option<String>) -> Vec<ScoreboardColumn> {
    DEFS.iter()
        .map(|def| {
            let measure = def.measure.map(declared);
            let reason = match &measure {
                None => def.reason.map(str::to_owned),
                Some(Err(reason)) => Some(reason.clone()),
                Some(Ok(_)) => None,
            };
            ScoreboardColumn {
                key: def.key,
                label: def.localised.and_then(&label).unwrap_or_else(|| def.label.to_owned()),
                unit: def.unit.to_owned(),
                decimals: match &measure {
                    Some(Ok((_, measure))) => measure.decimals,
                    _ => 0,
                },
                available: reason.is_none(),
                reason,
                note: def.note.map(str::to_owned),
                family: def.measure.filter(|_| matches!(measure, Some(Ok(_)))).map(family),
                partial: def.partial.filter(|partial| declared(*partial).is_ok()).map(family),
                none_at_zero: matches!(&measure, Some(Ok((series, _))) if series.absent_none),
            }
        })
        .collect()
}

pub struct ScoreboardColumns;

impl Product for ScoreboardColumns {
    const KEY: &'static str = "scoreboard-columns";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = ScoreboardColumnsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ScoreboardColumnsReply::decl(), ScoreboardColumn::decl(), ScoreboardColumnKey::decl(), ScoreboardFamily::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, _cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let mut warnings = Vec::new();
        let catalogue = match context.content.as_ref() {
            Some(options) => match catalogue::catalogue(source, options) {
                Ok(catalogue) => {
                    warnings.extend(catalogue.refusal_lines());
                    Some(catalogue)
                }
                Err(error) => {
                    warnings.push(format!("The game's column names are unavailable: {error}"));
                    None
                }
            },
            None => {
                warnings.push("The game's column names are unavailable until the game folders are chosen; plain names are shown.".into());
                None
            }
        };
        let columns = registry(|key| catalogue.as_ref().and_then(|catalogue| catalogue.localised(key, catalogue::View::Plain)));
        Ok(ScoreboardColumnsReply {
            branch_id: params.branch_id.clone(),
            default_columns: DEFAULT_COLUMNS.to_vec(),
            default_sort: DEFAULT_SORT,
            columns,
            warnings,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScoreboardTop {
    pub count: u32,
    #[doc = " The column it ranks by."]
    pub sort: ScoreboardColumnKey,
    #[doc = " The greatest first; else the least, least first."]
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScoreboardParams {
    pub branch_id: String,
    pub hour: i32,
    pub columns: Vec<ScoreboardColumnKey>,
    #[doc = " The countries shown, in order; ignored with `top`."]
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<ScoreboardTop>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardCell {
    pub value_raw: Option<String>,
    #[doc = " Why there is no value: never zero."]
    pub missing: Option<String>,
    #[doc = " A subtotal of what was recorded (shown with †)."]
    pub partial: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardRow {
    #[doc = " The country asked, or a ranked country's tag."]
    pub country: Country,
    #[doc = " The tag it goes by at the hour."]
    pub tag: String,
    #[doc = " One a column asked, in its order."]
    pub cells: Vec<ScoreboardCell>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScoreboardReply {
    pub branch_id: String,
    pub hour: i32,
    #[doc = " Why the hour holds no rows."]
    pub missing: Option<String>,
    #[doc = " A top's countries greatest first; chosen countries in their order, each held at the hour."]
    pub rows: Vec<ScoreboardRow>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ScoreboardColumnsReply, ScoreboardReply);

pub struct Scoreboard;

impl Product for Scoreboard {
    const KEY: &'static str = "scoreboard";
    const CONTENT_DEPENDENT: bool = false;
    const INTERACTIVE: bool = true;
    type Params = ScoreboardParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, hour: Read, columns, countries, top, "countries[]", "top.count"];
    type Reply = ScoreboardReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ScoreboardParams::decl(),
            ScoreboardReply::decl(),
            ScoreboardTop::decl(),
            ScoreboardRow::decl(),
            ScoreboardCell::decl(),
            ScoreboardColumnKey::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.columns.is_empty() {
            return Err("A scoreboard names at least one column");
        }
        if params.top.as_ref().is_some_and(|top| top.count == 0) {
            return Err("A scoreboard's top names at least one country");
        }
        params.countries.iter().try_for_each(dashboard_base::products::validate_country)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let mut reply = answer(&query, params)?;
        dashboard_text::metadata::check_cancel(cancelled)?;
        reply.branch_id.clone_from(&params.branch_id);
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub(crate) fn answer(query: &Query<'_>, params: &ScoreboardParams) -> io::Result<ScoreboardReply> {
    let mut keys: Vec<ScoreboardColumnKey> = Vec::new();
    let mut columns = Vec::new();
    let mut add = |key: ScoreboardColumnKey| -> io::Result<()> {
        let def = def_of(key);
        if keys.contains(&key) {
            return Ok(());
        }
        if let Some(measure) = def.measure
            && let Some(line) = line_of(query, measure)?
        {
            keys.push(key);
            let partial = def.partial.map(|partial| line_of(query, partial)).transpose()?.flatten();
            columns.push(board::Column { line, partial });
        }
        Ok(())
    };
    for key in &params.columns {
        add(*key)?;
    }
    let rows = match &params.top {
        Some(top) => {
            add(top.sort)?;
            let by = keys.iter().position(|key| *key == top.sort).ok_or_else(|| dashboard_core::invalid(format!("{} is not a measure to rank by", def_of(top.sort).label)))?;
            Rows::Top { by, n: top.count as usize, least: !top.descending }
        }
        None => Rows::Countries(&params.countries),
    };
    let board = if columns.is_empty() { unmeasured(query, &params.countries, params.hour) } else { board::at(&columns, rows, params.hour)? };
    let rows = board
        .rows
        .into_iter()
        .map(|row| ScoreboardRow {
            country: row.country,
            tag: row.tag,
            cells: params
                .columns
                .iter()
                .map(|key| match keys.iter().position(|held| held == key) {
                    Some(at) => {
                        let cell = &row.cells[at];
                        ScoreboardCell { value_raw: cell.value_raw.clone(), missing: cell.missing.clone(), partial: cell.partial }
                    }
                    None => ScoreboardCell { value_raw: None, missing: Some(unavailable(*key)), partial: false },
                })
                .collect(),
        })
        .collect();
    Ok(ScoreboardReply { branch_id: String::new(), hour: params.hour, missing: board.missing, rows, warnings: Vec::new() })
}

fn line_of<'q, 'a>(query: &'q Query<'a>, measure: (&str, &str)) -> io::Result<Option<Line<'q, 'a>>> {
    match declared(measure) {
        Ok((series, declared)) => Line::new(query, series, declared).map(Some),
        Err(_) => Ok(None),
    }
}

fn unmeasured(query: &Query<'_>, countries: &[Country], hour: i32) -> board::Board {
    if query.timeline.at(hour).is_none() {
        return board::Board { rows: Vec::new(), missing: Some(query.timeline.reason(hour)) };
    }
    let rows = countries
        .iter()
        .filter(|country| query.holders.at(*country, hour).is_some())
        .map(|country| board::Row { tag: query.holders.name(country, hour), country: country.clone(), cells: Vec::new() })
        .collect();
    board::Board { rows, missing: None }
}

fn unavailable(key: ScoreboardColumnKey) -> String {
    let def = def_of(key);
    match def.measure.map(declared) {
        Some(Err(reason)) => reason,
        _ => def.reason.unwrap_or("This column has no measure").to_owned(),
    }
}
