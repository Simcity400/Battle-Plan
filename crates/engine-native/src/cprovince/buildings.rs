
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};
use engine_native_parts::cbuilding;
use crate::{cgame_state, cprovince, cstate};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a1_5510);
pub const BUILDING_INDICES: usize = 0x20;
pub const BUILDING_INDEX_COUNT: usize = 0x2c;
pub const NO_BUILDING_INDEX: i32 = -1;
pub const BUILDINGS: usize = 0x38;
pub const ACTIVE_BUILDINGS: usize = 0x50;
pub const FLAGS: usize = 0x68;
pub const ID: usize = 0x6c;
pub const COUNTRY: usize = 0x70;
pub const PROVINCE_FLAG: u32 = 1;
pub const STATE_FLAG: u32 = 2;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x011f_e6b0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub states: Outcome,
    pub provinces: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub province: bool,
    pub status: usize,
    pub header: [u8; HEADER_BYTES],
    pub buildings: usize,
}

pub const HEADER_BYTES: usize = 60;

pub trait Carry {
    fn plan(&mut self, location: &Location) -> (usize, bool);
    fn carry(&mut self, token: usize, output: &mut [cbuilding::Row]);
    fn read(&mut self, token: usize, rows: &[cbuilding::Row]);
}

pub struct Fresh;
impl Carry for Fresh {
    fn plan(&mut self, _: &Location) -> (usize, bool) {
        (0, false)
    }
    fn carry(&mut self, _: usize, _: &mut [cbuilding::Row]) {}
    fn read(&mut self, _: usize, _: &[cbuilding::Row]) {}
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    states: &mut Vec<cbuilding::Row>,
    provinces: &mut Vec<cbuilding::Row>,
) -> Walk {
    walk_carried(memory, base, game_state, states, provinces, &mut Fresh)
}

pub fn walk_carried(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    states: &mut Vec<cbuilding::Row>,
    provinces: &mut Vec<cbuilding::Row>,
    carry: &mut impl Carry,
) -> Walk {
    let state_root = game_state
        .checked_add(cgame_state::STATES)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("building state array unreadable or invalid");
    Walk {
        states: walk_locations(memory, base, game_state, state_root, false, states, carry),
        provinces: walk_locations(memory, base, game_state, state_root, true, provinces, carry),
    }
}

