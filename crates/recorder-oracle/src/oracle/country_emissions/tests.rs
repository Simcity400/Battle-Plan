use super::super::country_items;
use super::*;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::ReservedCountryNamingRow, rows::TableRows,
};
use replay_frame::frame::TableView;

fn frame(array: Recorded<Option<Vec<u32>>>) -> DecodedFrame {
    let (mut frame, _, _, _) = super::super::fixtures::fixture();
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: b"ABC".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 1,
            canonical_slot: 1,
        }]));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"RES".to_vec(),
                country_array: array,
                ..Default::default()
            },
        ]));
    frame
}

fn document(countries: &str) -> SaveDocument {
    SaveDocument::read(
        format!(
            "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 \
         debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 \
         theater_group_index=6 military_deployment_line_index=7 \
         military_deployment_conveyor_index=8 equipment_variant_index=9 \
         country_leader_index=10 navy_id=11 land_combat_id=12 \
         multiplayer_random_count=-13 faction_system={{countries={{0}} extracted={{}}}} countries={{{countries}}} states={{}} provinces={{}}"
        )
        .as_bytes(),
    )
    .unwrap()
}

#[test]
fn full_census_admits_zero_one_and_repeated_reserved_emissions() {
    for (array, countries, count) in [
        (vec![0, 1], "ABC={}", 0),
        (vec![0, 1, 0], "ABC={} RES={}", 1),
        (vec![0, 1, 0, 0], "ABC={} RES={} RES={}", 2),
    ] {
        let saved =
            parse_country_emissions(&document(countries), &frame(Recorded::Value(Some(array))))
                .unwrap();
        let ledger = saved.country_emissions.unwrap();
        assert_eq!(
            ledger
                .iter()
                .filter(|entry| entry.identity.actual == ActualCountry::Reserved)
                .count(),
            count
        );
        assert_eq!(ledger[0].identity.actual, ActualCountry::Canonical(1));
    }
}

#[test]
fn historical_or_refused_census_never_admits_repeated_tags() {
    for array in [Recorded::NotRecorded, Recorded::Value(None)] {
        let frame = frame(array);
        assert!(parse_country_emissions(&document("RES={} RES={}"), &frame).is_err());
        assert!(country_items(&document("RES={} RES={}"), &frame).is_err());
        assert!(
            parse_country_emissions(&document("ABC={}"), &frame)
                .unwrap()
                .country_emissions
                .is_none()
        );
    }
}

