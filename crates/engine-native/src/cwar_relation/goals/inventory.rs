use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_allocation as allocation;
use crate::ccountry;
use engine_native_parts::cwar_goal_type;
use engine_sites::Rva;

pub const SERIALIZABLE_GOALS: usize = 0x80;
pub const HANDLE: usize = 8;
pub const EXPIRES: usize = 0x20;
pub const NAME_TOKEN: usize = 0x30;
pub const ACTOR: usize = 0x38;
pub const LIBERATE: usize = 0x60;
pub const PUPPETS: usize = 0x78;
pub const ACTIVITIES: usize = 8;
pub const JUSTIFICATION_VTABLE_RVA: Rva = Rva(0x029c_1d40);
pub const ACTIVITY_SOURCE: usize = 0x10;
pub const ACTIVITY_TARGET: usize = 0x14;
pub const TOTAL_COST: usize = 0x18;
pub const DAILY_COST: usize = 0x20;
pub const PROGRESS: usize = 0x28;
pub const START: usize = 0x38;
pub const END: usize = 0x50;
pub const PAY_UPFRONT: usize = 0x60;
pub const SELECTED_STATES: usize = 0x68;
pub const ACTIVITY_TYPE: usize = 0x80;
pub const THREAT: usize = 0x88;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Goal {
    pub ordinal: u32,
    pub handle: u64,
    pub actor: u32,
    pub recipient: u32,
    pub goal_type: u32,
    pub name_token: u32,
    pub expires: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Justification {
    pub ordinal: u32,
    pub actor: u32,
    pub recipient: u32,
    pub goal_type: u32,
    pub total_cost: i64,
    pub daily_cost: i64,
    pub progress: i64,
    pub start: i32,
    pub end: i32,
    pub pay_upfront: u8,
    pub threat: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub kind: u8,
    pub goal: u32,
    pub ordinal: u32,
    pub value: u64,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Inputs {
    pub goals: Vec<Goal>,
    pub justifications: Vec<Justification>,
    pub targets: Vec<Target>,
}

fn field<T: engine_native_memory::memory::Raw>(
    m: &impl Memory,
    p: usize,
    offset: usize,
) -> Result<T, &'static str> {
    p.checked_add(offset)
        .and_then(|p| read(m, p))
        .ok_or("war-goal inventory field unreadable or address overflow")
}
fn array(m: &impl Memory, p: usize, offset: usize, stride: usize) -> Result<Array, &'static str> {
    p.checked_add(offset)
        .and_then(|p| Array::read(m, p, stride))
        .ok_or("war-goal inventory array invalid or unreadable")
}
fn pointer(m: &impl Memory, a: Array, n: u32) -> Result<usize, &'static str> {
    a.element(n, 8)
        .and_then(|p| read::<u64>(m, p))
        .map(|p| p as usize)
        .ok_or("war-goal inventory pointer unreadable")
}
fn reserve<T>(v: &mut Vec<T>, n: usize) -> Result<(), &'static str> {
    allocation::reserve(v, n).map_err(|_| "war-goal inventory allocation failed")
}

impl Inputs {
    pub fn allocated_bytes(&self) -> usize {
        self.goals.capacity() * size_of::<Goal>()
            + self.justifications.capacity() * size_of::<Justification>()
            + self.targets.capacity() * size_of::<Target>()
    }

