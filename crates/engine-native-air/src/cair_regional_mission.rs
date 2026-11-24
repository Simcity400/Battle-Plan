
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read_slice,
    walk::{Buffer, Outcome},
};
use engine_native::ccountry;
use crate::cstrategic_air;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WingIdentity {
    pub mission_pointer: usize,
    pub pool: usize,
    pub wing: u64,
    pub owner: u32,
    pub owner_pointer: usize,
    pub owner_slot: u32,
    pub region: Option<u32>,
    pub region_pointer: usize,
    pub mission_mask: u32,
    pub air_base: u64,
    pub base_override: u64,
    pub country_override: i32,
    pub ace: u64,
    pub planes: i32,
    pub eligibility_counter: i32,
    pub eligibility_limit: i32,
    pub fuel_inputs: Option<crate::cair_wing::fuel_inputs::Inputs>,
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
            eligibility_counter: row.eligibility_counter,
            eligibility_limit: row.eligibility_limit,
            owner: row.owner,
            owner_pointer: row.owner_pointer,
            owner_slot: row.owner_slot,
            region: row.mission_region,
            region_pointer: row.region_pointer,
            mission_mask: row.mission_mask,
            air_base: row.air_base,
            base_override: row.base_override,
            country_override: row.country_override,
            ace: row.ace,
            planes: row.planes,
            fuel_inputs: row.fuel_inputs,
        })
    }

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FailureContext {
    pub country_slot: Option<u32>,
    pub region: Option<u32>,
}

const BATCH: usize = 64;
pub const HEADERS: usize =
    cstrategic_air::ACTIVE_MISSION_REGIONS + 16 - cstrategic_air::REGIONAL_MISSIONS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Root {
    pub address: usize,
    pub canonical_slot: u32,
    pub headers: [u8; HEADERS],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Registration {
    pub region: u32,
    pub ordinal: u32,
    pub pointer: usize,
}

pub trait Carry {
    fn plan(&mut self, root: &Root) -> (usize, bool);
    fn held(&self, token: usize) -> &[Registration];
    fn carried(&mut self, token: usize);
    fn record(&mut self, token: usize, registration: Registration);
    fn read(&mut self, token: usize);
}

pub struct Fresh;
impl Carry for Fresh {
    fn plan(&mut self, _: &Root) -> (usize, bool) {
        (0, false)
    }
    fn held(&self, _: usize) -> &[Registration] {
        &[]
    }
    fn carried(&mut self, _: usize) {}
    fn record(&mut self, _: usize, _: Registration) {}
    fn read(&mut self, _: usize) {}
}

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

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    output: &mut Vec<Row>,
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

pub struct MissionObservation<'a> {
    pub slot: u32,
    pub region: u32,
    pub ordinal: u32,
    pub pointer: usize,
    pub original: Result<&'a WingIdentity, &'static str>,
}

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

pub fn walk_with_transport(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    output: &mut Vec<Row>,
    failure: &mut FailureContext,
    observer: &mut impl TransportObserver,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        wings,
        Buffer::Growing(output),
        failure,
        observer,
        &mut Fresh,
    )
}

