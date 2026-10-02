//! Occupied states, as recorded, over a whole branch. Daily reads are dated
//! separately from equal-value runs; neither resumes after a gap without a read.
//!
//! P: StartResistance 140F85ED0 stores the OCCUPIED original tag at
//! CResistance+0x50, not the occupier. RefreshGarrisons 140FE9420 iterates
//! controlled states with a positive occupied link and CheckActivation(...,1)
//! (140F86330: stored +0x208). Controller != owner is NOT this test: annexed
//! non-core states can still require occupation.
use super::{LoadContext, Product, recorded_rows};
use crate::{
    Source,
    catalogue::{self, Catalogue, Refusals, View},
    metadata::{link_slot, mapped_slot, slot_read, validate_campaign},
    scripted_text,
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{CountryLinkRow, CountryRow, StateResistanceRow, StateRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct OccupiedStates;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccupiedStatesParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedValues {
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub owner: Option<String>,
    pub occupier: Option<String>,
    /// Original occupied country, which can differ from today's legal owner.
    pub occupied: Option<String>,
    // P: 140F82ED0 adds speeds to percentage-unit levels; 140F86E10 writes
    // both signed fixed5 speeds. DailyUpdate 1401D4390 -> 1406DAAD0 ->
    // 140FE1EC0 calls it daily: speeds are percentage points/day.
    /// Stored fixed5 percentage units. Speeds are percentage points/day.
    pub resistance_raw: String,
    pub compliance_raw: String,
    pub resistance_speed_raw: String,
    pub compliance_speed_raw: String,
    /// Native displayed strength in fixed5 percentage units, when established.
    pub garrison_strength_raw: Option<String>,
    // P: required manpower at +0x88, requirement writer 140FE78B0.
    pub required_manpower: Option<i32>,
    // P: 140FE78B0 writes the required template amount at +0xF8.
    /// Recorded required template amount, not garrison strength.
    pub garrison_required_raw: Option<String>,
    /// +0x60: native requirement failure, displayed as a red exclamation mark.
    pub garrison_alert: bool,
    /// Below the content threshold, only when displayed strength is established.
    pub garrison_red: Option<bool>,
    pub assigned_manpower: Option<i32>,
    pub garrison_missing: Option<String>,
    /// Known explicit per-state override only. See `law_for`.
    pub law: Option<String>,
    pub law_missing: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: OccupiedValues,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedSeries {
    pub state: u32,
    pub runs: Vec<OccupiedRun>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedObservation {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Read supplying resistance/compliance and the daily occupation tables.
    pub read_hour: i32,
    /// False means known rows are shown but the inventory is incomplete.
    pub complete: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupationLawInfo {
    pub token: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedStatesReply {
    pub branch_id: String,
    pub decimals: u32,
    pub states: Vec<OccupiedSeries>,
    /// Includes known empty inventories, distinguishing them from gaps.
    pub observed: Vec<OccupiedObservation>,
    pub laws: Vec<OccupationLawInfo>,
    pub strength_missing: String,
    pub warnings: Vec<String>,
}

// P: UI 14155DAE0 -> 140FE01E0(garrison, out, true); 14155CC50 converts
// its fraction to the strength bar's percent. It combines assigned manpower (+0x68), required manpower
// (+0x88), local/required equipment (+0x90/+0xD0), +0x60, and content's
// GARRISON_STR_POW_{MANPOWER,EQUIPMENT}. StateGarrisons +0xF0 is NOT read
// by this function. The four requested tables lack those equipment inputs.
const STRENGTH_MISSING: &str = "The game's garrison strength needs unrecorded equipment allocation and requirements (+0x90/+0xD0, 140B8A4E0). Assigned manpower and the content strength factors are available; the stored garrison ratio cannot replace the equipment input.";
const GROUP_LAW_MISSING: &str = "The law set for this occupied country is not recorded. It takes precedence over the occupier's default, so the state's law cannot be established.";

use scripted_text::StateNames;

fn name_of(
    state: &StateRow,
    cat: Option<&Catalogue>,
    names: &StateNames,
) -> Result<String, String> {
    let cat =
        cat.ok_or_else(|| "State names need the recorded content stack and language".to_owned())?;
    scripted_text::state_name_from(
        state,
        |bytes, what| {
            scripted_text::stored_text(&cat.localisation, &cat.recorded_language, bytes, what)
        },
        |id| {
            let key = names
                .get(&id)
                .ok_or_else(|| "No recorded-content state template".to_owned())?
                .as_ref()
                .map_err(Clone::clone)?;
            cat.localisation
                .localise(key.as_bytes())
                .map_err(|refusal| refusal.to_string())
        },
    )
    .and_then(|text| catalogue::plain_text(&text))
}

/// P: GetOccupationLaw 140F84BB0 -> 140FE2FE0: state override (+0xB0,
/// key CState+0x58), else group law (+0xA0), else occupier default (+0x118),
/// else database fallback (+0x48). Only the default branch validates scope
/// (140A665A0). Explicit overrides bypass it. A raw key 0 is a state key,
/// NOT the unrecorded +0xA0 pointer (engine::ccountry_occupation_data).
fn law_for(
    override_name: Option<&[u8]>,
    key_resolved: bool,
    complete: bool,
    default: Option<(bool, &[u8])>,
) -> (Option<String>, Option<String>) {
    if let Some(bytes) = override_name {
        return match std::str::from_utf8(bytes) {
            Ok(token) => (Some(token.into()), None),
            Err(_) => (
                None,
                Some("The recorded state override's law key is not UTF-8".into()),
            ),
        };
    }
    let reason = if !key_resolved {
        "The read's controller or occupied-country link is unresolved; the state's occupation-law key cannot be established".to_owned()
    } else if !complete {
        "The occupation-law overrides are incomplete; this state's override may not have been recorded".to_owned()
    } else {
        format!(
            "{GROUP_LAW_MISSING} {}",
            match default {
                None => "The occupier's default law is also missing.",
                Some((false, _)) => "The occupier's recorded default law was absent.",
                Some((true, _)) =>
                    "The recorded occupier default is known but has lower precedence.",
            }
        )
    };
    (None, Some(reason))
}

fn occupied(row: &StateResistanceRow) -> bool {
    row.active != 0 && (row.country as i32) > 0
}

fn tags<'a>(
    countries: &'a [CountryRow],
    links: &[CountryLinkRow],
    refused: &mut Refusals,
) -> HashMap<u32, &'a str> {
    let mut result = HashMap::with_capacity(countries.len());
    for row in countries {
        // Same identity guard tagged_country uses, indexed once rather than
        // scanning all countries once for each state or raw link.
        match mapped_slot(row, links).ok().zip(
            std::str::from_utf8(&row.tag)
                .ok()
                .filter(|tag| !tag.is_empty() && !tag.contains('\0')),
        ) {
            Some((slot, tag)) => {
                result.entry(slot).or_insert(tag);
            }
            None => refused.note(
                "country tag or CountryLinks identity is unresolved",
                format!("country {}", row.id),
            ),
        }
    }
    result
}

fn append(runs: &mut Vec<OccupiedRun>, hour: i32, value: OccupiedValues) {
    if let Some(last) = runs.last_mut()
        && last.last_hour.checked_add(1) == Some(hour)
        && last.value == value
    {
        last.last_hour = hour;
    } else {
        runs.push(OccupiedRun {
            first_hour: hour,
            last_hour: hour,
            value,
        });
    }
}

fn law_content(
    cat: &Catalogue,
    laws: &installed_content::occupation_laws::Laws,
    token: &str,
) -> OccupationLawInfo {
    // P: COccupationLaw ctor 140A63E00 localises the token itself.
    let name = cat.localised(token, View::Plain);
    let mut identity = format!("law {token}");
    let icon = (|| -> Result<String, String> {
        let law = laws
            .find(token)?
            .ok_or_else(|| "Occupation law has no content declaration".to_owned())?;
        identity = format!("law {token}, sprite {}, frame {}", law.sprite, law.frame);
        let sprite = cat
            .sprites
            .find(&law.sprite)?
            .ok_or_else(|| "Occupation-law sprite not in the recorded content stack".to_owned())?;
        let frame = u32::try_from(law.frame)
            .ok()
            .and_then(|frame| frame.checked_sub(1))
            .ok_or_else(|| "Occupation-law icon frame is not positive".to_owned())?;
        let decode = cat.gui_decode.clone()?;
        catalogue::sprite_frame_png(&cat.content.layers, sprite, decode, frame, 32)
            .map_err(|e| e.to_string())
    })();
    let missing = icon.as_ref().err().cloned();
    if let Some(reason) = &missing {
        cat.refuse(reason, &identity);
    }
    OccupationLawInfo {
        token: token.into(),
        name,
        icon: icon.ok(),
        missing,
    }
}

fn has_rows(frame: &DecodedFrame, table: TableId) -> bool {
    !matches!(&*frame.tables[table.index()], TableView::Unavailable { .. })
}

mod scan;
use scan::scan;

#[derive(Default)]
struct Cache {
    branch: Option<u64>,
    catalogue: Option<Arc<Catalogue>>,
    reply: Option<OccupiedStatesReply>,
}

impl Product for OccupiedStates {
    const KEY: &'static str = "occupied-states";
    type Params = OccupiedStatesParams;
    type Reply = OccupiedStatesReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid branch")
    }
    fn types() -> Vec<String> {
        vec![
            OccupiedStatesParams::decl(),
            OccupiedValues::decl(),
            OccupiedRun::decl(),
            OccupiedSeries::decl(),
            OccupiedObservation::decl(),
            OccupationLawInfo::decl(),
            OccupiedStatesReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse().expect("validated branch");
        let cat = context
            .content()
            .and_then(|options| catalogue::catalogue(source, options));
        let content_warning = cat
            .as_ref()
            .err()
            .map(|e| format!("State names and occupation law artwork unavailable: {e}"));
        let cat = cat.ok();
        let state = source.state::<Cache>();
        let mut cache = state.lock().unwrap_or_else(|e| e.into_inner());
        let same = match (&cache.catalogue, &cat) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if cache.branch == Some(branch)
            && same
            && let Some(reply) = &cache.reply
        {
            return Ok(reply.clone());
        }
        let mut reply = scan(&source.replay(), branch, cat.as_deref(), cancelled)?;
        reply.warnings.extend(content_warning);
        *cache = Cache {
            branch: Some(branch),
            catalogue: cat,
            reply: Some(reply.clone()),
        };
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
