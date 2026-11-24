
use super::{
    chain::{self, Pair},
    fixed::Point,
};
use replay_data::{
    rows::ArmyOrderPlanRow, rows::DivisionRow, rows::OrderChildFront, rows::OrdersGroupPlanRow,
    rows::OrdersGroupRow, rows::Recorded,
};
use std::{cell::OnceCell, collections::BTreeMap};

use viewer_gpu::ribbon_content::GROUP_SECTION_TYPE_NAME as TYPE_NAME;

pub fn extra_units(plan: &ArmyOrderPlanRow) -> Result<u8, &'static str> {
    let inputs = match &plan.child_fronts {
        Recorded::NotRecorded => return Err("front controller-area and child inputs not recorded"),
        Recorded::Value(None) => return Err("front controller-area inputs unavailable"),
        Recorded::Value(Some(inputs)) => inputs,
    };
    if inputs.area_province_count <= 2 {
        return Ok(0);
    }
    inputs
        .count
        .map(|count| u8::from(count != 0))
        .ok_or("front child count unavailable")
}

#[derive(Clone, Copy)]
pub struct Child<'a> {
    pub slot: u32,
    pub group: u64,
    pub pairs: &'a [Pair],
}

#[derive(Default)]
pub struct Children<'a> {
    pub rows: Vec<Child<'a>>,
    pub refused: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Prepared {
    pub slot: u32,
    pub group: u64,
    pub controls: Vec<Point>,
    pub side: i32,
}

pub struct DivisionGroups<'a> {
    rows: Option<&'a [DivisionRow]>,
    index: OnceCell<BTreeMap<u64, u64>>,
}
impl<'a> DivisionGroups<'a> {
    pub fn read(frame: &'a replay_frame::frame::DecodedFrame) -> Self {
        Self {
            rows: frame
                .complete::<replay_data::rows::tables::Divisions>()
                .ok(),
            index: OnceCell::new(),
        }
    }
    fn get(&self, unit: u64) -> Option<&u64> {
        let rows = self.rows?;
        self.index
            .get_or_init(|| rows.iter().map(|row| (row.id, row.orders_group)).collect())
            .get(&unit)
    }
}

pub struct Display<'a> {
    pub plan: &'a ArmyOrderPlanRow,
    pub group_plans: &'a BTreeMap<u64, &'a OrdersGroupPlanRow>,
    pub groups: &'a BTreeMap<u64, &'a OrdersGroupRow>,
    pub division_groups: &'a DivisionGroups<'a>,
    pub owner_is_ours: Option<bool>,
    pub kind: dashboard_core::protocol::PlanKind,
}

