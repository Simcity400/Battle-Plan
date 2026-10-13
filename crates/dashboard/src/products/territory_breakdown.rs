//! Exact territory weights grouped by current holder and content strategic region.
//! Region membership is content data, never a vanilla list or a guessed centroid.
//! Each state's exact contribution is split by its native content province roster.
use super::country_groups::{HourSpan, record_hour};
use super::{
    LoadContext, Product,
    territory::{
        self, CountryReads, LandReads, Territory, TerritoryParams, TerritoryRun, TerritorySeries,
    },
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{TaggedCountry, link_slots, tagged_countries_indexed},
};
use installed_content::province_area::ProvinceAreas;
use replay::{
    frame::DecodedFrame,
    rows::{CountryLinkRow, CountryRow, tables},
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
            let name = if !cat.has_presentation_key(&region.name_key) {
                Err(format!(
                    "localisation {} is missing; region title is unavailable",
                    region.name_key
                ))
            } else {
                cat.localisation
                    .localise(region.name_key.as_bytes())
                    .map_err(|error| error.to_string())
                    .and_then(|text| catalogue::plain_text(&text))
            };
            let (name, missing) = match name {
                // Successful empty localisation is stored too; the template's name text
                // is returned unchanged.
                Ok(name) => (Some(name), None),
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
                // Each valid province is appended, repeats included;
                // the first template for an id is kept.
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
    /// The recording's format, which dates its daily tables
    /// ([`crate::metadata::slot_read`]).
    format: replay::format::FormatVersion,
    /// One series per key, in key order once the scan ends.
    series: Vec<TerritoryRegionSeries>,
    tags: BTreeSet<String>,
    observed: [Vec<HourSpan>; 8],
    missing_states: BTreeSet<u32>,
    missing_provinces: BTreeSet<u32>,
}

/// One scan's working state. Holders, series keys and region sums are resolved
/// once and reused while the recorded fields they read are unchanged.
#[derive(Default)]
struct Builder {
    lineage: Option<Lineage>,
    /// The last hour with complete country tables.
    hour: Option<i32>,
    /// Interned holder tags.
    tags: Vec<String>,
    tag_ids: HashMap<String, usize>,
    keys: HashMap<Key, usize>,
    /// Each series' holder and last values, by its index.
    slots: Vec<Slot>,
    /// Series by raw holder, region and whether for a state, under the current holders.
    memo: HashMap<(u32, Option<u32>, bool), usize>,
    /// Each province row's id and raw holders, with their series under the current holders.
    provinces: Vec<([u32; 3], [usize; 2])>,
    reads: LandReads,
    /// The freshness the sums were taken with, and the sums by series.
    available: Option<[bool; 8]>,
    values: Vec<Option<RegionValues>>,
}
struct Lineage {
    reads: CountryReads,
    /// Each raw link's tag, where exactly one tag names its canonical slot.
    holders: HashMap<u32, usize>,
    /// By interned tag: the native identity of the country holding it, and
    /// whether it names a link's holder.
    identities: Vec<Option<(u32, u32)>>,
    present: Vec<bool>,
}
struct Slot {
    tag: Option<usize>,
    unknown_holder: bool,
    /// The values of the series' last run.
    last: Option<RegionValues>,
}

#[derive(Clone, PartialEq)]
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
    /// One hour's run. Equal values give equal text, so runs compare as numbers.
    fn run(&self, hour: i32, starts_segment: bool) -> TerritoryRegionRun {
        TerritoryRegionRun {
            first_hour: hour,
            last_hour: hour,
            starts_segment,
            values: self.values.map(|value| value.map(|sum| sum.to_string())),
            state_shares: self.states.each_ref().map(|terms| {
                terms.as_ref().map(|terms| {
                    terms
                        .iter()
                        .map(|(&denominator, numerator)| TerritoryStateShare {
                            numerator: numerator.to_string(),
                            denominator,
                        })
                        .collect()
                })
            }),
        }
    }
}

