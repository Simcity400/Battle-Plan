use super::*;

pub(super) fn compare_research(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    // Technology rows keep the hour their country's slice read them
    // (SliceReads). The saved side has no per-owner scope, so the comparison
    // needs every owner current, including owners with no rows, which rows
    // cannot show. With read hours supplied (the oracle command) that takes a
    // proven full refresh; otherwise the technologies are carried, with the
    // oldest recorded read. Without them (callers that do not scan the branch)
    // the earlier row check stays. ResearchSlots is a daily table and is
    // compared whenever its slot was read (its group's own carried check). An
    // unreadable table is not stale: its comparison below reports the failure.
    let tech = replay::rows::TableId::ResearchTechnologies;
    let oldest_read = complete::<tables::SliceReads>(frame)
        .ok()
        .and_then(|reads| {
            reads
                .iter()
                .filter(|row| row.table == tech.index() as u32 && row.sample_hour != frame.hour)
                .map(|row| row.sample_hour)
                .min()
        });
    let technologies_current = match complete::<tables::ResearchTechnologies>(frame) {
        Err(_) => true,
        Ok(_) if report.read_hours.is_some() => report.full_refresh(),
        Ok(rows) => complete::<tables::SliceReads>(frame).is_ok_and(|reads| {
            rows.iter()
                .all(|row| replay::sliced::read_hour(reads, tech, row.country) == Some(frame.hour))
        }),
    };
    let definitions = complete::<tables::TechnologyDefinitions>(frame)?;
    let mut dictionary = BTreeMap::new();
    let mut tokens = BTreeSet::new();
    for row in definitions {
        let token = literal(report, "TechnologyDefinitions.id", &row.id);
        if dictionary
            .insert(row.technology_handle, (token, row.max_level))
            .is_some()
            || !tokens.insert(token)
        {
            return Err(invalid(
                "duplicate technology definition handle or serialized token",
            ));
        }
    }
    report.compared("TechnologyDefinitions", 0);
    report.limitation("technology definitions provide exact token joins; runtime handles, database ordinals, year, max level and authored cost are not directly certified by a text save", "TechnologyDefinitions");
    // Without read hours (callers that do not scan the branch) a stale
    // ResearchSlots is a limitation, as before; with them, its comparison
    // group reports it carried.
    let slots_current = report.refresh_is_current(replay::rows::TableId::ResearchSlots, frame.hour);
    let mut complete = true;
    if slots_current || report.read_hours.is_some() {
        complete &= report.attempt("governance ResearchSlots", |report| {
            compare_research_slots(frame, pack, links, &dictionary, report)
        });
    } else {
        report.limitation(
            &format!("Research family {}", replay::cadence::STAGGER_LIMITATION),
            format!(
                "ResearchSlots refreshes on {} (oracle hour {})",
                replay::cadence::refresh_text(replay::rows::TableId::ResearchSlots),
                frame.hour
            ),
        );
    }
    // Active-slot coverage of the technologies depends on ResearchSlots.
    if technologies_current && slots_current {
        complete &= report.attempt("governance ResearchTechnologies", |report| {
            compare_research_technologies(frame, pack, links, &dictionary, report)
        });
    } else if report.read_hours.is_some() {
        report.carried.insert(
            "governance ResearchTechnologies (owners not read at this hour)".into(),
            oldest_read,
        );
    } else {
        report.limitation(
            &format!("Research family {}", replay::cadence::STAGGER_LIMITATION),
            format!(
                "ResearchTechnologies refreshes on {} (oracle hour {})",
                replay::cadence::refresh_text(replay::rows::TableId::ResearchTechnologies),
                frame.hour
            ),
        );
    }
    if complete {
        Ok(())
    } else {
        Err(invalid(
            "research comparisons incomplete; independent results retained",
        ))
    }
}

