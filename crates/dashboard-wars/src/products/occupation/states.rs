
use super::{
    Content, Judged, Judgements, WorldReads, law::{self, GroupLaw, LawInputs, OccupationLawInfo}, occupied, slot,
};
use content_reader_parser::{fixed5::decimal_i32, parser::{Item, Operator}};
use content_reader_politics::occupation_laws::Law;
use content_reader_script::scripted_trigger_grammar::{Lookup, ScriptedTriggers};
use dashboard_base::{Source, catalogue::{CachedResult, Refusals}, products::{LoadContext, at_hour::{At, AtHour, Edits, Keys, Owner, Read, ReadsAt, field_mask}}};
use dashboard_intelligence::products::intelligence::{Tags, branch, missing};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_text::{metadata::{RecordedRows, recorded_rows}, triggers::inputs::{Since, TableChange, TableChanges}};
use replay_data::rows::{
    CountryLinkRow, DeltaRow, GarrisonEquipment, GarrisonEquipmentInputs, GarrisonStrengthRow, OccupationSettingsRow, Recorded, StateAssignedManpowerRow, StateGarrisonRow, StateResistanceRow, StateRow,
    TableId, full_mask, tables,
};
use replay_frame::frame::{DecodedFrame, HourFrame};
use serde::{Deserialize, Serialize};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, hash_map::Entry},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

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

pub const LIVE: [(&str, &str); 4] = [("resistance", "level"), ("compliance", "compliance"), ("resistanceSpeed", "speed"), ("complianceSpeed", "compliance_speed")];

pub const GARRISON: [(&str, &str); 2] = [("garrison", "garrison_required"), ("requiredManpower", "required_manpower")];

pub const ASSIGNED: &str = "assignedManpower";

pub const STRENGTH: &str = "garrisonStrength";

pub const STRENGTH_MISSING: &str = "Garrison strength needs complete equipment operands, assigned manpower and the recorded content strength factors. The row names any missing input.";
const GROUP_LAW_MISSING: &str = "The law set for this occupied country is not recorded. It takes precedence over the occupier's default, so the state's law cannot be established.";

pub struct OccupiedStates;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccupiedStatesParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccupiedState {
    pub state: u32,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub owner: Option<String>,
    pub occupier: Option<String>,
    #[doc = " The original occupied country, which can differ from the legal owner."]
    pub occupied: Option<String>,
    #[doc = " The sign of the required amount (-1, 0, 1), with a garrison row: none required shows no"]
    #[doc = " bar, below zero the game shows unmet. It flips far more rarely than the amount moves."]
    pub garrison_required_sign: Option<i8>,
    #[doc = " The requirement failed: the game shows no strength."]
    pub garrison_alert: bool,
    #[doc = " The assigned manpower is known: its cell, or zero where the state has no assignment row"]
    #[doc = " and so no cell."]
    pub assigned_known: bool,
    pub garrison_missing: Option<String>,
    pub law: Option<String>,
    pub law_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccupiedStatesReply {
    #[doc = " Ascending by state."]
    pub states: Vec<OccupiedState>,
    #[doc = " Every row of StateResistance was read and every occupier and occupied country named."]
    pub complete: bool,
    #[doc = " Why no state can be shown at the hour."]
    pub missing: Option<String>,
    #[doc = " The laws the states hold."]
    pub laws: Vec<OccupationLawInfo>,
    pub strength_missing: String,
    #[doc = " The displayed strength below which a garrison shows red (fixed5); none where the content"]
    #[doc = " does not say."]
    pub garrison_red_raw: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OccupiedStatesReply);

type Key = (u32, u32, u32);

pub(super) struct Joins<'f> {
    garrisons: HashMap<Key, &'f StateGarrisonRow>,
    garrisons_missing: Option<String>,
    overrides: HashMap<Key, &'f [u8]>,
    overrides_missing: Option<String>,
    settings: HashMap<u32, &'f OccupationSettingsRow>,
    settings_missing: Option<String>,
    groups: HashMap<(u32, u32), (Option<&'f [u8]>, Option<u32>)>,
    group_failures: HashMap<u32, String>,
    assigned: HashMap<Key, (i32, Vec<u128>)>,
    unresolved_assigned: HashSet<u32>,
    assigned_missing: Option<String>,
    strengths: HashMap<u128, &'f GarrisonStrengthRow>,
    strengths_missing: Option<String>,
}

fn present_rows<'f, T: replay_data::rows::TableSchema>(frame: &'f DecodedFrame) -> io::Result<(&'f [T::Row], Option<String>)> {
    let read = recorded_rows::<T>(frame)?;
    let reason = read.missing_reason(T::ID, frame.hour);
    let rows = match read {
        RecordedRows::Complete(rows) | RecordedRows::Truncated { rows, .. } => rows,
        RecordedRows::Unavailable { .. } | RecordedRows::NotRecorded { .. } => &[],
    };
    Ok((rows, reason))
}

