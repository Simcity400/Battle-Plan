use content_reader_gui::gui::native;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use viewer_counter_kit::{CounterInputs, CounterReport, visible};
use viewer_foundation::perspective::Vision;
use replay_data::rows::AirCountryRelationRow;
use viewer_native_gui::native_gui::{self, lifecycle::Instance};
use replay_derive::{naval_mine_levels::Levels, naval_region_control::raw_roster};

type Fact<T> = Result<T, String>;
#[derive(Clone, Debug)]
pub struct Packet {
    pub mines: Fact<Option<[f32; 2]>>,
    mine_constructed: Fact<bool>,
    pub accidents: Fact<i32>,
    pub grid_counts: [usize; 3],
}
impl Default for Packet {
    fn default() -> Self {
        Self {
            mines: Ok(None),
            mine_constructed: Ok(false),
            accidents: Ok(0),
            grid_counts: [0; 3],
        }
    }
}

fn ratio(value: i64, maximum: i64) -> f32 {
    let fixed = if maximum == 0 || [i64::MIN, i64::MIN + 1, i64::MAX].contains(&value) {
        0xffff_ffff_i64
    } else {
        value.wrapping_mul(100_000).wrapping_div(maximum)
    };
    fixed as f32 / 100_000.0
}
fn enemy_sum(
    raw: &[u32],
    mut eligible: impl FnMut(u32) -> Fact<bool>,
    mut amount: impl FnMut(u32) -> Fact<i64>,
) -> Fact<i64> {
    let mut sum = 0_i64;
    for &country in raw {
        if eligible(country)? {
            sum = sum.wrapping_add(amount(country)?);
        }
    }
    Ok(sum)
}

pub fn prepare(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    vision: &Vision,
    regions: &[u32],
) -> BTreeMap<u32, Packet> {
    let links: Fact<BTreeMap<_, _>> = frame
        .complete::<tables::CountryLinks>()
        .map(|rows| rows.iter().map(|r| (r.id, r.canonical_slot)).collect())
        .map_err(|e| e.to_string());
    let slot = |raw| {
        links
            .as_ref()
            .map_err(Clone::clone)?
            .get(&raw)
            .copied()
            .ok_or_else(|| "naval status country link unavailable".to_owned())
    };
    let own_raw = frame
        .complete::<tables::Countries>()
        .map_err(|e| e.to_string())
        .and_then(|rows| {
            rows.iter()
                .find(|r| r.canonical_slot == vision.ours())
                .map(|r| r.id)
                .ok_or("naval status viewing country unavailable".into())
        });
    let levels = std::sync::OnceLock::new();
    let enemies = raw_roster(frame, vision.ours(), AirCountryRelationRow::ENEMY);
    let counters: Fact<BTreeMap<u32, i32>> = frame
        .complete::<tables::CountryCounterInputs>()
        .map_err(|e| e.to_string())
        .and_then(|rows| {
            rows.iter()
                .map(|row| Ok((slot(row.country)?, row.owned_state_count)))
                .collect()
        });
    let capitulations: Fact<BTreeMap<u32, u8>> = frame
        .complete::<tables::Capitulation>()
        .map_err(|e| e.to_string())
        .and_then(|rows| {
            rows.iter()
                .map(|row| Ok((slot(row.country)?, row.capitulated)))
                .collect()
        });
    let accidents: Fact<BTreeMap<u32, i32>> = frame
        .complete::<tables::NavalAccidentReports>()
        .map_err(|e| e.to_string())
        .and_then(|rows| {
            let mut counts = BTreeMap::new();
            for row in rows.iter().filter(|row| row.kind == 0) {
                if slot(row.country)? == vision.ours() {
                    let count = counts.entry(row.region).or_insert(0_i32);
                    *count = count.wrapping_add(1);
                }
            }
            Ok(counts)
        });
    let features = viewer_foundation::naval_mine_feature::enabled(frame);
    let maximum = inputs
        .assets
        .air_sortie_defines
        .as_ref()
        .map(|d| d.mine_maximum_raw)
        .map_err(Clone::clone);
    regions
        .iter()
        .map(|&region| {
            let height = inputs
                .content
                .region_icon(region)
                .map_or(inputs.camera.height, |anchor| inputs.icon_height(anchor));
            let detailed = visible(height, inputs.content.defines.icons_cutoff);
            let mines = (|| -> Fact<Option<[f32; 2]>> {
                if !detailed || !*features.as_ref().map_err(Clone::clone)? {
                    return Ok(None);
                }
                let max = *maximum.as_ref().map_err(Clone::clone)?;
                let levels = levels
                    .get_or_init(|| {
                        links
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|links| Levels::read(frame, links))
                    })
                    .as_ref()
                    .map_err(Clone::clone)?;
                let ours = ratio(
                    levels.amount(*own_raw.as_ref().map_err(Clone::clone)?, region)?,
                    max,
                );
                let enemy = enemy_sum(
                    enemies.as_ref().map_err(Clone::clone)?,
                    |raw| {
                        if (raw as i32) <= 0 {
                            return Ok(false);
                        }
                        let target = slot(raw)?;
                        let count = *counters
                            .as_ref()
                            .map_err(Clone::clone)?
                            .get(&target)
                            .ok_or("naval mine enemy owned-state count unavailable")?;
                        if count < 1 {
                            return Ok(false);
                        }
                        Ok(*capitulations
                            .as_ref()
                            .map_err(Clone::clone)?
                            .get(&target)
                            .ok_or("naval mine enemy capitulation unavailable")?
                            == 0)
                    },
                    |raw| levels.amount(raw, region),
                )?;
                let enemy = if enemy < 0 {
                    0
                } else if max < enemy {
                    max
                } else {
                    enemy
                };
                let enemy = ratio(enemy, max);
                Ok((ours > 0.0 || enemy > 0.0).then_some([ours, enemy]))
            })();
            let accidents = accidents
                .as_ref()
                .map(|counts| counts.get(&region).copied().unwrap_or(0))
                .map_err(Clone::clone);
            (
                region,
                Packet {
                    mines,
                    mine_constructed: features.clone(),
                    accidents,
                    grid_counts: [0; 3],
                },
            )
        })
        .collect()
}

