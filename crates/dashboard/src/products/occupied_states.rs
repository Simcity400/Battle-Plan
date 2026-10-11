//! Occupied states, as recorded, over a whole branch. Daily reads are dated
//! separately from equal-value runs; neither resumes after a gap without a read.
//! The branch is scanned once; `occupied-states-manifest` sends its replay-block
//! windows, laws and warnings, and `occupied-states` one window's values as
//! changes, so no reply grows with the campaign.
//!
//! Resistance stores the occupied original tag. Garrisons iterate controlled
//! states with a positive occupied link and active occupation.
//! Controller != owner is not this test: annexed non-core states can still
//! require occupation.
use super::{
    BranchParams, LoadContext, Product,
    country_groups::HourSpan,
    fixed5_runs::{Fixed5State, NativeFixed5Run, push_fixed5},
    recorded_rows,
};
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
pub struct OccupiedStatesManifest;

/// One window of the branch: the replay block at `window` in the manifest's `windows`.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccupiedStatesParams {
    pub branch_id: String,
    pub window: usize,
}

/// A state's values at a read, held until they change, as the scan keeps them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OccupiedValues {
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub owner: Option<String>,
    pub occupier: Option<String>,
    /// Original occupied country, which can differ from today's legal owner.
    pub occupied: Option<String>,
    /// Stored fixed5 percentage units. Speeds are percentage points/day.
    pub resistance_raw: i64,
    pub compliance_raw: i64,
    pub resistance_speed_raw: i64,
    pub compliance_speed_raw: i64,
    /// Native displayed strength fraction, signed fixed5 (100000 is full).
    pub garrison_strength_raw: Option<i64>,
    // Recorded required manpower.
    pub required_manpower: Option<i32>,
    /// Recorded required template amount, not garrison strength.
    pub garrison_required_raw: Option<i64>,
    /// Requirement failure makes displayed strength zero.
    pub garrison_alert: bool,
    /// Below the content threshold, only when displayed strength is established.
    pub garrison_red: Option<bool>,
    pub assigned_manpower: Option<i32>,
    pub garrison_missing: Option<String>,
    /// Effective native law, using override/group/default/fallback precedence.
    pub law: Option<String>,
    pub law_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OccupiedRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: OccupiedValues,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StateRuns {
    pub state: u32,
    pub runs: Vec<OccupiedRun>,
}

