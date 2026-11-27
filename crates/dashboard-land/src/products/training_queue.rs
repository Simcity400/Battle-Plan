
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::LoadContext;
use super::divisions::{Content, Refusals};
use dashboard_base::{Source, catalogue::{self, Catalogue, View}, products::at_hour::{At, AtHour, Edits, Held, Keys, Owner, Read, ScriptedInputs, field_columns}};
use content_reader_layers::content::ByMode;
use dashboard_text::{
    metadata::link_slot,
    scripted_text::ScriptedText,
    subunit_requirements::Sources,
};
use replay_data::rows::{DeploymentLineRow, ProvinceRow, TableId, tables};
use dashboard_query::coverage::{witnessed_rows, witnessed_selection};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, OnceLock, Weak},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const QUEUE: [TableId; 4] = [TableId::DeploymentConveyors, TableId::DeploymentLines, TableId::DeploymentManpower, TableId::DeploymentEquipment];
const ORDERS: [TableId; 2] = [TableId::DeploymentConveyors, TableId::DeploymentLines];
const NAMING: [TableId; 10] = [
    TableId::Countries,
    TableId::CountryAppearance,
    TableId::CountryLinks,
    TableId::Provinces,
    TableId::States,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::LexicalTokens,
    TableId::CountryLocalSubunits,
    TableId::VariantDefinitions,
];

const CONVEYOR_VIEW: &str = "military_deployment_conveyor_view";
const PRIORITY_BUTTONS: [&str; 3] = ["low_prio", "medium_prio", "high_prio"];

pub struct TrainingQueue;

type PreparedPriorities = ByMode<OnceLock<(Priorities, catalogue::OwnRefusals)>>;

#[derive(Default)]
struct PriorityCache {
    catalogue: Weak<Catalogue>,
    prepared: Arc<PreparedPriorities>,
}

