use super::*;
use crate::products::army_size::{prepared, test_fixtures};
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    prepass::extract_alone,
    series::tests::{Fixture, assert_payloads_equal},
};
use replay::container::{Replay, ReplayStream};
use replay_data::rows::{CountryCounterInputRow, SentVolunteerRow, TableRows, VolunteerHostRow};
use replay_frame::frame::TableView;
use std::sync::Arc;

fn scenario(hour: i32) -> DecodedFrame {
    let mut frame = test_fixtures::frame(hour);
    *frame.tables[TableId::SentVolunteers.index()] =
        TableView::Complete(TableRows::SentVolunteers(
            [1, 1, 0, u64::MAX]
                .into_iter()
                .enumerate()
                .map(|(at, division)| SentVolunteerRow {
                    country: 11,
                    ordinal: at as u32,
                    division,
                })
                .collect(),
        ));
    *frame.tables[TableId::VolunteerHosts.index()] =
        TableView::Complete(TableRows::VolunteerHosts(vec![
            VolunteerHostRow {
                country: 10,
                position: 0,
                host: 20,
                deployed_divisions: -3,
                ..Default::default()
            },
            VolunteerHostRow {
                country: 10,
                position: 1,
                host: 20,
                deployed_divisions: 99,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                country: 10,
                sent_expeditionaries: Some(vec![1, 1, 0, 99].into()).into(),
                ..Default::default()
            },
            CountryCounterInputRow {
                country: 20,
                sent_expeditionaries: Some(Vec::new().into()).into(),
                ..Default::default()
            },
            CountryCounterInputRow {
                country: 30,
                sent_expeditionaries: None.into(),
                ..Default::default()
            },
        ]));
    frame
}
#[test]
fn military_aid_native_occurrences_aliases_tallies_and_independent_reads() {
    let frame = scenario(100);
    let point = direct(&frame);
    let origin = point
        .senders
        .iter()
        .find(|sender| sender.actor.tag == "origin")
        .unwrap();
    assert_eq!(
        origin.channels[0]
            .values
            .values()
            .copied()
            .collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(
        origin.channels[1]
            .values
            .values()
            .copied()
            .collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(
        origin.channels[2]
            .values
            .values()
            .copied()
            .collect::<Vec<_>>(),
        [-3]
    );
    assert_eq!(origin.hosts.len(), 2);
    assert_eq!(origin.volunteers.len(), 4);
    assert_eq!(origin.expeditionaries.as_ref().unwrap().len(), 4);
    let empty = point
        .senders
        .iter()
        .find(|sender| sender.actor.tag == "empty")
        .unwrap();
    assert!(empty.channels[0].complete && empty.channels[0].values.is_empty());
    assert!(!empty.channels[1].complete);
    let mut unavailable = frame.clone();
    *unavailable.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "synthetic refusal".into(),
    };
    let point = direct(&unavailable);
    let origin = point
        .senders
        .iter()
        .find(|sender| sender.actor.tag == "origin")
        .unwrap();
    assert!(!origin.channels[0].complete && !origin.channels[1].complete);
    assert!(origin.channels[2].complete);
}
#[test]
fn military_aid_ambiguous_lists_noncontiguous_ordinals_and_wrong_owner_refuse_only_dependents() {
    let mut frame = scenario(100);
    if let TableView::Complete(TableRows::SentVolunteers(rows)) =
        &mut *frame.tables[TableId::SentVolunteers.index()]
    {
        rows.insert(
            0,
            SentVolunteerRow {
                country: 10,
                ordinal: 0,
                division: 1,
            },
        );
    }
    let point = direct(&frame);
    let origin = &point.senders[0];
    assert!(!origin.channels[0].complete);
    assert!(origin.channels[1].complete && origin.channels[2].complete);
    let mut frame = scenario(100);
    if let TableView::Complete(TableRows::SentVolunteers(rows)) =
        &mut *frame.tables[TableId::SentVolunteers.index()]
    {
        rows.remove(0);
    }
    assert!(!direct(&frame).senders[0].channels[0].complete);
    let mut frame = scenario(100);
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    {
        rows[0].expeditionary_owner = u32::MAX;
    }
    let point = direct(&frame);
    assert!(point.senders[0].channels[0].complete);
    assert!(!point.senders[0].channels[1].complete);
}
#[test]
fn military_aid_block_folds_resume_append_and_current_queries_agree() {
    use dashboard_base::test_fixtures::{CAMPAIGN, continuation_records, records};
    let flag = AtomicBool::new(false);
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (100..104).map(scenario).collect())],
    )
    .unwrap();
    for from in [104, 110, 114] {
        let frames = (from..from + 4)
            .map(|hour| {
                let mut frame = scenario(hour);
                if hour == 105 {
                    *frame.tables[TableId::SentVolunteers.index()] =
                        TableView::Complete(TableRows::SentVolunteers(Vec::new()));
                }
                if hour == 112 {
                    *frame.tables[TableId::VolunteerHosts.index()] = TableView::Unavailable {
                        reason: "native tally unavailable".into(),
                    };
                }
                frame
            })
            .collect();
        replay = replay
            .append(&[continuation_records(&replay, 1, frames)])
            .unwrap();
    }
    let stores = [Fixture::new(), Fixture::new()];
    let mut states = [prepared::Prepared::default(), prepared::Prepared::default()];
    let mut stream = ReplayStream::new(prepared::TABLES).unwrap();
    let mut tracker = FrameChangeTracker::new();
    for index in &replay.index {
        tracker.invalidate();
        stream
            .next_block_with(
                &replay,
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                    states[0].observe(frame, &replay, &changes)
                },
                || Ok(()),
            )
            .unwrap();
        let facts = extract_alone(&replay, index, &prepared::FAMILY).unwrap();
        states[1]
            .fold(facts, &replay, index, &|_, result| result, &mut || Ok(()))
            .map_err(|failure| failure.error)
            .unwrap();
        for (state, store) in states.iter_mut().zip(&stores) {
            let mut writer = store.writer();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &stores[0].payloads(),
            &stores[1].payloads(),
            "military aid folded pages",
        );
        assert_eq!(
            serde_json::to_value(&states[0]).unwrap(),
            serde_json::to_value(&states[1]).unwrap()
        );
        states = states
            .map(|state| serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap());
    }
    let catalogue =
        series::read_catalogue::<Catalogue>(&stores[0].store, &stores[0].key, "army-aid", 1)
            .unwrap()
            .unwrap()
            .1;
    let params = crate::products::volunteers_expeditionaries::VolunteersExpeditionariesParams {
        branch_id: "1".into(),
        kind: crate::products::volunteers_expeditionaries::MilitaryAidKind::Land,
        tags: None,
    };
    let prefix = dashboard_base::prepared::prefix::Prefix {
        coverage: dashboard_base::prepared::coverage::Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: 100,
            last_hour: 117,
            complete: true,
        },
        pending: Vec::new(),
    };
    let reply = crate::products::volunteers_expeditionaries::read(
        &stores[0].store,
        &stores[0].key,
        1,
        &params,
        catalogue,
        prefix,
        &flag,
    )
    .unwrap();
    let link = reply
        .links
        .iter()
        .find(|link| link.sender == "origin" && link.host == "host")
        .unwrap();
    assert_eq!(
        link.runs
            .iter()
            .find(|run| run.first_hour <= 105 && run.last_hour >= 105)
            .unwrap()
            .divisions,
        Some(0.0)
    );
    assert_eq!(
        link.runs
            .iter()
            .find(|run| run.first_hour <= 112 && run.last_hour >= 112)
            .unwrap()
            .divisions,
        Some(2.0)
    );
    assert!(
        link.runs
            .iter()
            .any(|run| run.first_hour == 110 && run.starts_segment)
    );
    assert!(
        reply.tallies[0]
            .runs
            .iter()
            .any(|run| run.first_hour == 112 && run.divisions.is_none())
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("tally differs"))
    );
    for hour in [100, 104, 106, 110, 114] {
        let point = direct(&scenario(hour));
        let sender = &point.senders[0];
        assert_eq!(
            link.runs
                .iter()
                .find(|run| run.first_hour <= hour && run.last_hour >= hour)
                .unwrap()
                .divisions,
            Some(*sender.channels[0].values.values().next().unwrap() as f64)
        );
    }
    assert!(
        crate::products::volunteers_expeditionaries::read(
            &stores[0].store,
            &stores[0].key,
            1,
            &params,
            series::read_catalogue::<Catalogue>(&stores[0].store, &stores[0].key, "army-aid", 1)
                .unwrap()
                .unwrap()
                .1,
            reply.prefix,
            &AtomicBool::new(true)
        )
        .is_err()
    );
}

