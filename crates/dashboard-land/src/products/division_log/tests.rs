use super::*;
use dashboard_base::{
    Source, SourceKey,
    products::{LoadContext, Product},
    test_fixtures::{CAMPAIGN, records, source},
};
use replay_data::rows::{DivisionEquipmentDeletionRow, DivisionRow, TableId, TableRows};
use replay_frame::frame::TableView;
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn sightings_and_removals_follow_the_observation_hour_with_separate_coverage() {
    let division = |id: u64, template: u64| DivisionRow {
        id: id << 32 | 9,
        roster_owner: 9001,
        logical_owner: 9001,
        template: template << 32 | 3,
        hit_points: 100_000,
        ..Default::default()
    };
    let frames = (100..=107)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let mut divisions = vec![division(1, 1)];
            if hour >= 102 {
                divisions.push(division(2, 2));
                if hour < 103 {
                    divisions.push(division(3, 2));
                }
            }
            if hour >= 104 {
                divisions.push(division(4, 3));
            }
            let deletions = if hour == 105 {
                vec![DivisionEquipmentDeletionRow {
                    division_handle: 1 << 32 | 9,
                    country: 9001,
                    disposition: Disposition::Merged,
                    cause: None,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            };
            for (id, rows, missing) in [
                (TableId::Divisions, TableRows::Divisions(divisions), 107),
                (TableId::DivisionEquipmentDeletions, TableRows::DivisionEquipmentDeletions(deletions), 106),
            ] {
                frame.metrics.table_ns[id.index()] = 1;
                *frame.tables[id.index()] =
                    if hour == missing { TableView::Unavailable { reason: "fixture gap".into() } } else { TableView::Complete(rows) };
            }
            frame
        })
        .collect();
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let cancel = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey { source_id: "divisions".into(), revision: "1".into() },
        "fixture.battleplan".into(),
        Arc::new(replay),
        &cancel,
    )
    .unwrap();
    let load = LoadContext { content: None };
    let ask = |hour, options: serde_json::Value| {
        let mut params = json!({ "branchId": "1", "hour": hour, "limit": 10 });
        params.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
        DivisionLogTail::load(&source, &serde_json::from_value(params).unwrap(), &load, &cancel).unwrap()
    };

    let before = ask(101, json!({"events": "lost"}));
    assert!(before.rows.is_empty() && before.templates.is_empty());
    assert_eq!(before.next_change, Some(102));
    let collapsed = ask(102, json!({"countries": [7], "totals": true}));
    assert_eq!(collapsed.templates.iter().map(|chip| (chip.id, chip.first_hour)).collect::<Vec<_>>(), [(2, 102)]);
    assert_eq!(collapsed.rows.len(), 1);
    let group = &collapsed.rows[0];
    assert_eq!((group.slot, group.members, group.open), (Some(7), 2, false));
    assert_eq!(collapsed.counts[0].total, "2");
    let child = ask(102, json!({"expanded": [&group.key], "offset": "2", "limit": 1}));
    assert_eq!(child.total, "3");
    assert_eq!(child.rows[0].parent.as_ref(), Some(&group.key));
    assert_eq!(child.rows[0].event.as_ref().unwrap().after_hour, Some(101));
    let filtered = ask(104, json!({"templates": [3]}));
    assert_eq!(filtered.rows.len(), 1);
    assert_eq!(filtered.rows[0].event.as_ref().unwrap().division, (4u64 << 32 | 9).to_string());
    assert!(filtered.warnings.iter().all(|reason| !reason.contains("left the census")));
    let vanished = |hour| ask(hour, json!({"events": "lost", "templates": [2]}));
    assert_eq!(vanished(102).next_change, Some(103));
    assert!(vanished(103).warnings.iter().any(|reason| reason.contains("left the census")));
    let lost = ask(107, json!({"events": "lost"}));
    assert!(lost.warnings.iter().any(|reason| reason.contains("left the census")));
    assert_eq!(lost.rows.len(), 1);
    assert_eq!(lost.rows[0].event.as_ref().unwrap().status, "merged");
    assert!(lost.missing.iter().any(|gap| gap.first_hour == 106 && gap.reason.contains("deletions")));
    let created = ask(107, json!({"events": "created"}));
    assert!(created.missing.iter().all(|gap| !gap.reason.contains("deletions")));
    assert!(created.missing.iter().any(|gap| gap.first_hour == 107 && gap.reason.contains("census")));
    assert_eq!(created.total, "2");
}

