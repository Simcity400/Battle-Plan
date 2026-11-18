//! Derive research speed from recorded hourly operands, never staggered tables.
use super::{ResearchModifier, ResearchModifierSource, ResearchModifiers};
use replay_data::{rows::ResearchInputRow, rows::ResearchModifierRow as R};

pub(super) fn breakdown(input: &ResearchInputRow, rows: Option<&[R]>) -> Option<ResearchModifiers> {
    if input.technology_present == 0 {
        return None;
    }
    let rows = rows?;
    let first = rows.partition_point(|r| r.country < input.country);
    let last = rows.partition_point(|r| r.country <= input.country);
    let country = &rows[first..last];
    let shared = &country[country.partition_point(|r| r.index < u32::MAX)..];
    let slot = &country[country.partition_point(|r| r.index < input.index)
        ..country.partition_point(|r| r.index <= input.index)];
    scoped(input, shared, slot)
}

/// [`breakdown`] of `input`, from its owner's rows of the country scope (index `u32::MAX`) and
/// of its slot, each in key order.
fn scoped(input: &ResearchInputRow, shared: &[R], slot: &[R]) -> Option<ResearchModifiers> {
    if input.technology_present == 0 {
        return None;
    }
    // No row-count cap: a sharing group writes one member row per country, so
    // any fixed limit would drop the breakdown for large groups. Every check
    // below is linear in these rows, or bounded by the 32 groups.
    if slot.is_empty()
        || shared.is_empty()
        || [shared, slot].iter().any(|rows| {
            rows.iter().enumerate().any(|(i, r)| {
                r.ordinal as usize != i
                    || r.name.len() > 512
                    || std::str::from_utf8(&r.name).is_err()
            })
        })
        || slot
            .iter()
            .any(|r| r.technology_handle != input.technology_handle)
        || shared.iter().any(|r| r.technology_handle != 0)
    {
        return None;
    }
    for scope in [shared, slot] {
        for row in scope {
            let owner = |ordinal: u32| scope.get(ordinal as usize).map(|r| r.kind);
            let valid = match row.kind {
                R::MODIFIER_METADATA => matches!(
                    owner(row.source_id),
                    Some(
                        R::COUNTRY_SPEED
                            | R::COUNTRY_SOURCE
                            | R::SHARING_MULTIPLIER
                            | R::SCIENTIST_SOURCE
                    )
                ),
                R::SHARING_MEMBER | R::SHARING_MISSING_MEMBER | R::SHARING_FILTER => {
                    owner(row.count) == Some(R::SHARING)
                }
                R::SHARING_PREDECESSOR => owner(row.count) == Some(R::SHARING_MEMBER),
                _ => true,
            };
            if !valid {
                return None;
            }
        }
    }
    let unique = |rows: &[R], kind| {
        let mut matching = rows.iter().filter(|r| r.kind == kind);
        let row = matching.next()?;
        matching.next().is_none().then_some(row.raw)
    };
    if unique(slot, R::TECHNOLOGY)? != i64::from(input.technology_handle) {
        return None;
    }
    let year = i32::try_from(unique(slot, R::YEAR)?).ok()?;
    let ahead_factor = unique(shared, R::AHEAD_FACTOR)?;
    let minimum = unique(shared, R::MINIMUM_SPEED)?;
    let sharing_maximum = unique(shared, R::SHARING_MAXIMUM)?;
    let effective = |rows: &[R], kind| {
        let mut matching = rows.iter().filter(|r| r.kind == kind);
        let row = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        modifier_value(rows, row)
    };
    let sharing_multiplier = effective(shared, R::SHARING_MULTIPLIER)?;
    let scientist_count = usize::try_from(unique(slot, R::SCIENTIST)?).ok()?;
    let scientist_rows: Vec<_> = slot
        .iter()
        .filter(|r| r.kind == R::SCIENTIST_CATEGORY)
        .collect();
    // Any number of scientist categories: the rows name as many as were recorded.
    if scientist_rows.len() != scientist_count {
        return None;
    }
    let scientist = i64::try_from(
        scientist_rows
            .iter()
            .map(|r| i128::from(r.raw))
            .sum::<i128>(),
    )
    .ok()?;
    // Native groups clamp separately, then their sum clamps a second time.
    if sharing_maximum < 0 {
        return None;
    }
    let mut sharing_sources = Vec::new();
    let mut sharing = 0_i128;
    // Sharing groups are content (`common/technology_sharing`): a mod may add any number.
    let groups: Vec<_> = slot.iter().filter(|r| r.kind == R::SHARING).collect();
    let mut predecessor_rows = std::collections::BTreeMap::<u32, Vec<&R>>::new();
    for row in slot.iter().filter(|r| r.kind == R::SHARING_PREDECESSOR) {
        u32::try_from(row.raw).ok()?;
        predecessor_rows.entry(row.count).or_default().push(row);
    }
    let doctrine = if groups.is_empty() {
        0
    } else {
        u8::try_from(unique(slot, R::SHARING_DOCTRINE)?).ok()?
    };
    for row in &groups {
        let members: Vec<_> = slot
            .iter()
            .filter(|r| {
                matches!(r.kind, R::SHARING_MEMBER | R::SHARING_MISSING_MEMBER)
                    && r.count == row.ordinal
            })
            .collect();
        if members.len() != row.count as usize {
            return None;
        }
        let mut contributors = 0_u32;
        for member in members {
            i32::try_from(member.raw).ok()?;
            i32::try_from(member.auxiliary).ok()?;
            let predecessors = predecessor_rows
                .get(&member.ordinal)
                .map_or(&[][..], Vec::as_slice);
            if member.kind == R::SHARING_MEMBER
                && member.raw >= member.auxiliary
                && (predecessors.is_empty()
                    || predecessors.iter().any(|p| p.raw != 6 && p.raw != 1))
            {
                contributors += 1;
            }
        }
        let filters: Vec<_> = slot
            .iter()
            .filter(|r| r.kind == R::SHARING_FILTER && r.count == row.ordinal)
            .collect();
        let matches = filters.is_empty()
            || filters.iter().any(|f| {
                slot.iter().any(|c| {
                    c.kind == R::SHARING_CATEGORY && c.source_id == f.source_id && c.name == f.name
                })
            });
        let contribution = ((i128::from(row.raw) + i128::from(row.auxiliary))
            .checked_mul(i128::from(contributors))?
            .checked_mul(100_000 + i128::from(sharing_multiplier))?
            / 100_000)
            .clamp(0, i128::from(sharing_maximum));
        let contribution = if doctrine == 0 && matches {
            contribution
        } else {
            0
        };
        sharing += contribution;
        sharing_sources.push(source(row, contribution.to_string()));
    }
    sharing = sharing.clamp(0, i128::from(sharing_maximum));
    let mut components = Vec::new();
    for (kind, raw, source_kinds, source_rows) in [
        (
            "country",
            effective(shared, R::COUNTRY_SPEED)?,
            &[R::COUNTRY_SOURCE][..],
            shared,
        ),
        (
            "category",
            unique(slot, R::CATEGORY_SPEED)?,
            &[R::IDEA_SOURCE, R::SCRIPT_SOURCE][..],
            slot,
        ),
        ("designTeam", unique(slot, R::DESIGN_TEAM)?, &[][..], slot),
        ("experience", unique(slot, R::EXPERIENCE)?, &[][..], slot),
        ("license", unique(slot, R::LICENSE)?, &[][..], slot),
        ("scientist", scientist, &[R::SCIENTIST_SOURCE][..], slot),
        ("applied", input.speed_bonus_raw, &[][..], slot),
    ] {
        components.push(ResearchModifier {
            kind: kind.into(),
            raw: raw.to_string(),
            sources: source_rows
                .iter()
                .filter(|r| source_kinds.contains(&r.kind))
                .map(|r| Some(source(r, modifier_value(source_rows, r)?.to_string())))
                .collect::<Option<Vec<_>>>()?,
        });
    }
    components.push(ResearchModifier {
        kind: "sharing".into(),
        raw: sharing.to_string(),
        sources: sharing_sources,
    });
    let total = components
        .iter()
        .try_fold(0_i128, |sum, c| Some(sum + c.raw.parse::<i128>().ok()?))?;
    Some(ResearchModifiers {
        components,
        total_bonus_raw: i64::try_from(total).ok()?.to_string(),
        year,
        ahead_factor_raw: ahead_factor.to_string(),
        minimum_speed_raw: minimum.to_string(),
    })
}

