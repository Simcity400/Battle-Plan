//! F97BF0 current radar/intel/weather operands from original callback loans.
//!
//! 141091B70 indexes CRadarsPool+10 by region+60 only when signed+1C !=0.
//! CEE3A0 reads reciprocal raw NAVY lanes; it resolves each nonzero intel owner
//! before checking its signed positivity. No displayed or saved cryptology
//! value substitutes for those current raw operands. F04E40's null result is
//! distinct from an unreadable weather record; present key2A uses 551820.
//!
//! Country, region and weather roots are borrowed from their original owners.
//! Array selectors and every failed attempt belong to this callback only.
//! CCountryIntel ctor CED590 stores owner+8 and initializes vector+10; CF2AE0
//! writes its complete positional raw vector. ResetCEE9B0 calls CF1D50 to
//! clear/free +10; daily1D2FA0 commits through CF1F40 (reset+1C, resize via
//! 1A8420, memcpy stride20), reached from daily1D4390. Load CF1C70 token30EC
//! calls CEB3B0/CEAD70 positional rows. DestructorCED910 frees +10/count.
//! Save CF2AE0 is a pure list write, with no lazy mutation. CCountry ctor6BD180 owns its
//! pointer at+FE8. Radar ctor108DB50 owns the embedded pool and vector+10;
//! 1091B70 consumes it independently of the coverage-group vector+28. Derived
//! reset6F1670 calls clear108F360/create1093140; create resizes +10 from live
//! gamestate+2EC and folds via1093880. That writer clears all stored counts
//! then accumulates group regional counts by original +60. Its incremental
//! callers are controller1093AC0 and building1093790. Destructor108DF40
//! clears/releases the same +10 array. Region
//! ctor E0B040 initializes +18/+60; AddProvince E0F2B0 appends +18 and sets
//! the province's region pointer. Region destructor E0B6E0 retires the vector.
//! Weather F0CF40 clears/folds regional+58 through54B860/54AD00. The outer
//! 1DEF80 and weather F03940 joins must precede capture. These source laws do
//! not independently prove exhaustiveness of every native mutator or actual
//! load/hook quiescence; that requires the coordinator's native witnesses.
use crate::{
    Memory, ccountry, ccountry_intel, country_intel_row, cprovince, cradars_pool,
    cstrategic_region, cweather_manager, modifier_accumulator, modifier_definition, read,
    sweather_per_region,
};

pub type Fact<T> = Result<T, &'static str>;
/// Original signed radar array, not CRadarsPool's coverage-group vector.
pub const RADAR_COUNTS: usize = 0x10;
pub const RADAR_COUNT: usize = 0x1c;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawArray {
    pub signed_count: i32,
    pub data: Fact<usize>,
}
#[derive(Clone, Copy, Debug)]
pub struct Intel {
    pub owner_raw: Fact<u32>,
    pub owner_slot: Fact<Option<u32>>,
    pub array: Fact<RawArray>,
}
#[derive(Clone, Copy, Debug)]
pub struct Owner {
    pub owner_slot: u32,
    pub owner_raw: u32,
    pub owned_states: Fact<i32>,
    pub radar: Fact<RawArray>,
    pub intel: Fact<Intel>,
}

