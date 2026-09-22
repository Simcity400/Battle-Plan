//! Saved operands for the three hourly map input tables.
use super::*;

fn eq<T: PartialEq + std::fmt::Debug>(report: &mut Report, label: &str, actual: T, expected: T) {
    report.scalar_checks += 1;
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

pub(super) fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::CountryAppearance, |r| countries(frame, save, r));
    report.attempt_table(TableId::ArmyIconInputs, |r| armies(frame, save, r));
    report.attempt_table(TableId::ShipModels, |r| ships(frame, save, r));
}

fn countries(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let identities: BTreeMap<_, _> = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let mut remaining: BTreeSet<_> = save.countries.keys().map(Vec::as_slice).collect();
    let mut seen = BTreeSet::new();
    let rows = frame.complete::<tables::CountryAppearance>()?;
    for row in rows {
        if !seen.insert(row.country) {
            return Err(invalid("duplicate CountryAppearance country"));
        }
        let country = identities
            .get(&row.country)
            .ok_or_else(|| invalid("CountryAppearance key has no Countries row"))?;
        let tag = country.tag.as_slice();
        let label = format!("CountryAppearance {}", String::from_utf8_lossy(tag));
        let Some(saved) = save.countries.get(tag) else {
            report.difference(label, "present", "absent");
            continue;
        };
        remaining.remove(tag);
        let cosmetic = saved
            .cosmetic_tag
            .as_ref()
            .ok_or_else(|| invalid(format!("{label} save lacks unconditional cosmetic_tag")))?;
        eq(
            report,
            &format!("{label}.cosmetic_tag"),
            &row.cosmetic_tag,
            cosmetic,
        );
    }
    for tag in remaining {
        report.difference(
            format!("CountryAppearance {}", String::from_utf8_lossy(tag)),
            "absent",
            "present",
        );
    }
    eq(
        report,
        "CountryAppearance Countries key census",
        seen,
        identities.keys().copied().collect(),
    );
    report.compared("CountryAppearance", rows.len());
    report.limitation("the save carries cosmetic_tag but has no counterpart for current primary RGBA; all four raw f32 bit patterns, including alpha, remain independently unverified", "CountryAppearance raw RGBA");
    Ok(())
}

fn armies(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::ArmyIconInputs>()?;
    let mut remaining: BTreeSet<_> = save.divisions.keys().copied().collect();
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.division == 0 || !seen.insert(row.division) {
            return Err(invalid("null or duplicate ArmyIconInputs division"));
        }
        let label = format!("ArmyIconInputs {}", row.division);
        let Some(saved) = save.divisions.get(&row.division) else {
            report.difference(label, "present", "absent");
            continue;
        };
        remaining.remove(&row.division);
        eq(
            report,
            &format!("{label}.separate_counter"),
            row.separate_counter != 0,
            saved.separate_counter,
        );
        eq(
            report,
            &format!("{label}.alternate_template"),
            row.alternate_template,
            saved.alternate_template.unwrap_or(0),
        );
    }
    for id in remaining {
        report.difference(format!("ArmyIconInputs {id}"), "absent", "present");
    }
    let parents = frame
        .complete::<tables::Divisions>()?
        .iter()
        .map(|r| r.id)
        .collect();
    eq(report, "ArmyIconInputs Divisions key census", seen, parents);
    report.compared("ArmyIconInputs", rows.len());
    report.limitation("the saved separate-counter boolean distinguishes zero from nonzero, not different nonzero raw byte values; alternate-template omission is the serializer's null-pointer case", "ArmyIconInputs");
    Ok(())
}

