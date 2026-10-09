use super::super::tests::{award_row, fuel_frame};
use super::*;
use crate::test_fixtures::{CAMPAIGN, records};
use replay::frame::{DecodedFrame, TableView};
use replay::rows::{Recorded, TableRows};

fn source(frames: Vec<DecodedFrame>, id: &str) -> Source {
    Source::from_replay(
        crate::SourceKey {
            source_id: id.into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn recipient_history_follows_native_aliases_without_crossing_reset_boundaries() {
    use replay::rows::{CountryLinkRow, CountryRow};
    let cancelled = AtomicBool::new(false);
    let first = (1000..1000 + DAY_HOURS)
        .find(|&hour| cadence::is_refresh(TableId::CountryFuel, hour))
        .unwrap();
    let row = award_row(first);
    let key = recipients_of(&row, &cancelled, crate::catalogue::display_text).unwrap()[0]
        .key
        .clone();
    let frames = (first..=first + 3 * DAY_HOURS)
        .map(|hour| {
            let mut frame = fuel_frame(hour, (hour - first) % DAY_HOURS == 0, 9001, (1, 2, 3, 4));
            let mut own = award_row(hour);
            if let Recorded::Value(window) = &mut own.awards {
                window.continuity_start_hour = Some(if hour < first + 2 * DAY_HOURS {
                    first - DAY_HOURS + 1
                } else {
                    first + DAY_HOURS + 1
                });
            }
            *frame.tables[TableId::CountryFuel.index()] =
                TableView::Complete(TableRows::CountryFuel(vec![own]));
            let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            else {
                panic!("countries")
            };
            countries
                .iter_mut()
                .find(|country| country.id == 9001)
                .unwrap()
                .tag = if hour < first + DAY_HOURS {
                b"X".to_vec()
            } else {
                b"Y".to_vec()
            };
            if hour >= first + DAY_HOURS {
                countries.push(CountryRow {
                    id: 9002,
                    canonical_slot: 9,
                    tag: b"X".to_vec(),
                    ..Default::default()
                });
                let TableView::Complete(TableRows::CountryLinks(links)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                else {
                    panic!("links")
                };
                links.push(CountryLinkRow {
                    id: 9002,
                    canonical_slot: 9,
                });
                links.sort_by_key(|link| link.id);
            }
            frame
        })
        .collect();
    let source = source(frames, "renamed-recipient");
    let reply = CountryFuelAwardHistory::load(
        &source,
        &FuelAwardHistoryParams {
            branch_id: "1".into(),
            tag: "X".into(),
            hour: first,
            country: "11".into(),
            country_instance: 9,
            window: 1,
            recipient_key: key,
        },
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    let at = |hour| {
        reply
            .runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .unwrap()
    };
    assert!(at(first + DAY_HOURS).amount_raw.is_some());
    assert!(
        at(first + 2 * DAY_HOURS)
            .reason
            .as_ref()
            .unwrap()
            .contains("reset/gap")
    );
}

#[test]
fn exact_recipient_history_preserves_zero_corrections_names_gaps_and_reset_lifetimes() {
    let cancelled = AtomicBool::new(false);
    let first = award_row(1000);
    let key = recipients_of(&first, &cancelled, crate::catalogue::display_text).unwrap()[0]
        .key
        .clone();
    let params = FuelAwardHistoryParams {
        branch_id: "1".into(),
        tag: "GER".into(),
        hour: 1000,
        country: "11".into(),
        country_instance: 9,
        window: 1,
        recipient_key: key,
    };
    let mut reads = Reads::new();
    let mut summary = ScannedFuel::default();
    for hour in [1000, 1024, 1048, 1072, 1096] {
        let mut row = award_row(hour);
        let Recorded::Value(window) = &mut row.awards else {
            panic!()
        };
        if hour == 1024 {
            for entry in &mut window.hours {
                entry.recipients.clear();
                entry.received_raw = [0; 3];
            }
        }
        if hour == 1048 {
            for entry in &mut window.hours {
                for part in &mut entry.recipients {
                    part.name = Some(b"renamed".to_vec());
                }
            }
        }
        if hour == 1072 {
            window.reason = b"native receipt root refused".to_vec();
            window.continuity_start_hour = None;
        }
        if hour == 1096 {
            window.continuity_start_hour = Some(1073);
        }
        let mut read = read_of(&row, hour, hour == 1000, None);
        read.last_hour = hour + 1;
        summary.reads.push(read);
        reads.insert(hour, BTreeMap::from([("GER".into(), row)]));
    }
    let runs = history_runs(
        &summary,
        &reads,
        &[&params.tag],
        &params,
        977,
        (999, 1100),
        &cancelled,
    )
    .unwrap();
    let at = |hour| runs.iter().find(|run| run.first_hour == hour).unwrap();
    assert_eq!(at(1000).amount_raw.as_deref(), Some("-12"));
    assert_eq!(at(1024).amount_raw.as_deref(), Some("0"));
    assert_eq!(at(1048).amount_raw.as_deref(), Some("0"));
    assert!(at(1072).amount_raw.is_none());
    assert!(
        at(1096)
            .reason
            .as_ref()
            .unwrap()
            .contains("reset/gap segment")
    );
    assert!(at(1002).amount_raw.is_none());
    assert!(at(1024).starts_segment);
    assert!(
        history_runs(
            &summary,
            &reads,
            &[&params.tag],
            &params,
            977,
            (999, 1100),
            &AtomicBool::new(true)
        )
        .is_err()
    );
}

#[test]
fn history_uses_one_visible_all_country_window_and_authenticates_basis_and_keys() {
    let count = 5000_u32;
    let hour = DETAIL_HOURS * 3 - 3;
    let frames = (hour..=hour + 4)
        .map(|at| {
            let mut frame = fuel_frame(at, at == hour, 9001, (1, 2, 3, 4));
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                (1..=count)
                    .map(|id| replay::rows::CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("C{id:04}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ));
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(
                    (1..=count)
                        .map(|id| replay::rows::CountryLinkRow {
                            id,
                            canonical_slot: id,
                        })
                        .collect(),
                ));
            *frame.tables[TableId::CountryFuel.index()] =
                TableView::Complete(TableRows::CountryFuel(
                    (1..=count)
                        .map(|country| {
                            let mut row = award_row(hour);
                            row.country = country;
                            row
                        })
                        .collect(),
                ));
            frame
        })
        .collect();
    let source = source(frames, "fuel-recipient-window-5000");
    let cancelled = AtomicBool::new(false);
    let summary = scanned(&source, 1, &cancelled).unwrap();
    let window = window_reads(&source, 1, 3, &summary, &cancelled).unwrap();
    assert_eq!(window[&hour].len(), count as usize);
    let key = recipients_of(&award_row(hour), &cancelled, crate::catalogue::display_text).unwrap()
        [0]
    .key
    .clone();
    for country in 1..=count {
        let params = FuelAwardHistoryParams {
            branch_id: "1".into(),
            tag: format!("C{country:04}"),
            hour,
            country: country.to_string(),
            country_instance: 9,
            window: 3,
            recipient_key: key.clone(),
        };
        let reply = CountryFuelAwardHistory::load(
            &source,
            &params,
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        assert_eq!(reply.runs[0].first_hour, hour); // supplying read before the visible window
        assert_eq!(reply.runs[0].last_hour, hour + 4);
        assert_eq!(reply.runs[0].amount_raw.as_deref(), Some("-12"));
    }
    assert!(Arc::ptr_eq(
        &window,
        &window_reads(&source, 1, 3, &summary, &cancelled).unwrap()
    ));
    let mut params = FuelAwardHistoryParams {
        branch_id: "1".into(),
        tag: "C0001".into(),
        hour,
        country: "1".into(),
        country_instance: 9,
        window: 3,
        recipient_key: key,
    };
    params.country_instance += 1;
    assert!(
        CountryFuelAwardHistory::load(&source, &params, &LoadContext { content: None }, &cancelled)
            .is_err()
    );
    params.country_instance -= 1;
    params.recipient_key.push('x');
    assert!(
        CountryFuelAwardHistory::load(&source, &params, &LoadContext { content: None }, &cancelled)
            .is_err()
    );
}
