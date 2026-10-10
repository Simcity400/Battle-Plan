//! Current-hour movement coefficients shared by a moving country's divisions.
//!
//! `CArmy::GetMovementSpeed@140C70670` selects the land owner's relation
//! accumulator by the destination's canonical controller (including slot zero)
//! and reads the effective `RIVER_CROSSING_FACTOR_AGAINST`. Transport uses
//! `141955DF0` for the logical owner and the source strategic region, before
//! the caller's transport multiplier. The existing passive readers own both
//! computations; this module only visits complete caller-supplied domains.
//!
//! The controller and region rosters must come from this frozen acquisition's
//! complete authenticated map census. Keys are unique and increasing. Country
//! owners are authenticated positive objects; zero is admitted only as a river
//! target. No selection, hover cache, game call or path search is involved.

use crate::{
    Memory, ccountry, cgame_state, cmodifier, cnaval_unit_transfer, cpdx_array::Array, cprovince,
    crelation_status, cstrategic_region, modifier_accumulator, modifier_definition, read,
};

mod demand;
pub use demand::{LiveDemand, MovingOwners, observe_live};

pub const RIVER_CAPACITY: &str = "country movement river arena capacity exhausted";
pub const TRANSPORT_CAPACITY: &str = "country movement transport arena capacity exhausted";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ControllerRiverSpeed {
    pub target_slot: u32,
    /// None is this target's refused native query, never numerical zero.
    pub modifier: Option<i64>,
}

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub river: usize,
    pub transport: usize,
}

impl Demand {
    /// Complete owner-shared arena demand, independent of native query faults.
    pub fn for_owners(
        land_owners: usize,
        controllers: usize,
        logical_owners: usize,
        regions: usize,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            river: land_owners
                .checked_mul(controllers)
                .ok_or("country movement river demand overflow")?,
            transport: logical_owners
                .checked_mul(regions)
                .ok_or("country movement transport demand overflow")?,
        })
    }

    pub fn max(self, other: Self) -> Self {
        Self {
            river: self.river.max(other.river),
            transport: self.transport.max(other.transport),
        }
    }
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

/// Independent live header bounds for sizing outside the acquisition callback.
/// Controllers need at most one scratch entry per positive province slot;
/// source regions need at most one per physical strategic-region slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainDemand {
    pub controllers: Result<usize, &'static str>,
    pub regions: Result<usize, &'static str>,
}

fn province_array(memory: &impl Memory, state: usize) -> Result<Array, &'static str> {
    let array = state
        .checked_add(cgame_state::PROVINCES)
        .filter(|_| state != 0)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("country movement province census unreadable or invalid")?;
    // cprovince::walk_rows and StaticGeometry exclude the reserved id-zero
    // slot and refuse a census without positive province slots.
    if array.count < 2 {
        return Err("country movement province census has no positive slots");
    }
    Ok(array)
}

fn region_array(memory: &impl Memory, state: usize) -> Result<Array, &'static str> {
    state
        .checked_add(cgame_state::STRATEGIC_REGIONS)
        .filter(|_| state != 0)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("country movement region census unreadable or invalid")
}

pub fn domain_demand(memory: &impl Memory, state: usize) -> DomainDemand {
    DomainDemand {
        controllers: province_array(memory, state).map(|array| (array.count - 1) as usize),
        regions: region_array(memory, state).map(|array| array.count as usize),
    }
}

/// Complete current map-controller domain, acquired once before owner queries.
/// Raw controller references resolve through the same-hour country census;
/// zero selects the real relation matrix slot zero. A failed census supplies
/// no usable prefix. No state/fort/owner graph or persistent geometry is read.
pub fn capture_controller_domain<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, M>,
    output: &mut [u32],
) -> DomainCapture {
    capture_controller_domain_buffer(
        memory,
        base,
        state,
        countries,
        crate::walk::Buffer::Fixed(output),
    )
}

pub fn capture_controller_domain_growing<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, M>,
    output: &mut Vec<u32>,
) -> DomainCapture {
    capture_controller_domain_buffer(
        memory,
        base,
        state,
        countries,
        crate::walk::Buffer::Growing(output),
    )
}

fn capture_controller_domain_buffer<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, M>,
    mut output: crate::walk::Buffer<'_, u32>,
) -> DomainCapture {
    let array = match province_array(memory, state) {
        Ok(array) => array,
        Err(reason) => return DomainCapture::refused(0, reason),
    };
    let required = (array.count - 1) as usize;
    if let Err(reason) = output.ensure(required) {
        return DomainCapture::refused(required, reason);
    }
    if required > output.len() {
        return DomainCapture::refused(
            required,
            "country movement controller-domain arena exhausted",
        );
    }
    let mut written = 0;
    let result = (|| {
        let positive = array.slice(1, array.count - 1, 8)?;
        crate::cpdx_array::pointers(
            memory,
            positive,
            "country movement province pointer unreadable",
            |index, pointer| {
                if pointer == 0 {
                    return Ok(());
                }
                let id = index + 1;
                if crate::cunit::registered_province(memory, base, array, pointer)? != id {
                    return Err("country movement province identity mismatch");
                }
                let raw = read::<u32>(
                    memory,
                    super::movement_address(pointer, cprovince::CONTROLLER)?,
                )
                .ok_or("country movement province controller unreadable")?;
                output[written] = if raw == 0 {
                    0
                } else {
                    countries.resolve(raw)?.canonical_slot
                };
                written += 1;
                Ok(())
            },
        )?;
        output[..written].sort_unstable();
        let mut unique = 0;
        for index in 0..written {
            if unique == 0 || output[unique - 1] != output[index] {
                output[unique] = output[index];
                unique += 1;
            }
        }
        Ok(unique)
    })();
    if result.is_err() {
        output[..written].fill(0);
    }
    DomainCapture {
        len: result,
        required,
        first_failure: None,
    }
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
    river_definition: Option<Result<modifier_definition::Operands, &'static str>>,
}

