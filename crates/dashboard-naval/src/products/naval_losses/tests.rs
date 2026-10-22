use super::*;
use crate::products::sinkings::{ConvoyEvent, CountryIdentitySpan, HistoryLoss, ShipEvent};
use replay_data::rows::SunkShipEntry;

#[test]
fn naval_losses_manual_alias_projection_keeps_all_root_holders_and_signed_corrections() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    let mut histories = RecordedHistories::default();
    for (tag, entries) in [
        (
            "root",
            vec![(first, first + 9, 1, 1), (first + 20, first + 30, 2, 2)],
        ),
        (
            "alias",
            vec![
                (first + 10, first + 19, 1, 1),
                (first + 20, first + 30, 3, 3),
            ],
        ),
    ] {
        let identities: Vec<_> = entries
            .into_iter()
            .map(
                |(first_hour, last_hour, slot, country_id)| CountryIdentityRun {
                    first_hour,
                    last_hour,
                    slot,
                    country_id,
                },
            )
            .collect();
        scanned.segments.insert(
            tag.into(),
            identities
                .iter()
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect(),
        );
        scanned.country_identities.insert(
            tag.into(),
            identities
                .iter()
                .map(|identity| CountryIdentitySpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                    identity: (identity.slot, identity.country_id),
                })
                .collect(),
        );
        let mut recorded = country_history::RecordedCountry::default();
        recorded.identities = std::sync::Arc::new(identities);
        histories.countries.insert(tag.into(), recorded);
    }
    scanned
        .segments
        .retain(|tag, _| tag == "root" || tag == "alias");
    scanned.convoys = [
        (first + 2, "root", 5),
        (first + 12, "alias", -2),
        (first + 22, "alias", 50),
    ]
    .into_iter()
    .map(|(hour, tag, amount)| {
        let mut event = convoy(hour, amount, false);
        event.owner = Some(tag.into());
        event.killer = Some(tag.into());
        event
    })
    .collect();
    let mut params = params();
    params.tags = vec!["root".into()];
    let reply = reply_with_histories(
        &scanned,
        &params,
        &AtomicBool::new(false),
        (true, true),
        (&[], &[]),
        vec![],
        Some(&histories),
    )
    .unwrap();
    assert_eq!(sample(&reply, "alias", first + 12), (Some(0), Some(3)));
    assert!(
        !reply
            .series
            .iter()
            .find(|series| series.tag == "alias")
            .unwrap()
            .runs
            .iter()
            .any(|run| run.last_hour >= first + 20)
    );
    assert_eq!(
        reply
            .series
            .iter()
            .find(|series| series.tag == "root")
            .unwrap()
            .identities
            .as_ref()
            .unwrap()
            .len(),
        2
    );
    let measured = reply_with_histories(
        &scanned,
        &params,
        &AtomicBool::new(false),
        (true, true),
        (&[], &[]),
        vec![String::new()],
        Some(&histories),
    )
    .unwrap();
    let mut warning =
        "x".repeat(MAX_PRODUCT_REPLY_BYTES - json_len(&measured, MAX_PRODUCT_REPLY_BYTES).unwrap());
    let exact = reply_with_histories(
        &scanned,
        &params,
        &AtomicBool::new(false),
        (true, true),
        (&[], &[]),
        vec![warning.clone()],
        Some(&histories),
    )
    .unwrap();
    assert_eq!(
        json_len(&exact, MAX_PRODUCT_REPLY_BYTES),
        Some(MAX_PRODUCT_REPLY_BYTES)
    );
    warning.push('x');
    assert!(
        reply_with_histories(
            &scanned,
            &params,
            &AtomicBool::new(false),
            (true, true),
            (&[], &[]),
            vec![warning],
            Some(&histories)
        )
        .is_err()
    );
}

