//! Exact territory weights grouped by current holder and content strategic region.
//! Region membership is content data, never a vanilla list or a guessed centroid.
//! Each state's exact contribution is split by its native content province roster.
use super::country_groups::{HourSpan, record_hour};
#[cfg(any(test, feature = "test-support"))]
use super::territory::{Territory, TerritoryParams};
use super::{
    LoadContext, Product,
    territory::{self, CountryReads, LandReads, TerritoryRun, TerritorySeries},
};
use crate::{
    Source,
    catalogue::{self, Catalogue},
};
use installed_content::province_area::ProvinceAreas;
use replay::{frame::DecodedFrame, rows::tables};
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
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryRegion {
    pub id: u32,
    pub name: Option<String>,
    pub missing: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryStateShare {
    pub numerator: String,
    pub denominator: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
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
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryRegionSeries {
    pub tag: Option<String>,
    pub region: Option<u32>,
    pub missing: Option<String>,
    pub runs: Vec<TerritoryRegionRun>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
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
pub struct Regions {
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
#[derive(Default, Serialize, Deserialize)]
pub struct Scan {
    /// The recording's format, which dates its daily tables
    /// ([`crate::metadata::slot_read`]).
    #[serde(skip)]
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
#[derive(Default, Serialize, Deserialize)]
struct Builder {
    #[serde(skip)]
    lineage: Option<Lineage>,
    identities: BTreeMap<usize, (u32, u32)>,
    /// The last hour with complete country tables.
    hour: Option<i32>,
    /// Interned holder tags.
    tags: Vec<String>,
    #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_map")]
    tag_ids: HashMap<String, usize>,
    #[serde(skip)]
    keys: HashMap<Key, usize>,
    /// Each series' holder and last values, by its index.
    slots: Vec<Slot>,
    /// Series by raw holder, region and whether for a state, under the current holders.
    #[serde(skip)]
    memo: HashMap<(u32, Option<u32>, bool), usize>,
    /// Each province row's id and raw holders, with their series under the current holders.
    #[serde(skip)]
    provinces: Vec<([u32; 3], [usize; 2])>,
    #[serde(skip)]
    reads: LandReads,
    /// The freshness the sums were taken with, and the sums by series.
    #[serde(skip)]
    available: Option<[bool; 8]>,
    #[serde(skip)]
    values: [BTreeMap<usize, RegionValues>; 2],
    active: BTreeSet<usize>,
    unknown: BTreeSet<usize>,
    #[serde(skip)]
    changed: BTreeSet<usize>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
}
struct Lineage {
    reads: CountryReads,
    /// Each raw link's tag, where exactly one tag names its canonical slot.
    holders: HashMap<u32, usize>,
    /// By interned tag: the native identity of the country holding it, and
    /// whether it names a link's holder.
    identities: BTreeMap<usize, (u32, u32)>,
    present: BTreeSet<usize>,
}
#[derive(Serialize, Deserialize)]
struct Slot {
    tag: Option<usize>,
    unknown_holder: bool,
    /// The values of the series' last run.
    last: Option<RegionValues>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
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
    fn read_lineage(&mut self, reads: CountryReads) -> Lineage {
        let mut found = Vec::new();
        let mut names: HashMap<u32, BTreeSet<usize>> = HashMap::new();
        for (tag, identity) in reads.holders() {
            let Some((slot, country)) = *identity else {
                continue;
            };
            // Holders are named only by non-empty UTF-8 tags.
            if tag.is_empty() {
                continue;
            }
            let next = self.tags.len();
            let id = *self.tag_ids.entry(tag.to_owned()).or_insert(next);
            if id == next {
                self.tags.push(tag.to_owned());
            }
            found.push((id, (slot, country)));
            names.entry(slot).or_default().insert(id);
        }
        let holders: HashMap<_, _> = reads
            .slots()
            .iter()
            .filter_map(|(raw, slot)| {
                let tags = names.get(slot).filter(|tags| tags.len() == 1)?;
                Some((*raw, *tags.first()?))
            })
            .collect();
        let mut identities = BTreeMap::new();
        for (id, identity) in found {
            identities.insert(id, identity);
        }
        let mut present = BTreeSet::new();
        for &id in holders.values() {
            present.insert(id);
        }
        Lineage {
            reads,
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
                self.keys.insert(key, self.slots.len() - 1);
                self.slots.len() - 1
            }
        };
        self.memo.insert(memo, slot);
        slot
    }

    /// This hour's sums for a series, from zero in each available measure.
    fn sums(&mut self, slot: usize, table: usize) -> &mut RegionValues {
        let available = self.available.expect("current sums");
        self.values[table]
            .entry(slot)
            .or_insert_with(|| RegionValues::zero(available))
    }
}

impl Builder {
    fn seal(&self, scan: &mut Scan) {
        let Some(hour) = self.hour else { return };
        for &index in &self.active {
            if let Some(run) = scan.series[index].runs.last_mut() {
                run.last_hour = hour;
            }
        }
        if let Some(available) = self.available {
            for (column, known) in available.into_iter().enumerate() {
                if known && let Some(span) = scan.observed[column].last_mut() {
                    span.last_hour = hour;
                }
            }
        }
    }
}

fn observe(
    scan: &mut Scan,
    frame: &DecodedFrame,
    regions: &Regions,
    areas: Option<&ProvinceAreas>,
    builder: &mut Builder,
    changes: Option<&crate::prepared::frame_changes::FrameChanges>,
) -> io::Result<()> {
    builder.changed.clear();
    let deferred = changes.is_some();
    let contiguous = builder.hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
    if deferred && !contiguous {
        builder.seal(scan);
    }
    let (Ok(countries), Ok(_)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) else {
        builder.seal(scan);
        if builder.hour.is_some() {
            builder.changed.extend(builder.active.iter().copied());
        }
        builder.unknown.clear();
        builder.hour = None;
        return Ok(());
    };
    let same = builder
        .lineage
        .as_ref()
        .is_some_and(|lineage| lineage.reads.matches_changes(frame, countries, changes));
    let mut affected = BTreeSet::new();
    if !same {
        let reads = builder.lineage.as_ref().filter(|_| {
            contiguous
                && changes.is_some_and(|changes| {
                    !changes.rows_changed[replay::rows::TableId::CountryLinks.index()]
                })
        });
        let reads = match reads {
            Some(lineage) => {
                CountryReads::from_slots(frame, countries, Arc::clone(lineage.reads.slots()))
            }
            None => CountryReads::new(frame, countries),
        };
        let lineage = builder.read_lineage(reads);
        scan.tags.extend(
            countries
                .iter()
                .filter_map(|country| std::str::from_utf8(&country.tag).ok())
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned),
        );
        if builder
            .lineage
            .as_ref()
            .is_none_or(|old| old.holders != lineage.holders)
        {
            builder.memo.clear();
            builder.provinces.clear();
            builder.available = None;
        }
        affected.extend(builder.active.iter().copied().filter(|&index| {
            let slot = &builder.slots[index];
            slot.tag
                .is_some_and(|tag| builder.identities.get(&tag) != lineage.identities.get(&tag))
        }));
        builder.lineage = Some(lineage);
    }
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
    let old_available = builder.available;
    let reads = builder.reads.update_changes(states, provinces, changes);
    #[cfg(any(test, feature = "test-support"))]
    {
        let touched = |table: replay::rows::TableId| {
            changes.is_none_or(|changes| {
                changes.rows_changed[table.index()] || changes.table_read[table.index()]
            })
        };
        builder.work.row_visits += usize::from(touched(replay::rows::TableId::Countries))
            * countries.len()
            + usize::from(touched(replay::rows::TableId::States))
                * states.map_or(0, <[replay::rows::StateRow]>::len)
            + usize::from(touched(replay::rows::TableId::Provinces))
                * provinces.map_or(0, <[replay::rows::ProvinceRow]>::len);
    }
    let refresh = reads.map(|same| !same || old_available.is_none());
    if old_available != Some(available) {
        affected.extend(builder.active.iter().copied());
        if let Some(old) = old_available
            && let Some(hour) = builder.hour
        {
            for (column, known) in old.into_iter().enumerate() {
                if known && let Some(span) = scan.observed[column].last_mut() {
                    span.last_hour = hour;
                }
            }
        }
    }
    builder.available = Some(available);
    for (column, known) in available.iter().enumerate() {
        if *known && (!deferred || old_available != Some(available) || !contiguous) {
            record_hour(&mut scan.observed[column], frame.hour);
        }
    }
    for (table, stale) in refresh.into_iter().enumerate() {
        if stale {
            affected.extend(builder.values[table].keys().copied());
            builder.values[table].clear();
        }
    }
    let reason = regions.state_reason.as_deref().unwrap_or(
        "State province roster absent or empty, or region membership missing or conflicting",
    );
    for state in states.filter(|_| refresh[0]).into_iter().flatten() {
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
                builder.sums(slot, 0).state(column, part)?;
                affected.insert(slot);
            }
        }
    }
    for (index, province) in provinces
        .filter(|_| refresh[1])
        .into_iter()
        .flatten()
        .enumerate()
    {
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
            builder.sums(slot, 1).province(column, &values)?;
            affected.insert(slot);
        }
    }
    if !deferred {
        affected.extend(0..builder.slots.len());
    }
    if !contiguous {
        affected.extend(builder.active.iter().copied());
        affected.extend(
            builder
                .values
                .iter()
                .flat_map(|values| values.keys())
                .copied(),
        );
    }
    affected.extend(builder.unknown.iter().copied());
    let lineage = builder.lineage.as_ref().expect("complete lineage");
    for index in affected {
        let slot = &mut builder.slots[index];
        let has_rows = builder
            .values
            .iter()
            .any(|values| values.contains_key(&index));
        let present = slot.tag.is_none_or(|tag| lineage.present.contains(&tag));
        if !has_rows && !present {
            if builder.active.remove(&index) {
                if let Some(hour) = builder.hour
                    && let Some(run) = scan.series[index].runs.last_mut()
                {
                    run.last_hour = hour;
                }
                builder.changed.insert(index);
            }
            builder.unknown.remove(&index);
            continue;
        }
        let mut values = RegionValues::zero(available);
        if let Some(states) = builder.values[0].get(&index) {
            values.states.clone_from(&states.states);
        }
        if let Some(provinces) = builder.values[1].get(&index) {
            values.values = provinces.values;
        }
        // A gap breaks every line, a holderless one too: its first hour after
        // the gap starts a run rather than stretching the one before over
        // hours not recorded.
        let identity_continues = !slot.unknown_holder
            && contiguous
            && slot.tag.is_none_or(|tag| {
                lineage
                    .identities
                    .get(&tag)
                    .is_some_and(|identity| builder.identities.get(&tag) == Some(identity))
            });
        if deferred
            && builder.active.contains(&index)
            && identity_continues
            && slot.last.as_ref() == Some(&values)
        {
            continue;
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            builder.work.history_visits += 1;
        }
        let series = &mut scan.series[index];
        if builder.active.contains(&index)
            && let Some(hour) = builder.hour
            && let Some(run) = series.runs.last_mut()
        {
            run.last_hour = hour;
        }
        if let Some(previous) = series.runs.last_mut()
            && identity_continues
            && previous.last_hour.checked_add(1) == Some(frame.hour)
            && slot.last.as_ref() == Some(&values)
        {
            previous.last_hour = frame.hour;
        } else {
            let starts_segment = !identity_continues
                || series
                    .runs
                    .last()
                    .is_none_or(|previous| previous.last_hour.checked_add(1) != Some(frame.hour));
            series.runs.push(values.run(frame.hour, starts_segment));
            slot.last = Some(values);
        }
        builder.active.insert(index);
        if slot.unknown_holder {
            builder.unknown.insert(index);
        }
        builder.changed.insert(index);
    }
    if !same {
        builder.identities.clone_from(&lineage.identities);
    }
    builder.hour = Some(frame.hour);
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub fn scan(
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
        observe(&mut result, frame, regions, areas, &mut builder, None)
    })?;
    result.series.sort_unstable_by(|a, b| {
        (&a.tag, a.region, &a.missing).cmp(&(&b.tag, b.region, &b.missing))
    });
    Ok(result)
}

