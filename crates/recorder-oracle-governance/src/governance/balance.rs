use super::*;

pub(super) fn compare_power_balance(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    _links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::PowerBalances>(frame)?;
    let Some(saved) = &pack.power_balances else {
        if !rows.is_empty() {
            report.difference("PowerBalances", "captured state", "missing save manager");
        }
        report.compared("PowerBalances", 0);
        return Ok(());
    };
    let mut records = BTreeSet::new();
    let mut count = 0;
    for row in rows {
        if !records.insert(row.ordinal) {
            return Err(invalid("duplicate power-balance ordinal"));
        }
        let Some(saved) = saved.get(row.ordinal as usize) else {
            report.difference("PowerBalances", row.ordinal, "missing saved record");
            continue;
        };
        equal(
            report,
            "PowerBalances.value",
            row.value_raw,
            saved.value.raw,
        );
        for (name, actual, expected) in [
            ("template", &row.balance, &saved.template),
            ("left", &row.left, &saved.left),
            ("right", &row.right, &saved.right),
            ("trending", &row.trending, &saved.trending),
        ] {
            compare_literal(
                report,
                &format!("PowerBalances.{}.{name}", row.ordinal),
                actual,
                expected,
            );
        }
        count += 1;
    }
    for ordinal in 0..saved.len() {
        if !records.contains(&(ordinal as u32)) {
            report.difference("PowerBalances", "missing captured record", ordinal);
        }
    }
    report.compared("PowerBalances", count);
    Ok(())
}

pub(super) fn compare_balance_members(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let members = complete::<tables::PowerBalanceMembers>(frame)?;
    let Some(saved) = &pack.power_balances else {
        if !members.is_empty() {
            report.difference(
                "PowerBalanceMembers",
                "captured state",
                "missing save manager",
            );
        }
        report.compared("PowerBalanceMembers", 0);
        return Ok(());
    };
    let mut member_keys = BTreeSet::new();
    let mut member_count = 0;
    for row in members {
        if !member_keys.insert((row.record, row.ordinal)) {
            return Err(invalid("orphan or duplicate power-balance member"));
        }
        let Some(tag) = saved
            .get(row.record as usize)
            .and_then(|r| r.countries.get(row.ordinal as usize))
        else {
            report.difference(
                "PowerBalanceMembers",
                (row.record, row.ordinal),
                "missing saved member",
            );
            continue;
        };
        equal(
            report,
            "PowerBalanceMembers.country",
            required_country_tag(row.country, links)?,
            tag.as_slice(),
        );
        member_count += 1;
    }
    for (record, saved) in saved.iter().enumerate() {
        for ordinal in 0..saved.countries.len() {
            if !member_keys.contains(&(record as u32, ordinal as u32)) {
                report.difference(
                    "PowerBalanceMembers",
                    "missing captured member",
                    (record, ordinal),
                );
            }
        }
    }
    report.compared("PowerBalanceMembers", member_count);
    report.limitation("power-balance member daily/weekly accumulator operands, presence and metadata flags are not serialized by the balance writer", "PowerBalanceMembers");
    Ok(())
}

pub(super) fn compare_balance_overrides(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    _links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let overrides = complete::<tables::PowerBalanceOverrides>(frame)?;
    let Some(saved) = &pack.power_balances else {
        if !overrides.is_empty() {
            report.difference(
                "PowerBalanceOverrides",
                "captured state",
                "missing save manager",
            );
        }
        report.compared("PowerBalanceOverrides", 0);
        return Ok(());
    };
    let mut override_keys = BTreeSet::new();
    let mut override_count = 0;
    for row in overrides {
        if !override_keys.insert((row.record, row.ordinal)) {
            return Err(invalid("orphan or duplicate power-balance override"));
        }
        let Some(saved) = saved
            .get(row.record as usize)
            .and_then(|r| r.overrides.get(row.ordinal as usize))
        else {
            report.difference(
                "PowerBalanceOverrides",
                (row.record, row.ordinal),
                "missing saved override",
            );
            continue;
        };
        compare_literal(report, "PowerBalanceOverrides.side", &row.side, &saved.side);
        compare_literal(report, "PowerBalanceOverrides.gfx", &row.gfx, &saved.gfx);
        override_count += 1;
    }
    for (record, saved) in saved.iter().enumerate() {
        for ordinal in 0..saved.overrides.len() {
            if !override_keys.contains(&(record as u32, ordinal as u32)) {
                report.difference(
                    "PowerBalanceOverrides",
                    "missing captured override",
                    (record, ordinal),
                );
            }
        }
    }
    report.compared("PowerBalanceOverrides", override_count);
    Ok(())
}

pub(super) fn compare_balance_ranges(
    frame: &DecodedFrame,
    _pack: &saved::Pack,
    _links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let ranges = complete::<tables::PowerBalanceRanges>(frame)?;
    let records = complete::<tables::PowerBalances>(frame)?
        .iter()
        .map(|row| row.ordinal)
        .collect::<BTreeSet<_>>();
    let mut keys = BTreeSet::new();
    for row in ranges {
        if !records.contains(&row.record) || !keys.insert((row.record, row.ordinal)) {
            return Err(invalid("orphan or duplicate power-balance range"));
        }
    }
    report.compared("PowerBalanceRanges", 0);
    report.limitation("power-balance active-range cache and range vector are not serialized; side tokens are not substitutes for range identity", format!("PowerBalanceRanges: {} rows", ranges.len()));
    Ok(())
}
