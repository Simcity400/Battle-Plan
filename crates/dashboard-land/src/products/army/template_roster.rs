use dashboard_base::products::{Country, validate_country};
use super::LoadContext;
use dashboard_base::{Source, products::at_hour::{At, AtHour, Owner, Read}};
use dashboard_text::{metadata::link_slot, template_facts};
use content_reader_localisation::localisation::Localisation;
use replay_data::rows::{CountryLinkRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, DecodedSource};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::CountryTemplateState,
    TableId::OrdersGroups,
    TableId::OrdersGroupCounterInputs,
    TableId::LexicalTokens,
];

const WHOLE: [TableId; 6] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::DivisionTemplates,
    TableId::CountryTemplateState,
    TableId::OrdersGroups,
    TableId::LexicalTokens,
];

pub struct TemplateRoster;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateRosterParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TemplateRosterReply {
    pub sections: Vec<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(TemplateRosterReply);

fn slot_of(links: &[CountryLinkRow], raw: u32) -> Option<u32> {
    links.binary_search_by_key(&raw, |link| link.id).ok().map(|at| links[at].canonical_slot)
}

fn owned(held: &DecodedFrame, slot: u32) -> (BTreeSet<u64>, BTreeSet<u64>) {
    let (mut templates, mut groups) = (BTreeSet::new(), BTreeSet::new());
    let Ok(links) = held.complete::<tables::CountryLinks>() else { return (templates, groups) };
    let mine = |raw: u32| slot_of(links, raw) == Some(slot);
    if let Ok(rows) = held.complete::<tables::DivisionTemplates>() {
        templates.extend(rows.iter().filter(|row| mine(row.owner)).map(|row| row.template));
    }
    if let Ok(rows) = held.complete::<tables::CountryTemplateState>() {
        for row in rows.iter().filter(|row| mine(row.country)) {
            templates.insert(row.selected_staff);
            if let Ok(Some(scopes)) = row.garrisons.require("garrison templates") {
                templates.extend(scopes.iter().filter_map(|scope| scope.template));
            }
        }
    }
    if let Ok(rows) = held.complete::<tables::OrdersGroups>() {
        groups.extend(rows.iter().filter(|row| mine(row.country)).map(|row| row.handle));
    }
    templates.remove(&0);
    (templates, groups)
}

fn named(held: &DecodedFrame) -> BTreeSet<u64> {
    let mut out = BTreeSet::new();
    if let Ok(rows) = held.complete::<tables::DivisionTemplateIcons>() {
        out.extend(rows.iter().filter_map(|row| row.settings.require("template settings").ok()?.as_ref().map(|settings| settings.parent)));
    }
    if let Ok(rows) = held.complete::<tables::OrdersGroupCounterInputs>() {
        out.extend(rows.iter().filter_map(|row| row.template_context.require("HQ template context").ok()?.as_ref().map(|context| context.target)));
    }
    out.remove(&0);
    out
}

impl AtHour for TemplateRoster {
    const KEY: &'static str = "template-roster";
    const TITLE: &'static str = "Template roster";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = TemplateRosterParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = TemplateRosterReply;
    type Context = Arc<Localisation>;

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
        TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => WHOLE.map(Read::all).into(),
            1 => {
                let (templates, groups) = owned(held, owner.slot);
                vec![
                    Read::these(TableId::DivisionTemplateIcons, templates.into_iter().map(u128::from)),
                    Read::these(TableId::OrdersGroupCounterInputs, groups.into_iter().map(u128::from)),
                ]
            }
            2 => {
                let mut wanted = owned(held, owner.slot).0;
                wanted.extend(named(held));
                let primary = held.tables[TableId::DivisionTemplateIcons.index()].keys();
                if wanted.iter().all(|handle| primary.contains(u128::from(*handle))) {
                    return Vec::new();
                }
                vec![Read::these(TableId::DivisionTemplateIcons, wanted.into_iter().map(u128::from))]
            }
            _ => Vec::new(),
        }
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        let fields: &[&str] = match table {
            TableId::DivisionTemplates => &["owner", "name", "name_key"],
            TableId::OrdersGroups => &["country"],
            TableId::OrdersGroupCounterInputs => &["hq_nearest_front", "template_context"],
            TableId::DivisionTemplateIcons => &["country", "display_name", "settings", "model_unreadable", "excluded_variants", "model_entity", "model_category", "foreign_country", "layout"],
            _ => return crate::products::divisions::naming_columns(table),
        };
        Some(dashboard_base::products::at_hour::field_columns(table, fields))
    }

    fn valid_reply_through(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>, reply: &Self::Reply, _partial: bool) -> i32 {
        let icons = frame.complete::<tables::DivisionTemplateIcons>().unwrap_or(&[]);
        crate::products::divisions::template_names_through(frame, icons.iter().map(|icon| icon.template))
            .min(crate::products::warnings_through(frame.hour, reply.warnings.iter().map(String::as_str)))
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let content = load.content.as_ref().ok_or_else(|| dashboard_core::invalid("Template roster needs the recorded content stack"))?;
        dashboard_base::content::localisation(source, content)
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Template roster country is unavailable at this hour"))?;
        let (sections, warnings) = sections(frame, owner.slot, at.context, at.cancelled)?;
        let reply = TemplateRosterReply { sections, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn sections(frame: &dyn DecodedSource, slot: u32, loc: &Localisation, cancelled: &AtomicBool) -> io::Result<(Vec<String>, Vec<String>)> {
    frame.complete::<tables::Countries>()?;
    let links = frame.complete::<tables::CountryLinks>()?;
    let canonical = |raw| link_slot(links, raw).ok_or_else(|| format!("country links country {raw} is absent"));
    let mut sections = Vec::new();
    let mut warnings = Vec::new();
    let mut append = |label: &str, value: Result<template_facts::Rendered, String>| match value {
        Ok(rendered) => {
            warnings.extend(rendered.refused.into_iter().map(|(field, reason)| format!("{label}/{field}: {reason}")));
            if !rendered.text.is_empty() {
                sections.push(rendered.text);
            }
        }
        Err(reason) => {
            sections.push(format!("{label}\nUnavailable"));
            warnings.push(format!("{label}: {reason}"));
        }
    };
    append("Garrison and staff templates", template_facts::country(frame, slot, loc, canonical));
    let roster = frame.complete::<tables::DivisionTemplates>();
    match roster {
        Err(error) => append("Template roster", Err(error.to_string())),
        Ok(rows) => {
            let mut selected = Vec::new();
            let mut unresolved = false;
            for row in rows {
                match canonical(row.owner) {
                    Ok(owner) if owner == slot => selected.push(row.template),
                    Ok(_) => (),
                    Err(reason) => {
                        unresolved = true;
                        append("Template roster owner", Err(reason));
                    }
                }
            }
            if selected.is_empty() && !unresolved {
                append("Template roster", Ok(template_facts::Rendered::text("No division templates.")));
            }
            let icons = frame.complete::<tables::DivisionTemplateIcons>();
            selected.sort_by_key(|handle| {
                icons
                    .as_ref()
                    .ok()
                    .and_then(|rows| rows.iter().find(|r| r.template == *handle))
                    .and_then(|r| r.settings.require("template settings").ok())
                    .and_then(Option::as_ref)
                    .map(|s| s.roster_ordinal)
            });
            for handle in selected {
                dashboard_text::metadata::check_cancel(cancelled)?;
                append("Template settings and layout", template_facts::template(frame, handle, loc));
            }
        }
    }
    match frame.complete::<tables::OrdersGroups>() {
        Err(error) => append("HQ groups", Err(error.to_string())),
        Ok(groups) => {
            for group in groups {
                match canonical(group.country) {
                    Ok(owner) if owner == slot => append("HQ deployment", template_facts::hq(frame, group.handle, loc)),
                    Ok(_) => (),
                    Err(reason) => append("HQ group owner", Err(reason)),
                }
            }
        }
    }
    Ok((sections, warnings))
}
