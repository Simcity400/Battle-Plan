//! Registered air missions in the order the flying-aircraft map visits them.
//!
//! `BuildRegionalAircraftScenarios@14122EFD0` scans each region's
//! `CStrategicAir+0x98` mission vector in order. The sorted active-region IDs
//! at +0xC8 also include carrier combat regions from +0x140: a listed region
//! can have no ordinary registrations. `_AllActiveMissions` at +0xB0 counts
//! only the ordinary registrations, so a missed region cannot pass as empty.
//!
//! Native writer audit (a729, virtual addresses): RegisterMission@140C4E360
//! appends to +0x98[region] and +0xB0 and sorted-inserts +0xC8;
//! UnregisterMission@140C4E570 removes both registrations and erases +0xC8
//! when the ordinary regional count reaches zero. CAirMission::SetRegion
//! @140F70850 calls these writers for the old and new regions.
//! Mission load @140F6FBC0 uses SetRegion; wing teardown @140F45430 calls
//! the mission destructor @140F60460, which uses SetRegion(NULL).
//! TransferToBase @140C55170 changes pools then refreshes the mission through
//! @140F4F220; incompatible targets are unassigned through SetRegion(NULL).
//!
//! The carrier pass @140C56D40 is a separate writer: it clears +0x140's
//! regional counts (@140C56D70), then visits carrier hosts in naval combat.
//! For a wing with no assigned region or a zero mission mask (@140C56EA2),
//! it appends the embedded mission to +0x140[combat region] (@140C56F06)
//! and inserts that region into +0xC8 (@140C56F68-140C56FF2), without
//! writing +0x98 or +0xB0. It does not clear +0xC8 first, so a special-only
//! region can remain listed after its carrier combat ends. This is a stable
//! empty ordinary vector, not evidence of a missed registration.
//!
//! PtrVector::PushBack_stride8@140120090 also temporarily zeroes a vector's
//! count while reallocating. Capture at HourlyUpdate@1401DCEF0 entry joins
//! every native read before the trampoline; it does not retry or wait out a
//! writer. Invalid headers, unreadable entries and count mismatches still
//! refuse the census; dependents must not treat that failure as absence.
//! The strategic-air hourly pass @140C485F0 calls the carrier refresh through
//! @140C368F0 / @140C57F80. The tag command @14113E4B0 changes the local
//! player through @1401EE470 (+0x520/+0x524), not these registries: SetRegion
//! resolves ownership through the wing's pool, and this census visits every
//! country air object independently of the local player.

use crate::{
    Memory, ccountry,
    cpdx_array::Array,
    cstrategic_air,
    memory::{RecordField, RecordLayout, RecordSources},
    read_slice,
    walk::{Buffer, Outcome},
};

/// One wing from this hour's complete AirWings census, joined by its embedded
/// CAirMission address. The pointer is acquisition context, never recorded.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WingIdentity {
    pub mission_pointer: usize,
    pub pool: usize,
    pub wing: u64,
    pub owner: u32,
    pub region: Option<u32>,
    pub mission_mask: u32,
    pub air_base: u64,
    pub base_override: u64,
    pub country_override: i32,
    pub ace: u64,
    pub planes: i32,
}

impl WingIdentity {
    pub fn new(
        pointer: usize,
        pool: usize,
        row: &crate::cair_wing::Row,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            mission_pointer: pointer
                .checked_add(crate::cair_wing::MISSION)
                .ok_or("air mission address overflow")?,
            pool,
            wing: row.id,
            owner: row.owner,
            region: row.mission_region,
            mission_mask: row.mission_mask,
            air_base: row.air_base,
            base_override: row.base_override,
            country_override: row.country_override,
            ace: row.ace,
            planes: row.planes,
        })
    }

    /// `CAirWing::GetEffectiveCountryTag@140F45950`: a positive override wins.
    pub fn effective_country(&self) -> u32 {
        if self.country_override > 0 {
            self.country_override as u32
        } else {
            self.owner
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub wing: u64,
    pub ordinal: u32,
}

/// The first failed country's canonical slot and, when known, region ID.
/// Acquisition retains only numbers; conversion can name the failure without
/// allocating on the capture path or changing the replay row format.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FailureContext {
    pub country_slot: Option<u32>,
    pub region: Option<u32>,
}

const BATCH: usize = 64;
/// +0x98 outer header through the +0xC8 region header, one guarded copy.
const HEADERS: usize =
    cstrategic_air::ACTIVE_MISSION_REGIONS + 16 - cstrategic_air::REGIONAL_MISSIONS;

