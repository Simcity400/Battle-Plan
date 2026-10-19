use super::{RecordedPortrait, leader_portrait};
use dashboard_base::test_fixtures::{CAMPAIGN, ownership_frame, records};
use replay::container::Replay;
use replay_data::{
    rows::CharacterPortrait, rows::CharacterPortraitRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::atomic::AtomicBool;

#[test]
fn a_leader_shows_the_civilian_large_entry_or_the_first_large_one_when_set() {
    let entry = |category, size, is_set, name: &str| CharacterPortrait {
        category,
        size,
        is_set,
        kind: 0,
        identity: name.as_bytes().to_vec(),
    };
    let row = |character, portraits| CharacterPortraitRow {
        character,
        portraits,
    };
    let holders = [
        row(
            1,
            vec![entry(0, 0, 1, "civ_small"), entry(0, 1, 1, "civ_large")],
        ),
        // No civilian large: the first large entry, as the game's search returns it.
        row(
            2,
            vec![entry(0, 0, 1, "civ_small"), entry(1, 1, 1, "army_large")],
        ),
        // An unset entry is not tried further: the game generates one.
        row(
            3,
            vec![entry(0, 1, 0, "unset"), entry(1, 1, 1, "army_large")],
        ),
        row(4, vec![entry(0, 0, 1, "civ_small")]),
    ];
    let named = |name: &str| RecordedPortrait::Entry {
        kind: 0,
        identity: name.as_bytes().to_vec(),
    };
    assert_eq!(leader_portrait(Some(&holders), 1), named("civ_large"));
    assert_eq!(leader_portrait(Some(&holders), 2), named("army_large"));
    assert_eq!(
        leader_portrait(Some(&holders), 3),
        RecordedPortrait::Generated
    );
    assert_eq!(
        leader_portrait(Some(&holders), 4),
        RecordedPortrait::Generated
    );
    // A holder this hour did not record is not a generated portrait.
    assert_eq!(
        leader_portrait(Some(&holders), 5),
        RecordedPortrait::Unrecorded
    );
    assert_eq!(leader_portrait(None, 1), RecordedPortrait::Unrecorded);
}

/// GER (link 9001) with two parties at the given popularities (raw), one ruling,
/// and its last election hour.
pub(super) fn party_frame(
    hour: i32,
    fascism: i64,
    democratic: i64,
    ruling: &str,
    election: i32,
) -> DecodedFrame {
    let mut frame = ownership_frame(1, hour, 44);
    let party = |id: u32, token: &str, popularity: i64| replay_data::rows::PoliticalPartyRow {
        country: 9001,
        id,
        popularity,
        is_ruling: u8::from(token == ruling),
        tag: 9001,
        name_default: 1,
        token: token.as_bytes().to_vec(),
        ..Default::default()
    };
    let mut parties = vec![
        party(1, "democratic", democratic),
        party(2, "fascism", fascism),
        // Another country's party is never GER's.
        replay_data::rows::PoliticalPartyRow {
            country: 44,
            id: 1,
            popularity: 1,
            is_ruling: 1,
            token: b"democratic".to_vec(),
            ..Default::default()
        },
    ];
    parties.sort_by_key(|row| (row.country, row.id));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(parties));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay_data::rows::PoliticsRow {
            country: 9001,
            last_election: election,
            ..Default::default()
        }]));
    frame
}

#[test]
fn party_history_follows_popularity_ruling_changes_and_elections() {
    use crate::products::party_history::{RulingRun, scan};
    let frames = vec![
        party_frame(100, 40_00000, 35_00000, "fascism", 10),
        party_frame(101, 40_00000, 35_00000, "fascism", 10),
        party_frame(102, 45_00000, 30_00000, "fascism", 10),
        // Hour 103 is missing; at 104 an election was held and the democrats rule.
        party_frame(104, 20_00000, 60_00000, "democratic", 104),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let fascism = &scanned.parties["fascism"];
    assert_eq!(
        fascism
            .runs
            .iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.value_raw.as_str(),
                run.starts_segment
            ))
            .collect::<Vec<_>>(),
        [
            (100, 101, "4000000", true),
            (102, 102, "4500000", false),
            (104, 104, "2000000", true)
        ]
    );
    let ruling = |token: &str, first_hour, last_hour| RulingRun {
        token: token.into(),
        first_hour,
        last_hour,
        starts_segment: true,
    };
    assert_eq!(
        scanned.ruling,
        [ruling("fascism", 100, 102), ruling("democratic", 104, 104)]
    );

    assert_eq!(scanned.elections, [104]);
    assert_eq!(
        scanned.unobserved,
        [dashboard_base::products::country_groups::HourSpan {
            first_hour: 103,
            last_hour: 103
        }]
    );
}