#[cfg(any(test, feature = "test-support"))]
pub type CachedScan = (u64, Arc<Catalogue>, bool, Arc<Scan>);
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
    #[cfg(any(test, feature = "test-support"))]
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
        build_reply(&scanned, &regions, totals)
    }
}

fn build_reply(
    scanned: &Scan,
    regions: &Regions,
    totals: super::territory::TerritoryReply,
) -> io::Result<TerritoryBreakdownReply> {
    let mut warnings = regions.warnings.clone();
    if let Some(reason) = &regions.state_reason {
        warnings.push(reason.clone());
    }
    if !scanned.missing_states.is_empty() {
        warnings.push(format!("{} recorded states have no resolved native province roster; their state contributions remain in Region not attributable", scanned.missing_states.len()));
    }
    if !scanned.missing_provinces.is_empty() {
        warnings.push(format!("{} recorded provinces have missing or conflicting strategic region membership; their contributions remain in Region not attributable", scanned.missing_provinces.len()));
    }
    let reply = TerritoryBreakdownReply {
        branch_id: totals.branch_id,
        regions: regions.names.clone(),
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

/// One reached hour beyond accepted preparation. The observer's own step runs
/// over that hour's frame alone, read directly from the recording, so each
/// region series carries exactly the values preparation dates at the hour.
/// Run extents and segment starts are the hour's own evidence. Series the
/// frame does not reach are absent rather than zero, and the observed spans
/// say which columns the frame proves; nothing is written to the store.
pub(crate) fn sampled_reply(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    hour: i32,
    areas: Option<&ProvinceAreas>,
    totals: super::territory::TerritoryReply,
    cancelled: &AtomicBool,
) -> io::Result<TerritoryBreakdownReply> {
    // Missing content refuses exactly as the prepared catalogue's recorded reason does.
    let regions = context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
        .and_then(|cat| Regions::read(&cat))
        .map_err(|error| io::Error::other(error.to_string()))?;
    let frame = read_frame(source, branch, hour, cancelled)?;
    sample_reply(
        frame.as_deref(),
        source.replay_ref().format,
        &regions,
        areas,
        totals,
    )
}

/// The territory tables at one hour, decoded directly; none inside a gap.
fn read_frame(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<Box<DecodedFrame>>> {
    crate::prepared::query::progressive::frame_at(
        source,
        branch,
        hour,
        territory::TABLES,
        cancelled,
    )
}

/// The stateless regional step over one frame, or none at a gap.
fn sample_reply(
    frame: Option<&DecodedFrame>,
    format: replay::format::FormatVersion,
    regions: &Regions,
    areas: Option<&ProvinceAreas>,
    totals: super::territory::TerritoryReply,
) -> io::Result<TerritoryBreakdownReply> {
    let mut scan = Scan {
        format,
        ..Scan::default()
    };
    if let Some(frame) = frame {
        observe(
            &mut scan,
            frame,
            regions,
            areas,
            &mut Builder::default(),
            None,
        )?;
    }
    let mut reply = build_reply(&scan, regions, totals)?;
    reply.series.sort_unstable_by(|a, b| {
        (&a.tag, a.region, &a.missing).cmp(&(&b.tag, b.region, &b.missing))
    });
    Ok(reply)
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    builder: Builder,
    keys: BTreeMap<String, u64>,
    facts: crate::prepared::series::Accumulator,
    observed: crate::prepared::series::Accumulator,
    reason: Option<String>,
    #[serde(skip)]
    regions: Option<Regions>,
}

impl Prepared {
    pub fn configure(&mut self, source: &Source, context: &LoadContext) -> io::Result<()> {
        if self.regions.is_none() && self.reason.is_none() {
            match context
                .content()
                .and_then(|content| catalogue::catalogue(source, content))
                .and_then(|cat| Regions::read(&cat))
            {
                Ok(regions) => self.regions = Some(regions),
                Err(error) => self.reason = Some(error.to_string()),
            }
        }
        Ok(())
    }

    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        format: replay::format::FormatVersion,
        areas: Option<&ProvinceAreas>,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.scan.format = format;
        self.observe_with(frame, areas, Some(changes))
    }

    fn observe_with(
        &mut self,
        frame: &DecodedFrame,
        areas: Option<&ProvinceAreas>,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        self.facts.advance(frame.hour)?;
        self.observed.advance(frame.hour)?;
        // A restored builder keeps no freshness; publication sealed the span of
        // each column observed at its last complete hour there.
        let was_observed: [bool; 8] = match self.builder.available {
            Some(available) => available.map(|known| known && self.builder.hour.is_some()),
            None => std::array::from_fn(|column| {
                self.builder.hour.is_some_and(|hour| {
                    self.scan.observed[column]
                        .last()
                        .is_some_and(|span| span.last_hour == hour)
                })
            }),
        };
        let contiguous = self.builder.hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        let Some(regions) = &self.regions else {
            return Ok(());
        };
        if self.builder.keys.is_empty() {
            self.builder.keys = self
                .scan
                .series
                .iter()
                .enumerate()
                .map(|(index, series)| {
                    (
                        (series.tag.clone(), series.region, series.missing.clone()),
                        index,
                    )
                })
                .collect();
        }
        observe(
            &mut self.scan,
            frame,
            regions,
            areas,
            &mut self.builder,
            changes,
        )?;
        for &index in &self.builder.changed {
            let series = &self.scan.series[index];
            let key = serde_json::to_string(&(&series.tag, series.region, &series.missing))?;
            self.keys.entry(key).or_insert(index as u64);
            let current = series.runs.last().filter(|run| run.last_hour == frame.hour);
            if let Some(run) = current {
                if run.first_hour == frame.hour {
                    self.facts.event(index as u64, frame.hour, run)?;
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.builder.work.emitted += 1;
                    }
                }
                self.facts.observe(
                    index as u64,
                    frame.hour,
                    Some(i128::from(run.first_hour)),
                    run.first_hour == frame.hour && run.starts_segment,
                );
            } else {
                self.facts.observe(index as u64, frame.hour, None, false);
            }
        }
        for (column, known) in self
            .builder
            .available
            .unwrap_or([false; 8])
            .into_iter()
            .enumerate()
        {
            let known = known && self.builder.hour == Some(frame.hour);
            if !contiguous || known != was_observed[column] {
                self.observed
                    .observe(column as u64, frame.hour, known.then_some(1), false);
            }
        }
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        self.facts.bytes() + self.observed.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        use crate::prepared::series;
        self.builder.seal(&mut self.scan);
        self.facts.flush(
            writer,
            &format!("territory-breakdown-facts-{branch}"),
            0,
            cancelled,
        )?;
        self.observed.flush(
            writer,
            &format!("territory-breakdown-observed-{branch}"),
            0,
            cancelled,
        )?;
        let metadata = self
            .regions
            .as_ref()
            .map(|regions| {
                build_reply(
                    &self.scan,
                    regions,
                    super::territory::TerritoryReply {
                        branch_id: branch.to_string(),
                        world_totals: Vec::new(),
                        series: Vec::new(),
                        area_unit: None,
                        area_reason: None,
                        warnings: Vec::new(),
                    },
                )
                .map(|mut reply| {
                    for series in &mut reply.series {
                        series.runs.clear();
                    }
                    reply.observed = Default::default();
                    reply
                })
            })
            .transpose()?;
        series::write_catalogue(
            writer,
            "territory-breakdown",
            branch,
            &self.keys,
            &(metadata, &self.reason),
        )?;
        for series in &mut self.scan.series {
            if series.runs.len() > 1 {
                series.runs.drain(..series.runs.len() - 1);
            }
        }
        for spans in &mut self.scan.observed {
            if spans.len() > 1 {
                spans.drain(..spans.len() - 1);
            }
        }
        Ok(())
    }

    pub fn reply(
        store: &store::Store,
        key: &store::SourceKey,
        branch: u64,
        from: i32,
        to: i32,
        totals: super::territory::TerritoryReply,
        cancelled: &AtomicBool,
    ) -> io::Result<TerritoryBreakdownReply> {
        use crate::prepared::series::{self, Reader};
        let (_, (metadata, reason)) = series::read_catalogue::<(
            Option<TerritoryBreakdownReply>,
            Option<String>,
        )>(store, key, "territory-breakdown", branch)?
        .ok_or_else(|| crate::invalid("Prepared regional catalogue is missing"))?;
        let mut reply = metadata.ok_or_else(|| {
            io::Error::other(reason.unwrap_or_else(|| "Territory regions are unavailable".into()))
        })?;
        let mut facts = Reader::new(
            store,
            key,
            format!("territory-breakdown-facts-{branch}"),
            cancelled,
        )?;
        for (index, series) in reply.series.iter_mut().enumerate() {
            let markers = if from == to {
                facts.value_at(index as u64, from)?.into_iter().collect()
            } else {
                facts.runs(index as u64, from, to)?
            };
            for marker in markers {
                let (_, row) = facts
                    .latest_event(index as u64, marker.first_hour)?
                    .ok_or_else(|| crate::invalid("Regional territory facts are missing"))?;
                let mut run: TerritoryRegionRun = serde_json::from_value(row)?;
                run.first_hour = marker.first_hour;
                run.last_hour = marker.last_hour;
                run.starts_segment &= marker.first_hour == marker.value as i32;
                series.runs.push(run);
            }
        }
        let mut observed = Reader::new(
            store,
            key,
            format!("territory-breakdown-observed-{branch}"),
            cancelled,
        )?;
        for (column, spans) in reply.observed.iter_mut().enumerate() {
            *spans = observed
                .runs(column as u64, from, to)?
                .into_iter()
                .map(|run| HourSpan {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                })
                .collect();
        }
        reply.countries = totals.series;
        reply.world_totals = totals.world_totals;
        reply.area_unit = totals.area_unit;
        reply.area_reason = totals.area_reason;
        reply.warnings.extend(totals.warnings);
        reply.series.sort_unstable_by(|a, b| {
            (&a.tag, a.region, &a.missing).cmp(&(&b.tag, b.region, &b.missing))
        });
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
#[cfg(test)]
mod tests;
