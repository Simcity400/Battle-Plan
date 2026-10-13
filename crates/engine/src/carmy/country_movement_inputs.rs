//! Current-hour transport speeds shared by a moving country's units.
//!
//! Transport speed is calculated for the logical owner and the source strategic
//! region, before the caller's transport multiplier. The existing passive
//! reader owns the computation; this module only visits the complete
//! caller-supplied region domain.
//!
//! The region roster must come from this frozen acquisition's complete
//! authenticated map census. Keys are unique and increasing. Country owners are
//! authenticated positive objects. No selection, hover cache, game call or path
//! search is involved.

use crate::{
    Memory, ccountry, cgame_state, cnaval_unit_transfer, cpdx_array::Array, cstrategic_region, read,
};

mod demand;
pub use demand::{LiveDemand, MovingOwners, observe_live};

pub const TRANSPORT_CAPACITY: &str = "country movement transport arena capacity exhausted";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RegionTransportSpeed {
    pub region: u32,
    /// Raw fixed5 speed before the division's transport multiplier.
    pub speed: Option<i64>,
    /// This query's named refusal, independent of every other owner/region.
    pub failure: Option<&'static str>,
}

/// A region identity selected by this hour's authenticated map census.
/// The pointer is borrowed acquisition input and never enters an output row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportRegion {
    pub id: u32,
    pub pointer: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainCapture {
    /// Ok admits the complete keyed domain, including entry-local None values.
    /// Err exposes no usable prefix (invalid domain, owner or arena shortage).
    pub len: Result<usize, &'static str>,
    /// Full supplied demand survives all refusals, including a short arena.
    /// Map domain readers instead report the header's conservative slot bound,
    /// which includes nullable entries and repeated controller observations.
    pub required: usize,
    /// First native query refusal in domain order, independent of admission.
    pub first_failure: Option<&'static str>,
}

fn region_array(memory: &impl Memory, state: usize) -> Result<Array, &'static str> {
    state
        .checked_add(cgame_state::STRATEGIC_REGIONS)
        .filter(|_| state != 0)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("country movement region census unreadable or invalid")
}

/// Live header bound for sizing outside the acquisition callback: source
/// regions need at most one scratch entry per physical strategic-region slot.
pub fn region_demand(memory: &impl Memory, state: usize) -> Result<usize, &'static str> {
    region_array(memory, state).map(|array| array.count as usize)
}

/// Complete native source-region superset, including a live region in physical
/// slot zero. `movement_speed` admits id zero and authenticates runtime slots;
/// unlike the province census, this array has no imposed positive-slot rule.
/// Null entries are absent; repeated IDs or inconsistent runtime slots refuse
/// the domain. The caller clears these private pointers before ending its pass.
pub fn capture_region_domain(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [TransportRegion],
) -> DomainCapture {
    capture_region_domain_buffer(memory, base, state, crate::walk::Buffer::Fixed(output))
}

pub fn capture_region_domain_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<TransportRegion>,
) -> DomainCapture {
    capture_region_domain_buffer(memory, base, state, crate::walk::Buffer::Growing(output))
}

fn capture_region_domain_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: crate::walk::Buffer<'_, TransportRegion>,
) -> DomainCapture {
    let array = match region_array(memory, state) {
        Ok(array) => array,
        Err(reason) => return DomainCapture::refused(0, reason),
    };
    let required = array.count as usize;
    if let Err(reason) = output.ensure(required) {
        return DomainCapture::refused(required, reason);
    }
    if required > output.len() {
        return DomainCapture::refused(required, "country movement region-domain arena exhausted");
    }
    let mut written = 0;
    let result = (|| {
        crate::cpdx_array::pointers(
            memory,
            array,
            "country movement region pointer unreadable",
            |slot, pointer| {
                if pointer == 0 {
                    return Ok(());
                }
                let id = cstrategic_region::id(memory, base, pointer)?
                    .ok_or("country movement source region is null")?;
                let runtime_slot = read::<u32>(
                    memory,
                    super::movement_address(pointer, cstrategic_region::RUNTIME_SLOT)?,
                )
                .ok_or("country movement region runtime slot unreadable")?;
                if id > i32::MAX as u32 || runtime_slot != slot {
                    return Err("country movement region identity differs from census");
                }
                output[written] = TransportRegion { id, pointer };
                written += 1;
                Ok(())
            },
        )?;
        output[..written].sort_unstable_by_key(|region| region.id);
        if output[..written]
            .windows(2)
            .any(|pair| pair[0].id == pair[1].id)
        {
            return Err("country movement region IDs are not unique");
        }
        Ok(written)
    })();
    if result.is_err() {
        output[..written].fill(TransportRegion::default());
    }
    DomainCapture {
        len: result,
        required,
        first_failure: None,
    }
}

