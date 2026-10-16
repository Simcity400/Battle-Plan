use super::*;

pub(super) fn compare_army_upgrades(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = report.attempt("ArmyUpgradeGroups", |report| {
        compare_army_upgrade_groups(frame, saved, links, report)
    });
    complete &= report.attempt("ArmyUpgradeRequests", |report| {
        compare_army_upgrade_requests(frame, saved, links, report)
    });
    complete &= report.attempt("ArmyUpgradeDeliveries", |report| {
        compare_army_upgrade_deliveries(frame, saved, links, report)
    });
    complete &= report.attempt("ArmyUpgradeProduced", |report| {
        compare_army_upgrade_produced(frame, saved, links, report)
    });
    complete &= report.attempt("ArmyUpgradeLinks", |_| check_army_upgrade_links(frame));
    report.limitation("delivery progress is serialized only for status1; inactive progress is absent in both the captured row contract and save", "ArmyUpgradeDeliveries.progress_raw");
    report.limitation("delivery allow_zero_entries is serialized as a boolean and cannot certify nonzero raw byte values beyond truthiness", "ArmyUpgradeDeliveries.allow_zero_entries");
    report.limitation("the upgrade-request writer omits a null request equipment pointer; the native reader currently makes that request table unavailable, preserving the absent save handle without inventing a variant", "ArmyUpgradeRequests.equipment");
    if complete {
        Ok(())
    } else {
        Err(invalid("one or more army upgrade comparisons failed"))
    }
}

fn compare_army_upgrade_groups(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let divisions = saved
        .divisions
        .as_ref()
        .ok_or_else(|| invalid("save lacks army upgrade parent census"))?;
    let mut groups = BTreeMap::new();
    for row in complete::<tables::ArmyUpgradeGroups>(frame)? {
        if row.army == 0 || groups.insert(row.army, row).is_some() {
            return Err(invalid("null or duplicate army upgrade group parent"));
        }
    }
    let mut group_matches = 0;
    for (&army, division) in divisions {
        let expected = division.upgrades.as_ref();
        let actual = groups.remove(&army);
        let label = format!("army {army}.upgrades");
        if let Some(actual) = actual {
            group_matches += 1;
            equal(
                report,
                format!("{label}.country"),
                required_country_tag(actual.country, links)?,
                division.roster_owner.as_slice(),
            );
        }
        match (actual, expected) {
            (Some(actual), Some(expected)) => {
                equal(report, format!("{label}.priority"), actual.priority, expected.priority.unwrap_or(1));
            }
            (None, Some(_)) => report.difference(&label, "absent army group", "present nonempty group"),
            (Some(_), None) => report.limitation("the upgrade-group writer omits an upgrade group when both child vectors are empty; an empty captured group's priority is not serialized", &label),
            (None, None) => {},
        }
    }
    for army in groups.keys() {
        report.difference(
            format!("army {army}.upgrades"),
            "present",
            "absent division",
        );
    }
    report.compared("ArmyUpgradeGroups", group_matches);
    Ok(())
}

fn compare_army_upgrade_requests(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let divisions = saved
        .divisions
        .as_ref()
        .ok_or_else(|| invalid("save lacks army upgrade parent census"))?;
    let mut requests = BTreeMap::<u64, Vec<_>>::new();
    for row in complete::<tables::ArmyUpgradeRequests>(frame)? {
        if row.army == 0 {
            return Err(invalid("null army upgrade request parent"));
        }
        requests.entry(row.army).or_default().push(row);
    }
    let mut request_matches = 0;
    for (&army, division) in divisions {
        let expected = division.upgrades.as_ref();
        let label = format!("army {army}.upgrades");
        let mut actual_requests = requests.remove(&army).unwrap_or_default();
        actual_requests.sort_unstable_by_key(|row| row.ordinal);
        ordinals(
            actual_requests.iter().map(|row| row.ordinal),
            &format!("{label}.requests"),
        )?;
        for row in &actual_requests {
            equal(
                report,
                format!("{label}.request[{}].country", row.ordinal),
                required_country_tag(row.country, links)?,
                division.roster_owner.as_slice(),
            );
        }
        let expected_requests = expected
            .map(|group| group.requests.as_slice())
            .unwrap_or_default();
        for index in 0..actual_requests.len().max(expected_requests.len()) {
            let field = format!("{label}.request[{index}]");
            match (actual_requests.get(index), expected_requests.get(index)) {
                (Some(actual), Some(expected)) => {
                    request_matches += 1;
                    equal(
                        report,
                        format!("{field}.equipment"),
                        Some(actual.equipment),
                        expected.equipment,
                    );
                    report.scalar(
                        &field,
                        "request",
                        Some(actual.request_raw),
                        Some(&expected.request),
                    );
                    report.scalar(
                        &field,
                        "total",
                        Some(actual.total_raw),
                        Some(&expected.total),
                    );
                }
                (Some(_), None) => report.difference(field, "present", "absent"),
                (None, Some(_)) => report.difference(field, "absent", "present"),
                (None, None) => unreachable!(),
            }
        }
    }
    for army in requests.keys() {
        report.difference(
            format!("army {army}.upgrades.requests"),
            "present",
            "absent division",
        );
    }
    report.compared("ArmyUpgradeRequests", request_matches);
    Ok(())
}