#[test]
fn land_division_comparison_uses_authenticated_alias_representatives_and_keeps_differences() {
    use super::super::{land, report::Report};
    use replay_data::rows::DivisionLandInputsRow;

    let body = "units={division={id={id=7 type=51} max_supply=2.00000 \
        army_current_supply_ratio=0.00000 army_manpower={army_manpower_value={}}}}";
    for copies in [1, 2] {
        let mut array = vec![0];
        array.extend(std::iter::repeat_n(1, copies));
        let mut captured = frame(Recorded::Value(Some(array)));
        let text = format!("ABC={{{body}}} ").repeat(copies);
        let document = document(&text);
        assert_eq!(country_items(&document, &captured).unwrap().len(), 1);
        let saved = land::read_projected_divisions(&document, &captured).unwrap();
        let row = DivisionLandInputsRow {
            division: (7_u64 << 32) | 51,
            ..Default::default()
        };
        *captured.tables[TableId::DivisionLandInputs.index()] =
            TableView::Complete(TableRows::DivisionLandInputs(vec![row.clone()]));
        let mut report = Report::default();
        land::compare_divisions(&captured, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        let unsaved = DivisionLandInputsRow {
            division: (8_u64 << 32) | 51,
            ..Default::default()
        };
        *captured.tables[TableId::DivisionLandInputs.index()] =
            TableView::Complete(TableRows::DivisionLandInputs(vec![row, unsaved]));
        let mut report = Report::default();
        land::compare_divisions(&captured, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        if copies == 2 {
            assert!(land::read(document.items(), land::Comparison::Divisions).is_err());
        }
    }
    let captured = frame(Recorded::Value(Some(vec![0, 1, 1])));
    let changed = document(&format!("ABC={{{body}}} ABC={{{body} extra=0}}"));
    assert!(land::read_projected_divisions(&changed, &captured).is_err());
}

#[test]
fn physical_order_and_canonical_roster_mismatches_are_failures() {
    let captured = frame(Recorded::Value(Some(vec![0, 1, 0])));
    assert!(parse_country_emissions(&document("RES={} ABC={}"), &captured).is_err());
    assert!(parse_country_emissions(&document("ABC={}"), &captured).is_err());
    assert!(country_emission_layout(&frame(Recorded::Value(Some(vec![0, 0, 1])))).is_err());
}

#[test]
fn projected_full_oracle_scopes_canonical_censuses_and_refuses_uncaptured_reserved_fields() {
    use replay_data::{rows::CountryAppearanceRow, rows::DivisionCensusRow};
    let mut baseline_extended = None;
    for aliases in [0, 1, 2] {
        let (mut captured, _, _, equipment) = super::super::fixtures::fixture();
        let country = captured.complete::<tables::Countries>().unwrap()[0].clone();
        let mut physical = vec![country.id; country.canonical_slot as usize + 1];
        physical[0] = 0;
        physical.extend(std::iter::repeat_n(0, aliases));
        let tag = std::str::from_utf8(&country.tag).unwrap();
        let ordinary = format!(
            "{tag}={{cosmetic_tag=\"\" politics={{political_power=10.00001}} \
            command_power=2 stability=0.7 war_support=0.3 convoys={{equipment={{}}}}}} "
        );
        let text = ordinary.repeat(country.canonical_slot as usize) + &"RES={} ".repeat(aliases);
        let document = document(&text);
        let TableView::Complete(TableRows::Clock(rows)) =
            &mut *captured.tables[TableId::Clock.index()]
        else {
            unreachable!()
        };
        rows[0].seed = 0;
        let TableView::Complete(TableRows::Campaign(rows)) =
            &mut *captured.tables[TableId::Campaign.index()]
        else {
            unreachable!()
        };
        rows[0].id = b"c".to_vec();
        *captured.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(Vec::new()));
        *captured.tables[TableId::Provinces.index()] =
            TableView::Complete(TableRows::Provinces(Vec::new()));
        *captured.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    tag: b"RES".to_vec(),
                    country_array: Recorded::Value(Some(physical)),
                    original_tag_reference: Recorded::Value(Some(0)),
                    original_tag: Recorded::Value(None),
                    land_theatre_count: Recorded::Value(Some(0)),
                    ..Default::default()
                },
            ]));
        *captured.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: country.id,
                original_tag_reference: Recorded::Value(Some(0)),
                original_tag: Recorded::Value(None),
                ..Default::default()
            }]));
        *captured.tables[TableId::DivisionCensus.index()] =
            TableView::Complete(TableRows::DivisionCensus(vec![DivisionCensusRow {
                country: country.id,
                grouped: 0,
                ungrouped: 0,
            }]));
        let mut saved = parse_country_emissions(&document, &captured).unwrap();
        let membership: installed_content_save::PartialMapMembership = installed_content_save::MapMembership {
            provinces: std::collections::BTreeMap::from([(0, None)]),
        }
        .into();
        let compare = |saved: &Save| {
            let mut report = super::super::compare_core(
                &captured,
                replay_data::identity::SampleBoundary::EndOfHour,
                saved,
                Ok(&membership),
                Ok(&equipment),
                None,
            )
            .unwrap();
            super::super::map_inputs::countries(&captured, saved, &mut report).unwrap();
            report
        };
        let mut report = compare(&saved);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.canonical_countries(&saved.countries).len(), 1);
        assert_eq!(
            saved.country_emissions.as_ref().unwrap().len(),
            country.canonical_slot as usize + aliases
        );
        assert_eq!(
            report.failed_comparisons.len(),
            usize::from(aliases != 0),
            "{:?}",
            report.failed_comparisons
        );
        assert_eq!(report.outcome().is_ok(), aliases == 0);
        if aliases != 0 {
            assert!(
                report
                    .failed_comparisons
                    .keys()
                    .any(|name| name.contains("uncaptured save operands"))
            );
        }
        let context = super::super::country_context(&captured).unwrap();
        super::super::combat_log::division_census(&captured, &saved, &context.links, &mut report)
            .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            !report
                .failed_comparisons
                .contains_key("DivisionCensus omitted-group qualification")
        );
        let mut full = compare(&saved);
        super::super::compare_extended(
            &captured,
            &saved,
            super::super::ExtendedContent {
                membership: Ok(&membership),
                equipment: Ok(&equipment),
                base_resources: Err("fixture has no building content".into()),
                railway_adjacency: Err("fixture has no railway content".into()),
                buildings: Err("fixture has no building content".into()),
                defines: Err("fixture has no defines".into()),
                raid_graph: Err("fixture has no raid graph".into()),
                leader_traits: Err("fixture has no leader traits".into()),
                sub_units: Err("fixture has no sub-units".into()),
                essential_caps: Err("fixture has no sub-units".into()),
                intelligence: Err("fixture has no agency upgrades".into()),
                temperature: Err("fixture has no temperature content".into()),
                province_modifiers: Err("fixture has no weather content".into()),
            },
            &mut full,
        );
        let failures: std::collections::BTreeMap<_, _> = full
            .failed_comparisons
            .iter()
            .filter(|(name, _)| !name.starts_with("reserved country "))
            .map(|(name, reason)| (name.clone(), reason.clone()))
            .collect();
        if let Some((differences, failed)) = &baseline_extended {
            assert_eq!(&full.differences, differences);
            assert_eq!(&failures, failed);
        } else {
            baseline_extended = Some((full.differences.clone(), failures));
        }
        saved
            .countries
            .get_mut(country.tag.as_slice())
            .unwrap()
            .cosmetic_tag = Some(b"changed".to_vec());
        assert!(
            compare(&saved)
                .differences
                .iter()
                .any(|field| field.contains("cosmetic_tag"))
        );
    }
}