#[test]
fn party_history_all_country_scan_keeps_every_country_and_segment_for_each_country() {
    use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::PoliticalPartyRow};
    let make = |hour, changed: bool| {
        let mut frame = ownership_frame(1, hour, 44);
        let countries: Vec<_> = (1..=4)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("opaque-{id}").into_bytes(),
                ..Default::default()
            })
            .collect();
        let links = countries
            .iter()
            .map(|row| CountryLinkRow {
                id: row.id,
                canonical_slot: row.canonical_slot,
            })
            .collect();
        let parties = countries
            .iter()
            .map(|row| PoliticalPartyRow {
                country: row.id,
                id: 1,
                tag: row.id,
                is_ruling: 1,
                token: if changed {
                    b"mod-rule-b".to_vec()
                } else {
                    b"mod-rule-a".to_vec()
                },
                popularity: 1,
                ..Default::default()
            })
            .collect();
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links));
        *frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(parties));
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![make(100, false), make(101, true), make(103, false)],
        )],
    )
    .unwrap();
    let scanned = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.countries.len(), 4);
    assert_eq!(scanned.refusals.len(), 1);
    assert_eq!(scanned.refusals[0].1, 4);
    for (tag, country) in &scanned.countries {
        assert_eq!(
            country
                .ruling
                .iter()
                .map(|run| (run.first_hour, run.starts_segment))
                .collect::<Vec<_>>(),
            [(100, true), (101, false), (103, true)]
        );
        assert_eq!(
            country.unobserved,
            [dashboard_base::products::country_groups::HourSpan {
                first_hour: 102,
                last_hour: 102,
            }]
        );
        assert_eq!(country.namings[1].tag.as_deref(), Some(tag.as_str()));
        assert!(country.parties.is_empty());
        assert!(country.leaders.is_empty());
        assert!(country.people.is_empty());
        assert!(country.elections.is_empty());
    }
    let selected =
        super::scan_selected(&replay, 1, Some("opaque-3"), &AtomicBool::new(false)).unwrap();
    assert_eq!(selected.countries.len(), 1);
    assert!(selected.refusals.is_empty());
    let country = selected.country("opaque-3");
    assert_eq!(country.parties.len(), 2);
    assert_eq!(country.ruling, scanned.country("opaque-3").ruling);
    assert_eq!(
        super::scan(&replay, 1, "opaque-3", &AtomicBool::new(false)).unwrap(),
        *country
    );
}

#[test]
fn party_history_feed_counts_unresolved_tokens_and_gap_observations() {
    let mut unresolved = party_frame(101, 1, 2, "fascism", 10);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *unresolved.tables[TableId::CountryLinks.index()]
    {
        rows.retain(|row| row.id != 9001);
    }
    let mut invalid = party_frame(103, 1, 2, "democratic", 10);
    if let TableView::Complete(TableRows::PoliticalParties(rows)) =
        &mut *invalid.tables[TableId::PoliticalParties.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 9001 && row.is_ruling != 0)
            .unwrap()
            .token = vec![0xff];
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                party_frame(100, 1, 2, "fascism", 10),
                unresolved,
                party_frame(102, 1, 2, "democratic", 10),
                invalid,
                party_frame(104, 1, 2, "democratic", 10),
            ],
        )],
    )
    .unwrap();
    let branch = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(branch.refusals.len(), 3);
    assert!(branch.refusals.iter().all(|(_, count)| *count == 1));
    assert!(
        branch
            .refusals
            .iter()
            .all(|(reason, _)| !reason.contains("GER:"))
    );
    assert!(
        branch
            .refusals
            .iter()
            .any(|(reason, _)| reason.contains("unobserved hours"))
    );
    assert!(
        branch
            .refusals
            .iter()
            .any(|(reason, _)| reason.contains("CountryLinks"))
    );
    assert!(
        branch
            .refusals
            .iter()
            .any(|(reason, _)| reason.contains("non-UTF-8"))
    );
    let topic = super::scan_selected(&replay, 1, Some("GER"), &AtomicBool::new(false)).unwrap();
    assert!(topic.refusals.is_empty());
    assert!(
        topic
            .country("GER")
            .warnings
            .iter()
            .any(|warning| warning.contains("count: 1"))
    );
    assert_eq!(topic.country("GER").ruling, branch.country("GER").ruling);
}

