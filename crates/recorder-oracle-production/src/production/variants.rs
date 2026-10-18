use super::*;

pub(super) fn compare_variants(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    // Tier-4 slot3; cadence table is the single source of truth.
    if !report.refresh_is_current(replay_data::rows::TableId::VariantDefinitions, frame.hour) {
        for table in [
            replay_data::rows::TableId::VariantDefinitions,
            replay_data::rows::TableId::VariantModules,
            replay_data::rows::TableId::VariantStats,
            replay_data::rows::TableId::VariantUpgrades,
        ] {
            report.limitation(
                &format!("Variants {}", replay_data::cadence::STAGGER_LIMITATION),
                format!(
                    "{table:?} refreshes on {} (oracle hour {})",
                    replay_data::cadence::refresh_text(table),
                    frame.hour
                ),
            );
        }
        return Ok(());
    }
    let mut complete = report.attempt("VariantDefinitions", |report| {
        compare_variant_definitions(frame, saved, links, tokens, report)
    });
    complete &= report.attempt("VariantModules", |report| {
        compare_variant_modules(frame, saved, tokens, report)
    });
    complete &= report.attempt("VariantUpgrades", |report| {
        compare_variant_upgrades(frame, saved, tokens, report)
    });
    complete &= report.attempt("VariantStats", |report| {
        compare_variant_stats(frame, saved, report)
    });
    report.limitation("the variant writer does not serialize the classifier_flags or mission_mask caches; their captured native operands have no direct save comparison", "VariantDefinitions.classifier_flags, VariantDefinitions.mission_mask");
    report.limitation("native boolean serialization maps every nonzero byte to yes; text flags cannot certify raw byte values beyond truthiness", "VariantDefinitions.append_variant_name, VariantDefinitions.obsolete, VariantDefinitions.icon_active, VariantModules.definition_active");
    report.limitation("the variant writer writes an active icon entry's string without its kind (the loader reclassifies a string holding / or . as a path), and nothing of an inactive entry", "VariantDefinitions.icon_kind, VariantDefinitions.icon of inactive entries");
    report.limitation("null module definitions currently make the complete native module table unavailable; the oracle never treats a failed module walk as an empty saved module list", "VariantModules");

    if complete {
        Ok(())
    } else {
        Err(invalid("one or more variant comparisons failed"))
    }
}

/// `VariantDefinitions.archetype_token` against the content: the save does
/// not write a variant's archetype, but its equipment type's archetype
/// (as the equipment database leaves it: the type
/// itself when it is an archetype, else its `archetype`, else its parent's)
/// is a fact of the recorded content stack. The type is found by name
/// without case (`type-name lookup`), and the archetype matches the
/// recorded literal under the lexer's token fold (A-Z).
/// Content is static within a session, so every recorded row is compared,
/// not only rows read at the frame's hour.
pub fn compare_variant_archetypes(
    frame: &DecodedFrame,
    equipment: &installed_content_core::equipment::Database,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut compared = 0;
    for row in complete::<tables::VariantDefinitions>(frame)? {
        let label = format!("variant {}.archetype_token", row.id);
        let (Some(&kind), Some(&archetype)) = (
            tokens.get(&row.equipment_token),
            tokens.get(&row.archetype_token),
        ) else {
            report.limitation(
                "the equipment type or archetype token is absent from the recorded lexical dictionary; the archetype cannot be matched by name",
                label,
            );
            continue;
        };
        let Ok(kind) = std::str::from_utf8(kind) else {
            report.limitation(
                "the equipment type literal is not UTF-8, so no content name can match it",
                label,
            );
            continue;
        };
        compared += 1;
        let Some(definition) = equipment.named(kind) else {
            report.difference(
                label,
                String::from_utf8_lossy(archetype),
                format!("equipment type {kind} is absent from the content"),
            );
            continue;
        };
        let expected = definition
            .archetype
            .map(|at| equipment.equipment[at].name.as_slice());
        if !expected.is_some_and(|expected| expected.eq_ignore_ascii_case(archetype)) {
            report.difference(
                label,
                String::from_utf8_lossy(archetype),
                expected.map(String::from_utf8_lossy),
            );
        }
    }
    report.compared("VariantDefinitions.archetype_token", compared);
    Ok(())
}

