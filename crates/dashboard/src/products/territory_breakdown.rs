//! Exact territory weights grouped by current holder and content strategic region.
//! Region membership is content data, never a vanilla list or a guessed centroid.
//! Each state's exact contribution is split by its native content province roster.
use super::country_groups::{HourSpan, record_hour};
use super::{
    LoadContext, Product,
    territory::{self, Territory, TerritoryParams, TerritoryRun, TerritorySeries},
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{
        TaggedCountry, check_cancel, link_slots, slot_read, tagged_countries_indexed,
        validate_campaign,
    },
};
use installed_content::province_area::ProvinceAreas;
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, SharedView},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct TerritoryBreakdown;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerritoryBreakdownParams {
    pub branch_id: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryRegion {
    pub id: u32,
    pub name: Option<String>,
    pub missing: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryStateShare {
    pub numerator: String,
    pub denominator: u32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryRegionRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    /// Provinces, victory points, area: controlled first, then owned.
    pub values: [Option<String>; 6],
    /// Controlled then owned. Terms with the same denominator are combined.
    pub state_shares: [Option<Vec<TerritoryStateShare>>; 2],
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryRegionSeries {
    pub tag: Option<String>,
    pub region: Option<u32>,
    pub missing: Option<String>,
    pub runs: Vec<TerritoryRegionRun>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryBreakdownReply {
    pub branch_id: String,
    pub regions: Vec<TerritoryRegion>,
    pub series: Vec<TerritoryRegionSeries>,
    pub countries: Vec<TerritorySeries>,
    pub world_totals: Vec<TerritoryRun>,
    /// Complete region inputs, in TerritoryRun's measure/rule order.
    #[ts(type = "Array<Array<HourSpan>>")]
    pub observed: [Vec<HourSpan>; 8],
    pub area_unit: Option<String>,
    pub area_reason: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct Regions {
    provinces: HashMap<u32, Option<u32>>,
    states: HashMap<u32, Vec<StatePart>>,
    names: Vec<TerritoryRegion>,
    warnings: Vec<String>,
    state_reason: Option<String>,
}
#[derive(Clone, Debug)]
struct StatePart {
    region: Option<u32>,
    numerator: u64,
    denominator: u32,
}

fn state_parts(
    provinces: &[u32],
    regions: &HashMap<u32, Option<u32>>,
) -> io::Result<Vec<StatePart>> {
    let denominator = u32::try_from(provinces.len())
        .map_err(|_| io::Error::other("State province roster exceeds native 32-bit count"))?;
    if denominator == 0 {
        return Ok(vec![StatePart {
            region: None,
            numerator: 1,
            denominator: 1,
        }]);
    }
    let mut counts = BTreeMap::<Option<u32>, u64>::new();
    for province in provinces {
        *counts
            .entry(regions.get(province).copied().flatten())
            .or_default() += 1;
    }
    Ok(counts
        .into_iter()
        .map(|(region, numerator)| StatePart {
            region,
            numerator,
            denominator,
        })
        .collect())
}
impl Regions {
    fn read(cat: &Catalogue) -> io::Result<Self> {
        let content = installed_content::regions::read(&cat.content.layers)?;
        let mut result = Self {
            warnings: content.notes,
            ..Default::default()
        };
        for region in content.regions.into_values() {
            let name = cat
                .localisation
                .localise(region.name_key.as_bytes())
                .map_err(|error| error.to_string())
                .and_then(|text| catalogue::plain_text(&text));
            let (name, missing) = match name {
                Ok(name) if !name.is_empty() => (Some(name), None),
                Ok(_) => (None, Some("Strategic region name is empty".into())),
                Err(reason) => (None, Some(reason)),
            };
            result.names.push(TerritoryRegion {
                id: region.id,
                name,
                missing,
            });
            for province in region.provinces {
                result
                    .provinces
                    .entry(province)
                    .and_modify(|old| {
                        if *old != Some(region.id) {
                            *old = None;
                        }
                    })
                    .or_insert(Some(region.id));
            }
        }
        let ambiguous = result
            .provinces
            .values()
            .filter(|region| region.is_none())
            .count();
        if ambiguous != 0 {
            result.warnings.push(format!("{ambiguous} provinces belong to conflicting strategic regions; their region is not attributable"));
        }
        let unnamed = result
            .names
            .iter()
            .filter(|region| region.name.is_none())
            .count();
        if unnamed != 0 {
            result.warnings.push(format!("{unnamed} strategic region names unavailable; region identities and named reasons are retained"));
        }
        match cat.cached_states() {
            Ok(content) => {
                result.warnings.extend(
                    crate::scripted_text::live_state_warnings(&content.warnings).map(str::to_owned),
                );
                // CStateTemplate::ReadToken@140AAF580 appends each valid province,
                // repeats included; 140AAD450 keeps the first template for an id.
                for state in content.states.definitions().values() {
                    result
                        .states
                        .insert(state.id, state_parts(&state.provinces, &result.provinces)?);
                }
                let missing = content
                    .states
                    .definitions()
                    .values()
                    .flat_map(|state| &state.provinces)
                    .filter(|province| !result.provinces.contains_key(province))
                    .count();
                let empty = content
                    .states
                    .definitions()
                    .values()
                    .filter(|state| state.provinces.is_empty())
                    .count();
                if missing != 0 {
                    result.warnings.push(format!("{missing} state province memberships have no strategic region definition; their region is not attributable"));
                }
                if empty != 0 {
                    result.warnings.push(format!("{empty} states have empty native province rosters; their region is not attributable"));
                }
            }
            Err(reason) => {
                result.state_reason =
                    Some(format!("State province membership unavailable: {reason}"))
            }
        }
        Ok(result)
    }
}

type Key = (Option<String>, Option<u32>, Option<String>);
#[derive(Default)]
struct Scan {
    series: BTreeMap<Key, TerritoryRegionSeries>,
    identities: HashMap<String, ((u32, u32), i32)>,
    tags: BTreeSet<String>,
    parts: Option<Parts>,
    observed: [Vec<HourSpan>; 8],
    missing_states: BTreeSet<u32>,
    missing_provinces: BTreeSet<u32>,
}
struct Parts {
    views: (SharedView, SharedView),
    holders: HashMap<u32, Option<String>>,
    available: [bool; 8],
    values: BTreeMap<Key, RegionValues>,
}

#[derive(Clone)]
struct RegionValues {
    values: [Option<i64>; 6],
    states: [Option<BTreeMap<u32, u64>>; 2],
}
impl RegionValues {
    fn zero(available: [bool; 8]) -> Self {
        Self {
            values: [1, 2, 3, 5, 6, 7].map(|column| available[column].then_some(0)),
            states: [0, 4].map(|column| available[column].then(BTreeMap::new)),
        }
    }
    fn province(&mut self, column: usize, values: &[Option<i64>]) -> io::Result<()> {
        for (offset, value) in values.iter().enumerate() {
            self.values[column + offset] = match (self.values[column + offset], value) {
                (Some(total), Some(value)) => Some(total.checked_add(*value).ok_or_else(|| {
                    io::Error::other("Territory region sum exceeds signed 64-bit range")
                })?),
                _ => None,
            };
        }
        Ok(())
    }
    fn state(&mut self, column: usize, part: &StatePart) -> io::Result<()> {
        if let Some(terms) = &mut self.states[column] {
            let total = terms.entry(part.denominator).or_default();
            *total = total.checked_add(part.numerator).ok_or_else(|| {
                io::Error::other("Territory state numerator exceeds unsigned 64-bit range")
            })?;
        }
        Ok(())
    }
}

fn holder_names(frame: &DecodedFrame) -> Option<HashMap<u32, Option<String>>> {
    let countries = frame.complete::<tables::Countries>().ok()?;
    let links = frame.complete::<tables::CountryLinks>().ok()?;
    let slots = link_slots(links);
    let mut names: HashMap<u32, BTreeSet<String>> = HashMap::new();
    for (tag, found) in tagged_countries_indexed(countries, &slots) {
        let TaggedCountry::Found(_, slot) = found else {
            continue;
        };
        if let Ok(tag) = std::str::from_utf8(tag)
            && !tag.is_empty()
        {
            names.entry(slot).or_default().insert(tag.to_owned());
        }
    }
    Some(
        slots
            .into_iter()
            .map(|(raw, slot)| {
                let tag = names
                    .get(&slot)
                    .filter(|tags| tags.len() == 1)
                    .and_then(|tags| tags.first())
                    .cloned();
                (raw, tag)
            })
            .collect(),
    )
}

fn observe(
    scan: &mut Scan,
    frame: &DecodedFrame,
    regions: &Regions,
    areas: Option<&ProvinceAreas>,
) -> io::Result<()> {
    let Some(holders) = holder_names(frame) else {
        return Ok(());
    };
    let slots = link_slots(
        frame
            .complete::<tables::CountryLinks>()
            .expect("holder inputs"),
    );
    let identities: HashMap<_, _> = tagged_countries_indexed(
        frame
            .complete::<tables::Countries>()
            .expect("holder inputs"),
        &slots,
    )
    .into_iter()
    .filter_map(|(tag, found)| {
        let TaggedCountry::Found(country, slot) = found else {
            return None;
        };
        Some((
            std::str::from_utf8(tag).ok()?.to_owned(),
            ((slot, country.id), frame.hour),
        ))
    })
    .collect();
    scan.tags.extend(
        frame
            .complete::<tables::Countries>()
            .expect("holder inputs")
            .iter()
            .filter_map(|country| std::str::from_utf8(&country.tag).ok())
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned),
    );
    let states = slot_read(&frame.metrics, TableId::States, frame.hour)
        .then(|| frame.complete::<tables::States>().ok())
        .flatten();
    let provinces = slot_read(&frame.metrics, TableId::Provinces, frame.hour)
        .then(|| frame.complete::<tables::Provinces>().ok())
        .flatten();
    let available = [
        states.is_some(),
        provinces.is_some(),
        provinces.is_some(),
        provinces.is_some() && areas.is_some(),
        states.is_some(),
        provinces.is_some(),
        provinces.is_some(),
        provinces.is_some() && areas.is_some(),
    ];
    for (column, known) in available.iter().enumerate() {
        if *known {
            record_hour(&mut scan.observed[column], frame.hour);
        }
    }
    let views = (
        frame.tables[TableId::States.index()].clone(),
        frame.tables[TableId::Provinces.index()].clone(),
    );
    let same = scan.parts.as_ref().is_some_and(|old| {
        old.available == available
            && old.holders == holders
            && SharedView::ptr_eq(&old.views.0, &views.0)
            && SharedView::ptr_eq(&old.views.1, &views.1)
    });
    // Reusing identical decoded tables avoids a province walk at every unchanged hour.
    // Freshness and holder attribution are part of the key, so gaps cannot reuse values.
    let mut parts = if same {
        scan.parts
            .as_ref()
            .expect("matching geographic inputs")
            .values
            .clone()
    } else {
        let mut parts: BTreeMap<Key, RegionValues> = BTreeMap::new();
        let key = |raw: u32, region: Option<u32>, state_reason: Option<&str>| {
            let tag = if raw == 0 {
                None
            } else {
                holders.get(&raw).cloned().flatten()
            };
            let reasons: Vec<_> = [
                (raw != 0 && tag.is_none()).then_some("Country holder not recorded or ambiguous"),
                region
                    .is_none()
                    .then_some(state_reason.unwrap_or("Region membership missing or conflicting")),
            ]
            .into_iter()
            .flatten()
            .collect();
            (
                tag,
                region,
                (!reasons.is_empty()).then(|| reasons.join("; ")),
            )
        };
        for state in states.into_iter().flatten() {
            let fallback = [StatePart {
                region: None,
                numerator: 1,
                denominator: 1,
            }];
            let shares = regions
                .states
                .get(&state.id)
                .map(Vec::as_slice)
                .unwrap_or(&fallback);
            if !regions.states.contains_key(&state.id) {
                scan.missing_states.insert(state.id);
            }
            let reason = regions.state_reason.as_deref().unwrap_or("State province roster absent or empty, or region membership missing or conflicting");
            for part in shares {
                for (raw, column) in [(state.controller, 0), (state.owner, 1)] {
                    parts
                        .entry(key(raw, part.region, Some(reason)))
                        .or_insert_with(|| RegionValues::zero(available))
                        .state(column, part)?;
                }
            }
        }
        for province in provinces.into_iter().flatten() {
            let region = regions.provinces.get(&province.id).copied().flatten();
            if region.is_none() {
                scan.missing_provinces.insert(province.id);
            }
            let values = territory::province_values(province, areas);
            for (raw, column) in [(province.controller, 0), (province.owner, 3)] {
                parts
                    .entry(key(raw, region, None))
                    .or_insert_with(|| RegionValues::zero(available))
                    .province(column, &values)?;
            }
        }
        scan.parts = Some(Parts {
            views,
            holders: holders.clone(),
            available,
            values: parts.clone(),
        });
        parts
    };
    // A holder/region with no rows has a proven zero only while its country is present.
    let present: BTreeSet<String> = holders.values().flatten().cloned().collect();
    for key in scan.series.keys() {
        if key.0.as_ref().is_none_or(|tag| present.contains(tag)) {
            parts
                .entry(key.clone())
                .or_insert_with(|| RegionValues::zero(available));
        }
    }
    for (key, values) in parts {
        let series = scan
            .series
            .entry(key.clone())
            .or_insert_with(|| TerritoryRegionSeries {
                tag: key.0,
                region: key.1,
                missing: key.2,
                runs: Vec::new(),
            });
        let state_shares = values.states.map(|terms| {
            terms.map(|terms| {
                terms
                    .into_iter()
                    .map(|(denominator, numerator)| TerritoryStateShare {
                        numerator: numerator.to_string(),
                        denominator,
                    })
                    .collect()
            })
        });
        let values = values
            .values
            .map(|value| value.map(|value| value.to_string()));
        let identity_continues = !series
            .missing
            .as_ref()
            .is_some_and(|reason| reason.starts_with("Country holder"))
            && series.tag.as_ref().is_none_or(|tag| {
                scan.identities
                    .get(tag)
                    .zip(identities.get(tag))
                    .is_some_and(|(before, now)| {
                        before.0 == now.0 && before.1.checked_add(1) == Some(frame.hour)
                    })
            });
        if let Some(previous) = series.runs.last_mut()
            && identity_continues
            && previous.last_hour.checked_add(1) == Some(frame.hour)
            && previous.values == values
            && previous.state_shares == state_shares
        {
            previous.last_hour = frame.hour;
        } else {
            let starts_segment = !identity_continues
                || series
                    .runs
                    .last()
                    .is_none_or(|previous| previous.last_hour.checked_add(1) != Some(frame.hour));
            series.runs.push(TerritoryRegionRun {
                first_hour: frame.hour,
                last_hour: frame.hour,
                starts_segment,
                values,
                state_shares,
            });
        }
    }
    scan.identities = identities;
    Ok(())
}

fn scan(
    replay: &Replay,
    branch: u64,
    regions: &Regions,
    areas: Option<&ProvinceAreas>,
    cancelled: &AtomicBool,
) -> io::Result<Scan> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::Provinces,
    ];
    let mut result = Scan::default();
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        check_cancel(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for frame in frames {
            check_cancel(cancelled)?;
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            observe(&mut result, decoded, regions, areas)?;
        }
    }
    Ok(result)
}

type CachedScan = (u64, Arc<Catalogue>, bool, Arc<Scan>);
impl Product for TerritoryBreakdown {
    const KEY: &'static str = "territory-breakdown";
    const CONTENT_DEPENDENT: bool = true;
    type Params = TerritoryBreakdownParams;
    type Reply = TerritoryBreakdownReply;
    fn types() -> Vec<String> {
        vec![
            TerritoryBreakdownParams::decl(),
            TerritoryRegion::decl(),
            TerritoryStateShare::decl(),
            TerritoryRegionRun::decl(),
            TerritoryRegionSeries::decl(),
            TerritoryBreakdownReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid branch")
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let cat = catalogue::catalogue(source, context.content()?)?;
        let regions = Regions::read(&cat)?;
        let areas = territory::area_data(source, context, cancelled)?.areas;
        let branch = params.branch_id.parse().expect("validated branch");
        let state = source.state::<Vec<CachedScan>>();
        let cached = state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .find(|(id, old, has_areas, _)| {
                *id == branch && Arc::ptr_eq(old, &cat) && *has_areas == areas.is_some()
            })
            .map(|(_, _, _, scan)| Arc::clone(scan));
        let scanned = if let Some(cached) = cached {
            cached
        } else {
            let built = Arc::new(scan(
                &source.replay(),
                branch,
                &regions,
                areas.as_deref(),
                cancelled,
            )?);
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache.push((
                branch,
                Arc::clone(&cat),
                areas.is_some(),
                Arc::clone(&built),
            ));
            if cache.len() > 2 {
                cache.remove(0);
            }
            built
        };
        let totals = Territory::load(
            source,
            &TerritoryParams {
                branch_id: params.branch_id.clone(),
                tags: scanned.tags.iter().cloned().collect(),
                top: None,
            },
            context,
            cancelled,
        )?;
        let mut warnings = regions.warnings;
        if let Some(reason) = regions.state_reason {
            warnings.push(reason);
        }
        if !scanned.missing_states.is_empty() {
            warnings.push(format!("{} recorded states have no resolved native province roster; their state contributions remain in Region not attributable", scanned.missing_states.len()));
        }
        if !scanned.missing_provinces.is_empty() {
            warnings.push(format!("{} recorded provinces have missing or conflicting strategic region membership; their contributions remain in Region not attributable", scanned.missing_provinces.len()));
        }
        let reply = TerritoryBreakdownReply {
            branch_id: params.branch_id.clone(),
            regions: regions.names,
            series: scanned.series.values().cloned().collect(),
            countries: totals.series,
            world_totals: totals.world_totals,
            observed: scanned.observed.clone(),
            area_unit: totals.area_unit,
            area_reason: totals.area_reason,
            warnings: warnings.into_iter().chain(totals.warnings).collect(),
        };
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
#[cfg(test)]
mod tests;