impl<'f> Joins<'f> {
    pub(super) fn read(frame: &'f DecodedFrame, links: &[CountryLinkRow], refused: &mut Refusals) -> io::Result<Self> {
        let key = |controller, occupied, state| Some((slot(links, controller)?, slot(links, occupied)?, state));
        let mut garrisons = HashMap::new();
        let (garrison_rows, garrisons_missing) = present_rows::<tables::StateGarrisons>(frame)?;
        for row in garrison_rows {
            match key(row.controller, row.occupied, row.state) {
                Some(key) => {
                    garrisons.insert(key, row);
                }
                None => refused.note("state garrisons name a country with no recorded country link", format!("state {}", row.state)),
            }
        }
        let mut overrides = HashMap::new();
        let (override_rows, overrides_missing) = present_rows::<tables::StateOccupationLaws>(frame)?;
        for row in override_rows {
            match key(row.controller, row.occupied, row.state) {
                Some(key) => {
                    overrides.insert(key, row.name.as_slice());
                }
                None => refused.note("state occupation laws name a country with no recorded country link", format!("state {}", row.state)),
            }
        }
        let mut settings = HashMap::new();
        let mut group_failures = HashMap::new();
        let (setting_rows, settings_missing) = present_rows::<tables::OccupationSettings>(frame)?;
        for row in setting_rows {
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
        let (assigned_rows, assigned_missing) = present_rows::<tables::StateAssignedManpower>(frame)?;
        for row in assigned_rows {
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
        let (strength_rows, strengths_missing) = present_rows::<tables::GarrisonStrengths>(frame)?;
        let strengths = strength_rows.iter().map(|row| (GarrisonStrengthRow::key_of(row.controller, row.state), row)).collect();
        Ok(Self {
            garrisons,
            garrisons_missing,
            overrides,
            overrides_missing,
            settings,
            settings_missing,
            groups,
            group_failures,
            assigned,
            unresolved_assigned,
            assigned_missing,
            strengths,
            strengths_missing,
        })
    }

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
                    GroupLaw::Unknown(self.settings_missing.clone().unwrap_or_else(|| "The occupier's occupation settings are missing".into()))
                }
            }
            None => GroupLaw::Unknown("Occupation country links are unresolved".into()),
        };
        let fallback = settings
            .ok_or_else(|| self.settings_missing.clone().unwrap_or_else(|| "The native database fallback law is missing".to_owned()))
            .and_then(|row| match &row.fallback_law {
                Recorded::Value(Some(bytes)) => Ok(bytes.as_slice()),
                Recorded::Value(None) => Err("The native database fallback-law capture failed".into()),
                Recorded::NotRecorded => Err("The native database fallback law is not recorded".into()),
            });
        let inputs = LawInputs {
            state: key.and_then(|key| self.overrides.get(&key).copied()),
            key_resolved: key.is_some(),
            overrides_missing: self.overrides_missing.clone(),
            group,
            default: settings.map(|row| (row.default_law_present != 0, row.law.as_slice())),
            fallback,
        };
        (inputs, scope)
    }

    fn assigned(&self, key: Key) -> Option<(i32, &[u128])> {
        (self.assigned_missing.is_none() && !self.unresolved_assigned.contains(&key.2))
            .then(|| self.assigned.get(&key).map_or((0, &[][..]), |(total, keys)| (*total, keys.as_slice())))
    }
}

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

fn read_operands(frame: &DecodedFrame, content: &Content, equipment: bool) -> (bool, Option<BTreeSet<u128>>) {
    let equipment = equipment && content.equipment.get().map_or(content.catalogue.is_ok(), Result::is_ok);
    (|| -> io::Result<_> {
        let links = frame.complete::<tables::CountryLinks>()?;
        let joins = Joins::read(frame, links, &mut Refusals::default())?;
        let (RecordedRows::Complete(states) | RecordedRows::Truncated { rows: states, .. }) = recorded_rows::<tables::States>(frame)? else {
            return Ok(Default::default());
        };
        let (RecordedRows::Complete(resistance) | RecordedRows::Truncated { rows: resistance, .. }) = recorded_rows::<tables::StateResistance>(frame)? else {
            return Ok(Default::default());
        };
        let mut world = false;
        let mut tokens = None;
        let complete_garrisons = frame.complete::<tables::StateGarrisons>().is_ok();
        for (_, state, key) in occupied_states(resistance, states, links) {
            if !world {
                let (inputs, scope) = joins.law_inputs(state.and_then(|state| slot(links, state.controller)), key);
                if inputs.reaches_default() {
                    let _ = law::law_for(inputs, |token| {
                        world = admission_rule(content, links, token, key, scope)?.is_some();
                        Ok(true)
                    });
                }
            }
            if !equipment && world { break; }
            if equipment && let Some(key) = key && let Some(row) = joins.garrisons.get(&key)
                && joins.strengths.get(&GarrisonStrengthRow::key_of(row.controller, row.state)).is_some_and(|row| row.strength != Recorded::NotRecorded)
            {
                let operands = strength_equipment(row, joins.assigned(key).map(|(total, _)| total));
                if !operands.is_empty() {
                    let keys = tokens.get_or_insert_with(BTreeSet::new);
                    if complete_garrisons {
                        keys.extend(operands.iter().flat_map(|entry| [u128::from(entry.equipment_token), u128::from(entry.archetype_token)]));
                    }
                }
            }
        }
        Ok((world, tokens))
    })()
    .unwrap_or_default()
}