impl Builder {
    /// Each raw link's holder tag and each tag's native identity this hour.
    fn read_lineage(
        &mut self,
        frame: &DecodedFrame,
        countries: &[CountryRow],
        links: &[CountryLinkRow],
    ) -> Lineage {
        let slots = link_slots(links);
        let mut found = Vec::new();
        let mut names: HashMap<u32, BTreeSet<usize>> = HashMap::new();
        for (tag, lookup) in tagged_countries_indexed(countries, &slots) {
            let TaggedCountry::Found(country, slot) = lookup else {
                continue;
            };
            let tag = match std::str::from_utf8(tag) {
                Ok(tag) if !tag.is_empty() => tag,
                // Holders are named only by non-empty UTF-8 tags, so no other is looked up.
                _ => continue,
            };
            let next = self.tags.len();
            let id = *self.tag_ids.entry(tag.to_owned()).or_insert(next);
            if id == next {
                self.tags.push(tag.to_owned());
            }
            found.push((id, (slot, country.id)));
            names.entry(slot).or_default().insert(id);
        }
        let holders: HashMap<_, _> = slots
            .into_iter()
            .filter_map(|(raw, slot)| {
                let tags = names.get(&slot).filter(|tags| tags.len() == 1)?;
                Some((raw, *tags.first()?))
            })
            .collect();
        let mut identities = vec![None; self.tags.len()];
        for (id, identity) in found {
            identities[id] = Some(identity);
        }
        let mut present = vec![false; self.tags.len()];
        for &id in holders.values() {
            present[id] = true;
        }
        Lineage {
            reads: CountryReads::new(frame, countries),
            holders,
            identities,
            present,
        }
    }

