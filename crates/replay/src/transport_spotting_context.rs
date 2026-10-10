//! Lazy current-hour spotting context admission and reciprocal raw intel lanes.
//! The caller supplies the recording's authenticated current raw-country link
//! mapping. Viewer selection never becomes a recorder query or native read.
use crate::rows::{
    TransportAirEnvironment, TransportNavyIntel, TransportSpottingContext, TransportSpottingOwner,
    TransportSpottingRegion,
};
pub type Fact<T> = Result<T, &'static str>;

#[derive(Clone, Copy)]
pub struct View<'a> {
    packet: &'a TransportSpottingContext,
}
impl<'a> View<'a> {
    pub fn new(packet: Option<&'a TransportSpottingContext>, hour: i32) -> Fact<Self> {
        let packet = packet.ok_or("selected transport current spotting context unavailable")?;
        if packet.sample_hour != hour {
            return Err("selected transport spotting context is stale");
        }
        Ok(Self { packet })
    }
    pub fn owner(
        &self,
        raw: u32,
        resolve: &mut impl FnMut(u32) -> Fact<u32>,
    ) -> Fact<&'a TransportSpottingOwner> {
        let slot = if raw == 0 { 0 } else { resolve(raw)? };
        if slot == 0 {
            let owner = self
                .packet
                .reserved
                .as_ref()
                .ok_or("selected transport authentic reserved spotting owner unavailable")?;
            if owner.owner_slot != 0 || owner.owner_raw_id != 0 {
                return Err("selected transport reserved spotting owner identity invalid");
            }
            return Ok(owner);
        }
        if !self.packet.owners_complete {
            return Err("selected transport original spotting owner domain incomplete");
        }
        let owners = &self.packet.owners;
        if owners.iter().any(|owner| owner.owner_slot == 0)
            || owners
                .windows(2)
                .any(|pair| pair[0].owner_slot >= pair[1].owner_slot)
        {
            return Err("selected transport original spotting owner domain invalid");
        }
        let index = owners
            .binary_search_by_key(&slot, |owner| owner.owner_slot)
            .map_err(|_| "selected transport resolved spotting owner absent")?;
        Ok(&owners[index])
    }
    pub fn region(&self, id: u32) -> Fact<&'a TransportSpottingRegion> {
        if !self.packet.regions_complete {
            return Err("selected transport original spotting region domain incomplete");
        }
        let regions = &self.packet.regions;
        if regions
            .windows(2)
            .any(|pair| pair[0].region_id >= pair[1].region_id)
        {
            return Err("selected transport original spotting region domain invalid");
        }
        let index = regions
            .binary_search_by_key(&id, |region| region.region_id)
            .map_err(|_| "selected transport original spotting region absent")?;
        Ok(&regions[index])
    }
    /// The raw radar zero gate skips the region runtime slot and array body.
    /// Strategic spotting still needs the stored signed province denominator.
    pub fn radar_counts(
        &self,
        owner: u32,
        region: u32,
        resolve: &mut impl FnMut(u32) -> Fact<u32>,
    ) -> Fact<(i32, i32)> {
        let radar = self
            .owner(owner, resolve)?
            .radar
            .as_ref()
            .ok_or("selected transport original radar header unavailable")?;
        let region = self.region(region)?;
        let provinces = region
            .signed_province_count
            .ok_or("selected transport original signed region province count unavailable")?;
        if radar.signed_count == 0 {
            return Ok((0, provinces));
        }
        if radar.signed_count < 0 {
            return Err("selected transport original radar extent negative");
        }
        let slot = region
            .runtime_slot
            .ok_or("selected transport original radar region runtime slot unavailable")?;
        if slot < 0 {
            return Err("selected transport radar runtime slot negative");
        }
        let values = radar
            .counts
            .as_ref()
            .ok_or("selected transport original radar body unavailable")?;
        if values.len() != radar.signed_count as usize {
            return Err("selected transport original radar body extent invalid");
        }
        Ok((
            *values
                .get(slot as usize)
                .ok_or("selected transport radar runtime slot outside original body")?,
            provinces,
        ))
    }
    /// A signed nonpositive target or target owned-state count below 1
    /// short-circuits before any reciprocal intel root/lane demand.
    pub fn navy_intel_difference(
        &self,
        owner: u32,
        target: u32,
        resolve: &mut impl FnMut(u32) -> Fact<u32>,
    ) -> Fact<i64> {
        if (target as i32) < 1 {
            return Ok(0);
        }
        let target = self.owner(target, resolve)?;
        let states = target
            .target_owned_states
            .ok_or("selected transport current target owned-state gate unavailable")?;
        if states < 1 {
            return Ok(0);
        }
        let target = target
            .navy_intel
            .as_ref()
            .ok_or("selected transport current target navy intel root unavailable")?;
        let source = self
            .owner(owner, resolve)?
            .navy_intel
            .as_ref()
            .ok_or("selected transport current owner navy intel root unavailable")?;
        let own = lane(source, target)?;
        let reciprocal = lane(target, source)?;
        Ok(own.wrapping_sub(reciprocal))
    }
    pub fn weather_detection(&self, region: u32) -> Fact<Option<i64>> {
        self.region(region)?
            .weather_detection
            .ok_or("selected transport original regional weather detection unavailable")
    }
    pub fn sampled_province(&self, region: u32) -> Fact<u32> {
        self.region(region)?
            .sampled_province
            .ok_or("selected transport original sampled province unavailable")
    }
    pub fn air_environment(&self, region: u32) -> Fact<TransportAirEnvironment> {
        self.region(region)?
            .air_environment
            .ok_or("selected transport original regional air environment unavailable")
    }
    pub fn runtime_slot(&self, region: u32) -> Fact<i32> {
        self.region(region)?
            .runtime_slot
            .ok_or("selected transport original region runtime slot unavailable")
    }
}

/// A nonzero raw owner always demands its original canonical translation,
/// including when its signed value is negative. The signed-positive gate is
/// then lazy: it skips the other vector's count and body for negative owners.
fn lane(values: &TransportNavyIntel, owner: &TransportNavyIntel) -> Fact<i64> {
    let raw = owner
        .owner_raw_id
        .ok_or("selected transport current intel raw owner unavailable")?;
    if raw == 0 {
        return Ok(0);
    }
    let slot = owner
        .owner_slot
        .ok_or("selected transport original intel canonical owner unavailable")?;
    if (raw as i32) < 1 {
        return Ok(0);
    }
    let count = values
        .signed_count
        .ok_or("selected transport current navy intel count unavailable")?;
    let signed_slot = slot as i32;
    if count <= signed_slot {
        return Ok(0);
    }
    if signed_slot < 0 {
        return Err("selected transport navy intel translated slot negative");
    }
    let lanes = values
        .lanes
        .as_ref()
        .ok_or("selected transport current raw navy intel body unavailable")?;
    if count < 0 || lanes.len() != count as usize {
        return Err("selected transport current navy intel body extent invalid");
    }
    lanes
        .get(slot as usize)
        .copied()
        .ok_or("selected transport raw navy intel lane outside original vector")
}

#[cfg(test)]
#[path = "transport_spotting_context_tests.rs"]
mod tests;