fn admission_rule<'c>(content: &'c Content, links: &[CountryLinkRow], token: &str, key: Option<Key>, scope: Option<u32>) -> Result<Option<(&'c Law, u32, u32)>, String> {
    let laws = content.laws.as_ref().map_err(Clone::clone)?;
    let definition = laws.find(token)?.ok_or_else(|| format!("Occupation law {token} has no recorded-content declaration"))?;
    if definition.visible.is_empty() && definition.available.is_empty() {
        return Ok(None);
    }
    let (controller, _, _) = key.expect("a law reaching the default has its key");
    let occupied = scope.and_then(|raw| slot(links, raw)).ok_or_else(|| "The occupied-country scope is not captured or its link is unresolved".to_owned())?;
    Ok((!definition.visible.is_empty() || controller != occupied).then_some((definition, controller, occupied)))
}

struct Admission<'f> {
    frame: &'f DecodedFrame,
    content: &'f Content,
    links: &'f [CountryLinkRow],
    world: OnceCell<Result<dashboard_text::triggers::World<'f>, String>>,
    decided: Judgements,
    read: WorldReads,
    through: i32,
}

impl<'f> Admission<'f> {
    fn new(frame: &'f DecodedFrame, content: &'f Content, links: &'f [CountryLinkRow]) -> Self {
        let judged = content.judged.lock().unwrap_or_else(|error| error.into_inner());
        let decided = if judged.at == Some((frame.branch, frame.hour)) { judged.decisions.clone() } else { Judgements::default() };
        Self { frame, content, links, world: OnceCell::new(), decided, read: WorldReads::default(), through: i32::MAX }
    }

    fn fresh(frame: &'f DecodedFrame, content: &'f Content, links: &'f [CountryLinkRow]) -> Self {
        Self { frame, content, links, world: OnceCell::new(), decided: Judgements::default(), read: WorldReads::default(), through: i32::MAX }
    }

    fn admit(&mut self, token: &str, key: Option<Key>, scope: Option<u32>) -> Result<bool, String> {
        let Some((definition, controller, occupied)) = admission_rule(self.content, self.links, token, key, scope)? else {
            return Ok(true);
        };
        let Self { frame, content, world, decided, read, through, .. } = self;
        let (frame, content) = (*frame, *content);
        let slot = match decided.entry((token.to_owned(), controller, occupied)) {
            Entry::Occupied(held) => return held.get().0.clone(),
            Entry::Vacant(slot) => slot,
        };
        let world = world
            .get_or_init(|| {
                dashboard_text::triggers::inputs::unrecorded(|| {
                    let cat = content.catalogue().map_err(|_| "Occupation law eligibility needs recorded content".to_owned())?;
                    cat.scripted_content()?.recorded_world_in(HourFrame::from(frame))
                })
            })
            .as_ref()
            .map_err(Clone::clone)?;
        let (made, inputs) = dashboard_text::triggers::inputs::capture_in(HourFrame::from(frame), || law::admitted(world, definition, controller, occupied));
        let mut decision_read = WorldReads::default();
        decision_read.note(&inputs);
        *through = (*through).min(decision_through(frame, content, definition, &inputs, &decision_read));
        read.note(&inputs);
        slot.insert((made.clone(), inputs));
        made
    }
}

fn day_start(hour: i32) -> i32 {
    hour.wrapping_sub(hour.wrapping_sub(game_model::calendar::HOUR_EPOCH) % 24)
}

