//! A country's training queue at a reached hour: each deployment order (a
//! template ordered in some amount) in the game's order, with the divisions in
//! training under it and their training, manpower and equipment.
//!
//! The queue is read once a day; the reply is the latest read at or before the
//! hour and says when it was read. The country is the one holding the tag at
//! the hour, and the read is used only while its slot holds that same country,
//! so a renamed country keeps its queue and a reused tag never lends another's.
//! Nothing is interpolated: training is the
//! stored value of that read, and an amount ordered never becomes invented
//! divisions. No ending date or readiness is derived. A stored halting reason
//! is reported only as present, since no verified mapping names it. An order's
//! priority and location take the game's text for its controls; a division's
//! own distribution priority copies its order's and is not shown again.
use super::army_content::{self, Refusals};
use dashboard_base::{
    prepared::query::{native, progressive},
    products::Product,
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_text::catalogue::Catalogue;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc, sync::atomic::AtomicBool};
use ts_rs::TS;

pub struct TrainingQueue;

/// The queue tables, read together once a day.
const QUEUE: [TableId; 4] = [
    TableId::DeploymentConveyors,
    TableId::DeploymentLines,
    TableId::DeploymentManpower,
    TableId::DeploymentEquipment,
];
/// What names the queue, read with it.
const NAMING: [TableId; 8] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Provinces,
    TableId::States,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::LexicalTokens,
    TableId::CountryLocalSubunits,
];

/// The queue's read: what names it, and every operand its priority names' scripted text may read.
pub fn read_tables() -> Vec<TableId> {
    let mut tables = QUEUE.to_vec();
    tables.extend(NAMING);
    tables.extend_from_slice(dashboard_text::scripted_text::TABLES);
    tables.sort_unstable();
    tables.dedup();
    tables
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingQueueParams {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
}

/// One country's manpower entry for a division in training.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrainingManpower {
    pub tag: Option<String>,
    /// Available (false) or needed (true).
    pub needed: bool,
    pub amount: i32,
}

/// One design held by a division in training.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrainingEquipment {
    pub variant: String,
    pub name: Option<String>,
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
    /// Signed fixed-point amount with five decimals.
    pub amount_raw: String,
}

/// One archetype a division in training needs, by its template.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrainingNeed {
    pub archetype: String,
    pub name: Option<String>,
    /// Fixed-point need with five decimals.
    pub need_raw: String,
    /// What the division holds of it, fixed-point with five decimals.
    pub held_raw: String,
}

/// One division in training.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrainingDivision {
    pub line: String,
    pub ordinal: u32,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    /// Fixed-point training and its maximum, five decimals.
    pub training_raw: String,
    pub max_training_raw: String,
    /// Signed sums of the available and needed entries, as the game adds them.
    pub manpower: i32,
    pub manpower_needed: i32,
    pub max_manpower: i32,
    pub contributors: Vec<TrainingManpower>,
    pub equipment: Vec<TrainingEquipment>,
    /// What the template needs; empty with `need_missing` when unknown.
    pub need: Vec<TrainingNeed>,
    pub need_missing: Option<String>,
    /// Why held equipment cannot be measured against the need: the need is
    /// unknown, or a held design's type is unresolved, so what it fills is
    /// unknown though the known needs and holdings stay.
    pub comparison_missing: Option<String>,
    /// A halting reason is stored; its meaning is not named.
    pub halted: bool,
}

/// One deployment order: a template ordered in an amount.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
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
    /// The stored priority, and the game's text for it.
    pub priority: u32,
    pub priority_name: Option<String>,
    pub priority_missing: Option<String>,
    /// The stored province (zero for none), and the name the game shows for it.
    pub location: u32,
    pub location_name: Option<String>,
    pub location_missing: Option<String>,
    pub closed: bool,
    /// A government in exile's order names that country.
    pub exile: Option<String>,
    pub divisions: Vec<TrainingDivision>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrainingQueueReply {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    /// No country holds the tag at this hour.
    pub absent: bool,
    /// The read the queue is from; none when none falls within a day.
    pub read_hour: Option<i32>,
    /// Why the queue is unknown at this hour, when it is.
    pub missing: Option<String>,
    pub orders: Vec<TrainingOrder>,
    pub warnings: Vec<String>,
}
dashboard_base::reply_warnings!(TrainingQueueReply);

