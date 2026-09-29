//! Unit requirement text: 141618D30 -> 141579E40 -> 1415790B0/141578C80.
use super::{
    ensure_newlines,
    rules::{Lines, Text},
    text::{Arg, localise},
};
use installed_content::{equipment::Equipment, localisation::Localisation, raids::RaidType};
use std::collections::BTreeSet;

/// 141580360 preserves signed integer limits, including an invalid interval.
pub fn limit(loc: &Localisation, min: Option<i32>, max: Option<i32>) -> Text {
    let key = match (min, max) {
        (None, None) => return Ok(String::new()),
        (Some(_), None) => "value_limit_min",
        (None, Some(_)) => "value_limit_max",
        (Some(a), Some(b)) if b < a => "value_limit_invalid",
        (Some(a), Some(b)) if a == b => "value_limit_exact",
        _ => "value_limit_min_max",
    };
    let mut args = Vec::new();
    if let Some(n) = min {
        args.push(("MIN", Arg::Integer(n)));
    }
    if let Some(n) = max {
        args.push(("MAX", Arg::Integer(n)));
    }
    localise(loc, key, &args)
}

/// 1409FEBA0: named groups localise as a whole when possible, otherwise
/// expand in authored order; terminal members are deduplicated globally.
fn group(
    loc: &Localisation,
    equipment: &Equipment,
    names: &[String],
    seen: &mut BTreeSet<String>,
    path: &mut BTreeSet<String>,
) -> Text {
    let mut out = Vec::new();
    for name in names {
        if let Some(reason) = equipment.unsupported_groups.get(name) {
            return Err(format!("equipment group {name}: unsupported {reason}"));
        }
        if let Some(members) = equipment.groups.get(name) {
            if !path.insert(name.clone()) {
                return Err(format!("equipment group cycle at {name}"));
            }
            if loc.get(name).is_some() {
                // The native localised-group arm still visits its terminal
                // members to prevent them appearing again later.
                collect_members(equipment, members, seen, path)?;
                out.push(localise(loc, name, &[])?);
            } else {
                out.push(group(loc, equipment, members, seen, path)?);
            }
            path.remove(name);
        } else if seen.insert(name.clone()) {
            out.push(localise(loc, name, &[])?);
        }
    }
    Ok(out.join(", "))
}

// Native CollectExpandedMembers does not format or localise the children.
fn collect_members(
    equipment: &Equipment,
    names: &[String],
    seen: &mut BTreeSet<String>,
    path: &mut BTreeSet<String>,
) -> Result<(), String> {
    for name in names {
        if let Some(reason) = equipment.unsupported_groups.get(name) {
            return Err(format!("equipment group {name}: unsupported {reason}"));
        }
        if let Some(members) = equipment.groups.get(name) {
            if !path.insert(name.clone()) {
                return Err(format!("equipment group cycle at {name}"));
            }
            collect_members(equipment, members, seen, path)?;
            path.remove(name);
        } else {
            seen.insert(name.clone());
        }
    }
    Ok(())
}

pub fn build(
    loc: &Localisation,
    equipment: &Equipment,
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
                let name = group(
                    loc,
                    equipment,
                    &req.types,
                    &mut BTreeSet::new(),
                    &mut BTreeSet::new(),
                )?;
                let mut text = localise(
                    loc,
                    "raid_unit_requirements_equipment_header",
                    &[
                        ("EQUIPMENT", Arg::Text(name)),
                        ("AMOUNT", Arg::Text(limit(loc, req.min, req.max)?)),
                    ],
                )?;
                let mut groups = String::new();
                for alternatives in &req.modules {
                    let mut modules = Vec::new();
                    for module in alternatives {
                        if equipment.refused_modules.contains(module) {
                            return Err(format!(
                                "module {module} has an unsupported duplicate definition"
                            ));
                        }
                        // Native resolves the module definition, skipping a null result.
                        if equipment.modules.contains_key(module) {
                            modules.push(localise(
                                loc,
                                "raid_unit_requirements_module",
                                &[("NAME", Arg::Text(country_name(module)?))],
                            )?);
                        }
                    }
                    let separator = format!(" {} ", localise(loc, "OR", &[])?); // 14295E708.
                    groups.push_str(&localise(
                        loc,
                        "raid_unit_requirements_module_group",
                        &[("LIST", Arg::Text(modules.join(&separator)))],
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
                let value = limit(loc, req.min, req.max)?;
                if value.is_empty() {
                    return Ok(value);
                }
                let mut text = localise(
                    loc,
                    "raid_unit_requirements_battalion",
                    &[
                        ("NAME", Arg::Text(country_name(&req.name)?)),
                        ("LIMIT", Arg::Text(value)),
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
                &Equipment::default(),
                &kind,
                |name| Ok(format!("local {name}")),
                |_| Ok(active),
            );
            assert_eq!(lines[0].1.as_deref(), Ok(expected));
            assert!(
                build(
                    &loc,
                    &Equipment::default(),
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
            assert_eq!(limit(&loc, min, max).unwrap(), expected);
        }
    }
    #[test]
    fn group_expansion_preserves_authored_order_deduplication_and_named_failures() {
        let loc: Localisation = [("a", "A"), ("b", "B"), ("named", "GROUP")]
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let mut equipment = Equipment::default();
        equipment
            .groups
            .insert("inner".into(), vec!["b".into(), "a".into()]);
        equipment
            .groups
            .insert("named".into(), vec!["a".into(), "unlocalised_child".into()]);
        let run = |e: &Equipment, names: &[&str]| {
            group(
                &loc,
                e,
                &names.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
                &mut BTreeSet::new(),
                &mut BTreeSet::new(),
            )
        };
        assert_eq!(run(&equipment, &["inner", "a"]).unwrap(), "B, A");
        assert_eq!(run(&equipment, &["named", "a", "b"]).unwrap(), "GROUP, B");
        equipment
            .groups
            .insert("cycle".into(), vec!["cycle".into()]);
        assert!(run(&equipment, &["cycle"]).unwrap_err().contains("cycle"));
        equipment
            .unsupported_groups
            .insert("bad".into(), "field".into());
        assert!(run(&equipment, &["bad"]).unwrap_err().contains("bad"));
    }
}
