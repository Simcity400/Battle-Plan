use content_reader_defines::defines::Defines;
use replay_data::{rows::AirBaseRow, rows::AirWingRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_unit_reads::air_registry_sources::AirRegistrySources;
use viewer_gpu::{
    province_map::HeightField,
    ribbon_content::{ArrowType, OrderField},
    ribbon_mesh,
    unit_pose,
};

pub const MISSION_ARROW: &str = "air_mission_direction";

#[derive(Clone, Copy, Debug)]
pub struct Colors {
    active: [f32; 4],
    inactive: [f32; 4],
    selected: [f32; 4],
}
impl Colors {
    pub fn read(defines: &Defines) -> std::io::Result<Self> {
        let read = |name| -> std::io::Result<[f32; 4]> {
            match defines.numbers("NMapMode", name)?.as_slice() {
                [r, g, b, a, ..] => Ok([*r as f32, *g as f32, *b as f32, *a as f32]),
                _ => Err(std::io::Error::other(format!(
                    "NMapMode.{name} requires four colour components"
                ))),
            }
        };
        Ok(Self {
            active: read("AIR_MISSION_ARROW_ACTIVE_COLOR")?,
            inactive: read("AIR_MISSION_ARROW_NONACTIVE_COLOR")?,
            selected: read("AIR_MISSION_ARROW_SELECTED_COLOR")?,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Prepared {
    pub field: OrderField,
    pub refused: BTreeMap<String, usize>,
    highlights: Vec<(u64, [f32; 4])>,
    selected_color: Option<[f32; 4]>,
    pub transfers: super::air_transfers::Prepared,
}
impl Prepared {
    pub fn highlighted(&self, bases: &[u64]) -> OrderField {
        let mut field = self.field.clone();
        if let Some(selected) = self.selected_color {
            for (draw, &(base, normal)) in field.draws.iter_mut().zip(&self.highlights) {
                let color = if bases.contains(&base) {
                    selected
                } else {
                    normal
                };
                draw.color = color;
                draw.secondary_color = color;
                draw.outer_color[3] = color[3];
            }
        }
        ribbon_mesh::append_into(field, &self.transfers.field)
    }
    pub fn warnings(&self) -> Vec<String> {
        self.refused
            .iter()
            .map(|(why, count)| format!("Air mission arrows: {count} refused, {why}."))
            .chain(self.transfers.warnings())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mission {
    base: u64,
    region: u32,
    active: bool,
}

fn fold<'a>(wings: impl IntoIterator<Item = &'a AirWingRow>) -> Vec<Mission> {
    let mut missions: Vec<Mission> = Vec::new();
    let mut indices = BTreeMap::new();
    for wing in wings {
        let Some(region) = wing.mission_region else {
            continue;
        };
        let key = (wing.air_base, region);
        if let Some(&index) = indices.get(&key) {
            let mission: &mut Mission = &mut missions[index];
            mission.active |= wing.mission_active != 0;
        } else {
            indices.insert(key, missions.len());
            missions.push(Mission {
                base: wing.air_base,
                region,
                active: wing.mission_active != 0,
            });
        }
    }
    missions
}

fn controls(from: [f32; 3], to: [f32; 3], width: u32) -> [[i64; 2]; 2] {
    let mut points = [unit_pose::fixed(from), unit_pose::fixed(to)];
    if (i64::from(width as i32 / 2) * 100_000)
        < points[0][0].wrapping_sub(points[1][0]).unsigned_abs() as i64
    {
        let index = usize::from(points[1][0] < points[0][0]);
        points[index][0] = points[index][0].wrapping_add(i64::from(width) * 100_000);
    }
    points
}

pub struct Inputs<'a> {
    pub arrow: Option<&'a ArrowType>,
    pub colors: Result<Colors, &'a str>,
    pub heights: HeightField<'a>,
    pub viewer: u32,
    pub highlighted_bases: &'a [u64],
    #[expect(clippy::type_complexity, reason = "the caller's base lookup, which can refuse or find no position")]
    pub base_position: &'a dyn Fn(&AirBaseRow) -> Result<Option<[f32; 3]>, String>,
    pub region_position: &'a dyn Fn(u32) -> Option<[f32; 3]>,
}

pub fn read(frame: &DecodedFrame, inputs: &Inputs<'_>) -> Prepared {
    let mut out = Prepared::default();
    let result = (|| -> Result<(), String> {
        let wings = frame
            .complete::<tables::AirWings>()
            .map_err(|e| e.to_string())?;
        let registered = frame
            .complete::<tables::AirRegionalMissions>()
            .map_err(|e| e.to_string())?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        let registry = AirRegistrySources::read(frame)
            .map_err(|e| e.to_string())?
            .ok_or("Air mission-arrow base registry is unavailable")?;
        let mut ordered = Vec::new();
        for row in registered {
            let Some(wing) = wings
                .binary_search_by_key(&row.wing, |wing| wing.id)
                .ok()
                .map(|at| &wings[at])
            else {
                return Err("Air mission-arrow registration has no wing".into());
            };
            let slot = links
                .binary_search_by_key(&wing.owner, |row| row.id)
                .ok()
                .map(|at| links[at].canonical_slot)
                .ok_or("Air mission-arrow pool owner is absent from CountryLinks")?;
            if slot != inputs.viewer {
                continue;
            }
            if wing.eligibility_veto != 0
                || wing.eligibility_counter > wing.eligibility_limit
                || !registry.bases.contains_key(&wing.air_base)
            {
                continue;
            }
            let raid = match wing.assigned_to_raid() {
                Some(value) => Some(value),
                None => match &wing.raid_reference {
                    replay_data::rows::Recorded::Value(Some(reference)) => Some(reference.resolves),
                    _ => None,
                },
            }
            .ok_or("Air mission-arrow raid-reference resolution is unavailable")?;
            if raid {
                continue;
            }
            let order = row
                .active_order
                .ok_or("Air mission-arrow all-active ordering is unavailable")?;
            ordered.push((order, wing));
        }
        if ordered.is_empty() {
            return Ok(());
        }
        ordered.sort_unstable_by_key(|&(order, _)| order);
        if ordered.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err("Air mission-arrow all-active order labels are not unique".into());
        }
        let colors = inputs.colors.map_err(str::to_owned)?;
        out.selected_color = Some(colors.selected);
        let arrow = inputs
            .arrow
            .ok_or_else(|| format!("maparrows declares no arrowType {MISSION_ARROW}"))?;
        for mission in fold(ordered.into_iter().map(|(_, wing)| wing)) {
            let result = (|| -> Result<(), String> {
                let Some(from) = (inputs.base_position)(&registry.bases[&mission.base])? else {
                    return Ok(());
                };
                let to = (inputs.region_position)(mission.region)
                    .ok_or("Air mission-arrow region has no content position")?;
                let normal = if mission.active {
                    colors.active
                } else {
                    colors.inactive
                };
                let color = if inputs.highlighted_bases.contains(&mission.base) {
                    colors.selected
                } else {
                    normal
                };
                if let Some(index) = ribbon_mesh::emit(
                    &mut out.field,
                    arrow,
                    &controls(from, to, inputs.heights.width),
                    1.0,
                    color,
                    &|x, z, _| ribbon_mesh::ribbon_height(&inputs.heights, x, z),
                ) {
                    out.field.draws[index].air_mode = true;
                    out.highlights.push((mission.base, normal));
                }
                Ok(())
            })();
            if let Err(why) = result {
                *out.refused.entry(why).or_default() += 1;
            }
        }
        ribbon_mesh::sort_draws(&mut out.field.draws);
        Ok(())
    })();
    if let Err(why) = result {
        out.refused.insert(why, 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_global_order_activity_and_local_base_highlight_reach_the_ribbons() {
        use replay_data::rows::*;
        use viewer_gpu::ribbon_content::ArrowTypes;
        use viewer_foundation::test_support::{test_frame, test_put as put};
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            }]),
        );
        let wings = vec![
            AirWingRow {
                id: 1,
                owner: 1,
                air_base: 10,
                mission_region: Some(5),
                map_flags: Some(16).into(),
                ..Default::default()
            },
            AirWingRow {
                id: 2,
                owner: 1,
                air_base: 10,
                mission_region: Some(5),
                mission_active: 1,
                map_flags: Some(16).into(),
                ..Default::default()
            },
            AirWingRow {
                id: 3,
                owner: 1,
                air_base: 20,
                mission_region: Some(6),
                map_flags: Some(16).into(),
                ..Default::default()
            },
        ];
        put(&mut frame, TableRows::AirWings(wings));
        let mut registrations = vec![
            AirRegionalMissionRow {
                wing: 1,
                ordinal: 0,
                active_order: Some(20),
            },
            AirRegionalMissionRow {
                wing: 2,
                ordinal: 1,
                active_order: Some(30),
            },
            AirRegionalMissionRow {
                wing: 3,
                ordinal: 0,
                active_order: Some(10),
            },
        ];
        put(
            &mut frame,
            TableRows::AirRegionalMissions(registrations.clone()),
        );
        put(
            &mut frame,
            TableRows::AirBases(
                [10, 20]
                    .map(|id| AirBaseRow {
                        id,
                        state: Some(1),
                        ..Default::default()
                    })
                    .into(),
            ),
        );
        put(
            &mut frame,
            TableRows::AirReferenceWitnesses(
                [10, 20]
                    .map(|handle| AirReferenceWitnessRow {
                        handle,
                        ..Default::default()
                    })
                    .into(),
            ),
        );
        let types = ArrowTypes::parse(
            b"arrowType={name=air_mission_direction body_width={1 1} body_uv_scale=1}",
        )
        .unwrap();
        let colors = Colors {
            active: [0., 1., 0., 0.5],
            inactive: [1., 1., 1., 0.2],
            selected: [1., 1., 0., 0.8],
        };
        let inputs = Inputs {
            arrow: types.get(MISSION_ARROW),
            colors: Ok(colors),
            viewer: 1,
            highlighted_bases: &[],
            heights: HeightField {
                width: 100,
                height: 1,
                values: &[0; 100],
            },
            base_position: &|base| Ok(Some([base.id as f32, 0., 0.])),
            region_position: &|region| Some([region as f32 * 10., 0., 0.]),
        };
        let ordinary = read(&frame, &inputs);
        assert!(ordinary.refused.is_empty(), "{:?}", ordinary.refused);
        assert_eq!(
            ordinary
                .field
                .draws
                .iter()
                .map(|d| d.color)
                .collect::<Vec<_>>(),
            [colors.inactive, colors.active]
        );
        assert!(ordinary.field.draws.iter().all(|d| d.air_mode));
        let tinted = ordinary.highlighted(&[10]);
        assert_eq!(tinted.draws[1].color, colors.selected);
        assert_eq!(ordinary.highlighted(&[]).draws[1].color, colors.active);
        let highlighted = read(
            &frame,
            &Inputs {
                highlighted_bases: &[10],
                ..inputs
            },
        );
        assert_eq!(
            highlighted
                .field
                .draws
                .iter()
                .map(|d| d.color)
                .collect::<Vec<_>>(),
            [colors.inactive, colors.selected]
        );
        registrations[0].active_order = None;
        put(&mut frame, TableRows::AirRegionalMissions(registrations));
        let unknown = read(&frame, &inputs);
        assert!(unknown.field.draws.is_empty());
        assert_eq!(
            unknown.refused["Air mission-arrow all-active ordering is unavailable"],
            1
        );
        assert_eq!(
            frame
                .complete::<tables::AirRegionalMissions>()
                .unwrap()
                .len(),
            3
        );
        put(&mut frame, TableRows::CountryLinks(Vec::new()));
        let unlinked = read(&frame, &inputs);
        assert!(unlinked.field.draws.is_empty());
        assert_eq!(
            unlinked.refused["Air mission-arrow pool owner is absent from CountryLinks"],
            1
        );
    }
    #[test]
    fn first_base_region_occurrence_sets_order_and_any_member_activates() {
        let wing = |base, region, active| AirWingRow {
            air_base: base,
            mission_region: Some(region),
            mission_active: active,
            ..Default::default()
        };
        let wings = [wing(9, 5, 0), wing(8, 4, 1), wing(9, 5, 1), wing(9, 4, 0)];
        assert_eq!(
            fold(&wings),
            [
                Mission {
                    base: 9,
                    region: 5,
                    active: true
                },
                Mission {
                    base: 8,
                    region: 4,
                    active: true
                },
                Mission {
                    base: 9,
                    region: 4,
                    active: false
                }
            ]
        );
    }
    #[test]
    fn seam_controls_keep_direction_and_half_width_equality() {
        assert_eq!(
            controls([1., 0., 3.], [99., 0., 4.], 100),
            [[10_100_000, 300_000], [9_900_000, 400_000]]
        );
        assert_eq!(
            controls([99., 0., 3.], [1., 0., 4.], 100),
            [[9_900_000, 300_000], [10_100_000, 400_000]]
        );
        assert_eq!(
            controls([1., 0., 3.], [51., 0., 4.], 100),
            [[100_000, 300_000], [5_100_000, 400_000]]
        );
    }
}
