
use engine_sites::Rva;
use super::{PlanRow, Row, TYPE_LINE, Visit, grow_payload, plan_field};
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};
use engine_native_parts::{carmy_group, ccontroller_area};
use crate::{corders_group, cprovince};

const ENTRY_VTABLE_RVA: Rva = Rva(0x02a0_2698);
const DATA: usize = 0x380;
const STRIDE: usize = 0x48;
const BATCH: usize = 16;
const FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 16,
    },
    RecordField {
        source_offset: 0x20,
        destination_offset: 16,
        len: 8,
    },
];
const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 24).expect("child front layout");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Section {
    pub group: u64,
    pub segment_start: i32,
    pub segment_end: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub area_province_count: i32,
    pub count: Option<u32>,
    pub group_child_count: Option<i32>,
    pub sections: Option<(usize, usize)>,
}
#[derive(Clone, Copy, Default)]
struct OwnerInputs {
    state: u8,
    child_count: i32,
}
#[derive(Clone, Copy, Debug, Default)]
struct Pending {
    plan: usize,
    owner: usize,
    array: Array,
}
#[derive(Default)]
pub struct Scratch {
    groups: Vec<(usize, u64)>,
    owners: Vec<OwnerInputs>,
    pending: Vec<Pending>,
}
impl Scratch {
    pub fn allocated_bytes(&self) -> usize {
        self.groups.capacity() * std::mem::size_of::<(usize, u64)>()
            + self.owners.capacity() * std::mem::size_of::<OwnerInputs>()
            + self.pending.capacity() * std::mem::size_of::<Pending>()
    }
    pub fn needs_group_source(&self) -> bool {
        self.pending.iter().any(|pending| pending.array.count != 0)
    }
    fn begin(&mut self, roots: &[corders_group::Root], plans: usize) -> Result<(), &'static str> {
        self.groups.clear();
        self.owners.clear();
        self.pending.clear();
        engine_allocation::reserve(&mut self.owners, roots.len())
            .map_err(|_| "child front group inputs allocation failed")?;
        engine_allocation::reserve(&mut self.pending, plans)
            .map_err(|_| "child front group inputs allocation failed")?;
        self.owners.resize(roots.len(), OwnerInputs::default());
        Ok(())
    }
    fn index(&mut self, roots: &[corders_group::Root]) -> Result<(), &'static str> {
        if !self.groups.is_empty() {
            return Ok(());
        }
        engine_allocation::reserve(&mut self.groups, roots.len())
            .map_err(|_| "child front group index allocation failed")?;
        self.groups
            .extend(roots.iter().map(|root| (root.pointer(), root.handle)));
        self.groups.sort_unstable_by_key(|&(pointer, _)| pointer);
        if self.groups.windows(2).any(|w| w[0].0 == w[1].0) {
            self.groups.clear();
            return Err("child front census repeats a group pointer");
        }
        Ok(())
    }
    fn child_count(
        &mut self,
        source: Option<&impl Memory>,
        root: &corders_group::Root,
        index: usize,
    ) -> Result<Option<i32>, &'static str> {
        let owner = &mut self.owners[index];
        match owner.state {
            1 => return Ok(None),
            2 => return Ok(Some(owner.child_count)),
            3 => return Err("child front owner discriminator unreadable"),
            4 => return Err("child front group count unreadable"),
            _ => {}
        }
        owner.state = 3;
        let source = source.ok_or("child front owner source unavailable")?;
        if plan_field::<u8>(source, root.pointer(), corders_group::IS_ARMY_GROUP)? == 0 {
            owner.state = 1;
            return Ok(None);
        }
        owner.state = 4;
        let address = root
            .pointer()
            .checked_add(carmy_group::MEMBER_DATA)
            .ok_or("child front group header address overflow")?;
        let mut header = [0_u8; 16];
        if !source.copy(address, &mut header) {
            return Err("child front group count unreadable");
        }
        let offset = carmy_group::MEMBER_COUNT - carmy_group::MEMBER_DATA;
        owner.child_count = i32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
        owner.state = 2;
        Ok(Some(owner.child_count))
    }
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Walk {
    pub sections: usize,
    pub required_sections: usize,
    pub area_failures: usize,
    pub header_failures: usize,
    pub section_failures: usize,
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    at: usize,
    key: Rva,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, key).map(|v| v as u64);
    if at == 0 || expected.is_none() || read::<u64>(memory, at) != expected {
        return Err("child front dependency identity mismatch");
    }
    Ok(())
}