fn compare_variant_definitions(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let saved_variants = saved
        .variants
        .as_ref()
        .ok_or_else(|| invalid("save lacks the equipment variant registry"))?;
    let mut definitions = BTreeMap::new();
    for row in complete::<tables::VariantDefinitions>(frame)? {
        if row.id == 0 || definitions.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate variant definition handle"));
        }
    }
    let mut definition_matches = 0;
    for (&id, expected) in saved_variants {
        let label = format!("variant {id}");
        let Some(actual) = definitions.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        definition_matches += 1;
        compare_token(
            tokens,
            actual.equipment_token,
            expected.definition.as_slice(),
            &format!("{label}.equipment_token"),
            TokenWriter::Token,
            report,
        )?;
        name_field(
            report,
            &format!("{label}.name"),
            &actual.display_name,
            expected.display_name.as_deref(),
        );
        name_field(
            report,
            &format!("{label}.position"),
            &actual.variant_name,
            expected.variant_name.as_deref(),
        );
        equal(
            report,
            format!("{label}.show_position"),
            actual.append_variant_name != 0,
            expected.append_variant_name.unwrap_or(true),
        );
        equal(
            report,
            format!("{label}.obsolete"),
            actual.obsolete != 0,
            expected.obsolete.unwrap_or(false),
        );
        if let Some(&version) = report.recorded(
            &actual.version,
            "VariantDefinitions.version",
            format!("{label}.version"),
        ) {
            // The game omits precisely zero and zero-extends every other byte.
            equal(
                report,
                format!("{label}.version"),
                (version != 0).then_some(version),
                expected.version,
            );
        }
        // The writer omits an unset explicit model; presence compares too.
        name_field(
            report,
            &format!("{label}.override_model"),
            &actual.explicit_entity,
            expected.override_model.as_deref(),
        );
        icon_field(
            report,
            &format!("{label}.override_sprite"),
            actual.icon_active,
            &actual.icon,
            expected.override_sprite.as_deref(),
        );
        for (field, reference, saved_tag) in [
            ("creator", actual.creator_link, &expected.creator),
            ("origin", actual.origin_link, &expected.origin),
        ] {
            let captured = country_tag(reference, links)?;
            if let (Some(captured), Some(saved_tag)) = (captured, saved_tag) {
                equal(
                    report,
                    format!("{label}.{field}"),
                    captured,
                    saved_tag.as_slice(),
                );
            } else if saved_tag.is_none() {
                report.difference(
                    format!("{label}.{field}"),
                    captured,
                    "missing serialized tag",
                );
            } else {
                report.limitation("country-tag text lookup maps a zero country reference through the unrecorded country-tag array entry0; its literal spelling is unmapped here, not an omitted field or an assumed tag", format!("{label}.{field}"));
            }
        }
    }
    for id in definitions.keys() {
        report.difference(format!("variant {id}"), "present", "absent");
    }
    report.compared("VariantDefinitions", definition_matches);
    Ok(())
}

fn compare_variant_modules(
    frame: &DecodedFrame,
    saved: &production::Pack,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let saved_variants = saved
        .variants
        .as_ref()
        .ok_or_else(|| invalid("save lacks the equipment variant registry"))?;
    let mut modules = BTreeMap::<u64, Vec<_>>::new();
    for row in complete::<tables::VariantModules>(frame)? {
        if row.variant == 0 {
            return Err(invalid("null variant module parent"));
        }
        modules.entry(row.variant).or_default().push(row);
    }
    let mut module_matches = 0;
    for (&id, expected) in saved_variants {
        let label = format!("variant {id}");
        let mut actual_modules = modules.remove(&id).unwrap_or_default();
        actual_modules.sort_unstable_by_key(|row| row.source_ordinal);
        ordinals(
            actual_modules.iter().map(|row| row.source_ordinal),
            &format!("{label}.modules"),
        )?;
        equal(
            report,
            format!("{label}.modules.present"),
            !actual_modules.is_empty(),
            expected.modules.is_some(),
        );
        let visible = actual_modules
            .iter()
            .filter(|row| row.definition_active != 0)
            .collect::<Vec<_>>();
        let expected_modules = expected.modules.as_deref().unwrap_or_default();
        for index in 0..visible.len().max(expected_modules.len()) {
            let field = format!("{label}.modules[{index}]");
            match (visible.get(index), expected_modules.get(index)) {
                (Some(actual), Some(expected)) => {
                    module_matches += 1;
                    compare_token(
                        tokens,
                        actual.slot,
                        expected.slot.as_slice(),
                        &format!("{field}.slot"),
                        TokenWriter::Token,
                        report,
                    )?;
                    compare_token(
                        tokens,
                        actual.definition_token,
                        expected.definition.as_slice(),
                        &format!("{field}.definition_token"),
                        TokenWriter::Token,
                        report,
                    )?;
                }
                (Some(_), None) => report.difference(field, "present", "absent"),
                (None, Some(_)) => report.difference(field, "absent", "present"),
                (None, None) => unreachable!(),
            }
        }
        if actual_modules.len() != visible.len() {
            report.limitation("the module writer omits inactive module definitions; active modules are compared in order, while inactive identities and absolute native source ordinals are not serialized", &label);
        }
    }
    for id in modules.keys() {
        report.difference(format!("variant {id}.modules"), "present", "absent variant");
    }
    report.compared("VariantModules", module_matches);
    Ok(())
}

