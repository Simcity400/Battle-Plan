//! The occupied states at an hour, a rows-at-an-hour product: a row per state the game holds
//! occupied, with its names, owner, occupier and occupied country, its garrison and its effective
//! law (the structure); its resistance, compliance and their speeds (StateResistance by state),
//! its garrison's required amount and manpower (StateGarrisons), its assigned manpower (the sum
//! of its StateAssignedManpower rows) and its displayed strength (GarrisonStrengths, which the
//! recorder derives from the equipment operands no cell can read) are live cells.

use super::{
    Content, law::{self, GroupLaw, LawInputs, OccupationLawInfo}, occupied, slot,
};
use crate::{
    Source,
    catalogue::Refusals,
    products::{
        LoadContext,
        at_hour::{At, AtHour, Owner, Read},
        intelligence::{Tags, branch, missing},
    },
};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_text::metadata::recorded_rows;
use replay_data::rows::{
    CountryLinkRow, GarrisonEquipment, GarrisonStrengthRow, OccupationSettingsRow, Recorded, StateGarrisonRow, StateResistanceRow, StateRow, TableId, tables,
};
use replay_frame::frame::{DecodedFrame, HourFrame};
use serde::{Deserialize, Serialize};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
};
use ts_rs::TS;

/// What a reply reads besides the trigger world: the occupation tables and the identity.
pub const TABLES: [TableId; 10] = [
    TableId::StateResistance,
    TableId::States,
    TableId::StateGarrisons,
    TableId::GarrisonStrengths,
    TableId::StateAssignedManpower,
    TableId::StateOccupationLaws,
    TableId::OccupationSettings,
    TableId::LexicalTokens,
    TableId::Countries,
    TableId::CountryLinks,
];

/// The live cells of a row: its column and StateResistance's field.
pub const LIVE: [(&str, &str); 4] = [("resistance", "level"), ("compliance", "compliance"), ("resistanceSpeed", "speed"), ("complianceSpeed", "compliance_speed")];

/// A garrison's live cells: its column and StateGarrisons' field.
pub const GARRISON: [(&str, &str); 2] = [("garrison", "garrison_required"), ("requiredManpower", "required_manpower")];

/// The live cell of a state's assigned manpower: the sum of its StateAssignedManpower rows'
/// amounts, named only where it has rows ([`OccupiedState::assigned_known`]).
pub const ASSIGNED: &str = "assignedManpower";

/// The live cell of a garrison's displayed strength: GarrisonStrengths' `strength`, named only
/// where it is known and its equipment has recorded content.
pub const STRENGTH: &str = "garrisonStrength";

pub const STRENGTH_MISSING: &str = "Garrison strength needs complete equipment operands, assigned manpower and the recorded content strength factors. The row names any missing input.";
const GROUP_LAW_MISSING: &str = "The law set for this occupied country is not recorded. It takes precedence over the occupier's default, so the state's law cannot be established.";

pub struct OccupiedStates;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccupiedStatesParams {
    pub branch_id: String,
}

/// A state occupied at the hour. Its moving numbers are live cells of row `state`: `resistance`,
/// `compliance`, `resistanceSpeed`, `complianceSpeed` (fixed5 percentage units, speeds per day);
/// with a garrison row, `garrison` (the required template amount, fixed5; not the
/// strength) and `requiredManpower`; `assignedManpower` where the state has assignment rows; and
/// `garrisonStrength` (a signed fixed5 fraction, 100000 full) where it is known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedState {
    pub state: u32,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub owner: Option<String>,
    pub occupier: Option<String>,
    /// The original occupied country, which can differ from the legal owner.
    pub occupied: Option<String>,
    /// The sign of the required amount (-1, 0, 1), with a garrison row: none required shows no
    /// bar, below zero the game shows unmet. It flips far more rarely than the amount moves.
    pub garrison_required_sign: Option<i8>,
    /// The requirement failed: the game shows no strength.
    pub garrison_alert: bool,
    /// The assigned manpower is known: its cell, or zero where the state has no assignment row
    /// and so no cell.
    pub assigned_known: bool,
    pub garrison_missing: Option<String>,
    pub law: Option<String>,
    pub law_missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupiedStatesReply {
    /// Ascending by state.
    pub states: Vec<OccupiedState>,
    /// Every row of StateResistance was read and every occupier and occupied country named.
    pub complete: bool,
    /// Why no state can be shown at the hour.
    pub missing: Option<String>,
    /// The laws the states hold.
    pub laws: Vec<OccupationLawInfo>,
    pub strength_missing: String,
    /// The displayed strength below which a garrison shows red (fixed5); none where the content
    /// does not say.
    pub garrison_red_raw: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OccupiedStatesReply);

