use super::*;
use replay::{
    frame::Metrics,
    rows::{CountryLinkRow, RaidIntelInputRow, TableRows},
};

fn frame(hour: i32, count: u32, amount: i64, decrypting: u8) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "outside cipher fixture".into(),
    });
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|slot| CountryRow {
                border_land_doctrine_level: replay::rows::Recorded::Value(None),
                id: slot * 2,
                canonical_slot: slot,
                tag: format!("Mod tag {slot}|suffix").into_bytes(),
                political_power_raw: None,
                command_power_raw: 0,
                stability_raw: 0,
                war_support_raw: 0,
                convoys_raw: None,
                peace_conference_participant: 0,
                manpower_residual: 0,
            })
            .collect(),
    ));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .flat_map(|slot| {
                [
                    CountryLinkRow {
                        id: slot * 2 - 1,
                        canonical_slot: slot,
                    },
                    CountryLinkRow {
                        id: slot * 2,
                        canonical_slot: slot,
                    },
                ]
            })
            .collect(),
    ));
    tables[TableId::Cryptology.index()] = TableView::Complete(TableRows::Cryptology(
        (2..=count)
            .map(|slot| CryptologyRow {
                owner_country: 1,
                target_country: slot * 2 - 1,
                amount_raw: amount,
                decrypting,
                bonus_days: -1,
                bonus_active: 0,
            })
            .collect(),
    ));
    tables[TableId::RaidIntelInputs.index()] = TableView::Complete(TableRows::RaidIntelInputs(
        (1..=count)
            .map(|slot| RaidIntelInputRow {
                country: slot * 2,
                crypto_strength: Some(150_000),
                ..Default::default()
            })
            .collect(),
    ));
    DecodedFrame {
        branch: 1,
        hour,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

fn progress(run: &DecryptionRun) -> Option<u8> {
    run.threshold_raw.as_ref().and_then(|required| {
        cryptology::progress_percent(
            run.amount_at(run.first_hour).unwrap(),
            run.bonus_active(),
            required.parse().unwrap(),
        )
    })
}

const DEFINES: cryptology::Defines = cryptology::Defines {
    base: 200_000,
    upgrade: 300_000,
};

fn scan_frames(
    frames: Vec<DecodedFrame>,
    defines: Result<cryptology::Defines, &str>,
) -> DecryptionReply {
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(1, frames)],
    )
    .unwrap();
    scan(&replay, 1, defines, &AtomicBool::new(false)).unwrap()
}

#[test]
fn unrepresentable_tags_keep_cipher_values_and_report_country_ids() {
    for tag in [vec![0xff, b'|'], vec![b'A', 0], vec![]] {
        let mut input = frame(100, 2, 400_000, 1);
        let TableView::Complete(TableRows::Countries(countries)) =
            &mut *input.tables[TableId::Countries.index()]
        else {
            panic!("complete country fixture")
        };
        countries[1].tag = tag;
        let reply = scan_frames(vec![input], Ok(DEFINES));
        let pair = &reply.pairs[0];
        assert_eq!(pair.tag_runs[0].target_tag, None);
        assert_eq!(pair.runs[0].start_amount_raw, "400000");
        assert_eq!(progress(&pair.runs[0]), Some(50));
        assert!(reply.coverage[0].complete);
        assert!(reply.warnings[0].contains("country 2, tag bytes"));
        assert!(reply.warnings[0].contains("cipher data retained"));
        assert!(!reply.warnings[0].contains("not shown"));
    }
}

#[test]
fn missing_country_links_count_the_pair_once_without_hourly_warnings() {
    let mut frames = Vec::new();
    for hour in [100, 101] {
        let mut input = frame(hour, 2, 400_000, 1);
        let TableView::Complete(TableRows::Cryptology(rows)) =
            &mut *input.tables[TableId::Cryptology.index()]
        else {
            panic!("complete cipher fixture")
        };
        rows[0].target_country = 99;
        frames.push(input);
    }
    let reply = scan_frames(frames, Ok(DEFINES));
    assert!(reply.pairs.is_empty());
    assert_eq!(
        reply.warnings,
        ["1 cipher entries affected: Cryptology country link or country is not recorded; 1 → 99"]
    );
}

#[test]
fn missing_thresholds_retain_amounts_and_count_the_target_once() {
    let mut frames = Vec::new();
    for hour in [100, 101] {
        let mut input = frame(hour, 2, 400_000, 1);
        *input.tables[TableId::RaidIntelInputs.index()] = TableView::Unavailable {
            reason: "missing target operands".into(),
        };
        frames.push(input);
    }
    let reply = scan_frames(frames, Ok(DEFINES));
    let pair = &reply.pairs[0];
    assert_eq!(pair.runs[0].last_hour, 101);
    assert_eq!(pair.runs[0].start_amount_raw, "400000");
    assert_eq!(pair.runs[0].threshold_raw, None);
    assert_eq!(
        reply.warnings,
        [
            "1 cipher entries affected: progress unknown: target RaidIntelInputs was not recorded at this hour; country 2"
        ]
    );
}

