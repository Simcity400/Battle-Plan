use super::*;

fn occupation_tag<'a>(
    id: u32,
    tags: &BTreeMap<u32, &'a [u8]>,
    subject: &str,
    report: &mut Report,
) -> io::Result<&'a [u8]> {
    let tag = tags
        .get(&id)
        .copied()
        .ok_or_else(|| invalid(format!("{subject}: raw country tag {id} was not acquired")))?;
    Ok(super::super::serialized_text(report, subject, tag))
}

fn occupation_key_failure(
    report: &mut Report,
    subject: &str,
    error: io::Error,
    failure: &mut Option<io::Error>,
) {
    report.limitation(
        "occupation saved-key witness is unavailable; the dependent join is not compared",
        format!("{subject}: {error}"),
    );
    failure.get_or_insert(error);
}

fn retained_group_country(
    controller: u32,
    occupied: u32,
    table: TableId,
    settings: Option<&[replay_data::rows::OccupationSettingsRow]>,
    report: &Report,
) -> io::Result<u32> {
    if let Some(hours) = &report.read_hours {
        let settings_hour = hours
            .get(&TableId::OccupationSettings)
            .ok_or_else(|| invalid("occupation settings supplying hour was not acquired"))?;
        let row_hour = hours
            .get(&table)
            .ok_or_else(|| invalid(format!("{table:?} supplying hour was not acquired")))?;
        if settings_hour != row_hour {
            return Err(invalid(format!(
                "{table:?} supplying hour {row_hour} differs from occupation settings hour {settings_hour}"
            )));
        }
    }
    let settings =
        settings.ok_or_else(|| invalid("occupation settings inventory is unavailable"))?;
    let mut owners = settings.iter().filter(|row| row.country == controller);
    let row = owners.next().ok_or_else(|| {
        invalid(format!(
            "occupation settings controller {controller} is absent"
        ))
    })?;
    if owners.next().is_some() {
        return Err(invalid("ambiguous occupation settings controller witness"));
    }
    let groups = match &row.group_laws {
        replay_data::rows::Recorded::Value(Some(groups)) => groups,
        replay_data::rows::Recorded::Value(None) => {
            return Err(invalid("occupation group-law inventory capture failed"));
        }
        replay_data::rows::Recorded::NotRecorded => {
            return Err(invalid("occupation group-law inventory was not acquired"));
        }
    };
    let mut groups = groups.iter().filter(|group| group.occupied == occupied);
    let group = groups.next().ok_or_else(|| {
        invalid(format!(
            "occupation lookup-country witness {occupied} is absent"
        ))
    })?;
    if groups.next().is_some() {
        return Err(invalid("ambiguous occupation lookup-country witness"));
    }
    group
        .scope_country
        .ok_or_else(|| invalid("retained occupation country capture failed"))
}

