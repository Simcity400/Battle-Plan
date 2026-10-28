//! Country-owned retained naval accidents, in their stored vector order.
//! Reports preserve their raw references after the live ship disappears.
use crate::{Memory, cpdx_array::Array, cstrategic_region, read, walk::Buffer};

/// Primary report identity.
pub const VTABLE_RVA: usize = 0x029d_a6e8;
/// Strategic navy's report-pointer vector.
pub const REPORTS: usize = 0xe0;
/// Strategic navy's mine-report pointer vector.
pub const MINE_REPORTS: usize = 0xf8;
/// Mine report primary identity.
pub const MINE_VTABLE_RVA: usize = 0x029d_a738;
/// Raw country reference responsible for the mine.
pub const MINE_OWNER: usize = 0x48;
/// Report's strategic-region pointer.
pub const REGION: usize = 0x08;
/// Stored dominant equipment-variant full reference.
pub const EQUIPMENT: usize = 0x10;
/// Accumulated raw signed fixed5 damage.
pub const DAMAGE: usize = 0x18;
/// Raw sunk byte, presented by its nonzero predicate.
pub const SUNK: usize = 0x20;
/// Stored full ship reference.
pub const SHIP: usize = 0x24;
/// Raw country reference at report creation.
pub const OWNER: usize = 0x2c;
/// Native expiry date's raw hour.
pub const EXPIRE_HOUR: usize = 0x38;
/// Report creation/update, regional clear and daily expiry writer sites.
pub const RECORD_ACCIDENT_RVA: usize = 0x00f1_e580;
/// Regional accident-clear writer.
pub const CLEAR_ACCIDENTS_RVA: usize = 0x00f1_6030;
/// Daily expiry writer shared by both report families.
pub const EXPIRE_REPORTS_RVA: usize = 0x00f1_74a0;
/// Mine incident creation and damage accumulation writer.
pub const RECORD_MINE_INCIDENT_RVA: usize = 0x00f1_e670;
/// Verified entry fingerprints for the complete report writer family.
pub const WRITERS: [(usize, u64); 4] = [
    (RECORD_ACCIDENT_RVA, 0x670a_924e_c0d9_c20a),
    (CLEAR_ACCIDENTS_RVA, 0xfe6c_f199_3e3e_5e06),
    (EXPIRE_REPORTS_RVA, 0xfd33_b773_6602_2ffb),
    (RECORD_MINE_INCIDENT_RVA, 0xcf0f_977d_6d16_ed55),
];

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Record {
    pub kind: u8,
    pub ordinal: u32,
    pub mine_owner: u32,
    pub region: u32,
    pub equipment: u64,
    pub damage: i64,
    pub sunk: u8,
    pub ship: u64,
    pub owner: u32,
    pub expire_hour: i32,
}