fn params() -> NavalLossesParams {
    NavalLossesParams {
        branch_id: "1".into(),
        tags: vec!["mod maritime country".into()],
        top: None,
        kind: NavalKind::Both,
        attribution: NavalAttribution::Lost,
        period: NavalPeriod::Running,
    }
}
fn fixture() -> sinkings::Scanned {
    let first = month_start(1940 * 12).unwrap();
    sinkings::Scanned {
        first_hour: first,
        last_hour: first + 2000,
        first_ship_read: Some(first),
        first_convoy_read: Some(first),
        country_candidates: BTreeMap::from([
            (1, BTreeSet::from([(1, 1)])),
            (2, BTreeSet::from([(2, 2)])),
        ]),
        country_identities: BTreeMap::from([
            (
                "mod maritime country".into(),
                vec![CountryIdentitySpan {
                    first_hour: first,
                    last_hour: first + 2000,
                    identity: (1, 1),
                }],
            ),
            (
                "opponent".into(),
                vec![CountryIdentitySpan {
                    first_hour: first,
                    last_hour: first + 2000,
                    identity: (2, 2),
                }],
            ),
        ]),
        segments: BTreeMap::from([
            (
                "mod maritime country".into(),
                vec![HourSpan {
                    first_hour: first,
                    last_hour: first + 2000,
                }],
            ),
            (
                "opponent".into(),
                vec![HourSpan {
                    first_hour: first,
                    last_hour: first + 2000,
                }],
            ),
        ]),
        ..Default::default()
    }
}
fn ship(hour: i32, cause: u8, assisted: u8) -> ShipEvent {
    ShipEvent {
        entry: SunkShipEntry {
            sink_hour: hour,
            class_token: 987654,
            class_name: b"mod sea skimmer".to_vec(),
            province: 54321,
            province_present: 1,
            cause_record: cause,
            assisted_kill: assisted,
            victim_country: 1,
            attacker_country: 2,
            ..Default::default()
        },
        read_hour: hour + 12,
        victim: Some("mod maritime country".into()),
        attacker: Some("opponent".into()),
        victim_as_of: None,
        attacker_as_of: None,
    }
}
fn convoy(hour: i32, count: i64, baseline: bool) -> ConvoyEvent {
    ConvoyEvent {
        month: month(hour).unwrap(),
        read_hour: hour,
        count,
        owner: Some("mod maritime country".into()),
        killer: Some("opponent".into()),
        baseline,
        previous_read: None,
        owner_raw: 1,
        killer_raw: 2,
    }
}
fn sample(reply: &NavalLossesReply, tag: &str, hour: i32) -> (Option<i64>, Option<i64>) {
    let run = reply
        .series
        .iter()
        .find(|s| s.tag == tag)
        .unwrap()
        .runs
        .iter()
        .find(|r| r.first_hour <= hour && hour <= r.last_hour)
        .unwrap();
    (run.ships, run.convoys)
}

#[test]
fn naval_losses_baseline_supplying_reads_modded_classes_and_cause_credit() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.ships = vec![
        ship(first - 1, 0, 0),
        ship(first + 2, 0, 0),
        ship(first + 3, 1, 0),
        ship(first + 4, 0, 1),
    ];
    scanned.convoys = vec![convoy(first, 100, true), convoy(first + 24, 5, false)];
    let reply = reply(&scanned, &params(), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&reply, "mod maritime country", first),
        (Some(0), Some(0))
    );
    assert_eq!(
        sample(&reply, "mod maritime country", first + 2),
        (Some(0), Some(0)),
        "the next daily read cannot be visible at the sinking hour"
    );
    assert_eq!(
        sample(&reply, "mod maritime country", first + 14),
        (Some(1), Some(0))
    );
    assert_eq!(
        sample(&reply, "mod maritime country", first + 24),
        (Some(3), Some(5))
    );
    let mut by = params();
    by.tags = vec!["opponent".into()];
    by.attribution = NavalAttribution::SunkBy;
    let reply = super::reply(&scanned, &by, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&reply, "opponent", first + 24),
        (Some(1), Some(5)),
        "cause and assistance establish no hostile primary kill"
    );
}

#[test]
fn naval_losses_short_gaps_are_unknown_only_while_unread_and_totals_recover() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.ship_unread = vec![HourSpan {
        first_hour: first + 30,
        last_hour: first + 50,
    }];
    scanned.convoy_unread = scanned.ship_unread.clone();
    scanned.ships = vec![ship(first + 20, 0, 0), ship(first + 40, 0, 0)];
    let running = reply(&scanned, &params(), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&running, "mod maritime country", first + 60),
        (Some(2), Some(0))
    );
    assert_eq!(
        sample(&running, "mod maritime country", first + 40),
        (None, None)
    );
    let mut monthly = params();
    monthly.period = NavalPeriod::Month;
    let monthly = reply(&scanned, &monthly, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&monthly, "mod maritime country", first + 20),
        (Some(0), Some(0)),
        "a short later gap does not taint earlier known hours"
    );
    assert_eq!(
        sample(&monthly, "mod maritime country", first + 60),
        (Some(2), Some(0))
    );
    let next = month_start(1940 * 12 + 1).unwrap();
    assert_eq!(
        sample(&monthly, "mod maritime country", next),
        (Some(0), Some(0))
    );
}

