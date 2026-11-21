use super::*;

pub(super) fn compare_sharing_groups(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    _links: &BTreeMap<u32, &CountryRow>,
    names: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let groups = complete::<tables::TechSharingGroups>(frame)?;
    let mut saved_groups = BTreeMap::new();
    for group in &pack.tech_sharing {
        if saved_groups.insert(group.token.as_slice(), group).is_some() {
            return Err(invalid("duplicate saved technology-sharing ID"));
        }
    }
    equal(
        report,
        "TechSharingGroups.count",
        groups.len(),
        saved_groups.len(),
    );
    let mut captured_groups = BTreeMap::new();
    let mut matched = BTreeSet::new();
    let mut group_count = 0;
    let mut unresolved_groups = 0;
    for row in groups {
        if captured_groups.insert(row.id, row).is_some() {
            return Err(invalid("duplicate captured technology-sharing ID"));
        }
        let Some(token) = names.get(&row.id) else {
            unresolved_groups += 1;
            report.limitation(
                "technology-sharing group token is absent from the recorded lexical dictionary; group identity, per-group saved member count and ordered member joins cannot be compared",
                format!("TechSharingGroups.{}.id; TechSharingMembers.{}.group/ordinal", row.id, row.id),
            );
            continue;
        };
        if token.contains(&0) {
            unresolved_groups += 1;
            report.limitation(
                "technology-sharing token contains NUL and its concrete serializer target is unproved; group identity, saved member count and ordered member joins have unavailable mapping",
                format!("TechSharingGroups.{}.id; TechSharingMembers.{}.group/ordinal", row.id, row.id),
            );
            continue;
        }
        let token = *token;
        if !matched.insert(token) {
            return Err(invalid(
                "duplicate captured technology-sharing serialized token",
            ));
        }
        match saved_groups.get(token) {
            Some(saved) => {
                equal(
                    report,
                    "TechSharingGroups.member_count",
                    row.member_count as usize,
                    saved.countries.len(),
                );
                group_count += 1;
            }
            None => report.difference("TechSharingGroups", token, "missing saved group"),
        }
    }
    if unresolved_groups == 0 {
        for token in saved_groups.keys() {
            if !matched.contains(token) {
                report.difference("TechSharingGroups", "missing captured group", token);
            }
        }
    }
    report.compared("TechSharingGroups", group_count);
    report.limitation("technology-sharing saves exact group tokens and ordered member bonuses; base bonus and numeric lexical ID are not serialized", "TechSharingGroups");
    Ok(())
}

pub(super) fn compare_sharing(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    names: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let groups = complete::<tables::TechSharingGroups>(frame)?;
    let members = complete::<tables::TechSharingMembers>(frame)?;
    let mut saved_groups = BTreeMap::new();
    for group in &pack.tech_sharing {
        if saved_groups.insert(group.token.as_slice(), group).is_some() {
            return Err(invalid("duplicate saved technology-sharing ID"));
        }
    }
    let mut captured_groups = BTreeMap::new();
    let mut resolved_groups = BTreeMap::new();
    for row in groups {
        if captured_groups.insert(row.id, row).is_some() {
            return Err(invalid("duplicate captured technology-sharing ID"));
        }
        if let Some(&token) = names.get(&row.id).filter(|token| !token.contains(&0))
            && let Some(saved) = saved_groups.get(token)
        {
            resolved_groups.insert(row.id, *saved);
        }
    }
    equal(
        report,
        "TechSharingMembers.count",
        members.len(),
        pack.tech_sharing
            .iter()
            .map(|group| group.countries.len())
            .sum::<usize>(),
    );
    let mut keys = BTreeSet::new();
    let mut ordinals: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut actual_members = Vec::with_capacity(members.len());
    let mut member_count = 0;
    for row in members {
        if !captured_groups.contains_key(&row.group) {
            return Err(invalid("technology-sharing member has no group"));
        }
        if !keys.insert((row.group, row.ordinal)) {
            return Err(invalid("duplicate technology-sharing member ordinal"));
        }
        ordinals.entry(row.group).or_default().insert(row.ordinal);
        let country = required_country_tag(row.country, links)?;
        actual_members.push((country, row.bonus));
        let Some(group) = resolved_groups.get(&row.group) else {
            continue;
        };
        let Some(saved) = group
            .countries
            .get(row.ordinal as usize)
            .zip(group.bonuses.get(row.ordinal as usize))
        else {
            report.difference(
                "TechSharingMembers",
                (row.group, row.ordinal),
                "missing saved member",
            );
            continue;
        };
        equal(
            report,
            "TechSharingMembers.country",
            country,
            saved.0.as_slice(),
        );
        equal(report, "TechSharingMembers.bonus", row.bonus, saved.1.raw);
        member_count += 1;
    }
    for (&id, row) in &captured_groups {
        let ordinals = ordinals.remove(&id).unwrap_or_default();
        equal(
            report,
            format!("TechSharingGroups.{id}.captured_member_count"),
            ordinals.len(),
            row.member_count as usize,
        );
        check_ordinals(ordinals)?;
    }
    for (&id, group) in &resolved_groups {
        for ordinal in 0..group.countries.len() {
            if !keys.contains(&(id, ordinal as u32)) {
                report.difference(
                    "TechSharingMembers",
                    "missing captured member",
                    (&group.token, ordinal),
                );
            }
        }
    }
    let mut expected_members = pack
        .tech_sharing
        .iter()
        .flat_map(|group| {
            group
                .countries
                .iter()
                .zip(&group.bonuses)
                .map(|(country, bonus)| (country.as_slice(), bonus.raw))
        })
        .collect::<Vec<_>>();
    actual_members.sort_unstable();
    expected_members.sort_unstable();
    equal(
        report,
        "TechSharingMembers.country_bonus_multiset",
        actual_members,
        expected_members,
    );
    report.compared("TechSharingMembers", member_count);
    Ok(())
}