impl DomainCapture {
    fn refused(required: usize, reason: &'static str) -> Self {
        Self {
            len: Err(reason),
            required,
            first_failure: None,
        }
    }
}

/// Existing successful transport observations, valid only inside one frozen
/// acquisition. Reserve off-thread, clear before every hour and again before
/// its native pointers leave that acquisition. Cache saturation uses the
/// existing correct uncached path; it never clips a native enemy roster.
pub struct CountryMovementPacket {
    transport: cnaval_unit_transfer::MovementSpeeds,
    transport_failure: Option<&'static str>,
}

impl CountryMovementPacket {
    pub fn try_new(owner_capacity: usize) -> std::io::Result<Self> {
        Ok(Self {
            transport: cnaval_unit_transfer::MovementSpeeds::try_with_capacity(owner_capacity)?,
            transport_failure: None,
        })
    }

    pub fn vacant() -> Self {
        Self {
            transport: cnaval_unit_transfer::MovementSpeeds::vacant(),
            transport_failure: None,
        }
    }

    pub fn reserve(&mut self, owner_capacity: usize) -> std::io::Result<()> {
        self.transport.reserve(owner_capacity)
    }

    pub fn reserve_transport(&mut self, queries: usize, regions: usize) -> std::io::Result<()> {
        self.transport.reserve_queries(queries, regions)
    }

    pub fn clear(&mut self) {
        self.transport.clear();
        self.transport_failure = None;
    }

    /// First transport refusal in this acquisition, for aggregate diagnostics.
    /// Each queried row retains its own reason; this does not gate later rows.
    pub fn transport_failure(&self) -> Option<&'static str> {
        self.transport_failure
    }

    pub fn allocated_bytes(&self) -> usize {
        self.transport.allocated_bytes()
    }
}

fn positive_owner(country: ccountry::Country) -> Result<(), &'static str> {
    if country.pointer == 0 || country.id == 0 || country.canonical_slot == 0 {
        Err("country movement owner is not an authenticated positive country")
    } else {
        Ok(())
    }
}

/// Copies one logical owner's complete source-region domain. Native region
/// identity and the supplied key must agree; the existing transport reader
/// additionally checks the region's runtime census slot. A pre-query identity
/// fault refuses only that region. The transport-speed and regional-factor
/// calculations query the supplied region independently; the reader retains
/// each first result at its actual shared-control, owner, region or
/// regional-operand boundary. Capacity and domain checks precede native reads.
#[allow(clippy::too_many_arguments)]
pub fn capture_transport(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner: ccountry::Country,
    regions: &[TransportRegion],
    output: &mut [RegionTransportSpeed],
    packet: &mut CountryMovementPacket,
) -> DomainCapture {
    let required = regions.len();
    if let Err(reason) = positive_owner(owner) {
        return DomainCapture::refused(required, reason);
    }
    if regions.windows(2).any(|pair| pair[0].id >= pair[1].id) {
        return DomainCapture::refused(required, "country movement region domain not increasing");
    }
    if required > output.len() {
        return DomainCapture::refused(required, TRANSPORT_CAPACITY);
    }
    let mut captured = DomainCapture {
        len: Ok(required),
        required,
        first_failure: None,
    };
    // Every owner visits the same complete key domain once per pass; enemy
    // navy operands are copied once per navy instead of per owner and key.
    packet
        .transport
        .bind_domain(base, state, regions.iter().map(|region| region.id));
    for (position, (row, region)) in output[..required].iter_mut().zip(regions).enumerate() {
        *row = RegionTransportSpeed {
            region: region.id,
            speed: None,
            failure: None,
        };
        let value = packet.transport.get_in_domain(
            memory,
            base,
            state,
            owner.id,
            region.pointer,
            region.id,
            position,
        );
        match value {
            Ok(value) => row.speed = Some(value),
            Err(reason) => {
                row.failure = Some(reason);
                packet.transport_failure.get_or_insert(reason);
                captured.first_failure.get_or_insert(reason);
            }
        }
    }
    captured
}

#[cfg(test)]
mod tests;
