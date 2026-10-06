use super::*;

#[test]
fn border_optional_values_and_pairs_survive_delta_clear_and_reintroduction() {
    let old = DivisionStatusRow {
        division: 1,
        border_combat_stats: crate::rows::Recorded::Value(Some([i64::MIN, -100_001, i64::MAX])),
        ..Default::default()
    };
    let absent = DivisionStatusRow {
        division: 1,
        ..Default::default()
    };
    let delta = absent.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(
        delta.apply::<DivisionStatusRow>(Some(old.clone())).unwrap(),
        absent
    );
    assert_eq!(
        old.delta(Some(&absent))
            .unwrap()
            .unwrap()
            .apply::<DivisionStatusRow>(Some(absent))
            .unwrap(),
        old
    );
    let country = CountryRow {
        id: 1,
        border_land_doctrine_level: crate::rows::Recorded::Value(Some(i32::MIN)),
        ..Default::default()
    };
    let absent = CountryRow {
        id: 1,
        ..Default::default()
    };
    assert_eq!(
        absent
            .delta(Some(&country))
            .unwrap()
            .unwrap()
            .apply::<CountryRow>(Some(country))
            .unwrap(),
        absent
    );
    let state = StateFlagRow {
        state: 1,
        border_conflict_active: crate::rows::Recorded::Value(255),
        border_conflict_countries: vec![[i32::MIN, 0], [7, 7]].into(),
        ..Default::default()
    };
    let inactive = StateFlagRow {
        state: 1,
        ..Default::default()
    };
    assert_eq!(
        inactive
            .delta(Some(&state))
            .unwrap()
            .unwrap()
            .apply::<StateFlagRow>(Some(state))
            .unwrap(),
        inactive
    );
}
#[test]
fn inactive_state_pair_payload_is_refused() {
    let row = StateFlagRow {
        state: 1,
        border_conflict_countries: vec![[1, 2]].into(),
        ..Default::default()
    };
    assert!(row.delta(None).is_err());
}

/// Isolate the added mask byte in a synthetic 24-hour DivisionStatus stream.
#[test]
#[ignore = "recording-size measurement"]
fn division_status_mask_recording_cost() {
    for count in [1_500, 5_000] {
        let fixture: Vec<_> = (1..=count)
            .map(|id| DivisionStatusRow {
                division: (id as u64) << 32 | 51,
                maximum_organisation: 3_000_000,
                ..Default::default()
            })
            .collect();
        for (name, stride) in [("quiet", None), ("typical", Some(24)), ("all", Some(1))] {
            let mut previous = fixture.clone();
            let mut current = Vec::new();
            let mut before = Vec::new();
            let mut changed = 0;
            for hour in 0..24 {
                let mut next = previous.clone();
                if let Some(stride) = stride {
                    for row in next.iter_mut().skip(hour % stride).step_by(stride) {
                        row.maximum_organisation += 1;
                    }
                }
                let mut encoded = Vec::new();
                encode_changes_typed(&next, &previous, &mut encoded, &mut Vec::new()).unwrap();
                let mut input = Decoder::new(&encoded);
                let rows = input.u32().unwrap();
                changed += rows as usize;
                before.extend_from_slice(&rows.to_le_bytes());
                let mut key = 0;
                for _ in 0..rows {
                    let start = encoded.len() - input.remaining();
                    let mut prefix = Decoder::new(&encoded[start..]);
                    prefix.varint().unwrap();
                    let mask = start + encoded[start..].len() - prefix.remaining();
                    let delta = decode_delta(TableId::DivisionStatus, key, &mut input).unwrap();
                    assert_eq!(delta.mask >> 16, 0);
                    key = delta.key;
                    let end = encoded.len() - input.remaining();
                    // Only remove the measured third mask byte; retain all production bytes.
                    before.extend_from_slice(&encoded[start..mask + 2]);
                    before.extend_from_slice(&encoded[mask + 3..end]);
                }
                assert_eq!(input.u32().unwrap(), 0);
                input.finish().unwrap();
                before.extend_from_slice(&0_u32.to_le_bytes());
                current.extend_from_slice(&encoded);
                previous = next;
            }
            assert_eq!(current.len() - before.len(), changed);
            let old_zstd = zstd::encode_all(before.as_slice(), 3).unwrap().len();
            let new_zstd = zstd::encode_all(current.as_slice(), 3).unwrap().len();
            println!(
                "DivisionStatus {count} {name}: changed={changed} raw_before={} raw_after={} zstd_before={old_zstd} zstd_after={new_zstd}",
                before.len(),
                current.len()
            );
        }
    }
}
