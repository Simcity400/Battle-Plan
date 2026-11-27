
use super::{Country, IDENTITY, Tags, agency_content::{self, AgencyContent}, branch, missing};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour, Owner, Read}}};
use replay_data::rows::{AgencyUpgradeLevelRow, IntelligenceAgencyRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct AgencyComparison;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyComparisonParams {
    pub branch_id: String,
    #[doc = " Countries whose agency is not created yet are listed too."]
    #[serde(default)]
    pub include_uncreated: bool,
    #[doc = " Only the first agencies by this measure, largest first; every agency, by country id, when none."]
    #[serde(default)]
    pub top: Option<AgencyTop>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AgencyRank {
    Defense,
    Active,
    Maximum,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyTop {
    pub count: u32,
    pub by: AgencyRank,
}

fn measure(agency: &Agency, by: AgencyRank) -> i64 {
    match by {
        AgencyRank::Defense => agency.defense_raw.parse().unwrap_or(i64::MIN),
        AgencyRank::Active => i64::from(agency.active_count),
        AgencyRank::Maximum => i64::from(agency.maximum),
    }
}

pub fn ranked(mut agencies: Vec<Agency>, top: Option<AgencyTop>) -> Vec<Agency> {
    let Some(top) = top else { return agencies };
    agencies.sort_by(|a, b| measure(b, top.by).cmp(&measure(a, top.by)).then(a.country.id.cmp(&b.country.id)));
    agencies.truncate(usize::try_from(top.count).unwrap_or(usize::MAX));
    agencies
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct UpgradeItem {
    pub ordinal: u32,
    #[doc = " The branch's and the definition's recorded tokens: a token alone is ambiguous."]
    pub branch: Option<String>,
    pub token: Option<String>,
    #[doc = " Signed, as stored."]
    pub level: i32,
    pub name: Option<String>,
    #[doc = " Why the name is not shown: the identity refused, a token not recorded, or the content."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Upgrades {
    #[doc = " Why the list cannot be summed: an entry not authenticated, or ordinals missing or repeated."]
    pub unavailable: Option<String>,
    #[doc = " The sum of each level at or above zero, as a decimal string; none where `unavailable`."]
    pub nonnegative_level_sum: Option<String>,
    pub items: Vec<UpgradeItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Agency {
    pub country: Country,
    pub created: bool,
    #[doc = " A created agency's stored name (even empty); the content's default for one not created."]
    pub name: Option<String>,
    pub name_missing: Option<String>,
    #[doc = " Fixed5, as recorded."]
    pub defense_raw: String,
    #[doc = " The defence line as the agency screen shows it, and its number alone."]
    pub defense_text: Option<String>,
    pub defense_value: Option<String>,
    pub active_count: i32,
    #[doc = " The maximum the agency screen shows, as recorded."]
    pub maximum: i32,
    #[doc = " None where acquired upgrades were not recorded at the hour (the reply says why)."]
    pub upgrades: Option<Upgrades>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AgencyComparisonReply {
    #[doc = " By country id."]
    pub agencies: Vec<Agency>,
    #[doc = " The defence column's label from the content; none where it cannot be read."]
    pub defense_label: Option<String>,
    pub upgrades_missing: Option<String>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AgencyComparisonReply);

pub fn upgrades(rows: &[&AgencyUpgradeLevelRow], tokens: &HashMap<u32, String>, content: Option<&AgencyContent>) -> Upgrades {
    let mut items: Vec<(UpgradeItem, bool)> = rows
        .iter()
        .map(|row| {
            let authenticated = row.identity_status == engine_native_intelligence::agency_upgrade_levels::AUTHENTICATED;
            let branch = row.branch_token.and_then(|token| tokens.get(&token).cloned());
            let token = row.upgrade_token.and_then(|token| tokens.get(&token).cloned());
            let name = match (authenticated, content, branch.as_deref(), token.as_deref()) {
                (false, ..) => Err(format!("Acquired upgrade identity refused: {}", engine_native_intelligence::agency_upgrade_levels::status_name(row.identity_status))),
                (true, Some(content), Some(branch), Some(token)) => content.upgrade_name(branch, token),
                (true, None, ..) => Err("Acquired upgrade content unavailable".to_owned()),
                _ => Err("Acquired upgrade branch or definition token not recorded".to_owned()),
            };
            (UpgradeItem { ordinal: row.ordinal, branch, token, level: row.level, name: name.as_ref().ok().cloned(), missing: name.err() }, authenticated)
        })
        .collect();
    items.sort_by_key(|(item, _)| item.ordinal);
    let unavailable = if items.iter().any(|(_, authenticated)| !authenticated) {
        Some("Acquired agency levels contain unauthenticated identities".to_owned())
    } else if items.iter().enumerate().any(|(at, (item, _))| item.ordinal as usize != at) {
        Some("Acquired agency list has missing or repeated ordinals".to_owned())
    } else {
        None
    };
    let sum = unavailable.is_none().then(|| items.iter().map(|(item, _)| item.level.max(0) as u128).sum::<u128>().to_string());
    Upgrades { unavailable, nonnegative_level_sum: sum, items: items.into_iter().map(|(item, _)| item).collect() }
}

fn stored_name(row: &IntelligenceAgencyRow) -> Option<String> {
    std::str::from_utf8(content_reader_parser::parser::c_string(&row.name)).ok().and_then(|name| dashboard_base::catalogue::plain_text(name).ok())
}

pub fn agencies(frame: &DecodedFrame, include_uncreated: bool, content: Result<&AgencyContent, &str>, warnings: &mut Vec<String>) -> io::Result<(Vec<Agency>, Option<String>)> {
    let tags = Tags::read(frame)?;
    let rows = frame.complete::<tables::IntelligenceAgencies>()?;
    let tokens: HashMap<u32, String> = frame
        .complete::<tables::LexicalTokens>()
        .map(|rows| rows.iter().filter_map(|row| Some((row.token, std::str::from_utf8(&row.literal).ok().filter(|text| !text.is_empty())?.to_owned()))).collect())
        .unwrap_or_default();
    let levels = frame.complete::<tables::AgencyUpgradeLevels>().ok();
    let upgrades_missing = levels.is_none().then(|| missing(frame, TableId::AgencyUpgradeLevels)).flatten();
    let mut by_country: HashMap<u32, Vec<&AgencyUpgradeLevelRow>> = HashMap::new();
    for row in levels.into_iter().flatten() {
        by_country.entry(row.country).or_default().push(row);
    }
    let content = content.ok();
    let mut unnamed = 0usize;
    let mut refusals: BTreeMap<String, usize> = BTreeMap::new();
    let agencies: Vec<Agency> = rows
        .iter()
        .filter(|row| include_uncreated || row.created != 0)
        .map(|row| {
            let created = row.created != 0;
            let name: Result<String, String> = if created {
                stored_name(row).ok_or_else(|| "The stored agency name has bytes that are not text".to_owned())
            } else {
                content.map_or_else(|| Err("Game content is not read".to_owned()), |content| content.default_name.clone())
            };
            if created && name.is_err() {
                unnamed += 1;
            }
            let (text, value) = content.map_or_else(|| (Err(String::new()), Err(String::new())), |content| content.defense(row.defense_raw));
            let upgrades = levels.map(|_| upgrades(by_country.get(&row.country).map(Vec::as_slice).unwrap_or_default(), &tokens, content));
            for item in upgrades.iter().flat_map(|upgrades| &upgrades.items) {
                if let Some(why) = &item.missing {
                    *refusals.entry(why.clone()).or_default() += 1;
                }
            }
            Agency {
                country: Country::of(&tags, row.country),
                created,
                name: name.as_ref().ok().cloned(),
                name_missing: name.err(),
                defense_raw: row.defense_raw.to_string(),
                defense_text: text.ok(),
                defense_value: value.ok(),
                active_count: row.active_count,
                maximum: row.unknown_f0 as i32,
                upgrades,
            }
        })
        .collect();
    if unnamed != 0 {
        warnings.push(format!("{unnamed} created agencies have stored names that are not text; their names are unknown."));
    }
    warnings.extend(refusals.into_iter().map(|(why, count)| format!("{why} ({count} acquired upgrades).")));
    Ok((agencies, upgrades_missing))
}

impl AtHour for AgencyComparison {
    const KEY: &'static str = "agency-comparison";
    const TITLE: &'static str = "Agency comparison";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AgencyComparisonParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, includeUncreated, top, "top.count", "top.by"];
    type Reply = AgencyComparisonReply;
    type Context = Result<Arc<AgencyContent>, String>;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AgencyComparisonParams::decl(), AgencyTop::decl(), AgencyRank::decl(), Country::decl(), UpgradeItem::decl(), Upgrades::decl(), Agency::decl(), AgencyComparisonReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }



    fn projection(_params: &Self::Params, table: TableId, _plan: &[(Vec<Read>, i32)], _context: &Self::Context) -> Option<Vec<usize>> {
        match table {
            TableId::IntelligenceAgencies => Some(dashboard_base::products::at_hour::field_columns(table, &["created", "name", "defense_raw", "active_count", "unknown_f0"])),
            TableId::Countries => Some(dashboard_base::products::at_hour::field_columns(table, &["canonical_slot", "tag"])),
            _ => None,
        }
    }

    fn plan_changes(stage: usize, edits: &dashboard_base::products::at_hour::Edits, _held: &DecodedFrame, _at: &dashboard_base::products::at_hour::ReadsAt<'_, Self::Context>) -> bool {
        stage == 1 && edits.fields(TableId::AgencyUpgradeLevels, u64::MAX)
    }

    fn advance(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, edits: &dashboard_base::products::at_hour::Edits, _live: &dashboard_follow::live::Live, data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        let table = TableId::IntelligenceAgencies;
        if params.top.is_some() { return Ok(false); }
        if !edits.only_columns(table, dashboard_base::products::at_hour::field_mask(table, &["defense_raw", "active_count", "unknown_f0"])) { return Ok(false); }
        let rows = frame.complete::<tables::IntelligenceAgencies>()?;
        let Some(agencies) = data["agencies"].as_array_mut() else { return Ok(false) };
        for &key in edits.tables.get(&table).into_iter().flat_map(|e| e.keys()) {
            let raw = key as u32;
            let Ok(i) = agencies.binary_search_by_key(&u64::from(raw), |a| a["country"]["id"].as_u64().unwrap()) else { continue };
            let row = &rows[rows.binary_search_by_key(&raw, |r| r.country).unwrap()];
            let a = &mut agencies[i];
            if a["defenseRaw"].as_str() != Some(&row.defense_raw.to_string()) {
                let (text, value) = at.context.as_ref().ok().map(|c| c.defense(row.defense_raw)).map_or((None, None), |(t,v)| (t.ok(),v.ok()));
                a["defenseRaw"] = row.defense_raw.to_string().into();
                a["defenseText"] = text.into();
                a["defenseValue"] = value.into();
            }
            a["activeCount"] = row.active_count.into();
            a["maximum"] = (row.unknown_f0 as i32).into();
        }
        Ok(true)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::IntelligenceAgencies, TableId::AgencyUpgradeLevels, TableId::LexicalTokens, IDENTITY[0], IDENTITY[1]]
    }

    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => vec![Read::all(TableId::IntelligenceAgencies), Read::all(TableId::AgencyUpgradeLevels), Read::all(IDENTITY[0]), Read::all(IDENTITY[1])],
            1 => {
                let tokens: BTreeSet<u32> = held
                    .complete::<tables::AgencyUpgradeLevels>()
                    .map(|rows| rows.iter().flat_map(|row| [row.branch_token, row.upgrade_token]).flatten().collect())
                    .unwrap_or_default();
                vec![Read::these(TableId::LexicalTokens, tokens.into_iter().map(u128::from))]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(agency_content::content(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let content = at.context.as_deref().map_err(String::as_str);
        let mut warnings = match content {
            Ok(content) => content.warnings.clone(),
            Err(why) => vec![why.to_owned()],
        };
        let defense_label = content.ok().and_then(|content| content.defense_label().ok().map(str::to_owned));
        Ok(match agencies(frame, params.include_uncreated, content, &mut warnings) {
            Ok((agencies, upgrades_missing)) => AgencyComparisonReply { agencies: ranked(agencies, params.top), defense_label, upgrades_missing, missing: None, warnings },
            Err(error) => AgencyComparisonReply { agencies: Vec::new(), defense_label, upgrades_missing: None, missing: Some(error.to_string()), warnings },
        })
    }
}