fn address(object: usize, offset: usize) -> Fact<usize> {
    object
        .checked_add(offset)
        .filter(|_| object != 0)
        .ok_or("transport spotting source address invalid")
}
fn field<T: crate::memory::Raw>(memory: &impl Memory, object: usize, offset: usize) -> Fact<T> {
    read(memory, address(object, offset)?).ok_or("transport spotting source field unreadable")
}
fn authenticate(memory: &impl Memory, base: usize, object: usize, rva: usize) -> Fact<()> {
    let expected = base
        .checked_add(rva)
        .ok_or("transport spotting vtable address overflow")?;
    if field::<u64>(memory, object, 0)? != expected as u64 {
        return Err("transport spotting source vtable mismatch");
    }
    Ok(())
}
/// Preserve a readable signed count even if its positive body is unsupported.
/// Native zero does not demand data/capacity. Intel nonpositive extents cannot
/// reach a lane; radar's negative extent remains explicitly unsupported.
fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
    allow_negative: bool,
) -> Fact<RawArray> {
    let root = address(object, offset)?;
    let count = field::<i32>(memory, root, 12)?;
    let data = if count == 0 || (allow_negative && count < 0) {
        Ok(0)
    } else if count < 0 {
        Err("transport spotting radar count negative")
    } else {
        (|| {
            let capacity = field::<i32>(memory, root, 8)?;
            if capacity < count {
                return Err("transport spotting array count exceeds capacity");
            }
            let data = usize::try_from(field::<u64>(memory, root, 0)?)
                .map_err(|_| "transport spotting array pointer invalid")?;
            let bytes = (count as usize)
                .checked_mul(stride)
                .ok_or("transport spotting array extent overflow")?;
            if data == 0 || data.checked_add(bytes).is_none() {
                return Err("transport spotting array body invalid");
            }
            Ok(data)
        })()
    };
    Ok(RawArray {
        signed_count: count,
        data,
    })
}

impl Owner {
    /// The original country reader already authenticated the root and stored
    /// country id. No country root/identity/count selector is repeated here.
    pub fn capture_authenticated<M: Memory>(
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        country: ccountry::Country,
    ) -> Self {
        Self::capture_inner(memory, base, countries, country, None)
    }
    /// Reuse the original reserved naming count attempt, including refusal.
    pub fn capture_with_owned_states<M: Memory>(
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        country: ccountry::Country,
        original: Fact<i32>,
    ) -> Self {
        Self::capture_inner(memory, base, countries, country, Some(original))
    }
    fn capture_inner<M: Memory>(
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        country: ccountry::Country,
        original_owned_states: Option<Fact<i32>>,
    ) -> Self {
        let owned_states = original_owned_states
            .unwrap_or_else(|| field(memory, country.pointer, ccountry::OWNED_STATE_COUNT));
        let radar = (|| {
            let pool = address(country.pointer, ccountry::RADARS_POOL)?;
            authenticate(memory, base, pool, cradars_pool::VTABLE_RVA)?;
            authenticate(
                memory,
                base,
                address(pool, 8)?,
                cradars_pool::SECONDARY_VTABLE_RVA,
            )?;
            // Ctor14108DB50 retains the original tag at +40. Dynamic country
            // allocation1401DA850/140700330 changes only CCountry+8;
            // getter141091B70 consumes this embedded array without comparing tags.
            array(memory, pool, RADAR_COUNTS, 4, false)
        })();
        let intel = (|| {
            let root = usize::try_from(field::<u64>(
                memory,
                country.pointer,
                ccountry::INTEL_TABLE,
            )?)
            .map_err(|_| "transport spotting intel pointer invalid")?;
            authenticate(memory, base, root, ccountry_intel::VTABLE_RVA)?;
            // Owner and vector observations remain independent within one root.
            let owner_raw = field::<u32>(memory, root, ccountry_intel::OWNER);
            let owner_slot = owner_raw.and_then(|raw| {
                if raw == 0 {
                    return Ok(None);
                }
                match countries.link(raw)? {
                    Some(slot) => Ok(Some(slot)),
                    // BA6240 returns the original readable translation word.
                    // CEE3A0 indexes lane0 directly; it does not resolve a
                    // fallback object through BA56A0 at this inner step.
                    None => Ok(Some(0)),
                }
            });
            let array = array(
                memory,
                root,
                ccountry_intel::DATA,
                country_intel_row::STRIDE,
                true,
            );
            Ok(Intel {
                owner_raw,
                owner_slot,
                array,
            })
        })();
        Self {
            owner_slot: country.canonical_slot,
            owner_raw: country.id,
            owned_states,
            radar,
            intel,
        }
    }
}

