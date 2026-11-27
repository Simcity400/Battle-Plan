use super::radar_symbols::{Centers, Field, Symbol};
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_unit_reads::{air_range::{self, Sources}, air_registry_sources::AirRegistrySources};
use viewer_gpu::province_map::HeightField;

const GROUPS: usize = 128;
type Sample = ([i32; 2], i64, bool);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RangeField {
    pub ranges: Field,
    pub highlighted_bases: Vec<u64>,
}
impl std::ops::Deref for RangeField {
    type Target = Field;
    fn deref(&self) -> &Field {
        &self.ranges
    }
}
impl std::ops::DerefMut for RangeField {
    fn deref_mut(&mut self) -> &mut Field {
        &mut self.ranges
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CarrierSamples {
    base: Option<u64>,
    samples: Vec<Sample>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields {
    pub selected: RangeField,
    pub hovered: BTreeMap<(u32, i32), RangeField>,
    carriers: BTreeMap<u64, Result<CarrierSamples, String>>,
    carrier_heights: BTreeMap<[i32; 2], f32>,
    carrier_refusal: Option<String>,
}

impl Fields {
    pub fn shown(&self, hovered: Option<(u32, i32)>) -> &RangeField {
        hovered
            .and_then(|state| self.hovered.get(&state))
            .filter(|field| {
                !field.symbols.is_empty()
                    || !field.refused.is_empty()
                    || !field.highlighted_bases.is_empty()
            })
            .unwrap_or(&self.selected)
    }

    pub fn carrier_ranges(&self, forces: &[u64]) -> RangeField {
        let mut groups = Vec::new();
        let mut field = RangeField::default();
        for force in forces {
            let samples = self
                .carriers
                .get(force)
                .ok_or_else(|| {
                    self.carrier_refusal.clone().unwrap_or_else(|| {
                        "hovered task force is absent from the carrier census".into()
                    })
                })
                .and_then(|samples| samples.as_ref().map_err(Clone::clone));
            match samples {
                Ok(samples) => {
                    if let Some(base) = samples.base
                        && !field.highlighted_bases.contains(&base)
                    {
                        field.highlighted_bases.push(base);
                    }
                    for &(origin, radius, empty) in &samples.samples {
                        group(&mut groups, origin, radius, empty);
                    }
                }
                Err(reason) => {
                    field.refused.insert(reason, 1);
                }
            }
        }
        if !field.refused.is_empty() {
            return field;
        }
        if groups.is_empty() {
            field.ranges.clone_from(&self.selected.ranges);
            for &base in &self.selected.highlighted_bases {
                if !field.highlighted_bases.contains(&base) {
                    field.highlighted_bases.push(base);
                }
            }
            return field;
        }
        for (index, (origin, radius, empty)) in groups.into_iter().enumerate() {
            field.symbols.push(symbol(
                index,
                origin,
                radius,
                empty,
                self.carrier_heights[&origin],
            ));
        }
        field
    }
}

pub fn read_fields(
    frame: &DecodedFrame,
    viewer: u32,
    selected: &[u64],
    defines: air_range::Defines,
    centers: &Centers,
    centroids: &BTreeMap<u32, [i32; 2]>,
    heights: HeightField<'_>,
) -> Fields {
    let sources = air_range::FrameSources::new(frame);
    let registry = AirRegistrySources::read(frame)
        .map_err(|error| error.to_string())
        .and_then(|registry| {
            registry.ok_or_else(|| "air range base registry is incomplete".into())
        });
    let mut fields = Fields {
        selected: read_at(
            frame, selected, defines, centers, centroids, heights, None, &sources, &registry,
        ),
        ..Default::default()
    };
    let result = (|| -> Result<(), String> {
        let bases = frame
            .complete::<tables::AirBases>()
            .map_err(|error| error.to_string())?;
        let registrations = frame
            .complete::<tables::AirBaseRegistrations>()
            .map_err(|error| error.to_string())?;
        for registration in registrations.iter().filter(|row| row.slot == viewer) {
            let Some(base) = bases
                .binary_search_by_key(&registration.base, |row| row.id)
                .ok()
                .map(|at| &bases[at])
            else {
                continue;
            };
            let Some(state) = base.state else {
                continue;
            };
            if ![0, 1].contains(&base.kind) {
                continue;
            }
            let mut field = read_at(
                frame,
                &registration.wings,
                defines,
                centers,
                centroids,
                heights,
                Some(state),
                &sources,
                &registry,
            );
            if !field.highlighted_bases.contains(&base.id) {
                field.highlighted_bases.push(base.id);
            }
            if field.symbols.is_empty() && field.refused.is_empty() {
                field.ranges.clone_from(&fields.selected.ranges);
                for &selected in &fields.selected.highlighted_bases {
                    if !field.highlighted_bases.contains(&selected) {
                        field.highlighted_bases.push(selected);
                    }
                }
            }
            fields.hovered.insert((state, base.kind), field);
        }
        Ok(())
    })();
    if let Err(reason) = result {
        fields
            .selected
            .refused
            .insert(format!("hovered air-base ranges: {reason}"), 1);
    }
    if let Err(reason) = fields.read_carriers(
        frame, viewer, &sources, defines, centers, centroids, heights,
    ) {
        fields.carrier_refusal = Some(format!("hovered carrier ranges: {reason}"));
    }
    fields
}

impl Fields {
    #[expect(clippy::too_many_arguments, reason = "the frame, the viewer, the range sources and defines, and the map's centres and heights")]
    fn read_carriers(
        &mut self,
        frame: &DecodedFrame,
        viewer: u32,
        sources: &air_range::FrameSources<'_>,
        defines: air_range::Defines,
        centers: &Centers,
        centroids: &BTreeMap<u32, [i32; 2]>,
        heights: HeightField<'_>,
    ) -> Result<(), String> {
        let forces = frame
            .complete::<tables::TaskForces>()
            .map_err(|e| e.to_string())?;
        let ships = frame
            .complete::<tables::Ships>()
            .map_err(|e| e.to_string())?;
        let models = frame
            .complete::<tables::ShipModels>()
            .map_err(|e| e.to_string())?;
        let bases = frame
            .complete::<tables::AirBases>()
            .map_err(|e| e.to_string())?;
        let registrations = frame
            .complete::<tables::AirBaseRegistrations>()
            .map_err(|e| e.to_string())?;
        let mut rosters: BTreeMap<u64, Vec<_>> = BTreeMap::new();
        for ship in ships {
            let model = models
                .binary_search_by_key(&ship.id, |row| row.ship)
                .ok()
                .map(|at| &models[at]);
            rosters.entry(ship.task_force).or_default().push(model);
        }
        for force in forces {
            let result = (|| {
                let Some(roster) = rosters.get_mut(&force.id) else {
                    return Ok(CarrierSamples::default());
                };
                let Some(base) = carrier_base(roster)? else {
                    return Ok(CarrierSamples::default());
                };
                let Some(registration) = registrations
                    .binary_search_by_key(&(base, viewer), |row| (row.base, row.slot))
                    .ok()
                    .map(|at| &registrations[at])
                else {
                    return Ok(CarrierSamples::default());
                };
                let base = bases
                    .binary_search_by_key(&base, |row| row.id)
                    .ok()
                    .map(|at| &bases[at])
                    .ok_or("hovered carrier air base is absent from AirBases")?;
                let origin = if let Some(state) = base.state {
                    centers.get(state)?
                } else {
                    let ship = ships
                        .binary_search_by_key(&base.host, |row| row.id)
                        .ok()
                        .map(|at| &ships[at])
                        .ok_or("hovered air base's host ship is absent")?;
                    let host = forces
                        .binary_search_by_key(&ship.task_force, |row| row.id)
                        .ok()
                        .map(|at| &forces[at])
                        .ok_or("hovered air base's host task force is absent")?;
                    *centroids
                        .get(&host.province)
                        .ok_or("hovered carrier province is absent from content")?
                };
                self.carrier_heights.insert(
                    origin,
                    heights.native_height(origin[0] as f32, origin[1] as f32),
                );
                let mut samples = Vec::new();
                for &handle in &registration.wings {
                    let Some(wing) = sources.wing(handle)? else {
                        continue;
                    };
                    samples.push((
                        origin,
                        air_range::wing_range(wing, defines, sources)?,
                        wing.planes == 0,
                    ));
                }
                Ok(CarrierSamples {
                    base: Some(base.id),
                    samples,
                })
            })();
            self.carriers.insert(force.id, result);
        }
        Ok(())
    }
}

fn carrier_base(
    roster: &mut [Option<&replay_data::rows::ShipModelRow>],
) -> Result<Option<u64>, String> {
    use viewer_unit_reads::counter_order;
    if roster
        .iter()
        .any(|model| model.is_none_or(|model| model.counter.is_none()))
    {
        return Err("hovered task force's ordered ship operands are incomplete".into());
    }
    roster.sort_by_key(|model| model.unwrap().counter.as_ref().unwrap().roster_ordinal);
    Ok(roster
        .iter()
        .map(|model| model.unwrap())
        .find(|model| counter_order::is_carrier(model) == Some(true))
        .map(|model| model.counter.as_ref().unwrap().carrier_air_base)
        .filter(|base| *base != 0))
}

fn symbol(index: usize, [x, z]: [i32; 2], range: i64, empty: bool, height: f32) -> Symbol {
    Symbol {
        state: index as u32,
        country: 0,
        own: !empty,
        roster: true,
        position: [x as f32, height, z as f32],
        diameter: range.wrapping_mul(2) as f32 / 100_000.,
        angle: None,
    }
}

fn group(groups: &mut Vec<([i32; 2], i64, bool)>, origin: [i32; 2], range: i64, empty: bool) {
    if let Some((_, _, held)) = groups
        .iter_mut()
        .find(|(point, radius, _)| *point == origin && *radius == range)
    {
        *held |= empty;
    } else if groups.len() < GROUPS {
        groups.push((origin, range, empty));
    }
}

#[expect(clippy::too_many_arguments, reason = "the frame, the selection, the range defines and sources, and the map's centres and heights")]
fn read_at(
    frame: &DecodedFrame,
    selected: &[u64],
    defines: air_range::Defines,
    centers: &Centers,
    centroids: &BTreeMap<u32, [i32; 2]>,
    heights: HeightField<'_>,
    fixed_state: Option<u32>,
    sources: &air_range::FrameSources<'_>,
    registry: &Result<AirRegistrySources, String>,
) -> RangeField {
    let mut out = RangeField::default();
    if selected.is_empty() {
        return out;
    }
    let result = (|| -> Result<(), String> {
        let registry = if fixed_state.is_none() {
            Some(registry.as_ref().map_err(Clone::clone)?)
        } else {
            None
        };
        let mut groups = Vec::new();
        for &handle in selected {
            let Some(wing) = sources.wing(handle)? else {
                continue;
            };
            let base = registry.and_then(|registry| registry.bases.get(&wing.air_base));
            let origin = if let Some(state) = fixed_state.or(base.and_then(|base| base.state)) {
                centers.get(state)?
            } else {
                let Some(base) = base else {
                    continue;
                };
                let ships = frame
                    .complete::<tables::Ships>()
                    .map_err(|e| e.to_string())?;
                let Some(ship) = ships
                    .binary_search_by_key(&base.host, |row| row.id)
                    .ok()
                    .map(|at| &ships[at])
                else {
                    continue;
                };
                let forces = frame
                    .complete::<tables::TaskForces>()
                    .map_err(|e| e.to_string())?;
                let Some(force) = forces
                    .binary_search_by_key(&ship.task_force, |row| row.id)
                    .ok()
                    .map(|at| &forces[at])
                else {
                    continue;
                };
                *centroids
                    .get(&force.province)
                    .ok_or("air range carrier province is absent from content")?
            };
            let range = air_range::wing_range(wing, defines, sources)?;
            group(&mut groups, origin, range, wing.planes == 0);
            if !out.highlighted_bases.contains(&wing.air_base) {
                out.highlighted_bases.push(wing.air_base);
            }
        }
        for (index, ([x, z], range, empty)) in groups.into_iter().enumerate() {
            out.symbols.push(Symbol {
                state: index as u32,
                country: 0,
                own: !empty,
                roster: true,
                position: [
                    x as f32,
                    heights.native_height(x as f32, z as f32),
                    z as f32,
                ],
                diameter: range.wrapping_mul(2) as f32 / 100_000.,
                angle: None,
            });
        }
        Ok(())
    })();
    if let Err(reason) = result {
        out.symbols.clear();
        out.refused.insert(reason, 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hovered_pool_access_does_not_require_capacity_or_a_state_registration_gate() {
        use replay_data::{
            rows::AirBaseRegistrationRow, rows::AirBaseRow, rows::TableId, rows::TableRows,
        };
        use replay_frame::{frame::Metrics, frame::TableView};
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "not demanded".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::AirBases.index()] =
            TableView::Complete(TableRows::AirBases(vec![AirBaseRow {
                id: 8,
                kind: 0,
                state: Some(4),
                ..Default::default()
            }]));
        *frame.tables[TableId::AirBaseRegistrations.index()] =
            TableView::Complete(TableRows::AirBaseRegistrations(vec![
                AirBaseRegistrationRow {
                    base: 8,
                    slot: 5,
                    capacity_raw: 0,
                    ..Default::default()
                },
            ]));
        let centers = Centers::new(&BTreeMap::new(), &BTreeMap::new(), 10);
        let fields = read_fields(
            &frame,
            5,
            &[],
            air_range::Defines {
                map_scale: 100_000,
                ace_wing_size: 80,
                ace_wing_size_max_bonus: 100_000,
            },
            &centers,
            &BTreeMap::new(),
            HeightField {
                width: 1,
                height: 1,
                values: &[0],
            },
        );
        assert!(fields.hovered.contains_key(&(4, 0)));
        assert_eq!(fields.shown(Some((4, 0))).highlighted_bases, [8]);
        assert!(fields.selected.highlighted_bases.is_empty());
        assert!(fields.selected.refused.is_empty());
    }
    #[test]
    fn a_hover_uses_the_first_carrier_in_ship_roster_order_even_when_its_base_is_null() {
        use replay_data::{rows::ShipCounterInputs, rows::ShipModelRow};
        use viewer_unit_reads::counter_order::{CARRIER_FLAG, MISSILE_CARRIER_FLAG};
        let model = |roster_ordinal,
                     formation_flags,
                     positive_equipment_flags,
                     carrier_air_base| ShipModelRow {
            formation_flags,
            counter: Some(ShipCounterInputs {
                roster_ordinal,
                positive_equipment_flags,
                carrier_air_base,
                ..Default::default()
            }),
            ..Default::default()
        };
        let later = model(2, CARRIER_FLAG, 0, 22);
        let first = model(1, 0, MISSILE_CARRIER_FLAG, 11);
        let ordinary = model(0, 0, 0, 77);
        assert_eq!(
            carrier_base(&mut [Some(&later), Some(&ordinary), Some(&first)]).unwrap(),
            Some(11)
        );
        let null = model(1, CARRIER_FLAG, 0, 0);
        assert_eq!(
            carrier_base(&mut [Some(&later), Some(&null)]).unwrap(),
            None
        );
        assert_eq!(carrier_base(&mut [Some(&ordinary)]).unwrap(), None);
        assert!(carrier_base(&mut [None, Some(&later)]).is_err());
    }
    #[test]
    fn hovered_carriers_share_the_scratch_in_source_order_and_zero_groups_fall_back() {
        let mut fields = Fields::default();
        fields.selected.refused.insert("selected range".into(), 1);
        fields.carriers.insert(
            1,
            Ok(CarrierSamples {
                base: Some(10),
                samples: vec![([4, 8], 20, false), ([4, 8], 30, false)],
            }),
        );
        fields.carriers.insert(
            2,
            Ok(CarrierSamples {
                base: Some(20),
                samples: vec![([4, 8], 20, true), ([2, 6], 40, false)],
            }),
        );
        fields.carriers.insert(3, Ok(CarrierSamples::default()));
        fields.carrier_heights = [([4, 8], 5.0), ([2, 6], 7.0)].into();
        let shown = fields.carrier_ranges(&[1, 2]);
        assert!(shown.refused.is_empty());
        assert_eq!(shown.symbols.len(), 3);
        assert_eq!(
            shown
                .symbols
                .iter()
                .map(|s| (s.state, s.own, s.position))
                .collect::<Vec<_>>(),
            [
                (0, false, [4., 5., 8.]),
                (1, true, [4., 5., 8.]),
                (2, true, [2., 7., 6.])
            ]
        );
        assert_eq!(fields.carrier_ranges(&[3]), fields.selected);
        assert_eq!(fields.carrier_ranges(&[1, 2]).highlighted_bases, [10, 20]);
        assert!(!fields.carrier_ranges(&[4]).refused.is_empty());
        fields.carriers.insert(4, Err("unreadable carrier".into()));
        let unknown = fields.carrier_ranges(&[1, 4]);
        assert!(unknown.symbols.is_empty());
        assert!(unknown.refused.contains_key("unreadable carrier"));
    }
    #[test]
    fn an_empty_hover_falls_back_to_selection_and_a_refused_hover_stays_unknown() {
        let fields = Fields {
            selected: RangeField {
                ranges: Field {
                    refused: [("selected".into(), 1)].into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            hovered: [((7, 0), RangeField::default())].into(),
            ..Default::default()
        };
        assert_eq!(fields.shown(Some((7, 0))), &fields.selected);
        assert_eq!(fields.shown(Some((7, 1))), &fields.selected);
        assert_eq!(fields.shown(None), &fields.selected);
        let mut fields = fields;
        fields
            .hovered
            .get_mut(&(7, 0))
            .unwrap()
            .refused
            .insert("unknown hover groups".into(), 1);
        assert_eq!(fields.shown(Some((7, 0))), &fields.hovered[&(7, 0)]);
    }
    #[test]
    fn ranges_share_origin_and_radius_and_empty_wings_choose_the_empty_colour() {
        let mut groups = Vec::new();
        group(&mut groups, [2, 4], 30, false);
        group(&mut groups, [2, 4], 30, true);
        group(&mut groups, [2, 4], 30, false);
        group(&mut groups, [2, 4], 31, false);
        assert_eq!(groups, [([2, 4], 30, true), ([2, 4], 31, false)]);
    }
    #[test]
    fn the_native_scratch_limit_keeps_first_groups_and_still_updates_existing_groups() {
        let mut groups = Vec::new();
        for range in 0..5000 {
            group(&mut groups, [2, 4], range, false);
        }
        group(&mut groups, [2, 4], 0, true);
        assert_eq!(groups.len(), 128);
        assert_eq!(groups[0], ([2, 4], 0, true));
        assert_eq!(groups[127], ([2, 4], 127, false));
    }
}