fn walk_locations(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state_root: Result<Array, &'static str>,
    province_scope: bool,
    output: &mut Vec<cbuilding::Row>,
    carry: &mut impl Carry,
) -> Outcome {
    let result = (|| {
        let states = state_root?;
        let locations = if province_scope {
            game_state
                .checked_add(cgame_state::PROVINCES)
                .and_then(|p| Array::read(memory, p, 8))
                .ok_or("building province array unreadable or invalid")?
        } else {
            states
        };
        if states.count < 2 || locations.count < 2 {
            return Err("building location table has no slots");
        }
        let (location_vtable, id_offset, status_offset) = if province_scope {
            (
                cprovince::VTABLE_RVA,
                cprovince::ID,
                cprovince::BUILDING_STATUS,
            )
        } else {
            (cstate::VTABLE_RVA, cstate::ID, cstate::BUILDING_STATUS)
        };
        let location_vtable =
            engine_sites::at(base, location_vtable).ok_or("module address overflow")? as u64;
        let state_vtable =
            engine_sites::at(base, cstate::VTABLE_RVA).ok_or("module address overflow")? as u64;
        let status_vtable =
            engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
        let fields = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 8,
            },
            RecordField {
                source_offset: id_offset,
                destination_offset: 8,
                len: 4,
            },
            RecordField {
                source_offset: cprovince::STATE,
                destination_offset: 12,
                len: if province_scope { 8 } else { 0 },
            },
            RecordField {
                source_offset: status_offset,
                destination_offset: 20,
                len: 8,
            },
            RecordField {
                source_offset: status_offset + FLAGS,
                destination_offset: 28,
                len: 12,
            },
            RecordField {
                source_offset: status_offset + BUILDINGS,
                destination_offset: 40,
                len: 16,
            },
            RecordField {
                source_offset: status_offset + BUILDING_INDICES,
                destination_offset: 56,
                len: 16,
            },
            RecordField {
                source_offset: status_offset + ACTIVE_BUILDINGS,
                destination_offset: 72,
                len: 16,
            },
        ];
        const RECORD_BYTES: usize = 88;
        let layout = RecordLayout::new(&fields, RECORD_BYTES).expect("valid record layout");
        let mut headers = [0_u8; 64 * RECORD_BYTES];
        let mut len = 0_usize;
        let mut state_identities = StateIdentities::new();
        visit_locations(memory, locations, |ids, pointers| {
            let completed =
                memory.copy_records(RecordSources::Pointers(pointers), &layout, &mut headers);
            let records = LocationRecords {
                headers: &headers,
                record_bytes: RECORD_BYTES,
                completed,
                fields: layout.field_count(),
                live_len: ids.len(),
            };
            if province_scope {
                state_identities.gather(memory, states, records);
            }
            let mut staged = [cbuilding::LocationBuildings::EMPTY; BATCH];
            let mut tokens = [0_usize; BATCH];
            let mut staged_len = 0;
            let mut identity_failure = None;
            for (slot, (&location, &pointer)) in ids.iter().zip(pointers).enumerate() {
                let record = &headers[slot * RECORD_BYTES..(slot + 1) * RECORD_BYTES];
                let copied = records.copied(slot);
                let identified = (|| {
                    let vtable = record[..8].try_into().unwrap();
                    let id = record[8..12].try_into().unwrap();
                    let state_link = record[12..20].try_into().unwrap();
                    let status_type = record[20..28].try_into().unwrap();
                    let status_fields = record[28..40].try_into().unwrap();
                    let building_header = record[40..56].try_into().unwrap();
                    let index_header = record[56..72].try_into().unwrap();
                    let active_header = record[72..88].try_into().unwrap();
                    let status = pointer.checked_add(status_offset);
                    if copied == 0 || u64::from_le_bytes(vtable) != location_vtable {
                        return Err("building location vtable mismatch");
                    }
                    if copied < 2 || u32::from_le_bytes(id) != location {
                        return Err("building location id differs from slot");
                    }
                    if province_scope {
                        let state_pointer = (copied >= 3)
                            .then_some(u64::from_le_bytes(state_link))
                            .and_then(|p| usize::try_from(p).ok())
                            .ok_or("building province state pointer unreadable")?;
                        if state_pointer != 0 {
                            let identity = state_identities.at(slot);
                            if identity.copied == 0 || identity.vtable != state_vtable {
                                return Err("building province state vtable mismatch");
                            }
                            (identity.copied == 2)
                                .then_some(identity.id)
                                .filter(|&id| id != 0)
                                .ok_or("building province state id unreadable or zero")?;
                            if !identity.table_matched {
                                return Err("building province state does not match state table");
                            }
                        }
                    }
                    let status = status.ok_or("building status address overflow")?;
                    if copied < 4 {
                        return Err("building status vtable unreadable");
                    }
                    if u64::from_le_bytes(status_type) != status_vtable {
                        return Err("building status vtable mismatch");
                    }
                    if copied < 5 {
                        return Err("building status identity unreadable");
                    }
                    let identity = Identity::from_fields(status_fields);
                    if identity.location != location
                        || (identity.flags & PROVINCE_FLAG != 0) != province_scope
                    {
                        return Err("building status location or scope mismatch");
                    }
                    let buildings = (copied >= 6)
                        .then(|| Array::from_header(building_header, 8))
                        .flatten()
                        .ok_or("building pointer vector unreadable or invalid")?;
                    let indices = (copied >= 7)
                        .then(|| Array::from_header(index_header, 4))
                        .flatten()
                        .ok_or("building index vector unreadable or invalid")?;
                    let active = (copied == 8)
                        .then(|| Array::from_header(active_header, 8))
                        .flatten()
                        .ok_or("active building vector unreadable or invalid")?;
                    let count = buildings.count as usize;
                    len.checked_add(count)
                        .ok_or("building row count overflow")?;
                    Ok(cbuilding::LocationBuildings {
                        buildings,
                        status,
                        indices,
                        active,
                        seed: cbuilding::Row {
                            location,
                            status_flags: identity.flags,
                            country: identity.country,
                            ..cbuilding::Row::default()
                        },
                        start: len,
                        len: count,
                    })
                })();
                match identified {
                    Ok(authenticated) => {
                        let mut header = [0; HEADER_BYTES];
                        header.copy_from_slice(&record[28..88]);
                        let (token, carried) = carry.plan(&Location {
                            province: province_scope,
                            status: authenticated.status,
                            header,
                            buildings: authenticated.len,
                        });
                        if carried {
                            let end = authenticated.start + authenticated.len;
                            if end > output.len() {
                                engine_allocation::reserve(output, end - output.len())
                                    .map_err(|_| "building row allocation failed")?;
                                output.resize(end, cbuilding::Row::default());
                            }
                            carry.carry(token, &mut output[authenticated.start..end]);
                        } else {
                            staged[staged_len] = authenticated;
                            tokens[staged_len] = token;
                            staged_len += 1;
                        }
                        len += authenticated.len;
                    }
                    Err(reason) => {
                        identity_failure = Some(reason);
                        break;
                    }
                }
            }
            cbuilding::read_rows(memory, base, &staged[..staged_len], output)?;
            for (location, &token) in staged[..staged_len].iter().zip(&tokens) {
                carry.read(
                    token,
                    &output[location.start..location.start + location.len],
                );
            }
            match identity_failure {
                Some(reason) => Err(reason),
                None => Ok(()),
            }
        })?;
        Ok(Outcome::Rows { len })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub fn roots(memory: &impl Memory, game_state: usize) -> Option<[[u8; 16]; 2]> {
    let mut states = [0_u8; 16];
    let mut provinces = [0_u8; 16];
    let copied = memory.copy_scattered(
        [
            game_state.checked_add(cgame_state::STATES)?,
            game_state.checked_add(cgame_state::PROVINCES)?,
        ],
        &mut [&mut states, &mut provinces],
    );
    (copied == 2).then_some([states, provinces])
}

const HELD_FIELDS: [RecordField; 5] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: FLAGS,
        destination_offset: 8,
        len: 12,
    },
    RecordField {
        source_offset: BUILDINGS,
        destination_offset: 20,
        len: 16,
    },
    RecordField {
        source_offset: BUILDING_INDICES,
        destination_offset: 36,
        len: 16,
    },
    RecordField {
        source_offset: ACTIVE_BUILDINGS,
        destination_offset: 52,
        len: 16,
    },
];
const HELD_BYTES: usize = 8 + HEADER_BYTES;
const HELD_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&HELD_FIELDS, HELD_BYTES).expect("valid record layout");

