
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::LoadContext;
use dashboard_base::{Source, catalogue::{self, CachedResult, Catalogue}, products::at_hour::{At, AtHour, Keys, Read}};
use dashboard_base::products::{
    artwork::sprites::{self, SpriteFrame},
};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_text::catalogue::View;
use replay_data::rows::{
    AirMissionCountRow, AirTheatreGroupRow, AirTheatreMemberRow, AirTheatreRow, AirWingRow, CountryLinkRow, Recorded, TableId, tables,
};
use replay_derive::{
    air_mission_counts::{MISSION_ENTRY_TEMPLATE, MISSIONS},
    air_mission_sources::{AirRegistrySources, MissionCountrySources, canonical, effective_raw},
    aircraft_count::{SortieSources, efficiency_override},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
use replay_data::rows::AirRegionalMissionRow;

#[cfg(test)]
mod tests;

const MISSION_KEYS: [&str; MISSIONS] = [
    "AIRWING_MISSION_TYPE_AIR_SUPERIORITY",
    "AIRWING_MISSION_TYPE_CAS",
    "AIRWING_MISSION_TYPE_INTERCEPTION",
    "AIRWING_MISSION_TYPE_STRATEGIC_BOMBER",
    "AIRWING_MISSION_TYPE_NAVAL_BOMBER",
    "AIRWING_MISSION_TYPE_DROP_NUKE",
    "AIRWING_MISSION_TYPE_PARADROP",
    "AIRWING_MISSION_TYPE_NAVAL_KAMIKAZE",
    "AIRWING_MISSION_TYPE_PORT_STRIKE",
    "AIRWING_MISSION_TYPE_ATTACK_LOGISTICS",
    "AIRWING_MISSION_TYPE_AIR_SUPPLY",
    "AIRWING_MISSION_TYPE_TRAINING",
    "AIRWING_MISSION_TYPE_NAVAL_MINE_PLANTING",
    "AIRWING_MISSION_TYPE_NAVAL_MINE_SWEEPING",
    "AIRWING_MISSION_TYPE_RECON",
    "AIRWING_MISSION_TYPE_NAVAL_PATROL",
    "AIRWING_MISSION_TYPE_BARRAGE",
    "AIRWING_MISSION_TYPE_SAM",
];

pub struct AirMissionsAt;
pub struct AirMissionWingsAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirMissionsParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirMissionWingsParams {
    pub branch_id: String,
    pub country: Country,
    pub region: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirMissionChoice {
    pub id: String,
    pub name: String,
    pub icon: Option<SpriteFrame>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirMissionRegion {
    pub region: String,
    pub name: Option<String>,
    #[doc = " Why the name is not shown, without one."]
    pub missing: Option<String>,
    #[doc = " The mission bits the region records: a wing of its registrations flies them."]
    pub missions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirMissionTheatre {
    pub id: String,
    pub name: String,
    pub regions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirMissionsReply {
    #[doc = " The hour of the registrations' read the rows hold; none when not known."]
    pub read_hour: Option<i32>,
    #[doc = " Why the missions are not shown at the hour, when they are not."]
    pub missing: Option<String>,
    #[doc = " The missions any region records, in bit order."]
    pub missions: Vec<AirMissionChoice>,
    pub regions: Vec<AirMissionRegion>,
    #[doc = " The owner's theatres, in the game's order; none where membership is not known."]
    pub theatres: Option<Vec<AirMissionTheatre>>,
    #[doc = " Why the theatres are not known at the hour."]
    pub theatres_missing: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirMissionWing {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    #[cfg_attr(any(test, feature = "export-types"), ts(rename = "type"))]
    pub aircraft_type: Option<String>,
    pub missions: Vec<String>,
    #[doc = " Raw fixed5 decimal text: 100,000 is one."]
    pub efficiency: Option<String>,
    #[doc = " Why a field above is not shown."]
    pub refusals: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirMissionWingsReply {
    pub read_hour: Option<i32>,
    pub region: u32,
    pub wings: Vec<AirMissionWing>,
    #[doc = " Why the region's wings are not shown at the hour, when they are not."]
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AirMissionsReply, AirMissionWingsReply);

pub struct Names {
    catalogue: Result<Arc<Catalogue>, String>,
    regions: Result<BTreeMap<u32, Result<String, String>>, String>,
    missions: Vec<Result<String, String>>,
    strip: Result<(String, Option<String>), String>,
    warnings: Vec<String>,
}

fn names(source: &Source, load: &LoadContext) -> Names {
    let mut warnings = Vec::new();
    let cat = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|error| format!("Recorded content stack unavailable; choose matching game and user folders ({error})"));
    let missions = MISSION_KEYS
        .iter()
        .map(|key| {
            let cat = cat.as_ref().map_err(Clone::clone)?;
            if cat.localisation.get(key).is_none() {
                return Err(format!("Localisation {key} is absent from the recorded content stack"));
            }
            dashboard_text::catalogue::localised(&cat.localisation, key, View::Plain)
        })
        .collect();
    let strip = cat.as_ref().map_err(Clone::clone).and_then(|cat| mission_strip(cat));
    let regions = cat.as_ref().map_err(Clone::clone).and_then(|cat| {
        warnings.extend(cat.verification.iter().cloned());
        warnings.extend(cat.warnings.iter().cloned());
        let content = dashboard_base::content::strategic_regions(&cat.content.layers).map_err(|error| error.to_string())?;
        warnings.extend(content.notes.iter().cloned());
        Ok(content.regions.keys().map(|&id| (id, dashboard_world::products::region_names::region_name(cat, &content, id))).collect())
    });
    Names { catalogue: cat, regions, missions, strip, warnings }
}

fn mission_strip(cat: &Catalogue) -> Result<(String, Option<String>), String> {
    let Some(recorded) = cat.recorded() else {
        return strip_in(cat).map(|key| (key, None));
    };
    let chosen = catalogue::present("air mission icons", cat, recorded, |drawing| strip_in(drawing).map(|key| (key, !std::ptr::eq(drawing, cat))));
    let (key, fell_back) = chosen.get().clone()?;
    if fell_back {
        return Ok((key.clone(), Some(key)));
    }
    let own = strip_in(recorded).ok().filter(|own| *own != key);
    Ok((key, own))
}

fn strip_in(cat: &Catalogue) -> Result<String, String> {
    let guis = cat.guis().map_err(|e| e.to_string())?;
    let button = guis
        .template(MISSION_ENTRY_TEMPLATE)
        .ok_or_else(|| format!("GUI template {MISSION_ENTRY_TEMPLATE} is absent"))?
        .find(content_reader_gui::gui::native::Kind::Button, "button")
        .and_then(|element| element.button())
        .ok_or_else(|| format!("GUI template {MISSION_ENTRY_TEMPLATE} has no button"))?;
    sprites::icon_key(cat, &button.sprite)
}

fn region_name(names: &Names, region: u32) -> (Option<String>, Option<String>) {
    match names.regions.as_ref().map(|regions| regions.get(&region)) {
        Ok(Some(Ok(name))) => (Some(name.clone()), None),
        Ok(Some(Err(why))) => (None, Some(why.clone())),
        Ok(None) => (None, Some(format!("Region {region} absent from the recorded stack"))),
        Err(why) => (None, Some(why.clone())),
    }
}

pub(crate) fn stored_in(cat: &Result<Arc<Catalogue>, String>, bytes: &[u8], what: &str) -> Result<String, String> {
    let cat = cat.as_ref().map_err(Clone::clone)?;
    dashboard_text::scripted_text::stored_text(&cat.localisation, &cat.recorded_language, bytes, what).and_then(|text| catalogue::plain_text(&text))
}

fn stored(names: &Names, bytes: &[u8], what: &str) -> Result<String, String> {
    stored_in(&names.catalogue, bytes, what)
}

pub(crate) fn mission_cells(row: &AirMissionCountRow) -> [Recorded<i32>; MISSIONS] {
    [
        row.mission_0, row.mission_1, row.mission_2, row.mission_3, row.mission_4, row.mission_5, row.mission_6, row.mission_7, row.mission_8,
        row.mission_9, row.mission_10, row.mission_11, row.mission_12, row.mission_13, row.mission_14, row.mission_15, row.mission_16, row.mission_17,
    ]
}

fn column(table: TableId, name: &str) -> io::Result<usize> {
    dashboard_declare::field(table, name)
}

pub(crate) fn reply(
    counts: io::Result<&[AirMissionCountRow]>,
    slot: u32,
    read_hour: Option<i32>,
    theatres: Result<Vec<AirMissionTheatre>, String>,
    names: &Names,
    live: &Cells,
) -> io::Result<AirMissionsReply> {
    let table = TableId::AirMissionCounts;
    let wings = column(table, "wings")?;
    let mission_columns = (0..MISSIONS).map(|bit| column(table, &format!("mission_{bit}"))).collect::<io::Result<Vec<_>>>()?;
    let mut warnings = names.warnings.clone();
    let mut shown = BTreeSet::new();
    let (regions, missing) = match counts {
        Err(error) => (Vec::new(), Some(format!("Air missions are not recorded at this hour: {error}"))),
        Ok(rows) => {
            let mut regions = Vec::new();
            for row in rows.iter().filter(|row| row.slot == slot) {
                let id = row.region.to_string();
                let key = u128::from(row.slot) << 32 | u128::from(row.region);
                live.value(&id, "wings", Cell { table, column: wings, key }, 1.0);
                let mut missions = Vec::new();
                for (bit, cell) in mission_cells(row).iter().enumerate() {
                    if let Recorded::Value(_) = cell {
                        live.value(&id, &format!("m{bit}"), Cell { table, column: mission_columns[bit], key }, 1.0);
                        shown.insert(bit);
                        missions.push(bit.to_string());
                    }
                }
                let (name, missing) = region_name(names, row.region);
                regions.push(AirMissionRegion { region: id, name, missing, missions });
            }
            (regions, None)
        }
    };
    let mut refused = BTreeMap::<String, usize>::new();
    let missions = shown
        .into_iter()
        .map(|bit| {
            let name = names.missions[bit].clone().unwrap_or_else(|why| {
                *refused.entry(why).or_default() += 1;
                "Mission name unavailable".to_owned()
            });
            let icon = match &names.strip {
                Ok((key, recorded)) => Some(SpriteFrame { key: key.clone(), frame: bit as u32, recorded: recorded.clone() }),
                Err(why) => {
                    *refused.entry(format!("icon: {why}")).or_default() += 1;
                    None
                }
            };
            AirMissionChoice { id: bit.to_string(), name, icon }
        })
        .collect();
    warnings.extend(refused.into_iter().map(|(why, count)| format!("{count} air mission presentation fields are not shown: {why}")));
    if let Err(why) = &names.regions {
        warnings.push(why.clone());
    }
    let (theatres, theatres_missing) = match theatres {
        Ok(theatres) => (Some(theatres), None),
        Err(why) => (None, Some(why)),
    };
    Ok(AirMissionsReply { read_hour, missing, missions, regions, theatres, theatres_missing, warnings })
}

fn slots_of(rows: &[CountryLinkRow]) -> BTreeMap<u32, u32> {
    rows.iter().map(|row| (row.id, row.canonical_slot)).collect()
}

fn owned_members<'a>(
    theatres: &'a [AirTheatreRow],
    groups: &[AirTheatreGroupRow],
    members: &'a [AirTheatreMemberRow],
    links: &BTreeMap<u32, u32>,
    slot: u32,
) -> Result<(Vec<&'a AirTheatreRow>, Vec<(u64, &'a AirTheatreMemberRow)>), String> {
    let mut owned = Vec::new();
    for theatre in theatres {
        let owner = canonical(theatre.owner, links).ok_or("Air-theatre owner is unresolved")?;
        if owner == slot {
            owned.push(theatre);
        }
    }
    owned.sort_by_key(|theatre| (theatre.ordinal, theatre.id));
    let ids: BTreeSet<u64> = owned.iter().map(|theatre| theatre.id).collect();
    let groups: HashMap<u64, u64> = groups.iter().map(|group| (group.id, group.theatre)).collect();
    let mut held = Vec::new();
    for member in members {
        let theatre = *groups.get(&member.group).ok_or("Air-theatre member's group is not recorded")?;
        if ids.contains(&theatre) {
            held.push((theatre, member));
        }
    }
    Ok((owned, held))
}

pub(crate) fn theatres(
    theatres: &[AirTheatreRow],
    groups: &[AirTheatreGroupRow],
    members: &[AirTheatreMemberRow],
    wings: &[AirWingRow],
    links: &BTreeMap<u32, u32>,
    slot: u32,
    name: impl Fn(&AirTheatreRow) -> String,
) -> Result<Vec<AirMissionTheatre>, String> {
    let (owned, held) = owned_members(theatres, groups, members, links, slot)?;
    let regions: HashMap<u64, Option<u32>> = wings.iter().map(|wing| (wing.id, wing.mission_region)).collect();
    let mut membership: BTreeMap<u64, BTreeSet<u32>> = owned.iter().map(|theatre| (theatre.id, BTreeSet::new())).collect();
    for (theatre, member) in held {
        let region = regions.get(&member.wing).ok_or("Air-theatre member's wing is not recorded")?;
        if let Some(region) = region {
            membership.entry(theatre).or_default().insert(*region);
        }
    }
    Ok(owned
        .into_iter()
        .map(|theatre| AirMissionTheatre {
            id: theatre.id.to_string(),
            name: name(theatre),
            regions: membership.remove(&theatre.id).unwrap_or_default().into_iter().map(|region| region.to_string()).collect(),
        })
        .collect())
}

pub(crate) const THEATRE_TABLES: [TableId; 4] = [TableId::AirTheatres, TableId::AirTheatreGroups, TableId::AirTheatreMembers, TableId::CountryLinks];

pub(crate) fn theatre_wings(held: &DecodedFrame, slot: u32) -> Option<Read> {
    let (Ok(theatres), Ok(groups), Ok(members), Ok(links)) = (
        held.complete::<tables::AirTheatres>(),
        held.complete::<tables::AirTheatreGroups>(),
        held.complete::<tables::AirTheatreMembers>(),
        held.complete::<tables::CountryLinks>(),
    ) else {
        return None;
    };
    match owned_members(theatres, groups, members, &slots_of(links), slot) {
        Ok((_, held)) if !held.is_empty() => Some(Read::these(TableId::AirWings, held.iter().map(|(_, member)| u128::from(member.wing)))),
        _ => None,
    }
}

pub(crate) fn theatres_at(frame: &DecodedFrame, slot: u32, cat: &Result<Arc<Catalogue>, String>) -> Result<Vec<AirMissionTheatre>, String> {
    let read = || -> io::Result<_> {
        Ok((
            frame.complete::<tables::AirTheatres>()?,
            frame.complete::<tables::AirTheatreGroups>()?,
            frame.complete::<tables::AirTheatreMembers>()?,
            frame.complete::<tables::CountryLinks>()?,
        ))
    };
    let (rows, groups, members, links) = read().map_err(|error| format!("Air-theatre membership is not recorded at this hour: {error}"))?;
    let links = slots_of(links);
    let held = owned_members(rows, groups, members, &links, slot)?.1;
    let wings: &[AirWingRow] = if held.is_empty() {
        &[]
    } else {
        frame.complete::<tables::AirWings>().map_err(|error| format!("Air-theatre membership is not recorded at this hour: {error}"))?
    };
    theatres(rows, groups, members, wings, &links, slot, |theatre| {
        stored_in(cat, &theatre.name, "Air theatre name").unwrap_or_else(|_| "Air theatre name unavailable".to_owned())
    })
}

impl AtHour for AirMissionsAt {
    const KEY: &'static str = "air-missions";
    const TITLE: &'static str = "Air missions";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirMissionsParams;
    type Reply = AirMissionsReply;
    type Context = Names;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirMissionsParams::decl(), AirMissionsReply::decl(), AirMissionChoice::decl(), AirMissionRegion::decl(), AirMissionTheatre::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![
            TableId::AirMissionCounts,
            TableId::AirRegionalMissions,
            TableId::AirTheatres,
            TableId::AirTheatreGroups,
            TableId::AirTheatreMembers,
            TableId::CountryLinks,
            TableId::AirWings,
        ]
    }

    fn reads(_params: &Self::Params, owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => {
                let low = u128::from(owner.slot) << 32;
                let mut reads = vec![
                    Read { table: TableId::AirMissionCounts, keys: Keys::Range(low, low | u128::from(u32::MAX)) },
                    Read { table: TableId::AirRegionalMissions, keys: Keys::These(Vec::new()) },
                ];
                reads.extend(THEATRE_TABLES.into_iter().map(Read::all));
                reads
            }
            1 => theatre_wings(held, owner.slot).into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(names(source, load))
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Air missions country is unavailable at this hour"))?;
        let read_hour = frame.tables[TableId::AirRegionalMissions.index()].sampled_at();
        let theatres = theatres_at(frame, owner.slot, &at.context.catalogue);
        reply(frame.complete::<tables::AirMissionCounts>(), owner.slot, read_hour, theatres, at.context, at.live)
    }
}

#[cfg(test)]
pub(crate) fn region_wings<'a>(registrations: &[AirRegionalMissionRow], wings: &'a [AirWingRow], links: &BTreeMap<u32, u32>, slot: u32, region: u32) -> Result<Vec<&'a AirWingRow>, String> {
    let (_, rosters) = replay_derive::air_mission_counts::regional_counts(registrations, wings, slot, |raw| canonical(raw, links))?;
    let mut seen = BTreeSet::new();
    Ok(rosters.get(&region).into_iter().flatten().copied().filter(|wing| seen.insert(wing.id)).collect())
}

const SORTIE_TABLES: [TableId; 8] = [
    TableId::AirSortieModifiers,
    TableId::AirSortieBaseCountries,
    TableId::AirSortieSpecialBases,
    TableId::AirSortieModifierMetadata,
    TableId::SimulationFeatures,
    TableId::AirReferenceWitnesses,
    TableId::AirBases,
    TableId::VolunteerHosts,
];

fn region_range(slot: u32, region: u32) -> (u128, u128) {
    let low = u128::from(slot) << 96 | u128::from(region) << 64;
    (low, low | u128::from(u64::MAX))
}

pub(crate) fn wings_in(frame: &DecodedFrame, slot: u32, region: u32) -> Result<Vec<&AirWingRow>, String> {
    let registered = frame.complete::<tables::AirMissionWings>().map_err(|error| error.to_string())?;
    let wings = frame.complete::<tables::AirWings>().map_err(|error| error.to_string())?;
    let mut ordered: Vec<(u32, u64)> = registered.iter().filter(|row| row.slot == slot && row.region == region).map(|row| (row.ordinal, row.wing)).collect();
    ordered.sort_unstable();
    ordered
        .into_iter()
        .map(|(_, id)| wings.binary_search_by_key(&id, |wing| wing.id).map(|at| &wings[at]).map_err(|_| format!("registered wing {id} has no air wings row")))
        .collect()
}

impl AtHour for AirMissionWingsAt {
    const KEY: &'static str = "air-mission-wings";
    const TITLE: &'static str = "Air mission wings";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirMissionWingsParams;
    type Reply = AirMissionWingsReply;
    type Context = Names;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirMissionWingsParams::decl(), AirMissionWingsReply::decl(), AirMissionWing::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![TableId::AirMissionWings, TableId::AirRegionalMissions, TableId::AirWings, TableId::CountryLinks, TableId::AirMissionInputs, TableId::Provinces];
        tables.extend(SORTIE_TABLES);
        tables
    }

    fn reads(params: &Self::Params, owner: Option<dashboard_base::products::at_hour::Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => {
                let (low, high) = region_range(owner.slot, params.region);
                let mut reads = vec![
                    Read { table: TableId::AirMissionWings, keys: Keys::Range(low, high) },
                    Read { table: TableId::AirRegionalMissions, keys: Keys::These(Vec::new()) },
                    Read::all(TableId::CountryLinks),
                ];
                reads.extend(SORTIE_TABLES.into_iter().map(Read::all));
                reads
            }
            1 => {
                let Ok(registered) = held.complete::<tables::AirMissionWings>() else { return Vec::new() };
                let wings: Vec<u128> = registered.iter().map(|row| u128::from(row.wing)).collect();
                let provinces: Vec<u128> = match AirRegistrySources::read(held) {
                    Ok(Some(registry)) => registry.bases.values().filter_map(|base| base.province).map(u128::from).collect(),
                    _ => Vec::new(),
                };
                vec![Read::these(TableId::AirWings, wings.iter().copied()), Read::these(TableId::AirMissionInputs, wings), Read::these(TableId::Provinces, provinces)]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(names(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Air missions country is unavailable at this hour"))?;
        let read_hour = frame.tables[TableId::AirRegionalMissions.index()].sampled_at();
        let mut reply = AirMissionWingsReply { read_hour, region: params.region, wings: Vec::new(), missing: None, warnings: Vec::new() };
        let selected = match wings_in(frame, owner.slot, params.region) {
            Ok(wings) => wings,
            Err(why) => {
                reply.missing = Some(format!("This region's wings are not recorded at this hour: {why}"));
                return Ok(reply);
            }
        };
        wings_reply(frame, &selected, at.context, at.live, &mut reply)?;
        Ok(reply)
    }
}

fn wings_reply(frame: &DecodedFrame, selected: &[&AirWingRow], names: &Names, live: &Cells, reply: &mut AirMissionWingsReply) -> io::Result<()> {
    let planes = column(TableId::AirWings, "planes")?;
    let links = frame.complete::<tables::CountryLinks>().map(slots_of).map_err(|error| error.to_string());
    let modifiers: Result<BTreeMap<(u8, u32, u32), i64>, String> =
        frame.complete::<tables::AirSortieModifiers>().map(|rows| rows.iter().map(|row| ((row.scope, row.subject, row.region), row.raw)).collect()).map_err(|error| error.to_string());
    let sources = SortieSources::read(frame).map_err(|error| error.to_string()).and_then(|sources| sources.map_err(str::to_owned));
    let registry = AirRegistrySources::read(frame).map_err(|error| error.to_string()).and_then(|registry| registry.ok_or_else(|| "air base registry is incomplete".to_owned()));
    let countries = match (&registry, &links) {
        (Ok(registry), Ok(links)) => MissionCountrySources::read(frame, links, registry).map_err(|error| error.to_string()),
        (Err(why), _) | (_, Err(why)) => Err(why.clone()),
    };
    let units = names.catalogue.as_ref().map_err(Clone::clone).and_then(|cat| {
        cat.content.layers.derived::<CachedResult<content_reader_script::sub_units::SubUnits>>().get_or_init(|| content_reader_key_names::unit_leader::sub_units(&cat.content.layers)).map_err(|error| error.to_string())
    });
    let fuel_penalty = names.catalogue.as_ref().map_err(Clone::clone).and_then(|cat| {
        let defines = cat.content.layers.derived::<CachedResult<content_reader_defines::defines::Defines>>().get_or_init(|| content_reader_defines::defines::read(&cat.content.layers)).map_err(|error| error.to_string())?;
        defines.fixed5("NAir", "MISSION_EFFICIENCY_MULT_AT_LACK_OF_FUEL").map_err(|error| error.to_string())
    });
    let mut notes = BTreeMap::<String, usize>::new();
    for wing in selected {
        let mut refusals = Vec::new();
        let name = wing
            .name
            .require("name of air wings")
            .and_then(|name| name.as_ref().ok_or_else(|| "Air wing name was not recorded".to_owned()))
            .and_then(|bytes| stored(names, bytes, "Air wing name"));
        let aircraft_type = wing
            .pool_subunit
            .require("pool subunit of air wings")
            .and_then(|index| (*index).ok_or_else(|| "Air wing pool type was not recorded".to_owned()))
            .and_then(|index| {
                let units = units.as_ref().map_err(Clone::clone)?;
                units.name(index).ok_or_else(|| format!("Air wing pool type {index} is absent from the recorded content"))?.map(str::to_owned)
            })
            .and_then(|key| {
                let cat = names.catalogue.as_ref().map_err(Clone::clone)?;
                if !key.is_empty() && cat.localisation.get(&key).is_none() {
                    return Err(format!("Localisation {key} is absent from the recorded content stack"));
                }
                dashboard_text::catalogue::localised(&cat.localisation, &key, View::Plain)
            });
        let efficiency = (|| -> Result<String, String> {
            let raw = effective_raw(wing);
            let slot = canonical(raw, links.as_ref().map_err(Clone::clone)?).ok_or("Sortie effective country is unresolved")?;
            if let Some(value) = efficiency_override(modifiers.as_ref().map_err(Clone::clone)?, (raw, slot)) {
                return Ok(value.to_string());
            }
            sources
                .as_ref()
                .map_err(Clone::clone)?
                .wing_efficiency(wing, registry.as_ref().map_err(Clone::clone)?, countries.as_ref().map_err(Clone::clone)?, *fuel_penalty.as_ref().map_err(Clone::clone)?)
                .map(|value| value.to_string())
                .map_err(str::to_owned)
        })();
        let mut field = |result: Result<String, String>| {
            result
                .map_err(|why| {
                    *notes.entry(why.clone()).or_default() += 1;
                    refusals.push(why);
                })
                .ok()
        };
        let (name, aircraft_type, efficiency) = (field(name), field(aircraft_type), field(efficiency));
        let id = wing.id.to_string();
        live.value(&id, "planes", Cell { table: TableId::AirWings, column: planes, key: u128::from(wing.id) }, 1.0);
        reply.wings.push(AirMissionWing {
            id,
            name,
            aircraft_type,
            missions: (0..u32::BITS).filter(|bit| wing.mission_mask & (1 << bit) != 0).map(|bit| bit.to_string()).collect(),
            efficiency,
            refusals,
        });
    }
    reply.warnings.extend(names.warnings.iter().cloned());
    reply.warnings.extend(notes.into_iter().map(|(why, count)| format!("{count} air wing fields are not shown: {why}")));
    Ok(())
}