pub fn walk_with_diagnostic(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    output: &mut Vec<Row>,
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

pub fn walk_carried(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    output: Buffer<'_, Row>,
    failure: &mut FailureContext,
    carry: &mut impl Carry,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        wings,
        output,
        failure,
        &mut NoTransport,
        carry,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the wings come from the caller, and the output, failure context, transport observer and carry are caller-owned"
)]
fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    wings: &[WingIdentity],
    mut output: Buffer<'_, Row>,
    failure: &mut FailureContext,
    observer: &mut impl TransportObserver,
    carry: &mut impl Carry,
) -> Outcome {
    let observer = std::cell::RefCell::new(observer);
    *failure = FailureContext::default();
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let _retirement = TransportRetirement(|| observer.borrow_mut().retire());
        let mut len = 0;
        let mut unread_rows = 0_u32;
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
                        engine_native_memory::cpdx_array::STRIDE,
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
            for (bytes, &(address, canonical_slot)) in headers.iter().zip(roots) {
                let (token, carryable) = carry.plan(&Root {
                    address,
                    canonical_slot,
                    headers: *bytes,
                });
                if carryable
                    && carried_missions(
                        &countries,
                        canonical_slot,
                        carry.held(token),
                        wings,
                        &mut output,
                        &mut len,
                        &mut unread_rows,
                    )
                {
                    carry.carried(token);
                    continue;
                }
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
                    &mut *carry,
                    token,
                )?;
                carry.read(token);
                let outer = header(
                    bytes,
                    cstrategic_air::REGIONAL_MISSIONS,
                    engine_native_memory::cpdx_array::STRIDE,
                )
                .expect("strict air root domain validated");
                if observer.borrow().complete_regions(canonical_slot) {
                    inactive_regions(memory, outer, canonical_slot, &mut **observer.borrow_mut());
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

const REGION_HEADER_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 16,
}];
const REGION_HEADER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&REGION_HEADER_FIELDS, 16).expect("air region header layout");