#[test]
fn unresolved_owner_and_undated_removal_keep_honest_identity_and_last_sighting() {
    let frames = (100..=105)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let mut divisions = Vec::new();
            if hour <= 103 {
                divisions.push(DivisionRow {
                    id: 1 << 32 | 9,
                    roster_owner: 9001,
                    logical_owner: 9001,
                    province: if hour < 103 { 55 } else { 77 },
                    template: 1 << 32 | 3,
                    hit_points: 100_000,
                    ..Default::default()
                });
            }
            if hour >= 102 {
                divisions.push(DivisionRow {
                    id: 2 << 32 | 9,
                    roster_owner: 9999,
                    logical_owner: 9999,
                    province: 88,
                    template: 2 << 32 | 3,
                    hit_points: 100_000,
                    ..Default::default()
                });
            }
            let deletions = if hour == 104 {
                vec![DivisionEquipmentDeletionRow {
                    division_handle: 1 << 32 | 9,
                    country: 9001,
                    disposition: Disposition::Loss,
                    cause: Some(Cause::DivisionDestroyed),
                    ..Default::default()
                }]
            } else {
                Vec::new()
            };
            for (id, rows) in [
                (TableId::Divisions, TableRows::Divisions(divisions)),
                (TableId::DivisionEquipmentDeletions, TableRows::DivisionEquipmentDeletions(deletions)),
            ] {
                frame.metrics.table_ns[id.index()] = 1;
                *frame.tables[id.index()] = TableView::Complete(rows);
            }
            frame
        })
        .collect();
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let cancel = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey { source_id: "divisions-honesty".into(), revision: "1".into() },
        "fixture.battleplan".into(),
        Arc::new(replay),
        &cancel,
    )
    .unwrap();
    let load = LoadContext { content: None };
    let ask = |hour, options: serde_json::Value| {
        let mut params = json!({ "branchId": "1", "hour": hour, "limit": 10 });
        params.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
        DivisionLogTail::load(&source, &serde_json::from_value(params).unwrap(), &load, &cancel).unwrap()
    };

    let created = ask(103, json!({"events": "created"}));
    assert_eq!(created.rows.len(), 1);
    let sighting = &created.rows[0];
    assert_eq!((sighting.slot, sighting.tag.as_deref()), (None, None), "an unresolved owner names no country, never a guess");
    assert_eq!((sighting.hour, sighting.observed), (102, 102));
    let sighting_detail = sighting.event.as_ref().unwrap();
    assert_eq!(sighting_detail.division, (2u64 << 32 | 9).to_string());
    assert_eq!(sighting_detail.status, "first_observed");
    assert!(!sighting_detail.exact);
    assert_eq!(sighting_detail.after_hour, Some(101));
    assert_eq!((sighting_detail.province, sighting_detail.place_hour), (Some(88), Some(102)));

    let filtered = ask(103, json!({"events": "created", "countries": [7]}));
    assert!(filtered.rows.is_empty(), "an unresolved owner never matches a country filter");
    assert!(
        filtered.warnings.iter().any(|reason| reason.contains("no recorded owner for the country filter")),
        "filtering by country warns about divisions with no recorded owner: {:?}",
        filtered.warnings
    );

    let lost = ask(105, json!({"events": "lost"}));
    assert_eq!(lost.rows.len(), 1);
    let removal = &lost.rows[0];
    assert_eq!((removal.hour, removal.observed), (104, 104));
    assert_eq!(removal.slot, Some(7));
    assert!(removal.tag.is_some());
    let removal_detail = removal.event.as_ref().unwrap();
    assert_eq!(removal_detail.division, (1u64 << 32 | 9).to_string());
    assert_eq!(removal_detail.status, "division_destroyed");
    assert!(!removal_detail.exact);
    assert_eq!(
        (removal_detail.province, removal_detail.place_hour),
        (Some(77), Some(103)),
        "a death not dated is placed where the division was last seen"
    );
    assert_eq!((removal_detail.killer_slot, removal_detail.killer_tag.as_deref(), removal_detail.killer_division.as_deref()), (None, None, None));
    assert_eq!(removal_detail.matched_deaths, Some(0));
}

