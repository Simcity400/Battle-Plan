//! Region assignment feedback from this view's selection and simulation operands.
use super::{Arg, Context, keyed};
use replay_data::{rows::AirWingRow, rows::tables};
use viewer_foundation::map::{
    air_range::{self, FrameSources},
    air_region_reach::{Origin, RegionGeometry},
    air_registry_sources::AirRegistrySources,
};

pub(super) fn text(context: &Context<'_>, region: u32, selected: &[u64]) -> Result<String, String> {
    if selected.is_empty() {
        return Ok(String::new());
    }
    let wings = context
        .frame
        .complete::<tables::AirWings>()
        .map_err(|e| e.to_string())?;
    let registry = AirRegistrySources::read(context.frame)
        .map_err(|e| e.to_string())?
        .ok_or("selected Air assignment base registry is incomplete")?;
    let first = keyed(wings, selected[0], |wing| wing.id)
        .ok_or("first selected Air wing is absent from AirWings")?;
    let kind = registry
        .bases
        .get(&first.air_base)
        .ok_or("first selected Air wing has an unresolved pool base")?
        .kind;
    let sources = FrameSources::new(context.frame);
    let mut counts = [0_i32; 6];
    for &id in selected {
        let Some(wing) = keyed(wings, id, |wing| wing.id) else {
            continue;
        };
        let reason = reason(context, wing, &registry, || {
            let members = &context
                .content
                .strategic_regions
                .get(&region)
                .ok_or("selected Air assignment region content is missing")?
                .provinces;
            let range = air_range::wing_range(
                wing,
                *context
                    .content
                    .province_hover
                    .air_range
                    .as_ref()
                    .map_err(Clone::clone)?,
                &sources,
            )?;
            let Some(base) = range_base(wing, &registry) else {
                return Ok(false);
            };
            let origin = match host(context, base.id, &registry)? {
                Some(force) => Some(Origin::Host(
                    context
                        .reads
                        .geometry
                        .centroids
                        .get(&force.province)
                        .copied()
                        .unwrap_or([0; 2]),
                )),
                None => {
                    base.state
                        .map(|state| {
                            let members =
                                context.content.province_hover.members.get(&state).ok_or(
                                    "selected Air assignment base state content is missing",
                                )?;
                            Ok::<_, String>(Origin::State(
                                context
                                    .reads
                                    .geometry
                                    .state_centre(members)
                                    .unwrap_or([0; 2]),
                            ))
                        })
                        .transpose()?
                }
            };
            Ok(origin.is_some_and(|origin| {
                RegionGeometry::new(context.reads.geometry, members).reaches(
                    context.reads.geometry,
                    members,
                    origin,
                    range,
                )
            }))
        })?;
        counts[reason] = counts[reason].wrapping_add(1);
    }
    feedback(kind, selected.len(), counts, |key, args| {
        context.loc(key, args)
    })
}

/// Assignment restrictions and wording use the pool base. The subsequent
/// reach query resolves a live transfer override before falling back to it.
fn range_base<'a>(
    wing: &AirWingRow,
    registry: &'a AirRegistrySources,
) -> Option<&'a replay_data::rows::AirBaseRow> {
    (wing.base_override != 0)
        .then(|| registry.bases.get(&wing.base_override))
        .flatten()
        .or_else(|| registry.bases.get(&wing.air_base))
}

fn host<'a>(
    context: &Context<'a>,
    base: u64,
    registry: &AirRegistrySources,
) -> Result<Option<&'a replay_data::rows::TaskForceRow>, String> {
    if registry.hosts_without_task_force.contains(&base) {
        return Err("selected Air assignment host has a null task force".into());
    }
    if !registry.host_owners.contains_key(&base) {
        return Ok(None);
    }
    let base = &registry.bases[&base];
    let ships = context
        .frame
        .complete::<tables::Ships>()
        .map_err(|e| e.to_string())?;
    let ship = keyed(ships, base.host, |row| row.id)
        .ok_or("selected Air assignment resolved host is absent from Ships")?;
    let forces = context
        .frame
        .complete::<tables::TaskForces>()
        .map_err(|e| e.to_string())?;
    keyed(forces, ship.task_force, |row| row.id)
        .map(Some)
        .ok_or_else(|| "selected Air assignment host task force is absent".into())
}

fn restriction(
    base_live: bool,
    carrier_mission: impl FnOnce() -> Result<bool, String>,
    raid: impl FnOnce() -> Result<bool, String>,
    training: bool,
    reaches: impl FnOnce() -> Result<bool, String>,
) -> Result<usize, String> {
    if !base_live {
        return Ok(5);
    }
    if carrier_mission()? {
        return Ok(2);
    }
    if raid()? {
        return Ok(4);
    }
    if training {
        return Ok(3);
    }
    Ok(if reaches()? { 0 } else { 1 })
}

fn reason(
    context: &Context<'_>,
    wing: &AirWingRow,
    registry: &AirRegistrySources,
    reaches: impl FnOnce() -> Result<bool, String>,
) -> Result<usize, String> {
    restriction(
        wing.air_base != 0 && registry.bases.contains_key(&wing.air_base),
        || Ok(host(context, wing.air_base, registry)?.is_some_and(|force| force.mission != 0)),
        || {
            wing.assigned_to_raid()
                .ok_or_else(|| "selected Air assignment raid liveness is unavailable".into())
        },
        wing.mission_mask & (1 << 11) != 0,
        reaches,
    )
}

