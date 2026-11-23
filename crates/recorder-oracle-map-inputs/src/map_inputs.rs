use super::*;

mod reserved_launch;

fn eq<T: PartialEq + std::fmt::Debug>(report: &mut Report, label: &str, actual: T, expected: T) {
    report.scalar_checks += 1;
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

pub fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::CountryAppearance, |r| countries(frame, save, r));
    report.attempt_table(TableId::ReservedCountryNaming, |r| {
        reserved_naming(frame, r)
    });
    report.attempt("ReservedCountryNaming saved original tag", |r| {
        reserved_original_tag(frame, save, r)
    });
    report.attempt("ReservedCountryNaming launch sources", |r| {
        reserved_launch::compare(frame, save, r)
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
    report.limitation("the game state save writer skips physical slot zero but writes positive physical aliases of the reserved object; its original_tag emission is compared separately through the admitted full-save ledger, while stored-length built and other naming caches remain runtime witnesses", "ReservedCountryNaming: singleton decoded, built and other naming caches remain unverified");
    report.limitation("the game state save writer starts its country census at slot one, so the save omits slot zero; a positive physical reserved alias supplies saved original_tag presence/bytes only through the admitted full-save ledger", "ReservedCountryNaming.original_tag: saved comparison requires an admitted physical alias");
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
    report.limitation("the country save writer writes original_tag bytes only for a signed positive reference; even a saved positive physical reserved alias cannot certify that reference's exact magnitude or alias identity", "ReservedCountryNaming.original_tag_reference: unsaved raw identity");
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
    report.limitation("the original-tag bucket cache is runtime state rebuilt by the game and changed by native setters; serialized countries cannot certify retained cache order, repeats or alias reference identity", "ReservedCountryNaming.original_tag_buckets: unsaved native cache");
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
    report.limitation("the country save census starts at physical slot one and serializes tags rather than exact raw country IDs; it cannot certify the full physical array's reserved-zero, alias and duplicate occurrences", "ReservedCountryNaming.country_array: raw physical order and identity remain unverified");
    if let Some(None) = report.recorded(
        &rows[0].owned_state_count,
        "ReservedCountryNaming.owned_state_count",
        "reserved country signed owned-state count",
    ) {
        report.limitation("reserved country owned-state count unreadable at capture; other naming operands remain available", "ReservedCountryNaming.owned_state_count: guarded read refusal");
    }
    report.limitation("the country save writer does not directly serialize the exact signed getter operand, even when a positive physical alias emits the reserved country; that owned-state count still requires a matched native witness", "ReservedCountryNaming.owned_state_count: unsaved reserved getter operand");
    if let Some(None) = report.recorded(
        &rows[0].faction,
        "ReservedCountryNaming.faction",
        "reserved country faction identity",
    ) {
        report.limitation(
            "reserved country faction identity unreadable at capture; naming and other operands remain independent",
            "ReservedCountryNaming.faction: guarded read refusal",
        );
    }
    report.limitation(
        "saved global faction membership cannot independently certify the reserved object's stored diplomacy faction operand; an authenticated null or full native faction handle still needs a matched VM witness",
        "ReservedCountryNaming.faction: unsaved reserved faction operand",
    );
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
    report.limitation("the dynamic-tag slot boundary is a runtime signed slot boundary read by the game; the game state save writer and the country save writer do not serialize it, so fresh-save original-tag text cannot certify this predicate", "ReservedCountryNaming.dynamic_tag_boundary: unsaved native predicate; matched-hour memory needed");
    Ok(())
}

fn reserved_original_tag(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let [row] = frame.complete::<tables::ReservedCountryNaming>()? else {
        return Err(invalid(
            "reserved original tag requires one complete naming row",
        ));
    };
    let Some(emissions) = save.country_emissions.as_deref() else {
        if report
            .recorded(
                &row.country_array,
                "ReservedCountryNaming.country_array",
                "reserved original tag physical emission join",
            )
            .is_some()
        {
            return Err(invalid(
                "reserved original tag requires the admitted full country emission ledger",
            ));
        }
        report.limitation(
            "historical absent physical country census cannot establish a saved reserved original_tag counterpart",
            "ReservedCountryNaming.original_tag: physical emission join unavailable",
        );
        return Ok(());
    };
    let mut count = 0;
    for emission in emissions
        .iter()
        .filter(|entry| entry.identity.actual == save::ActualCountry::Reserved)
    {
        let identity = &emission.identity;
        if identity.own_id != 0 || identity.physical_slot == 0 || identity.tag != row.tag {
            return Err(invalid(
                "reserved original tag emission has an inconsistent admitted identity",
            ));
        }
        let saved = save.countries.get(identity.tag.as_slice()).ok_or_else(|| {
            invalid("admitted reserved original tag emission lacks its saved country")
        })?;
        let label = format!(
            "reserved country physical slot {}.original_tag",
            identity.physical_slot
        );
        count += 1;
        let Some(reference) = report.recorded(
            &row.original_tag_reference,
            "ReservedCountryNaming.original_tag_reference",
            format!("{label}.reference"),
        ) else {
            continue;
        };
        let Some(reference) = reference else {
            report.difference(
                &label,
                "unavailable reference",
                "required reached saved emission gate",
            );
            continue;
        };
        report.limitation(
            "saved original_tag presence/text cannot certify the exact signed reference magnitude or alias identity",
            format!("{label}.reference={reference}"),
        );
        if *reference <= 0 {
            eq(report, &label, None::<&[u8]>, saved.original_tag.as_deref());
            continue;
        }
        match report.recorded(
            &row.original_tag,
            "ReservedCountryNaming.original_tag",
            &label,
        ) {
            Some(Some(bytes)) => {
                let comparable = content_reader_parser::parser::c_string(bytes);
                if comparable.len() < bytes.len() {
                    report.limitation("native text serialization ends at the first NUL; captured trailing original-tag bytes are not save-comparable", &label);
                }
                eq(
                    report,
                    &label,
                    Some(comparable),
                    saved.original_tag.as_deref(),
                );
            }
            bytes => {
                eq(
                    report,
                    &format!("{label} presence"),
                    true,
                    saved.original_tag.is_some(),
                );
                if matches!(bytes, Some(None)) {
                    report.difference(
                        &label,
                        "unavailable bytes",
                        "required positive-reference saved text",
                    );
                }
            }
        }
    }
    if count == 0 {
        report.limitation(
            "the admitted physical country census has no positive physical actual0 occurrence, so no saved reserved original_tag counterpart exists",
            "ReservedCountryNaming.original_tag: matched native witness required",
        );
    }
    report.compared("reserved original tag saved emissions", count);
    Ok(())
}

#[cfg(test)]
#[path = "map_inputs/reserved_original_tag_tests.rs"]
mod reserved_original_tag_tests;

#[cfg(test)]
mod original_tag_tests {
    use super::*;
    use replay_data::{rows::Recorded, rows::ReservedCountryNamingRow};

    #[test]
    fn ship_critical_presence_is_compared_independently_and_never_waives_unknown() {
        for (value, expected, differences) in [
            (Some(0), false, 0),
            (Some(1), true, 0),
            (Some(0), true, 1),
            (None, false, 1),
            (Some(2), true, 1),
        ] {
            let mut report = Report::default();
            ship_criticals(value, expected, "ShipModels 7", &mut report);
            assert_eq!(report.differences.len(), differences);
        }
    }

    #[test]
    fn reserved_original_tag_oracle_reports_unsaved_and_independent_refusal() {
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
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
                    .any(|reason| reason.contains("the game state save writer")
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
        use replay_data::rows::CountryAppearanceRow;
        for (reference, actual, saved, differences) in [
            (i32::MIN, Some(Vec::new()), None, 0),
            (0, Some(Vec::new()), Some(Vec::new()), 1),
            (7, Some(Vec::new()), Some(Vec::new()), 0),
            (7, Some(Vec::new()), None, 2),
            (7, Some(b"OWN".to_vec()), Some(b"OWN".to_vec()), 0),
            (7, Some(b"OTHER".to_vec()), Some(b"OTHER".to_vec()), 0),
            (-1, Some(Vec::new()), Some(b"OWN".to_vec()), 1),
            (7, Some(b"OWN".to_vec()), None, 2),
            (7, Some(b"OTHER".to_vec()), Some(b"OWN".to_vec()), 1),
            (7, Some(vec![b'A', 0, b'B']), Some(b"A".to_vec()), 0),
            (7, Some(b"\0suffix".to_vec()), Some(Vec::new()), 0),
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
            assert_eq!(report.scalar_checks, if reference > 0 { 2 } else { 1 });
        }
        for value in [Recorded::Value(None), Recorded::NotRecorded] {
            let historical = matches!(value, Recorded::NotRecorded);
            let row = CountryAppearanceRow {
                original_tag: value,
                original_tag_reference: Recorded::Value(Some(7)),
                ..Default::default()
            };
            let mut report = Report::default();
            original_tag(&row, &save::Country::default(), "OWN", &mut report);
            assert_eq!(
                report
                    .limitations
                    .contains_key("CountryAppearance.original_tag not recorded"),
                historical
            );
            assert_eq!(report.scalar_checks, 1);
            assert_eq!(report.differences.len(), if historical { 1 } else { 2 });
            assert!(report.outcome().is_err());
        }
    }

    #[test]
    fn raw_reference_and_native_domain_oracles_preserve_gaps_and_refusals() {
        for value in [Recorded::Value(None), Recorded::NotRecorded] {
            let historical = matches!(value, Recorded::NotRecorded);
            let row = replay_data::rows::CountryAppearanceRow {
                original_tag: Recorded::Value(Some(b"OWN".to_vec())),
                original_tag_reference: value,
                ..Default::default()
            };
            let mut report = Report::default();
            original_tag(&row, &save::Country::default(), "OWN", &mut report);
            assert_eq!(
                report
                    .limitations
                    .contains_key("CountryAppearance.original_tag_reference not recorded"),
                historical
            );
            assert_eq!(report.scalar_checks, 0);
            assert_eq!(report.differences.is_empty(), historical);
            assert_eq!(report.outcome().is_ok(), historical);
        }
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
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
                    .any(|reason| reason.contains("original-tag bucket cache is runtime state"))
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
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
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
                    .any(|reason| reason.contains("exact signed getter operand"))
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
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
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

#[cfg(test)]
#[path = "map_inputs/zero_faction_tests.rs"]
mod zero_faction_tests;

pub fn countries(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    report.record_country_emissions(save);
    let identities: BTreeMap<_, _> = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let mut remaining: BTreeSet<_> = report
        .canonical_countries(&save.countries)
        .into_iter()
        .map(|(tag, _)| tag.as_slice())
        .collect();
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
    report.limitation("the country writer serializes neither colour object (the country loader reads tokens 0x56 color and 0x31B8 color_ui, but no writer emits 0x31B8), so the current UI RGBA (the UI colour) has no save counterpart; its four raw f32 bit patterns need a matched-hour screenshot of a counter plate in country-colour mode", "CountryAppearance raw UI RGBA");
    report.limitation("name_tag and overlord are census only; the viewer's resolved names need a matched-hour screenshot", "CountryAppearance name operands");
    report.limitation("graphical_culture is not serialized by the country writer; it comes from the country's content file and needs a matched-hour screenshot of a model or portrait that depends on it", "CountryAppearance.graphical_culture");
    report.limitation("graphical_culture_2d is not serialized by the country writer; it comes from the country's content file and needs a matched-hour screenshot of an equipment icon that depends on it", "CountryAppearance.graphical_culture_2d");
    report.limitation("the old-name caches are runtime text: the country save writer does not save them and runtime updates write them, so a save cannot certify them; they need a live check of old-name text", "CountryAppearance old names");
    report.limitation("the diplomacy save writer does not save the ordered subject roster; active_relations do not certify this array's order, repeats or raw reference words", "CountryAppearance.subjects");
    Ok(())
}

fn original_tag(
    row: &replay_data::rows::CountryAppearanceRow,
    saved: &save::Country,
    label: &str,
    report: &mut Report,
) {
    let Some(reference) = report.recorded(
        &row.original_tag_reference,
        "CountryAppearance.original_tag_reference",
        format!("{label}.original_tag_reference"),
    ) else {
        return;
    };
    let Some(reference) = reference else {
        report.difference(
            format!("{label}.original_tag_reference"),
            "capture refused",
            "saved emission gate",
        );
        return;
    };
    report.limitation("the country save writer emits original tag bytes only for a signed-positive reference; it does not serialize the raw reference magnitude or alias identity, including exact nonpositive values", format!("{label}.original_tag_reference={reference}: unsaved raw identity"));
    let actual = if *reference > 0 {
        eq(
            report,
            &format!("{label}.original_tag presence"),
            true,
            saved.original_tag.is_some(),
        );
        let bytes = report.recorded(
            &row.original_tag,
            "CountryAppearance.original_tag",
            format!("{label}.original_tag"),
        );
        let Some(bytes) = bytes else { return };
        let Some(bytes) = bytes else {
            report.difference(
                format!("{label}.original_tag"),
                "capture refused",
                &saved.original_tag,
            );
            return;
        };
        let comparable = content_reader_parser::parser::c_string(bytes);
        if comparable.len() < bytes.len() {
            report.limitation("native text serialization ends at the first NUL; captured trailing original-tag bytes are not save-comparable", format!("{label}.original_tag"));
        }
        Some(comparable)
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

fn ruling_leader(
    save: &Save,
    tag: &[u8],
    label: &str,
    row: &replay_data::rows::CountryAppearanceRow,
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
            .map(|id| (id, engine_native::ccharacter::REFERENCE_TYPE)),
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

fn selected_variant(pool: &save::production::Pool) -> Option<u64> {
    let mut selected = None;
    for entry in &pool.equipment {
        if selected.is_none_or(|previous: &save::production::Equipment| {
            entry.amount.raw > previous.amount.raw
        }) {
            selected = Some(entry);
        }
    }
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
        let index_label = format!("{label}.subunit_index");
        if let Some(index) =
            report.recorded(&row.subunit_index, "ShipModels.subunit_index", &index_label)
        {
            if index.is_none() {
                report.limitation(
                    "ship subunit index unreadable at capture; model prefix remains available",
                    &index_label,
                );
            } else {
                report.limitation("the ship save writer saves the subunit token, not the signed ship subunit index; token equality cannot distinguish rejected-name aliases. The index/name pairing requires an independent matched native and write witness, still pending", &index_label);
            }
        }
        let Some(saved) = save.ships.get(&row.ship) else {
            report.difference(label, "present", "absent");
            continue;
        };
        remaining.remove(&row.ship);
        ship_criticals(
            row.has_criticals
                .require("ShipModels.has_criticals")
                .ok()
                .copied()
                .flatten(),
            saved.has_criticals,
            &label,
            report,
        );
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
            report.limitation("the pool writer omits zero-quantity entries unless allow_zero_entries is enabled; an empty or all-negative saved pool cannot identify the native maximum when omitted zeros may exist, so this ship's selected variant is not compared", format!("{label}.variant"));
        }
        ship_tooltip_suffixes(row, save, report);
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
    report.limitation("the ship save writer does not serialize the retained module-predicate cache or its engagement byte; rebuilding the predicate from saved designs would replace a valid stale value. The Air capacity subtype requires the matched native grid comparison", "ShipModels.air_capacity_kind");
    report.limitation("selected variant identity is independently derived from the saved equipment pool; saved registry definition keys and variant names are compared by VariantDefinitions on its refresh hours, not inferred from hull sprite bytes", "ShipModels.variant names");
    report.limitation("compared: the ship census against the save and the Ships table, each ship's roster ordinal and, where the saved pool identifies it, its selected variant. Ship hull/fallback sprite keys, model number, the full formation-selection flags, positive-equipment flag union, definition-valid byte, carrier air-base reference, strength maximum, anti-air base and derived organisation maximum have no counterpart in the ship save writer; they cannot be independently checked against this text save", "ShipModels hull_sprite, fallback_sprite, model_number, formation_flags, counter operands");
    Ok(())
}

fn ship_criticals(observed: Option<u32>, expected: bool, label: &str, report: &mut Report) {
    match observed {
        Some(value) if value <= 1 => eq(
            report,
            &format!("{label}.has_criticals"),
            value != 0,
            expected,
        ),
        _ => report.difference(
            format!("{label}.has_criticals"),
            "capture refused",
            "critical-damage wrapper presence",
        ),
    }
}

fn ship_tooltip_suffixes(row: &replay_data::rows::ShipModelRow, save: &Save, r: &mut Report) {
    let label = format!("ShipModels {}", row.ship);
    let hull_label = format!("{label}.hull_variant");
    let Some(hull) = r.recorded(&row.hull_variant, "ShipModels.hull_variant", &hull_label) else {
        return;
    };
    if row.variant == 0 {
        if hull.is_some() {
            r.difference(
                &hull_label,
                "present",
                "unreached for null selected variant",
            );
        }
        return;
    }
    let Some(hull) = hull else {
        r.difference(
            &hull_label,
            "capture refused",
            "native selected-variant packet",
        );
        return;
    };
    for (name, missing) in [
        ("definition_index", hull.definition_index.is_none()),
        ("module_count", hull.module_count.is_none()),
        ("upgrade_level_sum", hull.upgrade_level_sum.is_none()),
        ("module_priority_sum", hull.module_priority_sum.is_none()),
        ("version", hull.version.is_none()),
        ("organisation_marker", hull.organisation_marker.is_none()),
    ] {
        if missing {
            r.difference(
                format!("{hull_label}.{name}"),
                "capture refused",
                "native scalar",
            );
        }
    }
    let Some(variants) = &save.production.variants else {
        r.difference(
            &hull_label,
            "present",
            "save lacks equipment variant registry",
        );
        return;
    };
    let Some(expected) = variants.get(&row.variant) else {
        r.difference(
            &hull_label,
            row.variant,
            "selected variant absent from saved registry",
        );
        return;
    };
    if let Some(version) = hull.version {
        eq(
            r,
            &format!("{hull_label}.version"),
            (version != 0).then_some(version),
            expected.version,
        );
    }
    if let Some(marker) = hull.organisation_marker {
        eq(
            r,
            &format!("{hull_label}.organisation_marker"),
            (marker != 0).then_some(marker),
            expected.organisation_marker,
        );
    }
    if let Some(sum) = hull.upgrade_level_sum {
        let saved_sum = expected
            .upgrades
            .as_deref()
            .unwrap_or_default()
            .iter()
            .fold(0_i32, |sum, upgrade| {
                sum.wrapping_add(i32::from(upgrade.level))
            });
        eq(
            r,
            &format!("{hull_label}.upgrade_level_sum"),
            sum,
            saved_sum,
        );
    }
    r.limitation("the general equipment definition ordinal is not serialized; its authenticated recorded-content index/name join and runtime writers require independent evidence, not a guessed token ordinal", format!("{hull_label}.definition_index"));
    r.limitation("module serialization can omit inactive assignments; neither current count nor priority sum is universally reconstructible from the saved modules. Existing serialized-module comparisons remain exact on their refresh hours; these current caches need an independent matched native/cache-writer witness, still pending", format!("{hull_label}.module_count/module_priority_sum"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::ArmyIconInputRow, rows::CountryAppearanceRow, rows::Recorded, rows::ShipModelRow,
    };

    #[test]
    fn ship_tooltip_suffixes_use_current_saved_variant_and_keep_runtime_caches_explicit() {
        use replay_data::rows::HullVariantInputs;
        let (_, mut saved, _, _) = super::super::fixtures::fixture();
        let doc = content_reader_parser::parser::parse(
            br#"equipments={type={id={id=3 type=70}
            version=255 number_of_design_team_traits=-2147483648
            upgrades={upgrades={{engine 255} {engine 0}}}
        }}"#,
        )
        .unwrap();
        saved.production = save::production::parse(&doc.items).unwrap();
        let hull = HullVariantInputs {
            definition_index: Some(-1),
            module_count: Some(9),
            upgrade_level_sum: Some(255),
            module_priority_sum: Some(i32::MIN),
            version: Some(255),
            organisation_marker: Some(i32::MIN),
        };
        let mut row = ShipModelRow {
            ship: 9,
            variant: (3 << 32) | 70,
            hull_variant: Recorded::Value(Some(hull)),
            ..Default::default()
        };
        let mut report = Report::default();
        ship_tooltip_suffixes(&row, &saved, &mut report);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("omit inactive assignments"))
        );
        row.hull_variant = Recorded::Value(Some(HullVariantInputs {
            version: Some(0),
            organisation_marker: Some(0),
            upgrade_level_sum: Some(256),
            ..hull
        }));
        let mut report = Report::default();
        ship_tooltip_suffixes(&row, &saved, &mut report);
        assert_eq!(report.differences.len(), 3);
        let doc = content_reader_parser::parser::parse(b"equipments={type={id={id=3 type=70}}}").unwrap();
        saved.production = save::production::parse(&doc.items).unwrap();
        row.hull_variant = Recorded::Value(Some(HullVariantInputs {
            version: Some(0),
            organisation_marker: Some(0),
            upgrade_level_sum: Some(0),
            ..hull
        }));
        let mut report = Report::default();
        ship_tooltip_suffixes(&row, &saved, &mut report);
        assert!(report.differences.is_empty());
        row.hull_variant = Recorded::Value(Some(HullVariantInputs {
            version: None,
            ..hull
        }));
        let mut report = Report::default();
        ship_tooltip_suffixes(&row, &saved, &mut report);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("hull_variant.version"))
        );
        row.hull_variant = Recorded::NotRecorded;
        let mut old = Report::default();
        ship_tooltip_suffixes(&row, &saved, &mut old);
        assert!(old.differences.is_empty());
        assert!(
            old.limitations
                .contains_key("ShipModels.hull_variant not recorded")
        );
        row.variant = 0;
        row.hull_variant = Recorded::Value(None);
        let mut null = Report::default();
        ship_tooltip_suffixes(&row, &saved, &mut null);
        assert!(null.differences.is_empty());
    }

    fn pool(source: &[u8]) -> save::production::Pool {
        let doc = content_reader_parser::parser::parse(source).unwrap();
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
        let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
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
                ship_name: None,
                manpower: None,
                max_manpower: None,
                has_criticals: false,
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
            TableView::Complete(TableRows::Divisions(vec![replay_data::rows::DivisionRow {
                id: division,
                ..Default::default()
            }]));
        *frame.tables[TableId::Ships.index()] =
            TableView::Complete(TableRows::Ships(vec![replay_data::rows::ShipRow {
                id: ship,
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 101,
                cosmetic_tag: b"GER_cosmetic".to_vec(),
                red: u32::MAX,
                original_tag_reference: Recorded::Value(Some(0)),
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
                has_criticals: Some(0).into(),
                ship,
                variant: (3_u64 << 32) | 70,
                counter: Some(Default::default()),
                hull_variant: Recorded::NotRecorded,
                ..Default::default()
            }]));
        let mut report = Report::default();
        report.record_tables(&frame);
        compare(&frame, &saved, &mut report);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        assert_eq!(
            report.compared_tables.len(),
            4,
            "{:?}",
            report.compared_tables
        );
        assert!(
            !report
                .compared_tables
                .contains_key("ReservedCountryNaming.movement")
        );
        for field in [
            "ReservedCountryNaming.launch_sources",
            "ReservedCountryNaming.country_array",
            "ShipModels.hull_variant",
        ] {
            assert_eq!(
                report.limitations[&format!("{field} not recorded")].len(),
                1
            );
        }
        for index in [
            Recorded::NotRecorded,
            Recorded::Value(None),
            Recorded::Value(Some(0)),
            Recorded::Value(Some(i32::MIN)),
        ] {
            let TableView::Complete(TableRows::ShipModels(rows)) =
                &mut *frame.tables[TableId::ShipModels.index()]
            else {
                panic!("complete ship model fixture")
            };
            rows[0].subunit_index = index;
            let mut report = Report::default();
            ships(&frame, &saved, &mut report).unwrap();
            assert!(report.differences.is_empty());
            assert_eq!(
                report
                    .limitations
                    .keys()
                    .any(|r| r.contains("subunit_index not recorded")),
                matches!(index, Recorded::NotRecorded),
            );
            assert_eq!(
                report.limitations.contains_key(
                    "ship subunit index unreadable at capture; model prefix remains available"
                ),
                matches!(index, Recorded::Value(None)),
            );
            assert_eq!(
                report
                    .limitations
                    .keys()
                    .any(|r| r.contains("token equality cannot distinguish rejected-name aliases")),
                matches!(index, Recorded::Value(Some(_))),
            );
        }
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
        let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
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
                    original_tag_reference: Recorded::Value(Some(0)),
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
        politics(&mut saved)
            .leaders
            .insert(b"fascism".to_vec(), Vec::new());
        assert!(run(&mut frame, &saved, b"", None).differences.is_empty());
        politics(&mut saved).leaders.clear();
        let report = run(&mut frame, &saved, b"", None);
        assert!(report.failed_comparisons["CountryAppearance"].contains("not a saved party"));
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