/// The caller already authenticated this navy during its physical manager pass.
/// Invoke only first/on-change under the complete report writer certificate.
/// Returns the native count; retained output storage may be larger.
pub fn capture(
    memory: &impl Memory,
    base: usize,
    navy: usize,
    output: &mut Vec<Record>,
) -> Result<usize, &'static str> {
    let mut len = 0_usize;
    for (kind, offset, vtable) in [(0, REPORTS, VTABLE_RVA), (1, MINE_REPORTS, MINE_VTABLE_RVA)] {
        let root = navy
            .checked_add(offset)
            .ok_or("naval accident vector address overflow")?;
        let reports =
            Array::read(memory, root, 8).ok_or("naval accident vector unreadable or invalid")?;
        let demand = len
            .checked_add(reports.count as usize)
            .ok_or("naval accident count overflow")?;
        Buffer::Growing(output).ensure(demand)?;
        let expected = base
            .checked_add(vtable)
            .ok_or("naval accident identity overflow")? as u64;
        for ordinal in 0..reports.count {
            let at = reports
                .element(ordinal, 8)
                .ok_or("naval accident occurrence address overflow")?;
            let pointer = read::<u64>(memory, at)
                .and_then(|pointer| usize::try_from(pointer).ok())
                .filter(|&p| p != 0)
                .ok_or("naval accident occurrence null or unreadable")?;
            pointer
                .checked_add(EXPIRE_HOUR + 4)
                .ok_or("naval accident record address overflow")?;
            let bytes = read::<[u8; EXPIRE_HOUR + 4]>(memory, pointer)
                .ok_or("naval accident record unreadable")?;
            let word = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
            if word(0) != expected {
                return Err("naval accident report identity mismatch");
            }
            let region = cstrategic_region::id(memory, base, word(REGION) as usize)?
                .ok_or("naval accident region null")?;
            let mine_owner = if kind == 1 {
                pointer
                    .checked_add(MINE_OWNER)
                    .and_then(|at| read(memory, at))
                    .ok_or("naval mine report owner unreadable")?
            } else {
                0
            };
            output[len] = Record {
                kind,
                ordinal,
                mine_owner,
                region,
                equipment: word(EQUIPMENT),
                damage: word(DAMAGE) as i64,
                sunk: bytes[SUNK],
                ship: word(SHIP),
                owner: u32::from_le_bytes(bytes[OWNER..OWNER + 4].try_into().unwrap()),
                expire_hour: i32::from_le_bytes(
                    bytes[EXPIRE_HOUR..EXPIRE_HOUR + 4].try_into().unwrap(),
                ),
            };
            len += 1;
        }
    }
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::memory::SliceMemory;
    const NAVY: usize = 0x100;
    const VECTOR: usize = 0x300;
    const REPORT: usize = 0x400;
    const AREA: usize = 0x500;
    fn put<T: AsRef<[u8]>>(bytes: &mut [u8], at: usize, value: T) {
        let value = value.as_ref();
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn fixture() -> Vec<u8> {
        let mut b = vec![0; 0x600];
        put(&mut b, NAVY + REPORTS, (VECTOR as u64).to_le_bytes());
        put(&mut b, NAVY + REPORTS + 8, 2_u32.to_le_bytes());
        put(&mut b, NAVY + REPORTS + 12, 2_u32.to_le_bytes());
        put(&mut b, VECTOR, (REPORT as u64).to_le_bytes());
        put(&mut b, VECTOR + 8, (REPORT as u64).to_le_bytes());
        put(&mut b, REPORT, (VTABLE_RVA as u64).to_le_bytes());
        put(&mut b, REPORT + REGION, (AREA as u64).to_le_bytes());
        put(
            &mut b,
            REPORT + EQUIPMENT,
            0xfedc_ba98_0123_4567_u64.to_le_bytes(),
        );
        put(
            &mut b,
            REPORT + SHIP,
            0x0123_4567_fedc_ba98_u64.to_le_bytes(),
        );
        put(&mut b, REPORT + DAMAGE, (-12345_i64).to_le_bytes());
        b[REPORT + SUNK] = 7;
        put(&mut b, REPORT + OWNER, 0xaabb_ccdd_u32.to_le_bytes());
        put(&mut b, REPORT + EXPIRE_HOUR, (-42_i32).to_le_bytes());
        put(
            &mut b,
            AREA,
            (cstrategic_region::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut b, AREA + cstrategic_region::ID, 5001_u32.to_le_bytes());
        b
    }
    #[test]
    fn reports_keep_order_repeats_full_references_and_raw_operands() {
        let b = fixture();
        let mut out = Vec::new();
        assert_eq!(
            capture(&SliceMemory { base: 0, bytes: &b }, 0, NAVY, &mut out),
            Ok(2)
        );
        assert_eq!(out[0].ship, out[1].ship);
        assert_eq!(out[1].ordinal, 1);
        assert_eq!(
            out[0],
            Record {
                kind: 0,
                ordinal: 0,
                mine_owner: 0,
                region: 5001,
                equipment: 0xfedc_ba98_0123_4567,
                damage: -12345,
                sunk: 7,
                ship: 0x0123_4567_fedc_ba98,
                owner: 0xaabb_ccdd,
                expire_hour: -42
            }
        );
    }
    #[test]
    fn mine_reports_keep_the_independent_owner_and_family_ordinal() {
        let mut b = fixture();
        let mine = 0x580;
        put(
            &mut b,
            NAVY + MINE_REPORTS,
            (VECTOR as u64 + 16).to_le_bytes(),
        );
        put(&mut b, NAVY + MINE_REPORTS + 8, 1_u32.to_le_bytes());
        put(&mut b, NAVY + MINE_REPORTS + 12, 1_u32.to_le_bytes());
        put(&mut b, VECTOR + 16, (mine as u64).to_le_bytes());
        let prototype = b[REPORT..REPORT + EXPIRE_HOUR + 4].to_vec();
        b[mine..mine + prototype.len()].copy_from_slice(&prototype);
        put(&mut b, mine, (MINE_VTABLE_RVA as u64).to_le_bytes());
        put(&mut b, mine + MINE_OWNER, 0x1234_5678_u32.to_le_bytes());
        let mut out = Vec::new();
        assert_eq!(
            capture(&SliceMemory { base: 0, bytes: &b }, 0, NAVY, &mut out),
            Ok(3)
        );
        assert_eq!(
            (out[2].kind, out[2].ordinal, out[2].mine_owner),
            (1, 0, 0x1234_5678)
        );
        assert_eq!(out[2].owner, 0xaabb_ccdd);
        assert_eq!(out[1].ordinal, 1);
    }

    #[test]
    fn certified_empty_is_distinct_from_missing_or_wrong_identity() {
        let mut b = fixture();
        let mut out = Vec::new();
        b[REPORT] ^= 1;
        assert!(capture(&SliceMemory { base: 0, bytes: &b }, 0, NAVY, &mut out).is_err());
        put(&mut b, NAVY + REPORTS + 12, 0_u32.to_le_bytes());
        assert_eq!(
            capture(&SliceMemory { base: 0, bytes: &b }, 0, NAVY, &mut out),
            Ok(0)
        );
        assert!(
            capture(
                &SliceMemory {
                    base: 0,
                    bytes: &b[..NAVY + REPORTS + 12]
                },
                0,
                NAVY,
                &mut out
            )
            .is_err()
        );
    }
}