fn area_count(
    memory: &impl Memory,
    base: usize,
    provinces: Array,
    id: u32,
) -> Result<i32, &'static str> {
    let slot = (id != 0)
        .then(|| provinces.element(id, 8))
        .flatten()
        .ok_or("child front first path province outside census")?;
    let province =
        read::<u64>(memory, slot).ok_or("child front province slot unreadable")? as usize;
    authenticate(memory, base, province, cprovince::VTABLE_RVA)?;
    if plan_field::<u32>(memory, province, cprovince::ID)? != id {
        return Err("child front province identity mismatch");
    }
    let area = plan_field::<u64>(memory, province, cprovince::CONTROLLER_AREA)? as usize;
    authenticate(memory, base, area, ccontroller_area::VTABLE_RVA)?;
    plan_field(memory, area, ccontroller_area::PROVINCE_COUNT)
}

fn sections(
    memory: &impl Memory,
    base: usize,
    array: Array,
    limit: u32,
    scratch: &Scratch,
    output: &mut Vec<Section>,
    used: &mut usize,
) -> Result<(usize, usize), &'static str> {
    if limit > array.count {
        return Err("child front prefix outside stored array");
    }
    let start = *used;
    let mut bytes = [0_u8; BATCH * 24];
    let mut decoded = [Section::default(); BATCH];
    for first in (0..limit).step_by(BATCH) {
        let count = (limit - first).min(BATCH as u32) as usize;
        let address = array
            .element(first, STRIDE)
            .ok_or("child front entry address overflow")?;
        let completed = memory.copy_records(
            RecordSources::Strided {
                base: address,
                stride: STRIDE,
                count,
            },
            &LAYOUT,
            &mut bytes[..count * 24],
        );
        for index in 0..count {
            if completed < index * FIELDS.len() + 1 {
                return Err("child front entry unreadable");
            }
            let raw = &bytes[index * 24..(index + 1) * 24];
            let vtable = u64::from_le_bytes(raw[..8].try_into().unwrap());
            if Some(vtable) != engine_sites::at(base, ENTRY_VTABLE_RVA).map(|v| v as u64) {
                return Err("child front entry vtable mismatch");
            }
            let pointer = u64::from_le_bytes(raw[8..16].try_into().unwrap()) as usize;
            if pointer == 0 {
                return Err("child front group is null");
            }
            let at = scratch
                .groups
                .binary_search_by_key(&pointer, |&(pointer, _)| pointer)
                .map_err(|_| "child front group absent from authenticated census")?;
            if completed < (index + 1) * FIELDS.len() {
                return Err("child front entry spans unreadable");
            }
            decoded[index] = Section {
                group: scratch.groups[at].1,
                segment_start: i32::from_le_bytes(raw[16..20].try_into().unwrap()),
                segment_end: i32::from_le_bytes(raw[20..24].try_into().unwrap()),
            };
        }
        let end = used
            .checked_add(count)
            .ok_or("child front arena overflow")?;
        grow_payload(output, end, "child front arena allocation failed")?;
        output[*used..end].copy_from_slice(&decoded[..count]);
        *used = end;
    }
    Ok((start, *used - start))
}

