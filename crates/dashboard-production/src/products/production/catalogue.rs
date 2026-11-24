use dashboard_world::products::equipment_names::{self as names, EquipmentIcon, Named, VariantName};

use dashboard_base::products::{Country, validate_country, flows::{CountryFlowReply, FlowEdge, FlowPart, FlowShape, build_flows}};
use dashboard_base::{Source, products::{LoadContext, check_reply_budget, at_hour::{At, AtHour, Owner, Read}}};
use dashboard_follow::live::Source as Cell;
use dashboard_query::coverage::{Witness, witnessed};
use dashboard_text::{metadata::link_slot, variant_names::Texts};
use replay_data::rows::{CountryLinkRow, CountryRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    io,
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

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductionLicencesParams {
    pub branch_id: String,
    #[doc = " One country: the licences it holds and grants by partner. Otherwise the links between `countries` (every country when empty)."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub subject: Option<Country>,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default)]
    pub between: bool,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub limit: Option<u32>,
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
    #[doc = " Why this organisation's state (size, funds, traits, policy) is not known at the hour, when it is not."]
    pub details_missing: Option<String>,
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
pub struct DesignNow {
    pub name: VariantName,
    #[doc = " Flagged obsolete at the hour."]
    pub obsolete: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EquipmentDesignsReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " The country's saved designs at the hour, current before obsolete, then by name."]
    pub designs: Vec<DesignNow>,
    #[doc = " Why the designs are not known at the hour, when they are not."]
    pub missing: Option<String>,
    pub icons: Vec<EquipmentIcon>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(IndustrialOrganisationsReply, EquipmentDesignsReply);

fn slot_tags(countries: &[CountryRow]) -> BTreeMap<u32, String> {
    countries.iter().filter_map(|row| Some((row.canonical_slot, std::str::from_utf8(&row.tag).ok()?.to_owned()))).collect()
}

const LICENCES_NOT_RECORDED: &str = "A licence was not recorded at this hour";
const LICENCE_UNLINKED: &str = "Some licences name a country the recording cannot link at this hour; they are left out";

pub fn licence_edges(records: &[replay_data::rows::ProductionLicenseRecordRow], links: &[CountryLinkRow], tags: &BTreeMap<u32, String>) -> (Vec<FlowEdge>, bool) {
    let tag = |raw: u32| link_slot(links, raw).and_then(|slot| tags.get(&slot).cloned());
    let mut out = Vec::with_capacity(records.len());
    let mut unlinked = false;
    for row in records {
        match (tag(row.giver), tag(row.owner)) {
            (Some(from), Some(to)) => out.push(FlowEdge { from, to, part: 0, amount: Some(1) }),
            _ => unlinked = true,
        }
    }
    (out, unlinked)
}

fn licence_parts() -> Vec<FlowPart> {
    vec![FlowPart { key: "licences".to_owned(), name: Some("Licences".to_owned()), name_missing: None, ordinal: 0, icon: None }]
}

impl AtHour for ProductionLicences {
    const KEY: &'static str = "production-licences";
    const TITLE: &'static str = "Production licences";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = ProductionLicencesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, subject, countries, "countries[]", between, limit];
    type Reply = CountryFlowReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ProductionLicencesParams::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_branch(&params.branch_id)?;
        params.countries.iter().try_for_each(validate_country)?;
        params.subject.as_ref().map_or(Ok(()), validate_country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::ProductionLicenseRecords, TableId::Countries, TableId::CountryLinks]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let empty = |missing: String| CountryFlowReply::not_recorded(missing, 0, Vec::new());
        let tables = (frame.complete::<tables::ProductionLicenseRecords>(), frame.complete::<tables::CountryLinks>(), frame.complete::<tables::Countries>());
        let (records, links, countries) = match tables {
            (Ok(records), Ok(links), Ok(countries)) => (records, links, countries),
            (records, links, countries) => {
                let why = [records.err(), links.err(), countries.err()].into_iter().flatten().map(|error| error.to_string()).collect::<Vec<_>>().join("; ");
                return Ok(empty(format!("Licences are not recorded at this hour: {why}")));
            }
        };
        let (edges, unlinked) = licence_edges(records, links, &slot_tags(countries));
        let resolve = |country: &Country| at.holders.tag_at(country, at.hour).map(str::to_owned).or_else(|| country.tag().map(str::to_owned));
        let subject = params.subject.as_ref().and_then(resolve);
        if params.subject.is_some() && subject.is_none() {
            return Ok(empty("The chosen country holds no tag at this hour".to_owned()));
        }
        let chosen: Option<HashSet<String>> = (!params.countries.is_empty()).then(|| params.countries.iter().filter_map(resolve).collect());
        let shape = FlowShape {
            subject: subject.as_deref(),
            chosen: chosen.as_ref(),
            between: params.between,
            limit: params.limit.map(|limit| limit.max(1) as usize),
            table: false,
            not_recorded: LICENCES_NOT_RECORDED,
        };
        let mut reply = build_flows(&edges, licence_parts(), 0, &shape);
        reply.partial = unlinked.then(|| LICENCE_UNLINKED.to_owned());
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
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
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
                let policies = owned().into_iter().filter_map(|row| witnessed::<tables::IndustrialOrganisationDetails>(held, u128::from(row.organisation)).row()).map(|detail| detail.state.policy);
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
        let mut reply = IndustrialOrganisationsReply { branch_id: params.branch_id.clone(), tag: at.holders.name(&params.country, at.hour), organisations: Vec::new(), missing: None, icons: Vec::new(), warnings: Vec::new() };
        let rows = match organisations_of(frame, owner) {
            Ok(rows) => rows,
            Err(error) => {
                reply.missing = Some(format!("Industrial organisations are not recorded at this hour: {error}"));
                return Ok(reply);
            }
        };
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
            let (detail, details_missing) = match witnessed::<tables::IndustrialOrganisationDetails>(frame, u128::from(row.organisation)) {
                Witness::Row(detail) => (Some(detail), None),
                Witness::Absent => (None, Some("The organisation's size, funds and policy are not recorded at this hour".to_owned())),
                Witness::NotRecorded(reason) => (None, Some(format!("Organisation sizes, funds and policies are not recorded at this hour: {reason}"))),
            };
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
                details_missing,
            });
        }
        reply.organisations.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.id.len().cmp(&b.id.len())).then_with(|| a.id.cmp(&b.id)));
        reply.warnings.extend(std::mem::take(&mut named.warnings));
        reply.icons = named.icons();
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn designs_of(frame: &DecodedFrame, owner: Owner) -> io::Result<Vec<&replay_data::rows::VariantDefinitionRow>> {
    let links = frame.complete::<tables::CountryLinks>()?;
    Ok(frame.complete::<tables::VariantDefinitions>()?.iter().filter(|row| link_slot(links, row.creator_link) == Some(owner.slot)).collect())
}