impl Product for TrainingQueue {
    const KEY: &'static str = "training-queue";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = TrainingQueueParams;
    type Reply = TrainingQueueReply;
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
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        dashboard_base::products::validate_tag(&p.tag)?;
        p.branch_id.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
    }
    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = p.branch_id.parse::<u64>().map_err(|_| io::Error::other("Invalid branch"))?;
        let reply = dashboard_land_battles::products::reached::cached(
            source,
            context,
            Self::KEY,
            branch,
            p.hour,
            &p.tag,
            cancelled,
            || {
                let playback = progressive::frame_at(
                    source,
                    branch,
                    p.hour,
                    &[TableId::Countries, TableId::CountryLinks],
                    cancelled,
                )?
                .ok_or_else(|| dashboard_core::refused("This hour is not recorded"))?;
                let read = native::latest_read(
                    source,
                    branch,
                    p.hour,
                    TableId::DeploymentLines,
                    replay_data::cadence::STAGGER_PERIOD,
                    &read_tables(),
                    cancelled,
                )?;
                let Some(read) = read else {
                    return Ok(TrainingQueueReply {
                        branch_id: p.branch_id.clone(),
                        hour: p.hour,
                        tag: p.tag.clone(),
                        absent: false,
                        read_hour: None,
                        missing: Some("The training queue was not read within a day of this hour".into()),
                        orders: Vec::new(),
                        warnings: Vec::new(),
                    });
                };
                let mut refusals = Refusals::default();
                let cat = refusals.keep(army_content::content(source, context)).0;
                let designs = native::latest_read(
                    source,
                    branch,
                    read.hour,
                    TableId::VariantDefinitions,
                    replay_data::cadence::STAGGER_PERIOD,
                    &dashboard_base::products::equipment::DESIGN_TABLES,
                    cancelled,
                )?;
                let names = cat.as_ref().map(|cat| {
                    let (names, notes) =
                        dashboard_land_battles::products::division_log::division_name_groups(source, cat);
                    for note in notes {
                        refusals.note(note);
                    }
                    names
                });
                let need = super::division_needs::Needs::read(cat.as_ref(), &read);
                let mut reply = build(
                    &read,
                    &playback,
                    &p.tag,
                    &Context {
                        cat: cat.as_ref(),
                        designs: &army_content::Designs::new(designs.as_deref(), cat.as_ref()),
                        names: names.as_ref().map(|names| names.as_ref().map(Arc::as_ref).map_err(Clone::clone)),
                        icon: &|handle| army_content::template_icon(source, &read, handle, cat.as_ref()),
                        need: &need,
                    },
                    &mut refusals,
                    cancelled,
                )?;
                reply.branch_id.clone_from(&p.branch_id);
                reply.hour = p.hour;
                reply.warnings = refusals.lines();
                Ok(reply)
            },
        )?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

/// What the queue is named and measured with.
pub struct Context<'a> {
    pub cat: Option<&'a Arc<Catalogue>>,
    pub designs: &'a army_content::Designs<'a>,
    pub names: Option<Result<&'a installed_content_division_names::division_names::Database, String>>,
    pub icon: &'a dyn Fn(u64) -> Result<String, String>,
    pub need: &'a super::division_needs::Needs<'a>,
}

/// A division in training's name: its own stored name, else the next name of
/// its template's name group at its order.
fn division_name(
    line: &replay_data::rows::DeploymentLineRow,
    group: Option<&[u8]>,
    context: &Context<'_>,
) -> Result<String, String> {
    if !line.custom_name.is_empty() {
        return if line.custom_flag_b != 0 {
            let cat = context.cat.ok_or("Division names need the recorded content stack")?;
            dashboard_text::scripted_text::stored_text(
                &cat.localisation,
                &cat.recorded_language,
                &line.custom_name,
                "Programmatic division name",
            )
        } else {
            String::from_utf8(line.custom_name.clone()).map_err(|_| "Division name is not text".into())
        };
    }
    let group = group.ok_or("The template's name group was not read")?;
    let names = context.names.clone().ok_or("Division names need the recorded content stack")??;
    names
        .resolve(group, line.name_order, line.custom_flag_a != 0)
        .map_err(|why| format!("Division name unavailable: {why}"))
        .and_then(|name| dashboard_base::catalogue::plain_text(&name))
}