pub struct Context {
    names: Content,
    inputs: ScriptedInputs,
    priorities: Arc<PreparedPriorities>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingQueueParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TrainingManpower {
    pub tag: Option<String>,
    #[doc = " Available (false) or needed (true)."]
    pub needed: bool,
    pub amount: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TrainingEquipment {
    pub variant: String,
    #[doc = " The design's saved name; none where the design keeps the game's generated name."]
    pub name: Option<String>,
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
    #[doc = " Signed fixed-point amount with five decimals."]
    pub amount_raw: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TrainingNeed {
    pub archetype: String,
    pub name: Option<String>,
    #[doc = " Fixed-point need with five decimals."]
    pub need_raw: String,
    #[doc = " What the division holds of it, fixed-point with five decimals."]
    pub held_raw: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TrainingDivision {
    pub line: String,
    pub ordinal: u32,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    #[doc = " Fixed-point training and its maximum, five decimals."]
    pub training_raw: String,
    pub max_training_raw: String,
    #[doc = " Signed sums of the available and needed entries, as the game adds them; none with `manpower_missing` when the entries were not read whole."]
    pub manpower: Option<i32>,
    pub manpower_needed: Option<i32>,
    pub manpower_missing: Option<String>,
    pub max_manpower: i32,
    #[doc = " The manpower entries that were read."]
    pub contributors: Vec<TrainingManpower>,
    #[doc = " The held equipment that was read; `comparison_missing` says when it is not all of it."]
    pub equipment: Vec<TrainingEquipment>,
    #[doc = " What the template needs; empty with `need_missing` when unknown."]
    pub need: Vec<TrainingNeed>,
    pub need_missing: Option<String>,
    #[doc = " Why held equipment cannot be measured against the need."]
    pub comparison_missing: Option<String>,
    #[doc = " Held equipment as a share of the template's need, fixed-point with five decimals (100,000 is all of it):"]
    #[doc = " each type counts up to its need. None where it cannot be measured or nothing is needed."]
    pub equipment_fill_raw: Option<String>,
    #[doc = " A halting reason is stored; its meaning is not named."]
    pub halted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TrainingOrder {
    pub order: String,
    pub ordinal: u32,
    pub template: String,
    pub template_name: Option<String>,
    pub template_missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub amount: i32,
    #[doc = " The stored priority, and the game's text for it."]
    pub priority: u32,
    pub priority_name: Option<String>,
    pub priority_missing: Option<String>,
    #[doc = " The stored province (zero for none), and the name the game shows for it."]
    pub location: u32,
    pub location_name: Option<String>,
    pub location_missing: Option<String>,
    pub closed: bool,
    #[doc = " A government in exile's order names that country."]
    pub exile: Option<String>,
    pub divisions: Vec<TrainingDivision>,
    #[doc = " The divisions' available and needed manpower, added; none when any division's is not known."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub manpower: Option<i64>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "number | null"))]
    pub manpower_needed: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TrainingQueueReply {
    #[serde(skip)]
    text_through: i32,
    pub tag: String,
    #[doc = " The read the queue is from; none when none falls within a day."]
    pub read_hour: Option<i32>,
    #[doc = " Why the queue is unknown at this hour, when it is."]
    pub missing: Option<String>,
    pub orders: Vec<TrainingOrder>,
    #[doc = " Divisions in training across every order; none when the queue is unknown."]
    pub in_training: Option<u32>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(TrainingQueueReply);

#[cfg(test)]
mod tests;

const FILL_SCALE: i128 = 100_000;

fn equipment_fill(need: &[TrainingNeed]) -> Option<i128> {
    let (mut held, mut needed) = (0i128, 0i128);
    for entry in need {
        let amount: i128 = entry.need_raw.parse().ok()?;
        if amount <= 0 {
            continue;
        }
        let have: i128 = entry.held_raw.parse().ok()?;
        needed += amount;
        held += have.clamp(0, amount);
    }
    (needed > 0).then(|| held * FILL_SCALE / needed)
}

fn localised(cat: &Result<Arc<Catalogue>, String>, key: &str) -> Result<String, String> {
    let cat = cat.as_ref().map_err(Clone::clone)?;
    cat.localised(key, View::Plain).ok_or_else(|| format!("{key} has no localised name")).and_then(|text| catalogue::plain_text(&text))
}

fn unavailable(frame: &DecodedFrame, id: TableId) -> Option<String> {
    dashboard_text::scope::unread(id, &frame.tables[id.index()])
}

fn withheld(frame: &DecodedFrame) -> Option<String> {
    ORDERS.iter().find_map(|&id| unavailable(frame, id))
}

fn comparison_missing(need_missing: Option<&str>, equipment_missing: Option<&str>, unclassified: bool) -> Option<String> {
    need_missing.or(equipment_missing).map(str::to_owned).or_else(|| unclassified.then(|| "A held design's equipment type is unresolved".to_owned()))
}

fn template_name(frame: &DecodedFrame, handle: u64, cat: &Result<Arc<Catalogue>, String>) -> Result<String, String> {
    let cat = cat.as_ref().map_err(|_| "Template name needs the recorded content stack".to_owned())?;
    dashboard_text::template_facts::display_naming(frame, handle)
        .and_then(|naming| naming.localise(&cat.localisation))
        .and_then(|name| catalogue::plain_text(&name))
        .map_err(|why| format!("Template name unavailable: {why}"))
}

enum PriorityName {
    Fixed(Result<String, String>),
    Scripted(String),
}

struct Priorities {
    names: [PriorityName; PRIORITY_BUTTONS.len()],
    notes: BTreeSet<String>,
}

impl Priorities {
    fn unavailable(reason: &str) -> Self {
        Self { names: std::array::from_fn(|_| PriorityName::Fixed(Err(reason.to_owned()))), notes: BTreeSet::new() }
    }

    fn load(cat: &Result<Arc<Catalogue>, String>) -> Self {
        let Ok(cat) = cat else {
            return Self::unavailable("Names need the recorded content stack");
        };
        let guis = match cat.guis() {
            Ok(guis) => guis,
            Err(why) => return Self::unavailable(&format!("Deployment priority names unavailable: {why}")),
        };
        let scripted = cat.scripted_content().is_ok();
        let mut notes = BTreeSet::new();
        let names = PRIORITY_BUTTONS.map(|button| {
            let mut warnings = Vec::new();
            let key = content_reader_gui::gui::native::window_button_tooltip(guis, CONVEYOR_VIEW, button, &mut warnings);
            notes.extend(warnings);
            let key = match key {
                Ok(key) => key,
                Err(why) => return PriorityName::Fixed(Err(format!("Deployment priority name unavailable: {why}"))),
            };
            if cat.localisation.get(&key).is_none() {
                return PriorityName::Fixed(Err(format!("{key} has no localised name")));
            }
            if scripted && cat.localisation.localise(key.as_bytes()).is_ok_and(|text| text.contains('[')) {
                return PriorityName::Scripted(key);
            }
            PriorityName::Fixed(dashboard_text::catalogue::localised(&cat.localisation, &key, View::Scripted)
                .map_err(|why| format!("Deployment priority name refused: {why}")))
        });
        Self { names, notes }
    }

    fn resolve(&self, cat: &Result<Arc<Catalogue>, String>, frame: &DecodedFrame, root: u32) -> [Result<String, String>; PRIORITY_BUTTONS.len()] {
        let scripted = std::cell::OnceCell::new();
        self.names.each_ref().map(|name| match name {
            PriorityName::Fixed(text) => text.clone(),
            PriorityName::Scripted(key) => {
                let cat = cat.as_ref().map_err(Clone::clone)?;
                let text = scripted.get_or_init(|| cat.scripted_content().map(|content| ScriptedText::new(frame, &cat.localisation, content, root)));
                text.as_ref().map_err(Clone::clone)
                    .and_then(|text| dashboard_text::catalogue::localised_in(&cat.localisation, key, &[], text))
                    .map_err(|why| format!("Deployment priority name refused: {why}"))
            }
        })
    }

    fn tables(&self, located: bool) -> &'static [TableId] {
        if self.names.iter().any(|name| matches!(name, PriorityName::Scripted(_))) {
            dashboard_text::scripted_text::TABLES
        } else if located {
            &[TableId::States]
        } else {
            &[]
        }
    }
}

fn location(frame: &DecodedFrame, cat: &Result<Arc<Catalogue>, String>, province: u32, provinces: &io::Result<&[ProvinceRow]>) -> Result<String, String> {
    if province == 0 {
        return localised(cat, "CONVEYOR_SET_LOCATION_NOT_SET");
    }
    let cat = cat.as_ref().map_err(|_| "Location names need the recorded content stack".to_owned())?;
    let provinces = provinces.as_ref().map_err(|why| format!("Provinces were not read: {why}"))?;
    let row = provinces.binary_search_by_key(&province, |row| row.id).ok().map(|at| &provinces[at]).ok_or_else(|| format!("Province {province} was not read"))?;
    let text = ScriptedText::new(frame, &cat.localisation, cat.scripted_content()?, 0);
    text.plain_state_name(row.state).and_then(|name| catalogue::plain_text(&name))
}

fn design(frame: &DecodedFrame, names: &Content, units: Option<&content_reader_key_names::equipment::Database>, variant: u64, refusals: &mut Refusals) -> (Option<String>, Option<String>, Option<String>) {
    let cat = &names.cat;
    let Ok(rows) = frame.complete::<tables::VariantDefinitions>() else {
        refusals.note("Design definitions were not read at this hour");
        return (None, None, None);
    };
    let Some(row) = rows.iter().find(|row| row.id == variant) else {
        refusals.note(format!("Design {variant} has no definition at this hour"));
        return (None, None, None);
    };
    let name = refusals.keep(names.design_name(frame, row)).0;
    let token = frame.complete::<tables::LexicalTokens>().ok().and_then(|tokens| {
        let at = tokens.binary_search_by_key(&row.equipment_token, |token| token.token).ok()?;
        std::str::from_utf8(content_reader_parser::parser::c_string(&tokens[at].literal)).ok().map(str::to_owned)
    });
    let archetype = token.as_deref().zip(units).and_then(|(token, units)| {
        let ordinal = units.named(token)?.archetype?;
        units.equipment[ordinal].name_str().ok().map(str::to_owned)
    });
    if archetype.is_none() {
        refusals.note(format!("Design {variant}'s equipment type is unresolved"));
    }
    let archetype_name = archetype.as_deref().and_then(|key| localised(cat, key).ok());
    (name, archetype, archetype_name)
}

fn queue_sample(frame: &DecodedFrame) -> Option<i32> {
    frame.tables[TableId::DeploymentLines.index()].sampled_at().filter(|&read|
        read <= frame.hour && i64::from(frame.hour) - i64::from(read) < i64::from(replay_data::cadence::STAGGER_PERIOD))
}

fn queue_through(frame: &DecodedFrame) -> i32 {
    match frame.tables[TableId::DeploymentLines.index()].sampled_at() {
            Some(read) if read > frame.hour => read - 1,
            Some(read) if i64::from(frame.hour) - i64::from(read) < i64::from(replay_data::cadence::STAGGER_PERIOD) => {
                read.saturating_add(replay_data::cadence::STAGGER_PERIOD - 1)
            }
            _ => i32::MAX,
        }
}

impl AtHour for TrainingQueue {
    const KEY: &'static str = "training-queue";
    const TITLE: &'static str = "Training queue";
    const CONTENT_DEPENDENT: bool = true;
    type Params = TrainingQueueParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = TrainingQueueReply;
    type Context = Context;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            TrainingQueueParams::decl(),
            TrainingQueueReply::decl(),
            TrainingOrder::decl(),
            TrainingDivision::decl(),
            TrainingManpower::decl(),
            TrainingEquipment::decl(),
            TrainingNeed::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_country(&params.country)
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = QUEUE.to_vec();
        tables.extend(NAMING);
        tables.extend_from_slice(dashboard_text::scripted_text::TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    fn reads_in(_params: &Self::Params, owner: Option<Owner>, stage: usize, frame: &DecodedFrame, at: &dashboard_base::products::at_hour::ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match stage {
            0 => ORDERS.map(|table| Read::these(table, [])).into(),
            1 if withheld(frame).is_none() && queue_sample(frame).is_some() => [TableId::CountryLinks, TableId::DeploymentConveyors, TableId::DeploymentLines].map(Read::all).into(),
            2 => {
                let links = frame.complete::<tables::CountryLinks>().unwrap_or(&[]);
                let conveyors = frame.complete::<tables::DeploymentConveyors>().unwrap_or(&[]);
                let orders: BTreeSet<_> = conveyors.iter().filter(|row| link_slot(links, row.country) == Some(owner.slot)).map(|row| row.handle).collect();
                let owned = || conveyors.iter().filter(|row| orders.contains(&row.handle));
                let templates: BTreeSet<_> = owned().map(|row| u128::from(row.template)).collect();
                let lines = frame.complete::<tables::DeploymentLines>().unwrap_or(&[]);
                let lines: Vec<_> = lines.iter().filter(|line| orders.contains(&line.conveyor)).map(|line| u128::from(line.handle)).collect();
                let parts = |shift| Keys::Ranges(lines.iter().map(|&line| (line << shift, (line << shift) | ((1_u128 << shift) - 1))).collect());
                let mut reads = vec![
                    Read::all(TableId::Countries), Read::all(TableId::CountryAppearance), Read::all(TableId::LexicalTokens), Read::all(TableId::CountryLocalSubunits),
                    Read::these(TableId::DivisionTemplates, templates.iter().copied()), Read::these(TableId::DivisionTemplateIcons, templates),
                    Read { table: TableId::DeploymentManpower, keys: parts(33_u32) }, Read { table: TableId::DeploymentEquipment, keys: parts(32_u32) },
                ];
                if at.context.names.cat.is_ok() {
                    let locations = owned().map(|order| order.location).filter(|&id| id != 0).map(u128::from);
                    let mut locations = locations.peekable();
                    if locations.peek().is_some() { reads.push(Read::these(TableId::Provinces, locations)); }
                }
                reads
            }
            3 => {
                let rows = witnessed_selection::<tables::DeploymentEquipment>(frame);
                vec![Read::these(TableId::VariantDefinitions, rows.rows.iter().map(|row| u128::from(row.variant_handle)))]
            }
            _ => Vec::new(),
        }
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        let fields: &[&str] = match table {
            TableId::DeploymentConveyors => &["country", "template", "ordinal", "amount", "location", "priority", "closed", "exile_country"],
            TableId::DeploymentLines => &["conveyor", "ordinal", "name_order", "training_raw", "max_training_raw", "max_manpower", "halting_reason_present", "custom_name", "custom_flag_a", "custom_flag_b"],
            TableId::Provinces => &["state"],
            TableId::DivisionTemplateIcons => return None,
            _ => return super::divisions::naming_columns(table),
        };
        Some(field_columns(table, fields))
    }

    fn retained_reads(params: &Self::Params, owner: Option<Owner>, stage: usize, frame: &DecodedFrame, at: &dashboard_base::products::at_hour::ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        let through = if stage == 1 { queue_through(frame) } else { i32::MAX };
        (Self::reads_in(params, owner, stage, frame, at), through)
    }

    fn plan_changes(stage: usize, edits: &Edits, _frame: &DecodedFrame, _at: &dashboard_base::products::at_hour::ReadsAt<'_, Self::Context>) -> bool {
        match stage {
            1 => ORDERS.iter().any(|table| edits.replaced.contains(table)),
            2 => edits.fields(TableId::CountryLinks, u64::MAX) || edits.fields(TableId::DeploymentConveyors, dashboard_base::products::at_hour::field_mask(TableId::DeploymentConveyors, &["country", "template", "location"]))
                || edits.fields(TableId::DeploymentLines, dashboard_base::products::at_hour::field_mask(TableId::DeploymentLines, &["conveyor"])),
            3 => edits.fields(TableId::DeploymentEquipment, dashboard_base::products::at_hour::field_mask(TableId::DeploymentEquipment, &["variant_handle"])),
            _ => false,
        }
    }

    fn compares_previous(table: TableId) -> bool {
        matches!(table, TableId::DeploymentLines | TableId::DeploymentConveyors)
    }

    fn advance(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>, edits: &Edits, _live: &dashboard_follow::live::Live, _held: &mut Held<'_>) -> io::Result<bool> {
        if !edits.replaced.is_empty() || edits.tables.keys().any(|table| !matches!(table, TableId::DeploymentLines | TableId::DeploymentConveyors)) { return Ok(false); }
        let Some(owner) = at.owner else { return Ok(false) };
        let links = frame.complete::<tables::CountryLinks>()?;
        let orders = frame.complete::<tables::DeploymentConveyors>()?;
        let mine = |order: &replay_data::rows::DeploymentConveyorRow| link_slot(links, order.country) == Some(owner.slot);
        if let Some(keys) = edits.tables.get(&TableId::DeploymentConveyors) {
            let prior = edits.previous.get(&TableId::DeploymentConveyors).and_then(|v| match v.held() { replay_frame::frame::TableView::Complete(rows) => rows.rows::<tables::DeploymentConveyors>(), _ => None });
            let relevant = |rows: &[replay_data::rows::DeploymentConveyorRow]| keys.keys().any(|&key| rows.binary_search_by_key(&(key as u64), |o| o.handle).is_ok_and(|i| mine(&rows[i])));
            if prior.is_none_or(relevant) || relevant(orders) { return Ok(false); }
        }
        if let Some(keys) = edits.tables.get(&TableId::DeploymentLines) {
            let prior = edits.previous.get(&TableId::DeploymentLines).and_then(|v| match v.held() { replay_frame::frame::TableView::Complete(rows) => rows.rows::<tables::DeploymentLines>(), _ => None });
            let relevant = |rows: &[DeploymentLineRow]| keys.keys().any(|&key| rows.binary_search_by_key(&(key as u64), |line| line.handle).is_ok_and(|i|
                orders.binary_search_by_key(&rows[i].conveyor, |order| order.handle).is_ok_and(|j| mine(&orders[j]))));
            if prior.is_none_or(relevant) || relevant(frame.complete::<tables::DeploymentLines>()?) { return Ok(false); }
        }
        Ok(true)
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let names = Content::load(source, load);
        let catalogue = names.cat.as_ref().map(Arc::downgrade).unwrap_or_default();
        let cache = source.state::<PriorityCache>();
        let mut cache = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !Weak::ptr_eq(&cache.catalogue, &catalogue) {
            *cache = PriorityCache { catalogue, ..PriorityCache::default() };
        }
        Ok(Context { names, inputs: ScriptedInputs::new(source.replay()), priorities: Arc::clone(&cache.prepared) })
    }

    fn valid_through(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>) -> i32 {
        queue_through(frame)
    }

    fn valid_reply_through(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, reply: &Self::Reply, _partial: bool) -> i32 {
        Self::valid_through(frame, params, at)
            .min(reply.text_through)
            .min(super::divisions::names_through(frame, reply.orders.iter().map(|order| order.template.as_str()), &at.context.names))
            .min(dashboard_base::products::at_hour::warnings_through(frame.hour, reply.missing.iter().chain(&reply.warnings).map(String::as_str)))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Training queue country is unavailable at this hour"))?;
        let mut reply = TrainingQueueReply { text_through: i32::MAX, tag: at.holders.name(&params.country, at.hour), read_hour: None, missing: None, orders: Vec::new(), in_training: None, warnings: Vec::new() };
        if let Some(why) = withheld(frame) {
            reply.missing = Some(why);
            return Ok(reply);
        }
        let Some(read) = queue_sample(frame) else {
            reply.missing = Some("The training queue was not read within a day of this hour".into());
            return Ok(reply);
        };
        reply.read_hour = Some(read);
        let mut refusals = Refusals::default();
        let Context { names, inputs, priorities } = at.context;
        let cat = &names.cat;
        names.group_notes(&mut refusals);
        let links = frame.complete::<tables::CountryLinks>()?;
        let countries = witnessed_rows::<tables::Countries>(frame);
        if let Some(why) = countries.missing {
            refusals.note(format!("Country names unavailable: {why}"));
        }
        let tags: BTreeMap<u32, String> = countries.rows.iter().filter_map(|row| Some((row.canonical_slot, String::from_utf8(row.tag.clone()).ok()?))).collect();
        let tag_of = |raw: u32| tags.get(&link_slot(links, raw)?).cloned();
        let content = super::divisions::equipment::template_needs(cat);
        if let Ok(content) = &content {
            for warning in content.dropped.iter().chain(&content.equipment_content().misreads) {
                refusals.note(warning.clone());
            }
        }
        let sources = Sources::read(frame).map_err(|why| format!("Template equipment inputs unavailable: {why}"));
        let units = cat.as_ref().ok().and_then(|cat| catalogue::equipment_units(cat).ok());
        let conveyors = frame.complete::<tables::DeploymentConveyors>()?;
        let lines = frame.complete::<tables::DeploymentLines>()?;
        let manpower_read = witnessed_selection::<tables::DeploymentManpower>(frame);
        let equipment_read = witnessed_selection::<tables::DeploymentEquipment>(frame);
        let (manpower, manpower_missing) = (manpower_read.rows, manpower_read.missing.map(|why| format!("Training manpower not read whole: {why}")));
        let (equipment, equipment_missing) = (equipment_read.rows, equipment_read.missing.map(|why| format!("Held equipment not read whole: {why}")));
        let icons = frame.complete::<tables::DivisionTemplateIcons>().ok();
        let mut orders: Vec<_> = conveyors.iter().filter(|row| link_slot(links, row.country) == Some(owner.slot)).collect();
        let provinces = frame.complete::<tables::Provinces>();
        let (priorities, diagnostics) = priorities.current().get_or_init(|| catalogue::capture(|| Priorities::load(cat)));
        catalogue::credit(diagnostics);
        for note in &priorities.notes {
            refusals.note(note.clone());
        }
        let text = |world: &DecodedFrame| {
            let names = priorities.resolve(cat, world, owner.slot);
            let locations: BTreeMap<_, _> = orders.iter().map(|order| order.location).collect::<BTreeSet<_>>().into_iter()
                .map(|province| (province, location(world, cat, province, &provinces))).collect();
            (names, locations)
        };
        let tables = priorities.tables(cat.is_ok() && orders.iter().any(|order| order.location != 0));
        let ((priority_names, locations), through) = if tables.is_empty() {
            (text(frame), i32::MAX)
        } else {
            inputs.evaluate(frame, tables, at.cancelled, text)?
        };
        reply.text_through = through;
        orders.sort_by_key(|row| (row.ordinal, row.handle));
        let mut by_order = BTreeMap::<u64, Vec<&DeploymentLineRow>>::new();
        for line in lines {
            by_order.entry(line.conveyor).or_default().push(line);
        }
        for order in orders {
            dashboard_text::metadata::check_cancel(at.cancelled)?;
            let (template_name, template_missing) = refusals.keep(template_name(frame, order.template, cat));
            let group = icons
                .and_then(|rows| rows.binary_search_by_key(&order.template, |row| row.template).ok().map(|at| &rows[at]))
                .and_then(|row| row.settings.require("template settings").ok().cloned().flatten())
                .and_then(|settings| settings.names_group.clone());
            let need = super::divisions::equipment::template_need(&content, &sources, frame, order.template);
            let (icon, icon_missing) = refusals.keep(names.template_icon(frame, order.template));
            let mut divisions = Vec::new();
            let mut members = by_order.remove(&order.handle).unwrap_or_default();
            members.sort_by_key(|line| (line.ordinal, line.handle));
            for line in members {
                let name = names.next_name(&line.custom_name, line.custom_flag_b != 0, group.as_deref(), line.name_order, line.custom_flag_a != 0);
                let (name, name_missing) = refusals.keep(name);
                let entries = |side: u8| {
                    let start = manpower.partition_point(|row| (row.line, row.side) < (line.handle, side));
                    manpower[start..].iter().take_while(move |row| row.line == line.handle && row.side == side)
                };
                let sum = |side: u8| entries(side).fold(0i32, |sum, row| sum.wrapping_add(row.amount));
                let contributors = entries(0).chain(entries(1)).map(|row| TrainingManpower { tag: tag_of(row.source_country), needed: row.side == 1, amount: row.amount }).collect();
                let start = equipment.partition_point(|row| row.line < line.handle);
                let held: Vec<TrainingEquipment> = equipment[start..]
                    .iter()
                    .take_while(|row| row.line == line.handle)
                    .map(|row| {
                        let (name, archetype, archetype_name) = design(frame, names, units.as_deref(), row.variant_handle, &mut refusals);
                        TrainingEquipment { variant: row.variant_handle.to_string(), name, archetype, archetype_name, amount_raw: row.amount_raw.to_string() }
                    })
                    .collect();
                let unclassified = held.iter().any(|row| row.archetype.is_none());
                let (need, need_missing) = match &need {
                    Ok(need) => (
                        need.iter()
                            .map(|(archetype, amount)| TrainingNeed {
                                archetype: archetype.clone(),
                                name: localised(cat, archetype).ok(),
                                need_raw: amount.to_string(),
                                held_raw: held.iter().filter(|row| row.archetype.as_deref() == Some(archetype)).fold(0i128, |sum, row| sum + row.amount_raw.parse::<i128>().unwrap_or(0)).to_string(),
                            })
                            .collect(),
                        None,
                    ),
                    Err(why) => {
                        refusals.note(why.clone());
                        (Vec::new(), Some(why.clone()))
                    }
                };
                let comparison_missing = comparison_missing(need_missing.as_deref(), equipment_missing.as_deref(), unclassified);
                let equipment_fill_raw = if comparison_missing.is_none() { equipment_fill(&need).map(|fill| fill.to_string()) } else { None };
                divisions.push(TrainingDivision {
                    line: line.handle.to_string(),
                    ordinal: line.ordinal,
                    name,
                    name_missing,
                    training_raw: line.training_raw.to_string(),
                    max_training_raw: line.max_training_raw.to_string(),
                    manpower: manpower_missing.is_none().then(|| sum(0)),
                    manpower_needed: manpower_missing.is_none().then(|| sum(1)),
                    manpower_missing: manpower_missing.clone(),
                    max_manpower: line.max_manpower,
                    contributors,
                    equipment: held,
                    need,
                    need_missing,
                    comparison_missing,
                    equipment_fill_raw,
                    halted: line.halting_reason_present != 0,
                });
            }
            let priority = usize::try_from(order.priority).ok().and_then(|at| priority_names.get(at).cloned()).unwrap_or_else(|| Err(format!("Deployment priority {} has no proved name", order.priority)));
            let (priority_name, priority_missing) = refusals.keep(priority);
            let (location_name, location_missing) = refusals.keep(locations[&order.location].clone());
            let manpower_total = divisions.iter().map(|division| division.manpower.map(i64::from)).sum();
            let needed_total = divisions.iter().map(|division| division.manpower_needed.map(i64::from)).sum();
            reply.orders.push(TrainingOrder {
                order: order.handle.to_string(),
                ordinal: order.ordinal,
                template: order.template.to_string(),
                template_name,
                template_missing,
                icon,
                icon_missing,
                amount: order.amount,
                priority: order.priority,
                priority_name,
                priority_missing,
                location: order.location,
                location_name,
                location_missing,
                closed: order.closed != 0,
                exile: order.exile_country.and_then(tag_of),
                divisions,
                manpower: manpower_total,
                manpower_needed: needed_total,
            });
        }
        reply.in_training = Some(reply.orders.iter().map(|order| order.divisions.len() as u32).sum());
        reply.warnings = refusals.lines();
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
