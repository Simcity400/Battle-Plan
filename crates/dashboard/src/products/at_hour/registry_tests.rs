use super::*;

#[test]
fn every_rows_at_an_hour_product_is_registered_moved_and_shown_as_a_panel() {
    for entry in crate::products::AT_HOUR {
        let key = entry.registered.key;
        assert!(crate::products::find(key).is_some(), "{key} answers requests");
        assert!(crate::products::MOVED.contains(&key), "{key} is answered through the query layer");
    }
    assert!(crate::products::find(super::super::army::TemplateRoster::KEY).is_some());
}

#[test]
fn dated_products_seed_advance_confirm_and_reseed_across_a_recording_gap() {
    use crate::test_fixtures::{source, source_with_content};
    use dashboard_follow::panel::{dated, product::Delivery};
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    use serde_json::json;

    let root = std::env::temp_dir().join(format!("dated-products-{}", uuid::Uuid::new_v4()));
    let frames = [100, 101, 102, 103, 110, 111].map(|hour| {
        let mut frame = source(1, hour, None);
        for table in TableId::ALL {
            if matches!(frame.tables[table.index()].held(), TableView::Unavailable { .. }) {
                *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
            }
        }
        let changed = i32::from(hour >= 102);
        let set = |frame: &mut replay_frame::frame::DecodedFrame, table: TableId, rows| {
            *frame.tables[table.index()] = TableView::Complete(rows);
        };
        set(&mut frame, TableId::IntelligenceAgencies, TableRows::IntelligenceAgencies(vec![IntelligenceAgencyRow { country: 9001, created: 1, active_count: changed, ..Default::default() }]));
        set(&mut frame, TableId::CountryGameScores, TableRows::CountryGameScores(vec![CountryGameScoreRow { country: 9001, army: Recorded::Value(i64::from(changed) * 1000), ..Default::default() }]));
        set(&mut frame, TableId::CountryOccupationReserveManpower, TableRows::CountryOccupationReserveManpower(vec![CountryOccupationReserveManpowerRow { country: 9001, source_country: 11, amount: changed, ..Default::default() }]));
        set(&mut frame, TableId::DoctrineFolders, TableRows::DoctrineFolders(vec![DoctrineFolderRow { slot: 7, folder: 1, ..Default::default() }]));
        set(&mut frame, TableId::DoctrineTracks, TableRows::DoctrineTracks(vec![DoctrineTrackRow { slot: 7, mastery: changed.into(), ..Default::default() }]));
        let mut lines = vec![ProductionLineRow { country: 9001, handle: 1, active_factories: changed, ..Default::default() }];
        if hour >= 102 { lines.push(ProductionLineRow { country: 9001, handle: 2, ..Default::default() }); }
        set(&mut frame, TableId::ProductionLines, TableRows::ProductionLines(lines));
        set(&mut frame, TableId::TerritoryTotals, TableRows::TerritoryTotals(vec![
            TerritoryTotalRow { slot: 7, owned_states: Recorded::Value(changed.into()), ..Default::default() },
            TerritoryTotalRow { slot: TerritoryTotalRow::WORLD, owned_states: Recorded::Value(12), ..Default::default() },
        ]));
        set(&mut frame, TableId::ResearchCoverage, TableRows::ResearchCoverage(vec![ResearchCoverageRow { holder_slot: 7, holder_id: 9001, member: ResearchCoverageRow::HOLDER, since: 100 }]));
        set(&mut frame, TableId::ResearchInputs, TableRows::ResearchInputs(vec![ResearchInputRow { country: 9001, banked_days_raw: changed.into(), ..Default::default() }]));
        let mut project = ProjectRow { owner: 9001, local_id: 1, started: 1, ..Default::default() };
        project.state.prototype_progress = changed;
        set(&mut frame, TableId::Projects, TableRows::Projects(vec![project]));
        let mut samples: Vec<_> = [TableId::ResearchInputs, TableId::Projects, TableId::ResearchModifiers, TableId::ResearchAppliedBonuses].map(|table| SliceReadRow { table: table.index() as u32, owner: 9001, sample_hour: 100 + changed * 2 }).into();
        samples.sort_by_key(|row| row.table);
        set(&mut frame, TableId::SliceReads, TableRows::SliceReads(samples));
        frame
    });
    let (source, load) = source_with_content(frames.into(), &root);
    for name in ["special-project-details", "special-projects", "doctrine-mastery", "research-slots", "production-lines-at", "nuclear-arsenal", "game-score", "occupation-reserve-manpower", "agency-comparison", "territory-share", "lend-lease-balance"] {
        let mut params = json!({"branchId": "1", "panel": name});
        match name {
            "nuclear-arsenal" | "game-score" | "occupation-reserve-manpower" | "lend-lease-balance" => {}
            "agency-comparison" => params["includeUncreated"] = true.into(),
            "territory-share" => params["measure"] = "ownedStates".into(),
            _ => params["tag"] = "GER".into(),
        }
        let panel = dated_panel_of(&source, &load, &params).unwrap().unwrap();
        let exact = panel.exact().unwrap();
        let mut delivery = Delivery::default();
        let mut previous = None;
        for (hour, seek) in [(100, true), (101, false), (102, false), (102, false), (104, false), (106, false), (110, false), (100, true)] {
            let read = (exact.advance)(hour, seek, &false.into(), None);
            for confirmed in hour..=read.through.min(110) {
                assert_eq!(*read.value, (exact.cold)(confirmed, &false.into()), "{name} confirmed {confirmed} from {hour}");
            }
            let (data, encoded) = (read.value, read.encoded);
            let cold = (exact.cold)(hour, &false.into());
            assert_eq!(*data, cold, "{name} at {hour}");
            if (104..110).contains(&hour) {
                assert!(cold.as_ref().unwrap_err().contains("hour is not recorded"), "{name} gap at {hour}");
            } else {
                assert!(cold.is_ok(), "{name} at {hour}: {cold:?}");
            }
            let packet = delivery.packet((0, 1, 1), 1, hour, seek, &data, encoded);
            assert_eq!(packet.changed, seek || previous.as_ref() != Some(&cold), "{name} at {hour}");
            let body = match &cold { Ok(v) => rmp_serde::to_vec(v).unwrap(), Err(e) => e.as_bytes().to_vec() };
            assert_eq!(delivery.body(), Some(body.as_slice()), "{name} wire record at {hour}");
            assert_eq!(dated::records(&packet.bytes).unwrap()[0].hour, hour);
            previous = Some(cold);
        }
    }
    drop(source);
    std::fs::remove_dir_all(root).unwrap();
}
