//! LandCombatDetails.sides' organisation loss, stored as the sum of its 24
//! samples ([`LandCombatSideDetails::loss_sum`], `LOSS_SUM_FORMAT`).
use super::*;
use crate::rows::compatibility::{LOSS_SUM_FORMAT, POLITICAL_FORMAT};
use crate::value::Value;

/// Samples whose 64-bit sum wraps, as the survival estimate's additions wrap.
fn samples() -> [i64; 24] {
    std::array::from_fn(|slot| match slot {
        0 => i64::MAX,
        1 => 3,
        2 => -1,
        slot => slot as i64 * 100_003,
    })
}

fn side(sum: i64, count: u32) -> LandCombatSideDetails {
    LandCombatSideDetails {
        committed: vec![0x8000_0041_0000_0001, 2, 3],
        tactic_id: 3,
        damage: [7, -7],
        organisation_loss_sum: sum,
        organisation_loss_count: count,
        commander: 0x8000_0041_0000_0035,
        ..LandCombatSideDetails::default()
    }
}

fn encoded(side: &LandCombatSideDetails) -> Vec<u8> {
    let mut bytes = Vec::new();
    Value::encode(side, &mut bytes).unwrap();
    assert_eq!(bytes.len(), Value::encoded_bytes(side).unwrap());
    bytes
}

fn decoded(bytes: &[u8], format: u16) -> LandCombatSideDetails {
    let mut input = Decoder::new(bytes);
    input.format = format;
    let side = <LandCombatSideDetails as Value>::decode(&mut input).unwrap();
    input.finish().unwrap();
    side
}

/// Where `window` starts in `bytes`; it must occur once.
fn find(bytes: &[u8], window: &[u8]) -> usize {
    let found: Vec<usize> = bytes
        .windows(window.len())
        .enumerate()
        .filter(|(_, candidate)| *candidate == window)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(found.len(), 1, "{window:?} in {bytes:?}");
    found[0]
}

fn loss_window(loss: &[i64], count: u32) -> Vec<u8> {
    let mut window: Vec<u8> = loss.iter().flat_map(|value| value.to_le_bytes()).collect();
    window.extend(count.to_le_bytes());
    window
}

#[test]
fn the_loss_sum_wraps_as_the_survival_estimate_adds() {
    // The survival estimate loads slot 0 and adds the other 23 with 64-bit addition, so
    // the sum wraps.
    let expected = (3..24).fold(i64::MAX.wrapping_add(3).wrapping_sub(1), |sum, slot| {
        sum.wrapping_add(slot as i64 * 100_003)
    });
    assert_eq!(LandCombatSideDetails::loss_sum(&samples()), expected);
    let mut edge = [0; 24];
    edge[0] = i64::MAX;
    edge[1] = 1;
    assert_eq!(LandCombatSideDetails::loss_sum(&edge), i64::MIN);
    assert_eq!(LandCombatSideDetails::loss_sum(&[0; 24]), 0);
}

#[test]
fn a_side_stores_one_marked_loss_sum() {
    let sum = LandCombatSideDetails::loss_sum(&samples());
    let side = side(sum, 30);
    let bytes = encoded(&side);
    // The committed count, inverted, marks the layout.
    assert_eq!(bytes[..4], (!3_u32).to_le_bytes());
    // The sum sits where the samples sat, followed by the count.
    find(&bytes, &loss_window(&[sum], 30));
    assert_eq!(decoded(&bytes, crate::FORMAT_VERSION), side);
    assert_eq!(LOSS_SUM_FORMAT, 102);
    // The historical writer, standing in for format 101, stores 24 samples
    // and the count itself: 184 bytes more.
    let older = crate::rows::with_historical_stamp(0, || encoded(&side));
    assert_eq!(older[..4], 3_u32.to_le_bytes());
    let mut historical = [0; 24];
    historical[0] = sum;
    find(&older, &loss_window(&historical, 30));
}

#[test]
fn any_sum_count_and_committed_list_round_trips() {
    for sum in [i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX] {
        for count in [0, 23, 24, 1_000, u32::MAX] {
            for committed in [Vec::new(), vec![u64::MAX; 40]] {
                let side = LandCombatSideDetails {
                    committed,
                    ..side(sum, count)
                };
                let bytes = encoded(&side);
                assert!(u32::from_le_bytes(bytes[..4].try_into().unwrap()) > u32::MAX / 2);
                assert_eq!(
                    decoded(&bytes, crate::FORMAT_VERSION),
                    side,
                    "{sum} {count}"
                );
            }
        }
    }
}

/// `side`'s bytes as format-102 captures written before the sum stored
/// them: the committed count itself and the 24 `samples`.
fn earlier_provisional(side: &LandCombatSideDetails, samples: &[i64; 24]) -> Vec<u8> {
    let mut bytes = encoded(side);
    let committed = side.committed.len() as u32;
    assert_eq!(bytes[..4], (!committed).to_le_bytes());
    bytes[..4].copy_from_slice(&committed.to_le_bytes());
    let count = side.organisation_loss_count;
    let at = find(&bytes, &loss_window(&[side.organisation_loss_sum], count));
    let mut earlier = bytes[..at].to_vec();
    earlier.extend(loss_window(samples, count));
    earlier.extend(&bytes[at + 8 + 4..]);
    earlier
}

#[test]
fn earlier_provisional_captures_decode_their_samples_to_the_sum() {
    let samples = samples();
    let side = side(LandCombatSideDetails::loss_sum(&samples), 30);
    let earlier = earlier_provisional(&side, &samples);
    assert_eq!(earlier.len(), encoded(&side).len() + 23 * 8);
    assert_eq!(decoded(&earlier, crate::FORMAT_VERSION), side);
    // An empty committed list is marked too.
    let empty = LandCombatSideDetails {
        committed: Vec::new(),
        ..side.clone()
    };
    assert_eq!(encoded(&empty)[..4], u32::MAX.to_le_bytes());
    assert_eq!(
        decoded(
            &earlier_provisional(&empty, &samples),
            crate::FORMAT_VERSION
        ),
        empty
    );
}

#[test]
fn older_recordings_decode_their_samples_to_the_sum() {
    // Format 101 stored the 24 samples and the plain committed count.
    let samples = samples();
    let side = side(LandCombatSideDetails::loss_sum(&samples), 30);
    let mut older = Vec::new();
    // The historical writer stands in for format 101's retired side parts.
    let historical = crate::rows::with_historical_stamp(0, || encoded(&side));
    let at = find(
        &historical,
        &loss_window(
            &{
                let mut first = [0; 24];
                first[0] = side.organisation_loss_sum;
                first
            },
            30,
        ),
    );
    older.extend(&historical[..at]);
    older.extend(loss_window(&samples, 30));
    older.extend(&historical[at + 24 * 8 + 4..]);
    let decoded = decoded(&older, POLITICAL_FORMAT);
    assert_eq!(decoded.organisation_loss_sum, side.organisation_loss_sum);
    assert_eq!(decoded.organisation_loss_count, 30);
    assert_eq!(decoded.committed, side.committed);
}

#[test]
fn a_reader_older_than_the_sum_refuses_a_marked_side() {
    // That reader decoded the committed list as a plain list, whose marked
    // count exceeds any payload.
    let bytes = encoded(&side(5, 30));
    let mut input = Decoder::new(&bytes);
    let error = <Vec<u64> as Value>::decode(&mut input).unwrap_err();
    // The exact error, which fails the reader's whole block: it never
    // misreads the side.
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "nested collection length exceeds remaining payload"
    );
}