#[test]
fn party_history_topic_counts_invalid_non_ruling_tokens_without_recounting_each_read() {
    let mut first = party_frame(100, 1, 2, "fascism", 10);
    if let TableView::Complete(TableRows::PoliticalParties(rows)) =
        &mut *first.tables[TableId::PoliticalParties.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 9001 && row.is_ruling == 0)
            .unwrap()
            .token = vec![0xff];
        rows.push(replay_data::rows::PoliticalPartyRow {
            country: 9001,
            id: 3,
            ..Default::default()
        });
        rows.sort_by_key(|row| (row.country, row.id));
    }
    let mut second = first.clone();
    second.hour = 101;
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, second])]).unwrap();
    let topic = super::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(topic.parties.len(), 1);
    assert_eq!(topic.warnings.len(), 1);
    assert!(topic.warnings[0].contains("PoliticalParties ideology tokens"));
    assert!(topic.warnings[0].contains("count: 2"));
}

#[test]
fn party_history_feed_extends_unchanged_views_and_only_stores_changed_names() {
    let mut initial = party_frame(100, 1, 2, "fascism", 10);
    *initial.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay_data::rows::CountryAppearanceRow {
                country: 9001,
                original_tag_reference: replay_data::rows::Recorded::Value(Some(55)),
                original_tag: replay_data::rows::Recorded::Value(Some(b"mod_original".to_vec())),
                ..Default::default()
            },
        ]));
    let mut next = initial.clone();
    next.hour = 101;
    let mut renamed = next.clone();
    renamed.hour = 102;
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *renamed.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag = replay_data::rows::Recorded::Value(Some(b"mod_renamed".to_vec()));
    }
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![initial, next, renamed])]).unwrap();
    let feed = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    let country = feed.country("GER");
    assert_eq!(country.ruling.len(), 1);
    assert_eq!(
        (country.ruling[0].first_hour, country.ruling[0].last_hour),
        (100, 102)
    );
    assert_eq!(country.namings.len(), 2);
    assert_eq!(
        country
            .party_name_runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101), (102, 102)]
    );
    assert_eq!(
        super::scan(&replay, 1, "GER", &AtomicBool::new(false))
            .unwrap()
            .party_name_runs,
        country.party_name_runs
    );
}

#[test]
fn party_history_feed_counts_each_unchanged_unresolved_hour() {
    let mut unresolved = party_frame(101, 1, 2, "fascism", 10);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *unresolved.tables[TableId::CountryLinks.index()]
    {
        rows.retain(|row| row.id != 9001);
    }
    let mut unchanged = unresolved.clone();
    unchanged.hour = 102;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                party_frame(100, 1, 2, "fascism", 10),
                unresolved,
                unchanged,
                party_frame(103, 1, 2, "democratic", 10),
            ],
        )],
    )
    .unwrap();
    let feed = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        feed.refusals
            .iter()
            .find(|(reason, _)| reason.contains("CountryLinks"))
            .unwrap()
            .1,
        2
    );
    assert_eq!(
        feed.country("GER").unobserved,
        [dashboard_base::products::country_groups::HourSpan {
            first_hour: 101,
            last_hour: 102
        }]
    );
}

#[test]
fn party_history_topic_combines_table_and_identity_gaps_in_hour_order() {
    let mut missing_parties = party_frame(101, 1, 2, "fascism", 10);
    *missing_parties.tables[TableId::PoliticalParties.index()] = TableView::Unavailable {
        reason: "synthetic missing parties".into(),
    };
    let mut missing_links = party_frame(104, 1, 2, "democratic", 10);
    *missing_links.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "synthetic missing identity".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                party_frame(100, 1, 2, "fascism", 10),
                missing_parties,
                party_frame(103, 1, 2, "democratic", 10),
                missing_links,
                party_frame(106, 1, 2, "fascism", 10),
            ],
        )],
    )
    .unwrap();
    let topic = super::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(
        topic.unobserved,
        [
            dashboard_base::products::country_groups::HourSpan {
                first_hour: 101,
                last_hour: 102
            },
            dashboard_base::products::country_groups::HourSpan {
                first_hour: 104,
                last_hour: 105
            },
        ]
    );
    assert_eq!(
        topic
            .ruling
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
            .collect::<Vec<_>>(),
        [(100, 100, true), (103, 103, true), (106, 106, true)]
    );
}