#[test]
fn naval_losses_signed_corrections_keep_running_and_month_totals_exact() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.first_convoy_read = Some(first + 12);
    let mut monthly = params();
    monthly.period = NavalPeriod::Month;
    let reply = reply(&scanned, &monthly, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&reply, "mod maritime country", first + 24),
        (Some(0), Some(0))
    );
    scanned.first_convoy_read = Some(first);
    scanned.convoys = vec![
        convoy(first, 5, true),
        convoy(first + 24, 2, false),
        convoy(first + 48, -1, false),
        convoy(first + 72, -8, false),
    ];
    let running = super::reply(&scanned, &params(), &AtomicBool::new(false)).unwrap();
    let monthly = super::reply(&scanned, &monthly, &AtomicBool::new(false)).unwrap();
    for (hour, since, month) in [(0, 0, 5), (24, 2, 7), (48, 1, 6), (72, -7, -2)] {
        assert_eq!(
            sample(&running, "mod maritime country", first + hour).1,
            Some(since)
        );
        assert_eq!(
            sample(&monthly, "mod maritime country", first + hour).1,
            Some(month)
        );
    }
    assert!(running.warnings.is_empty() && monthly.warnings.is_empty());
}

#[test]
fn naval_losses_ranks_requested_measure_and_clips_membership() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.ships = vec![ship(first + 10, 0, 0)];
    let mut p = params();
    p.tags.clear();
    p.top = Some(1);
    p.kind = NavalKind::Ships;
    let reply = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    let series = reply
        .series
        .iter()
        .find(|s| s.tag == "mod maritime country")
        .unwrap();
    assert!(
        series
            .membership
            .as_ref()
            .unwrap()
            .iter()
            .any(|s| s.first_hour <= first + 10 && first + 10 <= s.last_hour)
    );
    assert_eq!(
        sample(&reply, "mod maritime country", first + 22).0,
        Some(1)
    );
    p.tags.push("opaque tag".into());
    assert!(NavalLosses::validate(&p).is_err());
}

#[test]
fn naval_losses_follow_native_identity_through_tag_switch_and_tag_reuse() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    let old_tag = "old opaque tag with spaces";
    let new_tag = "changed opaque tag";
    let original = (7, 42);
    let replacement = (8, 99);
    scanned.segments = BTreeMap::from([
        (
            old_tag.into(),
            vec![
                HourSpan {
                    first_hour: first,
                    last_hour: first + 9,
                },
                HourSpan {
                    first_hour: first + 10,
                    last_hour: scanned.last_hour,
                },
            ],
        ),
        (
            new_tag.into(),
            vec![HourSpan {
                first_hour: first + 10,
                last_hour: scanned.last_hour,
            }],
        ),
    ]);
    scanned.country_identities = BTreeMap::from([
        (
            old_tag.into(),
            vec![
                CountryIdentitySpan {
                    first_hour: first,
                    last_hour: first + 9,
                    identity: original,
                },
                CountryIdentitySpan {
                    first_hour: first + 10,
                    last_hour: scanned.last_hour,
                    identity: replacement,
                },
            ],
        ),
        (
            new_tag.into(),
            vec![CountryIdentitySpan {
                first_hour: first + 10,
                last_hour: scanned.last_hour,
                identity: original,
            }],
        ),
    ]);
    scanned.ships = [(2, old_tag), (11, new_tag), (12, old_tag)]
        .into_iter()
        .map(|(hour, tag)| {
            let mut event = ship(first + hour, 0, 0);
            event.read_hour = first + hour;
            event.victim = Some(tag.into());
            event.attacker = Some(tag.into());
            event
        })
        .collect();
    scanned.convoys = [(3, old_tag, 4), (11, new_tag, 6), (12, old_tag, 7)]
        .into_iter()
        .map(|(hour, tag, count)| {
            let mut event = convoy(first + hour, count, false);
            event.owner = Some(tag.into());
            event.killer = Some(tag.into());
            event
        })
        .collect();
    for attribution in [NavalAttribution::Lost, NavalAttribution::SunkBy] {
        let mut p = params();
        p.tags = vec![old_tag.into(), new_tag.into()];
        p.attribution = attribution;
        let built = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
        assert_eq!(sample(&built, old_tag, first + 5), (Some(1), Some(4)));
        assert_eq!(
            sample(&built, new_tag, first + 10),
            (Some(1), Some(4)),
            "native country keeps pre-switch amounts"
        );
        assert_eq!(
            sample(&built, old_tag, first + 10),
            (Some(0), Some(0)),
            "new native holder does not inherit the reused tag's amounts"
        );
        assert_eq!(sample(&built, new_tag, first + 12), (Some(2), Some(10)));
        assert_eq!(sample(&built, old_tag, first + 12), (Some(1), Some(7)));
        let reused = built
            .series
            .iter()
            .find(|series| series.tag == old_tag)
            .unwrap();
        assert!(
            reused
                .runs
                .iter()
                .any(|run| run.first_hour == first + 10 && run.starts_segment)
        );
    }
}

