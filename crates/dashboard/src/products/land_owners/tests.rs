use crate::test_fixtures::{CAMPAIGN, ownership_frame, records, scale_frame};
use replay::{
    container::Replay,
    frame::{TableUpdate, TableView},
    rows::{TableId, TableRows},
    spool::Record,
};
use std::sync::atomic::AtomicBool;

#[test]
fn land_owners_include_transient_and_other_branch_owners_but_not_controllers() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(
                1,
                vec![
                    ownership_frame(1, 100, 11),
                    ownership_frame(1, 101, 44),
                    ownership_frame(1, 1000, 11),
                ],
            ),
            records(2, vec![ownership_frame(2, 90, 66)]),
        ],
    )
    .unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["FRA", "GER", "MOD"]
    );
    let france = result
        .countries
        .iter()
        .find(|country| country.tag == "FRA")
        .unwrap();
    assert_eq!((france.id.as_str(), france.observed_hour), ("8", 101));
    assert_eq!(
        result
            .countries
            .iter()
            .find(|country| country.tag == "MOD")
            .unwrap()
            .observed_branch_id,
        "2"
    );
}

#[test]
fn land_owners_name_countries_in_the_given_language_and_fall_back_to_tags() {
    let mut frame = ownership_frame(1, 100, 44);
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay::rows::CountryAppearanceRow {
                country: 44,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay::rows::PoliticsRow {
            country: 44,
            ..Default::default()
        }]));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let localisation: installed_content::localisation::Localisation =
        [("FRA".to_owned(), "France".to_owned())]
            .into_iter()
            .collect();
    let result = crate::land_owners(&replay, Some(&localisation), &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries[0].name, "France");
    let untranslated = installed_content::localisation::Localisation::default();
    let result = crate::land_owners(&replay, Some(&untranslated), &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries[0].name, "FRA");
    assert!(result.warnings.iter().any(|w| w.contains("show their tag")));
}

#[test]
fn land_owners_report_dangling_owners_and_missing_country_identity() {
    for missing in ["link", "row", "tag"] {
        let mut frame = ownership_frame(1, 101, 44);
        match missing {
            "link" => {
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *frame.tables[TableId::CountryLinks.index()]
                {
                    rows.retain(|row| row.id != 44);
                }
            }
            "row" => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.retain(|row| row.id != 44);
                }
            }
            _ => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.iter_mut()
                        .find(|row| row.id == 44)
                        .unwrap()
                        .tag
                        .clear();
                }
            }
        }
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![ownership_frame(1, 100, 11), frame])],
        )
        .unwrap();
        let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            result
                .countries
                .iter()
                .map(|country| country.tag.as_str())
                .collect::<Vec<_>>(),
            ["GER"],
            "{missing}"
        );
        assert!(!result.warnings.is_empty(), "{missing}");
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![ownership_frame(1, 100, 0)])],
    )
    .unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert!(result.countries.is_empty());
    assert!(result.warnings.is_empty());
}

#[test]
fn land_owners_keep_partial_positive_evidence_and_do_not_reuse_lost_baselines() {
    let mut partial = ownership_frame(1, 102, 66);
    *partial.tables[TableId::States.index()] = TableView::Truncated {
        rows: TableRows::States(vec![replay::rows::StateRow {
            id: 1,
            owner: 66,
            controller: 55,
            movement_infrastructure: 0,
            ..Default::default()
        }]),
        missing: 1,
    };
    let mut input = records(
        1,
        vec![
            ownership_frame(1, 100, 11),
            ownership_frame(1, 101, 44),
            partial,
        ],
    );
    if let Record::Frame(frame) = &mut input[3] {
        frame.tables[TableId::States.index()] = TableUpdate::Unchanged { baseline: 99 };
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        result
            .countries
            .iter()
            .map(|country| country.tag.as_str())
            .collect::<Vec<_>>(),
        ["GER", "MOD"]
    );
    assert!(!result.warnings.is_empty());
}

#[test]
fn land_owner_scan_honors_cancellation_and_block_budget() {
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![ownership_frame(1, 100, 11)])],
    )
    .unwrap();
    assert_eq!(
        crate::land_owners(&replay, None, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    // The container refuses an oversized block before decompressing it; the
    // product reports the gap instead of trusting a partial catalogue.
    replay.index[0].raw_length = replay::container::MAX_BLOCK_RAW_BYTES as u64 + 1;
    let owners = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert!(owners.countries.is_empty());
    assert!(!owners.warnings.is_empty());
}

#[test]
fn land_owners_list_every_country_once_across_branches() {
    const COUNTRIES: u32 = 5_000;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(2, vec![scale_frame(2, 90, COUNTRIES)]),
            records(
                1,
                vec![
                    scale_frame(1, 100, COUNTRIES),
                    scale_frame(1, 101, COUNTRIES),
                ],
            ),
            records(3, vec![scale_frame(3, 80, COUNTRIES)]),
        ],
    )
    .unwrap();
    let result = crate::land_owners(&replay, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(result.countries.len(), COUNTRIES as usize);
    assert!(result.warnings.is_empty());
    // Sorted by tag, each witnessed at its first hour on the lowest branch.
    assert!(
        result
            .countries
            .windows(2)
            .all(|pair| pair[0].tag < pair[1].tag)
    );
    assert!(
        result.countries.iter().all(|country| (
            country.observed_branch_id.as_str(),
            country.observed_hour
        ) == ("1", 100))
    );
    let bytes = serde_json::to_vec(&result).unwrap().len();
    assert!(
        bytes < crate::transport::MAX_REPLY_BYTES / 4,
        "{bytes} bytes"
    );
}