#[test]
fn widget_party_names_resolve_party_aliases_exact_empty_and_reserved_tags_lazily() {
    use replay_data::{
        rows::CountryAppearanceRow, rows::CountryLinkRow, rows::Recorded,
        rows::ReservedCountryNamingRow,
    };
    let mut initial = party_frame(100, 1, 2, "fascism", 10);
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *initial.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 123,
            canonical_slot: 8,
        });
        rows.sort_by_key(|row| row.id);
    }
    if let TableView::Complete(TableRows::PoliticalParties(rows)) =
        &mut *initial.tables[TableId::PoliticalParties.index()]
    {
        for row in rows.iter_mut().filter(|row| row.country == 9001) {
            row.tag = 123;
        }
    }
    initial.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            CountryAppearanceRow {
                country: 44,
                name_tag: b"misleading_dynamic_tag".to_vec(),
                original_tag_reference: Recorded::Value(Some(55)),
                original_tag: Recorded::Value(Some(vec![])),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 9001,
                original_tag_reference: Recorded::Value(Some(55)),
                original_tag: Recorded::Value(Some(b"wrong_owner_original".to_vec())),
                ..Default::default()
            },
        ]))
        .into();
    // Unrelated map operands must not gate a recorded party name.
    initial.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unrelated map failure".into(),
    }
    .into();
    let mut missing = initial.clone();
    missing.hour = 101;
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *missing.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(None);
    }
    let mut reserved = missing.clone();
    reserved.hour = 102;
    if let TableView::Complete(TableRows::PoliticalParties(rows)) =
        &mut *reserved.tables[TableId::PoliticalParties.index()]
    {
        for row in rows.iter_mut().filter(|row| row.country == 9001) {
            row.tag = 0;
        }
    }
    reserved.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"reserved_native".to_vec(),
                original_tag_reference: Recorded::Value(Some(55)),
                original_tag: Recorded::Value(Some(vec![])),
                ..Default::default()
            },
        ]))
        .into();
    let mut custom = reserved.clone();
    custom.hour = 103;
    if let TableView::Complete(TableRows::PoliticalParties(rows)) =
        &mut *custom.tables[TableId::PoliticalParties.index()]
    {
        for row in rows.iter_mut().filter(|row| row.country == 9001) {
            row.tag = 9876;
            row.name_default = 0;
            row.name = b"stored_custom".to_vec();
        }
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![initial, missing, reserved, custom])],
    )
    .unwrap();
    let feed = super::scan_all(&replay, 1, &AtomicBool::new(false)).unwrap();
    let scanned = feed.country("GER");
    let keys = |hour, defined: &str| {
        let run = scanned
            .party_name_runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .unwrap();
        super::recorded_party_key(scanned, run.name as usize, |key| key == defined)
            .map(|(key, _)| key)
    };
    assert_eq!(keys(100, "_fascism_party").as_deref(), Ok("_fascism_party"));
    assert!(
        keys(101, "_fascism_party")
            .unwrap_err()
            .contains("original_tag_reference")
    );
    assert_eq!(
        keys(101, "FRA_fascism_party").as_deref(),
        Ok("FRA_fascism_party")
    );
    assert_eq!(keys(102, "_fascism_party").as_deref(), Ok("_fascism_party"));
    assert_eq!(keys(103, "").as_deref(), Ok("stored_custom"));
    let topic = super::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(topic.party_name_runs, scanned.party_name_runs);
    assert_eq!(topic.selected_tags, scanned.selected_tags);
}

fn check_party_country_delta_work(count: u32) {
    use super::TopicInputs;
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay_data::{format::FormatVersion, rows::CountryLinkRow, rows::CountryRow};
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    use std::sync::Arc;
    let format = FormatVersion::CURRENT;
    let mut frame = dashboard_base::test_fixtures::source(1, 100, None);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("realm {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(Vec::new()));
    let mut states = [TopicInputs::default(), TopicInputs::default()];
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 100..=106 {
        frame.hour = hour;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(i64::from(hour));
            if hour == 102 {
                rows[0].tag = b"changed alias".to_vec();
            }
            if hour == 105 {
                rows.pop();
            }
        }
        let wire = Arc::new(
            encoder
                .encode(frame.clone(), hour == 100 || hour == 104)
                .unwrap(),
        );
        let decoded = decoder.advance(&wire).unwrap();
        let changes = trackers[0].observe_updates(decoded, Arc::clone(&wire), format);
        let whole = trackers[1].observe(decoded, format);
        let before = states[0].keys.as_ref().map_or(0, |keys| keys.row_visits());
        let dirty = states[0].refresh(decoded, Some(&changes));
        let reference = states[1].refresh(decoded, Some(&whole));
        assert_eq!(dirty, reference, "dirty holders at {hour}");
        assert_eq!(states[0].holders, states[1].holders);
        // A kept reduced view can be reported rebuilt while its key allocation holds.
        let visits = states[0].keys.as_ref().unwrap().row_visits();
        states[0].refresh(decoded, Some(&changes));
        assert_eq!(states[0].keys.as_ref().unwrap().row_visits(), visits);
        if hour == 101 {
            let visits = states[0].keys.as_ref().unwrap().row_visits() - before;
            assert_eq!(
                visits, 1,
                "only the changed country's field mask is compared"
            );
        }
    }
}

#[test]
fn party_country_balance_deltas_visit_only_changed_keys() {
    check_party_country_delta_work(64);
}

#[test]
fn party_country_balance_delta_work_at_5000_countries() {
    check_party_country_delta_work(5_000);
}