#[allow(clippy::too_many_arguments)]
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    roots: &[corders_group::Root],
    visits: &[Visit],
    orders: &[Row],
    paths: &[u32],
    source: Outcome,
    plans: &mut [PlanRow],
    output: &mut Vec<Section>,
    scratch: &mut Scratch,
    group_source: Option<&impl Memory>,
) -> Walk {
    let areas = walk_areas(
        memory,
        base,
        game_state,
        roots,
        visits,
        orders,
        paths,
        source,
        plans,
        &[],
        &[],
        scratch,
    );
    walk_groups(
        memory,
        base,
        roots,
        plans,
        output,
        scratch,
        group_source,
        areas,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn walk_areas(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    roots: &[corders_group::Root],
    visits: &[Visit],
    orders: &[Row],
    paths: &[u32],
    source: Outcome,
    plans: &mut [PlanRow],
    placement: &[u32],
    census: &[u32],
    scratch: &mut Scratch,
) -> Walk {
    let mut report = Walk::default();
    for plan in plans.iter_mut() {
        plan.child_fronts = None;
    }
    scratch.pending.clear();
    if source != (Outcome::Rows { len: plans.len() })
        || orders.len() != plans.len()
        || scratch.begin(roots, plans.len()).is_err()
    {
        report.area_failures = plans.len();
        return report;
    }
    let mut provinces = None;
    for (ordinal, (_, visit)) in super::census_orders(visits, census).enumerate() {
        let Some(root) = roots.get(visit.owner) else {
            report.area_failures += 1;
            continue;
        };
        let key = (u128::from(root.handle) << 32) | u128::from(visit.instance);
        let guess = placement.get(ordinal).copied();
        let Ok(index) = super::keyed_row(plans, guess, key, |plan| plan.key()) else {
            report.area_failures += 1;
            continue;
        };
        let plan = &mut plans[index];
        if plan.front_valid == 0 {
            continue;
        }
        let Ok(order_index) =
            super::keyed_row(orders, Some(index as u32), key, |order| order.key())
        else {
            report.area_failures += 1;
            continue;
        };
        let order = &orders[order_index];
        if order.path_len == 0 {
            continue;
        }
        let Some(&first) = paths.get(order.path_start) else {
            report.area_failures += 1;
            continue;
        };
        let province_table = *provinces
            .get_or_insert_with(|| crate::cprovince::invasion_provinces(memory, game_state));
        let Ok(area) =
            province_table.and_then(|provinces| area_count(memory, base, provinces, first))
        else {
            report.area_failures += 1;
            continue;
        };
        let mut inputs = Inputs {
            area_province_count: area,
            ..Default::default()
        };
        if area > 2 {
            let array = visit
                .pointer
                .checked_add(DATA)
                .and_then(|at| Array::read(memory, at, STRIDE));
            if let Some(array) = array {
                inputs.count = Some(array.count);
                if array.count == 0 {
                    inputs.sections = Some((0, 0));
                }
                if array.count == 0 || order.order_type == TYPE_LINE {
                    if scratch.pending.len() < scratch.pending.capacity() {
                        scratch.pending.push(Pending {
                            plan: index,
                            owner: visit.owner,
                            array,
                        });
                    } else if array.count != 0 {
                        report.section_failures += 1;
                    }
                }
            } else {
                report.header_failures += 1;
            }
        }
        plan.child_fronts = Some(inputs);
    }
    report
}

#[allow(clippy::too_many_arguments)]
pub fn walk_groups(
    memory: &impl Memory,
    base: usize,
    roots: &[corders_group::Root],
    plans: &mut [PlanRow],
    output: &mut Vec<Section>,
    scratch: &mut Scratch,
    group_source: Option<&impl Memory>,
    areas: Walk,
) -> Walk {
    let mut report = areas;
    for at in 0..scratch.pending.len() {
        let Pending { plan, owner, array } = scratch.pending[at];
        let (Some(root), Some(Some(inputs))) = (
            roots.get(owner),
            plans.get_mut(plan).map(|row| row.child_fronts.as_mut()),
        ) else {
            continue;
        };
        if array.count == 0 {
            inputs.sections = Some((report.sections, 0));
            continue;
        }
        match scratch.child_count(group_source, root, owner) {
            Ok(Some(count)) => {
                inputs.group_child_count = Some(count);
                if count > 0 {
                    let before = report.sections;
                    match scratch.index(roots).and_then(|()| {
                        sections(
                            memory,
                            base,
                            array,
                            array.count.min(count as u32),
                            scratch,
                            output,
                            &mut report.sections,
                        )
                    }) {
                        Ok(span) => inputs.sections = Some(span),
                        Err(_) => {
                            report.sections = before;
                            report.section_failures += 1;
                        }
                    }
                } else if count < 0 {
                    report.section_failures += 1;
                }
            }
            Ok(None) => {}
            Err(_) => {
                report.section_failures += 1;
            }
        }
    }
    scratch.pending.clear();
    report.required_sections = report.sections;
    report
}

#[cfg(test)]
mod tests;