#[test]
fn repeated_reserved_original_blocks_refuse_any_changed_saved_field() {
    let captured = frame(Recorded::Value(Some(vec![0, 1, 0, 0])));
    for countries in [
        "ABC={} RES={original_tag=A} RES={original_tag=B}",
        "ABC={} RES={politics={political_power=1}} RES={politics={political_power=2}}",
        "ABC={} RES={modded_payload={x=1}} RES={modded_payload={x=2}}",
    ] {
        assert!(
            parse_country_emissions(&document(countries), &captured)
                .unwrap_err()
                .to_string()
                .contains("differ at")
        );
    }
    assert!(parse_country_emissions(&document("ABC={} RES={} ABC={}"), &captured).is_err());
}

#[test]
fn template_configuration_census_refuses_reserved_aliases_and_keeps_canonical_gaps() {
    for aliases in [0, 1, 2] {
        let mut array = vec![0, 1];
        array.extend(std::iter::repeat_n(0, aliases));
        let captured = frame(Recorded::Value(Some(array)));
        let saved = parse_country_emissions(
            &document(&format!("ABC={{}} {}", "RES={} ".repeat(aliases))),
            &captured,
        )
        .unwrap();
        let context = super::super::country_context(&captured).unwrap();
        let mut report = super::super::report::Report::default();
        report.record_country_emissions(&saved);
        report.refuse_uncaptured_reserved_operands();
        super::super::military::compare_country_template_state(
            &captured,
            &saved,
            &context,
            &mut report,
        )
        .unwrap();
        assert_eq!(
            report.differences,
            ["country ABC.template configuration: replay \"absent in recording\", save \"saved\""],
        );
        assert_eq!(report.failed_comparisons.len(), usize::from(aliases != 0));
        if aliases != 0 {
            assert!(
                report
                    .failed_comparisons
                    .contains_key("reserved country RES uncaptured save operands")
            );
            let mut strict = super::super::report::Report::default();
            super::super::military::compare_country_template_state(
                &captured,
                &saved,
                &context,
                &mut strict,
            )
            .unwrap();
            assert_eq!(strict.differences.len(), 2);
            assert!(
                strict
                    .differences
                    .iter()
                    .any(|why| why.starts_with("country RES.template configuration:"))
            );
        }
    }
}