#[test]
fn military_aid_complete_empty_lists_do_not_demand_live_divisions() {
    let mut frame = scenario(100);
    *frame.tables[TableId::SentVolunteers.index()] =
        TableView::Complete(TableRows::SentVolunteers(Vec::new()));
    *frame.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "no census".into(),
    };
    let point = direct(&frame);
    assert!(
        point
            .senders
            .iter()
            .all(|sender| sender.channels[0].complete && sender.channels[0].values.is_empty())
    );
    let host = point
        .senders
        .iter()
        .find(|sender| sender.actor.tag == "host")
        .unwrap();
    assert!(host.channels[1].complete && host.channels[1].values.is_empty());
}
#[test]
fn military_aid_division_deltas_revisit_only_referencing_senders() {
    use dashboard_base::test_fixtures::{CAMPAIGN, records};
    let frames = (100..104)
        .map(|hour| {
            let mut frame = scenario(hour);
            if let TableView::Complete(TableRows::Divisions(rows)) =
                &mut *frame.tables[TableId::Divisions.index()]
            {
                if hour >= 101 {
                    rows[1].hit_points += rows::DivisionRow::HIT_POINTS_STEP.raw();
                    rows[1].logical_owner = 30;
                } // unlisted division: no aid projection
                if hour >= 102 {
                    rows[0].logical_owner = 30;
                }
                if hour >= 103 {
                    rows[0].hit_points += rows::DivisionRow::HIT_POINTS_STEP.raw();
                }
            }
            if hour >= 103
                && let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
                    &mut *frame.tables[TableId::CountryCounterInputs.index()]
            {
                rows[0].controlled_states = Some(vec![987]).into();
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut stream = ReplayStream::new(prepared::TABLES).unwrap();
    let mut tracker = FrameChangeTracker::new();
    let mut state = super::super::ScanState::new(true);
    let mut scan = super::super::Scan::default();
    let mut runtime = Runtime::default();
    let mut work = Vec::new();
    stream
        .next_block_with(
            &replay,
            |raw, frame| {
                let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                state.step_updates(
                    &mut scan,
                    super::super::Updates::decoded(&changes, state.applied, true),
                    frame,
                    &replay,
                    Some(&changes),
                );
                runtime.step(
                    frame,
                    &changes,
                    state.inventory.as_ref().unwrap(),
                    &state.identities,
                    state.identity_changed,
                    false,
                );
                work.push(runtime.projections);
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(work, [3, 3, 4, 4]);
}

#[test]
fn military_aid_country_identity_reuse_splits_equal_deployments() {
    let flag = AtomicBool::new(false);
    for (country_at, country_id, slot) in [(0, 12, 1), (1, 21, 2)] {
        let fixture = Fixture::new();
        let mut history = History::default();
        history.observe(direct(&scenario(100))).unwrap();
        let mut replaced = scenario(101);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *replaced.tables[TableId::Countries.index()]
        {
            rows[country_at].id = country_id;
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *replaced.tables[TableId::CountryLinks.index()]
        {
            rows.push(rows::CountryLinkRow {
                id: country_id,
                canonical_slot: slot,
            });
            rows.sort_by_key(|row| row.id);
        }
        history.observe(direct(&replaced)).unwrap();
        let mut writer = fixture.writer();
        history.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        let catalogue =
            series::read_catalogue::<Catalogue>(&fixture.store, &fixture.key, "army-aid", 1)
                .unwrap()
                .unwrap()
                .1;
        let params = crate::products::volunteers_expeditionaries::VolunteersExpeditionariesParams {
            branch_id: "1".into(),
            kind: crate::products::volunteers_expeditionaries::MilitaryAidKind::Land,
            tags: Some(vec!["origin".into()]),
        };
        let prefix = dashboard_base::prepared::prefix::Prefix {
            coverage: dashboard_base::prepared::coverage::Coverage {
                schema: dashboard_base::prepared::coverage::SCHEMA,
                first_hour: 100,
                last_hour: 101,
                complete: true,
            },
            pending: Vec::new(),
        };
        let reply = crate::products::volunteers_expeditionaries::read(
            &fixture.store,
            &fixture.key,
            1,
            &params,
            catalogue,
            prefix,
            &flag,
        )
        .unwrap();
        assert_eq!(reply.links.len(), 2);
        assert_eq!(reply.tallies.len(), 2);
        let keys: BTreeSet<_> = reply
            .links
            .iter()
            .chain(&reply.tallies)
            .map(|link| &link.key)
            .collect();
        assert_eq!(keys.len(), 4);
        for (links, divisions) in [(&reply.links, 2.0), (&reply.tallies, -3.0)] {
            let mut links = links.iter().collect::<Vec<_>>();
            links.sort_by_key(|link| link.runs[0].first_hour);
            for (at, link) in links.into_iter().enumerate() {
                let sender_id = if country_at == 0 && at == 1 {
                    country_id
                } else {
                    10
                };
                let host_id = if country_at == 1 && at == 1 {
                    country_id
                } else {
                    20
                };
                assert_eq!(
                    (link.sender.as_str(), link.host.as_str()),
                    ("origin", "host")
                );
                assert_eq!(
                    (
                        link.sender_slot,
                        link.sender_country_id,
                        link.host_slot,
                        link.host_country_id
                    ),
                    (Some(1), Some(sender_id), Some(2), Some(host_id)),
                );
                let hour = 100 + at as i32;
                assert_eq!(
                    link.runs
                        .iter()
                        .map(|run| (
                            run.first_hour,
                            run.last_hour,
                            run.divisions,
                            run.starts_segment
                        ))
                        .collect::<Vec<_>>(),
                    [(hour, hour, Some(divisions), true)],
                );
            }
        }
    }
}

#[test]
fn military_aid_unresolved_senders_refuse_global_proof_without_erasing_independent_links() {
    let mut frame = scenario(100);
    if let TableView::Complete(TableRows::SentVolunteers(rows)) =
        &mut *frame.tables[TableId::SentVolunteers.index()]
    {
        rows.push(SentVolunteerRow {
            country: 999,
            ordinal: 0,
            division: 1,
        });
    }
    let projection = direct(&frame);
    assert!(!projection.reads[0]);
    assert!(projection.senders[0].channels[0].complete);
    assert_eq!(projection.reasons[0].values().sum::<usize>(), 1);
    assert!(projection.reasons[2].is_empty());
}

#[test]
fn military_aid_logical_host_is_independent_of_roster_host_and_optional_unit_operands() {
    let mut frame = scenario(100);
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    {
        rows[0].logical_owner = 30;
    }
    for table in [
        TableId::DivisionCensus,
        TableId::DivisionLandInputs,
        TableId::DivisionTemplateIcons,
    ] {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "independent optional operands".into(),
        };
    }
    let point = direct(&frame);
    let sender = &point.senders[0];
    assert!(sender.channels.iter().all(|channel| channel.complete));
    assert_eq!(
        sender.channels[0]
            .values
            .iter()
            .map(|(host, &n)| (host.tag.as_str(), n))
            .collect::<Vec<_>>(),
        [("empty", 2)]
    );
    assert_eq!(
        sender.channels[1]
            .values
            .iter()
            .map(|(host, &n)| (host.tag.as_str(), n))
            .collect::<Vec<_>>(),
        [("host", 2)]
    );
}

#[test]
fn military_aid_ambiguous_tally_and_expeditionary_aliases_remain_independent() {
    let mut frame = scenario(100);
    if let TableView::Complete(TableRows::VolunteerHosts(rows)) =
        &mut *frame.tables[TableId::VolunteerHosts.index()]
    {
        let mut extra = rows[0].clone();
        extra.country = 11;
        rows.push(extra);
    }
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *frame.tables[TableId::CountryCounterInputs.index()]
    {
        let mut extra = rows[0].clone();
        extra.country = 11;
        rows.insert(1, extra);
    }
    let point = direct(&frame);
    let sender = &point.senders[0];
    assert!(sender.channels[0].complete);
    assert!(!sender.channels[1].complete);
    assert!(!sender.channels[2].complete);
    assert_eq!(sender.hosts.len(), 3);
}

#[test]
fn military_aid_recorded_gap_rebuilds_identity_proof_even_when_country_rows_are_unchanged() {
    let mut state = super::super::ScanState::new(true);
    let mut runtime = Runtime::default();
    let mut tracker = FrameChangeTracker::new();
    for hour in 100..104 {
        let frame = scenario(hour);
        let changes = tracker.observe(&frame, replay_data::format::FormatVersion::CURRENT);
        state.identities(&frame);
        state.apply_updates(
            super::super::Updates::decoded(&changes, None, false),
            &frame,
        );
        let point = runtime.step(
            &frame,
            &changes,
            state.inventory.as_ref().unwrap(),
            &state.identities,
            hour == 100,
            hour == 101,
        );
        if hour == 101 {
            assert!(point.actors.as_ref().unwrap().is_empty());
            assert!(!point.reads[0]);
        } else {
            assert!(point.reads[0]);
            if hour == 102 {
                assert_eq!(point.actors.as_ref().unwrap().len(), 3);
                assert_eq!(point.senders[0].channels[0].values.values().sum::<i64>(), 2);
            }
        }
    }
}

#[test]
fn military_aid_empty_land_links_do_not_claim_unavailable_tallies_are_empty() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut frame = scenario(100);
    *frame.tables[TableId::SentVolunteers.index()] =
        TableView::Complete(TableRows::SentVolunteers(Vec::new()));
    *frame.tables[TableId::VolunteerHosts.index()] = TableView::Unavailable {
        reason: "native tally unavailable".into(),
    };
    let mut history = History::default();
    history.observe(direct(&frame)).unwrap();
    let mut writer = fixture.writer();
    history.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let catalogue =
        series::read_catalogue::<Catalogue>(&fixture.store, &fixture.key, "army-aid", 1)
            .unwrap()
            .unwrap()
            .1;
    let params = crate::products::volunteers_expeditionaries::VolunteersExpeditionariesParams {
        branch_id: "1".into(),
        kind: crate::products::volunteers_expeditionaries::MilitaryAidKind::Land,
        tags: None,
    };
    let prefix = dashboard_base::prepared::prefix::Prefix {
        coverage: dashboard_base::prepared::coverage::Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: 100,
            last_hour: 100,
            complete: true,
        },
        pending: Vec::new(),
    };
    let reply = crate::products::volunteers_expeditionaries::read(
        &fixture.store,
        &fixture.key,
        1,
        &params,
        catalogue,
        prefix,
        &flag,
    )
    .unwrap();
    assert!(reply.links.is_empty() && reply.tallies.is_empty());
    assert_eq!(reply.unobserved.len(), 1);
    assert_eq!(
        (
            reply.unobserved[0].first_hour,
            reply.unobserved[0].last_hour
        ),
        (100, 100)
    );
}

#[test]
fn military_aid_cached_identity_reread_keeps_host_removal_and_reappearance_across_blocks() {
    use dashboard_base::prepared::prepass::Folded;
    use dashboard_base::test_fixtures::{CAMPAIGN, continuation_records, records};
    let frames = |first| {
        (first..first + 4)
            .map(|hour| {
                let mut frame = scenario(hour);
                if (104..106).contains(&hour)
                    && let TableView::Complete(TableRows::Countries(rows)) =
                        &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.retain(|row| row.id != 20);
                }
                frame
            })
            .collect()
    };
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames(100))]).unwrap();
    for first in [104, 108] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(first))])
            .unwrap();
    }
    let flag = AtomicBool::new(false);
    let stores = [Fixture::new(), Fixture::new()];
    let mut continuous = prepared::Prepared::default();
    let mut folded = prepared::Prepared::default();
    let mut stream = ReplayStream::new(prepared::TABLES).unwrap();
    let mut tracker = FrameChangeTracker::new();
    for index in &replay.index {
        tracker.invalidate();
        stream
            .next_block_with(
                &replay,
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                    continuous.observe(frame, &replay, &changes)
                },
                || Ok(()),
            )
            .unwrap();
        let facts = extract_alone(&replay, index, &prepared::FAMILY).unwrap();
        assert_eq!(
            folded
                .fold(facts, &replay, index, &|_, result| result, &mut || Ok(()))
                .map_err(|failure| failure.error)
                .unwrap(),
            Folded::Fast,
        );
        for (state, store) in [(&mut continuous, &stores[0]), (&mut folded, &stores[1])] {
            let mut writer = store.writer();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        assert_payloads_equal(
            &stores[1].payloads(),
            &stores[0].payloads(),
            "identity changes across army blocks",
        );
        assert_eq!(
            serde_json::to_value(&folded).unwrap(),
            serde_json::to_value(&continuous).unwrap()
        );
        folded = serde_json::from_value(serde_json::to_value(&folded).unwrap()).unwrap();
    }
    let catalogue =
        series::read_catalogue::<Catalogue>(&stores[0].store, &stores[0].key, "army-aid", 1)
            .unwrap()
            .unwrap()
            .1;
    let host = catalogue
        .actors
        .iter()
        .position(|actor| actor.id == 20)
        .unwrap() as u64;
    let mut reader = series::Reader::new(
        &stores[0].store,
        &stores[0].key,
        "army-aid-proof-1".into(),
        &flag,
    )
    .unwrap();
    let presence = reader.runs(4 + host * 4 + 3, 100, 111).unwrap();
    assert_eq!(
        presence
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.break_before))
            .collect::<Vec<_>>(),
        [(100, 103, true), (106, 111, true)]
    );
}
