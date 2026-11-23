use content_reader_key_names::unit_leader::AirWings as _;
use super::{Context, carrier_summary};
use replay_data::{
    rows::AirBaseRegistrationRow, rows::AirBaseRow, rows::AirWingRow, rows::CarrierAirPool,
    rows::tables,
};
use std::{cell::OnceCell, collections::BTreeMap, io};
use viewer_unit_reads::{air_detection, air_range};

const ONE: i64 = 100_000;
const SENTINEL: i64 = u32::MAX as i64;
const MAXIMUM_MISSIONS: usize = 18;
const FUEL_KEY: u32 =
    engine_native::ccountry::AIR_MODIFIER_KEYS[air_detection::CountryModifier::Fuel as usize];

pub(crate) struct Defines {
    costs: Result<Vec<i64>, String>,
    thresholds: Result<Vec<i64>, String>,
    weight: Result<i64, String>,
    night_threshold: Result<i64, String>,
    ace: Result<air_range::Defines, String>,
}

impl Defines {
    pub fn load(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        Ok(Self {
            costs: defines
                .fixed5_array("NAir", "MISSION_FUEL_COSTS")
                .map_err(|error| error.to_string()),
            thresholds: defines
                .fixed5_array("NAir", "AGGRESSION_THRESHOLD")
                .map_err(|error| error.to_string()),
            weight: defines
                .fixed5("NAir", "FUEL_COST_MULT")
                .map_err(|error| error.to_string()),
            night_threshold: defines
                .fixed5("NAir", "DAY_NIGHT_COVERAGE_FACTOR")
                .map_err(|error| error.to_string()),
            ace: (|| -> io::Result<_> {
                Ok(air_range::Defines {
                    map_scale: 0,
                    ace_wing_size: defines.fixed5("NAir", "ACE_WING_SIZE")?,
                    ace_wing_size_max_bonus: defines.fixed5("NAir", "ACE_WING_SIZE_MAX_BONUS")?,
                })
            })()
            .map_err(|error| error.to_string()),
        })
    }
    fn maximum_cost(&self) -> Result<i64, String> {
        self.costs
            .as_ref()
            .map_err(Clone::clone)?
            .get(..MAXIMUM_MISSIONS)
            .map(|values| values.iter().copied().fold(0, i64::max))
            .ok_or_else(|| "Air maximum fuel needs the complete native mission-cost prefix".into())
    }
    fn cost(&self, index: usize) -> Result<i64, String> {
        self.costs
            .as_ref()
            .map_err(Clone::clone)?
            .get(index)
            .copied()
            .ok_or_else(|| {
                "Air fuel mission index is outside the recorded-content cost table".into()
            })
    }
}

pub(super) struct Reader<'a, 'b> {
    context: &'a Context<'b>,
    countries: OnceCell<Result<air_detection::Countries<'b>, String>>,
    sources: air_range::FrameSources<'b>,
    aces: OnceCell<Result<BTreeMap<u64, &'b replay_data::rows::AceRangeInputRow>, String>>,
    nights: BTreeMap<u32, Result<u8, String>>,
}