pub(super) fn compare_research_slots(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    dictionary: &BTreeMap<u32, (&[u8], i32)>,
    report: &mut Report,
) -> io::Result<()> {
    let slots = complete::<tables::ResearchSlots>(frame)?;
    // Serialized technology, research speed, banked days and applied days.
    type ResearchSlotValues<'a> = (&'a [u8], i64, i64, i64);
    let mut captured_slots: BTreeMap<&[u8], Vec<ResearchSlotValues<'_>>> = BTreeMap::new();
    let mut slot_indices: BTreeMap<&[u8], BTreeSet<u32>> = BTreeMap::new();
    for row in slots {
        let (tag, _) = country(pack, links, row.country)?;
        if row.technology_present > 1 {
            return Err(invalid("invalid research technology presence"));
        }
        if !slot_indices.entry(tag).or_default().insert(row.index) {
            return Err(invalid("duplicate research slot index"));
        }
        let token = if row.technology_handle == engine::cresearch_slot::IDLE_HANDLE {
            b"empty".as_slice()
        } else {
            dictionary
                .get(&row.technology_handle)
                .ok_or_else(|| invalid("research slot has no definition"))?
                .0
        };
        captured_slots.entry(tag).or_default().push((
            token,
            row.research_speed_raw,
            row.banked_days_raw,
            row.applied_days_raw,
        ));
    }
    let mut slot_count = 0;
    for (tag, country) in &pack.countries {
        if let Some(research) = &country.research {
            let mut actual = captured_slots.remove(tag.as_slice()).unwrap_or_default();
            if let Some(indices) = slot_indices.remove(tag.as_slice()) {
                check_ordinals(indices)?;
            }
            let mut expected = research
                .slots
                .iter()
                .map(|s| {
                    (
                        s.technology.as_slice(),
                        s.points_factor.as_ref().map_or(100_000, |v| v.raw),
                        s.points.as_ref().map_or(0, |v| v.raw),
                        s.used_saved_points.as_ref().map_or(0, |v| v.raw),
                    )
                })
                .collect::<Vec<_>>();
            // ECFF80 does not serialize the native slot index. Compare complete
            // entries with multiplicity, rather than inventing an ordinal join.
            actual.sort_unstable();
            expected.sort_unstable();
            if actual != expected {
                let native = slots
                    .iter()
                    .filter(|row| {
                        required_country_tag(row.country, links)
                            .is_ok_and(|owner| owner == tag.as_slice())
                    })
                    .map(|row| {
                        let token = if row.technology_handle == engine::cresearch_slot::IDLE_HANDLE
                        {
                            b"empty".as_slice()
                        } else {
                            dictionary[&row.technology_handle].0
                        };
                        (token, row)
                    });
                // Preserve the comparison projection above, while reporting
                // every captured field and the exact saved optional operands.
                report.difference(
                    label(tag, "research_slots.values_with_multiplicity"),
                    DebugRows(native),
                    &research.slots,
                );
            }
            slot_count += actual.len().min(expected.len());
        }
    }
    if !captured_slots.is_empty() {
        return Err(invalid("research slots have no saved technology container"));
    }
    report.compared("ResearchSlots", slot_count);
    report.limitation("research slots serialize technology token and three fixed5 operands, but not native index or technology-pointer presence; complete entry multiplicity is compared", "ResearchSlots");
    Ok(())
}

fn compare_research_technologies(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    dictionary: &BTreeMap<u32, (&[u8], i32)>,
    report: &mut Report,
) -> io::Result<()> {
    let technologies = complete::<tables::ResearchTechnologies>(frame)?;
    // Active-slot omission coverage depends on ResearchSlots, but serialized
    // technology fields and positive-value omission checks do not.
    let active = complete::<tables::ResearchSlots>(frame).and_then(|slots| {
        let mut active = BTreeSet::new();
        for row in slots {
            if row.technology_present > 1 {
                return Err(invalid("invalid research technology presence"));
            }
            let (tag, _) = country(pack, links, row.country)?;
            if row.technology_present != 0 {
                active.insert((tag, row.technology_handle));
            }
        }
        Ok(active)
    });
    let mut captured_technologies = BTreeSet::new();
    let mut technology_count = 0;
    let mut omitted_count = 0;
    for row in technologies {
        let (tag, country) = country(pack, links, row.country)?;
        let (token, max_level) = dictionary
            .get(&row.technology_handle)
            .copied()
            .ok_or_else(|| invalid("research technology has no definition"))?;
        if !captured_technologies.insert((tag, token)) {
            return Err(invalid("duplicate research technology key"));
        }
        let subject = format!(
            "{}.technology.{}",
            String::from_utf8_lossy(tag),
            String::from_utf8_lossy(token)
        );
        let Some(research) = &country.research else {
            report.difference(subject, "captured", "missing saved technology container");
            continue;
        };
        if let Some(saved) = research.technologies.get(token) {
            equal(
                report,
                format!("{subject}.level"),
                row.level,
                saved.level.unwrap_or(0),
            );
            equal(
                report,
                format!("{subject}.research_points"),
                row.progress_raw,
                saved.progress.as_ref().map_or(0, |v| v.raw),
            );
            equal(
                report,
                format!("{subject}.ahead_reduction"),
                row.ahead_reduction_years_raw,
                saved.ahead_reduction.as_ref().map_or(0, |v| v.raw),
            );
            technology_count += 1;
        } else {
            // ED02A0 must emit every positive level/progress and every occupied
            // slot below max level. Its other predicate uses uncaptured bonuses.
            if row.level > 0
                || row.progress_raw > 0
                || (row.level < max_level
                    && active
                        .as_ref()
                        .is_ok_and(|active| active.contains(&(tag, row.technology_handle))))
            {
                report.difference(
                    subject,
                    "serializer-required technology",
                    "missing saved technology",
                );
            }
            omitted_count += 1;
        }
    }
    for (tag, country) in &pack.countries {
        if let Some(research) = &country.research {
            for token in research.technologies.keys() {
                if !captured_technologies.contains(&(tag.as_slice(), token.as_slice())) {
                    report.difference(
                        label(tag, "technology"),
                        "missing captured technology",
                        token,
                    );
                }
            }
        }
    }
    report.compared("ResearchTechnologies", technology_count);
    report.limitation("ED02A0 filters the technology census; omitted records' nonpositive level/progress and ahead-reduction bits are not directly serialized", format!("ResearchTechnologies: {omitted_count} omitted records"));
    report.limitation("technology classifier and independent researched-vector membership are not serialized directly; saved level is not substituted for either", "ResearchTechnologies");
    active.map(|_| ()).map_err(|error| invalid(format!("active-slot omission checks unavailable after serialized technology comparisons: {error}")))
}
