use super::*;

pub(super) fn author_live(
    frame: &replay_frame::frame::DecodedFrame,
    handle: u64,
) -> std::io::Result<bool> {
    if handle == 0 {
        return Ok(false);
    }
    complete::<tables::DesignOrganisationLiveness>(frame)?
        .iter()
        .find(|row| row.organisation == handle)
        .map(|row| row.live)
        .ok_or_else(|| invalid(format!("design author {handle} liveness is not recorded")))
}

pub(super) fn compare_variants(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
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
        compare_variant_stats(frame, saved, report)?;
        mios::compare_designs(frame, saved, tokens, report)
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

pub fn compare_variant_archetypes(
    frame: &DecodedFrame,
    equipment: &content_reader_key_names::equipment::Database,
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
            equal(
                report,
                format!("{label}.version"),
                (version != 0).then_some(version),
                expected.version,
            );
        }
        if let Some(design) = report.recorded(
            &actual.design,
            "VariantDefinitions.design",
            format!("{label}.design"),
        ) {
            compare_design_state(
                frame,
                design,
                match actual.version {
                    replay_data::rows::Recorded::Value(v) => Some(v),
                    _ => None,
                },
                &expected.design,
                expected.organisation_marker,
                &label,
                tokens,
                report,
            )?;
        }
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
    let definitions: BTreeMap<u64, u64> = frame
        .complete::<tables::VariantDefinitions>()
        .map(|rows| {
            rows.iter()
                .map(|row| (row.id, row.classifier_flags))
                .collect()
        })
        .unwrap_or_default();
    let mut stats = BTreeSet::new();
    let mut census_matches = true;
    let mut published = [0_usize; replay_data::rows::VariantStatsRow::LANE_NAMES.len()];
    let mut withheld = [0_usize; replay_data::rows::VariantStatsRow::LANE_NAMES.len()];
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
        let is_plane = definitions
            .get(&row.variant)
            .is_some_and(|flags| flags & engine_native::cequipment_type::PLANE_MASK != 0);
        for (index, value) in row.values.into_iter().enumerate() {
            let lane = replay_data::rows::VariantStatsRow::LANE_NAMES[index];
            match value {
                Some(raw) => {
                    published[index] += 1;
                    if is_plane
                        && matches!(
                            index,
                            replay_data::rows::VariantStatsRow::AIR_DEFENCE
                                | replay_data::rows::VariantStatsRow::AIR_AGILITY
                        )
                    {
                        equal(
                            report,
                            format!("variant {}.stats.{lane} plane minimum", row.variant),
                            raw >= 100_000,
                            true,
                        );
                    }
                    if matches!(
                        index,
                        replay_data::rows::VariantStatsRow::MAXIMUM_SPEED
                            | replay_data::rows::VariantStatsRow::RELIABILITY
                    ) {
                        equal(
                            report,
                            format!("variant {}.stats.{lane} non-negative", row.variant),
                            raw >= 0,
                            true,
                        );
                    }
                }
                None => {
                    withheld[index] += 1;
                    report.limitation(
                        &format!("VariantStats.{lane} native witness withheld"),
                        format!("variant {}", row.variant),
                    );
                }
            }
        }
    }
    for id in saved_variants.keys() {
        if !stats.contains(id) {
            census_matches = false;
            report.difference(format!("variant {id}.stats"), "absent", "present variant");
        }
    }
    let total_published: usize = published.iter().sum();
    let total_withheld: usize = withheld.iter().sum();
    report.compared("VariantStats", 0);
    report.limitation(
        "the variant writer does not serialize the native stat sheet or mission-specific caches; capture verifies live stat lanes with native value and timing witnesses and withholds unverified lanes by name",
        format!(
            "VariantStats: {} decoded rows, {} published lanes, {} withheld lanes, zero save comparisons",
            stats.len(),
            total_published,
            total_withheld,
        ),
    );
    if census_matches {
        Ok(())
    } else {
        Err(invalid(
            "variant stats and complete save registry censuses differ",
        ))
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Compare the independent recorded version, saved design and marker with their existing join context."
)]
fn compare_design_state(
    frame: &DecodedFrame,
    actual: &replay_data::rows::VariantDesignState,
    version: Option<u8>,
    expected: &production::DesignState,
    marker: Option<i32>,
    label: &str,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    equal(
        report,
        format!("{label}.parent_id"),
        actual.parent,
        expected.parent,
    );
    equal(
        report,
        format!("{label}.max_version"),
        (version == Some(0)).then_some(actual.max_version),
        expected.max_version,
    );
    equal(
        report,
        format!("{label}.is_frame"),
        Some(actual.is_frame != 0),
        expected.is_frame,
    );
    for (field, value, saved) in [
        (
            "auto_upgraded",
            actual.auto_upgraded,
            expected.auto_upgraded,
        ),
        ("highlight", actual.highlight, expected.highlight),
        (
            "can_upgrade_type",
            actual.can_upgrade_type,
            expected.can_upgrade_type,
        ),
        (
            "can_upgrade_variant",
            actual.can_upgrade_variant,
            expected.can_upgrade_variant,
        ),
        (
            "can_upgrade_modules",
            actual.can_upgrade_modules,
            expected.can_upgrade_modules,
        ),
    ] {
        equal(
            report,
            format!("{label}.{field}"),
            (value != 0).then_some(true),
            saved,
        );
    }
    equal(
        report,
        format!("{label}.manpower"),
        (actual.manpower > 0).then_some(actual.manpower),
        expected.manpower,
    );
    equal(
        report,
        format!("{label}.role_icon_index"),
        (actual.role_icon_index > 0).then_some(actual.role_icon_index),
        expected.role_icon_index,
    );
    equal(
        report,
        format!("{label}.division_names_group"),
        actual
            .division_names_group
            .as_deref()
            .map(|text| text.split(|byte| *byte == 0).next().unwrap_or_default()),
        expected.division_names_group.as_deref(),
    );
    equal(
        report,
        format!("{label}.named_equipment_bonuses"),
        (!actual.named_bonuses.is_empty()).then_some(actual.named_bonuses.as_slice()),
        expected.named_bonuses.as_deref(),
    );
    equal(
        report,
        format!("{label}.number_of_design_team_traits"),
        (actual.design_team_traits != 0).then_some(actual.design_team_traits),
        marker,
    );
    let mut ideas = Vec::new();
    let snapshots = if actual.bonus_sources.is_empty() {
        &[][..]
    } else {
        complete::<tables::VariantBonusSnapshots>(frame)?
    };
    for id in &actual.bonus_sources {
        let source = &snapshots[snapshots
            .binary_search_by_key(id, |row| row.id)
            .map_err(|_| invalid("design bonus snapshot reference missing"))?]
        .source;
        if let Some(token) = source.idea_token
            && !ideas.contains(&token)
        {
            ideas.push(token);
        }
    }
    equal(
        report,
        format!("{label}.ideas.present"),
        !actual.bonus_sources.is_empty(),
        expected.ideas.is_some(),
    );
    let saved_ideas = expected.ideas.as_deref().unwrap_or_default();
    equal(
        report,
        format!("{label}.ideas.count"),
        ideas.len(),
        saved_ideas.len(),
    );
    for (index, (actual, expected)) in ideas.into_iter().zip(saved_ideas).enumerate() {
        compare_token(
            tokens,
            actual,
            expected,
            &format!("{label}.ideas[{index}]"),
            TokenWriter::Token,
            report,
        )?;
    }
    equal(
        report,
        format!("{label}.design_team"),
        author_live(frame, actual.design_team)?.then_some(actual.design_team),
        expected.design_team,
    );
    equal(
        report,
        format!("{label}.design_team_bonus.present"),
        !actual.design_team_bonus.is_empty(),
        expected.design_team_bonus.is_some(),
    );
    let saved = expected.design_team_bonus.as_deref().unwrap_or_default();
    equal(
        report,
        format!("{label}.design_team_bonus.count"),
        actual.design_team_bonus.len(),
        saved.len(),
    );
    for (index, (actual, (name, amount))) in actual.design_team_bonus.iter().zip(saved).enumerate()
    {
        compare_token(
            tokens,
            actual.index,
            name,
            &format!("{label}.design_team_bonus[{index}].name"),
            TokenWriter::Token,
            report,
        )?;
        equal(
            report,
            format!("{label}.design_team_bonus[{index}].value"),
            actual.value,
            amount.raw,
        );
    }
    Ok(())
}