#[test]
fn signed_division_overflow_retains_amounts_and_counts_the_pair_once() {
    // The fixed5 product wraps before signed division: this numerator is
    // i64::MIN and the threshold is -1.
    let reply = scan_frames(
        [100, 101]
            .into_iter()
            .map(|hour| frame(hour, 2, 1 << 58, 1))
            .collect(),
        Ok(cryptology::Defines {
            base: -1,
            upgrade: 0,
        }),
    );
    let pair = &reply.pairs[0];
    assert_eq!(pair.runs[0].last_hour, 101);
    assert_eq!(pair.runs[0].amount_at(100), Some(1 << 58));
    assert_eq!(progress(&pair.runs[0]), None);
    assert_eq!(
        reply.warnings,
        [
            "1 cipher entries affected: progress unknown: native signed division overflow; country 1 → country 2"
        ]
    );
}

#[test]
fn complete_reply_refuses_only_past_the_shared_byte_budget() {
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(
            1,
            vec![frame(100, 2, 400_000, 1)],
        )],
    )
    .unwrap();
    let cancelled = AtomicBool::new(false);
    let reply = scan(&replay, 1, Err(""), &cancelled).unwrap();
    let fixed_bytes = crate::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).unwrap();
    // The reason appears in the unknown threshold and its counted warning.
    let mut reason = "x".repeat((MAX_PRODUCT_REPLY_BYTES - fixed_bytes) / 2);
    scan(&replay, 1, Err(&reason), &cancelled).unwrap();
    reason.push('x');
    assert_eq!(
        scan(&replay, 1, Err(&reason), &cancelled)
            .unwrap_err()
            .to_string(),
        "Decryption history exceeds the complete-response byte budget"
    );
}

