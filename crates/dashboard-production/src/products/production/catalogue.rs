use dashboard_world::products::equipment_names::{self as names, EquipmentIcon, Named, VariantName};

use dashboard_base::products::{Country, validate_country};
use super::{
    held_hours,
};
use dashboard_base::{Source, products::{LoadContext, Product, check_reply_budget, at_hour::{At, AtHour, Owner, Read}}, span::HourSpan};
use dashboard_follow::live::Source as Cell;
use dashboard_query::{identity::Holder, read};
use dashboard_text::{metadata::link_slot, variant_names::Texts};
use replay_columnar::summary::Source as Column;
use replay_data::rows::{CountryLinkRow, CountryRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ProductionLicences;
pub struct IndustrialOrganisations;
pub struct EquipmentDesigns;

const UNSELECTED_POLICY: u32 = 0x4c17;

fn validate_branch(branch: &str) -> Result<(), &'static str> {
    branch.parse::<u64>().map(|_| ()).map_err(|_| "Invalid branch")
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LicenceParty {
    pub slot: Option<u32>,
    pub tag: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LicenceDesign {
    pub variant: String,
    #[doc = " Null where the content does not name it or it has no definition at the hour."]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LicenceNow {
    #[doc = " Unique at the hour: the licensee's reference and the licence's place in its roster."]
    pub key: String,
    pub licensor: LicenceParty,
    pub licensee: LicenceParty,
    #[doc = " Civilian factories the licensee lends."]
    pub lent: i32,
    pub required: i32,
    #[doc = " The hour the licence started."]
    pub since: i32,
    pub designs: Vec<LicenceDesign>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionLicencesParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ProductionLicencesReply {
    pub branch_id: String,
    pub licences: Vec<LicenceNow>,
    #[doc = " Why the licences are not known at the hour, when they are not: never shown as none."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndustrialOrganisationsParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OrganisationNow {
    pub id: String,
    pub token: String,
    pub name: Option<String>,
    pub name_unknown: Option<String>,
    #[doc = " The icon's key in `icons`."]
    pub icon: Option<String>,
    pub icon_unknown: Option<String>,
    pub size: Option<i32>,
    #[doc = " Traits unlocked."]
    pub traits: Option<u32>,
    #[doc = " The selected policy's script key; null for none or where the state was not read."]
    pub policy_token: Option<String>,
    pub policy_name: Option<String>,
    #[doc = " Why a selected policy cannot be named, when it cannot."]
    pub policy_unknown: Option<String>,
    #[doc = " The hour the policy may change again; null when none is set. The days left are counted"]
    #[doc = " where they are shown, as the game counts them: its day less the shown hour's."]
    pub cooldown_hour: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IndustrialOrganisationsReply {
    pub branch_id: String,
    pub tag: String,
    pub organisations: Vec<OrganisationNow>,
    #[doc = " Why the organisations are not known at the hour, when they are not."]
    pub missing: Option<String>,
    #[doc = " Why their state (size, funds, traits, policy) is not known at the hour, when it is not."]
    pub details_missing: Option<String>,
    pub icons: Vec<EquipmentIcon>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EquipmentDesignsParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DesignHistory {
    pub id: String,
    #[doc = " Hours it was the country's saved design, ascending."]
    pub lives: Vec<HourSpan>,
    #[doc = " Hours of `lives` it was flagged obsolete."]
    pub obsolete: Vec<HourSpan>,
    #[doc = " Its names and picture, read at one hour it was the country's."]
    pub name: VariantName,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentDesignsReply {
    pub branch_id: String,
    pub tag: String,
    pub designs: Vec<DesignHistory>,
    #[doc = " Hours a holder of the tag was recorded: outside them its designs are not known."]
    pub known: Vec<HourSpan>,
    pub icons: Vec<EquipmentIcon>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ProductionLicencesReply, IndustrialOrganisationsReply, EquipmentDesignsReply);

fn slot_tags(countries: &[CountryRow]) -> BTreeMap<u32, String> {
    countries.iter().filter_map(|row| Some((row.canonical_slot, std::str::from_utf8(&row.tag).ok()?.to_owned()))).collect()
}

fn party(links: &[CountryLinkRow], tags: &BTreeMap<u32, String>, raw: u32) -> LicenceParty {
    let slot = link_slot(links, raw);
    LicenceParty { slot, tag: slot.and_then(|slot| tags.get(&slot).cloned()) }
}

impl AtHour for ProductionLicences {
    const KEY: &'static str = "production-licences";
    const TITLE: &'static str = "Production licences";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ProductionLicencesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = ProductionLicencesReply;
    type Context = names::Context;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ProductionLicencesParams::decl(), ProductionLicencesReply::decl(), LicenceParty::decl(), LicenceDesign::decl(), LicenceNow::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::ProductionLicenseRecords, TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens, TableId::VariantDefinitions]
    }

    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => [TableId::ProductionLicenseRecords, TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance].into_iter().map(Read::all).collect(),
            1 => {
                let variants: BTreeSet<u64> = held.complete::<tables::ProductionLicenseRecords>().map(|rows| rows.iter().flat_map(|row| row.equipment.iter().copied()).collect()).unwrap_or_default();
                vec![Read::these(TableId::VariantDefinitions, variants.into_iter().map(u128::from))]
            }
            2 => names::spelling_reads(held, names::variant_tokens(held).collect(), false),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        names::context(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut reply = ProductionLicencesReply { branch_id: params.branch_id.clone(), licences: Vec::new(), missing: None, warnings: Vec::new() };
        let tables = (frame.complete::<tables::ProductionLicenseRecords>(), frame.complete::<tables::CountryLinks>(), frame.complete::<tables::Countries>());
        let (records, links, countries) = match tables {
            (Ok(records), Ok(links), Ok(countries)) => (records, links, countries),
            (records, links, countries) => {
                let why = [records.err(), links.err(), countries.err()].into_iter().flatten().map(|error| error.to_string()).collect::<Vec<_>>().join("; ");
                reply.missing = Some(format!("Licences are not recorded at this hour: {why}"));
                return Ok(reply);
            }
        };
        let tags = slot_tags(countries);
        let variants: Vec<u64> = records.iter().flat_map(|row| row.equipment.iter().copied()).collect::<BTreeSet<_>>().into_iter().collect();
        let names = names::plain_names(frame, &variants, at.context);
        if let Err(why) = at.context {
            reply.warnings.push(why.clone());
        }
        reply.licences = records
            .iter()
            .map(|row| LicenceNow {
                key: format!("{}:{}", row.owner, row.ordinal),
                licensor: party(links, &tags, row.giver),
                licensee: party(links, &tags, row.owner),
                lent: row.lended_cic,
                required: row.required_cic,
                since: row.start_date,
                designs: row.equipment.iter().map(|&variant| LicenceDesign { variant: variant.to_string(), name: names.get(&variant).cloned().flatten() }).collect(),
            })
            .collect();
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn organisations_of<'a>(frame: &'a DecodedFrame, owner: Owner) -> io::Result<Vec<&'a replay_data::rows::IndustrialOrganisationRow>> {
    let links = frame.complete::<tables::CountryLinks>()?;
    Ok(frame.complete::<tables::IndustrialOrganisations>()?.iter().filter(|row| link_slot(links, row.country) == Some(owner.slot)).collect())
}

impl AtHour for IndustrialOrganisations {
    const KEY: &'static str = "industrial-organisations";
    const TITLE: &'static str = "Industrial organisations";
    const CONTENT_DEPENDENT: bool = true;
    type Params = IndustrialOrganisationsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = IndustrialOrganisationsReply;
    type Context = names::Context;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![IndustrialOrganisationsParams::decl(), IndustrialOrganisationsReply::decl(), OrganisationNow::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_branch(&params.branch_id)?;
        validate_country(&params.country)
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = names::ORGANISATION_TABLES.to_vec();
        tables.push(TableId::IndustrialOrganisationDetails);
        tables
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        let owned = || organisations_of(held, owner).unwrap_or_default();
        match step {
            0 => names::ORGANISATION_WHOLE.iter().chain([&TableId::IndustrialOrganisations]).map(|&table| Read::all(table)).collect(),
            1 => vec![Read::these(TableId::IndustrialOrganisationDetails, owned().iter().map(|row| u128::from(row.organisation)))],
            2 => {
                let mut tokens: BTreeSet<u32> = owned().iter().map(|row| row.token).collect();
                let policies = held.complete::<tables::IndustrialOrganisationDetails>().ok().into_iter().flatten().map(|detail| detail.state.policy);
                tokens.extend(policies.filter(|&policy| policy != UNSELECTED_POLICY));
                names::spelling_reads(held, tokens, false)
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        names::context(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Industrial organisations country is unavailable at this hour"))?;
        let mut reply = IndustrialOrganisationsReply { branch_id: params.branch_id.clone(), tag: at.holders.name(&params.country, at.hour), organisations: Vec::new(), missing: None, details_missing: None, icons: Vec::new(), warnings: Vec::new() };
        let rows = match organisations_of(frame, owner) {
            Ok(rows) => rows,
            Err(error) => {
                reply.missing = Some(format!("Industrial organisations are not recorded at this hour: {error}"));
                return Ok(reply);
            }
        };
        let details = frame.complete::<tables::IndustrialOrganisationDetails>();
        if let Err(error) = &details {
            reply.details_missing = Some(format!("Organisation sizes, funds and policies are not recorded at this hour: {error}"));
        }
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        let texts: Option<&dyn Texts> = at.context.as_ref().ok().map(|content| content.catalogue.as_ref() as &dyn Texts);
        let ids: Vec<u64> = rows.iter().map(|row| row.organisation).collect();
        let mut named = Named::default();
        names::name(frame, &[], &ids, at.context, &mut named);
        let details_table = TableId::IndustrialOrganisationDetails;
        let numbers = dashboard_declare::field(details_table, "state.funds_raw").and_then(|funds| Ok((funds, dashboard_declare::field(details_table, "state.points")?)));
        match numbers {
            Ok((funds, points)) => {
                for row in &rows {
                    let (id, key) = (row.organisation.to_string(), u128::from(row.organisation));
                    at.live.value(&id, "funds", Cell { table: details_table, column: funds, key }, 1.0);
                    at.live.value(&id, "points", Cell { table: details_table, column: points, key }, 1.0);
                }
            }
            Err(error) => reply.warnings.push(format!("Organisation funds and points cannot be read: {error}")),
        }
        for row in rows {
            let name = named.organisations.remove(&row.organisation);
            let detail = details.as_ref().ok().and_then(|rows| rows.binary_search_by_key(&row.organisation, |detail| detail.organisation).ok().map(|at| &rows[at]));
            let selected = detail.map(|detail| detail.state.policy).filter(|&policy| policy != UNSELECTED_POLICY);
            let policy_token = selected.and_then(|policy| lexical.and_then(|lexical| names::spelled(lexical, policy)));
            let policy_unknown = match (selected, &policy_token, texts) {
                (None, ..) => None,
                (Some(_), None, _) => Some("lexical tokens does not spell the selected policy.".to_owned()),
                (Some(_), Some(_), None) => at.context.as_ref().err().cloned(),
                (Some(_), Some(token), Some(texts)) => (!texts.has(token)).then(|| "The content does not name the selected policy.".to_owned()),
            };
            let policy_name = policy_token.as_deref().and_then(|token| texts.and_then(|texts| texts.text(token)));
            reply.organisations.push(OrganisationNow {
                id: row.organisation.to_string(),
                token: name.as_ref().map(|name| name.token.clone()).unwrap_or_default(),
                name: name.as_ref().and_then(|name| name.name.clone()),
                name_unknown: name.as_ref().and_then(|name| name.name_unknown.clone()),
                icon: name.as_ref().and_then(|name| name.icon.clone()),
                icon_unknown: name.as_ref().and_then(|name| name.icon_unknown.clone()),
                size: detail.map(|detail| detail.state.size),
                traits: detail.map(|detail| detail.unlocked.len() as u32),
                policy_token,
                policy_name,
                policy_unknown,
                cooldown_hour: detail.and_then(|detail| detail.state.cooldown_hour),
            });
        }
        reply.organisations.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.id.len().cmp(&b.id.len())).then_with(|| a.id.cmp(&b.id)));
        reply.warnings.extend(std::mem::take(&mut named.warnings));
        reply.icons = named.icons();
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn push_span(spans: &mut Vec<HourSpan>, first: i32, last: i32) {
    match spans.last_mut() {
        Some(held) if held.last_hour.checked_add(1) >= Some(first) => held.last_hour = held.last_hour.max(last),
        _ => spans.push(HourSpan { first_hour: first, last_hour: last }),
    }
}

impl Product for EquipmentDesigns {
    const KEY: &'static str = "equipment-designs";
    const CONTENT_DEPENDENT: bool = true;
    type Params = EquipmentDesignsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = EquipmentDesignsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EquipmentDesignsParams::decl(), EquipmentDesignsReply::decl(), DesignHistory::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_branch(&params.branch_id)?;
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let content = names::context(source, context)?;
        let recording = query.replay.recording();
        let creator = dashboard_declare::field(TableId::VariantDefinitions, "creator_link")?;
        let obsolete = dashboard_declare::field(TableId::VariantDefinitions, "obsolete")?;
        let mut designs: BTreeMap<u64, (Vec<HourSpan>, Vec<HourSpan>)> = BTreeMap::new();
        let mut last_seen: BTreeMap<u64, (u32, i32)> = BTreeMap::new();
        for stretch in query.timeline.within(query.timeline.first, query.timeline.last) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let held: Vec<_> = query.holders.within(&params.country, stretch.first, stretch.last).into_iter().filter(|run| matches!(run.holder, Holder::Found { .. })).collect();
            let (Some(first), Some(last)) = (held.first().map(|run| run.first), held.last().map(|run| run.last)) else { continue };
            let links = dashboard_query::cost::storage(|| dashboard_base::queries::country_ids::run_links(recording, stretch.run))?;
            let creators = read::runs(recording, stretch.run, TableId::VariantDefinitions, Column::Cell(creator), None, first, last)?;
            let mut flags = read::runs(recording, stretch.run, TableId::VariantDefinitions, Column::Cell(obsolete), None, first, last)?;
            for (key, runs) in creators {
                let flag_runs = flags.remove(&key).unwrap_or_default();
                for run in &held {
                    let Holder::Found { slot, .. } = run.holder else { continue };
                    for &(from, to, value) in &runs {
                        let (from, to) = (from.max(run.first), to.min(run.last));
                        let Some(raw) = value else { continue };
                        if from > to {
                            continue;
                        }
                        for (_, from, to) in links.within(raw as u32, from, to).filter(|(linked, ..)| *linked == Some(slot)) {
                            let entry = designs.entry(key as u64).or_default();
                            push_span(&mut entry.0, from, to);
                            for &(flag_from, flag_to, flag) in &flag_runs {
                                let (flag_from, flag_to) = (flag_from.max(from), flag_to.min(to));
                                if flag == Some(1) && flag_from <= flag_to {
                                    push_span(&mut entry.1, flag_from, flag_to);
                                }
                            }
                            let seen = last_seen.entry(key as u64).or_insert((stretch.run, to));
                            if to >= seen.1 {
                                *seen = (stretch.run, to);
                            }
                        }
                    }
                }
            }
        }
        let mut by_hour: BTreeMap<(u32, i32), Vec<u64>> = BTreeMap::new();
        for (&id, &at) in &last_seen {
            by_hour.entry(at).or_default().push(id);
        }
        let mut named = Named::default();
        for ((run, hour), ids) in &by_hour {
            dashboard_text::metadata::check_cancel(cancelled)?;
            names::name_at(recording, *run, *hour, ids, &[], &content, &mut named)?;
        }
        let mut warnings: Vec<String> = std::mem::take(&mut named.warnings).into_iter().collect();
        if !query.holders.named(&params.country) {
            warnings.push(query.holders.say(&params.country, query.timeline.last, |name| format!("{name} is held by no country in this branch")));
        }
        let icons = named.icons();
        let reply = EquipmentDesignsReply {
            branch_id: params.branch_id.clone(),
            tag: query.holders.name(&params.country, query.timeline.last),
            designs: designs
                .into_iter()
                .filter_map(|(id, (lives, obsolete))| Some(DesignHistory { id: id.to_string(), lives, obsolete, name: named.variants.remove(&id)? }))
                .collect(),
            known: held_hours(&query, &params.country, None, query.timeline.first, query.timeline.last),
            icons,
            warnings,
        };
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
