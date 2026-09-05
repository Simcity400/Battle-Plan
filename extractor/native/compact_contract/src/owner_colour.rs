use std::collections::BTreeSet;

pub const OGS_VERSION: u16 = 3;
pub const ROW_BYTES: usize = 28;
pub const MAX_OWNERS: usize = 262_144;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum OwnerColourStatus {
    Exact = 1,
    Unreadable = 2,
    Unstable = 3,
    NonFinite = 4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerColourRow {
    pub owner_kind: u32,
    pub owner_id: u32,
    pub status: OwnerColourStatus,
    pub rgba_f32_bits: [u32; 4],
}

pub fn validate(rows: &[OwnerColourRow], owners: impl IntoIterator<Item = (u32, u32)>) -> Result<(), &'static str> {
    let owners: BTreeSet<_> = owners.into_iter().collect();
    if rows.len() != owners.len() || rows.len() > MAX_OWNERS {
        return Err("D54 owner colour census count mismatch");
    }
    let mut previous = None;
    for row in rows {
        let identity = (row.owner_kind, row.owner_id);
        if row.owner_kind > 1 || row.owner_id == 0 || row.owner_id > i32::MAX as u32
            || previous.is_some_and(|prior| prior >= identity) || !owners.contains(&identity) {
            return Err("D54 owner colour identity does not match the typed census");
        }
        previous = Some(identity);
        let finite = row.rgba_f32_bits.iter().all(|bits| f32::from_bits(*bits).is_finite());
        match row.status {
            OwnerColourStatus::Exact if !finite => return Err("D54 exact owner colour is non-finite"),
            OwnerColourStatus::NonFinite if finite => return Err("D54 non-finite owner colour has no witness"),
            OwnerColourStatus::Unreadable | OwnerColourStatus::Unstable if row.rgba_f32_bits != [0; 4] =>
                return Err("D54 unread owner colour carries value bits"),
            _ => {}
        }
    }
    Ok(())
}

pub fn encode(rows: &[OwnerColourRow], out: &mut Vec<u8>) {
    for row in rows {
        for value in [row.owner_kind, row.owner_id, row.status as u32].into_iter().chain(row.rgba_f32_bits) {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

pub fn decode(bytes: &[u8], owners: impl IntoIterator<Item = (u32, u32)>) -> Result<Vec<OwnerColourRow>, &'static str> {
    if bytes.len() % ROW_BYTES != 0 || bytes.len() / ROW_BYTES > MAX_OWNERS {
        return Err("D54 owner colour block length is invalid");
    }
    let rows = bytes.chunks_exact(ROW_BYTES).map(|row| {
        let value = |index| u32::from_le_bytes(row[index * 4..index * 4 + 4].try_into().unwrap());
        Ok(OwnerColourRow {
            owner_kind: value(0), owner_id: value(1),
            status: match value(2) {
                1 => OwnerColourStatus::Exact, 2 => OwnerColourStatus::Unreadable,
                3 => OwnerColourStatus::Unstable, 4 => OwnerColourStatus::NonFinite,
                _ => return Err("D54 owner colour status is invalid"),
            },
            rgba_f32_bits: std::array::from_fn(|index| value(index + 3)),
        })
    }).collect::<Result<Vec<_>, _>>()?;
    validate(&rows, owners)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_words_and_per_owner_refusals_round_trip_without_quantization() {
        let statuses = [OwnerColourStatus::Exact, OwnerColourStatus::Unreadable,
            OwnerColourStatus::Unstable, OwnerColourStatus::NonFinite];
        let rows: Vec<_> = statuses.into_iter().enumerate().map(|(i, status)| OwnerColourRow {
            owner_kind: u32::from(i == 3), owner_id: i as u32 + 1, status,
            rgba_f32_bits: match status {
                OwnerColourStatus::Exact => [0x80000000, 1, 0x3f800000, 0x7f7fffff],
                OwnerColourStatus::NonFinite => [0x7fc01234, 0xff800000, 0, 0x3f800000],
                _ => [0; 4],
            },
        }).collect();
        let owners = || rows.iter().map(|r| (r.owner_kind, r.owner_id));
        validate(&rows, owners()).unwrap();
        let mut bytes = vec![];
        encode(&rows, &mut bytes);
        assert_eq!(bytes.len(), 4 * ROW_BYTES);
        assert_eq!(decode(&bytes, owners()).unwrap(), rows);
        assert!(decode(&bytes[..bytes.len() - 1], owners()).is_err());
        assert!(decode(&[], owners()).is_err());
        assert_eq!(decode(&[], []).unwrap(), []);
    }

    #[test]
    fn census_identity_status_and_value_witnesses_are_mandatory() {
        let valid = OwnerColourRow { owner_kind: 0, owner_id: 17,
            status: OwnerColourStatus::Exact, rgba_f32_bits: [0; 4] };
        for mutate in [
            |r: &mut OwnerColourRow| r.owner_kind = 1,
            |r: &mut OwnerColourRow| r.owner_id = 18,
            |r: &mut OwnerColourRow| r.rgba_f32_bits[0] = 0x7fc00000,
            |r: &mut OwnerColourRow| r.status = OwnerColourStatus::NonFinite,
            |r: &mut OwnerColourRow| {r.status = OwnerColourStatus::Unreadable; r.rgba_f32_bits[0] = 1;},
            |r: &mut OwnerColourRow| {r.status = OwnerColourStatus::Unstable; r.rgba_f32_bits[0] = 1;},
        ] {
            let mut row = valid.clone(); mutate(&mut row);
            assert!(validate(&[row], [(0, 17)]).is_err());
        }
        assert!(validate(&[valid.clone(), valid.clone()], [(0, 17), (0, 18)]).is_err());
        let mut bytes = vec![]; encode(&[valid], &mut bytes);
        for status in [0u32, 5, u32::MAX] {
            bytes[8..12].copy_from_slice(&status.to_le_bytes());
            assert!(decode(&bytes, [(0, 17)]).is_err());
        }
    }
}