/// FFAC80 starts from entry zero and replaces only on strictly greater signed
/// quantity. Do not sum duplicates, filter nonpositive values, or use max_by_key
/// (whose tie behavior selects the last entry).
fn selected_variant(pool: &save::production::Pool) -> Option<u64> {
    let mut selected = None;
    for entry in &pool.equipment {
        if selected.is_none_or(|previous: &save::production::Equipment| {
            entry.amount.raw > previous.amount.raw
        }) {
            selected = Some(entry);
        }
    }
    // FFDB00 retains negative entries, but omits zeros unless allow_zero_entries
    // is enabled. Hidden zeros can beat every negative or make an empty saved
    // list nonempty natively. Never turn those cases into an invented variant0.
    if pool.allow_zero_entries == Some(true) || selected.is_some_and(|entry| entry.amount.raw >= 0)
    {
        Some(selected.map_or(0, |entry| entry.variant))
    } else {
        None
    }
}

fn ships(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::ShipModels>()?;
    let mut remaining: BTreeSet<_> = save.ships.keys().copied().collect();
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.ship == 0 || !seen.insert(row.ship) {
            return Err(invalid("null or duplicate ShipModels ship"));
        }
        let label = format!("ShipModels {}", row.ship);
        let Some(saved) = save.ships.get(&row.ship) else {
            report.difference(label, "present", "absent");
            continue;
        };
        remaining.remove(&row.ship);
        if let Some(counter) = &row.counter {
            eq(
                report,
                &format!("{label}.roster_ordinal"),
                counter.roster_ordinal,
                saved.roster_ordinal,
            );
        } else {
            return Err(invalid("ShipModels counter payload absent"));
        }
        let pool = saved
            .equipment
            .as_ref()
            .ok_or_else(|| invalid(format!("{label} save lacks unconditional equipment pool")))?;
        if let Some(expected) = selected_variant(pool) {
            eq(report, &format!("{label}.variant"), row.variant, expected);
        } else {
            report.limitation("FFDB00 omits zero-quantity entries unless allow_zero_entries is enabled; an empty or all-negative saved pool cannot identify the native maximum when omitted zeros may exist, so this ship's selected variant is not compared", format!("{label}.variant"));
        }
    }
    for id in remaining {
        report.difference(format!("ShipModels {id}"), "absent", "present");
    }
    let parents = frame
        .complete::<tables::Ships>()?
        .iter()
        .map(|r| r.id)
        .collect();
    eq(report, "ShipModels Ships key census", seen, parents);
    report.compared("ShipModels", rows.len());
    report.limitation("selected variant identity is independently derived from the saved equipment pool; saved registry definition keys and variant names are compared by VariantDefinitions on its refresh hours, not inferred from hull sprite bytes", "ShipModels.variant names");
    report.limitation("ship hull/fallback sprite keys, model number, and full formation-selection flags, positive-equipment flag union, definition-valid byte, carrier airbase reference, strength maximum and derived organisation maximum have no direct counterpart in the inspected ship serializer; they cannot be independently checked against this text save", "ShipModels hull_sprite, fallback_sprite, model_number, formation_flags");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{ArmyIconInputRow, CountryAppearanceRow, ShipModelRow};

    fn pool(source: &[u8]) -> save::production::Pool {
        let doc = installed_content::parser::parse(source).unwrap();
        save::production::parse_pool(&doc.items).unwrap()
    }

    #[test]
    fn ship_variant_selection_uses_signed_amounts_and_first_tie_without_merging() {
        assert_eq!(selected_variant(&pool(b"")), None);
        assert_eq!(selected_variant(&pool(b"allow_zero_entries=yes")), Some(0));
        assert_eq!(
            selected_variant(&pool(
                b"equipment={id={id=1 type=70} amount=-92233720368547.75808}"
            )),
            None
        );
        assert_eq!(selected_variant(&pool(b"allow_zero_entries=yes equipment={id={id=1 type=70} amount=-92233720368547.75808}")), Some((1_u64 << 32) | 70));
        let bank = pool(b"equipment={id={id=1 type=70} amount=-1} equipment={id={id=2 type=70} amount=0} equipment={id={id=3 type=70} amount=0} equipment={id={id=1 type=70} amount=-1}");
        assert_eq!(selected_variant(&bank), Some((2_u64 << 32) | 70));
        let bank = pool(b"equipment={id={id=1 type=70} amount=2} equipment={id={id=2 type=70} amount=3} equipment={id={id=1 type=70} amount=2}");
        assert_eq!(selected_variant(&bank), Some((2_u64 << 32) | 70));
    }

    #[test]
    fn map_tables_compare_saved_fields_and_preserve_source_failures() {
        let (mut frame, mut saved, _, _) = super::super::tests::fixture();
        let division = (1_u64 << 32) | 51;
        let ship = (2_u64 << 32) | 51;
        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .cosmetic_tag = Some(b"GER_cosmetic".to_vec());
        saved.divisions.insert(
            division,
            save::Division {
                id: division,
                roster_owner: b"GER".to_vec(),
                logical_owner: b"GER".to_vec(),
                expeditionary_owner: None,
                province: None,
                template: None,
                alternate_template: Some(123),
                separate_counter: true,
                hit_points: Decimal::parse(b"1").unwrap(),
                organisation: Decimal::parse(b"1").unwrap(),
                country_intel: Vec::new(),
                presentation: Default::default(),
            },
        );
        saved.ships.insert(
            ship,
            save::Ship {
                roster_ordinal: 0,
                id: ship,
                task_force: 9,
                strength: Decimal::parse(b"1").unwrap(),
                organisation: Decimal::parse(b"1").unwrap(),
                experience: Decimal::parse(b"0").unwrap(),
                sunk_convoys: 0,
                equipment: Some(pool(b"equipment={id={id=3 type=70} amount=1}")),
            },
        );
        frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![replay::rows::DivisionRow {
                id: division,
                ..Default::default()
            }]));
        frame.tables[TableId::Ships.index()] =
            TableView::Complete(TableRows::Ships(vec![replay::rows::ShipRow {
                id: ship,
                ..Default::default()
            }]));
        frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 101,
                cosmetic_tag: b"GER_cosmetic".to_vec(),
                red: u32::MAX,
                ..Default::default()
            }]));
        frame.tables[TableId::ArmyIconInputs.index()] =
            TableView::Complete(TableRows::ArmyIconInputs(vec![ArmyIconInputRow {
                division,
                separate_counter: 255,
                alternate_template: 123,
            }]));
        frame.tables[TableId::ShipModels.index()] =
            TableView::Complete(TableRows::ShipModels(vec![ShipModelRow {
                ship,
                variant: (3_u64 << 32) | 70,
                counter: Some(Default::default()),
                ..Default::default()
            }]));
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(report.compared_tables.len(), 3);
        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .cosmetic_tag = Some(Vec::new());
        saved.divisions.get_mut(&division).unwrap().separate_counter = false;
        saved
            .divisions
            .get_mut(&division)
            .unwrap()
            .alternate_template = None;
        saved.ships.get_mut(&ship).unwrap().equipment =
            Some(pool(b"equipment={id={id=4 type=70} amount=1}"));
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert_eq!(report.differences.len(), 4);
        saved.ships.get_mut(&ship).unwrap().roster_ordinal = 1;
        let mut roster = Report::default();
        ships(&frame, &saved, &mut roster).unwrap();
        assert!(
            roster
                .differences
                .iter()
                .any(|d| d.contains("roster_ordinal"))
        );
        saved.ships.get_mut(&ship).unwrap().equipment = None;
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert!(report.failed_comparisons["ShipModels"].contains("unconditional equipment"));
        frame.tables[TableId::ArmyIconInputs.index()] = TableView::Unavailable {
            reason: "injected read fault".into(),
        };
        frame.tables[TableId::CountryAppearance.index()] = TableView::Truncated {
            rows: TableRows::CountryAppearance(Vec::new()),
            missing: 1,
        };
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert!(report.failed_comparisons.contains_key("CountryAppearance"));
        assert!(report.failed_comparisons.contains_key("ArmyIconInputs"));
    }
}
