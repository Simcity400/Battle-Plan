use crate::extractor::LiveAceRosterInput;

pub const CCOUNTRY_ACE_VECTOR_OFF: u64 = 0x12D8;
pub const CCOUNTRY_ACE_COUNT_OFF: u64 = 0x12E4;
pub const ACE_OBJECT_VTABLE_RVA: u64 = 0x27C7DD0;
pub const ACE_OBJECT_ID_OFF: u64 = 0x0C;
pub const ACE_OBJECT_NAME_OFF: u64 = 0xF0;
pub const ACE_OBJECT_SURNAME_OFF: u64 = 0x110;
pub const ACE_OBJECT_CALLSIGN_OFF: u64 = 0x130;
pub const ACE_OBJECT_ALIVE_OFF: u64 = 0x158;
pub const ACE_OBJECT_KILL_TYPE_OFF: u64 = 0x15C;
pub const ACE_OBJECT_KILLER_ACE_ID_OFF: u64 = 0x164;
pub const ACE_OBJECT_KILLER_COUNTRY_OFF: u64 = 0x168;
pub const ACE_OBJECT_HANDLED_OFF: u64 = 0x16C;
pub const WING_BODY_ASSIGNED_ACE_ID_OFF: u64 = 0x2DC;

pub const MAX_ACE_COUNT: u32 = 1024;
pub const MAX_VECTOR_CAP: u32 = 4096;
pub const MAX_ACE_STRING_LEN: usize = 96;
pub const MAX_KILLER_COUNTRY_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryAceRoot {
    pub tag_db_id: u32,
    pub country_ptr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirWingAceAssignmentRoot {
    pub wing_id: u32,
    pub owner_tag_db_id: u32,
    pub wing_body_ptr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessLocalAceIdentity {
    pub tag_db_id: u32,
    pub ace_id: u32,
    pub ace_ptr: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AceRosterRootRead {
    pub rows: Vec<LiveAceRosterInput>,
    pub process_local_identities: Vec<ProcessLocalAceIdentity>,
    pub observed_country_rows: u32,
    pub observed_ace_rows: u32,
    pub invalid_country_rows: u32,
    pub invalid_ace_rows: u32,
    pub wing_assignment_rows: u32,
    pub ambiguous_assignment_rows: u32,
    pub assignment_scan_blocked: bool,
}

pub trait AceRosterMemory {
    fn read_u32(&self, addr: u64) -> Option<u32>;
    fn read_u64(&self, addr: u64) -> Option<u64>;
    fn read_bytes(&self, addr: u64, len: usize) -> Option<Vec<u8>>;
    fn is_heap_ptr(&self, addr: u64) -> bool;
}

pub fn read_ace_roster_from_roots<M: AceRosterMemory>(
    memory: &M,
    module_base: u64,
    countries: &[CountryAceRoot],
    wings: &[AirWingAceAssignmentRoot],
    assignment_validated: bool,
) -> AceRosterRootRead {
    let wing_by_ace = if assignment_validated {
        wing_assignments_by_ace(memory, wings)
    } else {
        WingAssignments::default()
    };
    let mut read = AceRosterRootRead {
        observed_country_rows: countries.len().min(u32::MAX as usize) as u32,
        ambiguous_assignment_rows: wing_by_ace.ambiguous_rows,
        assignment_scan_blocked: !assignment_validated,
        ..AceRosterRootRead::default()
    };
    let ace_vtable = module_base.wrapping_add(ACE_OBJECT_VTABLE_RVA);

    for country in countries {
        let Some(header) = read_ace_vector_header(memory, country.country_ptr) else {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        };
        if header.size == 0 {
            continue;
        }
        let Some(ptr_bytes) = memory.read_bytes(header.data_ptr, header.size as usize * 8) else {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        };
        if ptr_bytes.len() < header.size as usize * 8 {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        }
        if read_ace_vector_header(memory, country.country_ptr) != Some(header) {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        }

        for index in 0..header.size as usize {
            let ace_ptr = read_u64_le(&ptr_bytes, index * 8);
            if !memory.is_heap_ptr(ace_ptr) {
                read.invalid_ace_rows = read.invalid_ace_rows.saturating_add(1);
                continue;
            }
            if memory.read_u64(ace_ptr) != Some(ace_vtable) {
                read.invalid_ace_rows = read.invalid_ace_rows.saturating_add(1);
                continue;
            }
            let Some(ace_id) = memory.read_u32(ace_ptr + ACE_OBJECT_ID_OFF) else {
                read.invalid_ace_rows = read.invalid_ace_rows.saturating_add(1);
                continue;
            };
            if ace_id == 0 || ace_id > i32::MAX as u32 {
                read.invalid_ace_rows = read.invalid_ace_rows.saturating_add(1);
                continue;
            }
            let wing_id = wing_by_ace.lookup(ace_id, country.tag_db_id);
            if wing_id.is_some() {
                read.wing_assignment_rows = read.wing_assignment_rows.saturating_add(1);
            }
            let death = read_ace_death_record(memory, ace_ptr);
            read.rows.push(LiveAceRosterInput {
                tag_db_id: country.tag_db_id,
                ace_id,
                country_ace_index: index.min(u32::MAX as usize) as u32,
                wing_id,
                identity_layout_validated: true,
                save_layout_validated: true,
                wing_assignment_validated: assignment_validated,
                assignment_blocked: !assignment_validated,
                name: read_msvc_string(memory, ace_ptr + ACE_OBJECT_NAME_OFF),
                surname: read_msvc_string(memory, ace_ptr + ACE_OBJECT_SURNAME_OFF),
                callsign: read_msvc_string(memory, ace_ptr + ACE_OBJECT_CALLSIGN_OFF),
                alive: death.alive,
                handled: death.handled,
                kill_type_raw: death.kill_type_raw,
                killer_ace_id: death.killer_ace_id,
                killer_country_tag_db_id: death.killer_country_tag_db_id,
            });
            read.process_local_identities.push(ProcessLocalAceIdentity {
                tag_db_id: country.tag_db_id,
                ace_id,
                ace_ptr,
            });
            read.observed_ace_rows = read.observed_ace_rows.saturating_add(1);
        }
    }
    read.rows
        .sort_by_key(|row| (row.tag_db_id, row.country_ace_index, row.ace_id));
    read.process_local_identities
        .sort_by_key(|row| (row.tag_db_id, row.ace_id));
    read
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AceVectorHeader {
    data_ptr: u64,
    cap: u32,
    size: u32,
}

fn read_ace_vector_header<M: AceRosterMemory>(
    memory: &M,
    country_ptr: u64,
) -> Option<AceVectorHeader> {
    let count = memory.read_u32(country_ptr + CCOUNTRY_ACE_COUNT_OFF)?;
    let data_ptr = memory.read_u64(country_ptr + CCOUNTRY_ACE_VECTOR_OFF)?;
    let cap = memory.read_u32(country_ptr + CCOUNTRY_ACE_VECTOR_OFF + 8)?;
    let size = memory.read_u32(country_ptr + CCOUNTRY_ACE_VECTOR_OFF + 0x0C)?;
    if size != count || cap < size || cap > MAX_VECTOR_CAP || size > MAX_ACE_COUNT {
        return None;
    }
    if size > 0 && !memory.is_heap_ptr(data_ptr) {
        return None;
    }
    Some(AceVectorHeader {
        data_ptr,
        cap,
        size,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AceDeathRecord {
    alive: Option<bool>,
    handled: Option<bool>,
    kill_type_raw: Option<u32>,
    killer_ace_id: Option<u32>,
    killer_country_tag_db_id: Option<u32>,
}

fn read_ace_death_record<M: AceRosterMemory>(memory: &M, ace_ptr: u64) -> AceDeathRecord {
    fn read_tuple<M: AceRosterMemory>(memory: &M, ace_ptr: u64) -> Option<(u8, u32, u8)> {
        let alive = memory
            .read_bytes(ace_ptr + ACE_OBJECT_ALIVE_OFF, 1)?
            .first()
            .copied()?;
        let kill_type_raw = memory.read_u32(ace_ptr + ACE_OBJECT_KILL_TYPE_OFF)?;
        let handled = memory
            .read_bytes(ace_ptr + ACE_OBJECT_HANDLED_OFF, 1)?
            .first()
            .copied()?;
        Some((alive, kill_type_raw, handled))
    }
    fn stable_u32<M: AceRosterMemory>(memory: &M, addr: u64) -> Option<u32> {
        let first = memory.read_u32(addr)?;
        (memory.read_u32(addr)? == first).then_some(first)
    }

    let first = read_tuple(memory, ace_ptr);
    let second = read_tuple(memory, ace_ptr);
    let (alive, kill_type_raw, handled) = match (first, second) {
        (Some(a), Some(b)) if a == b && a.0 <= 1 && a.2 <= 1 => a,
        _ => return AceDeathRecord::default(),
    };
    match alive {
        1 => AceDeathRecord {
            alive: Some(true),
            handled: Some(handled != 0),
            kill_type_raw: Some(kill_type_raw),
            ..AceDeathRecord::default()
        },
        0 => {
            let killer_ace_id = stable_u32(memory, ace_ptr + ACE_OBJECT_KILLER_ACE_ID_OFF)
                .filter(|value| (1..=i32::MAX as u32).contains(value));
            let killer_country_tag_db_id =
                stable_u32(memory, ace_ptr + ACE_OBJECT_KILLER_COUNTRY_OFF)
                    .filter(|value| (1..=MAX_KILLER_COUNTRY_TAG_DB_ID).contains(value));
            if read_tuple(memory, ace_ptr) != Some((alive, kill_type_raw, handled)) {
                return AceDeathRecord::default();
            }
            AceDeathRecord {
                alive: Some(false),
                handled: Some(handled != 0),
                kill_type_raw: Some(kill_type_raw),
                killer_ace_id,
                killer_country_tag_db_id,
            }
        }
        _ => AceDeathRecord::default(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct WingAssignments {
    by_ace: Vec<(u32, u32, u32)>,
    ambiguous_rows: u32,
}

impl WingAssignments {
    fn lookup(&self, ace_id: u32, owner_tag_db_id: u32) -> Option<u32> {
        self.by_ace
            .iter()
            .find_map(|(candidate_ace_id, wing_id, wing_owner)| {
                (*candidate_ace_id == ace_id && *wing_owner == owner_tag_db_id).then_some(*wing_id)
            })
    }
}

fn wing_assignments_by_ace<M: AceRosterMemory>(
    memory: &M,
    wings: &[AirWingAceAssignmentRoot],
) -> WingAssignments {
    let mut raw: Vec<(u32, u32, u32)> = Vec::new();
    for wing in wings {
        if !memory.is_heap_ptr(wing.wing_body_ptr) {
            continue;
        }
        if let Some(ace_id) = memory.read_u32(wing.wing_body_ptr + WING_BODY_ASSIGNED_ACE_ID_OFF) {
            if ace_id != 0 && ace_id <= i32::MAX as u32 {
                let entry = (ace_id, wing.wing_id, wing.owner_tag_db_id);
                if !raw.contains(&entry) {
                    raw.push(entry);
                }
            }
        }
    }
    let mut out = WingAssignments::default();
    for entry in &raw {
        let claims = raw
            .iter()
            .filter(|other| other.0 == entry.0 && other.2 == entry.2)
            .count();
        if claims == 1 {
            out.by_ace.push(*entry);
        }
    }
    let ambiguous_entries = raw.len().saturating_sub(out.by_ace.len());
    out.ambiguous_rows = ambiguous_entries.min(u32::MAX as usize) as u32;
    out
}

fn read_msvc_string<M: AceRosterMemory>(memory: &M, addr: u64) -> Option<String> {
    let raw = memory.read_bytes(addr, 0x20)?;
    if raw.len() < 0x20 {
        return None;
    }
    let size = read_u64_le(&raw, 0x10) as usize;
    let cap = read_u64_le(&raw, 0x18) as usize;
    if size == 0 || size > MAX_ACE_STRING_LEN || cap < size || cap > (1 << 20) {
        return None;
    }
    let body = if cap < 16 {
        raw.get(0..size)?.to_vec()
    } else {
        let ptr = read_u64_le(&raw, 0);
        if !memory.is_heap_ptr(ptr) {
            return None;
        }
        let heap = memory.read_bytes(ptr, size.saturating_add(1))?;
        if heap.len() < size.saturating_add(1) || heap[size] != 0 {
            return None;
        }
        heap[..size].to_vec()
    };
    let text = String::from_utf8(body).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    Some(text)
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("u64 range"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const MODULE_BASE: u64 = 0x7ff6_0000_0000;

    #[derive(Default)]
    struct FakeMemory {
        bytes: BTreeMap<u64, u8>,
    }

    impl FakeMemory {
        fn write(&mut self, addr: u64, bytes: &[u8]) {
            for (index, byte) in bytes.iter().copied().enumerate() {
                self.bytes.insert(addr + index as u64, byte);
            }
        }

        fn write_u32(&mut self, addr: u64, value: u32) {
            self.write(addr, &value.to_le_bytes());
        }

        fn write_u64(&mut self, addr: u64, value: u64) {
            self.write(addr, &value.to_le_bytes());
        }

        fn write_inline_string(&mut self, addr: u64, value: &[u8]) {
            self.write(addr, &[0; 0x20]);
            self.write(addr, value);
            self.write_u64(addr + 0x10, value.len() as u64);
            self.write_u64(addr + 0x18, 15);
        }
    }

    impl AceRosterMemory for FakeMemory {
        fn read_u32(&self, addr: u64) -> Option<u32> {
            let bytes = self.read_bytes(addr, 4)?;
            Some(u32::from_le_bytes(bytes.try_into().ok()?))
        }

        fn read_u64(&self, addr: u64) -> Option<u64> {
            let bytes = self.read_bytes(addr, 8)?;
            Some(u64::from_le_bytes(bytes.try_into().ok()?))
        }

        fn read_bytes(&self, addr: u64, len: usize) -> Option<Vec<u8>> {
            let mut out = Vec::with_capacity(len);
            for index in 0..len {
                out.push(*self.bytes.get(&(addr + index as u64))?);
            }
            Some(out)
        }

        fn is_heap_ptr(&self, addr: u64) -> bool {
            (0x1000_0000_0000..0x7ff0_0000_0000).contains(&addr)
        }
    }

    fn setup_country_with_aces(mem: &mut FakeMemory, country: u64, aces: &[u64]) {
        let vector = 0x1000_0000_3000;
        mem.write_u64(country + CCOUNTRY_ACE_VECTOR_OFF, vector);
        mem.write_u32(country + CCOUNTRY_ACE_VECTOR_OFF + 8, aces.len() as u32);
        mem.write_u32(country + CCOUNTRY_ACE_VECTOR_OFF + 0x0C, aces.len() as u32);
        mem.write_u32(country + CCOUNTRY_ACE_COUNT_OFF, aces.len() as u32);
        for (index, ace) in aces.iter().enumerate() {
            mem.write_u64(vector + index as u64 * 8, *ace);
        }
    }

    fn setup_ace(mem: &mut FakeMemory, ace: u64, ace_id: u32, alive: u8) {
        mem.write_u64(ace, MODULE_BASE + ACE_OBJECT_VTABLE_RVA);
        mem.write_u32(ace + ACE_OBJECT_ID_OFF, ace_id);
        mem.write(ace + ACE_OBJECT_ALIVE_OFF, &[alive]);
        mem.write_u32(
            ace + ACE_OBJECT_KILL_TYPE_OFF,
            if alive == 0 { 1 } else { 0 },
        );
        mem.write(
            ace + ACE_OBJECT_HANDLED_OFF,
            &[if alive == 0 { 1 } else { 0 }],
        );
        mem.write_inline_string(ace + ACE_OBJECT_NAME_OFF, b"Hans");
        mem.write_inline_string(ace + ACE_OBJECT_SURNAME_OFF, b"Rudel");
        mem.write_inline_string(ace + ACE_OBJECT_CALLSIGN_OFF, b"Adler");
    }

    #[test]
    fn reads_alive_ace_with_wing_assignment_and_no_killer_fields() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let ace = 0x1000_0000_5000;
        let wing_body = 0x1000_0000_7000;
        setup_country_with_aces(&mut mem, country, &[ace]);
        setup_ace(&mut mem, ace, 17514, 1);
        mem.write_u32(ace + ACE_OBJECT_KILLER_ACE_ID_OFF, 999);
        mem.write_u32(ace + ACE_OBJECT_KILLER_COUNTRY_OFF, 77);
        mem.write_u32(wing_body + WING_BODY_ASSIGNED_ACE_ID_OFF, 17514);

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[AirWingAceAssignmentRoot {
                wing_id: 290,
                owner_tag_db_id: 1,
                wing_body_ptr: wing_body,
            }],
            true,
        );

        assert_eq!(read.observed_country_rows, 1);
        assert_eq!(read.observed_ace_rows, 1);
        assert_eq!(read.wing_assignment_rows, 1);
        assert_eq!(read.invalid_ace_rows, 0);
        assert_eq!(
            read.process_local_identities,
            vec![ProcessLocalAceIdentity {
                tag_db_id: 1,
                ace_id: 17514,
                ace_ptr: ace,
            }]
        );
        let row = &read.rows[0];
        assert_eq!(row.ace_id, 17514);
        assert_eq!(row.wing_id, Some(290));
        assert_eq!(row.name.as_deref(), Some("Hans"));
        assert_eq!(row.alive, Some(true));
        assert_eq!(row.handled, Some(false));
        assert_eq!(row.kill_type_raw, Some(0));
        assert_eq!(row.killer_ace_id, None);
        assert_eq!(row.killer_country_tag_db_id, None);
    }

    #[test]
    fn reads_dead_ace_killer_fields_independently() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let ace_with_killer = 0x1000_0000_5000;
        let ace_without_killer = 0x1000_0000_6000;
        setup_country_with_aces(&mut mem, country, &[ace_with_killer, ace_without_killer]);
        setup_ace(&mut mem, ace_with_killer, 5440, 0);
        mem.write_u32(ace_with_killer + ACE_OBJECT_KILLER_ACE_ID_OFF, 6265);
        mem.write_u32(ace_with_killer + ACE_OBJECT_KILLER_COUNTRY_OFF, 92);
        setup_ace(&mut mem, ace_without_killer, 6001, 0);
        mem.write_u32(ace_without_killer + ACE_OBJECT_KILLER_ACE_ID_OFF, 0);
        mem.write_u32(ace_without_killer + ACE_OBJECT_KILLER_COUNTRY_OFF, 92);

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[],
            true,
        );

        assert_eq!(read.rows.len(), 2);
        assert_eq!(read.rows[0].alive, Some(false));
        assert_eq!(read.rows[0].handled, Some(true));
        assert_eq!(read.rows[0].kill_type_raw, Some(1));
        assert_eq!(read.rows[0].killer_ace_id, Some(6265));
        assert_eq!(read.rows[0].killer_country_tag_db_id, Some(92));
        assert_eq!(read.rows[1].alive, Some(false));
        assert_eq!(read.rows[1].handled, Some(true));
        assert_eq!(read.rows[1].kill_type_raw, Some(1));
        assert_eq!(read.rows[1].killer_ace_id, None);
        assert_eq!(read.rows[1].killer_country_tag_db_id, Some(92));
    }

    #[test]
    fn out_of_domain_alive_byte_badges_death_record_unknown() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let ace = 0x1000_0000_5000;
        setup_country_with_aces(&mut mem, country, &[ace]);
        setup_ace(&mut mem, ace, 5440, 7);
        mem.write_u32(ace + ACE_OBJECT_KILLER_ACE_ID_OFF, 6265);
        mem.write_u32(ace + ACE_OBJECT_KILLER_COUNTRY_OFF, 92);

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[],
            true,
        );

        assert_eq!(read.rows.len(), 1);
        assert_eq!(read.rows[0].alive, None);
        assert_eq!(read.rows[0].handled, None);
        assert_eq!(read.rows[0].kill_type_raw, None);
        assert_eq!(read.rows[0].killer_ace_id, None);
        assert_eq!(read.rows[0].killer_country_tag_db_id, None);
    }

    #[test]
    fn out_of_domain_handled_byte_badges_entire_lifecycle_tuple_unknown() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let ace = 0x1000_0000_5000;
        setup_country_with_aces(&mut mem, country, &[ace]);
        setup_ace(&mut mem, ace, 5440, 0);
        mem.write(ace + ACE_OBJECT_HANDLED_OFF, &[7]);
        mem.write_u32(ace + ACE_OBJECT_KILLER_ACE_ID_OFF, 6265);
        mem.write_u32(ace + ACE_OBJECT_KILLER_COUNTRY_OFF, 92);

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[],
            true,
        );

        let row = &read.rows[0];
        assert_eq!(row.alive, None);
        assert_eq!(row.handled, None);
        assert_eq!(row.kill_type_raw, None);
        assert_eq!(row.killer_ace_id, None);
        assert_eq!(row.killer_country_tag_db_id, None);
    }

    #[test]
    fn rejects_ace_entry_with_wrong_vtable() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let ace = 0x1000_0000_5000;
        setup_country_with_aces(&mut mem, country, &[ace]);
        setup_ace(&mut mem, ace, 5440, 1);
        mem.write_u64(ace, MODULE_BASE + 0x1234);

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[],
            true,
        );

        assert_eq!(read.rows.len(), 0);
        assert!(read.process_local_identities.is_empty());
        assert_eq!(read.invalid_ace_rows, 1);
    }

    #[test]
    fn rejects_country_vector_when_count_disagrees() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let vector = 0x1000_0000_3000;
        mem.write_u64(country + CCOUNTRY_ACE_VECTOR_OFF, vector);
        mem.write_u32(country + CCOUNTRY_ACE_VECTOR_OFF + 8, 4);
        mem.write_u32(country + CCOUNTRY_ACE_COUNT_OFF, 2);
        mem.write_u32(country + CCOUNTRY_ACE_VECTOR_OFF + 0x0C, 1);

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[],
            false,
        );

        assert_eq!(read.rows.len(), 0);
        assert_eq!(read.invalid_country_rows, 1);
        assert!(read.assignment_scan_blocked);
    }

    #[test]
    fn ambiguous_wing_claims_drop_the_assignment() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let ace = 0x1000_0000_5000;
        let wing_a = 0x1000_0000_7000;
        let wing_b = 0x1000_0000_8000;
        setup_country_with_aces(&mut mem, country, &[ace]);
        setup_ace(&mut mem, ace, 17514, 1);
        mem.write_u32(wing_a + WING_BODY_ASSIGNED_ACE_ID_OFF, 17514);
        mem.write_u32(wing_b + WING_BODY_ASSIGNED_ACE_ID_OFF, 17514);

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[
                AirWingAceAssignmentRoot {
                    wing_id: 290,
                    owner_tag_db_id: 1,
                    wing_body_ptr: wing_a,
                },
                AirWingAceAssignmentRoot {
                    wing_id: 291,
                    owner_tag_db_id: 1,
                    wing_body_ptr: wing_b,
                },
            ],
            true,
        );

        assert_eq!(read.rows[0].wing_id, None);
        assert_eq!(read.wing_assignment_rows, 0);
        assert_eq!(read.ambiguous_assignment_rows, 2);
    }

    #[test]
    fn cross_country_wing_claim_is_not_joined() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let ace = 0x1000_0000_5000;
        let wing = 0x1000_0000_7000;
        setup_country_with_aces(&mut mem, country, &[ace]);
        setup_ace(&mut mem, ace, 17514, 1);
        mem.write_u32(wing + WING_BODY_ASSIGNED_ACE_ID_OFF, 17514);

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[AirWingAceAssignmentRoot {
                wing_id: 290,
                owner_tag_db_id: 2,
                wing_body_ptr: wing,
            }],
            true,
        );

        assert_eq!(read.rows[0].wing_id, None);
        assert_eq!(read.wing_assignment_rows, 0);
    }

    #[test]
    fn decodes_utf8_name_bytes() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let ace = 0x1000_0000_5000;
        setup_country_with_aces(&mut mem, country, &[ace]);
        setup_ace(&mut mem, ace, 17514, 1);
        mem.write_inline_string(ace + ACE_OBJECT_NAME_OFF, "Gr\u{fc}nemeier".as_bytes());
        mem.write_inline_string(ace + ACE_OBJECT_SURNAME_OFF, b"M\xFCller");

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[],
            true,
        );

        assert_eq!(read.rows[0].name.as_deref(), Some("Gr\u{fc}nemeier"));
        assert_eq!(read.rows[0].surname, None);
    }

    #[test]
    fn rejects_control_characters_in_strings() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let ace = 0x1000_0000_5000;
        setup_country_with_aces(&mut mem, country, &[ace]);
        setup_ace(&mut mem, ace, 17514, 1);
        mem.write_inline_string(ace + ACE_OBJECT_NAME_OFF, b"ctrl\x01name");
        mem.write_inline_string(ace + ACE_OBJECT_SURNAME_OFF, b"del\x7Fname");

        let read = read_ace_roster_from_roots(
            &mem,
            MODULE_BASE,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[],
            true,
        );

        assert_eq!(read.rows[0].name, None);
        assert_eq!(read.rows[0].surname, None);
    }
}