impl<'a, 'b> Reader<'a, 'b> {
    pub fn new(context: &'a Context<'b>) -> Self {
        Self {
            context,
            countries: OnceCell::new(),
            sources: air_range::FrameSources::new(context.frame),
            aces: OnceCell::new(),
            nights: BTreeMap::new(),
        }
    }
    fn countries(&self) -> Result<&air_detection::Countries<'b>, String> {
        self.countries
            .get_or_init(|| air_detection::Countries::read(self.context.frame))
            .as_ref()
            .map_err(Clone::clone)
    }
    fn defines(&self) -> Result<&Defines, String> {
        self.context
            .content
            .ok_or("state Air fuel content unavailable")?
            .province_hover
            .air_fuel
            .as_ref()
            .map_err(Clone::clone)
    }
    fn modified(&self, wing: &AirWingRow) -> Result<i64, String> {
        use air_range::Sources;
        let base = wing
            .fuel_base_raw
            .require("AirWings.fuel_base_raw")?
            .ok_or("state Air wing cached base fuel unavailable")?;
        let raw = if wing.country_override > 0 {
            wing.country_override as u32
        } else {
            wing.owner
        };
        let country = if raw == 0 { 0 } else { self.context.slot(raw)? };
        let countries = self.countries()?;
        let mut ace_term = 0;
        if wing.ace != 0 {
            let aces = self
                .aces
                .get_or_init(|| {
                    self.context
                        .frame
                        .complete::<tables::AceRangeInputs>()
                        .map(|rows| rows.iter().map(|ace| (ace.ace, ace)).collect())
                        .map_err(|error| error.to_string())
                })
                .as_ref()
                .map_err(Clone::clone)?;
            if let Some(ace) = aces.get(&wing.ace)
                && ace.alive != 0
            {
                let [raw] = ace
                    .fuel_modifier
                    .require("living ace fuel modifier")?
                    .ok_or("living ace fuel accumulator unavailable")?;
                let raw = countries.effective(FUEL_KEY, raw)?;
                let count = air_range::ace_scale(
                    ace,
                    *self.defines()?.ace.as_ref().map_err(Clone::clone)?,
                    &self.sources,
                )?;
                let owner = self
                    .sources
                    .command_power(ace.canonical_slot)?
                    .ace_effectiveness_factor
                    .ok_or("ace fuel effectiveness modifier unavailable")?;
                ace_term =
                    (raw.wrapping_mul(count) / ONE).wrapping_mul(ONE.wrapping_add(owner)) / ONE;
            }
        }
        Ok(modified_base(
            base,
            ace_term,
            countries.value(country, air_detection::CountryModifier::Fuel)?,
        ))
    }
    fn current_cost(
        &self,
        wing: &AirWingRow,
        base: &AirBaseRow,
        pool: &CarrierAirPool,
    ) -> Result<Option<i64>, String> {
        let current = wing
            .fuel_current_type
            .require("AirWings.fuel_current_type")?
            .ok_or("state Air current execution unavailable")?;
        let (count, index) = if current != 0 {
            if current < 0 || !(current as u32).is_power_of_two() {
                return Err("state Air current fuel mission is not one positive bit".into());
            }
            (
                wing.fuel_effective_aircraft
                    .require("AirWings.fuel_effective_aircraft")?
                    .ok_or("state Air execution aircraft unavailable")?,
                (current as u32).trailing_zeros() as usize,
            )
        } else {
            if wing.mission_mask & 0x800 == 0
                || wing.eligibility_veto != 0
                || wing.eligibility_counter > wing.eligibility_limit
            {
                return Ok(None);
            }
            if wing.planes <= 0 {
                return Ok(None);
            }
            match wing
                .fuel_override_resolved
                .require("AirWings.fuel_override_resolved")?
                .ok_or("state Air training base override liveness unavailable")?
            {
                0 => (),
                1 => return Ok(None),
                _ => return Err("state Air training base override liveness is malformed".into()),
            }
            let class = wing
                .fuel_capacity_class
                .require("AirWings.fuel_capacity_class")?
                .ok_or("state Air training capacity class unavailable")?;
            let thresholds = self.defines()?.thresholds.as_ref().map_err(Clone::clone)?;
            let threshold = usize::try_from(class)
                .ok()
                .and_then(|index| thresholds.get(index).copied())
                .ok_or(
                    "state Air training capacity class is outside recorded-content thresholds",
                )?;
            let capacity = self.capacity(base, pool)?;
            if !capacity_gate(wing.planes, capacity, threshold) {
                return Ok(None);
            }
            (wing.planes, 11)
        };
        Ok(Some(
            i64::from(count).wrapping_mul(self.defines()?.cost(index)?),
        ))
    }
    fn capacity(&self, base: &AirBaseRow, pool: &CarrierAirPool) -> Result<i32, String> {
        if base.state.is_none() {
            return Err("state Air fuel capacity queried for a host base".into());
        }
        let (name, flags) = self
            .context
            .units
            .as_ref()
            .map_err(Clone::clone)?
            .subunit(pool.subunit_index)?;
        if flags & (1_u64 << 33) != 0 {
            return Ok(base.capacity);
        }
        if flags & 0x1f_0037_fc00 == 0 {
            return Err("state Air training subtype is not air".into());
        }
        let content = self
            .context
            .content
            .ok_or("state Air fuel subtype content unavailable")?;
        Ok(content
            .sub_units
            .as_ref()
            .map_err(Clone::clone)?
            .air_wing_sizes(name.as_bytes(), &content.equipment)?[0])
    }
    fn region_nights(&mut self, region: u32) -> Result<u8, String> {
        if let Some(value) = self.nights.get(&region) {
            return value.clone();
        }
        let value = self.read_region_nights(region);
        self.nights.insert(region, value.clone());
        value
    }
    fn read_region_nights(&self, region: u32) -> Result<u8, String> {
        let content = self
            .context
            .content
            .ok_or("state Air forecast content unavailable")?;
        let members = &content
            .strategic_regions
            .get(&region)
            .ok_or("state Air forecast strategic region unavailable")?
            .provinces;
        let threshold = *self
            .defines()?
            .night_threshold
            .as_ref()
            .map_err(Clone::clone)?;
        if members.is_empty() {
            return Ok(if SENTINEL > threshold { 24 } else { 0 });
        }
        let reads = self
            .context
            .reads
            .ok_or("state Air forecast geometry unavailable")?;
        let defines = reads.day_night.as_ref().map_err(Clone::clone)?;
        let weather = self
            .context
            .frame
            .complete::<tables::WeatherProvinces>()
            .map_err(|error| error.to_string())?;
        let zero = weather
            .first()
            .filter(|row| row.id == 0)
            .ok_or("state Air forecast weather singleton unavailable")?;
        let daylight = zero
            .daylight
            .require("WeatherProvinces.daylight")?
            .as_ref()
            .ok_or("state Air forecast direction unavailable")?;
        let direction = daylight.direction(self.context.frame.hour)?;
        let feather = daylight.feather(defines)?;
        let mut sums = [0_i64; 24];
        for &province in members {
            weather
                .binary_search_by_key(&province, |row| row.id)
                .map_err(|_| "state Air forecast province clock is outside the recorded census")?;
            let centroid = reads
                .geometry
                .centroids
                .get(&province)
                .ok_or("state Air forecast province centroid unavailable")?;
            let forecast = replay_data::day_night::ProvinceClock::new(
                *centroid,
                [reads.geometry.width, reads.geometry.height],
                defines,
            )
            .forecast(direction, feather);
            for (sum, value) in sums.iter_mut().zip(forecast) {
                *sum = sum.wrapping_add(value);
            }
        }
        Ok(sums
            .into_iter()
            .filter(|&sum| region_average(sum, members.len()) > threshold)
            .count() as u8)
    }
    pub fn pool(
        &mut self,
        base: &AirBaseRow,
        row: &AirBaseRegistrationRow,
        pool: &CarrierAirPool,
        maximum: bool,
    ) -> Result<i64, String> {
        let start = pool.wing_start as usize;
        let end = start
            .checked_add(pool.wing_count as usize)
            .ok_or("state Air pool boundary overflow")?;
        let members = row
            .wings
            .get(start..end)
            .ok_or("state Air pool membership outside registration")?;
        if members.is_empty() {
            return Ok(0);
        }
        let max_cost = maximum
            .then(|| self.defines()?.maximum_cost())
            .transpose()?;
        members.iter().try_fold(0_i64, |sum, &id| {
            let wing = carrier_summary::wing(self.context, id)?;
            let term = if let Some(max_cost) = max_cost {
                maximum_term(
                    wing.planes,
                    self.modified(wing)?,
                    *self.defines()?.weight.as_ref().map_err(Clone::clone)?,
                    max_cost,
                )
            } else {
                let Some(cost) = self.current_cost(wing, base, pool)? else {
                    return Ok(sum);
                };
                let amount = current_term(
                    self.modified(wing)?,
                    *self.defines()?.weight.as_ref().map_err(Clone::clone)?,
                    cost,
                );
                let mode = wing
                    .fuel_context_mode
                    .require("AirWings.fuel_context_mode")?
                    .and_then(|mode| u8::try_from(mode).ok())
                    .ok_or("state Air fuel context mode unavailable")?;
                match wing.mission_region {
                    None => amount,
                    Some(_) if mode == 2 => amount,
                    Some(region) => period_term(amount, mode, self.region_nights(region)?),
                }
            };
            Ok(sum.wrapping_add(term))
        })
    }
}

