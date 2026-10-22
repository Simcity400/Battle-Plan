//! Stored organisation operands acquired from the existing authenticated census.
//! Content definitions are resolved offline; no native getters are called.
use crate::{Memory, allocation, cpdx_array::Array, read};
use engine_native::{cgame_date, msvc_tree_node::Cursor};
use engine_scripts::{cflag_manager, cscript_flag, cvariables};

pub const RESEARCH_BONUS: usize = 0xa0;
pub const TASK_CAPACITY: usize = 0xa8;
pub const RESEARCH_ASSIGN_COST: usize = 0xb0;
pub const PRODUCTION_ASSIGN_COST: usize = 0xb8;
pub const DESIGN_TEAM_CHANGE_COST: usize = 0xc0;
pub const FUNDS_GAIN_FACTOR: usize = 0xc8;
pub const SIZE_COST_FACTOR: usize = 0xd0;
pub const FUNDS: usize = 0x128;
pub const SIZE: usize = 0x130;
pub const POINTS: usize = 0x134;
pub const UPGRADES: usize = 0x138;
pub const UNLOCKED: usize = 0x140;
pub const QUEUED: usize = 0x150;
pub const QUEUE_INDEX: usize = 0x168;
pub const ALLOWED_POLICIES: usize = 0x170;
pub const POLICY: usize = 0x188;
pub const COOLDOWN: usize = 0x190;
pub const COOLDOWN_ACTIVE: usize = 0x1a8;
pub const SOURCE_LOCATION: usize = 0x1b0;
pub const SOURCE_LOCATION_ACTIVE: usize = 0x1b8;
pub const VARIABLES: usize = 0x1c0;
pub const FLAGS: usize = 0x1f8;
const TRAIT_ID_VTABLE_RVA: usize = 0x0272_f368;
const TECHNOLOGY_VTABLE_RVA: usize = 0x0295_e2b8;
const TECHNOLOGY_TEMPLATE: usize = 0x160;
const TECHNOLOGY_STATUS: usize = 0x158;
const TECHNOLOGY_HANDLE: usize = 8;
const TECHNOLOGY_STATUS_COUNTRY: usize = 0x130;
const TECHNOLOGY_TEMPLATE_INDEX: usize = 0x38;
const TECHNOLOGY_TEMPLATE_TOKEN: usize = 0x3c;
const UNLOCKED_TRAIT_VALUE: usize = 0x20;
const UNLOCKED_TRAIT_TOKEN: usize = 0x28;
const TASK_VTABLE_RVA: usize = 0x027d_1008;
const POLICY_NAME: usize = 0x10;
const POLICY_COUNTRY: usize = 0x30;
const POLICY_CACHED_SCORE: usize = 0x50;
const POLICY_COST: usize = 0x48;
const POLICY_DAYS: usize = 0x4c;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct State {
    pub research_bonus_raw: i64,
    pub task_capacity: i32,
    pub research_assign_cost_raw: i64,
    pub production_assign_cost_raw: i64,
    pub design_team_change_cost: i32,
    pub funds_gain_factor_raw: i64,
    pub size_cost_factor_raw: i64,
    pub funds_raw: i64,
    pub size: i32,
    pub points: i32,
    pub upgrades: u8,
    pub queue_index: i32,
    pub policy: u32,
    pub cooldown_hour: Option<i32>,
    /// Optional native source-file index and line; inactive storage is not read.
    pub source_location: Option<[u32; 2]>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Policy {
    pub token: u32,
    pub cost: i32,
    pub days: i32,
    pub country: u32,
    pub name: Vec<u8>,
    pub cached_score_raw: i64,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryPolicies {
    pub country: u32,
    pub policies: Vec<Policy>,
    /// The country's effective policy-cost modifier in signed fixed5, read in
    /// the same visit; `None` when its accumulator could not be read.
    pub cost_modifier_raw: Option<i64>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Technology {
    pub country: u32,
    pub handle: u32,
    pub index: i32,
    pub token: u32,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Task {
    /// Every native task slot is retained, including empty slots.
    pub technology: Option<Technology>,
    pub line: Option<u64>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: Vec<u8>,
    pub value_raw: i64,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Variables {
    pub seed: Option<u32>,
    pub entries: Vec<Variable>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Row {
    pub organisation: u64,
    pub country: u32,
    pub state: State,
    pub unlocked: Vec<u32>,
    pub queued: Vec<u32>,
    pub allowed_policies: Vec<u32>,
    pub tasks: Vec<Task>,
    pub variables: Variables,
    pub flags: Vec<cscript_flag::Row>,
}
#[derive(Default)]
pub struct Scratch {
    entries: Vec<cvariables::Row>,
    names: Vec<u8>,
}
impl Scratch {
    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<cvariables::Row>() + self.names.capacity()
    }
}
fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("organisation detail address overflow")
}
fn field<T: engine_native::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("organisation detail field unreadable")
}
fn pointer_field(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, pointer, offset)?)
        .map_err(|_| "organisation detail pointer overflow")
}
fn resize<T: Default>(output: &mut Vec<T>, len: usize) -> Result<(), &'static str> {
    allocation::resize_default(output, len).map_err(|_| "organisation detail allocation failed")
}
fn tokens(
    memory: &impl Memory,
    pointer: usize,
    stride: usize,
    offset: usize,
    output: &mut Vec<u32>,
) -> Result<(), &'static str> {
    let array = Array::read(memory, pointer, stride)
        .ok_or("organisation token array unreadable or invalid")?;
    resize(output, array.count as usize)?;
    for (index, token) in output.iter_mut().enumerate() {
        *token = field(
            memory,
            array
                .element(index as u32, stride)
                .ok_or("organisation token address overflow")?,
            offset,
        )?;
    }
    Ok(())
}
#[cfg(test)]
pub fn capture_policies(
    memory: &impl Memory,
    base: usize,
    status: usize,
    output: &mut Vec<Policy>,
) -> Result<(), &'static str> {
    capture_policies_selected(memory, base, status, output, true)
}

