use super::{
    country_goals::faction_of,
    details::{DetailContent, FactionDetails},
    goals::{self, Texts},
    ordinals, owned,
};
use dashboard_base::{
    Source,
    catalogue,
    products::{
        Country, LoadContext,
        at_hour::{At, AtHour, Clock, Keys, Owner, Read, ReadsAt},
        resources::{Learned, RESOURCE_DECIMALS, ResourceNaming},
    },
};
use dashboard_text::metadata::link_slot;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct FactionPools;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum FactionPoolKind {
    Resources,
    Manpower,
    Research,
    Doctrine,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionPoolsParams {
    pub branch_id: String,
    pub country: Option<Country>,
    #[doc = " Explicit recorded faction id as a decimal string; none follows `country`'s faction at the hour."]
    pub faction: Option<String>,
    pub pool: FactionPoolKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum FactionPoolStatus {
    Ok,
    NoFaction,
    FactionAbsent,
    MembershipNotRecorded,
    PoolLocked,
    PoolNotRecorded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionPoolResource {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    #[doc = " Exact pooled amount fixed-point at `decimals`; null when not recorded."]
    pub amount: Option<String>,
    pub name_reason: Option<String>,
    pub icon_reason: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionPoolManpower {
    pub ordinal: u32,
    pub slot: Option<u32>,
    pub tag: Option<String>,
    pub country_reason: Option<String>,
    #[doc = " Exact manpower contributed; null when not recorded."]
    pub amount: Option<String>,
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionPoolResearchMember {
    pub slot: Option<u32>,
    pub tag: Option<String>,
    pub country_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionPoolResearch {
    #[doc = " Exact scientist base fixed-point at `decimals`; null when not recorded."]
    pub scientist_base: Option<String>,
    pub scientist_base_missing: Option<String>,
    pub members: Vec<FactionPoolResearchMember>,
    pub members_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionPoolDoctrine {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionPoolsReply {
    pub pool: FactionPoolKind,
    pub status: FactionPoolStatus,
    pub reason: Option<String>,
    pub faction: Option<String>,
    pub decimals: u8,
    pub resources: Vec<FactionPoolResource>,
    pub manpower: Vec<FactionPoolManpower>,
    pub manpower_total: Option<String>,
    pub research: Option<FactionPoolResearch>,
    pub doctrines: Vec<FactionPoolDoctrine>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FactionPoolsReply);

fn decimals_for(pool: FactionPoolKind) -> u8 {
    match pool {
        FactionPoolKind::Resources | FactionPoolKind::Research => RESOURCE_DECIMALS,
        FactionPoolKind::Manpower | FactionPoolKind::Doctrine => 0,
    }
}

fn blank(pool: FactionPoolKind, status: FactionPoolStatus, reason: Option<String>, faction: Option<u64>) -> FactionPoolsReply {
    FactionPoolsReply {
        pool,
        status,
        reason,
        faction: faction.map(|id| id.to_string()),
        decimals: decimals_for(pool),
        resources: Vec::new(),
        manpower: Vec::new(),
        manpower_total: None,
        research: None,
        doctrines: Vec::new(),
        warnings: Vec::new(),
    }
}

fn resolve_faction(frame: &DecodedFrame, params: &FactionPoolsParams, owner: Option<Owner>) -> Result<Option<u64>, (FactionPoolStatus, String)> {
    if let Some(explicit) = params.faction.as_deref() {
        let faction = super::faction_id(explicit).map_err(|why| (FactionPoolStatus::MembershipNotRecorded, why.to_owned()))?;
        let rows = frame
            .complete::<tables::Factions>()
            .map_err(|why| (FactionPoolStatus::MembershipNotRecorded, format!("Factions are not recorded at this hour: {why}")))?;
        if rows.iter().any(|row| row.id == faction) {
            return Ok(Some(faction));
        }
        return Err((FactionPoolStatus::FactionAbsent, "This faction did not exist at this time.".into()));
    }
    let Some(owner) = owner else {
        return Ok(None);
    };
    let members = frame
        .complete::<tables::FactionMembers>()
        .map_err(|why| (FactionPoolStatus::MembershipNotRecorded, format!("Faction rosters were not recorded at this time: {why}")))?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| (FactionPoolStatus::MembershipNotRecorded, format!("Country links are not recorded at this hour: {why}")))?;
    Ok(faction_of(members, links, owner.slot))
}

fn pool_reads(pool: FactionPoolKind, faction: u64, include_links: bool) -> Vec<Read> {
    let (low, high) = ordinals(faction);
    let mut reads = match pool {
        FactionPoolKind::Resources => vec![
            Read { table: TableId::FactionResources, keys: Keys::Range(low, high) },
            Read::all(TableId::ResourceDefinitions),
            Read::all(TableId::LexicalTokens),
        ],
        FactionPoolKind::Manpower => vec![
            Read::these(TableId::FactionUnlocks, [u128::from(faction)]),
            Read { table: TableId::FactionManpower, keys: Keys::Range(low, high) },
            Read::all(TableId::ReservedCountryNaming),
        ],
        FactionPoolKind::Research => vec![
            Read::these(TableId::FactionUnlocks, [u128::from(faction)]),
            Read { table: TableId::FactionResearch, keys: Keys::Range(low, high) },
            Read::all(TableId::ReservedCountryNaming),
        ],
        FactionPoolKind::Doctrine => vec![
            Read { table: TableId::FactionDoctrine, keys: Keys::Range(low, high) },
            Read::all(TableId::LexicalTokens),
            Read::all(TableId::LocalPlayer),
            Read::all(TableId::Countries),
        ],
    };
    if include_links && matches!(pool, FactionPoolKind::Manpower | FactionPoolKind::Research | FactionPoolKind::Doctrine) {
        reads.push(Read::all(TableId::CountryLinks));
    }
    reads
}

fn doctrine_needs_scripted(frame: &DecodedFrame, faction: u64, cat: &dashboard_base::catalogue::Catalogue) -> bool {
    let Ok(rows) = frame.complete::<tables::FactionDoctrine>() else { return false };
    owned(rows, faction, |row| row.faction).iter().any(|row| {
        dashboard_text::faction_names::template_spelling(frame, Some(row.token))
            .ok()
            .flatten()
            .and_then(|spelling| cat.localisation.localise(spelling.as_bytes()).ok())
            .is_some_and(|text| text.contains('['))
    })
}

fn resolve_member_country(frame: &DecodedFrame, raw: u32) -> (Option<u32>, Option<String>, Option<String>) {
    if raw == 0 {
        let reserved = frame.complete::<tables::ReservedCountryNaming>().map_err(|why| why.to_string()).and_then(|rows| match rows {
            [row] => String::from_utf8(row.tag.clone()).map_err(|_| "The reserved country's tag is not UTF-8".to_owned()),
            _ => Err("Country zero has no unique reserved name".to_owned()),
        });
        return match reserved {
            Ok(tag) => (None, Some(tag), None),
            Err(why) => (None, None, Some(why)),
        };
    }
    match frame.complete::<tables::CountryLinks>() {
        Ok(links) => match link_slot(links, raw) {
            Some(slot) => (Some(slot), None, None),
            None => (None, None, Some(format!("Country {raw} has no link at this hour"))),
        },
        Err(why) => (None, None, Some(format!("Country links: {why}"))),
    }
}

fn build_resources(frame: &DecodedFrame, faction: u64, naming: &ResourceNaming, cancelled: &AtomicBool, reply: &mut FactionPoolsReply) -> io::Result<()> {
    let rows = match frame.complete::<tables::FactionResources>() {
        Ok(rows) => owned(rows, faction, |row| row.faction),
        Err(why) => {
            reply.status = FactionPoolStatus::PoolNotRecorded;
            reply.reason = Some(format!("Faction pooled resources were not recorded at this hour: {why}"));
            return Ok(());
        }
    };
    if rows.iter().enumerate().any(|(index, row)| row.ordinal as usize != index) {
        reply.status = FactionPoolStatus::PoolNotRecorded;
        reply.reason = Some("Faction resource slots are not contiguous".into());
        return Ok(());
    }
    let definitions = frame.complete::<tables::ResourceDefinitions>();
    let mut learned = Learned::default();
    if let (Ok(definitions), Ok(lexical)) = (&definitions, frame.complete::<tables::LexicalTokens>()) {
        learned.learn(definitions, lexical);
    }
    let (infos, warnings) = naming.infos(&learned.list(), cancelled)?;
    reply.warnings.extend(warnings);
    let mut held: Vec<(i64, u32, FactionPoolResource)> = Vec::new();
    for row in rows {
        if row.ordinal == 0 && row.raw == 0 {
            continue;
        }
        let mut detail = FactionPoolResource {
            ordinal: row.ordinal,
            token: None,
            name: None,
            icon: None,
            amount: Some(row.raw.to_string()),
            name_reason: None,
            icon_reason: None,
            missing: None,
        };
        let named = (|| -> Result<(), String> {
            let definition = definitions
                .as_ref()
                .map_err(ToString::to_string)?
                .iter()
                .find(|definition| definition.id == row.ordinal)
                .ok_or("Faction resource slot has no recorded definition")?;
            if definition.token != row.token {
                return Err("Faction resource slot token disagrees with its recorded definition".into());
            }
            let info = infos.iter().find(|info| info.id == row.ordinal).ok_or("Faction resource token spelling unavailable")?;
            detail.token = Some(info.token.clone());
            detail.name = info.name.clone();
            detail.icon = info.icon.clone();
            detail.name.as_ref().map(|_| ()).ok_or_else(|| "Faction resource name unavailable in recorded content".to_owned())
        })();
        if let Err(why) = named {
            detail.name_reason = Some(why);
        }
        if detail.icon.is_none() {
            detail.icon_reason = Some(
                detail
                    .name_reason
                    .clone()
                    .unwrap_or_else(|| "The current installation has no icon for this resource".to_owned()),
            );
        }
        held.push((row.raw, row.ordinal, detail));
    }
    held.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    reply.resources = held.into_iter().map(|(_, _, item)| item).collect();
    Ok(())
}

fn check_unlocked(frame: &DecodedFrame, faction: u64, pick: impl Fn(&replay_data::rows::FactionUnlockRow) -> u8, reply: &mut FactionPoolsReply) -> bool {
    match frame.complete::<tables::FactionUnlocks>() {
        Ok(rows) => match owned(rows, faction, |row| row.faction).first() {
            Some(row) if pick(row) == 0 => {
                reply.status = FactionPoolStatus::PoolLocked;
                reply.reason = Some("This faction has not unlocked this pool at this time.".into());
                false
            }
            Some(_) => true,
            None => {
                reply.warnings.push("Faction unlocks: the faction has no row at this hour".into());
                true
            }
        },
        Err(why) => {
            reply.warnings.push(format!("Faction unlocks: {why}"));
            true
        }
    }
}

fn build_manpower(frame: &DecodedFrame, faction: u64, reply: &mut FactionPoolsReply) {
    if !check_unlocked(frame, faction, |row| row.manpower, reply) {
        return;
    }
    let rows = match frame.complete::<tables::FactionManpower>() {
        Ok(rows) => owned(rows, faction, |row| row.faction),
        Err(why) => {
            reply.status = FactionPoolStatus::PoolNotRecorded;
            reply.reason = Some(format!("Faction manpower was not recorded at this hour: {why}"));
            return;
        }
    };
    if rows.iter().enumerate().any(|(index, row)| row.ordinal as usize != index) {
        reply.status = FactionPoolStatus::PoolNotRecorded;
        reply.reason = Some("Faction manpower ordinals are not contiguous".into());
        return;
    }
    let mut held: Vec<(i32, u32, FactionPoolManpower)> = Vec::with_capacity(rows.len());
    let mut total = 0i128;
    for row in rows {
        let (slot, tag, country_reason) = resolve_member_country(frame, row.country);
        if let Some(why) = &country_reason {
            reply.warnings.push(format!("Faction manpower entry {}: {why}", row.ordinal));
        }
        total += i128::from(row.value);
        held.push((
            row.value,
            row.ordinal,
            FactionPoolManpower {
                ordinal: row.ordinal,
                slot,
                tag,
                country_reason,
                amount: Some(row.value.to_string()),
                missing: None,
            },
        ));
    }
    held.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    reply.manpower = held.into_iter().map(|(_, _, item)| item).collect();
    if !reply.manpower.is_empty() {
        reply.manpower_total = Some(total.to_string());
    }
}

fn build_research(frame: &DecodedFrame, faction: u64, reply: &mut FactionPoolsReply) {
    if !check_unlocked(frame, faction, |row| row.research, reply) {
        return;
    }
    let rows = match frame.complete::<tables::FactionResearch>() {
        Ok(rows) => owned(rows, faction, |row| row.faction),
        Err(why) => {
            reply.status = FactionPoolStatus::PoolNotRecorded;
            reply.reason = Some(format!("Faction research sharing was not recorded at this hour: {why}"));
            return;
        }
    };
    let Some(header) = rows.first().filter(|row| row.ordinal == 0) else {
        reply.status = FactionPoolStatus::PoolNotRecorded;
        reply.reason = Some("Faction research sharing has no row at this hour".into());
        return;
    };
    let members = header
        .countries
        .iter()
        .map(|raw| {
            let (slot, tag, country_reason) = resolve_member_country(frame, *raw);
            if let Some(why) = &country_reason {
                reply.warnings.push(format!("Faction research member: {why}"));
            }
            FactionPoolResearchMember { slot, tag, country_reason }
        })
        .collect();
    reply.research = Some(FactionPoolResearch {
        scientist_base: Some(header.scientist_base_raw.to_string()),
        scientist_base_missing: None,
        members,
        members_missing: None,
    });
}

fn build_doctrines(frame: &DecodedFrame, faction: u64, params: &FactionPoolsParams, at: &At<'_, DetailContent>, reply: &mut FactionPoolsReply) {
    let rows = match frame.complete::<tables::FactionDoctrine>() {
        Ok(rows) => owned(rows, faction, |row| row.faction),
        Err(why) => {
            reply.status = FactionPoolStatus::PoolNotRecorded;
            reply.reason = Some(format!("Faction shared doctrines were not recorded at this hour: {why}"));
            return;
        }
    };
    let cat = at.context.catalogue().ok();
    let texts = cat.map(|cat| Texts::new(frame, faction, cat, at.context.art(), params.country.as_ref()));
    reply.doctrines = rows
        .iter()
        .map(|row| {
            let mut detail = FactionPoolDoctrine { ordinal: row.ordinal, token: None, name: None, reason: None };
            let named = (|| -> Result<String, String> {
                let spelling = dashboard_text::faction_names::template_spelling(frame, Some(row.token))?.ok_or("Faction doctrine token is absent")?;
                detail.token = Some(spelling.clone());
                let texts = texts.as_ref().ok_or("Faction doctrine names need the recorded game folders")?;
                let text = texts.localise(&spelling)?;
                let text = if text.contains('[') { texts.expand(&text)? } else { text };
                catalogue::plain_text(&text)
            })();
            match named {
                Ok(name) => detail.name = Some(name),
                Err(why) => detail.reason = Some(why),
            }
            detail
        })
        .collect();
}

impl AtHour for FactionPools {
    const KEY: &'static str = "faction-pools";
    const TITLE: &'static str = "Faction pooled strength";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: Clock = Clock::InputsOnly;
    const REPLY_NAMES_NO_HOUR: bool = true;
    type Params = FactionPoolsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, faction, pool];
    type Reply = FactionPoolsReply;
    type Context = DetailContent;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid faction branch")?;
        if let Some(country) = &params.country {
            dashboard_base::products::validate_country(country)?;
        }
        if let Some(faction) = &params.faction {
            super::faction_id(faction)?;
        }
        if params.country.is_none() && params.faction.is_none() {
            return Err("Choose a country or faction");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            FactionPoolKind::decl(),
            FactionPoolsParams::decl(),
            FactionPoolStatus::decl(),
            FactionPoolResource::decl(),
            FactionPoolManpower::decl(),
            FactionPoolResearchMember::decl(),
            FactionPoolResearch::decl(),
            FactionPoolDoctrine::decl(),
            FactionPoolsReply::decl(),
        ]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        if params.faction.is_some() { None } else { params.country.as_ref() }
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let mut tables = Vec::new();
        if params.faction.is_some() {
            tables.push(TableId::Factions);
        } else {
            tables.extend([TableId::FactionMembers, TableId::CountryLinks]);
        }
        tables.extend(pool_reads(params.pool, 0, params.faction.is_some()).into_iter().map(|read| read.table));
        if params.pool == FactionPoolKind::Doctrine {
            tables.extend(goals::text_tables());
        }
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    fn reads(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        if let Some(explicit) = params.faction.as_deref() {
            let Ok(faction) = super::faction_id(explicit) else { return Vec::new() };
            return match step {
                0 => {
                    let mut reads = vec![Read::all(TableId::Factions)];
                    reads.extend(pool_reads(params.pool, faction, true));
                    reads
                }
                _ => Vec::new(),
            };
        }
        match (step, owner) {
            (0, Some(_)) => vec![Read::all(TableId::FactionMembers), Read::all(TableId::CountryLinks)],
            (1, Some(owner)) => match resolve_faction(held, params, Some(owner)) {
                Ok(Some(faction)) => pool_reads(params.pool, faction, false),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let scripted_step = if params.faction.is_some() { 1 } else { 2 };
        if params.pool == FactionPoolKind::Doctrine && step == scripted_step {
            let Ok(cat) = at.context.catalogue() else { return Vec::new() };
            let Ok(Some(faction)) = resolve_faction(held, params, owner) else { return Vec::new() };
            if !doctrine_needs_scripted(held, faction, cat) {
                return Vec::new();
            }
            let read: Vec<TableId> = pool_reads(params.pool, faction, true).into_iter().map(|read| read.table).collect();
            return goals::text_tables().filter(|table| !read.contains(table)).map(Read::all).collect();
        }
        Self::reads(params, owner, step, held)
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        <FactionDetails as AtHour>::context(source, load)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let faction = match resolve_faction(frame, params, at.owner) {
            Ok(Some(faction)) => faction,
            Ok(None) => return Ok(blank(params.pool, FactionPoolStatus::NoFaction, None, None)),
            Err((status, reason)) => return Ok(blank(params.pool, status, Some(reason), None)),
        };
        let mut reply = blank(params.pool, FactionPoolStatus::Ok, None, Some(faction));
        match params.pool {
            FactionPoolKind::Resources => build_resources(frame, faction, at.context.naming(), at.cancelled, &mut reply)?,
            FactionPoolKind::Manpower => build_manpower(frame, faction, &mut reply),
            FactionPoolKind::Research => build_research(frame, faction, &mut reply),
            FactionPoolKind::Doctrine => build_doctrines(frame, faction, params, at, &mut reply),
        }
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