fn assigned_count(child: u64, display: &Display<'_>) -> Result<usize, &'static str> {
    if display.plan.virtual_order != 0 || display.plan.assigned_divisions.is_empty() {
        return Ok(0);
    }
    let group = display
        .groups
        .get(&child)
        .ok_or("army-group child group row unavailable")?;
    let hq = if group.leader != 0 {
        group.hq_provider
    } else {
        0
    };
    let filter = child != display.plan.owner_group && display.plan.owner_override == 0;
    let mut count = 0;
    for &unit in &display.plan.assigned_divisions {
        if unit == hq {
            continue;
        }
        if filter {
            let group = display
                .division_groups
                .get(unit)
                .ok_or("army-group child assigned division group unavailable")?;
            if *group != child {
                continue;
            }
        }
        count += 1;
    }
    Ok(count)
}

pub fn prepare(
    plan: &ArmyOrderPlanRow,
    pairs: &[Pair],
    world: &chain::ChainWorld<'_>,
    arrow: Option<&super::ArrowType>,
    defines: &super::OrderDefines,
) -> Result<(Vec<Prepared>, Vec<&'static str>), &'static str> {
    let inputs = match &plan.child_fronts {
        Recorded::Value(Some(inputs)) => inputs,
        _ => return Err("army-group child inputs unavailable"),
    };
    if inputs.area_province_count <= 2 || inputs.count == Some(0) {
        return Ok((Vec::new(), Vec::new()));
    }
    let choices = children(plan, pairs)?;
    if choices.rows.is_empty() {
        return Ok((Vec::new(), choices.refused));
    }
    let arrow = arrow.ok_or("army-group child arrow type is unavailable in recorded content")?;
    let mut refused = choices.refused;
    let mut prepared = Vec::new();
    for child in choices.rows {
        if child.pairs.is_empty() {
            continue;
        }
        let Some(built) = chain::build(
            child.pairs,
            world,
            chain::ChainParams {
                subdivided: true,
                smoothing: defines.root_front,
                extra_units: 0,
                offset: defines.root_front_offset,
                max_intersection_tests: defines.root_front_max_intersection_tests_order,
                probe_teeth: arrow.culling_direction != 0,
            },
        ) else {
            refused.push("army-group child border chain could not be built");
            continue;
        };
        prepared.push(Prepared {
            slot: child.slot,
            group: child.group,
            controls: built.points,
            side: built.side.unwrap_or(0),
        });
    }
    Ok((prepared, refused))
}

#[expect(clippy::too_many_arguments, reason = "the field and labels it fills, the order inputs and key, its children and the refusals it counts")]
pub fn emit(
    field: &mut super::OrderField,
    labels: &mut Vec<super::WorldName>,
    inputs: &super::OrderInputs<'_>,
    key: super::OrderKey,
    children: &[Prepared],
    display: &Display<'_>,
    callback: viewer_gpu::ribbon_content::ExecutionState,
    refused: &mut BTreeMap<&'static str, usize>,
) {
    if children.is_empty() {
        return;
    }
    let Some(arrow) = inputs.assets.types.get(TYPE_NAME) else {
        *refused
            .entry("army-group child arrow type is unavailable in recorded content")
            .or_default() += children.len();
        return;
    };
    let height = |x, z, _| super::ribbon_height(inputs.heights, x, z);
    for child in children {
        let group = display.group_plans.get(&child.group).copied();
        let color = super::callback_color(
            &inputs.assets.defines,
            display.plan.virtual_order != 0,
            display.owner_is_ours,
            group,
        );
        let Some(color) = color.filter(|color| color.iter().all(|value| value.is_finite())) else {
            *refused
                .entry("army-group child callback colour unavailable or nonfinite")
                .or_default() += 1;
            continue;
        };
        let count = match assigned_count(child.group, display) {
            Ok(value) => value,
            Err(reason) => {
                *refused.entry(reason).or_default() += 1;
                continue;
            }
        };
        let texture_choice =
            match super::texture_choice(arrow, count == 0 && display.plan.virtual_order == 0) {
                Ok(choice) => choice,
                Err(reason) => {
                    *refused.entry(reason).or_default() += 1;
                    continue;
                }
            };
        let Some(draw) = super::emit(
            field,
            arrow,
            &child.controls,
            arrow.global_scale(child.controls.len()),
            color,
            &height,
        ) else {
            continue;
        };
        field.draws[draw].texture_choice = texture_choice;
        field.draws[draw].execution = callback;
        field.draws[draw].kind = display.kind;
        field.draws[draw].culling_offset =
            super::culling_offset(arrow.culling_direction, child.side);
        field.report.child_sections_drawn += 1;
        let Some(label_inputs) = inputs.labels.filter(|_| display.plan.virtual_order == 0) else {
            continue;
        };
        let group_kind = display.groups.get(&child.group).copied();
        let composer = super::labels::Composer {
            localisation: label_inputs.localisation,
            order_type: super::TYPE_LINE,
            name: super::labels::OrderName::default(),
            group_name: group.map_or(&[][..], |group| group.name.as_slice()),
            army_group: group_kind.is_some_and(|group| group.is_army_group != 0),
            count,
        };
        let label_height = |x, z, _| inputs.heights.native_height(x, z);
        match super::labels::place(
            &|variant| {
                if variant == super::labels::Variant::Full {
                    if count == 0 && group_kind.is_none() {
                        return Err("army-group child kind row unavailable".into());
                    }
                    if (count > 0 || composer.army_group) && group.is_none() {
                        return Err("army-group child name row unavailable".into());
                    }
                }
                composer.drawn(variant)
            },
            &child.controls,
            label_inputs.font,
            arrow.text_size,
            arrow.text_offset,
            &label_height,
        ) {
            Ok(Some(placed)) => {
                let glyphs = placed.glyphs.iter().map(|glyph| {
                    (
                        glyph.corners,
                        glyph.uv,
                        glyph.page,
                        glyph.color.unwrap_or([1.0; 4]),
                    )
                });
                if let Some(name) = super::WorldName::from_glyphs(
                    format!("{}:{}:child:{}", key.0, key.1, child.slot),
                    placed.text,
                    glyphs,
                    color[3],
                ) {
                    field.draws[draw].label = Some(labels.len());
                    labels.push(name);
                    field.report.child_labels_drawn += 1;
                }
            }
            Ok(None) => {}
            Err(reason) => *field.report.labels_refused.entry(reason).or_default() += 1,
        }
    }
}

#[cfg(test)]
mod tests;

pub fn children<'a>(
    plan: &ArmyOrderPlanRow,
    pairs: &'a [Pair],
) -> Result<Children<'a>, &'static str> {
    let inputs = match &plan.child_fronts {
        Recorded::Value(Some(inputs)) => inputs,
        _ => return Err("army-group child inputs unavailable"),
    };
    if inputs.area_province_count <= 2 {
        return Ok(Children::default());
    }
    let count = inputs.count.ok_or("army-group child count unavailable")?;
    if count == 0 {
        return Ok(Children::default());
    }
    let group_count = inputs
        .group_child_count
        .ok_or("army-group current child count unavailable")?;
    let slots =
        usize::try_from(group_count).map_err(|_| "army-group current child count is negative")?;
    if slots == 0 {
        return Ok(Children::default());
    }
    let sections = inputs
        .sections
        .as_ref()
        .ok_or("army-group stored child spans unavailable")?;
    if sections.len() != slots.min(count as usize) {
        return Err("army-group demanded child prefix and spans disagree");
    }
    let mut first: BTreeMap<u64, &OrderChildFront> = BTreeMap::new();
    for section in sections {
        first.entry(section.group).or_insert(section);
    }
    let mut output = Children::default();
    for slot in (0..sections.len()).rev() {
        let indexed = &sections[slot];
        let section = first[&indexed.group];
        if section.segment_start < 0 {
            continue;
        }
        if section.segment_start > section.segment_end {
            output.rows.push(Child {
                slot: slot as u32,
                group: section.group,
                pairs: &pairs[..0],
            });
            continue;
        }
        if plan
            .segment_start
            .checked_add(section.segment_start)
            .is_none()
            || plan
                .segment_start
                .checked_add(section.segment_end)
                .is_none()
        {
            output
                .refused
                .push("army-group child absolute span overflows");
            continue;
        }
        let start = section.segment_start as usize;
        let Some(end) = (section.segment_end as usize).checked_add(1) else {
            output.refused.push("army-group child span overflows");
            continue;
        };
        let Some(child_pairs) = pairs.get(start..end) else {
            output
                .refused
                .push("army-group child span outside its parent reserved pairs");
            continue;
        };
        output.rows.push(Child {
            slot: slot as u32,
            group: section.group,
            pairs: child_pairs,
        });
    }
    Ok(output)
}
