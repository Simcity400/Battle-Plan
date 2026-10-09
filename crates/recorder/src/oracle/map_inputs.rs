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
    report.attempt_table(TableId::ReservedCountryNaming, |r| {
        reserved_naming(frame, r)
    });
    report.attempt_table(TableId::ArmyIconInputs, |r| armies(frame, save, r));
    report.attempt_table(TableId::ShipModels, |r| ships(frame, save, r));
}

fn reserved_naming(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::ReservedCountryNaming>()?;
    if rows.len() != 1 {
        return Err(invalid(
            "ReservedCountryNaming requires its one reserved-country row",
        ));
    }
    report.compared("ReservedCountryNaming", 0);
    if let Some(None) = report.recorded(
        &rows[0].original_tag,
        "ReservedCountryNaming.original_tag",
        "reserved country original tag",
    ) {
        report.limitation(
            "reserved country original tag unreadable at capture; the independent naming operands remain available",
            "ReservedCountryNaming.original_tag: 1 guarded read or arena refusal",
        );
    }
    report.limitation("CCountry::Serialize@14070C580 is reached through the positive-country save census; reserved slot zero naming operands and stored-length built witness have no proved save counterpart", "ReservedCountryNaming: singleton decoded, naming and built remain unverified");
    report.limitation("CCountry+0x130C original presence and exact tag_string@140BA5C20 bytes are runtime operands for the reserved object; CGameState::Write@1401F2A20 starts its +0x310 country census at slot one (1401F4185/1401F4190), so the save omits slot zero", "ReservedCountryNaming.original_tag: exact original presence/bytes are unsaved and remain unverified");
    if let Some(None) = report.recorded(
        &rows[0].original_tag_reference,
        "ReservedCountryNaming.original_tag_reference",
        "reserved country signed original tag reference",
    ) {
        report.limitation(
            "reserved country original tag reference unreadable at capture",
            "ReservedCountryNaming.original_tag_reference: guarded read refusal",
        );
    }
    report.limitation("the positive-country save census omits reserved slot zero, including its exact signed +0x130C reference", "ReservedCountryNaming.original_tag_reference: unsaved");
    if let Some(None) = report.recorded(
        &rows[0].original_tag_buckets,
        "ReservedCountryNaming.original_tag_buckets",
        "native original-tag bucket cache",
    ) {
        report.limitation(
            "native original-tag bucket cache unreadable or short of arena space at capture",
            "ReservedCountryNaming.original_tag_buckets: guarded read or arena refusal",
        );
    }
    report.limitation("CGameState+0x988 is a runtime original-tag bucket cache rebuilt by 1401DA010 and changed by native setters; serialized countries cannot certify retained cache order, repeats or alias reference identity", "ReservedCountryNaming.original_tag_buckets: unsaved native cache");
    if let Some(None) = report.recorded(
        &rows[0].country_array,
        "ReservedCountryNaming.country_array",
        "physical country-array occurrences",
    ) {
        report.limitation(
            "physical country array unreadable or short of arena space at capture",
            "ReservedCountryNaming.country_array: guarded read or arena refusal",
        );
    }
    report.limitation("the country save census starts at physical slot one and serializes tags rather than exact raw CCountry+0x8 IDs; it cannot certify the full physical array's reserved-zero, alias and duplicate occurrences", "ReservedCountryNaming.country_array: raw physical order and identity remain unverified");
    if let Some(None) = report.recorded(
        &rows[0].owned_state_count,
        "ReservedCountryNaming.owned_state_count",
        "reserved country signed owned-state count",
    ) {
        report.limitation("reserved country owned-state count unreadable at capture; other naming operands remain available", "ReservedCountryNaming.owned_state_count: guarded read refusal");
    }
    report.limitation("CGameState::Write@1401F2A20 begins its country census at physical slot one and omits reserved Country0, so its exact signed +0x484 getter operand has no fresh-save counterpart", "ReservedCountryNaming.owned_state_count: unsaved reserved getter operand");
    if let Some(None) = report.recorded(
        &rows[0].dynamic_tag_boundary,
        "ReservedCountryNaming.dynamic_tag_boundary",
        "native dynamic-country boundary",
    ) {
        report.limitation(
            "native dynamic-country boundary unreadable at capture",
            "ReservedCountryNaming.dynamic_tag_boundary: guarded read refusal",
        );
    }
    report.limitation("CountryDatabase+0x88 is a runtime signed slot boundary read by 140BA61F0; CGameState::Write and CCountry::Serialize do not serialize it, so fresh-save original-tag text cannot certify this predicate", "ReservedCountryNaming.dynamic_tag_boundary: unsaved native predicate; matched-hour memory needed");
    Ok(())
}