fn compare_variant_upgrades(
    frame: &DecodedFrame,
    saved: &production::Pack,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let saved_variants = saved
        .variants
        .as_ref()
        .ok_or_else(|| invalid("save lacks the equipment variant registry"))?;
    let mut upgrades = BTreeMap::<u64, Vec<_>>::new();
    for row in complete::<tables::VariantUpgrades>(frame)? {
        if row.variant == 0 {
            return Err(invalid("null variant upgrade parent"));
        }
        upgrades.entry(row.variant).or_default().push(row);
    }
    let mut upgrade_matches = 0;
    for (&id, expected) in saved_variants {
        let label = format!("variant {id}");
        let mut actual_upgrades = upgrades.remove(&id).unwrap_or_default();
        actual_upgrades.sort_unstable_by_key(|row| row.source_ordinal);
        ordinals(
            actual_upgrades.iter().map(|row| row.source_ordinal),
            &format!("{label}.upgrades"),
        )?;
        equal(
            report,
            format!("{label}.upgrades.present"),
            !actual_upgrades.is_empty(),
            expected.upgrades.is_some(),
        );
        let expected_upgrades = expected.upgrades.as_deref().unwrap_or_default();
        for index in 0..actual_upgrades.len().max(expected_upgrades.len()) {
            let field = format!("{label}.upgrades[{index}]");
            match (actual_upgrades.get(index), expected_upgrades.get(index)) {
                (Some(actual), Some(expected)) => {
                    upgrade_matches += 1;
                    compare_token(
                        tokens,
                        actual.definition_token as u32,
                        expected.definition.as_slice(),
                        &format!("{field}.definition_token"),
                        TokenWriter::String,
                        report,
                    )?;
                    equal(
                        report,
                        format!("{field}.level"),
                        actual.level,
                        expected.level,
                    );
                }
                (Some(_), None) => report.difference(field, "present", "absent"),
                (None, Some(_)) => report.difference(field, "absent", "present"),
                (None, None) => unreachable!(),
            }
        }
    }
    for id in upgrades.keys() {
        report.difference(
            format!("variant {id}.upgrades"),
            "present",
            "absent variant",
        );
    }
    report.compared("VariantUpgrades", upgrade_matches);
    Ok(())
}

fn compare_variant_stats(
    frame: &DecodedFrame,
    saved: &production::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let saved_variants = saved
        .variants
        .as_ref()
        .ok_or_else(|| invalid("save lacks the equipment variant registry"))?;
    let mut stats = BTreeSet::new();
    let mut census_matches = true;
    for row in complete::<tables::VariantStats>(frame)? {
        if row.variant == 0 || !stats.insert(row.variant) {
            return Err(invalid("null or duplicate variant stats parent"));
        }
        if !saved_variants.contains_key(&row.variant) {
            census_matches = false;
            report.difference(
                format!("variant {}.stats", row.variant),
                "present",
                "absent variant",
            );
        }
    }
    for id in saved_variants.keys() {
        if !stats.contains(id) {
            census_matches = false;
            report.difference(format!("variant {id}.stats"), "absent", "present variant");
        }
    }
    report.compared("VariantStats", 0);
    report.limitation("the variant writer does not serialize the ten native stat caches; saved definitions are not recomputed or substituted for speed, range, air attack, agility, maximum speed, reliability, air defense, thrust, fuel or build cost", format!("VariantStats: {} decoded rows, zero save comparisons", stats.len()));
    if census_matches {
        Ok(())
    } else {
        Err(invalid(
            "variant stats and complete save registry censuses differ",
        ))
    }
}