fn shown(
    instance: &mut Instance,
    root: &native::Element,
    element: &native::Element,
    yes: bool,
) -> Fact<()> {
    if yes {
        instance.set_hide_flag(root, element, false)?;
        instance.gui_show(root, element)
    } else {
        instance.hide(root, element)?;
        instance.set_hide_flag(root, element, true)
    }
}

pub fn apply(
    instance: &mut Instance,
    root: &native::Element,
    resources: &dyn native_gui::Resources,
    packet: &Packet,
    report: &mut CounterReport,
) -> Fact<()> {
    let mine_calls = (|| -> Fact<()> {
        if let Some(container) = root.find(native::Kind::Container, "mine_status_container") {
            match &packet.mines {
                Ok(Some(values)) => {
                    shown(instance, root, container, true)?;
                    for (name, &fraction) in ["progress_ours", "progress_enemies"]
                        .into_iter()
                        .zip(values)
                    {
                        let Some(control) = container.find(native::Kind::Icon, name) else {
                            continue;
                        };
                        let sprite = control
                            .icon()
                            .and_then(|icon| resources.sprite(&icon.sprite))
                            .ok_or_else(|| format!("{name} drawable unavailable"))?;
                        let native_gui::SpriteKind::Progress { steps, .. } = sprite.kind else {
                            return Err(format!("{name} drawable is not a progress bar"));
                        };
                        instance.set_progress_float_value(
                            root,
                            control,
                            native_gui::normalized_progress_value(fraction, steps),
                        )?;
                    }
                }
                Err(why) => {
                    *report
                        .naval_mission_gaps
                        .entry(format!("mine status: {why}").into())
                        .or_default() += 1;
                }
                _ => {}
            }
        }
        Ok(())
    })();
    if let Err(why) = mine_calls {
        *report
            .naval_mission_gaps
            .entry(format!("mine status controls: {why}").into())
            .or_default() += 1;
    }
    if let Some(container) = root.find(native::Kind::Container, "accidents_container") {
        match &packet.accidents {
            Ok(count) if *count > 0 => {
                shown(instance, root, container, true)?;
                if let Some(text) = container.find(native::Kind::TextBox, "count") {
                    instance.set_text(root, text, &count.to_string(), resources)?;
                }
            }
            Err(why) => {
                *report
                    .naval_mission_gaps
                    .entry(format!("accident status: {why}").into())
                    .or_default() += 1;
            }
            _ => {}
        }
    }
    Ok(())
}

