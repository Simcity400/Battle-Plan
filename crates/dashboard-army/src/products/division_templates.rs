//! A country's division templates at a reached hour: every template it owns,
//! used or not, in the game's roster order, with its battalions and support
//! companies, base combat width, settings and how many divisions field it.
//!
//! Roster ownership, not deployed membership, selects the templates; fielded
//! counts follow the Army size attribution and leave out intelligence copies.
//! The base width is the recorded sum of each sub-unit's width before
//! country and commander factors, never the designer's final width.
use super::army_content::{self, Refusals};
use dashboard_base::{prepared::query::progressive, products::Product, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_forces::products::army_size::{ArmyAttribution, attributed};
use replay_data::rows::{TableId, TemplateSubunit, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc, sync::atomic::AtomicBool};
use ts_rs::TS;

pub struct DivisionTemplates;

const TABLES: [TableId; 6] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::LexicalTokens,
];

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionTemplatesParams {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub count_for: ArmyAttribution,
}

/// One kind of sub-unit in a template grid, with how many cells hold it.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TemplateUnit {
    /// The sub-unit's content key, shown on hover only.
    pub key: Option<String>,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub count: i32,
    /// A support company rather than a battalion.
    pub support: bool,
    /// A country-local copy of the definition.
    pub local: bool,
}

