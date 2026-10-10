//! Same-pass spotting arrays; only numerical projection leaves capture.
use crate::table_capture::{Arena, grow_vec};
use engine::{Memory, ccountry, transport_spotting_context as native};
use replay::rows::{
    TransportAirEnvironment, TransportNavyIntel, TransportRadarCounts, TransportSpottingContext,
    TransportSpottingOwner, TransportSpottingRegion,
};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
type Fact<T> = Result<T, &'static str>;
type RegionEnvironmentObserver<'a> = dyn FnMut(Fact<u32>, u32, Fact<usize>) -> (Fact<Option<i64>>, Option<TransportAirEnvironment>)
    + 'a;

const NAMES: [&str; 21] = [
    "transport original spotting country domain refused",
    "transport current spotting owned-state gate refused",
    "transport original spotting radar header refused",
    "transport original spotting radar body refused",
    "transport original spotting intel root refused",
    "transport original spotting intel owner refused",
    "transport original spotting intel canonical owner refused",
    "transport original spotting intel count refused",
    "transport original spotting intel body refused",
    "transport original spotting region domain refused",
    "transport original spotting region runtime slot refused",
    "transport original spotting sampled province refused",
    "transport current spotting regional weather refused",
    "transport original regional air environment refused",
    "transport original regional air weather attack refused",
    "transport original regional air weather defence refused",
    "transport original regional air night factor refused",
    "transport original regional air night attack refused",
    "transport original regional air night defence refused",
    "transport original regional air target region refused",
    "transport original spotting signed region province count refused",
];
static REFUSALS: [AtomicU64; NAMES.len()] = [const { AtomicU64::new(0) }; NAMES.len()];
#[cfg(test)]
thread_local! {
    static OWNER_REFUSALS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
fn refuse(index: usize) {
    REFUSALS[index].fetch_add(1, Relaxed);
    #[cfg(test)]
    if index == 0 {
        OWNER_REFUSALS.with(|count| count.set(count.get() + 1));
    }
}
pub(crate) fn status() -> impl serde::Serialize {
    crate::status::map(
        NAMES
            .into_iter()
            .zip(REFUSALS.iter().map(|count| count.load(Relaxed))),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Capacities {
    pub owners: usize,
    pub regions: usize,
    pub radar_values: usize,
    pub intel_values: usize,
}
#[derive(Clone, Copy)]
struct SignedSpan {
    count: i32,
    span: Fact<(usize, usize)>,
}
#[derive(Clone, Copy)]
struct Intel {
    owner_raw: Option<u32>,
    owner_slot: Option<u32>,
    values: Option<SignedSpan>,
}
#[derive(Clone, Copy, Default)]
struct Owner {
    slot: u32,
    raw: u32,
    owned_states: Option<i32>,
    radar: Option<SignedSpan>,
    intel: Option<Intel>,
}

pub(crate) struct Captured {
    hour: Option<i32>,
    owner_count: Fact<usize>,
    region_count: Fact<usize>,
    owner_attempts: usize,
    region_attempts: usize,
    owner_complete: bool,
    region_complete: bool,
    owners: Arena<Owner>,
    regions: Arena<TransportSpottingRegion>,
    radar: Arena<i32>,
    intel: Arena<i64>,
    current_radar_required: usize,
    current_intel_required: usize,
    // Native identity/failure memo; retired at original callback join.
    roots: Vec<Option<ccountry::Country>>,
    reserved_attempted: bool,
    reserved_root: Option<ccountry::Country>,
    reserved: Option<Owner>,
    // Keep the existing holder layout for the enclosing pack's pool_bytes accounting.
    metadata: native::WeatherMetadata,
    finished: bool,
    regions_loaned: bool,
    epoch: u64,
}
impl Captured {
    pub(crate) fn new(capacity: Capacities) -> Self {
        Self {
            hour: None,
            owner_count: Err("transport spotting owner domain not supplied"),
            region_count: Err("transport spotting region domain not supplied"),
            owner_attempts: 0,
            region_attempts: 0,
            owner_complete: false,
            region_complete: false,
            owners: Arena::new(capacity.owners),
            regions: Arena::new(capacity.regions),
            radar: Arena::new(capacity.radar_values),
            intel: Arena::new(capacity.intel_values),
            current_radar_required: 0,
            current_intel_required: 0,
            roots: engine::allocation::owned_vec(engine::allocation::filled(capacity.owners, None)),
            reserved_attempted: false,
            reserved_root: None,
            reserved: None,
            metadata: native::WeatherMetadata::default(),
            finished: false,
            regions_loaned: false,
            epoch: 0,
        }
    }
    pub(crate) fn reset(&mut self) {
        let demand = [
            self.owners.required,
            self.regions.required,
            self.radar.required,
            self.intel.required,
        ];
        self.epoch = self.epoch.wrapping_add(1);
        self.hour = None;
        self.owner_count = Err("transport spotting owner domain not supplied");
        self.region_count = Err("transport spotting region domain not supplied");
        self.owner_attempts = 0;
        self.region_attempts = 0;
        self.current_radar_required = 0;
        self.current_intel_required = 0;
        self.owner_complete = false;
        self.region_complete = false;
        self.finished = false;
        self.owners.reset();
        self.regions.reset();
        self.radar.reset();
        self.intel.reset();
        self.owners.required = demand[0];
        self.regions.required = demand[1];
        self.radar.required = demand[2];
        self.intel.required = demand[3];
        self.roots.fill(None);
        self.reserved_attempted = false;
        self.reserved_root = None;
        self.reserved = None;
        self.metadata.clear();
    }
    pub(crate) fn begin(&mut self, hour: i32, original_count: Fact<usize>) {
        self.reset();
        self.hour = Some(hour);
        self.owner_count = original_count;
        self.owner_complete = original_count.is_ok();
        if let Ok(count) = original_count {
            self.owners.required = self.owners.required.max(count);
            grow_vec(&mut self.owners.first, count);
            grow_vec(&mut self.roots, count);
        }
        // Countries starts with an unauthenticated placeholder, replaced by
        // its original Reader domain. Count an unhealed failure at finish.
    }
    pub(crate) fn begin_regions(&mut self, original_count: Fact<usize>) {
        if self.finished || self.hour.is_none() || self.region_attempts != 0 {
            self.region_complete = false;
            refuse(9);
            return;
        }
        self.region_count = original_count;
        self.region_complete = original_count.is_ok();
        if let Ok(count) = original_count {
            self.regions.required = self.regions.required.max(count);
            grow_vec(&mut self.regions.first, count);
        } else {
            refuse(9);
        }
    }
    /// All original physical slots, including nulls and aliases, are supplied.
    /// An authenticated alias borrows its prior result; no array is retried.
    pub(crate) fn country<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        physical_slot: u32,
        original: Fact<Option<ccountry::Country>>,
    ) {
        if self.finished || self.hour.is_none() || physical_slot as usize != self.owner_attempts {
            self.owner_complete = false;
            refuse(0);
            return;
        }
        self.owner_attempts += 1;
        let country = match original {
            Ok(Some(country)) => country,
            Ok(None) => return,
            Err(_) => {
                self.owner_complete = false;
                refuse(0);
                return;
            }
        };
        // Genuine reserved naming owns the fallback payload/count loan. A
        // physical alias authenticates identity without rereading that payload.
        if country.canonical_slot == 0 {
            return;
        }
        let slot = country.canonical_slot as usize;
        let Some(root) = self.roots.get_mut(slot) else {
            self.owner_complete = false;
            refuse(0);
            return;
        };
        if let Some(previous) = *root {
            if previous != country {
                self.owner_complete = false;
                refuse(0);
            }
            return;
        }
        // Admit the attempt before source acquisition, including failures.
        *root = Some(country);
        if !grow_vec(&mut self.owners.first, self.owners.len.saturating_add(1)) {
            self.owner_complete = false;
            refuse(0);
            return;
        }
        let observed = native::Owner::capture_authenticated(memory, base, countries, country);
        let row = self.materialize(memory, observed);
        self.owners.first[self.owners.len] = row;
        self.owners.len += 1;
    }
    pub(crate) fn reserved_with_owned_count<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        original: Fact<ccountry::Country>,
        owned_states: Fact<i32>,
    ) {
        if self.finished || self.hour.is_none() {
            refuse(0);
            return;
        }
        if self.reserved_attempted {
            if original.is_ok_and(|country| Some(country) != self.reserved_root) {
                self.reserved = None;
                refuse(0);
            }
            return;
        }
        self.reserved_attempted = true;
        match original {
            Ok(country) if country.canonical_slot == 0 && country.id == 0 => {
                self.reserved_root = Some(country);
                let observed = native::Owner::capture_with_owned_states(
                    memory,
                    base,
                    countries,
                    country,
                    owned_states,
                );
                self.reserved = Some(self.materialize(memory, observed));
            }
            _ => {
                refuse(0);
            }
        }
    }
    fn materialize(&mut self, memory: &impl Memory, observed: native::Owner) -> Owner {
        if observed.owned_states.is_err() {
            refuse(1);
        }
        let radar = match observed.radar {
            Err(_) => {
                refuse(2);
                None
            }
            Ok(array) => {
                let count = array.signed_count.max(0) as usize;
                self.current_radar_required = self.current_radar_required.saturating_add(count);
                self.radar.required = self.radar.required.max(self.current_radar_required);
                let start = self.radar.len;
                let span = (|| {
                    let end = start
                        .checked_add(count)
                        .ok_or("transport spotting radar arena extent overflow")?;
                    // Getter141091B70 reaches the original pool+10/+1C.
                    // Grow from that acquired header without rereading it.
                    grow_vec(&mut self.radar.first, end);
                    let output = self
                        .radar
                        .first
                        .get_mut(start..end)
                        .ok_or("transport spotting radar arena exhausted")?;
                    native::copy_radar(memory, array, output)?;
                    self.radar.len = end;
                    Ok((start, count))
                })();
                if span.is_err() {
                    refuse(3);
                }
                Some(SignedSpan {
                    count: array.signed_count,
                    span,
                })
            }
        };
        let intel = match observed.intel {
            Err(_) => {
                refuse(4);
                None
            }
            Ok(intel) => {
                if intel.owner_raw.is_err() {
                    refuse(5);
                }
                if intel.owner_slot.is_err() {
                    refuse(6);
                }
                let values = match intel.array {
                    Err(_) => {
                        refuse(7);
                        None
                    }
                    Ok(array) => {
                        let count = array.signed_count.max(0) as usize;
                        self.current_intel_required =
                            self.current_intel_required.saturating_add(count);
                        self.intel.required = self.intel.required.max(self.current_intel_required);
                        let start = self.intel.len;
                        let span = (|| {
                            let end = start
                                .checked_add(count)
                                .ok_or("transport spotting intel arena extent overflow")?;
                            // Getter140CEE3A0 reaches signed intel+1C lanes.
                            grow_vec(&mut self.intel.first, end);
                            let output = self
                                .intel
                                .first
                                .get_mut(start..end)
                                .ok_or("transport spotting intel arena exhausted")?;
                            native::copy_intel(memory, array, output)?;
                            self.intel.len = end;
                            Ok((start, count))
                        })();
                        if span.is_err() {
                            refuse(8);
                        }
                        Some(SignedSpan {
                            count: array.signed_count,
                            span,
                        })
                    }
                };
                Some(Intel {
                    owner_raw: intel.owner_raw.ok(),
                    owner_slot: intel.owner_slot.ok().flatten(),
                    values,
                })
            }
        };
        Owner {
            slot: observed.owner_slot,
            raw: observed.owner_raw,
            owned_states: observed.owned_states.ok(),
            radar,
            intel,
        }
    }
    /// The source owner retains this selector attempt independently of context
    /// row capacity so the older strict borrower can consume the same result.
    pub(crate) fn region_with_observed_weather<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        slot: u32,
        original: Fact<Option<native::RegionOriginal>>,
        weather_record: Fact<usize>,
        observer: &mut RegionEnvironmentObserver<'_>,
    ) {
        let Some(original) = self.admit_region(slot, original) else {
            return;
        };
        let (region, air_environment) = native::capture_region_with_record(
            memory,
            base,
            self.hour.unwrap(),
            original,
            weather_record,
            observer,
        );
        self.store_region(region, air_environment);
    }
    fn admit_region(
        &mut self,
        slot: u32,
        original: Fact<Option<native::RegionOriginal>>,
    ) -> Option<native::RegionOriginal> {
        if self.finished || self.hour.is_none() || slot as usize != self.region_attempts {
            self.region_complete = false;
            refuse(9);
            return None;
        }
        self.region_attempts += 1;
        let original = match original {
            Ok(Some(original)) => original,
            Ok(None) => return None,
            Err(_) => {
                self.region_complete = false;
                refuse(9);
                return None;
            }
        };
        if self.regions.len == self.regions.first.len() {
            self.region_complete = false;
            refuse(9);
            return None;
        }
        Some(original)
    }
    fn store_region(
        &mut self,
        region: native::Region,
        air_environment: Option<TransportAirEnvironment>,
    ) {
        match air_environment {
            None => refuse(13),
            Some(environment) => {
                for (index, missing) in [
                    environment.weather[0].is_none(),
                    environment.weather[1].is_none(),
                    environment.night_factor.is_none(),
                    environment.night[0].is_none(),
                    environment.night[1].is_none(),
                    environment.target_region.is_none(),
                ]
                .into_iter()
                .enumerate()
                {
                    if missing {
                        refuse(index + 14);
                    }
                }
            }
        }
        if region.province_count.is_err() {
            refuse(20);
        }
        for (index, missing) in [
            (10, region.runtime_slot.is_err()),
            (11, region.sampled_province.is_err()),
            (12, region.weather_detection.is_err()),
        ] {
            if missing {
                refuse(index);
            }
        }
        self.regions.first[self.regions.len] = TransportSpottingRegion {
            region_id: region.id,
            runtime_slot: region.runtime_slot.ok(),
            signed_province_count: region.province_count.ok(),
            sampled_province: region.sampled_province.ok(),
            weather_detection: region.weather_detection.ok(),
            air_environment,
        };
        self.regions.len += 1;
    }
    pub(crate) fn finish(&mut self) {
        self.owner_complete &= self.owner_count == Ok(self.owner_attempts);
        self.region_complete &= self.region_count == Ok(self.region_attempts);
        if !self.owner_complete {
            refuse(0);
        }
        if !self.region_complete && self.region_count.is_ok() {
            refuse(9);
        }
        self.roots.fill(None);
        self.reserved_root = None;
        self.metadata.clear();
        self.finished = true;
    }
    pub(crate) fn fail(&mut self) {
        self.owner_complete = false;
        self.region_complete = false;
        self.finish();
    }
    pub(crate) fn sampled_regions(&self, hour: i32) -> Fact<&[TransportSpottingRegion]> {
        if !self.finished || self.hour != Some(hour) || !self.region_complete || self.regions_loaned
        {
            return Err("transport original same-hour sampled regions unavailable");
        }
        self.regions
            .first
            .get(..self.regions.len)
            .ok_or("transport original sampled region extent invalid")
    }
    pub(crate) fn take_regions(&mut self, hour: i32) -> RegionLoan {
        let length = self.sampled_regions(hour).map(|rows| rows.len());
        if self.regions_loaned {
            return RegionLoan {
                hour: self.hour,
                epoch: None,
                length: Err("transport original region rows already loaned"),
                rows: Vec::new(),
            };
        }
        self.regions_loaned = true;
        RegionLoan {
            hour: self.hour,
            epoch: Some(self.epoch),
            length,
            rows: std::mem::take(&mut self.regions.first),
        }
    }
    pub(crate) fn restore_regions(&mut self, loan: RegionLoan) -> Fact<()> {
        if !self.regions_loaned || loan.epoch.is_none() {
            return Err("transport original region loan has no owner");
        }
        self.regions_loaned = false;
        self.regions.first = loan.rows;
        if loan.hour != self.hour
            || loan.epoch != Some(self.epoch)
            || loan.length.is_ok_and(|len| len != self.regions.len)
            || self.regions.first.len() < self.regions.len
        {
            self.regions.len = 0;
            self.region_complete = false;
            return Err("transport original region loan association changed");
        }
        Ok(())
    }
    pub(crate) fn prepare(&mut self, capacity: Capacities) {
        grow_vec(&mut self.owners.first, capacity.owners);
        grow_vec(&mut self.roots, capacity.owners);
        grow_vec(&mut self.regions.first, capacity.regions);
        grow_vec(&mut self.radar.first, capacity.radar_values);
        grow_vec(&mut self.intel.first, capacity.intel_values);
        self.grow();
    }
    pub(crate) fn grow(&mut self) {
        let mut count = self.owners.first.len();
        self.owners.grow(&mut count);
        grow_vec(&mut self.roots, count);
        let mut count = self.regions.first.len();
        self.regions.grow(&mut count);
        let mut count = self.radar.first.len();
        self.radar.grow(&mut count);
        let mut count = self.intel.first.len();
        self.intel.grow(&mut count);
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.owners.allocated_bytes()
            + self.regions.allocated_bytes()
            + self.radar.allocated_bytes()
            + self.intel.allocated_bytes()
            + self.roots.capacity() * std::mem::size_of::<Option<ccountry::Country>>()
    }
    fn owner_projection(&self, original: Owner) -> std::io::Result<TransportSpottingOwner> {
        Ok(TransportSpottingOwner {
            owner_slot: original.slot,
            owner_raw_id: original.raw,
            target_owned_states: original.owned_states,
            radar: original
                .radar
                .map(|radar| {
                    Ok::<_, std::io::Error>(TransportRadarCounts {
                        signed_count: radar.count,
                        counts: radar
                            .span
                            .ok()
                            .map(|(start, count)| {
                                engine::allocation::copy_vec(
                                    &self.radar.first[start..start + count],
                                )
                            })
                            .transpose()?,
                    })
                })
                .transpose()?,
            navy_intel: original
                .intel
                .map(|intel| {
                    Ok::<_, std::io::Error>(TransportNavyIntel {
                        owner_raw_id: intel.owner_raw,
                        owner_slot: intel.owner_slot,
                        signed_count: intel.values.map(|values| values.count),
                        lanes: intel
                            .values
                            .and_then(|values| values.span.ok())
                            .map(|(start, count)| {
                                engine::allocation::copy_vec(
                                    &self.intel.first[start..start + count],
                                )
                            })
                            .transpose()?,
                    })
                })
                .transpose()?,
        })
    }
    pub(crate) fn projection(&self) -> std::io::Result<Option<TransportSpottingContext>> {
        let Some(sample_hour) = self.hour.filter(|_| self.finished && !self.regions_loaned) else {
            return Ok(None);
        };
        let mut owners = engine::allocation::vec(self.owners.len)?;
        for &owner in &self.owners.first[..self.owners.len] {
            owners.push(self.owner_projection(owner)?);
        }
        owners.sort_unstable_by_key(|owner| owner.owner_slot);
        Ok(Some(TransportSpottingContext {
            sample_hour,
            owners_complete: self.owner_complete,
            regions_complete: self.region_complete,
            owners,
            reserved: self
                .reserved
                .map(|owner| self.owner_projection(owner))
                .transpose()?,
            regions: engine::allocation::copy_vec(&self.regions.first[..self.regions.len])?,
        }))
    }
}