#[cfg(test)]
mod original_tag_tests {
    use super::*;
    use replay::rows::{Recorded, ReservedCountryNamingRow};

    #[test]
    fn reserved_original_tag_oracle_reports_unsaved_and_independent_refusal() {
        let (mut frame, _, _, _) = super::super::tests::fixture();
        for tag in [
            Recorded::Value(Some(vec![0xff, 0])),
            Recorded::Value(Some(Vec::new())),
            Recorded::Value(None),
            Recorded::NotRecorded,
        ] {
            *frame.tables[TableId::ReservedCountryNaming.index()] =
                TableView::Complete(TableRows::ReservedCountryNaming(vec![
                    ReservedCountryNamingRow {
                        tag: b"ZERO".to_vec(),
                        original_tag: tag.clone(),
                        ..Default::default()
                    },
                ]));
            let mut report = Report::default();
            reserved_naming(&frame, &mut report).unwrap();
            assert!(report.differences.is_empty());
            assert_eq!(report.scalar_checks, 0);
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("CGameState::Write@1401F2A20")
                        && reason.contains("save omits slot zero"))
            );
            assert_eq!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("reserved country original tag unreadable")),
                matches!(tag, Recorded::Value(None))
            );
            assert_eq!(
                report
                    .limitations
                    .contains_key("ReservedCountryNaming.original_tag not recorded"),
                matches!(tag, Recorded::NotRecorded)
            );
        }
    }

    #[test]
    fn positive_original_tag_oracle_compares_presence_bytes_and_preserves_refusals() {
        use replay::rows::CountryAppearanceRow;
        for (reference, actual, saved, differences) in [
            (i32::MIN, Some(Vec::new()), None, 0),
            (0, Some(Vec::new()), Some(Vec::new()), 1),
            (7, Some(Vec::new()), Some(Vec::new()), 0),
            (7, Some(Vec::new()), None, 1),
            (7, Some(b"OWN".to_vec()), Some(b"OWN".to_vec()), 0),
            (7, Some(b"OTHER".to_vec()), Some(b"OTHER".to_vec()), 0),
            (-1, Some(Vec::new()), Some(b"OWN".to_vec()), 1),
            (7, Some(b"OWN".to_vec()), None, 1),
            (7, Some(b"OTHER".to_vec()), Some(b"OWN".to_vec()), 1),
            (7, Some(vec![b'A', 0, b'B']), Some(b"A".to_vec()), 1),
        ] {
            let row = CountryAppearanceRow {
                original_tag: Recorded::Value(actual),
                original_tag_reference: Recorded::Value(Some(reference)),
                ..Default::default()
            };
            let saved = save::Country {
                original_tag: saved,
                ..Default::default()
            };
            let mut report = Report::default();
            original_tag(&row, &saved, "OWN", &mut report);
            assert_eq!(report.differences.len(), differences);
            assert_eq!(report.scalar_checks, 1);
        }
        for (value, reason) in [
            (
                Recorded::Value(None),
                "country original tag unreadable at capture; existing appearance remains available",
            ),
            (
                Recorded::NotRecorded,
                "CountryAppearance.original_tag not recorded",
            ),
        ] {
            let row = CountryAppearanceRow {
                original_tag: value,
                original_tag_reference: Recorded::Value(Some(7)),
                ..Default::default()
            };
            let mut report = Report::default();
            original_tag(&row, &save::Country::default(), "OWN", &mut report);
            assert!(report.limitations.contains_key(reason));
            assert_eq!(report.scalar_checks, 0);
            assert!(report.differences.is_empty());
        }
    }

    #[test]
    fn raw_reference_and_native_domain_oracles_preserve_gaps_and_refusals() {
        for (value, reason) in [
            (
                Recorded::Value(None),
                "country original tag reference unreadable at capture; existing appearance remains available",
            ),
            (
                Recorded::NotRecorded,
                "CountryAppearance.original_tag_reference not recorded",
            ),
        ] {
            let row = replay::rows::CountryAppearanceRow {
                original_tag: Recorded::Value(Some(b"OWN".to_vec())),
                original_tag_reference: value,
                ..Default::default()
            };
            let mut report = Report::default();
            original_tag(&row, &save::Country::default(), "OWN", &mut report);
            assert!(report.limitations.contains_key(reason));
            assert_eq!(report.scalar_checks, 0);
            assert!(report.differences.is_empty());
        }
        let (mut frame, _, _, _) = super::super::tests::fixture();
        for absent in [false, true] {
            let row = ReservedCountryNamingRow {
                original_tag_reference: if absent {
                    Recorded::NotRecorded
                } else {
                    Recorded::Value(None)
                },
                original_tag_buckets: if absent {
                    Recorded::NotRecorded
                } else {
                    Recorded::Value(None)
                },
                country_array: if absent {
                    Recorded::NotRecorded
                } else {
                    Recorded::Value(None)
                },
                ..Default::default()
            };
            *frame.tables[TableId::ReservedCountryNaming.index()] =
                TableView::Complete(TableRows::ReservedCountryNaming(vec![row]));
            let mut report = Report::default();
            reserved_naming(&frame, &mut report).unwrap();
            for field in [
                "original_tag_reference",
                "original_tag_buckets",
                "country_array",
            ] {
                assert_eq!(
                    report
                        .limitations
                        .contains_key(&format!("ReservedCountryNaming.{field} not recorded")),
                    absent
                );
            }
            assert_eq!(report.scalar_checks, 0);
            assert!(report.differences.is_empty());
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("runtime original-tag bucket cache"))
            );
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("full physical array"))
            );
        }
    }

    #[test]
    fn reserved_owned_state_count_oracle_preserves_unsaved_values_and_gaps() {
        let (mut frame, _, _, _) = super::super::tests::fixture();
        for value in [
            Recorded::Value(Some(i32::MIN)),
            Recorded::Value(Some(0)),
            Recorded::Value(Some(7)),
            Recorded::Value(None),
            Recorded::NotRecorded,
        ] {
            *frame.tables[TableId::ReservedCountryNaming.index()] =
                TableView::Complete(TableRows::ReservedCountryNaming(vec![
                    ReservedCountryNamingRow {
                        owned_state_count: value,
                        ..Default::default()
                    },
                ]));
            let mut report = Report::default();
            reserved_naming(&frame, &mut report).unwrap();
            assert!(report.differences.is_empty());
            assert_eq!(report.scalar_checks, 0);
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("exact signed +0x484 getter operand"))
            );
            assert_eq!(
                report
                    .limitations
                    .contains_key("ReservedCountryNaming.owned_state_count not recorded"),
                matches!(value, Recorded::NotRecorded)
            );
            assert_eq!(report.limitations.contains_key("reserved country owned-state count unreadable at capture; other naming operands remain available"), matches!(value, Recorded::Value(None)));
        }
    }
    #[test]
    fn dynamic_boundary_oracle_reports_unsaved_values_and_explicit_refusals() {
        let (mut frame, _, _, _) = super::super::tests::fixture();
        for value in [
            Recorded::NotRecorded,
            Recorded::Value(None),
            Recorded::Value(Some(i32::MIN)),
            Recorded::Value(Some(0)),
        ] {
            *frame.tables[TableId::ReservedCountryNaming.index()] =
                TableView::Complete(TableRows::ReservedCountryNaming(vec![
                    ReservedCountryNamingRow {
                        dynamic_tag_boundary: value,
                        ..Default::default()
                    },
                ]));
            let mut report = Report::default();
            reserved_naming(&frame, &mut report).unwrap();
            assert!(report.differences.is_empty());
            assert_eq!(report.scalar_checks, 0);
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("runtime signed slot boundary"))
            );
            assert_eq!(
                report
                    .limitations
                    .contains_key("ReservedCountryNaming.dynamic_tag_boundary not recorded"),
                matches!(value, Recorded::NotRecorded)
            );
            assert_eq!(
                report
                    .limitations
                    .contains_key("native dynamic-country boundary unreadable at capture"),
                matches!(value, Recorded::Value(None))
            );
        }
    }
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
        original_tag(row, saved, &label, report);
        ruling_leader(save, tag, &label, row, report)?;
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
    report.limitation("the country writer serializes neither colour object (CCountry::ReadMember 1406F9170 reads tokens 0x56 color and 0x31B8 color_ui, but no writer emits 0x31B8), so the current UI RGBA (CCountry+0x390) has no save counterpart; its four raw f32 bit patterns need a matched-hour screenshot of a counter plate in country-colour mode", "CountryAppearance raw UI RGBA");
    report.limitation("name_tag and overlord are census only; the viewer's resolved names need a matched-hour screenshot", "CountryAppearance name operands");
    report.limitation("graphical_culture (CCountry+0x1028) is not serialized by the country writer; it comes from the country's content file and needs a matched-hour screenshot of a model or portrait that depends on it", "CountryAppearance.graphical_culture");
    report.limitation("graphical_culture_2d (CCountry+0x1050) is not serialized by the country writer; it comes from the country's content file and needs a matched-hour screenshot of an equipment icon that depends on it", "CountryAppearance.graphical_culture_2d");
    report.limitation("the old-name caches (CCountry+0x90, +0xB0, +0xD0) are runtime text: CCountry::Write (14070C580) does not save them and only 140709950 writes them, so a save cannot certify them; they need a live check of GetOld* text", "CountryAppearance old names");
    report.limitation("CDiplomacyStatus::Serialize@140D32BD0 does not save the ordered subject roster at +0x170/count +0x17C; active_relations do not certify this array's order, repeats or raw reference words", "CountryAppearance.subjects");
    Ok(())
}