#[test]
fn naval_losses_absent_data_cancellation_and_calendar_boundaries() {
    let mut scanned = fixture();
    scanned.first_ship_read = None;
    scanned.first_convoy_read = None;
    assert!(
        !reply(&scanned, &params(), &AtomicBool::new(false))
            .unwrap()
            .available
    );
    assert!(reply(&scanned, &params(), &AtomicBool::new(true)).is_err());
    for m in [0, 1, 1940 * 12 + 1, 1941 * 12] {
        let h = month_start(m).unwrap();
        assert_eq!(month(h), Some(m));
        if m > 0 {
            assert_eq!(month(h - 1), Some(m - 1));
        }
    }
}

#[test]
fn naval_losses_both_uses_one_baseline_and_month_values_wait_for_their_read() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.first_convoy_read = Some(first + 10);
    scanned.ships = vec![ship(first + 5, 0, 0), ship(first + 11, 0, 0)];
    let built = reply(&scanned, &params(), &AtomicBool::new(false)).unwrap();
    assert_eq!(built.first_read, Some(first + 10));
    assert_eq!(
        sample(&built, "mod maritime country", first + 11).0,
        Some(0)
    );
    let mut monthly_params = params();
    monthly_params.period = NavalPeriod::Month;
    let monthly = reply(&scanned, &monthly_params, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&monthly, "mod maritime country", first + 11),
        (Some(0), Some(0)),
        "monthly parts use their own coverage, while running Both uses a common baseline"
    );
    assert_eq!(
        sample(&monthly, "mod maritime country", first + 24),
        (Some(2), Some(0))
    );
    scanned.first_convoy_read = Some(first);
    let next = month_start(1940 * 12 + 1).unwrap();
    let mut late = convoy(next + 1, 3, false);
    late.month -= 1;
    scanned.convoys.push(late);
    let mut p = params();
    p.period = NavalPeriod::Month;
    p.tags.push("opponent".into());
    let monthly = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&monthly, "mod maritime country", first + 20).1,
        Some(0)
    );
    assert_eq!(sample(&monthly, "opponent", first + 20).1, Some(0));
    assert_eq!(
        sample(
            &reply(&scanned, &params(), &AtomicBool::new(false)).unwrap(),
            "mod maritime country",
            next + 1
        )
        .1,
        Some(3)
    );
    scanned.convoys[0].owner = None;
    let monthly = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&monthly, "opponent", first + 20).1,
        Some(0),
        "an unresolved owner cannot poison an unrelated country"
    );
}

#[test]
fn naval_losses_convoy_delta_crossing_a_later_ship_baseline_is_unknown() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.first_ship_read = Some(first + 10);
    scanned.convoys = vec![
        convoy(first, 100, true),
        convoy(first + 24, 5, false),
        convoy(first + 48, 2, false),
    ];
    scanned.convoys[1].previous_read = Some(first);
    scanned.convoys[2].previous_read = Some(first + 24);
    let both = reply(&scanned, &params(), &AtomicBool::new(false)).unwrap();
    assert_eq!(both.first_read, Some(first + 10));
    assert_eq!(sample(&both, "mod maritime country", first + 10).1, Some(0));
    for hour in [first + 24, first + 48] {
        assert_eq!(sample(&both, "mod maritime country", hour).1, None);
    }
    assert!(both.warnings.iter().any(|w| w.contains("straddle")));
    let mut p = params();
    p.kind = NavalKind::Convoys;
    let only_convoys = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&only_convoys, "mod maritime country", first + 48).1,
        Some(7)
    );
    p.kind = NavalKind::Ships;
    let only_ships = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert!(!only_ships.warnings.iter().any(|w| w.contains("straddle")));
    p.kind = NavalKind::Both;
    p.period = NavalPeriod::Month;
    let monthly = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&monthly, "mod maritime country", first + 48).1,
        Some(107),
        "month buckets include the initial recorded bucket"
    );
}