/// Numerical buffer loan to the dependent air job, after original native roots
/// and weather metadata have retired. The allocation returns to its owner.
pub(crate) struct RegionLoan {
    hour: Option<i32>,
    epoch: Option<u64>,
    length: Fact<usize>,
    rows: Vec<TransportSpottingRegion>,
}
impl RegionLoan {
    pub(crate) fn samples(&self, hour: i32) -> Fact<&[TransportSpottingRegion]> {
        if self.hour != Some(hour) {
            return Err("transport sampled region loan is stale");
        }
        self.rows
            .get(..self.length?)
            .ok_or("transport sampled region loan extent invalid")
    }
}

/// Join numerical products after both independent original native jobs return.
/// Neither job shares a pointer holder; cross-hour assembly is a refusal.
pub(crate) fn merge(
    owners: Option<TransportSpottingContext>,
    regions: Option<TransportSpottingContext>,
) -> Option<TransportSpottingContext> {
    match (owners, regions) {
        (Some(mut owners), Some(regions)) if owners.sample_hour == regions.sample_hour => {
            owners.regions_complete = regions.regions_complete;
            owners.regions = regions.regions;
            Some(owners)
        }
        (Some(mut owners), None) => {
            owners.regions_complete = false;
            owners.regions.clear();
            Some(owners)
        }
        (None, Some(mut regions)) => {
            regions.owners_complete = false;
            regions.owners.clear();
            regions.reserved = None;
            Some(regions)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "transport_spotting_context_tests.rs"]
mod tests;