/// 14070C580 emits original_tag/token3551 iff signed +0x130C>0; it passes
/// that exact reference to tag_string (positive raw-indexed live +0x358).
/// The signed reference selects original bytes, including actual empty strings;
/// nonpositive references certify saved omission without inventing zero.
fn original_tag(
    row: &replay::rows::CountryAppearanceRow,
    saved: &save::Country,
    label: &str,
    report: &mut Report,
) {
    let bytes = report.recorded(
        &row.original_tag,
        "CountryAppearance.original_tag",
        format!("{label}.original_tag"),
    );
    if matches!(bytes, Some(None)) {
        report.limitation(
            "country original tag unreadable at capture; existing appearance remains available",
            format!("{label}.original_tag: guarded read or arena refusal"),
        );
    }
    let Some(reference) = report.recorded(
        &row.original_tag_reference,
        "CountryAppearance.original_tag_reference",
        format!("{label}.original_tag_reference"),
    ) else {
        return;
    };
    let Some(reference) = reference else {
        report.limitation("country original tag reference unreadable at capture; existing appearance remains available", format!("{label}.original_tag_reference: guarded read refusal"));
        return;
    };
    report.limitation("CCountry::Serialize@14070C580 emits original tag bytes only for signed positive +0x130C; it does not serialize the raw reference magnitude or alias identity, including exact nonpositive values", format!("{label}.original_tag_reference={reference}: unsaved raw identity"));
    let actual = if *reference > 0 {
        let Some(Some(bytes)) = bytes else { return };
        Some(bytes.as_slice())
    } else {
        None
    };
    eq(
        report,
        &format!("{label}.original_tag"),
        actual,
        saved.original_tag.as_deref(),
    );
}