impl AtHour for EquipmentDesigns {
    const KEY: &'static str = "equipment-designs";
    const TITLE: &'static str = "Equipment designs";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = EquipmentDesignsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = EquipmentDesignsReply;
    type Context = names::Context;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![EquipmentDesignsParams::decl(), EquipmentDesignsReply::decl(), DesignNow::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_branch(&params.branch_id)?;
        validate_country(&params.country)
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        names::TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => names::WHOLE.iter().chain([&TableId::VariantDefinitions]).map(|&table| Read::all(table)).collect(),
            1 => vec![names::capital_state_read(held)],
            2 => names::spelling_reads(held, designs_of(held, owner).unwrap_or_default().iter().map(|row| row.equipment_token).collect(), true),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        names::context(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Equipment designs country is unavailable at this hour"))?;
        let mut reply = EquipmentDesignsReply { branch_id: params.branch_id.clone(), tag: at.holders.name(&params.country, at.hour), designs: Vec::new(), missing: None, icons: Vec::new(), warnings: Vec::new() };
        let rows = match designs_of(frame, owner) {
            Ok(rows) => rows,
            Err(error) => {
                reply.missing = Some(format!("Equipment designs are not recorded at this hour: {error}"));
                return Ok(reply);
            }
        };
        let ids: Vec<u64> = rows.iter().map(|row| row.id).collect();
        let mut named = Named::default();
        names::name(frame, &ids, &[], at.context, &mut named);
        reply.designs = rows
            .iter()
            .filter_map(|row| Some(DesignNow { name: named.variants.remove(&row.id)?, obsolete: row.obsolete != 0 }))
            .collect();
        reply.designs.sort_by(|a, b| a.obsolete.cmp(&b.obsolete).then_with(|| a.name.name.cmp(&b.name.name)).then_with(|| a.name.id.len().cmp(&b.name.id.len())).then_with(|| a.name.id.cmp(&b.name.id)));
        reply.warnings.extend(std::mem::take(&mut named.warnings));
        reply.icons = named.icons();
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