    /// The series of a raw holder and region, resolved once under the current holders.
    fn slot(
        &mut self,
        series: &mut Vec<TerritoryRegionSeries>,
        raw: u32,
        region: Option<u32>,
        state_reason: Option<&str>,
    ) -> usize {
        let memo = (raw, region, state_reason.is_some());
        if let Some(&slot) = self.memo.get(&memo) {
            return slot;
        }
        let holders = &self.lineage.as_ref().expect("complete lineage").holders;
        let tag = if raw == 0 {
            None
        } else {
            holders.get(&raw).copied()
        };
        let unknown_holder = raw != 0 && tag.is_none();
        let reasons: Vec<_> = [
            unknown_holder.then_some("Country holder not recorded or ambiguous"),
            region
                .is_none()
                .then_some(state_reason.unwrap_or("Region membership missing or conflicting")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let key = (
            tag.map(|tag| self.tags[tag].clone()),
            region,
            (!reasons.is_empty()).then(|| reasons.join("; ")),
        );
        let slot = match self.keys.get(&key) {
            Some(&slot) => slot,
            None => {
                series.push(TerritoryRegionSeries {
                    tag: key.0.clone(),
                    region,
                    missing: key.2.clone(),
                    runs: Vec::new(),
                });
                self.slots.push(Slot {
                    tag,
                    unknown_holder,
                    last: None,
                });
                self.values.push(None);
                self.keys.insert(key, self.slots.len() - 1);
                self.slots.len() - 1
            }
        };
        self.memo.insert(memo, slot);
        slot
    }

    /// This hour's sums for a series, from zero in each available measure.
    fn sums(&mut self, slot: usize) -> &mut RegionValues {
        let available = self.available.expect("current sums");
        self.values[slot].get_or_insert_with(|| RegionValues::zero(available))
    }
}

fn observe(
    scan: &mut Scan,
    frame: &DecodedFrame,
    regions: &Regions,
    areas: Option<&ProvinceAreas>,
    builder: &mut Builder,
) -> io::Result<()> {
    let (Ok(countries), Ok(links)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) else {
        return Ok(());
    };
    let same = builder
        .lineage
        .as_ref()
        .is_some_and(|lineage| lineage.reads.matches(frame, countries));
    let replaced = if same {
        None
    } else {
        let lineage = builder.read_lineage(frame, countries, links);
        scan.tags.extend(
            countries
                .iter()
                .filter_map(|country| std::str::from_utf8(&country.tag).ok())
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned),
        );
        // Series resolved through other holders, and their sums, are stale.
        if builder
            .lineage
            .as_ref()
            .is_none_or(|old| old.holders != lineage.holders)
        {
            builder.memo.clear();
            builder.provinces.clear();
            builder.available = None;
        }
        Some(builder.lineage.replace(lineage))
    };
    let (states, provinces) = territory::fresh_tables(scan.format, frame);
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
    // Unchanged holders and read fields keep their sums without a province walk.
    // Freshness is part of the comparison, so gaps cannot reuse values.
    let same_reads = builder.reads.update(states, provinces);
    if !same_reads || builder.available != Some(available) {
        builder.available = Some(available);
        builder.values.clear();
        builder.values.resize_with(builder.slots.len(), || None);
        let reason = regions.state_reason.as_deref().unwrap_or(
            "State province roster absent or empty, or region membership missing or conflicting",
        );
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
            for part in shares {
                for (raw, column) in [(state.controller, 0), (state.owner, 1)] {
                    let slot = builder.slot(&mut scan.series, raw, part.region, Some(reason));
                    builder.sums(slot).state(column, part)?;
                }
            }
        }
        for (index, province) in provinces.into_iter().flatten().enumerate() {
            let read = [province.id, province.controller, province.owner];
            let slots = match builder.provinces.get(index) {
                Some(&(old, slots)) if old == read => slots,
                _ => {
                    let region = regions.provinces.get(&province.id).copied().flatten();
                    if region.is_none() {
                        scan.missing_provinces.insert(province.id);
                    }
                    let slots = [province.controller, province.owner]
                        .map(|raw| builder.slot(&mut scan.series, raw, region, None));
                    if let Some(row) = builder.provinces.get_mut(index) {
                        *row = (read, slots);
                    } else {
                        builder.provinces.push((read, slots));
                    }
                    slots
                }
            };
            let values = territory::province_values(province, areas);
            for (slot, column) in slots.into_iter().zip([0, 3]) {
                builder.sums(slot).province(column, &values)?;
            }
        }
    }
    let lineage = builder.lineage.as_ref().expect("complete lineage");
    // Identities at the last hour with complete country tables, which a reused
    // lineage shares.
    let before = match &replaced {
        None => lineage.identities.as_slice(),
        Some(Some(old)) => old.identities.as_slice(),
        Some(None) => &[],
    };
    let contiguous = builder.hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
    let zero = RegionValues::zero(available);
    for ((slot, values), series) in builder
        .slots
        .iter_mut()
        .zip(&builder.values)
        .zip(&mut scan.series)
    {
        let values = match values {
            Some(values) => values,
            // A holder/region with no rows has a proven zero only while its country is present.
            None if slot.tag.is_none_or(|tag| lineage.present[tag]) => &zero,
            None => continue,
        };
        let identity_continues = !slot.unknown_holder
            && slot.tag.is_none_or(|tag| {
                let now = lineage.identities[tag];
                contiguous && now.is_some() && before.get(tag) == Some(&now)
            });
        if let Some(previous) = series.runs.last_mut()
            && identity_continues
            && previous.last_hour.checked_add(1) == Some(frame.hour)
            && slot.last.as_ref() == Some(values)
        {
            previous.last_hour = frame.hour;
        } else {
            let starts_segment = !identity_continues
                || series
                    .runs
                    .last()
                    .is_none_or(|previous| previous.last_hour.checked_add(1) != Some(frame.hour));
            series.runs.push(values.run(frame.hour, starts_segment));
            slot.last = Some(values.clone());
        }
    }
    builder.hour = Some(frame.hour);
    Ok(())
}

fn scan(
    source: &Source,
    branch: u64,
    regions: &Regions,
    areas: Option<&ProvinceAreas>,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Scan> {
    let mut result = Scan {
        format: source.replay_ref().format,
        ..Scan::default()
    };
    let mut builder = Builder::default();
    // Territory totals read the same tables, so one walk of the branch serves both.
    territory::walk_branch(source, branch, areas, context, cancelled, |frame| {
        observe(&mut result, frame, regions, areas, &mut builder)
    })?;
    result.series.sort_unstable_by(|a, b| {
        (&a.tag, a.region, &a.missing).cmp(&(&b.tag, b.region, &b.missing))
    });
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
                source,
                branch,
                &regions,
                areas.as_deref(),
                context,
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
            series: scanned.series.clone(),
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