pub fn copy_radar(memory: &impl Memory, original: RawArray, output: &mut [i32]) -> Fact<()> {
    if original.signed_count < 0 {
        return Err("transport spotting radar count negative");
    }
    if output.len() != original.signed_count as usize {
        return Err("transport spotting radar body length mismatch");
    }
    if output.is_empty() {
        return Ok(());
    }
    let data = original.data?;
    let bytes = output
        .len()
        .checked_mul(4)
        .ok_or("transport spotting radar extent overflow")?;
    // SAFETY: i32 has no padding/invalid patterns; output is initialized and
    // writable. A partial guarded copy is never admitted by the caller.
    let destination =
        unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr().cast::<u8>(), bytes) };
    if !crate::read_slice(memory, data, destination) {
        return Err("transport spotting original radar body unreadable");
    }
    Ok(())
}

pub fn copy_intel(memory: &impl Memory, original: RawArray, output: &mut [i64]) -> Fact<()> {
    if output.len() != original.signed_count.max(0) as usize {
        return Err("transport spotting intel body length mismatch");
    }
    if output.is_empty() {
        return Ok(());
    }
    let data = original.data?;
    // A fixed batch is stack scratch, not a content limit. Copy the original
    // positional array in contiguous chunks, retaining every raw navy lane.
    const BATCH: usize = 64;
    let mut scratch = [0_u8; BATCH * country_intel_row::STRIDE];
    for (chunk, target) in output.chunks_mut(BATCH).enumerate() {
        let offset = chunk
            .checked_mul(BATCH)
            .and_then(|n| n.checked_mul(country_intel_row::STRIDE))
            .ok_or("transport spotting intel offset overflow")?;
        let pointer = data
            .checked_add(offset)
            .ok_or("transport spotting intel address overflow")?;
        let bytes = target.len() * country_intel_row::STRIDE;
        if !crate::read_slice(memory, pointer, &mut scratch[..bytes]) {
            return Err("transport spotting original intel body unreadable");
        }
        for (value, row) in target
            .iter_mut()
            .zip(scratch[..bytes].chunks_exact(country_intel_row::STRIDE))
        {
            *value = i64::from_le_bytes(
                row[country_intel_row::NAVY..country_intel_row::NAVY + 8]
                    .try_into()
                    .unwrap(),
            );
        }
    }
    Ok(())
}

