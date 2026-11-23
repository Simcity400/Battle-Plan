use super::*;
use replay_data::rows::ResearchTechnologyRow;

pub(super) fn compare_research(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    names: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let tech = replay_data::rows::TableId::ResearchTechnologies;
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
            rows.iter().all(|row| {
                replay_data::sliced::read_hour(reads, tech, row.country) == Some(frame.hour)
            })
        }),
    };
    let definitions = complete::<tables::TechnologyDefinitions>(frame)?;
    let mut dictionary = BTreeMap::new();
    let mut name_handles = BTreeMap::new();
    let spellings = reverse_spellings(names);
    for row in definitions {
        let token = literal(report, "TechnologyDefinitions.id", &row.id);
        let subject = format!(
            "technology {:?} handle {} lexical binding",
            String::from_utf8_lossy(&row.id),
            row.technology_handle
        );
        let token =
            token_spelling(names, &spellings, token, "technology").inspect_err(|error| {
                report.difference(&subject, &row.id, error.to_string());
            })?;
        let Some(writer) = names.get(&row.technology_handle).copied() else {
            let error = invalid(format!(
                "technology {:?} handle {} is absent from the recorded lexical dictionary",
                String::from_utf8_lossy(&row.id),
                row.technology_handle,
            ));
            report.difference(&subject, &row.id, error.to_string());
            return Err(error);
        };
        let writer = literal(report, "TechnologyDefinitions.handle", writer);
        let identity = content_reader_parser::tokens::Token::c_str(&row.id);
        if writer != token
            || (identity.is_rejected() && row.technology_handle != 0)
            || (!identity.is_rejected() && identity != content_reader_parser::tokens::Token::c_str(writer))
        {
            report.difference(&subject, &row.id, writer);
            return Err(invalid(format!(
                "technology {:?} has contradictory lexical binding to handle {} ({:?})",
                String::from_utf8_lossy(&row.id),
                row.technology_handle,
                String::from_utf8_lossy(writer),
            )));
        }
        if row.database_index <= 0
            || name_handles
                .insert(writer, row.technology_handle)
                .is_some_and(|handle| handle != row.technology_handle)
        {
            return Err(invalid(
                "technology occurrence has inconsistent token identity",
            ));
        }
        if dictionary
            .insert(
                row.database_index as u32,
                (writer, row.max_level, row.technology_handle),
            )
            .is_some()
        {
            return Err(invalid("duplicate technology definition occurrence"));
        }
    }
    report.compared("TechnologyDefinitions", 0);
    report.limitation("technology definitions join through the recorded lexical writer spelling; runtime handles, database ordinals, year, max level and authored cost are not directly certified by a text save", "TechnologyDefinitions");
    let slots_current =
        report.refresh_is_current(replay_data::rows::TableId::ResearchSlots, frame.hour);
    let mut complete = true;
    complete &= report.attempt("governance research grant inventory", |report| {
        compare_grants(frame, pack, links, names, report)
    });
    if slots_current || report.read_hours.is_some() {
        complete &= report.attempt("governance ResearchSlots", |report| {
            compare_research_slots(frame, pack, links, &dictionary, report)
        });
    } else {
        report.limitation(
            &format!(
                "Research family {}",
                replay_data::cadence::STAGGER_LIMITATION
            ),
            format!(
                "ResearchSlots refreshes on {} (oracle hour {})",
                replay_data::cadence::refresh_text(replay_data::rows::TableId::ResearchSlots),
                frame.hour
            ),
        );
    }
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
            &format!(
                "Research family {}",
                replay_data::cadence::STAGGER_LIMITATION
            ),
            format!(
                "ResearchTechnologies refreshes on {} (oracle hour {})",
                replay_data::cadence::refresh_text(
                    replay_data::rows::TableId::ResearchTechnologies
                ),
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

fn compare_grants(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    names: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    use replay_data::rows::ResearchModifierRow as R;
    let rows = complete::<tables::ResearchModifiers>(frame)?;
    let current_owners = if report.read_hours.is_some() && !report.full_refresh() {
        let reads = complete::<tables::SliceReads>(frame)?;
        let reads = reads.iter().filter(|read| {
            read.table == replay_data::rows::TableId::ResearchModifiers.index() as u32
        });
        let mut current = BTreeSet::new();
        let mut tags = BTreeSet::new();
        let mut oldest = None;
        for read in reads {
            if read.sample_hour == frame.hour {
                current.insert(read.owner);
                tags.insert(country(pack, links, read.owner)?.0.to_vec());
            } else {
                oldest =
                    Some(oldest.map_or(read.sample_hour, |old: i32| old.min(read.sample_hour)));
            }
        }
        if oldest.is_some() || current.is_empty() {
            report.carried_owners(replay_data::rows::TableId::ResearchModifiers, oldest);
        }
        Some((current, tags))
    } else {
        None
    };
    let mut captured = BTreeMap::<Vec<u8>, (Option<i32>, Vec<saved::ResearchGrant>)>::new();
    let mut owner_rows = BTreeMap::<u32, Vec<&R>>::new();
    for row in rows
        .iter()
        .filter(|row| row.index == engine_research::research_modifiers::grants::INVENTORY_INDEX)
    {
        owner_rows.entry(row.country).or_default().push(row);
    }
    let spelling = |id: i64| -> io::Result<Vec<u8>> {
        let id =
            u32::try_from(id).map_err(|_| invalid("research restriction token width invalid"))?;
        names
            .get(&id)
            .map(|text| text.to_vec())
            .ok_or_else(|| invalid(format!("research restriction token {id} missing")))
    };
    for (owner, mut rows) in owner_rows {
        if current_owners
            .as_ref()
            .is_some_and(|(owners, _)| !owners.contains(&owner))
        {
            continue;
        }
        let (tag, _) = country(pack, links, owner)?;
        rows.sort_by_key(|row| row.ordinal);
        let mut next = None;
        let mut grants: Vec<saved::ResearchGrant> = Vec::new();
        for row in rows {
            match row.kind {
                R::GRANT_COUNTER => {
                    if next
                        .replace(
                            i32::try_from(row.raw)
                                .map_err(|_| invalid("grant counter width invalid"))?,
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate research grant counter"));
                    }
                }
                R::SPEED_GRANT | R::COST_GRANT => {
                    grants.push(saved::ResearchGrant {
                        cost: row.kind == R::COST_GRANT,
                        id: row.source_id as i32,
                        name: literal(report, "Research grant name", &row.name).to_vec(),
                        speed_raw: if row.kind == R::SPEED_GRANT {
                            row.raw
                        } else {
                            0
                        },
                        ahead_raw: row.auxiliary,
                        cost_raw: if row.kind == R::COST_GRANT {
                            row.raw
                        } else {
                            0
                        },
                        ..Default::default()
                    });
                }
                R::GRANT_USES | R::GRANT_TECHNOLOGY | R::GRANT_CATEGORY => {
                    let grant = grants
                        .last_mut()
                        .filter(|grant| {
                            grant.id == row.source_id as i32 && u32::from(grant.cost) == row.count
                        })
                        .ok_or_else(|| invalid("grant child lacks its ordered parent"))?;
                    match row.kind {
                        R::GRANT_USES => {
                            grant.uses = i32::try_from(row.raw)
                                .map_err(|_| invalid("grant uses width invalid"))?;
                            grant.claims = i32::try_from(row.auxiliary)
                                .map_err(|_| invalid("grant claims width invalid"))?;
                        }
                        R::GRANT_TECHNOLOGY => grant.technologies.push(spelling(row.raw)?),
                        _ => grant.categories.push(spelling(row.raw)?),
                    }
                }
                _ => return Err(invalid("unknown grant inventory row kind")),
            }
        }
        if captured.insert(tag.to_vec(), (next, grants)).is_some() {
            return Err(invalid("multiple grant inventories resolve to one country"));
        }
    }
    let mut count = 0;
    for (tag, saved_country) in report.canonical_countries(&pack.countries) {
        if current_owners
            .as_ref()
            .is_some_and(|(_, tags)| !tags.contains(tag.as_slice()))
        {
            continue;
        }
        let Some(research) = &saved_country.research else {
            continue;
        };
        let actual = captured.remove(tag.as_slice());
        let expected = (research.next_bonus_id, research.grants.clone());
        equal(
            report,
            label(tag, "research grant inventory"),
            actual,
            Some(expected),
        );
        count += research.grants.len() + 1;
    }
    for tag in captured.keys() {
        report.difference(label(tag, "research grant inventory"), "present", "absent");
    }
    report.compared("Research grant inventory", count);
    Ok(())
}

pub(super) fn compare_research_slots(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    dictionary: &BTreeMap<u32, (&[u8], i32, u32)>,
    report: &mut Report,
) -> io::Result<()> {
    let slots = complete::<tables::ResearchSlots>(frame)?;
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
        let token = if row.technology_handle == engine_research::cresearch_slot::IDLE_HANDLE {
            b"empty".as_slice()
        } else {
            let definition = dictionary
                .get(
                    &(*row
                        .database_index
                        .require("ResearchSlots.database_index")
                        .map_err(invalid)? as u32),
                )
                .ok_or_else(|| invalid("research slot has no definition"))?;
            if definition.2 != row.technology_handle {
                return Err(invalid(
                    "research slot handle disagrees with definition occurrence",
                ));
            }
            definition.0
        };
        captured_slots.entry(tag).or_default().push((
            token,
            row.research_speed_raw,
            row.banked_days_raw,
            row.applied_days_raw,
        ));
    }
    let mut slot_count = 0;
    for (tag, country) in report.canonical_countries(&pack.countries) {
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
                        let token = if row.technology_handle
                            == engine_research::cresearch_slot::IDLE_HANDLE
                        {
                            b"empty".as_slice()
                        } else {
                            dictionary[&(*row
                                .database_index
                                .require("ResearchSlots.database_index")
                                .unwrap() as u32)]
                                .0
                        };
                        (token, row)
                    });
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