#[test]
fn encoded_branch_scan_compresses_only_consecutive_hours_and_checks_cancellation() {
    let replay = Replay::from_records(
        b"analytics-test".to_vec(),
        &[crate::test_fixtures::records(
            1,
            vec![
                frame(100, 2, 0, 1),
                frame(101, 2, 0, 1),
                frame(103, 2, 800_000, 1),
            ],
        )],
    )
    .unwrap();
    let reply = scan(&replay, 1, Ok(DEFINES), &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.pairs.len(), 1);
    let pair = &reply.pairs[0];
    assert_eq!(
        (pair.runs[0].first_hour, pair.runs[0].last_hour),
        (100, 101)
    );
    assert_eq!(
        pair.breaks,
        vec![CipherObservation {
            hour: 103,
            upper_bound: true
        }]
    );
    assert_eq!(reply.coverage.len(), 2);
    assert!(scan(&replay, 99, Ok(DEFINES), &AtomicBool::new(false)).is_err());
    assert_eq!(
        scan(&replay, 1, Ok(DEFINES), &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn aliases_all_countries_and_equal_hours_are_sparse_at_5000() {
    for count in [1_500, 5_000] {
        let mut scan = Scan::default();
        scan.frame(&frame(100, count, 0, 0), Ok(DEFINES)).unwrap();
        scan.frame(&frame(101, count, 0, 255), Ok(DEFINES)).unwrap();
        scan.frame(&frame(102, count, 0, 255), Ok(DEFINES)).unwrap();
        assert_eq!(scan.pairs.len(), count as usize - 1);
        assert_eq!(scan.coverage.len(), 1);
        for ((owner, target), state) in scan.pairs {
            let pair = state.pair;
            assert_eq!(owner, 1);
            assert_eq!(pair.target_id, target.to_string());
            assert_eq!(pair.runs.len(), 2);
            assert_eq!(pair.tag_runs.len(), 1);
            assert_eq!(pair.tag_runs[0].last_hour, 102);
            assert_eq!(pair.runs[1].last_hour, 102);
            assert_eq!(progress(&pair.runs[1]), Some(1));
            assert_eq!(
                pair.runs[1].started,
                Some(CipherObservation {
                    hour: 101,
                    upper_bound: true
                })
            );
        }
    }
}

#[test]
fn missing_strength_and_bonus_are_distinct_and_gaps_bound_dates() {
    let mut scan = Scan::default();
    scan.frame(&frame(100, 2, 400_000, 1), Ok(DEFINES)).unwrap();
    let mut missing = frame(102, 2, 800_000, 1);
    *missing.tables[TableId::RaidIntelInputs.index()] = TableView::Unavailable {
        reason: "missing operands".into(),
    };
    scan.frame(&missing, Ok(DEFINES)).unwrap();
    assert_eq!(scan.pairs[&(1, 2)].pair.runs[1].threshold_raw, None);
    assert_eq!(scan.pairs[&(1, 2)].pair.runs[1].broken, None);
    scan.frame(&frame(103, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    let mut active = frame(104, 2, 100_000, 0);
    *active.tables[TableId::Cryptology.index()] =
        TableView::Complete(TableRows::Cryptology(vec![CryptologyRow {
            owner_country: 1,
            target_country: 3,
            amount_raw: 100_000,
            bonus_active: 7,
            bonus_days: 23,
            decrypting: 0,
        }]));
    *active.tables[TableId::RaidIntelInputs.index()] = TableView::Unavailable {
        reason: "missing operands".into(),
    };
    scan.frame(&active, Ok(DEFINES)).unwrap();
    let pair = &scan.pairs[&(1, 2)].pair;
    assert_eq!(
        pair.runs[0].started,
        Some(CipherObservation {
            hour: 100,
            upper_bound: true
        })
    );
    assert_eq!(
        pair.runs[1].started,
        Some(CipherObservation {
            hour: 102,
            upper_bound: true,
        })
    );
    assert_eq!(pair.runs[2].started, pair.runs[1].started);
    assert_eq!(pair.runs[3].started, pair.runs[1].started);
    assert_eq!(
        pair.breaks,
        vec![CipherObservation {
            hour: 103,
            upper_bound: true
        }]
    );
    assert_eq!(pair.runs.len(), 4);
    assert_eq!(pair.runs[3].broken, Some(true));
    assert_eq!(pair.runs[3].bonus_days, 23);
    assert_eq!(progress(&pair.runs[3]), None);
}

#[test]
fn known_break_is_exact_but_scripted_start_after_initial_zero_is_bounded() {
    let mut scan = Scan::default();
    scan.frame(&frame(100, 2, 0, 0), Ok(DEFINES)).unwrap();
    scan.frame(&frame(101, 2, 800_000, 0), Ok(DEFINES)).unwrap();
    let pair = &scan.pairs[&(1, 2)].pair;
    assert_eq!(
        pair.runs[1].started,
        Some(CipherObservation {
            hour: 101,
            upper_bound: true
        })
    );
    assert_eq!(
        pair.breaks,
        vec![CipherObservation {
            hour: 101,
            upper_bound: false
        }]
    );
}

#[test]
fn truncated_and_unresolved_rows_are_counted_not_empty_and_restart_bounds() {
    let mut scan = Scan::default();
    let mut partial = frame(100, 2, 0, 0);
    *partial.tables[TableId::Cryptology.index()] = TableView::Truncated {
        rows: TableRows::Cryptology(vec![]),
        missing: 1,
    };
    scan.frame(&partial, Ok(DEFINES)).unwrap();
    assert!(!scan.coverage[0].complete);
    assert!(scan.coverage[0].reason.as_ref().unwrap().contains("1 rows"));
    scan.frame(&frame(101, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    assert!(scan.pairs[&(1, 2)].pair.breaks[0].upper_bound);
    let mut unresolved = frame(102, 2, 1, 1);
    *unresolved.tables[TableId::Cryptology.index()] =
        TableView::Complete(TableRows::Cryptology(vec![CryptologyRow {
            owner_country: 1,
            target_country: 999,
            ..Default::default()
        }]));
    scan.frame(&unresolved, Ok(DEFINES)).unwrap();
    assert!(!scan.coverage.last().unwrap().complete);
    assert!(scan.refusals.lines("cipher observations")[0].contains("1 cipher observations"));
}

#[test]
fn mod_define_stack_changes_threshold_and_unknown_defines_never_use_defaults() {
    use installed_content::content::ResolvedLayers;
    let root = std::env::temp_dir().join(format!("hoi4-decryption-mod-{}", uuid::Uuid::new_v4()));
    let base = root.join("base");
    let modified = root.join("mod");
    for folder in [&base, &modified] {
        std::fs::create_dir_all(folder.join("common/defines")).unwrap();
    }
    std::fs::write(base.join("common/defines/00_base.lua"), "NDefines = { NOperatives = { CRYPTO_BASE_CRYPTO_LEVEL = 2, CRYPTO_CRYPTO_LEVEL_PER_CRYPTO_UPGRADE = 3 } }").unwrap();
    std::fs::write(modified.join("common/defines/99_mod.lua"), "NDefines.NOperatives.CRYPTO_BASE_CRYPTO_LEVEL = 10.25\nNDefines.NOperatives.CRYPTO_CRYPTO_LEVEL_PER_CRYPTO_UPGRADE = 4.5").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&base, &[]), (&modified, &[])]).unwrap();
    let defines = cryptology::Defines::read(&installed_content::defines::read(&layers).unwrap()).unwrap();
    let mut scan = Scan::default();
    scan.frame(&frame(100, 2, 800_000, 1), Ok(defines)).unwrap();
    assert_eq!(
        scan.pairs[&(1, 2)].pair.runs[0].threshold_raw.as_deref(),
        Some("1925000")
    );
    assert_eq!(scan.pairs[&(1, 2)].pair.runs[0].broken, Some(false));
    scan.frame(
        &frame(101, 2, 800_000, 1),
        Err("mod define unavailable: CRYPTO_BASE_CRYPTO_LEVEL"),
    )
    .unwrap();
    assert_eq!(scan.pairs[&(1, 2)].pair.runs[1].threshold_raw, None);
    assert!(
        scan.refusals
            .lines("cipher observations")
            .iter()
            .any(|s| s.contains("CRYPTO_BASE_CRYPTO_LEVEL"))
    );
}

#[test]
fn native_division_overflow_is_named_and_counted() {
    let mut scan = Scan::default();
    scan.frame(
        &frame(100, 2, 1 << 58, 1),
        Ok(cryptology::Defines {
            base: -1,
            upgrade: 0,
        }),
    )
    .unwrap();
    assert_eq!(progress(&scan.pairs[&(1, 2)].pair.runs[0]), None);
    assert!(
        scan.refusals.diagnostic_lines("cipher entries affected")[0]
            .contains("native signed division overflow")
    );
}

#[test]
fn break_activation_expiry_and_rebreak_date_the_current_attempt() {
    let mut scan = Scan::default();
    scan.frame(&frame(100, 2, 0, 0), Ok(DEFINES)).unwrap();
    scan.frame(&frame(101, 2, 700_000, 1), Ok(DEFINES)).unwrap();
    scan.frame(&frame(102, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    // Activation scales the amount and stops work, but active still proves broken.
    for (hour, days) in [(103, 30), (104, 1)] {
        let mut active = frame(hour, 2, 400_000, 0);
        let TableView::Complete(TableRows::Cryptology(rows)) =
            &mut *active.tables[TableId::Cryptology.index()]
        else {
            panic!("complete cipher fixture")
        };
        rows[0].bonus_active = 1;
        rows[0].bonus_days = days;
        scan.frame(&active, Ok(DEFINES)).unwrap();
    }
    // Expiry clears active and stores -1; the remaining amount is below threshold.
    scan.frame(&frame(105, 2, 400_000, 0), Ok(DEFINES)).unwrap();
    scan.frame(&frame(106, 2, 400_000, 0), Ok(DEFINES)).unwrap();
    scan.frame(&frame(107, 2, 600_000, 1), Ok(DEFINES)).unwrap();
    scan.frame(&frame(108, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    scan.frame(&frame(109, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    let pair = &scan.pairs[&(1, 2)].pair;
    assert_eq!(
        pair.breaks,
        vec![
            CipherObservation {
                hour: 102,
                upper_bound: false,
            },
            CipherObservation {
                hour: 108,
                upper_bound: false,
            },
        ]
    );
    assert_eq!(
        pair.runs
            .iter()
            .find(|run| run.first_hour == 101)
            .unwrap()
            .started,
        Some(CipherObservation {
            hour: 101,
            upper_bound: true,
        })
    );
    let expired = pair.runs.iter().find(|run| run.first_hour == 105).unwrap();
    assert_eq!(expired.broken, Some(false));
    assert_eq!(expired.bonus_days, -1);
    assert_eq!(expired.last_hour, 106);
    assert_eq!(expired.started, None);
    let restarted = pair.runs.iter().find(|run| run.first_hour == 107).unwrap();
    assert_eq!(
        restarted.started,
        Some(CipherObservation {
            hour: 107,
            upper_bound: false
        })
    );
    let rebroken = pair.runs.iter().find(|run| run.first_hour == 108).unwrap();
    assert_eq!(rebroken.started, restarted.started);
    assert_eq!(pair.tag_runs.len(), 1);
    assert_eq!(
        (pair.tag_runs[0].first_hour, pair.tag_runs[0].last_hour),
        (100, 109)
    );
}

fn started_at(pair: &DecryptionPair, hour: i32) -> Option<CipherObservation> {
    pair.runs
        .iter()
        .find(|run| (run.first_hour..=run.last_hour).contains(&hour))
        .and_then(|run| run.started.clone())
}

#[test]
fn initial_zero_progress_idle_cannot_date_a_later_start_exactly() {
    for scripted in [false, true] {
        let mut scan = Scan::default();
        for hour in 100..=102 {
            scan.frame(&frame(hour, 2, 0, 0), Ok(DEFINES)).unwrap();
        }
        scan.frame(
            &frame(
                103,
                2,
                if scripted { 100_000 } else { 0 },
                u8::from(!scripted),
            ),
            Ok(DEFINES),
        )
        .unwrap();
        let pair = &scan.pairs[&(1, 2)].pair;
        assert_eq!(pair.runs[0].last_hour, 102);
        assert_eq!(started_at(pair, 102), None);
        assert_eq!(
            started_at(pair, 103),
            Some(CipherObservation {
                hour: 103,
                upper_bound: true,
            })
        );
    }
}

#[test]
fn zero_progress_idle_after_a_gap_cannot_date_a_later_start_exactly() {
    for scripted in [false, true] {
        for missing_read in [false, true] {
            let mut scan = Scan::default();
            scan.frame(&frame(100, 2, 100_000, 1), Ok(DEFINES)).unwrap();
            if missing_read {
                let mut missing = frame(101, 2, 0, 0);
                *missing.tables[TableId::Cryptology.index()] = TableView::Unavailable {
                    reason: "cipher read unavailable".into(),
                };
                scan.frame(&missing, Ok(DEFINES)).unwrap();
            }
            for hour in 102..=104 {
                scan.frame(&frame(hour, 2, 0, 0), Ok(DEFINES)).unwrap();
            }
            scan.frame(
                &frame(
                    105,
                    2,
                    if scripted { 100_000 } else { 0 },
                    u8::from(!scripted),
                ),
                Ok(DEFINES),
            )
            .unwrap();
            let pair = &scan.pairs[&(1, 2)].pair;
            assert_eq!(started_at(pair, 104), None);
            assert_eq!(
                started_at(pair, 105),
                Some(CipherObservation {
                    hour: 105,
                    upper_bound: true,
                })
            );
        }
    }
}

#[test]
fn zero_progress_idle_after_complete_absence_dates_a_later_start_exactly() {
    for scripted in [false, true] {
        let mut scan = Scan::default();
        scan.frame(&frame(98, 2, 100_000, 1), Ok(DEFINES)).unwrap();
        let mut absent = frame(99, 2, 0, 0);
        *absent.tables[TableId::Cryptology.index()] =
            TableView::Complete(TableRows::Cryptology(Vec::new()));
        scan.frame(&absent, Ok(DEFINES)).unwrap();
        for hour in 100..=102 {
            scan.frame(&frame(hour, 2, 0, 0), Ok(DEFINES)).unwrap();
        }
        scan.frame(
            &frame(
                103,
                2,
                if scripted { 100_000 } else { 0 },
                u8::from(!scripted),
            ),
            Ok(DEFINES),
        )
        .unwrap();
        assert_eq!(
            started_at(&scan.pairs[&(1, 2)].pair, 103),
            Some(CipherObservation {
                hour: 103,
                upper_bound: false,
            })
        );
    }
}

#[test]
fn observed_end_establishes_idle_for_a_later_start() {
    for scripted in [false, true] {
        for bonus_expiry in [false, true] {
            let mut scan = Scan::default();
            let mut working = frame(100, 2, 100_000, 0);
            if bonus_expiry {
                let TableView::Complete(TableRows::Cryptology(rows)) =
                    &mut *working.tables[TableId::Cryptology.index()]
                else {
                    panic!("complete cipher fixture")
                };
                rows[0].bonus_active = 1;
                rows[0].bonus_days = 1;
            }
            scan.frame(&working, Ok(DEFINES)).unwrap();
            for hour in 101..=103 {
                scan.frame(&frame(hour, 2, 0, 0), Ok(DEFINES)).unwrap();
            }
            scan.frame(
                &frame(
                    104,
                    2,
                    if scripted { 100_000 } else { 0 },
                    u8::from(!scripted),
                ),
                Ok(DEFINES),
            )
            .unwrap();
            let pair = &scan.pairs[&(1, 2)].pair;
            assert_eq!(started_at(pair, 103), None);
            assert_eq!(
                started_at(pair, 104),
                Some(CipherObservation {
                    hour: 104,
                    upper_bound: false,
                })
            );
        }
    }
}

#[test]
fn zero_progress_pauses_keep_the_attempt_but_cleared_work_opens_a_new_one() {
    let mut scan = Scan::default();
    for (hour, amount, decrypting) in [
        (100, 0, 0),
        (101, 0, 1),
        (102, 0, 0),
        (103, 0, 1),
        (104, 100_000, 1),
        // Scripted removal of progress does not stop ongoing decryption.
        (105, 0, 1),
        (106, 100_000, 1),
        (107, 100_000, 0),
        // Clearing all stopped work ends that attempt.
        (108, 0, 0),
        (109, 0, 0),
        // Scripted work can open the next attempt without its flag.
        (110, 100_000, 0),
        (111, 100_000, 0),
    ] {
        scan.frame(&frame(hour, 2, amount, decrypting), Ok(DEFINES))
            .unwrap();
    }
    let pair = &scan.pairs[&(1, 2)].pair;
    assert_eq!(started_at(pair, 100), None);
    for hour in 101..=107 {
        assert_eq!(
            started_at(pair, hour),
            Some(CipherObservation {
                hour: 101,
                upper_bound: true
            })
        );
    }
    assert_eq!(started_at(pair, 108), None);
    assert_eq!(started_at(pair, 109), None);
    for hour in 110..=111 {
        assert_eq!(
            started_at(pair, hour),
            Some(CipherObservation {
                hour: 110,
                upper_bound: false
            })
        );
    }
}

#[test]
fn expiry_residual_waits_for_a_new_start_or_scripted_work() {
    for resume_with_flag in [false, true] {
        let mut scan = Scan::default();
        scan.frame(&frame(100, 2, 800_000, 1), Ok(DEFINES)).unwrap();
        let mut active = frame(101, 2, 200_000, 0);
        let TableView::Complete(TableRows::Cryptology(rows)) =
            &mut *active.tables[TableId::Cryptology.index()]
        else {
            panic!("complete cipher fixture")
        };
        rows[0].bonus_active = 1;
        rows[0].bonus_days = 1;
        scan.frame(&active, Ok(DEFINES)).unwrap();
        for hour in 102..=126 {
            scan.frame(&frame(hour, 2, 200_000, 0), Ok(DEFINES))
                .unwrap();
        }
        scan.frame(
            &frame(
                127,
                2,
                if resume_with_flag { 200_000 } else { 250_000 },
                u8::from(resume_with_flag),
            ),
            Ok(DEFINES),
        )
        .unwrap();
        let pair = &scan.pairs[&(1, 2)].pair;
        assert_eq!(
            started_at(pair, 101),
            Some(CipherObservation {
                hour: 100,
                upper_bound: true
            })
        );
        assert_eq!(started_at(pair, 102), None);
        assert_eq!(started_at(pair, 126), None);
        assert_eq!(
            started_at(pair, 127),
            Some(CipherObservation {
                hour: 127,
                upper_bound: false
            })
        );
        assert_eq!(
            pair.runs
                .iter()
                .find(|run| run.first_hour == 102)
                .unwrap()
                .last_hour,
            126
        );
    }
}

#[test]
fn absent_pairs_end_attempts_and_capture_gaps_bound_the_next_observation() {
    for complete in [false, true] {
        let mut scan = Scan::default();
        scan.frame(&frame(100, 2, 100_000, 1), Ok(DEFINES)).unwrap();
        let mut absent = frame(101, 2, 0, 0);
        *absent.tables[TableId::Cryptology.index()] = if complete {
            TableView::Complete(TableRows::Cryptology(Vec::new()))
        } else {
            TableView::Unavailable {
                reason: "cipher read unavailable".into(),
            }
        };
        scan.frame(&absent, Ok(DEFINES)).unwrap();
        scan.frame(&frame(102, 2, 100_000, 0), Ok(DEFINES)).unwrap();
        let pair = &scan.pairs[&(1, 2)].pair;
        assert_eq!(
            started_at(pair, 100),
            Some(CipherObservation {
                hour: 100,
                upper_bound: true
            })
        );
        assert_eq!(started_at(pair, 101), None);
        assert_eq!(
            started_at(pair, 102),
            Some(CipherObservation {
                hour: 102,
                upper_bound: !complete
            })
        );
        assert_eq!(pair.runs.len(), 2);
        assert_eq!(pair.tag_runs.len(), 2);
    }
}

#[test]
fn active_bonus_at_first_read_bounds_the_attempt_even_without_progress() {
    let mut scan = Scan::default();
    let mut active = frame(100, 2, 0, 0);
    let TableView::Complete(TableRows::Cryptology(rows)) =
        &mut *active.tables[TableId::Cryptology.index()]
    else {
        panic!("complete cipher fixture")
    };
    rows[0].bonus_active = 7;
    rows[0].bonus_days = 10;
    scan.frame(&active, Err("target threshold unavailable"))
        .unwrap();
    let pair = &scan.pairs[&(1, 2)].pair;
    assert_eq!(
        started_at(pair, 100),
        Some(CipherObservation {
            hour: 100,
            upper_bound: true
        })
    );
    assert_eq!(pair.runs[0].broken, Some(true));
}

#[test]
fn target_crypto_upgrade_unbreaks_until_the_new_threshold_is_reached() {
    let mut scan = Scan::default();
    scan.frame(&frame(100, 2, 0, 0), Ok(DEFINES)).unwrap();
    scan.frame(&frame(101, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    for (hour, amount) in [(102, 800_000), (103, 1_100_000)] {
        let mut upgraded = frame(hour, 2, amount, 1);
        let TableView::Complete(TableRows::RaidIntelInputs(rows)) =
            &mut *upgraded.tables[TableId::RaidIntelInputs.index()]
        else {
            panic!("complete intel fixture")
        };
        rows[1].crypto_strength = Some(250_000);
        scan.frame(&upgraded, Ok(DEFINES)).unwrap();
    }
    let pair = &scan.pairs[&(1, 2)].pair;
    assert_eq!(pair.runs[2].threshold_raw.as_deref(), Some("1100000"));
    assert_eq!(pair.runs[2].broken, Some(false));
    assert_eq!(
        pair.breaks
            .iter()
            .map(|observation| (observation.hour, observation.upper_bound))
            .collect::<Vec<_>>(),
        [(101, false), (103, false)]
    );
}

#[test]
fn a_break_after_an_unknown_predicate_or_gap_is_a_new_upper_bound_observation() {
    let mut scan = Scan::default();
    scan.frame(&frame(100, 2, 0, 0), Ok(DEFINES)).unwrap();
    scan.frame(&frame(101, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    let mut unknown = frame(102, 2, 800_000, 1);
    *unknown.tables[TableId::RaidIntelInputs.index()] = TableView::Unavailable {
        reason: "missing crypto upgrade".into(),
    };
    scan.frame(&unknown, Ok(DEFINES)).unwrap();
    scan.frame(&frame(103, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    scan.frame(&frame(105, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    scan.frame(&frame(106, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    let pair = &scan.pairs[&(1, 2)].pair;
    assert_eq!(
        pair.breaks
            .iter()
            .map(|observation| (observation.hour, observation.upper_bound))
            .collect::<Vec<_>>(),
        [(101, false), (103, true), (105, true)]
    );
    assert_eq!(pair.tag_runs.len(), 2);
    assert_eq!(
        (pair.tag_runs[0].first_hour, pair.tag_runs[0].last_hour),
        (100, 103)
    );
    assert_eq!(
        (pair.tag_runs[1].first_hour, pair.tag_runs[1].last_hour),
        (105, 106)
    );
}

#[test]
fn tag_runs_compress_independently_preserve_opaque_changes_and_never_bridge_gaps() {
    let mut scan = Scan::default();
    scan.frame(&frame(100, 2, 800_000, 1), Ok(DEFINES)).unwrap();
    scan.frame(&frame(101, 2, 900_000, 1), Ok(DEFINES)).unwrap();
    for hour in [102, 103, 105, 106] {
        let mut renamed = frame(hour, 2, 900_000, 1);
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *renamed.tables[TableId::Countries.index()]
        else {
            panic!("complete country fixture")
        };
        rows[1].tag = if hour == 106 {
            vec![0xff]
        } else {
            b"Mod tag | moved".to_vec()
        };
        scan.frame(&renamed, Ok(DEFINES)).unwrap();
    }
    let pair = &scan.pairs[&(1, 2)].pair;
    assert_eq!(
        pair.runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (101, 103), (105, 106)]
    );
    assert_eq!(
        pair.tag_runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.target_tag.as_deref()))
            .collect::<Vec<_>>(),
        [
            (100, 101, Some("Mod tag 2|suffix")),
            (102, 103, Some("Mod tag | moved")),
            (105, 105, Some("Mod tag | moved")),
            (106, 106, None),
        ]
    );
    let numeric = serde_json::to_value(&pair.runs[0]).unwrap();
    assert!(numeric.get("ownerTag").is_none());
    assert!(numeric.get("targetTag").is_none());
}

fn observe_row(scan: &mut Scan, hour: i32, row: &CryptologyRow, threshold: Result<i64, &str>) {
    let previous_complete =
        scan.previous_complete.and_then(|last| last.checked_add(1)) == Some(hour);
    scan.observe(
        (1, 2),
        run(hour, row, threshold),
        DecryptionTagRun {
            first_hour: hour,
            last_hour: hour,
            owner_tag: Some("Mod decryptor | opaque".into()),
            target_tag: Some("Mod target with spaces".into()),
        },
        previous_complete,
    );
    scan.cover(hour, None);
}

#[test]
fn linear_runs_round_trip_every_recorded_value_and_never_fill_gaps() {
    let mut scan = Scan::default();
    let first = 43_800_007;
    let mut recorded = Vec::new();
    for offset in 0..24 * 180 {
        // Start partway through a native day; omit a read during steady work.
        if offset == 24 * 17 + 11 {
            continue;
        }
        let day = (offset + 7) / 24;
        let (amount, decrypting, active, days) = match day {
            0..=49 => (i64::from(day) * 6_000, 255, 0, -1),
            50..=69 => (300_000 + i64::from(day - 50) * 11_111, 255, 0, -1),
            70..=79 => (800_000, 255, 0, -1),
            80..=109 => (200_000, 0, 7, 110 - day),
            110..=129 => (200_000, 0, 0, -1),
            _ => (200_000 + i64::from(day - 130) * 30_000, 1, 0, -1),
        };
        // An arbitrary-hour script event, target upgrade and unknown operand.
        let amount = if offset == 24 * 35 + 5 {
            amount + 1
        } else {
            amount
        };
        let threshold = if day == 140 {
            Err("mod target operand missing")
        } else if day >= 150 {
            Ok(1_100_000)
        } else {
            Ok(800_000)
        };
        let row = CryptologyRow {
            amount_raw: amount,
            decrypting,
            bonus_active: active,
            bonus_days: days,
            ..Default::default()
        };
        let hour = first + offset;
        observe_row(&mut scan, hour, &row, threshold);
        recorded.push((hour, row, threshold));
    }
    let pair = &scan.pairs[&(1, 2)].pair;
    assert!(pair.runs.len() < 30);
    // Verify the actual serialized wire representation, not just the builder.
    let runs: Vec<DecryptionRun> =
        serde_json::from_str(&serde_json::to_string(&pair.runs).unwrap()).unwrap();
    for (hour, row, threshold) in &recorded {
        let encoded = runs
            .iter()
            .find(|run| (run.first_hour..=run.last_hour).contains(hour))
            .unwrap();
        assert_eq!(
            encoded.amount_at(*hour),
            Some(row.amount_raw),
            "amount at {hour}"
        );
        assert_eq!(
            encoded.bonus_days_at(*hour),
            Some(row.bonus_days),
            "countdown at {hour}"
        );
        assert_eq!(encoded.decrypting(), row.decrypting);
        assert_eq!(encoded.bonus_active(), row.bonus_active);
        assert_eq!(
            encoded.threshold_raw,
            threshold.ok().map(|value| value.to_string())
        );
        assert_eq!(encoded.threshold_unknown.as_deref(), threshold.err());
        assert_eq!(
            encoded.broken,
            cryptology::fully_decrypted(row.amount_raw, row.bonus_active, threshold.ok())
        );
        let percent = encoded.threshold_raw.as_ref().and_then(|required| {
            cryptology::progress_percent(
                encoded.amount_at(*hour).unwrap(),
                encoded.bonus_active(),
                required.parse().unwrap(),
            )
        });
        assert_eq!(
            percent,
            threshold
                .ok()
                .and_then(|required| cryptology::progress_percent(
                    row.amount_raw,
                    row.bonus_active,
                    required
                ))
        );
    }
    for hour in [first - 1, first + 24 * 17 + 11, first + 24 * 180] {
        assert!(runs.iter().all(|run| run.amount_at(hour).is_none()));
    }
    assert!(pair.breaks.len() >= 2);
}

#[test]
fn mismatches_signed_limits_and_arbitrary_daily_phase_are_lossless() {
    let mut scan = Scan::default();
    let mut recorded = Vec::new();
    for hour in -100..=100 {
        let amount = match hour {
            -100..=-51 => i64::MAX - i64::from((hour + 100) / 24),
            -50..=-1 => i64::MIN + i64::from((hour + 50) / 24),
            _ => i64::from(hour * hour % 13),
        };
        let row = CryptologyRow {
            amount_raw: amount,
            bonus_days: if hour < 0 { i32::MIN } else { i32::MAX },
            decrypting: 255,
            bonus_active: 255,
            ..Default::default()
        };
        observe_row(&mut scan, hour, &row, Err("threshold unavailable"));
        recorded.push((hour, row));
    }
    for (hour, row) in recorded {
        let run = scan.pairs[&(1, 2)]
            .pair
            .runs
            .iter()
            .find(|run| (run.first_hour..=run.last_hour).contains(&hour))
            .unwrap();
        assert_eq!(run.amount_at(hour), Some(row.amount_raw));
        assert_eq!(run.bonus_days_at(hour), Some(row.bonus_days));
        assert_eq!(run.state_flags, u16::MAX);
    }
}

#[test]
fn ten_year_500_pair_annual_history_fits_the_complete_response_budget() {
    let mut scan = Scan::default();
    let first = 43_800_000;
    for offset in 0..10 * 365 * 24 {
        let year = offset / (365 * 24);
        let day = (offset / 24) % 365;
        let initial = if year == 0 { 0 } else { 200_000 };
        let row = if day < 100 {
            CryptologyRow {
                amount_raw: initial + (800_000 - initial) * i64::from(day + 1) / 100,
                decrypting: 1,
                bonus_days: -1,
                ..Default::default()
            }
        } else {
            CryptologyRow {
                amount_raw: 200_000,
                bonus_active: u8::from(day < 130),
                bonus_days: if day < 130 { 130 - day } else { -1 },
                ..Default::default()
            }
        };
        observe_row(&mut scan, first + offset, &row, Ok(800_000));
    }
    let pair = &scan.pairs[&(1, 2)].pair;
    assert_eq!(pair.runs.len(), 40);
    assert_eq!(pair.breaks.len(), 10);
    let reply = DecryptionReply {
        branch_id: "1".into(),
        pairs: (0..500)
            .map(|index| {
                let mut pair = pair.clone();
                pair.target_id = (index + 2).to_string();
                pair
            })
            .collect(),
        coverage: scan.coverage,
        warnings: Vec::new(),
    };
    let json = crate::transport::bounded_json(
        &super::super::Envelope {
            source_id: "synthetic-lease",
            revision: "synthetic-revision",
            product: Decryption::KEY,
            data: reply,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(json.len() < crate::transport::MAX_REPLY_BYTES / 4);
}