fn inactive_regions(
    memory: &impl Memory,
    outer: Array,
    slot: u32,
    observer: &mut impl TransportObserver,
) {
    const INVALID: &str = "transport original inactive air region header invalid";
    let mut region = 0;
    while region < outer.count {
        if observer.region_attempted(slot, region) {
            region += 1;
            continue;
        }
        let mut end = region + 1;
        while end < outer.count
            && ((end - region) as usize) < BATCH
            && !observer.region_attempted(slot, end)
        {
            end += 1;
        }
        let count = (end - region) as usize;
        let mut headers = [[0_u8; 16]; BATCH];
        let copied = outer
            .element(region, engine_native_memory::cpdx_array::STRIDE)
            .map_or(0, |first| {
                memory.copy_records(
                    RecordSources::Strided {
                        base: first,
                        stride: engine_native_memory::cpdx_array::STRIDE,
                        count,
                    },
                    &REGION_HEADER_LAYOUT,
                    headers[..count].as_flattened_mut(),
                )
            });
        for (offset, header) in headers[..copied].iter().enumerate() {
            let original = Array::from_header(*header, 8).ok_or(INVALID);
            observer.inactive_region(slot, region + offset as u32, original);
        }
        region += copied as u32;
        if copied < count {
            observer.inactive_region(slot, region, Err(INVALID));
            region += 1;
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one root's missions write the shared output, counts, failure context, observer and carry"
)]
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
    carry: &mut impl Carry,
    token: usize,
) -> Result<(), &'static str> {
    failure.country_slot = Some(canonical_slot);
    let (Some(outer), Some(all), Some(regions)) = (
        header(
            bytes,
            cstrategic_air::REGIONAL_MISSIONS,
            engine_native_memory::cpdx_array::STRIDE,
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
        engine_native_memory::cpdx_array::copy_u32s(
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
                .element(region, engine_native_memory::cpdx_array::STRIDE)
                .and_then(|at| Array::read(memory, at, 8))
                .ok_or("active mission region vector missing or invalid");
            observer.region(canonical_slot, region, missions);
            let missions = missions?;
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
                    carry.record(
                        token,
                        Registration {
                            region,
                            ordinal: batch + index as u32,
                            pointer,
                        },
                    );
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

fn carried_missions<M: Memory>(
    countries: &ccountry::Reader<'_, M>,
    canonical_slot: u32,
    held: &[Registration],
    wings: &[WingIdentity],
    output: &mut Buffer<'_, Row>,
    len: &mut usize,
    unread_rows: &mut u32,
) -> bool {
    let start = (*len, *unread_rows);
    let agreed = (|| {
        output.ensure(len.checked_add(held.len())?).ok()?;
        for registration in held {
            let wing = wings
                .binary_search_by_key(&registration.pointer, |wing| wing.mission_pointer)
                .ok()
                .map(|index| &wings[index])?;
            if wing.region != Some(registration.region)
                || countries.link(wing.owner).ok()? != Some(canonical_slot)
            {
                return None;
            }
            match output.get_mut(*len) {
                Some(destination) => {
                    *destination = Row {
                        wing: wing.wing,
                        ordinal: registration.ordinal,
                    };
                    *len += 1;
                }
                None => *unread_rows = unread_rows.checked_add(1)?,
            }
        }
        Some(())
    })();
    if agreed.is_none() {
        (*len, *unread_rows) = start;
    }
    agreed.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cair_wing, cstrategic_air_manager};
    use engine_native_memory::memory::SliceMemory;

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
        put64(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        put32(&mut bytes, COUNTRY + 8, 3);
        put64(&mut bytes, GS + 0x690, MANAGER as u64);
        put64(
            &mut bytes,
            MANAGER,
            cstrategic_air_manager::VTABLE_RVA.0 as u64,
        );
        array(&mut bytes, MANAGER + 0x30, OWNERS, 4);
        put64(&mut bytes, OWNERS + 8, OWNER as u64);
        put64(&mut bytes, OWNER, cstrategic_air::VTABLE_RVA.0 as u64);
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
    fn capture(bytes: &[u8], wings: &[WingIdentity], output: &mut Vec<Row>) -> Outcome {
        walk(&SliceMemory { base: 0, bytes }, 0, GS, wings, output)
    }

    #[test]
    fn original_regional_headers_grow_first_and_later_mod_membership() {
        let (mut bytes, _) = fixture();
        let mut output = Vec::new();
        for count in [50_u32, 123] {
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
            let result = walk_with_diagnostic(
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
        }
    }

    #[test]
    fn registration_order_is_the_regional_vector_order() {
        let (bytes, wings) = fixture();
        let mut rows = vec![Row::default(); 2];
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
    }

    #[test]
    fn a_listed_empty_carrier_region_preserves_ordinary_registrations() {
        let (mut bytes, wings) = fixture();
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
            let mut rows = vec![Row::default(); 2];
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
        let mut rows = vec![Row::default(); 2];
        assert_eq!(capture(&bytes, &wings, &mut rows), Outcome::Rows { len: 0 });
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
        assert_eq!(capture(&bytes, &wings, &mut Vec::new()), Outcome::Rows { len: 0 });
    }

    #[test]
    fn invalid_region_headers_keep_the_country_and_region_in_the_diagnostic() {
        let (mut bytes, wings) = fixture();
        let mut rows = vec![Row::default(); 2];
        let mut failure = FailureContext::default();
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
        let mut rows = vec![Row::default(); 2];
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

    #[derive(Default)]
    struct Holding {
        held: Vec<Registration>,
        recorded: Vec<Registration>,
        roots: Vec<Root>,
        carry: bool,
        carried: usize,
        read: usize,
    }
    impl Carry for Holding {
        fn plan(&mut self, root: &Root) -> (usize, bool) {
            self.roots.push(*root);
            self.recorded.clear();
            (7, self.carry)
        }
        fn held(&self, token: usize) -> &[Registration] {
            assert_eq!(token, 7);
            &self.held
        }
        fn carried(&mut self, _: usize) {
            self.carried += 1;
        }
        fn record(&mut self, _: usize, registration: Registration) {
            self.recorded.push(registration);
        }
        fn read(&mut self, _: usize) {
            self.read += 1;
        }
    }

    struct Traced<'a> {
        bytes: &'a [u8],
        reads: std::cell::RefCell<Vec<std::ops::Range<usize>>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads
                .borrow_mut()
                .push(address..address + output.len());
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }

    fn carried(
        bytes: &[u8],
        wings: &[WingIdentity],
        output: &mut Vec<Row>,
        carry: &mut Holding,
    ) -> (Outcome, FailureContext, Vec<std::ops::Range<usize>>) {
        let memory = Traced {
            bytes,
            reads: Default::default(),
        };
        let mut failure = FailureContext::default();
        let outcome = walk_carried(
            &memory,
            0,
            GS,
            wings,
            Buffer::Growing(output),
            &mut failure,
            carry,
        );
        (outcome, failure, memory.reads.into_inner())
    }

    fn regional(reads: &[std::ops::Range<usize>]) -> bool {
        reads.iter().any(|read| {
            [
                IDS..IDS + 4,
                REGIONS..REGIONS + 6 * 0x18,
                MEMBERS..MEMBERS + 16,
            ]
            .iter()
            .any(|span| read.start < span.end && span.start < read.end)
        })
    }

    #[test]
    fn carried_registrations_publish_a_fresh_walks_rows_without_regional_reads() {
        let (bytes, wings) = fixture();
        let mut carry = Holding::default();
        let mut fresh = vec![Row::default(); 2];
        let (outcome, _, reads) = carried(&bytes, &wings, &mut fresh, &mut carry);
        assert_eq!(outcome, Outcome::Rows { len: 2 });
        assert!(regional(&reads));
        assert_eq!((carry.read, carry.carried), (1, 0));
        let pointer = |wing: usize| wing + cair_wing::MISSION;
        assert_eq!(
            carry.recorded,
            [
                Registration {
                    region: 5,
                    ordinal: 0,
                    pointer: pointer(WING_B)
                },
                Registration {
                    region: 5,
                    ordinal: 1,
                    pointer: pointer(WING_A)
                },
            ]
        );
        assert_eq!(
            carry.roots[0].headers[..],
            bytes[OWNER + cstrategic_air::REGIONAL_MISSIONS..][..HEADERS]
        );
        assert_eq!(
            (carry.roots[0].address, carry.roots[0].canonical_slot),
            (OWNER, 1)
        );
        carry.held = std::mem::take(&mut carry.recorded);
        carry.carry = true;
        for _ in 0..2 {
            let mut rows = Vec::new();
            let (outcome, failure, reads) = carried(&bytes, &wings, &mut rows, &mut carry);
            assert_eq!(outcome, capture(&bytes, &wings, &mut fresh));
            assert_eq!(rows, fresh);
            assert_eq!(failure, FailureContext::default());
            assert!(!regional(&reads), "{reads:x?}");
            assert!(reads.iter().any(|read| read.start == OWNER + 0x98));
        }
        assert_eq!((carry.read, carry.carried), (1, 2));
        let mut renamed = wings;
        renamed[0].wing = 303;
        let mut rows = vec![Row::default(); 2];
        carried(&bytes, &renamed, &mut rows, &mut carry);
        capture(&bytes, &renamed, &mut fresh);
        assert_eq!(rows, fresh);
        assert_eq!(rows[1].wing, 303);
    }

    #[test]
    fn a_census_disagreement_reads_the_carried_object_afresh() {
        let (bytes, wings) = fixture();
        let mut carry = Holding::default();
        carried(&bytes, &wings, &mut vec![Row::default(); 2], &mut carry);
        carry.held = std::mem::take(&mut carry.recorded);
        carry.carry = true;
        let mut moved = wings;
        moved[1].region = Some(6);
        let mut foreign = wings;
        foreign[0].owner = 2;
        let missing = [wings[0]];
        for census in [&moved[..], &foreign[..], &missing[..]] {
            let before = carry.read;
            let mut rows = vec![Row::default(); 2];
            let (outcome, failure, reads) = carried(&bytes, census, &mut rows, &mut carry);
            let mut expected_failure = FailureContext::default();
            let expected = walk_with_diagnostic(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                GS,
                census,
                &mut vec![Row::default(); 2],
                &mut expected_failure,
            );
            assert!(matches!(expected, Outcome::Unavailable { .. }));
            assert_eq!((outcome, failure), (expected, expected_failure));
            assert!(regional(&reads));
            assert_eq!(carry.read, before);
        }
        assert_eq!(carry.carried, 0);
    }

    #[test]
    fn inactive_region_headers_are_gathered_in_runs_with_per_region_verdicts() {
        const OUTER: usize = 0x1_0000;
        const REGIONS: u32 = 200;
        struct Counted<'a> {
            bytes: &'a [u8],
            denied: Option<usize>,
            calls: std::cell::Cell<usize>,
        }
        impl Memory for Counted<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.set(self.calls.get() + 1);
                !self
                    .denied
                    .is_some_and(|at| (address..address + output.len()).contains(&at))
                    && SliceMemory {
                        base: 0,
                        bytes: self.bytes,
                    }
                    .copy(address, output)
            }
            fn copy_records(
                &self,
                sources: RecordSources<'_>,
                layout: &RecordLayout<'_>,
                destination: &mut [u8],
            ) -> usize {
                let calls = self.calls.get();
                let completed = Unbatched(self).copy_records(sources, layout, destination);
                self.calls.set(calls + 1);
                completed
            }
        }
        struct Unbatched<'a, 'b>(&'a Counted<'b>);
        impl Memory for Unbatched<'_, '_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.0.copy(address, output)
            }
        }
        #[derive(Default)]
        struct Observer {
            seen: Vec<(u32, Result<Array, &'static str>)>,
        }
        impl TransportObserver for Observer {
            fn inactive_region(
                &mut self,
                _: u32,
                region: u32,
                original: Result<Array, &'static str>,
            ) {
                self.seen.push((region, original));
            }
            fn region_attempted(&self, _: u32, region: u32) -> bool {
                region == 5 || region == 150
            }
        }
        let mut bytes = vec![0; OUTER + REGIONS as usize * engine_native_memory::cpdx_array::STRIDE];
        for region in 0..REGIONS as usize {
            let at = OUTER + region * engine_native_memory::cpdx_array::STRIDE;
            array(&mut bytes, at, 0x9_0000 + region * 0x100, region as u32 % 3);
        }
        put32(&mut bytes, OUTER + 7 * engine_native_memory::cpdx_array::STRIDE + 8, 0);
        let outer = Array {
            data: OUTER,
            count: REGIONS,
        };
        let expected = |denied: Option<usize>| -> Vec<(u32, Result<Array, &'static str>)> {
            (0..REGIONS)
                .filter(|region| *region != 5 && *region != 150)
                .map(|region| {
                    let at = OUTER + region as usize * engine_native_memory::cpdx_array::STRIDE;
                    let readable = denied.is_none_or(|denied| !(at..at + 16).contains(&denied));
                    let header = readable
                        .then(|| {
                            Array::read(
                                &SliceMemory {
                                    base: 0,
                                    bytes: &bytes,
                                },
                                at,
                                8,
                            )
                        })
                        .flatten()
                        .ok_or("transport original inactive air region header invalid");
                    (region, header)
                })
                .collect()
        };
        for denied in [None, Some(OUTER + 100 * engine_native_memory::cpdx_array::STRIDE + 9)] {
            let memory = Counted {
                bytes: &bytes,
                denied,
                calls: std::cell::Cell::new(0),
            };
            let mut observer = Observer::default();
            inactive_regions(&memory, outer, 3, &mut observer);
            assert_eq!(observer.seen, expected(denied));
            let refused = |region| {
                observer
                    .seen
                    .iter()
                    .any(|(at, seen)| *at == region && seen.is_err())
            };
            assert!(refused(7) && refused(100) == denied.is_some() && !refused(8));
            assert_eq!(memory.calls.get(), 5);
        }
    }
}