/// Ephemeral original region-header loan. The selecting owner authenticates
/// vtable/id first; this helper reads +18/+24/+60 once, preserving faults.
#[derive(Clone, Copy, Debug)]
pub struct RegionOriginal {
    pub pointer: usize,
    pub id: u32,
    pub runtime_slot: Fact<i32>,
    pub provinces: Fact<RawArray>,
}
impl RegionOriginal {
    pub fn capture_authenticated(memory: &impl Memory, pointer: usize, id: u32) -> Self {
        Self {
            pointer,
            id,
            runtime_slot: field(memory, pointer, cstrategic_region::RUNTIME_SLOT),
            provinces: array(memory, pointer, cstrategic_region::PROVINCES, 8, false),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub id: u32,
    pub runtime_slot: Fact<i32>,
    pub province_count: Fact<i32>,
    pub sampled_province: Fact<u32>,
    pub weather_detection: Fact<Option<i64>>,
}
#[derive(Default)]
pub struct WeatherMetadata {
    operands: Option<Fact<modifier_definition::Operands>>,
}
impl WeatherMetadata {
    pub fn clear(&mut self) {
        self.operands = None;
    }
    pub fn detection(
        &mut self,
        memory: &impl Memory,
        base: usize,
        record: Fact<usize>,
    ) -> Fact<Option<i64>> {
        record.and_then(|record| {
            if record == 0 {
                return Ok(None);
            }
            self.value(
                memory,
                base,
                address(record, sweather_per_region::MODIFIERS)?,
            )
            .map(Some)
        })
    }
    fn value(&mut self, memory: &impl Memory, base: usize, root: usize) -> Fact<i64> {
        let value =
            modifier_accumulator::lookup_at(memory, root, crate::modifier_keys::NAVAL_DETECTION)?;
        if !value.present {
            return Ok(0);
        }
        let operands = *self.operands.get_or_insert_with(|| {
            modifier_definition::operands(memory, base, crate::modifier_keys::NAVAL_DETECTION)
        });
        Ok(operands?.effective(value.raw))
    }
}

fn sampled_province(
    memory: &impl Memory,
    base: usize,
    hour: i32,
    original: RegionOriginal,
) -> Fact<u32> {
    let provinces = original.provinces?;
    let count = provinces.signed_count;
    if count <= 0 {
        return Err("transport spotting sampled region province extent nonpositive");
    }
    let offset = hour.wrapping_sub(43_800_000) % count;
    if offset < 0 {
        return Err("transport spotting sampled province signed index negative");
    }
    let pointer = provinces
        .data?
        .checked_add(
            (offset as usize)
                .checked_mul(8)
                .ok_or("transport spotting province index overflow")?,
        )
        .ok_or("transport spotting province address overflow")?;
    let province = usize::try_from(
        read::<u64>(memory, pointer)
            .ok_or("transport spotting sampled province pointer unreadable")?,
    )
    .map_err(|_| "transport spotting sampled province pointer invalid")?;
    authenticate(memory, base, province, cprovince::VTABLE_RVA)?;
    if field::<u64>(memory, province, cprovince::STRATEGIC_REGION)? != original.pointer as u64 {
        return Err("transport spotting sampled province region mismatch");
    }
    field(memory, province, cprovince::ID)
}

pub fn capture_region<M: Memory>(
    memory: &M,
    base: usize,
    hour: i32,
    original: RegionOriginal,
    weather: Fact<&cweather_manager::Reader<'_, M>>,
    metadata: &mut WeatherMetadata,
) -> Region {
    capture_region_observed(memory, base, hour, original, weather, metadata).0
}

/// Loan the SAME original nullable weather selector to other independent
/// regional operands before its callback lifetime ends. No selector retry.
pub fn capture_region_observed<M: Memory>(
    memory: &M,
    base: usize,
    hour: i32,
    original: RegionOriginal,
    weather: Fact<&cweather_manager::Reader<'_, M>>,
    metadata: &mut WeatherMetadata,
) -> (Region, Fact<usize>) {
    capture_region_with_weather(memory, base, hour, original, weather, |_, _, record| {
        (metadata.detection(memory, base, record), record)
    })
}

/// The original selected record/sample loan can be consumed by a joint
/// weather-key producer. Its key2A failure/success is the detection fact; no
/// separate lookup/header exists to repair a failed original observation.
pub fn capture_region_with_weather<M: Memory, T>(
    memory: &M,
    base: usize,
    hour: i32,
    original: RegionOriginal,
    weather: Fact<&cweather_manager::Reader<'_, M>>,
    observer: impl FnMut(Fact<u32>, u32, Fact<usize>) -> (Fact<Option<i64>>, T),
) -> (Region, T) {
    let weather_record = weather.and_then(|weather| weather.region_record(original.id));
    capture_region_with_record(memory, base, hour, original, weather_record, observer)
}

/// Consume an original weather record attempt already selected by its owning
/// mission pass. Both native absence and selection failure are borrowed exactly;
/// this entry point performs no manager, array or weather-record identity read.
pub fn capture_region_with_record<M: Memory, T>(
    memory: &M,
    base: usize,
    hour: i32,
    original: RegionOriginal,
    weather_record: Fact<usize>,
    mut observer: impl FnMut(Fact<u32>, u32, Fact<usize>) -> (Fact<Option<i64>>, T),
) -> (Region, T) {
    let sampled_province = sampled_province(memory, base, hour, original);
    let (weather_detection, result) = observer(sampled_province, original.id, weather_record);
    (
        Region {
            id: original.id,
            runtime_slot: original.runtime_slot,
            province_count: original.provinces.map(|array| array.signed_count),
            sampled_province,
            weather_detection,
        },
        result,
    )
}

#[cfg(test)]
#[path = "transport_spotting_context_tests.rs"]
mod tests;