pub fn refresh(
    memory: &impl Memory,
    base: usize,
    held: &[Location],
    fresh: &mut [Location],
    output: &mut Vec<cbuilding::Row>,
) -> Result<usize, &'static str> {
    debug_assert_eq!(held.len(), fresh.len());
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    let mut len = 0_usize;
    for (first, batch) in (0..).step_by(BATCH).zip(held.chunks(BATCH)) {
        let mut statuses = [0_usize; BATCH];
        for (slot, location) in statuses.iter_mut().zip(batch) {
            *slot = location.status;
        }
        let mut records = [0_u8; BATCH * HELD_BYTES];
        let copied = memory.copy_records(
            RecordSources::Pointers(&statuses[..batch.len()]),
            &HELD_LAYOUT,
            &mut records[..batch.len() * HELD_BYTES],
        );
        let mut staged = [cbuilding::LocationBuildings::EMPTY; BATCH];
        for (slot, location) in batch.iter().enumerate() {
            let record = &records[slot * HELD_BYTES..(slot + 1) * HELD_BYTES];
            let completed = copied
                .saturating_sub(slot * HELD_FIELDS.len())
                .min(HELD_FIELDS.len());
            if completed == 0 || u64::from_le_bytes(record[..8].try_into().unwrap()) != expected {
                return Err("held building status vtable unreadable or changed");
            }
            let header: [u8; HEADER_BYTES] = record[8..].try_into().unwrap();
            if completed != HELD_FIELDS.len() || header[..12] != location.header[..12] {
                return Err("held building status differs from its held read");
            }
            let identity = Identity::from_fields(header[..12].try_into().unwrap());
            let buildings = Array::from_header(header[12..28].try_into().unwrap(), 8)
                .ok_or("held building pointer vector invalid")?;
            let indices = Array::from_header(header[28..44].try_into().unwrap(), 4)
                .ok_or("held building index vector invalid")?;
            let active = Array::from_header(header[44..60].try_into().unwrap(), 8)
                .ok_or("held active building vector invalid")?;
            let count = buildings.count as usize;
            fresh[first + slot] = Location {
                header,
                buildings: count,
                ..*location
            };
            staged[slot] = cbuilding::LocationBuildings {
                buildings,
                status: location.status,
                indices,
                active,
                seed: cbuilding::Row {
                    location: identity.location,
                    status_flags: identity.flags,
                    country: identity.country,
                    ..cbuilding::Row::default()
                },
                start: len,
                len: count,
            };
            len = len
                .checked_add(count)
                .ok_or("building row count overflow")?;
        }
        cbuilding::read_rows(memory, base, &staged[..batch.len()], output)?;
    }
    Ok(len)
}

