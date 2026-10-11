//! Battalion composition from the hourly template companion's grouped grids.
//! The sub-unit's authored group survives template copying; no template
//! representative enters this calculation. Placed regiments use their grid's
//! group; the shared database's air/naval inventory does not establish army reach.
use super::*;
use crate::catalogue::{Catalogue, catalogue};
use installed_content::unit_leader::SubUnits;

#[derive(Clone, Debug)]
pub(super) struct RawMixRun {
    pub first: i32,
    pub last: i32,
    pub parts: [Option<BTreeMap<String, i64>>; 2],
}

#[derive(Default)]
struct Groups {
    content: Option<(Arc<Catalogue>, Result<SubUnits, String>)>,
}

fn group_info(
    units: Option<&SubUnits>,
    catalogue: Option<&Catalogue>,
    name: &str,
) -> (String, ArmyType, Option<String>) {
    let group = units.map_or_else(
        || Err("game content is unavailable".into()),
        |units| units.group(name.as_bytes()),
    );
    let (key, label, mut warning) = match group {
        Ok(Some(group)) => {
            let identity = match installed_content::tokens::Token::c_str(group.as_bytes()) {
                installed_content::tokens::Token::Name(name) => {
                    String::from_utf8(name).unwrap_or_else(|_| group.to_owned())
                }
                installed_content::tokens::Token::Rejected => String::new(),
            };
            (format!("group:{identity}"), group, None)
        }
        // An omitted authored group is a known token-zero default.
        Ok(None) => ("group:".into(), "", None),
        Err(why) => (
            format!("unknown:{name}"),
            name,
            Some(format!("{name}: unknown unit group: {why}")),
        ),
    };
    // The authored group remains the identity. Presentation resolves that
    // exact content key, without inventing category prefixes or suffixes.
    let localised = if label.is_empty() {
        Some(Ok(String::new()))
    } else {
        catalogue
            .filter(|catalogue| catalogue.localisation.get(label).is_some())
            .map(|catalogue| {
                crate::catalogue::localised(
                    &catalogue.localisation,
                    label,
                    crate::catalogue::View::Plain,
                )
            })
    };
    let label = match localised {
        Some(Ok(label)) => label,
        result => {
            let reason = result
                .and_then(Result::err)
                .unwrap_or_else(|| "localized title is absent from the recorded language".into());
            if warning.is_none() {
                warning = Some(format!("{key}: {reason}"));
            }
            "Unit group name unavailable".into()
        }
    };
    (key.clone(), ArmyType { key, name: label }, warning)
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keeps independent scan, window, content, output and cancellation inputs explicit"
)]
pub(super) fn composition_reply(
    source: &Source,
    context: &LoadContext,
    scan: &Scan,
    params: &ArmySizeParams,
    range: &HourSpan,
    series: &mut [ArmySeries],
    warnings: &mut Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<ArmyType>> {
    let catalogue = match context
        .content
        .as_ref()
        .map(|options| catalogue(source, options))
        .transpose()
    {
        Ok(catalogue) => catalogue,
        Err(error) => {
            warnings.push(format!("Unit-group content unavailable: {error}"));
            None
        }
    };
    let state = source.state::<Groups>();
    let mut groups = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(catalogue) = &catalogue
        && groups
            .content
            .as_ref()
            .is_none_or(|(old, _)| !Arc::ptr_eq(old, catalogue))
    {
        let units = installed_content::unit_leader::sub_units(&catalogue.content.layers)
            .map_err(|error| error.to_string());
        groups.content = Some((Arc::clone(catalogue), units));
    }
    let content = catalogue
        .as_ref()
        .and_then(|_| groups.content.as_ref().map(|(_, units)| units));
    if let Some(Err(reason)) = content {
        warnings.push(format!("Unit-group reader refused: {reason}"));
    }
    let units = content.and_then(|units| units.as_ref().ok());
    if let Some(units) = units {
        warnings.extend(units.warnings().iter().cloned());
    }
    let index = usize::from(params.count_for == ArmyAttribution::Host);
    let names: std::collections::BTreeSet<_> = series
        .iter()
        .filter_map(|series| scan.tracks.get(&series.tag))
        .flat_map(|track| window_mix(track, range))
        .filter_map(|run| run.parts[index].as_ref())
        .flat_map(|parts| parts.keys().cloned())
        .collect();
    let mut mapping = BTreeMap::new();
    let mut types = BTreeMap::new();
    let mut unresolved = 0;
    for name in names {
        let (key, info, warning) = group_info(units, catalogue.as_deref(), &name);
        mapping.insert(name, key.clone());
        types.insert(key, info);
        if let Some(warning) = warning {
            unresolved += 1;
            warnings.push(warning);
        }
    }
    if unresolved > 0 {
        warnings.push(format!("{unresolved} sub-unit group/name lookups are unresolved or refused; battalions remain counted."));
    }
    let types: Vec<_> = types.into_values().collect();
    let positions: HashMap<_, _> = types
        .iter()
        .enumerate()
        .map(|(index, info)| (&info.key, index))
        .collect();
    for series in series {
        let Some(track) = scan.tracks.get(&series.tag) else {
            continue;
        };
        let spans: Vec<_> = series
            .runs
            .iter()
            .map(|run| HourSpan {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
            })
            .collect();
        let mut mix: Vec<ArmyMixRun> = Vec::new();
        let mut visit = |run: &RawMixRun, first, last| -> io::Result<()> {
            let Some(parts) = &run.parts[index] else {
                return Ok(());
            };
            let mut values = vec![Some(0i64); types.len()];
            for (name, count) in parts {
                add(&mut values[positions[&mapping[name]]], *count);
            }
            let raw_values = Some(
                values
                    .iter()
                    .map(|raw| raw.map(|raw| raw.to_string()))
                    .collect::<Vec<_>>(),
            );
            let values: Vec<_> = values
                .into_iter()
                .map(|raw| amount(raw.map(i128::from), ArmyMeasure::Battalions))
                .collect();
            if let Some(previous) = mix.last_mut()
                && previous.last_hour.checked_add(1) == Some(first)
                && previous.raw_values == raw_values
                && track.segments.iter().any(|segment| {
                    segment.first_hour <= previous.first_hour && last <= segment.last_hour
                })
            {
                previous.last_hour = last;
            } else {
                mix.push(ArmyMixRun {
                    first_hour: first,
                    last_hour: last,
                    values,
                    raw_values,
                });
            }
            Ok(())
        };
        crate::ranking::for_each_piece(
            window_mix(track, range),
            &spans,
            |run| (run.first, run.last),
            cancelled,
            &mut visit,
        )?;
        series.mix = (!mix.is_empty()).then_some(mix);
    }
    Ok(types)
}

pub(crate) fn check_content(
    catalogue: &Catalogue,
) -> io::Result<Vec<crate::content_check::Finding>> {
    use crate::content_check::{Finding, Kind};
    let units = installed_content::unit_leader::sub_units(&catalogue.content.layers)?;
    let mut findings: Vec<_> = units
        .warnings()
        .iter()
        .map(|message| Finding::new(Kind::Refused, "unit group reader", message))
        .collect();
    for name in units.registered() {
        match std::str::from_utf8(name) {
            Ok(name) => {
                let (_, _, warning) = group_info(Some(&units), Some(catalogue), name);
                if let Some(reason) = warning {
                    findings.push(Finding::new(Kind::Refused, name, reason));
                }
            }
            Err(_) => findings.push(Finding::new(
                Kind::Refused,
                name.escape_ascii().to_string(),
                "Sub-unit name is not UTF-8",
            )),
        }
    }
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn army_size_grouping_uses_authored_groups_and_keeps_unknown_names() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let mut units = SubUnits::default();
        units.read(
            b"sub_units = { foot = { group = mod_group } \
            support = { group = mod_group } orphan = {} }",
            "synthetic",
        );
        assert_eq!(group_info(Some(&units), None, "foot").0, "group:mod_group");
        assert_eq!(
            group_info(Some(&units), None, "support").0,
            "group:mod_group"
        );
        let resolved = group_info(Some(&units), None, "foot");
        assert_eq!(resolved.1.name, "Unit group name unavailable");
        assert!(resolved.2.is_some());
        let default = group_info(Some(&units), None, "orphan");
        assert_eq!(default.0, "group:");
        assert_eq!(default.1.name, "");
        assert_eq!(default.2, None);
        assert_eq!(
            group_info(Some(&units), None, "missing").0,
            "unknown:missing"
        );
    }

    #[test]
    fn army_group_titles_resolve_the_content_key_and_preserve_defined_empty() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
            "localisation/groups_l_english.yml",
            "\u{feff}l_english:\n mod_group:0 \"Mod battalions\"\n empty_group:0 \"\"\n",
        )]);
        let mut units = SubUnits::default();
        units.read(b"sub_units={foot={group=mod_group} support={group=empty_group} missing={group=missing_group}}", "synthetic");
        let named = group_info(Some(&units), Some(&catalogue), "foot");
        assert_eq!(named.0, "group:mod_group");
        assert_eq!(named.1.name, "Mod battalions");
        assert!(named.2.is_none());
        assert_eq!(
            group_info(Some(&units), Some(&catalogue), "support").1.name,
            ""
        );
        let missing = group_info(Some(&units), Some(&catalogue), "missing");
        assert_eq!(missing.0, "group:missing_group");
        assert_eq!(missing.1.name, "Unit group name unavailable");
        assert!(missing.2.is_some());
    }

    #[test]
    fn army_size_content_accepts_native_default_groups_without_claiming_army_reach() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/units/mod.txt",
                "sub_units={foot={group=mod_group} ungrouped={} air={} naval={}}",
            ),
            (
                "localisation/groups_l_english.yml",
                "\u{feff}l_english:\n mod_group:0 \"Authored group\"\n",
            ),
        ]);
        assert!(check_content(&catalogue).unwrap().is_empty());
    }
}