#[test]
fn naval_losses_retention_content_and_transport_refusal_are_explicit() {
    use dashboard_text::catalogue::tests::folder_catalogue;
    let mut scanned = fixture();
    let first = scanned.first_hour;
    let (cat, _folder) = folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines={NNavy={NAVAL_COMBAT_RESULT_TIMEOUT_YEARS=2, CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS=7}}",
        ),
        (
            "common/units/mod.txt",
            "sub_units={mod_sea_skimmer={type=naval}}",
        ),
        (
            "map/strategicregions/mod.txt",
            "strategic_region={id=54321 name=renamed_mod_sea provinces={54321}}",
        ),
    ]);
    let safe = sinkings::retention(&scanned, Some(&cat));
    assert!(safe.ships && safe.convoys);
    scanned.ships.push(ship(first + 1, 0, 0));
    let built = reply_with_retention(
        &scanned,
        &params(),
        &AtomicBool::new(false),
        (safe.ships, safe.convoys),
        (&safe.ship_loss, &safe.convoy_loss),
        safe.warnings.clone(),
    )
    .unwrap();
    assert_eq!(
        sample(&built, "mod maritime country", first + 13).0,
        Some(1)
    );
    let built = reply_with_retention(
        &scanned,
        &params(),
        &AtomicBool::new(false),
        (false, false),
        (&[], &[]),
        vec!["retention unavailable".into()],
    )
    .unwrap();
    assert_eq!(
        sample(&built, "mod maritime country", first + 1),
        (None, None)
    );
    scanned.warnings.push("x".repeat(MAX_PRODUCT_REPLY_BYTES));
    assert!(
        reply(&scanned, &params(), &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("transport budget")
    );
}

#[test]
fn naval_losses_retention_taints_only_possible_loss_months_and_running_suffix() {
    use dashboard_text::catalogue::tests::folder_catalogue;
    let (cat, _folder) = folder_catalogue(&[(
        "common/defines/mod.lua",
        "NDefines={NNavy={NAVAL_COMBAT_RESULT_TIMEOUT_YEARS=1, CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS=3}}",
    )]);
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.ships = vec![ship(first + 10, 0, 0)];
    scanned.ship_unread = vec![HourSpan {
        first_hour: first + 100,
        last_hour: first + 8859,
    }];
    scanned.convoy_unread = vec![HourSpan {
        first_hour: first + 100,
        last_hour: month_start(1940 * 12 + 4).unwrap(),
    }];
    scanned.ship_breaks = scanned.ship_unread.clone();
    scanned.convoy_breaks = scanned.convoy_unread.clone();
    scanned.last_hour = first + 17_000;
    for spans in scanned.segments.values_mut() {
        spans[0].last_hour = scanned.last_hour;
    }
    for spans in scanned.country_identities.values_mut() {
        spans[0].last_hour = scanned.last_hour;
    }
    let coverage = sinkings::retention(&scanned, Some(&cat));
    assert_eq!(
        coverage.ship_loss,
        [HistoryLoss {
            first_hour: first + 100,
            last_hour: first + 100,
            known_at: first + 8860
        }]
    );
    assert_eq!(
        coverage.convoy_loss,
        [HistoryLoss {
            first_hour: first,
            last_hour: month_start(1940 * 12 + 1).unwrap() - 1,
            known_at: scanned.convoy_breaks[0].last_hour + 1
        }]
    );
    let running = reply_with_retention(
        &scanned,
        &params(),
        &AtomicBool::new(false),
        (coverage.ships, coverage.convoys),
        (&coverage.ship_loss, &coverage.convoy_loss),
        coverage.warnings.clone(),
    )
    .unwrap();
    assert_eq!(
        sample(&running, "mod maritime country", first + 22),
        (Some(1), Some(0))
    );
    assert_eq!(
        sample(&running, "mod maritime country", scanned.last_hour),
        (None, None)
    );
    let mut p = params();
    p.period = NavalPeriod::Month;
    let monthly = reply_with_retention(
        &scanned,
        &p,
        &AtomicBool::new(false),
        (coverage.ships, coverage.convoys),
        (&coverage.ship_loss, &coverage.convoy_loss),
        coverage.warnings,
    )
    .unwrap();
    assert_eq!(
        sample(&monthly, "mod maritime country", first + 30),
        (Some(1), Some(0)),
        "a future retention loss cannot taint a past read"
    );
    assert_eq!(
        sample(&monthly, "mod maritime country", scanned.last_hour),
        (Some(0), Some(0))
    );
    scanned.ship_unread[0].last_hour -= 1;
    scanned.convoy_unread[0].last_hour = month_start(1940 * 12 + 3).unwrap();
    scanned.ship_breaks = scanned.ship_unread.clone();
    scanned.convoy_breaks = scanned.convoy_unread.clone();
    let shorter = sinkings::retention(&scanned, Some(&cat));
    assert!(
        shorter.ship_loss.is_empty() && shorter.convoy_loss.is_empty(),
        "native equality retains the convoy bucket"
    );
}