impl CountryMovementPacket {
    pub fn try_new(owner_capacity: usize) -> std::io::Result<Self> {
        Ok(Self {
            transport: cnaval_unit_transfer::MovementSpeeds::try_with_capacity(owner_capacity)?,
            transport_failure: None,
            river_definition: None,
        })
    }

    pub fn vacant() -> Self {
        Self {
            transport: cnaval_unit_transfer::MovementSpeeds::vacant(),
            transport_failure: None,
            river_definition: None,
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
        self.river_definition = None;
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

/// Exact relation contribution shared with the stored-route ETA reader. The
/// caller authenticates the status, its owner and selected matrix target.
pub(super) fn river_modifier(
    memory: &impl Memory,
    base: usize,
    status: usize,
    definition: &mut Option<Result<modifier_definition::Operands, &'static str>>,
) -> Result<i64, &'static str> {
    if status == 0 {
        return Err("country movement river relation is null");
    }
    let accumulator =
        super::movement_address(status, crelation_status::MODIFIER + cmodifier::ACCUMULATOR)?;
    let key = crate::modifier_keys::RIVER_CROSSING_FACTOR_AGAINST;
    let value = modifier_accumulator::lookup_at(memory, accumulator, key)?;
    if !value.present {
        return Ok(0);
    }
    // The metadata is shared by this one key across all owner/target pairs.
    // Retain its first refusal too; a later target cannot retry a failed root.
    let operands =
        *definition.get_or_insert_with(|| modifier_definition::operands(memory, base, key));
    Ok(operands?.effective(value.raw))
}

/// Copies one land owner's complete river-controller domain into a bounded
/// arena tail. Read the diplomacy/matrix controls once, then query every entry
/// independently. A root failure gives every admitted key None without another
/// root attempt; a single status/modifier failure preserves later targets.
/// An admitted empty domain does not inspect the native owner graph.
pub fn capture_river<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    owner: ccountry::Country,
    controllers: &[u32],
    output: &mut [ControllerRiverSpeed],
    packet: &mut CountryMovementPacket,
) -> DomainCapture {
    let required = controllers.len();
    if let Err(reason) = positive_owner(owner) {
        return DomainCapture::refused(required, reason);
    }
    if controllers.windows(2).any(|pair| pair[0] >= pair[1]) {
        return DomainCapture::refused(
            required,
            "country movement controller domain not increasing",
        );
    }
    if required > output.len() {
        return DomainCapture::refused(required, RIVER_CAPACITY);
    }
    let output = &mut output[..required];
    for (row, &target_slot) in output.iter_mut().zip(controllers) {
        *row = ControllerRiverSpeed {
            target_slot,
            modifier: None,
        };
    }
    let mut captured = DomainCapture {
        len: Ok(required),
        required,
        first_failure: None,
    };
    if required == 0 {
        return captured;
    }
    // Reuse the relation reader's owner, status and target authentication.
    let relations = match crelation_status::MovementRelations::new(memory, base, owner) {
        Ok(relations) => relations,
        Err(reason) => {
            captured.first_failure = Some(reason);
            return captured;
        }
    };
    for row in output {
        let value = relations
            .status(memory, base, countries, row.target_slot)
            .and_then(|status| river_modifier(memory, base, status, &mut packet.river_definition));
        match value {
            Ok(value) => row.modifier = Some(value),
            Err(reason) => {
                captured.first_failure.get_or_insert(reason);
            }
        }
    }
    captured
}

/// Copies one logical owner's complete source-region domain. Native region
/// identity and the supplied key must agree; the existing transport reader
/// additionally checks the region's runtime census slot. A pre-query identity
/// fault refuses only that region. `141955DF0`/`141956170` query the supplied
/// region independently; the reader retains each first result at its actual
/// shared-control, owner, region or regional-operand boundary.
/// Capacity and domain checks precede native reads.
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
    for (row, region) in output[..required].iter_mut().zip(regions) {
        *row = RegionTransportSpeed {
            region: region.id,
            speed: None,
            failure: None,
        };
        let value = packet.transport.get_for_region(
            memory,
            base,
            state,
            owner.id,
            region.pointer,
            region.id,
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