/// The headers of [`BATCH`] country air objects per gather.
const HEADER_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: cstrategic_air::REGIONAL_MISSIONS,
    destination_offset: 0,
    len: HEADERS,
}];
const HEADER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&HEADER_FIELDS, HEADERS).expect("valid layout");

fn header(bytes: &[u8; HEADERS], offset: usize, stride: usize) -> Option<Array> {
    let at = offset - cstrategic_air::REGIONAL_MISSIONS;
    Array::from_header(bytes[at..at + 16].try_into().ok()?, stride)
}

/// `wings` must be this hour's complete AirWings census sorted by mission
/// pointer. Every member must be a census wing of the root's country
/// assigned to that region.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    output: &mut [Row],
) -> Outcome {
    walk_with_diagnostic(
        memory,
        base,
        game_state,
        wings,
        output,
        &mut FailureContext::default(),
    )
}

/// The same census, retaining the first failure's location for diagnostics.
/// A complete or capacity-truncated walk leaves no failure context.
pub fn walk_with_diagnostic(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    output: &mut [Row],
    failure: &mut FailureContext,
) -> Outcome {
    walk_with_transport(
        memory,
        base,
        game_state,
        wings,
        output,
        failure,
        &mut NoTransport,
    )
}

/// One occurrence in the original regional mission-pointer pass.
pub struct MissionObservation<'a> {
    pub slot: u32,
    pub region: u32,
    pub ordinal: u32,
    pub pointer: usize,
    pub original: Result<&'a WingIdentity, &'static str>,
}

/// Observer of this exact original manager/header/mission-pointer pass.
/// It must memoize original failures and retire every native address before
/// the loan returns. Optional transport failures never change strict rows.
pub trait TransportObserver {
    fn slot(&mut self, _original: cstrategic_air::SlotObservation) {}
    fn root(&mut self, _slot: u32, _pointer: usize, _outer: Result<Array, &'static str>) {}
    fn region(&mut self, _slot: u32, _region: u32, _original: Result<Array, &'static str>) {}
    fn inactive_region(&mut self, slot: u32, region: u32, original: Result<Array, &'static str>) {
        self.region(slot, region, original);
    }
    fn mission<M: Memory>(
        &mut self,
        _memory: &M,
        _countries: &ccountry::Reader<'_, M>,
        _wings: &[WingIdentity],
        _mission: MissionObservation<'_>,
    ) {
    }
    fn complete_regions(&self, _slot: u32) -> bool {
        false
    }
    fn region_attempted(&self, _slot: u32, _region: u32) -> bool {
        false
    }
    fn retire(&mut self) {}
}

struct TransportRetirement<F: FnMut()>(F);
impl<F: FnMut()> Drop for TransportRetirement<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

struct NoTransport;
impl TransportObserver for NoTransport {}

/// Adds independent transport observations without a replacement census.
pub fn walk_with_transport(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    output: &mut [Row],
    failure: &mut FailureContext,
    observer: &mut impl TransportObserver,
) -> Outcome {
    walk_with_transport_buffer(
        memory,
        base,
        game_state,
        wings,
        Buffer::Fixed(output),
        failure,
        observer,
    )
}

/// Worker-owned rows grow from the same regional headers the census follows.
pub fn walk_with_transport_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    output: &mut Vec<Row>,
    failure: &mut FailureContext,
    observer: &mut impl TransportObserver,
) -> Outcome {
    walk_with_transport_buffer(
        memory,
        base,
        game_state,
        wings,
        Buffer::Growing(output),
        failure,
        observer,
    )
}

pub fn walk_with_diagnostic_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    output: &mut Vec<Row>,
    failure: &mut FailureContext,
) -> Outcome {
    walk_with_transport_growing(
        memory,
        base,
        game_state,
        wings,
        output,
        failure,
        &mut NoTransport,
    )
}