const BATCH: usize = 64;

const STATE_IDENTITY_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cstate::ID,
        destination_offset: 8,
        len: 4,
    },
];
const STATE_SLOT_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const STATE_IDENTITY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATE_IDENTITY_FIELDS, 12).expect("valid record layout");
const STATE_SLOT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATE_SLOT_FIELD, 8).expect("valid record layout");

#[derive(Clone, Copy)]
struct LocationRecords<'a> {
    headers: &'a [u8],
    record_bytes: usize,
    completed: usize,
    fields: usize,
    live_len: usize,
}

impl LocationRecords<'_> {
    fn copied(&self, slot: usize) -> usize {
        self.completed
            .saturating_sub(slot * self.fields)
            .min(self.fields)
    }
}

#[derive(Clone, Copy, Default)]
struct StateIdentity {
    copied: usize,
    vtable: u64,
    id: u32,
    table_matched: bool,
}

struct StateIdentities {
    entries: [StateIdentity; BATCH],
    pointers: [usize; BATCH],
    slots: [usize; BATCH],
    identities: [u8; BATCH * 12],
    table_slots: [u8; BATCH * 8],
}

impl StateIdentities {
    fn new() -> Self {
        Self {
            entries: [StateIdentity::default(); BATCH],
            pointers: [0; BATCH],
            slots: [0; BATCH],
            identities: [0; BATCH * 12],
            table_slots: [0; BATCH * 8],
        }
    }

    fn gather(&mut self, memory: &impl Memory, states: Array, locations: LocationRecords<'_>) {
        self.entries = [StateIdentity::default(); BATCH];
        let mut staged = 0;
        for slot in 0..locations.live_len.min(BATCH) {
            if locations.copied(slot) < 3 {
                break;
            }
            let start = slot * locations.record_bytes + 12;
            let Ok(pointer) = usize::try_from(u64::from_le_bytes(
                locations.headers[start..start + 8].try_into().unwrap(),
            )) else {
                break;
            };
            if pointer == 0 {
                continue;
            }
            self.pointers[staged] = pointer;
            self.slots[staged] = slot;
            staged += 1;
        }
        if staged == 0 {
            return;
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&self.pointers[..staged]),
            &STATE_IDENTITY_LAYOUT,
            &mut self.identities[..staged * 12],
        );
        let mut table_pointers = [0_usize; BATCH];
        for (row, table_pointer) in table_pointers[..staged].iter_mut().enumerate() {
            let bytes = &self.identities[row * 12..row * 12 + 12];
            let entry = StateIdentity {
                copied: copied
                    .saturating_sub(row * STATE_IDENTITY_FIELDS.len())
                    .min(STATE_IDENTITY_FIELDS.len()),
                vtable: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
                id: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                table_matched: false,
            };
            *table_pointer = if entry.copied == STATE_IDENTITY_FIELDS.len() {
                states.element(entry.id, 8).unwrap_or(0)
            } else {
                0
            };
            self.entries[self.slots[row]] = entry;
        }
        let copied = memory.copy_records(
            RecordSources::Pointers(&table_pointers[..staged]),
            &STATE_SLOT_LAYOUT,
            &mut self.table_slots[..staged * 8],
        );
        for row in 0..copied.min(staged) {
            let slot = self.table_slots[row * 8..row * 8 + 8].try_into().unwrap();
            self.entries[self.slots[row]].table_matched =
                u64::from_le_bytes(slot) == self.pointers[row] as u64;
        }
    }

    fn at(&self, slot: usize) -> StateIdentity {
        self.entries[slot]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub location: u32,
    pub flags: u32,
    pub country: u32,
}

impl Identity {
    fn from_fields(fields: [u8; 12]) -> Self {
        Self {
            flags: u32::from_le_bytes(fields[..4].try_into().unwrap()),
            location: u32::from_le_bytes(fields[4..8].try_into().unwrap()),
            country: u32::from_le_bytes(fields[8..].try_into().unwrap()),
        }
    }
}