/// A designer that opens a template: the ordinary one or the army headquarters'.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum DesignerKind {
    Ordinary,
    Headquarters,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TemplateEntry {
    /// The template's full native handle.
    pub template: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    /// Position in the country's roster.
    pub roster_ordinal: Option<u32>,
    /// Divisions of the template counted for the country; none when the
    /// division census was not read.
    pub fielded: Option<u32>,
    pub units: Vec<TemplateUnit>,
    pub units_missing: Option<String>,
    /// Signed fixed-point base combat width with five decimals.
    pub base_width_raw: Option<String>,
    /// The template's stored priority and the game's name for it.
    pub priority: Option<i32>,
    pub priority_name: Option<String>,
    pub priority_missing: Option<String>,
    pub locked: Option<bool>,
    pub obsolete: Option<bool>,
    /// The hour the template was made obsolete, when recorded.
    pub obsolete_hour: Option<i32>,
    pub designer: Option<DesignerKind>,
    pub force_recruiting: Option<bool>,
    pub new_equipment: Option<bool>,
    pub foreign_equipment: Option<bool>,
    pub manpower: Option<i32>,
    pub training_days: Option<i32>,
    pub settings_missing: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionTemplatesReply {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub count_for: ArmyAttribution,
    /// No country holds the tag at this hour.
    pub absent: bool,
    pub templates: Vec<TemplateEntry>,
    /// Why fielded counts are unknown, when they are.
    pub fielded_missing: Option<String>,
    pub warnings: Vec<String>,
}
dashboard_base::reply_warnings!(DivisionTemplatesReply);

impl Product for DivisionTemplates {
    const KEY: &'static str = "division-templates";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = DivisionTemplatesParams;
    type Reply = DivisionTemplatesReply;
    fn types() -> Vec<String> {
        vec![
            DivisionTemplatesParams::decl(),
            DivisionTemplatesReply::decl(),
            TemplateEntry::decl(),
            TemplateUnit::decl(),
            DesignerKind::decl(),
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
            &(&p.tag, p.count_for),
            cancelled,
            || {
                let frame = progressive::frame_at(source, branch, p.hour, &TABLES, cancelled)?
                    .ok_or_else(|| io::Error::other("This hour is not recorded"))?;
                let mut refusals = Refusals::default();
                let cat = refusals.keep(army_content::content(source, context)).0;
                let mut reply = build(
                    &frame,
                    &p.tag,
                    p.count_for,
                    cat.as_ref(),
                    &|handle| army_content::template_icon(source, &frame, handle, cat.as_ref()),
                    &mut refusals,
                    cancelled,
                )?;
                reply.branch_id.clone_from(&p.branch_id);
                reply.warnings = refusals.lines();
                Ok(reply)
            },
        )?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn fixed(raw: i64) -> String {
    raw.to_string()
}

const UNLINKED_DIVISION: &str =
    "A template's fielded count is unknown: a division of it has a country with no recorded link";
const UNLINKED_TEMPLATE: &str = "A template's country has no recorded link; it is left out";
const MODEL_UNREADABLE: &str = "Template composition could not be read at capture";

/// The roster from one reached frame. `icon` draws a template's artwork.
pub fn build(
    frame: &DecodedFrame,
    tag: &str,
    count_for: ArmyAttribution,
    cat: Option<&Arc<dashboard_text::catalogue::Catalogue>>,
    icon: &dyn Fn(u64) -> Result<String, String>,
    refusals: &mut Refusals,
    cancelled: &AtomicBool,
) -> io::Result<DivisionTemplatesReply> {
    let mut reply = DivisionTemplatesReply {
        branch_id: frame.branch.to_string(),
        hour: frame.hour,
        tag: tag.to_owned(),
        count_for,
        absent: false,
        templates: Vec::new(),
        fielded_missing: None,
        warnings: Vec::new(),
    };
    let Some(slot) = army_content::country_slot(frame, tag)? else {
        reply.absent = true;
        return Ok(reply);
    };
    let countries = army_content::Countries::read(frame)?;
    let roster = frame.complete::<tables::DivisionTemplates>()?;
    let icons = frame.complete::<tables::DivisionTemplateIcons>().ok();
    // None for a raw country with no link, which may be this one; raw zero is
    // the reserved country, never a tagged one.
    let ours = |raw: u32| match countries.slot(raw) {
        Some(owner) => Some(owner == slot),
        None => (raw == 0).then_some(false),
    };
    // A division that may count for the country leaves its template's count
    // unknown, never a lower number.
    let mut fielded = BTreeMap::<u64, Option<u32>>::new();
    match frame.complete::<tables::Divisions>() {
        Ok(divisions) => {
            for division in divisions.iter().filter(|row| row.reveal_skip == 0) {
                match ours(attributed(division, count_for)) {
                    Some(true) => {
                        if let Some(count) = fielded.entry(division.template).or_insert(Some(0)) {
                            *count += 1;
                        }
                    }
                    Some(false) => {}
                    None => {
                        fielded.insert(division.template, None);
                    }
                }
            }
        }
        Err(why) => reply.fielded_missing = Some(format!("Divisions were not read at this hour: {why}")),
    }
    let mut owned = Vec::new();
    for row in roster.iter() {
        match ours(row.owner) {
            Some(true) => owned.push(row),
            Some(false) => {}
            None => refusals.note(UNLINKED_TEMPLATE),
        }
    }
    let settings_of = |handle: u64| {
        icons
            .and_then(|rows| rows.binary_search_by_key(&handle, |row| row.template).ok().map(|at| &rows[at]))
    };
    // The game's roster order; templates whose settings were not read follow in handle order.
    owned.sort_by_key(|row| {
        let ordinal = settings_of(row.template)
            .and_then(|icon| icon.settings.require("template settings").ok().cloned().flatten())
            .map(|settings| settings.roster_ordinal);
        (ordinal.is_none(), ordinal, row.template)
    });
    for row in owned {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let handle = row.template;
        let (name, name_missing) = refusals.keep(army_content::template_name(frame, handle, cat));
        let (icon, icon_missing) = refusals.keep(icon(handle));
        let mut entry = TemplateEntry {
            template: handle.to_string(),
            name,
            name_missing,
            icon,
            icon_missing,
            roster_ordinal: None,
            fielded: match fielded.get(&handle) {
                _ if reply.fielded_missing.is_some() => None,
                Some(None) => {
                    refusals.note(UNLINKED_DIVISION);
                    None
                }
                Some(&count) => count,
                None => Some(0),
            },
            units: Vec::new(),
            units_missing: None,
            base_width_raw: None,
            priority: None,
            priority_name: None,
            priority_missing: None,
            locked: None,
            obsolete: None,
            obsolete_hour: None,
            designer: None,
            force_recruiting: None,
            new_equipment: None,
            foreign_equipment: None,
            manpower: None,
            training_days: None,
            settings_missing: None,
        };
        let Some(composition) = settings_of(handle) else {
            entry.units_missing = Some("Template composition was not read at this hour".into());
            entry.settings_missing = entry.units_missing.clone();
            refusals.note("Template composition was not read at this hour");
            reply.templates.push(entry);
            continue;
        };
        entry.base_width_raw = composition
            .base_width
            .require("base width")
            .ok()
            .copied()
            .flatten()
            .map(fixed);
        // A failed model read keeps empty grids beside its flag.
        match composition.grids.require("template grids") {
            Ok(_) if composition.model_unreadable != 0 => {
                refusals.note(MODEL_UNREADABLE);
                entry.units_missing = Some(MODEL_UNREADABLE.into());
            }
            Ok(grids) => {
                let mut add = |cells: &[TemplateSubunit], support: bool| {
                    for unit in cells.iter().filter(|unit| unit.token != 0) {
                        let key = army_content::token_key(frame, unit.token);
                        let name = key.clone().and_then(|key| army_content::localised(cat, &key));
                        let (name, name_missing) = refusals.keep(name);
                        entry.units.push(TemplateUnit {
                            key: key.ok(),
                            name,
                            name_missing,
                            count: unit.count,
                            support,
                            local: unit.local != 0,
                        });
                    }
                };
                add(&grids.regiments, false);
                add(&grids.regimental_support, false);
                add(&grids.support, true);
            }
            Err(why) => {
                refusals.note("Template grids were not recorded");
                entry.units_missing = Some(why);
            }
        }
        match composition.settings.require("template settings") {
            Ok(Some(settings)) => {
                entry.roster_ordinal = Some(settings.roster_ordinal);
                entry.priority = Some(settings.priority);
                (entry.priority_name, entry.priority_missing) =
                    refusals.keep(army_content::template_priority(cat, settings.priority));
                entry.locked = Some(settings.locked != 0);
                entry.obsolete = Some(settings.obsolete != 0);
                entry.obsolete_hour = (settings.obsolete != 0
                    && settings.obsolete_change_date != engine_native::cgame_date::NULL_RAW_HOUR)
                    .then_some(settings.obsolete_change_date);
                entry.designer = Some(if settings.is_army_hq != 0 {
                    DesignerKind::Headquarters
                } else {
                    DesignerKind::Ordinary
                });
                entry.force_recruiting = Some(settings.force_allow_recruiting != 0);
                entry.new_equipment = Some(settings.allow_new_equipment != 0);
                entry.foreign_equipment = Some(settings.allow_foreign_equipment != 0);
                entry.manpower = Some(settings.total_manpower);
                entry.training_days = Some(settings.training_time);
            }
            Ok(None) => {
                refusals.note("Template settings were refused at capture");
                entry.settings_missing = Some("Template settings were refused at capture".into());
            }
            Err(why) => {
                refusals.note("Template settings were not recorded");
                entry.settings_missing = Some(why);
            }
        }
        reply.templates.push(entry);
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;