#[test]
fn naval_losses_gap_attribution_uncertainty_does_not_poison_other_countries() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned
        .uncertain_victims
        .insert((1, 1), vec![(first + 20, first + 25)]);
    scanned.ships.push(ship(first + 10, 0, 0));
    let mut p = params();
    p.tags.push("opponent".into());
    for period in [NavalPeriod::Running, NavalPeriod::Month] {
        p.period = period;
        let built = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            sample(&built, "mod maritime country", first + 24).0,
            Some(1)
        );
        assert_eq!(sample(&built, "mod maritime country", first + 30).0, None);
        assert_eq!(sample(&built, "opponent", first + 30).0, Some(0));
    }
}

#[test]
fn naval_losses_trailing_hours_hold_totals_as_of_last_reads() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.last_ship_read = Some(first + 24);
    scanned.last_convoy_read = Some(first + 25);
    scanned.ship_reads = vec![first, first + 24];
    scanned.convoy_reads = vec![first, first + 25];
    scanned.ships.push(ship(first + 2, 0, 0));
    scanned.convoys.push(convoy(first + 25, 3, false));
    let built = reply(&scanned, &params(), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&built, "mod maritime country", scanned.last_hour),
        (Some(1), Some(3))
    );
    assert_eq!(built.ship_last_read, Some(first + 24));
    assert_eq!(built.convoy_last_read, Some(first + 25));
    assert_eq!(built.ship_reads, [first, first + 24]);
    assert_eq!(built.convoy_reads, [first, first + 25]);
}

#[test]
fn naval_losses_waits_for_ship_read_across_the_calendar_month_boundary() {
    let mut scanned = fixture();
    let next = month_start(1940 * 12 + 1).unwrap();
    let mut event = ship(next - 2, 0, 0);
    event.read_hour = next + 10;
    scanned.ships.push(event);
    let mut p = params();
    p.kind = NavalKind::Ships;
    for period in [NavalPeriod::Running, NavalPeriod::Month] {
        p.period = period;
        let built = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
        assert_eq!(sample(&built, "mod maritime country", next - 1).0, Some(0));
        assert_eq!(sample(&built, "mod maritime country", next + 9).0, Some(0));
        assert_eq!(
            sample(&built, "mod maritime country", next + 10).0,
            Some(if period == NavalPeriod::Running { 1 } else { 0 }),
            "the supplying read reveals the ship without moving its sinking month"
        );
    }
}

#[test]
fn naval_losses_refuses_negative_counts_outside_javascript_precision() {
    let mut scanned = fixture();
    scanned.convoys.push(convoy(
        scanned.first_hour + 24,
        -JS_MAX_SAFE_INTEGER - 1,
        false,
    ));
    let mut p = params();
    p.kind = NavalKind::Convoys;
    assert!(
        reply(&scanned, &p, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("MAX_SAFE_INTEGER")
    );
}

#[test]
fn naval_losses_follow_month_before_a_late_read_does_not_see_its_losses() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.ships.push(ship(first + 5, 0, 0));
    scanned.convoys.push(convoy(first + 24, 7, false));
    let mut p = params();
    p.period = NavalPeriod::Month;
    let built = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&built, "mod maritime country", first + 10),
        (Some(0), Some(0))
    );
    assert_eq!(
        sample(&built, "mod maritime country", first + 17),
        (Some(1), Some(0))
    );
    assert_eq!(
        sample(&built, "mod maritime country", first + 24),
        (Some(1), Some(7))
    );
}

