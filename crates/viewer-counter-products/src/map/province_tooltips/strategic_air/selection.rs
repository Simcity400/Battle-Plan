use super::{Arg, Context, keyed};
use replay_data::{rows::AirWingRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use viewer_unit_reads::{
    air_range::{self, FrameSources},
    air_region_reach::{Origin, RegionGeometry},
    air_registry_sources::AirRegistrySources,
    movement_reads::ReadOperands,
};

const ROCKET_TYPES: u32 = 0x10000;
const GUN_EMPLACEMENT_TYPES: u32 = 0x20000;
const REFUSED_TYPES: u32 = 0x20;
const SITE_FEATURE: u32 = 1 << 13;

type Counters = super::super::counter_content::Content;

struct Assigning<'a> {
    content: &'a Counters,
    reads: ReadOperands<'a>,
    frame: &'a DecodedFrame,
    registry: AirRegistrySources,
    sources: FrameSources<'a>,
}

impl<'a> Assigning<'a> {
    fn new(content: &'a Counters, reads: ReadOperands<'a>, frame: &'a DecodedFrame) -> Result<Self, String> {
        let registry = AirRegistrySources::read(frame)
            .map_err(|e| e.to_string())?
            .ok_or("selected Air assignment base registry is incomplete")?;
        Ok(Self { content, reads, frame, registry, sources: FrameSources::new(frame) })
    }

    fn reason(&self, wing: &AirWingRow, region: u32) -> Result<usize, String> {
        reason(self.frame, wing, &self.registry, || {
            let members = &self
                .content
                .strategic_regions
                .get(&region)
                .ok_or("selected Air assignment region content is missing")?
                .provinces;
            let range = air_range::wing_range(
                wing,
                *self.content.province_hover.air_range.as_ref().map_err(Clone::clone)?,
                &self.sources,
            )?;
            let Some(base) = range_base(wing, &self.registry) else {
                return Ok(false);
            };
            let geometry = self.reads.geometry;
            let origin = match host(self.frame, base.id, &self.registry)? {
                Some(force) => Some(Origin::Host(geometry.centroids.get(&force.province).copied().unwrap_or([0; 2]))),
                None => base
                    .state
                    .map(|state| {
                        let members = self
                            .content
                            .province_hover
                            .members
                            .get(&state)
                            .ok_or("selected Air assignment base state content is missing")?;
                        Ok::<_, String>(Origin::State(geometry.state_centre(members).unwrap_or([0; 2])))
                    })
                    .transpose()?,
            };
            Ok(origin.is_some_and(|origin| {
                RegionGeometry::new(geometry, members).reaches(geometry, members, origin, range)
            }))
        })
    }

    fn accepts(&self, wing: &AirWingRow, region: u32) -> Result<bool, String> {
        let feature = || -> Result<bool, String> {
            let mask = self
                .frame
                .complete::<tables::SimulationFeatures>()
                .map_err(|e| e.to_string())?
                .first()
                .ok_or("simulation features absent")?
                .dlc_mask;
            Ok(mask & SITE_FEATURE != 0)
        };
        accepted(
            mission_types(self.frame, wing)?,
            region,
            feature,
            || {
                self.registry
                    .bases
                    .get(&wing.air_base)
                    .map(|base| site_region(self.content, self.frame, &self.registry, base))
                    .transpose()
            },
            || self.reason(wing, region),
            || super::pool_region(self.content, self.frame, &self.registry, wing.air_base).map(|(at, _)| at),
        )
    }
}

fn accepted(
    types: u32,
    region: u32,
    feature: impl Fn() -> Result<bool, String>,
    site: impl FnOnce() -> Result<Option<u32>, String>,
    reason: impl FnOnce() -> Result<usize, String>,
    pool: impl FnOnce() -> Result<u32, String>,
) -> Result<bool, String> {
    match types {
        ROCKET_TYPES if !feature()? => Ok(false),
        GUN_EMPLACEMENT_TYPES => Ok(feature()? && site()? == Some(region)),
        REFUSED_TYPES => Ok(false),
        _ => match reason()? {
            2 => Ok(pool()? == region),
            reason => Ok(reason == 0),
        },
    }
}

fn mission_types(frame: &DecodedFrame, wing: &AirWingRow) -> Result<u32, String> {
    let variant = frame
        .complete::<tables::WingEquipment>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| row.wing == wing.id)
        .min_by_key(|row| row.ordinal)
        .ok_or_else(|| format!("wing {} holds no equipment, so its mission types are unknown", wing.id))?
        .variant;
    let variants = frame.complete::<tables::VariantDefinitions>().map_err(|e| e.to_string())?;
    keyed(variants, variant, |row| row.id)
        .map(|row| row.mission_mask)
        .ok_or_else(|| format!("wing {}'s equipment variant is absent from VariantDefinitions", wing.id))
}