/// Apply native definition metadata only after decoding. Metadata refers to the
/// value row's ordinal within the same country/slot scope.
fn modifier_value(rows: &[R], row: &R) -> Option<i64> {
    if !matches!(
        row.kind,
        R::COUNTRY_SPEED | R::COUNTRY_SOURCE | R::SHARING_MULTIPLIER | R::SCIENTIST_SOURCE
    ) {
        return Some(row.raw);
    }
    let mut matching = rows
        .iter()
        .filter(|r| r.kind == R::MODIFIER_METADATA && r.source_id == row.ordinal);
    let Some(metadata) = matching.next() else {
        return (row.raw == 0).then_some(0);
    };
    if matching.next().is_some() {
        return None;
    }
    let flags = u32::try_from(metadata.raw).ok()?;
    let eligibility = u32::try_from(metadata.auxiliary).ok()?;
    Some(
        if metadata.count != 0 && metadata.count & eligibility == 0 {
            0
        } else if flags & (1 << 4) != 0 {
            row.raw.clamp(0, 100_000)
        } else {
            row.raw
        },
    )
}

fn source(row: &R, raw: String) -> ResearchModifierSource {
    ResearchModifierSource {
        token: (!row.name.is_empty()).then(|| String::from_utf8(row.name.clone()).unwrap()),
        name: None,
        raw,
    }
}

