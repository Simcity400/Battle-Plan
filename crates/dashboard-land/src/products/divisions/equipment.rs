
use dashboard_base::products::{Country, validate_country};
use super::{ArmyAttribution, Content, Refusals, design_handle, find, side, templates::slot_side_range, token_key, unavailable};
use dashboard_base::{Source, catalogue, products::{LoadContext, at_hour::{At, AtHour, Keys, Owner, Read}}};
use dashboard_text::subunit_requirements::{Content as Needs, Sources};
use replay_data::rows::{DivisionEquipmentCountRow, DivisionEquipmentDesignRow, DivisionEquipmentTemplateRow, DivisionEquipmentTypeRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const NAMING: [TableId; 4] = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens];
const TYPES: [TableId; 4] = [TableId::VariantDefinitions, TableId::DivisionEquipmentCounts, TableId::DivisionEquipmentByType, TableId::DivisionEquipmentByDesign];
const TEMPLATES: [TableId; 4] = [TableId::DivisionTemplates, TableId::DivisionTemplateIcons, TableId::DivisionEquipmentCounts, TableId::DivisionEquipmentByTemplate];
const UNCLASSIFIED: &str = "unclassified";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum DivisionEquipmentView {
    Types,
    Templates,
}

impl DivisionEquipmentView {
    fn operands(self) -> &'static [TableId] {
        match self {
            Self::Types => &TYPES,
            Self::Templates => &TEMPLATES,
        }
    }

    fn withheld_by(self) -> &'static [TableId] {
        match self {
            Self::Types => &[TableId::DivisionEquipmentCounts, TableId::DivisionEquipmentByType, TableId::DivisionEquipmentByDesign],
            Self::Templates => &[TableId::DivisionEquipmentCounts, TableId::DivisionEquipmentByTemplate],
        }
    }
}

pub(super) fn type_key(token: Option<u32>) -> String {
    token.map_or_else(|| UNCLASSIFIED.to_owned(), |token| token.to_string())
}


