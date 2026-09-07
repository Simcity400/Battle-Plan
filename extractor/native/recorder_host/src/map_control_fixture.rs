//! Explicit synthetic MAPC2 construction for owning host tests only.
//! Callers supply the current physical province identities and record hour;
//! this is not a parser repair or a production observation certificate.
use compact_contract::province_occupants::{
    self as wire, Cause, Census, Family, Province, Status, Vector,
};
use std::collections::BTreeSet;

pub(crate) fn encode_rows(
    rows: &[u8],
    game_hour_raw: i32,
    observation_ordinal: u64,
    province_ids: &[u32],
) -> Vec<u8> {
    let ids: BTreeSet<_> = province_ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        province_ids.len(),
        "fixture identities must be unique"
    );
    assert!(!ids.contains(&0), "root slot zero is vacant");
    let count = ids
        .last()
        .copied()
        .unwrap_or(1)
        .checked_add(1)
        .unwrap()
        .max(2);
    assert!(count as usize <= wire::MAX_PROVINCES);
    let census = Census {
        game_hour_raw,
        observation_ordinal,
        root_slot_count: Some(count),
        status: Status::Measured,
        cause: Cause::None,
        root_covered_prefix: count,
        provinces: (0..count)
            .map(|physical_ordinal| {
                if !ids.contains(&physical_ordinal) {
                    Province {
                        physical_ordinal,
                        province_id: None,
                        status: Status::Vacant,
                        cause: Cause::None,
                        vectors: Vec::new(),
                    }
                } else {
                    Province {
                        physical_ordinal,
                        province_id: Some(physical_ordinal),
                        status: Status::Measured,
                        cause: Cause::None,
                        vectors: Family::ALL
                            .into_iter()
                            .map(|family| Vector {
                                family,
                                status: Status::Measured,
                                cause: Cause::None,
                                declared_count: Some(0),
                                covered_prefix: 0,
                                slots: Vec::new(),
                            })
                            .collect(),
                    }
                }
            })
            .collect(),
    };
    with_census(rows, &census)
}

pub(crate) fn encode_refused_rows(
    rows: &[u8],
    game_hour_raw: i32,
    observation_ordinal: u64,
    cause: Cause,
) -> Vec<u8> {
    with_census(
        rows,
        &Census::failed(game_hour_raw, observation_ordinal, cause),
    )
}

fn with_census(rows: &[u8], census: &Census) -> Vec<u8> {
    let leaf = wire::encode(census).expect("explicit synthetic census is valid");
    let mut payload = Vec::with_capacity(wire::MAP_HEADER_LEN + rows.len() + leaf.len());
    payload.extend_from_slice(&[b'M', b'A', b'P', b'C', 2, 0, 8, 0]);
    payload.extend_from_slice(rows);
    payload.extend_from_slice(&leaf);
    payload
}