    pub fn read_owner(
        &mut self,
        m: &impl Memory,
        base: usize,
        country: engine_native_memory::country_identity::Country,
        diplomacy: usize,
    ) -> Result<(), &'static str> {
        self.read_parts(m, base, country, diplomacy, [true, true])
    }
    pub fn read_parts(
        &mut self,
        m: &impl Memory,
        base: usize,
        country: engine_native_memory::country_identity::Country,
        diplomacy: usize,
        changed: [bool; 2],
    ) -> Result<(), &'static str> {
        if changed[0] {
            self.goals.clear();
            self.targets.retain(|row| row.kind == 4);
        }
        if changed[1] {
            self.justifications.clear();
            self.targets.retain(|row| row.kind != 4);
        }
        if changed[0] {
            let goals = array(m, diplomacy, SERIALIZABLE_GOALS, 8)?;
            reserve(&mut self.goals, goals.count as usize)?;
            for ordinal in 0..goals.count {
                let p = pointer(m, goals, ordinal)?;
                if p == 0
                    || field::<u64>(m, p, 0)?
                        != engine_sites::at(base, super::VTABLE_RVA)
                            .ok_or("war-goal vtable withheld")? as u64
                {
                    return Err("war-goal inventory goal vtable mismatch");
                }
                let definition = field::<u64>(m, p, super::GOAL_TYPE)? as usize;
                let row = Goal {
                    ordinal,
                    handle: field(m, p, HANDLE)?,
                    actor: field(m, p, ACTOR)?,
                    recipient: field(m, p, super::RECIPIENT)?,
                    goal_type: cwar_goal_type::token(m, base, definition)?,
                    name_token: field(m, p, NAME_TOKEN)?,
                    expires: field(m, p, EXPIRES)?,
                };
                self.copy_targets(m, base, p, super::STATES, 0, ordinal, true)?;
                self.copy_targets(m, base, p, LIBERATE, 1, ordinal, false)?;
                self.copy_targets(m, base, p, PUPPETS, 2, ordinal, false)?;
                self.goals.push(row);
            }
            let unused = array(m, diplomacy, engine_native_parts::cdiplomacy_status_layout::OWNED_WARGOALS, 8)?;
            reserve(&mut self.targets, unused.count as usize)?;
            for ordinal in 0..unused.count {
                let p = pointer(m, unused, ordinal)?;
                if p != 0
                    && field::<u64>(m, p, 0)?
                        != engine_sites::at(base, super::VTABLE_RVA)
                            .ok_or("war-goal vtable withheld")? as u64
                {
                    return Err("unused war-goal vtable mismatch");
                }
                self.targets.push(Target {
                    kind: 3,
                    goal: 0,
                    ordinal,
                    value: if p == 0 { 0 } else { field(m, p, HANDLE)? },
                });
            }
        }
        if changed[1] {
            let politics = field::<u64>(m, country.pointer, ccountry::POLITICAL_STATUS)? as usize;
            if politics == 0
                || field::<u64>(m, politics, 0)?
                    != engine_sites::at(base, engine_native_parts::cpolitical_status_layout::VTABLE_RVA)
                        .ok_or("political-status vtable withheld")? as u64
                || field::<u64>(m, politics, engine_native_parts::cpolitical_status_layout::COUNTRY)? != country.pointer as u64
            {
                return Err("war-goal justification political-status identity mismatch");
            }
            let activities = array(m, politics, ACTIVITIES, 8)?;
            for ordinal in 0..activities.count {
                let p = pointer(m, activities, ordinal)?;
                if p == 0 {
                    return Err("timed-activity pointer is null");
                }
                if field::<u64>(m, p, 0)?
                    != engine_sites::at(base, JUSTIFICATION_VTABLE_RVA)
                        .ok_or("justification vtable withheld")? as u64
                {
                    continue;
                }
                reserve(&mut self.justifications, 1)?;
                let definition = field::<u64>(m, p, ACTIVITY_TYPE)? as usize;
                let row = Justification {
                    ordinal,
                    actor: field(m, p, ACTIVITY_SOURCE)?,
                    recipient: field(m, p, ACTIVITY_TARGET)?,
                    goal_type: cwar_goal_type::token(m, base, definition)?,
                    total_cost: field(m, p, TOTAL_COST)?,
                    daily_cost: field(m, p, DAILY_COST)?,
                    progress: field(m, p, PROGRESS)?,
                    start: field(m, p, START)?,
                    end: field(m, p, END)?,
                    pay_upfront: field(m, p, PAY_UPFRONT)?,
                    threat: field(m, p, THREAT)?,
                };
                self.copy_targets(m, base, p, SELECTED_STATES, 4, ordinal, false)?;
                self.justifications.push(row);
            }
        }
        self.targets
            .sort_unstable_by_key(|row| (row.kind, row.goal, row.ordinal));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_targets(
        &mut self,
        m: &impl Memory,
        base: usize,
        p: usize,
        offset: usize,
        kind: u8,
        goal: u32,
        state_pointers: bool,
    ) -> Result<(), &'static str> {
        let stride = if state_pointers { 8 } else { 4 };
        let source = array(m, p, offset, stride)?;
        reserve(&mut self.targets, source.count as usize)?;
        for ordinal in 0..source.count {
            let value = if state_pointers {
                let p = pointer(m, source, ordinal)?;
                if p == 0 {
                    0
                } else {
                    if field::<u64>(m, p, 0)?
                        != engine_sites::at(base, crate::cstate::VTABLE_RVA)
                            .ok_or("state vtable withheld")? as u64
                    {
                        return Err("war-goal target state vtable mismatch");
                    }
                    field::<u32>(m, p, crate::cstate::ID)? as u64
                }
            } else {
                source
                    .element(ordinal, 4)
                    .and_then(|p| read::<u32>(m, p))
                    .ok_or("war-goal target unreadable")? as u64
            };
            self.targets.push(Target {
                kind,
                goal,
                ordinal,
                value,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ccountry::synthetic::World,  cstate};

    #[test]
    fn complete_inventory_keeps_used_goals_all_target_lists_and_native_progress() {
        let mut w = World::new(0, 2, 0x1600);
        let country = w.object(1);
        let diplomacy = w.alloc(0x300);
        let politics = w.alloc(0x120);
        w.u64(country + ccountry::POLITICAL_STATUS, politics as u64);
        w.u64(politics, engine_native_parts::cpolitical_status_layout::VTABLE_RVA.0 as u64);
        w.u64(politics + engine_native_parts::cpolitical_status_layout::COUNTRY, country as u64);
        let goal = w.alloc(0x90);
        let used = w.alloc(0x90);
        let definition = w.alloc(0x10);
        w.u64(definition, cwar_goal_type::VTABLE_RVA.0 as u64);
        w.u32(definition + cwar_goal_type::TOKEN, 9123);
        for (p, handle) in [(goal, 41u64 << 32 | 123), (used, 42u64 << 32 | 123)] {
            w.u64(p, super::super::VTABLE_RVA.0 as u64);
            w.u64(p + HANDLE, handle);
            w.u32(p + ACTOR, 1);
            w.u32(p + super::super::RECIPIENT, 2);
            w.u64(p + super::super::GOAL_TYPE, definition as u64);
            w.u32(p + EXPIRES, 43800000 + 57);
            w.u32(p + NAME_TOKEN, 9124);
        }
        let list = w.alloc(16);
        w.u64(list, goal as u64);
        w.u64(list + 8, used as u64);
        w.array(diplomacy + SERIALIZABLE_GOALS, list, 2);
        w.array(diplomacy + engine_native_parts::cdiplomacy_status_layout::OWNED_WARGOALS, list, 1);
        let state = w.alloc(0x100);
        w.u64(state, cstate::VTABLE_RVA.0 as u64);
        w.u32(state + cstate::ID, 5001);
        let states = w.alloc(24);
        w.u64(states, state as u64);
        w.u64(states + 8, 0);
        w.u64(states + 16, state as u64);
        w.array(goal + super::super::STATES, states, 3);
        let tags = w.alloc(12);
        for (i, v) in [2, 1, 2].into_iter().enumerate() {
            w.u32(tags + i * 4, v);
        }
        w.array(goal + LIBERATE, tags, 3);
        w.array(goal + PUPPETS, tags, 2);
        let activity = w.alloc(0x90);
        w.u64(activity, JUSTIFICATION_VTABLE_RVA.0 as u64);
        w.u32(activity + ACTIVITY_SOURCE, 1);
        w.u32(activity + ACTIVITY_TARGET, 2);
        w.u64(activity + ACTIVITY_TYPE, definition as u64);
        w.u64(activity + TOTAL_COST, 9000000);
        w.u64(activity + DAILY_COST, 100000);
        w.u64(activity + PROGRESS, 1700000);
        w.u32(activity + START, 43800024);
        w.u32(activity + END, 43802160);
        w.u64(activity + THREAT, 76543);
        w.array(activity + SELECTED_STATES, tags, 3);
        let activities = w.alloc(8);
        w.u64(activities, activity as u64);
        w.array(politics + ACTIVITIES, activities, 1);
        let mut out = {
            let memory = w.memory();
            let reader = ccountry::Reader::new(&memory, 0, w.game_state()).unwrap();
            let owner = reader.at(1).unwrap().unwrap();
            let mut out = Inputs::default();
            out.read_owner(&memory, 0, owner, diplomacy).unwrap();
            out
        };
        assert_eq!(out.goals.len(), 2);
        assert_eq!(out.goals[1].handle, 42u64 << 32 | 123);
        assert_eq!(out.goals[0].expires, 43800057);
        assert_eq!(out.goals[0].name_token, 9124);
        assert_eq!(out.goals[0].goal_type, 9123);
        assert_eq!(
            out.targets
                .iter()
                .filter(|r| r.kind == 0)
                .map(|r| r.value)
                .collect::<Vec<_>>(),
            [5001, 0, 5001]
        );
        assert_eq!(
            out.targets
                .iter()
                .filter(|r| r.kind == 1)
                .map(|r| r.value)
                .collect::<Vec<_>>(),
            [2, 1, 2]
        );
        assert_eq!(
            out.targets
                .iter()
                .filter(|r| r.kind == 2)
                .map(|r| r.value)
                .collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(
            out.targets
                .iter()
                .filter(|r| r.kind == 3)
                .map(|r| r.value)
                .collect::<Vec<_>>(),
            [41u64 << 32 | 123]
        );
        assert_eq!(out.justifications[0].progress, 1700000);
        assert_eq!(
            out.targets
                .iter()
                .filter(|r| r.kind == 4)
                .map(|r| r.value)
                .collect::<Vec<_>>(),
            [2, 1, 2]
        );
        w.u64(goal, 0);
        let memory = w.memory();
        let reader = ccountry::Reader::new(&memory, 0, w.game_state()).unwrap();
        out.read_parts(
            &memory,
            0,
            reader.at(1).unwrap().unwrap(),
            diplomacy,
            [false, true],
        )
        .unwrap();
        assert_eq!(
            out.goals.len(),
            2,
            "a justification update must retain goals without rereading them"
        );
        assert_eq!(
            out.read_owner(&memory, 0, reader.at(1).unwrap().unwrap(), diplomacy),
            Err("war-goal inventory goal vtable mismatch")
        );
    }
}