/// (controller slot, occupied slot, state).
type Key = (u32, u32, u32);

/// The hour's occupation tables indexed by resolved country slots.
pub(super) struct Joins<'f> {
    garrisons: HashMap<Key, &'f StateGarrisonRow>,
    overrides: HashMap<Key, &'f [u8]>,
    overrides_complete: bool,
    settings: HashMap<u32, &'f OccupationSettingsRow>,
    /// (controller, occupied) to the law set for the occupied country and the scope it names.
    groups: HashMap<(u32, u32), (Option<&'f [u8]>, Option<u32>)>,
    group_failures: HashMap<u32, String>,
    /// Each garrison key's assigned total and its rows' keys.
    assigned: HashMap<Key, (i32, Vec<u128>)>,
    unresolved_assigned: HashSet<u32>,
    assigned_complete: bool,
    /// Each garrison's derived strength, by its StateGarrisons key, while the table is read.
    strengths: HashMap<u128, &'f GarrisonStrengthRow>,
    strengths_missing: Option<String>,
}

impl<'f> Joins<'f> {
    pub(super) fn read(frame: &'f DecodedFrame, links: &[CountryLinkRow], refused: &mut Refusals) -> io::Result<Self> {
        let key = |controller, occupied, state| Some((slot(links, controller)?, slot(links, occupied)?, state));
        let mut garrisons = HashMap::new();
        for row in recorded_rows::<tables::StateGarrisons>(frame)? {
            match key(row.controller, row.occupied, row.state) {
                Some(key) => {
                    garrisons.insert(key, row);
                }
                None => refused.note("state garrisons name a country with no recorded country link", format!("state {}", row.state)),
            }
        }
        let mut overrides = HashMap::new();
        for row in recorded_rows::<tables::StateOccupationLaws>(frame)? {
            match key(row.controller, row.occupied, row.state) {
                Some(key) => {
                    overrides.insert(key, row.name.as_slice());
                }
                None => refused.note("state occupation laws name a country with no recorded country link", format!("state {}", row.state)),
            }
        }
        let mut settings = HashMap::new();
        let mut group_failures = HashMap::new();
        for row in recorded_rows::<tables::OccupationSettings>(frame)? {
            let Some(slot) = slot(links, row.country) else {
                refused.note("occupation settings name a country with no recorded country link", format!("raw link {}", row.country));
                continue;
            };
            if group_failures.contains_key(&slot) {
                continue;
            }
            if settings.insert(slot, row).is_some() {
                settings.remove(&slot);
                group_failures.insert(slot, "occupation settings hold two rows for one country".to_owned());
                refused.note("occupation settings hold two rows for one country", format!("slot {slot}"));
            }
        }
        let mut groups = HashMap::new();
        for (&controller, row) in &settings {
            let laws = match &row.group_laws {
                Recorded::NotRecorded => {
                    group_failures.insert(controller, GROUP_LAW_MISSING.to_owned());
                    continue;
                }
                Recorded::Value(None) => {
                    group_failures.insert(controller, "The occupied-country law inventory capture failed".into());
                    continue;
                }
                Recorded::Value(Some(laws)) => laws,
            };
            for group in laws {
                let Some(occupied) = slot(links, group.occupied) else {
                    group_failures.insert(controller, "The occupied-country law inventory has an unresolved country link".into());
                    continue;
                };
                if groups.insert((controller, occupied), (group.law.as_deref(), group.scope_country)).is_some() {
                    group_failures.insert(controller, "The occupied-country law inventory has duplicate canonical country links".into());
                }
            }
        }
        let mut assigned = HashMap::<Key, (i32, Vec<u128>)>::new();
        let mut unresolved_assigned = HashSet::new();
        for row in recorded_rows::<tables::StateAssignedManpower>(frame)? {
            match key(row.controller, row.occupied, row.state) {
                Some(key) => {
                    let (total, keys) = assigned.entry(key).or_default();
                    *total = total.wrapping_add(row.amount);
                    keys.push(u128::from(row.controller) << 64 | u128::from(row.state) << 32 | u128::from(row.ordinal));
                }
                None => {
                    unresolved_assigned.insert(row.state);
                    refused.note("state assigned manpower name a country with no recorded country link", format!("state {}", row.state));
                }
            }
        }
        let strengths_missing = missing(frame, TableId::GarrisonStrengths);
        let strengths = match strengths_missing {
            Some(_) => HashMap::new(),
            None => recorded_rows::<tables::GarrisonStrengths>(frame)?.iter().map(|row| (GarrisonStrengthRow::key_of(row.controller, row.state), row)).collect(),
        };
        Ok(Self {
            garrisons,
            overrides,
            overrides_complete: frame.complete::<tables::StateOccupationLaws>().is_ok(),
            settings,
            groups,
            group_failures,
            assigned,
            unresolved_assigned,
            assigned_complete: frame.complete::<tables::StateAssignedManpower>().is_ok(),
            strengths,
            strengths_missing,
        })
    }

