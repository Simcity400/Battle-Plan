//! CountryFactories against a text save. The save writes none of the stored
//! factory totals: CProductionStatus::Serialize (140E5D3B0) omits +0x2B0..+0x4A8
//! and a load recalculates them. What it writes are their inputs, so the one
//! comparison is each group's building levels: the levels of the saved state
//! buildings the country controls and of its saved offsite buildings whose
//!
//! The stored levels are those of the last recalculation. A country whose
//! save shows one pending (production `dirty`, or `refresh` bit 1 or 9, which
//! CGameState::PreHourlyUpdate applies at the next hour) is listed, not
//! compared.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::{
    buildings::Buildings,
    save::{Save, StateControl},
};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
};

use super::{Report, complete, invalid, required_country_tag};

/// RefreshDerivedStateByFlags (1406F2680): bit 1 rebuilds modifiers, which
/// recalculates the factory totals when modifier 0x68 changed (1406CE2A0 ->
/// 140E58F30); bit 9 recalculates them (140E57F10).
const REFRESH_RECALCULATES: u32 = 1 << 1 | 1 << 9;

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    control: &[StateControl],
    content: &Buildings,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = BTreeMap::new();
    for row in complete::<tables::CountryFactories>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        if actual.insert(tag, row).is_some() {
            return Err(invalid("duplicate resolved country in factory table"));
        }
    }
    let controllers = control
        .iter()
        .map(|state| (state.id, state.controller.as_slice()))
        .collect::<BTreeMap<_, _>>();
    // [military, naval, civilian] levels by country, and the countries a type
    // with ambiguous flags reaches.
    let mut expected = BTreeMap::<&[u8], [i64; 3]>::new();
    let mut ambiguous = BTreeSet::<&[u8]>::new();
    let placed = saved
        .construction
        .state_buildings
        .iter()
        .map(|((state, _), building)| {
            controllers
                .get(state)
                .copied()
                .map(|country| (country, building))
                .ok_or_else(|| {
                    invalid(format!(
                        "saved buildings of state {state} have no saved controller"
                    ))
                })
        })
        .chain(
            saved
                .construction
                .country_buildings
                .iter()
                .map(|((country, _), building)| Ok((country.as_slice(), building))),
        );
    for entry in placed {
        let (country, building) = entry?;
        let name = String::from_utf8_lossy(&building.building_type);
        let flags = content.factory_flags.get(name.as_ref()).ok_or_else(|| {
            invalid(format!(
                "saved building type {name} is not declared in the content"
            ))
        })?;
        if content.ambiguous_factory_types.contains(name.as_ref()) {
            ambiguous.insert(country);
        }
        let levels = expected.entry(country).or_default();
        for (sum, counted) in
            levels
                .iter_mut()
                .zip([flags.military(), flags.naval(), flags.civilian()])
        {
            if counted {
                *sum += i64::from(building.level);
            }
        }
    }
    let productions = saved
        .production
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks the production countries"))?;
    let (mut count, mut pending, mut refused) = (0, 0, 0);
    for (tag, country) in &saved.countries {
        let text = String::from_utf8_lossy(tag);
        let label = format!("{text}.factories");
        let Some(actual) = actual.remove(tag.as_slice()) else {
            report.difference(label, "absent", "saved country present");
            continue;
        };
        let refresh = country
            .refresh
            .ok_or_else(|| invalid(format!("saved country {text} lacks refresh")))?;
        let dirty = productions
            .get(tag)
            .and_then(|country| country.production.as_ref())
            .and_then(|production| production.dirty)
            .ok_or_else(|| {
                invalid(format!(
                    "saved country {text} lacks its production dirty flag"
                ))
            })?;
        if dirty || refresh & REFRESH_RECALCULATES != 0 {
            pending += 1;
            continue;
        }
        if ambiguous.contains(tag.as_slice()) {
            refused += 1;
            continue;
        }
        count += 1;
        let levels = expected.get(tag.as_slice()).copied().unwrap_or_default();
        for ((name, actual), expected) in [
            ("military_levels", actual.military_levels),
            ("naval_levels", actual.naval_levels),
            ("civilian_levels", actual.civilian_levels),
        ]
        .into_iter()
        .zip(levels)
        {
            if i64::from(actual) != expected {
                report.difference(format!("{label}.{name}"), actual, expected);
            }
        }
    }
    for tag in actual.keys() {
        report.difference(
            format!("{}.factories", String::from_utf8_lossy(tag)),
            "present",
            "country absent",
        );
    }
    report.compared("CountryFactories", count);
    report.limitation(
        "the save writes neither the stored group-head levels nor the other factory totals (CProductionStatus::Serialize 140E5D3B0 omits them; RecalculateFactoryTotals 140E57F10 rebuilds them); levels are compared only with saved building inputs: totals, controlled, owned and damaged factories (state output factors and country modifiers are not saved), assigned and withheld factories, subject and other-country transfers, licence and trade exports and each civilian consumer's allocation, consumer goods included, are not compared",
        "CountryFactories",
    );
    if pending > 0 {
        report.limitation(
            "a country whose save shows a factory recalculation pending (production dirty, or refresh bit 1 or 9) holds totals from before it; its levels are not compared",
            format!("CountryFactories: {pending} countries"),
        );
    }
    if refused > 0 {
        report.limitation(
            "a building type declared again with different factory flags: which declaration the game uses is unverified, so countries with such buildings are not compared",
            format!("CountryFactories: {refused} countries"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{CountryFactoriesRow, TableId, TableRows},
    };

    /// GER controls state 1 and occupies ITA's state 2; ITA owns state 3.
    fn save(ger: &str, ita: &str) -> Save {
        installed_content::save::parse(
            format!(
                r#"HOI4txt
date="1936.1.2.17" game_unique_id="campaign" version="build" game_unique_seed=-42 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={{ GER={{ {ger} buildings={{ arms={{level=2 healthy_levels=2 partial_health=0}} }} }} ITA={{ {ita} }} }}
states={{
  1={{owner="GER" buildings={{ civ={{level=5 healthy_levels=3 partial_health=0}} arms={{level=1 healthy_levels=1 partial_health=0}} infrastructure={{level=4 healthy_levels=4 partial_health=0}} }} }}
  2={{owner="ITA" controller="GER" buildings={{ civ={{level=2 healthy_levels=2 partial_health=0}} yard={{level=3 healthy_levels=3 partial_health=0}} }} }}
  3={{owner="ITA" buildings={{ both={{level=1 healthy_levels=1 partial_health=0}} }} }}
}}
provinces={{}}
"#
            )
            .as_bytes(),
        )
        .unwrap()
    }

    const CLEAN: &str = "refresh=8 production={ dirty=no }";

    fn content(extra: &str) -> Buildings {
        installed_content::buildings::parse(
            format!("buildings={{ civ={{general_production=1}} arms={{military_production=1}} yard={{naval_production=1}} infrastructure={{}} both={{military_production=1 general_production=1}} {extra} }}")
                .as_bytes(),
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap()
    }

    fn row(country: u32, levels: [i32; 3]) -> CountryFactoriesRow {
        CountryFactoriesRow {
            country,
            military_levels: levels[0],
            naval_levels: levels[1],
            civilian_levels: levels[2],
            // Totals are not compared.
            civilian_total_raw: -1,
            consumer_goods: -1,
            ..Default::default()
        }
    }

    fn run(
        saved: &Save,
        content: &Buildings,
        rows: Vec<CountryFactoriesRow>,
    ) -> io::Result<Report> {
        run_with_membership(
            saved,
            content,
            rows,
            &installed_content::save::MapMembership {
                provinces: BTreeMap::from([(1, Some(1)), (2, Some(2)), (3, Some(3))]),
            },
        )
    }

    fn run_with_membership(
        saved: &Save,
        content: &Buildings,
        rows: Vec<CountryFactoriesRow>,
        membership: &installed_content::save::MapMembership,
    ) -> io::Result<Report> {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::CountryFactories.index()] =
            TableView::Complete(TableRows::CountryFactories(rows));
        let germany = CountryRow {
            id: 5,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let italy = CountryRow {
            id: 6,
            tag: b"ITA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(5, &germany), (6, &italy)]);
        let control = saved.state_control(membership)?;
        let mut report = Report::default();
        compare(&frame, saved, &control, content, &links, &mut report)?;
        Ok(report)
    }

    #[test]
    fn history_building_effects_compare_saved_levels_without_waiving_a_difference() {
        let membership = super::super::state_history_tests::membership(
            "state={id=1 provinces={1} history={buildings={civ=99} \
             1936.1.1={IF={limit={} remove_building={type=civ level=2} \
             strategic_state_location={authored_location=1} \
             strategic_province_location={authored_location=1}}}}} \
             state={id=2 provinces={2}} state={id=3 provinces={3}}",
            3,
        );
        let saved = save(CLEAN, CLEAN);
        let mut rows = vec![row(5, [3, 3, 7]), row(6, [1, 0, 1])];
        let report = run_with_membership(&saved, &content(""), rows.clone(), &membership).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryFactories"], 2);
        rows[0].civilian_levels -= 1;
        let report = run_with_membership(&saved, &content(""), rows, &membership).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("GER.factories.civilian_levels"));
    }

    #[test]
    fn controlled_and_offsite_levels_compare_per_group() {
        let saved = save(CLEAN, CLEAN);
        // GER: state 1 and occupied state 2 plus two offsite arms levels.
        let rows = vec![row(5, [1 + 2, 3, 5 + 2]), row(6, [1, 0, 1])];
        let report = run(&saved, &content(""), rows.clone()).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryFactories"], 2);
        let mut off = rows;
        off[0].naval_levels = 2;
        off[1].civilian_levels = 0;
        let report = run(&saved, &content(""), off).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    }

    #[test]
    fn pending_recalculations_and_ambiguous_types_are_listed_not_compared() {
        let rows = || vec![row(5, [0; 3]), row(6, [0; 3])];
        for ger in [
            "refresh=10 production={ dirty=no }",
            "refresh=512 production={ dirty=no }",
            "refresh=0 production={ dirty=yes }",
        ] {
            let report = run(&save(ger, CLEAN), &content(""), rows()).unwrap();
            // Only ITA is compared, and its wrong levels are a difference.
            assert_eq!(report.compared_tables["CountryFactories"], 1, "{ger}");
            assert_eq!(report.differences.len(), 2, "{ger}");
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("recalculation pending"))
            );
        }
        // `both` declared again with other flags reaches only ITA.
        let report = run(
            &save(CLEAN, CLEAN),
            &content("both={general_production=1}"),
            vec![row(5, [3, 3, 7]), row(6, [0; 3])],
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryFactories"], 1);
    }

    #[test]
    fn missing_rows_inputs_or_content_are_not_silently_compared() {
        let report = run(&save(CLEAN, CLEAN), &content(""), vec![row(5, [3, 3, 7])]).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        let rows = || vec![row(5, [3, 3, 7]), row(6, [1, 0, 1])];
        assert!(
            run(
                &save("production={ dirty=no }", CLEAN),
                &content(""),
                rows()
            )
            .is_err()
        );
        assert!(run(&save("refresh=0", CLEAN), &content(""), rows()).is_err());
        let undeclared = installed_content::buildings::parse(
            b"buildings={ civ={} arms={} yard={} infrastructure={} }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        assert!(run(&save(CLEAN, CLEAN), &undeclared, rows()).is_err());
    }
}