pub(super) fn compare_research_technologies(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    dictionary: &BTreeMap<u32, (&[u8], i32, u32)>,
    report: &mut Report,
) -> io::Result<()> {
    let technologies = complete::<tables::ResearchTechnologies>(frame)?;
    let active = (|| {
        let mut active = BTreeSet::new();
        for row in complete::<tables::ResearchSlots>(frame)? {
            if row.technology_present > 1 {
                return Err(invalid("invalid research technology presence"));
            }
            if row.technology_present != 0 {
                let index = *row
                    .database_index
                    .require("ResearchSlots.database_index")
                    .map_err(invalid)?;
                if index <= 0 {
                    return Err(invalid("active research slot lacks positive occurrence"));
                }
                if dictionary
                    .get(&(index as u32))
                    .is_none_or(|definition| definition.2 != row.technology_handle)
                {
                    return Err(invalid(
                        "active research slot handle disagrees with definition occurrence",
                    ));
                }
                let (tag, _) = country(pack, links, row.country)?;
                active.insert((tag, index));
            }
        }
        Ok(active)
    })();
    let mut ambiguous = BTreeSet::new();
    let mut captured: BTreeMap<&[u8], Vec<(&[u8], &ResearchTechnologyRow)>> = BTreeMap::new();
    let mut keys = BTreeSet::new();
    let mut omitted_count = 0;
    for row in technologies {
        let (tag, _) = country(pack, links, row.country)?;
        let (token, max_level, handle) = dictionary
            .get(&(row.database_index as u32))
            .copied()
            .ok_or_else(|| invalid("research technology occurrence has no definition"))?;
        if row.database_index <= 0 || row.technology_handle != handle {
            return Err(invalid(
                "research technology handle disagrees with definition occurrence",
            ));
        }
        if !keys.insert((tag, row.database_index)) {
            return Err(invalid("duplicate research technology occurrence"));
        }
        if row.level > 0
            || row.progress_raw > 0
            || (row.level < max_level
                && active
                    .as_ref()
                    .is_ok_and(|active| active.contains(&(tag, row.database_index))))
            || row.applied_bonus_count != 0
        {
            captured.entry(tag).or_default().push((token, row));
        } else {
            if active.is_err() && row.level < max_level {
                ambiguous.insert(tag);
            }
            omitted_count += 1;
        }
    }
    let mut technology_count = 0;
    for (tag, country) in report.canonical_countries(&pack.countries) {
        let mut actual = captured.remove(tag.as_slice()).unwrap_or_default();
        actual.sort_unstable_by_key(|(_, row)| row.database_index);
        let expected = country
            .research
            .as_ref()
            .map(|research| research.technologies.as_slice())
            .unwrap_or_default();
        if ambiguous.contains(tag.as_slice()) {
            report.limitation("research occurrence order, multiplicity and serialized fields cannot be certified without exact active-slot filtering", label(tag, "technology"));
            let mut saved = expected.iter();
            for (token, row) in &actual {
                if !saved.by_ref().any(|(saved_token, saved)| {
                    *token == saved_token.as_slice()
                        && row.level == saved.level.unwrap_or(0)
                        && row.progress_raw == saved.progress.as_ref().map_or(0, |v| v.raw)
                        && row.ahead_reduction_years_raw
                            == saved.ahead_reduction.as_ref().map_or(0, |v| v.raw)
                }) {
                    report.difference(
                        format!(
                            "{}.serializer_required_occurrence[{}]",
                            label(tag, "technology"),
                            row.database_index
                        ),
                        row,
                        expected,
                    );
                    break;
                }
            }
            continue;
        }
        equal(
            report,
            label(tag, "technology.serialized_occurrences"),
            actual.len(),
            expected.len(),
        );
        for (ordinal, ((token, row), (saved_token, saved))) in
            actual.iter().zip(expected).enumerate()
        {
            let subject = format!(
                "{}.technology.{}[{ordinal}]",
                String::from_utf8_lossy(tag),
                String::from_utf8_lossy(token)
            );
            if *token != saved_token.as_slice() {
                report.difference(format!("{subject}.token_order"), token, saved_token);
            }
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
        }
    }
    if !captured.is_empty() {
        return Err(invalid("research occurrences have no saved country"));
    }
    report.compared("ResearchTechnologies", technology_count);
    report.limitation("the game filters each indexed instance; omitted nonpositive state is not serialized, and a text save does not certify omitted database indexes", format!("ResearchTechnologies: {omitted_count} omitted records"));
    report.limitation("technology classifier and independent researched-vector membership are not serialized directly; saved level is not substituted for either", "ResearchTechnologies");
    active.map(|_| ()).map_err(|error| invalid(format!("active-slot omission checks unavailable after serialized technology comparisons: {error}")))
}
