use content_reader_defines::defines::Defines;
use content_reader_map::province_geometry::ProvinceGeometry;
use replay_data::{rows::AirWingRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_unit_reads::{
    air_range,
    air_region_reach::{Origin, RegionGeometry},
    air_registry_sources::AirRegistrySources,
};

#[derive(Clone, Copy, Default)]
pub struct Colors {
    allowed: [u8; 4],
    blocked: [u8; 4],
    fallback: [[u8; 4]; 3],
    country_alpha: u8,
    fallback_enabled: bool,
}
impl Colors {
    pub fn read(defines: &Defines) -> std::io::Result<Self> {
        let color = |name| -> std::io::Result<[u8; 4]> {
            let values = defines.numbers("NMapMode", name)?;
            if values.len() < 4 {
                return Err(std::io::Error::other(format!(
                    "{name} requires four components"
                )));
            }
            Ok(std::array::from_fn(|at| {
                content_reader_parser::crt::truncate_f32(values[at] as f32 * 255.0) as u8
            }))
        };
        let names = [
            "STRATEGIC_MODE_ENEMY_STRIPES_COLOR",
            "STRATEGIC_MODE_ALLY_STRIPES_COLOR",
            "STRATEGIC_MODE_OUR_STRIPES_COLOR",
        ];
        let fallback = [color(names[0])?, color(names[1])?, color(names[2])?];
        let country_alpha =
            defines.number("NMapMode", "STRATEGIC_MODE_COUNTRY_COLOR_STRIPES_TRANSP")? as f32;
        let fallback_enabled = country_alpha > 0.0
            || names.iter().any(|name| {
                defines
                    .numbers("NMapMode", name)
                    .is_ok_and(|values| values[3] as f32 > 0.0)
            });
        Ok(Self {
            fallback,
            country_alpha: content_reader_parser::crt::truncate_f32(country_alpha * 255.0) as u8,
            fallback_enabled,
            allowed: color("AIR_RANGE_CAN_ASSIGN_MISSION_STRIPES_COLOR")?,
            blocked: color("AIR_RANGE_CANNOT_ASSIGN_MISSION_STRIPES_COLOR")?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Field {
    pub provinces: BTreeMap<u32, [u8; 4]>,
    pub refused: BTreeMap<String, usize>,
}

pub struct Content<'a> {
    pub geometry: &'a ProvinceGeometry,
    pub states: &'a BTreeMap<u32, Vec<u32>>,
    pub regions: &'a BTreeMap<u32, Vec<u32>>,
    pub colors: Result<Colors, &'a str>,
}

fn representatives<'a>(
    selected: &[u64],
    wings: &'a [AirWingRow],
    mut base_live: impl FnMut(u64) -> bool,
    mut range: impl FnMut(&AirWingRow) -> Result<i64, String>,
) -> Result<Vec<(&'a AirWingRow, i64)>, String> {
    let mut result: Vec<(&AirWingRow, i64)> = Vec::new();
    for handle in selected {
        let Ok(at) = wings.binary_search_by_key(handle, |wing| wing.id) else {
            continue;
        };
        let wing = &wings[at];
        if wing.air_base == 0 || !base_live(wing.air_base) {
            continue;
        }
        let value = range(wing)?;
        if let Some(held) = result
            .iter_mut()
            .find(|(held, _)| held.air_base == wing.air_base)
        {
            if held.1 < value {
                *held = (wing, value);
            }
        } else {
            result.push((wing, value));
        }
    }
    Ok(result)
}

pub fn read(
    frame: &DecodedFrame,
    selected: &[u64],
    viewer: u32,
    range: air_range::Defines,
    content: Content<'_>,
) -> Field {
    let mut out = Field::default();
    if selected.is_empty() {
        if let Err(why) = fallback(frame, viewer, content.colors, &mut out) {
            out.refused.insert(why, 1);
        }
        return out;
    }
    let result = (|| {
        let wings = frame
            .complete::<tables::AirWings>()
            .map_err(|e| e.to_string())?;
        let registry = AirRegistrySources::read(frame)
            .map_err(|e| e.to_string())?
            .ok_or("Air assignment base registry is unavailable")?;
        let sources = air_range::FrameSources::new(frame);
        let representatives = representatives(
            selected,
            wings,
            |id| registry.bases.contains_key(&id),
            |wing| air_range::wing_range(wing, range, &sources),
        )?;
        if representatives.is_empty() {
            return fallback(frame, viewer, content.colors, &mut out);
        }
        let colors = content.colors.map_err(str::to_owned)?;
        let mut candidates = Vec::new();
        let mut unknown = Vec::new();
        for (wing, range) in representatives {
            let candidate = (|| {
                let base = &registry.bases[&wing.air_base];
                let host = if registry.hosts_without_task_force.contains(&base.id) {
                    return Err("Air assignment host has a null task force".to_owned());
                } else if registry.host_owners.contains_key(&base.id) {
                    let ships = frame
                        .complete::<tables::Ships>()
                        .map_err(|e| e.to_string())?;
                    let ship = ships
                        .binary_search_by_key(&base.host, |ship| ship.id)
                        .ok()
                        .map(|at| &ships[at])
                        .ok_or("Air assignment resolved host is absent from Ships")?;
                    let forces = frame
                        .complete::<tables::TaskForces>()
                        .map_err(|e| e.to_string())?;
                    Some(
                        forces
                            .binary_search_by_key(&ship.task_force, |force| force.id)
                            .ok()
                            .map(|at| &forces[at])
                            .ok_or("Air assignment host task force is absent")?,
                    )
                } else {
                    None
                };
                if host.is_some_and(|force| force.mission != 0) {
                    return Ok(None);
                }
                let raid = match wing.assigned_to_raid() {
                    Some(value) => Some(value),
                    None => match wing.raid_reference {
                        replay_data::rows::Recorded::Value(Some(value)) => Some(value.resolves),
                        _ => None,
                    },
                }
                .ok_or("Air assignment representative raid resolution is unavailable")?;
                if raid || wing.mission_mask & (1 << 11) != 0 {
                    return Ok(None);
                }
                let origin = if let Some(force) = host {
                    Origin::Host(
                        content
                            .geometry
                            .centroids
                            .get(&force.province)
                            .copied()
                            .unwrap_or([0; 2]),
                    )
                } else if let Some(state) = base.state {
                    let provinces = content
                        .states
                        .get(&state)
                        .ok_or("Air assignment base state is absent from content")?;
                    Origin::State(content.geometry.state_centre(provinces).unwrap_or([0; 2]))
                } else {
                    return Ok(None);
                };
                Ok::<_, String>(Some((origin, range)))
            })();
            match candidate {
                Ok(Some(value)) => candidates.push(value),
                Ok(None) => (),
                Err(why) => unknown.push(why),
            }
        }
        for (&region, provinces) in content.regions {
            if region == 0 {
                continue;
            }
            let shape = RegionGeometry::new(content.geometry, provinces);
            let allowed = candidates
                .iter()
                .any(|&(origin, range)| shape.reaches(content.geometry, provinces, origin, range));
            if !allowed && !unknown.is_empty() {
                for why in &unknown {
                    *out.refused.entry(why.clone()).or_default() += 1;
                }
                continue;
            }
            let color = if allowed {
                colors.allowed
            } else {
                colors.blocked
            };
            for &province in provinces.iter().filter(|&&id| id != 0) {
                out.provinces.insert(province, color);
            }
        }
        Ok::<_, String>(())
    })();
    if let Err(why) = result {
        out.refused.insert(why, 1);
    }
    out
}

fn fallback(
    frame: &DecodedFrame,
    viewer: u32,
    colors: Result<Colors, &str>,
    out: &mut Field,
) -> Result<(), String> {
    let colors = colors.map_err(str::to_owned)?;
    if !colors.fallback_enabled {
        return Ok(());
    }
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?;
    let slot = |raw| -> Result<u32, String> {
        if raw == 0 {
            return Ok(0);
        }
        links
            .binary_search_by_key(&raw, |row| row.id)
            .ok()
            .map(|at| links[at].canonical_slot)
            .ok_or_else(|| "Air country stripe controller or relation country is unresolved".into())
    };
    let relations = frame
        .complete::<tables::AirCountryRelations>()
        .map_err(|e| e.to_string())?;
    let enemy = replay_data::rows::AirCountryRelationRow::ENEMY;
    let ally = replay_data::rows::AirCountryRelationRow::ALLY;
    let at_war = relations
        .iter()
        .any(|row| row.country == viewer && row.kind == enemy);
    let mut palette = BTreeMap::new();
    if at_war {
        for (kind, color) in [(enemy, colors.fallback[0]), (ally, colors.fallback[1])] {
            for relation in relations
                .iter()
                .filter(|row| row.country == viewer && row.kind == kind)
            {
                palette.insert(slot(relation.other)?, color);
            }
        }
        palette.insert(viewer, colors.fallback[2]);
    } else if colors.country_alpha != 0 {
        for row in frame
            .complete::<tables::CountryAppearance>()
            .map_err(|e| e.to_string())?
        {
            let [r, g, b] = row.map_color();
            palette.insert(slot(row.country)?, [r, g, b, colors.country_alpha]);
        }
    }
    for province in frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string())?
    {
        if province.id == 0 || province.controller == 0 {
            continue;
        }
        let controller = slot(province.controller)?;
        if let Some(&color) = palette.get(&controller) {
            out.provinces.insert(province.id, color);
        } else if !at_war && colors.country_alpha != 0 {
            return Err(format!(
                "Air country stripe controller {controller} has no CountryAppearance row"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn longest_range_representative_precedes_restrictions_and_first_ties_survive() {
        let wings = [
            AirWingRow {
                id: 1,
                air_base: 10,
                ..Default::default()
            },
            AirWingRow {
                id: 2,
                air_base: 10,
                mission_mask: 1 << 11,
                ..Default::default()
            },
            AirWingRow {
                id: 3,
                air_base: 20,
                ..Default::default()
            },
        ];
        let ids = |rows: Vec<(&AirWingRow, i64)>| {
            rows.into_iter()
                .map(|(wing, _)| wing.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(representatives(&[1, 2, 3], &wings, |_| true, |wing| Ok(wing.id as i64)).unwrap()),
            [2, 3]
        );
        assert_eq!(
            ids(representatives(&[2, 1, 3], &wings, |_| true, |_| Ok(100)).unwrap()),
            [2, 3]
        );
        assert_eq!(
            ids(representatives(&[1, 2, 3], &wings, |base| base == 20, |_| Ok(100)).unwrap()),
            [3]
        );
    }
    #[test]
    fn a_training_representative_blocks_and_unknown_raid_does_not_invent_red() {
        use replay_data::rows::*;
        use viewer_foundation::test_support::{test_frame, test_put as put};
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            }]),
        );
        put(
            &mut frame,
            TableRows::RaidCountryInputs(vec![RaidCountryInputRow {
                canonical_slot: 1,
                air_range_factor: Some(0),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::AirBases(vec![AirBaseRow {
                id: 10,
                state: Some(1),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::AirReferenceWitnesses(vec![AirReferenceWitnessRow {
                handle: 10,
                ..Default::default()
            }]),
        );
        let wing = AirWingRow {
            id: 1,
            owner: 1,
            air_base: 10,
            mission_mask: 1 << 11,
            base_range_raw: 100_000_000,
            map_flags: Some(16).into(),
            ..Default::default()
        };
        put(&mut frame, TableRows::AirWings(vec![wing.clone()]));
        let geometry = ProvinceGeometry {
            width: 100,
            height: 100,
            bounds: [(1, [0, 0, 2, 2])].into(),
            centroids: [(1, [1, 1])].into(),
            ..Default::default()
        };
        let states = [(1, vec![1])].into();
        let regions = [(1, vec![1])].into();
        let colors = Colors {
            allowed: [0, 255, 0, 0],
            blocked: [255, 0, 0, 127],
            ..Default::default()
        };
        let range = air_range::Defines {
            map_scale: 100_000,
            ace_wing_size: 100,
            ace_wing_size_max_bonus: 100_000,
        };
        let read = |frame: &DecodedFrame| {
            super::read(
                frame,
                &[1],
                1,
                range,
                Content {
                    geometry: &geometry,
                    states: &states,
                    regions: &regions,
                    colors: Ok(colors),
                },
            )
        };
        assert_eq!(read(&frame).provinces[&1], colors.blocked);
        let mut wing = wing;
        wing.mission_mask = 0;
        put(&mut frame, TableRows::AirWings(vec![wing.clone()]));
        assert_eq!(read(&frame).provinces[&1], colors.allowed);
        wing.map_flags = None.into();
        put(&mut frame, TableRows::AirWings(vec![wing]));
        let unknown = read(&frame);
        assert!(unknown.provinces.is_empty());
        assert_eq!(
            unknown.refused["Air assignment representative raid resolution is unavailable"],
            1
        );
    }
    #[test]
    fn unselected_country_stripes_use_stored_enemy_allied_own_overwrite_order() {
        use replay_data::rows::*;
        use viewer_foundation::test_support::{test_frame, test_put as put};
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 1,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 2,
                    canonical_slot: 2,
                },
                CountryLinkRow {
                    id: 3,
                    canonical_slot: 3,
                },
                CountryLinkRow {
                    id: 22,
                    canonical_slot: 2,
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::Provinces(
                (1..=3)
                    .map(|id| ProvinceRow {
                        id,
                        controller: id,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::AirCountryRelations(vec![
                AirCountryRelationRow {
                    country: 1,
                    kind: AirCountryRelationRow::ENEMY,
                    other: 2,
                    ..Default::default()
                },
                AirCountryRelationRow {
                    country: 1,
                    kind: AirCountryRelationRow::ALLY,
                    other: 22,
                    ..Default::default()
                },
            ]),
        );
        let enemy = [255, 0, 0, 64];
        let ally = [0, 255, 0, 64];
        let own = [0, 0, 255, 64];
        let colors = Colors {
            fallback: [enemy, ally, own],
            fallback_enabled: true,
            country_alpha: 63,
            ..Default::default()
        };
        let mut field = super::Field::default();
        fallback(&frame, 1, Ok(colors), &mut field).unwrap();
        assert_eq!(field.provinces, [(1, own), (2, ally)].into());
        put(&mut frame, TableRows::AirCountryRelations(vec![]));
        put(
            &mut frame,
            TableRows::CountryAppearance(
                (1..=3)
                    .map(|country| CountryAppearanceRow {
                        country,
                        red: 1.0f32.to_bits(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let mut field = super::Field::default();
        fallback(&frame, 1, Ok(colors), &mut field).unwrap();
        assert_eq!(
            field.provinces,
            (1..=3).map(|id| (id, [255, 0, 0, 63])).collect()
        );
    }
}