/// The ruling party's first saved `country_leader` against the recorded leader
/// ideology and character. CCountry::Write@14070C580 writes `politics`
/// unconditionally; CPoliticalStatus::vf2@140BA12D0 omits `ruling_party` for a
/// null party, and the party writer lists its leaders in vector order, the
/// and recorded as missing.
fn ruling_leader(
    save: &Save,
    tag: &[u8],
    label: &str,
    row: &replay::rows::CountryAppearanceRow,
    report: &mut Report,
) -> io::Result<()> {
    let politics = save
        .governance
        .countries
        .get(tag)
        .and_then(|country| country.politics.as_ref())
        .ok_or_else(|| invalid(format!("{label} save lacks unconditional politics")))?;
    let leader = match &politics.ruling_party {
        None => None,
        Some(party) => politics
            .leaders
            .get(party)
            .ok_or_else(|| invalid(format!("{label} ruling party is not a saved party")))?
            .first(),
    };
    eq(
        report,
        &format!("{label}.leader_ideology"),
        row.leader_ideology.as_slice(),
        leader.map_or(&[][..], |leader| leader.ideology.as_slice()),
    );
    eq(
        report,
        &format!("{label}.leader_character"),
        row.leader_character
            .map(|id| (id, engine::ccharacter::REFERENCE_TYPE)),
        leader
            .map(|leader| leader.character)
            .filter(|&reference| reference != (0, 0)),
    );
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
    report.limitation("compared: the ship census against the save and the Ships table, each ship's roster ordinal and, where the saved pool identifies it, its selected variant. Ship hull/fallback sprite keys, model number, the full formation-selection flags, positive-equipment flag union, definition-valid byte, carrier air-base reference, strength maximum, anti-air base (CShip+0x218) and derived organisation maximum have no counterpart in CShip::Serialize@140C2EF80; they cannot be independently checked against this text save", "ShipModels hull_sprite, fallback_sprite, model_number, formation_flags, counter operands");
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
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![replay::rows::DivisionRow {
                id: division,
                ..Default::default()
            }]));
        *frame.tables[TableId::Ships.index()] =
            TableView::Complete(TableRows::Ships(vec![replay::rows::ShipRow {
                id: ship,
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 101,
                cosmetic_tag: b"GER_cosmetic".to_vec(),
                red: u32::MAX,
                ..Default::default()
            }]));
        *frame.tables[TableId::ArmyIconInputs.index()] =
            TableView::Complete(TableRows::ArmyIconInputs(vec![ArmyIconInputRow {
                division,
                separate_counter: 255,
                alternate_template: 123,
            }]));
        *frame.tables[TableId::ShipModels.index()] =
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
        assert_eq!(report.compared_tables.len(), 4);
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
        *frame.tables[TableId::ArmyIconInputs.index()] = TableView::Unavailable {
            reason: "injected read fault".into(),
        };
        *frame.tables[TableId::CountryAppearance.index()] = TableView::Truncated {
            rows: TableRows::CountryAppearance(Vec::new()),
            missing: 1,
        };
        let mut report = Report::default();
        compare(&frame, &saved, &mut report);
        assert!(report.failed_comparisons.contains_key("CountryAppearance"));
        assert!(report.failed_comparisons.contains_key("ArmyIconInputs"));
    }

    #[test]
    fn ruling_leader_compares_the_first_saved_leader_and_keeps_missing_distinct() {
        fn politics(saved: &mut Save) -> &mut save::governance::Politics {
            saved
                .governance
                .countries
                .get_mut(b"GER".as_slice())
                .unwrap()
                .politics
                .as_mut()
                .unwrap()
        }
        let (mut frame, mut saved, _, _) = super::super::tests::fixture();
        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .cosmetic_tag = Some(Vec::new());
        politics(&mut saved).ruling_party = Some(b"fascism".to_vec());
        let leader = |ideology: &[u8], character| save::governance::PartyLeader {
            ideology: ideology.to_vec(),
            character,
        };
        politics(&mut saved).leaders.insert(
            b"fascism".to_vec(),
            vec![
                leader(b"nazism", (1800, 73)),
                leader(b"fascism_ideology", (1801, 73)),
            ],
        );
        let run = |frame: &mut DecodedFrame, saved: &Save, ideology: &[u8], character| {
            *frame.tables[TableId::CountryAppearance.index()] =
                TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                    country: 101,
                    leader_ideology: ideology.to_vec(),
                    leader_character: character,
                    ..Default::default()
                }]));
            let mut report = Report::default();
            report.attempt_table(TableId::CountryAppearance, |r| countries(frame, saved, r));
            report
        };
        let report = run(&mut frame, &saved, b"nazism", Some(1800));
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        // A missing character, another character or leader, and a zero id all differ.
        for (ideology, character) in [
            (b"nazism".as_slice(), None),
            (b"nazism", Some(1801)),
            (b"nazism", Some(0)),
            (b"fascism_ideology", Some(1801)),
            (b"", None),
        ] {
            let report = run(&mut frame, &saved, ideology, character);
            assert!(!report.differences.is_empty(), "{ideology:?} {character:?}");
        }
        // A saved null parent reference is recorded as missing.
        politics(&mut saved)
            .leaders
            .get_mut(b"fascism".as_slice())
            .unwrap()[0]
            .character = (0, 0);
        assert!(
            run(&mut frame, &saved, b"nazism", None)
                .differences
                .is_empty()
        );
        assert!(
            !run(&mut frame, &saved, b"nazism", Some(0))
                .differences
                .is_empty()
        );
        // A ruling party without saved leaders has none; one without a party
        // block cannot be compared.
        politics(&mut saved)
            .leaders
            .insert(b"fascism".to_vec(), Vec::new());
        assert!(run(&mut frame, &saved, b"", None).differences.is_empty());
        politics(&mut saved).leaders.clear();
        let report = run(&mut frame, &saved, b"", None);
        assert!(report.failed_comparisons["CountryAppearance"].contains("not a saved party"));
        // Politics is written unconditionally, so its absence is refused.
        saved
            .governance
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .politics = None;
        let report = run(&mut frame, &saved, b"", None);
        assert!(report.failed_comparisons["CountryAppearance"].contains("unconditional politics"));
    }
}