/// The queue from its daily read, for the country holding `tag` in `playback`
/// (the reached hour's countries).
pub fn build(
    read: &DecodedFrame,
    playback: &DecodedFrame,
    tag: &str,
    context: &Context<'_>,
    refusals: &mut Refusals,
    cancelled: &AtomicBool,
) -> io::Result<TrainingQueueReply> {
    let mut reply = TrainingQueueReply {
        branch_id: read.branch.to_string(),
        hour: read.hour,
        tag: tag.to_owned(),
        absent: false,
        read_hour: Some(read.hour),
        missing: None,
        orders: Vec::new(),
        warnings: Vec::new(),
    };
    if let Some(why) = QUEUE.iter().find_map(|&id| army_content::unavailable(read, id)) {
        reply.missing = Some(why);
        return Ok(reply);
    }
    let Some(holder) = army_content::holder(playback, tag)? else {
        reply.absent = true;
        return Ok(reply);
    };
    if let Err(why) = army_content::same_holder(read, holder) {
        reply.missing = Some(format!("The training queue's latest read is left out: {why} at that read"));
        return Ok(reply);
    }
    let slot = holder.slot;
    let countries = army_content::Countries::read(read)?;
    let priorities = army_content::DeploymentPriorities::read(context.cat, read, slot, refusals);
    for warning in context.need.warnings() {
        refusals.note(warning);
    }
    let conveyors = read.complete::<tables::DeploymentConveyors>()?;
    let lines = read.complete::<tables::DeploymentLines>()?;
    let manpower = read.complete::<tables::DeploymentManpower>()?;
    let equipment = read.complete::<tables::DeploymentEquipment>()?;
    let icons = read.complete::<tables::DivisionTemplateIcons>().ok();
    let mut orders: Vec<_> = conveyors
        .iter()
        .filter(|row| countries.slot(row.country) == Some(slot))
        .collect();
    orders.sort_by_key(|row| (row.ordinal, row.handle));
    let mut by_order = BTreeMap::<u64, Vec<&replay_data::rows::DeploymentLineRow>>::new();
    for line in lines {
        by_order.entry(line.conveyor).or_default().push(line);
    }
    for order in orders {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (template_name, template_missing) =
            refusals.keep(army_content::template_name(read, order.template, context.cat));
        let (icon, icon_missing) = refusals.keep((context.icon)(order.template));
        let settings = icons
            .and_then(|rows| rows.binary_search_by_key(&order.template, |row| row.template).ok().map(|at| &rows[at]))
            .and_then(|row| row.settings.require("template settings").ok().cloned().flatten());
        let group = settings.as_ref().and_then(|settings| settings.names_group.clone());
        let need = context.need.template(read, order.template);
        let mut divisions = Vec::new();
        let mut members = by_order.remove(&order.handle).unwrap_or_default();
        members.sort_by_key(|line| (line.ordinal, line.handle));
        for line in members {
            let (name, name_missing) = refusals.keep(division_name(line, group.as_deref(), context));
            let entries = |side: u8| {
                let start = manpower.partition_point(|row| (row.line, row.side) < (line.handle, side));
                manpower[start..].iter().take_while(move |row| row.line == line.handle && row.side == side)
            };
            let sum = |side: u8| entries(side).fold(0i32, |sum, row| sum.wrapping_add(row.amount));
            let contributors = entries(0)
                .chain(entries(1))
                .map(|row| TrainingManpower {
                    tag: countries.tag(row.source_country),
                    needed: row.side == 1,
                    amount: row.amount,
                })
                .collect();
            let start = equipment.partition_point(|row| row.line < line.handle);
            let held: Vec<_> = equipment[start..]
                .iter()
                .take_while(|row| row.line == line.handle)
                .map(|row| {
                    let design = context.designs.design(row.variant_handle, refusals);
                    TrainingEquipment {
                        variant: row.variant_handle.to_string(),
                        name: design.name,
                        archetype: design.archetype,
                        archetype_name: design.archetype_name,
                        amount_raw: row.amount_raw.to_string(),
                    }
                })
                .collect();
            // A held design of unresolved type may fill any need.
            let unclassified = held.iter().any(|row| row.archetype.is_none());
            let (need, need_missing) = match &need {
                Ok(need) => (
                    need.iter()
                        .map(|(archetype, amount)| TrainingNeed {
                            archetype: archetype.clone(),
                            name: army_content::localised(context.cat, archetype).ok(),
                            need_raw: amount.to_string(),
                            held_raw: held
                                .iter()
                                .filter(|row| row.archetype.as_deref() == Some(archetype))
                                .fold(0i128, |sum, row| sum + row.amount_raw.parse::<i128>().unwrap_or(0))
                                .to_string(),
                        })
                        .collect(),
                    None,
                ),
                Err(why) => {
                    refusals.note(why.clone());
                    (Vec::new(), Some(why.clone()))
                }
            };
            let comparison_missing = need_missing.clone().or_else(|| {
                unclassified.then(|| "A held design's equipment type is unresolved".to_owned())
            });
            divisions.push(TrainingDivision {
                line: line.handle.to_string(),
                ordinal: line.ordinal,
                name,
                name_missing,
                training_raw: line.training_raw.to_string(),
                max_training_raw: line.max_training_raw.to_string(),
                manpower: sum(0),
                manpower_needed: sum(1),
                max_manpower: line.max_manpower,
                contributors,
                equipment: held,
                need,
                need_missing,
                comparison_missing,
                halted: line.halting_reason_present != 0,
            });
        }
        let (priority_name, priority_missing) =
            refusals.keep(priorities.name(order.priority));
        let (location_name, location_missing) =
            refusals.keep(army_content::deployment_location(read, context.cat, order.location));
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
            exile: order.exile_country.and_then(|raw| countries.tag(raw)),
            divisions,
        });
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;
