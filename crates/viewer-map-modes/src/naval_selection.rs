use super::{air_assignment, map_mode};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet};
use viewer_runtime::unit_handle::Unit;

#[derive(Clone, Copy)]
pub struct Colors {
    neutral: [f32; 4],
    active: [f32; 4],
    hold: [f32; 4],
    fade: f32,
}
impl Colors {
    pub fn read(defines: &content_reader_defines::defines::Defines) -> std::io::Result<Self> {
        let read = |name| -> std::io::Result<[f32; 4]> {
            let values = defines.numbers("NGraphics", name)?;
            if values.len() < 4 {
                return Err(std::io::Error::other(format!(
                    "{name} needs four components"
                )));
            }
            Ok(std::array::from_fn(|at| values[at] as f32))
        };
        Ok(Self {
            neutral: read("STRATEGIC_NAVY_COLOR_NEUTRAL")?,
            active: read("STRATEGIC_NAVY_COLOR_NEUTRAL_HIGHLIGHTED")?,
            hold: read("STRATEGIC_NAVY_COLOR_ON_HOLD_HIGHLIGHTED")?,
            fade: defines.number("NMapMode", "NAVAL_REGION_FADE_WHEN_FLEET_SELECTED")? as f32,
        })
    }
}
pub fn naval_fraction(
    members: &[u32],
    geometry: &content_reader_map::province_geometry::ProvinceGeometry,
) -> i64 {
    let (mut total, mut sea) = (0_i32, 0_i32);
    for &province in members {
        let Some(flags) = geometry.kind_flags(province) else {
            continue;
        };
        total = total.wrapping_add(1);
        if flags & 3 == 0 {
            sea = sea.wrapping_add(1);
        }
    }
    if total < 1 {
        return 0;
    }
    i64::from(sea).wrapping_mul(10_000_000_000) / (i64::from(total) * 100_000)
}

pub fn eligible_regions(
    regions: &BTreeMap<u32, Vec<u32>>,
    geometry: &content_reader_map::province_geometry::ProvinceGeometry,
) -> BTreeSet<u32> {
    regions
        .iter()
        .filter_map(|(&region, members)| {
            (naval_fraction(members, geometry) >= 1_000).then_some(region)
        })
        .collect()
}

#[derive(Default)]
pub struct Field {
    pub assignment: air_assignment::Field,
    pub styles: BTreeMap<u32, map_display_data::landscape::Style>,
    pub refused: BTreeMap<String, usize>,
}

fn stripe(count: i32, sources: i32) -> [u8; 4] {
    let ratio = (count as f32 / sources as f32).clamp(0.0, 1.0);
    let component = |start: f32, end: f32| {
        content_reader_parser::crt::truncate_f32((ratio * (end - start) + start) * 255.0) as u8
    };
    [
        component(0.0, 0.2),
        component(0.5, 1.0),
        component(0.0, 0.2),
        47,
    ]
}