    /// What `key`'s law is resolved from, and the raw occupied country its group scopes the
    /// law's eligibility to.
    pub(super) fn law_inputs(&self, controller: Option<u32>, key: Option<Key>) -> (LawInputs<'f>, Option<u32>) {
        let settings = controller.and_then(|slot| self.settings.get(&slot).copied());
        let mut scope = None;
        let group = match key {
            Some((controller, occupied, _)) => {
                if let Some(reason) = self.group_failures.get(&controller) {
                    GroupLaw::Unknown(reason.clone())
                } else if let Some(&(law, scoped)) = self.groups.get(&(controller, occupied)) {
                    scope = scoped;
                    GroupLaw::Law(law)
                } else if settings.is_some() {
                    GroupLaw::Missing
                } else {
                    GroupLaw::Unknown("The occupier's occupation settings are missing".into())
                }
            }
            None => GroupLaw::Unknown("Occupation country links are unresolved".into()),
        };
        let fallback = settings.ok_or_else(|| "The native database fallback law is missing".to_owned()).and_then(|row| match &row.fallback_law {
            Recorded::Value(Some(bytes)) => Ok(bytes.as_slice()),
            Recorded::Value(None) => Err("The native database fallback-law capture failed".into()),
            Recorded::NotRecorded => Err("The native database fallback law is not recorded".into()),
        });
        let inputs = LawInputs {
            state: key.and_then(|key| self.overrides.get(&key).copied()),
            key_resolved: key.is_some(),
            overrides_complete: self.overrides_complete,
            group,
            default: settings.map(|row| (row.default_law_present != 0, row.law.as_slice())),
            fallback,
        };
        (inputs, scope)
    }
}

/// The occupied rows of StateResistance with their States row, and the key each joins by.
fn occupied_states<'f>(resistance: &'f [StateResistanceRow], states: &'f [StateRow], links: &[CountryLinkRow]) -> Vec<(&'f StateResistanceRow, Option<&'f StateRow>, Option<Key>)> {
    let by_id: HashMap<u32, &StateRow> = states.iter().map(|state| (state.id, state)).collect();
    resistance
        .iter()
        .filter(|row| occupied(row))
        .map(|row| {
            let state = by_id.get(&row.state).copied();
            let key = state.and_then(|state| Some((slot(links, state.controller)?, slot(links, row.country)?, state.id)));
            (row, state, key)
        })
        .collect()
}

/// Whether some occupied state's law falls to its occupier's default, whose eligibility reads the
/// trigger world.
fn needs_world(frame: &DecodedFrame) -> bool {
    (|| -> io::Result<bool> {
        let links = frame.complete::<tables::CountryLinks>()?;
        let joins = Joins::read(frame, links, &mut Refusals::default())?;
        let states = recorded_rows::<tables::States>(frame)?;
        Ok(occupied_states(recorded_rows::<tables::StateResistance>(frame)?, states, links)
            .into_iter()
            .any(|(_, state, key)| joins.law_inputs(state.and_then(|state| slot(links, state.controller)), key).0.reaches_default()))
    })()
    .unwrap_or(false)
}

