//! Unit requirement text: 141618D30 -> 141579E40 -> 1415790B0/141578C80.
use super::{
    ensure_newlines,
    rules::{Lines, Text},
};
use crate::combat::text::localise;
use installed_content::localisation::Argument as Arg;
use installed_content::{
    equipment::{Database, Groups},
    localisation::Localisation,
    parser,
    raids::RaidType,
};

/// A stored name as the port's text, or why it is not.
fn stored<'a>(what: &str, bytes: &'a [u8]) -> Result<&'a str, String> {
    parser::text(bytes).map_err(|refused| format!("{what}: {refused}"))
}

/// 1409FEBA0 over a member list, in authored order: a member naming a group
/// ([`Groups::group`]: its token, found without case) shows as the group's
/// key (its description, else its name) localised when that key has
/// localisation, its terminal members still marked seen (1409FE360, which
/// does not format them); else as the group's own members, expanded the
/// same way. Any other member shows localised the first time it is seen,
/// compared without case (compare_name 14253D9F8). Entries join with ", ".
/// The recursion has no guard, so a group inside itself is refused by name.
fn group(
    loc: &Localisation,
    groups: &Groups,
    names: &[Vec<u8>],
    seen: &mut Vec<Vec<u8>>,
    path: &mut Vec<Vec<u8>>,
) -> Text {
    let mut out = Vec::new();
    for name in names {
        match groups.group(name)? {
            Some(inner) => {
                if path
                    .iter()
                    .any(|outer| outer.eq_ignore_ascii_case(&inner.name))
                {
                    return Err(format!(
                        "equipment group {} contains itself; 1409FEBA0 recurses into it with no guard",
                        String::from_utf8_lossy(&inner.name)
                    ));
                }
                let key = stored("equipment group key", inner.key())?;
                if loc.get(key).is_some() {
                    groups.expand_into(&inner.members, seen)?;
                    out.push(localise(loc, key, &[])?);
                } else {
                    path.push(inner.name.clone());
                    out.push(group(loc, groups, &inner.members, seen, path)?);
                    path.pop();
                }
            }
            None if !seen.iter().any(|known| known.eq_ignore_ascii_case(name)) => {
                seen.push(name.clone());
                out.push(localise(loc, stored("equipment group member", name)?, &[])?);
            }
            None => {}
        }
    }
    Ok(out.join(", "))
}

