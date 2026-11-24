use super::radar_symbols::{Field, Symbol};
use content_reader_defines::defines::Defines;
use replay_data::{rows::AirBaseRow, rows::AirTransferInputRow, rows::AirWingRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet};
use viewer_unit_reads::air_registry_sources::AirRegistrySources;
use viewer_gpu::{
    province_map::HeightField,
    ribbon_content::{ArrowType, OrderField},
    ribbon_mesh,
    unit_pose,
};

pub const ARROW: &str = "air_transfer";

pub fn color(defines: &Defines) -> std::io::Result<[f32; 4]> {
    match defines
        .numbers("NMapMode", "AIR_TRANSFER_ARROW_COLOR")?
        .as_slice()
    {
        [r, g, b, a, ..] => Ok([*r as f32, *g as f32, *b as f32, *a as f32]),
        _ => Err(std::io::Error::other(
            "NMapMode.AIR_TRANSFER_ARROW_COLOR requires four components",
        )),
    }
}

#[derive(Clone, Debug, Default)]
pub struct Prepared {
    pub field: OrderField,
    pub progress: Field,
    pub refused: BTreeMap<String, usize>,
}
impl Prepared {
    pub fn warnings(&self) -> Vec<String> {
        self.refused
            .iter()
            .map(|(why, count)| format!("Air transfers: {count} refused, {why}."))
            .collect()
    }
}

pub struct Inputs<'a> {
    pub viewer: u32,
    pub selected: &'a [u64],
    pub arrow: Option<&'a ArrowType>,
    pub color: Result<[f32; 4], &'a str>,
    pub heights: HeightField<'a>,
    #[expect(clippy::type_complexity, reason = "the caller's base lookup, which can refuse or find no position")]
    pub base_position: &'a dyn Fn(&AirBaseRow) -> Result<Option<[f32; 3]>, String>,
}

#[cfg(test)]
#[expect(clippy::type_complexity, reason = "the arrow's endpoints, its interpolated map position and its fixed-point angle, returned together")]
fn location(
    from: [f32; 3],
    to: [f32; 3],
    width: u32,
    progress: i64,
    distance: i64,
) -> Result<([[i64; 2]; 2], [f32; 2], i64), &'static str> {
    locate(
        [unit_pose::fixed(from), unit_pose::fixed(to)],
        width,
        progress,
        distance,
    )
}

#[expect(clippy::type_complexity, reason = "the arrow's endpoints, its interpolated map position and its fixed-point angle, returned together")]
fn locate(
    mut points: [[i64; 2]; 2],
    width: u32,
    progress: i64,
    distance: i64,
) -> Result<([[i64; 2]; 2], [f32; 2], i64), &'static str> {
    if i64::from(width as i32 / 2) * 100_000
        < points[0][0].wrapping_sub(points[1][0]).unsigned_abs() as i64
    {
        let smaller = usize::from(points[1][0] < points[0][0]);
        points[smaller][0] = points[smaller][0].wrapping_add(i64::from(width) * 100_000);
    }
    let ratio = if distance <= 0 {
        0
    } else if !viewer_unit_reads::air_range::finite(progress) {
        0xffff_ffff
    } else {
        progress
            .wrapping_mul(100_000)
            .checked_div(distance)
            .ok_or("Air transfer progress division overflows")?
    };
    let from = points[0].map(|v| v as f32 / 100_000.);
    let to = points[1].map(|v| v as f32 / 100_000.);
    let delta = [to[0] - from[0], to[1] - from[1]];
    let fixed = |v: f32| ((if v >= 0. { 1. } else { 0. }) + v * 100_000. - 0.5) as i64;
    let position = std::array::from_fn(|axis| {
        fixed(from[axis] + delta[axis] * (ratio as f32 / 100_000.)) as f32 / 100_000.
    });
    let angle = fixed(delta[0].atan2(-delta[1])).wrapping_sub(157_080);
    Ok((points, position, angle))
}