/// The branch's occupied states as scanned: each state's equal-value runs.
pub(super) struct OccupiedScan {
    pub states: Vec<StateRuns>,
    pub observed: Vec<OccupiedObservation>,
    pub laws: Vec<OccupationLawInfo>,
    pub warnings: Vec<String>,
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

/// A state's values in one window, each as `[hour, value]` changes. Every span
/// the state is occupied restates each value at its first hour, so none carries
/// across a gap or from another window. Text values index the reply's `texts`.
#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedSeries {
    pub state: u32,
    /// Stored fixed5 percentage units, one run per span the state is occupied.
    pub resistance: Vec<NativeFixed5Run>,
    pub compliance: Vec<NativeFixed5Run>,
    /// Percentage points/day, over the same spans.
    pub resistance_speed: Vec<NativeFixed5Run>,
    pub compliance_speed: Vec<NativeFixed5Run>,
    pub name: Vec<(i32, Option<usize>)>,
    pub name_missing: Vec<(i32, Option<usize>)>,
    pub owner: Vec<(i32, Option<usize>)>,
    pub occupier: Vec<(i32, Option<usize>)>,
    /// Original occupied country, which can differ from today's legal owner.
    pub occupied: Vec<(i32, Option<usize>)>,
    /// Native displayed strength fraction, signed fixed5 (100000 is full).
    pub garrison_strength_raw: Vec<(i32, Option<String>)>,
    pub required_manpower: Vec<(i32, Option<i32>)>,
    /// Recorded required template amount, not garrison strength.
    pub garrison_required_raw: Vec<(i32, Option<String>)>,
    /// Native requirement failure; its strength getter returns zero.
    pub garrison_alert: Vec<(i32, bool)>,
    /// Below the content threshold, only when displayed strength is established.
    pub garrison_red: Vec<(i32, Option<bool>)>,
    pub assigned_manpower: Vec<(i32, Option<i32>)>,
    pub garrison_missing: Vec<(i32, Option<usize>)>,
    /// Effective native law token, using override/group/default/fallback precedence.
    pub law: Vec<(i32, Option<usize>)>,
    pub law_missing: Vec<(i32, Option<usize>)>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedStatesReply {
    pub branch_id: String,
    pub window: usize,
    /// The window's hours, both included.
    pub interval: HourSpan,
    /// Each text the window's states show, once.
    pub texts: Vec<String>,
    /// Every state occupied at an hour of the window, by id.
    pub states: Vec<OccupiedSeries>,
    /// Includes known empty inventories, distinguishing them from gaps. Only one
    /// starting at the window's first hour can hold an earlier read.
    pub observed: Vec<OccupiedObservation>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedStatesManifestReply {
    pub branch_id: String,
    pub decimals: u32,
    /// The branch's replay blocks, each one `occupied-states` window.
    pub windows: Vec<HourSpan>,
    /// Every law a state holds in the branch.
    pub laws: Vec<OccupationLawInfo>,
    pub strength_missing: String,
    pub warnings: Vec<String>,
}

// The strength bar converts the displayed fraction to percent. It combines
// assigned and required manpower, local and required equipment, requirement
// failure, and content's GARRISON_STR_POW_{MANPOWER,EQUIPMENT}.
// The saved strength cache is not read by this function.
// StateGarrisons records the equipment operands.
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

/// No group means database fallback without validation; otherwise state and
/// group overrides bypass validation. Only the default path checks scope
/// availability before choosing the database fallback.
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
    // The occupier FROM points to occupied and occupied ROOT to occupier.
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
    // Skip availability when raw/canonical country identities match.
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
    // Occupation laws localise the token itself.
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
    pub states: OccupiedScan,
    pub trend: super::occupation_trend::History,
}

#[derive(Default)]
struct Cache {
    branch: Option<u64>,
    catalogue: Option<Arc<Catalogue>>,
    reply: Option<Arc<OccupationHistory>>,
}

/// The occupation products share the daily joins, decoder walk and cache.
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

/// The branch's replay blocks, the windows `occupied-states` answers.
fn windows(source: &Source, branch: u64) -> Vec<HourSpan> {
    source
        .replay_ref()
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .map(|block| HourSpan {
            first_hour: block.first_hour,
            last_hour: block.last_hour,
        })
        .collect()
}

/// Lists `value` where it changes, and again where a span starts.
fn change<T: PartialEq>(changes: &mut Vec<(i32, T)>, hour: i32, starts: bool, value: T) {
    if starts || changes.last().is_none_or(|(_, last)| *last != value) {
        changes.push((hour, value));
    }
}

/// The runs and observations holding any hour of `interval`, clipped to it: a
/// window states every value it shows, whatever the branch holds around it.
fn window_reply<'a>(
    scan: &'a OccupiedScan,
    branch_id: &str,
    window: usize,
    interval: HourSpan,
) -> OccupiedStatesReply {
    let clip =
        |first: i32, last: i32| (first.max(interval.first_hour), last.min(interval.last_hour));
    let mut texts = Vec::new();
    let mut indices = HashMap::new();
    let mut text = |value: &'a Option<String>| {
        value.as_ref().map(|value| {
            *indices.entry(value.as_str()).or_insert_with(|| {
                texts.push(value.clone());
                texts.len() - 1
            })
        })
    };
    let mut states = Vec::new();
    for scanned in &scan.states {
        let first = scanned
            .runs
            .partition_point(|run| run.last_hour < interval.first_hour);
        let last = scanned
            .runs
            .partition_point(|run| run.first_hour <= interval.last_hour);
        if first == last {
            continue;
        }
        let mut series = OccupiedSeries {
            state: scanned.state,
            ..Default::default()
        };
        let mut fixed5 = [Fixed5State::default(); 4];
        let mut end = None;
        for run in &scanned.runs[first..last] {
            let (from, to) = clip(run.first_hour, run.last_hour);
            let starts = end.and_then(|end: i32| end.checked_add(1)) != Some(from);
            end = Some(to);
            let value = &run.value;
            for ((runs, state), raw) in [
                &mut series.resistance,
                &mut series.compliance,
                &mut series.resistance_speed,
                &mut series.compliance_speed,
            ]
            .into_iter()
            .zip(&mut fixed5)
            .zip([
                value.resistance_raw,
                value.compliance_raw,
                value.resistance_speed_raw,
                value.compliance_speed_raw,
            ]) {
                push_fixed5(runs, state, from, Some(raw), None, false, starts);
                runs.last_mut().expect("pushed a run").last_hour = to;
            }
            change(&mut series.name, from, starts, text(&value.name));
            change(
                &mut series.name_missing,
                from,
                starts,
                text(&value.name_missing),
            );
            change(&mut series.owner, from, starts, text(&value.owner));
            change(&mut series.occupier, from, starts, text(&value.occupier));
            change(&mut series.occupied, from, starts, text(&value.occupied));
            change(
                &mut series.garrison_strength_raw,
                from,
                starts,
                value.garrison_strength_raw.map(|raw| raw.to_string()),
            );
            change(
                &mut series.required_manpower,
                from,
                starts,
                value.required_manpower,
            );
            change(
                &mut series.garrison_required_raw,
                from,
                starts,
                value.garrison_required_raw.map(|raw| raw.to_string()),
            );
            change(
                &mut series.garrison_alert,
                from,
                starts,
                value.garrison_alert,
            );
            change(&mut series.garrison_red, from, starts, value.garrison_red);
            change(
                &mut series.assigned_manpower,
                from,
                starts,
                value.assigned_manpower,
            );
            change(
                &mut series.garrison_missing,
                from,
                starts,
                text(&value.garrison_missing),
            );
            change(&mut series.law, from, starts, text(&value.law));
            change(
                &mut series.law_missing,
                from,
                starts,
                text(&value.law_missing),
            );
        }
        states.push(series);
    }
    let first = scan
        .observed
        .partition_point(|span| span.last_hour < interval.first_hour);
    let last = scan
        .observed
        .partition_point(|span| span.first_hour <= interval.last_hour);
    let observed = scan.observed[first..last]
        .iter()
        .map(|span| {
            let (first_hour, last_hour) = clip(span.first_hour, span.last_hour);
            OccupiedObservation {
                first_hour,
                last_hour,
                read_hour: span.read_hour,
                complete: span.complete,
            }
        })
        .collect();
    OccupiedStatesReply {
        branch_id: branch_id.to_owned(),
        window,
        interval,
        texts,
        states,
        observed,
        warnings: Vec::new(),
    }
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
            NativeFixed5Run::decl(),
            OccupiedSeries::decl(),
            OccupiedObservation::decl(),
            HourSpan::decl(),
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
        let interval = windows(source, branch)
            .get(params.window)
            .cloned()
            .ok_or_else(|| crate::invalid("Occupied states window not in this recording"))?;
        let scanned = history(source, branch, context, cancelled)?;
        let reply = window_reply(&scanned.states, &params.branch_id, params.window, interval);
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

impl Product for OccupiedStatesManifest {
    const KEY: &'static str = "occupied-states-manifest";
    const CONTENT_DEPENDENT: bool = true;
    type Params = BranchParams;
    type Reply = OccupiedStatesManifestReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
    fn types() -> Vec<String> {
        vec![
            BranchParams::decl(),
            HourSpan::decl(),
            OccupationLawInfo::decl(),
            OccupiedStatesManifestReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params.branch();
        let scanned = history(source, branch, context, cancelled)?;
        let reply = OccupiedStatesManifestReply {
            branch_id: params.branch_id.clone(),
            decimals: 5,
            windows: windows(source, branch),
            laws: scanned.states.laws.clone(),
            strength_missing: STRENGTH_MISSING.into(),
            warnings: scanned.states.warnings.clone(),
        };
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;

mod strength;