pub fn build(
    loc: &Localisation,
    equipment: &Database,
    groups: &Groups,
    kind: &RaidType,
    country_name: impl Fn(&str) -> Text,
    active_subunit: impl Fn(&str) -> Result<bool, String>,
) -> Lines {
    let mut lines = Vec::new();
    for requirement in &kind.unit_requirements {
        for req in &requirement.equipment {
            if req.types.is_empty() {
                continue;
            } // 1415790B0 IsEmpty guard.
            let text = (|| {
                let types: Vec<Vec<u8>> = req
                    .types
                    .iter()
                    .map(|name| name.as_bytes().to_vec())
                    .collect();
                let name = group(loc, groups, &types, &mut Vec::new(), &mut Vec::new())?;
                let mut text = localise(
                    loc,
                    "raid_unit_requirements_equipment_header",
                    &[
                        ("EQUIPMENT", Arg::Text(&name)),
                        (
                            "AMOUNT",
                            Arg::Text(&crate::combat::text::value_limit(loc, req.min, req.max)?),
                        ),
                    ],
                )?;
                let mut groups = String::new();
                for alternatives in &req.modules {
                    let mut modules = Vec::new();
                    for module in alternatives {
                        // GetModuleByToken (1409EB610), a null result
                        // skipped; the module's own name (+0x18) is
                        // localised for the country (140BBA630).
                        if let Some(found) = equipment.module(module) {
                            let found = stored("equipment module name", found)?;
                            modules.push(localise(
                                loc,
                                "raid_unit_requirements_module",
                                &[("NAME", Arg::Text(&country_name(found)?))],
                            )?);
                        }
                    }
                    let separator = format!(" {} ", localise(loc, "OR", &[])?); // 14295E708.
                    groups.push_str(&localise(
                        loc,
                        "raid_unit_requirements_module_group",
                        &[("LIST", Arg::Text(&modules.join(&separator)))],
                    )?);
                    ensure_newlines(&mut groups, 1);
                }
                if !groups.is_empty() {
                    ensure_newlines(&mut text, 1);
                    text.push_str(&localise(loc, "raid_unit_requirements_module_header", &[])?);
                    ensure_newlines(&mut text, 1);
                    text.push_str(&groups);
                }
                ensure_newlines(&mut text, 2);
                Ok(text)
            })();
            lines.push(("raid_unit_requirements_equipment_header".into(), text));
        }
        for req in &requirement.battalions {
            let text = (|| {
                if !active_subunit(&req.name)? {
                    return Ok(String::new());
                }
                let value = crate::combat::text::value_limit(loc, req.min, req.max)?;
                if value.is_empty() {
                    return Ok(value);
                }
                let mut text = localise(
                    loc,
                    "raid_unit_requirements_battalion",
                    &[
                        ("NAME", Arg::Text(&country_name(&req.name)?)),
                        ("LIMIT", Arg::Text(&value)),
                    ],
                )?;
                ensure_newlines(&mut text, 1);
                Ok(text)
            })();
            lines.push(("raid_unit_requirements_battalion".into(), text));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn battalion_lines_require_active_definition_and_nonempty_limit() {
        use installed_content::raids::{BattalionRequirement, UnitRequirement};
        let loc: Localisation = [
            ("value_limit_min", "$MIN$"),
            ("raid_unit_requirements_battalion", "$NAME$ $LIMIT$"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        for (active, min, expected) in [
            (false, Some(1), ""),
            (true, None, ""),
            (true, Some(1), "local unit 1\n"),
        ] {
            let kind = RaidType {
                unit_requirements: vec![UnitRequirement {
                    battalions: vec![BattalionRequirement {
                        name: "unit".into(),
                        min,
                        max: None,
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let lines = build(
                &loc,
                &Database::default(),
                &Groups::default(),
                &kind,
                |name| Ok(format!("local {name}")),
                |_| Ok(active),
            );
            assert_eq!(lines[0].1.as_deref(), Ok(expected));
            assert!(
                build(
                    &loc,
                    &Database::default(),
                    &Groups::default(),
                    &kind,
                    |name| Ok(name.into()),
                    |_| Err("missing witness".into())
                )[0]
                .1
                .is_err()
            );
        }
    }
    #[test]
    fn signed_limits_cover_every_native_branch() {
        let loc: Localisation = [
            ("value_limit_min", "min $MIN$"),
            ("value_limit_max", "max $MAX$"),
            ("value_limit_invalid", "invalid"),
            ("value_limit_exact", "exact $MIN$"),
            ("value_limit_min_max", "$MIN$..$MAX$"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        for (min, max, expected) in [
            (None, None, ""),
            (Some(-4), None, "min -4"),
            (None, Some(-2), "max -2"),
            (Some(3), Some(2), "invalid"),
            (Some(3), Some(3), "exact 3"),
            (Some(-1), Some(9), "-1..9"),
        ] {
            assert_eq!(
                crate::combat::text::value_limit(&loc, min, max).unwrap(),
                expected
            );
        }
    }
    /// Groups through the database's load (140165060): a group found by its
    /// token without case, its description as its key, terminal members
    /// deduplicated without case, and a group inside itself refused by
    /// name; a module shows by its own name, found by token (1409EB610).
    #[test]
    fn group_expansion_preserves_authored_order_deduplication_and_named_failures() {
        let loc: Localisation = [
            ("a", "A"),
            ("b", "B"),
            ("named", "GROUP"),
            ("raid_unit_requirements_equipment_header", "$EQUIPMENT$"),
            ("raid_unit_requirements_module", "$NAME$"),
            ("raid_unit_requirements_module_group", "$LIST$"),
            ("raid_unit_requirements_module_header", ":"),
            ("OR", "or"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let (equipment, groups) = installed_content::equipment::fixture::load(&[
            (
                "common/units/equipment/modules/m.txt",
                "equipment_modules = { Gun_1 = { } }",
            ),
            (
                "common/equipment_groups/g.txt",
                "inner = { equipment_type = { b a } }
                 named = { equipment_type = { a unlocalised_child } }
                 described = { description = named equipment_type = { b } }
                 cycle = { equipment_type = { outer } }
                 outer = { equipment_type = { CYCLE } }",
            ),
        ]);
        let run = |names: &[&str]| {
            group(
                &loc,
                &groups,
                &names
                    .iter()
                    .map(|name| name.as_bytes().to_vec())
                    .collect::<Vec<_>>(),
                &mut Vec::new(),
                &mut Vec::new(),
            )
        };
        assert_eq!(run(&["inner", "a"]).unwrap(), "B, A");
        assert_eq!(run(&["INNER", "A"]).unwrap(), "B, A");
        assert_eq!(run(&["named", "a", "b"]).unwrap(), "GROUP, B");
        assert_eq!(run(&["described", "b"]).unwrap(), "GROUP");
        let cycle = run(&["cycle"]).unwrap_err();
        assert!(
            cycle.contains("equipment group cycle contains itself"),
            "{cycle}"
        );
        let kind = RaidType {
            unit_requirements: vec![installed_content::raids::UnitRequirement {
                equipment: vec![installed_content::raids::EquipmentRequirement {
                    types: vec!["a".into()],
                    modules: vec![vec!["GUN_1".into(), "missing".into()]],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let lines = build(
            &loc,
            &equipment,
            &groups,
            &kind,
            |name| Ok(format!("local {name}")),
            |_| Ok(true),
        );
        assert_eq!(lines[0].1.as_deref(), Ok("A\n:\nlocal Gun_1\n\n"));
    }
}
