use alloc::string::ToString;
use alloc::vec;

use super::*;

fn fixture() -> Payload {
    Payload {
        frame_hour: 900,
        status_flags: 0,
        elapsed_wall_ms: 7,
        roster_identity_sha256: [0x11; 32],
        countries: vec![Country {
            tag_db_id: 17,
            tag: "ITA".to_string(),
        }],
        records: vec![Record {
            manager_ordinal: 0,
            balance_id: "ITA_power_balance".to_string(),
            left_side_id: "ITA_grand_council_side".to_string(),
            right_side_id: "ITA_mussolini_side".to_string(),
            trending_side_id: "ITA_mussolini_side".to_string(),
            active_range_id: "ITA_mussolini_medium_control_range".to_string(),
            value_raw_fixed5: 35_000,
            members: vec![MemberAccumulator {
                tag_db_id: 17,
                daily_raw_fixed5: 1_000,
                weekly_raw_fixed5: 0,
            }],
            side_gfx_overrides: vec![SideGfxOverride {
                side_id: "ITA_mussolini_side".to_string(),
                gfx: "GFX_ITA_bop_override".to_string(),
            }],
        }],
    }
}

#[test]
fn pbs1_fixture_round_trip_preserves_every_runtime_leaf() {
    let source = fixture();
    let bytes = encode(&source).unwrap();
    assert_eq!(&bytes[..4], b"PBS1");
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 1);
    assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), HEADER_LEN);
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded, source);
    assert_eq!(decoded.records[0].members[0].daily_raw_fixed5, 1_000);
    assert_eq!(
        decoded.records[0].side_gfx_overrides[0].gfx,
        "GFX_ITA_bop_override"
    );
}

#[test]
fn pbs1_refuses_truncation_bad_magic_and_wrong_version_by_name() {
    let bytes = encode(&fixture()).unwrap();
    assert_eq!(decode(&bytes[..bytes.len() - 1]), Err(Error::Truncated));
    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert_eq!(decode(&bad_magic), Err(Error::BadMagic));
    let mut wrong_version = bytes;
    wrong_version[4..6].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(decode(&wrong_version), Err(Error::WrongVersion));
}

#[test]
fn pbs1_refuses_unbacked_counts_and_string_lengths_before_allocation() {
    let mut count = encode(&fixture()).unwrap();
    count[28..32].copy_from_slice(&((MAX_COUNTRIES as u32) + 1).to_le_bytes());
    assert_eq!(decode(&count), Err(Error::CountryCountLimit));

    let mut member_count = encode(&fixture()).unwrap();
    let record_at = HEADER_LEN as usize + 4 + 4 + 3;
    let member_count_at = record_at
        + 4
        + 8
        + [
            "ITA_power_balance",
            "ITA_grand_council_side",
            "ITA_mussolini_side",
            "ITA_mussolini_side",
            "ITA_mussolini_medium_control_range",
        ]
        .iter()
        .map(|s| 4 + s.len())
        .sum::<usize>();
    member_count[member_count_at..member_count_at + 4].copy_from_slice(&5u32.to_le_bytes());
    assert_eq!(decode(&member_count), Err(Error::Truncated));

    let mut string_len = encode(&fixture()).unwrap();
    let balance_len_at = record_at + 4 + 8;
    string_len[balance_len_at..balance_len_at + 4].copy_from_slice(&64u32.to_le_bytes());
    string_len.truncate(balance_len_at + 4 + 3);
    assert_eq!(decode(&string_len), Err(Error::Truncated));
}

#[test]
fn pbs1_refuses_unsorted_and_duplicate_member_ids() {
    let mut unsorted = fixture();
    unsorted.countries.push(Country {
        tag_db_id: 18,
        tag: "GER".to_string(),
    });
    unsorted.records[0].members = vec![
        MemberAccumulator {
            tag_db_id: 18,
            daily_raw_fixed5: 0,
            weekly_raw_fixed5: 0,
        },
        MemberAccumulator {
            tag_db_id: 17,
            daily_raw_fixed5: 0,
            weekly_raw_fixed5: 0,
        },
    ];
    assert_eq!(encode(&unsorted), Err(Error::UnsortedMemberIds));
    unsorted.records[0].members[1].tag_db_id = 18;
    assert_eq!(encode(&unsorted), Err(Error::DuplicateMemberId));
}

#[test]
fn pbs1_admits_a_record_with_no_member_countries() {
    let mut empty = fixture();
    empty.records[0].members.clear();
    let bytes = encode(&empty).unwrap();
    let decoded = decode(&bytes).unwrap();
    assert!(decoded.records[0].members.is_empty());
    assert_eq!(decoded, empty);
}

#[test]
fn pbs1_admits_a_trending_side_that_is_neither_left_nor_right() {
    let mut third = fixture();
    third.records[0].trending_side_id = "ITA_army_side".to_string();
    let decoded = decode(&encode(&third).unwrap()).unwrap();
    assert_eq!(decoded.records[0].trending_side_id, "ITA_army_side");
    assert_eq!(decoded, third);

    let mut same = fixture();
    same.records[0].right_side_id = same.records[0].left_side_id.clone();
    same.records[0].trending_side_id = same.records[0].left_side_id.clone();
    assert_eq!(decode(&encode(&same).unwrap()).unwrap(), same);

    let mut empty = fixture();
    empty.records[0].trending_side_id = " ".to_string();
    assert_eq!(encode(&empty), Err(Error::EmptyToken));
}

#[test]
fn pbs1_refuses_duplicate_side_override_ids() {
    let mut duplicate = fixture();
    let repeated = duplicate.records[0].side_gfx_overrides[0].clone();
    duplicate.records[0].side_gfx_overrides.push(repeated);
    assert_eq!(encode(&duplicate), Err(Error::DuplicateSideOverride));
}