fn walk_with_transport_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    mut output: Buffer<'_, Row>,
    failure: &mut FailureContext,
    observer: &mut impl TransportObserver,
) -> Outcome {
    let observer = std::cell::RefCell::new(observer);
    *failure = FailureContext::default();
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        // Declared after the original reader: successful, failed and unwound
        // passes retire observer controls before that reader's loan ends.
        let _retirement = TransportRetirement(|| observer.borrow_mut().retire());
        let mut len = 0;
        let mut unread_rows = 0_u32;
        // Roots are visited in slot order and their headers gathered
        // [`BATCH`] at a time, then each is followed in that order.
        let mut roots = [(0_usize, 0_u32); BATCH];
        let mut pending = 0;
        let mut flush = |roots: &[(usize, u32)]| -> Result<(), &'static str> {
            let mut pointers = [0_usize; BATCH];
            for (pointer, &(address, _)) in pointers.iter_mut().zip(roots) {
                *pointer = address;
            }
            let mut headers = [[0_u8; HEADERS]; BATCH];
            let copied = memory.copy_records(
                RecordSources::Pointers(&pointers[..roots.len()]),
                &HEADER_LAYOUT,
                headers
                    .as_flattened_mut()
                    .get_mut(..roots.len() * HEADERS)
                    .unwrap(),
            );
            for (index, &(_, slot)) in roots.iter().enumerate() {
                let outer = if index < copied {
                    header(
                        &headers[index],
                        cstrategic_air::REGIONAL_MISSIONS,
                        crate::cpdx_array::STRIDE,
                    )
                    .ok_or("transport original air region domain invalid")
                } else {
                    Err("transport original air root headers unreadable")
                };
                observer.borrow_mut().root(slot, roots[index].0, outer);
            }
            if copied != roots.len() {
                failure.country_slot = roots.get(copied).map(|&(_, slot)| slot);
                return Err("regional mission headers unreadable");
            }
            for (bytes, &(_, canonical_slot)) in headers.iter().zip(roots) {
                root_missions(
                    memory,
                    &countries,
                    canonical_slot,
                    bytes,
                    wings,
                    &mut output,
                    &mut len,
                    &mut unread_rows,
                    failure,
                    &mut **observer.borrow_mut(),
                )?;
                // Active headers were acquired above in the original walk.
                // Only originally unattempted physical headers are borrowed.
                let outer = header(
                    bytes,
                    cstrategic_air::REGIONAL_MISSIONS,
                    crate::cpdx_array::STRIDE,
                )
                .expect("strict air root domain validated");
                if observer.borrow().complete_regions(canonical_slot) {
                    for region in 0..outer.count {
                        if observer.borrow().region_attempted(canonical_slot, region) {
                            continue;
                        }
                        let original = outer
                            .element(region, crate::cpdx_array::STRIDE)
                            .and_then(|at| Array::read(memory, at, 8))
                            .ok_or("transport original inactive air region header invalid");
                        observer
                            .borrow_mut()
                            .inactive_region(canonical_slot, region, original);
                    }
                }
                *failure = FailureContext::default();
            }
            Ok(())
        };
        cstrategic_air::visit_with_slots(
            memory,
            base,
            game_state,
            |root| {
                roots[pending] = (root.address, root.canonical_slot);
                pending += 1;
                if pending == BATCH {
                    flush(&roots)?;
                    pending = 0;
                }
                Ok(())
            },
            |original| observer.borrow_mut().slot(original),
        )?;
        flush(&roots[..pending])?;
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// One country air object's registrations from its gathered headers.
#[allow(clippy::too_many_arguments)]
fn root_missions<M: Memory>(
    memory: &M,
    countries: &ccountry::Reader<'_, M>,
    canonical_slot: u32,
    bytes: &[u8; HEADERS],
    wings: &[WingIdentity],
    output: &mut Buffer<'_, Row>,
    len: &mut usize,
    unread_rows: &mut u32,
    failure: &mut FailureContext,
    observer: &mut impl TransportObserver,
) -> Result<(), &'static str> {
    failure.country_slot = Some(canonical_slot);
    let (Some(outer), Some(all), Some(regions)) = (
        header(
            bytes,
            cstrategic_air::REGIONAL_MISSIONS,
            crate::cpdx_array::STRIDE,
        ),
        header(bytes, cstrategic_air::ALL_ACTIVE_MISSIONS, 8),
        header(bytes, cstrategic_air::ACTIVE_MISSION_REGIONS, 4),
    ) else {
        return Err("regional mission header invalid");
    };
    let mut registered = 0_u32;
    let mut ids = [0_u32; BATCH];
    let mut previous = None;
    for first in (0..regions.count).step_by(BATCH) {
        failure.region = None;
        let count = (regions.count - first).min(BATCH as u32);
        crate::cpdx_array::copy_u32s(
            memory,
            regions.slice(first, count, 4)?,
            &mut ids[..count as usize],
            "active mission regions unreadable",
        )?;
        for &region in &ids[..count as usize] {
            failure.region = Some(region);
            if previous.is_some_and(|last| region <= last) {
                return Err("active mission regions out of order");
            }
            previous = Some(region);
            let missions = outer
                .element(region, crate::cpdx_array::STRIDE)
                .and_then(|at| Array::read(memory, at, 8))
                .ok_or("active mission region vector missing or invalid");
            observer.region(canonical_slot, region, missions);
            let missions = missions?;
            // RegisterMission@140C4E360 appends every occurrence to this
            // regional vector; grow before visiting its original members.
            output.ensure(
                len.checked_add(missions.count as usize)
                    .ok_or("regional mission row count overflow")?,
            )?;
            registered = registered
                .checked_add(missions.count)
                .ok_or("regional mission count overflow")?;
            let mut pointers = [0_u8; BATCH * 8];
            for batch in (0..missions.count).step_by(BATCH) {
                let n = (missions.count - batch).min(BATCH as u32) as usize;
                let at = missions
                    .element(batch, 8)
                    .ok_or("regional mission address overflow")?;
                if !read_slice(memory, at, &mut pointers[..n * 8]) {
                    return Err("regional mission vector unreadable");
                }
                for (index, bytes) in pointers[..n * 8].chunks_exact(8).enumerate() {
                    let pointer = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                        .map_err(|_| "regional mission pointer overflow")?;
                    let wing = wings
                        .binary_search_by_key(&pointer, |wing| wing.mission_pointer)
                        .map(|index| &wings[index])
                        .map_err(|_| "registered mission missing from AirWings");
                    observer.mission(
                        memory,
                        countries,
                        wings,
                        MissionObservation {
                            slot: canonical_slot,
                            region,
                            ordinal: batch + index as u32,
                            pointer,
                            original: wing,
                        },
                    );
                    let wing = wing?;
                    if wing.region != Some(region)
                        || countries.link(wing.owner)? != Some(canonical_slot)
                    {
                        return Err("registered mission owner or region differs from AirWings");
                    }
                    if let Some(destination) = output.get_mut(*len) {
                        *destination = Row {
                            wing: wing.wing,
                            ordinal: batch + index as u32,
                        };
                        *len += 1;
                    } else {
                        *unread_rows = unread_rows
                            .checked_add(1)
                            .ok_or("regional mission row count overflow")?;
                    }
                }
            }
        }
    }
    failure.region = None;
    if registered != all.count {
        return Err("regional and all-active mission counts differ");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cair_wing, cstrategic_air_manager, memory::SliceMemory};

    const GS: usize = 0x1000;
    const COUNTRY: usize = 0x2000;
    const COUNTRIES: usize = 0x4000;
    const LINKS: usize = 0x4100;
    const MANAGER: usize = 0x5000;
    const OWNERS: usize = 0x6000;
    const OWNER: usize = 0x7000;
    const REGIONS: usize = 0x8000;
    const IDS: usize = 0x9000;
    const ALL: usize = 0xa000;
    const MEMBERS: usize = 0xb000;
    const WING_A: usize = 0xc000;
    const WING_B: usize = 0xd000;

    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put64(bytes, at, data as u64);
        put32(bytes, at + 8, count);
        put32(bytes, at + 12, count);
    }
    fn wing(pointer: usize, id: u64) -> WingIdentity {
        WingIdentity {
            mission_pointer: pointer + cair_wing::MISSION,
            wing: id,
            owner: 3,
            region: Some(5),
            ..Default::default()
        }
    }
    fn fixture() -> (Vec<u8>, [WingIdentity; 2]) {
        let mut bytes = vec![0; 0xe000];
        array(&mut bytes, GS + 0x310, COUNTRIES, 4);
        array(&mut bytes, GS + 0x340, LINKS, 4);
        put64(&mut bytes, COUNTRIES + 8, COUNTRY as u64);
        put32(&mut bytes, LINKS + 4, 1);
        put32(&mut bytes, LINKS + 12, 1);
        put64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA as u64);
        put32(&mut bytes, COUNTRY + 8, 3);
        put64(&mut bytes, GS + 0x690, MANAGER as u64);
        put64(
            &mut bytes,
            MANAGER,
            cstrategic_air_manager::VTABLE_RVA as u64,
        );
        array(&mut bytes, MANAGER + 0x30, OWNERS, 4);
        put64(&mut bytes, OWNERS + 8, OWNER as u64);
        put64(&mut bytes, OWNER, cstrategic_air::VTABLE_RVA as u64);
        put32(&mut bytes, OWNER + cstrategic_air::COUNTRY, 3);
        array(
            &mut bytes,
            OWNER + cstrategic_air::REGIONAL_MISSIONS,
            REGIONS,
            6,
        );
        array(
            &mut bytes,
            OWNER + cstrategic_air::ACTIVE_MISSION_REGIONS,
            IDS,
            1,
        );
        put32(&mut bytes, IDS, 5);
        array(
            &mut bytes,
            OWNER + cstrategic_air::ALL_ACTIVE_MISSIONS,
            ALL,
            2,
        );
        array(&mut bytes, REGIONS + 5 * 0x18, MEMBERS, 2);
        put64(&mut bytes, MEMBERS, (WING_B + cair_wing::MISSION) as u64);
        put64(
            &mut bytes,
            MEMBERS + 8,
            (WING_A + cair_wing::MISSION) as u64,
        );
        (bytes, [wing(WING_A, 101), wing(WING_B, 202)])
    }
    fn capture(bytes: &[u8], wings: &[WingIdentity], output: &mut [Row]) -> Outcome {
        walk(&SliceMemory { base: 0, bytes }, 0, GS, wings, output)
    }

    #[test]
    fn original_regional_headers_grow_first_and_later_mod_membership() {
        let (mut bytes, _) = fixture();
        let mut output = Vec::new();
        for count in [5_000u32, 12_345] {
            let members = bytes.len();
            bytes.resize(members + count as usize * 8, 0);
            let wings: Vec<_> = (0..count)
                .map(|index| wing(0x10_0000 + index as usize * 0x200, 100 + u64::from(index)))
                .collect();
            for (index, wing) in wings.iter().enumerate() {
                put64(&mut bytes, members + index * 8, wing.mission_pointer as u64);
            }
            array(&mut bytes, REGIONS + 5 * 0x18, members, count);
            array(
                &mut bytes,
                OWNER + cstrategic_air::ALL_ACTIVE_MISSIONS,
                members,
                count,
            );
            let result = walk_with_diagnostic_growing(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                GS,
                &wings,
                &mut output,
                &mut Default::default(),
            );
            assert_eq!(
                result,
                Outcome::Rows {
                    len: count as usize
                }
            );
            assert_eq!(
                output[count as usize - 1],
                Row {
                    wing: 99 + u64::from(count),
                    ordinal: count - 1
                }
            );
            let mut fixed = [Row::default()];
            assert_eq!(
                capture(&bytes, &wings, &mut fixed),
                Outcome::Truncated {
                    len: 1,
                    unread_rows: count - 1
                }
            );
        }
    }

    #[test]
    fn registration_order_is_the_regional_vector_order() {
        let (bytes, wings) = fixture();
        let mut rows = [Row::default(); 2];
        assert_eq!(capture(&bytes, &wings, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(
            rows,
            [
                Row {
                    wing: 202,
                    ordinal: 0
                },
                Row {
                    wing: 101,
                    ordinal: 1
                }
            ]
        );
        assert_eq!(
            capture(&bytes, &wings, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn a_listed_empty_carrier_region_preserves_ordinary_registrations() {
        let (mut bytes, wings) = fixture();
        // Carrier combat lists region 2, whose ordinary vector is empty.
        // Exercise both a never-allocated vector and retained empty storage.
        array(
            &mut bytes,
            OWNER + cstrategic_air::ACTIVE_MISSION_REGIONS,
            IDS,
            2,
        );
        put32(&mut bytes, IDS, 2);
        put32(&mut bytes, IDS + 4, 5);
        for data in [0, MEMBERS + 0x100] {
            array(&mut bytes, REGIONS + 2 * 0x18, data, 0);
            if data != 0 {
                put32(&mut bytes, REGIONS + 2 * 0x18 + 8, 2);
            }
            let mut rows = [Row::default(); 2];
            assert_eq!(capture(&bytes, &wings, &mut rows), Outcome::Rows { len: 2 });
            assert_eq!(
                rows[0],
                Row {
                    wing: 202,
                    ordinal: 0
                }
            );
            assert_eq!(
                rows[1],
                Row {
                    wing: 101,
                    ordinal: 1
                }
            );
        }
    }

    #[test]
    fn a_carrier_only_region_has_zero_ordinary_registrations() {
        let (mut bytes, mut wings) = fixture();
        for wing in &mut wings {
            wing.region = None;
        }
        array(&mut bytes, REGIONS + 5 * 0x18, 0, 0);
        array(
            &mut bytes,
            OWNER + cstrategic_air::ALL_ACTIVE_MISSIONS,
            0,
            0,
        );
        let mut rows = [Row::default(); 2];
        assert_eq!(capture(&bytes, &wings, &mut rows), Outcome::Rows { len: 0 });
        // An empty vector cannot hide registrations counted by all-active.
        array(
            &mut bytes,
            OWNER + cstrategic_air::ALL_ACTIVE_MISSIONS,
            ALL,
            2,
        );
        assert_eq!(
            capture(&bytes, &wings, &mut rows),
            Outcome::Unavailable {
                reason: "regional and all-active mission counts differ"
            }
        );
    }

    #[test]
    fn removing_the_last_ordinary_region_leaves_a_complete_empty_census() {
        let (mut bytes, mut wings) = fixture();
        for wing in &mut wings {
            wing.region = None;
        }
        // UnregisterMission removes the region after its last registration.
        array(&mut bytes, REGIONS + 5 * 0x18, MEMBERS, 0);
        put32(&mut bytes, REGIONS + 5 * 0x18 + 8, 2);
        array(
            &mut bytes,
            OWNER + cstrategic_air::ACTIVE_MISSION_REGIONS,
            IDS,
            0,
        );
        array(
            &mut bytes,
            OWNER + cstrategic_air::ALL_ACTIVE_MISSIONS,
            ALL,
            0,
        );
        assert_eq!(capture(&bytes, &wings, &mut []), Outcome::Rows { len: 0 });
    }

    #[test]
    fn invalid_region_headers_keep_the_country_and_region_in_the_diagnostic() {
        let (mut bytes, wings) = fixture();
        let mut rows = [Row::default(); 2];
        let mut failure = FailureContext::default();
        // The region exists, but its count exceeds its capacity.
        put32(&mut bytes, REGIONS + 5 * 0x18 + 8, 1);
        assert_eq!(
            walk_with_diagnostic(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                GS,
                &wings,
                &mut rows,
                &mut failure,
            ),
            Outcome::Unavailable {
                reason: "active mission region vector missing or invalid"
            }
        );
        assert_eq!(
            failure,
            FailureContext {
                country_slot: Some(1),
                region: Some(5)
            }
        );
        put32(&mut bytes, REGIONS + 5 * 0x18 + 8, 2);
        assert_eq!(
            walk_with_diagnostic(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                GS,
                &wings,
                &mut rows,
                &mut failure,
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(failure, FailureContext::default());
    }

    #[test]
    fn a_missing_region_or_unknown_member_is_unavailable() {
        let (mut bytes, wings) = fixture();
        let mut rows = [Row::default(); 2];
        // The all-active count still holds both registrations.
        put32(
            &mut bytes,
            OWNER + cstrategic_air::ACTIVE_MISSION_REGIONS + 12,
            0,
        );
        assert!(matches!(
            capture(&bytes, &wings, &mut rows),
            Outcome::Unavailable { .. }
        ));
        let (mut bytes, wings) = fixture();
        put64(&mut bytes, MEMBERS, 0xdead_beef);
        assert!(matches!(
            capture(&bytes, &wings, &mut rows),
            Outcome::Unavailable { .. }
        ));
        let (bytes, mut wings) = fixture();
        wings[0].region = Some(6);
        assert!(matches!(
            capture(&bytes, &wings, &mut rows),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn effective_country_prefers_a_positive_override() {
        let mut identity = wing(WING_A, 1);
        assert_eq!(identity.effective_country(), 3);
        identity.country_override = 9;
        assert_eq!(identity.effective_country(), 9);
        identity.country_override = -1;
        assert_eq!(identity.effective_country(), 3);
    }

    #[test]
    fn transport_controls_retire_before_original_success_and_failure_returns() {
        #[derive(Default)]
        struct Observer {
            roots: usize,
            retired: bool,
        }
        impl TransportObserver for Observer {
            fn root(&mut self, _: u32, _: usize, _: Result<Array, &'static str>) {
                assert!(!self.retired);
                self.roots += 1;
            }
            fn retire(&mut self) {
                self.retired = true;
            }
        }
        for fail in [false, true] {
            let (mut bytes, wings) = fixture();
            if fail {
                put64(&mut bytes, MEMBERS, 0xdead_beef);
            }
            let mut observer = Observer::default();
            let result = walk_with_transport(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                GS,
                &wings,
                &mut [Row::default(); 2],
                &mut FailureContext::default(),
                &mut observer,
            );
            assert_eq!(matches!(result, Outcome::Unavailable { .. }), fail);
            assert_eq!(observer.roots, 1);
            assert!(observer.retired);
        }
    }
}
