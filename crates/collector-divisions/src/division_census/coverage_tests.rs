use super::*;
use crate::census_test_fixture as kit;
use replay_data::rows::DivisionCensusRow;

#[test]
fn all_countries_and_raw_aliases_follow_the_native_writer_without_extra_reads() {
    let mut bytes = kit::fixture();
    bytes[kit::A3 + engine_native::cunit::EXPEDITIONARY_OWNER
        ..kit::A3 + engine_native::cunit::EXPEDITIONARY_OWNER + 4]
        .copy_from_slice(&3_u32.to_le_bytes());
    bytes[kit::A1 + engine_native::cunit::EXPEDITIONARY_OWNER
        ..kit::A1 + engine_native::cunit::EXPEDITIONARY_OWNER + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    let mut census = Census::new();
    census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    let mut output = vec![DivisionCensusRow::default(); 8];
    assert_eq!(census.install_counts(&mut output), Outcome::Rows { len: 3 });
    assert_eq!(
        &output[..3],
        &[
            DivisionCensusRow {
                country: 1,
                grouped: 1,
                ungrouped: 1
            },
            DivisionCensusRow {
                country: 2,
                grouped: 0,
                ungrouped: 0
            },
            DivisionCensusRow {
                country: 3,
                grouped: 1,
                ungrouped: 0
            },
        ]
    );
    assert_eq!(
        census.install_counts(&mut output[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    census.clear();
    bytes[kit::A2..kit::A2 + 8].fill(0);
    census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    assert!(matches!(
        census.install_counts(&mut output),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn short_country_counts_grow_in_the_capture() {
    let bytes = kit::fixture();
    let mut census = Census::new();
    census.counts.truncate(2);
    census.capture(&kit::memory(&bytes), kit::BASE, kit::GAME_STATE);
    let mut output = vec![DivisionCensusRow::default(); 8];
    assert_eq!(census.install_counts(&mut output), Outcome::Rows { len: 2 });
}

#[test]
fn owner_outside_country_links_is_not_a_capacity_failure() {
    for capacity in [3, 8] {
        for expeditionary_owner in [0, 3] {
            let mut census = Census::new();
            census.counts.resize(capacity, None);
            census.required_counts = 3;
            let row = carmy::Row {
                roster_owner: if expeditionary_owner == 0 { 3 } else { 1 },
                expeditionary_owner,
                ..Default::default()
            };
            Census::count_division(
                &mut census.counts,
                census.required_counts,
                &mut census.counts_error,
                &row,
            );
            assert_eq!(
                census.install_counts(&mut []),
                Outcome::Unavailable {
                    reason: "division owner outside country links",
                }
            );
            assert!(!census.counts_need_growth());
            assert!(census.counts.iter().all(Option::is_none));
        }
    }
}

#[test]
#[ignore]
fn division_coverage_counts_scale() {
    use std::{hint::black_box, time::Instant};
    for countries in [1500_u32, 5000] {
        let mut census = Census::new();
        census.counts.resize(countries as usize + 1, None);
        let rows: Vec<_> = (1..=countries)
            .flat_map(|owner| {
                (0_u32..100).map(move |division| carmy::Row {
                    roster_owner: owner,
                    orders_group: u64::from(division % 2),
                    ..Default::default()
                })
            })
            .collect();
        let start = Instant::now();
        for _ in 0..100 {
            census.counts.fill(None);
            for row in &rows {
                Census::count_division(
                    &mut census.counts,
                    countries as usize + 1,
                    &mut census.counts_error,
                    black_box(row),
                );
            }
            black_box(&census.counts);
        }
        println!(
            "division coverage countries={countries}, divisions={}, count_ms_per_hour={:.6} (accumulation and clearing only; no native reads or builds)",
            rows.len(),
            start.elapsed().as_secs_f64() * 1000.0 / 100.0
        );
        assert_eq!(
            census.counts[countries as usize]
                .as_ref()
                .unwrap()
                .ungrouped,
            50
        );
    }
}