/// Policy cost/cooldown effects can write directly. Always read their scalar span
/// from this daily visit; the certificate covers only identity/name/context/score.
pub fn capture_policies_selected(
    memory: &impl Memory,
    base: usize,
    status: usize,
    output: &mut Vec<Policy>,
    fresh_cached: bool,
) -> Result<(), &'static str> {
    let policies = Array::read(memory, at(status, crate::cproduction_status::POLICIES)?, 8)
        .ok_or("organisation country policy array invalid")?;
    let fresh_cached = fresh_cached || output.len() != policies.count as usize;
    resize(output, policies.count as usize)?;
    for (index, output) in output.iter_mut().enumerate() {
        let pointer = pointer_field(
            memory,
            policies
                .element(index as u32, 8)
                .ok_or("organisation country policy address overflow")?,
            0,
        )?;
        let mut identity = [0_u8; crate::cproduction_status::POLICY_TOKEN + 4];
        let mut scalars = [0_u8; POLICY_DAYS + 4 - POLICY_COST];
        if memory.copy_scattered(
            [pointer, at(pointer, POLICY_COST)?],
            &mut [&mut identity, &mut scalars],
        ) != 2
        {
            return Err("organisation policy fields unreadable");
        }
        if u64::from_le_bytes(identity[..8].try_into().unwrap())
            != base
                .checked_add(crate::cproduction_status::POLICY_VTABLE_RVA)
                .ok_or("organisation policy module overflow")? as u64
        {
            return Err("organisation policy vtable mismatch");
        }
        let token = u32::from_le_bytes(
            identity[crate::cproduction_status::POLICY_TOKEN..][..4]
                .try_into()
                .unwrap(),
        );
        output.cost = i32::from_le_bytes(scalars[..4].try_into().unwrap());
        output.days = i32::from_le_bytes(scalars[4..8].try_into().unwrap());
        if fresh_cached || output.token != token {
            output.token = token;
            output.country = field(memory, pointer, POLICY_COUNTRY)?;
            output.cached_score_raw = field(memory, pointer, POLICY_CACHED_SCORE)?;
            let name =
                engine_native::msvc_string::Descriptor::read(memory, at(pointer, POLICY_NAME)?)?;
            name.copy_buffer(
                memory,
                &mut crate::walk::Buffer::Growing(&mut output.name),
                0,
            )?;
            output.name.truncate(name.len);
        }
    }
    Ok(())
}
/// The effective policy-cost modifier of a country the caller has
/// authenticated: the owner's term of every attach quote. Feature eligibility
/// and the definition's clamp apply as the game's getter applies them; an
/// absent entry is zero.
pub fn policy_cost_modifier(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<i64, &'static str> {
    crate::modifier_accumulator::requested_effective_at(
        memory,
        base,
        at(country, crate::ccountry::MODIFIER_ACCUMULATOR)?,
        crate::modifier_keys::MILITARY_INDUSTRIAL_ORGANIZATION_POLICY_COST,
    )
}

