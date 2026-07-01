use crate::extractor::LiveAceRosterInput;

pub const CCOUNTRY_ACE_VECTOR_OFF: u64 = 0x12D8;
pub const CCOUNTRY_ACE_COUNT_OFF: u64 = 0x12E4;
pub const ACE_OBJECT_ID_OFF: u64 = 0x0C;
pub const ACE_OBJECT_NAME_OFF: u64 = 0xF0;
pub const ACE_OBJECT_SURNAME_OFF: u64 = 0x110;
pub const ACE_OBJECT_CALLSIGN_OFF: u64 = 0x130;
pub const WING_BODY_ASSIGNED_ACE_ID_OFF: u64 = 0x2DC;

pub const MAX_ACE_COUNT: u32 = 1024;
pub const MAX_VECTOR_CAP: u32 = 4096;
pub const MAX_ACE_STRING_LEN: usize = 96;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryAceRoot {
    pub tag_db_id: u32,
    pub country_ptr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirWingAceAssignmentRoot {
    pub wing_id: u32,
    pub wing_body_ptr: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AceRosterRootRead {
    pub rows: Vec<LiveAceRosterInput>,
    pub observed_country_rows: u32,
    pub observed_ace_rows: u32,
    pub invalid_country_rows: u32,
    pub invalid_ace_rows: u32,
    pub wing_assignment_rows: u32,
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
    countries: &[CountryAceRoot],
    wings: &[AirWingAceAssignmentRoot],
    assignment_validated: bool,
) -> AceRosterRootRead {
    let wing_by_ace = if assignment_validated {
        wing_assignments_by_ace(memory, wings)
    } else {
        Vec::new()
    };
    let mut read = AceRosterRootRead {
        observed_country_rows: countries.len().min(u32::MAX as usize) as u32,
        assignment_scan_blocked: !assignment_validated,
        ..AceRosterRootRead::default()
    };

    for country in countries {
        let Some(count) = memory.read_u32(country.country_ptr + CCOUNTRY_ACE_COUNT_OFF) else {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        };
        if count == 0 {
            continue;
        }
        if count > MAX_ACE_COUNT {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        }
        let Some(data_ptr) = memory.read_u64(country.country_ptr + CCOUNTRY_ACE_VECTOR_OFF) else {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        };
        let Some(cap) = memory.read_u32(country.country_ptr + CCOUNTRY_ACE_VECTOR_OFF + 8) else {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        };
        let Some(size) = memory.read_u32(country.country_ptr + CCOUNTRY_ACE_VECTOR_OFF + 0x0C)
        else {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        };
        if size != count || cap < size || cap > MAX_VECTOR_CAP || !memory.is_heap_ptr(data_ptr) {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        }
        let Some(ptr_bytes) = memory.read_bytes(data_ptr, size as usize * 8) else {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        };
        if ptr_bytes.len() < size as usize * 8 {
            read.invalid_country_rows = read.invalid_country_rows.saturating_add(1);
            continue;
        }

        for index in 0..size as usize {
            let ace_ptr = read_u64_le(&ptr_bytes, index * 8);
            if !memory.is_heap_ptr(ace_ptr) {
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
            let wing_id = wing_by_ace.iter().find_map(|(candidate_ace_id, wing_id)| {
                (*candidate_ace_id == ace_id).then_some(*wing_id)
            });
            if wing_id.is_some() {
                read.wing_assignment_rows = read.wing_assignment_rows.saturating_add(1);
            }
            read.rows.push(LiveAceRosterInput {
                tag_db_id: country.tag_db_id,
                ace_id,
                deleted: false,
                country_ace_index: index.min(u32::MAX as usize) as u32,
                wing_id,
                identity_layout_validated: true,
                save_layout_validated: true,
                wing_assignment_validated: assignment_validated,
                assignment_blocked: !assignment_validated,
                name: read_msvc_string(memory, ace_ptr + ACE_OBJECT_NAME_OFF),
                surname: read_msvc_string(memory, ace_ptr + ACE_OBJECT_SURNAME_OFF),
                callsign: read_msvc_string(memory, ace_ptr + ACE_OBJECT_CALLSIGN_OFF),
            });
            read.observed_ace_rows = read.observed_ace_rows.saturating_add(1);
        }
    }
    read.rows
        .sort_by_key(|row| (row.tag_db_id, row.country_ace_index, row.ace_id));
    read
}

fn wing_assignments_by_ace<M: AceRosterMemory>(
    memory: &M,
    wings: &[AirWingAceAssignmentRoot],
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for wing in wings {
        if !memory.is_heap_ptr(wing.wing_body_ptr) {
            continue;
        }
        if let Some(ace_id) = memory.read_u32(wing.wing_body_ptr + WING_BODY_ASSIGNED_ACE_ID_OFF) {
            if ace_id != 0 && ace_id <= i32::MAX as u32 {
                out.push((ace_id, wing.wing_id));
            }
        }
    }
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
    if !body.iter().all(|byte| (32..127).contains(byte)) {
        return None;
    }
    String::from_utf8(body).ok()
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("u64 range"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

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

        fn write_inline_string(&mut self, addr: u64, value: &str) {
            self.write(addr, &[0; 0x20]);
            let bytes = value.as_bytes();
            self.write(addr, bytes);
            self.write_u64(addr + 0x10, bytes.len() as u64);
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

    #[test]
    fn reads_validated_ace_roster_and_wing_assignment() {
        let mut mem = FakeMemory::default();
        let country = 0x1000_0000_1000;
        let vector = 0x1000_0000_3000;
        let ace = 0x1000_0000_5000;
        let wing_body = 0x1000_0000_7000;
        mem.write_u64(country + CCOUNTRY_ACE_VECTOR_OFF, vector);
        mem.write_u32(country + CCOUNTRY_ACE_VECTOR_OFF + 8, 4);
        mem.write_u32(country + CCOUNTRY_ACE_COUNT_OFF, 1);
        mem.write_u64(vector, ace);
        mem.write_u32(ace + ACE_OBJECT_ID_OFF, 17514);
        mem.write_inline_string(ace + ACE_OBJECT_NAME_OFF, "Hans");
        mem.write_inline_string(ace + ACE_OBJECT_SURNAME_OFF, "Rudel");
        mem.write_inline_string(ace + ACE_OBJECT_CALLSIGN_OFF, "Adler");
        mem.write_u32(wing_body + WING_BODY_ASSIGNED_ACE_ID_OFF, 17514);

        let read = read_ace_roster_from_roots(
            &mem,
            &[CountryAceRoot {
                tag_db_id: 1,
                country_ptr: country,
            }],
            &[AirWingAceAssignmentRoot {
                wing_id: 290,
                wing_body_ptr: wing_body,
            }],
            true,
        );

        assert_eq!(read.observed_country_rows, 1);
        assert_eq!(read.observed_ace_rows, 1);
        assert_eq!(read.wing_assignment_rows, 1);
        assert_eq!(read.rows[0].ace_id, 17514);
        assert_eq!(read.rows[0].wing_id, Some(290));
        assert_eq!(read.rows[0].name.as_deref(), Some("Hans"));
        assert_eq!(read.rows[0].surname.as_deref(), Some("Rudel"));
        assert_eq!(read.rows[0].callsign.as_deref(), Some("Adler"));
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
}