#[test]
fn naval_losses_convoy_read_at_the_both_baseline_is_excluded_without_taint() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.first_ship_read = Some(first + 10);
    scanned.convoys = vec![convoy(first + 10, 5, false), convoy(first + 24, 3, false)];
    scanned.convoys[0].previous_read = Some(first);
    scanned.convoys[1].previous_read = Some(first + 10);
    let built = reply(&scanned, &params(), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&built, "mod maritime country", first + 10).1,
        Some(0)
    );
    assert_eq!(
        sample(&built, "mod maritime country", first + 24).1,
        Some(3)
    );
    assert!(!built.warnings.iter().any(|w| w.contains("straddle")));
}

#[test]
fn naval_losses_first_retained_month_includes_ships_before_the_first_read() {
    let mut scanned = fixture();
    let start = scanned.first_hour;
    let first = start + 120;
    scanned.first_hour = first;
    scanned.first_ship_read = Some(first);
    scanned.first_convoy_read = Some(first);
    for spans in scanned.segments.values_mut() {
        spans[0].first_hour = first;
    }
    for spans in scanned.country_identities.values_mut() {
        spans[0].first_hour = first;
    }
    let mut retained = ship(start + 20, 0, 0);
    retained.read_hour = first;
    retained.victim_as_of = Some(first);
    retained.attacker_as_of = Some(first);
    scanned.ships.push(retained);
    scanned
        .country_candidates
        .get_mut(&1)
        .unwrap()
        .insert((1, 91));
    let mut p = params();
    p.period = NavalPeriod::Month;
    let built = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(sample(&built, "mod maritime country", first).0, Some(1));
    p.attribution = NavalAttribution::SunkBy;
    p.tags = vec!["opponent".into()];
    let sunk_by = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(sample(&sunk_by, "opponent", first).0, Some(1));
    let running = reply(&scanned, &params(), &AtomicBool::new(false)).unwrap();
    assert_eq!(sample(&running, "mod maritime country", first).0, Some(0));
}

#[test]
fn naval_losses_attributes_ship_totals_at_the_country_naming_read() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    scanned.segments.get_mut("mod maritime country").unwrap()[0].first_hour = first + 12;
    scanned
        .country_identities
        .get_mut("mod maritime country")
        .unwrap()[0]
        .first_hour = first + 12;
    let mut retained = ship(first + 5, 0, 0);
    retained.read_hour = first + 20;
    retained.victim_as_of = Some(first + 20);
    scanned.ships.push(retained);
    let mut p = params();
    p.kind = NavalKind::Ships;
    let running = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&running, "mod maritime country", first + 30).0,
        Some(1)
    );
    p.period = NavalPeriod::Month;
    let monthly = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        sample(&monthly, "mod maritime country", first + 19).0,
        Some(0)
    );
    assert_eq!(
        sample(&monthly, "mod maritime country", first + 20).0,
        Some(1)
    );
}

#[test]
fn naval_losses_uncertain_convoy_holders_are_unknown_only_from_the_read() {
    let mut scanned = fixture();
    let first = scanned.first_hour;
    let mut unresolved = convoy(first + 30, 3, false);
    unresolved.owner = None;
    scanned.convoys.push(unresolved);
    scanned
        .uncertain_convoy_owners
        .insert((1, 1), vec![(month(first).unwrap(), first + 30)]);
    let mut p = params();
    p.tags.push("opponent".into());
    for period in [NavalPeriod::Running, NavalPeriod::Month] {
        p.period = period;
        let built = reply(&scanned, &p, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            sample(&built, "mod maritime country", first + 20).1,
            Some(0)
        );
        assert_eq!(sample(&built, "mod maritime country", first + 30).1, None);
        assert_eq!(sample(&built, "opponent", first + 30).1, Some(0));
    }
    scanned.country_candidates.clear();
    assert!(
        reply(&scanned, &p, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("cannot identify possible holders of convoy country reference 1")
    );
    scanned.convoys.clear();
    let mut unresolved = ship(first + 30, 0, 0);
    unresolved.victim = None;
    scanned.ships.push(unresolved);
    assert!(
        reply(&scanned, &p, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("cannot identify possible holders of ship country reference 1")
    );
}