/// The caller has authenticated this organisation in its country's roster.
/// The caller owns scratch across all MIOs; no per-object temporary arenas.
pub fn capture(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    organisation: u64,
    country: u32,
    row: &mut Row,
    scratch: &mut Scratch,
) -> Result<(), &'static str> {
    capture_selected(
        memory,
        base,
        pointer,
        organisation,
        country,
        row,
        scratch,
        Freshness::ALL,
    )
}
/// Domains whose certificates did not stand, or whose sample is being verified.
#[derive(Clone, Copy)]
pub struct Freshness {
    pub rosters: bool,
    pub variables: bool,
    pub flags: bool,
}
impl Freshness {
    pub const ALL: Self = Self {
        rosters: true,
        variables: true,
        flags: true,
    };
}
#[allow(clippy::too_many_arguments)]
pub fn capture_selected(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    organisation: u64,
    country: u32,
    row: &mut Row,
    scratch: &mut Scratch,
    fresh: Freshness,
) -> Result<(), &'static str> {
    row.organisation = organisation;
    row.country = country;
    // Effects can assign these operands directly, bypassing adjustment helpers.
    // Read every scalar inside the existing daily census; only rosters/stores carry.
    let mut bases = [0_u8; SIZE_COST_FACTOR + 8 - RESEARCH_BONUS];
    let mut daily = [0_u8; UPGRADES + 1 - FUNDS];
    let mut cursor = [0_u8; 4];
    let mut policy = [0_u8; 4];
    let mut cooldown_active = [0_u8; 1];
    let mut source_active = [0_u8; 1];
    if memory.copy_scattered(
        [
            at(pointer, RESEARCH_BONUS)?,
            at(pointer, FUNDS)?,
            at(pointer, QUEUE_INDEX)?,
            at(pointer, POLICY)?,
            at(pointer, COOLDOWN_ACTIVE)?,
            at(pointer, SOURCE_LOCATION_ACTIVE)?,
        ],
        &mut [
            &mut bases,
            &mut daily,
            &mut cursor,
            &mut policy,
            &mut cooldown_active,
            &mut source_active,
        ],
    ) != 6
    {
        return Err("organisation detail field unreadable");
    }
    let fixed = |offset: usize| {
        i64::from_le_bytes(bases[offset - RESEARCH_BONUS..][..8].try_into().unwrap())
    };
    let integer = |offset: usize| {
        i32::from_le_bytes(bases[offset - RESEARCH_BONUS..][..4].try_into().unwrap())
    };
    row.state = State {
        research_bonus_raw: fixed(RESEARCH_BONUS),
        task_capacity: integer(TASK_CAPACITY),
        research_assign_cost_raw: fixed(RESEARCH_ASSIGN_COST),
        production_assign_cost_raw: fixed(PRODUCTION_ASSIGN_COST),
        design_team_change_cost: integer(DESIGN_TEAM_CHANGE_COST),
        funds_gain_factor_raw: fixed(FUNDS_GAIN_FACTOR),
        size_cost_factor_raw: fixed(SIZE_COST_FACTOR),
        funds_raw: i64::from_le_bytes(daily[..8].try_into().unwrap()),
        size: i32::from_le_bytes(daily[SIZE - FUNDS..][..4].try_into().unwrap()),
        points: i32::from_le_bytes(daily[POINTS - FUNDS..][..4].try_into().unwrap()),
        upgrades: daily[UPGRADES - FUNDS],
        queue_index: i32::from_le_bytes(cursor),
        policy: u32::from_le_bytes(policy),
        cooldown_hour: if cooldown_active[0] != 0 {
            Some(field(memory, pointer, COOLDOWN + cgame_date::DATE_BASE)?)
        } else {
            None
        },
        source_location: if source_active[0] != 0 {
            let bytes = field::<[u8; 8]>(memory, pointer, SOURCE_LOCATION)?;
            Some([
                u32::from_le_bytes(bytes[..4].try_into().unwrap()),
                u32::from_le_bytes(bytes[4..].try_into().unwrap()),
            ])
        } else {
            None
        },
    };
    if fresh.rosters {
        let words = field::<[u8; 16]>(memory, pointer, UNLOCKED)?;
        let (head, _) = Cursor::header(words)?;
        let mut cursor = Cursor::from_parts(
            words,
            read(memory, head).ok_or("organisation unlocked sentinel unreadable")?,
        )?;
        resize(&mut row.unlocked, cursor.count() as usize)?;
        let mut index = 0;
        while let Some(node) = cursor.next(memory)? {
            if field::<u64>(memory, node, UNLOCKED_TRAIT_VALUE)?
                != base
                    .checked_add(TRAIT_ID_VTABLE_RVA)
                    .ok_or("organisation trait module overflow")? as u64
            {
                return Err("organisation unlocked trait identity mismatch");
            }
            row.unlocked[index] = field(memory, node, UNLOCKED_TRAIT_TOKEN)?;
            index += 1;
        }
        tokens(memory, at(pointer, QUEUED)?, 16, 8, &mut row.queued)?;
        tokens(
            memory,
            at(pointer, ALLOWED_POLICIES)?,
            4,
            0,
            &mut row.allowed_policies,
        )?;
    }
    let tasks = Array::read(memory, at(pointer, super::PRODUCTION_LINES)?, 24)
        .ok_or("organisation task array unreadable or invalid")?;
    resize(&mut row.tasks, tasks.count as usize)?;
    for (index, task) in row.tasks.iter_mut().enumerate() {
        let address = tasks
            .element(index as u32, 24)
            .ok_or("organisation task address overflow")?;
        let bytes = read::<[u8; 24]>(memory, address).ok_or("organisation task unreadable")?;
        let fields: [usize; 3] = std::array::from_fn(|index| {
            u64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap()) as usize
        });
        if fields[0]
            != base
                .checked_add(TASK_VTABLE_RVA)
                .ok_or("organisation task module overflow")?
        {
            return Err("organisation task vtable mismatch");
        }
        task.technology = if fields[1] == 0 {
            None
        } else {
            if pointer_field(memory, fields[1], 0)?
                != base
                    .checked_add(TECHNOLOGY_VTABLE_RVA)
                    .ok_or("organisation technology module overflow")?
            {
                return Err("organisation task technology vtable mismatch");
            }
            let template = pointer_field(memory, fields[1], TECHNOLOGY_TEMPLATE)?;
            let status = pointer_field(memory, fields[1], TECHNOLOGY_STATUS)?;
            let owner = pointer_field(memory, status, TECHNOLOGY_STATUS_COUNTRY)?;
            Some(Technology {
                country: field(memory, owner, crate::ccountry::ID)?,
                handle: field(memory, fields[1], TECHNOLOGY_HANDLE)?,
                index: field(memory, template, TECHNOLOGY_TEMPLATE_INDEX)?,
                token: field(memory, template, TECHNOLOGY_TEMPLATE_TOKEN)?,
            })
        };
        task.line = if fields[2] == 0 {
            None
        } else {
            let header = field::<[u8; 16]>(memory, fields[2], 0)?;
            let vtable = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
                .map_err(|_| "organisation line pointer overflow")?;
            if ![
                engine_native::cmilitary_production_line::VTABLE_RVA,
                engine_native::cnaval_production_line::VTABLE_RVA,
                engine_native::cship_refit_production_line::VTABLE_RVA,
                engine_native::crailway_gun_production_line::VTABLE_RVA,
            ]
            .iter()
            .any(|&rva| base.checked_add(rva) == Some(vtable))
            {
                return Err("organisation task line identity mismatch");
            }
            Some(u64::from_le_bytes(header[8..].try_into().unwrap()))
        };
    }
    if fresh.variables {
        let Scratch { entries, names } = scratch;
        let (variables, seed) = cvariables::read_store_carried(
            memory,
            base,
            cvariables::Scope::Global,
            at(pointer, VARIABLES)?,
            crate::walk::Buffer::Growing(entries),
            crate::walk::Buffer::Growing(names),
            &mut cvariables::Fresh,
        )?;
        let crate::walk::Outcome::Rows { len } = variables.outcome else {
            return Err("organisation variable store incomplete");
        };
        row.variables.seed = seed;
        resize(&mut row.variables.entries, len)?;
        for (output, entry) in row.variables.entries.iter_mut().zip(entries.iter()) {
            let bytes = names
                .get(
                    entry.name_start
                        ..entry
                            .name_start
                            .checked_add(entry.name_len)
                            .ok_or("organisation variable name overflow")?,
                )
                .ok_or("organisation variable name outside arena")?;
            resize(&mut output.name, bytes.len())?;
            output.name.copy_from_slice(bytes);
            output.value_raw = entry.value_raw;
        }
        row.variables
            .entries
            .sort_unstable_by(|a, b| a.name.cmp(&b.name));
    }
    if fresh.flags {
        match cflag_manager::walk_growing(memory, base, at(pointer, FLAGS)?, &mut row.flags) {
            crate::walk::Outcome::Rows { len } => row.flags.truncate(len),
            crate::walk::Outcome::Unavailable { reason } => return Err(reason),
            _ => return Err("organisation flag store incomplete"),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