pub fn read(
    frame: &DecodedFrame,
    viewer: u32,
    selected: &BTreeSet<Unit>,
    fleets_selected: &[u64],
    regions: &BTreeMap<u32, Vec<u32>>,
    eligible: &BTreeSet<u32>,
    colors: Result<Colors, &str>,
) -> Field {
    let mut out = Field::default();
    let selected_forces: BTreeSet<_> = selected
        .iter()
        .filter_map(|unit| match unit {
            Unit::TaskForce(id) => Some(*id),
            _ => None,
        })
        .collect();
    if selected_forces.is_empty() && fleets_selected.is_empty() {
        return out;
    }
    let result = (|| -> Result<(), String> {
        let colors = colors.map_err(str::to_owned)?;
        let forces = frame
            .complete::<tables::TaskForces>()
            .map_err(|e| e.to_string())?;
        let fleet_inputs = if fleets_selected.is_empty() {
            None
        } else {
            Some((
                frame
                    .complete::<tables::Fleets>()
                    .map_err(|e| e.to_string())?,
                frame
                    .complete::<tables::FleetCounterInputs>()
                    .map_err(|e| e.to_string())?,
            ))
        };
        let missions = frame
            .complete::<tables::TaskForceMissions>()
            .map_err(|e| e.to_string())?;
        let ledgers = frame
            .complete::<tables::NavalMissionLedgers>()
            .map_err(|e| e.to_string())?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        let slot = |id| {
            links
                .binary_search_by_key(&id, |row| row.id)
                .ok()
                .map(|at| links[at].canonical_slot)
        };
        let mut counts = BTreeMap::<u32, i32>::new();
        let mut highlighted = BTreeSet::new();
        let mut sources = 0_i32;
        let mut any = false;
        for id in &selected_forces {
            if forces.binary_search_by_key(id, |row| row.id).is_err() {
                continue;
            }
            any = true;
            if let Ok(at) = missions.binary_search_by_key(id, |row| row.task_force) {
                let member = &missions[at];
                if !member.regions.is_empty() {
                    sources = sources.wrapping_add(1);
                }
                for &region in &member.regions {
                    let count = counts.entry(region).or_default();
                    *count = count.wrapping_add(1);
                    highlighted.insert(region);
                }
            }
        }
        for id in fleets_selected {
            let (fleets, counter) =
                fleet_inputs.expect("nonempty fleet selection demanded its inputs");
            if fleets.binary_search_by_key(id, |row| row.id).is_err() {
                continue;
            }
            any = true;
            let fleet = counter
                .binary_search_by_key(id, |row| row.fleet)
                .ok()
                .map(|at| &counter[at])
                .ok_or("selected fleet region membership is absent")?;
            let members = fleet
                .mission_regions
                .require("selected fleet mission regions")?
                .as_ref()
                .ok_or("selected fleet region membership is uncertified")?;
            if !members.is_empty() {
                sources = sources.wrapping_add(1);
            }
            for &region in members.iter() {
                let count = counts.entry(region).or_default();
                *count = count.wrapping_add(1);
                highlighted.insert(region);
            }
        }
        if !any {
            return Ok(());
        }
        if sources > 0 {
            for (region, count) in counts {
                let color = stripe(count, sources);
                let provinces = regions
                    .get(&region)
                    .ok_or_else(|| format!("selected naval region {region} is undefined"))?;
                out.assignment
                    .provinces
                    .extend(provinces.iter().map(|&province| (province, color)));
            }
        }
        let selected_fleets: BTreeSet<_> = fleets_selected.iter().copied().collect();
        let mut mission_highlights = BTreeMap::<u32, (bool, bool)>::new();
        for ledger in ledgers
            .iter()
            .filter(|row| slot(row.owner) == Some(viewer) && eligible.contains(&row.region))
        {
            let highlights = mission_highlights.entry(ledger.region).or_default();
            for id in &ledger.task_forces {
                let force = forces
                    .binary_search_by_key(id, |row| row.id)
                    .ok()
                    .map(|at| &forces[at])
                    .ok_or("naval mission registration has no task force")?;
                if selected_forces.contains(id) || selected_fleets.contains(&force.fleet) {
                    if force.mission & !8 == 0 {
                        highlights.0 = true;
                    } else {
                        highlights.1 = true;
                    }
                }
            }
        }
        for &region in regions.keys().filter(|&&region| eligible.contains(&region)) {
            let (hold, active) = mission_highlights.get(&region).copied().unwrap_or_default();
            let mut color = if active {
                colors.active
            } else if hold {
                colors.hold
            } else {
                colors.neutral
            };
            let fade = if highlighted.contains(&region) {
                1.0
            } else {
                colors.fade
            };
            if fade != 1.0 {
                color[3] = fade;
            }
            if let Some(key) = region.checked_add(1) {
                out.styles.insert(
                    key,
                    map_display_data::landscape::Style {
                        color: map_mode::pack(color),
                        pulse: false,
                        fill: fade,
                    },
                );
            } else {
                return Err("selected naval region identity cannot be represented".into());
            }
        }
        Ok(())
    })();
    if let Err(why) = result {
        out.assignment.provinces.clear();
        out.styles.clear();
        out.refused.insert(why, 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlap_stripes_preserve_fraction_and_clamp_repeated_membership() {
        assert_eq!(stripe(0, 2), [0, 127, 0, 47]);
        assert_eq!(stripe(1, 2), [25, 191, 25, 47]);
        assert_eq!(stripe(2, 2), [51, 255, 51, 47]);
        assert_eq!(stripe(3, 2), stripe(2, 2));
    }
    #[test]
    fn taskforce_and_fleet_sources_keep_repeats_empty_gates_and_distinct_highlights() {
        use replay_data::rows::*;
        use std::sync::Arc;
        use viewer_foundation::test_support::{test_frame, test_put as put};
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 10,
                canonical_slot: 1,
            }]),
        );
        put(
            &mut frame,
            TableRows::Fleets(vec![
                FleetRow {
                    id: 40,
                    ..Default::default()
                },
                FleetRow {
                    id: 50,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::FleetCounterInputs(vec![
                FleetCounterInputRow {
                    fleet: 40,
                    mission_regions: (Some(Arc::new(vec![1, 1, 2]))).into(),
                    ..Default::default()
                },
                FleetCounterInputRow {
                    fleet: 50,
                    mission_regions: (Some(Arc::new(Vec::new()))).into(),
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 1,
                    fleet: 40,
                    mission: 8,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 2,
                    fleet: 40,
                    mission: 1,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 3,
                    fleet: 50,
                    mission: 0,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::TaskForceMissions(vec![TaskForceMissionRow {
                task_force: 1,
                regions: vec![1, 3],
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::NavalMissionLedgers(vec![
                NavalMissionLedgerRow {
                    owner: 10,
                    region: 1,
                    task_forces: vec![1],
                    ..Default::default()
                },
                NavalMissionLedgerRow {
                    owner: 10,
                    region: 2,
                    task_forces: vec![1, 2],
                    ..Default::default()
                },
            ]),
        );
        let regions = BTreeMap::from([(1, vec![10]), (2, vec![20]), (3, vec![30]), (4, vec![40])]);
        let eligible = regions.keys().copied().collect();
        let colors = Colors {
            neutral: [0.1, 0.2, 0.3, 0.5],
            active: [0.4, 0.5, 0.6, 0.9],
            hold: [0.2, 0.6, 0.7, 0.5],
            fade: 0.15,
        };
        let selected = BTreeSet::from([Unit::TaskForce(1), Unit::TaskForce(3)]);
        let product = read(
            &frame,
            1,
            &selected,
            &[40, 50],
            &regions,
            &eligible,
            Ok(colors),
        );
        assert!(product.refused.is_empty());
        assert_eq!(product.assignment.provinces[&10], stripe(3, 2));
        assert_eq!(product.assignment.provinces[&20], stripe(1, 2));
        assert_eq!(product.assignment.provinces[&30], stripe(1, 2));
        assert!(!product.assignment.provinces.contains_key(&40));
        assert_eq!(product.styles[&2].color, map_mode::pack(colors.hold));
        assert_eq!(product.styles[&3].color, map_mode::pack(colors.active));
        let tiny = read(
            &frame,
            1,
            &selected,
            &[40, 50],
            &regions,
            &BTreeSet::from([2, 3, 4]),
            Ok(colors),
        );
        assert!(!tiny.styles.contains_key(&2));
        assert!(tiny.assignment.provinces.contains_key(&10));
        assert!(tiny.refused.is_empty());
        assert_eq!(product.styles[&5].fill, colors.fade);
        assert_eq!(
            product.styles[&5].color[3],
            map_mode::pack([0.0, 0.0, 0.0, colors.fade])[3]
        );
        let task = read(&frame, 1, &selected, &[], &regions, &eligible, Ok(colors));
        assert!(!task.assignment.provinces.contains_key(&20));
        assert_eq!(
            task.styles[&3].color[0..3],
            map_mode::pack(colors.hold)[0..3]
        );
        put(
            &mut frame,
            TableRows::FleetCounterInputs(vec![FleetCounterInputRow {
                fleet: 40,
                mission_regions: (None).into(),
                ..Default::default()
            }]),
        );
        assert!(
            read(&frame, 1, &selected, &[], &regions, &eligible, Ok(colors))
                .refused
                .is_empty()
        );
        let refused = read(&frame, 1, &selected, &[40], &regions, &eligible, Ok(colors));
        assert!(refused.styles.is_empty());
        assert!(refused.assignment.provinces.is_empty());
        assert!(
            refused
                .refused
                .contains_key("selected fleet region membership is uncertified")
        );
    }

    #[test]
    fn empty_selection_demands_no_recorded_table_or_colour() {
        let frame = viewer_foundation::test_support::test_frame(1);
        let product = read(
            &frame,
            1,
            &BTreeSet::new(),
            &[],
            &BTreeMap::new(),
            &BTreeSet::new(),
            Err("unreached colours"),
        );
        assert!(product.styles.is_empty() && product.refused.is_empty());
    }
    #[test]
    fn naval_fraction_uses_valid_template_occurrences_and_inclusive_one_percent_threshold() {
        use content_reader_map::province_geometry::ProvinceGeometry;
        let geometry = ProvinceGeometry {
            centroids: [(1, [0, 0]), (2, [0, 0]), (3, [0, 0])].into(),
            land: [2].into(),
            lake: [3].into(),
            ..Default::default()
        };
        let mut members = vec![2; 99];
        members.push(1);
        assert_eq!(naval_fraction(&members, &geometry), 1_000);
        let regions = BTreeMap::from([(1, members.clone())]);
        assert!(eligible_regions(&regions, &geometry).contains(&1));
        members.push(3);
        assert_eq!(naval_fraction(&members, &geometry), 990);
        assert!(!eligible_regions(&BTreeMap::from([(1, members.clone())]), &geometry).contains(&1));
        members.extend([99, 99]);
        assert_eq!(naval_fraction(&members, &geometry), 990);
        assert_eq!(naval_fraction(&[1, 1, 2, 3], &geometry), 50_000);
        assert_eq!(naval_fraction(&[], &geometry), 0);
    }
}