pub struct DivisionEquipment;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionEquipmentParams {
    pub branch_id: String,
    pub country: Country,
    pub count_for: ArmyAttribution,
    #[doc = " The view whose tables are read and returned: the other view's are neither read nor withheld for."]
    pub view: DivisionEquipmentView,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentVariant {
    pub variant: String,
    pub name: Option<String>,
    pub held_raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentType {
    #[doc = " The recorded type token, the row's identity; `unclassified` for equipment whose design names no type."]
    pub key: String,
    pub unclassified: bool,
    #[doc = " The type's content key; none when unclassified or when its token's spelling was not read (`name_missing` says which)."]
    pub archetype: Option<String>,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    #[doc = " What every read pool holds, its comparison known or not; none when not known."]
    pub held_raw: Option<String>,
    pub needed_raw: Option<String>,
    #[doc = " Each known division's holding capped at its own need, summed."]
    pub filled_raw: Option<String>,
    pub missing_raw: Option<String>,
    pub divisions_short: Option<u32>,
    pub unknown_divisions: Option<u32>,
    #[doc = " National stock, never used in the shortfall."]
    pub stockpile_raw: Option<String>,
    pub stockpile_hour: Option<i32>,
    pub variants: Vec<EquipmentVariant>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentTemplate {
    pub template: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub divisions: Option<u32>,
    pub manpower: Option<String>,
    pub manpower_required: Option<String>,
    pub hp_raw: Option<String>,
    pub hp_max_raw: Option<String>,
    #[doc = " Known divisions only, `unknown_divisions` naming the rest."]
    pub needed_raw: Option<String>,
    #[doc = " Needed archetypes only, each division capped at its own need."]
    pub held_raw: Option<String>,
    pub types_short: Option<u32>,
    pub unknown_divisions: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionEquipmentReply {
    pub tag: String,
    pub count_for: ArmyAttribution,
    #[doc = " Why the country's division equipment is not known at this hour, when it is not."]
    pub missing: Option<String>,
    pub pool_hour: Option<i32>,
    pub divisions: Option<u32>,
    #[doc = " Divisions with a known shortfall of any type."]
    pub divisions_short: Option<u32>,
    pub unknown_divisions: Option<u32>,
    pub types: Vec<EquipmentType>,
    pub templates: Vec<EquipmentTemplate>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DivisionEquipmentReply);

pub struct EquipmentContext {
    pub content: Content,
}

impl AtHour for DivisionEquipment {
    const KEY: &'static str = "division-equipment";
    const TITLE: &'static str = "Division equipment";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = DivisionEquipmentParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, countFor, view];
    type Reply = DivisionEquipmentReply;
    type Context = EquipmentContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            DivisionEquipmentParams::decl(),
            DivisionEquipmentReply::decl(),
            EquipmentVariant::decl(),
            EquipmentType::decl(),
            EquipmentTemplate::decl(),
            DivisionEquipmentView::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_country(&params.country)?;
        params.branch_id.parse::<u64>().map(drop).map_err(|_| "Invalid branch")
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let mut tables = [&NAMING[..], params.view.operands()].concat();
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    fn reads(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        let side = side(params.count_for);
        let slot = u128::from(owner.slot);
        let side128 = u128::from(side);
        match step {
            0 => {
                let (template_lo, template_hi) = slot_side_range(owner.slot, side);
                let types = slot << 41 | side128 << 33;
                let designs = slot << 74 | side128 << 66;
                let mut reads: Vec<Read> = NAMING.into_iter().map(Read::all).collect();
                reads.push(Read::these(TableId::DivisionEquipmentCounts, [slot << 8 | side128]));
                match params.view {
                    DivisionEquipmentView::Types => reads.extend([
                        Read { table: TableId::DivisionEquipmentByType, keys: Keys::Range(types, types | ((1 << 33) - 1)) },
                        Read { table: TableId::DivisionEquipmentByDesign, keys: Keys::Range(designs, designs | ((1 << 66) - 1)) },
                    ]),
                    DivisionEquipmentView::Templates => reads.push(Read { table: TableId::DivisionEquipmentByTemplate, keys: Keys::Range(template_lo, template_hi) }),
                }
                reads
            }
            1 => match params.view {
                DivisionEquipmentView::Templates => {
                    let templates = held.complete::<tables::DivisionEquipmentByTemplate>().map(|rows| rows.iter().map(|row| u128::from(row.template)).collect::<Vec<_>>()).unwrap_or_default();
                    vec![Read::these(TableId::DivisionTemplates, templates.clone()), Read::these(TableId::DivisionTemplateIcons, templates)]
                }
                DivisionEquipmentView::Types => {
                    let designs = held.complete::<tables::DivisionEquipmentByDesign>().map(|rows| rows.iter().filter_map(|row| row.variant).map(|id| u128::from(design_handle(id))).collect::<Vec<_>>()).unwrap_or_default();
                    vec![Read::these(TableId::VariantDefinitions, designs)]
                }
            },
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(EquipmentContext { content: Content::load(source, load) })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Division equipment country is unavailable at this hour"))?;
        let mut refusals = Refusals::default();
        let mut reply = overview(frame, params, at.holders.name(&params.country, at.hour), owner.slot, &at.context.content, &mut refusals)?;
        reply.warnings = refusals.lines();
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub fn template_needs(cat: &Result<Arc<catalogue::Catalogue>, String>) -> Result<Arc<Needs>, String> {
    cat.as_ref()
        .map_err(|_| "Template equipment needs require the recorded content stack".to_owned())
        .and_then(|cat| cat.content.layers.derived::<catalogue::CachedResult<Needs>>().get_or_init(|| Needs::load(&cat.content.layers)).map_err(|why| format!("Template equipment content unavailable: {why}")))
}

pub fn template_need(needs: &Result<Arc<Needs>, String>, sources: &Result<Sources<'_>, String>, frame: &DecodedFrame, handle: u64) -> Result<Vec<(String, i64)>, String> {
    let needs = needs.as_ref().map_err(Clone::clone)?;
    let sources = sources.as_ref().map_err(Clone::clone)?;
    let rows = frame.complete::<tables::DivisionTemplateIcons>().map_err(|why| format!("Template equipment composition unavailable: {why}"))?;
    let template = find(Some(rows), handle, |row| row.template).ok_or_else(|| format!("Template {handle} equipment composition was not recorded"))?;
    if template.model_unreadable != 0 {
        return Err(format!("Template {handle} grouped sub-units were not readable"));
    }
    let mut out = Vec::<(String, i64)>::new();
    let mut indices = BTreeMap::<String, usize>::new();
    for unit in template.subunits.iter().filter(|unit| unit.token != 0) {
        let group = needs.group(template, sources, unit.token, unit.local, unit.count).map_err(|why| format!("Template {handle} equipment need: {why}"))?;
        for (archetype, amount) in group.requirements.iter() {
            let overflow = || format!("Template {handle} {archetype} equipment need overflows fixed5");
            let amount = amount.checked_mul(i64::from(group.count)).ok_or_else(overflow)?;
            let index = *indices.entry(archetype.clone()).or_insert_with(|| {
                out.push((archetype.clone(), 0));
                out.len() - 1
            });
            out[index].1 = out[index].1.checked_add(amount).ok_or_else(overflow)?;
        }
    }
    Ok(out)
}

fn text(value: &Recorded<i64>) -> Option<String> {
    match value {
        Recorded::Value(value) => Some(value.to_string()),
        Recorded::NotRecorded => None,
    }
}

fn count(value: &Recorded<i64>) -> Option<u32> {
    match value {
        Recorded::Value(value) => u32::try_from(*value).ok(),
        Recorded::NotRecorded => None,
    }
}

pub(super) fn short_count(short: &Recorded<i64>, compared: bool) -> Option<u32> {
    if compared { count(short) } else { None }
}

fn times(value: &Recorded<i64>) -> usize {
    match value {
        Recorded::Value(value) => usize::try_from(*value).unwrap_or(0),
        Recorded::NotRecorded => 0,
    }
}

fn archetype(frame: &DecodedFrame, content: &Content, token: Option<u32>, refusals: &mut Refusals) -> (Option<String>, Option<String>, Option<String>) {
    let Some(token) = token else { return (None, None, Some("Equipment archetype is unclassified".into())) };
    let key = frame.complete::<tables::LexicalTokens>().map_err(|why| format!("Equipment names unavailable: {why}")).and_then(|rows| token_key(rows, token));
    match key {
        Ok(key) => {
            let (name, missing) = refusals.keep(content.localised(&key));
            (Some(key), name, missing)
        }
        Err(why) => {
            refusals.note(why.clone());
            (None, None, Some(why))
        }
    }
}

fn design(frame: &DecodedFrame, content: &Content, id: Option<u32>, refusals: &mut Refusals) -> Option<String> {
    let Some(id) = id else {
        refusals.note("A pool entry names no equipment design");
        return None;
    };
    let definition = frame.complete::<tables::VariantDefinitions>().ok().and_then(|rows| find(Some(rows), design_handle(id), |row| row.id));
    let Some(definition) = definition else {
        refusals.note("An equipment design has no recorded definition");
        return None;
    };
    refusals.keep(content.design_name(frame, definition)).0
}

fn overview(frame: &DecodedFrame, params: &DivisionEquipmentParams, tag: String, slot: u32, content: &Content, refusals: &mut Refusals) -> io::Result<DivisionEquipmentReply> {
    let mut reply = DivisionEquipmentReply {
        tag,
        count_for: params.count_for,
        missing: None,
        pool_hour: None,
        divisions: None,
        divisions_short: None,
        unknown_divisions: None,
        types: Vec::new(),
        templates: Vec::new(),
        warnings: Vec::new(),
    };
    if let Some(why) = params.view.withheld_by().iter().find_map(|&id| unavailable(frame, id)) {
        reply.missing = Some(format!("Division equipment was not recorded at this hour: {why}"));
        return Ok(reply);
    }
    let side = side(params.count_for);
    let mine = |row_slot: u32, row_side: u8| row_slot == slot && row_side == side;
    let counts = frame.complete::<tables::DivisionEquipmentCounts>()?.iter().find(|row: &&DivisionEquipmentCountRow| mine(row.slot, row.side));
    let Some(counts) = counts else {
        reply.divisions = Some(0);
        reply.divisions_short = Some(0);
        reply.unknown_divisions = Some(0);
        return Ok(reply);
    };
    reply.pool_hour = counts.pool_hour;
    reply.divisions = count(&counts.divisions);
    let none_compared = matches!((count(&counts.divisions), count(&counts.unknown)), (Some(divisions), Some(unknown)) if divisions > 0 && unknown >= divisions);
    reply.divisions_short = short_count(&counts.short, !none_compared);
    reply.unknown_divisions = count(&counts.unknown);
    refusals.note_times("Division equipment pool was not recorded within a day of this hour", times(&counts.unpooled));
    refusals.note_times("Division equipment design has no resolved archetype; holdings are unclassified", times(&counts.unclassified));
    refusals.note_times("A division's country has no recorded link; this attribution's counts are not known", usize::try_from(counts.unresolved).unwrap_or(0));
    if counts.stock_hour.is_none() {
        refusals.note("National equipment stockpile was not read within a day of this hour");
    }
    if params.view == DivisionEquipmentView::Templates {
        templates(frame, &mine, content, refusals, &mut reply)?;
        return Ok(reply);
    }
    let designs: Vec<&DivisionEquipmentDesignRow> = frame.complete::<tables::DivisionEquipmentByDesign>()?.iter().filter(|row| mine(row.slot, row.side)).collect();
    for row in frame.complete::<tables::DivisionEquipmentByType>()?.iter().filter(|row: &&DivisionEquipmentTypeRow| mine(row.slot, row.side)) {
        let (key, name, name_missing) = archetype(frame, content, row.archetype, refusals);
        let variants = designs
            .iter()
            .filter(|design| design.archetype == row.archetype)
            .map(|row| EquipmentVariant { variant: row.variant.map(design_handle).unwrap_or(0).to_string(), name: design(frame, content, row.variant, refusals), held_raw: text(&row.held) })
            .collect();
        let stock = text(&row.stock);
        reply.types.push(EquipmentType {
            key: type_key(row.archetype),
            unclassified: row.archetype.is_none(),
            archetype: key,
            name,
            name_missing,
            held_raw: text(&row.held),
            needed_raw: text(&row.needed),
            filled_raw: text(&row.filled),
            missing_raw: text(&row.missing),
            divisions_short: short_count(&row.short, row.needed != Recorded::NotRecorded),
            unknown_divisions: count(&counts.unknown),
            stockpile_hour: stock.as_ref().and(counts.stock_hour),
            stockpile_raw: stock,
            variants,
        });
    }
    Ok(reply)
}

fn templates(frame: &DecodedFrame, mine: &impl Fn(u32, u8) -> bool, content: &Content, refusals: &mut Refusals, reply: &mut DivisionEquipmentReply) -> io::Result<()> {
    for row in frame.complete::<tables::DivisionEquipmentByTemplate>()?.iter().filter(|row: &&DivisionEquipmentTemplateRow| mine(row.slot, row.side)) {
        let divisions = times(&row.divisions);
        if row.required == Recorded::NotRecorded {
            refusals.note_times("Template required manpower was not recorded", divisions);
        }
        refusals.note_times("Division manpower was not recorded", times(&row.manpower_lacking));
        refusals.note_times("Division maximum hit points were not recorded", times(&row.hp_max_lacking));
        if !row.need_missing.is_empty() {
            refusals.note_times(String::from_utf8_lossy(&row.need_missing).into_owned(), divisions);
        }
        let (name, name_missing) = refusals.keep(content.template_name(frame, row.template));
        let (icon, icon_missing) = refusals.keep(content.template_icon(frame, row.template));
        reply.templates.push(EquipmentTemplate {
            template: row.template.to_string(),
            name,
            name_missing,
            icon,
            icon_missing,
            divisions: count(&row.divisions),
            manpower: text(&row.manpower),
            manpower_required: text(&row.required),
            hp_raw: text(&row.hp),
            hp_max_raw: text(&row.hp_max),
            needed_raw: text(&row.needed),
            held_raw: text(&row.held),
            types_short: short_count(&row.types_short, row.needed != Recorded::NotRecorded),
            unknown_divisions: count(&row.unknown),
        });
    }
    Ok(())
}