fn modified_base(base: i64, ace: i64, country: i64) -> i64 {
    ONE.wrapping_add(ace)
        .wrapping_add(country)
        .max(0)
        .wrapping_mul(base)
        / ONE
}
fn current_term(modified: i64, weight: i64, cost: i64) -> i64 {
    (modified.wrapping_mul(weight) / ONE).wrapping_mul(cost) / ONE
}
fn maximum_term(planes: i32, modified: i64, weight: i64, cost: i64) -> i64 {
    let amount = i64::from(planes).wrapping_mul(modified).wrapping_mul(ONE) / ONE;
    (amount.wrapping_mul(weight) / ONE).wrapping_mul(cost) / ONE
}
fn capacity_gate(planes: i32, capacity: i32, threshold: i64) -> bool {
    let denominator = i64::from(capacity).wrapping_mul(ONE);
    if denominator == 0 {
        threshold < ONE + 1
    } else {
        threshold <= i64::from(planes).wrapping_mul(10_000_000_000) / denominator
    }
}
fn region_average(sum: i64, count: usize) -> i64 {
    if count == 0 || !air_range::finite(sum) {
        SENTINEL
    } else {
        sum.wrapping_mul(ONE) / (count as i64).wrapping_mul(ONE)
    }
}
fn period_term(amount: i64, mode: u8, nights: u8) -> i64 {
    let count = if mode == 0 {
        24 - i64::from(nights)
    } else {
        i64::from(nights)
    };
    (count.wrapping_mul(10_000_000_000) / 2_400_000).wrapping_mul(amount) / ONE
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fuel_keeps_native_signed_wrapping_and_separate_truncations() {
        assert_eq!(modified_base(100_001, -50_001, 2), 50_001);
        assert_eq!(modified_base(77, -100_001, 0), 0);
        assert_eq!(current_term(100_003, 35_000, 200_001), 70_002);
        assert_eq!(maximum_term(-3, 200_000, 100_000, 100_000), -600_000);
        assert_ne!(
            maximum_term(i32::MAX, i64::MAX, 100_000, 100_000),
            i64::from(i32::MAX).wrapping_mul(i64::MAX)
        );
    }
    #[test]
    fn zero_capacity_threshold_and_period_fractions_are_not_normalized() {
        assert!(capacity_gate(1, 0, 100_000));
        assert!(!capacity_gate(1, 0, 100_001));
        assert!(capacity_gate(3, 6, 50_000));
        assert!(!capacity_gate(3, 6, 50_001));
        assert_eq!(period_term(100_000, 0, 6), 75_000);
        assert_eq!(period_term(100_000, 1, 6), 25_000);
        assert_eq!(region_average(0, 0), SENTINEL);
        assert_eq!(region_average(i64::MIN, 2), SENTINEL);
    }
}