fn positions(
    widths: [i32; 4],
    visible: [bool; 4],
    base: [i32; 2],
    offset: i32,
) -> [Option<[i32; 2]>; 4] {
    let total = widths
        .into_iter()
        .zip(visible)
        .filter(|(_, show)| *show)
        .fold(0_i32, |sum, (width, _)| sum.wrapping_add(width));
    let mut x = base[0].wrapping_sub(total / 2);
    std::array::from_fn(|at| {
        if visible[at] && total > 0 {
            let point = [x, base[1].wrapping_add(offset)];
            x = x.wrapping_add(widths[at]);
            Some(point)
        } else {
            None
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub fn relayout(
    instance: &mut Instance,
    root: &native::Element,
    entry: &native::Element,
    packet: &Packet,
    counts: &[Result<usize, String>; 2],
    listed: usize,
    viewport: [i32; 2],
) -> Fact<()> {
    let Some(content) = root.find(native::Kind::Container, "content") else {
        return Ok(());
    };
    let parent = native_gui::container_frame(content, Some(viewport))?.content_size;
    let entry_height = native_gui::container_frame(entry, Some(viewport))?.size[1];
    let mut offset = if listed > 0 {
        0
    } else {
        entry_height.wrapping_neg()
    };
    let names = [
        "mine_status_container",
        "ongoing_combat_container",
        "combat_results_container",
        "accidents_container",
    ];
    let mine_constructed = if content
        .find(native::Kind::Container, "mine_status_container")
        .is_some()
    {
        *packet.mine_constructed.as_ref().map_err(Clone::clone)?
    } else {
        false
    };
    let containers: [Option<&native::Element>; 4] = std::array::from_fn(|at| {
        if at == 0 && !mine_constructed {
            None
        } else {
            content.find(native::Kind::Container, names[at])
        }
    });
    let frames = containers
        .map(|element| {
            element
                .map(|e| native_gui::container_frame(e, Some(parent)))
                .transpose()
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let widths = std::array::from_fn(|at| frames[at].map_or(0, |frame| frame.size[0]));
    let visible = [
        matches!(&packet.mines, Ok(Some(_))),
        counts[0].as_ref().is_ok_and(|&n| n > 0),
        counts[1].as_ref().is_ok_and(|&n| n > 0),
        packet.accidents.as_ref().is_ok_and(|&n| n > 0),
    ];
    let base = frames[0]
        .map(|frame| frame.at)
        .or_else(|| frames.iter().flatten().next().map(|frame| frame.at))
        .unwrap_or([0; 2]);
    let points = positions(widths, visible, base, offset);
    for (container, point) in containers.into_iter().zip(points) {
        if let (Some(container), Some(point)) = (container, point) {
            instance.set_position(root, container, point)?;
        }
    }
    let status_height = frames
        .iter()
        .flatten()
        .map(|f| f.size[1])
        .max()
        .unwrap_or(0)
        .max(0);
    let status_visible = points.iter().any(Option::is_some);
    if !status_visible {
        offset = offset.wrapping_sub(status_height);
    }
    let grid_names = ["unit_grid", "enemy_unit_grid", "enemy_convoy_grid"];
    let grids = grid_names.map(|name| content.find(native::Kind::Grid, name));
    let mut origins = [[0; 2]; 3];
    let mut heights = [0_i32; 3];
    let mut members = [0_usize; 3];
    for at in 0..3 {
        if let Some(element) = grids[at] {
            let grid = element.grid().ok_or("naval status grid type mismatch")?;
            let (origin, _, cell) = native_gui::grid_geometry(element, grid, parent)?;
            origins[at] = origin;
            members[at] =
                native_gui::grid_slots(element, parent, packet.grid_counts[at], None)?.len();
            heights[at] = (members[at] as u32 as i32).wrapping_mul(cell[1]);
        }
    }
    let middle = [
        origins[0][0].wrapping_add(origins[1][0]) / 2,
        origins[0][1].wrapping_add(origins[1][1]) / 2,
    ];
    if members[1] == 0 && members[2] == 0 {
        origins[0] = middle;
    }
    if members[0] == 0 {
        origins[1] = middle;
    }
    origins[0][1] = origins[0][1].wrapping_add(offset);
    origins[1][1] = origins[1][1].wrapping_add(offset);
    origins[2] = [origins[1][0], origins[1][1].wrapping_add(heights[1])];
    for (element, point) in grids.into_iter().zip(origins) {
        if let Some(element) = element {
            instance.control(root, element)?.position = Some(point);
        }
    }
    let grids_height = heights[0].max(heights[1].wrapping_add(heights[2]));
    let total = if listed > 0 { entry_height } else { 0 };
    let total = total
        .wrapping_add(if status_visible { status_height } else { 0 })
        .wrapping_add(grids_height);
    instance.set_position(root, content, [0, (total / 2).wrapping_neg()])
}

#[cfg(test)]
mod tests {
    use super::{enemy_sum, native_gui, positions, ratio};
    #[test]
    fn status_row_centers_only_visible_widths_in_native_order() {
        assert_eq!(
            positions([20, 30, 40, 50], [true, false, true, true], [3, 65], -64),
            [Some([-52, 1]), None, Some([-32, 1]), Some([8, 1])]
        );
        assert_eq!(positions([20; 4], [false; 4], [3, 65], 0), [None; 4]);
    }
    #[test]
    fn progress_normalization_keeps_own_overcapacity_and_native_wrapping() {
        assert_eq!(ratio(150_000, 100_000), 1.5);
        assert_eq!(ratio(-10, 100_000), -0.0001);
        assert_eq!(ratio(i64::MAX, 100_000), 0xffff_ffff_i64 as f32 / 100_000.0);
        assert_eq!(native_gui::normalized_progress_value(0.125, 12), 2);
        assert_eq!(native_gui::normalized_progress_value(-0.125, 12), 0);
        assert_eq!(native_gui::normalized_progress_value(1.5, 12), 12);
    }
    #[test]
    fn enemy_roster_keeps_repeats_and_skips_ineligible_owners_without_amount_demand() {
        assert_eq!(
            enemy_sum(
                &[1, 2, 1],
                |raw| Ok(raw != 2),
                |raw| {
                    assert_eq!(raw, 1);
                    Ok(7)
                }
            ),
            Ok(14)
        );
        assert_eq!(enemy_sum(&[1, 1], |_| Ok(true), |_| Ok(i64::MAX)), Ok(-2));
    }
}