/// What a state's garrison shows: the structure, and the keys its live cells read.
struct Garrison {
    required: Option<i64>,
    alert: bool,
    /// Its StateGarrisons row's key.
    row: Option<u128>,
    /// Its GarrisonStrengths row's key, where the strength is known.
    strength: Option<u128>,
    /// Its StateAssignedManpower rows' keys, where the assigned manpower is known.
    assigned: Option<Vec<u128>>,
    missing: Option<String>,
}

/// Why a derived strength is not recorded, by its row's reason.
fn not_recorded(row: &StateGarrisonRow, reason: u8) -> String {
    match reason {
        GarrisonStrengthRow::UNRESOLVED => "The controller or occupied-country link is unresolved; the garrison's assigned manpower cannot be joined".into(),
        GarrisonStrengthRow::ASSIGNED => "state assigned manpower is incomplete or unresolved at this read".into(),
        GarrisonStrengthRow::OPERANDS => "This recording lacks the garrison equipment strength operands".into(),
        GarrisonStrengthRow::FAILED => match &row.equipment {
            Recorded::Value(inputs) if inputs.failure.is_some() => String::from_utf8_lossy(inputs.failure.as_deref().unwrap_or_default()).into_owned(),
            _ => "The garrison equipment operands' capture failed".into(),
        },
        other => format!("Garrison strength is not recorded for a reason this version does not know ({other})"),
    }
}

/// `row`'s derived strength's key where it is known and, where the game read its equipment
/// operands, each operand's equipment has a recorded-content definition; else why not.
fn strength(frame: &DecodedFrame, content: &Content, joins: &Joins<'_>, row: &StateGarrisonRow, assigned: Option<i32>) -> Result<u128, String> {
    if let Some(why) = &joins.strengths_missing {
        return Err(format!("Garrison strength is not recorded at this read: {why}"));
    }
    let key = GarrisonStrengthRow::key_of(row.controller, row.state);
    let derived = joins.strengths.get(&key).ok_or_else(|| "No garrison strength was derived for this garrison at this read".to_owned())?;
    if derived.strength == Recorded::NotRecorded {
        return Err(not_recorded(row, derived.reason));
    }
    if let Recorded::Value(inputs) = &row.equipment
        && row.serialized_flag == 0
        && assigned.is_some_and(|total| total != 0)
    {
        for entry in &inputs.rows {
            defined(frame, content, entry)?;
        }
    }
    Ok(key)
}

/// Whether `entry`'s equipment and archetype tokens name recorded-content equipment.
fn defined(frame: &DecodedFrame, content: &Content, entry: &GarrisonEquipment) -> Result<(), String> {
    let equipment = content.equipment()?;
    let tokens = frame.complete::<tables::LexicalTokens>().map_err(|why| format!("Garrison equipment dictionary unavailable: {why}"))?;
    for token in [entry.equipment_token, entry.archetype_token] {
        let text = tokens
            .binary_search_by_key(&token, |row| row.token)
            .ok()
            .map(|at| tokens[at].literal.as_slice())
            .filter(|text| !text.is_empty() && !text.contains(&0))
            .ok_or_else(|| format!("Garrison equipment token {token} has no recorded literal"))?;
        if equipment.named(text).is_none() {
            return Err(format!("Garrison equipment {} has no recorded-content definition", String::from_utf8_lossy(text)));
        }
    }
    Ok(())
}

fn garrison(frame: &DecodedFrame, content: &Content, joins: &Joins<'_>, state: u32, key: Option<Key>, refused: &mut Refusals) -> Garrison {
    let row = key.and_then(|key| joins.garrisons.get(&key).copied());
    let assigned = key.filter(|_| row.is_some() && joins.assigned_complete && !joins.unresolved_assigned.contains(&state)).map(|key| joins.assigned.get(&key).cloned().unwrap_or_default());
    let alert = row.is_some_and(|row| row.serialized_flag != 0);
    let mut missing = if key.is_none() {
        Some("The controller or occupied-country link is unresolved; the garrison key cannot be established".into())
    } else if row.is_none() {
        Some("No state garrisons row was recorded for the controller, occupied country and state".into())
    } else if !alert && assigned.is_none() {
        Some("state assigned manpower is incomplete or its links are unresolved; a zero manpower total cannot be established".into())
    } else {
        None
    };
    let strength = row.and_then(|row| {
        strength(frame, content, joins, row, assigned.as_ref().map(|(total, _)| *total))
            .map_err(|why| {
                refused.note(why.clone(), format!("state {state}"));
                missing = Some(why);
            })
            .ok()
    });
    Garrison {
        required: row.map(|row| row.garrison_required),
        alert,
        row: row.map(|row| u128::from(row.controller) << 32 | u128::from(row.state)),
        strength,
        assigned: assigned.map(|(_, keys)| keys),
        missing,
    }
}

