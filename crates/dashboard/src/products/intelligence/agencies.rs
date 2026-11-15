//! Agency comparison: every country's intelligence agency at an hour as the agency screen shows it
//! (rows at an hour): its name, its defence line, its active and maximum operatives, and its
//! acquired upgrades with their levels.

use super::{Country, IDENTITY, Tags, agency_content::{self, AgencyContent}, branch, missing};
use crate::{
    Source,
    products::{
        LoadContext,
        at_hour::{At, AtHour, Owner, Read},
    },
};
use replay_data::rows::{AgencyUpgradeLevelRow, IntelligenceAgencyRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
use ts_rs::TS;

pub struct AgencyComparison;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgencyComparisonParams {
    pub branch_id: String,
    /// Countries whose agency is not created yet are listed too.
    #[serde(default)]
    pub include_uncreated: bool,
}

/// One entry of an agency's acquired upgrades, in the stored order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeItem {
    pub ordinal: u32,
    /// The branch's and the definition's recorded tokens: a token alone is ambiguous.
    pub branch: Option<String>,
    pub token: Option<String>,
    /// Signed, as stored.
    pub level: i32,
    pub name: Option<String>,
    /// Why the name is not shown: the identity refused, a token not recorded, or the content.
    pub missing: Option<String>,
}

/// An agency's acquired upgrades at the hour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Upgrades {
    /// Why the list cannot be summed: an entry not authenticated, or ordinals missing or repeated.
    pub unavailable: Option<String>,
    /// The sum of each level at or above zero, as a decimal string; none where `unavailable`.
    pub nonnegative_level_sum: Option<String>,
    pub items: Vec<UpgradeItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Agency {
    pub country: Country,
    pub created: bool,
    /// A created agency's stored name (even empty); the content's default for one not created.
    pub name: Option<String>,
    pub name_missing: Option<String>,
    /// Fixed5, as recorded.
    pub defense_raw: String,
    /// The defence line as the agency screen shows it, and its number alone.
    pub defense_text: Option<String>,
    pub defense_value: Option<String>,
    pub active_count: i32,
    /// The maximum the agency screen shows, as recorded.
    pub maximum: i32,
    /// None where acquired upgrades were not recorded at the hour (the reply says why).
    pub upgrades: Option<Upgrades>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgencyComparisonReply {
    /// By country id.
    pub agencies: Vec<Agency>,
    /// The defence column's label from the content; none where it cannot be read.
    pub defense_label: Option<String>,
    pub upgrades_missing: Option<String>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AgencyComparisonReply);

/// An agency's acquired upgrades from its rows, by the capture's rules: every entry kept, its
/// identity refused by name, its level summed only where every entry is authenticated and ordered.
pub fn upgrades(rows: &[&AgencyUpgradeLevelRow], tokens: &HashMap<u32, String>, content: Option<&AgencyContent>) -> Upgrades {
    let mut items: Vec<(UpgradeItem, bool)> = rows
        .iter()
        .map(|row| {
            let authenticated = row.identity_status == engine_native::agency_upgrade_levels::AUTHENTICATED;
            let branch = row.branch_token.and_then(|token| tokens.get(&token).cloned());
            let token = row.upgrade_token.and_then(|token| tokens.get(&token).cloned());
            let name = match (authenticated, content, branch.as_deref(), token.as_deref()) {
                (false, ..) => Err(format!("Acquired upgrade identity refused: {}", engine_native::agency_upgrade_levels::status_name(row.identity_status))),
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

/// A created agency's stored name as plain text; none where its bytes are not text.
fn stored_name(row: &IntelligenceAgencyRow) -> Option<String> {
    std::str::from_utf8(installed_content_core::parser::c_string(&row.name)).ok().and_then(|name| dashboard_base::catalogue::plain_text(name).ok())
}

/// The frame's agencies.
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
                // The recorded bits are shown as a signed count, as the screen does.
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
    type Reply = AgencyComparisonReply;
    type Context = Result<Arc<AgencyContent>, String>;

    fn types() -> Vec<String> {
        vec![AgencyComparisonParams::decl(), Country::decl(), UpgradeItem::decl(), Upgrades::decl(), Agency::decl(), AgencyComparisonReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::IntelligenceAgencies, TableId::AgencyUpgradeLevels, TableId::LexicalTokens, IDENTITY[0], IDENTITY[1]]
    }

    /// The agencies, their upgrades and the identity first; then the tokens the upgrades name.
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
            Ok((agencies, upgrades_missing)) => AgencyComparisonReply { agencies, defense_label, upgrades_missing, missing: None, warnings },
            Err(error) => AgencyComparisonReply { agencies: Vec::new(), defense_label, upgrades_missing: None, missing: Some(error.to_string()), warnings },
        })
    }
}