pub fn read(frame: &DecodedFrame, inputs: &Inputs<'_>) -> Prepared {
    let mut out = Prepared::default();
    if inputs.selected.is_empty() {
        return out;
    }
    let result = (|| -> Result<(), String> {
        let wings = frame
            .complete::<tables::AirWings>()
            .map_err(|e| e.to_string())?;
        let selected: BTreeMap<u64, &AirWingRow> = inputs
            .selected
            .iter()
            .filter_map(|id| {
                wings
                    .binary_search_by_key(id, |wing| wing.id)
                    .ok()
                    .map(|at| (*id, &wings[at]))
            })
            .filter(|(_, wing)| {
                wing.base_override != 0 && wing.eligibility_counter <= wing.eligibility_limit
            })
            .collect();
        if selected.is_empty() {
            return Ok(());
        }
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        let own = |raw| {
            links
                .binary_search_by_key(&raw, |row| row.id)
                .ok()
                .map(|at| links[at].canonical_slot)
                .ok_or_else(|| "Air transfer pool owner is unresolved".to_owned())
        };
        let rows = frame
            .complete::<tables::AirTransferInputs>()
            .map_err(|e| e.to_string())?;
        let mut incoming = BTreeMap::<u64, Vec<(&AirTransferInputRow, &AirWingRow)>>::new();
        for row in rows {
            let Some(&wing) = selected.get(&row.wing) else {
                continue;
            };
            if own(wing.owner)? == inputs.viewer {
                incoming.entry(row.base).or_default().push((row, wing));
            }
        }
        if incoming.is_empty() {
            return Ok(());
        }
        let registry = AirRegistrySources::read(frame)
            .map_err(|e| e.to_string())?
            .ok_or("Air transfer base registry is unavailable")?;
        let sites = frame
            .complete::<tables::RaidAirSites>()
            .map_err(|e| e.to_string())?;
        let mut sites: Vec<_> = sites
            .iter()
            .filter(|site| site.canonical_slot == inputs.viewer && site.list == 0)
            .collect();
        sites.sort_unstable_by_key(|site| site.ordinal);
        let color = inputs.color.map_err(str::to_owned)?;
        let arrow = inputs
            .arrow
            .ok_or("maparrows declares no air_transfer arrowType")?;
        let mut pairs = BTreeSet::new();
        for site in sites {
            let Some(incoming) = incoming.get_mut(&site.base) else {
                continue;
            };
            if incoming.iter().any(|(row, _)| row.ordinal.is_none()) {
                *out.refused
                    .entry("Air transfer incoming order is uncertified or unreadable".into())
                    .or_default() += 1;
                continue;
            }
            incoming.sort_by_key(|(row, _)| row.ordinal);
            let mut destination = None;
            for &(row, wing) in incoming.iter() {
                let result = (|| -> Result<(), String> {
                    let base = |id| {
                        registry
                            .bases
                            .get(&id)
                            .ok_or("Air transfer base does not resolve".to_owned())
                    };
                    let (Some(from), Some(to)) = (
                        (inputs.base_position)(base(wing.air_base)?)?,
                        (inputs.base_position)(base(row.base)?)?,
                    ) else {
                        return Ok(());
                    };
                    let progress = row
                        .progress_raw
                        .ok_or("Air transfer progress was not captured")?;
                    let distance = row
                        .distance_raw
                        .ok_or("Air transfer total distance was not captured")?;
                    let (controls, [x, z], angle) = locate(
                        [
                            unit_pose::fixed(from),
                            destination.unwrap_or_else(|| unit_pose::fixed(to)),
                        ],
                        inputs.heights.width,
                        progress,
                        distance,
                    )?;
                    destination = Some(controls[1]);
                    out.progress.symbols.push(Symbol {
                        country: 0,
                        state: 0,
                        own: true,
                        roster: true,
                        position: [x, inputs.heights.native_height(x, z), z],
                        diameter: 2.,
                        angle: Some(angle),
                    });
                    if pairs.insert((wing.air_base, row.base))
                        && let Some(at) = ribbon_mesh::emit(
                            &mut out.field,
                            arrow,
                            &controls,
                            1.,
                            color,
                            &|x, z, _| ribbon_mesh::ribbon_height(&inputs.heights, x, z),
                        )
                    {
                        out.field.draws[at].air_mode = true;
                    }
                    Ok(())
                })();
                if let Err(why) = result {
                    *out.refused.entry(why).or_default() += 1;
                }
            }
        }
        ribbon_mesh::sort_draws(&mut out.field.draws);
        Ok(())
    })();
    if let Err(why) = result {
        *out.refused.entry(why).or_default() += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_transfers_follow_site_and_incoming_order_and_coalesce_pairs() {
        use replay_data::{
            rows::AirReferenceWitnessRow, rows::CountryLinkRow, rows::RaidAirSiteRow,
            rows::TableRows,
        };
        use viewer_foundation::test_support::{test_frame, test_put as put};
        let mut frame = test_frame(7);
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 101,
                canonical_slot: 1,
            }]),
        );
        put(
            &mut frame,
            TableRows::AirWings(
                (1..=3)
                    .map(|id| AirWingRow {
                        id,
                        owner: 101,
                        air_base: 10,
                        base_override: if id <= 2 { 20 } else { 30 },
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        let mut rows = vec![
            AirTransferInputRow {
                wing: 1,
                base: 20,
                ordinal: Some(1),
                progress_raw: Some(500_000),
                distance_raw: Some(1_000_000),
            },
            AirTransferInputRow {
                wing: 2,
                base: 20,
                ordinal: Some(0),
                progress_raw: Some(0),
                distance_raw: Some(1_000_000),
            },
            AirTransferInputRow {
                wing: 3,
                base: 30,
                ordinal: Some(0),
                progress_raw: Some(1_000_000),
                distance_raw: Some(1_000_000),
            },
        ];
        put(&mut frame, TableRows::AirTransferInputs(rows.clone()));
        put(
            &mut frame,
            TableRows::AirBases(
                [10, 20, 30]
                    .map(|id| AirBaseRow {
                        id,
                        state: Some(id as u32),
                        ..Default::default()
                    })
                    .into(),
            ),
        );
        put(
            &mut frame,
            TableRows::AirReferenceWitnesses(
                [10, 20, 30]
                    .map(|handle| AirReferenceWitnessRow {
                        handle,
                        ..Default::default()
                    })
                    .into(),
            ),
        );
        put(
            &mut frame,
            TableRows::RaidAirSites(
                [20, 30]
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, base)| RaidAirSiteRow {
                        canonical_slot: 1,
                        list: 0,
                        ordinal: ordinal as u32,
                        base,
                        container_capacity_raw: None,
                    })
                    .collect(),
            ),
        );
        let types = viewer_gpu::ribbon_content::ArrowTypes::parse(
            b"arrowType={name=air_transfer body_width={1 1} body_uv_scale=1}",
        )
        .unwrap();
        let input = Inputs {
            viewer: 1,
            selected: &[1, 3, 2],
            arrow: types.get(ARROW),
            color: Ok([1.; 4]),
            heights: HeightField {
                width: 100,
                height: 2,
                values: &[0; 200],
            },
            base_position: &|base| Ok(Some([base.id as f32, 0., 0.])),
        };
        let out = read(&frame, &input);
        assert!(out.refused.is_empty(), "{:?}", out.refused);
        assert_eq!(out.field.draws.len(), 2);
        assert_eq!(
            out.progress
                .symbols
                .iter()
                .map(|s| s.position[0])
                .collect::<Vec<_>>(),
            [10., 15., 30.]
        );
        assert!(out.field.draws.iter().all(|draw| draw.air_mode));
        assert!(
            out.progress
                .symbols
                .iter()
                .all(|symbol| symbol.angle.is_some() && symbol.diameter == 2.)
        );
        rows[0].ordinal = None;
        put(&mut frame, TableRows::AirTransferInputs(rows));
        let partial = read(&frame, &input);
        assert_eq!(partial.field.draws.len(), 1);
        assert_eq!(partial.progress.symbols.len(), 1);
        assert_eq!(partial.refused.values().sum::<usize>(), 1);
        assert!(
            read(
                &frame,
                &Inputs {
                    selected: &[],
                    ..input
                }
            )
            .refused
            .is_empty()
        );
    }
    #[test]
    fn progress_preserves_zero_total_unclamped_values_and_native_seam_interpolation() {
        let from = [99., 0., 4.];
        let to = [1., 0., 8.];
        let (points, at, _) = location(from, to, 100, 50_000, 100_000).unwrap();
        assert_eq!(points, [[9_900_000, 400_000], [10_100_000, 800_000]]);
        assert_eq!(at, [100., 6.]);
        assert_eq!(
            location(from, to, 100, 200_000, 100_000).unwrap().1,
            [103., 12.]
        );
        assert_eq!(location(from, to, 100, 200_000, 0).unwrap().1, [99., 4.]);
        assert_eq!(
            location([1., 0., 4.], [51., 0., 4.], 100, 0, 1).unwrap().0,
            [[100_000, 400_000], [5_100_000, 400_000]]
        );
    }
}