fn feedback(
    kind: i32,
    selected: usize,
    counts: [i32; 6],
    mut localise: impl FnMut(&str, &[(&str, Arg<'_>)]) -> Result<String, String>,
) -> Result<String, String> {
    if selected == 0 {
        return Ok(String::new());
    }
    let prefix = match kind {
        0 => "",
        1 => "ROCKET_",
        2 => "GUN_EMPLACEMENT_",
        _ => {
            return Err(format!(
                "selected Air assignment base kind {kind} is unsupported"
            ));
        }
    };
    let mut text = localise(&format!("{prefix}AIR_REGION_RIGHT_CLICK"), &[])? + "\n";
    let accepted = counts[0];
    if accepted == 0 || (accepted as usize) < selected {
        let suffix = if accepted == 0 { "NONE" } else { "NOT_ALL" };
        text.push_str(&localise(
            &format!("{prefix}AIR_REGION_RIGHT_CLICK_{suffix}"),
            &[],
        )?);
        text.push('\n');
        // Raid and unresolved-base rejections contribute to the partial gate but
        // deliberately have no explanation text in the native reason loop.
        for (count, key) in counts[1..4].iter().zip([
            "AIR_REGION_RIGHT_CLICK_REASON_NO_COVERAGE",
            "AIR_REGION_RIGHT_CLICK_REASON_CARRIER_ON_MISSION",
            "AIR_REGION_RIGHT_CLICK_REASON_TRAINING_MISSION",
        ]) {
            if *count > 0 {
                text.push_str(&localise(key, &[("COUNT", Arg::Int32(*count))])?);
                text.push('\n');
            }
        }
        text.push('\n');
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(kind: i32, selected: usize, counts: [i32; 6]) -> String {
        feedback(kind, selected, counts, |key, _| Ok(key.to_owned())).unwrap()
    }

    #[test]
    fn subtype_headers_partial_reasons_and_unlabelled_rejections_keep_native_order() {
        assert_eq!(render(0, 0, [0; 6]), "");
        assert_eq!(
            render(1, 2, [2, 0, 0, 0, 0, 0]),
            "ROCKET_AIR_REGION_RIGHT_CLICK\n"
        );
        assert_eq!(
            render(2, 6, [1, 1, 1, 1, 1, 1]),
            concat!(
                "GUN_EMPLACEMENT_AIR_REGION_RIGHT_CLICK\n",
                "GUN_EMPLACEMENT_AIR_REGION_RIGHT_CLICK_NOT_ALL\n",
                "AIR_REGION_RIGHT_CLICK_REASON_NO_COVERAGE\n",
                "AIR_REGION_RIGHT_CLICK_REASON_CARRIER_ON_MISSION\n",
                "AIR_REGION_RIGHT_CLICK_REASON_TRAINING_MISSION\n\n",
            )
        );
        assert_eq!(
            render(0, 2, [0, 0, 0, 0, 1, 1]),
            "AIR_REGION_RIGHT_CLICK\nAIR_REGION_RIGHT_CLICK_NONE\n\n"
        );
    }

    #[test]
    fn restriction_order_demands_only_the_reached_simulation_predicates() {
        let fail = || Err("unreached".into());
        assert_eq!(restriction(false, fail, fail, true, fail).unwrap(), 5);
        assert_eq!(restriction(true, || Ok(true), fail, true, fail).unwrap(), 2);
        assert_eq!(
            restriction(true, || Ok(false), || Ok(true), true, fail).unwrap(),
            4
        );
        assert_eq!(
            restriction(true, || Ok(false), || Ok(false), true, fail).unwrap(),
            3
        );
        assert_eq!(
            restriction(true, || Ok(false), || Ok(false), false, || Ok(false)).unwrap(),
            1
        );
        assert_eq!(
            restriction(true, || Ok(false), || Ok(false), false, || Ok(true)).unwrap(),
            0
        );
        assert!(restriction(true, fail, || Ok(false), false, || Ok(true)).is_err());
    }

    #[test]
    fn range_uses_a_live_transfer_override_and_falls_back_only_when_it_does_not_resolve() {
        let mut registry = AirRegistrySources::default();
        for id in [10, 20] {
            registry.bases.insert(
                id,
                replay_data::rows::AirBaseRow {
                    id,
                    ..Default::default()
                },
            );
        }
        let mut wing = AirWingRow {
            air_base: 10,
            base_override: 20,
            ..Default::default()
        };
        assert_eq!(range_base(&wing, &registry).unwrap().id, 20);
        wing.base_override = 30;
        assert_eq!(range_base(&wing, &registry).unwrap().id, 10);
        wing.air_base = 30;
        assert!(range_base(&wing, &registry).is_none());
    }

    #[test]
    fn installed_reason_arguments_keep_signed_counts_and_ordinary_prefixes() {
        let loc: installed_content_localisation::localisation::Localisation = [
            ("AIR_REGION_RIGHT_CLICK", "Assign"),
            ("AIR_REGION_RIGHT_CLICK_NOT_ALL", "Some blocked"),
            (
                "AIR_REGION_RIGHT_CLICK_REASON_NO_COVERAGE",
                "$COUNT$ beyond range",
            ),
            (
                "AIR_REGION_RIGHT_CLICK_REASON_TRAINING_MISSION",
                "$COUNT$ training",
            ),
        ]
        .into_iter()
        .map(|(key, text)| (key.into(), text.into()))
        .collect();
        let text = feedback(0, 6, [1, 2, 0, 3, 0, 0], |key, args| {
            loc.localise_with(key.as_bytes(), args)
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(text, "Assign\nSome blocked\n2 beyond range\n3 training\n\n");
    }
}
