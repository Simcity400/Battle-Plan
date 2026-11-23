
use dashboard_base::products::{Country, validate_country};
use super::{ArmyAttribution, Content, Countries, Refusals, attributed, design_handle, find, side, templates::slot_side_range, token_key, unavailable};
use dashboard_base::{Source, catalogue, products::{LoadContext, at_hour::{At, AtHour, Keys, Owner, Read}}};
use dashboard_text::subunit_requirements::{Content as Needs, Sources};
use replay_data::rows::{DivisionEquipmentCountRow, DivisionEquipmentDesignRow, DivisionEquipmentRow, DivisionEquipmentTemplateRow, DivisionEquipmentTypeRow, DivisionRow, DivisionStatusRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const OVERVIEW: [TableId; 10] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::DivisionTemplateIcons,
    TableId::DivisionEquipmentCounts,
    TableId::DivisionEquipmentByType,
    TableId::DivisionEquipmentByDesign,
    TableId::DivisionEquipmentByTemplate,
];

const DRILL: [TableId; 8] = [
    TableId::Divisions,
    TableId::DivisionStatus,
    TableId::DivisionPanelStats,
    TableId::DivisionLandInputs,
    TableId::DivisionTemplates,
    TableId::CountryLocalSubunits,
    TableId::DivisionEquipment,
    TableId::NewDivisionEquipment,
];