/// Integer days, half-away rounding of reduced years,
/// then denominator penalty and minimum clamp. Positive fixed5 is faster.
pub(super) fn rate(modifiers: &ResearchModifiers, reduction: i64, hour: i32) -> Option<i128> {
    let reduction = i128::from(reduction) * 365;
    let rounded = (reduction.abs() + 50_000) / 100_000 * reduction.signum();
    let days =
        (i128::from(modifiers.year) * 365 - (i128::from(hour) - 43_800_000) / 24 - rounded).max(0);
    let penalty = days.checked_mul(modifiers.ahead_factor_raw.parse::<i128>().ok()?)? / 365;
    let numerator = 100_000 + modifiers.total_bonus_raw.parse::<i128>().ok()?;
    let denominator = 100_000 + penalty;
    if denominator <= 0 {
        return None;
    }
    Some((numerator * 100_000 / denominator).max(modifiers.minimum_speed_raw.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (ResearchInputRow, Vec<R>) {
        let input = ResearchInputRow {
            country: 7,
            index: 0,
            technology_handle: 22,
            database_index: replay_data::rows::Recorded::Value(22),
            technology_present: 1,
            token: b"radio".to_vec(),
            cost_raw: 100_000,
            cost_scale_raw: 100_000,
            ..Default::default()
        };
        let slot = [
            (R::TECHNOLOGY, 22),
            (R::YEAR, 1938),
            (R::CATEGORY_SPEED, -10_000),
            (R::DESIGN_TEAM, 5_000),
            (R::EXPERIENCE, 0),
            (R::LICENSE, 0),
            (R::SCIENTIST, 0),
        ];
        let shared = [
            (R::COUNTRY_SPEED, 25_000),
            (R::AHEAD_FACTOR, 200_000),
            (R::MINIMUM_SPEED, 10_000),
            (R::SHARING_MAXIMUM, 50_000),
            (R::SHARING_MULTIPLIER, 100_000),
        ];
        let mut rows: Vec<_> = slot
            .into_iter()
            .enumerate()
            .map(|(ordinal, (kind, raw))| R {
                country: 7,
                index: 0,
                technology_handle: 22,
                ordinal: ordinal as u32,
                kind,
                raw,
                ..Default::default()
            })
            .collect();
        rows.extend(
            shared
                .into_iter()
                .enumerate()
                .map(|(ordinal, (kind, raw))| R {
                    country: 7,
                    index: u32::MAX,
                    ordinal: ordinal as u32,
                    kind,
                    raw,
                    ..Default::default()
                }),
        );
        for source_id in [0, 4] {
            rows.push(R {
                country: 7,
                index: u32::MAX,
                ordinal: rows.iter().filter(|r| r.index == u32::MAX).count() as u32,
                kind: R::MODIFIER_METADATA,
                source_id,
                ..Default::default()
            });
        }
        (input, rows)
    }
    fn slot_row(rows: &mut Vec<R>, mut row: R) -> u32 {
        let at = rows.partition_point(|r| r.index == 0);
        row.country = 7;
        row.index = 0;
        row.technology_handle = 22;
        row.ordinal = at as u32;
        rows.insert(at, row);
        at as u32
    }
    #[test]
    fn positive_and_negative_components_and_day_boundaries_follow_native_formula() {
        let (input, rows) = fixture();
        let mut m = breakdown(&input, Some(&rows)).unwrap();
        assert_eq!(m.total_bonus_raw, "20000");
        let hour = 43_800_000 + 1937 * 8760;
        assert_eq!(rate(&m, 0, hour), Some(40_000)); // +20%, divided by 1+200%.
        assert_eq!(rate(&m, 0, hour + 23), Some(40_000));
        assert!(rate(&m, 0, hour + 24).unwrap() > 40_000);
        assert_eq!(rate(&m, 100_000, hour), Some(120_000));
        assert_eq!(rate(&m, 0, hour + 8760), Some(120_000));
        m.total_bonus_raw = "-250000".into();
        assert_eq!(rate(&m, 0, hour), Some(10_000));
    }
    #[test]
    fn sharing_retains_sources_and_clamps_per_group_then_combined() {
        let (input, mut rows) = fixture();
        slot_row(
            &mut rows,
            R {
                kind: R::SHARING_DOCTRINE,
                ..Default::default()
            },
        );
        for (i, raw) in [50_000, -50_000, 40_000].into_iter().enumerate() {
            let group = slot_row(
                &mut rows,
                R {
                    kind: R::SHARING,
                    raw,
                    auxiliary: 10_000,
                    count: 2,
                    name: format!("group_{i}").into_bytes(),
                    ..Default::default()
                },
            );
            for source_id in [1, 2] {
                slot_row(
                    &mut rows,
                    R {
                        kind: R::SHARING_MEMBER,
                        raw: 1,
                        auxiliary: 1,
                        count: group,
                        source_id,
                        ..Default::default()
                    },
                );
            }
        }
        let m = breakdown(&input, Some(&rows)).unwrap();
        let sharing = m.components.iter().find(|c| c.kind == "sharing").unwrap();
        assert_eq!(sharing.raw, "50000");
        assert_eq!(
            sharing
                .sources
                .iter()
                .map(|s| s.raw.as_str())
                .collect::<Vec<_>>(),
            ["50000", "0", "50000"]
        );
        assert_eq!(m.total_bonus_raw, "70000");
        rows.iter_mut()
            .find(|r| r.kind == R::SHARING_MEMBER)
            .unwrap()
            .raw = i64::MAX;
        assert!(breakdown(&input, Some(&rows)).is_none());
    }
    #[test]
    fn sharing_groups_with_thousands_of_member_countries_keep_their_breakdown() {
        const MEMBERS: u32 = 5_000;
        let (input, rows) = fixture();
        let at = rows.partition_point(|r| r.index == 0);
        let (slot, shared) = rows.split_at(at);
        let mut rows = slot.to_vec();
        let mut push = |mut row: R| -> u32 {
            row.country = 7;
            row.index = 0;
            row.technology_handle = 22;
            row.ordinal = rows.len() as u32;
            rows.push(row);
            rows.len() as u32 - 1
        };
        push(R {
            kind: R::SHARING_DOCTRINE,
            ..Default::default()
        });
        let group = push(R {
            kind: R::SHARING,
            raw: 1,
            count: MEMBERS,
            ..Default::default()
        });
        for source_id in 0..MEMBERS {
            push(R {
                kind: R::SHARING_MEMBER,
                raw: 1,
                auxiliary: 1,
                count: group,
                source_id,
                ..Default::default()
            });
        }
        rows.extend_from_slice(shared);
        let m = breakdown(&input, Some(&rows)).unwrap();
        let sharing = m.components.iter().find(|c| c.kind == "sharing").unwrap();
        // One point per member, doubled by the fixture's +100% multiplier,
        // within the fixture's 50,000 maximum.
        assert_eq!(sharing.raw, "10000");
        assert_eq!(sharing.sources[0].raw, "10000");
        assert_eq!(m.total_bonus_raw, "30000");
    }

    #[test]
    fn missing_tables_incomplete_rows_and_changed_assignments_never_become_zero() {
        let (input, mut rows) = fixture();
        assert!(breakdown(&input, None).is_none());
        assert!(breakdown(&input, Some(&[])).is_none());
        rows[0].technology_handle = 23;
        assert!(breakdown(&input, Some(&rows)).is_none());
        rows[0].technology_handle = 22;
        rows[0].kind = R::YEAR;
        assert!(breakdown(&input, Some(&rows)).is_none());
    }
    #[test]
    fn raw_metadata_and_scientist_categories_are_derived_only_after_decode() {
        let (input, mut rows) = fixture();
        rows.iter_mut()
            .find(|r| r.kind == R::COUNTRY_SPEED)
            .unwrap()
            .raw = 250_000;
        rows.iter_mut()
            .find(|r| r.kind == R::MODIFIER_METADATA && r.source_id == 0)
            .unwrap()
            .raw = 1 << 4;
        assert_eq!(
            breakdown(&input, Some(&rows)).unwrap().total_bonus_raw,
            "95000"
        );
        rows.iter_mut()
            .find(|r| r.kind == R::MODIFIER_METADATA && r.source_id == 0)
            .unwrap()
            .count = 4;
        assert_eq!(
            breakdown(&input, Some(&rows)).unwrap().total_bonus_raw,
            "-5000"
        );
        rows.iter_mut()
            .find(|r| r.kind == R::MODIFIER_METADATA && r.source_id == 0)
            .unwrap()
            .auxiliary = 4;
        rows.iter_mut()
            .find(|r| r.kind == R::SCIENTIST)
            .unwrap()
            .raw = 2;
        for (source_id, raw) in [(17, 12_000), (18, -2_000)] {
            slot_row(
                &mut rows,
                R {
                    kind: R::SCIENTIST_CATEGORY,
                    raw,
                    source_id,
                    ..Default::default()
                },
            );
        }
        assert_eq!(
            breakdown(&input, Some(&rows)).unwrap().total_bonus_raw,
            "105000"
        );
        let country_value = rows.iter().find(|r| r.kind == R::COUNTRY_SPEED).unwrap();
        assert_eq!(modifier_value(&[], country_value), None);
        let malformed = R {
            kind: R::MODIFIER_METADATA,
            source_id: country_value.ordinal,
            raw: -1,
            ..Default::default()
        };
        assert_eq!(modifier_value(&[malformed], country_value), None);
        for row in rows.iter_mut().filter(|r| r.kind == R::SCIENTIST_CATEGORY) {
            row.raw = i64::MAX;
        }
        assert!(breakdown(&input, Some(&rows)).is_none());
    }

    #[test]
    fn sharing_witnesses_preserve_members_but_replay_decides_contribution() {
        let (input, mut rows) = fixture();
        let doctrine = slot_row(
            &mut rows,
            R {
                kind: R::SHARING_DOCTRINE,
                ..Default::default()
            },
        );
        let group = slot_row(
            &mut rows,
            R {
                kind: R::SHARING,
                raw: 10_000,
                count: 3,
                ..Default::default()
            },
        );
        let first = slot_row(
            &mut rows,
            R {
                kind: R::SHARING_MEMBER,
                raw: 1,
                auxiliary: 1,
                count: group,
                source_id: 1,
                ..Default::default()
            },
        );
        let predecessor = slot_row(
            &mut rows,
            R {
                kind: R::SHARING_PREDECESSOR,
                raw: 6,
                count: first,
                ..Default::default()
            },
        );
        let second = slot_row(
            &mut rows,
            R {
                kind: R::SHARING_MEMBER,
                raw: 0,
                auxiliary: 1,
                count: group,
                source_id: 2,
                ..Default::default()
            },
        );
        slot_row(
            &mut rows,
            R {
                kind: R::SHARING_MISSING_MEMBER,
                auxiliary: 1,
                count: group,
                source_id: 3,
                ..Default::default()
            },
        );
        let sharing = |rows: &[R]| {
            breakdown(&input, Some(rows))
                .unwrap()
                .components
                .into_iter()
                .find(|c| c.kind == "sharing")
                .unwrap()
                .raw
        };
        assert_eq!(sharing(&rows), "0");
        rows[second as usize].raw = 1;
        assert_eq!(sharing(&rows), "20000");
        rows[predecessor as usize].raw = 4;
        assert_eq!(sharing(&rows), "40000");
        slot_row(
            &mut rows,
            R {
                kind: R::SHARING_FILTER,
                count: group,
                source_id: 42,
                name: b"electronics".to_vec(),
                ..Default::default()
            },
        );
        let category = slot_row(
            &mut rows,
            R {
                kind: R::SHARING_CATEGORY,
                source_id: 42,
                name: b"infantry".to_vec(),
                ..Default::default()
            },
        );
        assert_eq!(sharing(&rows), "0"); // Matching hashes do not establish equal categories.
        rows[category as usize].name = b"electronics".to_vec();
        assert_eq!(sharing(&rows), "40000");
        rows[doctrine as usize].raw = 1;
        assert_eq!(sharing(&rows), "0");
        rows[predecessor as usize].count = u32::MAX;
        assert!(breakdown(&input, Some(&rows)).is_none());
    }
}
