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
    /// Native displayed strength fraction, signed fixed5 (100000 is full).
    pub garrison_strength_raw: Option<String>,
    // P: required manpower at +0x88, requirement writer 140FE78B0.
    pub required_manpower: Option<i32>,
    // P: 140FE78B0 writes the required template amount at +0xF8.
    /// Recorded required template amount, not garrison strength.
    pub garrison_required_raw: Option<String>,
    /// +0x60: native requirement failure; its strength getter returns zero.
    pub garrison_alert: bool,
    /// Below the content threshold, only when displayed strength is established.
    pub garrison_red: Option<bool>,
    pub assigned_manpower: Option<i32>,
    pub garrison_missing: Option<String>,
    /// Effective native law, using override/group/default/fallback precedence.
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
// by this function. StateGarrisons records the equipment getter's operands.
const STRENGTH_MISSING: &str = "Garrison strength needs complete equipment operands, assigned manpower and the recorded content strength factors. The row names any missing input.";
const GROUP_LAW_MISSING: &str = "The law set for this occupied country is not recorded. It takes precedence over the occupier's default, so the state's law cannot be established.";

use scripted_text::StateNames;

fn name_of(
    state: &StateRow,
    cat: Option<&Catalogue>,
    names: &StateNames,
) -> Result<String, String> {
    let cat =
        cat.ok_or_else(|| "State names need the recorded content stack and language".to_owned())?;
    scripted_text::state_display_name(
        state,
        cat,
        Ok(names),
        scripted_text::StateNameUsage::Occupation,
    )
}

/// 140F84BB0 / 140FE2FE0: no group means database fallback, without validation;
/// otherwise state and group overrides bypass validation. Only the default path
/// checks IsValidForScope@140A665A0 before choosing the database fallback.
enum GroupLaw<'a> {
    Missing,
    Law(Option<&'a [u8]>),
    Unknown(String),
}

struct LawInputs<'a> {
    state: Option<&'a [u8]>,
    key_resolved: bool,
    overrides_complete: bool,
    group: GroupLaw<'a>,
    default: Option<(bool, &'a [u8])>,
    fallback: Result<&'a [u8], String>,
}

fn law_for(
    input: LawInputs<'_>,
    valid: impl FnOnce(&str) -> Result<bool, String>,
) -> (Option<String>, Option<String>) {
    let resolve = || -> Result<String, String> {
        let token = |bytes: &[u8]| {
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|_| "The recorded occupation-law key is not UTF-8".to_owned())
        };
        if !input.key_resolved {
            return Err("The read's controller or occupied-country link is unresolved; the state's occupation-law key cannot be established".into());
        }
        if matches!(input.group, GroupLaw::Missing) {
            return token(input.fallback?);
        }
        if let Some(bytes) = input.state {
            return token(bytes);
        }
        if !input.overrides_complete {
            return Err("The occupation-law overrides are incomplete; this state's override may not have been recorded".into());
        }
        match input.group {
            GroupLaw::Law(Some(bytes)) => return token(bytes),
            GroupLaw::Unknown(reason) => return Err(reason),
            GroupLaw::Law(None) => {}
            GroupLaw::Missing => unreachable!(),
        }
        let candidate = match input.default {
            Some((true, bytes)) => token(bytes)?,
            Some((false, _)) => token(input.fallback.clone()?)?,
            None => return Err("The occupier's default law is missing".into()),
        };
        if valid(&candidate)? {
            Ok(candidate)
        } else {
            token(input.fallback?)
        }
    };
    match resolve() {
        Ok(law) => (Some(law), None),
        Err(reason) => (None, Some(reason)),
    }
}