fn compare_army_upgrade_deliveries(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let divisions = saved
        .divisions
        .as_ref()
        .ok_or_else(|| invalid("save lacks army upgrade parent census"))?;
    let mut deliveries = BTreeMap::<u64, Vec<_>>::new();
    for row in complete::<tables::ArmyUpgradeDeliveries>(frame)? {
        if row.army == 0 {
            return Err(invalid("null army upgrade delivery parent"));
        }
        deliveries.entry(row.army).or_default().push(row);
    }
    let mut delivery_matches = 0;
    for (&army, division) in divisions {
        let expected = division.upgrades.as_ref();
        let label = format!("army {army}.upgrades");
        let mut actual_deliveries = deliveries.remove(&army).unwrap_or_default();
        actual_deliveries.sort_unstable_by_key(|row| row.ordinal);
        ordinals(
            actual_deliveries.iter().map(|row| row.ordinal),
            &format!("{label}.deliveries"),
        )?;
        for row in &actual_deliveries {
            equal(
                report,
                format!("{label}.delivery[{}].country", row.ordinal),
                required_country_tag(row.country, links)?,
                division.roster_owner.as_slice(),
            );
        }
        let expected_deliveries = expected
            .map(|group| group.deliveries.as_slice())
            .unwrap_or_default();
        for index in 0..actual_deliveries.len().max(expected_deliveries.len()) {
            let field = format!("{label}.delivery[{index}]");
            let expected = expected_deliveries.get(index);
            match (actual_deliveries.get(index), expected) {
                (Some(actual), Some(expected)) => {
                    delivery_matches += 1;
                    equal(
                        report,
                        format!("{field}.status"),
                        actual.status,
                        expected.status.unwrap_or(0),
                    );
                    equal(
                        report,
                        format!("{field}.progress"),
                        actual.progress_raw,
                        expected.progress.as_ref().map(|value| value.raw),
                    );
                    report.scalar_checks += usize::from(expected.progress.is_some()) + 1;
                    equal(
                        report,
                        format!("{field}.total_progress"),
                        actual.total_progress_raw,
                        expected
                            .total_progress
                            .as_ref()
                            .map_or(100_000, |value| value.raw),
                    );
                    equal(
                        report,
                        format!("{field}.allow_zero_entries"),
                        Some(actual.allow_zero_entries != 0),
                        expected.produced.allow_zero_entries,
                    );
                }
                (Some(_), None) => report.difference(&field, "present", "absent"),
                (None, Some(_)) => report.difference(&field, "absent", "present"),
                (None, None) => unreachable!(),
            }
        }
    }
    for army in deliveries.keys() {
        report.difference(
            format!("army {army}.upgrades.deliveries"),
            "present",
            "absent division",
        );
    }
    report.compared("ArmyUpgradeDeliveries", delivery_matches);
    Ok(())
}

fn compare_army_upgrade_produced(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let divisions = saved
        .divisions
        .as_ref()
        .ok_or_else(|| invalid("save lacks army upgrade parent census"))?;
    let mut produced = BTreeMap::<(u64, u32), Vec<_>>::new();
    for row in complete::<tables::ArmyUpgradeProduced>(frame)? {
        if row.army == 0 {
            return Err(invalid("null produced equipment army parent"));
        }
        produced
            .entry((row.army, row.delivery_ordinal))
            .or_default()
            .push(row);
    }
    let mut produced_matches = 0;
    for (&army, division) in divisions {
        let Some(expected) = &division.upgrades else {
            continue;
        };
        for (index, delivery) in expected.deliveries.iter().enumerate() {
            let ordinal =
                u32::try_from(index).map_err(|_| invalid("saved delivery index exceeds u32"))?;
            let mut rows = produced.remove(&(army, ordinal)).unwrap_or_default();
            rows.sort_unstable_by_key(|row| row.ordinal);
            for row in &rows {
                equal(
                    report,
                    format!(
                        "army {army}.upgrades.delivery[{index}].produced[{}].country",
                        row.ordinal
                    ),
                    required_country_tag(row.country, links)?,
                    division.roster_owner.as_slice(),
                );
            }
            produced_matches += compare_pool(
                rows.iter()
                    .map(|row| (row.ordinal, row.equipment, row.amount_raw)),
                &delivery.produced,
                &format!("army {army}.upgrades.delivery[{index}].produced"),
                report,
            )?;
        }
    }
    for ((army, delivery), rows) in produced {
        report.difference(
            format!("army {army}.upgrades.delivery[{delivery}].produced"),
            rows.len(),
            "absent delivery",
        );
    }
    report.compared("ArmyUpgradeProduced", produced_matches);
    Ok(())
}

fn check_army_upgrade_links(frame: &DecodedFrame) -> io::Result<()> {
    let mut groups = BTreeMap::new();
    for row in complete::<tables::ArmyUpgradeGroups>(frame)? {
        if row.army == 0 || groups.insert(row.army, row).is_some() {
            return Err(invalid("null or duplicate army upgrade group parent"));
        }
    }
    for row in complete::<tables::ArmyUpgradeRequests>(frame)? {
        let group = groups
            .get(&row.army)
            .ok_or_else(|| invalid("army upgrade request has no captured group"))?;
        if row.country != group.country {
            return Err(invalid(
                "army upgrade request country differs from its group",
            ));
        }
    }
    let mut delivery_keys = BTreeSet::new();
    for row in complete::<tables::ArmyUpgradeDeliveries>(frame)? {
        let group = groups
            .get(&row.army)
            .ok_or_else(|| invalid("army upgrade delivery has no captured group"))?;
        if row.country != group.country || !delivery_keys.insert((row.army, row.ordinal)) {
            return Err(invalid(
                "duplicate army delivery key or inconsistent country",
            ));
        }
    }
    for row in complete::<tables::ArmyUpgradeProduced>(frame)? {
        if !delivery_keys.contains(&(row.army, row.delivery_ordinal)) {
            return Err(invalid(
                "produced equipment has no captured army upgrade delivery",
            ));
        }
        if row.country != groups[&row.army].country {
            return Err(invalid(
                "produced equipment country differs from its army upgrade group",
            ));
        }
    }
    Ok(())
}