fn site_region(
    content: &Counters,
    frame: &DecodedFrame,
    registry: &AirRegistrySources,
    base: &replay_data::rows::AirBaseRow,
) -> Result<u32, String> {
    match base.state {
        Some(state) => {
            let first = content
                .province_hover
                .members
                .get(&state)
                .and_then(|members| members.first())
                .ok_or("a gun emplacement's state has no province in the content")?;
            content.region_of.get(first).copied().ok_or_else(|| "a gun emplacement's state region is unavailable".into())
        }
        None => super::pool_region(content, frame, registry, base.id).map(|(region, _)| region),
    }
}

pub fn assignment_checks(
    content: &Counters,
    reads: ReadOperands<'_>,
    frame: &DecodedFrame,
    region: u32,
    selected: &[u64],
) -> Vec<Result<bool, String>> {
    let checked = (|| {
        let assigning = Assigning::new(content, reads, frame)?;
        let wings = frame.complete::<tables::AirWings>().map_err(|e| e.to_string())?;
        Ok::<_, String>(
            selected
                .iter()
                .map(|&id| {
                    let wing = keyed(wings, id, |wing| wing.id).ok_or_else(|| format!("selected wing {id} is absent from AirWings"))?;
                    assigning.accepts(wing, region)
                })
                .collect(),
        )
    })();
    checked.unwrap_or_else(|why| vec![Err(why); selected.len()])
}

pub(super) fn text(context: &Context<'_>, region: u32, selected: &[u64]) -> Result<String, String> {
    if selected.is_empty() {
        return Ok(String::new());
    }
    let wings = context
        .frame
        .complete::<tables::AirWings>()
        .map_err(|e| e.to_string())?;
    let assigning = Assigning::new(context.content, context.reads, context.frame)?;
    let first = keyed(wings, selected[0], |wing| wing.id)
        .ok_or("first selected Air wing is absent from AirWings")?;
    let kind = assigning
        .registry
        .bases
        .get(&first.air_base)
        .ok_or("first selected Air wing has an unresolved pool base")?
        .kind;
    let mut counts = [0_i32; 6];
    for &id in selected {
        let Some(wing) = keyed(wings, id, |wing| wing.id) else {
            continue;
        };
        let reason = assigning.reason(wing, region)?;
        counts[reason] = counts[reason].wrapping_add(1);
    }
    feedback(kind, selected.len(), counts, |key, args| {
        context.loc(key, args)
    })
}

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
    frame: &'a DecodedFrame,
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
    let ships = frame
        .complete::<tables::Ships>()
        .map_err(|e| e.to_string())?;
    let ship = keyed(ships, base.host, |row| row.id)
        .ok_or("selected Air assignment resolved host is absent from Ships")?;
    let forces = frame
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
    frame: &DecodedFrame,
    wing: &AirWingRow,
    registry: &AirRegistrySources,
    reaches: impl FnOnce() -> Result<bool, String>,
) -> Result<usize, String> {
    restriction(
        wing.air_base != 0 && registry.bases.contains_key(&wing.air_base),
        || Ok(host(frame, wing.air_base, registry)?.is_some_and(|force| force.mission != 0)),
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

    #[test]
    fn the_assignment_check_follows_each_wing_kind() {
        let fail = || -> Result<bool, String> { Err("unread".into()) };
        let no_site = || -> Result<Option<u32>, String> { Err("unread".into()) };
        let no_reason = || -> Result<usize, String> { Err("unread".into()) };
        let no_pool = || -> Result<u32, String> { Err("unread".into()) };
        assert_eq!(accepted(0x1, 7, fail, no_site, || Ok(0), no_pool), Ok(true));
        assert_eq!(accepted(0x1, 7, fail, no_site, || Ok(1), no_pool), Ok(false));
        assert_eq!(accepted(0x1, 7, fail, no_site, || Ok(2), || Ok(7)), Ok(true));
        assert_eq!(accepted(0x1, 7, fail, no_site, || Ok(2), || Ok(0xaa)), Ok(false));
        assert_eq!(accepted(ROCKET_TYPES, 7, || Ok(false), no_site, no_reason, no_pool), Ok(false));
        assert_eq!(accepted(ROCKET_TYPES, 7, || Ok(true), no_site, || Ok(0), no_pool), Ok(true));
        assert_eq!(accepted(GUN_EMPLACEMENT_TYPES, 7, || Ok(false), no_site, no_reason, no_pool), Ok(false));
        assert_eq!(accepted(GUN_EMPLACEMENT_TYPES, 7, || Ok(true), || Ok(Some(7)), no_reason, no_pool), Ok(true));
        assert_eq!(accepted(GUN_EMPLACEMENT_TYPES, 7, || Ok(true), || Ok(Some(8)), no_reason, no_pool), Ok(false));
        assert_eq!(accepted(GUN_EMPLACEMENT_TYPES, 7, || Ok(true), || Ok(None), no_reason, no_pool), Ok(false));
        assert_eq!(accepted(REFUSED_TYPES, 7, fail, no_site, no_reason, no_pool), Ok(false));
    }

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
        let loc: content_reader_localisation::localisation::Localisation = [
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