#[test]
fn hours_not_read_are_the_stretch_less_its_read_runs() {
    assert_eq!(unread(&[(10, 19), (25, 30)], 5, 40), vec![(5, 9), (20, 24), (31, 40)]);
    assert_eq!(unread(&[(0, 50)], 5, 40), Vec::<(i32, i32)>::new());
    assert_eq!(unread(&[], 5, 6), vec![(5, 6)]);
}

#[test]
fn alternating_filtered_views_reuse_cached_indices_with_lru_eviction() {
    let division = |id: u64, template: u64| DivisionRow {
        id: id << 32 | 9,
        roster_owner: 9001,
        logical_owner: 9001,
        template: template << 32 | 3,
        hit_points: 100_000,
        ..Default::default()
    };
    let frames = (100..=103)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let mut divisions = vec![division(1, 1)];
            if hour >= 101 {
                divisions.push(division(2, 2));
                divisions.push(division(3, 2));
            }
            if hour >= 102 {
                for id in 3..=(tail::HELD_VIEWS as u64 + 1) {
                    divisions.push(division(id + 10, id));
                }
            }
            let deletions = if hour == 103 {
                vec![DivisionEquipmentDeletionRow {
                    division_handle: 1 << 32 | 9,
                    country: 9001,
                    disposition: Disposition::Merged,
                    cause: None,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            };
            for (id, rows) in [
                (TableId::Divisions, TableRows::Divisions(divisions)),
                (TableId::DivisionEquipmentDeletions, TableRows::DivisionEquipmentDeletions(deletions)),
            ] {
                frame.metrics.table_ns[id.index()] = 1;
                *frame.tables[id.index()] = TableView::Complete(rows);
            }
            frame
        })
        .collect();
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let cancel = AtomicBool::new(false);
    let loaded = Source::from_replay(
        SourceKey { source_id: "divisions-view-cache".into(), revision: "1".into() },
        "fixture.battleplan".into(),
        Arc::new(replay),
        &cancel,
    )
    .unwrap();
    let load = LoadContext { content: None };
    let ask = |hour, options: serde_json::Value| {
        let mut params = json!({ "branchId": "1", "hour": hour, "limit": 10 });
        params.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
        DivisionLogTail::load(&loaded, &serde_json::from_value(params).unwrap(), &load, &cancel).unwrap()
    };

    let created = ask(103, json!({"events": "created"}));
    let lost = ask(103, json!({"events": "lost"}));
    assert_eq!(tail::view_builds(&loaded, 1), 2);
    for _ in 0..3 {
        assert_eq!(ask(103, json!({"events": "created"})), created);
        assert_eq!(ask(103, json!({"events": "lost"})), lost);
    }
    assert_eq!(tail::view_builds(&loaded, 1), 2, "alternating event filters must reuse cached indices");

    let collapsed = ask(101, json!({}));
    let group_key = collapsed.rows[0].key.clone();
    assert!(!collapsed.rows[0].open);
    let expanded = ask(101, json!({"expanded": [&group_key]}));
    assert!(expanded.rows[0].open);
    assert_eq!(tail::view_builds(&loaded, 1), 4);
    for _ in 0..2 {
        assert_eq!(ask(101, json!({})), collapsed);
        assert_eq!(ask(101, json!({"expanded": [&group_key]})), expanded);
    }
    assert_eq!(tail::view_builds(&loaded, 1), 4, "alternating open groups must reuse cached indices");

    forget(&loaded);
    for template in 1..=(tail::HELD_VIEWS as u32) {
        let reply = ask(103, json!({"templates": [template]}));
        assert!(!reply.rows.is_empty());
    }
    assert_eq!(tail::view_builds(&loaded, 1), tail::HELD_VIEWS);
    let _ = ask(103, json!({"templates": [1]}));
    assert_eq!(tail::view_builds(&loaded, 1), tail::HELD_VIEWS, "touching oldest entry promotes it before eviction");
    let _ = ask(103, json!({"templates": [tail::HELD_VIEWS as u32 + 1]}));
    assert_eq!(tail::view_builds(&loaded, 1), tail::HELD_VIEWS + 1);
    let _ = ask(103, json!({"templates": [1]}));
    assert_eq!(tail::view_builds(&loaded, 1), tail::HELD_VIEWS + 1, "recently used filter stays cached after cap overflow");
    let _ = ask(103, json!({"templates": [2]}));
    assert_eq!(tail::view_builds(&loaded, 1), tail::HELD_VIEWS + 2, "least recently used filter is evicted when cap is exceeded");
}
