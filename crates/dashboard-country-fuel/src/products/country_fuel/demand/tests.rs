use super::super::tests::{award_row, fuel_frame, replay_source, reply_read};
use super::*;
use replay_data::{
    rows::FuelDemandFleet, rows::FuelDemandGroup, rows::Recorded, rows::TableId, rows::TableRows,
};
use replay_frame::frame::TableView;

fn pack(hour: i32) -> FuelDemandRead {
    FuelDemandRead {
        hour: replay_data::rows::FuelDemandHour::Stamped(hour),
        country_instance: Some(9),
        divisions: vec![FuelDemandUnit {
            reference: 7 << 32 | 29,
            owners: Some([11, 11, 11]),
            rate_raw: Some(10_000),
            name: Some(Vec::new()),
            ..Default::default()
        }],
        ..Default::default()
    }
}
fn row(pack: FuelDemandRead) -> CountryFuelRow {
    CountryFuelRow {
        demand: Recorded::Value(pack),
        ..award_row(100)
    }
}
fn consumption(row: &CountryFuelRow) -> FuelConsumptionRead {
    FuelConsumptionRead::from(&summary_of(row, 100, 100, true, &mut Default::default()))
}

#[test]
fn occurrence_history_follows_native_rename_but_refuses_reuse_changed_keys_gaps_and_instances() {
    use replay_data::{rows::CountryLinkRow, rows::CountryRow};
    let frames = (100..=220)
        .filter(|hour| ![160, 161].contains(hour))
        .map(|hour| {
            let mut frame = fuel_frame(
                hour,
                [100, 124, 148, 172, 196, 220].contains(&hour),
                9001,
                (1, 2, 3, 4),
            );
            let mut demand = pack(hour);
            if hour == 124 {
                demand.divisions[0].rate_raw = Some(20_000);
            }
            if hour == 148 {
                demand.divisions[0].ordinal = 1;
            }
            if hour >= 196 {
                demand.country_instance = Some(10);
            }
            let mut own = row(demand);
            if hour >= 196 {
                own.country_instance = Recorded::Value(Some(10));
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
                .tag = if hour < 124 {
                b"X".to_vec()
            } else {
                b"Y".to_vec()
            };
            if hour >= 124 {
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
    let source = replay_source(frames, "fuel-current-consumers");
    let cancelled = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let basis = FuelAwardsParams {
        branch_id: "1".into(),
        tag: "X".into(),
        hour: 100,
        country: "11".into(),
        country_instance: 9,
    };
    let inventory = CountryFuelDemand::load(&source, &basis, &context, &cancelled).unwrap();
    let history = CountryFuelDemandHistory::load(
        &source,
        &FuelDemandHistoryParams {
            branch_id: basis.branch_id,
            tag: basis.tag,
            hour: basis.hour,
            country: basis.country,
            country_instance: basis.country_instance,
            window: 0,
            key: inventory.parts[0].key.clone(),
        },
        &context,
        &cancelled,
    )
    .unwrap();
    let at = |hour| {
        history
            .runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .unwrap()
    };
    assert_eq!(at(124).amount_raw.as_deref(), Some("480000"));
    assert!(at(148).reason.as_ref().unwrap().contains("occurrence"));
    assert!(at(160).amount_raw.is_none());
    assert!(at(172).reason.as_ref().unwrap().contains("segment"));
    assert!(at(196).amount_raw.is_none());
}

#[test]
fn current_required_rows_keep_signed_wrapping_separate_from_burn_and_widened_sums() {
    let mut pack = pack(100);
    pack.divisions[0].rate_raw = Some(5_000_000_000_000);
    pack.divisions.push(FuelDemandUnit {
        reference: 8 << 32 | 29,
        rate_raw: Some(5_000_000_000_000),
        ..Default::default()
    });
    let row = row(pack);
    let summary = consumption(&row);
    let individual = engine_fuel::fuel_demand::daily_required(5_000_000_000_000);
    assert!(individual < 0);
    assert_eq!(
        summary.known_daily_raw[0],
        (2 * i128::from(individual)).to_string()
    );
    assert_eq!(summary.positive_daily_raw[0], "0");
    assert_eq!(
        summary.native_daily_raw[0].as_deref(),
        Some(
            engine_fuel::fuel_demand::daily_required(10_000_000_000_000)
                .to_string()
                .as_str()
        )
    );
    assert_ne!(
        summary.native_daily_raw[0].as_ref(),
        Some(&summary.known_daily_raw[0])
    );
    assert_ne!(
        summary.known_daily_raw[0],
        reply_read(&row, 100, None).day.unwrap().burned_raw
    );
}

#[test]
fn repeated_air_occurrences_and_partial_names_do_not_discard_known_quantities() {
    let mut pack = pack(100);
    pack.divisions[0].rate_raw = Some(-10_000);
    pack.divisions[0].name = Some(vec![0xff]);
    pack.army_reason = b"synthetic partial division census".to_vec();
    let wing = FuelDemandWing {
        reference: 9 << 32 | 29,
        owner_reason: b"synthetic owner unavailable".to_vec(),
        name: None,
        inputs: Some(engine_fuel::fuel_demand::AirInputs::default()),
        ..Default::default()
    };
    pack.sites = (0..2)
        .map(|list| FuelDemandSite {
            list,
            ordinal: 0,
            base: 1 << 32 | 62,
            wings: vec![wing.clone()],
            reason: b"synthetic incomplete site closure".to_vec(),
            ..Default::default()
        })
        .collect();
    let row = row(pack);
    let summary = consumption(&row);
    assert_eq!(summary.known_daily_raw[0], "-240000");
    assert!(summary.native_daily_raw[0].is_none());
    assert!(summary.native_daily_raw[1].is_none());
    let pack = pack_of(&row, 100).unwrap();
    let source = replay_source(
        vec![fuel_frame(100, true, 9001, (1, 2, 3, 4))],
        "fuel-current-consumers",
    );
    let cache = std::sync::OnceLock::new();
    let mut names =
        |bytes: &[u8]| read_name(&source, &LoadContext { content: None }, &cache, bytes);
    let parts: Vec<_> = occurrences(pack)
        .map(|row| part_of(row, &mut names).unwrap())
        .collect();
    assert_eq!(parts[0].amount_raw.as_deref(), Some("-240000"));
    assert!(parts[0].name_reason.as_ref().unwrap().contains("UTF-8"));
    assert!(parts[0].recorded_name.is_none());
    assert_ne!(parts[1].key, parts[2].key);
    assert_eq!(parts[1].reference, parts[2].reference);
    assert_eq!(
        parts[1].amount_raw.as_deref(),
        Some("0"),
        "native skipped wings remain known despite independent owner/name failure"
    );
    assert!(parts[1].topology_reason.is_some());
    assert_eq!(parts[1].site.as_ref().unwrap().list, 0);
    assert_eq!(parts[2].site.as_ref().unwrap().list, 1);
}

#[test]
fn actual_products_use_supplying_names_topology_and_windows_without_receipt_continuity() {
    let frames = (100..=196)
        .filter(|hour| ![150, 151].contains(hour))
        .map(|hour| {
            let read = [100, 124, 148, 172, 196].contains(&hour);
            let mut frame = fuel_frame(hour, read, 9001, (1, 2, 3, 4));
            let mut pack = pack(hour);
            pack.groups.push(FuelDemandGroup {
                reference: 7 << 32 | 49,
                country: 0,
                name: Some(Vec::new()),
                members: vec![pack.divisions[0].reference],
                ..Default::default()
            });
            pack.divisions[0].group = Some(pack.groups[0].reference);
            pack.fleets.push(FuelDemandFleet {
                reference: 7 << 32 | 72,
                name: Some(Vec::new()),
                ..Default::default()
            });
            pack.sites.push(FuelDemandSite {
                list: 1,
                ordinal: 7,
                reason: b"synthetic empty refused site".to_vec(),
                ..Default::default()
            });
            if hour == 124 {
                pack.divisions[0].name = Some(vec![0xff]);
            }
            if hour == 148 {
                pack.divisions.clear();
            }
            if hour >= 196 {
                pack.country_instance = Some(10);
            }
            let mut row = row(pack);
            if hour >= 196 {
                row.country_instance = Recorded::Value(Some(10));
            }
            if let Recorded::Value(awards) = &mut row.award_day {
                awards.continuity_start_hour = Some(if hour < 124 { 77 } else { 101 });
            }
            *frame.tables[TableId::CountryFuel.index()] =
                TableView::Complete(TableRows::CountryFuel(vec![row]));
            frame
        })
        .collect();
    let source = replay_source(frames, "fuel-current-consumers");
    let cancelled = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let params = FuelAwardsParams {
        branch_id: "1".into(),
        tag: "GER".into(),
        hour: 100,
        country: "11".into(),
        country_instance: 9,
    };
    let inventory = CountryFuelDemand::load(&source, &params, &context, &cancelled).unwrap();
    assert_eq!(inventory.parts[0].recorded_name.as_deref(), Some(""));
    assert_eq!(inventory.commands[0].country, "0");
    assert_eq!(
        inventory.commands[0].members[0],
        inventory.parts[0].reference
    );
    assert_eq!(inventory.sites[0].ordinal, 7);
    assert!(inventory.sites[0].wings.is_empty());
    assert!(inventory.sites[0].reason.is_some());
    let params = FuelDemandHistoryParams {
        branch_id: params.branch_id,
        tag: params.tag,
        hour: params.hour,
        country: params.country,
        country_instance: params.country_instance,
        window: 0,
        key: inventory.parts[0].key.clone(),
    };
    let history = CountryFuelDemandHistory::load(&source, &params, &context, &cancelled).unwrap();
    let at = |hour| {
        history
            .runs
            .iter()
            .find(|run| run.first_hour <= hour && run.last_hour >= hour)
            .unwrap()
    };
    assert_eq!((at(100).first_hour, at(100).last_hour), (100, 112));
    assert_eq!((at(123).first_hour, at(123).last_hour), (113, 123));
    assert!(!at(113).starts_segment);
    assert_eq!(at(124).amount_raw.as_deref(), Some("240000"));
    assert!(
        !at(124).starts_segment,
        "receipt reset cannot change current-demand identity"
    );
    assert!(at(124).recorded_name.is_none());
    assert!(at(124).name_reason.as_ref().unwrap().contains("UTF-8"));
    assert!(
        at(148).amount_raw.is_none(),
        "absent occurrence is not a zero receipt"
    );
    assert!(at(150).amount_raw.is_none());
    assert!(at(172).reason.as_ref().unwrap().contains("segment"));
    assert!(at(196).amount_raw.is_none());
    assert!(
        history
            .warnings
            .iter()
            .any(|reason| reason.contains("UTF-8"))
    );
    let stack = CountryFuelDemandStack::load(
        &source,
        &FuelDemandStackParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            country: params.country.clone(),
            country_instance: params.country_instance,
            window: params.window,
            keys: vec![params.key.clone()],
        },
        &context,
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&stack.parts[0].runs).unwrap(),
        serde_json::to_value(&history.runs).unwrap()
    );
    assert!(
        CountryFuelDemand::load(
            &source,
            &FuelAwardsParams {
                hour: 101,
                ..FuelAwardsParams {
                    branch_id: "1".into(),
                    tag: "GER".into(),
                    hour: 100,
                    country: "11".into(),
                    country_instance: 9
                }
            },
            &context,
            &cancelled
        )
        .is_err()
    );
    assert!(
        CountryFuelDemandHistory::load(
            &source,
            &FuelDemandHistoryParams {
                key: "unrecorded-key".into(),
                ..params
            },
            &context,
            &cancelled
        )
        .is_err()
    );
}

#[test]
fn current_demand_history_keeps_the_same_country_across_surrender_target_changes() {
    let frames = (100..=124)
        .map(|hour| {
            let mut frame = fuel_frame(hour, true, 9001, (1, 2, 3, 4));
            let mut demand = pack(hour);
            demand.country_instance = Some(if hour < 112 { 9 } else { 10 });
            let mut own = row(demand);
            own.country_instance = Recorded::Value(Some(if hour < 112 { 9 } else { 10 }));
            *frame.tables[TableId::CountryFuel.index()] =
                TableView::Complete(TableRows::CountryFuel(vec![own]));
            frame
        })
        .collect();
    let source = replay_source(frames, "fuel-surrender-target-change");
    let cancelled = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let basis = FuelAwardsParams {
        branch_id: "1".into(),
        tag: "GER".into(),
        hour: 100,
        country: "11".into(),
        country_instance: 9,
    };
    let inventory = CountryFuelDemand::load(&source, &basis, &context, &cancelled).unwrap();
    let history = CountryFuelDemandHistory::load(
        &source,
        &FuelDemandHistoryParams {
            branch_id: basis.branch_id,
            tag: basis.tag,
            hour: basis.hour,
            country: basis.country,
            country_instance: basis.country_instance,
            window: 0,
            key: inventory.parts[0].key.clone(),
        },
        &context,
        &cancelled,
    )
    .unwrap();
    for hour in [111, 112, 124] {
        let run = history
            .runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .unwrap();
        assert_eq!(run.amount_raw.as_deref(), Some("240000"));
        assert!(run.reason.is_none());
        if run.first_hour >= 112 {
            assert!(!run.starts_segment);
        }
    }
}