fn scalar_number(text: &[u8]) -> Option<i32> {
    let body = text.strip_prefix(b"-").unwrap_or(text);
    let mut dots = 0u8;
    if body.is_empty()
        || !body.iter().all(|&byte| {
            if byte == b'.' {
                dots += 1;
                dots <= 1
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return None;
    }
    let value = decimal_i32(text);
    (value != i32::MIN && value != i32::MAX).then_some(value)
}

fn collect_flag_days(items: &[Item], scripted: Option<&ScriptedTriggers>, visited: &mut HashSet<Vec<u8>>, out: &mut Vec<(Operator, i32)>) -> bool {
    for item in items {
        let keyword = item.keyword().map(|keyword| keyword.folded());
        if matches!(keyword.as_deref(), Some(b"date" | b"has_character_flag")) {
            return false;
        }
        if matches!(keyword.as_deref(), Some(b"has_country_flag" | b"has_state_flag" | b"has_global_flag")) {
            if let Some(block) = item.value.items() {
                for field in block {
                    if field.keyword().is_some_and(|keyword| keyword.folded() == b"days") && matches!(field.operator, Operator::Greater | Operator::Less) {
                        let Some(count) = field.value.scalar().and_then(scalar_number) else { return false };
                        out.push((field.operator, count));
                    }
                }
            }
            continue;
        }
        if let Some(key) = keyword.as_deref()
            && let Some(scripted) = scripted
            && let Lookup::Template(template) = scripted.lookup(key)
            && visited.insert(key.to_vec())
        {
            match &template.body {
                Ok(body) => {
                    if !collect_flag_days(body, Some(scripted), visited, out) {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        }
        if let Some(block) = item.value.items()
            && !collect_flag_days(block, scripted, visited, out)
        {
            return false;
        }
    }
    true
}

fn war_relation_clock_through(frame: &DecodedFrame) -> i32 {
    match frame.tables[TableId::WarRelations.index()].held() {
        replay_frame::frame::TableView::Complete(rows) | replay_frame::frame::TableView::Truncated { rows, .. } => {
            rows.rows::<tables::WarRelations>().map_or(frame.hour, |rows| {
                rows.iter()
                    .flat_map(|row| [
                        (row.start_hour > frame.hour).then_some(row.start_hour),
                        (row.end_hour > row.start_hour && row.end_hour > frame.hour).then_some(row.end_hour),
                    ])
                    .flatten()
                    .min()
                    .map_or(i32::MAX, |at| at.saturating_sub(1))
            })
        }
        _ => frame.hour,
    }
}

fn decision_through(frame: &DecodedFrame, content: &Content, definition: &Law, inputs: &dashboard_text::triggers::inputs::Inputs, read: &WorldReads) -> i32 {
    use dashboard_text::triggers::{inputs::Key, world::Query};
    if read.undeclared {
        return frame.hour;
    }
    let mut through = ends_through(frame, &read.ends);
    if !read.clock {
        return through;
    }
    let mut accounted = false;
    let mut flag_queries: Vec<(u8, u32, &[u8])> = Vec::new();
    for key in inputs.keys() {
        match key {
            Key::Table(TableId::WarRelations) | Key::Fields(TableId::WarRelations, _) => {
                accounted = true;
                through = through.min(war_relation_clock_through(frame));
            }
            Key::World(Query::IndustrialOrganisationFlags(_)) => return frame.hour,
            Key::World(Query::Flag(kind, id, name)) => flag_queries.push((*kind, *id, name.as_slice())),
            _ => {}
        }
    }
    if !flag_queries.is_empty() {
        let scripted = content.catalogue().ok().and_then(|cat| {
            cat.content
                .layers
                .derived::<CachedResult<ScriptedTriggers>>()
                .get_or_init(|| content_reader_key_names::scripted_triggers::read(&cat.content.layers))
                .as_ref()
                .ok()
                .cloned()
        });
        let mut days = Vec::new();
        let mut visited = HashSet::new();
        if !collect_flag_days(&definition.visible, scripted.as_deref(), &mut visited, &mut days)
            || !collect_flag_days(&definition.available, scripted.as_deref(), &mut visited, &mut days)
            || days.is_empty()
        {
            return frame.hour;
        }
        let (Ok(tokens), Ok(flags)) = (frame.complete::<tables::LexicalTokens>(), frame.complete::<tables::ScriptFlags>()) else {
            return frame.hour;
        };
        accounted = true;
        for (kind, id, name) in flag_queries {
            for tok in tokens.iter().filter(|row| row.literal.eq_ignore_ascii_case(name)) {
                let key = (u128::from(kind) << 64) | (u128::from(id) << 32) | u128::from(tok.token);
                if let Ok(at) = flags.binary_search_by_key(&key, |row| (u128::from(row.scope_kind) << 64) | (u128::from(row.scope_id) << 32) | u128::from(row.token)) {
                    let start = day_start(flags[at].set_hour);
                    for &(op, count) in &days {
                        let Some(delta) = count.checked_mul(24) else { return frame.hour };
                        let Some(threshold) = start.checked_add(delta) else { return frame.hour };
                        let target = match op {
                            Operator::Greater => match threshold.checked_add(24) {
                                Some(value) => value,
                                None => return frame.hour,
                            },
                            Operator::Less => threshold,
                            _ => continue,
                        };
                        let flip = if target <= game_model::calendar::HOUR_EPOCH {
                            match target.checked_sub(23) {
                                Some(value) => value,
                                None => return frame.hour,
                            }
                        } else {
                            target
                        };
                        if flip > frame.hour {
                            through = through.min(flip.saturating_sub(1));
                        }
                    }
                }
            }
        }
    }
    if !accounted {
        return frame.hour;
    }
    through
}

struct EditChanges {
    rows: BTreeMap<TableId, Vec<DeltaRow>>,
    replaced: BTreeSet<TableId>,
}

impl EditChanges {
    fn new(edits: &Edits) -> Self {
        let rows = edits.tables.iter().map(|(&table, keys)| {
            let fields = table.schema().len();
            (table, keys.iter().map(|(&key, &mask)| DeltaRow { format: 0, key, mask: if mask == u64::MAX { full_mask(fields) } else { mask }, recorded_fields: fields, values: Vec::new() }).collect())
        }).collect();
        Self { rows, replaced: edits.replaced.clone() }
    }
}

impl TableChanges for EditChanges {
    fn table(&self, table: TableId) -> TableChange<'_> {
        if self.replaced.contains(&table) { return TableChange::Any; }
        self.rows.get(&table).map_or(TableChange::Same, |rows| TableChange::Rows(rows, &[]))
    }
}

fn ends_through(frame: &DecodedFrame, ends: &BTreeSet<TableId>) -> i32 {
    let mut through = i32::MAX;
    for &table in ends {
        let next = match table {
            TableId::WarRelations => match frame.tables[table.index()].held() {
                replay_frame::frame::TableView::Complete(rows) | replay_frame::frame::TableView::Truncated { rows, .. } => {
                    rows.rows::<tables::WarRelations>().map_or(Some(frame.hour.saturating_add(1)), |rows| {
                        rows.iter().filter(|row| row.end_hour > row.start_hour && row.end_hour > frame.hour).map(|row| row.end_hour).min()
                    })
                }
                _ => None,
            },
            TableId::Subjects => match frame.tables[table.index()].held() {
                replay_frame::frame::TableView::Complete(rows) | replay_frame::frame::TableView::Truncated { rows, .. } => {
                    rows.rows::<tables::Subjects>().map_or(Some(frame.hour.saturating_add(1)), |rows| {
                        rows.iter().filter(|row| row.end_hour > row.start_hour && row.end_hour > frame.hour).map(|row| row.end_hour).min()
                    })
                }
                _ => None,
            },
            _ => return frame.hour,
        };
        if let Some(at) = next {
            through = through.min(at.saturating_sub(1));
        }
    }
    through
}

pub(super) fn world_valid_through(frame: &DecodedFrame, world: &WorldReads) -> i32 {
    if world.undeclared {
        return frame.hour;
    }
    let mut through = ends_through(frame, &world.ends);
    if world.clock && world.fields.contains_key(&TableId::WarRelations) {
        through = through.min(war_relation_clock_through(frame));
    }
    through
}

fn world_touched(edits: &Edits, world: &WorldReads) -> bool {
    edits.replaced.iter().any(|table| !own(table) || world.fields.contains_key(table))
        || edits.tables.iter().any(|(table, rows)| !own(table) || world.fields.get(table).is_some_and(|&mask| rows.values().any(|&changed| changed & mask != 0)))
}

fn decision_holds(inputs: &dashboard_text::triggers::inputs::Inputs, since: &Since<'_>) -> bool {
    use dashboard_text::triggers::{inputs::Key, world::Query};
    inputs.keys().all(|key| match key {
        Key::World(Query::Clock) => true,
        _ => key.dependence().is_some_and(|reads| reads.iter().all(|read| !since.changed(read))),
    })
}

pub(super) fn judged_through(content: &Content, held: &DecodedFrame, since: Option<i32>, edits: &Edits) -> bool {
    let mut judged = content.judged.lock().unwrap_or_else(|error| error.into_inner());
    if judged.at == Some((held.branch, held.hour)) { return true; }
    if since.filter(|&from| judged.at == Some((held.branch, from))).is_none() { return false; }
    if judged.read.undeclared || held.hour > judged.through {
        judged.at = None;
        return false;
    }
    if !world_touched(edits, &judged.read) {
        judged.at = Some((held.branch, held.hour));
        judged.carried = true;
        return true;
    }
    let changes = EditChanges::new(edits);
    let since = Since::span_in(held.branch, judged.inputs_at, held.hour, Some(HourFrame::from(held)), &changes);
    if !judged.decisions.values().all(|(_, inputs)| decision_holds(inputs, &since)) {
        judged.at = None;
        return false;
    }
    judged.at = Some((held.branch, held.hour));
    judged.inputs_at = held.hour;
    judged.through = judged.through.min(world_valid_through(held, &judged.read));
    judged.carried = true;
    true
}

pub(super) fn law_stands(content: &Content, held: &DecodedFrame, edits: &Edits) -> bool {
    let judged = content.judged.lock().unwrap_or_else(|error| error.into_inner());
    let clocked = !judged.read.ends.is_empty() || judged.read.clock || judged.read.undeclared;
    if !clocked && !world_touched(edits, &judged.read) { return true; }
    judged.carried && judged.at == Some((held.branch, held.hour))
}

fn eligibility(frame: &DecodedFrame, content: &Content) -> (Judgements, WorldReads, i32) {
    (|| -> io::Result<(Judgements, WorldReads, i32)> {
        let links = frame.complete::<tables::CountryLinks>()?;
        let joins = Joins::read(frame, links, &mut Refusals::default())?;
        let (RecordedRows::Complete(resistance) | RecordedRows::Truncated { rows: resistance, .. }) = recorded_rows::<tables::StateResistance>(frame)? else {
            return Ok((Judgements::default(), WorldReads::default(), i32::MAX));
        };
        let (RecordedRows::Complete(states) | RecordedRows::Truncated { rows: states, .. }) = recorded_rows::<tables::States>(frame)? else {
            return Ok((Judgements::default(), WorldReads::default(), i32::MAX));
        };
        let mut admission = Admission::fresh(frame, content, links);
        for (_, state, key) in occupied_states(resistance, states, links) {
            let Some(state) = state else { continue };
            let (inputs, scope) = joins.law_inputs(slot(links, state.controller), key);
            let _ = law::law_for(inputs, |token| admission.admit(token, key, scope));
        }
        Ok((admission.decided, admission.read, admission.through))
    })()
    .unwrap_or_else(|_| (Judgements::default(), WorldReads::default(), i32::MAX))
}

const WORLD_CORE: [TableId; 1] = [TableId::Provinces];

fn world_missing(params: &OccupiedStatesParams, held: &DecodedFrame, content: &Content, stage: usize) -> Vec<TableId> {
    let (decided, read, through) = eligibility(held, content);
    let lacking = |table: &TableId| {
        let view = &held.tables[table.index()];
        view.held().is_not_selected() || *view.keys() != Keys::All
    };
    let missing: Vec<TableId> = if read.undeclared {
        OccupiedStates::tables(params).into_iter().filter(lacking).collect()
    } else {
        read.fields.keys().copied().filter(lacking).collect()
    };
    let mut world = content.world.lock().unwrap_or_else(|error| error.into_inner());
    let before = world.clone();
    world.add(&read);
    let mut judged = content.judged.lock().unwrap_or_else(|error| error.into_inner());
    if missing.is_empty() && *world == before {
        *judged = Judged {
            at: Some((held.branch, held.hour)),
            inputs_at: held.hour,
            stage,
            through,
            read,
            decisions: decided,
            carried: false,
        };
    } else {
        judged.at = None;
    }
    missing
}

fn judge_none(held: &DecodedFrame, content: &Content, stage: usize) {
    *content.judged.lock().unwrap_or_else(|error| error.into_inner()) = Judged {
        at: Some((held.branch, held.hour)),
        inputs_at: held.hour,
        stage,
        through: i32::MAX,
        read: WorldReads::default(),
        decisions: Judgements::default(),
        carried: false,
    };
}

fn world_planned(plan: &[(Vec<Read>, i32)]) -> bool {
    plan.get(1).is_some_and(|(reads, _)| reads.iter().any(|read| read.table != TableId::LexicalTokens))
}

struct Garrison {
    required: Option<i64>,
    alert: bool,
    row: Option<u128>,
    strength: Option<u128>,
    assigned: Option<Vec<u128>>,
    missing: Option<String>,
}

fn not_recorded(row: &StateGarrisonRow, reason: u8, assigned_missing: Option<&str>) -> String {
    match reason {
        GarrisonStrengthRow::UNRESOLVED => "The controller or occupied-country link is unresolved; the garrison's assigned manpower cannot be joined".into(),
        GarrisonStrengthRow::ASSIGNED => assigned_missing
            .map(str::to_owned)
            .unwrap_or_else(|| "state assigned manpower is incomplete or unresolved at this read".into()),
        GarrisonStrengthRow::OPERANDS => "This recording lacks the garrison equipment strength operands".into(),
        GarrisonStrengthRow::FAILED => match &row.equipment {
            Recorded::Value(inputs) if inputs.failure.is_some() => String::from_utf8_lossy(inputs.failure.as_deref().unwrap_or_default()).into_owned(),
            _ => "The garrison equipment operands' capture failed".into(),
        },
        other => format!("Garrison strength is not recorded for a reason this version does not know ({other})"),
    }
}

fn strength(frame: &DecodedFrame, content: &Content, joins: &Joins<'_>, row: &StateGarrisonRow, assigned: Option<i32>) -> Result<u128, String> {
    let key = GarrisonStrengthRow::key_of(row.controller, row.state);
    let derived = joins.strengths.get(&key).ok_or_else(|| match &joins.strengths_missing {
        Some(why) => format!("Garrison strength is not recorded at this read: {why}"),
        None => "No garrison strength was derived for this garrison at this read".to_owned(),
    })?;
    if derived.strength == Recorded::NotRecorded {
        return Err(not_recorded(row, derived.reason, joins.assigned_missing.as_deref()));
    }
    for entry in strength_equipment(row, assigned) {
        defined(frame, content, entry)?;
    }
    Ok(key)
}

fn strength_equipment(row: &StateGarrisonRow, assigned: Option<i32>) -> &[GarrisonEquipment] {
    if let Recorded::Value(inputs) = &row.equipment
        && row.serialized_flag == 0
        && assigned.is_some_and(|total| total != 0)
    {
        return &inputs.rows;
    }
    &[]
}

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
    let assigned = key.filter(|_| row.is_some()).and_then(|key| joins.assigned(key));
    let alert = row.is_some_and(|row| row.serialized_flag != 0);
    let mut missing = if key.is_none() {
        Some("The controller or occupied-country link is unresolved; the garrison key cannot be established".into())
    } else if row.is_none() {
        Some(joins.garrisons_missing.clone().unwrap_or_else(|| "No state garrisons row was recorded for the controller, occupied country and state".into()))
    } else if !alert && assigned.is_none() {
        Some(match &joins.assigned_missing {
            Some(why) => format!("{why}; a zero manpower total cannot be established"),
            None => "state assigned manpower links are unresolved; a zero manpower total cannot be established".into(),
        })
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
        assigned: assigned.map(|(_, keys)| keys.to_vec()),
        missing,
    }
}

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
    let mut admission = Admission::new(frame, content, links);
    let (resistance_rows, mut complete) = match recorded_rows::<tables::StateResistance>(frame)?.into_rows(TableId::StateResistance, frame.hour) {
        Ok((rows, incomplete)) => {
            let complete = incomplete.is_none();
            if let Some(why) = incomplete {
                reply.warnings.push(why);
            }
            (rows, complete)
        }
        Err(why) => {
            reply.missing = Some(why);
            return Ok(reply);
        }
    };
    let mut tokens = BTreeSet::new();
    let mut rows = BTreeMap::new();
    for (resistance, state, key) in occupied_states(resistance_rows, states, links) {
        let Some(state) = state else {
            complete = false;
            refused.note("active resistance names a state absent from States", resistance.state.to_string());
            continue;
        };
        let controller = slot(links, state.controller);
        let (inputs, scope) = joins.law_inputs(controller, key);
        let made = law::law_for(inputs, |token| admission.admit(token, key, scope));
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

const MOVING: [(TableId, &[&str]); 3] = [
    (TableId::StateGarrisons, &["garrison_required", "equipment"]),
    (TableId::StateAssignedManpower, &["amount"]),
    (TableId::GarrisonStrengths, &["strength"]),
];

fn own(table: &TableId) -> bool {
    TABLES.contains(table)
}

fn moving_only(edits: &Edits) -> bool {
    !edits.replaced.iter().any(own) && edits.tables.iter().filter(|(table, _)| own(table)).all(|(table, rows)| {
        MOVING.iter().find(|(moving, _)| moving == table).is_some_and(|&(table, fields)| {
            let mask = field_mask(table, fields);
            rows.values().all(|changed| changed & !mask == 0)
        })
    })
}

fn find<R>(rows: &[R], key: u128, key_of: impl Fn(&R) -> u128) -> Option<&R> {
    rows.binary_search_by_key(&key, &key_of).ok().map(|at| &rows[at]).or_else(|| rows.iter().find(|row| key_of(row) == key))
}

fn same_tokens(before: &Recorded<GarrisonEquipmentInputs>, after: &Recorded<GarrisonEquipmentInputs>) -> bool {
    match (before, after) {
        (Recorded::NotRecorded, Recorded::NotRecorded) => true,
        (Recorded::Value(before), Recorded::Value(after)) => before.failure == after.failure && before.rows.len() == after.rows.len()
            && before.rows.iter().zip(&after.rows).all(|(a, b)| (a.equipment_token, a.archetype_token) == (b.equipment_token, b.archetype_token)),
        _ => false,
    }
}

pub(super) fn structure_holds(frame: &DecodedFrame, edits: &Edits) -> bool {
    if !moving_only(edits) { return false; }
    let before = |table: TableId| match edits.previous.get(&table).map(|view| view.held()) {
        Some(replay_frame::frame::TableView::Complete(rows)) => Some(rows),
        _ => None,
    };
    if let Some(changed) = edits.tables.get(&TableId::StateGarrisons) {
        let (Some(old), Ok(new)) = (before(TableId::StateGarrisons).and_then(|rows| rows.rows::<tables::StateGarrisons>()), frame.complete::<tables::StateGarrisons>()) else { return false };
        let key_of = |row: &StateGarrisonRow| u128::from(row.controller) << 32 | u128::from(row.state);
        for &key in changed.keys() {
            let (Some(a), Some(b)) = (find(old, key, key_of), find(new, key, key_of)) else { return false };
            if a.garrison_required.signum() != b.garrison_required.signum() || !same_tokens(&a.equipment, &b.equipment) { return false; }
        }
    }
    if let Some(changed) = edits.tables.get(&TableId::GarrisonStrengths) {
        let (Some(old), Ok(new)) = (before(TableId::GarrisonStrengths).and_then(|rows| rows.rows::<tables::GarrisonStrengths>()), frame.complete::<tables::GarrisonStrengths>()) else { return false };
        let key_of = |row: &GarrisonStrengthRow| GarrisonStrengthRow::key_of(row.controller, row.state);
        for &key in changed.keys() {
            let (Some(a), Some(b)) = (find(old, key, key_of), find(new, key, key_of)) else { return false };
            if (a.strength == Recorded::NotRecorded) != (b.strength == Recorded::NotRecorded) { return false; }
        }
    }
    if let Some(changed) = edits.tables.get(&TableId::StateAssignedManpower) {
        let (Some(old), Ok(new), Ok(links)) = (before(TableId::StateAssignedManpower).and_then(|rows| rows.rows::<tables::StateAssignedManpower>()),
            frame.complete::<tables::StateAssignedManpower>(), frame.complete::<tables::CountryLinks>()) else { return false };
        let touched: BTreeSet<u32> = changed.keys().map(|&key| (key >> 32) as u32).collect();
        let shown = |rows: &[StateAssignedManpowerRow]| {
            let mut totals = BTreeMap::<Key, i32>::new();
            for row in rows.iter().filter(|row| touched.contains(&row.state)) {
                if let (Some(controller), Some(occupied)) = (slot(links, row.controller), slot(links, row.occupied)) {
                    let total = totals.entry((controller, occupied, row.state)).or_default();
                    *total = total.wrapping_add(row.amount);
                }
            }
            totals.into_iter().map(|(key, total)| (key, total != 0)).collect::<Vec<_>>()
        };
        if shown(old) != shown(new) { return false; }
    }
    true
}

impl AtHour for OccupiedStates {
    const KEY: &'static str = "occupied-states";
    const TITLE: &'static str = "Occupied states";
    const CONTENT_DEPENDENT: bool = true;
    type Params = OccupiedStatesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = OccupiedStatesReply;
    type Context = Content;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![OccupiedStatesParams::decl(), OccupiedStatesReply::decl(), OccupiedState::decl(), OccupationLawInfo::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = TABLES.to_vec();
        tables.extend_from_slice(dashboard_text::scripted_text::TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    fn reads_in(params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        if step == 0 { return TABLES.into_iter().filter(|&table| table != TableId::LexicalTokens).map(Read::all).collect(); }
        let (needs_world, tokens) = read_operands(held, at.context, step == 1);
        if step > 1 {
            if needs_world { return world_missing(params, held, at.context, step).into_iter().map(Read::all).collect(); }
            judge_none(held, at.context, step);
            return Vec::new();
        }
        let learned = at.context.world.lock().unwrap_or_else(|error| error.into_inner()).clone();
        let full_lexicon = learned.undeclared || learned.fields.contains_key(&TableId::LexicalTokens);
        let lexicon = if full_lexicon && (needs_world || tokens.as_ref().is_some_and(BTreeSet::is_empty)) {
            Some(Read::all(TableId::LexicalTokens))
        } else {
            tokens.map(|keys| Read::these(TableId::LexicalTokens, keys))
        };
        let world: BTreeSet<TableId> = if !needs_world { BTreeSet::new() }
            else if learned.undeclared { Self::tables(params).into_iter().collect() }
            else { WORLD_CORE.into_iter().chain(learned.fields.keys().copied()).collect() };
        let reads: Vec<_> = world.into_iter().filter(|table| !TABLES.contains(table)).map(Read::all).chain(lexicon).collect();
        if reads.is_empty() { judge_none(held, at.context, step); }
        reads
    }

    fn retained_reads(params: &Self::Params, owner: Option<Owner>, stage: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        let reads = Self::reads_in(params, owner, stage, held, at);
        let through = if stage != 0 && reads.is_empty() {
            let judged = at.context.judged.lock().unwrap_or_else(|error| error.into_inner());
            if judged.at == Some((held.branch, held.hour)) { judged.through } else { held.hour }
        } else {
            i32::MAX
        };
        (reads, through)
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        let fields: &[&str] = match table {
            TableId::StateResistance => &["country", "active", "level", "speed", "compliance", "compliance_speed"],
            TableId::States => &["owner", "controller", "custom_name", "owner_name", "continent", "population", "manpower_available", "best_vp_index"],
            TableId::Countries => &["canonical_slot", "tag", "manpower_residual"],
            TableId::Provinces => &["state", "owner", "controller", "victory_points", "controller_area"],
            TableId::StateGarrisons => &["occupied", "required_manpower", "garrison_required", "serialized_flag", "equipment"],
            TableId::StateAssignedManpower => &["occupied", "amount"],
            TableId::OccupationSettings => &["default_law_present", "law", "group_laws", "fallback_law"],
            _ => return None,
        };
        Some(dashboard_base::products::at_hour::field_columns(table, fields))
    }

    fn projection(_params: &Self::Params, table: TableId, plan: &[(Vec<Read>, i32)], content: &Self::Context) -> Option<Vec<usize>> {
        let reply: &[&str] = match table {
            TableId::StateResistance => &["country", "active"],
            TableId::StateGarrisons => &["occupied", "garrison_required", "serialized_flag", "equipment"],
            TableId::StateAssignedManpower => &["occupied", "amount"],
            TableId::States => &["owner", "controller", "custom_name", "owner_name"],
            TableId::Countries => &["canonical_slot", "tag"],
            _ if TABLES.contains(&table) => return Self::columns(table),
            _ => &[],
        };
        let mut fields = dashboard_base::products::at_hour::field_columns(table, reply).into_iter().fold(0u64, |mask, field| mask | 1 << field);
        if world_planned(plan) {
            let world = content.world.lock().unwrap_or_else(|error| error.into_inner());
            if world.undeclared { return None; }
            fields |= world.fields.get(&table).copied().unwrap_or(0);
        }
        Some((0..table.field_count()).filter(|&field| fields >> field & 1 != 0).collect())
    }

    fn compares_previous(table: TableId) -> bool {
        MOVING.iter().any(|(moving, _)| *moving == table)
    }

    fn plan_changes(stage: usize, edits: &Edits, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> bool {
        let terminal = at.context.judged.lock().unwrap_or_else(|error| error.into_inner()).stage;
        match stage {
            0 => false,
            1 => terminal > 2 || !structure_holds(held, edits) || (terminal == 1 && !judged_through(at.context, held, at.since, edits)),
            _ if stage < terminal => true,
            _ => !(moving_only(edits) && judged_through(at.context, held, at.since, edits)),
        }
    }

    fn valid_through(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        let judged = at.context.judged.lock().unwrap_or_else(|error| error.into_inner());
        if judged.at == Some((frame.branch, frame.hour)) { judged.through } else { frame.hour }
    }

    fn valid_reply_through(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, _reply: &Self::Reply, partial: bool) -> i32 {
        if partial || TABLES.into_iter().any(|table| matches!(frame.tables[table.index()].held(), replay_frame::frame::TableView::NotRecorded { .. })) {
            at.hour
        } else {
            Self::valid_through(frame, params, at)
        }
    }

    fn advance(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>, edits: &Edits, _live: &dashboard_follow::live::Live, data: &mut dashboard_base::products::at_hour::Held<'_>) -> io::Result<bool> {
        Ok(data["missing"].is_null() && structure_holds(frame, edits) && law_stands(at.context, frame, edits))
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(Content::load(source, load))
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        reply(frame, at.context, at.live)
    }
}