pub struct DivisionEquipment;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionEquipmentParams {
    pub branch_id: String,
    pub country: Country,
    pub count_for: ArmyAttribution,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub division: Option<String>,
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
    #[doc = " None is the unclassified bucket, not a content key."]
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
pub struct EquipmentDivision {
    pub division: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub manpower: Option<String>,
    pub manpower_required: Option<String>,
    pub hp_raw: Option<String>,
    pub hp_max_raw: Option<String>,
    pub needed_raw: Option<String>,
    pub held_raw: Option<String>,
    #[doc = " None when the comparison with the need is not known."]
    pub types_short: Option<u32>,
    pub pool_known: bool,
    #[doc = " The previous template's name during conversion."]
    pub converting_from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionPoolType {
    pub archetype: Option<String>,
    pub name: Option<String>,
    #[doc = " None when the division's pool was not read."]
    pub held_raw: Option<String>,
    pub needed_raw: Option<String>,
    pub missing_raw: Option<String>,
    pub variants: Vec<EquipmentVariant>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DivisionPool {
    pub division: String,
    pub name: Option<String>,
    pub template: String,
    pub template_name: Option<String>,
    pub manpower: Option<String>,
    pub manpower_required: Option<String>,
    pub hp_raw: Option<String>,
    pub hp_max_raw: Option<String>,
    pub types: Vec<DivisionPoolType>,
    #[doc = " An unread pool lists template needs with unknown holdings, not an empty pool."]
    pub pool_known: bool,
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
    #[doc = " A template's divisions, when one is asked."]
    pub division_rows: Option<Vec<EquipmentDivision>>,
    #[doc = " A division's pool, when one is asked."]
    pub pool: Option<DivisionPool>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(DivisionEquipmentReply);

pub struct EquipmentContext {
    pub content: Content,
    needs: Result<Arc<Needs>, String>,
}

fn handle(text: &Option<String>) -> Option<u64> {
    text.as_ref().and_then(|text| text.parse::<u64>().ok())
}

impl AtHour for DivisionEquipment {
    const KEY: &'static str = "division-equipment";
    const TITLE: &'static str = "Division equipment";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DivisionEquipmentParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, countFor, template, division];
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
            EquipmentDivision::decl(),
            DivisionPoolType::decl(),
            DivisionPool::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_country(&params.country)?;
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if params.template.is_some() && params.division.is_some() {
            return Err("Choose a template or a division, not both");
        }
        for text in [&params.template, &params.division].into_iter().flatten() {
            if text.parse::<u64>().ok().filter(|handle| *handle != 0).is_none() {
                return Err("Invalid full unit or template handle");
            }
        }
        Ok(())
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let mut tables = OVERVIEW.to_vec();
        if params.template.is_some() || params.division.is_some() {
            tables.extend(DRILL);
        }
        tables.sort_unstable();
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
                let mut reads = vec![
                    Read::all(TableId::Countries),
                    Read::all(TableId::CountryLinks),
                    Read::all(TableId::CountryAppearance),
                    Read::all(TableId::LexicalTokens),
                    Read::these(TableId::DivisionEquipmentCounts, [slot << 8 | side128]),
                    Read { table: TableId::DivisionEquipmentByType, keys: Keys::Range(types, types | ((1 << 33) - 1)) },
                    Read { table: TableId::DivisionEquipmentByDesign, keys: Keys::Range(designs, designs | ((1 << 66) - 1)) },
                    Read { table: TableId::DivisionEquipmentByTemplate, keys: Keys::Range(template_lo, template_hi) },
                ];
                if params.template.is_some() || params.division.is_some() {
                    reads.extend(DRILL.map(Read::all));
                    reads.extend([Read::all(TableId::DivisionTemplateIcons), Read::all(TableId::VariantDefinitions)]);
                }
                reads
            }
            1 if params.template.is_none() && params.division.is_none() => {
                let templates = held.complete::<tables::DivisionEquipmentByTemplate>().map(|rows| rows.iter().map(|row| u128::from(row.template)).collect::<Vec<_>>()).unwrap_or_default();
                let designs = held.complete::<tables::DivisionEquipmentByDesign>().map(|rows| rows.iter().filter_map(|row| row.variant).map(|id| u128::from(design_handle(id))).collect::<Vec<_>>()).unwrap_or_default();
                vec![Read::these(TableId::DivisionTemplateIcons, templates), Read::these(TableId::VariantDefinitions, designs)]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let content = Content::load(source, load);
        let needs = template_needs(&content.cat);
        Ok(EquipmentContext { content, needs })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Division equipment country is unavailable at this hour"))?;
        let mut refusals = Refusals::default();
        let mut reply = overview(frame, params, at.holders.name(&params.country, at.hour), owner.slot, &at.context.content, &mut refusals)?;
        if params.template.is_some() || params.division.is_some() {
            drill(frame, params, owner.slot, at, &mut reply, &mut refusals)?;
        }
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
        division_rows: None,
        pool: None,
        warnings: Vec::new(),
    };
    let tables = [TableId::DivisionEquipmentCounts, TableId::DivisionEquipmentByType, TableId::DivisionEquipmentByDesign, TableId::DivisionEquipmentByTemplate];
    if let Some(why) = tables.iter().find_map(|&id| unavailable(frame, id)) {
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
    Ok(reply)
}

struct Compared<'a> {
    row: &'a DivisionRow,
    status: Option<&'a DivisionStatusRow>,
    manpower: Option<i64>,
    required: Option<i64>,
    hp_max: Option<i64>,
    need: Result<Vec<(String, i64)>, String>,
    pool_known: bool,
    held: BTreeMap<Option<String>, (i128, BTreeMap<u64, i128>)>,
    types: Option<Vec<(Option<String>, i128, i128, i128)>>,
}

impl Compared<'_> {
    fn needed(&self) -> Option<i128> {
        self.types.as_ref().map(|types| types.iter().map(|kind| kind.1).sum())
    }

    fn filled(&self) -> Option<i128> {
        self.types.as_ref().map(|types| types.iter().map(|kind| kind.2).sum())
    }

    fn short(&self) -> Option<u32> {
        self.types.as_ref().map(|types| types.iter().filter(|kind| kind.0.is_some() && kind.3 > 0).count() as u32)
    }
}

fn pool<'a>(new: Option<&'a [DivisionEquipmentRow]>, daily: Option<&'a [DivisionEquipmentRow]>, division: u64) -> Option<&'a [DivisionEquipmentRow]> {
    let slice = |rows: &'a [DivisionEquipmentRow]| {
        let start = rows.partition_point(|row| row.division < division);
        &rows[start..start + rows[start..].partition_point(|row| row.division == division)]
    };
    if let Some(rows) = new.map(slice).filter(|rows| !rows.is_empty()) {
        return Some(rows);
    }
    if let Some(rows) = daily.map(slice).filter(|rows| !rows.is_empty()) {
        return Some(rows);
    }
    new.map(|_| &[][..])
}

fn design_archetype(frame: &DecodedFrame, variant: u64) -> Option<String> {
    let definition = find(frame.complete::<tables::VariantDefinitions>().ok(), variant, |row| row.id)?;
    (definition.archetype_token != 0).then_some(())?;
    token_key(frame.complete::<tables::LexicalTokens>().ok()?, definition.archetype_token).ok()
}

fn drill(frame: &DecodedFrame, params: &DivisionEquipmentParams, slot: u32, at: &At<'_, EquipmentContext>, reply: &mut DivisionEquipmentReply, refusals: &mut Refusals) -> io::Result<()> {
    let content = &at.context.content;
    let countries = Countries::read(frame)?;
    let template = handle(&params.template);
    let division = handle(&params.division);
    let divisions = frame.complete::<tables::Divisions>()?;
    let members: Vec<&DivisionRow> = divisions
        .iter()
        .filter(|row| row.reveal_skip == 0)
        .filter(|row| template.is_none_or(|handle| row.template == handle) && division.is_none_or(|handle| row.id == handle))
        .filter(|row| countries.slot(attributed(row, params.count_for)) == Some(slot))
        .collect();
    if division.is_some() && members.is_empty() {
        refusals.note("The selected division is not in this country's army at this hour");
    }
    let daily = frame.complete::<tables::DivisionEquipment>().ok().filter(|_| {
        frame.tables[TableId::DivisionEquipment.index()].sampled_at().is_some_and(|read| read <= frame.hour && i64::from(frame.hour) - i64::from(read) < i64::from(replay_data::cadence::STAGGER_PERIOD))
    });
    let new = frame.complete::<tables::NewDivisionEquipment>().ok();
    let sources = Sources::read(frame).map_err(|why| format!("Template equipment inputs unavailable: {why}"));
    let mut needs = BTreeMap::<u64, Result<Vec<(String, i64)>, String>>::new();
    let mut archetypes = BTreeMap::<u64, Option<String>>::new();
    let mut rows = Vec::new();
    for row in members {
        dashboard_text::metadata::check_cancel(at.cancelled)?;
        let status = find(frame.complete::<tables::DivisionStatus>().ok(), row.id, |row| row.division);
        let manpower = find(frame.complete::<tables::DivisionLandInputs>().ok(), row.id, |row| row.division).map(|land| i64::from(land.manpower.iter().fold(0i32, |sum, part| sum.wrapping_add(part.amount))));
        let required = find(frame.complete::<tables::DivisionTemplateIcons>().ok(), row.template, |row| row.template)
            .and_then(|icon| icon.settings.require("template settings").ok()?.as_ref().map(|settings| i64::from(settings.total_manpower)));
        let hp_max = find(frame.complete::<tables::DivisionPanelStats>().ok(), row.id, |row| row.division).and_then(|panel| panel.maximum_hit_points());
        let need = needs.entry(row.template).or_insert_with(|| template_need(&at.context.needs, &sources, frame, row.template)).clone();
        if let Err(why) = &need {
            refusals.note(why.clone());
        }
        let entries = pool(new, daily, row.id);
        if entries.is_none() {
            refusals.note("Division equipment pool was not recorded within a day of this hour");
        }
        let mut held = BTreeMap::<Option<String>, (i128, BTreeMap<u64, i128>)>::new();
        for entry in entries.unwrap_or_default() {
            let kind = archetypes.entry(entry.variant_handle).or_insert_with(|| design_archetype(frame, entry.variant_handle)).clone();
            if kind.is_none() {
                refusals.note("Division equipment design has no resolved archetype; holdings are unclassified");
            }
            let total = held.entry(kind).or_default();
            total.0 += i128::from(entry.amount_raw);
            *total.1.entry(entry.variant_handle).or_default() += i128::from(entry.amount_raw);
        }
        let types = match (&need, entries.is_some() && !held.contains_key(&None)) {
            (Ok(need), true) => Some(compare(need, &held)),
            _ => None,
        };
        rows.push(Compared { row, status, manpower, required, hp_max, need, pool_known: entries.is_some(), held, types });
    }
    let named_design = |variant: u64, refusals: &mut Refusals| {
        let definition = find(frame.complete::<tables::VariantDefinitions>().ok(), variant, |row| row.id);
        match definition {
            Some(definition) => refusals.keep(content.design_name(frame, definition)).0,
            None => {
                refusals.note("An equipment design has no recorded definition");
                None
            }
        }
    };
    if template.is_some() {
        let mut out = Vec::new();
        for division in &rows {
            let (name, name_missing) = refusals.keep(content.division_name(division.status));
            let converting_from = match division.status.and_then(|row| row.pending_template.require("pending template of division status").ok()).copied().flatten() {
                Some(0) => None,
                Some(previous) => refusals.keep(content.template_name(frame, previous)).0,
                None => {
                    refusals.note("Division conversion context was not recorded");
                    None
                }
            };
            out.push(EquipmentDivision {
                division: division.row.id.to_string(),
                name,
                name_missing,
                manpower: division.manpower.map(|value| value.to_string()),
                manpower_required: division.required.map(|value| value.to_string()),
                hp_raw: Some(division.row.hit_points.to_string()),
                hp_max_raw: division.hp_max.map(|value| value.to_string()),
                needed_raw: division.needed().map(|value| value.to_string()),
                held_raw: division.filled().map(|value| value.to_string()),
                types_short: division.short(),
                pool_known: division.pool_known,
                converting_from,
            });
        }
        reply.division_rows = Some(out);
    }
    if let Some(division) = rows.first().filter(|_| division.is_some()) {
        let mut keys: Vec<Option<String>> = division.need.as_ref().map(|need| need.iter().map(|(key, _)| Some(key.clone())).collect()).unwrap_or_default();
        keys.extend(division.held.keys().filter(|key| !keys.contains(key)).cloned().collect::<Vec<_>>());
        let mut types = Vec::new();
        for key in keys {
            let compared = division.types.as_ref().and_then(|types| types.iter().find(|kind| kind.0 == key));
            let needed = division.need.as_ref().ok().and_then(|need| {
                let key = key.as_ref()?;
                Some(need.iter().find(|(archetype, _)| same_archetype(archetype, key)).map_or(0, |(_, amount)| i128::from(*amount)))
            });
            let held = division.held.get(&key);
            types.push(DivisionPoolType {
                name: key.as_ref().and_then(|key| refusals.keep(content.localised(key)).0),
                archetype: key,
                held_raw: division.pool_known.then(|| held.map_or(0, |held| held.0).to_string()),
                needed_raw: needed.map(|amount| amount.to_string()),
                missing_raw: compared.map(|kind| kind.3.to_string()),
                variants: held
                    .map(|held| held.1.iter().map(|(&variant, amount)| EquipmentVariant { variant: variant.to_string(), name: named_design(variant, refusals), held_raw: Some(amount.to_string()) }).collect())
                    .unwrap_or_default(),
            });
        }
        let (name, _) = refusals.keep(content.division_name(division.status));
        reply.pool = Some(DivisionPool {
            division: division.row.id.to_string(),
            name,
            template: division.row.template.to_string(),
            template_name: refusals.keep(content.template_name(frame, division.row.template)).0,
            manpower: division.manpower.map(|value| value.to_string()),
            manpower_required: division.required.map(|value| value.to_string()),
            hp_raw: Some(division.row.hit_points.to_string()),
            hp_max_raw: division.hp_max.map(|value| value.to_string()),
            types,
            pool_known: division.pool_known,
        });
    }
    Ok(())
}

fn same_archetype(need: &str, design: &str) -> bool {
    need.eq_ignore_ascii_case(design)
}

fn compare(need: &[(String, i64)], held: &BTreeMap<Option<String>, (i128, BTreeMap<u64, i128>)>) -> Vec<(Option<String>, i128, i128, i128)> {
    let mut out: Vec<(Option<String>, i128, i128, i128)> = Vec::new();
    let mut seen = BTreeSet::new();
    for (key, amount) in need {
        let amount = i128::from(*amount);
        let holding: i128 = held.iter().filter(|(kind, _)| kind.as_deref().is_some_and(|kind| same_archetype(key, kind))).map(|(kind, held)| {
            seen.insert(kind.clone());
            held.0
        }).sum();
        out.push((Some(key.clone()), amount, holding.clamp(0, amount.max(0)), (amount - holding).max(0)));
    }
    out.extend(held.keys().filter(|kind| !seen.contains(*kind)).map(|kind| (kind.clone(), 0, 0, 0)));
    out
}

#[cfg(test)]
pub(super) fn compare_division(need: &[(String, i64)], held: &[(Option<&str>, i128)]) -> Vec<(Option<String>, i128, i128, i128)> {
    let held = held.iter().map(|&(kind, amount)| (kind.map(str::to_owned), (amount, BTreeMap::new()))).collect();
    compare(need, &held)
}
