use super::*;

pub(in crate::oracle) fn occupation_settings(
    rows: &[replay::rows::OccupationSettingsRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
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
                    let occupied = occupied
                        .iter()
                        .map(|&raw| required_country_tag(raw, links))
                        .collect::<io::Result<Vec<_>>>()?;
                    value(
                        report,
                        &format!("{subject}.occupied"),
                        occupied,
                        saved.occupied.iter().map(Vec::as_slice).collect::<Vec<_>>(),
                    );
                }
                None => report.limitation(
                    "occupied-country roster unavailable from native capture",
                    format!("{subject}.occupied"),
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
        if let Some(law) = &saved.default_law {
            value(
                report,
                &format!("{subject}.default_law"),
                row.law.as_slice(),
                law.as_slice(),
            );
        }
    }
    missing(
        report,
        "occupation settings",
        &seen,
        saved.occupation_settings.keys().cloned(),
    );
    report.compared("OccupationSettings", matched);
    Ok(())
}

pub(in crate::oracle) fn occupation_laws(
    rows: &[replay::rows::StateOccupationLawRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        let controller = required_country_tag(row.controller, links)?;
        let occupied = required_country_tag(row.occupied, links)?;
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
        value(report, &subject, row.name.as_slice(), saved.law.as_slice());
    }
    missing(
        report,
        "occupation law",
        &seen,
        saved.state_occupation_laws.keys().cloned(),
    );
    report.compared("StateOccupationLaws", matched);
    Ok(())
}

pub(in crate::oracle) fn garrisons(
    rows: &[replay::rows::StateGarrisonRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        let controller = required_country_tag(row.controller, links)?;
        let occupied = required_country_tag(row.occupied, links)?;
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
        value(
            report,
            &format!("{subject}.occupied"),
            occupied,
            saved.occupied.as_slice(),
        );
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
    Ok(())
}

pub(in crate::oracle) fn assigned_manpower(
    rows: &[replay::rows::StateAssignedManpowerRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut groups = BTreeMap::<saved::GarrisonKey, Vec<(u32, u32, i32)>>::new();
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
        value(
            report,
            &format!("{subject}.occupied"),
            required_country_tag(row.occupied, links)?,
            saved.occupied.as_slice(),
        );
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
    Ok(())
}

pub(in crate::oracle) fn reserve_manpower(
    rows: &[replay::rows::CountryOccupationReserveManpowerRow],
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
    for (country, saved) in &saved.country_reserve_manpower {
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

pub(in crate::oracle) fn resistance(
    rows: &[replay::rows::StateResistanceRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
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
            value(
                report,
                &format!("{subject}.country"),
                Some(required_country_tag(row.country, links)?),
                saved.country.as_deref(),
            );
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
    Ok(())
}

pub(in crate::oracle) fn collaboration(
    rows: &[replay::rows::CountryCollaborationRow],
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