fn visit_locations(
    memory: &impl Memory,
    locations: Array,
    mut visit: impl FnMut(&[u32], &[usize]) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    let mut pointers = [[0_u8; 8]; BATCH];
    let mut live_pointers = [0_usize; BATCH];
    let mut live_ids = [0_u32; BATCH];
    let mut first = 1;
    while first < locations.count {
        let count = ((locations.count - first) as usize).min(BATCH);
        let addresses: [usize; BATCH] = std::array::from_fn(|slot| {
            if slot < count {
                locations.element(first + slot as u32, 8).unwrap_or(0)
            } else {
                0
            }
        });
        let mut fields = pointers.iter_mut().enumerate();
        let copied = memory.copy_scattered(
            addresses,
            &mut std::array::from_fn(|_| {
                let (slot, bytes) = fields.next().unwrap();
                &mut bytes[..if slot < count { 8 } else { 0 }]
            }),
        );
        let mut live_len = 0;
        for (slot, bytes) in pointers.iter().take(copied.min(count)).enumerate() {
            let pointer = usize::try_from(u64::from_le_bytes(*bytes))
                .map_err(|_| "building location pointer unreadable")?;
            if pointer != 0 {
                live_pointers[live_len] = pointer;
                live_ids[live_len] = first + slot as u32;
                live_len += 1;
            }
        }
        visit(&live_ids[..live_len], &live_pointers[..live_len])?;
        if copied < count {
            return Err("building location pointer unreadable");
        }
        first += count as u32;
    }
    Ok(())
}

pub struct Locations {
    states: Array,
    provinces: Array,
}

impl Locations {
    pub fn new(memory: &impl Memory, game_state: usize) -> Result<Self, &'static str> {
        let states = game_state
            .checked_add(cgame_state::STATES)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("construction target state array unreadable or invalid")?;
        let provinces = game_state
            .checked_add(cgame_state::PROVINCES)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("construction target province array unreadable or invalid")?;
        if states.count < 2 || provinces.count < 2 {
            return Err("construction target map has no slots");
        }
        Ok(Self { states, provinces })
    }

    pub fn resolve(
        &self,
        memory: &impl Memory,
        base: usize,
        status: usize,
    ) -> Result<(Identity, u32), &'static str> {
        let identity = identity(memory, base, status)?;
        let province = identity.flags & PROVINCE_FLAG != 0;
        let (array, vtable, id_offset, status_offset) = if province {
            (
                self.provinces,
                cprovince::VTABLE_RVA,
                cprovince::ID,
                cprovince::BUILDING_STATUS,
            )
        } else {
            (
                self.states,
                cstate::VTABLE_RVA,
                cstate::ID,
                cstate::BUILDING_STATUS,
            )
        };
        if identity.location == 0 {
            return Err("construction target location id is zero");
        }
        let pointer = array
            .element(identity.location, 8)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("construction target location absent or unreadable")?;
        let expected = engine_sites::at(base, vtable).ok_or("module address overflow")? as u64;
        if read::<u64>(memory, pointer) != Some(expected) {
            return Err("construction target location vtable mismatch");
        }
        if pointer.checked_add(status_offset) != Some(status)
            || pointer
                .checked_add(id_offset)
                .and_then(|p| read::<u32>(memory, p))
                != Some(identity.location)
        {
            return Err("construction target status does not match map location");
        }
        let state = if province {
            let state_pointer = pointer
                .checked_add(cprovince::STATE)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .ok_or("construction target province state unreadable")?;
            if state_pointer == 0 {
                0
            } else {
                let expected = engine_sites::at(base, cstate::VTABLE_RVA)
                    .ok_or("module address overflow")? as u64;
                if read::<u64>(memory, state_pointer) != Some(expected) {
                    return Err("construction target state vtable mismatch");
                }
                let id = state_pointer
                    .checked_add(cstate::ID)
                    .and_then(|p| read::<u32>(memory, p))
                    .filter(|&id| id != 0)
                    .ok_or("construction target state id unreadable or zero")?;
                if self
                    .states
                    .element(id, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    != Some(state_pointer as u64)
                {
                    return Err("construction target state differs from map state");
                }
                id
            }
        } else {
            identity.location
        };
        Ok((identity, state))
    }
}

pub(crate) fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Identity, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64;
    let mut vtable = [0; 8];
    let mut fields = [0; 12];
    let copied = memory.copy_scattered(
        [
            pointer,
            pointer
                .checked_add(FLAGS)
                .ok_or("building status address overflow")?,
        ],
        &mut [&mut vtable, &mut fields],
    );
    if copied == 0 {
        return Err("building status vtable unreadable");
    }
    if u64::from_le_bytes(vtable) != expected {
        return Err("building status vtable mismatch");
    }
    if copied != 2 {
        return Err("building status identity unreadable");
    }
    Ok(Identity::from_fields(fields))
}

#[cfg(test)]
mod tests;