pub fn occupation_settings(
    rows: &[replay_data::rows::OccupationSettingsRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tags: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    let mut failure = None;
    for row in rows {
        let country = required_country_tag(row.country, links)?;
        if !seen.insert(country.to_vec()) {
            return Err(invalid("duplicate resolved occupation settings country"));
        }
        let subject = format!("occupation settings.{}", String::from_utf8_lossy(country));
        let Some(saved) = saved.occupation_settings.get(country) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        if let Some(occupied) = report.recorded(
            &row.occupied,
            "OccupationSettings.occupied",
            format!("{subject}.occupied"),
        ) {
            match occupied {
                Some(occupied) => {
                    let field = format!("{subject}.occupied");
                    let occupied = occupied
                        .iter()
                        .map(|&raw| occupation_tag(raw, tags, &field, report))
                        .collect::<io::Result<Vec<_>>>();
                    match occupied {
                        Ok(occupied) => value(
                            report,
                            &field,
                            occupied,
                            saved.occupied.iter().map(Vec::as_slice).collect::<Vec<_>>(),
                        ),
                        Err(error) => occupation_key_failure(report, &field, error, &mut failure),
                    }
                }
                None => occupation_key_failure(
                    report,
                    &format!("{subject}.occupied"),
                    invalid("occupied-country roster capture failed"),
                    &mut failure,
                ),
            }
        }
        value(
            report,
            &format!("{subject}.garrison_template"),
            row.garrison_template,
            saved.garrison_template,
        );
        value(
            report,
            &format!("{subject}.default_law_present"),
            row.default_law_present,
            u8::from(saved.default_law.is_some()),
        );
        match &row.group_laws {
            replay_data::rows::Recorded::Value(Some(groups)) => {
                let mut recorded = BTreeMap::new();
                let mut complete = true;
                for group in groups {
                    let field = format!("{subject}.group_laws.scope_country");
                    let occupied = group
                        .scope_country
                        .ok_or_else(|| invalid("retained occupation country capture failed"))
                        .and_then(|id| occupation_tag(id, tags, &field, report));
                    let occupied = match occupied {
                        Ok(occupied) => occupied,
                        Err(error) => {
                            complete = false;
                            occupation_key_failure(report, &field, error, &mut failure);
                            continue;
                        }
                    };
                    let field =
                        format!("{subject}.group_laws.{}", String::from_utf8_lossy(occupied));
                    let law = group
                        .law
                        .as_deref()
                        .map(|law| super::super::serialized_text(report, &field, law));
                    if recorded.insert(occupied, law).is_some() {
                        return Err(invalid("duplicate resolved occupied-country law"));
                    }
                }
                if complete {
                    value(
                        report,
                        &format!("{subject}.group_laws"),
                        &recorded,
                        &saved
                            .group_laws
                            .iter()
                            .map(|(tag, law)| (tag.as_slice(), law.as_deref()))
                            .collect::<BTreeMap<_, _>>(),
                    );
                } else {
                    for (occupied, law) in recorded {
                        let field =
                            format!("{subject}.group_laws.{}", String::from_utf8_lossy(occupied));
                        match saved.group_laws.get(occupied) {
                            Some(saved) => value(report, &field, law, saved.as_deref()),
                            None => report.difference(field, "present", "missing saved group"),
                        }
                    }
                }
            }
            replay_data::rows::Recorded::Value(None) => {
                report.difference(
                    format!("{subject}.group_laws"),
                    "capture failed",
                    "saved inventory present",
                );
                occupation_key_failure(
                    report,
                    &format!("{subject}.group_laws"),
                    invalid("occupation group-law inventory capture failed"),
                    &mut failure,
                );
            }
            replay_data::rows::Recorded::NotRecorded => occupation_key_failure(
                report,
                &format!("{subject}.group_laws"),
                invalid("occupation group-law inventory was not acquired"),
                &mut failure,
            ),
        }
        report.limitation(
            "the occupation group's embedded lookup link is not serialized; saved group keys certify the retained country and law, not the lookup link",
            format!("{subject}.lookup_country"),
        );
        report.limitation(
            "the database fallback pointer and eligibility trigger results have no text-save counterpart",
            format!("{subject}.fallback_law"),
        );
        if let Some(law) = &saved.default_law {
            let field = format!("{subject}.default_law");
            let name = super::super::serialized_text(report, &field, &row.law);
            value(report, &field, name, law.as_slice());
        }
    }
    missing(
        report,
        "occupation settings",
        &seen,
        saved.occupation_settings.keys().cloned(),
    );
    report.compared("OccupationSettings", matched);
    failure.map_or(Ok(()), Err)
}

pub fn occupation_laws(
    rows: &[replay_data::rows::StateOccupationLawRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    settings: Option<&[replay_data::rows::OccupationSettingsRow]>,
    tags: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut unresolved = BTreeSet::new();
    let mut matched = 0;
    let mut failure = None;
    for row in rows {
        let controller = required_country_tag(row.controller, links)?;
        let field = format!(
            "occupation law.{}.{}/{}.occupied",
            String::from_utf8_lossy(controller),
            row.occupied,
            row.state
        );
        let occupied = retained_group_country(
            row.controller,
            row.occupied,
            TableId::StateOccupationLaws,
            settings,
            report,
        )
        .and_then(|id| occupation_tag(id, tags, &field, report));
        let occupied = match occupied {
            Ok(occupied) => occupied,
            Err(error) => {
                unresolved.insert((controller.to_vec(), row.state));
                occupation_key_failure(report, &field, error, &mut failure);
                continue;
            }
        };
        let key = (controller.to_vec(), occupied.to_vec(), row.state);
        if !seen.insert(key.clone()) {
            return Err(invalid(
                "duplicate resolved controller/occupied/state law key",
            ));
        }
        let subject = format!(
            "occupation law.{}.{}/{:}",
            String::from_utf8_lossy(controller),
            String::from_utf8_lossy(occupied),
            row.state
        );
        let Some(saved) = saved.state_occupation_laws.get(&key) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        let name = super::super::serialized_text(report, &subject, &row.name);
        value(report, &subject, name, saved.law.as_slice());
    }
    missing(
        report,
        "occupation law",
        &seen,
        saved
            .state_occupation_laws
            .keys()
            .filter(|(controller, _, state)| !unresolved.contains(&(controller.clone(), *state)))
            .cloned(),
    );
    report.compared("StateOccupationLaws", matched);
    failure.map_or(Ok(()), Err)
}

pub fn garrisons(
    rows: &[replay_data::rows::StateGarrisonRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    settings: Option<&[replay_data::rows::OccupationSettingsRow]>,
    tags: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    let mut failure = None;
    for row in rows {
        let controller = required_country_tag(row.controller, links)?;
        let key = (controller.to_vec(), row.state);
        if !seen.insert(key.clone()) {
            return Err(invalid("duplicate resolved controller/state garrison key"));
        }
        let subject = format!(
            "garrison.{}.{:}",
            String::from_utf8_lossy(controller),
            row.state
        );
        let Some(saved) = saved.state_garrisons.get(&key) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        let field = format!("{subject}.occupied");
        let occupied = retained_group_country(
            row.controller,
            row.occupied,
            TableId::StateGarrisons,
            settings,
            report,
        )
        .and_then(|id| occupation_tag(id, tags, &field, report));
        match occupied {
            Ok(occupied) => value(report, &field, occupied, saved.occupied.as_slice()),
            Err(error) => occupation_key_failure(report, &field, error, &mut failure),
        }
        value(
            report,
            &format!("{subject}.required_manpower"),
            row.required_manpower,
            saved.required_manpower,
        );
        report.scalar(
            &subject,
            "strength_ratio",
            Some(row.strength_ratio),
            Some(&saved.strength_ratio),
        );
        report.scalar(
            &subject,
            "garrison_required",
            Some(row.garrison_required),
            Some(&saved.garrison_required),
        );
        value(
            report,
            &format!("{subject}.serialized_flag"),
            row.serialized_flag != 0,
            saved.serialized_flag,
        );
        if row.serialized_flag > 1 {
            report.limitation("garrison serializer represents its flag as a boolean; nonzero byte magnitude is not serialized", subject);
        }
    }
    missing(
        report,
        "garrison",
        &seen,
        saved.state_garrisons.keys().cloned(),
    );
    report.compared("StateGarrisons", matched);
    failure.map_or(Ok(()), Err)
}

pub fn assigned_manpower(
    rows: &[replay_data::rows::StateAssignedManpowerRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    settings: Option<&[replay_data::rows::OccupationSettingsRow]>,
    tags: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut groups = BTreeMap::<saved::GarrisonKey, Vec<(u32, u32, i32)>>::new();
    let mut failure = None;
    for row in rows {
        let controller = required_country_tag(row.controller, links)?;
        let key = (controller.to_vec(), row.state);
        let subject = format!(
            "assigned manpower.{}.{:}",
            String::from_utf8_lossy(controller),
            row.state
        );
        let Some(saved) = saved.state_garrisons.get(&key) else {
            report.difference(subject, "present", "missing saved garrison");
            continue;
        };
        let field = format!("{subject}.occupied");
        let occupied = retained_group_country(
            row.controller,
            row.occupied,
            TableId::StateAssignedManpower,
            settings,
            report,
        )
        .and_then(|id| occupation_tag(id, tags, &field, report));
        match occupied {
            Ok(occupied) => value(report, &field, occupied, saved.occupied.as_slice()),
            Err(error) => occupation_key_failure(report, &field, error, &mut failure),
        }
        groups
            .entry(key)
            .or_default()
            .push((row.ordinal, row.source_country, row.amount));
    }
    let mut matched = 0;
    for (key, garrison) in &saved.state_garrisons {
        let rows = groups.remove(key).unwrap_or_default();
        matched += rows
            .iter()
            .filter(|row| row.2 > 0)
            .count()
            .min(garrison.manpower.len());
        manpower(
            report,
            &format!(
                "assigned manpower.{}.{:}",
                String::from_utf8_lossy(&key.0),
                key.1
            ),
            rows,
            &garrison.manpower,
            links,
        )?;
    }
    report.compared("StateAssignedManpower", matched);
    failure.map_or(Ok(()), Err)
}

pub fn garrison_equipment(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    names: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut matched = 0;
    for row in complete::<tables::StateGarrisons>(frame)? {
        let controller = required_country_tag(row.controller, links)?;
        let subject = format!(
            "garrison.{}.{}.equipment",
            String::from_utf8_lossy(controller),
            row.state
        );
        let Some(saved) = saved.state_garrisons.get(&(controller.to_vec(), row.state)) else {
            report.difference(subject, "captured garrison", "no saved garrison");
            continue;
        };
        let inputs = match &row.equipment {
            replay_data::rows::Recorded::NotRecorded => {
                report.limitation(
                    "older garrison rows lack the equipment strength operands",
                    subject,
                );
                continue;
            }
            replay_data::rows::Recorded::Value(inputs) => inputs,
        };
        if let Some(reason) = &inputs.failure {
            report.difference(
                subject,
                String::from_utf8_lossy(reason),
                "saved equipment objects",
            );
            continue;
        }
        let requirements = inputs
            .rows
            .iter()
            .filter(|entry| entry.required_raw != 0)
            .map(|entry| {
                Ok((
                    token(names, entry.equipment_token)
                        .map_err(invalid)?
                        .to_vec(),
                    entry.required_raw,
                ))
            })
            .collect::<io::Result<Vec<_>>>()?;
        match &saved.equipment_need {
            Some(pool) => value(
                report,
                &format!("{subject}.requirements"),
                requirements,
                pool.iter()
                    .map(|(name, amount)| (name.clone(), amount.raw))
                    .collect(),
            ),
            None => report.difference(
                format!("{subject}.requirements"),
                "captured inventory",
                "missing serialized equipment_need",
            ),
        }
        report.limitation("the garrison save writer saves the variant pool, not the allocation cache used by archetype quantity lookup; resistance damage floors each separately, so the save cannot certify allocations", format!("{subject}.allocations"));
        matched += 1;
        report.limitation("the game omits exactly zero requirements; their membership and absolute ordinal are not save-certified", format!("{subject}.zero_requirements"));
        report.limitation("the two pool serializers omit live equipment weights, archetype active bytes and pointer-identity lookup operands; these require the source-byte read witness and matched native display", format!("{subject}.live_operands"));
    }
    report.compared("StateGarrisons equipment", matched);
    Ok(())
}

pub fn reserve_manpower(
    rows: &[replay_data::rows::CountryOccupationReserveManpowerRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut groups = BTreeMap::<Vec<u8>, Vec<(u32, u32, i32)>>::new();
    for row in rows {
        let country = required_country_tag(row.country, links)?;
        if !saved.country_reserve_manpower.contains_key(country) {
            report.difference(
                format!("occupation reserve.{}", String::from_utf8_lossy(country)),
                "present",
                "missing saved occupation status",
            );
            continue;
        }
        groups.entry(country.to_vec()).or_default().push((
            row.ordinal,
            row.source_country,
            row.amount,
        ));
    }
    let mut matched = 0;
    for (country, saved) in report.canonical_countries(&saved.country_reserve_manpower) {
        let rows = groups.remove(country).unwrap_or_default();
        matched += rows.iter().filter(|row| row.2 > 0).count().min(saved.len());
        manpower(
            report,
            &format!("occupation reserve.{}", String::from_utf8_lossy(country)),
            rows,
            saved,
            links,
        )?;
    }
    report.compared("CountryOccupationReserveManpower", matched);
    Ok(())
}

pub fn resistance(
    rows: &[replay_data::rows::StateResistanceRow],
    saved: &saved::Pack,
    tags: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    let mut failure = None;
    for row in rows {
        if !seen.insert(row.state) {
            return Err(invalid("duplicate captured state resistance"));
        }
        let subject = format!("resistance.{}", row.state);
        let Some(saved) = saved.state_resistance.get(&row.state) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        report.scalar(&subject, "level", Some(row.level), Some(&saved.level));
        report.scalar(&subject, "speed", Some(row.speed), Some(&saved.speed));
        report.scalar(
            &subject,
            "compliance",
            Some(row.compliance),
            Some(&saved.compliance),
        );
        report.scalar(
            &subject,
            "compliance_speed",
            Some(row.compliance_speed),
            Some(&saved.compliance_speed),
        );
        value(
            report,
            &format!("{subject}.active"),
            row.active != 0,
            saved.active,
        );
        if (row.country as i32) > 0 {
            let field = format!("{subject}.country");
            match occupation_tag(row.country, tags, &field, report) {
                Ok(tag) => value(report, &field, Some(tag), saved.country.as_deref()),
                Err(error) => occupation_key_failure(report, &field, error, &mut failure),
            }
        } else if saved.country.is_some() {
            report.difference(
                format!("{subject}.country"),
                "omitted: signed-nonpositive",
                &saved.country,
            );
        } else {
            report.limitation("resistance serializer omits signed-nonpositive country links; the exact omitted raw link is not compared", format!("{subject}.country"));
        }
        if row.active > 1 {
            report.limitation("resistance serializer records active as a boolean; nonzero byte magnitude is not serialized", subject);
        }
    }
    missing(
        report,
        "resistance",
        &seen,
        saved.state_resistance.keys().copied(),
    );
    report.compared("StateResistance", matched);
    failure.map_or(Ok(()), Err)
}

pub fn resistance_map(
    rows: &[replay_data::rows::StateResistanceRow],
    saved: &saved::Pack,
    states: &BTreeMap<u32, installed_content_save::State>,
    state_rows: &[replay_data::rows::StateRow],
    controllers: &BTreeMap<u32, &[u8]>,
    tags: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    use replay_data::rows::Recorded;
    let tag = |id| {
        tags.get(&id)
            .copied()
            .ok_or_else(|| invalid(format!("occupation map country tag {id} was not acquired")))
    };
    let mut recorded_states = BTreeMap::new();
    for state in state_rows {
        if recorded_states.insert(state.id, state).is_some() {
            return Err(invalid("duplicate captured state for occupation map"));
        }
    }
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        if !seen.insert(row.state) {
            return Err(invalid("duplicate captured state occupation map"));
        }
        let subject = format!("resistance.{}.map", row.state);
        let Recorded::Value(map) = &row.map else {
            return Err(invalid(format!(
                "{subject}: occupation map witness was not acquired"
            )));
        };
        if (row.country as i32) <= 0 {
            value(report, &format!("{subject}.present"), map.is_some(), false);
            matched += 1;
            continue;
        }
        let map = map
            .as_ref()
            .ok_or_else(|| invalid(format!("{subject}: occupation map capture failed")))?;
        let state = states
            .get(&row.state)
            .ok_or_else(|| invalid(format!("{subject}: saved state is absent")))?;
        let captured_state = recorded_states
            .get(&row.state)
            .ok_or_else(|| invalid(format!("{subject}: captured state controller is absent")))?;
        let saved_state = saved
            .state_resistance
            .get(&row.state)
            .ok_or_else(|| invalid(format!("{subject}: saved resistance is absent")))?;
        let occupied = saved_state
            .country
            .as_deref()
            .ok_or_else(|| invalid(format!("{subject}: saved occupied country is absent")))?;
        let captured_country = super::super::serialized_text(
            report,
            &format!("{subject}.occupied"),
            tag(row.country)?,
        );
        value(
            report,
            &format!("{subject}.occupied"),
            captured_country,
            occupied,
        );
        let controller = if let Some(owner) = state.owner.as_deref() {
            state.controller.as_deref().unwrap_or(owner)
        } else {
            if state.controller.is_some() {
                return Err(invalid(
                    "saved occupation map state has controller without owner",
                ));
            }
            report.limitation(
                "the state serializer omits controller with a nonpositive owner; the occupation map core join uses the captured state controller",
                format!("{subject}.controller"),
            );
            let controller = controllers
                .get(&captured_state.controller)
                .copied()
                .ok_or_else(|| {
                    invalid(format!(
                        "{subject}: captured controller link was not resolved"
                    ))
                })?;
            super::super::serialized_text(report, &format!("{subject}.controller"), controller)
        };
        let settings = saved.occupation_settings.get(controller).ok_or_else(|| {
            invalid(format!(
                "{subject}: saved controller occupation status is absent"
            ))
        })?;
        let core = if let Some(country) = map.core_country {
            let key = super::super::serialized_text(
                report,
                &format!("{subject}.core_country"),
                tag(country)?,
            );
            if !settings.occupied.iter().any(|tag| tag.as_slice() == key) {
                return Err(invalid(format!(
                    "{subject}: saved occupation group for retained country is absent"
                )));
            }
            Some(
                saved
                    .occupation_definitions
                    .get(&(controller.to_vec(), key.to_vec()))
                    .ok_or_else(|| {
                        invalid(format!(
                            "{subject}: saved occupation definition group is absent"
                        ))
                    })?,
            )
        } else {
            report.limitation(
                "the occupation group's embedded lookup link is not serialized; native group absence cannot be independently joined to a saved group key",
                format!("{subject}.core_group"),
            );
            None
        };
        for (name, captured, state_names, core_names) in [
            (
                "resistance",
                &map.resistance,
                &saved_state.definitions.resistance,
                core.map(|definitions| &definitions.resistance),
            ),
            (
                "compliance",
                &map.compliance,
                &saved_state.definitions.compliance,
                core.map(|definitions| &definitions.compliance),
            ),
        ] {
            let field = format!("{subject}.{name}");
            let captured = captured
                .iter()
                .enumerate()
                .map(|(index, token)| {
                    super::super::serialized_text(report, &format!("{field}.{index}"), token)
                })
                .collect::<Vec<_>>();
            let expected = state_names
                .iter()
                .chain(core_names.into_iter().flatten())
                .map(Vec::as_slice)
                .collect::<Vec<_>>();
            value(report, &field, captured, expected);
        }
        if core.is_some() {
            report.limitation(
                "the occupation group's embedded lookup link is not serialized; its saved key certifies the retained country but not the native group lookup",
                format!("{subject}.core_group"),
            );
        }
        let target_demanded = row.level == 0 && row.compliance <= 0;
        value(
            report,
            &format!("{subject}.stored_target_present"),
            map.stored_target.is_some(),
            target_demanded,
        );
        if target_demanded {
            report.scalar(
                &subject,
                "stored_target",
                map.stored_target,
                Some(&saved_state.target),
            );
        } else {
            report.limitation(
                "the occupation map captures its stored suppression target only for zero resistance and nonpositive compliance; other saved targets are outside this acquisition",
                format!("{subject}.stored_target"),
            );
        }
        matched += 1;
    }
    report.compared("StateResistance map", matched);
    Ok(())
}

pub fn collaboration(
    rows: &[replay_data::rows::CountryCollaborationRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        let source = required_country_tag(row.source, links)?;
        let target = required_country_tag(row.target, links)?;
        let key = (source.to_vec(), target.to_vec());
        if !seen.insert(key.clone()) {
            return Err(invalid("duplicate resolved country collaboration pair"));
        }
        let subject = format!(
            "collaboration.{}.{}",
            String::from_utf8_lossy(source),
            String::from_utf8_lossy(target)
        );
        let Some(saved) = saved.country_collaboration.get(&key) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        report.scalar(
            &subject,
            "progress",
            Some(row.progress),
            Some(&saved.progress),
        );
    }
    missing(
        report,
        "collaboration",
        &seen,
        saved.country_collaboration.keys().cloned(),
    );
    report.compared("CountryCollaboration", matched);
    Ok(())
}

pub fn resistance_hover(
    rows: &[replay_data::rows::StateResistanceRow],
    saved: &saved::Pack,
    intel: &installed_content_save::intel::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    use replay_data::rows::Recorded;
    let selector = |raw: i32| -> io::Result<Option<&[u8]>> {
        if raw > 0 {
            Ok(Some(required_country_tag(raw as u32, links)?))
        } else {
            Ok(None)
        }
    };
    let mut matched = 0;
    for row in rows.iter().filter(|row| row.active != 0) {
        let subject = format!("resistance.{}.hover", row.state);
        let Recorded::Value(Some(hover)) = &row.hover else {
            return Err(invalid("active state occupation hover was not acquired"));
        };
        let saved = saved
            .state_resistance
            .get(&row.state)
            .ok_or_else(|| invalid("active state resistance save is absent"))?;
        value(
            report,
            &format!("{subject}.added.count"),
            hover.added.len(),
            saved.added.len(),
        );
        for (index, (captured, saved)) in hover.added.iter().zip(&saved.added).enumerate() {
            let subject = format!("{subject}.added.{index}");
            report.scalar(
                &subject,
                "amount",
                Some(captured.value),
                Some(&saved.amount),
            );
            value(
                report,
                &format!("{subject}.controller"),
                selector(captured.controller)?,
                saved.controller.as_deref(),
            );
            value(
                report,
                &format!("{subject}.occupied"),
                selector(captured.occupied)?,
                saved.occupied.as_deref(),
            );
            let tooltip = (!captured.localisation.is_empty()).then(|| {
                captured
                    .localisation
                    .split(|byte| *byte == 0)
                    .next()
                    .unwrap()
            });
            value(
                report,
                &format!("{subject}.tooltip"),
                tooltip,
                saved.tooltip.as_deref(),
            );
        }
        let tag = required_country_tag(row.country, links)?;
        let diplomacy = intel
            .countries
            .get(tag)
            .and_then(|country| country.diplomacy.as_ref())
            .ok_or_else(|| invalid("occupied country diplomacy save is absent"))?;
        value(
            report,
            &format!("{subject}.exile_host"),
            selector(hover.exile_host)?,
            diplomacy.exile_host.as_deref(),
        );
        value(
            report,
            &format!("{subject}.legitimacy"),
            hover.legitimacy,
            diplomacy.legitimacy.as_ref().map_or(0, |v| v.raw),
        );
        let Recorded::Value(sources) = &row.modifier_sources else {
            return Err(invalid("occupation modifier sources were not acquired"));
        };
        let mut keys = BTreeSet::new();
        for source in sources {
            if !keys.insert((source.section, source.source))
                || source.section >= 6
                || source.source >= 4
            {
                return Err(invalid("occupation modifier source identity invalid"));
            }
            if !source.failure.is_empty() {
                return Err(invalid("occupation modifier source read failed"));
            }
            let total = source
                .total
                .ok_or_else(|| invalid("occupation modifier root total unavailable"))?;
            if total != 0 && source.section != 5 {
                replay_data::rows::check_tree(&source.nodes)?;
                value(
                    report,
                    &format!(
                        "{subject}.source.{}.{}.total",
                        source.section, source.source
                    ),
                    total,
                    source.nodes.first().map_or(0, |node| node.value),
                );
            }
        }
        if sources.len() != 22 {
            return Err(invalid("occupation modifier sources are incomplete"));
        }
        for effects in [&hover.resistance_effects, &hover.compliance_effects] {
            if effects.windows(2).any(|pair| pair[0].key >= pair[1].key) {
                return Err(invalid("occupation effect keys are not strictly ordered"));
            }
        }
        matched += 1;
    }
    report.compared("StateResistance hover", matched);
    report.limitation("resistance spread caches, compliance gain ratio, local effect accumulators and modifier source trees are runtime-only; their display needs a matched native hover", "StateResistance hover");
    Ok(())
}