/// `state`'s display name, kept while its custom and owner names stay.
fn name(content: &Content, state: &StateRow) -> Result<String, String> {
    let cat = content.catalogue().map_err(|_| "State names need the recorded content stack and language".to_owned())?;
    let mut names = content.names.lock().unwrap_or_else(|error| error.into_inner());
    let held = names.get(&state.id).filter(|held| held.0 == state.custom_name && held.1 == state.owner_name);
    if let Some(held) = held {
        return held.2.clone();
    }
    let made = cat
        .cached_states()
        .and_then(|content| dashboard_text::scripted_text::state_display_name(state, cat, Ok(&content.names), dashboard_text::scripted_text::StateNameUsage::Occupation));
    names.insert(state.id, (state.custom_name.clone(), state.owner_name.clone(), made.clone()));
    made
}

/// The reply at `frame`'s hour, its rows' moving numbers named in `live`.
pub(super) fn reply(frame: &DecodedFrame, content: &Content, live: &Cells) -> io::Result<OccupiedStatesReply> {
    let mut reply = OccupiedStatesReply {
        states: Vec::new(),
        complete: false,
        missing: None,
        laws: Vec::new(),
        strength_missing: STRENGTH_MISSING.into(),
        garrison_red_raw: content.red.map(|raw| raw.to_string()),
        warnings: content.warnings.clone(),
    };
    if let Some(why) = [TableId::StateResistance, TableId::States, TableId::Countries, TableId::CountryLinks].into_iter().find_map(|table| {
        let partial = table == TableId::StateResistance && matches!(&*frame.tables[table.index()], replay_frame::frame::TableView::Truncated { .. });
        missing(frame, table).filter(|_| !partial)
    }) {
        reply.missing = Some(why);
        return Ok(reply);
    }
    let tags = Tags::read(frame)?;
    let links = frame.complete::<tables::CountryLinks>()?;
    let states = frame.complete::<tables::States>()?;
    let mut refused = Refusals::default();
    let joins = Joins::read(frame, links, &mut refused)?;
    let fields: Vec<usize> = LIVE.iter().map(|(_, field)| dashboard_declare::field(TableId::StateResistance, field)).collect::<io::Result<_>>()?;
    let garrison_fields: Vec<usize> = GARRISON.iter().map(|(_, field)| dashboard_declare::field(TableId::StateGarrisons, field)).collect::<io::Result<_>>()?;
    let amount = dashboard_declare::field(TableId::StateAssignedManpower, "amount")?;
    let strength_field = dashboard_declare::field(TableId::GarrisonStrengths, "strength")?;
    let world = OnceCell::new();
    let mut admitted: HashMap<(String, u32, u32), Result<bool, String>> = HashMap::new();
    let mut complete = frame.complete::<tables::StateResistance>().is_ok();
    let mut tokens = BTreeSet::new();
    let mut rows = BTreeMap::new();
    for (resistance, state, key) in occupied_states(recorded_rows::<tables::StateResistance>(frame)?, states, links) {
        let Some(state) = state else {
            complete = false;
            refused.note("active resistance names a state absent from States", resistance.state.to_string());
            continue;
        };
        let controller = slot(links, state.controller);
        let (inputs, scope) = joins.law_inputs(controller, key);
        let made = law::law_for(inputs, |token| {
            let laws = content.laws.as_ref().map_err(Clone::clone)?;
            let definition = laws.find(token)?.ok_or_else(|| format!("Occupation law {token} has no recorded-content declaration"))?;
            if definition.visible.is_empty() && definition.available.is_empty() {
                return Ok(true);
            }
            let (controller, _, _) = key.expect("a law reaching the default has its key");
            let occupied = scope.and_then(|raw| slot(links, raw)).ok_or_else(|| "The occupied-country scope is not captured or its link is unresolved".to_owned())?;
            if definition.visible.is_empty() && controller == occupied {
                return Ok(true);
            }
            admitted
                .entry((token.to_owned(), controller, occupied))
                .or_insert_with(|| {
                    let world = world
                        .get_or_init(|| {
                            dashboard_text::triggers::inputs::unrecorded(|| {
                                let cat = content.catalogue().map_err(|_| "Occupation law eligibility needs recorded content".to_owned())?;
                                cat.scripted_content()?.recorded_world_in(HourFrame::from(frame))
                            })
                        })
                        .as_ref()
                        .map_err(Clone::clone)?;
                    law::admitted(world, definition, controller, occupied)
                })
                .clone()
        });
        if let Err(why) = &made {
            refused.note(why.clone(), format!("state {}", state.id));
        }
        let named = name(content, state);
        if let Err(why) = &named {
            refused.note(why.clone(), format!("state {}", state.id));
        }
        let garrison = garrison(frame, content, &joins, state.id, key, &mut refused);
        let row = OccupiedState {
            state: state.id,
            name: named.as_ref().ok().cloned(),
            name_missing: named.err(),
            owner: tags.tag(state.owner),
            occupier: tags.tag(state.controller),
            occupied: tags.tag(resistance.country),
            garrison_required_sign: garrison.required.map(|raw| raw.signum() as i8),
            garrison_alert: garrison.alert,
            assigned_known: garrison.assigned.is_some(),
            garrison_missing: garrison.missing,
            law: made.as_ref().ok().cloned(),
            law_missing: made.err(),
        };
        for (field, raw, absent) in [("owner", state.owner, row.owner.is_none()), ("occupier", state.controller, row.occupier.is_none()), ("occupied country", resistance.country, row.occupied.is_none())] {
            if absent {
                refused.note(format!("The {field} names no recorded country"), format!("state {}, raw link {raw}", state.id));
            }
        }
        complete &= row.occupier.is_some() && row.occupied.is_some();
        let id = state.id.to_string();
        for ((column, _), &field) in LIVE.iter().zip(&fields) {
            live.value(&id, column, Cell { table: TableId::StateResistance, column: field, key: u128::from(state.id) }, 1.0);
        }
        if let Some(key) = garrison.row {
            for ((column, _), &field) in GARRISON.iter().zip(&garrison_fields) {
                live.value(&id, column, Cell { table: TableId::StateGarrisons, column: field, key }, 1.0);
            }
        }
        if let Some(keys) = garrison.assigned.as_deref().filter(|keys| !keys.is_empty()) {
            let sources: Vec<Cell> = keys.iter().map(|&key| Cell { table: TableId::StateAssignedManpower, column: amount, key }).collect();
            live.sum(&id, ASSIGNED, &sources, 1.0);
        }
        if let Some(key) = garrison.strength {
            live.value(&id, STRENGTH, Cell { table: TableId::GarrisonStrengths, column: strength_field, key }, 1.0);
        }
        tokens.extend(row.law.clone());
        rows.insert(state.id, row);
    }
    let laws = content.laws.as_deref().map_err(String::as_str);
    reply.laws = tokens.iter().map(|token| law::info(content.catalogue(), laws, token)).collect();
    reply.states = rows.into_values().collect();
    reply.complete = complete;
    reply.warnings.extend(refused.lines("recorded occupation facts"));
    Ok(reply)
}

impl AtHour for OccupiedStates {
    const KEY: &'static str = "occupied-states";
    const TITLE: &'static str = "Occupied states";
    const CONTENT_DEPENDENT: bool = true;
    type Params = OccupiedStatesParams;
    type Reply = OccupiedStatesReply;
    type Context = Content;

    fn types() -> Vec<String> {
        vec![OccupiedStatesParams::decl(), OccupiedStatesReply::decl(), OccupiedState::decl(), OccupationLawInfo::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    /// The occupation tables and the trigger world a law's eligibility reads.
    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = TABLES.to_vec();
        tables.extend_from_slice(dashboard_text::scripted_text::TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    /// The occupation tables, then the trigger world only where a state's law falls to its
    /// occupier's default.
    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => TABLES.into_iter().map(Read::all).collect(),
            1 if needs_world(held) => Self::tables(params).into_iter().filter(|table| !TABLES.contains(table)).map(Read::all).collect(),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Content::load(source, load))
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        reply(frame, at.context, at.live)
    }
}