fn occupation_law_valid(
    world: &crate::triggers::World<'_>,
    law: &installed_content::occupation_laws::Law,
    controller: u32,
    occupied: u32,
) -> Result<bool, String> {
    use scripted_text::{EventObject, EventScope, EventScopes, Link};
    // 140FE2FE0 sets occupier FROM to occupied and occupied ROOT to occupier.
    // Each fresh scope otherwise keeps ROOT/FROM/PREV pointing to itself.
    let mut occupier_scope = EventScope::of(EventObject::Country(controller));
    occupier_scope.from = Link::Scope(1);
    let mut occupied_scope = EventScope::of(EventObject::Country(occupied));
    occupied_scope.root = Link::Scope(0);
    let graph = std::rc::Rc::new(std::cell::RefCell::new(EventScopes::new(vec![
        occupier_scope,
        occupied_scope,
    ])));
    let evaluator = crate::triggers::Evaluator::new(world, None).with_event_scopes(graph);
    let scope = evaluator.event_scopes(0)?;
    let valid = evaluator.evaluate(&law.visible, scope);
    // 140A665A0 skips available when raw/canonical country identities match.
    valid
        .and(|| {
            if controller == occupied {
                crate::triggers::Eval::True
            } else {
                evaluator.evaluate(&law.available, scope)
            }
        })
        .decided()
}

fn occupied(row: &StateResistanceRow) -> bool {
    row.active != 0 && (row.country as i32) > 0
}

#[derive(Default)]
struct CountryIndex {
    identities: HashMap<u32, u32>,
    names: HashMap<super::country_history::Identity, String>,
    name_refusals: HashMap<super::country_history::Identity, &'static str>,
}

fn country_index(
    countries: &[CountryRow],
    links: &[CountryLinkRow],
    refused: &mut Refusals,
) -> CountryIndex {
    // Use the shared ambiguity rule before naming any slot: neither of two
    // Countries rows wins. The same index serves both occupation consumers.
    let unique = super::country_history::unique_by_slot(countries, links, |row| row.id);
    let mut result = CountryIndex::default();
    let mut named = std::collections::HashSet::new();
    for row in countries {
        let Ok(slot) = mapped_slot(row, links) else {
            refused.note(
                "CountryLinks identity is unresolved",
                format!("country {}", row.id),
            );
            continue;
        };
        if unique.rows.get(&slot).is_none_or(Option::is_none) {
            refused.note(
                "Countries canonical slot is ambiguous",
                format!("slot {slot}, country {}", row.id),
            );
            continue;
        }
        result.identities.insert(slot, row.id);
        let Some(tag) = std::str::from_utf8(&row.tag)
            .ok()
            .filter(|tag| !tag.is_empty() && !tag.contains('\0'))
        else {
            let reason = "country tag is empty, contains NUL or is not UTF-8";
            result.name_refusals.insert((slot, row.id), reason);
            refused.note(reason, format!("country {}", row.id));
            continue;
        };
        // Preserve tagged_countries' first valid holder for a repeated tag.
        if named.insert(tag) {
            result.names.insert((slot, row.id), tag.to_owned());
        } else {
            let reason = "country tag repeats an earlier country";
            result.name_refusals.insert((slot, row.id), reason);
            refused.note(reason, format!("country {}", row.id));
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

pub(crate) fn law_content(
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
#[cfg(test)]
pub(super) use scan::scan_history;
mod trend;

pub(super) struct OccupationHistory {
    pub states: OccupiedStatesReply,
    pub trend: super::occupation_trend::History,
}

#[derive(Default)]
struct Cache {
    branch: Option<u64>,
    catalogue: Option<Arc<Catalogue>>,
    reply: Option<Arc<OccupationHistory>>,
}

/// Both occupation products share the daily joins, decoder walk and cache.
pub(super) fn history(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Arc<OccupationHistory>> {
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
    let mut reply = scan::scan_history(&source.replay(), branch, cat.as_deref(), cancelled)?;
    reply.states.warnings.extend(content_warning);
    let reply = Arc::new(reply);
    *cache = Cache {
        branch: Some(branch),
        catalogue: cat,
        reply: Some(reply.clone()),
    };
    Ok(reply)
}

impl Product for OccupiedStates {
    const KEY: &'static str = "occupied-states";
    const CONTENT_DEPENDENT: bool = true;
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
        Ok(history(source, branch, context, cancelled)?.states.clone())
    }
}

#[cfg(test)]
mod tests;

mod strength;
